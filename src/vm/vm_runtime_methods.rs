use super::{
    BigInt, BuiltinFunction, ClassObject, Frame, GeneratorResumeOutcome, HashMap, InstanceObject,
    InternalCallOutcome, IteratorKind, IteratorObject, ModuleObject, NativeMethodKind, ObjRef,
    Object, Ordering, RuntimeError, Value, Vm, builtin_exception_parent, class_attr_lookup,
    class_name_for_instance, ensure_hashable, format_repr, memoryview_bounds,
    memoryview_decode_element, memoryview_element_offset, memoryview_format_for_view,
    memoryview_layout_1d, memoryview_logical_nbytes, memoryview_shape_and_strides_from_parts,
    module_globals_version, runtime_error_matches_exception, slice_bounds_for_step_one,
    slice_indices, value_from_bigint, value_to_bytes_payload, value_to_int, with_bytes_like_source,
};
use crate::runtime::{DictViewKind, SliceValue};

impl Vm {
    fn warnings_increment_filters_version(module: &ObjRef) {
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            let next = match module_data.globals.get("_filters_version") {
                Some(Value::Int(value)) => value.saturating_add(1),
                _ => 1,
            };
            module_data
                .globals
                .insert("_filters_version".to_string(), Value::Int(next));
        }
    }

    pub(super) fn dict_get_value_runtime(
        &mut self,
        dict: &ObjRef,
        key: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some((_index, _matched_key, value, _hash)) =
            self.dict_lookup_entry_runtime(dict, key)?
        else {
            return Ok(None);
        };
        Ok(Some(value))
    }

    pub(super) fn dict_contains_key_checked_runtime(
        &mut self,
        dict: &ObjRef,
        key: &Value,
    ) -> Result<bool, RuntimeError> {
        Ok(self.dict_lookup_entry_runtime(dict, key)?.is_some())
    }

    pub(super) fn dict_set_value_checked_runtime(
        &mut self,
        dict: &ObjRef,
        key: Value,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let hash = self.hash_value_runtime(&key)? as u64;
        let readonly = {
            let dict_kind = dict.kind();
            match &*dict_kind {
                Object::Dict(entries) => entries.is_readonly(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "unsupported operand type for mapping assignment",
                    ));
                }
            }
        };
        if readonly {
            return Err(RuntimeError::type_error(
                "'mappingproxy' object does not support item assignment",
            ));
        }

        if let Some((index, matched_key, _matched_value, _)) =
            self.dict_lookup_entry_runtime(dict, &key)?
        {
            let mut dict_kind = dict.kind_mut();
            let Object::Dict(entries) = &mut *dict_kind else {
                return Err(RuntimeError::type_error(
                    "unsupported operand type for mapping assignment",
                ));
            };
            if let Some((stored_key, _)) = entries.entry_at(index)
                && stored_key == matched_key
            {
                entries.set_value_at(index, value);
                return Ok(());
            }
        }

        let mut dict_kind = dict.kind_mut();
        let Object::Dict(entries) = &mut *dict_kind else {
            return Err(RuntimeError::type_error(
                "unsupported operand type for mapping assignment",
            ));
        };
        entries.insert_with_hash(key, value, hash);
        Ok(())
    }

    pub(super) fn dict_remove_value_runtime(
        &mut self,
        dict: &ObjRef,
        key: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some((index, matched_key, _value, _hash)) =
            self.dict_lookup_entry_runtime(dict, key)?
        else {
            return Ok(None);
        };
        let mut dict_kind = dict.kind_mut();
        let Object::Dict(entries) = &mut *dict_kind else {
            return Ok(None);
        };
        let Some((stored_key, _)) = entries.entry_at(index) else {
            return Ok(None);
        };
        if stored_key != matched_key {
            return Ok(None);
        }
        let (_, removed_value) = entries.remove(index);
        Ok(Some(removed_value))
    }

    pub(super) fn sequence_contains_runtime_value(
        &mut self,
        haystack: &[Value],
        needle: &Value,
    ) -> Result<bool, RuntimeError> {
        for candidate in haystack {
            let equals = self.compare_eq_runtime(candidate.clone(), needle.clone())?;
            if self.truthy_from_value(&equals)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn dedup_hashable_values_runtime(
        &mut self,
        values: Vec<Value>,
    ) -> Result<Vec<Value>, RuntimeError> {
        let mut deduped = Vec::new();
        for value in values {
            ensure_hashable(&value)?;
            if !self.sequence_contains_runtime_value(&deduped, &value)? {
                deduped.push(value);
            }
        }
        Ok(deduped)
    }

    pub(super) fn set_contains_runtime(
        &mut self,
        set: &ObjRef,
        needle: &Value,
    ) -> Result<bool, RuntimeError> {
        let values = {
            let set_kind = set.kind();
            match &*set_kind {
                Object::Set(values) | Object::FrozenSet(values) => values.to_vec(),
                _ => return Err(RuntimeError::type_error("receiver must be set")),
            }
        };
        self.sequence_contains_runtime_value(&values, needle)
    }

    pub(super) fn set_insert_checked_runtime(
        &mut self,
        set: &ObjRef,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        ensure_hashable(&value)?;
        if self.set_contains_runtime(set, &value)? {
            return Ok(false);
        }
        let mut set_kind = set.kind_mut();
        let Object::Set(values) = &mut *set_kind else {
            return Err(RuntimeError::type_error("receiver must be set"));
        };
        values.push(value);
        Ok(true)
    }

    pub(super) fn set_remove_checked_runtime(
        &mut self,
        set: &ObjRef,
        target: &Value,
    ) -> Result<bool, RuntimeError> {
        ensure_hashable(target)?;
        let values_snapshot = {
            let set_kind = set.kind();
            let Object::Set(values) = &*set_kind else {
                return Err(RuntimeError::type_error("receiver must be set"));
            };
            values.to_vec()
        };
        let mut remove_index = None;
        for (index, candidate) in values_snapshot.into_iter().enumerate() {
            let equals = self.compare_eq_runtime(candidate, target.clone())?;
            if self.truthy_from_value(&equals)? {
                remove_index = Some(index);
                break;
            }
        }
        let Some(remove_index) = remove_index else {
            return Ok(false);
        };
        let mut set_kind = set.kind_mut();
        let Object::Set(values) = &mut *set_kind else {
            return Err(RuntimeError::type_error("receiver must be set"));
        };
        if remove_index >= values.len() {
            return Ok(false);
        }
        let _ = values.remove(remove_index);
        Ok(true)
    }

    fn dict_lookup_entry_runtime(
        &mut self,
        dict: &ObjRef,
        key: &Value,
    ) -> Result<Option<(usize, Value, Value, u64)>, RuntimeError> {
        let hash = self.hash_value_runtime(key)? as u64;
        let (candidates, allow_legacy_fallback, all_entries) = {
            let dict_kind = dict.kind();
            let Object::Dict(entries) = &*dict_kind else {
                return Ok(None);
            };
            let allow_legacy_fallback = entries.requires_legacy_hash_fallback();
            (
                entries.candidate_entries_with_hash(hash),
                allow_legacy_fallback,
                allow_legacy_fallback.then(|| entries.to_vec()),
            )
        };
        let mut tested_indices = Vec::with_capacity(candidates.len());
        for (index, candidate_key, candidate_value) in candidates {
            let equals = self.compare_eq_runtime(candidate_key.clone(), key.clone())?;
            if self.truthy_from_value(&equals)? {
                return Ok(Some((index, candidate_key, candidate_value, hash)));
            }
            tested_indices.push(index);
        }
        if !allow_legacy_fallback {
            return Ok(None);
        }
        let Some(all_entries) = all_entries else {
            return Ok(None);
        };
        for (index, (candidate_key, candidate_value)) in all_entries.into_iter().enumerate() {
            if tested_indices.contains(&index) {
                continue;
            }
            let equals = self.compare_eq_runtime(candidate_key.clone(), key.clone())?;
            if self.truthy_from_value(&equals)? {
                return Ok(Some((index, candidate_key, candidate_value, hash)));
            }
        }
        Ok(None)
    }

    pub(super) fn builtin_warnings_filters_mutated(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_filters_mutated() expects no arguments"));
        }
        let mut bumped = false;
        if let Some(target_module) = self.frames.last().and_then(|frame| {
            frame
                .locals
                .get("_wm")
                .cloned()
                .or_else(|| match &*frame.function_globals.kind() {
                    Object::Module(module_data) => module_data.globals.get("_wm").cloned(),
                    _ => None,
                })
                .and_then(|value| match value {
                    Value::Module(module) => Some(module),
                    _ => None,
                })
        }) {
            Self::warnings_increment_filters_version(&target_module);
            bumped = true;
        }
        if !bumped && let Some(module) = self.modules.get("warnings").cloned() {
            Self::warnings_increment_filters_version(&module);
            bumped = true;
        }
        if !bumped && let Some(module) = self.modules.get("_py_warnings").cloned() {
            Self::warnings_increment_filters_version(&module);
            bumped = true;
        }
        if !bumped && let Some(module) = self.modules.get("_warnings").cloned() {
            Self::warnings_increment_filters_version(&module);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_warnings_acquire_lock(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_acquire_lock() expects no arguments"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_warnings_release_lock(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_release_lock() expects no arguments"));
        }
        Ok(Value::None)
    }

    pub(super) fn thread_info_dict(&mut self, name: &str) -> Result<Value, RuntimeError> {
        let ident = self.current_thread_ident_value();
        let info = if let Some(existing) = self.thread_info_objects.get(&ident).cloned() {
            existing
        } else {
            let thread_class = self
                .modules
                .get("threading")
                .and_then(|module| match &*module.kind() {
                    Object::Module(module_data) => match module_data.globals.get("Thread") {
                        Some(Value::Class(class_ref)) => Some(class_ref.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .unwrap_or_else(|| {
                    match self
                        .heap
                        .alloc_class(ClassObject::new("Thread".to_string(), Vec::new()))
                    {
                        Value::Class(class_ref) => class_ref,
                        _ => unreachable!(),
                    }
                });
            let info = match self.heap.alloc_instance(InstanceObject::new(thread_class)) {
                Value::Instance(instance) => instance,
                _ => unreachable!(),
            };
            self.thread_info_objects.insert(ident, info.clone());
            info
        };
        if let Object::Instance(instance_data) = &mut *info.kind_mut() {
            instance_data
                .attrs
                .insert("name".to_string(), Value::Str(name.to_string()));
            instance_data
                .attrs
                .insert("ident".to_string(), Value::Int(ident));
            instance_data
                .attrs
                .insert("native_id".to_string(), Value::Int(ident));
            instance_data
                .attrs
                .insert("daemon".to_string(), Value::Bool(false));
        }
        Ok(Value::Instance(info))
    }

    pub(super) fn range_object_parts(&self, obj: &ObjRef) -> Option<(BigInt, BigInt, BigInt)> {
        let kind = obj.kind();
        let Object::Iterator(state) = &*kind else {
            return None;
        };
        let IteratorKind::RangeObject { start, stop, step } = &state.kind else {
            return None;
        };
        Some((start.clone(), stop.clone(), step.clone()))
    }

    pub(super) fn range_object_len_bigint(
        &self,
        start: &BigInt,
        stop: &BigInt,
        step: &BigInt,
    ) -> BigInt {
        let one = BigInt::one();
        if step.is_negative() {
            if start.cmp_total(stop) != Ordering::Greater {
                return BigInt::zero();
            }
            let distance = start.sub(stop);
            let numerator = distance.sub(&one);
            let step_abs = step.negated();
            let (q, _) = numerator
                .div_mod_floor(&step_abs)
                .expect("step is non-zero");
            q.add(&one)
        } else {
            if start.cmp_total(stop) != Ordering::Less {
                return BigInt::zero();
            }
            let distance = stop.sub(start);
            let numerator = distance.sub(&one);
            let (q, _) = numerator.div_mod_floor(step).expect("step is non-zero");
            q.add(&one)
        }
    }

    pub(super) fn range_object_index_value(
        &self,
        start: &BigInt,
        step: &BigInt,
        index: i64,
    ) -> Value {
        let offset = step.mul(&BigInt::from_i64(index));
        value_from_bigint(start.add(&offset))
    }

    pub(super) fn list_reverse_iterator(&self, list: ObjRef) -> Result<Value, RuntimeError> {
        let next_index = match &*list.kind() {
            Object::List(values) => {
                if values.is_empty() {
                    -1
                } else if values.len() - 1 > i64::MAX as usize {
                    return Err(RuntimeError::new("list reverse iterator index overflow"));
                } else {
                    (values.len() - 1) as i64
                }
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "list.__reversed__ receiver must be list",
                ));
            }
        };
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::ListReverse { list, next_index },
            index: 0,
        }))
    }

    pub(super) fn range_reverse_iterator(&self, range: &ObjRef) -> Result<Value, RuntimeError> {
        let Some((start, stop, step)) = self.range_object_parts(range) else {
            return Err(RuntimeError::type_error(
                "range.__reversed__ receiver must be range",
            ));
        };
        let len = self.range_object_len_bigint(&start, &stop, &step);
        let (current, reverse_stop, reverse_step) = if len.is_zero() {
            (start.clone(), start, BigInt::one())
        } else {
            let last_offset = len.sub(&BigInt::one());
            (
                start.add(&step.mul(&last_offset)),
                start.sub(&step),
                step.negated(),
            )
        };
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::Range {
                current,
                stop: reverse_stop,
                step: reverse_step,
            },
            index: 0,
        }))
    }

    pub(super) fn dict_view_iterator(
        &self,
        dict: ObjRef,
        kind: DictViewKind,
    ) -> Result<Value, RuntimeError> {
        if !matches!(&*dict.kind(), Object::Dict(_)) {
            return Err(RuntimeError::type_error("dict view iterator requires dict"));
        }
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::DictView { dict, kind },
            index: 0,
        }))
    }

    pub(super) fn dict_view_reverse_iterator(
        &self,
        dict: ObjRef,
        kind: DictViewKind,
    ) -> Result<Value, RuntimeError> {
        let next_index = match &*dict.kind() {
            Object::Dict(values) => {
                if values.is_empty() {
                    -1
                } else if values.len() - 1 > i64::MAX as usize {
                    return Err(RuntimeError::new("dict reverse iterator index overflow"));
                } else {
                    (values.len() - 1) as i64
                }
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "dict reverse iterator requires dict",
                ));
            }
        };
        Ok(self.heap.alloc_iterator(IteratorObject {
            kind: IteratorKind::DictReverse {
                dict,
                kind,
                next_index,
            },
            index: 0,
        }))
    }

    pub(super) fn getitem_value(
        &mut self,
        value: Value,
        index: Value,
    ) -> Result<Value, RuntimeError> {
        if self.trace_flags.getitem_entry {
            eprintln!(
                "[getitem-entry] value_type={} index_type={}",
                self.value_type_name_for_error(&value),
                self.value_type_name_for_error(&index)
            );
        }
        if let Value::Instance(instance) = &value {
            let receiver_value = Value::Instance(instance.clone());
            if let Some(values) = self.namedtuple_instance_values(instance) {
                return self.getitem_value(self.heap.alloc_tuple(values), index);
            }
            if let Some(backing_list) = self.instance_backing_list(instance) {
                if let Some(getitem) =
                    self.lookup_bound_special_method(&receiver_value, "__getitem__")?
                {
                    return match self.call_internal(getitem, vec![index], HashMap::new())? {
                        InternalCallOutcome::Value(value) => Ok(value),
                        InternalCallOutcome::CallerExceptionHandled => Err(
                            self.runtime_error_from_active_exception("subscript lookup failed"),
                        ),
                    };
                }
                return self.getitem_value(Value::List(backing_list), index);
            }
            if let Some(backing_tuple) = self.instance_backing_tuple(instance) {
                if let Some(getitem) =
                    self.lookup_bound_special_method(&receiver_value, "__getitem__")?
                {
                    return match self.call_internal(getitem, vec![index], HashMap::new())? {
                        InternalCallOutcome::Value(value) => Ok(value),
                        InternalCallOutcome::CallerExceptionHandled => Err(
                            self.runtime_error_from_active_exception("subscript lookup failed"),
                        ),
                    };
                }
                return self.getitem_value(Value::Tuple(backing_tuple), index);
            }
            if let Some(backing_str) = self.instance_backing_str(instance) {
                if let Some(getitem) =
                    self.lookup_bound_special_method(&receiver_value, "__getitem__")?
                {
                    return match self.call_internal(getitem, vec![index], HashMap::new())? {
                        InternalCallOutcome::Value(value) => Ok(value),
                        InternalCallOutcome::CallerExceptionHandled => Err(
                            self.runtime_error_from_active_exception("subscript lookup failed"),
                        ),
                    };
                }
                return self.getitem_value(Value::Str(backing_str), index);
            }
            if let Some(backing_dict) = self.instance_backing_dict(instance) {
                let is_exact_builtin_dict = match &*instance.kind() {
                    Object::Instance(instance_data) => match &*instance_data.class.kind() {
                        Object::Class(class_data) => class_data.name == "dict",
                        _ => false,
                    },
                    _ => false,
                };
                if is_exact_builtin_dict {
                    return self.getitem_value(Value::Dict(backing_dict), index);
                }
                if let Some(getitem) =
                    self.lookup_bound_special_method(&receiver_value, "__getitem__")?
                {
                    return match self.call_internal(getitem, vec![index], HashMap::new())? {
                        InternalCallOutcome::Value(value) => Ok(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            Err(self.runtime_error_from_active_exception("subscript lookup failed"))
                        }
                    };
                }
                if let Some(value) = self.dict_get_value_runtime(&backing_dict, &index)? {
                    return Ok(value);
                }
                if let Some(missing) =
                    self.lookup_bound_special_method(&receiver_value, "__missing__")?
                {
                    return match self.call_internal(missing, vec![index], HashMap::new())? {
                        InternalCallOutcome::Value(value) => Ok(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            Err(self.runtime_error_from_active_exception("__missing__() failed"))
                        }
                    };
                }
                return Err(RuntimeError::key_error("key not found"));
            }
        }
        match index {
            Value::Slice(slice) => {
                let lower = slice.lower;
                let upper = slice.upper;
                let step = slice.step;
                match value {
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => {
                            let indices = slice_indices(values.len(), lower, upper, step)?;
                            let mut result = Vec::with_capacity(indices.len());
                            for idx in indices {
                                result.push(values[idx].clone());
                            }
                            Ok(self.heap.alloc_list(result))
                        }
                        _ => Err(RuntimeError::type_error("subscript unsupported type")),
                    },
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => {
                            let indices = slice_indices(values.len(), lower, upper, step)?;
                            let mut result = Vec::with_capacity(indices.len());
                            for idx in indices {
                                result.push(values[idx].clone());
                            }
                            Ok(self.heap.alloc_tuple(result))
                        }
                        _ => Err(RuntimeError::type_error("subscript unsupported type")),
                    },
                    Value::Str(value) => {
                        let chars: Vec<char> = value.chars().collect();
                        let indices = slice_indices(chars.len(), lower, upper, step)?;
                        let mut result = String::new();
                        for idx in indices {
                            result.push(chars[idx]);
                        }
                        Ok(Value::Str(result))
                    }
                    Value::Bytes(obj) => match &*obj.kind() {
                        Object::Bytes(values) => {
                            let indices = slice_indices(values.len(), lower, upper, step)?;
                            let mut result = Vec::with_capacity(indices.len());
                            for idx in indices {
                                result.push(values[idx]);
                            }
                            Ok(self.heap.alloc_bytes(result))
                        }
                        _ => Err(RuntimeError::type_error("subscript unsupported type")),
                    },
                    Value::ByteArray(obj) => match &*obj.kind() {
                        Object::ByteArray(values) => {
                            let indices = slice_indices(values.len(), lower, upper, step)?;
                            let mut result = Vec::with_capacity(indices.len());
                            for idx in indices {
                                result.push(values[idx]);
                            }
                            Ok(self.heap.alloc_bytearray(result))
                        }
                        _ => Err(RuntimeError::type_error("subscript unsupported type")),
                    },
                    Value::MemoryView(obj) => match &*obj.kind() {
                        Object::MemoryView(view) => {
                            with_bytes_like_source(&view.source, |values| {
                                let step_value = step.unwrap_or(1);
                                if let Some((shape, strides)) = memoryview_shape_and_strides_from_parts(
                                    view.start,
                                    view.length,
                                    view.shape.as_ref(),
                                    view.strides.as_ref(),
                                    view.itemsize,
                                    values.len(),
                                ) && shape.len() > 1
                                {
                                    if shape.len() != strides.len() {
                                        return Err(RuntimeError::type_error("subscript unsupported type"));
                                    }
                                    let rows = usize::try_from(shape[0])
                                        .map_err(|_| RuntimeError::type_error("subscript unsupported type"))?;
                                    let indices = slice_indices(rows, lower, upper, step)?;
                                    let origin = isize::try_from(view.start)
                                        .map_err(|_| RuntimeError::type_error("subscript unsupported type"))?;
                                    let base_stride = strides[0];
                                    let new_start = if let Some(first) = indices.first() {
                                        let delta = base_stride
                                            .checked_mul(*first as isize)
                                            .ok_or_else(|| RuntimeError::type_error("subscript unsupported type"))?;
                                        origin
                                            .checked_add(delta)
                                            .ok_or_else(|| RuntimeError::type_error("subscript unsupported type"))?
                                    } else {
                                        origin
                                    };
                                    if new_start < 0 {
                                        return Err(RuntimeError::type_error("subscript unsupported type"));
                                    }
                                    let mut new_shape = shape.clone();
                                    new_shape[0] = indices.len() as isize;
                                    let mut new_strides = strides.clone();
                                    new_strides[0] = base_stride
                                        .checked_mul(step_value as isize)
                                        .ok_or_else(|| RuntimeError::type_error("subscript unsupported type"))?;
                                    let byte_len = memoryview_logical_nbytes(&new_shape, view.itemsize)
                                        .ok_or_else(|| RuntimeError::type_error("subscript unsupported type"))?;
                                    let sliced = self.heap.alloc_memoryview_with(
                                        view.source.clone(),
                                        view.itemsize,
                                        view.format.clone(),
                                    );
                                    if let Value::MemoryView(sliced_obj) = &sliced
                                        && let Object::MemoryView(sliced_view) =
                                            &mut *sliced_obj.kind_mut()
                                    {
                                        sliced_view.start = new_start as usize;
                                        sliced_view.length = Some(byte_len);
                                        sliced_view.shape = Some(new_shape);
                                        sliced_view.strides = Some(new_strides);
                                        sliced_view.contiguous = view.contiguous && step_value == 1;
                                        sliced_view.export_owner = view.export_owner.clone();
                                        sliced_view.released = view.released;
                                    }
                                    return Ok(sliced);
                                }
                                if let Some((origin, logical_len, stride, itemsize)) =
                                    memoryview_layout_1d(view, values.len())
                                {
                                    if step_value == 1 && stride == itemsize as isize {
                                        let (start_idx, stop_idx) =
                                            slice_bounds_for_step_one(logical_len, lower, upper);
                                        let start_delta =
                                            stride.checked_mul(start_idx as isize).ok_or_else(
                                                || RuntimeError::type_error("subscript unsupported type"),
                                            )?;
                                        let new_start =
                                            origin.checked_add(start_delta).ok_or_else(|| {
                                                RuntimeError::type_error("subscript unsupported type")
                                            })?;
                                        if new_start < 0 {
                                            return Err(RuntimeError::new(
                                                "subscript unsupported type",
                                            ));
                                        }
                                        let sliced = self.heap.alloc_memoryview_with(
                                            view.source.clone(),
                                            view.itemsize,
                                            view.format.clone(),
                                        );
                                        if let Value::MemoryView(sliced_obj) = &sliced
                                            && let Object::MemoryView(sliced_view) =
                                                &mut *sliced_obj.kind_mut()
                                        {
                                            sliced_view.start = new_start as usize;
                                            sliced_view.length = Some(
                                                stop_idx
                                                    .saturating_sub(start_idx)
                                                    .saturating_mul(itemsize),
                                            );
                                            sliced_view.shape = None;
                                            sliced_view.strides = None;
                                            sliced_view.contiguous = view.contiguous;
                                            sliced_view.export_owner = view.export_owner.clone();
                                            sliced_view.released = view.released;
                                        }
                                        Ok(sliced)
                                    } else {
                                        let indices =
                                            slice_indices(logical_len, lower, upper, step)?;
                                        let element_count = indices.len();
                                        let (new_start, new_stride) = if let Some(first) =
                                            indices.first()
                                        {
                                            let first_delta = stride
                                                .checked_mul(*first as isize)
                                                .ok_or_else(|| {
                                                RuntimeError::type_error("subscript unsupported type")
                                            })?;
                                            let start = origin
                                                .checked_add(first_delta)
                                                .ok_or_else(|| {
                                                    RuntimeError::type_error("subscript unsupported type")
                                                })?;
                                            let stride_scaled = stride
                                                .checked_mul(step_value as isize)
                                                .ok_or_else(|| {
                                                    RuntimeError::type_error("subscript unsupported type")
                                                })?;
                                            (start, stride_scaled)
                                        } else {
                                            (origin, stride)
                                        };
                                        if new_start < 0 {
                                            return Err(RuntimeError::new(
                                                "subscript unsupported type",
                                            ));
                                        }
                                        let sliced = self.heap.alloc_memoryview_with(
                                            view.source.clone(),
                                            view.itemsize,
                                            view.format.clone(),
                                        );
                                        if let Value::MemoryView(sliced_obj) = &sliced
                                            && let Object::MemoryView(sliced_view) =
                                                &mut *sliced_obj.kind_mut()
                                        {
                                            sliced_view.start = new_start as usize;
                                            sliced_view.length =
                                                Some(element_count.saturating_mul(itemsize));
                                            sliced_view.shape = Some(vec![element_count as isize]);
                                            sliced_view.strides = Some(vec![new_stride]);
                                            sliced_view.contiguous = false;
                                            sliced_view.export_owner = view.export_owner.clone();
                                            sliced_view.released = view.released;
                                        }
                                        Ok(sliced)
                                    }
                                } else {
                                    let (view_start, view_end) =
                                        memoryview_bounds(view.start, view.length, values.len());
                                    let view_len = view_end.saturating_sub(view_start);
                                    let indices = slice_indices(view_len, lower, upper, step)?;
                                    let mut result = Vec::with_capacity(indices.len());
                                    for idx in indices {
                                        result.push(values[view_start + idx]);
                                    }
                                    let source = match self.heap.alloc_bytes(result) {
                                        Value::Bytes(obj) => obj,
                                        _ => unreachable!(),
                                    };
                                    let sliced = self.heap.alloc_memoryview_with(
                                        source,
                                        view.itemsize,
                                        view.format.clone(),
                                    );
                                    if let Value::MemoryView(sliced_obj) = &sliced
                                        && let Object::MemoryView(sliced_view) =
                                            &mut *sliced_obj.kind_mut()
                                    {
                                        sliced_view.contiguous = false;
                                        sliced_view.export_owner = view.export_owner.clone();
                                        sliced_view.released = view.released;
                                        sliced_view.start = 0;
                                        sliced_view.length = None;
                                    }
                                    Ok(sliced)
                                }
                            })
                            .unwrap_or_else(|| Err(RuntimeError::type_error("subscript unsupported type")))
                        }
                        _ => Err(RuntimeError::type_error("subscript unsupported type")),
                    },
                    Value::Iterator(obj) => {
                        let Some((start, stop, step_value)) = self.range_object_parts(&obj) else {
                            return Err(RuntimeError::type_error("subscript unsupported type"));
                        };
                        let length = self.range_object_len_bigint(&start, &stop, &step_value);
                        let Some(length_i64) = length.to_i64() else {
                            return Err(RuntimeError::new("range too large for slicing"));
                        };
                        if length_i64 < 0 {
                            return Err(RuntimeError::new("range too large for slicing"));
                        }
                        let indices = slice_indices(length_i64 as usize, lower, upper, step)?;
                        let mut out = Vec::with_capacity(indices.len());
                        for idx in indices {
                            out.push(self.range_object_index_value(
                                &start,
                                &step_value,
                                idx as i64,
                            ));
                        }
                        Ok(self.heap.alloc_list(out))
                    }
                    Value::Dict(_) => Err(RuntimeError::new("slicing unsupported for dict")),
                    other => {
                        if let Some(proxy_result) = self.cpython_proxy_get_item(
                            &other,
                            Value::Slice(Box::new(SliceValue::new(lower, upper, step))),
                        ) {
                            return proxy_result;
                        }
                        if let Some(getitem) =
                            self.lookup_bound_special_method(&other, "__getitem__")?
                        {
                            match self.call_internal(
                                getitem,
                                vec![Value::Slice(Box::new(SliceValue::new(lower, upper, step)))],
                                HashMap::new(),
                            )? {
                                InternalCallOutcome::Value(value) => Ok(value),
                                InternalCallOutcome::CallerExceptionHandled => Err(self
                                    .runtime_error_from_active_exception(
                                        "subscript lookup failed",
                                    )),
                            }
                        } else {
                            Err(RuntimeError::type_error("subscript unsupported type"))
                        }
                    }
                }
            }
            index => match value {
                Value::List(obj) => match &*obj.kind() {
                    Object::List(values) => {
                        let mut index_int = match value_to_int(index.clone()) {
                            Ok(index) => index as isize,
                            Err(err) => {
                                if self.trace_flags.getitem_index {
                                    eprintln!(
                                        "[getitem-index] list index conversion failed container={} index={}",
                                        format_repr(&Value::List(obj.clone())),
                                        format_repr(&index)
                                    );
                                }
                                return Err(err);
                            }
                        };
                        if index_int < 0 {
                            index_int += values.len() as isize;
                        }
                        if index_int < 0 || index_int as usize >= values.len() {
                            return Err(RuntimeError::index_error("list index out of range"));
                        }
                        Ok(values[index_int as usize].clone())
                    }
                    _ => Err(RuntimeError::type_error("subscript unsupported type")),
                },
                Value::Tuple(obj) => match &*obj.kind() {
                    Object::Tuple(values) => {
                        if tuple_is_typing_alias_shape(values)
                            && typing_alias_index_shape(&index)
                        {
                            // Typing-only tuple aliases (e.g. unions used as TypeAlias bases)
                            // are subscripted in NumPy/typing modules for metadata construction.
                            // Preserve them as symbolic runtime objects instead of treating as
                            // concrete tuple indexing.
                            return Ok(Value::Tuple(obj.clone()));
                        }
                        let mut index_int = match value_to_int(index.clone()) {
                            Ok(index) => index as isize,
                            Err(err) => {
                                if self.trace_flags.getitem_index {
                                    eprintln!(
                                        "[getitem-index] tuple index conversion failed container={} index={}",
                                        format_repr(&Value::Tuple(obj.clone())),
                                        format_repr(&index)
                                    );
                                }
                                return Err(err);
                            }
                        };
                        if index_int < 0 {
                            index_int += values.len() as isize;
                        }
                        if index_int < 0 || index_int as usize >= values.len() {
                            return Err(RuntimeError::new("tuple index out of range"));
                        }
                        Ok(values[index_int as usize].clone())
                    }
                    _ => Err(RuntimeError::type_error("subscript unsupported type")),
                },
                Value::Str(value) => {
                    let mut index_int = match value_to_int(index.clone()) {
                        Ok(index) => index as isize,
                        Err(err) => {
                            if self.trace_flags.getitem_index {
                                eprintln!(
                                    "[getitem-index] str index conversion failed container={} index={}",
                                    format_repr(&Value::Str(value.clone())),
                                    format_repr(&index)
                                );
                            }
                            return Err(err);
                        }
                    };
                    let chars: Vec<char> = value.chars().collect();
                        if index_int < 0 {
                            index_int += chars.len() as isize;
                        }
                        if index_int < 0 || index_int as usize >= chars.len() {
                            return Err(RuntimeError::index_error("string index out of range"));
                        }
                    Ok(Value::Str(chars[index_int as usize].to_string()))
                }
                Value::Dict(obj) => {
                    let existing = self.dict_get_value_runtime(&obj, &index)?;
                    if let Some(value) = existing {
                        return Ok(value);
                    }
                    let default_factory = self.defaultdict_factories.get(&obj.id()).cloned();
                    if let Some(default_factory) = default_factory {
                        if matches!(default_factory, Value::None) {
                            return Err(RuntimeError::key_error("key not found"));
                        }
                        let generated = match self.call_internal(
                            default_factory,
                            Vec::new(),
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => value,
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(RuntimeError::new("default factory raised"));
                            }
                        };
                        self.dict_set_value_checked_runtime(&obj, index, generated.clone())?;
                        Ok(generated)
                    } else {
                        Err(RuntimeError::key_error("key not found"))
                    }
                }
                Value::Module(obj) => {
                    let module_kind = obj.kind();
                    let Object::Module(module_data) = &*module_kind else {
                        return Err(RuntimeError::type_error("subscript unsupported type"));
                    };
                    if module_data.name == "__module_spec__" {
                        let key = match index {
                            Value::Str(name) => name,
                            _ => return Err(RuntimeError::type_error("subscript unsupported type")),
                        };
                        return module_data
                            .globals
                            .get(&key)
                            .cloned()
                            .ok_or_else(|| RuntimeError::key_error("key not found"));
                    }
                    if module_data.name == "__re_match__" {
                        let getitem = self
                            .alloc_native_bound_method(NativeMethodKind::ReMatchGroup, obj.clone());
                        return match self.call_internal(getitem, vec![index], HashMap::new())? {
                            InternalCallOutcome::Value(value) => Ok(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                Err(self
                                    .runtime_error_from_active_exception("subscript lookup failed"))
                            }
                        };
                    }
                    Err(RuntimeError::type_error("subscript unsupported type"))
                }
                Value::Bytes(obj) => match &*obj.kind() {
                    Object::Bytes(values) => {
                        let mut index_int = value_to_int(index)? as isize;
                        if index_int < 0 {
                            index_int += values.len() as isize;
                        }
                        if index_int < 0 || index_int as usize >= values.len() {
                            return Err(RuntimeError::index_error("index out of range"));
                        }
                        Ok(Value::Int(values[index_int as usize] as i64))
                    }
                    _ => Err(RuntimeError::type_error("subscript unsupported type")),
                },
                Value::ByteArray(obj) => match &*obj.kind() {
                    Object::ByteArray(values) => {
                        let mut index_int = value_to_int(index)? as isize;
                        if index_int < 0 {
                            index_int += values.len() as isize;
                        }
                        if index_int < 0 || index_int as usize >= values.len() {
                            return Err(RuntimeError::index_error("index out of range"));
                        }
                        Ok(Value::Int(values[index_int as usize] as i64))
                    }
                    _ => Err(RuntimeError::type_error("subscript unsupported type")),
                },
                Value::MemoryView(obj) => match &*obj.kind() {
                    Object::MemoryView(view) => with_bytes_like_source(&view.source, |values| {
                        if view.shape.as_ref().is_some_and(|shape| shape.len() > 1) {
                            if value_to_int(index.clone()).is_err() {
                                return Err(RuntimeError::new(
                                    "TypeError: memoryview: invalid slice key",
                                ));
                            }
                            return Err(RuntimeError::new(
                                "NotImplementedError: multi-dimensional sub-views are not implemented",
                            ));
                        }
                        let itemsize = view.itemsize.max(1);
                        let format =
                            memoryview_format_for_view(itemsize, view.format.as_deref())?;
                        if let Some((origin, logical_len, stride, _itemsize)) =
                            memoryview_layout_1d(view, values.len())
                        {
                            let index_int = value_to_int(index)? as isize;
                            let offset =
                                memoryview_element_offset(origin, logical_len, stride, index_int)
                                    .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            let end = offset
                                .checked_add(itemsize)
                                .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            let chunk = values
                                .get(offset..end)
                                .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            memoryview_decode_element(chunk, format, itemsize, &self.heap)
                        } else {
                            let (start, end) =
                                memoryview_bounds(view.start, view.length, values.len());
                            let span_len = end.saturating_sub(start);
                            if span_len % itemsize != 0 {
                                return Err(RuntimeError::new(
                                    "memoryview length is not a multiple of itemsize",
                                ));
                            }
                            let logical_len = span_len / itemsize;
                            let mut index_int = value_to_int(index)? as isize;
                            if index_int < 0 {
                                index_int += logical_len as isize;
                            }
                            if index_int < 0 || index_int as usize >= logical_len {
                                return Err(RuntimeError::index_error("index out of range"));
                            }
                            let offset = start + (index_int as usize).saturating_mul(itemsize);
                            let end = offset
                                .checked_add(itemsize)
                                .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            let chunk = values
                                .get(offset..end)
                                .ok_or_else(|| RuntimeError::index_error("index out of range"))?;
                            memoryview_decode_element(chunk, format, itemsize, &self.heap)
                        }
                    })
                    .unwrap_or_else(|| Err(RuntimeError::type_error("subscript unsupported type"))),
                    _ => Err(RuntimeError::type_error("subscript unsupported type")),
                },
                Value::Iterator(obj) => {
                    let Some((start, stop, step_value)) = self.range_object_parts(&obj) else {
                        return Err(RuntimeError::type_error("subscript unsupported type"));
                    };
                    let length = self.range_object_len_bigint(&start, &stop, &step_value);
                    let Some(length_i64) = length.to_i64() else {
                        return Err(RuntimeError::new("range too large for indexing"));
                    };
                    let mut index_int = value_to_int(index)?;
                    if index_int < 0 {
                        index_int += length_i64;
                    }
                    if index_int < 0 || index_int >= length_i64 {
                        return Err(RuntimeError::new("range index out of range"));
                    }
                    Ok(self.range_object_index_value(&start, &step_value, index_int))
                }
                Value::Builtin(builtin)
                    if matches!(
                        builtin,
                        BuiltinFunction::Type
                            | BuiltinFunction::Bool
                            | BuiltinFunction::Int
                            | BuiltinFunction::Float
                            | BuiltinFunction::Str
                            | BuiltinFunction::List
                            | BuiltinFunction::Tuple
                            | BuiltinFunction::Dict
                            | BuiltinFunction::Set
                            | BuiltinFunction::FrozenSet
                            | BuiltinFunction::Bytes
                            | BuiltinFunction::ByteArray
                            | BuiltinFunction::MemoryView
                            | BuiltinFunction::Complex
                    ) =>
                {
                    Ok(self.alloc_generic_alias_instance(Value::Builtin(builtin), index))
                }
                Value::Class(class) => {
                    if self.is_union_type_class(&class) || self.is_typing_union_class(&class) {
                        let members = self.subscript_items_from_index(index);
                        return self
                            .build_union_value_from_members_with_forward_lenient(members, true);
                    }
                    let class_value = Value::Class(class.clone());
                    let class_getitem = class_attr_lookup(&class, "__class_getitem__")
                        .or_else(|| self.load_cpython_proxy_attr(&class, "__class_getitem__"));
                    if let Some(class_getitem_attr) = class_getitem {
                        let class_getitem = self
                            .bind_descriptor_method(class_getitem_attr.clone(), &class_value)?
                            .unwrap_or(class_getitem_attr);
                        return match self.call_internal(
                            class_getitem,
                            vec![index.clone()],
                            HashMap::new(),
                        )?
                        {
                            InternalCallOutcome::Value(value) => {
                                if value == class_value
                                    && typing_alias_index_shape(&index)
                                    && Self::cpython_proxy_raw_ptr_from_value(&class_value)
                                        .is_some()
                                {
                                    Ok(self.alloc_generic_alias_instance(
                                        class_value.clone(),
                                        index.clone(),
                                    ))
                                } else {
                                    Ok(value)
                                }
                            }
                            InternalCallOutcome::CallerExceptionHandled => Err(self
                                .runtime_error_from_active_exception("subscript lookup failed")),
                        };
                    }
                    if let Some(meta_getitem) =
                        self.lookup_bound_special_method(&class_value, "__getitem__")?
                    {
                        return match self.call_internal(meta_getitem, vec![index], HashMap::new())?
                        {
                            InternalCallOutcome::Value(value) => Ok(value),
                            InternalCallOutcome::CallerExceptionHandled => Err(self
                                .runtime_error_from_active_exception("subscript lookup failed")),
                        };
                    }
                    let inherits_builtin_generic_alias_base = self.class_has_builtin_tuple_base(&class)
                        || self.class_has_builtin_list_base(&class)
                        || self.class_has_builtin_dict_base(&class)
                        || self.class_has_builtin_set_base(&class)
                        || self.class_has_builtin_frozenset_base(&class);
                    if inherits_builtin_generic_alias_base {
                        return Ok(self.alloc_generic_alias_instance(class_value, index));
                    }
                    let (has_type_params, legacy_typing_generic_base, class_parameters) =
                        match &*class.kind() {
                        Object::Class(class_data) => {
                            let class_parameters = match class_data.attrs.get("__parameters__") {
                                Some(Value::Tuple(items_obj)) => {
                                    match &*items_obj.kind() {
                                        Object::Tuple(items) => Some(items.clone()),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            }
                            .or_else(|| match class_data.attrs.get("__type_params__") {
                                Some(Value::Tuple(items_obj)) => {
                                    match &*items_obj.kind() {
                                        Object::Tuple(items) => Some(items.clone()),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            });
                            let has_type_params = match class_data.attrs.get("__type_params__") {
                                Some(Value::Tuple(items_obj)) => {
                                    matches!(&*items_obj.kind(), Object::Tuple(items) if !items.is_empty())
                                }
                                _ => false,
                            };
                            let legacy_typing_generic_base = class_data.mro.iter().any(|entry| {
                                let Object::Class(entry_data) = &*entry.kind() else {
                                    return false;
                                };
                                let module_name = match entry_data.attrs.get("__module__") {
                                    Some(Value::Str(name)) => Some(name.as_str()),
                                    _ => None,
                                };
                                matches!(
                                    (module_name, entry_data.name.as_str()),
                                    (Some("_typing" | "typing"), "Generic" | "Protocol")
                                )
                            });
                            (
                                has_type_params,
                                legacy_typing_generic_base,
                                class_parameters,
                            )
                        }
                        _ => (false, false, None),
                    };
                    if has_type_params || legacy_typing_generic_base {
                        if let Some(parameters) = class_parameters
                            && !parameters.is_empty()
                        {
                            let template = self.alloc_generic_alias_instance(
                                class_value.clone(),
                                self.heap.alloc_tuple(parameters),
                            );
                            return self.subscript_generic_alias_value(template, index);
                        }
                        return Ok(self.alloc_generic_alias_instance(class_value, index));
                    }
                    Err(RuntimeError::type_error("subscript unsupported type"))
                }
                other => {
                    if self.is_type_alias_type_instance(&other) {
                        return self.subscript_type_alias_instance(other, index);
                    }
                    if self.union_args_from_value(&other).is_some()
                        && typing_alias_index_shape(&index)
                    {
                        return self.subscript_union_value(other, index);
                    }
                    if self.is_exact_types_generic_alias_value(&other)
                        && typing_alias_index_shape(&index)
                    {
                        return self.subscript_generic_alias_value(other, index);
                    }
                    if self.is_type_parameter_value(&other) && typing_alias_index_shape(&index) {
                        // Re-subscripting raw type-parameter markers should preserve the marker.
                        return Ok(other);
                    }
                    if let Some(proxy_result) =
                        self.cpython_proxy_get_item(&other, index.clone())
                    {
                        return proxy_result;
                    }
                    if let Some(getitem) =
                        self.lookup_bound_special_method(&other, "__getitem__")?
                    {
                        match self.call_internal(getitem, vec![index], HashMap::new())? {
                            InternalCallOutcome::Value(value) => Ok(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                Err(self
                                    .runtime_error_from_active_exception("subscript lookup failed"))
                            }
                        }
                    } else {
                        if self.trace_flags.getitem_unsupported {
                            eprintln!(
                                "[getitem-unsupported] value={} index={}",
                                format_repr(&other),
                                format_repr(&index)
                            );
                        }
                        Err(RuntimeError::type_error("subscript unsupported type"))
                    }
                }
            },
        }
    }

    pub(super) fn generic_alias_class(&self) -> Option<ObjRef> {
        let module = self.modules.get("types")?;
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        match module_data.globals.get("GenericAlias") {
            Some(Value::Class(class)) => Some(class.clone()),
            _ => None,
        }
    }

    fn typing_generic_alias_class(&self) -> Option<ObjRef> {
        let module = self.modules.get("typing")?;
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        match module_data.globals.get("_GenericAlias") {
            Some(Value::Class(class)) => Some(class.clone()),
            _ => None,
        }
    }

    fn ensure_typing_generic_alias_class(&mut self) -> Option<ObjRef> {
        if let Some(class) = self.typing_generic_alias_class() {
            return Some(class);
        }
        if self.import_module("typing").is_ok() {
            return self.typing_generic_alias_class();
        }
        None
    }

    fn origin_prefers_typing_generic_alias(&self, origin: &Value) -> bool {
        let Value::Class(class) = origin else {
            return false;
        };
        let Object::Class(class_data) = &*class.kind() else {
            return false;
        };
        matches!(
            class_data.attrs.get("__type_params__"),
            Some(Value::Tuple(_))
        )
    }

    pub(super) fn ensure_generic_alias_class(&mut self) -> ObjRef {
        const CACHE_KEY: &str = "__types_generic_alias__";
        if let Some(existing) = self.generic_alias_class() {
            self.synthetic_builtin_classes
                .insert(CACHE_KEY.to_string(), existing.clone());
            return existing;
        }
        if let Some(existing) = self.synthetic_builtin_classes.get(CACHE_KEY).cloned() {
            return existing;
        }
        let class = self.synthetic_builtin_class("GenericAlias");
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("types".to_string()));
            class_data.attrs.insert(
                "__name__".to_string(),
                Value::Str("GenericAlias".to_string()),
            );
            class_data.attrs.insert(
                "__qualname__".to_string(),
                Value::Str("GenericAlias".to_string()),
            );
            class_data.attrs.insert(
                "__getitem__".to_string(),
                Value::Builtin(BuiltinFunction::OperatorGetItem),
            );
        }
        self.synthetic_builtin_classes
            .insert(CACHE_KEY.to_string(), class.clone());
        class
    }

    pub(super) fn alloc_generic_alias_instance(&mut self, origin: Value, index: Value) -> Value {
        let alias_class = if self.origin_prefers_typing_generic_alias(&origin) {
            self.ensure_typing_generic_alias_class()
                .unwrap_or_else(|| self.ensure_generic_alias_class())
        } else {
            self.ensure_generic_alias_class()
        };
        let origin_module = self
            .optional_getattr_value(origin.clone(), "__module__")
            .ok()
            .flatten();
        let alias = self.heap.alloc_instance(InstanceObject::new(alias_class));
        if let Value::Instance(instance) = &alias
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            let typing_generic_alias_instance = {
                let class_kind = instance_data.class.kind();
                matches!(
                    &*class_kind,
                    Object::Class(class_data)
                        if class_data.name.contains("GenericAlias")
                            && matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module)) if module == "typing"
                            )
                )
            };
            instance_data
                .attrs
                .insert("__origin__".to_string(), origin.clone());
            let args = match index {
                Value::Tuple(tuple_obj) => Value::Tuple(tuple_obj),
                value => self.heap.alloc_tuple(vec![value]),
            };
            let mut parameters = Vec::new();
            if let Some(items) = Self::tuple_items_from_value(&args) {
                for item in &items {
                    self.collect_union_type_parameters_from_value(item, &mut parameters);
                }
            }
            instance_data.attrs.insert("__args__".to_string(), args);
            instance_data.attrs.insert(
                "__parameters__".to_string(),
                self.heap.alloc_tuple(parameters),
            );
            if let Some(flags) = self
                .optional_getattr_value(origin.clone(), "__flags__")
                .ok()
                .flatten()
            {
                instance_data.attrs.insert("__flags__".to_string(), flags);
            }
            instance_data
                .attrs
                .insert("__unpacked__".to_string(), Value::Bool(false));
            if typing_generic_alias_instance {
                instance_data
                    .attrs
                    .entry("_name".to_string())
                    .or_insert(Value::None);
                instance_data
                    .attrs
                    .entry("_inst".to_string())
                    .or_insert(Value::Bool(true));
                let should_set_origin_module = match instance_data.attrs.get("_name") {
                    None | Some(Value::None) => true,
                    Some(Value::Str(name)) => name.is_empty(),
                    _ => false,
                };
                if should_set_origin_module && let Some(module_name) = origin_module.clone() {
                    instance_data
                        .attrs
                        .insert("__module__".to_string(), module_name);
                }
            }
        }
        alias
    }

    fn tuple_items_from_value(value: &Value) -> Option<Vec<Value>> {
        match value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(items) => Some(items.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn list_items_from_value(value: &Value) -> Option<Vec<Value>> {
        match value {
            Value::List(list_obj) => match &*list_obj.kind() {
                Object::List(items) => Some(items.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn class_name_and_module(class: &ObjRef) -> Option<(String, Option<String>)> {
        let class_kind = class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return None;
        };
        let module_name = match class_data.attrs.get("__module__") {
            Some(Value::Str(name)) => Some(name.clone()),
            _ => None,
        };
        Some((class_data.name.clone(), module_name))
    }

    fn class_mro_has_name(class: &ObjRef, needle: &str) -> bool {
        let class_kind = class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return false;
        };
        if class_data.name == needle {
            return true;
        }
        class_data.mro.iter().any(|entry| {
            let entry_kind = entry.kind();
            matches!(&*entry_kind, Object::Class(entry_data) if entry_data.name == needle)
        })
    }

    pub(super) fn is_type_parameter_value(&self, value: &Value) -> bool {
        let Value::Instance(instance) = value else {
            return false;
        };
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return false;
        };
        let class_kind = instance_data.class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return false;
        };
        if !matches!(
            class_data.name.as_str(),
            "TypeVar" | "TypeVarTuple" | "ParamSpec"
        ) {
            return false;
        }
        matches!(
            class_data.attrs.get("__module__"),
            Some(Value::Str(module)) if matches!(module.as_str(), "typing" | "_typing")
        )
    }

    fn is_type_alias_type_instance(&self, value: &Value) -> bool {
        let Value::Instance(instance) = value else {
            return false;
        };
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return false;
        };
        let class_kind = instance_data.class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return false;
        };
        if class_data.name != "TypeAliasType" {
            return false;
        }
        matches!(
            class_data.attrs.get("__module__"),
            Some(Value::Str(module)) if module == "typing" || module == "_typing"
        )
    }

    fn subscript_type_alias_instance(
        &mut self,
        alias: Value,
        index: Value,
    ) -> Result<Value, RuntimeError> {
        let mut alias_instance = self.alloc_generic_alias_instance(alias.clone(), index);
        let type_params = self.optional_getattr_value(alias, "__type_params__")?;
        let args_items = if let Value::Instance(instance) = &alias_instance {
            let instance_kind = instance.kind();
            match &*instance_kind {
                Object::Instance(instance_data) => instance_data
                    .attrs
                    .get("__args__")
                    .and_then(Self::tuple_items_from_value)
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let mut parameters = Vec::new();
        for arg in args_items {
            if self
                .optional_getattr_value(arg.clone(), "__typing_subst__")?
                .is_some()
            {
                parameters.push(arg);
            }
        }
        if let Value::Instance(instance) = &mut alias_instance
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data.attrs.insert(
                "__parameters__".to_string(),
                self.heap.alloc_tuple(parameters),
            );
            if let Some(type_params) = type_params {
                instance_data
                    .attrs
                    .insert("__type_params__".to_string(), type_params);
            }
        }
        Ok(alias_instance)
    }

    fn union_type_class(&self) -> Option<ObjRef> {
        let module = self.modules.get("types")?;
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        match module_data.globals.get("UnionType") {
            Some(Value::Class(class)) => Some(class.clone()),
            _ => None,
        }
    }

    fn normalize_union_type_class_identity(class: &ObjRef) {
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.name = "Union".to_string();
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("typing".to_string()));
            class_data
                .attrs
                .insert("__name__".to_string(), Value::Str("Union".to_string()));
            class_data
                .attrs
                .insert("__qualname__".to_string(), Value::Str("Union".to_string()));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
        }
    }

    fn publish_union_type_aliases(&mut self, class: &ObjRef) {
        if let Some(types_module) = self.modules.get("types").cloned()
            && let Object::Module(module_data) = &mut *types_module.kind_mut()
        {
            module_data
                .globals
                .insert("UnionType".to_string(), Value::Class(class.clone()));
        }
        if let Some(typing_module) = self.modules.get("typing").cloned()
            && let Object::Module(module_data) = &mut *typing_module.kind_mut()
        {
            module_data
                .globals
                .insert("Union".to_string(), Value::Class(class.clone()));
        }
        if let Some(private_typing_module) = self.modules.get("_typing").cloned()
            && let Object::Module(module_data) = &mut *private_typing_module.kind_mut()
        {
            module_data
                .globals
                .insert("Union".to_string(), Value::Class(class.clone()));
        }
    }

    pub(super) fn ensure_union_type_class(&mut self) -> ObjRef {
        const CACHE_KEY: &str = "__types_union_type__";
        if let Some(existing) = self.union_type_class() {
            Self::normalize_union_type_class_identity(&existing);
            self.synthetic_builtin_classes
                .insert(CACHE_KEY.to_string(), existing.clone());
            self.publish_union_type_aliases(&existing);
            return existing;
        }
        if let Some(existing) = self.synthetic_builtin_classes.get(CACHE_KEY).cloned() {
            Self::normalize_union_type_class_identity(&existing);
            self.publish_union_type_aliases(&existing);
            return existing;
        }
        let class = self.synthetic_builtin_class("Union");
        Self::normalize_union_type_class_identity(&class);
        self.synthetic_builtin_classes
            .insert(CACHE_KEY.to_string(), class.clone());
        self.publish_union_type_aliases(&class);
        class
    }

    pub(super) fn is_union_type_class(&self, class: &ObjRef) -> bool {
        if self.is_typing_union_class(class) {
            return true;
        }
        let Some((name, _module)) = Self::class_name_and_module(class) else {
            return false;
        };
        if name == "UnionType" {
            return true;
        }
        Self::class_mro_has_name(class, "UnionType")
    }

    pub(super) fn is_typing_union_class(&self, class: &ObjRef) -> bool {
        let Some((name, module_name)) = Self::class_name_and_module(class) else {
            return false;
        };
        if name != "Union" {
            return false;
        }
        matches!(module_name.as_deref(), Some("_typing" | "typing"))
    }

    fn is_union_origin_value(&self, origin: &Value) -> bool {
        let Value::Class(class) = origin else {
            return false;
        };
        self.is_union_type_class(class) || self.is_typing_union_class(class)
    }

    fn none_type_value(&mut self) -> Value {
        if let Some(module) = self.modules.get("types")
            && let Object::Module(module_data) = &*module.kind()
            && let Some(none_type) = module_data.globals.get("NoneType")
        {
            return none_type.clone();
        }
        Value::Class(
            self.types_module_or_private_class("NoneType")
                .unwrap_or_else(|| self.fallback_none_type_class()),
        )
    }

    pub(super) fn generic_alias_parts_from_value(
        &self,
        value: &Value,
    ) -> Option<(Value, Vec<Value>)> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return None;
        };
        if !Self::class_mro_has_name(&instance_data.class, "GenericAlias")
            && !Self::class_mro_has_name(&instance_data.class, "_GenericAlias")
        {
            return None;
        }
        let origin = instance_data.attrs.get("__origin__")?.clone();
        let args = instance_data.attrs.get("__args__")?;
        let args = Self::tuple_items_from_value(args)?;
        Some((origin, args))
    }

    pub(super) fn is_types_generic_alias_value(&self, value: &Value) -> bool {
        let Value::Instance(instance) = value else {
            return false;
        };
        let Object::Instance(instance_data) = &*instance.kind() else {
            return false;
        };
        self.class_has_generic_alias_base(&instance_data.class)
            && self.generic_alias_parts_from_value(value).is_some()
    }

    pub(super) fn is_exact_types_generic_alias_value(&self, value: &Value) -> bool {
        let Value::Instance(instance) = value else {
            return false;
        };
        let Object::Instance(instance_data) = &*instance.kind() else {
            return false;
        };
        self.class_is_exact_types_generic_alias(&instance_data.class)
            && self.generic_alias_parts_from_value(value).is_some()
    }

    pub(super) fn class_has_generic_alias_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class).iter().any(|entry| {
            let entry_kind = entry.kind();
            let Object::Class(class_data) = &*entry_kind else {
                return false;
            };
            if class_data.name != "GenericAlias" {
                return false;
            }
            match class_data.attrs.get("__module__") {
                Some(Value::Str(module)) => matches!(module.as_str(), "types" | "_types"),
                _ => true,
            }
        })
    }

    pub(super) fn class_is_exact_types_generic_alias(&self, class: &ObjRef) -> bool {
        let class_kind = class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return false;
        };
        if class_data.name != "GenericAlias" {
            return false;
        }
        match class_data.attrs.get("__module__") {
            Some(Value::Str(module)) => matches!(module.as_str(), "types" | "_types"),
            _ => true,
        }
    }

    pub(super) fn instantiate_generic_alias_class(
        &mut self,
        class: ObjRef,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::type_error(format!(
                "GenericAlias() takes exactly 2 arguments ({} given)",
                args.len()
            )));
        }

        let origin = args.remove(0);
        let index = args.remove(0);
        let origin_module = self.optional_getattr_value(origin.clone(), "__module__")?;
        let mut name_attr: Option<Value> = None;
        let mut inst_attr: Option<Value> = None;
        let mut extra_attrs: Vec<(String, Value)> = Vec::new();
        for (name, value) in kwargs {
            match name.as_str() {
                "name" => name_attr = Some(value),
                "inst" => inst_attr = Some(value),
                _ => extra_attrs.push((name, value)),
            }
        }
        let alias = self.heap.alloc_instance(InstanceObject::new(class));
        if let Value::Instance(instance) = &alias
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            let typing_generic_alias_instance = {
                let class_kind = instance_data.class.kind();
                matches!(
                    &*class_kind,
                    Object::Class(class_data)
                        if class_data.name.contains("GenericAlias")
                            && matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module)) if module == "typing"
                            )
                )
            };
            instance_data
                .attrs
                .insert("__origin__".to_string(), origin.clone());
            let args = match index {
                Value::Tuple(tuple_obj) => Value::Tuple(tuple_obj),
                value => self.heap.alloc_tuple(vec![value]),
            };
            let mut parameters = Vec::new();
            if let Some(items) = Self::tuple_items_from_value(&args) {
                for item in &items {
                    self.collect_union_type_parameters_from_value(item, &mut parameters);
                }
            }
            instance_data.attrs.insert("__args__".to_string(), args);
            instance_data.attrs.insert(
                "__parameters__".to_string(),
                self.heap.alloc_tuple(parameters),
            );
            if let Some(flags) = self
                .optional_getattr_value(origin.clone(), "__flags__")
                .ok()
                .flatten()
            {
                instance_data.attrs.insert("__flags__".to_string(), flags);
            }
            instance_data
                .attrs
                .insert("__unpacked__".to_string(), Value::Bool(false));
            if typing_generic_alias_instance {
                instance_data
                    .attrs
                    .insert("_name".to_string(), name_attr.unwrap_or(Value::None));
                instance_data
                    .attrs
                    .insert("_inst".to_string(), inst_attr.unwrap_or(Value::Bool(true)));
                let should_set_origin_module = match instance_data.attrs.get("_name") {
                    None | Some(Value::None) => true,
                    Some(Value::Str(name)) => name.is_empty(),
                    _ => false,
                };
                if should_set_origin_module && let Some(module_name) = origin_module.clone() {
                    instance_data
                        .attrs
                        .insert("__module__".to_string(), module_name);
                }
            } else {
                if let Some(name_attr) = name_attr {
                    instance_data.attrs.insert("name".to_string(), name_attr);
                }
                if let Some(inst_attr) = inst_attr {
                    instance_data.attrs.insert("inst".to_string(), inst_attr);
                }
            }
            for (name, value) in extra_attrs {
                instance_data.attrs.insert(name, value);
            }
        }
        Ok(alias)
    }

    pub(super) fn union_args_from_value(&self, value: &Value) -> Option<Vec<Value>> {
        match value {
            Value::Instance(instance) => {
                let instance_kind = instance.kind();
                let Object::Instance(instance_data) = &*instance_kind else {
                    return None;
                };
                if self.is_union_type_class(&instance_data.class) {
                    let args = instance_data.attrs.get("__args__")?;
                    return Self::tuple_items_from_value(args);
                }
                if let Some((origin, args)) = self.generic_alias_parts_from_value(value)
                    && self.is_union_origin_value(&origin)
                {
                    return Some(args);
                }
                None
            }
            _ => None,
        }
    }

    fn value_contains_type_parameter(&self, value: &Value) -> bool {
        if self.is_type_parameter_value(value) {
            return true;
        }
        if let Some(args) = self.union_args_from_value(value) {
            return args
                .iter()
                .any(|item| self.value_contains_type_parameter(item));
        }
        if let Some((_origin, args)) = self.generic_alias_parts_from_value(value) {
            return args
                .iter()
                .any(|item| self.value_contains_type_parameter(item));
        }
        false
    }

    fn union_operand_value_with_forward(
        &self,
        value: &Value,
        allow_forward_ref_strings: bool,
    ) -> bool {
        if self.union_args_from_value(value).is_some() {
            return true;
        }
        if self.generic_alias_parts_from_value(value).is_some() {
            return true;
        }
        match value {
            Value::None | Value::Class(_) | Value::ExceptionType(_) => true,
            Value::Str(_) => allow_forward_ref_strings,
            Value::Function(_) => true,
            Value::Builtin(
                BuiltinFunction::Type
                | BuiltinFunction::Bool
                | BuiltinFunction::Int
                | BuiltinFunction::Float
                | BuiltinFunction::Str
                | BuiltinFunction::List
                | BuiltinFunction::Tuple
                | BuiltinFunction::Dict
                | BuiltinFunction::Set
                | BuiltinFunction::FrozenSet
                | BuiltinFunction::Bytes
                | BuiltinFunction::ByteArray
                | BuiltinFunction::MemoryView
                | BuiltinFunction::Range
                | BuiltinFunction::Slice
                | BuiltinFunction::Complex
                | BuiltinFunction::ClassMethod
                | BuiltinFunction::StaticMethod
                | BuiltinFunction::Property
                | BuiltinFunction::Map,
            ) => true,
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(items) => items.iter().all(|item| {
                    self.union_operand_value_with_forward(item, allow_forward_ref_strings)
                }),
                _ => false,
            },
            Value::Instance(instance) => {
                if self.is_type_parameter_value(value) {
                    return true;
                }
                let Some(class_name) = class_name_for_instance(instance) else {
                    return false;
                };
                if matches!(
                    class_name.as_str(),
                    "GenericAlias" | "_GenericAlias" | "UnionType" | "TypeAliasType" | "ForwardRef"
                ) {
                    return true;
                }
                let module_name = {
                    let instance_kind = instance.kind();
                    match &*instance_kind {
                        Object::Instance(instance_data) => {
                            let class_kind = instance_data.class.kind();
                            match &*class_kind {
                                Object::Class(class_data) => {
                                    match class_data.attrs.get("__module__") {
                                        Some(Value::Str(name)) => Some(name.clone()),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                };
                if matches!(
                    class_name.as_str(),
                    "TypeVar" | "TypeVarTuple" | "ParamSpec"
                ) {
                    return matches!(module_name.as_deref(), Some("typing" | "_typing"));
                }
                if matches!(module_name.as_deref(), Some("typing" | "_typing" | "types"))
                    && (class_name.contains("GenericAlias")
                        || class_name.contains("SpecialForm")
                        || class_name.contains("SpecialGenericAlias")
                        || class_name.contains("LiteralGenericAlias")
                        || matches!(
                            class_name.as_str(),
                            "Union"
                                | "NewType"
                                | "_SpecialForm"
                                | "_TypedCacheSpecialForm"
                                | "_AnyMeta"
                                | "_TupleType"
                                | "_TypingEllipsis"
                        ))
                {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    pub(super) fn union_operand_value(&self, value: &Value) -> bool {
        self.union_operand_value_with_forward(value, false)
    }

    fn collect_union_type_parameters_from_value(&mut self, value: &Value, out: &mut Vec<Value>) {
        if self.is_type_parameter_value(value) {
            if !out.contains(value) {
                out.push(value.clone());
            }
            return;
        }

        if let Some(union_args) = self.union_args_from_value(value) {
            for item in union_args {
                self.collect_union_type_parameters_from_value(&item, out);
            }
            return;
        }

        if let Some((_origin, generic_args)) = self.generic_alias_parts_from_value(value) {
            for item in generic_args {
                self.collect_union_type_parameters_from_value(&item, out);
            }
            return;
        }

        if let Some(items) = Self::tuple_items_from_value(value) {
            for item in items {
                self.collect_union_type_parameters_from_value(&item, out);
            }
            return;
        }

        if let Some(items) = Self::list_items_from_value(value) {
            for item in items {
                self.collect_union_type_parameters_from_value(&item, out);
            }
            return;
        }

        let mut maybe_parameters = None;
        if let Value::Instance(instance) = value {
            let instance_kind = instance.kind();
            if let Object::Instance(instance_data) = &*instance_kind {
                maybe_parameters = instance_data.attrs.get("__parameters__").cloned();
                if maybe_parameters.is_none() {
                    maybe_parameters = class_attr_lookup(&instance_data.class, "__parameters__");
                }
            }
        }
        if maybe_parameters.is_none() {
            maybe_parameters = self
                .optional_getattr_value(value.clone(), "__parameters__")
                .ok()
                .flatten();
        }
        if let Some(parameters) = maybe_parameters {
            if let Some(items) = Self::tuple_items_from_value(&parameters) {
                for item in items {
                    self.collect_union_type_parameters_from_value(&item, out);
                }
            } else if let Some(items) = Self::list_items_from_value(&parameters) {
                for item in items {
                    self.collect_union_type_parameters_from_value(&item, out);
                }
            }
        }
    }

    pub(super) fn literal_alias_args_from_value(&self, value: &Value) -> Option<Vec<Value>> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let instance_kind = instance.kind();
        let Object::Instance(instance_data) = &*instance_kind else {
            return None;
        };
        let class_kind = instance_data.class.kind();
        let Object::Class(class_data) = &*class_kind else {
            return None;
        };
        let module_name = match class_data.attrs.get("__module__") {
            Some(Value::Str(name)) => name.as_str(),
            _ => "",
        };
        if module_name != "typing" || !class_data.name.contains("LiteralGenericAlias") {
            return None;
        }
        let args = instance_data.attrs.get("__args__")?;
        Self::tuple_items_from_value(args)
    }

    fn literal_value_type_pairs_runtime(
        &mut self,
        args: &[Value],
    ) -> Result<Vec<Value>, RuntimeError> {
        let mut pairs = Vec::with_capacity(args.len());
        for arg in args {
            let arg_type = self.builtin_type(vec![arg.clone()], HashMap::new())?;
            pairs.push(self.heap.alloc_tuple(vec![arg.clone(), arg_type]));
        }
        self.dedup_hashable_values_runtime(pairs)
    }

    pub(super) fn literal_alias_args_equal_runtime(
        &mut self,
        left: &[Value],
        right: &[Value],
    ) -> Result<bool, RuntimeError> {
        let left_pairs = self.literal_value_type_pairs_runtime(left)?;
        let right_pairs = self.literal_value_type_pairs_runtime(right)?;
        if left_pairs.len() != right_pairs.len() {
            return Ok(false);
        }
        for left_pair in &left_pairs {
            if !self.sequence_contains_runtime_value(&right_pairs, left_pair)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn literal_alias_args_hash_runtime(
        &mut self,
        args: &[Value],
    ) -> Result<u64, RuntimeError> {
        let pairs = self.literal_value_type_pairs_runtime(args)?;
        let pair_set = self.heap.alloc_frozenset(pairs);
        self.hash_value_runtime(&pair_set).map(|hash| hash as u64)
    }

    fn forward_ref_value_from_string(&mut self, text: String) -> Value {
        if self.import_module("typing").is_err() {
            return Value::Str(text);
        }
        let Some(module) = self.modules.get("typing").cloned() else {
            return Value::Str(text);
        };
        let forward_ref_ctor = {
            let Object::Module(module_data) = &*module.kind() else {
                return Value::Str(text);
            };
            module_data.globals.get("ForwardRef").cloned()
        }
        .or_else(|| {
            self.builtin_getattr(
                vec![
                    Value::Module(module.clone()),
                    Value::Str("ForwardRef".to_string()),
                ],
                HashMap::new(),
            )
            .ok()
        });
        let Some(forward_ref_ctor) = forward_ref_ctor else {
            return Value::Str(text);
        };
        match self.call_internal(
            forward_ref_ctor.clone(),
            vec![Value::Str(text.clone())],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => value,
            _ => {
                let Value::Class(class) = forward_ref_ctor else {
                    return Value::Str(text);
                };
                let forward_ref = self.heap.alloc_instance(InstanceObject::new(class));
                if let Value::Instance(instance) = &forward_ref
                    && let Object::Instance(instance_data) = &mut *instance.kind_mut()
                {
                    instance_data
                        .attrs
                        .insert("__forward_arg__".to_string(), Value::Str(text.clone()));
                    instance_data
                        .attrs
                        .insert("__forward_is_class__".to_string(), Value::Bool(false));
                    instance_data
                        .attrs
                        .insert("__forward_module__".to_string(), Value::None);
                    instance_data
                        .attrs
                        .insert("__owner__".to_string(), Value::None);
                }
                forward_ref
            }
        }
    }

    fn collect_union_member(
        &mut self,
        value: Value,
        allow_forward_ref_strings: bool,
        strict_operands: bool,
        out: &mut Vec<Value>,
    ) -> Result<(), RuntimeError> {
        if let Some(items) = self.union_args_from_value(&value) {
            for item in items {
                self.collect_union_member(item, allow_forward_ref_strings, strict_operands, out)?;
            }
            return Ok(());
        }

        let normalized = match value {
            Value::None => self.none_type_value(),
            Value::Str(text) if allow_forward_ref_strings => {
                self.forward_ref_value_from_string(text)
            }
            other => other,
        };

        if strict_operands
            && !self.union_operand_value_with_forward(&normalized, allow_forward_ref_strings)
        {
            return Err(RuntimeError::type_error("unsupported operand type for |"));
        }

        for existing in out.iter() {
            if let (Some(left_literal_args), Some(right_literal_args)) = (
                self.literal_alias_args_from_value(existing),
                self.literal_alias_args_from_value(&normalized),
            ) {
                if self.literal_alias_args_equal_runtime(&left_literal_args, &right_literal_args)? {
                    return Ok(());
                }
                continue;
            }
            let existing_hash = self.hash_value_runtime(existing);
            let normalized_hash = self.hash_value_runtime(&normalized);
            match (&existing_hash, &normalized_hash) {
                (Ok(left_hash), Ok(right_hash)) if left_hash != right_hash => continue,
                (Err(left_err), _) if !runtime_error_matches_exception(left_err, "TypeError") => {
                    return Err(left_err.clone());
                }
                (_, Err(right_err)) if !runtime_error_matches_exception(right_err, "TypeError") => {
                    return Err(right_err.clone());
                }
                _ => {}
            }
            let is_equal = self.compare_eq_runtime(existing.clone(), normalized.clone())?;
            if self.truthy_from_value(&is_equal)? {
                return Ok(());
            }
        }

        out.push(normalized);
        Ok(())
    }

    pub(super) fn build_union_value_from_members(
        &mut self,
        members: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let allow_forward_ref_strings = members
            .iter()
            .any(|value| self.value_contains_type_parameter(value));
        self.build_union_value_from_members_with_forward(members, allow_forward_ref_strings)
    }

    fn build_union_value_from_members_with_forward(
        &mut self,
        members: Vec<Value>,
        allow_forward_ref_strings: bool,
    ) -> Result<Value, RuntimeError> {
        let mut flat_members = Vec::new();
        for member in members {
            self.collect_union_member(member, allow_forward_ref_strings, true, &mut flat_members)?;
        }

        self.build_union_value_from_flat_members(flat_members)
    }

    fn build_union_value_from_members_with_forward_lenient(
        &mut self,
        members: Vec<Value>,
        allow_forward_ref_strings: bool,
    ) -> Result<Value, RuntimeError> {
        let mut flat_members = Vec::new();
        for member in members {
            self.collect_union_member(member, allow_forward_ref_strings, false, &mut flat_members)?;
        }

        self.build_union_value_from_flat_members(flat_members)
    }

    fn build_union_value_from_flat_members(
        &mut self,
        mut flat_members: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if flat_members.is_empty() {
            return Err(RuntimeError::type_error("Cannot take a Union of no types."));
        }
        if flat_members.len() == 1 {
            return Ok(flat_members.remove(0));
        }

        let mut parameters = Vec::new();
        for item in &flat_members {
            self.collect_union_type_parameters_from_value(item, &mut parameters);
        }

        let unhashable_count = 0i64;
        let union_class = self.ensure_union_type_class();
        let union = self
            .heap
            .alloc_instance(InstanceObject::new(union_class.clone()));
        if let Value::Instance(instance) = &union
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data
                .attrs
                .insert("__origin__".to_string(), Value::Class(union_class.clone()));
            instance_data
                .attrs
                .insert("__args__".to_string(), self.heap.alloc_tuple(flat_members));
            instance_data.attrs.insert(
                "__parameters__".to_string(),
                self.heap.alloc_tuple(parameters),
            );
            instance_data
                .attrs
                .insert("__module__".to_string(), Value::Str("typing".to_string()));
            instance_data
                .attrs
                .insert("__name__".to_string(), Value::Str("Union".to_string()));
            instance_data
                .attrs
                .insert("__qualname__".to_string(), Value::Str("Union".to_string()));
            instance_data.attrs.insert(
                "__pyrs_union_unhashable_count__".to_string(),
                Value::Int(unhashable_count),
            );
        }
        Ok(union)
    }

    pub(super) fn build_union_value_from_pair(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        if self.union_args_from_value(&left).is_some()
            || self.union_args_from_value(&right).is_some()
        {
            return self
                .build_union_value_from_members_with_forward_lenient(vec![left, right], true);
        }
        self.build_union_value_from_members(vec![left, right])
    }

    fn subscript_items_from_index(&self, index: Value) -> Vec<Value> {
        if let Some(items) = Self::tuple_items_from_value(&index) {
            return items;
        }
        vec![index]
    }

    fn sequence_items_from_typing_value(value: &Value) -> Option<Vec<Value>> {
        Self::tuple_items_from_value(value).or_else(|| Self::list_items_from_value(value))
    }

    fn is_ellipsis_marker_value(&self, value: &Value) -> bool {
        let Some(ellipsis) = self.builtins.get("Ellipsis") else {
            return false;
        };
        match (value, ellipsis) {
            (Value::Instance(left), Value::Instance(right)) => left.id() == right.id(),
            _ => value == ellipsis,
        }
    }

    fn generic_alias_substitution_lookup(
        &mut self,
        target: &Value,
        substitutions: &[(Value, Value)],
    ) -> Result<Option<Value>, RuntimeError> {
        for (param, replacement) in substitutions {
            if target == param {
                return Ok(Some(replacement.clone()));
            }
        }
        Ok(None)
    }

    fn value_is_unpacked_typevartuple_marker(
        &mut self,
        value: &Value,
    ) -> Result<bool, RuntimeError> {
        if let Some(flag) =
            self.optional_getattr_value(value.clone(), "__typing_is_unpacked_typevartuple__")?
        {
            return self.truthy_from_value(&flag);
        }
        Ok(false)
    }

    fn value_has_unpacked_marker(&mut self, value: &Value) -> Result<bool, RuntimeError> {
        if let Some(flag) = self.optional_getattr_value(value.clone(), "__unpacked__")? {
            return self.truthy_from_value(&flag);
        }
        Ok(false)
    }

    fn generic_alias_preprocess_subscript_args(
        &mut self,
        args: Vec<Value>,
    ) -> Result<Vec<Value>, RuntimeError> {
        let mut processed = Vec::new();
        for arg in args {
            let Some(subargs_value) =
                self.optional_getattr_value(arg.clone(), "__typing_unpacked_tuple_args__")?
            else {
                processed.push(arg);
                continue;
            };
            let Some(subargs) = Self::sequence_items_from_typing_value(&subargs_value) else {
                processed.push(arg);
                continue;
            };
            let ends_with_ellipsis = subargs
                .last()
                .is_some_and(|last| self.is_ellipsis_marker_value(last));
            if ends_with_ellipsis {
                processed.push(arg);
            } else {
                processed.extend(subargs);
            }
        }
        Ok(processed)
    }

    fn substitute_type_parameters_in_value(
        &mut self,
        value: Value,
        substitutions: &[(Value, Value)],
    ) -> Result<Value, RuntimeError> {
        if let Some(replacement) = self.generic_alias_substitution_lookup(&value, substitutions)? {
            return Ok(replacement);
        }

        if let Some(items) = Self::tuple_items_from_value(&value) {
            let mut substituted = Vec::with_capacity(items.len());
            for item in items {
                substituted.push(self.substitute_type_parameters_in_value(item, substitutions)?);
            }
            return Ok(self.heap.alloc_tuple(substituted));
        }

        if let Some(items) = Self::list_items_from_value(&value) {
            let mut substituted = Vec::with_capacity(items.len());
            for item in items {
                substituted.push(self.substitute_type_parameters_in_value(item, substitutions)?);
            }
            return Ok(self.heap.alloc_list(substituted));
        }

        if let Some(items) = self.union_args_from_value(&value) {
            let mut substituted = Vec::with_capacity(items.len());
            for item in items {
                substituted.push(self.substitute_type_parameters_in_value(item, substitutions)?);
            }
            return self.build_union_value_from_members(substituted);
        }

        if self.generic_alias_parts_from_value(&value).is_some() {
            let subparams = self
                .optional_getattr_value(value.clone(), "__parameters__")?
                .and_then(|value| Self::sequence_items_from_typing_value(&value))
                .unwrap_or_default();
            if subparams.is_empty() {
                return Ok(value);
            }
            let mut subargs = Vec::new();
            let mut changed = false;
            for subparam in subparams {
                let replacement_lookup =
                    self.generic_alias_substitution_lookup(&subparam, substitutions)?;
                let replacement = replacement_lookup.clone().unwrap_or(subparam.clone());
                if !changed && replacement_lookup.is_some() && replacement != subparam {
                    changed = true;
                }
                if typing_typevartuple_param_marker(&subparam) {
                    if let Some(items) = Self::sequence_items_from_typing_value(&replacement) {
                        subargs.extend(items);
                    } else {
                        subargs.push(replacement);
                    }
                } else {
                    subargs.push(replacement);
                }
            }
            if let Some((_origin, current_args)) = self.generic_alias_parts_from_value(&value)
                && current_args == subargs
            {
                return Ok(value);
            }
            if !changed {
                return Ok(value);
            }
            return self.getitem_value(value, self.heap.alloc_tuple(subargs));
        }

        Ok(value)
    }

    pub(super) fn subscript_union_value(
        &mut self,
        value: Value,
        index: Value,
    ) -> Result<Value, RuntimeError> {
        let args = self
            .union_args_from_value(&value)
            .ok_or_else(|| RuntimeError::type_error("subscript unsupported type"))?;
        let mut parameters = Vec::new();
        for item in &args {
            self.collect_union_type_parameters_from_value(item, &mut parameters);
        }
        let replacement_values = self.subscript_items_from_index(index);
        if parameters.is_empty() {
            return Err(RuntimeError::type_error("union is not a generic type"));
        }
        if parameters.len() != replacement_values.len() {
            let actual = replacement_values.len();
            let expected = parameters.len();
            let many_or_few = if actual > expected { "many" } else { "few" };
            return Err(RuntimeError::type_error(format!(
                "Too {many_or_few} arguments for {}; actual {actual}, expected {expected}",
                format_repr(&value)
            )));
        }
        let substitutions = parameters
            .into_iter()
            .zip(replacement_values)
            .collect::<Vec<_>>();

        let mut substituted_args = Vec::with_capacity(args.len());
        for item in args {
            substituted_args.push(self.substitute_type_parameters_in_value(item, &substitutions)?);
        }
        self.build_union_value_from_members(substituted_args)
    }

    pub(super) fn subscript_generic_alias_value(
        &mut self,
        value: Value,
        index: Value,
    ) -> Result<Value, RuntimeError> {
        let trace = self
            .host
            .env_var_os("PYRS_TRACE_GENERIC_ALIAS_SUBSCRIPT")
            .is_some();
        if trace {
            eprintln!(
                "[ga-sub] enter value_type={} index_type={}",
                self.value_type_name_for_error(&value),
                self.value_type_name_for_error(&index)
            );
        }
        let (origin, args) = self
            .generic_alias_parts_from_value(&value)
            .ok_or_else(|| RuntimeError::type_error("subscript unsupported type"))?;
        let origin_is_collections_callable = match &origin {
            Value::Class(class) => {
                Self::class_name_and_module(class).is_some_and(|(name, module)| {
                    name == "Callable"
                        && matches!(
                            module.as_deref(),
                            Some("collections.abc" | "_collections_abc")
                        )
                })
            }
            _ => false,
        };
        if trace {
            eprintln!("[ga-sub] parts args_len={}", args.len());
        }
        let mut parameters = self
            .optional_getattr_value(value.clone(), "__parameters__")?
            .and_then(|value| Self::sequence_items_from_typing_value(&value))
            .unwrap_or_default();
        if trace {
            eprintln!("[ga-sub] initial parameters_len={}", parameters.len());
        }
        if parameters.is_empty() {
            for item in &args {
                self.collect_union_type_parameters_from_value(item, &mut parameters);
            }
            if trace {
                eprintln!("[ga-sub] collected parameters_len={}", parameters.len());
            }
        }
        if parameters.is_empty() {
            return Err(RuntimeError::type_error(format!(
                "{} is not a generic class",
                format_repr(&value)
            )));
        }
        let mut replacement_values =
            self.generic_alias_preprocess_subscript_args(self.subscript_items_from_index(index))?;
        if trace {
            eprintln!(
                "[ga-sub] replacement_len_prepared={}",
                replacement_values.len()
            );
        }
        for param in parameters.iter().cloned() {
            let Some(prepare) =
                self.optional_getattr_value(param.clone(), "__typing_prepare_subst__")?
            else {
                continue;
            };
            let prepared = match self.call_internal(
                prepare,
                vec![
                    value.clone(),
                    self.heap.alloc_tuple(replacement_values.clone()),
                ],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(
                        self.runtime_error_from_active_exception("typing prepare_subst failed")
                    );
                }
            };
            if let Some(items) = Self::sequence_items_from_typing_value(&prepared) {
                replacement_values = items;
            } else if self.is_type_parameter_value(&param) {
                return Err(RuntimeError::type_error(
                    "typing parameters must be a sequence",
                ));
            }
            if trace {
                eprintln!(
                    "[ga-sub] replacement_len_after_prepare={}",
                    replacement_values.len()
                );
            }
        }
        if parameters.len() != replacement_values.len() {
            let actual = replacement_values.len();
            let expected = parameters.len();
            let many_or_few = if actual > expected { "many" } else { "few" };
            return Err(RuntimeError::type_error(format!(
                "Too {many_or_few} arguments for {}; actual {actual}, expected {expected}",
                format_repr(&value)
            )));
        }
        let substitutions = parameters
            .into_iter()
            .zip(replacement_values)
            .collect::<Vec<_>>();
        let mut substituted_args = Vec::with_capacity(args.len());
        for (item_idx, item) in args.into_iter().enumerate() {
            if trace {
                eprintln!(
                    "[ga-sub] item_idx={item_idx} item_type={}",
                    self.value_type_name_for_error(&item)
                );
            }
            let direct_replacement =
                self.generic_alias_substitution_lookup(&item, &substitutions)?;
            let substfunc = self.optional_getattr_value(item.clone(), "__typing_subst__")?;
            let substituted = if let (Some(substfunc), Some(replacement)) =
                (substfunc, direct_replacement.clone())
            {
                match self.call_internal(substfunc, vec![replacement], HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("typing substitution failed")
                        );
                    }
                }
            } else {
                let item_is_generic_alias = self.generic_alias_parts_from_value(&item).is_some();
                if !item_is_generic_alias {
                    self.substitute_type_parameters_in_value(
                        direct_replacement.unwrap_or(item.clone()),
                        &substitutions,
                    )?
                } else {
                    let subparams = self
                        .optional_getattr_value(item.clone(), "__parameters__")?
                        .and_then(|value| Self::sequence_items_from_typing_value(&value))
                        .unwrap_or_default();
                    if subparams.is_empty() {
                        self.substitute_type_parameters_in_value(
                            direct_replacement.unwrap_or(item.clone()),
                            &substitutions,
                        )?
                    } else {
                        let mut subargs = Vec::new();
                        for subparam in subparams {
                            let replacement = self
                                .generic_alias_substitution_lookup(&subparam, &substitutions)?
                                .unwrap_or(subparam.clone());
                            if typing_typevartuple_param_marker(&subparam) {
                                if let Some(items) =
                                    Self::sequence_items_from_typing_value(&replacement)
                                {
                                    subargs.extend(items);
                                } else {
                                    subargs.push(replacement);
                                }
                            } else {
                                subargs.push(replacement);
                            }
                        }
                        if trace {
                            eprintln!(
                                "[ga-sub] item_idx={item_idx} recurse_getitem subargs_len={}",
                                subargs.len()
                            );
                        }
                        self.getitem_value(item.clone(), self.heap.alloc_tuple(subargs))?
                    }
                }
            };
            if self.value_is_unpacked_typevartuple_marker(&item)? {
                if let Some(items) = Self::sequence_items_from_typing_value(&substituted) {
                    substituted_args.extend(items);
                    continue;
                }
                return Err(RuntimeError::type_error(format!(
                    "expected __typing_subst__ of {} objects to return a tuple, not {}",
                    self.value_type_name_for_error(&item),
                    self.value_type_name_for_error(&substituted),
                )));
            }
            if origin_is_collections_callable
                && let Some(items) = Self::tuple_items_from_value(&substituted)
            {
                substituted_args.extend(items);
                continue;
            }
            substituted_args.push(substituted);
        }
        let index_value = self.heap.alloc_tuple(substituted_args);
        let preserve_unpacked_marker = self.value_has_unpacked_marker(&value)?;
        let result = self.alloc_generic_alias_instance(origin.clone(), index_value.clone());
        if preserve_unpacked_marker
            && let Value::Instance(instance) = &result
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data
                .attrs
                .insert("__unpacked__".to_string(), Value::Bool(true));
            let origin_is_tuple_alias = match &origin {
                Value::Builtin(BuiltinFunction::Tuple) => true,
                Value::Class(class) => {
                    let class_kind = class.kind();
                    matches!(&*class_kind, Object::Class(class_data) if class_data.name == "tuple")
                }
                _ => false,
            };
            if origin_is_tuple_alias {
                instance_data
                    .attrs
                    .insert("__typing_unpacked_tuple_args__".to_string(), index_value);
            }
        }
        Ok(result)
    }

    pub(super) fn collect_iterable_values(
        &mut self,
        source: Value,
    ) -> Result<Vec<Value>, RuntimeError> {
        // Keep the original iterable alive while we consume it. This avoids
        // premature finalization for temporary objects with __del__.
        let _source_guard = source.clone();
        let iter = match self.to_iterator_value(source) {
            Ok(iter) => iter,
            Err(err) if runtime_error_matches_exception(&err, "TypeError") => {
                if self.host.env_var("PYRS_DEBUG_EXPECTED_ITERABLE").is_some() {
                    let frames = self
                        .frames
                        .iter()
                        .rev()
                        .take(4)
                        .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                        .collect::<Vec<_>>()
                        .join(" <= ");
                    eprintln!(
                        "expected iterable: type={} repr={} frames={}",
                        self.value_type_name_for_error(&_source_guard),
                        format_repr(&_source_guard),
                        frames
                    );
                }
                return Err(RuntimeError::type_error("expected iterable"));
            }
            Err(err) => return Err(err),
        };
        match iter {
            Value::Iterator(iterator_ref) => {
                let mut out = Vec::new();
                while let Some(value) = self.iterator_next_value(&iterator_ref)? {
                    out.push(value);
                }
                Ok(out)
            }
            Value::Generator(generator) => {
                let mut out = Vec::new();
                loop {
                    match self.generator_for_iter_next(&generator)? {
                        GeneratorResumeOutcome::Yield(value) => out.push(value),
                        GeneratorResumeOutcome::Complete(_) => break,
                        GeneratorResumeOutcome::PropagatedException => {
                            return Err(self.iteration_error_from_state("iteration failed")?);
                        }
                    }
                }
                Ok(out)
            }
            Value::Instance(_) => {
                let mut out = Vec::new();
                loop {
                    match self.next_from_iterator_value(&iter)? {
                        GeneratorResumeOutcome::Yield(value) => out.push(value),
                        GeneratorResumeOutcome::Complete(_) => break,
                        GeneratorResumeOutcome::PropagatedException => {
                            return Err(self.iteration_error_from_state("iteration failed")?);
                        }
                    }
                }
                Ok(out)
            }
            _ => Err(RuntimeError::type_error("expected iterable")),
        }
    }

    pub(super) fn value_to_bytes_payload(&mut self, value: Value) -> Result<Vec<u8>, RuntimeError> {
        match value {
            Value::Iterator(iterator_ref) => {
                let mut out = Vec::new();
                while let Some(value) = self.iterator_next_value(&iterator_ref)? {
                    let byte = value_to_int(value)?;
                    if !(0..=255).contains(&byte) {
                        return Err(RuntimeError::value_error("byte must be in range(0, 256)"));
                    }
                    out.push(byte as u8);
                }
                Ok(out)
            }
            Value::Generator(generator) => {
                let mut out = Vec::new();
                loop {
                    match self.generator_for_iter_next(&generator)? {
                        GeneratorResumeOutcome::Yield(value) => {
                            let byte = value_to_int(value)?;
                            if !(0..=255).contains(&byte) {
                                return Err(RuntimeError::value_error(
                                    "byte must be in range(0, 256)",
                                ));
                            }
                            out.push(byte as u8);
                        }
                        GeneratorResumeOutcome::Complete(_) => break,
                        GeneratorResumeOutcome::PropagatedException => {
                            return Err(self.iteration_error_from_state("iteration failed")?);
                        }
                    }
                }
                Ok(out)
            }
            Value::Instance(instance) => {
                let instance_value = Value::Instance(instance);
                if let Some(bytes_value) =
                    self.call_unary_special_method(&instance_value, "__bytes__")?
                {
                    let is_bytes_result = matches!(&bytes_value, Value::Bytes(_))
                        || matches!(&bytes_value, Value::Instance(result_instance)
                            if matches!(&*result_instance.kind(), Object::Instance(instance_data)
                                if self.class_has_builtin_bytes_base(&instance_data.class)));
                    if !is_bytes_result {
                        return Err(RuntimeError::type_error(format!(
                            "__bytes__ returned non-bytes (type {})",
                            self.value_type_name_for_error(&bytes_value)
                        )));
                    }
                    return value_to_bytes_payload(bytes_value);
                }
                match value_to_bytes_payload(instance_value.clone()) {
                    Ok(payload) => Ok(payload),
                    Err(_) => {
                        self.ensure_sync_iterator_target(&instance_value)?;
                        let iterator = self
                            .to_iterator_value(instance_value)
                            .map_err(|_| RuntimeError::type_error("expected bytes-like payload"))?;
                        let mut out = Vec::new();
                        loop {
                            match self.next_from_iterator_value(&iterator)? {
                                GeneratorResumeOutcome::Yield(value) => {
                                    let byte = value_to_int(value)?;
                                    if !(0..=255).contains(&byte) {
                                        return Err(RuntimeError::new(
                                            "byte must be in range(0, 256)",
                                        ));
                                    }
                                    out.push(byte as u8);
                                }
                                GeneratorResumeOutcome::Complete(_) => break,
                                GeneratorResumeOutcome::PropagatedException => {
                                    return Err(
                                        self.iteration_error_from_state("iteration failed")?
                                    );
                                }
                            }
                        }
                        Ok(out)
                    }
                }
            }
            other => value_to_bytes_payload(other),
        }
    }

    pub(super) fn random_randbelow(&mut self, upper: i64) -> Result<i64, RuntimeError> {
        if upper <= 0 {
            return Err(RuntimeError::value_error("empty range for randrange()"));
        }
        let upper = upper as u64;
        let zone = u64::MAX - (u64::MAX % upper);
        loop {
            let value = ((self.random.next_u32() as u64) << 32) | self.random.next_u32() as u64;
            if value < zone {
                return Ok((value % upper) as i64);
            }
        }
    }

    pub(super) fn install_builtins(&mut self) {
        self.builtins.insert("True".to_string(), Value::Bool(true));
        self.builtins
            .insert("False".to_string(), Value::Bool(false));
        self.builtins.insert("None".to_string(), Value::None);
        self.builtins
            .insert("print".to_string(), Value::Builtin(BuiltinFunction::Print));
        self.builtins
            .insert("input".to_string(), Value::Builtin(BuiltinFunction::Input));
        self.builtins
            .insert("repr".to_string(), Value::Builtin(BuiltinFunction::Repr));
        self.builtins
            .insert("ascii".to_string(), Value::Builtin(BuiltinFunction::Ascii));
        self.builtins
            .insert("len".to_string(), Value::Builtin(BuiltinFunction::Len));
        self.builtins
            .insert("range".to_string(), Value::Builtin(BuiltinFunction::Range));
        self.builtins
            .insert("slice".to_string(), Value::Builtin(BuiltinFunction::Slice));
        self.builtins
            .insert("bool".to_string(), Value::Builtin(BuiltinFunction::Bool));
        self.builtins
            .insert("int".to_string(), Value::Builtin(BuiltinFunction::Int));
        self.builtins
            .insert("float".to_string(), Value::Builtin(BuiltinFunction::Float));
        self.builtins
            .insert("str".to_string(), Value::Builtin(BuiltinFunction::Str));
        self.builtins
            .insert("ord".to_string(), Value::Builtin(BuiltinFunction::Ord));
        self.builtins
            .insert("chr".to_string(), Value::Builtin(BuiltinFunction::Chr));
        self.builtins
            .insert("bin".to_string(), Value::Builtin(BuiltinFunction::Bin));
        self.builtins
            .insert("oct".to_string(), Value::Builtin(BuiltinFunction::Oct));
        self.builtins
            .insert("hex".to_string(), Value::Builtin(BuiltinFunction::Hex));
        self.builtins
            .insert("abs".to_string(), Value::Builtin(BuiltinFunction::Abs));
        self.builtins
            .insert("sum".to_string(), Value::Builtin(BuiltinFunction::Sum));
        self.builtins
            .insert("min".to_string(), Value::Builtin(BuiltinFunction::Min));
        self.builtins
            .insert("max".to_string(), Value::Builtin(BuiltinFunction::Max));
        self.builtins
            .insert("all".to_string(), Value::Builtin(BuiltinFunction::All));
        self.builtins
            .insert("any".to_string(), Value::Builtin(BuiltinFunction::Any));
        self.builtins
            .insert("map".to_string(), Value::Builtin(BuiltinFunction::Map));
        self.builtins.insert(
            "filter".to_string(),
            Value::Builtin(BuiltinFunction::Filter),
        );
        self.builtins
            .insert("pow".to_string(), Value::Builtin(BuiltinFunction::Pow));
        self.builtins
            .insert("round".to_string(), Value::Builtin(BuiltinFunction::Round));
        self.builtins.insert(
            "format".to_string(),
            Value::Builtin(BuiltinFunction::Format),
        );
        self.builtins
            .insert("list".to_string(), Value::Builtin(BuiltinFunction::List));
        self.builtins
            .insert("tuple".to_string(), Value::Builtin(BuiltinFunction::Tuple));
        self.builtins
            .insert("dict".to_string(), Value::Builtin(BuiltinFunction::Dict));
        self.builtins
            .insert("set".to_string(), Value::Builtin(BuiltinFunction::Set));
        self.builtins.insert(
            "frozenset".to_string(),
            Value::Builtin(BuiltinFunction::FrozenSet),
        );
        self.builtins
            .insert("bytes".to_string(), Value::Builtin(BuiltinFunction::Bytes));
        self.builtins.insert(
            "bytearray".to_string(),
            Value::Builtin(BuiltinFunction::ByteArray),
        );
        self.builtins.insert(
            "memoryview".to_string(),
            Value::Builtin(BuiltinFunction::MemoryView),
        );
        self.builtins.insert(
            "complex".to_string(),
            Value::Builtin(BuiltinFunction::Complex),
        );
        self.builtins.insert(
            "divmod".to_string(),
            Value::Builtin(BuiltinFunction::DivMod),
        );
        self.builtins.insert(
            "sorted".to_string(),
            Value::Builtin(BuiltinFunction::Sorted),
        );
        self.builtins.insert(
            "enumerate".to_string(),
            Value::Builtin(BuiltinFunction::Enumerate),
        );
        self.builtins
            .insert("zip".to_string(), Value::Builtin(BuiltinFunction::Zip));
        self.builtins
            .insert("id".to_string(), Value::Builtin(BuiltinFunction::Id));
        self.builtins
            .insert("dir".to_string(), Value::Builtin(BuiltinFunction::Dir));
        self.builtins
            .insert("iter".to_string(), Value::Builtin(BuiltinFunction::Iter));
        self.builtins
            .insert("next".to_string(), Value::Builtin(BuiltinFunction::Next));
        self.builtins
            .insert("aiter".to_string(), Value::Builtin(BuiltinFunction::AIter));
        self.builtins
            .insert("anext".to_string(), Value::Builtin(BuiltinFunction::ANext));
        let object_class = match self
            .heap
            .alloc_class(ClassObject::new("object".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *object_class.kind_mut() {
            class_data.mro = vec![object_class.clone()];
            class_data
                .attrs
                .insert("__name__".to_string(), Value::Str("object".to_string()));
            class_data
                .attrs
                .insert("__bases__".to_string(), self.heap.alloc_tuple(Vec::new()));
            class_data.attrs.insert(
                "__mro__".to_string(),
                self.heap
                    .alloc_tuple(vec![Value::Class(object_class.clone())]),
            );
            class_data
                .attrs
                .insert("__flags__".to_string(), Value::Int(0));
            class_data.attrs.insert(
                "__new__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectNew),
            );
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectInit),
            );
            class_data.attrs.insert(
                "__init_subclass__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectInitSubclass),
            );
            class_data.attrs.insert(
                "__getattribute__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectGetAttribute),
            );
            class_data.attrs.insert(
                "__setattr__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectSetAttr),
            );
            class_data.attrs.insert(
                "__delattr__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectDelAttr),
            );
            class_data
                .attrs
                .insert("__dir__".to_string(), Value::Builtin(BuiltinFunction::Dir));
            class_data.attrs.insert(
                "__getstate__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectGetState),
            );
            class_data.attrs.insert(
                "__setstate__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectSetState),
            );
            class_data.attrs.insert(
                "__eq__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectEq),
            );
            class_data.attrs.insert(
                "__ne__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectNe),
            );
            class_data.attrs.insert(
                "__lt__".to_string(),
                Value::Builtin(BuiltinFunction::OperatorLt),
            );
            class_data
                .attrs
                .insert("__hash__".to_string(), Value::Builtin(BuiltinFunction::Id));
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::Repr),
            );
            class_data
                .attrs
                .insert("__str__".to_string(), Value::Builtin(BuiltinFunction::Str));
            class_data.attrs.insert(
                "__format__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectFormat),
            );
            class_data.attrs.insert(
                "__reduce_ex__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectReduceEx),
            );
            class_data.attrs.insert(
                "__reduce__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectReduce),
            );
        }
        let type_class = match self.heap.alloc_class(ClassObject::new(
            "type".to_string(),
            vec![object_class.clone()],
        )) {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(type_data) = &mut *type_class.kind_mut() {
            type_data.mro = vec![type_class.clone(), object_class.clone()];
            type_data.metaclass = Some(type_class.clone());
            type_data
                .attrs
                .insert("__name__".to_string(), Value::Str("type".to_string()));
            type_data
                .attrs
                .insert("__qualname__".to_string(), Value::Str("type".to_string()));
            type_data
                .attrs
                .insert("__module__".to_string(), Value::Str("builtins".to_string()));
            type_data.attrs.insert(
                "__bases__".to_string(),
                self.heap
                    .alloc_tuple(vec![Value::Class(object_class.clone())]),
            );
            type_data.attrs.insert(
                "__mro__".to_string(),
                self.heap.alloc_tuple(vec![
                    Value::Class(type_class.clone()),
                    Value::Class(object_class.clone()),
                ]),
            );
            type_data
                .attrs
                .insert("__new__".to_string(), Value::Builtin(BuiltinFunction::Type));
            type_data.attrs.insert(
                "__call__".to_string(),
                Value::Builtin(BuiltinFunction::TypeCall),
            );
            type_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::TypeInit),
            );
            type_data.attrs.insert(
                "__reduce_ex__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectReduceEx),
            );
            type_data.attrs.insert(
                "__reduce__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectReduce),
            );
            type_data
                .attrs
                .insert("__flags__".to_string(), Value::Int(0));
        }
        if let Object::Class(class_data) = &mut *object_class.kind_mut() {
            class_data.metaclass = Some(type_class.clone());
        }
        self.builtins
            .insert("object".to_string(), Value::Class(object_class));
        self.builtins
            .insert("open".to_string(), Value::Builtin(BuiltinFunction::IoOpen));
        self.builtins
            .insert("type".to_string(), Value::Builtin(BuiltinFunction::Type));
        self.builtins.insert(
            "classmethod".to_string(),
            Value::Builtin(BuiltinFunction::ClassMethod),
        );
        self.builtins.insert(
            "staticmethod".to_string(),
            Value::Builtin(BuiltinFunction::StaticMethod),
        );
        self.builtins.insert(
            "property".to_string(),
            Value::Builtin(BuiltinFunction::Property),
        );
        let ellipsis = self.heap.ellipsis_singleton();
        self.builtins.insert("Ellipsis".to_string(), ellipsis);
        let not_implemented = {
            let class = match self.heap.alloc_class(ClassObject::new(
                "NotImplementedType".to_string(),
                Vec::new(),
            )) {
                Value::Class(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Class(class_data) = &mut *class.kind_mut() {
                class_data
                    .attrs
                    .insert("__module__".to_string(), Value::Str("builtins".to_string()));
                class_data.attrs.insert(
                    "__name__".to_string(),
                    Value::Str("NotImplementedType".to_string()),
                );
                class_data.attrs.insert(
                    "__qualname__".to_string(),
                    Value::Str("NotImplementedType".to_string()),
                );
            }
            self.heap.alloc_instance(InstanceObject::new(class))
        };
        self.builtins
            .insert("NotImplemented".to_string(), not_implemented);
        self.builtins.insert(
            "locals".to_string(),
            Value::Builtin(BuiltinFunction::Locals),
        );
        self.builtins.insert(
            "globals".to_string(),
            Value::Builtin(BuiltinFunction::Globals),
        );
        self.builtins.insert(
            "getattr".to_string(),
            Value::Builtin(BuiltinFunction::GetAttr),
        );
        self.builtins.insert(
            "setattr".to_string(),
            Value::Builtin(BuiltinFunction::SetAttr),
        );
        self.builtins.insert(
            "delattr".to_string(),
            Value::Builtin(BuiltinFunction::DelAttr),
        );
        self.builtins.insert(
            "hasattr".to_string(),
            Value::Builtin(BuiltinFunction::HasAttr),
        );
        self.builtins.insert(
            "callable".to_string(),
            Value::Builtin(BuiltinFunction::Callable),
        );
        self.builtins.insert(
            "isinstance".to_string(),
            Value::Builtin(BuiltinFunction::IsInstance),
        );
        self.builtins.insert(
            "issubclass".to_string(),
            Value::Builtin(BuiltinFunction::IsSubclass),
        );
        self.builtins.insert(
            "reversed".to_string(),
            Value::Builtin(BuiltinFunction::Reversed),
        );
        self.builtins
            .insert("super".to_string(), Value::Builtin(BuiltinFunction::Super));
        self.builtins.insert(
            "__import__".to_string(),
            Value::Builtin(BuiltinFunction::Import),
        );
        self.builtins.insert(
            "compile".to_string(),
            Value::Builtin(BuiltinFunction::Compile),
        );
        self.builtins
            .insert("eval".to_string(), Value::Builtin(BuiltinFunction::Eval));
        self.builtins
            .insert("exec".to_string(), Value::Builtin(BuiltinFunction::Exec));
        self.builtins
            .insert("hash".to_string(), Value::Builtin(BuiltinFunction::Hash));
        self.builtins
            .insert("vars".to_string(), Value::Builtin(BuiltinFunction::Vars));
        self.builtins.insert(
            "breakpoint".to_string(),
            Value::Builtin(BuiltinFunction::Breakpoint),
        );
        self.builtins
            .insert("__debug__".to_string(), Value::Bool(true));
        self.builtins.insert(
            "BaseException".to_string(),
            Value::ExceptionType("BaseException".to_string()),
        );
        self.builtins.insert(
            "Exception".to_string(),
            Value::ExceptionType("Exception".to_string()),
        );
        self.builtins.insert(
            "GeneratorExit".to_string(),
            Value::ExceptionType("GeneratorExit".to_string()),
        );
        self.builtins.insert(
            "SystemExit".to_string(),
            Value::ExceptionType("SystemExit".to_string()),
        );
        self.builtins.insert(
            "KeyboardInterrupt".to_string(),
            Value::ExceptionType("KeyboardInterrupt".to_string()),
        );
        self.builtins.insert(
            "StopIteration".to_string(),
            Value::ExceptionType("StopIteration".to_string()),
        );
        self.builtins.insert(
            "StopAsyncIteration".to_string(),
            Value::ExceptionType("StopAsyncIteration".to_string()),
        );
        self.builtins.insert(
            "EOFError".to_string(),
            Value::ExceptionType("EOFError".to_string()),
        );
        self.builtins.insert(
            "MemoryError".to_string(),
            Value::ExceptionType("MemoryError".to_string()),
        );
        self.builtins.insert(
            "OverflowError".to_string(),
            Value::ExceptionType("OverflowError".to_string()),
        );
        self.builtins.insert(
            "RecursionError".to_string(),
            Value::ExceptionType("RecursionError".to_string()),
        );
        self.builtins.insert(
            "ReferenceError".to_string(),
            Value::ExceptionType("ReferenceError".to_string()),
        );
        self.builtins.insert(
            "SyntaxError".to_string(),
            Value::ExceptionType("SyntaxError".to_string()),
        );
        self.builtins.insert(
            "IndentationError".to_string(),
            Value::ExceptionType("IndentationError".to_string()),
        );
        self.builtins.insert(
            "TabError".to_string(),
            Value::ExceptionType("TabError".to_string()),
        );
        self.builtins.insert(
            "_IncompleteInputError".to_string(),
            Value::ExceptionType("_IncompleteInputError".to_string()),
        );
        self.builtins.insert(
            "BaseExceptionGroup".to_string(),
            Value::ExceptionType("BaseExceptionGroup".to_string()),
        );
        self.builtins.insert(
            "ExceptionGroup".to_string(),
            Value::ExceptionType("ExceptionGroup".to_string()),
        );
        self.builtins.insert(
            "ArithmeticError".to_string(),
            Value::ExceptionType("ArithmeticError".to_string()),
        );
        self.builtins.insert(
            "FloatingPointError".to_string(),
            Value::ExceptionType("FloatingPointError".to_string()),
        );
        self.builtins.insert(
            "ValueError".to_string(),
            Value::ExceptionType("ValueError".to_string()),
        );
        self.builtins.insert(
            "TypeError".to_string(),
            Value::ExceptionType("TypeError".to_string()),
        );
        self.builtins.insert(
            "IndexError".to_string(),
            Value::ExceptionType("IndexError".to_string()),
        );
        self.builtins.insert(
            "KeyError".to_string(),
            Value::ExceptionType("KeyError".to_string()),
        );
        self.builtins.insert(
            "AssertionError".to_string(),
            Value::ExceptionType("AssertionError".to_string()),
        );
        self.builtins.insert(
            "NameError".to_string(),
            Value::ExceptionType("NameError".to_string()),
        );
        self.builtins.insert(
            "UnboundLocalError".to_string(),
            Value::ExceptionType("UnboundLocalError".to_string()),
        );
        self.builtins.insert(
            "AttributeError".to_string(),
            Value::ExceptionType("AttributeError".to_string()),
        );
        self.builtins.insert(
            "ZeroDivisionError".to_string(),
            Value::ExceptionType("ZeroDivisionError".to_string()),
        );
        self.builtins.insert(
            "RuntimeError".to_string(),
            Value::ExceptionType("RuntimeError".to_string()),
        );
        self.builtins.insert(
            "PythonFinalizationError".to_string(),
            Value::ExceptionType("PythonFinalizationError".to_string()),
        );
        self.builtins.insert(
            "BufferError".to_string(),
            Value::ExceptionType("BufferError".to_string()),
        );
        self.builtins.insert(
            "OSError".to_string(),
            Value::ExceptionType("OSError".to_string()),
        );
        self.builtins.insert(
            "EnvironmentError".to_string(),
            Value::ExceptionType("OSError".to_string()),
        );
        self.builtins.insert(
            "IOError".to_string(),
            Value::ExceptionType("OSError".to_string()),
        );
        self.builtins.insert(
            "FileNotFoundError".to_string(),
            Value::ExceptionType("FileNotFoundError".to_string()),
        );
        self.builtins.insert(
            "FileExistsError".to_string(),
            Value::ExceptionType("FileExistsError".to_string()),
        );
        self.builtins.insert(
            "IsADirectoryError".to_string(),
            Value::ExceptionType("IsADirectoryError".to_string()),
        );
        self.builtins.insert(
            "BlockingIOError".to_string(),
            Value::ExceptionType("BlockingIOError".to_string()),
        );
        self.builtins.insert(
            "InterruptedError".to_string(),
            Value::ExceptionType("InterruptedError".to_string()),
        );
        self.builtins.insert(
            "ProcessLookupError".to_string(),
            Value::ExceptionType("ProcessLookupError".to_string()),
        );
        self.builtins.insert(
            "ChildProcessError".to_string(),
            Value::ExceptionType("ChildProcessError".to_string()),
        );
        self.builtins.insert(
            "ConnectionError".to_string(),
            Value::ExceptionType("ConnectionError".to_string()),
        );
        self.builtins.insert(
            "BrokenPipeError".to_string(),
            Value::ExceptionType("BrokenPipeError".to_string()),
        );
        self.builtins.insert(
            "ConnectionAbortedError".to_string(),
            Value::ExceptionType("ConnectionAbortedError".to_string()),
        );
        self.builtins.insert(
            "ConnectionRefusedError".to_string(),
            Value::ExceptionType("ConnectionRefusedError".to_string()),
        );
        self.builtins.insert(
            "ConnectionResetError".to_string(),
            Value::ExceptionType("ConnectionResetError".to_string()),
        );
        self.builtins.insert(
            "TimeoutError".to_string(),
            Value::ExceptionType("TimeoutError".to_string()),
        );
        self.builtins.insert(
            "NotADirectoryError".to_string(),
            Value::ExceptionType("NotADirectoryError".to_string()),
        );
        self.builtins.insert(
            "PermissionError".to_string(),
            Value::ExceptionType("PermissionError".to_string()),
        );
        self.builtins.insert(
            "NotImplementedError".to_string(),
            Value::ExceptionType("NotImplementedError".to_string()),
        );
        self.builtins.insert(
            "SystemError".to_string(),
            Value::ExceptionType("SystemError".to_string()),
        );
        self.builtins.insert(
            "Warning".to_string(),
            Value::ExceptionType("Warning".to_string()),
        );
        self.builtins.insert(
            "UserWarning".to_string(),
            Value::ExceptionType("UserWarning".to_string()),
        );
        self.builtins.insert(
            "DeprecationWarning".to_string(),
            Value::ExceptionType("DeprecationWarning".to_string()),
        );
        self.builtins.insert(
            "PendingDeprecationWarning".to_string(),
            Value::ExceptionType("PendingDeprecationWarning".to_string()),
        );
        self.builtins.insert(
            "RuntimeWarning".to_string(),
            Value::ExceptionType("RuntimeWarning".to_string()),
        );
        self.builtins.insert(
            "SyntaxWarning".to_string(),
            Value::ExceptionType("SyntaxWarning".to_string()),
        );
        self.builtins.insert(
            "FutureWarning".to_string(),
            Value::ExceptionType("FutureWarning".to_string()),
        );
        self.builtins.insert(
            "ImportWarning".to_string(),
            Value::ExceptionType("ImportWarning".to_string()),
        );
        self.builtins.insert(
            "UnicodeWarning".to_string(),
            Value::ExceptionType("UnicodeWarning".to_string()),
        );
        self.builtins.insert(
            "EncodingWarning".to_string(),
            Value::ExceptionType("EncodingWarning".to_string()),
        );
        self.builtins.insert(
            "UnicodeError".to_string(),
            Value::ExceptionType("UnicodeError".to_string()),
        );
        self.builtins.insert(
            "UnicodeEncodeError".to_string(),
            Value::ExceptionType("UnicodeEncodeError".to_string()),
        );
        self.builtins.insert(
            "UnicodeDecodeError".to_string(),
            Value::ExceptionType("UnicodeDecodeError".to_string()),
        );
        self.builtins.insert(
            "UnicodeTranslateError".to_string(),
            Value::ExceptionType("UnicodeTranslateError".to_string()),
        );
        self.builtins.insert(
            "BytesWarning".to_string(),
            Value::ExceptionType("BytesWarning".to_string()),
        );
        self.builtins.insert(
            "ResourceWarning".to_string(),
            Value::ExceptionType("ResourceWarning".to_string()),
        );
        self.builtins.insert(
            "ImportError".to_string(),
            Value::ExceptionType("ImportError".to_string()),
        );
        self.builtins.insert(
            "LookupError".to_string(),
            Value::ExceptionType("LookupError".to_string()),
        );
        self.builtins.insert(
            "ModuleNotFoundError".to_string(),
            Value::ExceptionType("ModuleNotFoundError".to_string()),
        );
        self.builtins.insert(
            "StopIteration".to_string(),
            Value::ExceptionType("StopIteration".to_string()),
        );
        self.builtins.insert(
            "StopAsyncIteration".to_string(),
            Value::ExceptionType("StopAsyncIteration".to_string()),
        );
        self.builtins.insert(
            "SystemExit".to_string(),
            Value::ExceptionType("SystemExit".to_string()),
        );
        self.builtins.insert(
            "KeyboardInterrupt".to_string(),
            Value::ExceptionType("KeyboardInterrupt".to_string()),
        );
        self.builtins.insert(
            "BaseExceptionGroup".to_string(),
            Value::ExceptionType("BaseExceptionGroup".to_string()),
        );
        self.builtins.insert(
            "ExceptionGroup".to_string(),
            Value::ExceptionType("ExceptionGroup".to_string()),
        );
        self.builtins.insert(
            "GeneratorExit".to_string(),
            Value::ExceptionType("GeneratorExit".to_string()),
        );
        self.touch_builtins_version();
    }

    pub(super) fn call_build_class(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "__build_class__ expects at least a function and a name",
            ));
        }
        let metaclass = kwargs.remove("metaclass");
        let name = match args.remove(1) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("class name must be a string")),
        };
        let func = match args.remove(0) {
            Value::Function(func) => func,
            _ => return Err(RuntimeError::new("class body must be a function")),
        };
        let func_data = match &*func.kind() {
            Object::Function(data) => data.clone(),
            _ => return Err(RuntimeError::new("class body must be a function")),
        };
        let orig_bases_tuple = self.heap.alloc_tuple(args.clone());
        let trace_build_class = self.trace_flags.build_class;
        let trace_this_class = trace_build_class && name == "_TagInfo";
        let mut resolved_bases = Vec::new();
        let mut used_mro_entries = false;
        for base in args {
            let maybe_mro_entries = if matches!(base, Value::Class(_)) {
                None
            } else {
                match self.builtin_getattr(
                    vec![base.clone(), Value::Str("__mro_entries__".to_string())],
                    HashMap::new(),
                ) {
                    Ok(callable) => Some(callable),
                    Err(err) if runtime_error_matches_exception(&err, "AttributeError") => None,
                    Err(err) => return Err(err),
                }
            };
            if let Some(mro_entries) = maybe_mro_entries {
                used_mro_entries = true;
                let entries = match self.call_internal(
                    mro_entries,
                    vec![orig_bases_tuple.clone()],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("__mro_entries__ call failed")
                        );
                    }
                };
                let Value::Tuple(entries_tuple) = entries else {
                    return Err(RuntimeError::type_error(
                        "__mro_entries__ must return a tuple",
                    ));
                };
                let Object::Tuple(items) = &*entries_tuple.kind() else {
                    return Err(RuntimeError::type_error(
                        "__mro_entries__ must return a tuple",
                    ));
                };
                resolved_bases.extend(items.iter().cloned());
            } else {
                resolved_bases.push(base);
            }
        }
        if trace_this_class {
            let base_tags = resolved_bases
                .iter()
                .map(format_repr)
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("[build-class] name={} resolved_bases=[{}]", name, base_tags);
        }

        let mut base_classes = Vec::new();
        for base in resolved_bases {
            match self.class_from_base_value(base.clone()) {
                Ok(class) => base_classes.push(class),
                Err(err) => {
                    if self.trace_flags.class_base
                        && runtime_error_matches_exception(&err, "TypeError")
                    {
                        eprintln!("[class-base] __build_class__ base={}", format_repr(&base));
                    }
                    return Err(err);
                }
            }
        }
        if trace_this_class {
            let base_names = base_classes
                .iter()
                .map(|class| match &*class.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<non-class>".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("[build-class] name={} base_classes=[{}]", name, base_names);
        }

        let class_metaclass = metaclass.filter(|value| !matches!(value, Value::None));
        let resolved_metaclass =
            self.resolve_class_metaclass(&base_classes, class_metaclass.as_ref())?;
        let effective_metaclass = class_metaclass
            .clone()
            .or_else(|| resolved_metaclass.map(Value::Class));
        let mut prepared_namespace = self.heap.alloc_dict(Vec::new());
        if let Some(meta) = effective_metaclass {
            let prepare_callable = match self.builtin_getattr(
                vec![meta.clone(), Value::Str("__prepare__".to_string())],
                HashMap::new(),
            ) {
                Ok(value) => Some(value),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => None,
                Err(err) => return Err(err),
            };
            if let Some(prepare_callable) = prepare_callable {
                if self.trace_flags.prepare_call {
                    let callable_type = self.value_type_name_for_error(&prepare_callable);
                    let callable_repr = format_repr(&prepare_callable);
                    let meta_name = match &meta {
                        Value::Class(class_ref) => match &*class_ref.kind() {
                            Object::Class(data) => data.name.clone(),
                            _ => "<non-class>".to_string(),
                        },
                        _ => "<metaclass>".to_string(),
                    };
                    eprintln!(
                        "[prepare-call] class={} meta={} callable_type={} callable={}",
                        name, meta_name, callable_type, callable_repr
                    );
                }
                let bases_tuple = self.heap.alloc_tuple(
                    base_classes
                        .iter()
                        .cloned()
                        .map(Value::Class)
                        .collect::<Vec<_>>(),
                );
                prepared_namespace = match self.call_internal(
                    prepare_callable,
                    vec![Value::Str(name.clone()), bases_tuple],
                    kwargs.clone(),
                )? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(self.runtime_error_from_active_exception(
                            "metaclass __prepare__ call failed",
                        ));
                    }
                };
                if self
                    .class_namespace_backing_dict(&prepared_namespace)
                    .is_none()
                {
                    let meta_name = match &meta {
                        Value::Class(class_ref) => match &*class_ref.kind() {
                            Object::Class(class_data) => class_data.name.clone(),
                            _ => "<metaclass>".to_string(),
                        },
                        _ => "<metaclass>".to_string(),
                    };
                    return Err(RuntimeError::type_error(format!(
                        "{}.__prepare__() must return a mapping, not {}",
                        meta_name,
                        self.value_type_name_for_error(&prepared_namespace)
                    )));
                }
            }
        }
        let class_declared_global = self
            .frames
            .last()
            .is_some_and(|frame| self.class_assignment_is_global(frame, &name));
        let class_qualname = self
            .frames
            .last()
            .and_then(|frame| {
                if frame.return_class {
                    let Object::Module(module_data) = &*frame.module.kind() else {
                        return None;
                    };
                    let outer_qualname = module_data
                        .globals
                        .get("__qualname__")
                        .and_then(|value| match value {
                            Value::Str(name) => Some(name.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| module_data.name.clone());
                    return Some(format!("{outer_qualname}.{name}"));
                }
                if frame.is_module || class_declared_global {
                    return None;
                }
                let mut outer_qualname = frame.code.name.clone();
                let owner_value = frame
                    .locals
                    .get("self")
                    .cloned()
                    .or_else(|| frame.locals.get("cls").cloned())
                    .or_else(|| {
                        frame
                            .code
                            .names
                            .iter()
                            .position(|entry| entry == "self" || entry == "cls")
                            .and_then(|idx| frame.fast_locals.get(idx))
                            .and_then(|slot| slot.clone())
                    });
                if let Some(owner) = owner_value {
                    match owner {
                        Value::Instance(instance) => {
                            if let Object::Instance(instance_data) = &*instance.kind()
                                && let Object::Class(class_data) = &*instance_data.class.kind()
                            {
                                outer_qualname = format!("{}.{}", class_data.name, outer_qualname);
                            }
                        }
                        Value::Class(class) => {
                            if let Object::Class(class_data) = &*class.kind() {
                                outer_qualname = format!("{}.{}", class_data.name, outer_qualname);
                            }
                        }
                        _ => {}
                    }
                }
                Some(format!("{outer_qualname}.<locals>.{name}"))
            })
            .unwrap_or_else(|| name.clone());
        let class_module = match self.heap.alloc_module(ModuleObject::new(name.clone())) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *class_module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name.clone()));
            module_data
                .globals
                .insert("__qualname__".to_string(), Value::Str(class_qualname));
        }

        let outer_globals = func_data.module.clone();
        let outer_locals = self
            .frames
            .last()
            .and_then(|frame| Self::class_lookup_fallback_from_frame(frame));
        let cells = self.build_cells(&func_data.code, func_data.closure.clone());
        let mut frame = Frame::new(
            func_data.code.clone(),
            class_module,
            true,
            false,
            cells,
            None,
        );
        frame.function_globals = outer_globals.clone();
        frame.function_globals_version = module_globals_version(&outer_globals);
        frame.globals_fallback = Some(outer_globals);
        frame.locals_fallback = outer_locals;
        frame.class_namespace = Some(prepared_namespace.clone());
        frame.module_locals_dict = self.class_namespace_backing_dict(&prepared_namespace);
        frame.locals.insert(
            "__classdict__".to_string(),
            self.heap.alloc_dict(Vec::new()),
        );
        frame.return_class = true;
        frame.class_bases = base_classes;
        frame.class_orig_bases = if used_mro_entries {
            Some(orig_bases_tuple)
        } else {
            None
        };
        frame.class_metaclass = class_metaclass;
        frame.class_keywords = kwargs;
        self.push_frame_checked(Box::new(frame))?;
        Ok(None)
    }

    pub(super) fn alloc_synthetic_class(&mut self, name: &str) -> ObjRef {
        match self
            .heap
            .alloc_class(ClassObject::new(name.to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        }
    }

    pub(super) fn alloc_synthetic_exception_class(&mut self, name: &str) -> ObjRef {
        if let Some(existing) = self.synthetic_exception_classes.get(name).cloned() {
            return existing;
        }

        let bases = if let Some(parent_name) = builtin_exception_parent(name) {
            vec![self.alloc_synthetic_exception_class(parent_name)]
        } else if let Some(Value::Class(object_class)) = self.builtins.get("object") {
            vec![object_class.clone()]
        } else {
            Vec::new()
        };

        let class = match self
            .heap
            .alloc_class(ClassObject::new(name.to_string(), bases.clone()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        let mro = self.build_class_mro(&class, &bases).unwrap_or_else(|_| {
            let mut fallback = vec![class.clone()];
            fallback.extend(bases.iter().cloned());
            fallback
        });
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.bases = bases;
            class_data.mro = mro;
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("builtins".to_string()));
            class_data
                .attrs
                .insert("__qualname__".to_string(), Value::Str(name.to_string()));
            class_data.attrs.insert(
                "__new__".to_string(),
                Value::Builtin(BuiltinFunction::ObjectNew),
            );
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ExceptionTypeInit),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::ExceptionTypeStr),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::ExceptionTypeRepr),
            );
        }
        self.synthetic_exception_classes
            .insert(name.to_string(), class.clone());
        class
    }

    fn alloc_synthetic_reprenum_data_class(&mut self, name: &str) -> ObjRef {
        let class = self.synthetic_builtin_class(name);
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            let new_builtin = match name {
                "int" => BuiltinFunction::Int,
                "float" => BuiltinFunction::Float,
                _ => BuiltinFunction::ObjectNew,
            };
            class_data
                .attrs
                .entry("__new__".to_string())
                .or_insert(Value::Builtin(new_builtin));
        }
        class
    }

    fn mark_synthetic_static_type_with_module(&mut self, class: &ObjRef, module: &str, name: &str) {
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data
                .attrs
                .entry("__name__".to_string())
                .or_insert_with(|| Value::Str(name.to_string()));
            class_data
                .attrs
                .entry("__qualname__".to_string())
                .or_insert_with(|| Value::Str(name.to_string()));
            class_data
                .attrs
                .entry("__module__".to_string())
                .or_insert_with(|| Value::Str(module.to_string()));
            // These stand in for static builtin types in class-base resolution paths.
            // Mark them as non-heap so copyreg._reduce_ex and similar stdlib logic
            // treat them like CPython static types.
            class_data
                .attrs
                .insert("__flags__".to_string(), Value::Int(0));
        }
    }

    pub(super) fn synthetic_runtime_type_class(&mut self, module: &str, name: &str) -> ObjRef {
        let cache_key = if module == "builtins" {
            name.to_string()
        } else {
            format!("{module}.{name}")
        };
        if let Some(existing) = self.synthetic_builtin_classes.get(&cache_key).cloned() {
            return existing;
        }
        let class = self.alloc_synthetic_class(name);
        self.mark_synthetic_static_type_with_module(&class, module, name);

        let object_base = self.builtins.get("object").and_then(|value| match value {
            Value::Class(class) => Some(class.clone()),
            _ => None,
        });
        let explicit_bases = match (module, name) {
            ("collections", "Counter" | "OrderedDict" | "defaultdict") => {
                vec![self.synthetic_builtin_class("dict")]
            }
            _ => Vec::new(),
        };
        let default_meta = self.default_type_metaclass();
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            if class_data.bases.is_empty()
                && name != "object"
                && name != "type"
            {
                if !explicit_bases.is_empty() {
                    class_data.bases.extend(explicit_bases);
                } else if let Some(base) = object_base {
                    class_data.bases.push(base);
                }
            }
            if class_data.metaclass.is_none() {
                class_data.metaclass = default_meta;
            }
        }

        let bases = match &*class.kind() {
            Object::Class(class_data) => class_data.bases.clone(),
            _ => Vec::new(),
        };
        let mro = if bases.is_empty() {
            vec![class.clone()]
        } else {
            self.build_class_mro(&class, &bases).unwrap_or_else(|_| {
                let mut fallback = vec![class.clone()];
                fallback.extend(bases.iter().cloned());
                fallback
            })
        };
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.mro = mro.clone();
            class_data.attrs.insert(
                "__bases__".to_string(),
                self.heap
                    .alloc_tuple(bases.iter().cloned().map(Value::Class).collect::<Vec<_>>()),
            );
            class_data.attrs.insert(
                "__mro__".to_string(),
                self.heap
                    .alloc_tuple(mro.into_iter().map(Value::Class).collect::<Vec<_>>()),
            );
        }

        self.synthetic_builtin_classes
            .insert(cache_key, class.clone());
        class
    }

    fn synthetic_builtin_class(&mut self, name: &str) -> ObjRef {
        self.synthetic_runtime_type_class("builtins", name)
    }

    pub(super) fn class_from_base_value(&mut self, base: Value) -> Result<ObjRef, RuntimeError> {
        fn typing_non_base_name_from_class(class: &ObjRef) -> Option<String> {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return None;
            };
            let module_name = match class_data.attrs.get("__module__") {
                Some(Value::Str(module_name)) => module_name.as_str(),
                _ => return None,
            };
            if !matches!(module_name, "typing" | "_typing") {
                return None;
            }
            if matches!(
                class_data.name.as_str(),
                "TypeVar"
                    | "TypeVarTuple"
                    | "ParamSpec"
                    | "ParamSpecArgs"
                    | "ParamSpecKwargs"
                    | "Union"
            ) {
                return Some(class_data.name.clone());
            }
            None
        }

        fn typing_non_base_name_from_instance(instance: &ObjRef) -> Option<String> {
            let instance_kind = instance.kind();
            let Object::Instance(instance_data) = &*instance_kind else {
                return None;
            };
            typing_non_base_name_from_class(&instance_data.class)
        }

        match base {
            Value::Class(class) => {
                if let Some(name) = typing_non_base_name_from_class(&class) {
                    return Err(RuntimeError::type_error(format!(
                        "type 'typing.{name}' is not an acceptable base type"
                    )));
                }
                Ok(class)
            }
            Value::Instance(instance) => {
                if let Some(name) = typing_non_base_name_from_instance(&instance) {
                    if name == "Union" {
                        let repr_text = match self
                            .builtin_repr(vec![Value::Instance(instance.clone())], HashMap::new())
                        {
                            Ok(Value::Str(text)) => text,
                            _ => "typing.Union".to_string(),
                        };
                        return Err(RuntimeError::type_error(format!(
                            "Cannot subclass {repr_text}"
                        )));
                    }
                    return Err(RuntimeError::type_error(format!(
                        "Cannot subclass an instance of {name}"
                    )));
                }
                // CPython-extension proxy types can flow through class creation as proxy
                // instances when metatype detection is incomplete. Treat those as class-like
                // bases so stdlib/native class statements do not fail at compile/runtime.
                if let Object::Instance(instance_data) = &*instance.kind()
                    && let Object::Class(class_data) = &*instance_data.class.kind()
                {
                    if matches!(
                        class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                        Some(Value::Bool(true))
                    ) {
                        return Ok(instance_data.class.clone());
                    }
                    if class_data.name == "GenericAlias"
                        && let Some(origin) = instance_data.attrs.get("__origin__").cloned()
                    {
                        return self.class_from_base_value(origin);
                    }
                }
                Err(RuntimeError::type_error("bases must be types"))
            }
            Value::ExceptionType(name) => Ok(self.alloc_synthetic_exception_class(&name)),
            Value::Builtin(BuiltinFunction::TypingTypeVar) => Err(RuntimeError::type_error(
                "type 'typing.TypeVar' is not an acceptable base type",
            )),
            Value::Builtin(BuiltinFunction::TypingTypeVarTuple) => Err(RuntimeError::type_error(
                "type 'typing.TypeVarTuple' is not an acceptable base type",
            )),
            Value::Builtin(BuiltinFunction::TypingParamSpec) => Err(RuntimeError::type_error(
                "type 'typing.ParamSpec' is not an acceptable base type",
            )),
            Value::Builtin(BuiltinFunction::Type) => Ok(self
                .default_type_metaclass()
                .unwrap_or_else(|| self.synthetic_builtin_class("type"))),
            Value::Builtin(BuiltinFunction::Bool) => Ok(self.synthetic_builtin_class("bool")),
            Value::Builtin(BuiltinFunction::Int) => {
                Ok(self.alloc_synthetic_reprenum_data_class("int"))
            }
            Value::Builtin(BuiltinFunction::Float) => {
                Ok(self.alloc_synthetic_reprenum_data_class("float"))
            }
            Value::Builtin(BuiltinFunction::Str) => {
                Ok(self.alloc_synthetic_reprenum_data_class("str"))
            }
            Value::Builtin(BuiltinFunction::List) => Ok(self.synthetic_builtin_class("list")),
            Value::Builtin(BuiltinFunction::Tuple) => Ok(self.synthetic_builtin_class("tuple")),
            Value::Builtin(BuiltinFunction::Dict) => {
                let class = self.synthetic_builtin_class("dict");
                if let Object::Class(class_data) = &mut *class.kind_mut()
                    && !class_data.attrs.contains_key("fromkeys")
                {
                    class_data.attrs.insert(
                        "fromkeys".to_string(),
                        self.alloc_builtin_unbound_method(
                            "__dict_unbound_method__",
                            Value::Class(class.clone()),
                            BuiltinFunction::DictFromKeys,
                        ),
                    );
                }
                Ok(class)
            }
            Value::Builtin(BuiltinFunction::Set) => {
                let class = self.synthetic_builtin_class("set");
                if let Object::Class(class_data) = &mut *class.kind_mut()
                    && !class_data.attrs.contains_key("__reduce__")
                {
                    class_data.attrs.insert(
                        "__reduce__".to_string(),
                        Value::Builtin(BuiltinFunction::SetReduce),
                    );
                }
                Ok(class)
            }
            Value::Builtin(BuiltinFunction::FrozenSet) => {
                let class = self.synthetic_builtin_class("frozenset");
                if let Object::Class(class_data) = &mut *class.kind_mut()
                    && !class_data.attrs.contains_key("__reduce__")
                {
                    class_data.attrs.insert(
                        "__reduce__".to_string(),
                        Value::Builtin(BuiltinFunction::SetReduce),
                    );
                }
                Ok(class)
            }
            Value::Builtin(BuiltinFunction::Enumerate) => {
                Ok(self.synthetic_builtin_class("enumerate"))
            }
            Value::Builtin(BuiltinFunction::Bytes) => Ok(self.synthetic_builtin_class("bytes")),
            Value::Builtin(BuiltinFunction::ByteArray) => {
                Ok(self.synthetic_builtin_class("bytearray"))
            }
            Value::Builtin(BuiltinFunction::ArrayArray) => {
                Ok(self.synthetic_runtime_type_class("array", "array"))
            }
            Value::Builtin(BuiltinFunction::MemoryView) => {
                Ok(self.synthetic_builtin_class("memoryview"))
            }
            Value::Builtin(BuiltinFunction::Complex) => Ok(self.synthetic_builtin_class("complex")),
            Value::Builtin(BuiltinFunction::TypesModuleType) => {
                let module_class = self.synthetic_builtin_class("module");
                if let Object::Class(class_data) = &mut *module_class.kind_mut()
                    && !class_data.attrs.contains_key("__init__")
                {
                    class_data.attrs.insert(
                        "__init__".to_string(),
                        Value::Builtin(BuiltinFunction::TypesModuleType),
                    );
                }
                Ok(module_class)
            }
            Value::Builtin(BuiltinFunction::ClassMethod) => {
                Ok(self.synthetic_builtin_class("classmethod"))
            }
            Value::Builtin(BuiltinFunction::StaticMethod) => {
                Ok(self.synthetic_builtin_class("staticmethod"))
            }
            Value::Builtin(BuiltinFunction::Property) => {
                Ok(self.synthetic_builtin_class("property"))
            }
            Value::Builtin(BuiltinFunction::FunctoolsPartial) => {
                Ok(self.synthetic_builtin_class("partial"))
            }
            Value::Builtin(BuiltinFunction::CollectionsCounter) => {
                Ok(self.synthetic_runtime_type_class("collections", "Counter"))
            }
            Value::Builtin(BuiltinFunction::CollectionsDeque) => {
                Ok(self.synthetic_runtime_type_class("collections", "deque"))
            }
            Value::Builtin(BuiltinFunction::CollectionsDefaultDict) => {
                Ok(self.synthetic_runtime_type_class("collections", "defaultdict"))
            }
            Value::Builtin(BuiltinFunction::CollectionsOrderedDict) => {
                Ok(self.synthetic_runtime_type_class("collections", "OrderedDict"))
            }
            other => {
                if self.trace_flags.class_base {
                    eprintln!(
                        "[class-base] unsupported base value={}",
                        format_repr(&other)
                    );
                }
                let _ = other;
                Err(RuntimeError::type_error("bases must be types"))
            }
        }
    }

    pub(super) fn class_base_values_from_value(
        &self,
        bases_value: &Value,
    ) -> Result<Vec<Value>, RuntimeError> {
        match bases_value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(values) => Ok(values.clone()),
                _ => Err(RuntimeError::type_error("bases must be types")),
            },
            Value::List(list_obj) => match &*list_obj.kind() {
                Object::List(values) => Ok(values.clone()),
                _ => Err(RuntimeError::type_error("bases must be types")),
            },
            Value::Instance(instance) => {
                if let Some(backing_tuple) = self.instance_backing_tuple(instance)
                    && let Object::Tuple(values) = &*backing_tuple.kind()
                {
                    return Ok(values.clone());
                }
                if let Some(backing_list) = self.instance_backing_list(instance)
                    && let Object::List(values) = &*backing_list.kind()
                {
                    return Ok(values.clone());
                }
                Err(RuntimeError::type_error("bases must be types"))
            }
            _ => Err(RuntimeError::type_error("bases must be types")),
        }
    }

    pub(super) fn ensure_unique_base_classes(&self, bases: &[ObjRef]) -> Result<(), RuntimeError> {
        for (idx, base) in bases.iter().enumerate() {
            if bases
                .iter()
                .skip(idx + 1)
                .any(|other| other.id() == base.id())
            {
                let base_name = match &*base.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "object".to_string(),
                };
                return Err(RuntimeError::type_error(format!(
                    "duplicate base class {}",
                    base_name
                )));
            }
        }
        Ok(())
    }

    pub(super) fn update_class_bases_attr(
        &mut self,
        class: &ObjRef,
        bases_value: Value,
    ) -> Result<(), RuntimeError> {
        let base_values = self.class_base_values_from_value(&bases_value)?;
        let mut bases = Vec::with_capacity(base_values.len());
        for base in base_values {
            bases.push(self.class_from_base_value(base)?);
        }
        self.ensure_unique_base_classes(&bases)?;
        let mro = if bases.is_empty() {
            vec![class.clone()]
        } else {
            self.build_class_mro(class, &bases).unwrap_or_else(|_| {
                let mut fallback = vec![class.clone()];
                fallback.extend(bases.iter().cloned());
                fallback
            })
        };
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.bases = bases.clone();
            class_data.mro = mro.clone();
            class_data
                .attrs
                .insert("__bases__".to_string(), bases_value);
            class_data.attrs.insert(
                "__mro__".to_string(),
                self.heap
                    .alloc_tuple(mro.into_iter().map(Value::Class).collect::<Vec<_>>()),
            );
        }
        Ok(())
    }
}

fn tuple_is_typing_alias_shape(values: &[Value]) -> bool {
    values.iter().any(typing_alias_marker_value)
}

fn typing_alias_index_shape(value: &Value) -> bool {
    match value {
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(items) => items.iter().all(typing_alias_operand_value),
            _ => false,
        },
        other => typing_alias_operand_value(other),
    }
}

fn typing_alias_operand_value(value: &Value) -> bool {
    match value {
        Value::None | Value::Class(_) | Value::ExceptionType(_) => true,
        Value::Builtin(_) => true,
        Value::List(obj) => match &*obj.kind() {
            Object::List(items) => items.iter().all(typing_alias_operand_value),
            _ => false,
        },
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(items) => items.iter().all(typing_alias_operand_value),
            _ => false,
        },
        Value::Instance(instance) => typing_alias_marker_instance(instance),
        _ => false,
    }
}

fn typing_alias_marker_value(value: &Value) -> bool {
    match value {
        Value::Instance(instance) => typing_alias_marker_instance(instance),
        Value::Tuple(obj) => match &*obj.kind() {
            Object::Tuple(items) => items.iter().any(typing_alias_marker_value),
            _ => false,
        },
        _ => false,
    }
}

fn typing_alias_marker_instance(instance: &ObjRef) -> bool {
    let (class_name, module_name) = match &*instance.kind() {
        Object::Instance(instance_data) => match &*instance_data.class.kind() {
            Object::Class(class_data) => {
                let module_name = match class_data.attrs.get("__module__") {
                    Some(Value::Str(name)) => Some(name.clone()),
                    _ => None,
                };
                (class_data.name.clone(), module_name)
            }
            _ => return false,
        },
        _ => return false,
    };
    if matches!(
        class_name.as_str(),
        "GenericAlias"
            | "_GenericAlias"
            | "UnionType"
            | "TypeVar"
            | "TypeVarTuple"
            | "ParamSpec"
            | "TypeAliasType"
            | "ForwardRef"
    ) {
        return true;
    }
    if matches!(module_name.as_deref(), Some("typing" | "_typing" | "types"))
        && (class_name.contains("GenericAlias")
            || class_name.contains("SpecialForm")
            || class_name.contains("SpecialGenericAlias")
            || class_name.contains("LiteralGenericAlias")
            || matches!(
                class_name.as_str(),
                "Union"
                    | "NewType"
                    | "_SpecialForm"
                    | "_TypedCacheSpecialForm"
                    | "_AnyMeta"
                    | "_TupleType"
                    | "_TypingEllipsis"
            ))
    {
        return true;
    }
    false
}

fn typing_typevartuple_param_marker(value: &Value) -> bool {
    let Value::Instance(instance) = value else {
        return false;
    };
    let Object::Instance(instance_data) = &*instance.kind() else {
        return false;
    };
    let Object::Class(class_data) = &*instance_data.class.kind() else {
        return false;
    };
    if class_data.name != "TypeVarTuple" {
        return false;
    }
    matches!(
        class_data.attrs.get("__module__"),
        Some(Value::Str(module)) if matches!(module.as_str(), "typing" | "_typing")
    )
}
