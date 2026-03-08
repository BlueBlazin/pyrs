use super::{
    BoundMethod, BuiltinFunction, ClassObject, DEQUE_BACKING_STORAGE_ATTR, HashMap, InstanceObject,
    InternalCallOutcome, IteratorKind, IteratorObject, MAPPING_PROXY_STORAGE_ATTR, ModuleObject,
    NativeMethodKind, ObjRef, Object, RuntimeError, Value, Vm, add_values, and_values,
    binary_operator, bytes_like_from_value, class_attr_lookup, class_name_for_instance, compare_ge,
    compare_gt, compare_le, compare_lt, dict_remove_value, dict_set_value_checked, ensure_hashable,
    format_repr, is_missing_attribute_error, is_truthy, lshift_values, pow_values, rshift_values,
    runtime_error_matches_exception, unary_predicate, value_to_int, xor_values,
};
use crate::runtime::FunctionObject;

impl Vm {
    pub(super) const FUNCTION_GLOBALS_MAPPING_KEY: &'static str =
        "__pyrs_function_globals_mapping__";

    pub(super) fn builtin_operator_add(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("binary operator expects two arguments"));
        }
        self.binary_add_runtime(args[0].clone(), args[1].clone())
    }

    pub(super) fn builtin_operator_sub(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("binary operator expects two arguments"));
        }
        self.binary_sub_runtime(args[0].clone(), args[1].clone())
    }

    pub(super) fn builtin_operator_mul(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("binary operator expects two arguments"));
        }
        self.binary_mul_runtime(args[0].clone(), args[1].clone())
    }

    pub(super) fn builtin_operator_mod(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("binary operator expects two arguments"));
        }
        self.binary_mod_runtime(args[0].clone(), args[1].clone())
    }

    pub(super) fn builtin_operator_pow(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, pow_values)
    }

    pub(super) fn builtin_operator_and(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| {
            and_values(left, right, &self.heap)
        })
    }

    pub(super) fn builtin_operator_or(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| {
            self.binary_or_runtime(left, right)
        })
    }

    pub(super) fn builtin_operator_xor(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| {
            xor_values(left, right, &self.heap)
        })
    }

    pub(super) fn builtin_operator_lshift(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, lshift_values)
    }

    pub(super) fn builtin_operator_rshift(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, rshift_values)
    }

    pub(super) fn builtin_operator_matmul(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| {
            self.binary_matmul_runtime(left, right)
        })
    }

    pub(super) fn builtin_operator_neg(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("operator.neg expects one argument"));
        }
        self.unary_neg_runtime(args[0].clone())
    }

    pub(super) fn builtin_operator_pos(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("operator.pos expects one argument"));
        }
        self.unary_pos_runtime(args[0].clone())
    }

    pub(super) fn builtin_operator_invert(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("operator.invert expects one argument"));
        }
        self.unary_invert_runtime(args[0].clone())
    }

    pub(super) fn builtin_operator_truediv(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("binary operator expects two arguments"));
        }
        self.binary_div_runtime(args[0].clone(), args[1].clone())
    }

    pub(super) fn builtin_operator_floordiv(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("binary operator expects two arguments"));
        }
        self.binary_floor_div_runtime(args[0].clone(), args[1].clone())
    }

    pub(super) fn builtin_operator_index(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("operator.index expects one argument"));
        }
        Ok(Value::Int(self.io_index_arg_to_int(args[0].clone())?))
    }

    pub(super) fn builtin_operator_eq(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.eq expects two arguments"));
        }
        Ok(Value::Bool(args[0] == args[1]))
    }

    pub(super) fn builtin_object_eq(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("object.__eq__ expects two arguments"));
        }
        if args[0] == args[1] {
            return Ok(Value::Bool(true));
        }
        Ok(self
            .builtins
            .get("NotImplemented")
            .cloned()
            .unwrap_or(Value::None))
    }

    pub(super) fn builtin_object_ne(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("object.__ne__ expects two arguments"));
        }
        let eq_value = match self.lookup_bound_special_method(&args[0], "__eq__")? {
            Some(eq_callable) => {
                match self.call_internal(eq_callable, vec![args[1].clone()], HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(self.runtime_error_from_active_exception(
                            "object.__ne__ comparison failed",
                        ));
                    }
                }
            }
            None => self.compare_eq_runtime(args[0].clone(), args[1].clone())?,
        };
        let is_not_implemented =
            self.builtins
                .get("NotImplemented")
                .is_some_and(|marker| match (&eq_value, marker) {
                    (Value::Instance(left), Value::Instance(right)) => left.id() == right.id(),
                    _ => &eq_value == marker,
                });
        if is_not_implemented {
            return Ok(self
                .builtins
                .get("NotImplemented")
                .cloned()
                .unwrap_or(Value::None));
        }
        Ok(Value::Bool(!self.truthy_from_value(&eq_value)?))
    }

    pub(super) fn builtin_operator_ne(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.ne expects two arguments"));
        }
        Ok(Value::Bool(args[0] != args[1]))
    }

    pub(super) fn builtin_operator_lt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, compare_lt)
    }

    pub(super) fn builtin_operator_le(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, compare_le)
    }

    pub(super) fn builtin_operator_gt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, compare_gt)
    }

    pub(super) fn builtin_operator_ge(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, compare_ge)
    }

    pub(super) fn builtin_operator_contains(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.contains expects two arguments"));
        }
        Ok(Value::Bool(
            self.compare_in_runtime(args[1].clone(), args[0].clone())?,
        ))
    }

    pub(super) fn builtin_operator_getitem(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.getitem expects two arguments"));
        }
        let receiver = args[0].clone();
        let index = args[1].clone();
        if self.is_types_generic_alias_value(&receiver) {
            return self.subscript_generic_alias_value(receiver, index);
        }
        self.getitem_value(receiver, index)
    }

    pub(super) fn builtin_operator_compare_digest(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "_compare_digest() expects two positional arguments",
            ));
        }

        fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
            let mut diff: usize = left.len() ^ right.len();
            let max_len = left.len().max(right.len());
            for idx in 0..max_len {
                let l = *left.get(idx).unwrap_or(&0);
                let r = *right.get(idx).unwrap_or(&0);
                diff |= (l ^ r) as usize;
            }
            diff == 0
        }

        let left_str = match &args[0] {
            Value::Str(text) => Some(text.clone()),
            Value::Instance(instance) => self.instance_backing_str(instance),
            _ => None,
        };
        let right_str = match &args[1] {
            Value::Str(text) => Some(text.clone()),
            Value::Instance(instance) => self.instance_backing_str(instance),
            _ => None,
        };

        match (left_str, right_str) {
            (Some(left), Some(right)) => {
                if !left.is_ascii() || !right.is_ascii() {
                    return Err(RuntimeError::type_error(
                        "comparing strings with non-ASCII characters is not supported",
                    ));
                }
                Ok(Value::Bool(constant_time_eq(
                    left.as_bytes(),
                    right.as_bytes(),
                )))
            }
            (Some(_), None) | (None, Some(_)) => Err(RuntimeError::type_error(
                "a bytes-like object is required, not 'str'",
            )),
            (None, None) => {
                let left = bytes_like_from_value(args[0].clone()).ok();
                let right = bytes_like_from_value(args[1].clone()).ok();
                match (left, right) {
                    (Some(left), Some(right)) => Ok(Value::Bool(constant_time_eq(&left, &right))),
                    _ => Err(RuntimeError::new(
                        "unsupported operand types for _compare_digest()",
                    )),
                }
            }
        }
    }

    pub(super) fn builtin_operator_itemgetter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() {
            return Err(RuntimeError::new(
                "operator.itemgetter expects at least one argument",
            ));
        }
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__operator_itemgetter__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data
                .globals
                .insert("items".to_string(), self.heap.alloc_list(args));
        }
        Ok(self.alloc_native_bound_method(NativeMethodKind::OperatorItemGetterCall, receiver))
    }

    pub(super) fn builtin_operator_attrgetter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() {
            return Err(RuntimeError::new(
                "operator.attrgetter expects at least one argument",
            ));
        }
        let mut attrs = Vec::with_capacity(args.len());
        for attr in args {
            match attr {
                Value::Str(value) => attrs.push(Value::Str(value)),
                _ => return Err(RuntimeError::new("attribute name must be a string")),
            }
        }
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__operator_attrgetter__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data
                .globals
                .insert("attrs".to_string(), self.heap.alloc_list(attrs));
        }
        Ok(self.alloc_native_bound_method(NativeMethodKind::OperatorAttrGetterCall, receiver))
    }

    pub(super) fn builtin_operator_methodcaller(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "operator.methodcaller expects at least one argument",
            ));
        }
        let method_name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("method name must be a string")),
        };
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__operator_methodcaller__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        let frozen_kwargs = kwargs
            .into_iter()
            .map(|(name, value)| (Value::Str(name), value))
            .collect::<Vec<_>>();
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data
                .globals
                .insert("name".to_string(), Value::Str(method_name));
            module_data
                .globals
                .insert("args".to_string(), self.heap.alloc_list(args));
            module_data
                .globals
                .insert("kwargs".to_string(), self.heap.alloc_dict(frozen_kwargs));
        }
        Ok(self.alloc_native_bound_method(NativeMethodKind::OperatorMethodCallerCall, receiver))
    }

    pub(super) fn builtin_itertools_accumulate(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "accumulate() expects iterable and optional function",
            ));
        }
        let iterable = args.remove(0);
        let mut func = if args.is_empty() {
            None
        } else {
            Some(args.remove(0))
        };
        if let Some(value) = kwargs.remove("func") {
            if func.is_some() {
                return Err(RuntimeError::new(
                    "accumulate() got multiple values for func",
                ));
            }
            func = Some(value);
        }
        let initial = kwargs.remove("initial");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "accumulate() got an unexpected keyword argument",
            ));
        }

        let iterator = self.to_iterator_value(iterable)?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Accumulate {
                iterator,
                func,
                total: None,
                initial,
                emitted_initial: false,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_combinations(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("combinations() expects iterable and r"));
        }
        let values = self.collect_iterable_values(args.remove(0))?;
        let r = value_to_int(args.remove(0))?;
        if r < 0 {
            return Err(RuntimeError::new("r must be non-negative"));
        }
        let r = usize::try_from(r).map_err(|_| RuntimeError::new("r is too large"))?;
        let n = values.len();
        let done = r > n;
        let indices = if done { Vec::new() } else { (0..r).collect() };
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Combinations {
                pool: values,
                r,
                indices,
                first: true,
                done,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_combinations_with_replacement(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "combinations_with_replacement() expects iterable and r",
            ));
        }
        let values = self.collect_iterable_values(args.remove(0))?;
        let r = value_to_int(args.remove(0))?;
        if r < 0 {
            return Err(RuntimeError::new("r must be non-negative"));
        }
        let r = usize::try_from(r).map_err(|_| RuntimeError::new("r is too large"))?;
        let done = values.is_empty() && r > 0;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::CombinationsWithReplacement {
                pool: values,
                r,
                indices: vec![0; r],
                first: true,
                done,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_compress(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("compress() expects data and selectors"));
        }
        let data = self.to_iterator_value(args.remove(0))?;
        let selectors = self.to_iterator_value(args.remove(0))?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Compress { data, selectors },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_dropwhile(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "dropwhile() expects predicate and iterable",
            ));
        }
        let predicate = args.remove(0);
        let iterator = self.to_iterator_value(args.remove(0))?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::DropWhile {
                predicate,
                iterator,
                dropping: true,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_filterfalse(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "filterfalse() expects predicate and iterable",
            ));
        }
        let predicate = args.remove(0);
        let iterator = self.to_iterator_value(args.remove(0))?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::FilterFalse {
                predicate,
                iterator,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_groupby(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "groupby() expects iterable and optional key",
            ));
        }
        let iterable = args.remove(0);
        let mut key_func = if args.is_empty() {
            None
        } else {
            Some(args.remove(0))
        };
        if let Some(value) = kwargs.remove("key") {
            if key_func.is_some() {
                return Err(RuntimeError::new("groupby() got multiple values for key"));
            }
            key_func = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "groupby() got an unexpected keyword argument",
            ));
        }
        let source = self.to_iterator_value(iterable)?;
        let shared = match self
            .heap
            .alloc_module(ModuleObject::new("__itertools_groupby_shared__"))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *shared.kind_mut() {
            module_data.globals.insert("source".to_string(), source);
            module_data
                .globals
                .insert("keyfunc".to_string(), key_func.unwrap_or(Value::None));
            module_data
                .globals
                .insert("exhausted".to_string(), Value::Bool(false));
            module_data
                .globals
                .insert("group_index".to_string(), Value::Int(0));
        }
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::GroupBy {
                shared: Value::Module(shared),
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_islice(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 4 {
            return Err(RuntimeError::new(
                "islice() expects iterable and slice indices",
            ));
        }
        let iterable = args.remove(0);

        let parse_opt_index = |value: Value| -> Result<Option<i64>, RuntimeError> {
            if matches!(value, Value::None) {
                Ok(None)
            } else {
                Ok(Some(value_to_int(value)?))
            }
        };

        let (start, stop, step) = match args.len() {
            1 => (0_i64, parse_opt_index(args.remove(0))?, 1_i64),
            2 => (
                value_to_int(args.remove(0))?,
                parse_opt_index(args.remove(0))?,
                1_i64,
            ),
            3 => (
                value_to_int(args.remove(0))?,
                parse_opt_index(args.remove(0))?,
                value_to_int(args.remove(0))?,
            ),
            _ => unreachable!(),
        };
        if start < 0 {
            return Err(RuntimeError::new("islice() start must be non-negative"));
        }
        if let Some(stop) = stop
            && stop < 0
        {
            return Err(RuntimeError::new("islice() stop must be non-negative"));
        }
        if step <= 0 {
            return Err(RuntimeError::new("islice() step must be positive"));
        }

        let iterator = self.to_iterator_value(iterable)?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Islice {
                iterator,
                next_index: start,
                stop,
                step,
                source_index: 0,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_pairwise(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "pairwise() expects one iterable argument",
            ));
        }
        let iterator = self.to_iterator_value(args.remove(0))?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Pairwise {
                iterator,
                previous: None,
                primed: false,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_starmap(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("starmap() expects function and iterable"));
        }
        let callable = args.remove(0);
        let iterator = self.to_iterator_value(args.remove(0))?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::StarMap { callable, iterator },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_takewhile(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "takewhile() expects predicate and iterable",
            ));
        }
        let predicate = args.remove(0);
        let iterator = self.to_iterator_value(args.remove(0))?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::TakeWhile {
                predicate,
                iterator,
                done: false,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_tee(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("tee() expects iterable and optional n"));
        }
        let iterable = args.remove(0);
        let n = if args.is_empty() {
            2
        } else {
            value_to_int(args.remove(0))?
        };
        if n < 0 {
            return Err(RuntimeError::new("tee() n must be non-negative"));
        }
        let slot_count =
            usize::try_from(n).map_err(|_| RuntimeError::new("tee() n is too large"))?;
        if slot_count == 0 {
            return Ok(self.heap.alloc_tuple(Vec::new()));
        }
        let source = self.to_iterator_value(iterable)?;
        let shared = match self
            .heap
            .alloc_module(ModuleObject::new("__itertools_tee_shared__"))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *shared.kind_mut() {
            module_data.globals.insert("source".to_string(), source);
            module_data
                .globals
                .insert("buffer".to_string(), self.heap.alloc_list(Vec::new()));
            module_data.globals.insert(
                "positions".to_string(),
                self.heap.alloc_list(vec![Value::Int(0); slot_count]),
            );
            module_data
                .globals
                .insert("exhausted".to_string(), Value::Bool(false));
        }
        let mut out = Vec::with_capacity(slot_count);
        for slot in 0..slot_count {
            out.push(self.heap.alloc_iterator(IteratorObject {
                kind: IteratorKind::Tee {
                    shared: Value::Module(shared.clone()),
                    slot,
                },
                index: 0,
            }));
        }
        Ok(self.heap.alloc_tuple(out))
    }

    pub(super) fn builtin_itertools_zip_longest(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let fillvalue = kwargs.remove("fillvalue").unwrap_or(Value::None);
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "zip_longest() got an unexpected keyword argument",
            ));
        }
        let mut iterators = Vec::with_capacity(args.len());
        for source in args {
            iterators.push(self.to_iterator_value(source)?);
        }
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::ZipLongest {
                active: vec![true; iterators.len()],
                iterators,
                fillvalue,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_chain(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "chain() does not accept keyword arguments",
            ));
        }
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Chain {
                sources: args,
                active: 0,
                current: None,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_chain_from_iterable(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "chain.from_iterable() expects one iterable argument",
            ));
        }
        let source_iterable = args.remove(0);
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::ChainFromIterable {
                source_iterable,
                source: None,
                current: None,
                source_exhausted: false,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_count(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new("count() expects at most start and step"));
        }
        let mut start = kwargs.remove("start");
        let mut step = kwargs.remove("step");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "count() got an unexpected keyword argument",
            ));
        }
        match args.len() {
            0 => {}
            1 => {
                if start.is_some() {
                    return Err(RuntimeError::new("count() got multiple values for start"));
                }
                start = Some(args.remove(0));
            }
            2 => {
                if start.is_some() || step.is_some() {
                    return Err(RuntimeError::new("count() got multiple values"));
                }
                start = Some(args.remove(0));
                step = Some(args.remove(0));
            }
            _ => unreachable!(),
        }
        let start = value_to_int(start.unwrap_or(Value::Int(0)))?;
        let step = value_to_int(step.unwrap_or(Value::Int(1)))?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Count {
                current: start,
                step,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_cycle(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("cycle() expects one iterable argument"));
        }
        let source = self.to_iterator_value(args[0].clone())?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Cycle {
                source,
                values: Vec::new(),
                source_exhausted: false,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_repeat(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "repeat() expects value and optional count",
            ));
        }
        let value = args.remove(0);
        let remaining = if args.is_empty() {
            None
        } else {
            let count = value_to_int(args.remove(0))?;
            if count <= 0 {
                Some(0)
            } else {
                Some(
                    usize::try_from(count)
                        .map_err(|_| RuntimeError::new("repeat() count is too large"))?,
                )
            }
        };
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Repeat { value, remaining },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_batched(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::type_error(format!(
                "batched() takes exactly 2 positional arguments ({} given)",
                args.len()
            )));
        }

        let mut iterable_arg = if args.is_empty() {
            None
        } else {
            Some(args.remove(0))
        };
        let mut n_arg = if args.is_empty() {
            None
        } else {
            Some(args.remove(0))
        };

        if let Some(value) = kwargs.remove("iterable") {
            if iterable_arg.is_some() {
                return Err(RuntimeError::type_error(
                    "argument for batched() given by name ('iterable') and position (1)",
                ));
            }
            iterable_arg = Some(value);
        }
        if let Some(value) = kwargs.remove("n") {
            if n_arg.is_some() {
                return Err(RuntimeError::type_error(
                    "argument for batched() given by name ('n') and position (2)",
                ));
            }
            n_arg = Some(value);
        }
        let strict = if let Some(value) = kwargs.remove("strict") {
            is_truthy(&value)
        } else {
            false
        };
        if !kwargs.is_empty() {
            let mut keys: Vec<String> = kwargs.keys().cloned().collect();
            keys.sort();
            let unexpected = keys.first().cloned().unwrap_or_default();
            return Err(RuntimeError::type_error(format!(
                "batched() got an unexpected keyword argument '{}'",
                unexpected
            )));
        }

        let iterable = iterable_arg.ok_or_else(|| {
            RuntimeError::type_error("batched() missing required argument 'iterable' (pos 1)")
        })?;
        let n = value_to_int(n_arg.ok_or_else(|| {
            RuntimeError::type_error("batched() missing required argument 'n' (pos 2)")
        })?)?;
        if n <= 0 {
            return Err(RuntimeError::value_error("n must be at least one"));
        }
        let size = usize::try_from(n).map_err(|_| RuntimeError::new("n is too large"))?;
        let iterator = self.to_iterator_value(iterable)?;
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Batched {
                iterator,
                size,
                strict,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_permutations(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "permutations() expects iterable and optional r",
            ));
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "permutations() got an unexpected keyword argument",
            ));
        }
        let values = self.collect_iterable_values(args.remove(0))?;
        let r = if let Some(r) = args.pop() {
            let r = value_to_int(r)?;
            if r < 0 {
                return Err(RuntimeError::new("r must be non-negative"));
            }
            usize::try_from(r).map_err(|_| RuntimeError::new("r is too large"))?
        } else {
            values.len()
        };
        let n = values.len();
        let done = r > n;
        let indices: Vec<usize> = (0..n).collect();
        let cycles: Vec<usize> = (0..r).map(|offset| n.saturating_sub(offset)).collect();
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Permutations {
                pool: values,
                r,
                indices,
                cycles,
                first: true,
                done,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_itertools_product(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let repeat = if let Some(value) = kwargs.remove("repeat") {
            value_to_int(value)?
        } else {
            1
        };
        if repeat < 0 {
            return Err(RuntimeError::new("repeat argument cannot be negative"));
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "product() got an unexpected keyword argument",
            ));
        }

        let mut base_pools: Vec<Vec<Value>> = Vec::new();
        for arg in args {
            base_pools.push(self.collect_iterable_values(arg)?);
        }
        let repeat =
            usize::try_from(repeat).map_err(|_| RuntimeError::new("repeat is too large"))?;
        let pools = if repeat == 0 {
            Vec::new()
        } else {
            let mut expanded = Vec::with_capacity(base_pools.len().saturating_mul(repeat));
            for _ in 0..repeat {
                expanded.extend(base_pools.iter().cloned());
            }
            expanded
        };
        let done = !pools.is_empty() && pools.iter().any(Vec::is_empty);
        let indices = vec![0; pools.len()];
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Product {
                pools,
                indices,
                first: true,
                done,
            },
            index: 0,
        }))
    }

    pub(super) fn builtin_functools_reduce(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new("reduce() expects 2-3 arguments"));
        }
        let callable = args[0].clone();
        let values = self.collect_iterable_values(args[1].clone())?;
        let mut iter = values.into_iter();
        let mut accumulator = if args.len() == 3 {
            args[2].clone()
        } else {
            iter.next().ok_or_else(|| {
                RuntimeError::new("reduce() of empty iterable with no initial value")
            })?
        };

        for item in iter {
            match self.call_internal(
                callable.clone(),
                vec![accumulator.clone(), item],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => accumulator = value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("reduce() callback raised"));
                }
            }
        }
        Ok(accumulator)
    }

    pub(super) fn builtin_functools_singledispatch(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("singledispatch() expects one callable"));
        }
        let target = args[0].clone();
        if !self.is_callable_value(&target) {
            return Err(RuntimeError::new("singledispatch() expects callable"));
        }
        if let Value::Function(func) = &target {
            self.store_attr_function(
                func,
                "register".to_string(),
                Value::Builtin(BuiltinFunction::FunctoolsSingleDispatchRegister),
            )?;
        }
        Ok(target)
    }

    pub(super) fn builtin_functools_singledispatch_register(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("register() expects 1-2 arguments"));
        }
        if args.len() == 1 {
            let wrapper = match self.heap.alloc_module(ModuleObject::new(
                "__functools_singledispatch_register__".to_string(),
            )) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                module_data.globals.insert(
                    "__pyrs_singledispatch_register__".to_string(),
                    Value::Bool(true),
                );
                module_data
                    .globals
                    .insert("registered_type".to_string(), args[0].clone());
            }
            return Ok(self.alloc_builtin_bound_method(
                BuiltinFunction::FunctoolsSingleDispatchRegisterDecorator,
                wrapper,
            ));
        }
        Ok(args[1].clone())
    }

    pub(super) fn builtin_functools_singledispatch_register_decorator(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "register() decorator expects one callable",
            ));
        }
        let _receiver = args.remove(0);
        Ok(args.remove(0))
    }

    pub(super) fn builtin_enum_convert(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 3 || args.len() > 4 {
            return Err(RuntimeError::new(
                "_convert_() expects class (optional), name, module, and filter callable",
            ));
        }

        let (enum_base, class_name_value, module_name_value, predicate) = if args.len() == 4 {
            (
                Some(args[0].clone()),
                args[1].clone(),
                args[2].clone(),
                args[3].clone(),
            )
        } else {
            (None, args[0].clone(), args[1].clone(), args[2].clone())
        };

        let class_name = match class_name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("_convert_() class name must be string")),
        };
        let module_name = match module_name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("_convert_() module name must be string")),
        };
        if !self.is_callable_value(&predicate) {
            return Err(RuntimeError::new("_convert_() filter must be callable"));
        }

        let module = self.import_module_object(&module_name)?;
        let mut candidates = match &*module.kind() {
            Object::Module(module_data) => module_data
                .globals
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect::<Vec<_>>(),
            _ => return Err(RuntimeError::new("_convert_() target module is invalid")),
        };
        candidates.sort_by(|left, right| left.0.cmp(&right.0));

        let mut members = Vec::new();
        for (name, value) in candidates {
            let matches = match self.call_internal(
                predicate.clone(),
                vec![Value::Str(name.clone())],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => is_truthy(&value),
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("_convert_() filter callback raised"));
                }
            };
            if !matches {
                continue;
            }
            members.push((name, value));
        }

        let enum_base = if let Some(base) = enum_base {
            match base {
                Value::Class(class) => class,
                _ => return Err(RuntimeError::new("_convert_() class must be a type")),
            }
        } else {
            let preferred = if class_name.contains("Flag") {
                "IntFlag"
            } else {
                "IntEnum"
            };
            let enum_module = self
                .modules
                .get("enum")
                .cloned()
                .ok_or_else(|| RuntimeError::new("_convert_() enum module unavailable"))?;
            let fallback = "Enum";
            let pick = match &*enum_module.kind() {
                Object::Module(module_data) => module_data
                    .globals
                    .get(preferred)
                    .cloned()
                    .or_else(|| module_data.globals.get(fallback).cloned()),
                _ => None,
            };
            match pick {
                Some(Value::Class(class)) => class,
                _ => {
                    return Err(RuntimeError::new(
                        "_convert_() enum base class lookup failed",
                    ));
                }
            }
        };

        let enum_metaclass = match &*enum_base.kind() {
            Object::Class(class_data) => class_data.metaclass.clone(),
            _ => None,
        };
        let class_value = self.build_default_class_value(
            class_name.clone(),
            HashMap::new(),
            vec![enum_base],
            enum_metaclass,
            None,
        )?;
        let class_ref = match &class_value {
            Value::Class(class) => class.clone(),
            _ => unreachable!(),
        };

        let members_dict = match self.heap.alloc_dict(Vec::new()) {
            Value::Dict(dict) => dict,
            _ => unreachable!(),
        };

        if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str(module_name.clone()));
            class_data
                .attrs
                .insert("__qualname__".to_string(), Value::Str(class_name.clone()));
            for (name, value) in members {
                class_data.attrs.insert(name.clone(), value.clone());
                dict_set_value_checked(&members_dict, Value::Str(name), value)?;
            }
            class_data
                .attrs
                .insert("__members__".to_string(), Value::Dict(members_dict));
        }

        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert(class_name.clone(), class_value.clone());
        }
        Ok(class_value)
    }

    pub(super) fn builtin_functools_wraps(
        &mut self,
        mut args: Vec<Value>,
        _kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("wraps() expects at least one argument"));
        }
        let wrapped = args.remove(0);
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__functools_wraps__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data.globals.insert("wrapped".to_string(), wrapped);
        }
        Ok(self.alloc_native_bound_method(NativeMethodKind::FunctoolsWrapsDecorator, receiver))
    }

    pub(super) fn maybe_get_attribute(
        &mut self,
        target: Value,
        name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        match self.builtin_getattr(vec![target, Value::Str(name.to_string())], HashMap::new()) {
            Ok(value) => Ok(Some(value)),
            Err(err) if is_missing_attribute_error(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(super) fn apply_functools_wraps_metadata(
        &mut self,
        wrapper: &Value,
        wrapped: &Value,
    ) -> Result<(), RuntimeError> {
        let metadata_source = if let Value::BoundMethod(bound) = wrapped {
            let bound_kind = bound.kind();
            match &*bound_kind {
                Object::BoundMethod(bound_data) => {
                    let function_kind = bound_data.function.kind();
                    if matches!(&*function_kind, Object::Function(_)) {
                        Value::Function(bound_data.function.clone())
                    } else {
                        wrapped.clone()
                    }
                }
                _ => wrapped.clone(),
            }
        } else {
            wrapped.clone()
        };

        for attr in [
            "__module__",
            "__name__",
            "__qualname__",
            "__doc__",
            "__annotations__",
        ] {
            if let Some(value) = self.maybe_get_attribute(metadata_source.clone(), attr)? {
                self.builtin_setattr(
                    vec![wrapper.clone(), Value::Str(attr.to_string()), value],
                    HashMap::new(),
                )?;
            }
        }

        let wrapped_dict = self.maybe_get_attribute(metadata_source, "__dict__")?;
        if let Some(Value::Dict(source_dict)) = wrapped_dict {
            let wrapper_dict = self.builtin_getattr(
                vec![wrapper.clone(), Value::Str("__dict__".to_string())],
                HashMap::new(),
            )?;
            if let Value::Dict(target_dict) = wrapper_dict {
                let entries = {
                    let source_kind = source_dict.kind();
                    match &*source_kind {
                        Object::Dict(entries) => entries.to_vec(),
                        _ => Vec::new(),
                    }
                };
                for (key, value) in entries {
                    dict_set_value_checked(&target_dict, key, value)?;
                }
            }
        }

        self.builtin_setattr(
            vec![
                wrapper.clone(),
                Value::Str("__wrapped__".to_string()),
                wrapped.clone(),
            ],
            HashMap::new(),
        )?;
        Ok(())
    }

    pub(super) fn builtin_functools_partial(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("partial() expects at least one argument"));
        }
        let mut callable = args.remove(0);
        if let Some(unwrapped) = self.unwrap_staticmethod_attr(&callable) {
            callable = unwrapped;
        }
        if let Some(unwrapped) = self.unwrap_classmethod_attr(&callable) {
            callable = unwrapped;
        }
        if !self.is_callable_value(&callable) {
            return Err(RuntimeError::new("first argument must be callable"));
        }
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__functools_partial__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        let frozen_kwargs = kwargs
            .into_iter()
            .map(|(name, value)| (Value::Str(name), value))
            .collect::<Vec<_>>();
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data.globals.insert("callable".to_string(), callable);
            module_data
                .globals
                .insert("args".to_string(), self.heap.alloc_list(args));
            module_data
                .globals
                .insert("kwargs".to_string(), self.heap.alloc_dict(frozen_kwargs));
        }
        Ok(self.alloc_native_bound_method(NativeMethodKind::FunctoolsPartialCall, receiver))
    }

    pub(super) fn builtin_functools_cmp_to_key(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("cmp_to_key() expects one callable"));
        }
        let comparator = args[0].clone();
        if !self.is_callable_value(&comparator) {
            return Err(RuntimeError::new("cmp_to_key() expects callable"));
        }
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__functools_cmp_to_key__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data.globals.insert("cmp".to_string(), comparator);
        }
        Ok(self.alloc_native_bound_method(NativeMethodKind::FunctoolsCmpToKeyCall, receiver))
    }

    pub(super) fn builtin_functools_cached_property(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("cached_property() expects one callable"));
        }
        let func = args[0].clone();
        if !self.is_callable_value(&func) {
            return Err(RuntimeError::new("cached_property() expects callable"));
        }
        let doc = self
            .builtin_getattr(
                vec![func.clone(), Value::Str("__doc__".to_string())],
                HashMap::new(),
            )
            .unwrap_or(Value::None);
        let module = self
            .builtin_getattr(
                vec![func.clone(), Value::Str("__module__".to_string())],
                HashMap::new(),
            )
            .unwrap_or(Value::None);
        let class = match self
            .heap
            .alloc_class(ClassObject::new("cached_property".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        let mut instance = InstanceObject::new(class);
        instance
            .attrs
            .insert("__pyrs_cached_property__".to_string(), Value::Bool(true));
        instance.attrs.insert("func".to_string(), func);
        instance.attrs.insert("attrname".to_string(), Value::None);
        instance.attrs.insert("__doc__".to_string(), doc);
        instance.attrs.insert("__module__".to_string(), module);
        Ok(self.heap.alloc_instance(instance))
    }

    fn functools_notimplemented_value(&self) -> Value {
        self.builtins
            .get("NotImplemented")
            .cloned()
            .unwrap_or(Value::None)
    }

    fn functools_is_notimplemented_value(&self, value: &Value) -> bool {
        let marker = self.functools_notimplemented_value();
        match (value, &marker) {
            (Value::Instance(left), Value::Instance(right)) => left.id() == right.id(),
            _ => value == &marker,
        }
    }

    fn functools_call_ordering_root_method(
        &mut self,
        receiver: Value,
        other: Value,
        root_name: &str,
    ) -> Result<Value, RuntimeError> {
        let Some(class_ref) = self.class_of_value(&receiver) else {
            return Err(RuntimeError::type_error(
                "total_ordering synthetic method requires instance receiver",
            ));
        };
        let Some(root_method) = class_attr_lookup(&class_ref, root_name) else {
            return Err(RuntimeError::attribute_error(format!(
                "type has no attribute '{root_name}'"
            )));
        };
        let callable = match self.bind_descriptor_method(root_method.clone(), &receiver)? {
            Some(bound) => bound,
            None => root_method,
        };
        match self.call_internal(callable, vec![other], HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => Err(self
                .runtime_error_from_active_exception(
                    "functools.total_ordering synthetic compare failed",
                )),
        }
    }

    fn functools_total_ordering_from_root(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        root_name: &str,
        pattern: u8,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "total_ordering synthetic comparator expects self and other",
            ));
        }
        let receiver = args.remove(0);
        let other = args.remove(0);
        let root_result =
            self.functools_call_ordering_root_method(receiver.clone(), other.clone(), root_name)?;
        if self.functools_is_notimplemented_value(&root_result) {
            return Ok(self.functools_notimplemented_value());
        }
        let root_truth = self.truthy_from_value(&root_result)?;
        let equals_value = self.compare_eq_runtime(receiver, other)?;
        let equals = self.truthy_from_value(&equals_value)?;
        let result = match pattern {
            // !root and receiver != other
            0 => !root_truth && !equals,
            // root or receiver == other
            1 => root_truth || equals,
            // !root
            2 => !root_truth,
            // !root or receiver == other
            3 => !root_truth || equals,
            // root and receiver != other
            4 => root_truth && !equals,
            _ => unreachable!(),
        };
        Ok(Value::Bool(result))
    }

    pub(super) fn builtin_functools_total_ordering(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "total_ordering() expects one class argument",
            ));
        }
        let class = match args.remove(0) {
            Value::Class(class) => class,
            _ => return Err(RuntimeError::type_error("total_ordering() expects a class")),
        };
        let root = {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::type_error("total_ordering() expects a class"));
            };
            ["__lt__", "__le__", "__gt__", "__ge__"]
                .into_iter()
                .find(|name| class_data.attrs.contains_key(*name))
                .map(str::to_string)
                .ok_or_else(|| {
                    RuntimeError::value_error(
                        "must define at least one ordering operation: < > <= >=",
                    )
                })?
        };

        if let Object::Class(class_data) = &mut *class.kind_mut() {
            let add_missing = |attrs: &mut HashMap<String, Value>,
                               method_name: &str,
                               builtin: BuiltinFunction| {
                if !attrs.contains_key(method_name) {
                    attrs.insert(method_name.to_string(), Value::Builtin(builtin));
                }
            };
            match root.as_str() {
                "__lt__" => {
                    add_missing(
                        &mut class_data.attrs,
                        "__gt__",
                        BuiltinFunction::FunctoolsTotalOrderingGtFromLt,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__le__",
                        BuiltinFunction::FunctoolsTotalOrderingLeFromLt,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__ge__",
                        BuiltinFunction::FunctoolsTotalOrderingGeFromLt,
                    );
                }
                "__le__" => {
                    add_missing(
                        &mut class_data.attrs,
                        "__ge__",
                        BuiltinFunction::FunctoolsTotalOrderingGeFromLe,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__lt__",
                        BuiltinFunction::FunctoolsTotalOrderingLtFromLe,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__gt__",
                        BuiltinFunction::FunctoolsTotalOrderingGtFromLe,
                    );
                }
                "__gt__" => {
                    add_missing(
                        &mut class_data.attrs,
                        "__lt__",
                        BuiltinFunction::FunctoolsTotalOrderingLtFromGt,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__ge__",
                        BuiltinFunction::FunctoolsTotalOrderingGeFromGt,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__le__",
                        BuiltinFunction::FunctoolsTotalOrderingLeFromGt,
                    );
                }
                "__ge__" => {
                    add_missing(
                        &mut class_data.attrs,
                        "__le__",
                        BuiltinFunction::FunctoolsTotalOrderingLeFromGe,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__gt__",
                        BuiltinFunction::FunctoolsTotalOrderingGtFromGe,
                    );
                    add_missing(
                        &mut class_data.attrs,
                        "__lt__",
                        BuiltinFunction::FunctoolsTotalOrderingLtFromGe,
                    );
                }
                _ => unreachable!(),
            }
        }
        Ok(Value::Class(class))
    }

    pub(super) fn builtin_functools_total_ordering_gt_from_lt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__lt__", 0)
    }

    pub(super) fn builtin_functools_total_ordering_le_from_lt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__lt__", 1)
    }

    pub(super) fn builtin_functools_total_ordering_ge_from_lt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__lt__", 2)
    }

    pub(super) fn builtin_functools_total_ordering_ge_from_le(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__le__", 3)
    }

    pub(super) fn builtin_functools_total_ordering_lt_from_le(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__le__", 4)
    }

    pub(super) fn builtin_functools_total_ordering_gt_from_le(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__le__", 2)
    }

    pub(super) fn builtin_functools_total_ordering_lt_from_gt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__gt__", 0)
    }

    pub(super) fn builtin_functools_total_ordering_ge_from_gt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__gt__", 1)
    }

    pub(super) fn builtin_functools_total_ordering_le_from_gt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__gt__", 2)
    }

    pub(super) fn builtin_functools_total_ordering_le_from_ge(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__ge__", 3)
    }

    pub(super) fn builtin_functools_total_ordering_gt_from_ge(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__ge__", 4)
    }

    pub(super) fn builtin_functools_total_ordering_lt_from_ge(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.functools_total_ordering_from_root(args, kwargs, "__ge__", 2)
    }

    fn weakset_backing_set(&mut self, receiver: &Value) -> Result<ObjRef, RuntimeError> {
        let instance = match receiver {
            Value::Instance(obj) => obj.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "WeakSet method receiver must be WeakSet",
                ));
            }
        };
        let mut instance_kind = instance.kind_mut();
        let Object::Instance(instance_data) = &mut *instance_kind else {
            return Err(RuntimeError::type_error(
                "WeakSet method receiver must be WeakSet",
            ));
        };
        if !matches!(instance_data.attrs.get("_data"), Some(Value::Set(_))) {
            instance_data
                .attrs
                .insert("_data".to_string(), self.heap.alloc_set(Vec::new()));
        }
        match instance_data.attrs.get("_data").cloned() {
            Some(Value::Set(set_obj)) => Ok(set_obj),
            _ => Err(RuntimeError::new("WeakSet backing storage is invalid")),
        }
    }

    pub(super) fn builtin_weakset_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::type_error(
                "WeakSet.__init__() takes at most 1 argument",
            ));
        }
        let receiver = args.remove(0);
        let backing = self.weakset_backing_set(&receiver)?;
        if let Object::Set(values) = &mut *backing.kind_mut() {
            values.clear();
        }
        if let Some(iterable) = args.pop() {
            let values = self.collect_iterable_values(iterable)?;
            for value in values {
                self.set_insert_checked_runtime(&backing, value)?;
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_weakset_len(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "WeakSet.__len__() takes no arguments",
            ));
        }
        let backing = self.weakset_backing_set(&args[0])?;
        let Object::Set(values) = &*backing.kind() else {
            return Err(RuntimeError::new("WeakSet backing storage is invalid"));
        };
        Ok(Value::Int(values.len() as i64))
    }

    pub(super) fn builtin_weakset_contains(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "WeakSet.__contains__() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let target = args.remove(0);
        let backing = self.weakset_backing_set(&receiver)?;
        Ok(Value::Bool(self.set_contains_runtime(&backing, &target)?))
    }

    pub(super) fn builtin_weakset_iter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "WeakSet.__iter__() takes no arguments",
            ));
        }
        let backing = self.weakset_backing_set(&args[0])?;
        let Object::Set(values) = &*backing.kind() else {
            return Err(RuntimeError::new("WeakSet backing storage is invalid"));
        };
        self.to_iterator_value(self.heap.alloc_list(values.to_vec()))
    }

    pub(super) fn builtin_weakset_add(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "WeakSet.add() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let value = args.remove(0);
        let backing = self.weakset_backing_set(&receiver)?;
        self.set_insert_checked_runtime(&backing, value)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_weakset_discard(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "WeakSet.discard() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let value = args.remove(0);
        let backing = self.weakset_backing_set(&receiver)?;
        let _ = self.set_remove_checked_runtime(&backing, &value)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_weakset_remove(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "WeakSet.remove() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let value = args.remove(0);
        let backing = self.weakset_backing_set(&receiver)?;
        if self.set_remove_checked_runtime(&backing, &value)? {
            Ok(Value::None)
        } else {
            Err(RuntimeError::key_error("key not found"))
        }
    }

    pub(super) fn builtin_weakset_clear(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "WeakSet.clear() takes no arguments",
            ));
        }
        let backing = self.weakset_backing_set(&args[0])?;
        if let Object::Set(values) = &mut *backing.kind_mut() {
            values.clear();
            return Ok(Value::None);
        }
        Err(RuntimeError::new("WeakSet backing storage is invalid"))
    }

    pub(super) fn builtin_weakset_update(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "WeakSet.update() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let iterable = args.remove(0);
        let backing = self.weakset_backing_set(&receiver)?;
        for value in self.collect_iterable_values(iterable)? {
            self.set_insert_checked_runtime(&backing, value)?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_weakset_copy(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "WeakSet.copy() takes no arguments",
            ));
        }
        let receiver = match &args[0] {
            Value::Instance(obj) => obj.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "WeakSet.copy() receiver must be WeakSet",
                ));
            }
        };
        let backing = self.weakset_backing_set(&args[0])?;
        let values = match &*backing.kind() {
            Object::Set(values) => values.clone(),
            _ => return Err(RuntimeError::new("WeakSet backing storage is invalid")),
        };
        let class = {
            let receiver_kind = receiver.kind();
            let Object::Instance(instance_data) = &*receiver_kind else {
                return Err(RuntimeError::type_error(
                    "WeakSet.copy() receiver must be WeakSet",
                ));
            };
            instance_data.class.clone()
        };
        let copy = self.heap.alloc_instance(InstanceObject::new(class));
        if let Value::Instance(copy_obj) = &copy
            && let Object::Instance(copy_data) = &mut *copy_obj.kind_mut()
        {
            copy_data
                .attrs
                .insert("_data".to_string(), self.heap.alloc_set(values.to_vec()));
        }
        Ok(copy)
    }

    fn weakdict_backing_dict(&mut self, receiver: &Value) -> Result<ObjRef, RuntimeError> {
        let instance = match receiver {
            Value::Instance(obj) => obj.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "weak dictionary method receiver must be weak dictionary",
                ));
            }
        };
        let mut instance_kind = instance.kind_mut();
        let Object::Instance(instance_data) = &mut *instance_kind else {
            return Err(RuntimeError::type_error(
                "weak dictionary method receiver must be weak dictionary",
            ));
        };
        let class_is_weakdict = match &*instance_data.class.kind() {
            Object::Class(class_data) => matches!(
                class_data.attrs.get("__pyrs_weakdict__"),
                Some(Value::Bool(true))
            ),
            _ => false,
        };
        if !class_is_weakdict {
            return Err(RuntimeError::type_error(
                "weak dictionary method receiver must be weak dictionary",
            ));
        }
        if !matches!(instance_data.attrs.get("_data"), Some(Value::Dict(_))) {
            instance_data
                .attrs
                .insert("_data".to_string(), self.heap.alloc_dict(Vec::new()));
        }
        match instance_data.attrs.get("_data").cloned() {
            Some(Value::Dict(dict_obj)) => Ok(dict_obj),
            _ => Err(RuntimeError::new(
                "weak dictionary backing storage is invalid",
            )),
        }
    }

    fn weakdict_entries_snapshot(
        &self,
        backing: &ObjRef,
    ) -> Result<Vec<(Value, Value)>, RuntimeError> {
        match &*backing.kind() {
            Object::Dict(entries) => Ok(entries
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()),
            _ => Err(RuntimeError::new(
                "weak dictionary backing storage is invalid",
            )),
        }
    }

    fn weakdict_update_from_source(
        &mut self,
        backing: &ObjRef,
        source: Value,
    ) -> Result<(), RuntimeError> {
        let dict_source = match &source {
            Value::Dict(dict_obj) => Some(dict_obj.clone()),
            Value::Instance(instance) => self.instance_backing_dict(instance),
            _ => None,
        };
        if let Some(dict_source) = dict_source {
            for (key, value) in self.weakdict_entries_snapshot(&dict_source)? {
                self.dict_set_value_checked_runtime(backing, key, value)?;
            }
            return Ok(());
        }

        for item in self.collect_iterable_values(source)? {
            let parts = self.collect_iterable_values(item)?;
            if parts.len() != 2 {
                return Err(RuntimeError::type_error(
                    "weak dictionary update sequence element has length != 2",
                ));
            }
            self.dict_set_value_checked_runtime(backing, parts[0].clone(), parts[1].clone())?;
        }
        Ok(())
    }

    pub(super) fn builtin_weakdict_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::type_error(
                "weak dictionary __init__() takes at most 1 argument",
            ));
        }
        let receiver = args.remove(0);
        let backing = self.weakdict_backing_dict(&receiver)?;
        if let Object::Dict(entries) = &mut *backing.kind_mut() {
            entries.clear();
        }
        if let Some(source) = args.pop() {
            self.weakdict_update_from_source(&backing, source)?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_weakdict_len(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "weak dictionary __len__() takes no arguments",
            ));
        }
        let backing = self.weakdict_backing_dict(&args[0])?;
        let Object::Dict(entries) = &*backing.kind() else {
            return Err(RuntimeError::new(
                "weak dictionary backing storage is invalid",
            ));
        };
        Ok(Value::Int(entries.len() as i64))
    }

    pub(super) fn builtin_weakdict_contains(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "weak dictionary __contains__() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let backing = self.weakdict_backing_dict(&receiver)?;
        Ok(Value::Bool(
            self.dict_contains_key_checked_runtime(&backing, &key)?,
        ))
    }

    pub(super) fn builtin_weakdict_iter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "weak dictionary __iter__() takes no arguments",
            ));
        }
        let backing = self.weakdict_backing_dict(&args[0])?;
        let keys = self
            .weakdict_entries_snapshot(&backing)?
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        self.to_iterator_value(self.heap.alloc_list(keys))
    }

    pub(super) fn builtin_weakdict_getitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "weak dictionary __getitem__() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let backing = self.weakdict_backing_dict(&receiver)?;
        if let Some(value) = self.dict_get_value_runtime(&backing, &key)? {
            return Ok(value);
        }
        Err(RuntimeError::key_error("key not found"))
    }

    pub(super) fn builtin_weakdict_setitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(RuntimeError::type_error(
                "weak dictionary __setitem__() takes exactly two arguments",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let value = args.remove(0);
        let backing = self.weakdict_backing_dict(&receiver)?;
        self.dict_set_value_checked_runtime(&backing, key, value)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_weakdict_delitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "weak dictionary __delitem__() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let backing = self.weakdict_backing_dict(&receiver)?;
        if self.dict_remove_value_runtime(&backing, &key)?.is_none() {
            return Err(RuntimeError::key_error("key not found"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_weakdict_clear(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "weak dictionary clear() takes no arguments",
            ));
        }
        let backing = self.weakdict_backing_dict(&args[0])?;
        if let Object::Dict(entries) = &mut *backing.kind_mut() {
            entries.clear();
            return Ok(Value::None);
        }
        Err(RuntimeError::new(
            "weak dictionary backing storage is invalid",
        ))
    }

    pub(super) fn builtin_weakdict_get(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::type_error(
                "weak dictionary get() takes one or two arguments",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let default = args.pop().unwrap_or(Value::None);
        let backing = self.weakdict_backing_dict(&receiver)?;
        Ok(self
            .dict_get_value_runtime(&backing, &key)?
            .unwrap_or(default))
    }

    pub(super) fn builtin_weakdict_pop(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::type_error(
                "weak dictionary pop() takes one or two arguments",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let default = args.pop();
        let backing = self.weakdict_backing_dict(&receiver)?;
        if let Some(value) = self.dict_remove_value_runtime(&backing, &key)? {
            return Ok(value);
        }
        if let Some(default) = default {
            return Ok(default);
        }
        Err(RuntimeError::key_error("key not found"))
    }

    pub(super) fn builtin_weakdict_popitem(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "weak dictionary popitem() takes no arguments",
            ));
        }
        let backing = self.weakdict_backing_dict(&args[0])?;
        let pair = match &mut *backing.kind_mut() {
            Object::Dict(entries) if entries.is_empty() => None,
            Object::Dict(entries) => Some(entries.remove(entries.len() - 1)),
            _ => None,
        };
        let Some((key, value)) = pair else {
            return Err(RuntimeError::key_error("popitem(): dictionary is empty"));
        };
        Ok(self.heap.alloc_tuple(vec![key, value]))
    }

    pub(super) fn builtin_weakdict_update(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "weak dictionary update() takes exactly one argument",
            ));
        }
        let receiver = args.remove(0);
        let source = args.remove(0);
        let backing = self.weakdict_backing_dict(&receiver)?;
        self.weakdict_update_from_source(&backing, source)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_weakdict_copy(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "weak dictionary copy() takes no arguments",
            ));
        }
        let receiver = match &args[0] {
            Value::Instance(obj) => obj.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "weak dictionary method receiver must be weak dictionary",
                ));
            }
        };
        let backing = self.weakdict_backing_dict(&args[0])?;
        let entries = self.weakdict_entries_snapshot(&backing)?;
        let class = {
            let receiver_kind = receiver.kind();
            let Object::Instance(instance_data) = &*receiver_kind else {
                return Err(RuntimeError::type_error(
                    "weak dictionary method receiver must be weak dictionary",
                ));
            };
            instance_data.class.clone()
        };
        let copy = self.heap.alloc_instance(InstanceObject::new(class));
        if let Value::Instance(copy_obj) = &copy
            && let Object::Instance(copy_data) = &mut *copy_obj.kind_mut()
        {
            copy_data
                .attrs
                .insert("_data".to_string(), self.heap.alloc_dict(entries));
        }
        Ok(copy)
    }

    pub(super) fn builtin_collections_counter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("Counter() expects at most one argument"));
        }
        let mut entries: Vec<(Value, Value)> = Vec::new();
        if let Some(source) = args.into_iter().next() {
            for item in self.collect_iterable_values(source)? {
                ensure_hashable(&item)?;
                if let Some((_, count)) = entries.iter_mut().find(|(key, _)| *key == item) {
                    *count = add_values(count.clone(), Value::Int(1), &self.heap)?;
                } else {
                    entries.push((item, Value::Int(1)));
                }
            }
        }
        Ok(self.heap.alloc_dict(entries))
    }

    pub(super) fn builtin_collections_deque(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let collections = self
            .modules
            .get("collections")
            .cloned()
            .ok_or_else(|| RuntimeError::new("collections module is not loaded"))?;
        let Object::Module(module_data) = &*collections.kind() else {
            return Err(RuntimeError::new("collections module is not loaded"));
        };
        let deque_class = module_data
            .globals
            .get("deque")
            .cloned()
            .ok_or_else(|| RuntimeError::new("collections.deque is unavailable"))?;
        self.call_internal(deque_class, args, kwargs)
            .and_then(|outcome| match outcome {
                InternalCallOutcome::Value(value) => Ok(value),
                InternalCallOutcome::CallerExceptionHandled => {
                    Err(self.runtime_error_from_active_exception("deque() raised an exception"))
                }
            })
    }

    fn dequeue_storage_list(
        &self,
        instance: &ObjRef,
        method_name: &str,
    ) -> Result<ObjRef, RuntimeError> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new(format!(
                "{method_name}() expected deque instance"
            )));
        };
        match instance_data.attrs.get(DEQUE_BACKING_STORAGE_ATTR) {
            Some(Value::List(list)) => Ok(list.clone()),
            _ => Err(RuntimeError::new(format!(
                "{method_name}() expected deque instance"
            ))),
        }
    }

    fn dequeue_maxlen(&self, instance: &ObjRef) -> Result<Option<usize>, RuntimeError> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return Err(RuntimeError::new("deque instance expected"));
        };
        match instance_data.attrs.get("__pyrs_deque_maxlen__") {
            None | Some(Value::None) => Ok(None),
            Some(value) => {
                let maxlen = value_to_int(value.clone())?;
                if maxlen < 0 {
                    return Err(RuntimeError::new("maxlen must be non-negative"));
                }
                Ok(Some(maxlen as usize))
            }
        }
    }

    fn dequeue_apply_maxlen_right(
        values: &mut Vec<Value>,
        maxlen: Option<usize>,
    ) -> Result<(), RuntimeError> {
        if let Some(limit) = maxlen {
            while values.len() > limit {
                if values.is_empty() {
                    return Err(RuntimeError::new("deque underflow"));
                }
                values.remove(0);
            }
        }
        Ok(())
    }

    fn dequeue_apply_maxlen_left(
        values: &mut Vec<Value>,
        maxlen: Option<usize>,
    ) -> Result<(), RuntimeError> {
        if let Some(limit) = maxlen {
            while values.len() > limit {
                if values.pop().is_none() {
                    return Err(RuntimeError::new("deque underflow"));
                }
            }
        }
        Ok(())
    }

    pub(super) fn builtin_collections_deque_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "deque.__init__")?;
        let mut iterable = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut maxlen = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "deque.__init__() takes at most 2 arguments",
            ));
        }
        if let Some(value) = kwargs.remove("iterable") {
            if iterable.is_some() {
                return Err(RuntimeError::new(
                    "deque.__init__() got multiple values for argument 'iterable'",
                ));
            }
            iterable = Some(value);
        }
        if let Some(value) = kwargs.remove("maxlen") {
            if maxlen.is_some() {
                return Err(RuntimeError::new(
                    "deque.__init__() got multiple values for argument 'maxlen'",
                ));
            }
            maxlen = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "deque.__init__() got an unexpected keyword argument",
            ));
        }

        let mut values = if let Some(source) = iterable {
            self.collect_iterable_values(source)?
        } else {
            Vec::new()
        };
        let maxlen = match maxlen {
            None | Some(Value::None) => None,
            Some(value) => {
                let maxlen = value_to_int(value)?;
                if maxlen < 0 {
                    return Err(RuntimeError::new("maxlen must be non-negative"));
                }
                Some(maxlen as usize)
            }
        };
        Self::dequeue_apply_maxlen_right(&mut values, maxlen)?;

        let storage = self.heap.alloc_list(values);
        let Object::Instance(instance_data) = &mut *instance.kind_mut() else {
            return Err(RuntimeError::new(
                "deque.__init__() expected deque instance",
            ));
        };
        instance_data
            .attrs
            .insert(DEQUE_BACKING_STORAGE_ATTR.to_string(), storage);
        instance_data.attrs.insert(
            "__pyrs_deque_maxlen__".to_string(),
            maxlen
                .map(|limit| Value::Int(limit as i64))
                .unwrap_or(Value::None),
        );
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_deque_append(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("deque.append() expects one argument"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.append")?;
        let item = args.remove(0);
        let maxlen = self.dequeue_maxlen(&instance)?;
        let storage = self.dequeue_storage_list(&instance, "deque.append")?;
        let Object::List(values) = &mut *storage.kind_mut() else {
            return Err(RuntimeError::new("deque.append() expected deque instance"));
        };
        values.push(item);
        Self::dequeue_apply_maxlen_right(values, maxlen)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_deque_appendleft(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("deque.appendleft() expects one argument"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.appendleft")?;
        let item = args.remove(0);
        let maxlen = self.dequeue_maxlen(&instance)?;
        let storage = self.dequeue_storage_list(&instance, "deque.appendleft")?;
        let Object::List(values) = &mut *storage.kind_mut() else {
            return Err(RuntimeError::new(
                "deque.appendleft() expected deque instance",
            ));
        };
        values.insert(0, item);
        Self::dequeue_apply_maxlen_left(values, maxlen)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_deque_pop(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("deque.pop() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.pop")?;
        let storage = self.dequeue_storage_list(&instance, "deque.pop")?;
        let Object::List(values) = &mut *storage.kind_mut() else {
            return Err(RuntimeError::new("deque.pop() expected deque instance"));
        };
        values
            .pop()
            .ok_or_else(|| RuntimeError::index_error("pop from an empty deque"))
    }

    pub(super) fn builtin_collections_deque_popleft(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("deque.popleft() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.popleft")?;
        let storage = self.dequeue_storage_list(&instance, "deque.popleft")?;
        let Object::List(values) = &mut *storage.kind_mut() else {
            return Err(RuntimeError::new("deque.popleft() expected deque instance"));
        };
        if values.is_empty() {
            return Err(RuntimeError::index_error("pop from an empty deque"));
        }
        Ok(values.remove(0))
    }

    pub(super) fn builtin_collections_deque_clear(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("deque.clear() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.clear")?;
        let storage = self.dequeue_storage_list(&instance, "deque.clear")?;
        let Object::List(values) = &mut *storage.kind_mut() else {
            return Err(RuntimeError::new("deque.clear() expected deque instance"));
        };
        values.clear();
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_deque_extend(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("deque.extend() expects one argument"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.extend")?;
        let source = args.remove(0);
        let maxlen = self.dequeue_maxlen(&instance)?;
        let storage = self.dequeue_storage_list(&instance, "deque.extend")?;
        let values_to_add = self.collect_iterable_values(source)?;
        let Object::List(values) = &mut *storage.kind_mut() else {
            return Err(RuntimeError::new("deque.extend() expected deque instance"));
        };
        for value in values_to_add {
            values.push(value);
            Self::dequeue_apply_maxlen_right(values, maxlen)?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_deque_extendleft(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("deque.extendleft() expects one argument"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.extendleft")?;
        let source = args.remove(0);
        let maxlen = self.dequeue_maxlen(&instance)?;
        let storage = self.dequeue_storage_list(&instance, "deque.extendleft")?;
        let values_to_add = self.collect_iterable_values(source)?;
        let Object::List(values) = &mut *storage.kind_mut() else {
            return Err(RuntimeError::new(
                "deque.extendleft() expected deque instance",
            ));
        };
        for value in values_to_add {
            values.insert(0, value);
            Self::dequeue_apply_maxlen_left(values, maxlen)?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_deque_len(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("deque.__len__() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.__len__")?;
        let storage = self.dequeue_storage_list(&instance, "deque.__len__")?;
        let Object::List(values) = &*storage.kind() else {
            return Err(RuntimeError::new("deque.__len__() expected deque instance"));
        };
        Ok(Value::Int(values.len() as i64))
    }

    pub(super) fn builtin_collections_deque_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("deque.__iter__() takes no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "deque.__iter__")?;
        let storage = self.dequeue_storage_list(&instance, "deque.__iter__")?;
        let snapshot = match &*storage.kind() {
            Object::List(values) => values.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "deque.__iter__() expected deque instance",
                ));
            }
        };
        self.to_iterator_value(self.heap.alloc_list(snapshot))
    }

    pub(super) fn builtin_collections_chainmap_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "ChainMap.__init__() does not accept keyword arguments",
            ));
        }
        if args.is_empty() {
            return Err(RuntimeError::new("ChainMap.__init__() missing self"));
        }
        let receiver = args.remove(0);
        let instance = match receiver {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.__init__() expected a ChainMap instance",
                ));
            }
        };
        let maps = if args.is_empty() {
            vec![self.heap.alloc_dict(Vec::new())]
        } else {
            args
        };
        let maps_value = self.heap.alloc_list(maps);
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data.attrs.insert("maps".to_string(), maps_value);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_chainmap_new_child(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "ChainMap.new_child() expects self and optional map argument",
            ));
        }
        let receiver = args.remove(0);
        let instance = match receiver {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.new_child() expected a ChainMap instance",
                ));
            }
        };
        let mut child_map = args.into_iter().next().unwrap_or(Value::None);
        if child_map == Value::None {
            child_map = self.heap.alloc_dict(Vec::new());
        }
        if !kwargs.is_empty() {
            let dict_obj = match &child_map {
                Value::Dict(dict) => dict.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "ChainMap.new_child() map must be dict when kwargs are used",
                    ));
                }
            };
            for (key, value) in kwargs {
                dict_set_value_checked(&dict_obj, Value::Str(key), value)?;
            }
            child_map = Value::Dict(dict_obj);
        }

        let (instance_class, maps) = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new(
                    "ChainMap.new_child() expected a ChainMap instance",
                ));
            };
            let maps = match instance_data.attrs.get("maps") {
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            (instance_data.class.clone(), maps)
        };

        let mut ctor_args = Vec::with_capacity(maps.len() + 1);
        ctor_args.push(child_map);
        ctor_args.extend(maps);
        match self.call_internal(Value::Class(instance_class), ctor_args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("ChainMap.new_child() failed"))
            }
        }
    }

    pub(super) fn builtin_collections_chainmap_repr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "ChainMap.__repr__() expects one argument",
            ));
        }
        let instance = match &args[0] {
            Value::Instance(instance) => instance.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.__repr__() expected a ChainMap instance",
                ));
            }
        };
        let maps = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new(
                    "ChainMap.__repr__() expected a ChainMap instance",
                ));
            };
            match instance_data.attrs.get("maps") {
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            }
        };
        let class_name =
            class_name_for_instance(&instance).unwrap_or_else(|| "ChainMap".to_string());
        let rendered = maps.iter().map(format_repr).collect::<Vec<_>>().join(", ");
        Ok(Value::Str(format!("{class_name}({rendered})")))
    }

    pub(super) fn builtin_collections_chainmap_items(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("ChainMap.items() expects no arguments"));
        }
        let instance = match &args[0] {
            Value::Instance(instance) => instance.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.items() expected a ChainMap instance",
                ));
            }
        };
        let maps = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new(
                    "ChainMap.items() expected a ChainMap instance",
                ));
            };
            match instance_data.attrs.get("maps") {
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            }
        };

        let mut seen_keys: Vec<Value> = Vec::new();
        let mut out = Vec::new();
        for map in maps {
            let Value::Dict(dict) = map else {
                continue;
            };
            let Object::Dict(entries) = &*dict.kind() else {
                continue;
            };
            for (key, value) in entries {
                if seen_keys.iter().any(|seen| seen == key) {
                    continue;
                }
                seen_keys.push(key.clone());
                out.push(self.heap.alloc_tuple(vec![key.clone(), value.clone()]));
            }
        }
        Ok(self.heap.alloc_list(out))
    }

    pub(super) fn builtin_collections_chainmap_get(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "ChainMap.get() expects key and optional default",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let default = args.into_iter().next().unwrap_or(Value::None);
        let instance = match receiver {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.get() expected a ChainMap instance",
                ));
            }
        };
        let maps = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new(
                    "ChainMap.get() expected a ChainMap instance",
                ));
            };
            match instance_data.attrs.get("maps") {
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            }
        };

        for map in maps {
            let Value::Dict(dict) = map else {
                continue;
            };
            let Object::Dict(entries) = &*dict.kind() else {
                continue;
            };
            if let Some(value) = entries.find(&key) {
                return Ok(value.clone());
            }
        }
        Ok(default)
    }

    pub(super) fn builtin_collections_chainmap_getitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "ChainMap.__getitem__() expects one key argument",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let instance = match receiver {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.__getitem__() expected a ChainMap instance",
                ));
            }
        };
        let maps = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new(
                    "ChainMap.__getitem__() expected a ChainMap instance",
                ));
            };
            match instance_data.attrs.get("maps") {
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            }
        };
        for map in maps {
            let Value::Dict(dict) = map else {
                continue;
            };
            let Object::Dict(entries) = &*dict.kind() else {
                continue;
            };
            if let Some(value) = entries.find(&key) {
                return Ok(value.clone());
            }
        }
        Err(RuntimeError::key_error("key not found"))
    }

    pub(super) fn builtin_collections_chainmap_setitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(RuntimeError::new(
                "ChainMap.__setitem__() expects key and value arguments",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let value = args.remove(0);
        let instance = match receiver {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.__setitem__() expected a ChainMap instance",
                ));
            }
        };
        let first_map = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new(
                    "ChainMap.__setitem__() expected a ChainMap instance",
                ));
            };
            match instance_data.attrs.get("maps") {
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.first().cloned(),
                    _ => None,
                },
                _ => None,
            }
        };
        let Some(Value::Dict(dict)) = first_map else {
            return Err(RuntimeError::new(
                "ChainMap.__setitem__() first map must be dict",
            ));
        };
        dict_set_value_checked(&dict, key, value)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_chainmap_delitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "ChainMap.__delitem__() expects one key argument",
            ));
        }
        let receiver = args.remove(0);
        let key = args.remove(0);
        let instance = match receiver {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "ChainMap.__delitem__() expected a ChainMap instance",
                ));
            }
        };
        let first_map = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new(
                    "ChainMap.__delitem__() expected a ChainMap instance",
                ));
            };
            match instance_data.attrs.get("maps") {
                Some(Value::List(list)) => match &*list.kind() {
                    Object::List(values) => values.first().cloned(),
                    _ => None,
                },
                _ => None,
            }
        };
        let Some(Value::Dict(dict)) = first_map else {
            return Err(RuntimeError::new(
                "ChainMap.__delitem__() first map must be dict",
            ));
        };
        if dict_remove_value(&dict, &key).is_none() {
            return Err(RuntimeError::key_error("key not found"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_collections_defaultdict(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "defaultdict() expects optional default_factory and optional iterable",
            ));
        }
        let default_factory = if args.is_empty() {
            Value::None
        } else {
            args.remove(0)
        };
        if !matches!(default_factory, Value::None) && !self.is_callable_value(&default_factory) {
            return Err(RuntimeError::new("default_factory must be callable"));
        }
        let dict = match self.heap.alloc_dict(Vec::new()) {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };
        if let Some(source) = args.into_iter().next() {
            match source {
                Value::Dict(obj) => {
                    if let Object::Dict(entries) = &*obj.kind() {
                        for (key, value) in entries {
                            dict_set_value_checked(&dict, key.clone(), value.clone())?;
                        }
                    }
                }
                Value::List(obj) => {
                    if let Object::List(items) = &*obj.kind() {
                        for item in items {
                            if let Value::Tuple(tuple_obj) = item
                                && let Object::Tuple(parts) = &*tuple_obj.kind()
                                && parts.len() == 2
                            {
                                dict_set_value_checked(&dict, parts[0].clone(), parts[1].clone())?;
                                continue;
                            }
                            return Err(RuntimeError::new(
                                "defaultdict() iterable items must be key/value pairs",
                            ));
                        }
                    }
                }
                _ => return Err(RuntimeError::new("defaultdict() unsupported initializer")),
            }
        }
        self.defaultdict_factories
            .insert(dict.id(), default_factory);
        Ok(Value::Dict(dict))
    }

    pub(super) fn builtin_collections_ordereddict(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let value = self.builtin_dict(args, kwargs)?;
        let Value::Dict(dict) = &value else {
            return Err(RuntimeError::new(
                "OrderedDict() constructor returned non-dict",
            ));
        };
        self.ordered_dict_instances.insert(dict.id());
        Ok(value)
    }

    pub(super) fn builtin_collections_ordereddict_with_order(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        kwargs_order: Option<Vec<String>>,
    ) -> Result<Value, RuntimeError> {
        let value = self.builtin_dict_with_order(args, kwargs, kwargs_order)?;
        let Value::Dict(dict) = &value else {
            return Err(RuntimeError::new(
                "OrderedDict() constructor returned non-dict",
            ));
        };
        self.ordered_dict_instances.insert(dict.id());
        Ok(value)
    }

    pub(super) fn builtin_collections_count_elements(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "_count_elements() expects mapping and iterable arguments",
            ));
        }
        let mapping = args[0].clone();
        let iterable = args[1].clone();
        let bound_get = self.builtin_getattr(
            vec![mapping.clone(), Value::Str("get".to_string())],
            HashMap::new(),
        )?;
        let bound_setitem = self.lookup_bound_special_method(&mapping, "__setitem__")?;
        let dict_backing = match &mapping {
            Value::Dict(dict) => Some(dict.clone()),
            Value::Instance(instance) => self.instance_backing_dict(instance),
            _ => None,
        };
        if bound_setitem.is_none() && dict_backing.is_none() {
            return Err(RuntimeError::new(
                "_count_elements() mapping must support __setitem__",
            ));
        }

        for item in self.collect_iterable_values(iterable)? {
            let current = match self.call_internal(
                bound_get.clone(),
                vec![item.clone(), Value::Int(0)],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception(
                        "_count_elements() mapping.get() failed",
                    ));
                }
            };
            let next = add_values(current, Value::Int(1), &self.heap)?;
            if let Some(bound_setitem) = &bound_setitem {
                match self.call_internal(bound_setitem.clone(), vec![item, next], HashMap::new())? {
                    InternalCallOutcome::Value(_) => {}
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(self.runtime_error_from_active_exception(
                            "_count_elements() mapping.__setitem__() failed",
                        ));
                    }
                }
            } else if let Some(backing_dict) = &dict_backing {
                self.dict_set_value_checked_runtime(backing_dict, item, next)?;
            }
        }
        Ok(Value::None)
    }

    pub(super) fn inspect_module_for_value(&self, value: &Value) -> Option<Value> {
        match value {
            Value::Module(module) => Some(Value::Module(module.clone())),
            Value::Function(function) => match &*function.kind() {
                Object::Function(function_data) => {
                    Some(Value::Module(function_data.module.clone()))
                }
                _ => None,
            },
            Value::BoundMethod(method) => match &*method.kind() {
                Object::BoundMethod(bound_method) => match &*bound_method.function.kind() {
                    Object::Function(function_data) => {
                        Some(Value::Module(function_data.module.clone()))
                    }
                    _ => None,
                },
                _ => None,
            },
            Value::Class(class_ref) => match &*class_ref.kind() {
                Object::Class(class_data) => match class_data.attrs.get("__module__") {
                    Some(Value::Str(module_name)) => self
                        .modules
                        .get(module_name)
                        .map(|module| Value::Module(module.clone())),
                    _ => None,
                },
                _ => None,
            },
            Value::Instance(instance_ref) => match &*instance_ref.kind() {
                Object::Instance(instance_data) => match &*instance_data.class.kind() {
                    Object::Class(class_data) => match class_data.attrs.get("__module__") {
                        Some(Value::Str(module_name)) => self
                            .modules
                            .get(module_name)
                            .map(|module| Value::Module(module.clone())),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
            Value::Builtin(_) | Value::ExceptionType(_) => self
                .modules
                .get("builtins")
                .map(|module| Value::Module(module.clone())),
            _ => None,
        }
    }

    pub(super) fn inspect_file_for_value(&self, value: &Value) -> Option<String> {
        match value {
            Value::Module(module) => match &*module.kind() {
                Object::Module(module_data) => match module_data.globals.get("__file__") {
                    Some(Value::Str(path)) => Some(path.clone()),
                    _ => None,
                },
                _ => None,
            },
            Value::Function(function) => match &*function.kind() {
                Object::Function(function_data) => Some(function_data.code.filename.clone()),
                _ => None,
            },
            Value::BoundMethod(method) => match &*method.kind() {
                Object::BoundMethod(bound_method) => match &*bound_method.function.kind() {
                    Object::Function(function_data) => Some(function_data.code.filename.clone()),
                    _ => None,
                },
                _ => None,
            },
            Value::Code(code) => Some(code.filename.clone()),
            Value::Class(class_ref) => self
                .inspect_module_for_value(&Value::Class(class_ref.clone()))
                .and_then(|module| self.inspect_file_for_value(&module)),
            Value::Instance(instance_ref) => self
                .inspect_module_for_value(&Value::Instance(instance_ref.clone()))
                .and_then(|module| self.inspect_file_for_value(&module)),
            _ => None,
        }
    }

    pub(super) fn builtin_inspect_getmodule(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "getmodule() expects object and optional _filename",
            ));
        }
        let value = args.remove(0);
        Ok(self.inspect_module_for_value(&value).unwrap_or(Value::None))
    }

    pub(super) fn builtin_inspect_getfile(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("getfile() expects one object argument"));
        }
        let value = args.remove(0);
        let Some(path) = self.inspect_file_for_value(&value) else {
            return Err(RuntimeError::new(
                "TypeError: module, class, method, function, traceback, frame, or code object was expected",
            ));
        };
        Ok(Value::Str(path))
    }

    pub(super) fn builtin_inspect_getsourcefile(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "getsourcefile() expects one object argument",
            ));
        }
        let value = args.remove(0);
        let path = match self.builtin_inspect_getfile(vec![value], HashMap::new())? {
            Value::Str(path) => path,
            _ => return Ok(Value::None),
        };
        if path.ends_with(".pyc") {
            let mut source_path = path;
            source_path.pop();
            Ok(Value::Str(source_path))
        } else {
            Ok(Value::Str(path))
        }
    }

    pub(super) fn builtin_inspect_getdoc(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("getdoc() expects one argument"));
        }
        let target = args.remove(0);
        let doc_value = if matches!(target, Value::Str(_)) {
            target
        } else {
            match self.builtin_getattr(
                vec![target, Value::Str("__doc__".to_string()), Value::None],
                HashMap::new(),
            ) {
                Ok(value) => value,
                Err(err) if is_missing_attribute_error(&err) => Value::None,
                Err(err) => return Err(err),
            }
        };
        match doc_value {
            Value::None => Ok(Value::None),
            Value::Str(doc) => Ok(Value::Str(Self::inspect_cleandoc_text(&doc))),
            _ => Ok(Value::None),
        }
    }

    fn inspect_cleandoc_text(doc: &str) -> String {
        let mut expanded = String::with_capacity(doc.len());
        let mut column = 0usize;
        for ch in doc.chars() {
            match ch {
                '\t' => {
                    let spaces = 8 - (column % 8);
                    for _ in 0..spaces {
                        expanded.push(' ');
                    }
                    column += spaces;
                }
                '\n' => {
                    expanded.push('\n');
                    column = 0;
                }
                _ => {
                    expanded.push(ch);
                    column += 1;
                }
            }
        }

        let mut lines = expanded
            .split('\n')
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let mut margin: Option<usize> = None;
        for line in lines.iter().skip(1) {
            let stripped = line.trim_start_matches(' ');
            if stripped.is_empty() {
                continue;
            }
            let indent = line.len().saturating_sub(stripped.len());
            margin = Some(margin.map_or(indent, |current| current.min(indent)));
        }

        if let Some(first) = lines.first_mut() {
            *first = first.trim_start_matches(' ').to_string();
        }
        if let Some(indent) = margin {
            for line in lines.iter_mut().skip(1) {
                if line.len() >= indent {
                    *line = line[indent..].to_string();
                } else {
                    line.clear();
                }
            }
        }

        let mut start = 0usize;
        let mut end = lines.len();
        while start < end && lines[start].is_empty() {
            start += 1;
        }
        while end > start && lines[end - 1].is_empty() {
            end -= 1;
        }
        lines[start..end].join("\n")
    }

    pub(super) fn builtin_inspect_cleandoc(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("cleandoc() expects one argument"));
        }
        let mut doc_value = args.remove(0);
        if !matches!(doc_value, Value::Str(_) | Value::None) {
            doc_value = match self.builtin_getattr(
                vec![doc_value, Value::Str("__doc__".to_string()), Value::None],
                HashMap::new(),
            ) {
                Ok(value) => value,
                Err(err) if is_missing_attribute_error(&err) => Value::None,
                Err(err) => return Err(err),
            };
        }
        match doc_value {
            Value::None => Ok(Value::None),
            Value::Str(doc) => Ok(Value::Str(Self::inspect_cleandoc_text(&doc))),
            _ => Ok(Value::None),
        }
    }

    pub(super) fn builtin_inspect_signature(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "signature() expects one callable argument",
            ));
        }
        let follow_wrapped = kwargs
            .remove("follow_wrapped")
            .map(|value| self.truthy_from_value(&value))
            .transpose()?
            .unwrap_or(true);
        for name in ["globals", "locals", "eval_str", "annotation_format"] {
            kwargs.remove(name);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "signature() got an unexpected keyword argument",
            ));
        }

        let mut callable = args.remove(0);
        if follow_wrapped {
            let mut visited = std::collections::HashSet::new();
            let value_identity = |value: &Value| -> Option<u64> {
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
                    | Value::Cell(obj) => Some(obj.id()),
                    _ => None,
                }
            };
            for _ in 0..64 {
                let Some(identity) = value_identity(&callable) else {
                    break;
                };
                if !visited.insert(identity) {
                    break;
                }
                let wrapped = match self.builtin_getattr(
                    vec![callable.clone(), Value::Str("__wrapped__".to_string())],
                    HashMap::new(),
                ) {
                    Ok(value) => value,
                    Err(err) if is_missing_attribute_error(&err) => break,
                    Err(err) => return Err(err),
                };
                callable = wrapped;
            }
        }
        let signature_class = self
            .modules
            .get("inspect")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("Signature").cloned(),
                _ => None,
            })
            .and_then(|value| match value {
                Value::Class(class) => Some(class),
                _ => None,
            })
            .ok_or_else(|| RuntimeError::new("inspect.Signature unavailable"))?;
        let signature_empty = match &*signature_class.kind() {
            Object::Class(class_data) => class_data
                .attrs
                .get("empty")
                .cloned()
                .unwrap_or(Value::None),
            _ => Value::None,
        };
        let parameter_class = self
            .modules
            .get("inspect")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("Parameter").cloned(),
                _ => None,
            })
            .and_then(|value| match value {
                Value::Class(class) => Some(class),
                _ => None,
            })
            .ok_or_else(|| RuntimeError::new("inspect.Parameter unavailable"))?;
        let parameter_empty = match &*parameter_class.kind() {
            Object::Class(class_data) => class_data
                .attrs
                .get("empty")
                .cloned()
                .unwrap_or(Value::None),
            _ => Value::None,
        };

        let text_signature_override = match self.builtin_getattr(
            vec![
                callable.clone(),
                Value::Str("__text_signature__".to_string()),
            ],
            HashMap::new(),
        ) {
            Ok(Value::Str(text)) if !text.is_empty() => Some(text),
            Ok(_) => None,
            Err(err) if is_missing_attribute_error(&err) => None,
            Err(err) => return Err(err),
        };

        let mut params = Vec::new();
        let mut parts = Vec::new();
        let mut return_annotation = signature_empty;

        let make_param =
            |name: String, kind: &str, default: Option<Value>| -> (String, (Value, Value)) {
                let has_default = default.is_some();
                let default_value = default.unwrap_or_else(|| parameter_empty.clone());
                let rendered = if has_default {
                    let default_text = format_repr(&default_value);
                    format!("{name}={default_text}")
                } else {
                    name.clone()
                };
                let kind_value = match &*parameter_class.kind() {
                    Object::Class(class_data) => {
                        class_data.attrs.get(kind).cloned().unwrap_or(Value::None)
                    }
                    _ => Value::None,
                };
                let parameter_instance = match self
                    .heap
                    .alloc_instance(InstanceObject::new(parameter_class.clone()))
                {
                    Value::Instance(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Instance(instance_data) = &mut *parameter_instance.kind_mut() {
                    instance_data
                        .attrs
                        .insert("name".to_string(), Value::Str(name.clone()));
                    instance_data.attrs.insert("kind".to_string(), kind_value);
                    instance_data
                        .attrs
                        .insert("default".to_string(), default_value);
                    instance_data
                        .attrs
                        .insert("annotation".to_string(), parameter_empty.clone());
                }
                (
                    rendered,
                    (Value::Str(name), Value::Instance(parameter_instance)),
                )
            };

        let mut populate_from_function = |func: ObjRef,
                                          skip_first: bool|
         -> Result<(), RuntimeError> {
            let (
                posonly_params,
                positional_params,
                vararg,
                kwarg,
                kwonly_params,
                defaults,
                kwonly_defaults,
                annotations,
                function_dict,
            ) = {
                let function_ref = func.kind();
                let function = match &*function_ref {
                    Object::Function(function) => function,
                    _ => unreachable!(),
                };
                (
                    function.code.posonly_params.clone(),
                    function.code.params.clone(),
                    function.code.vararg.clone(),
                    function.code.kwarg.clone(),
                    function.code.kwonly_params.clone(),
                    function.defaults.clone(),
                    function.kwonly_defaults.clone(),
                    function.annotations.clone(),
                    function.dict.clone(),
                )
            };
            let dict_annotations = function_dict.as_ref().and_then(|dict| match &*dict.kind() {
                Object::Dict(entries) => entries.iter().find_map(|(key, value)| {
                    if matches!(key, Value::Str(name) if name == "__annotations__")
                        && let Value::Dict(dict_obj) = value
                    {
                        return Some(dict_obj.clone());
                    }
                    None
                }),
                _ => None,
            });
            let annotations_dict = dict_annotations.or(annotations);
            let posonly_len = posonly_params.len();
            let positional_len = posonly_len + positional_params.len();
            let default_start = positional_len.saturating_sub(defaults.len());
            let skip = usize::from(skip_first);
            let mut rendered_posonly = 0usize;
            let params_start = params.len();

            for (idx, name) in posonly_params.iter().enumerate() {
                if idx < skip {
                    continue;
                }
                let default = if idx >= default_start {
                    Some(defaults[idx - default_start].clone())
                } else {
                    None
                };
                let (rendered, entry) = make_param(name.clone(), "POSITIONAL_ONLY", default);
                parts.push(rendered);
                params.push(entry);
                rendered_posonly = rendered_posonly.saturating_add(1);
            }
            if rendered_posonly > 0 {
                parts.push("/".to_string());
            }

            for (idx, name) in positional_params.iter().enumerate() {
                let param_idx = posonly_len + idx;
                if param_idx < skip {
                    continue;
                }
                let default = if param_idx >= default_start {
                    Some(defaults[param_idx - default_start].clone())
                } else {
                    None
                };
                let (rendered, entry) = make_param(name.clone(), "POSITIONAL_OR_KEYWORD", default);
                parts.push(rendered);
                params.push(entry);
            }

            if let Some(vararg) = &vararg {
                let (rendered, entry) = make_param(vararg.clone(), "VAR_POSITIONAL", None);
                parts.push(format!("*{rendered}"));
                params.push(entry);
            } else if !kwonly_params.is_empty() {
                parts.push("*".to_string());
            }

            for name in &kwonly_params {
                let default = kwonly_defaults.get(name).cloned();
                let (rendered, entry) = make_param(name.clone(), "KEYWORD_ONLY", default);
                parts.push(rendered);
                params.push(entry);
            }

            if let Some(kwarg) = &kwarg {
                let (rendered, entry) = make_param(kwarg.clone(), "VAR_KEYWORD", None);
                parts.push(format!("**{rendered}"));
                params.push(entry);
            }

            if let Some(annotations) = annotations_dict
                && let Object::Dict(entries) = &*annotations.kind()
            {
                for (key, value) in entries.iter() {
                    let Value::Str(name) = key else {
                        continue;
                    };
                    if name == "return" {
                        return_annotation = value.clone();
                        continue;
                    }
                    for (_param_name, param) in params.iter_mut().skip(params_start) {
                        let Value::Instance(param_instance) = param else {
                            continue;
                        };
                        let Object::Instance(param_data) = &mut *param_instance.kind_mut() else {
                            continue;
                        };
                        if matches!(
                            param_data.attrs.get("name"),
                            Some(Value::Str(param_name)) if param_name == name
                        ) {
                            param_data
                                .attrs
                                .insert("annotation".to_string(), value.clone());
                        }
                    }
                }
            }
            Ok(())
        };

        let mut live_annotations_function: Option<ObjRef> = None;
        match callable {
            Value::Function(func) => {
                live_annotations_function = Some(func.clone());
                populate_from_function(func, false)?;
            }
            Value::BoundMethod(method_obj) => {
                let mut handled_native_descriptor = false;
                if let Object::BoundMethod(method_data) = &*method_obj.kind()
                    && let Object::NativeMethod(native) = &*method_data.function.kind()
                    && native.kind == NativeMethodKind::FunctionDescriptorGet
                    && let Object::Module(module_data) = &*method_data.receiver.kind()
                    && module_data.name == "__builtin_descriptor__"
                {
                    let (instance_rendered, instance_entry) =
                        make_param("instance".to_string(), "POSITIONAL_OR_KEYWORD", None);
                    parts.push(instance_rendered);
                    params.push(instance_entry);
                    let (owner_rendered, owner_entry) = make_param(
                        "owner".to_string(),
                        "POSITIONAL_OR_KEYWORD",
                        Some(Value::None),
                    );
                    parts.push(owner_rendered);
                    params.push(owner_entry);
                    handled_native_descriptor = true;
                }
                if !handled_native_descriptor && text_signature_override.is_none() {
                    let (args_rendered, args_entry) =
                        make_param("args".to_string(), "VAR_POSITIONAL", None);
                    parts.push(format!("*{args_rendered}"));
                    params.push(args_entry);
                    let (kwargs_rendered, kwargs_entry) =
                        make_param("kwargs".to_string(), "VAR_KEYWORD", None);
                    parts.push(format!("**{kwargs_rendered}"));
                    params.push(kwargs_entry);
                }
            }
            Value::Class(class_ref) => {
                if let Some(Value::Function(func)) = class_attr_lookup(&class_ref, "__init__") {
                    live_annotations_function = Some(func.clone());
                    populate_from_function(func, true)?;
                } else if text_signature_override.is_none() {
                    let (args_rendered, args_entry) =
                        make_param("args".to_string(), "VAR_POSITIONAL", None);
                    parts.push(format!("*{args_rendered}"));
                    params.push(args_entry);
                    let (kwargs_rendered, kwargs_entry) =
                        make_param("kwargs".to_string(), "VAR_KEYWORD", None);
                    parts.push(format!("**{kwargs_rendered}"));
                    params.push(kwargs_entry);
                }
            }
            _ => {
                if text_signature_override.is_none() {
                    let (args_rendered, args_entry) =
                        make_param("args".to_string(), "VAR_POSITIONAL", None);
                    parts.push(format!("*{args_rendered}"));
                    params.push(args_entry);
                    let (kwargs_rendered, kwargs_entry) =
                        make_param("kwargs".to_string(), "VAR_KEYWORD", None);
                    parts.push(format!("**{kwargs_rendered}"));
                    params.push(kwargs_entry);
                }
            }
        }
        if let Some(func) = live_annotations_function {
            match self.optional_getattr_value(Value::Function(func), "__annotations__") {
                Ok(Some(Value::Dict(annotations))) => {
                    if let Object::Dict(entries) = &*annotations.kind()
                        && let Some((_, value)) = entries
                            .iter()
                            .find(|(key, _)| matches!(key, Value::Str(name) if name == "return"))
                    {
                        return_annotation = value.clone();
                    }
                }
                Ok(_) => {}
                Err(err)
                    if runtime_error_matches_exception(&err, "NotImplementedError")
                        || runtime_error_matches_exception(&err, "RecursionError") => {}
                Err(err) => return Err(err),
            }
        }

        let signature_text = Value::Str(
            text_signature_override.unwrap_or_else(|| format!("({})", parts.join(", "))),
        );
        let instance = match self
            .heap
            .alloc_instance(InstanceObject::new(signature_class))
        {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("__text__".to_string(), signature_text);
            instance_data
                .attrs
                .insert("parameters".to_string(), self.heap.alloc_dict(params));
            instance_data
                .attrs
                .insert("return_annotation".to_string(), return_annotation);
        }
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_inspect_signature_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let normalize_parameters = |vm: &mut Vm, value: Value| -> Value {
            match value {
                Value::Dict(_) => value,
                Value::List(obj) => {
                    let list_values = if let Object::List(values) = &*obj.kind() {
                        values.clone()
                    } else {
                        Vec::new()
                    };
                    let mut entries = Vec::with_capacity(list_values.len());
                    for (index, item) in list_values.into_iter().enumerate() {
                        let key = match &item {
                            Value::Instance(parameter_obj) => {
                                if let Object::Instance(parameter_data) = &*parameter_obj.kind() {
                                    match parameter_data.attrs.get("name") {
                                        Some(Value::Str(name)) => Value::Str(name.clone()),
                                        _ => Value::Str(format!("arg{index}")),
                                    }
                                } else {
                                    Value::Str(format!("arg{index}"))
                                }
                            }
                            _ => Value::Str(format!("arg{index}")),
                        };
                        entries.push((key, item));
                    }
                    vm.heap.alloc_dict(entries)
                }
                Value::Tuple(obj) => {
                    let tuple_values = if let Object::Tuple(values) = &*obj.kind() {
                        values.clone()
                    } else {
                        Vec::new()
                    };
                    let mut entries = Vec::with_capacity(tuple_values.len());
                    for (index, item) in tuple_values.into_iter().enumerate() {
                        let key = match &item {
                            Value::Instance(parameter_obj) => {
                                if let Object::Instance(parameter_data) = &*parameter_obj.kind() {
                                    match parameter_data.attrs.get("name") {
                                        Some(Value::Str(name)) => Value::Str(name.clone()),
                                        _ => Value::Str(format!("arg{index}")),
                                    }
                                } else {
                                    Value::Str(format!("arg{index}"))
                                }
                            }
                            _ => Value::Str(format!("arg{index}")),
                        };
                        entries.push((key, item));
                    }
                    vm.heap.alloc_dict(entries)
                }
                other => other,
            }
        };
        let instance = self.take_bound_instance_arg(&mut args, "Signature.__init__")?;
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "Signature.__init__() takes at most 2 positional arguments",
            ));
        }
        let positional_parameters = args.first().cloned();
        let positional_return = args.get(1).cloned();
        let parameters = kwargs
            .remove("parameters")
            .or(positional_parameters)
            .unwrap_or_else(|| self.heap.alloc_dict(Vec::new()));
        let parameters = normalize_parameters(self, parameters);
        let return_annotation = kwargs
            .remove("return_annotation")
            .or(positional_return)
            .unwrap_or(Value::None);
        kwargs.remove("__validate_parameters__");
        if let Some(name) = kwargs.into_keys().next() {
            return Err(RuntimeError::new(format!(
                "Signature.__init__() got an unexpected keyword argument '{name}'"
            )));
        }
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            if !instance_data.attrs.contains_key("__text__") {
                instance_data
                    .attrs
                    .insert("__text__".to_string(), Value::Str("()".to_string()));
            }
            instance_data
                .attrs
                .insert("parameters".to_string(), parameters);
            instance_data
                .attrs
                .insert("return_annotation".to_string(), return_annotation);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_inspect_parameter_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "Parameter.__init__")?;
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "Parameter.__init__() takes at most 2 positional arguments",
            ));
        }
        let name = kwargs
            .remove("name")
            .or_else(|| args.first().cloned())
            .unwrap_or(Value::Str(String::new()));
        let kind = kwargs
            .remove("kind")
            .or_else(|| args.get(1).cloned())
            .unwrap_or(Value::Int(1));
        let default = kwargs.remove("default").unwrap_or(Value::None);
        let annotation = kwargs.remove("annotation").unwrap_or(Value::None);
        if let Some(key) = kwargs.into_keys().next() {
            return Err(RuntimeError::new(format!(
                "Parameter.__init__() got an unexpected keyword argument '{key}'"
            )));
        }
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data.attrs.insert("name".to_string(), name);
            instance_data.attrs.insert("kind".to_string(), kind);
            instance_data.attrs.insert("default".to_string(), default);
            instance_data
                .attrs
                .insert("annotation".to_string(), annotation);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_inspect_parameter_replace(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "Parameter.replace")?;
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "Parameter.replace() takes no positional arguments",
            ));
        }
        let name_override = kwargs.remove("name");
        let kind_override = kwargs.remove("kind");
        let default_override = kwargs.remove("default");
        let annotation_override = kwargs.remove("annotation");
        if let Some(key) = kwargs.into_keys().next() {
            return Err(RuntimeError::new(format!(
                "Parameter.replace() got an unexpected keyword argument '{key}'"
            )));
        }
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "Parameter.replace() receiver must be Parameter instance",
                ));
            }
        };
        let replacement = match self.heap.alloc_instance(InstanceObject::new(class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        let current_name = Self::instance_attr_get(&instance, "name");
        let current_kind = Self::instance_attr_get(&instance, "kind");
        let current_default = Self::instance_attr_get(&instance, "default");
        let current_annotation = Self::instance_attr_get(&instance, "annotation");
        if let Object::Instance(instance_data) = &mut *replacement.kind_mut() {
            instance_data.attrs.insert(
                "name".to_string(),
                name_override.unwrap_or_else(|| current_name.unwrap_or(Value::Str(String::new()))),
            );
            instance_data.attrs.insert(
                "kind".to_string(),
                kind_override.unwrap_or_else(|| current_kind.unwrap_or(Value::Int(1))),
            );
            instance_data.attrs.insert(
                "default".to_string(),
                default_override.unwrap_or_else(|| current_default.unwrap_or(Value::None)),
            );
            instance_data.attrs.insert(
                "annotation".to_string(),
                annotation_override.unwrap_or_else(|| current_annotation.unwrap_or(Value::None)),
            );
        }
        Ok(Value::Instance(replacement))
    }

    pub(super) fn builtin_inspect_signature_str(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "Signature.__str__() expects no arguments",
            ));
        }
        let instance = self.receiver_from_value(&args[0])?;
        match Self::instance_attr_get(&instance, "__text__") {
            Some(Value::Str(text)) => Ok(Value::Str(text)),
            _ => Ok(Value::Str("<Signature instance>".to_string())),
        }
    }

    pub(super) fn builtin_inspect_signature_repr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "Signature.__repr__() expects no arguments",
            ));
        }
        let instance = self.receiver_from_value(&args[0])?;
        match Self::instance_attr_get(&instance, "__text__") {
            Some(Value::Str(text)) => Ok(Value::Str(format!("<Signature {text}>"))),
            _ => Ok(Value::Str("<Signature instance>".to_string())),
        }
    }

    pub(super) fn builtin_inspect_signature_eq(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("Signature.__eq__() expects one argument"));
        }
        let left = self.receiver_from_value(&args[0])?;
        let Value::Instance(right) = &args[1] else {
            return Ok(Value::Bool(false));
        };
        let is_right_signature = match &*right.kind() {
            Object::Instance(instance_data) => match &*instance_data.class.kind() {
                Object::Class(class_data) => class_data.name == "Signature",
                _ => false,
            },
            _ => false,
        };
        if !is_right_signature {
            return Ok(Value::Bool(false));
        }
        let left_text = Self::instance_attr_get(&left, "__text__").unwrap_or(Value::None);
        let right_text = Self::instance_attr_get(right, "__text__").unwrap_or(Value::None);
        let text_compare = self.compare_eq_runtime(left_text, right_text)?;
        let text_equal = self.truthy_from_value(&text_compare)?;
        if !text_equal {
            return Ok(Value::Bool(false));
        }
        let left_return =
            Self::instance_attr_get(&left, "return_annotation").unwrap_or(Value::None);
        let right_return =
            Self::instance_attr_get(right, "return_annotation").unwrap_or(Value::None);
        let return_compare = self.compare_eq_runtime(left_return, right_return)?;
        let return_equal = self.truthy_from_value(&return_compare)?;
        Ok(Value::Bool(return_equal))
    }

    pub(super) fn builtin_inspect_signature_replace(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let normalize_parameters = |vm: &mut Vm, value: Value| -> Value {
            match value {
                Value::Dict(_) => value,
                Value::List(obj) => {
                    let list_values = if let Object::List(values) = &*obj.kind() {
                        values.clone()
                    } else {
                        Vec::new()
                    };
                    let mut entries = Vec::with_capacity(list_values.len());
                    for (index, item) in list_values.into_iter().enumerate() {
                        let key = match &item {
                            Value::Instance(parameter_obj) => {
                                if let Object::Instance(parameter_data) = &*parameter_obj.kind() {
                                    match parameter_data.attrs.get("name") {
                                        Some(Value::Str(name)) => Value::Str(name.clone()),
                                        _ => Value::Str(format!("arg{index}")),
                                    }
                                } else {
                                    Value::Str(format!("arg{index}"))
                                }
                            }
                            _ => Value::Str(format!("arg{index}")),
                        };
                        entries.push((key, item));
                    }
                    vm.heap.alloc_dict(entries)
                }
                Value::Tuple(obj) => {
                    let tuple_values = if let Object::Tuple(values) = &*obj.kind() {
                        values.clone()
                    } else {
                        Vec::new()
                    };
                    let mut entries = Vec::with_capacity(tuple_values.len());
                    for (index, item) in tuple_values.into_iter().enumerate() {
                        let key = match &item {
                            Value::Instance(parameter_obj) => {
                                if let Object::Instance(parameter_data) = &*parameter_obj.kind() {
                                    match parameter_data.attrs.get("name") {
                                        Some(Value::Str(name)) => Value::Str(name.clone()),
                                        _ => Value::Str(format!("arg{index}")),
                                    }
                                } else {
                                    Value::Str(format!("arg{index}"))
                                }
                            }
                            _ => Value::Str(format!("arg{index}")),
                        };
                        entries.push((key, item));
                    }
                    vm.heap.alloc_dict(entries)
                }
                other => other,
            }
        };
        let instance = self.take_bound_instance_arg(&mut args, "Signature.replace")?;
        if !args.is_empty() {
            return Err(RuntimeError::new("replace() takes no positional arguments"));
        }
        let parameters_override = kwargs.remove("parameters");
        let return_annotation_override = kwargs.remove("return_annotation");
        if let Some(name) = kwargs.into_keys().next() {
            return Err(RuntimeError::new(format!(
                "replace() got an unexpected keyword argument '{name}'"
            )));
        }
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "replace() receiver must be Signature instance",
                ));
            }
        };
        let copied_text = Self::instance_attr_get(&instance, "__text__");
        let copied_parameters = Self::instance_attr_get(&instance, "parameters");
        let copied_return_annotation = Self::instance_attr_get(&instance, "return_annotation");
        let replacement = match self.heap.alloc_instance(InstanceObject::new(class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(replacement_data) = &mut *replacement.kind_mut() {
            if let Some(text) = copied_text {
                replacement_data.attrs.insert("__text__".to_string(), text);
            }
            let normalized_parameters = parameters_override
                .map(|value| normalize_parameters(self, value))
                .or_else(|| copied_parameters.map(|value| normalize_parameters(self, value)))
                .unwrap_or(Value::None);
            replacement_data
                .attrs
                .insert("parameters".to_string(), normalized_parameters);
            replacement_data.attrs.insert(
                "return_annotation".to_string(),
                return_annotation_override
                    .unwrap_or_else(|| copied_return_annotation.unwrap_or(Value::None)),
            );
        }
        Ok(Value::Instance(replacement))
    }

    pub(super) fn builtin_inspect_signature_bind(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        allow_partial: bool,
    ) -> Result<Value, RuntimeError> {
        let method_name = if allow_partial {
            "Signature.bind_partial"
        } else {
            "Signature.bind"
        };
        let instance = self.take_bound_instance_arg(&mut args, method_name)?;
        let Some(parameters_value) = Self::instance_attr_get(&instance, "parameters") else {
            return Err(RuntimeError::new(
                "Signature.bind() receiver is missing parameters",
            ));
        };
        let Some(inspect_module) = self.modules.get("inspect").cloned() else {
            return Err(RuntimeError::new("inspect module is unavailable"));
        };
        let empty_sentinel = match &*inspect_module.kind() {
            Object::Module(module_data) => module_data
                .globals
                .get("_empty")
                .cloned()
                .unwrap_or(Value::None),
            _ => Value::None,
        };
        let bound_arguments_class = match &*inspect_module.kind() {
            Object::Module(module_data) => module_data.globals.get("BoundArguments").cloned(),
            _ => None,
        }
        .and_then(|value| match value {
            Value::Class(class_obj) => Some(class_obj),
            _ => None,
        });
        let is_empty = |value: &Value| match (value, &empty_sentinel) {
            (Value::Instance(left), Value::Instance(right)) => left.id() == right.id(),
            _ => value == &empty_sentinel,
        };

        let mut parameter_entries: Vec<(String, i64, Value)> = Vec::new();
        if let Value::Dict(parameter_dict) = parameters_value
            && let Object::Dict(entries) = &*parameter_dict.kind()
        {
            for (key, parameter) in entries.iter() {
                let Value::Str(name) = key else {
                    continue;
                };
                let kind = match &parameter {
                    Value::Instance(parameter_instance) => {
                        match Self::instance_attr_get(parameter_instance, "kind") {
                            Some(Value::Int(raw_kind)) => raw_kind,
                            _ => 1,
                        }
                    }
                    _ => 1,
                };
                let default = match &parameter {
                    Value::Instance(parameter_instance) => {
                        Self::instance_attr_get(parameter_instance, "default")
                            .unwrap_or_else(|| empty_sentinel.clone())
                    }
                    _ => empty_sentinel.clone(),
                };
                parameter_entries.push((name.clone(), kind, default));
            }
        }

        let mut positional_index = 0usize;
        let mut consumed_keywords: HashMap<String, bool> = HashMap::new();
        let mut bound_entries: Vec<(Value, Value)> = Vec::new();
        let mut var_keyword_parameter: Option<String> = None;

        for (name, kind, default) in parameter_entries {
            match kind {
                0 => {
                    if kwargs.contains_key(&name) {
                        return Err(RuntimeError::new(format!(
                            "TypeError: {method_name}() positional-only parameter '{name}' passed as keyword"
                        )));
                    }
                    if let Some(value) = args.get(positional_index).cloned() {
                        positional_index += 1;
                        bound_entries.push((Value::Str(name), value));
                    } else if !allow_partial && is_empty(&default) {
                        return Err(RuntimeError::new(format!(
                            "TypeError: missing a required argument: '{name}'"
                        )));
                    }
                }
                1 => {
                    let positional_value = args.get(positional_index).cloned();
                    let keyword_value = kwargs.get(&name).cloned();
                    match (positional_value, keyword_value) {
                        (Some(_), Some(_)) => {
                            return Err(RuntimeError::new(format!(
                                "TypeError: multiple values for argument '{name}'"
                            )));
                        }
                        (Some(value), None) => {
                            positional_index += 1;
                            bound_entries.push((Value::Str(name), value));
                        }
                        (None, Some(value)) => {
                            consumed_keywords.insert(name.clone(), true);
                            bound_entries.push((Value::Str(name), value));
                        }
                        (None, None) => {
                            if !allow_partial && is_empty(&default) {
                                return Err(RuntimeError::new(format!(
                                    "TypeError: missing a required argument: '{name}'"
                                )));
                            }
                        }
                    }
                }
                2 => {
                    let mut rest = Vec::new();
                    while let Some(value) = args.get(positional_index).cloned() {
                        positional_index += 1;
                        rest.push(value);
                    }
                    bound_entries.push((Value::Str(name), self.heap.alloc_tuple(rest)));
                }
                3 => {
                    if let Some(value) = kwargs.get(&name).cloned() {
                        consumed_keywords.insert(name.clone(), true);
                        bound_entries.push((Value::Str(name), value));
                    } else if !allow_partial && is_empty(&default) {
                        return Err(RuntimeError::new(format!(
                            "TypeError: missing a required argument: '{name}'"
                        )));
                    }
                }
                4 => {
                    var_keyword_parameter = Some(name);
                }
                _ => {}
            }
        }

        if positional_index < args.len() {
            return Err(RuntimeError::new(
                "TypeError: too many positional arguments",
            ));
        }

        let mut remaining_keywords: Vec<(Value, Value)> = Vec::new();
        for (name, value) in kwargs {
            if consumed_keywords.contains_key(&name) {
                continue;
            }
            remaining_keywords.push((Value::Str(name), value));
        }
        if let Some(parameter_name) = var_keyword_parameter {
            bound_entries.push((
                Value::Str(parameter_name),
                self.heap.alloc_dict(remaining_keywords),
            ));
        } else if let Some((Value::Str(name), _)) = remaining_keywords.first() {
            return Err(RuntimeError::new(format!(
                "TypeError: got an unexpected keyword argument '{name}'"
            )));
        }

        let arguments_value = self.heap.alloc_dict(bound_entries);
        if let Some(bound_class) = bound_arguments_class {
            let bound_instance = match self.heap.alloc_instance(InstanceObject::new(bound_class)) {
                Value::Instance(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Instance(instance_data) = &mut *bound_instance.kind_mut() {
                instance_data
                    .attrs
                    .insert("arguments".to_string(), arguments_value);
                instance_data
                    .attrs
                    .insert("signature".to_string(), Value::Instance(instance));
            }
            return Ok(Value::Instance(bound_instance));
        }
        Ok(arguments_value)
    }

    pub(super) fn builtin_inspect_isfunction(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        // CPython inspect.isfunction() is true for user-defined function objects only.
        // Bound methods must return false here (they are covered by inspect.ismethod()).
        unary_predicate(args, kwargs, |value| matches!(value, Value::Function(_)))
    }

    pub(super) fn builtin_inspect_ismethod(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::BoundMethod(_)))
    }

    pub(super) fn builtin_inspect_markcoroutinefunction(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "markcoroutinefunction() expects one argument",
            ));
        }
        let mut target = args.remove(0);
        let bound_function = if let Value::BoundMethod(method) = &target {
            match &*method.kind() {
                Object::BoundMethod(method_data)
                    if matches!(&*method_data.function.kind(), Object::Function(_)) =>
                {
                    Some(method_data.function.clone())
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(function) = bound_function {
            target = Value::Function(function);
        }
        if let Some(unbound) = self.optional_getattr_value(target.clone(), "__func__")?
            && matches!(unbound, Value::Function(_))
        {
            target = unbound;
        }
        self.builtin_setattr(
            vec![
                target.clone(),
                Value::Str("_is_coroutine_marker".to_string()),
                Value::Bool(true),
            ],
            HashMap::new(),
        )?;
        Ok(target)
    }

    pub(super) fn builtin_inspect_isroutine(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            matches!(
                value,
                Value::Function(_) | Value::Builtin(_) | Value::BoundMethod(_)
            )
        })
    }

    pub(super) fn builtin_inspect_ismethoddescriptor(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::Builtin(_)))
    }

    pub(super) fn builtin_inspect_isdatadescriptor(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("predicate expects one argument"));
        }
        let value = &args[0];
        if matches!(
            value,
            Value::Class(_) | Value::BoundMethod(_) | Value::Function(_)
        ) {
            return Ok(Value::Bool(false));
        }
        let is_data_descriptor = self
            .class_of_value(value)
            .map(|class| {
                class_attr_lookup(&class, "__set__").is_some()
                    || class_attr_lookup(&class, "__delete__").is_some()
            })
            .unwrap_or(false);
        let object_has_data_descriptor_slot = ["__set__", "__delete__"].iter().any(|name| {
            matches!(
                self.builtin_hasattr(
                    vec![value.clone(), Value::Str((*name).to_string())],
                    HashMap::new()
                ),
                Ok(Value::Bool(true))
            )
        });
        Ok(Value::Bool(
            is_data_descriptor || object_has_data_descriptor_slot,
        ))
    }

    pub(super) fn builtin_inspect_ismethodwrapper(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |_value| false)
    }

    pub(super) fn builtin_inspect_istraceback(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| match value {
            Value::Instance(obj) => match &*obj.kind() {
                Object::Instance(instance_data) => matches!(
                    instance_data.attrs.get("__pyrs_traceback_marker__"),
                    Some(Value::Bool(true))
                ),
                _ => false,
            },
            _ => false,
        })
    }

    pub(super) fn builtin_inspect_isframe(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |_value| false)
    }

    pub(super) fn builtin_inspect_iscode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::Code(_)))
    }

    pub(super) fn builtin_inspect_unwrap(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if kwargs.remove("stop").is_some() {
            if !kwargs.is_empty() {
                return Err(RuntimeError::new(
                    "unwrap() got an unexpected keyword argument",
                ));
            }
        } else if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "unwrap() got an unexpected keyword argument",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new("unwrap() expects one argument"));
        }
        Ok(args.remove(0))
    }

    pub(super) fn builtin_inspect_isabstract(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isabstract() expects one argument"));
        }
        let Some(Value::Class(class_ref)) = args.first() else {
            return Ok(Value::Bool(false));
        };
        if let Some(abstract_methods) = self.optional_internal_getattr_value(
            Value::Class(class_ref.clone()),
            "__abstractmethods__",
        )? && self.truthy_from_value(&abstract_methods)?
        {
            return Ok(Value::Bool(true));
        }
        let class_attrs = match &*class_ref.kind() {
            Object::Class(class_data) => class_data.attrs.values().cloned().collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        for attr_value in class_attrs {
            if self.object_is_abstract(&attr_value)? {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_inspect_isclass(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::Class(_)))
    }

    pub(super) fn builtin_inspect_ismodule(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::Module(_)))
    }

    pub(super) fn builtin_inspect_isgenerator(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            if let Value::Generator(generator) = value
                && let Object::Generator(state) = &*generator.kind()
            {
                return !state.is_coroutine && !state.is_async_generator;
            }
            false
        })
    }

    pub(super) fn builtin_inspect_iscoroutine(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            if let Value::Generator(generator) = value
                && let Object::Generator(state) = &*generator.kind()
            {
                return state.is_coroutine;
            }
            false
        })
    }

    pub(super) fn builtin_inspect_iscoroutinefunction(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("predicate expects one argument"));
        }
        let value = args.remove(0);
        let code_marks_coroutine = match &value {
            Value::Function(func) => match &*func.kind() {
                Object::Function(function_data) => function_data.code.is_coroutine,
                _ => false,
            },
            Value::BoundMethod(method) => match &*method.kind() {
                Object::BoundMethod(method_data) => match &*method_data.function.kind() {
                    Object::Function(function_data) => function_data.code.is_coroutine,
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        };
        if code_marks_coroutine {
            return Ok(Value::Bool(true));
        }
        let marker_target = match &value {
            Value::BoundMethod(method) => match &*method.kind() {
                Object::BoundMethod(method_data)
                    if matches!(&*method_data.function.kind(), Object::Function(_)) =>
                {
                    Value::Function(method_data.function.clone())
                }
                _ => value.clone(),
            },
            _ => value.clone(),
        };
        let has_marker = self
            .optional_getattr_value(marker_target, "_is_coroutine_marker")?
            .map(|marker| is_truthy(&marker))
            .unwrap_or(false);
        Ok(Value::Bool(has_marker))
    }

    pub(super) fn builtin_inspect_isawaitable(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("predicate expects one argument"));
        }
        Ok(Value::Bool(self.is_awaitable_value(&args[0])))
    }

    pub(super) fn builtin_inspect_isasyncgen(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            if let Value::Generator(generator) = value
                && let Object::Generator(state) = &*generator.kind()
            {
                return state.is_async_generator;
            }
            false
        })
    }

    pub(super) fn builtin_inspect_getattr_static(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "getattr_static() expects object, attribute, and optional default",
            ));
        }
        let target = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::type_error("attribute name must be string")),
        };
        let default = args.pop();
        match target {
            Value::Class(class_ref) => {
                if let Some(value) = class_attr_lookup(&class_ref, &name) {
                    return Ok(value);
                }
                if let Some(default) = default {
                    return Ok(default);
                }
                let class_name = match &*class_ref.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "type".to_string(),
                };
                Err(RuntimeError::attribute_error(format!(
                    "type object '{}' has no attribute '{}'",
                    class_name, name
                )))
            }
            Value::Instance(instance) => {
                let class_name =
                    class_name_for_instance(&instance).unwrap_or_else(|| "object".to_string());
                if let Object::Instance(instance_data) = &*instance.kind() {
                    if let Some(value) = instance_data.attrs.get(&name).cloned() {
                        return Ok(value);
                    }
                    if let Some(value) = class_attr_lookup(&instance_data.class, &name) {
                        return Ok(value);
                    }
                }
                if let Some(default) = default {
                    return Ok(default);
                }
                Err(RuntimeError::attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    class_name, name
                )))
            }
            Value::Module(module) => {
                if let Object::Module(module_data) = &*module.kind()
                    && let Some(value) = module_data.globals.get(&name).cloned()
                {
                    return Ok(value);
                }
                if let Some(default) = default {
                    return Ok(default);
                }
                Err(RuntimeError::attribute_error(format!(
                    "module has no attribute '{}'",
                    name
                )))
            }
            other => {
                if let Some(class_ref) = self.class_of_value(&other)
                    && let Some(value) = class_attr_lookup(&class_ref, &name)
                {
                    return Ok(value);
                }
                if name == "__call__" && self.is_callable_value(&other) {
                    // CPython inspect.getattr_static() exposes a non-None descriptor
                    // for callable instances/functions even when callability is slot-based.
                    return Ok(Value::Builtin(BuiltinFunction::TypeCall));
                }
                if let Some(default) = default {
                    return Ok(default);
                }
                Err(RuntimeError::attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    self.value_type_name_for_error(&other),
                    name
                )))
            }
        }
    }

    pub(super) fn builtin_inspect_static_getmro(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_static_getmro() expects one argument"));
        }
        let class = args
            .first()
            .cloned()
            .ok_or_else(|| RuntimeError::new("_static_getmro() expects one argument"))?;
        let values = match class {
            Value::Class(class_ref) => self
                .class_mro_entries(&class_ref)
                .into_iter()
                .map(Value::Class)
                .collect::<Vec<_>>(),
            Value::Builtin(builtin) => {
                let mut out = vec![Value::Builtin(builtin)];
                if let Some(Value::Class(object_class)) = self.builtins.get("object") {
                    out.push(Value::Class(object_class.clone()));
                }
                out
            }
            Value::ExceptionType(name) => vec![Value::ExceptionType(name)],
            _ => {
                return Err(RuntimeError::new(
                    "_static_getmro() expects a class-like argument",
                ));
            }
        };
        Ok(self.heap.alloc_tuple(values))
    }

    pub(super) fn builtin_inspect_get_dunder_dict_of_class(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_get_dunder_dict_of_class() expects one argument",
            ));
        }
        match args.first().cloned() {
            Some(Value::Class(class_value)) => self
                .builtin_getattr(
                    vec![
                        Value::Class(class_value),
                        Value::Str("__dict__".to_string()),
                    ],
                    HashMap::new(),
                )
                .map_err(|err| {
                    if runtime_error_matches_exception(&err, "AttributeError") {
                        RuntimeError::new(
                            "_get_dunder_dict_of_class() expects a class-like argument",
                        )
                    } else {
                        err
                    }
                }),
            Some(Value::Builtin(builtin)) => self
                .builtin_getattr(
                    vec![Value::Builtin(builtin), Value::Str("__dict__".to_string())],
                    HashMap::new(),
                )
                .map_err(|err| {
                    if runtime_error_matches_exception(&err, "AttributeError") {
                        RuntimeError::new(
                            "_get_dunder_dict_of_class() expects a class-like argument",
                        )
                    } else {
                        err
                    }
                }),
            Some(Value::ExceptionType(name)) => Ok(self.heap.alloc_dict(vec![
                (Value::Str("__name__".to_string()), Value::Str(name.clone())),
                (
                    Value::Str("__module__".to_string()),
                    Value::Str("builtins".to_string()),
                ),
            ])),
            _ => Err(RuntimeError::new(
                "_get_dunder_dict_of_class() expects a class-like argument",
            )),
        }
    }

    fn simple_namespace_not_implemented(&self) -> Value {
        self.builtins
            .get("NotImplemented")
            .cloned()
            .unwrap_or(Value::None)
    }

    fn is_simple_namespace_instance(&self, instance: &ObjRef) -> bool {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return false;
        };
        self.class_mro_entries(&instance_data.class)
            .iter()
            .any(|class_ref| {
                matches!(&*class_ref.kind(), Object::Class(class_data) if class_data.name == "SimpleNamespace")
            })
    }

    fn simple_namespace_ordered_kwargs(
        &self,
        mut kwargs: HashMap<String, Value>,
        kwargs_order: Option<Vec<String>>,
    ) -> Vec<(String, Value)> {
        let mut ordered = Vec::with_capacity(kwargs.len());
        if let Some(order) = kwargs_order {
            for name in order {
                if let Some(value) = kwargs.remove(&name) {
                    ordered.push((name, value));
                }
            }
        }
        ordered.extend(kwargs);
        ordered
    }

    fn simple_namespace_dict(&mut self, instance: &ObjRef) -> Result<ObjRef, RuntimeError> {
        let dict_value = self.builtin_getattr(
            vec![
                Value::Instance(instance.clone()),
                Value::Str("__dict__".to_string()),
            ],
            HashMap::new(),
        )?;
        match dict_value {
            Value::Dict(dict_obj) => Ok(dict_obj),
            _ => Err(RuntimeError::type_error(
                "SimpleNamespace.__dict__ is not a dict",
            )),
        }
    }

    fn simple_namespace_assign_attr(
        &mut self,
        instance: &ObjRef,
        key: String,
        value: Value,
    ) -> Result<(), RuntimeError> {
        self.builtin_setattr(
            vec![Value::Instance(instance.clone()), Value::Str(key), value],
            HashMap::new(),
        )?;
        Ok(())
    }

    fn simple_namespace_assign_key_value(
        &mut self,
        instance: &ObjRef,
        key: Value,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let Value::Str(key) = key else {
            return Err(RuntimeError::type_error(
                "SimpleNamespace() keyword names must be strings",
            ));
        };
        self.simple_namespace_assign_attr(instance, key, value)
    }

    fn simple_namespace_apply_source(
        &mut self,
        instance: &ObjRef,
        source: Value,
    ) -> Result<(), RuntimeError> {
        match source.clone() {
            Value::Dict(dict_obj) => {
                let Object::Dict(entries) = &*dict_obj.kind() else {
                    return Err(RuntimeError::type_error(
                        "SimpleNamespace() source mapping is invalid",
                    ));
                };
                for (key, value) in entries.iter() {
                    self.simple_namespace_assign_key_value(instance, key.clone(), value.clone())?;
                }
                return Ok(());
            }
            _ => {}
        }

        match self.builtin_getattr(
            vec![source.clone(), Value::Str("keys".to_string())],
            HashMap::new(),
        ) {
            Ok(keys_method) => {
                let keys_iterable =
                    match self.call_internal(keys_method, Vec::new(), HashMap::new())? {
                        InternalCallOutcome::Value(value) => value,
                        InternalCallOutcome::CallerExceptionHandled => {
                            return Err(self.runtime_error_from_active_exception(
                                "SimpleNamespace() keys() failed",
                            ));
                        }
                    };
                for key in self.collect_iterable_values(keys_iterable)? {
                    let mapped_value = self.builtin_operator_getitem(
                        vec![source.clone(), key.clone()],
                        HashMap::new(),
                    )?;
                    self.simple_namespace_assign_key_value(instance, key, mapped_value)?;
                }
                Ok(())
            }
            Err(err) => {
                if !is_missing_attribute_error(&err) {
                    return Err(err);
                }
                for pair in self.collect_iterable_values(source)? {
                    let values = self.collect_iterable_values(pair)?;
                    if values.len() != 2 {
                        return Err(RuntimeError::value_error(format!(
                            "dictionary update sequence element has length {}; 2 is required",
                            values.len()
                        )));
                    }
                    let mut values_iter = values.into_iter();
                    let key = values_iter.next().expect("len checked");
                    let value = values_iter.next().expect("len checked");
                    self.simple_namespace_assign_key_value(instance, key, value)?;
                }
                Ok(())
            }
        }
    }

    fn simple_namespace_entries(
        &mut self,
        instance: &ObjRef,
    ) -> Result<Vec<(String, Value)>, RuntimeError> {
        let dict = self.simple_namespace_dict(instance)?;
        let Object::Dict(entries) = &*dict.kind() else {
            return Err(RuntimeError::type_error(
                "SimpleNamespace.__dict__ is not a dict",
            ));
        };
        let mut out = Vec::with_capacity(entries.len());
        for (key, value) in entries.iter() {
            if let Value::Str(name) = key {
                out.push((name.clone(), value.clone()));
            }
        }
        Ok(out)
    }

    fn simple_namespace_repr_inner(
        &mut self,
        instance: &ObjRef,
        seen: &mut Vec<u64>,
    ) -> Result<String, RuntimeError> {
        if seen.contains(&instance.id()) {
            return Ok("namespace(...)".to_string());
        }
        seen.push(instance.id());
        let entries = self.simple_namespace_entries(instance)?;
        let mut rendered = Vec::with_capacity(entries.len());
        for (name, value) in entries {
            let value_repr = if let Value::Instance(nested) = &value {
                if self.is_simple_namespace_instance(nested) {
                    self.simple_namespace_repr_inner(nested, seen)?
                } else {
                    self.render_value_repr_for_display(value)?
                }
            } else {
                self.render_value_repr_for_display(value)?
            };
            rendered.push(format!("{name}={value_repr}"));
        }
        seen.pop();
        Ok(format!("namespace({})", rendered.join(", ")))
    }

    pub(super) fn builtin_types_simplenamespace_init_with_order(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        kwargs_order: Option<Vec<String>>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::type_error(
                "SimpleNamespace.__init__() takes from 1 to 2 positional arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "SimpleNamespace.__init__")?;
        if let Some(source) = args.pop() {
            self.simple_namespace_apply_source(&instance, source)?;
        }
        for (key, value) in self.simple_namespace_ordered_kwargs(kwargs, kwargs_order) {
            self.simple_namespace_assign_attr(&instance, key, value)?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_types_simplenamespace_init(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_types_simplenamespace_init_with_order(args, kwargs, None)
    }

    pub(super) fn builtin_types_simplenamespace_repr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "SimpleNamespace.__repr__() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "SimpleNamespace.__repr__")?;
        let mut seen = Vec::new();
        let repr = self.simple_namespace_repr_inner(&instance, &mut seen)?;
        Ok(Value::Str(repr))
    }

    pub(super) fn builtin_types_simplenamespace_eq(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::type_error(
                "SimpleNamespace.__eq__() expects one argument",
            ));
        }
        let left = self.take_bound_instance_arg(&mut args, "SimpleNamespace.__eq__")?;
        let right = args.remove(0);
        let Value::Instance(right_instance) = right else {
            return Ok(self.simple_namespace_not_implemented());
        };
        if !self.is_simple_namespace_instance(&left)
            || !self.is_simple_namespace_instance(&right_instance)
        {
            return Ok(self.simple_namespace_not_implemented());
        }
        let left_dict = self.simple_namespace_dict(&left)?;
        let right_dict = self.simple_namespace_dict(&right_instance)?;
        let equals = self.compare_eq_runtime(Value::Dict(left_dict), Value::Dict(right_dict))?;
        Ok(Value::Bool(self.truthy_from_value(&equals)?))
    }

    pub(super) fn builtin_types_simplenamespace_reduce(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::type_error(
                "SimpleNamespace.__reduce__() expects no arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "SimpleNamespace.__reduce__")?;
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "SimpleNamespace.__reduce__() receiver must be instance",
                ));
            }
        };
        let mut state_entries = Vec::new();
        for (name, value) in self.simple_namespace_entries(&instance)? {
            state_entries.push((Value::Str(name), value));
        }
        Ok(self.heap.alloc_tuple(vec![
            Value::Class(class),
            self.heap.alloc_tuple(Vec::new()),
            self.heap.alloc_dict(state_entries),
        ]))
    }

    pub(super) fn builtin_types_simplenamespace_replace(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::type_error(
                "SimpleNamespace.__replace__() expects no positional arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "SimpleNamespace.__replace__")?;
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "SimpleNamespace.__replace__() receiver must be instance",
                ));
            }
        };
        let replaced = self.alloc_instance_for_class(&class);
        for (name, value) in self.simple_namespace_entries(&instance)? {
            self.simple_namespace_assign_attr(&replaced, name, value)?;
        }
        for (name, value) in kwargs {
            self.simple_namespace_assign_attr(&replaced, name, value)?;
        }
        Ok(Value::Instance(replaced))
    }

    pub(super) fn builtin_types_moduletype(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let kw_name = kwargs.remove("name");
        let kw_doc = kwargs.remove("doc");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "ModuleType() got an unexpected keyword argument",
            ));
        }

        if args.is_empty() && kw_name.is_none() {
            return Err(RuntimeError::new(
                "ModuleType() expects at least one argument",
            ));
        }

        let mut init_target: Option<Value> = None;
        if let Some(first) = args.first() {
            match first {
                Value::Module(_) | Value::Instance(_) => {
                    init_target = Some(args.remove(0));
                }
                _ => {}
            }
        }

        let name_value = if let Some(value) = kw_name {
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("module name must be string"));
        };
        let name = match name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("module name must be string")),
        };

        let doc_value = if let Some(value) = kw_doc {
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };
        if !args.is_empty() {
            return Err(RuntimeError::new("ModuleType() takes at most 3 arguments"));
        }

        if let Some(target) = init_target {
            match target {
                Value::Module(module_obj) => {
                    if let Object::Module(module_data) = &mut *module_obj.kind_mut() {
                        module_data
                            .globals
                            .insert("__name__".to_string(), Value::Str(name));
                        module_data
                            .globals
                            .insert("__doc__".to_string(), doc_value.clone());
                        module_data
                            .globals
                            .insert("__package__".to_string(), Value::None);
                        module_data
                            .globals
                            .insert("__loader__".to_string(), Value::None);
                        module_data
                            .globals
                            .insert("__spec__".to_string(), Value::None);
                    }
                    Ok(Value::None)
                }
                Value::Instance(instance_obj) => {
                    if let Object::Instance(instance_data) = &mut *instance_obj.kind_mut() {
                        instance_data
                            .attrs
                            .insert("__name__".to_string(), Value::Str(name));
                        instance_data.attrs.insert("__doc__".to_string(), doc_value);
                        instance_data
                            .attrs
                            .insert("__package__".to_string(), Value::None);
                        instance_data
                            .attrs
                            .insert("__loader__".to_string(), Value::None);
                        instance_data
                            .attrs
                            .insert("__spec__".to_string(), Value::None);
                    }
                    Ok(Value::None)
                }
                _ => Err(RuntimeError::new("module name must be string")),
            }
        } else {
            let module = self.alloc_module(name);
            if let Value::Module(module_obj) = &module
                && let Object::Module(module_data) = &mut *module_obj.kind_mut()
            {
                module_data
                    .globals
                    .insert("__name__".to_string(), Value::Str(module_data.name.clone()));
                module_data.globals.insert("__doc__".to_string(), doc_value);
                module_data
                    .globals
                    .insert("__package__".to_string(), Value::None);
                module_data
                    .globals
                    .insert("__loader__".to_string(), Value::None);
                module_data
                    .globals
                    .insert("__spec__".to_string(), Value::None);
            }
            Ok(module)
        }
    }

    fn value_supports_mapping_protocol(&self, value: &Value) -> bool {
        match value {
            Value::Dict(_) => true,
            Value::List(_) | Value::Tuple(_) => false,
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    if self.instance_backing_dict(instance).is_some() {
                        return true;
                    }
                    if self.instance_backing_list(instance).is_some()
                        || self.instance_backing_tuple(instance).is_some()
                        || self.class_has_builtin_list_base(&instance_data.class)
                        || self.class_has_builtin_tuple_base(&instance_data.class)
                    {
                        return false;
                    }
                    class_attr_lookup(&instance_data.class, "__getitem__").is_some()
                }
                _ => false,
            },
            _ => false,
        }
    }

    pub(super) fn builtin_types_mappingproxy(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("MappingProxyType() expects one argument"));
        }
        let mapping = if args.len() == 1 {
            args.remove(0)
        } else {
            args.remove(0);
            args.remove(0)
        };
        if !self.value_supports_mapping_protocol(&mapping) {
            return Err(RuntimeError::type_error(format!(
                "mappingproxy() argument must be a mapping, not {}",
                self.value_type_name_for_error(&mapping)
            )));
        }
        let class = self
            .mappingproxy_type_class
            .clone()
            .or_else(|| self.types_module_class("__pyrs_mappingproxy_type__"))
            .unwrap_or_else(|| self.alloc_synthetic_class("mappingproxy"));
        let instance = self.alloc_instance_for_class(&class);
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert(MAPPING_PROXY_STORAGE_ATTR.to_string(), mapping);
        }
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_types_functiontype(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 3 || args.len() > 7 {
            return Err(RuntimeError::type_error(
                "function() takes from 3 to 7 positional arguments",
            ));
        }
        let class_arg = args.remove(0);
        if !matches!(class_arg, Value::Class(_)) {
            return Err(RuntimeError::type_error(
                "descriptor '__new__' requires a type object",
            ));
        }
        let code = match args.remove(0) {
            Value::Code(code) => code,
            other => {
                return Err(RuntimeError::type_error(format!(
                    "function() argument 'code' must be code, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        let (globals_dict, globals_mapping) = match args.remove(0) {
            Value::Dict(dict) => (dict, None),
            Value::Instance(instance) => {
                if let Some(dict) = self.instance_backing_dict(&instance) {
                    (dict, Some(Value::Instance(instance)))
                } else {
                    return Err(RuntimeError::type_error(format!(
                        "function() argument 'globals' must be dict, not {}",
                        self.value_type_name_for_error(&Value::Instance(instance))
                    )));
                }
            }
            other => {
                return Err(RuntimeError::type_error(format!(
                    "function() argument 'globals' must be dict, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };

        let mut name_arg = args.first().cloned();
        if !args.is_empty() {
            args.remove(0);
        }
        let mut defaults_arg = args.first().cloned();
        if !args.is_empty() {
            args.remove(0);
        }
        let mut closure_arg = args.first().cloned();
        if !args.is_empty() {
            args.remove(0);
        }
        let mut kwdefaults_arg = args.first().cloned();
        if !args.is_empty() {
            args.remove(0);
        }
        if !args.is_empty() {
            return Err(RuntimeError::type_error(
                "function() takes from 3 to 7 positional arguments",
            ));
        }

        let mut take_kw = |key: &str, target: &mut Option<Value>| -> Result<(), RuntimeError> {
            if let Some(value) = kwargs.remove(key) {
                if target.is_some() {
                    return Err(RuntimeError::type_error(format!(
                        "argument for function() given by name ('{}') and position",
                        key
                    )));
                }
                *target = Some(value);
            }
            Ok(())
        };
        take_kw("name", &mut name_arg)?;
        take_kw("argdefs", &mut defaults_arg)?;
        take_kw("closure", &mut closure_arg)?;
        take_kw("kwdefaults", &mut kwdefaults_arg)?;
        if !kwargs.is_empty() {
            let mut keys = kwargs.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let key = keys.first().cloned().unwrap_or_default();
            return Err(RuntimeError::type_error(format!(
                "function() got an unexpected keyword argument '{}'",
                key
            )));
        }

        let code_name = match name_arg {
            None | Some(Value::None) => code.name.clone(),
            Some(Value::Str(name)) => name,
            Some(_) => {
                return Err(RuntimeError::type_error(
                    "arg 3 (name) must be None or string",
                ));
            }
        };
        let defaults = match defaults_arg {
            None | Some(Value::None) => Vec::new(),
            Some(Value::Tuple(defaults)) => match &*defaults.kind() {
                Object::Tuple(values) => values.to_vec(),
                _ => Vec::new(),
            },
            Some(_) => {
                return Err(RuntimeError::type_error(
                    "arg 4 (defaults) must be None or tuple",
                ));
            }
        };
        let closure_values = match closure_arg {
            None => {
                if !code.freevars.is_empty() {
                    return Err(RuntimeError::type_error("arg 5 (closure) must be tuple"));
                }
                Vec::new()
            }
            Some(Value::None) => {
                if !code.freevars.is_empty() {
                    return Err(RuntimeError::type_error("arg 5 (closure) must be tuple"));
                }
                Vec::new()
            }
            Some(Value::Tuple(closure)) => {
                let Object::Tuple(values) = &*closure.kind() else {
                    return Err(RuntimeError::type_error("arg 5 (closure) must be tuple"));
                };
                let mut cells = Vec::with_capacity(values.len());
                for value in values {
                    match value {
                        Value::Cell(cell) => cells.push(cell.clone()),
                        other => {
                            return Err(RuntimeError::type_error(format!(
                                "arg 5 (closure) expected cell, found {}",
                                self.value_type_name_for_error(other)
                            )));
                        }
                    }
                }
                cells
            }
            Some(_) => return Err(RuntimeError::type_error("arg 5 (closure) must be tuple")),
        };
        if closure_values.len() != code.freevars.len() {
            return Err(RuntimeError::value_error(format!(
                "{} requires closure of length {}, not {}",
                code_name,
                code.freevars.len(),
                closure_values.len()
            )));
        }
        let kwonly_defaults = match kwdefaults_arg {
            None | Some(Value::None) => HashMap::new(),
            Some(Value::Dict(dict)) => {
                let Object::Dict(entries) = &*dict.kind() else {
                    return Err(RuntimeError::type_error(
                        "arg 6 (kwdefaults) must be None or dict",
                    ));
                };
                let mut mapped = HashMap::new();
                for (key, value) in entries.iter() {
                    if let Value::Str(name) = key {
                        mapped.insert(name.clone(), value.clone());
                    }
                }
                mapped
            }
            Some(Value::Instance(instance)) => {
                let Some(dict) = self.instance_backing_dict(&instance) else {
                    return Err(RuntimeError::type_error(
                        "arg 6 (kwdefaults) must be None or dict",
                    ));
                };
                let Object::Dict(entries) = &*dict.kind() else {
                    return Err(RuntimeError::type_error(
                        "arg 6 (kwdefaults) must be None or dict",
                    ));
                };
                let mut mapped = HashMap::new();
                for (key, value) in entries.iter() {
                    if let Value::Str(name) = key {
                        mapped.insert(name.clone(), value.clone());
                    }
                }
                mapped
            }
            Some(_) => {
                return Err(RuntimeError::type_error(
                    "arg 6 (kwdefaults) must be None or dict",
                ));
            }
        };

        let Object::Dict(global_entries) = &*globals_dict.kind() else {
            return Err(RuntimeError::type_error(
                "function() argument 'globals' must be dict",
            ));
        };
        let module_name = global_entries
            .iter()
            .find_map(|(key, value)| match (key, value) {
                (Value::Str(name), Value::Str(value)) if name == "__name__" => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "__main__".to_string());
        let module = match self
            .heap
            .alloc_module(ModuleObject::new(module_name.clone()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            for (key, value) in global_entries.iter() {
                if let Value::Str(name) = key {
                    module_data.globals.insert(name.clone(), value.clone());
                }
            }
            module_data
                .globals
                .entry("__name__".to_string())
                .or_insert_with(|| Value::Str(module_name));
            if !module_data.globals.contains_key("__builtins__")
                && let Some(builtins) = self.modules.get("builtins")
            {
                module_data
                    .globals
                    .insert("__builtins__".to_string(), Value::Module(builtins.clone()));
            }
            if let Some(mapping) = globals_mapping {
                module_data
                    .globals
                    .insert(Self::FUNCTION_GLOBALS_MAPPING_KEY.to_string(), mapping);
            }
        }

        let code = if code_name == code.name {
            code
        } else {
            let mut overridden = (*code).clone();
            overridden.name = code_name;
            std::rc::Rc::new(overridden)
        };
        let function = FunctionObject::new(
            code,
            module,
            defaults,
            kwonly_defaults,
            closure_values,
            None,
            false,
        );
        Ok(self.heap.alloc_function(function))
    }

    pub(super) fn builtin_types_methodtype(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "MethodType() expects function and instance",
            ));
        }
        let function = args.remove(0);
        let instance = args.remove(0);
        let receiver = self.receiver_from_value(&instance)?;
        match function {
            Value::Function(func) => Ok(self
                .heap
                .alloc_bound_method(BoundMethod::new(func, receiver))),
            Value::BoundMethod(method) => Ok(Value::BoundMethod(method)),
            _ => Err(RuntimeError::new(
                "first argument must be a Python function",
            )),
        }
    }

    pub(super) fn builtin_types_coroutine(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("coroutine() expects one argument"));
        }
        let function = match args.remove(0) {
            Value::Function(function) => function,
            _ => return Err(RuntimeError::new("coroutine() expects a Python function")),
        };
        let mut wrapped = match &*function.kind() {
            Object::Function(function_data) => function_data.clone(),
            _ => unreachable!(),
        };
        let metadata_dict = match wrapped.dict.clone() {
            Some(existing) => existing,
            None => match self.heap.alloc_dict(Vec::new()) {
                Value::Dict(dict) => dict,
                _ => unreachable!(),
            },
        };
        dict_set_value_checked(
            &metadata_dict,
            Value::Str("__wrapped__".to_string()),
            Value::Function(function.clone()),
        )?;
        wrapped.dict = Some(metadata_dict);
        Ok(self.heap.alloc_function(wrapped))
    }

    pub(super) fn builtin_types_new_class(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 4 {
            return Err(RuntimeError::new(
                "new_class() expects name, optional bases, kwds, exec_body",
            ));
        }
        let name = match args.remove(0) {
            Value::Str(name) => Value::Str(name),
            _ => return Err(RuntimeError::new("new_class() name must be string")),
        };
        let bases = if args.is_empty() {
            self.heap.alloc_tuple(Vec::new())
        } else {
            args.remove(0)
        };
        if let Some(kwds) = args.first() {
            match kwds {
                Value::None => {}
                Value::Dict(_) => {}
                _ => return Err(RuntimeError::new("new_class() kwds must be dict or None")),
            }
        }
        let namespace = self.heap.alloc_dict(Vec::new());
        self.builtin_type(vec![name, bases, namespace], HashMap::new())
    }
}
