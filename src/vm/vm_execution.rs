use super::*;

impl Vm {
    #[inline]
    fn write_fast_local_slot(slot: &mut Option<Value>, value: Value) {
        match value {
            Value::Int(new_value) => {
                if let Some(Value::Int(existing)) = slot.as_mut() {
                    *existing = new_value;
                } else {
                    *slot = Some(Value::Int(new_value));
                }
            }
            Value::Bool(new_value) => {
                if let Some(Value::Bool(existing)) = slot.as_mut() {
                    *existing = new_value;
                } else {
                    *slot = Some(Value::Bool(new_value));
                }
            }
            Value::None => {
                if !matches!(slot, Some(Value::None)) {
                    *slot = Some(Value::None);
                }
            }
            other => {
                *slot = Some(other);
            }
        }
    }

    pub(super) fn run(&mut self) -> Result<Value, RuntimeError> {
        loop {
            if let Some(stop_depth) = self.run_stop_depth {
                if self.frames.len() <= stop_depth {
                    return Ok(Value::None);
                }
            }
            if self.frames.is_empty() {
                return Ok(Value::None);
            }
            if let Some(target) = self.active_generator_resume {
                if self.generator_resume_outcome.is_some() {
                    return Ok(Value::None);
                }
                let target_active = self.frames.iter().any(|frame| {
                    frame
                        .generator_owner
                        .as_ref()
                        .map(|owner| owner.id() == target)
                        .unwrap_or(false)
                });
                if !target_active {
                    self.generator_resume_outcome =
                        Some(GeneratorResumeOutcome::PropagatedException);
                    return Ok(Value::None);
                }
            }

            let pending_resume = {
                let frame = self.frames.last_mut().expect("frame exists");
                if frame.generator_owner.is_some() && frame.generator_awaiting_resume_value {
                    frame.generator_awaiting_resume_value = false;
                    let thrown = frame.generator_pending_throw.take();
                    let sent = frame.generator_resume_value.take().unwrap_or(Value::None);
                    Some((thrown, sent))
                } else {
                    None
                }
            };
            if let Some((thrown, sent)) = pending_resume {
                if let Some(exc) = thrown {
                    self.raise_exception(exc)?;
                    continue;
                }
                self.push_value(sent);
            }

            let should_return = {
                let frame = self.frames.last().expect("frame exists");
                frame.ip >= frame.code.instructions.len()
            };

            if should_return {
                let mut frame = self.frames.pop().expect("frame exists");
                if let Some(module_dict) = frame.module_locals_dict.take() {
                    self.sync_module_locals_dict_to_module(&frame.module, &module_dict);
                }
                if frame.is_module {
                    self.sync_re_module_flag_aliases(&frame.module);
                }
                let can_recycle = !frame.is_module
                    && frame.generator_owner.is_none()
                    && !frame.return_class
                    && frame.return_instance.is_none()
                    && !frame.return_module;
                if let Some(owner) = frame.generator_owner.take() {
                    self.finish_generator_resume(owner, Value::None);
                    continue;
                }
                if can_recycle {
                    let discard = frame.discard_result;
                    if let Some(caller) = self.frames.last_mut() {
                        if !discard {
                            caller.stack.push(Value::None);
                        }
                        self.recycle_frame(frame);
                        continue;
                    }
                    self.recycle_frame(frame);
                    return Ok(Value::None);
                }
                let value = if frame.return_class {
                    match self.class_value_from_module(
                        &frame.module,
                        frame.class_bases,
                        frame.class_metaclass,
                        frame.class_keywords,
                    )? {
                        ClassBuildOutcome::Value(value) => value,
                        ClassBuildOutcome::ExceptionHandled => continue,
                    }
                } else if let Some(instance) = frame.return_instance {
                    Value::Instance(instance)
                } else if frame.return_module {
                    Value::Module(frame.module.clone())
                } else {
                    Value::None
                };
                if let Some(caller) = self.frames.last_mut() {
                    if !frame.discard_result {
                        caller.stack.push(value);
                    }
                    continue;
                }
                return Ok(value);
            }

            let instr = {
                let frame = self.frames.last_mut().expect("frame exists");
                frame.last_ip = frame.ip;
                let instr = frame.code.instructions[frame.ip];
                frame.ip += 1;
                instr
            };
            let step_result = (|| -> Result<Option<Value>, RuntimeError> {
                match instr.opcode {
                    Opcode::Nop => {}
                    Opcode::LoadConst => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing const argument"))?
                            as usize;
                        let value = {
                            let frame = self.frames.last().expect("frame exists");
                            if idx >= frame.code.constants.len() {
                                return Err(RuntimeError::new("constant index out of range"));
                            }
                            frame.code.constants[idx].clone()
                        };
                        self.frames
                            .last_mut()
                            .expect("frame exists")
                            .stack
                            .push(value);
                    }
                    Opcode::LoadName => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing name argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let value = self.lookup_name(&name)?;
                        self.frames
                            .last_mut()
                            .expect("frame exists")
                            .stack
                            .push(value);
                    }
                    Opcode::LoadLocals => {
                        let value = self.builtin_locals(Vec::new(), HashMap::new())?;
                        self.push_value(value);
                    }
                    Opcode::LoadFast => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing local argument"))?
                            as usize;
                        #[cfg(not(debug_assertions))]
                        let mut fused_compare_jump = false;
                        #[cfg(not(debug_assertions))]
                        {
                            let fused = {
                                let frame = self.frames.last().expect("frame exists");
                                if idx >= frame.fast_locals.len() {
                                    None
                                } else if let Some(left) = frame.fast_locals[idx].as_ref() {
                                    if let (Some(next), Some(jump)) = (
                                        frame.code.instructions.get(frame.ip),
                                        frame.code.instructions.get(frame.ip + 1),
                                    ) {
                                        if next.opcode == Opcode::CompareLtConst
                                            && jump.opcode == Opcode::JumpIfFalse
                                        {
                                            if let (Some(const_idx), Some(target)) =
                                                (next.arg.map(|arg| arg as usize), jump.arg)
                                            {
                                                if const_idx < frame.code.constants.len() {
                                                    let left_int = match left {
                                                        Value::Int(integer) => Some(*integer),
                                                        Value::Bool(flag) => {
                                                            Some(if *flag { 1 } else { 0 })
                                                        }
                                                        _ => None,
                                                    };
                                                    let right_int =
                                                        match &frame.code.constants[const_idx] {
                                                            Value::Int(integer) => Some(*integer),
                                                            Value::Bool(flag) => {
                                                                Some(if *flag { 1 } else { 0 })
                                                            }
                                                            _ => None,
                                                        };
                                                    if let (Some(left_int), Some(right_int)) =
                                                        (left_int, right_int)
                                                    {
                                                        Some((left_int < right_int, target as usize))
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            };
                            if let Some((truthy, target)) = fused {
                                let frame = self.frames.last_mut().expect("frame exists");
                                if truthy {
                                    frame.ip += 2;
                                } else {
                                    frame.ip = target;
                                }
                                fused_compare_jump = true;
                            }
                        }
                        #[cfg(not(debug_assertions))]
                        if !fused_compare_jump {
                            let fast_hit = {
                                let frame = self.frames.last_mut().expect("frame exists");
                                if idx < frame.fast_locals.len() {
                                    if let Some(value) = &frame.fast_locals[idx] {
                                        frame.stack.push(value.clone());
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            };
                            if !fast_hit {
                                let value = self.load_fast_local(idx)?;
                                self.frames
                                    .last_mut()
                                    .expect("frame exists")
                                    .stack
                                    .push(value);
                            }
                        }
                        #[cfg(debug_assertions)]
                        {
                            let fast_hit = {
                                let frame = self.frames.last_mut().expect("frame exists");
                                if idx < frame.fast_locals.len() {
                                    if let Some(value) = &frame.fast_locals[idx] {
                                        frame.stack.push(value.clone());
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            };
                            if !fast_hit {
                                let value = self.load_fast_local(idx)?;
                                self.frames
                                    .last_mut()
                                    .expect("frame exists")
                                    .stack
                                    .push(value);
                            }
                        }
                    }
                    Opcode::LoadDeref => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing deref argument"))?
                            as usize;
                        let value = self.load_deref(idx)?;
                        self.push_value(value);
                    }
                    Opcode::LoadClosure => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing closure argument"))?
                            as usize;
                        let cell = self.get_cell(idx)?;
                        self.push_value(Value::Cell(cell));
                    }
                    Opcode::LoadFast2 => {
                        let arg = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing locals argument"))?;
                        let first = (arg >> 16) as usize;
                        let second = (arg & 0xFFFF) as usize;
                        let (first_value, second_value) = {
                            let frame = self.frames.last().expect("frame exists");
                            let first_value = if first < frame.fast_locals.len() {
                                match &frame.fast_locals[first] {
                                    Some(value) => Some(value.clone()),
                                    None => None,
                                }
                            } else {
                                None
                            };
                            let second_value = if second < frame.fast_locals.len() {
                                match &frame.fast_locals[second] {
                                    Some(value) => Some(value.clone()),
                                    None => None,
                                }
                            } else {
                                None
                            };
                            (first_value, second_value)
                        };
                        let first_value = match first_value {
                            Some(value) => value,
                            None => self.load_fast_local(first)?,
                        };
                        let second_value = match second_value {
                            Some(value) => value,
                            None => self.load_fast_local(second)?,
                        };
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame.stack.push(first_value);
                        frame.stack.push(second_value);
                    }
                    Opcode::LoadFastAndClear => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing local argument"))?
                            as usize;
                        let value = self.take_fast_local(idx)?;
                        self.push_value(value);
                    }
                    Opcode::LoadGlobal => {
                        let raw = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing global argument"))?
                            as usize;
                        let push_null = raw & 1 == 1;
                        let idx = raw >> 1;
                        let site_index = self.current_site_index();
                        let (globals_module_id, globals_version) = {
                            let frame = self.frames.last().expect("frame exists");
                            (frame.function_globals.id(), frame.function_globals_version)
                        };
                        let mut value = None;
                        #[cfg(not(debug_assertions))]
                        let mut fused_candidate: Option<(usize, usize)> = None;
                        #[cfg(not(debug_assertions))]
                        let mut fused_direct_one_arg_no_cells = false;
                        #[cfg(not(debug_assertions))]
                        let mut fused_direct_cached: Option<(Rc<CodeObject>, ObjRef, Option<ObjRef>)> =
                            None;
                        #[cfg(not(debug_assertions))]
                        let mut fused_const_small_int: Option<i64> = None;
                        #[cfg(not(debug_assertions))]
                        let mut fused_from_cached_direct = false;
                        if let Some(frame) = self.frames.last() {
                            if let Some(entry) = frame.load_global_inline_cache.get(site_index) {
                                if let Some(cached) = entry {
                                    if cached.globals_module_id == globals_module_id
                                        && cached.globals_version == globals_version
                                        && cached.builtins_version == self.builtins_version
                                    {
                                        value = Some(cached.value.clone());
                                        #[cfg(not(debug_assertions))]
                                        if let (Some(local_idx), Some(const_idx)) =
                                            (cached.fused_local_idx, cached.fused_const_idx)
                                        {
                                            fused_candidate =
                                                Some((local_idx as usize, const_idx as usize));
                                        }
                                        #[cfg(not(debug_assertions))]
                                        {
                                            fused_direct_one_arg_no_cells =
                                                cached.fused_direct_one_arg_no_cells;
                                            fused_const_small_int = cached.fused_const_small_int;
                                        }
                                        #[cfg(not(debug_assertions))]
                                        if let (Some(code), Some(module)) = (
                                            cached.fused_direct_code.clone(),
                                            cached.fused_direct_module.clone(),
                                        ) {
                                            fused_direct_cached = Some((
                                                code,
                                                module,
                                                cached.fused_direct_owner_class.clone(),
                                            ));
                                        }
                                        #[cfg(not(debug_assertions))]
                                        if !push_null
                                            && fused_direct_one_arg_no_cells
                                            && fused_candidate.is_some()
                                        {
                                            if let (
                                                Some((local_idx, const_idx)),
                                                Some((code, module, owner_class)),
                                            ) = (fused_candidate, fused_direct_cached.as_ref())
                                            {
                                                let arg = if let Some(right_int) =
                                                    fused_const_small_int
                                                {
                                                    let left_small = {
                                                        let frame =
                                                            self.frames.last().expect("frame exists");
                                                        frame
                                                            .fast_locals
                                                            .get(local_idx)
                                                            .and_then(Option::as_ref)
                                                            .and_then(|value| match value {
                                                                Value::Int(integer) => Some(*integer),
                                                                Value::Bool(flag) => {
                                                                    Some(if *flag { 1 } else { 0 })
                                                                }
                                                                _ => None,
                                                            })
                                                    };
                                                    if let Some(left_int) = left_small {
                                                        match left_int.checked_sub(right_int) {
                                                            Some(diff) => Value::Int(diff),
                                                            None => sub_values(
                                                                Value::Int(left_int),
                                                                Value::Int(right_int),
                                                                &self.heap,
                                                            )?,
                                                        }
                                                    } else {
                                                        self.fused_fast_local_sub_small_int_arg(
                                                            local_idx, right_int,
                                                        )?
                                                    }
                                                } else {
                                                    self.fused_fast_local_sub_const_arg(
                                                        local_idx, const_idx,
                                                    )?
                                                };
                                                {
                                                    let caller =
                                                        self.frames.last_mut().expect("frame exists");
                                                    caller.ip += 3;
                                                }
                                                self.push_simple_positional_function_frame_one_arg_no_cells_ref(
                                                    code,
                                                    module,
                                                    owner_class.as_ref(),
                                                    arg,
                                                )?;
                                                fused_from_cached_direct = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        #[cfg(not(debug_assertions))]
                        let value = if fused_from_cached_direct {
                            Value::None
                        } else if let Some(value) = value {
                            value
                        } else {
                            if !push_null {
                                fused_candidate = self.fused_global_fast_sub_call_one_arg_pattern();
                            }
                            let (value, cacheable, globals_module_id, globals_version) =
                                self.resolve_load_global_value(idx)?;
                            if cacheable {
                                if fused_candidate.is_some() {
                                    fused_direct_cached =
                                        self.fused_direct_one_arg_no_cells_metadata(&value);
                                    fused_direct_one_arg_no_cells =
                                        fused_direct_cached.is_some();
                                    fused_const_small_int =
                                        fused_candidate.and_then(|(_, const_idx)| {
                                            self.frames
                                                .last()
                                                .and_then(|frame| frame.code.constants.get(const_idx))
                                                .and_then(|constant| match constant {
                                                    Value::Int(integer) => Some(*integer),
                                                    Value::Bool(flag) => Some(if *flag { 1 } else { 0 }),
                                                    _ => None,
                                                })
                                        });
                                }
                                if let Some(frame) = self.frames.last_mut() {
                                    if let Some(slot) =
                                        frame.load_global_inline_cache.get_mut(site_index)
                                    {
                                        let (
                                            fused_direct_code,
                                            fused_direct_module,
                                            fused_direct_owner_class,
                                        ) = match &fused_direct_cached {
                                            Some((code, module, owner_class)) => (
                                                Some(code.clone()),
                                                Some(module.clone()),
                                                owner_class.clone(),
                                            ),
                                            None => (None, None, None),
                                        };
                                        *slot = Some(LoadGlobalSiteCacheEntry {
                                            globals_module_id,
                                            globals_version,
                                            builtins_version: self.builtins_version,
                                            value: value.clone(),
                                            fused_local_idx: fused_candidate
                                                .as_ref()
                                                .map(|(local_idx, _)| *local_idx as u32),
                                            fused_const_idx: fused_candidate
                                                .as_ref()
                                                .map(|(_, const_idx)| *const_idx as u32),
                                            fused_const_small_int,
                                            fused_direct_one_arg_no_cells,
                                            fused_direct_code,
                                            fused_direct_module,
                                            fused_direct_owner_class,
                                        });
                                    }
                                }
                            }
                            value
                        };
                        #[cfg(debug_assertions)]
                        let value = if let Some(value) = value {
                            value
                        } else {
                            let (value, cacheable, globals_module_id, globals_version) =
                                self.resolve_load_global_value(idx)?;
                            if cacheable {
                                if let Some(frame) = self.frames.last_mut() {
                                    if let Some(slot) =
                                        frame.load_global_inline_cache.get_mut(site_index)
                                    {
                                        *slot = Some(LoadGlobalSiteCacheEntry {
                                            globals_module_id,
                                            globals_version,
                                            builtins_version: self.builtins_version,
                                            value: value.clone(),
                                            fused_local_idx: None,
                                            fused_const_idx: None,
                                            fused_const_small_int: None,
                                            fused_direct_one_arg_no_cells: false,
                                            fused_direct_code: None,
                                            fused_direct_module: None,
                                            fused_direct_owner_class: None,
                                        });
                                    }
                                }
                            }
                            value
                        };
                        #[cfg(not(debug_assertions))]
                        let mut fused = fused_from_cached_direct;
                        #[cfg(debug_assertions)]
                        let fused = false;

                        #[cfg(not(debug_assertions))]
                        {
                            if !push_null {
                                if let Some((local_idx, const_idx)) = fused_candidate {
                                    let arg = if let Some(right_int) = fused_const_small_int {
                                        let left_small = {
                                            let frame = self.frames.last().expect("frame exists");
                                            frame
                                                .fast_locals
                                                .get(local_idx)
                                                .and_then(Option::as_ref)
                                                .and_then(|value| match value {
                                                    Value::Int(integer) => Some(*integer),
                                                    Value::Bool(flag) => {
                                                        Some(if *flag { 1 } else { 0 })
                                                    }
                                                    _ => None,
                                                })
                                        };
                                        if let Some(left_int) = left_small {
                                            match left_int.checked_sub(right_int) {
                                                Some(diff) => Value::Int(diff),
                                                None => sub_values(
                                                    Value::Int(left_int),
                                                    Value::Int(right_int),
                                                    &self.heap,
                                                )?,
                                            }
                                        } else {
                                            self.fused_fast_local_sub_small_int_arg(
                                                local_idx, right_int,
                                            )?
                                        }
                                    } else {
                                        self.fused_fast_local_sub_const_arg(local_idx, const_idx)?
                                    };
                                    if let Value::Function(func_obj) = &value {
                                        {
                                            let caller = self.frames.last_mut().expect("frame exists");
                                            caller.ip += 3;
                                        }
                                        if fused_direct_one_arg_no_cells {
                                            if let Some((code, module, owner_class)) =
                                                fused_direct_cached.as_ref()
                                            {
                                                self.push_simple_positional_function_frame_one_arg_no_cells_ref(
                                                    code,
                                                    module,
                                                    owner_class.as_ref(),
                                                    arg,
                                                )?;
                                            } else {
                                                self.push_simple_positional_function_frame_one_arg_no_cells_from_func(
                                                    func_obj, arg,
                                                )?;
                                            }
                                        } else {
                                            self.push_function_call_one_arg_from_obj(func_obj, arg)?;
                                        }
                                        fused = true;
                                    }
                                }
                            }
                        }
                        if !fused {
                            if push_null {
                                self.push_value(Value::None);
                            }
                            self.push_value(value);
                        }
                    }
                    Opcode::LoadBuildClass => {
                        self.push_value(Value::Builtin(BuiltinFunction::BuildClass));
                    }
                    Opcode::PushNull => {
                        self.push_value(Value::None);
                    }
                    Opcode::GetAwaitable => {
                        let value = self.pop_value()?;
                        let awaitable = self.awaitable_from_value(value)?;
                        self.push_value(awaitable);
                    }
                    Opcode::LoadAttr => {
                        let raw = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing attribute argument"))?
                            as usize;
                        let push_null = raw & 1 == 1;
                        let idx = raw >> 1;
                        let attr_name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let value = self.pop_value()?;
                        let attr = if attr_name == "__class__" {
                            self.load_dunder_class_attr(&value)?
                        } else {
                            match value {
                                Value::Module(module) => {
                                    self.load_attr_module(&module, &attr_name)?
                                }
                                Value::Class(class) => {
                                    match self.load_attr_class(&class, &attr_name)? {
                                        AttrAccessOutcome::Value(attr) => attr,
                                        AttrAccessOutcome::ExceptionHandled => {
                                            return Err(self.runtime_error_from_active_exception(
                                                "attribute access failed",
                                            ))
                                        }
                                    }
                                }
                                Value::Instance(instance) => {
                                    match self.load_attr_instance(&instance, &attr_name)? {
                                        AttrAccessOutcome::Value(attr) => attr,
                                        AttrAccessOutcome::ExceptionHandled => {
                                            return Err(self.runtime_error_from_active_exception(
                                                "attribute access failed",
                                            ))
                                        }
                                    }
                                }
                                Value::Super(super_obj) => {
                                    match self.load_attr_super(&super_obj, &attr_name)? {
                                        AttrAccessOutcome::Value(attr) => attr,
                                        AttrAccessOutcome::ExceptionHandled => {
                                            return Err(self.runtime_error_from_active_exception(
                                                "attribute access failed",
                                            ))
                                        }
                                    }
                                }
                                Value::List(list) => {
                                    self.load_attr_list_method(list, &attr_name)?
                                }
                                Value::Int(value) => {
                                    self.load_attr_int_method(Value::Int(value), &attr_name)?
                                }
                                Value::BigInt(value) => {
                                    self.load_attr_int_method(Value::BigInt(value), &attr_name)?
                                }
                                Value::Bool(value) => {
                                    self.load_attr_int_method(Value::Bool(value), &attr_name)?
                                }
                                Value::Complex { real, imag } => match attr_name.as_str() {
                                    "__reduce_ex__" | "__reduce__" => {
                                        let wrapper =
                                            match self.heap.alloc_module(ModuleObject::new(
                                                "__complex_reduce_ex__".to_string(),
                                            )) {
                                                Value::Module(obj) => obj,
                                                _ => unreachable!(),
                                            };
                                        if let Object::Module(module_data) =
                                            &mut *wrapper.kind_mut()
                                        {
                                            module_data.globals.insert(
                                                "value".to_string(),
                                                Value::Complex { real, imag },
                                            );
                                        }
                                        self.alloc_native_bound_method(
                                            NativeMethodKind::ComplexReduceEx,
                                            wrapper,
                                        )
                                    }
                                    "real" => Value::Float(real),
                                    "imag" => Value::Float(imag),
                                    _ => {
                                        return Err(RuntimeError::new(format!(
                                            "complex has no attribute '{}'",
                                            attr_name
                                        )));
                                    }
                                },
                                Value::Str(text) => self.load_attr_str_method(text, &attr_name)?,
                                Value::Bytes(obj) => {
                                    let is_bytes = matches!(&*obj.kind(), Object::Bytes(_));
                                    if !is_bytes {
                                        return Err(RuntimeError::new(
                                            "attribute access unsupported type",
                                        ));
                                    }
                                    self.load_attr_bytes_method(Value::Bytes(obj), &attr_name)?
                                }
                                Value::ByteArray(obj) => {
                                    let is_bytearray = matches!(&*obj.kind(), Object::ByteArray(_));
                                    if !is_bytearray {
                                        return Err(RuntimeError::new(
                                            "attribute access unsupported type",
                                        ));
                                    }
                                    self.load_attr_bytes_method(Value::ByteArray(obj), &attr_name)?
                                }
                                Value::Iterator(iterator) => {
                                    self.load_attr_iterator(iterator, &attr_name)?
                                }
                                Value::MemoryView(view) => {
                                    self.load_attr_memoryview(view, &attr_name)?
                                }
                                Value::Set(set) => self.load_attr_set_method(set, &attr_name)?,
                                Value::FrozenSet(set) => {
                                    self.load_attr_set_method(set, &attr_name)?
                                }
                                Value::Dict(dict) => {
                                    self.load_attr_dict_method(dict, &attr_name)?
                                }
                                Value::Builtin(builtin) => {
                                    self.load_attr_builtin(builtin, &attr_name)?
                                }
                                Value::Function(func) => {
                                    self.load_attr_function(&func, &attr_name)?
                                }
                                Value::BoundMethod(method) => {
                                    self.load_attr_bound_method(&method, &attr_name)?
                                }
                                Value::Code(code) => self.load_attr_code(&code, &attr_name)?,
                                Value::Generator(generator) => {
                                    let kind = match &*generator.kind() {
                                        Object::Generator(state) if state.is_async_generator => {
                                            match attr_name.as_str() {
                                                "__aiter__" => NativeMethodKind::GeneratorIter,
                                                "__anext__" => NativeMethodKind::GeneratorANext,
                                                "asend" => NativeMethodKind::GeneratorANext,
                                                "athrow" => NativeMethodKind::GeneratorThrow,
                                                "aclose" => NativeMethodKind::GeneratorClose,
                                                "throw" => NativeMethodKind::GeneratorThrow,
                                                "close" => NativeMethodKind::GeneratorClose,
                                                _ => {
                                                    return Err(RuntimeError::new(format!(
                                                        "async_generator has no attribute '{}'",
                                                        attr_name
                                                    )));
                                                }
                                            }
                                        }
                                        Object::Generator(state) if state.is_coroutine => {
                                            match attr_name.as_str() {
                                                "__await__" => NativeMethodKind::GeneratorAwait,
                                                "send" => NativeMethodKind::GeneratorSend,
                                                "throw" => NativeMethodKind::GeneratorThrow,
                                                "close" => NativeMethodKind::GeneratorClose,
                                                _ => {
                                                    return Err(RuntimeError::new(format!(
                                                        "coroutine has no attribute '{}'",
                                                        attr_name
                                                    )));
                                                }
                                            }
                                        }
                                        Object::Generator(_) => match attr_name.as_str() {
                                            "__iter__" => NativeMethodKind::GeneratorIter,
                                            "__next__" => NativeMethodKind::GeneratorNext,
                                            "send" => NativeMethodKind::GeneratorSend,
                                            "throw" => NativeMethodKind::GeneratorThrow,
                                            "close" => NativeMethodKind::GeneratorClose,
                                            _ => {
                                                return Err(RuntimeError::new(format!(
                                                    "generator has no attribute '{}'",
                                                    attr_name
                                                )));
                                            }
                                        },
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "attribute access unsupported type",
                                            ));
                                        }
                                    };
                                    let native = self
                                        .heap
                                        .alloc_native_method(NativeMethodObject::new(kind));
                                    let bound = BoundMethod::new(native, generator);
                                    self.heap.alloc_bound_method(bound)
                                }
                                Value::Exception(exception) => match attr_name.as_str() {
                                    "__reduce_ex__" | "__reduce__" => self
                                        .alloc_reduce_ex_bound_method(Value::Exception(
                                            exception.clone(),
                                        )),
                                    "with_traceback" => {
                                        let wrapper =
                                            match self.heap.alloc_module(ModuleObject::new(
                                                "__exception_with_traceback__".to_string(),
                                            )) {
                                                Value::Module(obj) => obj,
                                                _ => unreachable!(),
                                            };
                                        if let Object::Module(module_data) =
                                            &mut *wrapper.kind_mut()
                                        {
                                            module_data.globals.insert(
                                                "exception".to_string(),
                                                Value::Exception(exception.clone()),
                                            );
                                        }
                                        self.alloc_native_bound_method(
                                            NativeMethodKind::ExceptionWithTraceback,
                                            wrapper,
                                        )
                                    }
                                    "add_note" => {
                                        let wrapper =
                                            match self.heap.alloc_module(ModuleObject::new(
                                                "__exception_add_note__".to_string(),
                                            )) {
                                                Value::Module(obj) => obj,
                                                _ => unreachable!(),
                                            };
                                        if let Object::Module(module_data) =
                                            &mut *wrapper.kind_mut()
                                        {
                                            module_data.globals.insert(
                                                "exception".to_string(),
                                                Value::Exception(exception.clone()),
                                            );
                                        }
                                        self.alloc_native_bound_method(
                                            NativeMethodKind::ExceptionAddNote,
                                            wrapper,
                                        )
                                    }
                                    "__class__" => Value::ExceptionType(exception.name.clone()),
                                    "__notes__" => {
                                        if exception.notes.is_empty() {
                                            Value::None
                                        } else {
                                            self.heap.alloc_list(
                                                exception
                                                    .notes
                                                    .iter()
                                                    .cloned()
                                                    .map(Value::Str)
                                                    .collect(),
                                            )
                                        }
                                    }
                                    "__cause__" => exception
                                        .cause
                                        .as_ref()
                                        .map(|cause| Value::Exception((**cause).clone()))
                                        .unwrap_or(Value::None),
                                    "__context__" => exception
                                        .context
                                        .as_ref()
                                        .map(|context| Value::Exception((**context).clone()))
                                        .unwrap_or(Value::None),
                                    "__traceback__" => Value::None,
                                    "__suppress_context__" => {
                                        Value::Bool(exception.suppress_context)
                                    }
                                    "exceptions" => {
                                        let members = exception
                                            .exceptions
                                            .iter()
                                            .cloned()
                                            .map(Value::Exception)
                                            .collect::<Vec<_>>();
                                        self.heap.alloc_tuple(members)
                                    }
                                    _ => {
                                        if let Some(value) =
                                            exception.attrs.borrow().get(&attr_name).cloned()
                                        {
                                            value
                                        } else {
                                            return Err(RuntimeError::new(format!(
                                                "exception has no attribute '{}'",
                                                attr_name
                                            )));
                                        }
                                    }
                                },
                                Value::ExceptionType(name) => {
                                    self.load_attr_exception_type(&name, &attr_name)?
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attribute access unsupported type",
                                    ));
                                }
                            }
                        };
                        if push_null {
                            let frame = self.frames.get_mut(caller_idx).ok_or_else(|| {
                                RuntimeError::new("attribute caller frame missing")
                            })?;
                            frame.stack.push(Value::None);
                        }
                        let frame = self
                            .frames
                            .get_mut(caller_idx)
                            .ok_or_else(|| RuntimeError::new("attribute caller frame missing"))?;
                        frame.stack.push(attr);
                    }
                    Opcode::StoreName => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing name argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let value = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (StoreName)"))?
                        };
                        self.store_name(name, value);
                    }
                    Opcode::DeleteName => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing name argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let mut removed = false;
                        let mut touched_module_version: Option<(u64, u64)> = None;
                        if let Some(frame) = self.frames.last_mut() {
                            if !frame.is_module {
                                if let Some(slot_idx) = frame.code.name_to_index.get(&name).copied() {
                                    if let Some(slot) = frame.fast_locals.get_mut(slot_idx) {
                                        removed = slot.take().is_some();
                                    }
                                }
                                if removed {
                                    frame.locals.remove(&name);
                                } else {
                                    removed = frame.locals.remove(&name).is_some();
                                }
                            }
                            if !removed {
                                if let Some(dict) = frame.module_locals_dict.clone() {
                                    removed =
                                        dict_remove_value(&dict, &Value::Str(name.clone())).is_some();
                                }
                            }
                            if !removed {
                                if let Object::Module(module_data) = &mut *frame.module.kind_mut() {
                                    removed = module_data.globals.remove(&name).is_some();
                                    if removed {
                                        module_data.touch_globals_version();
                                        touched_module_version =
                                            Some((frame.module.id(), module_data.globals_version));
                                    }
                                }
                            }
                        }
                        if !removed {
                            return Err(RuntimeError::new(format!(
                                "name '{}' is not defined",
                                name
                            )));
                        }
                        if let Some((module_id, version)) = touched_module_version {
                            self.propagate_module_globals_version(module_id, version);
                        }
                    }
                    Opcode::StoreFast => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing local argument"))?
                            as usize;
                        let value = self.pop_value()?;
                        self.store_fast_local(idx, value)?;
                    }
                    Opcode::StoreDeref => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing deref argument"))?
                            as usize;
                        let value = self.pop_value()?;
                        self.store_deref(idx, value)?;
                    }
                    Opcode::StoreFastLoadFast => {
                        let arg = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing locals argument"))?;
                        let first = (arg >> 16) as usize;
                        let second = (arg & 0xFFFF) as usize;
                        let value = self.pop_value()?;
                        self.store_fast_local(first, value)?;
                        let second_value = self.load_fast_local(second)?;
                        self.push_value(second_value);
                    }
                    Opcode::StoreFastStoreFast => {
                        let arg = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing locals argument"))?;
                        let first = (arg >> 16) as usize;
                        let second = (arg & 0xFFFF) as usize;
                        let value2 = self.pop_value()?;
                        let value1 = self.pop_value()?;
                        self.store_fast_local(first, value1)?;
                        self.store_fast_local(second, value2)?;
                    }
                    Opcode::StoreAttr => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing attribute argument"))?
                            as usize;
                        let attr_name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let value = self.pop_value()?;
                        let target = self.pop_value()?;
                        match target {
                            Value::Module(module) => {
                                self.upsert_module_global(&module, &attr_name, value);
                            }
                            Value::Instance(instance) => {
                                match self.store_attr_instance(&instance, &attr_name, value)? {
                                    AttrMutationOutcome::Done => {}
                                    AttrMutationOutcome::ExceptionHandled => {
                                        return Err(self.runtime_error_from_active_exception(
                                            "attribute assignment failed",
                                        ))
                                    }
                                }
                            }
                            Value::Class(class) => {
                                if let Object::Class(class_data) = &mut *class.kind_mut() {
                                    class_data.attrs.insert(attr_name, value);
                                }
                            }
                            Value::Function(func) => {
                                self.store_attr_function(&func, attr_name, value)?
                            }
                            Value::Exception(mut exception) => {
                                self.store_attr_exception(&mut exception, &attr_name, value)?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "attribute assignment unsupported type",
                                ));
                            }
                        }
                    }
                    Opcode::StoreAttrCpython => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing attribute argument"))?
                            as usize;
                        let attr_name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let target = self.pop_value()?;
                        let value = self.pop_value()?;
                        match target {
                            Value::Module(module) => {
                                self.upsert_module_global(&module, &attr_name, value);
                            }
                            Value::Instance(instance) => {
                                match self.store_attr_instance(&instance, &attr_name, value)? {
                                    AttrMutationOutcome::Done => {}
                                    AttrMutationOutcome::ExceptionHandled => {
                                        return Err(self.runtime_error_from_active_exception(
                                            "attribute assignment failed",
                                        ))
                                    }
                                }
                            }
                            Value::Class(class) => {
                                if let Object::Class(class_data) = &mut *class.kind_mut() {
                                    class_data.attrs.insert(attr_name, value);
                                }
                            }
                            Value::Function(func) => {
                                self.store_attr_function(&func, attr_name, value)?
                            }
                            Value::Exception(mut exception) => {
                                self.store_attr_exception(&mut exception, &attr_name, value)?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "attribute assignment unsupported type",
                                ));
                            }
                        }
                    }
                    Opcode::DeleteAttr => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing attribute argument"))?
                            as usize;
                        let attr_name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let target = self.pop_value()?;
                        let mut touched_module_version: Option<(u64, u64)> = None;
                        match target {
                            Value::Module(module) => {
                                if let Object::Module(module_data) = &mut *module.kind_mut() {
                                    if module_data.globals.remove(&attr_name).is_none() {
                                        return Err(RuntimeError::new(format!(
                                            "module attribute '{}' does not exist",
                                            attr_name
                                        )));
                                    }
                                    module_data.touch_globals_version();
                                    touched_module_version =
                                        Some((module.id(), module_data.globals_version));
                                }
                            }
                            Value::Class(class) => {
                                if let Object::Class(class_data) = &mut *class.kind_mut() {
                                    if class_data.attrs.remove(&attr_name).is_none() {
                                        return Err(RuntimeError::new(format!(
                                            "class attribute '{}' does not exist",
                                            attr_name
                                        )));
                                    }
                                }
                            }
                            Value::Instance(instance) => {
                                match self.delete_attr_instance(&instance, &attr_name)? {
                                    AttrMutationOutcome::Done => {}
                                    AttrMutationOutcome::ExceptionHandled => {
                                        return Err(self.runtime_error_from_active_exception(
                                            "attribute deletion failed",
                                        ))
                                    }
                                }
                            }
                            Value::Function(func) => {
                                self.delete_attr_function(&func, &attr_name)?;
                            }
                            Value::Exception(exception) => {
                                self.delete_attr_exception(&exception, &attr_name)?;
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "attribute deletion unsupported type",
                                ));
                            }
                        }
                        if let Some((module_id, version)) = touched_module_version {
                            self.propagate_module_globals_version(module_id, version);
                        }
                    }
                    Opcode::StoreGlobal => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing name argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .code
                                .names
                                .get(idx)
                                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                                .clone()
                        };
                        let value = self.pop_value()?;
                        let globals_module = self
                            .frames
                            .last()
                            .map(|frame| frame.function_globals.clone());
                        if let Some(globals_module) = globals_module {
                            self.upsert_module_global(&globals_module, &name, value);
                        }
                    }
                    Opcode::BinaryAdd => {
                        let site_index = self.current_site_index();
                        let quickened_int =
                            self.is_quickened_site(site_index, QuickenedSiteKind::AddInt);
                        let (left, right) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            let right = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (BinaryAdd rhs)"))?;
                            let left = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (BinaryAdd lhs)"))?;
                            (left, right)
                        };
                        let (value, can_quicken) = if quickened_int {
                            let value = match (left, right) {
                                (Value::Int(a), Value::Int(b)) => match a.checked_add(b) {
                                    Some(sum) => Value::Int(sum),
                                    None => add_values(Value::Int(a), Value::Int(b), &self.heap)?,
                                },
                                (left, right) => {
                                    self.clear_quickened_site(site_index);
                                    add_values(left, right, &self.heap)?
                                }
                            };
                            (value, false)
                        } else {
                            match (left, right) {
                                (Value::Int(a), Value::Int(b)) => {
                                    let value = match a.checked_add(b) {
                                        Some(sum) => Value::Int(sum),
                                        None => add_values(Value::Int(a), Value::Int(b), &self.heap)?,
                                    };
                                    (value, true)
                                }
                                (left, right) => (add_values(left, right, &self.heap)?, false),
                            }
                        };
                        if can_quicken {
                            self.mark_quickened_site(site_index, QuickenedSiteKind::AddInt);
                        }
                        self.frames
                            .last_mut()
                            .expect("frame exists")
                            .stack
                            .push(value);
                    }
                    Opcode::BinarySub => {
                        let site_index = self.current_site_index();
                        let quickened_int =
                            self.is_quickened_site(site_index, QuickenedSiteKind::SubInt);
                        let (left, right) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            let right = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (BinarySub rhs)"))?;
                            let left = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (BinarySub lhs)"))?;
                            (left, right)
                        };
                        let (value, can_quicken) = if quickened_int {
                            let value = match (left, right) {
                                (Value::Int(a), Value::Int(b)) => match a.checked_sub(b) {
                                    Some(diff) => Value::Int(diff),
                                    None => sub_values(Value::Int(a), Value::Int(b), &self.heap)?,
                                },
                                (left, right) => {
                                    self.clear_quickened_site(site_index);
                                    sub_values(left, right, &self.heap)?
                                }
                            };
                            (value, false)
                        } else {
                            match (left, right) {
                                (Value::Int(a), Value::Int(b)) => {
                                    let value = match a.checked_sub(b) {
                                        Some(diff) => Value::Int(diff),
                                        None => sub_values(Value::Int(a), Value::Int(b), &self.heap)?,
                                    };
                                    (value, true)
                                }
                                (left, right) => (sub_values(left, right, &self.heap)?, false),
                            }
                        };
                        if can_quicken {
                            self.mark_quickened_site(site_index, QuickenedSiteKind::SubInt);
                        }
                        self.frames
                            .last_mut()
                            .expect("frame exists")
                            .stack
                            .push(value);
                    }
                    Opcode::BinarySubConst => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing const argument"))?
                            as usize;
                        let (left, right_int, right_value) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            if idx >= frame.code.constants.len() {
                                return Err(RuntimeError::new("constant index out of range"));
                            }
                            let left = frame.stack.pop().ok_or_else(|| {
                                RuntimeError::new("stack underflow (BinarySubConst lhs)")
                            })?;
                            let (right_int, right_value) = match &frame.code.constants[idx] {
                                Value::Int(value) => (Some(*value), None),
                                Value::Bool(flag) => (Some(if *flag { 1 } else { 0 }), None),
                                value => (None, Some(value.clone())),
                            };
                            (left, right_int, right_value)
                        };
                        let value = match (left, right_int, right_value) {
                            (Value::Int(a), Some(b), _) => match a.checked_sub(b) {
                                Some(diff) => Value::Int(diff),
                                None => sub_values(Value::Int(a), Value::Int(b), &self.heap)?,
                            },
                            (left, Some(b), _) => sub_values(left, Value::Int(b), &self.heap)?,
                            (left, None, Some(right)) => sub_values(left, right, &self.heap)?,
                            (_, None, None) => {
                                return Err(RuntimeError::new("invalid constant for BinarySubConst"));
                            }
                        };
                        self.frames
                            .last_mut()
                            .expect("frame exists")
                            .stack
                            .push(value);
                    }
                    Opcode::BinaryMul => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(mul_values(left, right, &self.heap)?);
                    }
                    Opcode::BinaryMatMul => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(matmul_values(left, right)?);
                    }
                    Opcode::BinaryDiv => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let value = self.binary_div_runtime(left, right)?;
                        self.push_value(value);
                    }
                    Opcode::BinaryPow => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(pow_values(left, right)?);
                    }
                    Opcode::BinaryFloorDiv => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(floor_div_values(left, right)?);
                    }
                    Opcode::BinaryMod => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(mod_values(left, right)?);
                    }
                    Opcode::BinaryLShift => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(lshift_values(left, right)?);
                    }
                    Opcode::BinaryRShift => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(rshift_values(left, right)?);
                    }
                    Opcode::BinaryAnd => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(and_values(left, right, &self.heap)?);
                    }
                    Opcode::BinaryXor => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(xor_values(left, right, &self.heap)?);
                    }
                    Opcode::BinaryOr => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        self.push_value(or_values(left, right, &self.heap)?);
                    }
                    Opcode::CompareEq => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let result = self.compare_eq_runtime(left, right)?;
                        self.push_value(result);
                    }
                    Opcode::CompareNe => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let result = self.compare_ne_runtime(left, right)?;
                        self.push_value(result);
                    }
                    Opcode::CompareLt => {
                        let site_index = self.current_site_index();
                        let quickened_int =
                            self.is_quickened_site(site_index, QuickenedSiteKind::CompareLtInt);
                        let (left, right) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            let right = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (CompareLt rhs)"))?;
                            let left = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (CompareLt lhs)"))?;
                            (left, right)
                        };
                        let (result, can_quicken) = if quickened_int {
                            let result = match (left, right) {
                                (Value::Int(a), Value::Int(b)) => Value::Bool(a < b),
                                (left, right) => {
                                    self.clear_quickened_site(site_index);
                                    self.compare_lt_runtime(left, right)?
                                }
                            };
                            (result, false)
                        } else {
                            match (left, right) {
                                (Value::Int(a), Value::Int(b)) => (Value::Bool(a < b), true),
                                (left, right) => (self.compare_lt_runtime(left, right)?, false),
                            }
                        };
                        if can_quicken {
                            self.mark_quickened_site(site_index, QuickenedSiteKind::CompareLtInt);
                        }
                        if let Some(target) = self.next_jump_if_false_target() {
                            let truthy = match result {
                                Value::Bool(flag) => flag,
                                other => self.truthy_from_value(&other)?,
                            };
                            let frame = self.frames.last_mut().expect("frame exists");
                            if truthy {
                                frame.ip += 1;
                            } else {
                                frame.ip = target;
                            }
                        } else {
                            self.frames
                                .last_mut()
                                .expect("frame exists")
                                .stack
                                .push(result);
                        }
                    }
                    Opcode::CompareLtConst => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing const argument"))?
                            as usize;
                        let (left, right_int, right_value) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            if idx >= frame.code.constants.len() {
                                return Err(RuntimeError::new("constant index out of range"));
                            }
                            let left = frame.stack.pop().ok_or_else(|| {
                                RuntimeError::new("stack underflow (CompareLtConst lhs)")
                            })?;
                            let (right_int, right_value) = match &frame.code.constants[idx] {
                                Value::Int(value) => (Some(*value), None),
                                Value::Bool(flag) => (Some(if *flag { 1 } else { 0 }), None),
                                value => (None, Some(value.clone())),
                            };
                            (left, right_int, right_value)
                        };
                        let result = match (left, right_int, right_value) {
                            (Value::Int(a), Some(b), _) => Value::Bool(a < b),
                            (left, Some(b), _) => self.compare_lt_runtime(left, Value::Int(b))?,
                            (left, None, Some(right)) => self.compare_lt_runtime(left, right)?,
                            (_, None, None) => {
                                return Err(RuntimeError::new("invalid constant for CompareLtConst"));
                            }
                        };
                        if let Some(target) = self.next_jump_if_false_target() {
                            let truthy = match result {
                                Value::Bool(flag) => flag,
                                other => self.truthy_from_value(&other)?,
                            };
                            let frame = self.frames.last_mut().expect("frame exists");
                            if truthy {
                                frame.ip += 1;
                            } else {
                                frame.ip = target;
                            }
                        } else {
                            self.frames
                                .last_mut()
                                .expect("frame exists")
                                .stack
                                .push(result);
                        }
                    }
                    Opcode::CompareLe => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let result = self.compare_le_runtime(left, right)?;
                        self.push_value(result);
                    }
                    Opcode::CompareGt => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let result = self.compare_gt_runtime(left, right)?;
                        self.push_value(result);
                    }
                    Opcode::CompareGe => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let result = self.compare_ge_runtime(left, right)?;
                        self.push_value(result);
                    }
                    Opcode::CompareIn => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let contains = self.compare_in_runtime(left, right)?;
                        self.push_value(Value::Bool(contains));
                    }
                    Opcode::CompareNotIn => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let contains = self.compare_in_runtime(left, right)?;
                        self.push_value(Value::Bool(!contains));
                    }
                    Opcode::CompareIs => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let same = self.heap.id_of(&left) == self.heap.id_of(&right);
                        self.push_value(Value::Bool(same));
                    }
                    Opcode::CompareIsNot => {
                        let right = self.pop_value()?;
                        let left = self.pop_value()?;
                        let same = self.heap.id_of(&left) == self.heap.id_of(&right);
                        self.push_value(Value::Bool(!same));
                    }
                    Opcode::UnaryNeg => {
                        let value = self.pop_value()?;
                        self.push_value(neg_value(value)?);
                    }
                    Opcode::UnaryNot => {
                        let value = self.pop_value()?;
                        let truthy = self.truthy_from_value(&value)?;
                        self.push_value(Value::Bool(!truthy));
                    }
                    Opcode::UnaryPos => {
                        let value = self.pop_value()?;
                        self.push_value(pos_value(value)?);
                    }
                    Opcode::UnaryInvert => {
                        let value = self.pop_value()?;
                        self.push_value(invert_value(value)?);
                    }
                    Opcode::ToBool => {
                        let value = self.pop_value()?;
                        let truthy = self.truthy_from_value(&value)?;
                        self.push_value(Value::Bool(truthy));
                    }
                    Opcode::BuildList => {
                        let count = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing list size"))?
                            as usize;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(self.pop_value()?);
                        }
                        values.reverse();
                        self.push_value(self.heap.alloc_list(values));
                    }
                    Opcode::BuildTuple => {
                        let count = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing tuple size"))?
                            as usize;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(self.pop_value()?);
                        }
                        values.reverse();
                        self.push_value(self.heap.alloc_tuple(values));
                    }
                    Opcode::BuildDict => {
                        let count = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing dict size"))?
                            as usize;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            let value = self.pop_value()?;
                            let key = self.pop_value()?;
                            ensure_hashable(&key)?;
                            values.push((key, value));
                        }
                        values.reverse();
                        self.push_value(self.heap.alloc_dict(values));
                    }
                    Opcode::UnpackSequence => {
                        let count = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing unpack size"))?
                            as usize;
                        let value = self.pop_value()?;
                        let items = match value {
                            Value::List(obj) => match &*obj.kind() {
                                Object::List(values) => values.clone(),
                                _ => return Err(RuntimeError::new("unpack expects iterable")),
                            },
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => values.clone(),
                                _ => return Err(RuntimeError::new("unpack expects iterable")),
                            },
                            other => self
                                .collect_iterable_values(other)
                                .map_err(|_| RuntimeError::new("unpack expects iterable"))?,
                        };
                        if items.len() != count {
                            return Err(RuntimeError::new("unpack length mismatch"));
                        }
                        for item in items {
                            self.push_value(item);
                        }
                    }
                    Opcode::UnpackEx => {
                        let packed = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing unpack sizes"))?;
                        let before = (packed & 0xFFFF) as usize;
                        let after = (packed >> 16) as usize;
                        let value = self.pop_value()?;
                        let mut items = match value {
                            Value::List(obj) => match &*obj.kind() {
                                Object::List(values) => values.clone(),
                                _ => return Err(RuntimeError::new("unpack expects iterable")),
                            },
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => values.clone(),
                                _ => return Err(RuntimeError::new("unpack expects iterable")),
                            },
                            other => self
                                .collect_iterable_values(other)
                                .map_err(|_| RuntimeError::new("unpack expects iterable"))?,
                        };
                        if items.len() < before + after {
                            return Err(RuntimeError::new("unpack length mismatch"));
                        }
                        let trailing = items.split_off(items.len() - after);
                        let middle = items.split_off(before);
                        for item in items {
                            self.push_value(item);
                        }
                        self.push_value(self.heap.alloc_list(middle));
                        for item in trailing {
                            self.push_value(item);
                        }
                    }
                    Opcode::ListAppend => {
                        let value = self.pop_value()?;
                        let list = self.pop_value()?;
                        match list {
                            Value::List(obj) => {
                                if let Object::List(values) = &mut *obj.kind_mut() {
                                    values.push(value);
                                }
                                self.push_value(Value::List(obj));
                            }
                            _ => return Err(RuntimeError::new("list append expects list")),
                        }
                    }
                    Opcode::ListExtend => {
                        let other = self.pop_value()?;
                        let list = self.pop_value()?;
                        match list {
                            Value::List(obj) => {
                                let extra = self.collect_iterable_values(other).map_err(|_| {
                                    RuntimeError::new("list extend expects iterable")
                                })?;
                                if let Object::List(values) = &mut *obj.kind_mut() {
                                    values.extend(extra);
                                }
                                self.push_value(Value::List(obj));
                            }
                            _ => return Err(RuntimeError::new("list extend expects list")),
                        }
                    }
                    Opcode::DictSet => {
                        let value = self.pop_value()?;
                        let key = self.pop_value()?;
                        let dict = self.pop_value()?;
                        match dict {
                            Value::Dict(obj) => {
                                dict_set_value_checked(&obj, key, value)?;
                                self.push_value(Value::Dict(obj));
                            }
                            _ => return Err(RuntimeError::new("dict set expects dict")),
                        }
                    }
                    Opcode::DictUpdate => {
                        let other = self.pop_value()?;
                        let dict = self.pop_value()?;
                        match (dict, other) {
                            (Value::Dict(obj), Value::Dict(other)) => {
                                let other_entries = match &*other.kind() {
                                    Object::Dict(entries) => entries.clone(),
                                    _ => return Err(RuntimeError::new("dict update expects dict")),
                                };
                                if !matches!(&*obj.kind(), Object::Dict(_)) {
                                    return Err(RuntimeError::new("dict update expects dict"));
                                }
                                for (key, value) in other_entries {
                                    dict_set_value_checked(&obj, key, value)?;
                                }
                                self.push_value(Value::Dict(obj));
                            }
                            _ => return Err(RuntimeError::new("dict update expects dict")),
                        }
                    }
                    Opcode::BuildSlice => {
                        let step = self.pop_value()?;
                        let upper = self.pop_value()?;
                        let lower = self.pop_value()?;
                        let lower = value_to_optional_index(lower)?;
                        let upper = value_to_optional_index(upper)?;
                        let step = value_to_optional_index(step)?;
                        self.push_value(Value::Slice { lower, upper, step });
                    }
                    Opcode::Subscript => {
                        let index = self.pop_value()?;
                        let value = self.pop_value()?;
                        let result = self.getitem_value(value, index)?;
                        self.push_value(result);
                    }
                    Opcode::StoreSubscript => {
                        let value = self.pop_value()?;
                        let index = self.pop_value()?;
                        let target = self.pop_value()?;
                        match target {
                            Value::List(obj) => match index {
                                Value::Slice { lower, upper, step } => {
                                    let replacement =
                                        self.collect_iterable_values(value).map_err(|_| {
                                            RuntimeError::new("can only assign an iterable")
                                        })?;
                                    if let Object::List(values) = &mut *obj.kind_mut() {
                                        let step_value = step.unwrap_or(1);
                                        if step_value == 1 {
                                            let (start, stop) = slice_bounds_for_step_one(
                                                values.len(),
                                                lower,
                                                upper,
                                            );
                                            values.splice(start..stop, replacement);
                                        } else {
                                            let indices =
                                                slice_indices(values.len(), lower, upper, step)?;
                                            if indices.len() != replacement.len() {
                                                return Err(RuntimeError::new(format!(
                                                    "attempt to assign sequence of size {} to extended slice of size {}",
                                                    replacement.len(),
                                                    indices.len()
                                                )));
                                            }
                                            for (idx, item) in indices.into_iter().zip(replacement)
                                            {
                                                values[idx] = item;
                                            }
                                        }
                                    }
                                    self.push_value(Value::List(obj));
                                }
                                index => {
                                    if let Object::List(values) = &mut *obj.kind_mut() {
                                        let mut idx = value_to_int(index)? as isize;
                                        if idx < 0 {
                                            idx += values.len() as isize;
                                        }
                                        if idx < 0 || idx as usize >= values.len() {
                                            return Err(RuntimeError::new(
                                                "list index out of range",
                                            ));
                                        }
                                        values[idx as usize] = value;
                                    }
                                    self.push_value(Value::List(obj));
                                }
                            },
                            Value::ByteArray(obj) => match index {
                                Value::Slice { lower, upper, step } => {
                                    let replacement = self.value_to_bytes_payload(value)?;
                                    if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                        let step_value = step.unwrap_or(1);
                                        if step_value == 1 {
                                            let (start, stop) = slice_bounds_for_step_one(
                                                values.len(),
                                                lower,
                                                upper,
                                            );
                                            values.splice(start..stop, replacement);
                                        } else {
                                            let indices =
                                                slice_indices(values.len(), lower, upper, step)?;
                                            if indices.len() != replacement.len() {
                                                return Err(RuntimeError::new(format!(
                                                    "attempt to assign sequence of size {} to extended slice of size {}",
                                                    replacement.len(),
                                                    indices.len()
                                                )));
                                            }
                                            for (idx, item) in indices.into_iter().zip(replacement)
                                            {
                                                values[idx] = item;
                                            }
                                        }
                                    }
                                    self.push_value(Value::ByteArray(obj));
                                }
                                index => {
                                    if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                        let mut idx = value_to_int(index)? as isize;
                                        if idx < 0 {
                                            idx += values.len() as isize;
                                        }
                                        if idx < 0 || idx as usize >= values.len() {
                                            return Err(RuntimeError::new("index out of range"));
                                        }
                                        let byte = value_to_int(value)?;
                                        if !(0..=255).contains(&byte) {
                                            return Err(RuntimeError::new(
                                                "byte must be in range(0, 256)",
                                            ));
                                        }
                                        values[idx as usize] = byte as u8;
                                    }
                                    self.push_value(Value::ByteArray(obj));
                                }
                            },
                            Value::Instance(instance) => match index {
                                Value::Slice { .. } => {
                                    if self.instance_backing_dict(&instance).is_some() {
                                        return Err(RuntimeError::new(
                                            "slicing unsupported for dict",
                                        ));
                                    }
                                    let target_value = Value::Instance(instance.clone());
                                    if let Some(setitem) = self
                                        .lookup_bound_special_method(&target_value, "__setitem__")?
                                    {
                                        match self.call_internal(
                                            setitem,
                                            vec![index, value],
                                            HashMap::new(),
                                        )? {
                                            InternalCallOutcome::Value(_) => {}
                                            InternalCallOutcome::CallerExceptionHandled => {
                                                return Ok(None);
                                            }
                                        }
                                        self.push_value(target_value);
                                    } else {
                                        return Err(RuntimeError::new(
                                            "slice assignment not supported",
                                        ));
                                    }
                                }
                                index => {
                                    if let Some(backing_dict) = self.instance_backing_dict(&instance)
                                    {
                                        dict_set_value_checked(&backing_dict, index, value)?;
                                        self.push_value(Value::Instance(instance));
                                    } else {
                                        let target_value = Value::Instance(instance.clone());
                                        if let Some(setitem) = self
                                            .lookup_bound_special_method(
                                                &target_value,
                                                "__setitem__",
                                            )?
                                        {
                                            match self.call_internal(
                                                setitem,
                                                vec![index, value],
                                                HashMap::new(),
                                            )? {
                                                InternalCallOutcome::Value(_) => {}
                                                InternalCallOutcome::CallerExceptionHandled => {
                                                    return Ok(None);
                                                }
                                            }
                                            self.push_value(target_value);
                                        } else {
                                            return Err(RuntimeError::new(
                                                "store subscript unsupported type",
                                            ));
                                        }
                                    }
                                }
                            },
                            target => match index {
                                Value::Slice { .. } => {
                                    return Err(RuntimeError::new(
                                        "slice assignment not supported",
                                    ));
                                }
                                index => match target {
                                    Value::Dict(obj) => {
                                        dict_set_value_checked(&obj, index, value)?;
                                        self.push_value(Value::Dict(obj));
                                    }
                                    Value::MemoryView(obj) => {
                                        let source = match &*obj.kind() {
                                            Object::MemoryView(view) => view.source.clone(),
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "store subscript unsupported type",
                                                ));
                                            }
                                        };
                                        match &mut *source.kind_mut() {
                                            Object::ByteArray(values) => {
                                                let mut idx = value_to_int(index)? as isize;
                                                if idx < 0 {
                                                    idx += values.len() as isize;
                                                }
                                                if idx < 0 || idx as usize >= values.len() {
                                                    return Err(RuntimeError::new(
                                                        "index out of range",
                                                    ));
                                                }
                                                let byte = value_to_int(value)?;
                                                if !(0..=255).contains(&byte) {
                                                    return Err(RuntimeError::new(
                                                        "byte must be in range(0, 256)",
                                                    ));
                                                }
                                                values[idx as usize] = byte as u8;
                                            }
                                            Object::Bytes(_) => {
                                                return Err(RuntimeError::new(
                                                    "cannot modify read-only memory",
                                                ));
                                            }
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "store subscript unsupported type",
                                                ));
                                            }
                                        }
                                        self.push_value(Value::MemoryView(obj));
                                    }
                                    _ => {
                                        let target_value = target.clone();
                                        if let Some(setitem) =
                                            self.lookup_bound_special_method(&target, "__setitem__")?
                                        {
                                            match self.call_internal(
                                                setitem,
                                                vec![index, value],
                                                HashMap::new(),
                                            )? {
                                                InternalCallOutcome::Value(_) => {}
                                                InternalCallOutcome::CallerExceptionHandled => {
                                                    return Ok(None);
                                                }
                                            }
                                            self.push_value(target_value);
                                        } else {
                                            return Err(RuntimeError::new(
                                                "store subscript unsupported type",
                                            ));
                                        }
                                    }
                                },
                            },
                        };
                    }
                    Opcode::DeleteSubscript => {
                        let index = self.pop_value()?;
                        let target = self.pop_value()?;
                        match target {
                            Value::List(obj) => match index {
                                Value::Slice { lower, upper, step } => {
                                    if let Object::List(values) = &mut *obj.kind_mut() {
                                        let step_value = step.unwrap_or(1);
                                        if step_value == 1 {
                                            let (start, stop) =
                                                slice_bounds_for_step_one(values.len(), lower, upper);
                                            values.drain(start..stop);
                                        } else {
                                            let mut indices =
                                                slice_indices(values.len(), lower, upper, step)?;
                                            indices.sort_unstable();
                                            for idx in indices.into_iter().rev() {
                                                values.remove(idx);
                                            }
                                        }
                                    }
                                }
                                index => {
                                    if let Object::List(values) = &mut *obj.kind_mut() {
                                        let mut idx = value_to_int(index)? as isize;
                                        if idx < 0 {
                                            idx += values.len() as isize;
                                        }
                                        if idx < 0 || idx as usize >= values.len() {
                                            return Err(RuntimeError::new(
                                                "list index out of range",
                                            ));
                                        }
                                        values.remove(idx as usize);
                                    }
                                }
                            },
                            Value::ByteArray(obj) => match index {
                                Value::Slice { lower, upper, step } => {
                                    if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                        let step_value = step.unwrap_or(1);
                                        if step_value == 1 {
                                            let (start, stop) =
                                                slice_bounds_for_step_one(values.len(), lower, upper);
                                            values.drain(start..stop);
                                        } else {
                                            let mut indices =
                                                slice_indices(values.len(), lower, upper, step)?;
                                            indices.sort_unstable();
                                            for idx in indices.into_iter().rev() {
                                                values.remove(idx);
                                            }
                                        }
                                    }
                                }
                                index => {
                                    if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                        let mut idx = value_to_int(index)? as isize;
                                        if idx < 0 {
                                            idx += values.len() as isize;
                                        }
                                        if idx < 0 || idx as usize >= values.len() {
                                            return Err(RuntimeError::new("index out of range"));
                                        }
                                        values.remove(idx as usize);
                                    }
                                }
                            },
                            Value::Instance(instance) => match index {
                                Value::Slice { .. } => {
                                    if self.instance_backing_dict(&instance).is_some() {
                                        return Err(RuntimeError::new(
                                            "slice deletion not supported",
                                        ));
                                    }
                                    let target_value = Value::Instance(instance.clone());
                                    if let Some(delitem) = self
                                        .lookup_bound_special_method(&target_value, "__delitem__")?
                                    {
                                        match self.call_internal(
                                            delitem,
                                            vec![index],
                                            HashMap::new(),
                                        )? {
                                            InternalCallOutcome::Value(_) => {}
                                            InternalCallOutcome::CallerExceptionHandled => {
                                                return Ok(None);
                                            }
                                        }
                                    } else {
                                        return Err(RuntimeError::new(
                                            "slice deletion not supported",
                                        ));
                                    }
                                }
                                index => {
                                    if let Some(backing_dict) = self.instance_backing_dict(&instance)
                                    {
                                        ensure_hashable(&index)?;
                                        if dict_remove_value(&backing_dict, &index).is_none() {
                                            return Err(RuntimeError::new("key not found"));
                                        }
                                    } else {
                                        let target_value = Value::Instance(instance.clone());
                                        if let Some(delitem) = self.lookup_bound_special_method(
                                            &target_value,
                                            "__delitem__",
                                        )? {
                                            match self.call_internal(
                                                delitem,
                                                vec![index],
                                                HashMap::new(),
                                            )? {
                                                InternalCallOutcome::Value(_) => {}
                                                InternalCallOutcome::CallerExceptionHandled => {
                                                    return Ok(None);
                                                }
                                            }
                                        } else {
                                            return Err(RuntimeError::new(
                                                "delete subscript unsupported type",
                                            ));
                                        }
                                    }
                                }
                            },
                            target => match index {
                                Value::Slice { .. } => {
                                    return Err(RuntimeError::new("slice deletion not supported"));
                                }
                                index => match target {
                                    Value::Dict(obj) => {
                                        ensure_hashable(&index)?;
                                        if dict_remove_value(&obj, &index).is_none() {
                                            return Err(RuntimeError::new("key not found"));
                                        }
                                    }
                                    _ => {
                                        if let Some(delitem) = self
                                            .lookup_bound_special_method(&target, "__delitem__")?
                                        {
                                            match self.call_internal(
                                                delitem,
                                                vec![index],
                                                HashMap::new(),
                                            )? {
                                                InternalCallOutcome::Value(_) => {}
                                                InternalCallOutcome::CallerExceptionHandled => {
                                                    return Ok(None);
                                                }
                                            }
                                        } else {
                                            return Err(RuntimeError::new(
                                                "subscript deletion unsupported type",
                                            ));
                                        }
                                    }
                                },
                            },
                        };
                    }
                    Opcode::MakeFunction => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing function argument"))?
                            as usize;
                        let value =
                            {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
                            };
                        let code = match value {
                            Value::Code(code) => code,
                            _ => {
                                return Err(RuntimeError::new("expected code object for function"));
                            }
                        };
                        let kwonly_value = self.pop_value()?;
                        let kwonly_defaults = match kwonly_value {
                            Value::Dict(obj) => match &*obj.kind() {
                                Object::Dict(entries) => {
                                    let mut map = HashMap::new();
                                    for (key, value) in entries {
                                        let key = match key {
                                            Value::Str(name) => name.clone(),
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "kwonly default name must be string",
                                                ));
                                            }
                                        };
                                        map.insert(key, value.clone());
                                    }
                                    map
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "expected kwonly defaults dict for function",
                                    ));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected kwonly defaults dict for function",
                                ));
                            }
                        };
                        let defaults_value = self.pop_value()?;
                        let defaults = match defaults_value {
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => values.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "expected defaults tuple for function",
                                    ));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected defaults tuple for function",
                                ));
                            }
                        };
                        let module = {
                            let frame = self.frames.last().expect("frame exists");
                            if frame.return_class && is_comprehension_code(&code) {
                                frame.module.clone()
                            } else {
                                frame.function_globals.clone()
                            }
                        };
                        let func = FunctionObject::new(
                            code,
                            module,
                            defaults,
                            kwonly_defaults,
                            Vec::new(),
                            None,
                        );
                        self.push_value(self.heap.alloc_function(func));
                    }
                    Opcode::BuildClass => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing class code argument"))?
                            as usize;
                        let value =
                            {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
                            };
                        let code = match value {
                            Value::Code(code) => code,
                            _ => {
                                return Err(RuntimeError::new(
                                    "expected code object for class body",
                                ));
                            }
                        };
                        let top = self.pop_value()?;
                        let (class_keywords, metaclass_value) = match top {
                            Value::Dict(obj) => match &*obj.kind() {
                                Object::Dict(entries) => {
                                    let mut out = HashMap::new();
                                    for (key, value) in entries {
                                        let key = match key {
                                            Value::Str(name) => name.clone(),
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "class keyword names must be strings",
                                                ));
                                            }
                                        };
                                        out.insert(key, value.clone());
                                    }
                                    (out, self.pop_value()?)
                                }
                                _ => {
                                    return Err(RuntimeError::new(
                                        "class keyword arguments must be a dict",
                                    ));
                                }
                            },
                            other => (HashMap::new(), other),
                        };
                        let class_metaclass = if matches!(metaclass_value, Value::None) {
                            None
                        } else {
                            Some(metaclass_value)
                        };
                        let name_value = self.pop_value()?;
                        let bases_value = self.pop_value()?;
                        let class_name = match name_value {
                            Value::Str(name) => name,
                            _ => return Err(RuntimeError::new("class name must be a string")),
                        };
                        let bases = match bases_value {
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => values.clone(),
                                _ => return Err(RuntimeError::new("class bases must be a tuple")),
                            },
                            _ => return Err(RuntimeError::new("class bases must be a tuple")),
                        };
                        let mut base_classes = Vec::new();
                        for base in bases {
                            base_classes.push(self.class_from_base_value(base)?);
                        }

                        let class_qualname = self
                            .frames
                            .last()
                            .and_then(|frame| {
                                if !frame.return_class {
                                    return None;
                                }
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
                                Some(format!("{outer_qualname}.{class_name}"))
                            })
                            .unwrap_or_else(|| class_name.clone());

                        let class_module = match self
                            .heap
                            .alloc_module(ModuleObject::new(class_name.clone()))
                        {
                            Value::Module(obj) => obj,
                            _ => unreachable!(),
                        };
                        if let Object::Module(module_data) = &mut *class_module.kind_mut() {
                            module_data
                                .globals
                                .insert("__name__".to_string(), Value::Str(class_name));
                            module_data
                                .globals
                                .insert("__qualname__".to_string(), Value::Str(class_qualname));
                        }

                        let (outer_globals, outer_locals) = self
                            .frames
                            .last()
                            .map(|frame| {
                                (
                                    frame.function_globals.clone(),
                                    Self::class_lookup_fallback_from_frame(frame),
                                )
                            })
                            .unwrap_or_else(|| (self.main_module.clone(), None));
                        let class_closure = self.capture_closure_cells_for_code(&code)?;
                        let cells = self.build_cells(&code, class_closure);
                        let mut frame =
                            self.acquire_frame(code, class_module, true, false, cells, None);
                        frame.function_globals = outer_globals.clone();
                        frame.globals_fallback = Some(outer_globals);
                        frame.locals_fallback = outer_locals;
                        frame.locals.insert(
                            "__classdict__".to_string(),
                            self.heap.alloc_dict(Vec::new()),
                        );
                        frame.return_class = true;
                        frame.class_bases = base_classes;
                        frame.class_metaclass = class_metaclass;
                        frame.class_keywords = class_keywords;
                        self.frames.push(frame);
                    }
                    Opcode::MakeFunctionStack => {
                        let value = self.pop_value()?;
                        let code = match value {
                            Value::Code(code) => code,
                            _ => {
                                return Err(RuntimeError::new("expected code object for function"));
                            }
                        };
                        let module = {
                            let frame = self.frames.last().expect("frame exists");
                            if frame.return_class && is_comprehension_code(&code) {
                                frame.module.clone()
                            } else {
                                frame.function_globals.clone()
                            }
                        };
                        let func = FunctionObject::new(
                            code,
                            module,
                            Vec::new(),
                            HashMap::new(),
                            Vec::new(),
                            None,
                        );
                        self.push_value(self.heap.alloc_function(func));
                    }
                    Opcode::SetFunctionAttribute => {
                        let func_value = self.pop_value()?;
                        let attr = self.pop_value()?;
                        let func = match func_value {
                            Value::Function(func) => func,
                            _ => return Err(RuntimeError::new("expected function")),
                        };
                        let attr_kind = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing function attribute kind"))?;
                        match attr_kind {
                            0x01 => {
                                let defaults = match attr {
                                    Value::Tuple(obj) => match &*obj.kind() {
                                        Object::Tuple(values) => values.clone(),
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "defaults must be tuple",
                                            ));
                                        }
                                    },
                                    _ => return Err(RuntimeError::new("defaults must be tuple")),
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.defaults = defaults;
                                    func_data.refresh_plain_positional_call_arity();
                                }
                            }
                            0x02 => {
                                let kwonly = match attr {
                                    Value::Dict(obj) => match &*obj.kind() {
                                        Object::Dict(entries) => {
                                            let mut map = HashMap::new();
                                            for (key, value) in entries {
                                                let name = match key {
                                                    Value::Str(name) => name.clone(),
                                                    _ => {
                                                        return Err(RuntimeError::new(
                                                            "kwonly default name must be string",
                                                        ));
                                                    }
                                                };
                                                map.insert(name, value.clone());
                                            }
                                            map
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "kwonly defaults must be dict",
                                            ));
                                        }
                                    },
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "kwonly defaults must be dict",
                                        ));
                                    }
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.kwonly_defaults = kwonly;
                                    func_data.refresh_plain_positional_call_arity();
                                }
                            }
                            0x04 => {
                                let annotations = match attr {
                                    Value::Dict(obj) => obj,
                                    _ => return Err(RuntimeError::new("annotations must be dict")),
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.annotations = Some(annotations);
                                }
                            }
                            0x08 => {
                                let closure = match attr {
                                    Value::Tuple(obj) => match &*obj.kind() {
                                        Object::Tuple(values) => {
                                            let mut cells = Vec::with_capacity(values.len());
                                            for value in values {
                                                match value {
                                                    Value::Cell(cell) => cells.push(cell.clone()),
                                                    _ => {
                                                        return Err(RuntimeError::new(
                                                            "closure entries must be cells",
                                                        ));
                                                    }
                                                }
                                            }
                                            cells
                                        }
                                        _ => {
                                            return Err(RuntimeError::new("closure must be tuple"));
                                        }
                                    },
                                    _ => return Err(RuntimeError::new("closure must be tuple")),
                                };
                                if let Object::Function(func_data) = &mut *func.kind_mut() {
                                    func_data.closure = closure;
                                    func_data.touch_call_cache_epoch();
                                }
                            }
                            _ => {
                                // ignore annotations for now
                            }
                        }
                        self.push_value(Value::Function(func));
                    }
                    Opcode::CallFunction => {
                        let argc = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing call argument"))?
                            as usize;
                        if argc == 1 {
                            let site_index = self.current_site_index();
                            let quickened_one_arg = self
                                .is_quickened_site(site_index, QuickenedSiteKind::CallFunctionOneArg);
                            let (func, arg0) = {
                                let frame = self.frames.last_mut().expect("frame exists");
                                let arg0 = frame
                                    .stack
                                    .pop()
                                    .ok_or_else(|| RuntimeError::new("stack underflow (CallFunction arg0)"))?;
                                let func = frame
                                    .stack
                                    .pop()
                                    .ok_or_else(|| RuntimeError::new("stack underflow (CallFunction func)"))?;
                                (func, arg0)
                            };
                            match func {
                                Value::Function(func_obj) => {
                                    if !quickened_one_arg {
                                        self.mark_quickened_site(
                                            site_index,
                                            QuickenedSiteKind::CallFunctionOneArg,
                                        );
                                    }
                                    self.push_function_call_one_arg_from_obj(&func_obj, arg0)?;
                                }
                                other => {
                                    if quickened_one_arg {
                                        self.clear_quickened_site(site_index);
                                    }
                                    self.dispatch_call_no_kwargs(other, vec![arg0])?
                                }
                            }
                        } else if argc == 2 {
                            let (func, arg0, arg1) = {
                                let frame = self.frames.last_mut().expect("frame exists");
                                let arg1 = frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (CallFunction arg1)")
                                })?;
                                let arg0 = frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (CallFunction arg0)")
                                })?;
                                let func = frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (CallFunction func)")
                                })?;
                                (func, arg0, arg1)
                            };
                            match func {
                                Value::Function(func_obj) => {
                                    self.push_function_call_two_args_from_obj(&func_obj, arg0, arg1)?;
                                }
                                other => self.dispatch_call_no_kwargs(other, vec![arg0, arg1])?,
                            }
                        } else if argc == 3 {
                            let (func, arg0, arg1, arg2) = {
                                let frame = self.frames.last_mut().expect("frame exists");
                                let arg2 = frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (CallFunction arg2)")
                                })?;
                                let arg1 = frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (CallFunction arg1)")
                                })?;
                                let arg0 = frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (CallFunction arg0)")
                                })?;
                                let func = frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (CallFunction func)")
                                })?;
                                (func, arg0, arg1, arg2)
                            };
                            match func {
                                Value::Function(func_obj) => {
                                    self.push_function_call_three_args_from_obj(
                                        &func_obj, arg0, arg1, arg2,
                                    )?;
                                }
                                other => {
                                    self.dispatch_call_no_kwargs(other, vec![arg0, arg1, arg2])?
                                }
                            }
                        } else {
                            let mut args = Vec::with_capacity(argc);
                            for _ in 0..argc {
                                args.push(self.pop_value()?);
                            }
                            args.reverse();
                            let func = self.pop_value()?;
                            self.dispatch_call_no_kwargs(func, args)?;
                        }
                    }
                    Opcode::CallFunction1 => {
                        let site_index = self.current_site_index();
                        let quickened_one_arg =
                            self.is_quickened_site(site_index, QuickenedSiteKind::CallFunctionOneArg);
                        let (func, arg0) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            let arg0 = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (CallFunction1 arg0)"))?;
                            let func = frame
                                .stack
                                .pop()
                                .ok_or_else(|| RuntimeError::new("stack underflow (CallFunction1 func)"))?;
                            (func, arg0)
                        };
                        match func {
                            Value::Function(func_obj) => {
                                if !quickened_one_arg {
                                    self.mark_quickened_site(
                                        site_index,
                                        QuickenedSiteKind::CallFunctionOneArg,
                                    );
                                }
                                self.push_function_call_one_arg_from_obj(&func_obj, arg0)?;
                            }
                            other => {
                                if quickened_one_arg {
                                    self.clear_quickened_site(site_index);
                                }
                                self.dispatch_call_no_kwargs(other, vec![arg0])?
                            }
                        }
                    }
                    Opcode::CallCpython => {
                        let arg = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing call argument"))?;
                        let pos_count = (arg & 0xFFFF) as usize;
                        let kw_idx = (arg >> 16) as u16;
                        let kw_names = if kw_idx == u16::MAX {
                            None
                        } else {
                            let idx = kw_idx as usize;
                            let value = {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
                            };
                            Some(value)
                        };

                        let kw_names = if let Some(value) = kw_names {
                            match value {
                                Value::Tuple(obj) => match &*obj.kind() {
                                    Object::Tuple(values) => {
                                        let mut names = Vec::new();
                                        for value in values {
                                            match value {
                                                Value::Str(name) => names.push(name.clone()),
                                                _ => {
                                                    return Err(RuntimeError::new(
                                                        "kw_names must be tuple of strings",
                                                    ));
                                                }
                                            }
                                        }
                                        Some(names)
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "kw_names must be tuple of strings",
                                        ));
                                    }
                                },
                                Value::None => None,
                                _ => {
                                    return Err(RuntimeError::new(
                                        "kw_names must be tuple of strings",
                                    ));
                                }
                            }
                        } else {
                            None
                        };

                        let kw_count = kw_names.as_ref().map(|names| names.len()).unwrap_or(0);
                        if pos_count < kw_count {
                            return Err(RuntimeError::new("call arg count mismatch"));
                        }
                        let mut kwargs = HashMap::new();
                        for idx in (0..kw_count).rev() {
                            let value = self.pop_value()?;
                            let name = kw_names
                                .as_ref()
                                .expect("kw names")
                                .get(idx)
                                .cloned()
                                .ok_or_else(|| RuntimeError::new("kw name index out of range"))?;
                            kwargs.insert(name, value);
                        }
                        let mut args = Vec::with_capacity(pos_count - kw_count);
                        for _ in 0..(pos_count - kw_count) {
                            args.push(self.pop_value()?);
                        }
                        args.reverse();
                        let mut func = self.pop_value()?;
                        if matches!(func, Value::None) {
                            func = self.pop_value()?;
                        }
                        if let Some(Value::None) = self
                            .frames
                            .last()
                            .and_then(|frame| frame.stack.last())
                            .cloned()
                        {
                            let _ = self.pop_value();
                        }

                        match func {
                            Value::Function(func) => {
                                self.push_function_call_from_obj(&func, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(_) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call_from_obj(
                                            &method_data.function,
                                            bound_args,
                                            kwargs,
                                        )?;
                                    }
                                    Object::NativeMethod(native) => {
                                        let caller_depth = self.frames.len();
                                        let caller_idx = caller_depth.saturating_sub(1);
                                        let caller_ip = self
                                            .frames
                                            .get(caller_idx)
                                            .map(|frame| frame.ip)
                                            .unwrap_or(0);
                                        let call_result = self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        );
                                        self.finalize_native_opcode_call(
                                            caller_depth,
                                            caller_ip,
                                            call_result,
                                        )?;
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
                            }
                            Value::Class(class) => {
                                match self.call_internal(Value::Class(class), args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::Builtin(BuiltinFunction::BuildClass) => {
                                let class_value = self.call_build_class(args, kwargs)?;
                                if let Some(value) = class_value {
                                    self.push_value(value);
                                }
                            }
                            Value::Builtin(builtin) => {
                                let caller_depth = self.frames.len();
                                let caller_idx = caller_depth.saturating_sub(1);
                                let caller_ip = self
                                    .frames
                                    .get(caller_idx)
                                    .map(|frame| frame.ip)
                                    .unwrap_or(0);
                                let call_result = self.call_builtin(builtin, args, kwargs);
                                self.finalize_builtin_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                            Value::Instance(instance) => {
                                let receiver = Value::Instance(instance.clone());
                                let call_target = self
                                    .lookup_bound_special_method(&receiver, "__call__")?
                                    .ok_or_else(|| {
                                        RuntimeError::new("attempted to call non-function")
                                    })?;
                                match self.call_internal(call_target, args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::ExceptionType(name) => {
                                let value =
                                    self.instantiate_exception_type(&name, &args, &kwargs)?;
                                self.push_value(value);
                            }
                            _ => return Err(RuntimeError::new("attempted to call non-function")),
                        }
                    }
                    Opcode::CallCpythonKwStack => {
                        let pos_total = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing call argument"))?
                            as usize;
                        let kw_names_value = self.pop_value()?;
                        let kw_names = match kw_names_value {
                            Value::Tuple(obj) => match &*obj.kind() {
                                Object::Tuple(values) => {
                                    let mut names = Vec::new();
                                    for value in values {
                                        match value {
                                            Value::Str(name) => names.push(name.clone()),
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "kw names must be strings",
                                                ));
                                            }
                                        }
                                    }
                                    names
                                }
                                _ => return Err(RuntimeError::new("kw names must be tuple")),
                            },
                            _ => return Err(RuntimeError::new("kw names must be tuple")),
                        };
                        let kw_count = kw_names.len();
                        if pos_total < kw_count {
                            return Err(RuntimeError::new("call arg count mismatch"));
                        }
                        let mut kwargs = HashMap::new();
                        for idx in (0..kw_count).rev() {
                            let value = self.pop_value()?;
                            let name = kw_names
                                .get(idx)
                                .cloned()
                                .ok_or_else(|| RuntimeError::new("kw name index out of range"))?;
                            kwargs.insert(name, value);
                        }
                        let mut args = Vec::with_capacity(pos_total - kw_count);
                        for _ in 0..(pos_total - kw_count) {
                            args.push(self.pop_value()?);
                        }
                        args.reverse();
                        let mut func = self.pop_value()?;
                        if matches!(func, Value::None) {
                            func = self.pop_value()?;
                        }
                        if let Some(Value::None) = self
                            .frames
                            .last()
                            .and_then(|frame| frame.stack.last())
                            .cloned()
                        {
                            let _ = self.pop_value();
                        }

                        match func {
                            Value::Function(func) => {
                                self.push_function_call_from_obj(&func, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(_) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call_from_obj(
                                            &method_data.function,
                                            bound_args,
                                            kwargs,
                                        )?;
                                    }
                                    Object::NativeMethod(native) => {
                                        let caller_depth = self.frames.len();
                                        let caller_idx = caller_depth.saturating_sub(1);
                                        let caller_ip = self
                                            .frames
                                            .get(caller_idx)
                                            .map(|frame| frame.ip)
                                            .unwrap_or(0);
                                        let call_result = self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        );
                                        self.finalize_native_opcode_call(
                                            caller_depth,
                                            caller_ip,
                                            call_result,
                                        )?;
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
                            }
                            Value::Class(class) => {
                                match self.call_internal(Value::Class(class), args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::Builtin(BuiltinFunction::BuildClass) => {
                                let class_value = self.call_build_class(args, kwargs)?;
                                if let Some(value) = class_value {
                                    self.push_value(value);
                                }
                            }
                            Value::Builtin(builtin) => {
                                let caller_depth = self.frames.len();
                                let caller_idx = caller_depth.saturating_sub(1);
                                let caller_ip = self
                                    .frames
                                    .get(caller_idx)
                                    .map(|frame| frame.ip)
                                    .unwrap_or(0);
                                let call_result = self.call_builtin(builtin, args, kwargs);
                                self.finalize_builtin_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                            Value::Instance(instance) => {
                                let receiver = Value::Instance(instance.clone());
                                let call_target = self
                                    .lookup_bound_special_method(&receiver, "__call__")?
                                    .ok_or_else(|| {
                                        RuntimeError::new("attempted to call non-function")
                                    })?;
                                match self.call_internal(call_target, args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::ExceptionType(name) => {
                                let value =
                                    self.instantiate_exception_type(&name, &args, &kwargs)?;
                                self.push_value(value);
                            }
                            _ => return Err(RuntimeError::new("attempted to call non-function")),
                        }
                    }
                    Opcode::CallFunctionKw => {
                        let arg = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing call argument"))?;
                        let (pos_count, kw_count) = decode_call_counts(arg);
                        let mut kwargs = HashMap::new();
                        for _ in 0..kw_count {
                            let value = self.pop_value()?;
                            let name = self.pop_value()?;
                            let name = match name {
                                Value::Str(name) => name,
                                _ => return Err(RuntimeError::new("keyword name must be string")),
                            };
                            if kwargs.contains_key(&name) {
                                return Err(RuntimeError::new("duplicate keyword argument"));
                            }
                            kwargs.insert(name, value);
                        }
                        let mut args = Vec::with_capacity(pos_count);
                        for _ in 0..pos_count {
                            args.push(self.pop_value()?);
                        }
                        args.reverse();
                        let func = self.pop_value()?;
                        match func {
                            Value::Function(func) => {
                                self.push_function_call_from_obj(&func, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(_) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call_from_obj(
                                            &method_data.function,
                                            bound_args,
                                            kwargs,
                                        )?;
                                    }
                                    Object::NativeMethod(native) => {
                                        let caller_depth = self.frames.len();
                                        let caller_idx = caller_depth.saturating_sub(1);
                                        let caller_ip = self
                                            .frames
                                            .get(caller_idx)
                                            .map(|frame| frame.ip)
                                            .unwrap_or(0);
                                        let call_result = self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        );
                                        self.finalize_native_opcode_call(
                                            caller_depth,
                                            caller_ip,
                                            call_result,
                                        )?;
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
                            }
                            Value::Class(class) => {
                                match self.call_internal(Value::Class(class), args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::Builtin(builtin) => {
                                let caller_depth = self.frames.len();
                                let caller_idx = caller_depth.saturating_sub(1);
                                let caller_ip = self
                                    .frames
                                    .get(caller_idx)
                                    .map(|frame| frame.ip)
                                    .unwrap_or(0);
                                let call_result = self.call_builtin(builtin, args, kwargs);
                                self.finalize_builtin_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                            Value::Instance(instance) => {
                                let receiver = Value::Instance(instance.clone());
                                let call_target = self
                                    .lookup_bound_special_method(&receiver, "__call__")?
                                    .ok_or_else(|| {
                                        RuntimeError::new("attempted to call non-function")
                                    })?;
                                match self.call_internal(call_target, args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::ExceptionType(name) => {
                                let value =
                                    self.instantiate_exception_type(&name, &args, &kwargs)?;
                                self.push_value(value);
                            }
                            _ => return Err(RuntimeError::new("attempted to call non-function")),
                        }
                    }
                    Opcode::CallFunctionVar => {
                        let kwargs_value = self.pop_value()?;
                        let args_value = self.pop_value()?;
                        let func = self.pop_value()?;
                        let kwargs = match kwargs_value {
                            Value::Dict(obj) => match &*obj.kind() {
                                Object::Dict(entries) => {
                                    let mut map = HashMap::new();
                                    for (key, value) in entries {
                                        let key = match key {
                                            Value::Str(name) => name.clone(),
                                            _ => {
                                                return Err(RuntimeError::new(
                                                    "keyword name must be string",
                                                ));
                                            }
                                        };
                                        if map.contains_key(&key) {
                                            return Err(RuntimeError::new(
                                                "duplicate keyword argument",
                                            ));
                                        }
                                        map.insert(key, value.clone());
                                    }
                                    map
                                }
                                _ => return Err(RuntimeError::new("call kwargs must be dict")),
                            },
                            _ => return Err(RuntimeError::new("call kwargs must be dict")),
                        };
                        let args = match args_value {
                            Value::List(obj) => match &*obj.kind() {
                                Object::List(values) => values.clone(),
                                _ => return Err(RuntimeError::new("call args must be list")),
                            },
                            _ => return Err(RuntimeError::new("call args must be list")),
                        };

                        match func {
                            Value::Function(func) => {
                                self.push_function_call_from_obj(&func, args, kwargs)?;
                            }
                            Value::BoundMethod(method) => {
                                let method_data = match &*method.kind() {
                                    Object::BoundMethod(data) => data.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                };
                                match &*method_data.function.kind() {
                                    Object::Function(_) => {
                                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                                        bound_args
                                            .push(self.receiver_value(&method_data.receiver)?);
                                        bound_args.extend(args);
                                        self.push_function_call_from_obj(
                                            &method_data.function,
                                            bound_args,
                                            kwargs,
                                        )?;
                                    }
                                    Object::NativeMethod(native) => {
                                        let caller_depth = self.frames.len();
                                        let caller_idx = caller_depth.saturating_sub(1);
                                        let caller_ip = self
                                            .frames
                                            .get(caller_idx)
                                            .map(|frame| frame.ip)
                                            .unwrap_or(0);
                                        let call_result = self.call_native_method(
                                            native.kind,
                                            method_data.receiver.clone(),
                                            args,
                                            kwargs,
                                        );
                                        self.finalize_native_opcode_call(
                                            caller_depth,
                                            caller_ip,
                                            call_result,
                                        )?;
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "attempted to call non-function",
                                        ));
                                    }
                                }
                            }
                            Value::Class(class) => {
                                match self.call_internal(Value::Class(class), args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::Builtin(builtin) => {
                                let caller_depth = self.frames.len();
                                let caller_idx = caller_depth.saturating_sub(1);
                                let caller_ip = self
                                    .frames
                                    .get(caller_idx)
                                    .map(|frame| frame.ip)
                                    .unwrap_or(0);
                                let call_result = self.call_builtin(builtin, args, kwargs);
                                self.finalize_builtin_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                            Value::Instance(instance) => {
                                let receiver = Value::Instance(instance.clone());
                                let call_target = self
                                    .lookup_bound_special_method(&receiver, "__call__")?
                                    .ok_or_else(|| {
                                        RuntimeError::new("attempted to call non-function")
                                    })?;
                                match self.call_internal(call_target, args, kwargs)? {
                                    InternalCallOutcome::Value(value) => self.push_value(value),
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                            Value::ExceptionType(name) => {
                                let value =
                                    self.instantiate_exception_type(&name, &args, &kwargs)?;
                                self.push_value(value);
                            }
                            _ => return Err(RuntimeError::new("attempted to call non-function")),
                        }
                    }
                    Opcode::ImportName => {
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing import argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            if let Some(Value::Str(name)) = frame.code.constants.get(idx) {
                                name.clone()
                            } else if let Some(name) = frame.code.names.get(idx) {
                                name.clone()
                            } else {
                                return Err(RuntimeError::new("import name index out of range"));
                            }
                        };
                        let module = self.import_module_object(&name)?;
                        let result_module = self.module_for_plain_import(&name, module);
                        self.push_value_to_caller_frame(caller_idx, Value::Module(result_module))?;
                    }
                    Opcode::ImportNameCpython => {
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing import argument"))?
                            as usize;
                        let name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame.code.names.get(idx).cloned().ok_or_else(|| {
                                RuntimeError::new("import name index out of range")
                            })?
                        };
                        let fromlist = self.pop_value()?;
                        let level_value = self.pop_value()?;
                        let level = value_to_int(level_value)?;
                        if level < 0 {
                            return Err(RuntimeError::new("negative import level"));
                        }
                        let resolved_name = self.resolve_import_name(&name, level as usize)?;
                        let module = self.import_module_object(&resolved_name)?;
                        let result_module = if self.fromlist_requested(&fromlist) {
                            module
                        } else {
                            self.module_for_plain_import(&resolved_name, module)
                        };
                        self.push_value_to_caller_frame(caller_idx, Value::Module(result_module))?;
                    }
                    Opcode::ImportFromCpython => {
                        let caller_idx = self.frames.len().saturating_sub(1);
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing import argument"))?
                            as usize;
                        let attr_name = {
                            let frame = self.frames.last().expect("frame exists");
                            frame.code.names.get(idx).cloned().ok_or_else(|| {
                                RuntimeError::new("import name index out of range")
                            })?
                        };
                        let module = self
                            .frames
                            .get(caller_idx)
                            .and_then(|frame| frame.stack.last())
                            .cloned()
                            .ok_or_else(|| {
                                RuntimeError::new("stack underflow (ImportFromCpython module)")
                            })?;
                        match module {
                            Value::Module(module_obj) => {
                                if attr_name == "*" {
                                    let exports = match &*module_obj.kind() {
                                        Object::Module(module_data) => {
                                            if let Some(all_names) =
                                                module_data.globals.get("__all__")
                                            {
                                                match all_names {
                                                    Value::List(obj) => match &*obj.kind() {
                                                        Object::List(values) => values
                                                            .iter()
                                                            .filter_map(|value| match value {
                                                                Value::Str(name) => {
                                                                    Some(name.clone())
                                                                }
                                                                _ => None,
                                                            })
                                                            .collect::<Vec<_>>(),
                                                        _ => Vec::new(),
                                                    },
                                                    Value::Tuple(obj) => match &*obj.kind() {
                                                        Object::Tuple(values) => values
                                                            .iter()
                                                            .filter_map(|value| match value {
                                                                Value::Str(name) => {
                                                                    Some(name.clone())
                                                                }
                                                                _ => None,
                                                            })
                                                            .collect::<Vec<_>>(),
                                                        _ => Vec::new(),
                                                    },
                                                    _ => Vec::new(),
                                                }
                                            } else {
                                                module_data
                                                    .globals
                                                    .keys()
                                                    .filter(|name| !name.starts_with('_'))
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                            }
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                "import from expects module object",
                                            ));
                                        }
                                    };
                                    let values = match &*module_obj.kind() {
                                        Object::Module(module_data) => exports
                                            .into_iter()
                                            .filter_map(|name| {
                                                module_data
                                                    .globals
                                                    .get(&name)
                                                    .cloned()
                                                    .map(|value| (name, value))
                                            })
                                            .collect::<Vec<_>>(),
                                        _ => Vec::new(),
                                    };
                                    let frame = if let Some(frame) = self.frames.get_mut(caller_idx)
                                    {
                                        frame
                                    } else if let Some(frame) = self.frames.last_mut() {
                                        frame
                                    } else {
                                        return Err(RuntimeError::new(
                                            "import caller frame missing",
                                        ));
                                    };
                                    let mut touched_globals_version = None;
                                    {
                                        for (name, value) in values {
                                            if let Some(slot_idx) =
                                                frame.code.name_to_index.get(&name).copied()
                                            {
                                                if let Some(slot) = frame.fast_locals.get_mut(slot_idx)
                                                {
                                                    Self::write_fast_local_slot(slot, value.clone());
                                                }
                                                if let Some(existing) = frame.locals.get_mut(&name) {
                                                    *existing = value.clone();
                                                }
                                            } else {
                                                frame.locals.insert(name.clone(), value.clone());
                                            }
                                            if let Object::Module(module_data) =
                                                &mut *frame.function_globals.kind_mut()
                                            {
                                                module_data.globals.insert(name, value);
                                                module_data.touch_globals_version();
                                                touched_globals_version = Some((
                                                    frame.function_globals.id(),
                                                    module_data.globals_version,
                                                ));
                                            }
                                        }
                                        frame.stack.push(Value::None);
                                    }
                                    if let Some((module_id, version)) = touched_globals_version {
                                        self.propagate_module_globals_version(module_id, version);
                                    }
                                    return Ok(None);
                                }

                                let module_name = match &*module_obj.kind() {
                                    Object::Module(module_data) => module_data.name.clone(),
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "import from expects module object",
                                        ));
                                    }
                                };
                                let attr = match self.load_attr_module(&module_obj, &attr_name) {
                                    Ok(attr) => attr,
                                    Err(load_err) => {
                                        if let Some(module) =
                                            self.load_submodule(&module_obj, &attr_name)
                                        {
                                            Value::Module(module)
                                        } else if load_err
                                            .message
                                            .contains("has no attribute")
                                        {
                                            return Err(RuntimeError::new(format!(
                                                "cannot import name '{}' from '{}'",
                                                attr_name, module_name
                                            )));
                                        } else {
                                            return Err(load_err);
                                        }
                                    }
                                };
                                self.push_value_to_caller_frame(caller_idx, attr)?;
                            }
                            _ => {
                                return Err(RuntimeError::new("import from expects module object"));
                            }
                        }
                    }
                    Opcode::JumpIfFalse => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let value = self.pop_value()?;
                        let truthy = match value {
                            Value::Bool(flag) => flag,
                            other => self.truthy_from_value(&other)?,
                        };
                        if !truthy {
                            let frame = self.frames.last_mut().expect("frame exists");
                            frame.ip = target;
                        }
                    }
                    Opcode::JumpIfTrue => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let value = self.pop_value()?;
                        let truthy = match value {
                            Value::Bool(flag) => flag,
                            other => self.truthy_from_value(&other)?,
                        };
                        if truthy {
                            let frame = self.frames.last_mut().expect("frame exists");
                            frame.ip = target;
                        }
                    }
                    Opcode::JumpIfNone => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let value = self.pop_value()?;
                        if matches!(value, Value::None) {
                            let frame = self.frames.last_mut().expect("frame exists");
                            frame.ip = target;
                        }
                    }
                    Opcode::JumpIfNotNone => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let value = self.pop_value()?;
                        if !matches!(value, Value::None) {
                            let frame = self.frames.last_mut().expect("frame exists");
                            frame.ip = target;
                        }
                    }
                    Opcode::DupTop => {
                        let value = self
                            .frames
                            .last()
                            .and_then(|frame| frame.stack.last())
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("stack underflow (CopyTop)"))?;
                        self.push_value(value);
                    }
                    Opcode::Jump => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame.ip = target;
                    }
                    Opcode::EndFor => {
                        // END_FOR is a sentinel in CPython; no-op for now.
                    }
                    Opcode::GetIter => {
                        let value = self.pop_value()?;
                        self.ensure_sync_iterator_target(&value)?;
                        let iterator = self
                            .to_iterator_value(value)
                            .map_err(|_| RuntimeError::new("object is not iterable"))?;
                        self.push_value(iterator);
                    }
                    Opcode::ForIter => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing jump target"))?
                            as usize;
                        let iterator_value = self.pop_value()?;
                        match iterator_value {
                            Value::Generator(obj) => match self.generator_for_iter_next(&obj)? {
                                GeneratorResumeOutcome::Yield(value) => {
                                    self.push_value(Value::Generator(obj));
                                    self.push_value(value);
                                }
                                GeneratorResumeOutcome::Complete(_) => {
                                    let frame = self.frames.last_mut().expect("frame exists");
                                    frame.ip = target;
                                }
                                GeneratorResumeOutcome::PropagatedException => {
                                    self.propagate_pending_generator_exception()?;
                                }
                            },
                            Value::Iterator(iterator_ref) => {
                                let next_value = self.iterator_next_value(&iterator_ref)?;
                                if let Some(value) = next_value {
                                    self.push_value(Value::Iterator(iterator_ref));
                                    self.push_value(value);
                                } else {
                                    let frame = self.frames.last_mut().expect("frame exists");
                                    frame.ip = target;
                                }
                            }
                            Value::Instance(instance) => {
                                let iterator = Value::Instance(instance.clone());
                                match self.next_from_iterator_value(&iterator)? {
                                    GeneratorResumeOutcome::Yield(value) => {
                                        self.push_value(Value::Instance(instance));
                                        self.push_value(value);
                                    }
                                    GeneratorResumeOutcome::Complete(_) => {
                                        let frame = self.frames.last_mut().expect("frame exists");
                                        frame.ip = target;
                                    }
                                    GeneratorResumeOutcome::PropagatedException => {
                                        if self
                                            .frames
                                            .last()
                                            .and_then(|frame| frame.active_exception.as_ref())
                                            .is_some()
                                        {
                                            return Ok(None);
                                        }
                                        return Err(RuntimeError::new("iterator __next__ failed"));
                                    }
                                }
                            }
                            _ => return Err(RuntimeError::new("FOR_ITER expects iterator")),
                        }
                    }
                    Opcode::YieldValue => {
                        let owner = self
                            .frames
                            .last()
                            .and_then(|frame| frame.generator_owner.clone())
                            .ok_or_else(|| RuntimeError::new("yield outside generator"))?;
                        let yielded = self.pop_value()?;
                        let mut frame = self.frames.pop().expect("frame exists");
                        let resume_kind = frame
                            .generator_resume_kind
                            .take()
                            .unwrap_or(GeneratorResumeKind::Next);
                        frame.generator_awaiting_resume_value = true;
                        frame.generator_pending_throw = None;
                        frame.generator_resume_value = None;
                        let owner_id = owner.id();
                        self.set_generator_running(&owner, false)?;
                        self.set_generator_started(&owner, true)?;
                        self.generator_states.insert(owner_id, frame);
                        if resume_kind == GeneratorResumeKind::Close {
                            return Err(RuntimeError::new("generator ignored GeneratorExit"));
                        }
                        if self.active_generator_resume == Some(owner_id) {
                            self.generator_resume_outcome =
                                Some(GeneratorResumeOutcome::Yield(yielded));
                        } else if let Some(caller) = self.frames.last_mut() {
                            caller.stack.push(yielded);
                        } else {
                            return Ok(Some(Value::None));
                        }
                    }
                    Opcode::Send => {
                        let target = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing send target"))?
                            as usize;
                        let sent = self.pop_value()?;
                        let iter = self.pop_value()?;
                        match self.delegate_yield_from(
                            &iter,
                            sent,
                            None,
                            GeneratorResumeKind::Next,
                        )? {
                            GeneratorResumeOutcome::Yield(value) => {
                                self.push_value(iter);
                                self.push_value(value);
                            }
                            GeneratorResumeOutcome::Complete(value) => {
                                self.push_value(value);
                                let frame = self.frames.last_mut().expect("frame exists");
                                frame.ip = target;
                            }
                            GeneratorResumeOutcome::PropagatedException => {
                                self.propagate_pending_generator_exception()?;
                                return Ok(None);
                            }
                        }
                    }
                    Opcode::YieldFrom => {
                        let owner = self
                            .frames
                            .last()
                            .and_then(|frame| frame.generator_owner.clone())
                            .ok_or_else(|| RuntimeError::new("yield from outside generator"))?;
                        let owner_id = owner.id();
                        let (iter_opt, source_opt, sent, thrown, resume_kind) = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            let source = if frame.yield_from_iter.is_some() {
                                None
                            } else {
                                Some(frame.stack.pop().ok_or_else(|| {
                                    RuntimeError::new("stack underflow (Send source)")
                                })?)
                            };
                            let iter = frame.yield_from_iter.take();
                            let sent = frame.generator_resume_value.take().unwrap_or(Value::None);
                            let thrown = frame.generator_pending_throw.take();
                            let resume_kind = frame
                                .generator_resume_kind
                                .take()
                                .unwrap_or(GeneratorResumeKind::Next);
                            (iter, source, sent, thrown, resume_kind)
                        };
                        let iter = if let Some(iter) = iter_opt {
                            iter
                        } else {
                            self.to_iterator_value(source_opt.expect("source present"))?
                        };
                        match self.delegate_yield_from(&iter, sent, thrown, resume_kind)? {
                            GeneratorResumeOutcome::Yield(value) => {
                                let mut frame = self.frames.pop().expect("frame exists");
                                frame.ip = frame.ip.saturating_sub(1);
                                frame.yield_from_iter = Some(iter);
                                frame.generator_awaiting_resume_value = false;
                                frame.generator_resume_value = None;
                                frame.generator_pending_throw = None;
                                self.set_generator_running(&owner, false)?;
                                self.set_generator_started(&owner, true)?;
                                self.generator_states.insert(owner_id, frame);
                                if resume_kind == GeneratorResumeKind::Close {
                                    return Err(RuntimeError::new(
                                        "generator ignored GeneratorExit",
                                    ));
                                }
                                if self.active_generator_resume == Some(owner_id) {
                                    self.generator_resume_outcome =
                                        Some(GeneratorResumeOutcome::Yield(value));
                                } else if let Some(caller) = self.frames.last_mut() {
                                    caller.stack.push(value);
                                } else {
                                    return Ok(Some(Value::None));
                                }
                            }
                            GeneratorResumeOutcome::Complete(value) => {
                                let frame = self.frames.last_mut().expect("frame exists");
                                frame.yield_from_iter = None;
                                frame.generator_resume_value = None;
                                frame.generator_pending_throw = None;
                                frame.generator_awaiting_resume_value = false;
                                frame.stack.push(value);
                            }
                            GeneratorResumeOutcome::PropagatedException => {
                                self.propagate_pending_generator_exception()?;
                                return Ok(None);
                            }
                        }
                    }
                    Opcode::SetupAnnotations => {
                        let dict = self.heap.alloc_dict(Vec::new());
                        self.store_name("__annotations__".to_string(), dict);
                    }
                    Opcode::SetupExcept => {
                        let handler = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing handler target"))?
                            as usize;
                        let frame = self.frames.last_mut().expect("frame exists");
                        let stack_len = frame.stack.len();
                        frame.blocks.push(Block { handler, stack_len });
                    }
                    Opcode::PopBlock => {
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame
                            .blocks
                            .pop()
                            .ok_or_else(|| RuntimeError::new("no block to pop"))?;
                    }
                    Opcode::Raise => {
                        let mode = instr.arg.unwrap_or(1);
                        match mode {
                            0 => {
                                let frame = self.frames.last().expect("frame exists");
                                let value = frame
                                    .active_exception
                                    .clone()
                                    .or_else(|| {
                                        frame
                                            .stack
                                            .iter()
                                            .rev()
                                            .find(|value| matches!(value, Value::Exception(_)))
                                            .cloned()
                                    })
                                    .ok_or_else(|| {
                                        let location = frame.code.locations.get(frame.last_ip);
                                        let line = location.map(|loc| loc.line).unwrap_or(0);
                                        let column = location.map(|loc| loc.column).unwrap_or(0);
                                        RuntimeError::new(format!(
                                            "no active exception to reraise at {}:{}:{} in {}",
                                            frame.code.filename, line, column, frame.code.name
                                        ))
                                    })?;
                                self.raise_exception(value)?;
                            }
                            1 => {
                                let value = self.pop_value()?;
                                self.raise_exception(value)?;
                            }
                            2 => {
                                let cause = self.pop_value()?;
                                let value = self.pop_value()?;
                                self.raise_exception_with_cause(value, Some(cause))?;
                            }
                            _ => {
                                return Err(RuntimeError::new("invalid raise mode"));
                            }
                        }
                    }
                    Opcode::MatchException => {
                        let handler_type = self.pop_value()?;
                        let exception = self.pop_value()?;
                        let matches = self.exception_matches(&exception, &handler_type)?;
                        self.push_value(Value::Bool(matches));
                    }
                    Opcode::MatchExceptionStar => {
                        let handler_type = self.pop_value()?;
                        let exception = self.pop_value()?;
                        let (matched, remaining) =
                            self.exception_split_for_star(&exception, &handler_type)?;
                        let matched_value = matched.map(Value::Exception).unwrap_or(Value::None);
                        if let Some(frame) = self.frames.last_mut() {
                            frame.active_exception = match &matched_value {
                                Value::Exception(exc) => Some(Value::Exception(exc.clone())),
                                _ => frame.active_exception.clone(),
                            };
                        }
                        self.push_value(matched_value);
                        self.push_value(remaining.map(Value::Exception).unwrap_or(Value::None));
                    }
                    Opcode::ClearException => {
                        if let Some(frame) = self.frames.last_mut() {
                            frame.active_exception = None;
                        }
                    }
                    Opcode::PopTop => {
                        let _ = self.pop_value()?;
                    }
                    Opcode::ReturnConst => {
                        let idx = instr
                            .arg
                            .ok_or_else(|| RuntimeError::new("missing const argument"))?
                            as usize;
                        let value =
                            {
                                let frame = self.frames.last().expect("frame exists");
                                frame.code.constants.get(idx).cloned().ok_or_else(|| {
                                    RuntimeError::new("constant index out of range")
                                })?
                            };
                        let mut frame = self.frames.pop().expect("frame exists");
                        if let Some(module_dict) = frame.module_locals_dict.take() {
                            self.sync_module_locals_dict_to_module(&frame.module, &module_dict);
                        }
                        if frame.expect_none_return && value != Value::None {
                            return Err(RuntimeError::new("__init__() should return None"));
                        }
                        let can_recycle = !frame.is_module
                            && frame.generator_owner.is_none()
                            && !frame.return_class
                            && frame.return_instance.is_none()
                            && !frame.return_module;
                        if let Some(owner) = frame.generator_owner.take() {
                            self.finish_generator_resume(owner, value);
                            return Ok(None);
                        }
                        if can_recycle {
                            let discard = frame.discard_result;
                            let simple_no_cells = frame.simple_one_arg_no_cells;
                            if let Some(caller) = self.frames.last_mut() {
                                if !discard {
                                    caller.stack.push(value);
                                }
                                if simple_no_cells {
                                    self.recycle_simple_frame(frame);
                                } else {
                                    self.recycle_frame(frame);
                                }
                                return Ok(None);
                            }
                            if simple_no_cells {
                                self.recycle_simple_frame(frame);
                            } else {
                                self.recycle_frame(frame);
                            }
                            return Ok(Some(value));
                        }
                        let value = if frame.return_class {
                            match self.class_value_from_module(
                                &frame.module,
                                frame.class_bases,
                                frame.class_metaclass,
                                frame.class_keywords,
                            )? {
                                ClassBuildOutcome::Value(value) => value,
                                ClassBuildOutcome::ExceptionHandled => return Ok(None),
                            }
                        } else if let Some(instance) = frame.return_instance {
                            Value::Instance(instance)
                        } else if frame.return_module {
                            Value::Module(frame.module.clone())
                        } else {
                            value
                        };
                        if let Some(caller) = self.frames.last_mut() {
                            if !frame.discard_result {
                                caller.stack.push(value);
                            }
                            return Ok(None);
                        }
                        return Ok(Some(value));
                    }
                    Opcode::ReturnValue => {
                        let simple_fast_return = {
                            if self.frames.len() <= 1 {
                                false
                            } else {
                                let frame = self.frames.last().expect("frame exists");
                                frame.simple_one_arg_no_cells
                                    && !frame.is_module
                                    && !frame.discard_result
                                    && frame.generator_owner.is_none()
                                    && !frame.return_class
                                    && frame.return_instance.is_none()
                                    && !frame.return_module
                                    && frame.module_locals_dict.is_none()
                                    && !frame.expect_none_return
                                    && frame.active_exception.is_none()
                            }
                        };
                        if simple_fast_return {
                            let value = self.pop_value().unwrap_or(Value::None);
                            let frame = self.frames.pop().expect("frame exists");
                            let caller = self.frames.last_mut().expect("caller frame exists");
                            caller.stack.push(value);
                            self.recycle_simple_frame(frame);
                            return Ok(None);
                        }
                        let value = self.pop_value().unwrap_or(Value::None);
                        let mut frame = self.frames.pop().expect("frame exists");
                        if let Some(module_dict) = frame.module_locals_dict.take() {
                            self.sync_module_locals_dict_to_module(&frame.module, &module_dict);
                        }
                        if frame.expect_none_return && value != Value::None {
                            return Err(RuntimeError::new("__init__() should return None"));
                        }
                        let can_recycle = !frame.is_module
                            && frame.generator_owner.is_none()
                            && !frame.return_class
                            && frame.return_instance.is_none()
                            && !frame.return_module;
                        if let Some(owner) = frame.generator_owner.take() {
                            self.finish_generator_resume(owner, value);
                            return Ok(None);
                        }
                        if can_recycle {
                            let discard = frame.discard_result;
                            let simple_no_cells = frame.simple_one_arg_no_cells;
                            if let Some(caller) = self.frames.last_mut() {
                                if !discard {
                                    caller.stack.push(value);
                                }
                                if simple_no_cells {
                                    self.recycle_simple_frame(frame);
                                } else {
                                    self.recycle_frame(frame);
                                }
                                return Ok(None);
                            }
                            if simple_no_cells {
                                self.recycle_simple_frame(frame);
                            } else {
                                self.recycle_frame(frame);
                            }
                            return Ok(Some(value));
                        }
                        let value = if frame.return_class {
                            match self.class_value_from_module(
                                &frame.module,
                                frame.class_bases,
                                frame.class_metaclass,
                                frame.class_keywords,
                            )? {
                                ClassBuildOutcome::Value(value) => value,
                                ClassBuildOutcome::ExceptionHandled => return Ok(None),
                            }
                        } else if let Some(instance) = frame.return_instance {
                            Value::Instance(instance)
                        } else if frame.return_module {
                            Value::Module(frame.module.clone())
                        } else {
                            value
                        };
                        if let Some(caller) = self.frames.last_mut() {
                            if !frame.discard_result {
                                caller.stack.push(value);
                            }
                            return Ok(None);
                        }
                        return Ok(Some(value));
                    }
                }
                Ok(None)
            })();

            match step_result {
                Ok(Some(value)) => return Ok(value),
                Ok(None) => {}
                Err(err) => match self.handle_runtime_error(err) {
                    Ok(()) => {}
                    Err(err) => return Err(err),
                },
            }
            // Keep __del__ suppressed only while an active exception is being processed.
            // Refcount-style cleanup in CPython can happen while ordinary operands are live,
            // and several stdlib paths (tempfile/shutil) rely on that eagerness.
            let safe_for_pending_finalizers = self
                .frames
                .last()
                .map(|frame| frame.active_exception.is_none())
                .unwrap_or(false);
            if safe_for_pending_finalizers
                && (!self.pending_del_instances.is_empty() || !self.weakref_finalizers.is_empty())
            {
                self.run_pending_del_finalizers();
            }
        }
    }

    pub(super) fn raise_exception(&mut self, value: Value) -> Result<(), RuntimeError> {
        self.raise_exception_with_cause(value, None)
    }

    pub(super) fn raise_exception_with_cause(
        &mut self,
        value: Value,
        explicit_cause: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let mut exc = self.normalize_exception_value(value)?;
        if let Value::Exception(exc_data) = &mut exc {
            if let Some(cause_value) = explicit_cause {
                if matches!(cause_value, Value::None) {
                    exc_data.suppress_context = true;
                    exc_data.cause = None;
                } else {
                    let cause = self.normalize_exception_value(cause_value)?;
                    if let Value::Exception(cause_data) = cause {
                        exc_data.cause = Some(Box::new(cause_data));
                        exc_data.suppress_context = true;
                    }
                }
            } else if let Some(current) = self
                .frames
                .last()
                .and_then(|frame| frame.active_exception.clone())
            {
                let context = self.normalize_exception_value(current)?;
                if let Value::Exception(context_data) = context {
                    exc_data.context = Some(Box::new(context_data));
                }
            }
        }

        let mut traceback = Vec::new();
        loop {
            let Some(frame) = self.frames.last_mut() else {
                let message = self.format_traceback(&traceback, &exc);
                return Err(RuntimeError::new(message));
            };

            traceback.push(Self::frame_trace(frame));

            if let Some(block) = frame.blocks.pop() {
                frame.stack.truncate(block.stack_len);
                frame.stack.push(exc.clone());
                frame.ip = block.handler;
                frame.active_exception = Some(exc);
                return Ok(());
            }

            if let Some(boundary) = self.active_generator_resume_boundary {
                if self.frames.len() <= boundary {
                    self.pending_generator_exception = Some(exc);
                    self.generator_resume_outcome =
                        Some(GeneratorResumeOutcome::PropagatedException);
                    return Ok(());
                }
            }

            // Preserve the caller frame for internal Rust-managed calls (e.g.
            // descriptor/builtin helper dispatch). The caller should receive
            // the exception as a regular error value instead of being silently
            // unwound beneath the internal call boundary.
            if let Some(stop_depth) = self.run_stop_depth {
                if self.frames.len() <= stop_depth {
                    let message = self.format_traceback(&traceback, &exc);
                    return Err(RuntimeError::new(message));
                }
            }

            let frame = self.frames.pop().expect("frame exists");
            if let Some(owner) = frame.generator_owner {
                self.generator_states.remove(&owner.id());
                let _ = self.set_generator_running(&owner, false);
                let _ = self.set_generator_started(&owner, true);
                let _ = self.set_generator_closed(&owner, true);
                if self.active_generator_resume == Some(owner.id()) {
                    self.generator_resume_outcome =
                        Some(GeneratorResumeOutcome::PropagatedException);
                }
            }
        }
    }

    pub(super) fn handle_runtime_error(&mut self, err: RuntimeError) -> Result<(), RuntimeError> {
        let exception_type = classify_runtime_error(&err.message);
        let exception = ExceptionObject::new(exception_type.to_string(), Some(err.message.clone()));
        if is_os_error_family(exception_type) {
            if let Some(errno) = extract_os_error_errno(&err.message) {
                exception
                    .attrs
                    .borrow_mut()
                    .insert("errno".to_string(), Value::Int(errno));
            }
            if let Some(strerror) = extract_os_error_strerror(&err.message) {
                exception
                    .attrs
                    .borrow_mut()
                    .insert("strerror".to_string(), Value::Str(strerror));
            }
        }
        if matches!(exception_type, "ImportError" | "ModuleNotFoundError") {
            if let Some(name) = extract_import_error_name(&err.message) {
                exception
                    .attrs
                    .borrow_mut()
                    .insert("name".to_string(), Value::Str(name));
            }
        }
        let exception = Value::Exception(exception);
        self.raise_exception(exception)
    }

    pub(super) fn normalize_exception_value(&self, value: Value) -> Result<Value, RuntimeError> {
        match value {
            Value::Exception(_) => Ok(value),
            Value::ExceptionType(name) => Ok(Value::Exception(ExceptionObject::new(name, None))),
            Value::Class(class) => {
                if self.class_is_exception_class(&class) {
                    let class_name = match &*class.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "Exception".to_string(),
                    };
                    Ok(Value::Exception(ExceptionObject::new(class_name, None)))
                } else {
                    Err(RuntimeError::new("can only raise Exception types"))
                }
            }
            Value::Instance(instance) => {
                let class_name = self
                    .exception_class_name_for_instance(&instance)
                    .ok_or_else(|| RuntimeError::new("can only raise Exception types"))?;
                let message = self.exception_message_for_instance(&instance);
                let exception = ExceptionObject::new(class_name, message);
                if let Object::Instance(instance_data) = &*instance.kind() {
                    if !instance_data.attrs.is_empty() {
                        exception
                            .attrs
                            .borrow_mut()
                            .extend(instance_data.attrs.clone());
                    }
                }
                Ok(Value::Exception(exception))
            }
            _ => Err(RuntimeError::new("can only raise Exception types")),
        }
    }

    pub(super) fn exception_matches(
        &self,
        exception: &Value,
        handler_type: &Value,
    ) -> Result<bool, RuntimeError> {
        let exception_name = match exception {
            Value::Exception(exc) => exc.name.as_str(),
            _ => return Err(RuntimeError::new("expected exception instance")),
        };

        let handler_name = match handler_type {
            Value::Tuple(obj) => {
                let Object::Tuple(items) = &*obj.kind() else {
                    return Err(RuntimeError::new("except expects exception type"));
                };
                for item in items {
                    if self.exception_matches(exception, item)? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            Value::List(obj) => {
                let Object::List(items) = &*obj.kind() else {
                    return Err(RuntimeError::new("except expects exception type"));
                };
                for item in items {
                    if self.exception_matches(exception, item)? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            Value::ExceptionType(name) => name.as_str(),
            Value::Exception(exc) => exc.name.as_str(),
            Value::Class(class) => {
                if !self.class_is_exception_class(class) {
                    return Err(RuntimeError::new("except expects exception type"));
                }
                let class_kind = class.kind();
                let Object::Class(class_data) = &*class_kind else {
                    return Err(RuntimeError::new("except expects exception type"));
                };
                return Ok(self.exception_inherits(exception_name, &class_data.name));
            }
            _ => return Err(RuntimeError::new("except expects exception type")),
        };

        Ok(self.exception_inherits(exception_name, handler_name))
    }

    pub(super) fn exception_split_for_star(
        &self,
        exception: &Value,
        handler_type: &Value,
    ) -> Result<(Option<ExceptionObject>, Option<ExceptionObject>), RuntimeError> {
        let Value::Exception(exception_obj) = exception else {
            return Err(RuntimeError::new("expected exception instance"));
        };
        self.exception_split_for_star_object(exception_obj, handler_type)
    }

    pub(super) fn exception_split_for_star_object(
        &self,
        exception: &ExceptionObject,
        handler_type: &Value,
    ) -> Result<(Option<ExceptionObject>, Option<ExceptionObject>), RuntimeError> {
        if self.exception_inherits(&exception.name, "BaseExceptionGroup") {
            let mut matched_members = Vec::new();
            let mut remaining_members = Vec::new();
            for member in &exception.exceptions {
                let (matched, remaining) =
                    self.exception_split_for_star_object(member, handler_type)?;
                if let Some(matched) = matched {
                    matched_members.push(matched);
                }
                if let Some(remaining) = remaining {
                    remaining_members.push(remaining);
                }
            }
            let matched_group = if matched_members.is_empty() {
                None
            } else {
                Some(self.clone_exception_group_with_members(exception, matched_members))
            };
            let remaining_group = if remaining_members.is_empty() {
                None
            } else {
                Some(self.clone_exception_group_with_members(exception, remaining_members))
            };
            return Ok((matched_group, remaining_group));
        }

        let matches = self.exception_matches(&Value::Exception(exception.clone()), handler_type)?;
        if matches {
            Ok((Some(exception.clone()), None))
        } else {
            Ok((None, Some(exception.clone())))
        }
    }

    pub(super) fn clone_exception_group_with_members(
        &self,
        template: &ExceptionObject,
        members: Vec<ExceptionObject>,
    ) -> ExceptionObject {
        let mut clone =
            ExceptionObject::with_members(template.name.clone(), template.message.clone(), members);
        clone.notes = template.notes.clone();
        clone.cause = template.cause.clone();
        clone.context = template.context.clone();
        clone.suppress_context = template.suppress_context;
        clone.attrs = template.attrs.clone();
        clone
    }

    pub(super) fn exception_inherits(&self, exception_name: &str, handler_name: &str) -> bool {
        if exception_name == handler_name {
            return true;
        }
        let mut seen = HashSet::new();
        let mut current = self.exception_parent_name(exception_name);
        while let Some(name) = current {
            if !seen.insert(name.clone()) {
                break;
            }
            if name == handler_name {
                return true;
            }
            current = self.exception_parent_name(&name);
        }
        false
    }

    pub(super) fn exception_parent_name(&self, name: &str) -> Option<String> {
        if let Some(parent) = self.exception_parents.get(name) {
            return Some(parent.clone());
        }
        builtin_exception_parent(name).map(ToOwned::to_owned)
    }

    pub(super) fn class_is_exception_class(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => {
                    self.exception_inherits(&class_data.name, "BaseException")
                }
                _ => false,
            })
    }

    pub(super) fn exception_class_name_for_instance(&self, instance: &ObjRef) -> Option<String> {
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return None,
        };
        if !self.class_is_exception_class(&class) {
            return None;
        }
        match &*class.kind() {
            Object::Class(class_data) => Some(class_data.name.clone()),
            _ => None,
        }
    }

    pub(super) fn record_exception_parent_for_class(&mut self, class: &ObjRef) {
        let (class_name, bases) = match &*class.kind() {
            Object::Class(class_data) => (class_data.name.clone(), class_data.bases.clone()),
            _ => return,
        };
        if class_name == "BaseException" {
            return;
        }

        for base in bases {
            let (base_name, is_exception) = match &*base.kind() {
                Object::Class(base_data) => {
                    (base_data.name.clone(), self.class_is_exception_class(&base))
                }
                _ => continue,
            };
            if is_exception {
                self.exception_parents.insert(class_name.clone(), base_name);
                return;
            }
        }
    }

    pub(super) fn exception_message_for_instance(&self, instance: &ObjRef) -> Option<String> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        let Some(args_value) = instance_data.attrs.get("args") else {
            return None;
        };
        let Value::Tuple(args_obj) = args_value else {
            return None;
        };
        let Object::Tuple(args) = &*args_obj.kind() else {
            return None;
        };
        if args.len() == 1 {
            return Some(format_value(&args[0]));
        }
        None
    }

    pub(super) fn instantiate_exception_type(
        &self,
        name: &str,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let _ = kwargs;

        if self.exception_inherits(name, "BaseExceptionGroup") {
            let message = args.first().map(format_value);
            let members = if let Some(value) = args.get(1) {
                self.exception_members_from_value(value)?
            } else {
                Vec::new()
            };
            return Ok(Value::Exception(ExceptionObject::with_members(
                name.to_string(),
                message,
                members,
            )));
        }

        let message = exception_message_from_call_args(args);
        Ok(Value::Exception(ExceptionObject::new(
            name.to_string(),
            message,
        )))
    }

    pub(super) fn exception_members_from_value(
        &self,
        value: &Value,
    ) -> Result<Vec<ExceptionObject>, RuntimeError> {
        let entries = match value {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "ExceptionGroup members must be a sequence",
                    ));
                }
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "ExceptionGroup members must be a sequence",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::new(
                    "ExceptionGroup members must be a sequence",
                ));
            }
        };
        let mut members = Vec::with_capacity(entries.len());
        for entry in entries {
            let normalized = self.normalize_exception_value(entry)?;
            let Value::Exception(exception) = normalized else {
                return Err(RuntimeError::new(
                    "ExceptionGroup members must be exceptions",
                ));
            };
            members.push(exception);
        }
        Ok(members)
    }

    pub(super) fn frame_trace(frame: &Frame) -> TraceFrame {
        let location = frame.code.locations.get(frame.last_ip);
        let line = location.map(|loc| loc.line).unwrap_or(0);
        let column = location.map(|loc| loc.column).unwrap_or(0);
        TraceFrame {
            filename: frame.code.filename.clone(),
            line,
            column,
            name: frame.code.name.clone(),
        }
    }

    pub(super) fn format_traceback(&self, frames: &[TraceFrame], exc: &Value) -> String {
        let mut output = String::from("Traceback (most recent call last):\n");
        for frame in frames.iter().rev() {
            output.push_str(&format!(
                "  File \"{}\", line {}, column {}, in {}\n",
                frame.filename, frame.line, frame.column, frame.name
            ));
        }
        match exc {
            Value::Exception(exception) => output.push_str(&self.format_exception_object(exception)),
            _ => output.push_str(&format_value(exc)),
        }
        if let Value::Exception(exception) = exc {
            if let Some(cause) = &exception.cause {
                output.push_str(
                    "\nThe above exception was the direct cause of the following exception:\n",
                );
                output.push_str(&self.format_exception_object(cause));
            } else if !exception.suppress_context {
                if let Some(context) = &exception.context {
                    output.push_str(
                        "\nDuring handling of the above exception, another exception occurred:\n",
                    );
                    output.push_str(&self.format_exception_object(context));
                }
            }
        }
        output
    }

    pub(super) fn format_exception_object(&self, exception: &ExceptionObject) -> String {
        match &exception.message {
            Some(message) if !message.is_empty() => format!("{}: {}", exception.name, message),
            _ => exception.name.clone(),
        }
    }

    pub(super) fn class_value_from_module(
        &mut self,
        module: &ObjRef,
        mut bases: Vec<ObjRef>,
        metaclass: Option<Value>,
        class_keywords: HashMap<String, Value>,
    ) -> Result<ClassBuildOutcome, RuntimeError> {
        if bases.is_empty() {
            if let Some(Value::Class(object_class)) = self.builtins.get("object") {
                bases.push(object_class.clone());
            }
        }
        let (name, attrs) = match &*module.kind() {
            Object::Module(module_data) => (module_data.name.clone(), module_data.globals.clone()),
            _ => ("<class>".to_string(), HashMap::new()),
        };

        let resolved_metaclass = self.resolve_class_metaclass(&bases, metaclass.as_ref())?;
        if let Some(meta) = metaclass {
            if matches!(
                meta,
                Value::Builtin(BuiltinFunction::Type) | Value::Class(_)
            ) {
                let class_value =
                    self.build_default_class_value(name, attrs, bases, resolved_metaclass);
                if let Value::Class(class_ref) = &class_value {
                    if self.call_init_subclass_hook(class_ref, &class_keywords)? {
                        return Ok(ClassBuildOutcome::ExceptionHandled);
                    }
                }
                return Ok(ClassBuildOutcome::Value(class_value));
            }
            if self.frames.is_empty() {
                return Err(RuntimeError::new("metaclass call requires active frame"));
            }
            let namespace = self.heap.alloc_dict(
                attrs
                    .iter()
                    .map(|(key, value)| (Value::Str(key.clone()), value.clone()))
                    .collect::<Vec<_>>(),
            );
            let bases_tuple = self
                .heap
                .alloc_tuple(bases.iter().cloned().map(Value::Class).collect::<Vec<_>>());
            return match self.call_internal(
                meta,
                vec![Value::Str(name), bases_tuple, namespace],
                class_keywords,
            )? {
                InternalCallOutcome::Value(value) => {
                    if let Value::Class(class) = &value {
                        if let Some(meta_class) = resolved_metaclass {
                            if let Object::Class(class_data) = &mut *class.kind_mut() {
                                class_data.metaclass = Some(meta_class);
                            }
                        }
                        self.record_exception_parent_for_class(class);
                        Ok(ClassBuildOutcome::Value(value))
                    } else {
                        Err(RuntimeError::new("metaclass must return a class object"))
                    }
                }
                InternalCallOutcome::CallerExceptionHandled => {
                    Ok(ClassBuildOutcome::ExceptionHandled)
                }
            };
        }

        let class_value = self.build_default_class_value(name, attrs, bases, resolved_metaclass);
        if let Value::Class(class_ref) = &class_value {
            if self.call_init_subclass_hook(class_ref, &class_keywords)? {
                return Ok(ClassBuildOutcome::ExceptionHandled);
            }
        }
        Ok(ClassBuildOutcome::Value(class_value))
    }

    pub(super) fn call_init_subclass_hook(
        &mut self,
        class: &ObjRef,
        class_keywords: &HashMap<String, Value>,
    ) -> Result<bool, RuntimeError> {
        let mro = self.class_mro_entries(class);
        let init_subclass = mro
            .into_iter()
            .skip(1)
            .find_map(|candidate| class_attr_lookup_direct(&candidate, "__init_subclass__"));
        let Some(init_subclass) = init_subclass else {
            return Ok(false);
        };
        match self.call_internal(
            init_subclass,
            vec![Value::Class(class.clone())],
            class_keywords.clone(),
        )? {
            InternalCallOutcome::Value(_) => Ok(false),
            InternalCallOutcome::CallerExceptionHandled => Ok(true),
        }
    }

    pub(super) fn resolve_class_metaclass(
        &self,
        bases: &[ObjRef],
        explicit_metaclass: Option<&Value>,
    ) -> Result<Option<ObjRef>, RuntimeError> {
        let mut winner = match explicit_metaclass {
            Some(Value::Class(class)) => Some(class.clone()),
            _ => None,
        };

        for base in bases {
            let base_meta = match &*base.kind() {
                Object::Class(class_data) => class_data.metaclass.clone(),
                _ => None,
            };
            winner = self.merge_metaclass_candidates(winner, base_meta)?;
        }

        Ok(winner)
    }

    pub(super) fn merge_metaclass_candidates(
        &self,
        left: Option<ObjRef>,
        right: Option<ObjRef>,
    ) -> Result<Option<ObjRef>, RuntimeError> {
        match (left, right) {
            (None, None) => Ok(None),
            (Some(meta), None) | (None, Some(meta)) => Ok(Some(meta)),
            (Some(left_meta), Some(right_meta)) => {
                if left_meta.id() == right_meta.id() {
                    return Ok(Some(left_meta));
                }
                if self.class_is_subclass_of(&left_meta, &right_meta) {
                    return Ok(Some(left_meta));
                }
                if self.class_is_subclass_of(&right_meta, &left_meta) {
                    return Ok(Some(right_meta));
                }
                Err(RuntimeError::new(
                    "metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases",
                ))
            }
        }
    }

    pub(super) fn class_is_subclass_of(&self, class: &ObjRef, target: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| entry.id() == target.id())
    }

    pub(super) fn build_default_class_value(
        &mut self,
        name: String,
        attrs: HashMap<String, Value>,
        bases: Vec<ObjRef>,
        metaclass: Option<ObjRef>,
    ) -> Value {
        let module_name = self
            .frames
            .last()
            .and_then(|frame| match &*frame.function_globals.kind() {
                Object::Module(module_data) => Some(module_data.name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "__main__".to_string());
        let class = ClassObject::new(name, bases.clone());
        let class_value = self.heap.alloc_class(class);
        if let Value::Class(class_ref) = &class_value {
            if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                class_data.attrs.extend(attrs);
                class_data.metaclass = metaclass;
                if matches!(
                    class_data.name.as_str(),
                    "Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" | "ReprEnum"
                ) {
                    class_data
                        .attrs
                        .entry("_use_args_".to_string())
                        .or_insert(Value::Bool(false));
                    let member_type = match class_data.name.as_str() {
                        "IntEnum" | "IntFlag" => Value::Builtin(BuiltinFunction::Int),
                        "StrEnum" => Value::Builtin(BuiltinFunction::Str),
                        _ => self.builtins.get("object").cloned().unwrap_or(Value::None),
                    };
                    class_data
                        .attrs
                        .entry("_member_type_".to_string())
                        .or_insert(member_type);
                    class_data
                        .attrs
                        .entry("_value_repr_".to_string())
                        .or_insert(Value::Builtin(BuiltinFunction::Repr));
                }
                if let Some(slots_value) = class_data.attrs.get("__slots__").cloned() {
                    if let Some(slot_names) = slot_names_from_value(Some(slots_value.clone())) {
                        class_data.slots = Some(slot_names);
                        // Preserve the declared __slots__ object shape (str/list/tuple/etc.)
                        // while retaining normalized slot names in ClassObject::slots.
                        class_data.attrs.insert("__slots__".to_string(), slots_value);
                    }
                }
                class_data
                    .attrs
                    .insert("__name__".to_string(), Value::Str(class_data.name.clone()));
                let qualname = class_data
                    .attrs
                    .get("__qualname__")
                    .and_then(|value| match value {
                        Value::Str(name) => Some(name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| class_data.name.clone());
                class_data
                    .attrs
                    .insert("__qualname__".to_string(), Value::Str(qualname));
                class_data
                    .attrs
                    .entry("__module__".to_string())
                    .or_insert(Value::Str(module_name));
                class_data
                    .attrs
                    .entry("__pyrs_user_class__".to_string())
                    .or_insert(Value::Bool(true));
                class_data
                    .attrs
                    .insert("__flags__".to_string(), Value::Int(PY_TPFLAGS_HEAPTYPE));
                class_data.attrs.insert(
                    "__bases__".to_string(),
                    self.heap.alloc_tuple(
                        class_data
                            .bases
                            .iter()
                            .cloned()
                            .map(Value::Class)
                            .collect::<Vec<_>>(),
                    ),
                );
            }
            self.attach_owner_class_to_attrs(class_ref);
            if let Ok(mro) = self.build_class_mro(class_ref, &bases) {
                if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                    class_data.mro = mro.clone();
                    let mro_values = mro.into_iter().map(Value::Class).collect::<Vec<_>>();
                    class_data
                        .attrs
                        .insert("__mro__".to_string(), self.heap.alloc_tuple(mro_values));
                }
            }
            self.record_exception_parent_for_class(class_ref);
        }
        class_value
    }

    pub(super) fn attach_owner_class_to_attrs(&mut self, class_ref: &ObjRef) {
        let Object::Class(class_data) = &mut *class_ref.kind_mut() else {
            return;
        };
        for value in class_data.attrs.values() {
            self.attach_owner_class_to_value(value, class_ref);
        }
    }

    pub(super) fn attach_owner_class_to_value(&mut self, value: &Value, owner: &ObjRef) {
        match value {
            Value::Function(func) => self.set_function_owner_class(func, owner),
            Value::Module(module) => {
                let Object::Module(module_data) = &*module.kind() else {
                    return;
                };
                if module_data.name == "__classmethod__" || module_data.name == "__staticmethod__" {
                    if let Some(Value::Function(func)) = module_data.globals.get("__func__") {
                        self.set_function_owner_class(func, owner);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn set_function_owner_class(&mut self, func: &ObjRef, owner: &ObjRef) {
        if let Object::Function(func_data) = &mut *func.kind_mut() {
            func_data.owner_class = Some(owner.clone());
        }
    }

    pub(super) fn class_mro_entries(&self, class: &ObjRef) -> Vec<ObjRef> {
        match &*class.kind() {
            Object::Class(class_data) if !class_data.mro.is_empty() => class_data.mro.clone(),
            Object::Class(_) => vec![class.clone()],
            _ => Vec::new(),
        }
    }

    pub(super) fn build_class_mro(
        &self,
        class: &ObjRef,
        bases: &[ObjRef],
    ) -> Result<Vec<ObjRef>, RuntimeError> {
        if bases.is_empty() {
            return Ok(vec![class.clone()]);
        }

        let mut seqs: Vec<Vec<ObjRef>> = Vec::new();
        for base in bases {
            seqs.push(self.class_mro_entries(base));
        }
        seqs.push(bases.to_vec());

        let mut merged = Vec::new();
        loop {
            seqs.retain(|seq| !seq.is_empty());
            if seqs.is_empty() {
                break;
            }

            let mut candidate = None;
            for seq in &seqs {
                let head = seq[0].clone();
                let in_tail = seqs
                    .iter()
                    .any(|other| other.iter().skip(1).any(|entry| entry.id() == head.id()));
                if !in_tail {
                    candidate = Some(head);
                    break;
                }
            }

            let Some(head) = candidate else {
                return Err(RuntimeError::new(
                    "cannot create a consistent method resolution order (MRO)",
                ));
            };
            merged.push(head.clone());
            for seq in &mut seqs {
                if !seq.is_empty() && seq[0].id() == head.id() {
                    seq.remove(0);
                }
            }
        }

        let mut out = vec![class.clone()];
        out.extend(merged);
        Ok(out)
    }

    pub(super) fn pop_value(&mut self) -> Result<Value, RuntimeError> {
        let frame = self.frames.last_mut().expect("frame exists");
        if let Some(value) = frame.stack.pop() {
            return Ok(value);
        }
        let ip = frame.ip.saturating_sub(1);
        let opcode_name = frame
            .code
            .instructions
            .get(ip)
            .map(|instr| format!("{:?}", instr.opcode))
            .unwrap_or_else(|| "<unknown>".to_string());
        Err(RuntimeError::new(format!(
            "stack underflow (pop_value) in frame '{}' at ip {} opcode {}",
            frame.code.name, ip, opcode_name
        )))
    }

    pub(super) fn push_value(&mut self, value: Value) {
        let frame = self.frames.last_mut().expect("frame exists");
        frame.stack.push(value);
    }

    pub(super) fn push_value_to_caller_frame(
        &mut self,
        caller_idx: usize,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if let Some(frame) = self.frames.get_mut(caller_idx) {
            frame.stack.push(value);
            return Ok(());
        }
        if let Some(frame) = self.frames.last_mut() {
            frame.stack.push(value);
            return Ok(());
        }
        Err(RuntimeError::new("caller frame missing"))
    }

    pub(super) fn get_cell(&self, idx: usize) -> Result<ObjRef, RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        frame
            .cells
            .get(idx)
            .cloned()
            .ok_or_else(|| RuntimeError::new("cell index out of range"))
    }

    pub(super) fn load_deref(&self, idx: usize) -> Result<Value, RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        let cell = frame
            .cells
            .get(idx)
            .ok_or_else(|| RuntimeError::new("cell index out of range"))?;
        match &*cell.kind() {
            Object::Cell(cell_data) => cell_data.value.clone().ok_or_else(|| {
                let name = deref_name(&frame.code, idx).unwrap_or("<cell>");
                RuntimeError::new(format!(
                    "free variable '{}' referenced before assignment",
                    name
                ))
            }),
            _ => Err(RuntimeError::new("invalid cell object")),
        }
    }

    pub(super) fn store_deref(&mut self, idx: usize, value: Value) -> Result<(), RuntimeError> {
        let frame = self.frames.last_mut().expect("frame exists");
        let cell = frame
            .cells
            .get(idx)
            .cloned()
            .ok_or_else(|| RuntimeError::new("cell index out of range"))?;
        match &mut *cell.kind_mut() {
            Object::Cell(cell_data) => {
                cell_data.value = Some(value);
                Ok(())
            }
            _ => Err(RuntimeError::new("invalid cell object")),
        }
    }

    pub(super) fn load_fast_local(&mut self, idx: usize) -> Result<Value, RuntimeError> {
        let cached = {
            let frame = self.frames.last().expect("frame exists");
            if idx < frame.fast_locals.len() {
                match &frame.fast_locals[idx] {
                    Some(value) => Some(value.clone()),
                    None => None,
                }
            } else {
                None
            }
        };
        if let Some(value) = cached {
            return Ok(value);
        }

        let (name, value) = {
            let frame = self.frames.last().expect("frame exists");
            let name = frame
                .code
                .names
                .get(idx)
                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                .clone();
            let value = frame.locals.get(&name).cloned();
            (name, value)
        };

        let value = value.ok_or_else(|| RuntimeError::new(format!("local '{name}' not set")))?;
        if let Some(frame) = self.frames.last_mut() {
            if let Some(slot) = frame.fast_locals.get_mut(idx) {
                Self::write_fast_local_slot(slot, value.clone());
            }
        }
        Ok(value)
    }

    pub(super) fn store_fast_local(&mut self, idx: usize, value: Value) -> Result<(), RuntimeError> {
        let name = {
            let frame = self.frames.last().expect("frame exists");
            frame
                .code
                .names
                .get(idx)
                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                .clone()
        };
        let frame = self.frames.last_mut().expect("frame exists");
        if let Some(slot) = frame.fast_locals.get_mut(idx) {
            Self::write_fast_local_slot(slot, value.clone());
        } else {
            return Err(RuntimeError::new("name index out of range"));
        }
        if let Some(existing) = frame.locals.get_mut(&name) {
            *existing = value;
        }
        Ok(())
    }

    pub(super) fn take_fast_local(&mut self, idx: usize) -> Result<Value, RuntimeError> {
        let name = {
            let frame = self.frames.last().expect("frame exists");
            frame
                .code
                .names
                .get(idx)
                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                .clone()
        };
        let frame = self.frames.last_mut().expect("frame exists");
        let value = if let Some(slot) = frame.fast_locals.get_mut(idx) {
            slot.take()
        } else {
            None
        }
        .or_else(|| frame.locals.remove(&name));
        frame.locals.remove(&name);
        value.ok_or_else(|| RuntimeError::new(format!("local '{name}' not set")))
    }

    pub(super) fn ensure_frame_module_locals_dict(&mut self, frame_index: usize) -> ObjRef {
        if let Some(existing) = self.frames[frame_index].module_locals_dict.clone() {
            return existing;
        }
        let module = self.frames[frame_index].module.clone();
        let entries = match &*module.kind() {
            Object::Module(module_data) => module_data
                .globals
                .iter()
                .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let dict = match self.heap.alloc_dict(entries) {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };
        self.frames[frame_index].module_locals_dict = Some(dict.clone());
        dict
    }

    pub(super) fn sync_module_locals_dict_to_module(&mut self, module: &ObjRef, dict: &ObjRef) {
        let mut map = HashMap::new();
        if let Object::Dict(entries) = &*dict.kind() {
            for (key, value) in entries {
                if let Value::Str(name) = key {
                    map.insert(name.clone(), value.clone());
                }
            }
        }
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data.globals = map;
        }
    }

    pub(super) fn module_namespace_lookup(&self, frame: &Frame, name: &str) -> Option<Value> {
        if let Some(dict) = &frame.module_locals_dict {
            return dict_get_value(dict, &Value::Str(name.to_string()));
        }
        if let Object::Module(module_data) = &*frame.module.kind() {
            return module_data.globals.get(name).cloned();
        }
        None
    }

    pub(super) fn frame_local_value(frame: &Frame, name: &str) -> Option<Value> {
        if let Some(idx) = frame.code.name_to_index.get(name).copied() {
            if idx < frame.fast_locals.len() {
                if let Some(value) = &frame.fast_locals[idx] {
                    return Some(value.clone());
                }
            }
        }
        if let Some(value) = frame.locals.get(name) {
            return Some(value.clone());
        }
        None
    }

    pub(super) fn lookup_name(&self, name: &str) -> Result<Value, RuntimeError> {
        if let Some(frame) = self.frames.last() {
            if let Some(value) = Self::frame_local_value(frame, name) {
                return Ok(value.clone());
            }
            if let Some(fallback) = &frame.locals_fallback {
                if let Some(value) = fallback.get(name) {
                    return Ok(value.clone());
                }
            }
            if let Some(value) = self.module_namespace_lookup(frame, name) {
                return Ok(value);
            }
            if let Some(fallback) = &frame.globals_fallback {
                if let Object::Module(module_data) = &*fallback.kind() {
                    if let Some(value) = module_data.globals.get(name) {
                        return Ok(value.clone());
                    }
                }
            }
        }
        self.builtins
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("name '{name}' is not defined")))
    }

    pub(super) fn store_name(&mut self, name: String, value: Value) {
        let mut module_write: Option<(ObjRef, String, Value)> = None;
        if let Some(frame) = self.frames.last_mut() {
            if frame.is_module {
                if let Some(dict) = frame.module_locals_dict.clone() {
                    dict_set_value(&dict, Value::Str(name.clone()), value.clone());
                }
                module_write = Some((frame.module.clone(), name, value));
            } else {
                if let Some(slot_idx) = frame.code.name_to_index.get(&name).copied() {
                    if let Some(slot) = frame.fast_locals.get_mut(slot_idx) {
                        Self::write_fast_local_slot(slot, value.clone());
                    }
                }
                if let Some(existing) = frame.locals.get_mut(&name) {
                    *existing = value;
                } else {
                    // Keep fast locals authoritative; only retain truly dynamic names here.
                    if !frame.code.name_to_index.contains_key(&name) {
                        frame.locals.insert(name, value);
                    }
                }
            }
        }
        if let Some((module, name, value)) = module_write {
            self.upsert_module_global(&module, &name, value);
        }
    }

    #[inline]
    fn upsert_module_global(&mut self, module: &ObjRef, name: &str, value: Value) {
        let mut version = None;
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            if let Some(existing) = module_data.globals.get_mut(name) {
                *existing = value;
            } else {
                module_data.globals.insert(name.to_string(), value);
            }
            module_data.touch_globals_version();
            version = Some(module_data.globals_version);
        }
        if let Some(version) = version {
            self.propagate_module_globals_version(module.id(), version);
        }
    }

    #[inline]
    fn current_site_index(&self) -> usize {
        let frame = self.frames.last().expect("frame exists");
        frame.last_ip
    }

    #[inline]
    fn mark_quickened_site(&mut self, site_index: usize, kind: QuickenedSiteKind) {
        if let Some(frame) = self.frames.last_mut() {
            if let Some(slot) = frame.quickened_sites.get_mut(site_index) {
                *slot = kind;
            }
        }
    }

    #[inline]
    fn clear_quickened_site(&mut self, site_index: usize) {
        if let Some(frame) = self.frames.last_mut() {
            if let Some(slot) = frame.quickened_sites.get_mut(site_index) {
                *slot = QuickenedSiteKind::None;
            }
        }
    }

    #[inline]
    fn is_quickened_site(&self, site_index: usize, kind: QuickenedSiteKind) -> bool {
        self.frames
            .last()
            .and_then(|frame| frame.quickened_sites.get(site_index))
            .copied()
            .map(|stored| stored == kind)
            .unwrap_or(false)
    }

    #[inline]
    fn resolve_load_global_value(
        &self,
        name_idx: usize,
    ) -> Result<(Value, bool, u64, u64), RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        let name = frame
            .code
            .names
            .get(name_idx)
            .ok_or_else(|| RuntimeError::new("name index out of range"))?;
        let value = if let Object::Module(module_data) = &*frame.function_globals.kind() {
            module_data.globals.get(name).cloned()
        } else {
            None
        };
        let value = value.or_else(|| {
            if let Some(fallback) = &frame.locals_fallback {
                if let Some(value) = fallback.get(name) {
                    return Some(value.clone());
                }
            }
            if let Some(fallback) = &frame.globals_fallback {
                if let Object::Module(module_data) = &*fallback.kind() {
                    return module_data.globals.get(name).cloned();
                }
            }
            None
        });
        let value = value
            .or_else(|| self.builtins.get(name).cloned())
            .ok_or_else(|| RuntimeError::new(format!("name '{name}' is not defined")))?;
        let cacheable = frame.locals_fallback.is_none()
            && frame.globals_fallback.is_none()
            && frame.function_globals_version != 0;
        Ok((
            value,
            cacheable,
            frame.function_globals.id(),
            frame.function_globals_version,
        ))
    }

    #[inline]
    fn next_jump_if_false_target(&self) -> Option<usize> {
        let frame = self.frames.last()?;
        let next = frame.code.instructions.get(frame.ip)?;
        if next.opcode == Opcode::JumpIfFalse {
            return next.arg.map(|arg| arg as usize);
        }
        None
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    fn fused_global_fast_sub_call_one_arg_pattern(&self) -> Option<(usize, usize)> {
        let frame = self.frames.last()?;
        let load_fast = frame.code.instructions.get(frame.ip)?;
        let binary_sub_const = frame.code.instructions.get(frame.ip + 1)?;
        let call = frame.code.instructions.get(frame.ip + 2)?;
        if load_fast.opcode != Opcode::LoadFast {
            return None;
        }
        if binary_sub_const.opcode != Opcode::BinarySubConst {
            return None;
        }
        let call_is_one_arg = call.opcode == Opcode::CallFunction1
            || (call.opcode == Opcode::CallFunction && call.arg == Some(1));
        if !call_is_one_arg {
            return None;
        }
        Some((load_fast.arg? as usize, binary_sub_const.arg? as usize))
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    fn fused_direct_one_arg_no_cells_metadata(
        &self,
        value: &Value,
    ) -> Option<(Rc<CodeObject>, ObjRef, Option<ObjRef>)> {
        let func = match value {
            Value::Function(func) => func,
            _ => return None,
        };
        let func_kind = func.kind();
        let func_data = match &*func_kind {
            Object::Function(data) => data,
            _ => return None,
        };
        let code = &func_data.code;
        if func_data.plain_positional_call_arity != Some(1) {
            return None;
        }
        if code.plain_positional_arg0_cell.is_some()
            || !code.cellvars.is_empty()
            || !func_data.closure.is_empty()
            || code.is_generator
            || code.is_comprehension
        {
            return None;
        }
        Some((
            func_data.code.clone(),
            func_data.module.clone(),
            func_data.owner_class.clone(),
        ))
    }

    #[inline]
    #[cfg(not(debug_assertions))]
    fn fused_fast_local_sub_const_arg(
        &mut self,
        local_idx: usize,
        const_idx: usize,
    ) -> Result<Value, RuntimeError> {
        #[inline]
        fn small_int_like(value: &Value) -> Option<i64> {
            match value {
                Value::Int(integer) => Some(*integer),
                Value::Bool(flag) => Some(if *flag { 1 } else { 0 }),
                _ => None,
            }
        }

        {
            let frame = self.frames.last().expect("frame exists");
            if const_idx >= frame.code.constants.len() {
                return Err(RuntimeError::new("constant index out of range"));
            }
            if let Some(left_ref) = frame.fast_locals.get(local_idx).and_then(Option::as_ref) {
                if let (Some(left_int), Some(right_int)) = (
                    small_int_like(left_ref),
                    small_int_like(&frame.code.constants[const_idx]),
                ) {
                    return match left_int.checked_sub(right_int) {
                        Some(diff) => Ok(Value::Int(diff)),
                        None => sub_values(Value::Int(left_int), Value::Int(right_int), &self.heap),
                    };
                }
            }
        }

        let left_fast = {
            let frame = self.frames.last().expect("frame exists");
            if local_idx < frame.fast_locals.len() {
                frame.fast_locals[local_idx].clone()
            } else {
                None
            }
        };
        let left = if let Some(value) = left_fast {
            value
        } else {
            self.load_fast_local(local_idx)?
        };
        let right = {
            let frame = self.frames.last().expect("frame exists");
            if const_idx >= frame.code.constants.len() {
                return Err(RuntimeError::new("constant index out of range"));
            }
            frame.code.constants[const_idx].clone()
        };
        match right {
            Value::Int(right) => match left {
                Value::Int(left_int) => match left_int.checked_sub(right) {
                    Some(diff) => Ok(Value::Int(diff)),
                    None => sub_values(Value::Int(left_int), Value::Int(right), &self.heap),
                },
                other => sub_values(other, Value::Int(right), &self.heap),
            },
            Value::Bool(flag) => {
                let right = if flag { 1 } else { 0 };
                match left {
                    Value::Int(left_int) => match left_int.checked_sub(right) {
                        Some(diff) => Ok(Value::Int(diff)),
                        None => sub_values(Value::Int(left_int), Value::Int(right), &self.heap),
                    },
                    other => sub_values(other, Value::Int(right), &self.heap),
                }
            }
            right => sub_values(left, right, &self.heap),
        }
    }

    #[inline]
    #[cfg(not(debug_assertions))]
    fn fused_fast_local_sub_small_int_arg(
        &mut self,
        local_idx: usize,
        right_int: i64,
    ) -> Result<Value, RuntimeError> {
        #[inline]
        fn small_int_like(value: &Value) -> Option<i64> {
            match value {
                Value::Int(integer) => Some(*integer),
                Value::Bool(flag) => Some(if *flag { 1 } else { 0 }),
                _ => None,
            }
        }

        if let Some(left_int) = self
            .frames
            .last()
            .and_then(|frame| frame.fast_locals.get(local_idx))
            .and_then(Option::as_ref)
            .and_then(small_int_like)
        {
            return match left_int.checked_sub(right_int) {
                Some(diff) => Ok(Value::Int(diff)),
                None => sub_values(Value::Int(left_int), Value::Int(right_int), &self.heap),
            };
        }

        let left = {
            let frame = self.frames.last().expect("frame exists");
            if local_idx < frame.fast_locals.len() {
                frame.fast_locals[local_idx].clone()
            } else {
                None
            }
        };
        let left = if let Some(value) = left {
            value
        } else {
            self.load_fast_local(local_idx)?
        };
        sub_values(left, Value::Int(right_int), &self.heap)
    }

    fn dispatch_call_no_kwargs(&mut self, func: Value, args: Vec<Value>) -> Result<(), RuntimeError> {
        match func {
            Value::Function(func) => {
                self.push_function_call_from_obj(&func, args, HashMap::new())?;
            }
            Value::BoundMethod(method) => {
                let method_data = match &*method.kind() {
                    Object::BoundMethod(data) => data.clone(),
                    _ => return Err(RuntimeError::new("attempted to call non-function")),
                };
                match &*method_data.function.kind() {
                    Object::Function(_) => {
                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                        bound_args.push(self.receiver_value(&method_data.receiver)?);
                        bound_args.extend(args);
                        self.push_function_call_from_obj(
                            &method_data.function,
                            bound_args,
                            HashMap::new(),
                        )?;
                    }
                    Object::NativeMethod(native) => {
                        let caller_depth = self.frames.len();
                        let caller_idx = caller_depth.saturating_sub(1);
                        let caller_ip = self
                            .frames
                            .get(caller_idx)
                            .map(|frame| frame.ip)
                            .unwrap_or(0);
                        let call_result = self.call_native_method(
                            native.kind,
                            method_data.receiver.clone(),
                            args,
                            HashMap::new(),
                        );
                        self.finalize_native_opcode_call(caller_depth, caller_ip, call_result)?;
                    }
                    _ => return Err(RuntimeError::new("attempted to call non-function")),
                }
            }
            Value::Class(class) => {
                match self.call_internal(Value::Class(class), args, HashMap::new())? {
                    InternalCallOutcome::Value(value) => self.push_value(value),
                    InternalCallOutcome::CallerExceptionHandled => {}
                }
            }
            Value::Builtin(builtin) => {
                let caller_depth = self.frames.len();
                let caller_idx = caller_depth.saturating_sub(1);
                let caller_ip = self
                    .frames
                    .get(caller_idx)
                    .map(|frame| frame.ip)
                    .unwrap_or(0);
                let call_result = self.call_builtin(builtin, args, HashMap::new());
                self.finalize_builtin_opcode_call(caller_depth, caller_ip, call_result)?;
            }
            Value::Instance(instance) => {
                let receiver = Value::Instance(instance.clone());
                let call_target = self
                    .lookup_bound_special_method(&receiver, "__call__")?
                    .ok_or_else(|| RuntimeError::new("attempted to call non-function"))?;
                match self.call_internal(call_target, args, HashMap::new())? {
                    InternalCallOutcome::Value(value) => self.push_value(value),
                    InternalCallOutcome::CallerExceptionHandled => {}
                }
            }
            Value::ExceptionType(name) => {
                let value = self.instantiate_exception_type(&name, &args, &HashMap::new())?;
                self.push_value(value);
            }
            _ => return Err(RuntimeError::new("attempted to call non-function")),
        }
        Ok(())
    }

    fn push_function_call_one_arg_from_obj(
        &mut self,
        func: &ObjRef,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        let site_index = self.current_site_index();
        let mut clear_cached = false;
        let cached_entry = self
            .frames
            .last()
            .and_then(|frame| frame.one_arg_inline_cache.get(site_index))
            .and_then(|slot| slot.as_ref())
            .cloned()
            .and_then(|entry| {
                if entry.func_id != func.id() {
                    clear_cached = true;
                    return None;
                }
                let valid = {
                    let func_kind = func.kind();
                    match &*func_kind {
                        Object::Function(data) => data.call_cache_epoch == entry.func_epoch,
                        _ => false,
                    }
                };
                if !valid {
                    clear_cached = true;
                    return None;
                }
                Some(entry)
            });
        if let Some(entry) = cached_entry {
            let OneArgCallSiteCacheEntry {
                hot_path,
                cached_code,
                cached_module,
                cached_owner_class,
                cached_closure,
                ..
            } = entry;
            return match hot_path {
                OneArgCallHotPath::SimplePositionalNoCells => {
                    if let (Some(code), Some(module)) = (cached_code, cached_module) {
                        self.push_simple_positional_function_frame_one_arg_no_cells(
                            code,
                            module,
                            cached_owner_class,
                            arg0,
                        )
                    } else {
                        self.push_simple_positional_function_frame_one_arg_no_cells_from_func(
                            func, arg0,
                        )
                    }
                }
                OneArgCallHotPath::SimplePositional => {
                    if let (Some(code), Some(module), Some(closure)) =
                        (cached_code, cached_module, cached_closure)
                    {
                        self.push_simple_positional_function_frame_one_arg(
                            code,
                            module,
                            cached_owner_class,
                            closure,
                            arg0,
                        )
                    } else {
                        self.push_simple_positional_function_frame_one_arg_from_func(func, arg0)
                    }
                }
                OneArgCallHotPath::Generic => {
                    self.push_function_call_from_obj(func, vec![arg0], HashMap::new())
                }
            };
        }
        if clear_cached {
            if let Some(frame) = self.frames.last_mut() {
                if let Some(slot) = frame.one_arg_inline_cache.get_mut(site_index) {
                    *slot = None;
                }
            }
        }

        let (code, module, closure, owner_class, simple_positional_path, no_cells_hot, func_epoch) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::new("attempted to call non-function")),
            };
            let code = func_data.code.clone();
            let simple_positional_path = func_data.plain_positional_call_arity == Some(1);
            let no_cells_hot = simple_positional_path
                && code.plain_positional_arg0_cell.is_none()
                && code.cellvars.is_empty()
                && func_data.closure.is_empty()
                && !code.is_generator
                && !code.is_comprehension;
            (
                code,
                func_data.module.clone(),
                func_data.closure.clone(),
                func_data.owner_class.clone(),
                simple_positional_path,
                no_cells_hot,
                func_data.call_cache_epoch,
            )
        };
        if simple_positional_path {
            if let Some(frame) = self.frames.last_mut() {
                if let Some(slot) = frame.one_arg_inline_cache.get_mut(site_index) {
                    *slot = Some(OneArgCallSiteCacheEntry {
                        func_id: func.id(),
                        func_epoch,
                        hot_path: if no_cells_hot {
                            OneArgCallHotPath::SimplePositionalNoCells
                        } else {
                            OneArgCallHotPath::SimplePositional
                        },
                        cached_code: Some(code.clone()),
                        cached_module: Some(module.clone()),
                        cached_owner_class: owner_class.clone(),
                        cached_closure: if no_cells_hot {
                            None
                        } else {
                            Some(closure.clone())
                        },
                    });
                }
            }
            if no_cells_hot {
                return self.push_simple_positional_function_frame_one_arg_no_cells(
                    code,
                    module,
                    owner_class,
                    arg0,
                );
            }
            return self.push_simple_positional_function_frame_one_arg(
                code,
                module,
                owner_class,
                closure,
                arg0,
            );
        }
        if let Some(frame) = self.frames.last_mut() {
            if let Some(slot) = frame.one_arg_inline_cache.get_mut(site_index) {
                *slot = Some(OneArgCallSiteCacheEntry {
                    func_id: func.id(),
                    func_epoch,
                    hot_path: OneArgCallHotPath::Generic,
                    cached_code: None,
                    cached_module: None,
                    cached_owner_class: None,
                    cached_closure: None,
                });
            }
        }
        self.push_function_call_from_obj(func, vec![arg0], HashMap::new())
    }

    #[inline]
    fn push_simple_positional_function_frame_one_arg_no_cells_from_func(
        &mut self,
        func: &ObjRef,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        let (code, module, owner_class) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::new("attempted to call non-function")),
            };
            (
                func_data.code.clone(),
                func_data.module.clone(),
                func_data.owner_class.clone(),
            )
        };
        self.push_simple_positional_function_frame_one_arg_no_cells(code, module, owner_class, arg0)
    }

    #[inline]
    fn push_simple_positional_function_frame_one_arg_from_func(
        &mut self,
        func: &ObjRef,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        let (code, module, owner_class, closure) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::new("attempted to call non-function")),
            };
            (
                func_data.code.clone(),
                func_data.module.clone(),
                func_data.owner_class.clone(),
                func_data.closure.clone(),
            )
        };
        self.push_simple_positional_function_frame_one_arg(code, module, owner_class, closure, arg0)
    }

    #[inline]
    fn push_simple_positional_function_frame_one_arg_no_cells_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        owner_class: Option<&ObjRef>,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        let slot_idx = code.plain_positional_arg0_slot;
        let mut frame = self.acquire_simple_frame_no_cells_ref(code, module, owner_class);
        if let Some(active_exception) = self
            .frames
            .last()
            .and_then(|caller| caller.active_exception.as_ref())
        {
            frame.active_exception = Some(active_exception.clone());
        }
        if slot_idx == Some(0) && frame.fast_locals.len() == 1 {
            frame.fast_locals[0] = Some(arg0);
            self.frames.push(frame);
            return Ok(());
        }
        if let Some(slot_idx) = slot_idx {
            if slot_idx < frame.fast_locals.len() {
                frame.fast_locals[slot_idx] = Some(arg0);
            } else if let Some(name) = frame
                .code
                .posonly_params
                .first()
                .or_else(|| frame.code.params.first())
                .cloned()
            {
                frame.locals.insert(name, arg0);
            }
        } else if let Some(name) = frame
            .code
            .posonly_params
            .first()
            .or_else(|| frame.code.params.first())
            .cloned()
        {
            frame.locals.insert(name, arg0);
        }
        self.frames.push(frame);
        Ok(())
    }

    fn push_simple_positional_function_frame_one_arg_no_cells(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        self.push_simple_positional_function_frame_one_arg_no_cells_ref(
            &code,
            &module,
            owner_class.as_ref(),
            arg0,
        )
    }

    fn push_function_call_two_args_from_obj(
        &mut self,
        func: &ObjRef,
        arg0: Value,
        arg1: Value,
    ) -> Result<(), RuntimeError> {
        let (code, module, closure, owner_class, simple_positional_path) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::new("attempted to call non-function")),
            };
            (
                func_data.code.clone(),
                func_data.module.clone(),
                func_data.closure.clone(),
                func_data.owner_class.clone(),
                func_data.plain_positional_call_arity == Some(2),
            )
        };
        if simple_positional_path {
            return self.push_simple_positional_function_frame_two_args(
                code, module, owner_class, closure, arg0, arg1,
            );
        }
        self.push_function_call_from_obj(func, vec![arg0, arg1], HashMap::new())
    }

    fn push_function_call_three_args_from_obj(
        &mut self,
        func: &ObjRef,
        arg0: Value,
        arg1: Value,
        arg2: Value,
    ) -> Result<(), RuntimeError> {
        let (code, module, closure, owner_class, simple_positional_path) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::new("attempted to call non-function")),
            };
            (
                func_data.code.clone(),
                func_data.module.clone(),
                func_data.closure.clone(),
                func_data.owner_class.clone(),
                func_data.plain_positional_call_arity == Some(3),
            )
        };
        if simple_positional_path {
            return self.push_simple_positional_function_frame_three_args(
                code, module, owner_class, closure, arg0, arg1, arg2,
            );
        }
        self.push_function_call_from_obj(func, vec![arg0, arg1, arg2], HashMap::new())
    }

    fn push_simple_positional_function_frame_one_arg(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        closure: Vec<ObjRef>,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        let cells = if code.cellvars.is_empty() && closure.is_empty() {
            Vec::new()
        } else {
            self.build_cells(&code, closure)
        };
        let mut frame = self.prepare_function_frame(&code, module, owner_class, cells);

        self.store_fast_positional_arg(&code, &mut frame, 0, arg0);

        if code.is_generator {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.is_coroutine,
                code.is_async_generator,
            )) {
                Value::Generator(obj) => obj,
                _ => unreachable!(),
            };
            frame.generator_owner = Some(generator.clone());
            self.generator_states.insert(generator.id(), frame);
            self.push_value(Value::Generator(generator));
            return Ok(());
        }
        self.frames.push(frame);
        Ok(())
    }

    #[inline]
    fn store_fast_positional_arg(
        &self,
        code: &CodeObject,
        frame: &mut Frame,
        arg_index: usize,
        value: Value,
    ) {
        let (slot_idx, cell_idx) = match arg_index {
            0 => (code.plain_positional_arg0_slot, code.plain_positional_arg0_cell),
            1 => (code.plain_positional_arg1_slot, code.plain_positional_arg1_cell),
            2 => (code.plain_positional_arg2_slot, code.plain_positional_arg2_cell),
            _ => (
                code.positional_param_slot_indexes
                    .get(arg_index)
                    .and_then(|idx| *idx),
                code.positional_param_cell_indexes
                    .get(arg_index)
                    .and_then(|idx| *idx),
            ),
        };
        if let Some(cell_idx) = cell_idx {
            if let Some(cell) = frame.cells.get(cell_idx) {
                if let Object::Cell(cell_data) = &mut *cell.kind_mut() {
                    cell_data.value = Some(value);
                    return;
                }
            }
        }
        if let Some(slot_idx) = slot_idx {
            if let Some(slot) = frame.fast_locals.get_mut(slot_idx) {
                Self::write_fast_local_slot(slot, value);
                return;
            }
        }
        let posonly_len = code.posonly_params.len();
        let fallback_name = if arg_index < posonly_len {
            code.posonly_params.get(arg_index)
        } else {
            code.params.get(arg_index - posonly_len)
        };
        if let Some(name) = fallback_name {
            frame.locals.insert(name.clone(), value);
        }
    }

    #[inline]
    fn prepare_function_frame(
        &mut self,
        code: &Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        cells: Vec<ObjRef>,
    ) -> Box<Frame> {
        let caller_active_exception = self
            .frames
            .last()
            .and_then(|frame| frame.active_exception.clone());
        let module_id = module.id();
        let mut frame = self.acquire_frame(code.clone(), module, false, false, cells, owner_class);
        frame.active_exception = caller_active_exception;
        if code.is_comprehension {
            if let Some(caller) = self.frames.last() {
                if caller.return_class && caller.module.id() == module_id {
                    frame.globals_fallback = Some(caller.function_globals.clone());
                }
            }
        }
        frame
    }

    fn push_simple_positional_function_frame_two_args(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        closure: Vec<ObjRef>,
        arg0: Value,
        arg1: Value,
    ) -> Result<(), RuntimeError> {
        let cells = if code.cellvars.is_empty() && closure.is_empty() {
            Vec::new()
        } else {
            self.build_cells(&code, closure)
        };
        let mut frame = self.prepare_function_frame(&code, module, owner_class, cells);

        self.store_fast_positional_arg(&code, &mut frame, 0, arg0);
        self.store_fast_positional_arg(&code, &mut frame, 1, arg1);

        if code.is_generator {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.is_coroutine,
                code.is_async_generator,
            )) {
                Value::Generator(obj) => obj,
                _ => unreachable!(),
            };
            frame.generator_owner = Some(generator.clone());
            self.generator_states.insert(generator.id(), frame);
            self.push_value(Value::Generator(generator));
            return Ok(());
        }
        self.frames.push(frame);
        Ok(())
    }

    fn push_simple_positional_function_frame_three_args(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        closure: Vec<ObjRef>,
        arg0: Value,
        arg1: Value,
        arg2: Value,
    ) -> Result<(), RuntimeError> {
        let cells = if code.cellvars.is_empty() && closure.is_empty() {
            Vec::new()
        } else {
            self.build_cells(&code, closure)
        };
        let mut frame = self.prepare_function_frame(&code, module, owner_class, cells);

        self.store_fast_positional_arg(&code, &mut frame, 0, arg0);
        self.store_fast_positional_arg(&code, &mut frame, 1, arg1);
        self.store_fast_positional_arg(&code, &mut frame, 2, arg2);

        if code.is_generator {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.is_coroutine,
                code.is_async_generator,
            )) {
                Value::Generator(obj) => obj,
                _ => unreachable!(),
            };
            frame.generator_owner = Some(generator.clone());
            self.generator_states.insert(generator.id(), frame);
            self.push_value(Value::Generator(generator));
            return Ok(());
        }
        self.frames.push(frame);
        Ok(())
    }

    pub(super) fn push_function_call_from_obj(
        &mut self,
        func: &ObjRef,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        let (code, module, closure, owner_class, simple_positional_path) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::new("attempted to call non-function")),
            };
            let code = func_data.code.clone();
            let simple_positional_path =
                kwargs.is_empty() && func_data.plain_positional_call_arity == Some(args.len());
            (
                code,
                func_data.module.clone(),
                func_data.closure.clone(),
                func_data.owner_class.clone(),
                simple_positional_path,
            )
        };
        if simple_positional_path {
            return self.push_simple_positional_function_frame(
                code,
                module,
                owner_class,
                closure,
                args,
            );
        }
        let bindings = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::new("attempted to call non-function")),
            };
            bind_arguments(func_data, &self.heap, args, kwargs)?
        };
        let cells = if code.cellvars.is_empty() && closure.is_empty() {
            Vec::new()
        } else {
            self.build_cells(&code, closure)
        };
        self.push_function_frame(code, module, owner_class, bindings, cells)
    }

    fn push_simple_positional_function_frame(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        closure: Vec<ObjRef>,
        args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        let cells = if code.cellvars.is_empty() && closure.is_empty() {
            Vec::new()
        } else {
            self.build_cells(&code, closure)
        };
        let mut frame = self.prepare_function_frame(&code, module, owner_class, cells);

        for (arg_idx, value) in args.into_iter().enumerate() {
            self.store_fast_positional_arg(&code, &mut frame, arg_idx, value);
        }

        if code.is_generator {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.is_coroutine,
                code.is_async_generator,
            )) {
                Value::Generator(obj) => obj,
                _ => unreachable!(),
            };
            frame.generator_owner = Some(generator.clone());
            self.generator_states.insert(generator.id(), frame);
            self.push_value(Value::Generator(generator));
            return Ok(());
        }
        self.frames.push(frame);
        Ok(())
    }

    fn push_function_frame(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        bindings: BoundArguments,
        cells: Vec<ObjRef>,
    ) -> Result<(), RuntimeError> {
        let mut frame = self.prepare_function_frame(&code, module, owner_class, cells);
        apply_bindings(&mut frame, &code, bindings, &self.heap);
        if code.is_generator {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.is_coroutine,
                code.is_async_generator,
            )) {
                Value::Generator(obj) => obj,
                _ => unreachable!(),
            };
            frame.generator_owner = Some(generator.clone());
            self.generator_states.insert(generator.id(), frame);
            self.push_value(Value::Generator(generator));
            return Ok(());
        }
        self.frames.push(frame);
        Ok(())
    }

    pub(super) fn receiver_value(&self, receiver: &ObjRef) -> Result<Value, RuntimeError> {
        match &*receiver.kind() {
            Object::Instance(_) => Ok(Value::Instance(receiver.clone())),
            Object::Class(_) => Ok(Value::Class(receiver.clone())),
            Object::Generator(_) => Ok(Value::Generator(receiver.clone())),
            Object::Module(_) => Ok(Value::Module(receiver.clone())),
            Object::List(_) => Ok(Value::List(receiver.clone())),
            Object::Tuple(_) => Ok(Value::Tuple(receiver.clone())),
            Object::Dict(_) => Ok(Value::Dict(receiver.clone())),
            Object::Set(_) => Ok(Value::Set(receiver.clone())),
            Object::FrozenSet(_) => Ok(Value::FrozenSet(receiver.clone())),
            Object::Bytes(_) => Ok(Value::Bytes(receiver.clone())),
            Object::ByteArray(_) => Ok(Value::ByteArray(receiver.clone())),
            Object::MemoryView(_) => Ok(Value::MemoryView(receiver.clone())),
            _ => Err(RuntimeError::new("unsupported bound method receiver")),
        }
    }

    pub(super) fn bound_method_reduce_receiver_value(&self, receiver: &ObjRef) -> Result<Value, RuntimeError> {
        if let Object::Module(module_data) = &*receiver.kind() {
            if let Some(value) = module_data.globals.get("value") {
                return Ok(value.clone());
            }
            if let Some(owner) = module_data.globals.get("owner") {
                return Ok(owner.clone());
            }
        }
        self.receiver_value(receiver)
    }

    pub(super) fn native_method_pickle_name(&self, kind: NativeMethodKind) -> Option<&'static str> {
        match kind {
            NativeMethodKind::TupleCount => Some("count"),
            NativeMethodKind::StrCount => Some("count"),
            NativeMethodKind::StrIndex => Some("index"),
            NativeMethodKind::SetContains => Some("__contains__"),
            NativeMethodKind::Builtin(BuiltinFunction::DictFromKeys) => Some("fromkeys"),
            NativeMethodKind::Builtin(BuiltinFunction::BytesMakeTrans) => Some("maketrans"),
            NativeMethodKind::Builtin(BuiltinFunction::StrMakeTrans) => Some("maketrans"),
            NativeMethodKind::Builtin(BuiltinFunction::Len) => Some("__len__"),
            NativeMethodKind::Builtin(BuiltinFunction::OperatorContains) => Some("__contains__"),
            _ => None,
        }
    }

    pub(super) fn receiver_from_value(&self, value: &Value) -> Result<ObjRef, RuntimeError> {
        match value {
            Value::Instance(obj)
            | Value::Class(obj)
            | Value::Generator(obj)
            | Value::Module(obj)
            | Value::List(obj)
            | Value::Tuple(obj)
            | Value::Dict(obj)
            | Value::Set(obj)
            | Value::FrozenSet(obj)
            | Value::Bytes(obj)
            | Value::ByteArray(obj)
            | Value::MemoryView(obj) => Ok(obj.clone()),
            _ => Err(RuntimeError::new("unsupported bound-method receiver value")),
        }
    }

    pub(super) fn class_of_value(&self, value: &Value) -> Option<ObjRef> {
        match value {
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => Some(instance_data.class.clone()),
                _ => None,
            },
            Value::Class(class) => match &*class.kind() {
                Object::Class(class_data) => class_data
                    .metaclass
                    .clone()
                    .or_else(|| self.default_type_metaclass()),
                _ => self.default_type_metaclass(),
            },
            Value::Super(super_obj) => match &*super_obj.kind() {
                Object::Super(data) => Some(data.object_type.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn default_type_metaclass(&self) -> Option<ObjRef> {
        let Value::Class(object_class) = self.builtins.get("object")? else {
            return None;
        };
        let Object::Class(class_data) = &*object_class.kind() else {
            return None;
        };
        class_data.metaclass.clone()
    }

    pub(super) fn alloc_native_bound_method(&self, kind: NativeMethodKind, receiver: ObjRef) -> Value {
        let native = self.heap.alloc_native_method(NativeMethodObject::new(kind));
        let bound = BoundMethod::new(native, receiver);
        self.heap.alloc_bound_method(bound)
    }

    pub(super) fn alloc_builtin_bound_method(&self, builtin: BuiltinFunction, receiver: ObjRef) -> Value {
        self.alloc_native_bound_method(NativeMethodKind::Builtin(builtin), receiver)
    }

    pub(super) fn alloc_builtin_unbound_method(
        &self,
        wrapper_name: &str,
        owner: Value,
        builtin: BuiltinFunction,
    ) -> Value {
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new(wrapper_name.to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data.globals.insert("owner".to_string(), owner);
        }
        self.alloc_native_bound_method(NativeMethodKind::Builtin(builtin), receiver)
    }

    pub(super) fn alloc_reduce_ex_bound_method(&self, value: Value) -> Value {
        let wrapper = match self
            .heap
            .alloc_module(ModuleObject::new("__object_reduce_ex_bound__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
            module_data.globals.insert("value".to_string(), value);
        }
        self.alloc_native_bound_method(NativeMethodKind::ObjectReduceExBound, wrapper)
    }

    pub(super) fn load_dunder_class_attr(&self, value: &Value) -> Result<Value, RuntimeError> {
        if let Some(class) = self.class_of_value(value) {
            return Ok(Value::Class(class));
        }
        if let Value::Function(_) = value {
            if let Some(class) = self.types_module_class("FunctionType") {
                return Ok(Value::Class(class));
            }
        }
        if let Value::Builtin(builtin) = value {
            if self.builtin_is_type_object(*builtin) {
                return Ok(Value::Builtin(BuiltinFunction::Type));
            }
            if let Some(class) = self.types_module_class("BuiltinFunctionType") {
                return Ok(Value::Class(class));
            }
        }
        if matches!(value, Value::BoundMethod(_)) {
            return Ok(Value::Builtin(BuiltinFunction::TypesMethodType));
        }
        if let Value::Code(_) = value {
            if let Some(class) = self.types_module_class("CodeType") {
                return Ok(Value::Class(class));
            }
        }
        if let Value::None = value {
            if let Some(class) = self.types_module_class("NoneType") {
                return Ok(Value::Class(class));
            }
        }
        BuiltinFunction::Type.call(&self.heap, vec![value.clone()])
    }

    pub(super) fn property_descriptor_parts(
        &self,
        descriptor: &ObjRef,
    ) -> Option<(Value, Value, Value, Value)> {
        let descriptor_kind = descriptor.kind();
        let instance_data = match &*descriptor_kind {
            Object::Instance(instance_data) => instance_data,
            _ => return None,
        };
        match instance_data.attrs.get("__pyrs_property__") {
            Some(Value::Bool(true)) => {}
            _ => return None,
        }
        let fget = instance_data
            .attrs
            .get("fget")
            .cloned()
            .unwrap_or(Value::None);
        let fset = instance_data
            .attrs
            .get("fset")
            .cloned()
            .unwrap_or(Value::None);
        let fdel = instance_data
            .attrs
            .get("fdel")
            .cloned()
            .unwrap_or(Value::None);
        let doc = instance_data
            .attrs
            .get("__doc__")
            .cloned()
            .unwrap_or(Value::None);
        Some((fget, fset, fdel, doc))
    }

    pub(super) fn cached_property_descriptor_parts(
        &self,
        descriptor: &ObjRef,
    ) -> Option<(Value, Option<String>, Value)> {
        let descriptor_kind = descriptor.kind();
        let instance_data = match &*descriptor_kind {
            Object::Instance(instance_data) => instance_data,
            _ => return None,
        };
        match instance_data.attrs.get("__pyrs_cached_property__") {
            Some(Value::Bool(true)) => {}
            _ => return None,
        }
        let func = instance_data
            .attrs
            .get("func")
            .cloned()
            .unwrap_or(Value::None);
        let attr_name = match instance_data.attrs.get("attrname") {
            Some(Value::Str(name)) => Some(name.clone()),
            _ => None,
        };
        let doc = instance_data
            .attrs
            .get("__doc__")
            .cloned()
            .unwrap_or(Value::None);
        Some((func, attr_name, doc))
    }

    pub(super) fn build_property_descriptor(
        &self,
        fget: Value,
        fset: Value,
        fdel: Value,
        doc: Value,
    ) -> Value {
        let class = match self
            .heap
            .alloc_class(ClassObject::new("property".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        let mut instance = InstanceObject::new(class);
        instance
            .attrs
            .insert("__pyrs_property__".to_string(), Value::Bool(true));
        instance.attrs.insert("fget".to_string(), fget);
        instance.attrs.insert("fset".to_string(), fset);
        instance.attrs.insert("fdel".to_string(), fdel);
        instance.attrs.insert("__doc__".to_string(), doc);
        self.heap.alloc_instance(instance)
    }

    pub(super) fn clone_property_descriptor_with(
        &self,
        descriptor: &ObjRef,
        fget: Option<Value>,
        fset: Option<Value>,
        fdel: Option<Value>,
        doc: Option<Value>,
    ) -> Result<Value, RuntimeError> {
        let Some((current_get, current_set, current_del, current_doc)) =
            self.property_descriptor_parts(descriptor)
        else {
            return Err(RuntimeError::new(
                "property method receiver must be property",
            ));
        };
        Ok(self.build_property_descriptor(
            fget.unwrap_or(current_get),
            fset.unwrap_or(current_set),
            fdel.unwrap_or(current_del),
            doc.unwrap_or(current_doc),
        ))
    }

    pub(super) fn load_attr_property_instance(&self, instance: &ObjRef, attr_name: &str) -> Option<Value> {
        let Some((fget, fset, fdel, doc)) = self.property_descriptor_parts(instance) else {
            return None;
        };
        match attr_name {
            "fget" => Some(fget),
            "fset" => Some(fset),
            "fdel" => Some(fdel),
            "__doc__" => Some(doc),
            "__isabstractmethod__" => Some(Value::Bool(false)),
            "__get__" => Some(
                self.alloc_native_bound_method(NativeMethodKind::PropertyGet, instance.clone()),
            ),
            "__set__" => Some(
                self.alloc_native_bound_method(NativeMethodKind::PropertySet, instance.clone()),
            ),
            "__delete__" => Some(
                self.alloc_native_bound_method(NativeMethodKind::PropertyDelete, instance.clone()),
            ),
            "getter" => Some(
                self.alloc_native_bound_method(NativeMethodKind::PropertyGetter, instance.clone()),
            ),
            "setter" => Some(
                self.alloc_native_bound_method(NativeMethodKind::PropertySetter, instance.clone()),
            ),
            "deleter" => Some(
                self.alloc_native_bound_method(NativeMethodKind::PropertyDeleter, instance.clone()),
            ),
            _ => None,
        }
    }

    pub(super) fn load_attr_cached_property_instance(
        &self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Option<Value> {
        let Some((func, attr_name_value, doc)) = self.cached_property_descriptor_parts(instance)
        else {
            return None;
        };
        match attr_name {
            "func" => Some(func),
            "attrname" => Some(attr_name_value.map(Value::Str).unwrap_or(Value::None)),
            "__doc__" => Some(doc),
            "__isabstractmethod__" => Some(Value::Bool(false)),
            "__get__" => {
                Some(self.alloc_native_bound_method(
                    NativeMethodKind::CachedPropertyGet,
                    instance.clone(),
                ))
            }
            _ => None,
        }
    }

}
