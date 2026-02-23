use super::{
    AttrAccessOutcome, BigInt, BuiltinFunction, ClassObject, Frame, GeneratorResumeOutcome,
    HashMap, InstanceObject, InternalCallOutcome, IteratorKind, ModuleObject, NativeMethodKind,
    ObjRef, Object, Ordering, RuntimeError, Value, Vm, builtin_exception_parent, class_attr_lookup,
    dict_get_value, dict_set_value_checked, ensure_hashable, format_repr, memoryview_bounds,
    memoryview_decode_element, memoryview_element_offset, memoryview_format_for_view,
    memoryview_layout_1d, memoryview_logical_nbytes, memoryview_shape_and_strides_from_parts,
    module_globals_version, runtime_error_matches_exception, slice_bounds_for_step_one,
    slice_indices, value_from_bigint, value_to_bytes_payload, value_to_int, with_bytes_like_source,
};
use crate::runtime::SliceValue;

impl Vm {
    pub(super) fn builtin_warnings_filters_mutated(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_filters_mutated() expects no arguments"));
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
        let ident = self.builtin_threading_get_ident(Vec::new(), HashMap::new())?;
        let info = match self
            .heap
            .alloc_module(ModuleObject::new("__thread_info__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *info.kind_mut() {
            module_data
                .globals
                .insert("name".to_string(), Value::Str(name.to_string()));
            module_data.globals.insert("ident".to_string(), ident);
            module_data
                .globals
                .insert("daemon".to_string(), Value::Bool(false));
        }
        Ok(Value::Module(info))
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

    pub(super) fn getitem_value(
        &mut self,
        value: Value,
        index: Value,
    ) -> Result<Value, RuntimeError> {
        if let Value::Instance(instance) = &value {
            if let Some(values) = self.namedtuple_instance_values(instance) {
                return self.getitem_value(self.heap.alloc_tuple(values), index);
            }
            if let Some(backing_list) = self.instance_backing_list(instance) {
                return self.getitem_value(Value::List(backing_list), index);
            }
            if let Some(backing_tuple) = self.instance_backing_tuple(instance) {
                return self.getitem_value(Value::Tuple(backing_tuple), index);
            }
            if let Some(backing_str) = self.instance_backing_str(instance) {
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
                ensure_hashable(&index)?;
                if let Some(value) = dict_get_value(&backing_dict, &index) {
                    return Ok(value);
                }
                let receiver_value = Value::Instance(instance.clone());
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
                                if std::env::var_os("PYRS_TRACE_GETITEM_INDEX").is_some() {
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
                                if std::env::var_os("PYRS_TRACE_GETITEM_INDEX").is_some() {
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
                            if std::env::var_os("PYRS_TRACE_GETITEM_INDEX").is_some() {
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
                    ensure_hashable(&index)?;
                    let existing = dict_get_value(&obj, &index);
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
                        dict_set_value_checked(&obj, index, generated.clone())?;
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
                    let origin = Value::Class(class.clone());
                    Ok(self.alloc_generic_alias_instance(origin, index))
                }
                other => {
                    if typing_alias_marker_value(&other) && typing_alias_index_shape(&index) {
                        // Typing/generic alias marker instances can be re-subscripted while
                        // building annotation metadata in scientific-stack imports.
                        // Preserve the symbolic alias object instead of treating it as a
                        // concrete runtime container.
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
                        if std::env::var_os("PYRS_TRACE_GETITEM_UNSUPPORTED").is_some() {
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
        }
        self.synthetic_builtin_classes
            .insert(CACHE_KEY.to_string(), class.clone());
        class
    }

    pub(super) fn alloc_generic_alias_instance(&mut self, origin: Value, index: Value) -> Value {
        let alias_class = self.ensure_generic_alias_class();
        let alias = self.heap.alloc_instance(InstanceObject::new(alias_class));
        if let Value::Instance(instance) = &alias
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data
                .attrs
                .insert("__origin__".to_string(), origin.clone());
            let args = match index {
                Value::Tuple(tuple_obj) => Value::Tuple(tuple_obj),
                value => self.heap.alloc_tuple(vec![value]),
            };
            instance_data.attrs.insert("__args__".to_string(), args);
        }
        alias
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
                if std::env::var("PYRS_DEBUG_EXPECTED_ITERABLE").is_ok() {
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
                Value::Builtin(BuiltinFunction::OperatorEq),
            );
            class_data.attrs.insert(
                "__ne__".to_string(),
                Value::Builtin(BuiltinFunction::OperatorNe),
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
                .insert("__str__".to_string(), Value::Builtin(BuiltinFunction::Repr));
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
        let trace_build_class = std::env::var_os("PYRS_TRACE_BUILD_CLASS").is_some();
        let trace_this_class = trace_build_class && name == "_TagInfo";
        let mut resolved_bases = Vec::new();
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
                    if std::env::var_os("PYRS_TRACE_CLASS_BASE").is_some()
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
        if let Some(Value::Class(meta_class)) = effective_metaclass
            && class_attr_lookup(&meta_class, "__prepare__").is_some()
        {
            let prepare_callable = match self.load_attr_class(&meta_class, "__prepare__")? {
                AttrAccessOutcome::Value(value) => value,
                AttrAccessOutcome::ExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception(
                        "metaclass __prepare__ lookup failed",
                    ));
                }
            };
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
                    return Err(self
                        .runtime_error_from_active_exception("metaclass __prepare__ call failed"));
                }
            };
            if self
                .class_namespace_backing_dict(&prepared_namespace)
                .is_none()
            {
                return Err(RuntimeError::new(
                    "metaclass __prepare__() must return a mapping",
                ));
            }
        }
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
                .insert("__qualname__".to_string(), Value::Str(name));
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
        frame.class_metaclass = class_metaclass;
        frame.class_keywords = kwargs;
        self.frames.push(Box::new(frame));
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

    fn mark_synthetic_static_type(&mut self, class: &ObjRef, name: &str) {
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
                .or_insert_with(|| Value::Str("builtins".to_string()));
            // These stand in for static builtin types in class-base resolution paths.
            // Mark them as non-heap so copyreg._reduce_ex and similar stdlib logic
            // treat them like CPython static types.
            class_data
                .attrs
                .insert("__flags__".to_string(), Value::Int(0));
        }
    }

    fn synthetic_builtin_class(&mut self, name: &str) -> ObjRef {
        if let Some(existing) = self.synthetic_builtin_classes.get(name).cloned() {
            return existing;
        }
        let class = self.alloc_synthetic_class(name);
        self.mark_synthetic_static_type(&class, name);

        let object_base = self.builtins.get("object").and_then(|value| match value {
            Value::Class(class) => Some(class.clone()),
            _ => None,
        });
        let default_meta = self.default_type_metaclass();
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            if class_data.bases.is_empty()
                && name != "object"
                && name != "type"
                && let Some(base) = object_base
            {
                class_data.bases.push(base);
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
            .insert(name.to_string(), class.clone());
        class
    }

    pub(super) fn class_from_base_value(&mut self, base: Value) -> Result<ObjRef, RuntimeError> {
        match base {
            Value::Class(class) => Ok(class),
            Value::Instance(instance) => {
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
                Err(RuntimeError::type_error(
                    "class base must be a class object",
                ))
            }
            Value::ExceptionType(name) => Ok(self.alloc_synthetic_exception_class(&name)),
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
            Value::Builtin(BuiltinFunction::MemoryView) => {
                Ok(self.synthetic_builtin_class("memoryview"))
            }
            Value::Builtin(BuiltinFunction::Complex) => Ok(self.synthetic_builtin_class("complex")),
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
                Ok(self.synthetic_builtin_class("Counter"))
            }
            Value::Builtin(BuiltinFunction::CollectionsDeque) => {
                Ok(self.synthetic_builtin_class("deque"))
            }
            Value::Builtin(BuiltinFunction::CollectionsDefaultDict) => {
                Ok(self.synthetic_builtin_class("defaultdict"))
            }
            other => {
                if std::env::var_os("PYRS_TRACE_CLASS_BASE").is_some() {
                    eprintln!(
                        "[class-base] unsupported base value={}",
                        format_repr(&other)
                    );
                }
                let _ = other;
                Err(RuntimeError::type_error(
                    "class base must be a class object",
                ))
            }
        }
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
    match &*instance.kind() {
        Object::Instance(instance_data) => match &*instance_data.class.kind() {
            Object::Class(class_data) => matches!(
                class_data.name.as_str(),
                "GenericAlias"
                    | "UnionType"
                    | "TypeVar"
                    | "TypeVarTuple"
                    | "ParamSpec"
                    | "TypeAliasType"
            ),
            _ => false,
        },
        _ => false,
    }
}
