use super::*;

impl Vm {
    pub(super) fn builtin_operator_add(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| {
            add_values(left, right, &self.heap)
        })
    }

    pub(super) fn builtin_operator_sub(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| {
            sub_values(left, right, &self.heap)
        })
    }

    pub(super) fn builtin_operator_mul(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, |left, right| {
            mul_values(left, right, &self.heap)
        })
    }

    pub(super) fn builtin_operator_mod(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, mod_values)
    }

    pub(super) fn builtin_operator_truediv(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, div_values)
    }

    pub(super) fn builtin_operator_floordiv(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        binary_operator(args, kwargs, floor_div_values)
    }

    pub(super) fn builtin_operator_index(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("operator.index expects one argument"));
        }
        Ok(Value::Int(value_to_int(args[0].clone())?))
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
        Ok(Value::Bool(compare_in(&args[1], &args[0])?))
    }

    pub(super) fn builtin_operator_getitem(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("operator.getitem expects two arguments"));
        }
        self.getitem_value(args[0].clone(), args[1].clone())
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

        let mut out = Vec::new();
        let values = self.collect_iterable_values(iterable)?;
        let mut running = initial;
        if let Some(value) = running.clone() {
            out.push(value);
        }
        for value in values {
            match running.take() {
                None => {
                    running = Some(value.clone());
                    out.push(value);
                }
                Some(current) => {
                    let next = if let Some(callable) = func.clone() {
                        if matches!(callable, Value::None) {
                            add_values(current, value, &self.heap)?
                        } else {
                            match self.call_internal(
                                callable,
                                vec![current, value],
                                HashMap::new(),
                            )? {
                                InternalCallOutcome::Value(value) => value,
                                InternalCallOutcome::CallerExceptionHandled => {
                                    return Err(RuntimeError::new("accumulate() function raised"));
                                }
                            }
                        }
                    } else {
                        add_values(current, value, &self.heap)?
                    };
                    running = Some(next.clone());
                    out.push(next);
                }
            }
        }
        Ok(self.heap.alloc_list(out))
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
        let r = r as usize;
        if r > values.len() {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let mut out = Vec::new();
        let mut current = Vec::with_capacity(r);
        fn build_combinations(
            values: &[Value],
            start: usize,
            target_len: usize,
            current: &mut Vec<Value>,
            out: &mut Vec<Vec<Value>>,
        ) {
            if current.len() == target_len {
                out.push(current.clone());
                return;
            }
            for idx in start..values.len() {
                current.push(values[idx].clone());
                build_combinations(values, idx + 1, target_len, current, out);
                current.pop();
            }
        }
        build_combinations(&values, 0, r, &mut current, &mut out);
        Ok(self.heap.alloc_list(
            out.into_iter()
                .map(|row| self.heap.alloc_tuple(row))
                .collect(),
        ))
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
        let r = r as usize;
        if values.is_empty() && r > 0 {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let mut out = Vec::new();
        let mut current = Vec::with_capacity(r);
        fn build_combinations_replacement(
            values: &[Value],
            start: usize,
            target_len: usize,
            current: &mut Vec<Value>,
            out: &mut Vec<Vec<Value>>,
        ) {
            if current.len() == target_len {
                out.push(current.clone());
                return;
            }
            for idx in start..values.len() {
                current.push(values[idx].clone());
                build_combinations_replacement(values, idx, target_len, current, out);
                current.pop();
            }
        }
        build_combinations_replacement(&values, 0, r, &mut current, &mut out);
        Ok(self.heap.alloc_list(
            out.into_iter()
                .map(|row| self.heap.alloc_tuple(row))
                .collect(),
        ))
    }

    pub(super) fn builtin_itertools_compress(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("compress() expects data and selectors"));
        }
        let data = self.collect_iterable_values(args.remove(0))?;
        let selectors = self.collect_iterable_values(args.remove(0))?;
        let mut out = Vec::new();
        for (item, selector) in data.into_iter().zip(selectors) {
            if is_truthy(&selector) {
                out.push(item);
            }
        }
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(args.remove(0))?;
        let mut out = Vec::new();
        let mut dropping = true;
        for value in values {
            if dropping {
                let keep_dropping = match self.call_internal(
                    predicate.clone(),
                    vec![value.clone()],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(result) => is_truthy(&result),
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::new("dropwhile() predicate raised"));
                    }
                };
                if keep_dropping {
                    continue;
                }
                dropping = false;
            }
            out.push(value);
        }
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(args.remove(0))?;
        let mut out = Vec::new();
        for value in values {
            let passed = if matches!(predicate, Value::None) {
                is_truthy(&value)
            } else {
                match self.call_internal(predicate.clone(), vec![value.clone()], HashMap::new())? {
                    InternalCallOutcome::Value(result) => is_truthy(&result),
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::new("filterfalse() predicate raised"));
                    }
                }
            };
            if !passed {
                out.push(value);
            }
        }
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(iterable)?;
        if values.is_empty() {
            return Ok(self.heap.alloc_list(Vec::new()));
        }

        let key_of = |vm: &mut Vm,
                      item: Value,
                      key: Option<Value>|
         -> Result<Value, RuntimeError> {
            match key {
                None => Ok(item),
                Some(callable) if matches!(callable, Value::None) => Ok(item),
                Some(callable) => match vm.call_internal(callable, vec![item], HashMap::new())? {
                    InternalCallOutcome::Value(value) => Ok(value),
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(RuntimeError::new("groupby() key raised"))
                    }
                },
            }
        };

        let mut out = Vec::new();
        let mut current_group = Vec::new();
        let mut iter = values.into_iter();
        let first_value = iter.next().expect("checked not empty");
        let mut current_key = key_of(self, first_value.clone(), key_func.clone())?;
        current_group.push(first_value);
        for value in iter {
            let key = key_of(self, value.clone(), key_func.clone())?;
            if key == current_key {
                current_group.push(value);
            } else {
                out.push(
                    self.heap
                        .alloc_tuple(vec![current_key, self.heap.alloc_list(current_group)]),
                );
                current_key = key;
                current_group = vec![value];
            }
        }
        out.push(
            self.heap
                .alloc_tuple(vec![current_key, self.heap.alloc_list(current_group)]),
        );
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(iterable)?;

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
        if let Some(stop) = stop {
            if stop < 0 {
                return Err(RuntimeError::new("islice() stop must be non-negative"));
            }
        }
        if step <= 0 {
            return Err(RuntimeError::new("islice() step must be positive"));
        }

        let mut out = Vec::new();
        let stop_index = stop.unwrap_or(values.len() as i64).min(values.len() as i64) as usize;
        let mut idx = start as usize;
        while idx < stop_index && idx < values.len() {
            out.push(values[idx].clone());
            idx += step as usize;
        }
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(args.remove(0))?;
        if values.len() < 2 {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let mut out = Vec::with_capacity(values.len().saturating_sub(1));
        for idx in 0..values.len() - 1 {
            out.push(
                self.heap
                    .alloc_tuple(vec![values[idx].clone(), values[idx + 1].clone()]),
            );
        }
        Ok(self.heap.alloc_list(out))
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
        let rows = self.collect_iterable_values(args.remove(0))?;
        let mut out = Vec::new();
        for row in rows {
            let call_args = self.collect_iterable_values(row)?;
            let value = match self.call_internal(callable.clone(), call_args, HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("starmap() function raised"));
                }
            };
            out.push(value);
        }
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(args.remove(0))?;
        let mut out = Vec::new();
        for value in values {
            let keep =
                match self.call_internal(predicate.clone(), vec![value.clone()], HashMap::new())? {
                    InternalCallOutcome::Value(result) => is_truthy(&result),
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(RuntimeError::new("takewhile() predicate raised"));
                    }
                };
            if !keep {
                break;
            }
            out.push(value);
        }
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(iterable)?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            out.push(self.heap.alloc_list(values.clone()));
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
        if args.is_empty() {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let mut columns = Vec::with_capacity(args.len());
        for source in args {
            columns.push(self.collect_iterable_values(source)?);
        }
        let max_len = columns.iter().map(Vec::len).max().unwrap_or(0);
        let mut out = Vec::with_capacity(max_len);
        for idx in 0..max_len {
            let mut row = Vec::with_capacity(columns.len());
            for values in &columns {
                if idx < values.len() {
                    row.push(values[idx].clone());
                } else {
                    row.push(fillvalue.clone());
                }
            }
            out.push(self.heap.alloc_tuple(row));
        }
        Ok(self.heap.alloc_list(out))
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
        let mut out = Vec::new();
        for source in args {
            out.extend(self.collect_iterable_values(source)?);
        }
        Ok(self.heap.alloc_list(out))
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
        let values = self.collect_iterable_values(args[0].clone())?;
        Ok(self.heap.alloc_list(values))
    }

    pub(super) fn builtin_itertools_repeat(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("repeat() expects value and count"));
        }
        let count = value_to_int(args[1].clone())?;
        if count < 0 {
            return Err(RuntimeError::new("repeat count must be >= 0"));
        }
        let mut out = Vec::with_capacity(count as usize);
        for _ in 0..count {
            out.push(args[0].clone());
        }
        Ok(self.heap.alloc_list(out))
    }

    pub(super) fn builtin_itertools_batched(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "batched() expects iterable, n, optional strict",
            ));
        }
        let strict = if args.len() == 3 {
            is_truthy(&args[2])
        } else if let Some(value) = kwargs.remove("strict") {
            is_truthy(&value)
        } else {
            false
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "batched() got an unexpected keyword argument",
            ));
        }
        let iterable = args.remove(0);
        let n = value_to_int(args.remove(0))?;
        if n <= 0 {
            return Err(RuntimeError::new("n must be at least one"));
        }
        let values = self.collect_iterable_values(iterable)?;
        let mut out = Vec::new();
        let mut idx = 0usize;
        while idx < values.len() {
            let end = (idx + n as usize).min(values.len());
            if strict && end - idx < n as usize {
                return Err(RuntimeError::new("batched(): incomplete batch"));
            }
            out.push(self.heap.alloc_tuple(values[idx..end].to_vec()));
            idx = end;
        }
        Ok(self.heap.alloc_list(out))
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
            value_to_int(r)? as usize
        } else {
            values.len()
        };
        if r > values.len() {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let mut out: Vec<Value> = Vec::new();
        let mut used = vec![false; values.len()];
        let mut current: Vec<Value> = Vec::with_capacity(r);

        fn build_permutations(
            heap: &Heap,
            values: &[Value],
            used: &mut [bool],
            current: &mut Vec<Value>,
            out: &mut Vec<Value>,
            target_len: usize,
        ) {
            if current.len() == target_len {
                out.push(heap.alloc_tuple(current.clone()));
                return;
            }
            for idx in 0..values.len() {
                if used[idx] {
                    continue;
                }
                used[idx] = true;
                current.push(values[idx].clone());
                build_permutations(heap, values, used, current, out, target_len);
                current.pop();
                used[idx] = false;
            }
        }

        build_permutations(&self.heap, &values, &mut used, &mut current, &mut out, r);
        Ok(self.heap.alloc_list(out))
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

        let mut pools: Vec<Vec<Value>> = Vec::new();
        for arg in args {
            pools.push(self.collect_iterable_values(arg)?);
        }
        let base_pools = pools.clone();
        for _ in 1..repeat {
            pools.extend(base_pools.clone());
        }

        let mut out = Vec::new();
        let mut current = Vec::new();
        fn build_product(
            heap: &Heap,
            pools: &[Vec<Value>],
            depth: usize,
            current: &mut Vec<Value>,
            out: &mut Vec<Value>,
        ) {
            if depth == pools.len() {
                out.push(heap.alloc_tuple(current.clone()));
                return;
            }
            for value in &pools[depth] {
                current.push(value.clone());
                build_product(heap, pools, depth + 1, current, out);
                current.pop();
            }
        }
        if pools.is_empty() {
            out.push(self.heap.alloc_tuple(Vec::new()));
        } else {
            build_product(&self.heap, &pools, 0, &mut current, &mut out);
        }
        Ok(self.heap.alloc_list(out))
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
            return Ok(Value::Builtin(BuiltinFunction::TypingIdFunc));
        }
        Ok(args[1].clone())
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
        );
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
        let attr_name = match &func {
            Value::Function(func_ref) => match &*func_ref.kind() {
                Object::Function(func_data) => Some(func_data.code.name.clone()),
                _ => None,
            },
            _ => None,
        };
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
        instance.attrs.insert(
            "attrname".to_string(),
            attr_name.map(Value::Str).unwrap_or(Value::None),
        );
        instance.attrs.insert("__doc__".to_string(), Value::None);
        Ok(self.heap.alloc_instance(instance))
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
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("deque() expects at most one argument"));
        }
        if let Some(source) = args.into_iter().next() {
            let values = self.collect_iterable_values(source)?;
            Ok(self.heap.alloc_list(values))
        } else {
            Ok(self.heap.alloc_list(Vec::new()))
        }
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
            _ => return Err(RuntimeError::new("ChainMap.get() expected a ChainMap instance")),
        };
        let maps = {
            let instance_ref = instance.kind();
            let Object::Instance(instance_data) = &*instance_ref else {
                return Err(RuntimeError::new("ChainMap.get() expected a ChainMap instance"));
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
            return Err(RuntimeError::new("ChainMap.__getitem__() expects one key argument"));
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
        Err(RuntimeError::new("key not found"))
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
            return Err(RuntimeError::new("ChainMap.__setitem__() first map must be dict"));
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
            return Err(RuntimeError::new("ChainMap.__delitem__() expects one key argument"));
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
            return Err(RuntimeError::new("ChainMap.__delitem__() first map must be dict"));
        };
        if dict_remove_value(&dict, &key).is_none() {
            return Err(RuntimeError::new("key not found"));
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
                            if let Value::Tuple(tuple_obj) = item {
                                if let Object::Tuple(parts) = &*tuple_obj.kind() {
                                    if parts.len() == 2 {
                                        dict_set_value_checked(
                                            &dict,
                                            parts[0].clone(),
                                            parts[1].clone(),
                                        )?;
                                        continue;
                                    }
                                }
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
        let mapping = match &args[0] {
            Value::Dict(dict) => dict.clone(),
            _ => return Err(RuntimeError::new("_count_elements() mapping must be dict")),
        };
        for item in self.collect_iterable_values(args[1].clone())? {
            let current = dict_get_value(&mapping, &item).unwrap_or(Value::Int(0));
            let next = add_values(current, Value::Int(1), &self.heap)?;
            dict_set_value_checked(&mapping, item, next)?;
        }
        Ok(Value::None)
    }

    pub(super) fn inspect_module_for_value(&self, value: &Value) -> Option<Value> {
        match value {
            Value::Module(module) => Some(Value::Module(module.clone())),
            Value::Function(function) => match &*function.kind() {
                Object::Function(function_data) => Some(Value::Module(function_data.module.clone())),
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
            return Err(RuntimeError::new("getsourcefile() expects one object argument"));
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
        for name in [
            "follow_wrapped",
            "globals",
            "locals",
            "eval_str",
            "annotation_format",
        ] {
            kwargs.remove(name);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "signature() got an unexpected keyword argument",
            ));
        }

        let callable = args.remove(0);
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

        let mut params = Vec::new();
        let mut parts = Vec::new();
        let mut return_annotation = Value::None;

        let make_param =
            |name: String, kind: &str, default: Option<Value>| -> (String, (Value, Value)) {
                let rendered = match default.clone() {
                    Some(value) => format!("{name}={}", format_value(&value)),
                    None => name.clone(),
                };
                let entry = (
                    Value::Str(name),
                    self.heap.alloc_tuple(vec![
                        Value::Str(kind.to_string()),
                        default.unwrap_or(Value::None),
                    ]),
                );
                (rendered, entry)
            };

        match callable {
            Value::Function(func) => {
                let (
                    posonly_params,
                    positional_params,
                    vararg,
                    kwarg,
                    kwonly_params,
                    defaults,
                    kwonly_defaults,
                    annotations,
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
                    )
                };
                let posonly_len = posonly_params.len();
                let positional_len = posonly_len + positional_params.len();
                let default_start = positional_len.saturating_sub(defaults.len());

                for (idx, name) in posonly_params.iter().enumerate() {
                    let default = if idx >= default_start {
                        Some(defaults[idx - default_start].clone())
                    } else {
                        None
                    };
                    let (rendered, entry) = make_param(name.clone(), "POSITIONAL_ONLY", default);
                    parts.push(rendered);
                    params.push(entry);
                }
                if posonly_len > 0 {
                    parts.push("/".to_string());
                }

                for (idx, name) in positional_params.iter().enumerate() {
                    let param_idx = posonly_len + idx;
                    let default = if param_idx >= default_start {
                        Some(defaults[param_idx - default_start].clone())
                    } else {
                        None
                    };
                    let (rendered, entry) =
                        make_param(name.clone(), "POSITIONAL_OR_KEYWORD", default);
                    parts.push(rendered);
                    params.push(entry);
                }

                if let Some(vararg) = &vararg {
                    parts.push(format!("*{vararg}"));
                    params.push((
                        Value::Str(vararg.clone()),
                        self.heap.alloc_tuple(vec![
                            Value::Str("VAR_POSITIONAL".to_string()),
                            Value::None,
                        ]),
                    ));
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
                    parts.push(format!("**{kwarg}"));
                    params.push((
                        Value::Str(kwarg.clone()),
                        self.heap
                            .alloc_tuple(vec![Value::Str("VAR_KEYWORD".to_string()), Value::None]),
                    ));
                }

                if let Some(annotations) = &annotations {
                    if let Object::Dict(entries) = &*annotations.kind() {
                        if let Some((_, value)) = entries
                            .iter()
                            .find(|(key, _)| matches!(key, Value::Str(name) if name == "return"))
                        {
                            return_annotation = value.clone();
                        }
                    }
                }
            }
            _ => {
                parts.push("*args".to_string());
                params.push((
                    Value::Str("args".to_string()),
                    self.heap
                        .alloc_tuple(vec![Value::Str("VAR_POSITIONAL".to_string()), Value::None]),
                ));
                parts.push("**kwargs".to_string());
                params.push((
                    Value::Str("kwargs".to_string()),
                    self.heap
                        .alloc_tuple(vec![Value::Str("VAR_KEYWORD".to_string()), Value::None]),
                ));
            }
        }

        let signature_text = Value::Str(format!("({})", parts.join(", ")));
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

    pub(super) fn builtin_inspect_isfunction(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| {
            matches!(value, Value::Function(_) | Value::BoundMethod(_))
        })
    }

    pub(super) fn builtin_inspect_ismethod(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        unary_predicate(args, kwargs, |value| matches!(value, Value::BoundMethod(_)))
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
        unary_predicate(args, kwargs, |_value| false)
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
                return Err(RuntimeError::new("unwrap() got an unexpected keyword argument"));
            }
        } else if !kwargs.is_empty() {
            return Err(RuntimeError::new("unwrap() got an unexpected keyword argument"));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new("unwrap() expects one argument"));
        }
        Ok(args.remove(0))
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
            if let Value::Generator(generator) = value {
                if let Object::Generator(state) = &*generator.kind() {
                    return !state.is_coroutine && !state.is_async_generator;
                }
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
            if let Value::Generator(generator) = value {
                if let Object::Generator(state) = &*generator.kind() {
                    return state.is_coroutine;
                }
            }
            false
        })
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
            if let Value::Generator(generator) = value {
                if let Object::Generator(state) = &*generator.kind() {
                    return state.is_async_generator;
                }
            }
            false
        })
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
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_get_dunder_dict_of_class() expects one argument",
            ));
        }
        match args.first() {
            Some(Value::Class(class_ref)) => match &*class_ref.kind() {
                Object::Class(class_data) => Ok(self.heap.alloc_dict(
                    class_data
                        .attrs
                        .iter()
                        .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                        .collect::<Vec<_>>(),
                )),
                _ => Err(RuntimeError::new(
                    "_get_dunder_dict_of_class() expects a class-like argument",
                )),
            },
            Some(Value::Builtin(builtin)) => Ok(self
                .heap
                .alloc_dict(self.builtin_type_dict_entries(*builtin))),
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

    pub(super) fn builtin_types_moduletype(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("ModuleType() expects one argument"));
        }
        let name = match &args[0] {
            Value::Str(name) => name.clone(),
            _ => return Err(RuntimeError::new("module name must be string")),
        };
        Ok(self.alloc_module(name))
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
