//! VM instruction execution, exception unwind, and traceback plumbing.
//!
//! This file contains the hot-path interpreter loop plus the control-flow
//! machinery that maps runtime failures into CPython-shaped exception and
//! traceback behavior.

use std::cell::RefMut;

#[cfg(not(debug_assertions))]
use super::FusedDirectOneArgNoCellsMetadata;
#[cfg(not(debug_assertions))]
use super::LoadFastSiteCacheEntry;
use super::{
    AttrAccessOutcome, AttrMutationOutcome, Block, BoundArguments, BoundMethod, BuiltinFunction,
    ClassBuildOutcome, ClassObject, CodeObject, ExceptionObject, Frame, FunctionObject,
    GeneratorObject, GeneratorResumeKind, GeneratorResumeOutcome, HashMap, HashSet,
    INSTANCE_DICT_STORAGE_ATTR, ImportReturnPolicy, InstanceObject, Instruction,
    InternalCallOutcome, LoadAttrSiteCacheEntry, LoadAttrSiteCacheKind, LoadGlobalSiteCacheEntry,
    MAPPING_PROXY_STORAGE_ATTR, ModuleObject, NativeMethodKind, NativeMethodObject, ObjRef, Object,
    OneArgCallHotPath, OneArgCallSiteCacheEntry, Opcode, PY_TPFLAGS_HEAPTYPE,
    PY_TPFLAGS_IMMUTABLETYPE, QuickenedSiteKind, Rc, RuntimeError, SOURCE_FILE_LOADER,
    SOURCELESS_FILE_LOADER, TraceFrame, Value, Vm, and_values, apply_bindings, bind_arguments,
    builtin_exception_parent, class_attr_lookup, class_attr_lookup_direct, decode_call_counts,
    deref_name, dict_get_value, dict_remove_value, dict_set_value, dict_set_value_checked,
    exception_message_from_call_args, format_repr, format_value, is_comprehension_code,
    is_import_error_family, is_missing_attribute_error, is_os_error_family, is_truthy,
    lshift_values, memoryview_bounds, memoryview_element_offset, memoryview_encode_element,
    memoryview_format_for_view, memoryview_layout_1d_from_parts, module_globals_version, pos_value,
    pow_values, rshift_values, runtime_error_matches_exception, slice_bounds_for_step_one,
    slice_indices, slot_names_from_value, source_path_from_cache_path, value_from_bigint,
    value_from_object_ref, value_to_int, value_to_optional_index,
};
use crate::bytecode::Location;
use crate::runtime::{
    BoundMethodDispatchKind, DictViewKind, ExceptionTracebackFrame, InstanceAttributes, SliceValue,
    builtin_type_name_info,
};

unsafe extern "C" {
    fn PyErr_Clear();
}

pub(super) trait SyntaxErrorAttrStore {
    fn insert_attr(&mut self, name: &str, value: Value);
}

impl SyntaxErrorAttrStore for HashMap<String, Value> {
    fn insert_attr(&mut self, name: &str, value: Value) {
        self.insert(name.to_string(), value);
    }
}

impl SyntaxErrorAttrStore for InstanceAttributes {
    fn insert_attr(&mut self, name: &str, value: Value) {
        self.insert(name.to_string(), value);
    }
}

impl<T> SyntaxErrorAttrStore for RefMut<'_, T>
where
    T: SyntaxErrorAttrStore,
{
    fn insert_attr(&mut self, name: &str, value: Value) {
        (**self).insert_attr(name, value);
    }
}

thread_local! {
    static DEBUG_HANDLE_RUNTIME_DEPTH: Cell<usize> = const { Cell::new(0) };
    static DEBUG_RAISE_DEPTH: Cell<usize> = const { Cell::new(0) };
    static DEBUG_UNWIND_DEPTH: Cell<usize> = const { Cell::new(0) };
    static DEBUG_EXEC_INSTR_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Optional recursion/depth guard used while debugging unwind/raise flows.
///
/// The guard is only active when `PYRS_DEBUG_EXCEPTION_UNWIND_DEPTH` is set.
struct DebugDepthGuard {
    key: &'static std::thread::LocalKey<Cell<usize>>,
}

impl DebugDepthGuard {
    #[inline(always)]
    fn enter_for_vm(
        vm: &Vm,
        key: &'static std::thread::LocalKey<Cell<usize>>,
        label: &'static str,
    ) -> Option<Self> {
        if !vm.debug_exception_unwind_depth_enabled {
            return None;
        }
        let limit = vm.debug_exception_unwind_depth_limit;
        let depth = key.with(|cell| {
            let next = cell.get().saturating_add(1);
            cell.set(next);
            next
        });
        if depth > limit {
            panic!(
                "exception unwind recursion depth exceeded in {label}: depth={depth} limit={limit}"
            );
        }
        Some(Self { key })
    }
}

impl Drop for DebugDepthGuard {
    fn drop(&mut self) {
        self.key.with(|cell| {
            cell.set(cell.get().saturating_sub(1));
        });
    }
}

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

    #[inline(always)]
    fn clone_fast_local_stack_value(value: &Value) -> Value {
        match value {
            Value::Int(integer) => Value::Int(*integer),
            Value::Float(number) => Value::Float(*number),
            Value::Bool(flag) => Value::Bool(*flag),
            Value::None => Value::None,
            _ => value.clone(),
        }
    }

    #[inline]
    fn is_fast_local_unbound_marker(&self, value: &Value) -> bool {
        match (value, &self.fast_local_unbound_marker) {
            (Value::Instance(lhs), Value::Instance(rhs)) => lhs.id() == rhs.id(),
            _ => false,
        }
    }

    pub(super) fn class_assignment_is_global(&self, frame: &Frame, class_name: &str) -> bool {
        for instr in frame
            .code
            .instructions
            .iter()
            .skip(frame.last_ip.saturating_add(1))
        {
            match instr.opcode {
                Opcode::StoreGlobal => {
                    let Some(name_idx) = instr.arg.map(|idx| idx as usize) else {
                        continue;
                    };
                    if frame
                        .code
                        .names
                        .get(name_idx)
                        .is_some_and(|name| name == class_name)
                    {
                        return true;
                    }
                }
                Opcode::StoreName => {
                    let Some(name_idx) = instr.arg.map(|idx| idx as usize) else {
                        continue;
                    };
                    if frame
                        .code
                        .names
                        .get(name_idx)
                        .is_some_and(|name| name == class_name)
                    {
                        return false;
                    }
                }
                Opcode::StoreFast => {
                    let Some(name_idx) = instr.arg.map(|idx| idx as usize) else {
                        continue;
                    };
                    if frame
                        .code
                        .names
                        .get(name_idx)
                        .is_some_and(|name| name == class_name)
                    {
                        return false;
                    }
                }
                Opcode::StoreDeref => {
                    let Some(name_idx) = instr.arg.map(|idx| idx as usize) else {
                        continue;
                    };
                    let maybe_name = if name_idx < frame.code.cellvars.len() {
                        frame.code.cellvars.get(name_idx)
                    } else {
                        frame
                            .code
                            .freevars
                            .get(name_idx.saturating_sub(frame.code.cellvars.len()))
                    };
                    if maybe_name.is_some_and(|name| name == class_name) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        false
    }

    #[inline]
    fn nearest_active_exception(&self) -> Option<(Value, Option<usize>)> {
        for frame in self.frames.iter().rev() {
            if let Some(exc) = frame.active_exception.clone() {
                return Some((exc, frame.except_star_match_lasti));
            }
        }
        None
    }

    #[inline]
    fn rewrite_module_value_reference(value: &mut Value, replaced_id: u64, canonical: &ObjRef) {
        if let Value::Module(module) = value
            && module.id() == replaced_id
        {
            *value = Value::Module(canonical.clone());
        }
    }

    #[inline]
    fn rewrite_module_refs_in_values(values: &mut [Value], replaced_id: u64, canonical: &ObjRef) {
        for value in values {
            Self::rewrite_module_value_reference(value, replaced_id, canonical);
        }
    }

    #[inline]
    fn rewrite_module_refs_in_named_values(
        values: &mut HashMap<String, Value>,
        replaced_id: u64,
        canonical: &ObjRef,
    ) {
        for value in values.values_mut() {
            Self::rewrite_module_value_reference(value, replaced_id, canonical);
        }
    }

    #[inline]
    fn reconcile_live_module_references_after_replacement(
        &mut self,
        replaced: &ObjRef,
        canonical: &ObjRef,
    ) {
        if replaced.id() == canonical.id() {
            return;
        }
        let replaced_id = replaced.id();
        for frame in self.frames.iter_mut() {
            Self::rewrite_module_refs_in_values(&mut frame.stack, replaced_id, canonical);
            Self::rewrite_module_refs_in_named_values(&mut frame.locals, replaced_id, canonical);
            if let Some(locals_fallback) = frame.locals_fallback.as_mut() {
                Self::rewrite_module_refs_in_named_values(locals_fallback, replaced_id, canonical);
            }
            for value in frame.fast_locals.iter_mut().flatten() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            if let Some(value) = frame.class_namespace.as_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            if let Some(value) = frame.class_orig_bases.as_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            if let Some(value) = frame.class_metaclass.as_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            for value in frame.class_keywords.values_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            if let Some(value) = frame.active_exception.as_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            if let Some(value) = frame.generator_resume_value.as_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            if let Some(value) = frame.generator_pending_throw.as_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
            if let Some(value) = frame.yield_from_iter.as_mut() {
                Self::rewrite_module_value_reference(value, replaced_id, canonical);
            }
        }
    }

    #[inline]
    fn finalize_module_frame_success(&mut self, frame: &Frame) {
        if !frame.is_module {
            return;
        }
        self.clear_module_initializing(&frame.module);
        let module_name = match &*frame.module.kind() {
            Object::Module(module_data) => module_data.name.clone(),
            _ => String::new(),
        };
        let canonical = if module_name.is_empty() {
            frame.module.clone()
        } else {
            let canonical =
                self.canonical_imported_module_for_name(&module_name, frame.module.clone());
            if canonical.id() != frame.module.id() {
                self.reconcile_live_module_references_after_replacement(&frame.module, &canonical);
                self.link_module_chain(&module_name, canonical.clone());
            }
            canonical
        };
        self.sync_standard_os_path_aliases(&module_name);
        self.sync_re_module_flag_aliases(&canonical);
    }

    fn sync_standard_os_path_aliases(&mut self, module_name: &str) {
        let platform_path_name = if cfg!(windows) { "ntpath" } else { "posixpath" };
        if module_name != "os" && module_name != platform_path_name {
            return;
        }
        let Some(path_module) = self.modules.get(platform_path_name).cloned() else {
            return;
        };
        if let Some(os_module) = self.modules.get("os").cloned() {
            self.upsert_module_global(&os_module, "path", Value::Module(path_module.clone()));
        }
        self.modules
            .insert("os.path".to_string(), path_module.clone());
        self.refresh_sys_modules_dict();
        self.link_module_chain("os.path", path_module);
    }

    #[inline]
    fn cleanup_failed_module_frame(&mut self, frame: &Frame) -> Result<bool, RuntimeError> {
        if !frame.is_module {
            return Ok(false);
        }
        self.clear_module_initializing(&frame.module);
        let (module_name, loader_name, is_package, package_dirs) = match &*frame.module.kind() {
            Object::Module(module_data) => {
                let package_dirs = match module_data.globals.get("__path__") {
                    Some(Value::List(paths)) => match &*paths.kind() {
                        Object::List(values) => values
                            .iter()
                            .filter_map(|value| match value {
                                Value::Str(path) => Some(std::path::PathBuf::from(path)),
                                _ => None,
                            })
                            .collect::<Vec<_>>(),
                        _ => Vec::new(),
                    },
                    _ => Vec::new(),
                };
                (
                    module_data.name.clone(),
                    Vm::module_loader_name(&frame.module).unwrap_or_default(),
                    !package_dirs.is_empty(),
                    package_dirs,
                )
            }
            _ => return Ok(false),
        };
        if module_name.is_empty() || module_name == "__main__" {
            return Ok(false);
        }
        if self.prefer_pyc_when_source_available
            && loader_name == SOURCELESS_FILE_LOADER
            && let Some(origin_path) = Vm::module_origin_path(&frame.module)
        {
            let source_path = std::path::PathBuf::from(source_path_from_cache_path(
                &origin_path.to_string_lossy(),
            ));
            if source_path.is_file() {
                let runtime_reason = if self.trace_flags.import_perf_verbose {
                    self.frames
                        .last()
                        .and_then(|f| f.active_exception.as_ref())
                        .map(|exc| {
                            format!("active_exception={}", self.value_type_name_for_error(exc))
                        })
                } else {
                    None
                };
                if self.import_perf_enabled {
                    self.import_perf_counters.pyc_load_fallback_to_source = self
                        .import_perf_counters
                        .pyc_load_fallback_to_source
                        .saturating_add(1);
                }
                if self.trace_flags.import_perf_verbose {
                    let reason = runtime_reason.unwrap_or_else(|| "runtime exception".to_string());
                    eprintln!(
                        "[import-perf] pyc-runtime-fallback module={} pyc={} source={} reason={}",
                        module_name,
                        origin_path.display(),
                        source_path.display(),
                        reason
                    );
                }
                self.clear_active_exception();
                self.remove_module_entry_and_parent_binding(&module_name);
                let replacement =
                    self.create_module_for_loader(&module_name, SOURCE_FILE_LOADER)?;
                self.set_module_metadata(
                    &replacement,
                    &module_name,
                    Some(&source_path),
                    None,
                    Some(SOURCE_FILE_LOADER),
                    is_package,
                    package_dirs.clone(),
                    false,
                );
                self.register_module(&module_name, replacement.clone());
                self.link_module_chain(&module_name, replacement.clone());
                self.queue_source_module_execution(&replacement, &module_name, &source_path)?;
                return Ok(true);
            }
        }
        let import_loader =
            loader_name == SOURCE_FILE_LOADER || loader_name == SOURCELESS_FILE_LOADER;
        if import_loader {
            self.remove_module_entry_and_parent_binding(&module_name);
        }
        Ok(false)
    }

    fn stack_dict_target_for_merge(
        &self,
        oparg: usize,
        opname: &str,
    ) -> Result<ObjRef, RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        if frame.stack.len() < oparg {
            return Err(RuntimeError::new(format!("{opname} stack underflow")));
        }
        let dict_index = frame.stack.len() - oparg;
        let dict_value = frame
            .stack
            .get(dict_index)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("{opname} stack underflow")))?;
        match dict_value {
            Value::Dict(dict) => Ok(dict),
            _ => Err(RuntimeError::new(format!("{opname} expects dict target"))),
        }
    }

    fn stack_list_target(&self, oparg: usize, opname: &str) -> Result<ObjRef, RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        if frame.stack.len() < oparg {
            return Err(RuntimeError::new(format!("{opname} stack underflow")));
        }
        let list_index = frame.stack.len() - oparg;
        let list_value = frame
            .stack
            .get(list_index)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("{opname} stack underflow")))?;
        match list_value {
            Value::List(list) => Ok(list),
            _ => Err(RuntimeError::new(format!("{opname} expects list target"))),
        }
    }

    fn stack_set_target(&self, oparg: usize, opname: &str) -> Result<ObjRef, RuntimeError> {
        let frame = self.frames.last().expect("frame exists");
        if frame.stack.len() < oparg {
            return Err(RuntimeError::new(format!("{opname} stack underflow")));
        }
        let set_index = frame.stack.len() - oparg;
        let set_value = frame
            .stack
            .get(set_index)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("{opname} stack underflow")))?;
        match set_value {
            Value::Set(set) => Ok(set),
            _ => Err(RuntimeError::new(format!("{opname} expects set target"))),
        }
    }

    fn mapping_entries_for_update(
        &mut self,
        source: Value,
        require_mapping: bool,
    ) -> Result<Vec<(Value, Value)>, RuntimeError> {
        if let Value::Dict(other) = &source {
            let Object::Dict(entries) = &*other.kind() else {
                return Err(RuntimeError::new("dict update expects dict"));
            };
            return Ok(entries.to_vec());
        }

        let keys_callable = self
            .builtin_getattr(
                vec![source.clone(), Value::Str("keys".to_string())],
                HashMap::new(),
            )
            .ok();
        if let Some(keys_callable) = keys_callable {
            if self.trace_flags.dict_merge {
                eprintln!(
                    "[dict-merge] keys() path for {}",
                    self.value_type_name_for_error(&source)
                );
            }
            let keys_value = match self.call_internal(keys_callable, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(
                        self.runtime_error_from_active_exception("mapping keys() lookup failed")
                    );
                }
            };
            let mut incoming = Vec::new();
            for key in self.collect_iterable_values(keys_value)? {
                let value = self.getitem_value(source.clone(), key.clone())?;
                incoming.push((key, value));
            }
            return Ok(incoming);
        }

        if require_mapping {
            return Err(RuntimeError::new(format!(
                "'{}' object is not a mapping",
                self.value_type_name_for_error(&source)
            )));
        }

        let mut incoming = Vec::new();
        let pairs = self.collect_iterable_values(source)?;
        for pair in pairs {
            let pair_tuple = match pair {
                Value::Tuple(obj) => match &*obj.kind() {
                    Object::Tuple(values) if values.len() == 2 => {
                        (values[0].clone(), values[1].clone())
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "dict.update() expects mapping or iterable of pairs",
                        ));
                    }
                },
                Value::List(obj) => match &*obj.kind() {
                    Object::List(values) if values.len() == 2 => {
                        (values[0].clone(), values[1].clone())
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "dict.update() expects mapping or iterable of pairs",
                        ));
                    }
                },
                _ => {
                    return Err(RuntimeError::new(
                        "dict.update() expects mapping or iterable of pairs",
                    ));
                }
            };
            incoming.push(pair_tuple);
        }
        Ok(incoming)
    }

    fn apply_dict_stack_merge(
        &mut self,
        oparg: usize,
        update: Value,
        reject_duplicates: bool,
        opname: &str,
    ) -> Result<(), RuntimeError> {
        let dict = self.stack_dict_target_for_merge(oparg, opname)?;
        let incoming = self.mapping_entries_for_update(update, reject_duplicates)?;
        for (key, value) in incoming {
            if reject_duplicates && self.dict_contains_key_checked_runtime(&dict, &key)? {
                let key_text = match &key {
                    Value::Str(name) => name.clone(),
                    _ => format_value(&key),
                };
                return Err(RuntimeError::new(format!(
                    "got multiple values for keyword argument '{key_text}'"
                )));
            }
            self.dict_set_value_checked_runtime(&dict, key, value)?;
        }
        Ok(())
    }

    #[inline]
    fn load_special_spec(oparg: u32) -> Option<(&'static str, &'static str)> {
        match oparg {
            0 => Some((
                "__enter__",
                "object does not support the context manager protocol (missed __enter__ method)",
            )),
            1 => Some((
                "__exit__",
                "object does not support the context manager protocol (missed __exit__ method)",
            )),
            2 => Some((
                "__aenter__",
                "object does not support the asynchronous context manager protocol (missed __aenter__ method)",
            )),
            3 => Some((
                "__aexit__",
                "object does not support the asynchronous context manager protocol (missed __aexit__ method)",
            )),
            _ => None,
        }
    }

    fn load_special_method(
        &mut self,
        receiver: &Value,
        method_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let trace = self.trace_flags.load_special;
        if trace && method_name == "__exit__" {
            eprintln!(
                "[load-special] start receiver_type={}",
                self.value_type_name_for_error(receiver)
            );
        }
        let Some(class_ref) = self.class_of_value(receiver) else {
            if let Value::MemoryView(view) = receiver {
                let method_kind = match method_name {
                    "__enter__" => Some(NativeMethodKind::MemoryViewEnter),
                    "__exit__" => Some(NativeMethodKind::MemoryViewExit),
                    _ => None,
                };
                if let Some(kind) = method_kind {
                    return Ok(Some(self.alloc_native_bound_method(kind, view.clone())));
                }
            }
            if Self::cpython_proxy_raw_ptr_from_value(receiver).is_some()
                && let Some(method) = self.load_cpython_proxy_attr_for_value(receiver, method_name)
            {
                if trace && method_name == "__exit__" {
                    eprintln!("[load-special] proxy attr hit (no class)");
                }
                return Ok(Some(method));
            }
            if trace && method_name == "__exit__" {
                eprintln!("[load-special] no class");
            }
            return Ok(None);
        };
        let Some(method) = class_attr_lookup(&class_ref, method_name) else {
            if let Value::MemoryView(view) = receiver {
                let method_kind = match method_name {
                    "__enter__" => Some(NativeMethodKind::MemoryViewEnter),
                    "__exit__" => Some(NativeMethodKind::MemoryViewExit),
                    _ => None,
                };
                if let Some(kind) = method_kind {
                    return Ok(Some(self.alloc_native_bound_method(kind, view.clone())));
                }
            }
            if Self::cpython_proxy_raw_ptr_from_value(receiver).is_some()
                && let Some(method) = self.load_cpython_proxy_attr_for_value(receiver, method_name)
            {
                if trace && method_name == "__exit__" {
                    eprintln!("[load-special] proxy attr hit");
                }
                return Ok(Some(method));
            }
            if trace && method_name == "__exit__" {
                eprintln!("[load-special] miss");
            }
            return Ok(None);
        };
        if trace && method_name == "__exit__" {
            eprintln!("[load-special] class attr hit");
        }
        let bound = self.bind_descriptor_method(method.clone(), receiver)?;
        if let Some(bound) = bound {
            Ok(Some(bound))
        } else {
            Ok(Some(method))
        }
    }

    fn current_frame_annotation_locals(&self) -> Option<HashMap<String, Value>> {
        self.frames.last().and_then(|frame| {
            if frame.is_module && !frame.return_class {
                return None;
            }
            let mut map = frame.locals.clone();
            if let Some(fallback_locals) = &frame.locals_fallback {
                for (name, value) in fallback_locals {
                    map.entry(name.clone()).or_insert_with(|| value.clone());
                }
            }
            for (idx, slot) in frame.fast_locals.iter().enumerate() {
                if let Some(value) = slot
                    && let Some(name) = frame.code.names.get(idx)
                {
                    map.insert(name.clone(), value.clone());
                }
            }
            for (idx, name) in frame.code.cellvars.iter().enumerate() {
                if !map.contains_key(name)
                    && let Some(cell) = frame.cells.get(idx)
                    && let Object::Cell(cell_data) = &*cell.kind()
                {
                    map.insert(name.clone(), cell_data.value.clone().unwrap_or(Value::None));
                }
            }
            let cell_offset = frame.code.cellvars.len();
            for (idx, name) in frame.code.freevars.iter().enumerate() {
                if !map.contains_key(name)
                    && let Some(cell) = frame.cells.get(cell_offset + idx)
                    && let Object::Cell(cell_data) = &*cell.kind()
                {
                    map.insert(name.clone(), cell_data.value.clone().unwrap_or(Value::None));
                }
            }
            if map.is_empty() { None } else { Some(map) }
        })
    }

    fn import_star_entries_from_module(
        &mut self,
        module_obj: &ObjRef,
    ) -> Result<Vec<(String, Value)>, RuntimeError> {
        let Object::Module(module_data) = &*module_obj.kind() else {
            return Err(RuntimeError::new("import from expects module object"));
        };

        let mut explicit_all = false;
        let export_names: Vec<String> = if let Some(all_names) = module_data.globals.get("__all__")
        {
            explicit_all = true;
            match all_names {
                Value::List(obj) => {
                    let Object::List(values) = &*obj.kind() else {
                        return Err(RuntimeError::new("__all__ must be a sequence of strings"));
                    };
                    let mut names = Vec::with_capacity(values.len());
                    for value in values {
                        match value {
                            Value::Str(name) => names.push(name.clone()),
                            _ => {
                                return Err(RuntimeError::new("__all__ must contain only strings"));
                            }
                        }
                    }
                    names
                }
                Value::Tuple(obj) => {
                    let Object::Tuple(values) = &*obj.kind() else {
                        return Err(RuntimeError::new("__all__ must be a sequence of strings"));
                    };
                    let mut names = Vec::with_capacity(values.len());
                    for value in values {
                        match value {
                            Value::Str(name) => names.push(name.clone()),
                            _ => {
                                return Err(RuntimeError::new("__all__ must contain only strings"));
                            }
                        }
                    }
                    names
                }
                _ => return Err(RuntimeError::new("__all__ must be a sequence of strings")),
            }
        } else {
            module_data
                .globals
                .keys()
                .filter(|name| !name.starts_with('_'))
                .cloned()
                .collect()
        };

        if explicit_all {
            let mut values = Vec::with_capacity(export_names.len());
            for name in export_names {
                // CPython resolves each `__all__` name via attribute lookup,
                // which propagates missing-name errors instead of silently
                // skipping missing keys in module globals.
                let value = self.load_attr_module(module_obj, &name)?;
                values.push((name, value));
            }
            return Ok(values);
        }

        let Object::Module(module_data) = &*module_obj.kind() else {
            return Err(RuntimeError::new("import from expects module object"));
        };
        let mut values = Vec::with_capacity(export_names.len());
        for name in export_names {
            if let Some(value) = module_data.globals.get(&name).cloned() {
                values.push((name, value));
            }
        }
        Ok(values)
    }

    fn import_from_resolve_attr(
        &mut self,
        module_obj: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let requested_module_name = match &*module_obj.kind() {
            Object::Module(module_data) => module_data.name.clone(),
            _ => {
                return Err(RuntimeError::new("import from expects module object"));
            }
        };
        let mut current_module = module_obj.clone();
        let mut retried_with_canonical = false;
        loop {
            if self.trace_flags.numpy_core_importfrom
                && requested_module_name.starts_with("numpy._core")
            {
                let current_has_attr = match &*current_module.kind() {
                    Object::Module(module_data) => module_data.globals.contains_key(attr_name),
                    _ => false,
                };
                let cache_state = self
                    .modules
                    .get(&requested_module_name)
                    .map(|cached| {
                        let has_attr = match &*cached.kind() {
                            Object::Module(module_data) => {
                                module_data.globals.contains_key(attr_name)
                            }
                            _ => false,
                        };
                        format!("{}:{}", cached.id(), has_attr)
                    })
                    .unwrap_or_else(|| "-".to_string());
                let sys_state = self
                    .sys_dict_obj("modules")
                    .and_then(|modules_dict| {
                        dict_get_value(&modules_dict, &Value::Str(requested_module_name.clone()))
                    })
                    .and_then(|value| match value {
                        Value::Module(module) => {
                            let has_attr = match &*module.kind() {
                                Object::Module(module_data) => {
                                    module_data.globals.contains_key(attr_name)
                                }
                                _ => false,
                            };
                            Some(format!("{}:{}", module.id(), has_attr))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| "-".to_string());
                eprintln!(
                    "[numpy-core-importfrom] attr={} module={} current={} current_has={} cache={} sys={}",
                    attr_name,
                    requested_module_name,
                    current_module.id(),
                    current_has_attr,
                    cache_state,
                    sys_state
                );
            }
            let attr = match self.load_attr_module(&current_module, attr_name) {
                Ok(attr) => attr,
                Err(load_err) => {
                    if let Some(module) = self.load_submodule_with_policy(
                        &current_module,
                        attr_name,
                        ImportReturnPolicy::DeferredWhenFramesQueued,
                    )? {
                        Value::Module(module)
                    } else if !retried_with_canonical
                        && load_err.message.contains("has no attribute")
                    {
                        let canonical = self.canonical_imported_module_for_name(
                            &requested_module_name,
                            current_module.clone(),
                        );
                        if canonical.id() != current_module.id() {
                            current_module = canonical;
                            retried_with_canonical = true;
                            continue;
                        }
                        return Err(RuntimeError::new(format!(
                            "cannot import name '{}' from '{}'",
                            attr_name, requested_module_name
                        )));
                    } else if load_err.message.contains("has no attribute") {
                        return Err(RuntimeError::new(format!(
                            "cannot import name '{}' from '{}'",
                            attr_name, requested_module_name
                        )));
                    } else {
                        return Err(load_err);
                    }
                }
            };
            return Ok(attr);
        }
    }

    fn simple_fromlist_names(&self, fromlist: &Value) -> Option<Vec<String>> {
        let mut names = Vec::new();
        match fromlist {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => {
                    names.reserve(values.len());
                    for value in values {
                        let Value::Str(name) = value else {
                            return None;
                        };
                        if name == "*" {
                            return None;
                        }
                        names.push(name.clone());
                    }
                }
                _ => return None,
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => {
                    names.reserve(values.len());
                    for value in values {
                        let Value::Str(name) = value else {
                            return None;
                        };
                        if name == "*" {
                            return None;
                        }
                        names.push(name.clone());
                    }
                }
                _ => return None,
            },
            _ => return None,
        }
        Some(names)
    }

    fn defer_dotted_import_until_parent_ready(
        &mut self,
        name: &str,
        caller_idx: usize,
    ) -> Result<bool, RuntimeError> {
        if !self.should_defer_running_import_completion() {
            return Ok(false);
        }
        let Some((parent, _)) = name.rsplit_once('.') else {
            return Ok(false);
        };
        let parent_needs_load = self
            .modules
            .get(parent)
            .cloned()
            .is_none_or(|parent_module| self.module_requires_realization(parent, &parent_module));
        if !parent_needs_load {
            return Ok(false);
        }
        let frames_before = self.frames.len();
        let _ = self.import_module_object_with_policy(
            parent,
            ImportReturnPolicy::DeferredWhenFramesQueued,
        )?;
        if self.frames.len() <= frames_before {
            return Ok(false);
        }
        if let Some(frame) = self.frames.get_mut(caller_idx) {
            frame.ip = frame.last_ip;
        }
        Ok(true)
    }

    fn bind_import_star_entries_to_caller(
        &mut self,
        caller_idx: usize,
        entries: Vec<(String, Value)>,
    ) -> Result<(), RuntimeError> {
        let frame = if let Some(frame) = self.frames.get_mut(caller_idx) {
            frame
        } else if let Some(frame) = self.frames.last_mut() {
            frame
        } else {
            return Err(RuntimeError::new("import caller frame missing"));
        };

        let mut touched_globals_version = None;
        for (name, value) in entries {
            if let Some(slot_idx) = frame.code.name_to_index.get(&name).copied() {
                if let Some(slot) = frame.fast_locals.get_mut(slot_idx) {
                    Self::write_fast_local_slot(slot, value.clone());
                }
                if let Some(existing) = frame.locals.get_mut(&name) {
                    *existing = value.clone();
                }
            } else {
                frame.locals.insert(name.clone(), value.clone());
            }
            if frame.is_module
                && let Some(dict) = frame.module_locals_dict.clone()
            {
                dict_set_value(&dict, Value::Str(name.clone()), value.clone());
            }
            if let Object::Module(module_data) = &mut *frame.function_globals.kind_mut() {
                module_data.globals.insert(name, value);
                module_data.touch_globals_version();
                touched_globals_version =
                    Some((frame.function_globals.id(), module_data.globals_version));
            }
        }
        if let Some((module_id, version)) = touched_globals_version {
            self.propagate_module_globals_version(module_id, version);
        }
        Ok(())
    }

    fn import_star_into_caller_scope(
        &mut self,
        caller_idx: usize,
        source: Value,
    ) -> Result<(), RuntimeError> {
        let module = match source {
            Value::Module(module) => module,
            _ => {
                return Err(RuntimeError::new(
                    "from-import-* object has no __dict__ and no __all__",
                ));
            }
        };
        let entries = self.import_star_entries_from_module(&module)?;
        self.bind_import_star_entries_to_caller(caller_idx, entries)
    }

    pub(super) fn run(&mut self) -> Result<Value, RuntimeError> {
        self.active_run_depth = self.active_run_depth.saturating_add(1);
        let result = (|| {
            loop {
                if let Some(stop_depth) = self.run_stop_depth
                    && self.frames.len() <= stop_depth
                {
                    return Ok(Value::None);
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
                    if let Some(filter) = self.trace_text_filters.module_return_ip.as_ref()
                        && let Some(frame) = self.frames.last()
                        && frame.is_module
                        && frame.code.filename.contains(filter)
                    {
                        eprintln!(
                            "[module-return] file={} ip={} instr_len={} active_exc={} blocks={}",
                            frame.code.filename,
                            frame.ip,
                            frame.code.instructions.len(),
                            frame.active_exception.is_some(),
                            frame.blocks.len()
                        );
                    }
                    if let Some(frame) = self.frames.last_mut()
                        && frame.is_module
                        && let Some(exc) = frame.active_exception.take()
                    {
                        self.unwind_exception(exc)?;
                        continue;
                    }
                    let mut frame = self.frames.pop().expect("frame exists");
                    if let Some(module_dict) = frame.module_locals_dict.take() {
                        self.sync_module_locals_dict_to_module(&frame.module, &module_dict);
                    }
                    self.finalize_module_frame_success(&frame);
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
                            frame.class_orig_bases,
                            frame.class_metaclass,
                            frame.class_keywords,
                            frame.class_namespace,
                            Some(frame.function_globals.clone()),
                            frame.locals_fallback.clone(),
                            frame.code.future_annotations_import,
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
                if let Some(limit) = self.instruction_step_limit {
                    self.instruction_steps = self.instruction_steps.saturating_add(1);
                    if self.instruction_steps > limit {
                        let frame = self.frames.last().expect("frame exists");
                        return Err(RuntimeError::new(format!(
                            "instruction step limit exceeded at {}:{} in {} ({:?})",
                            frame.code.filename, frame.last_ip, frame.code.name, instr.opcode
                        )));
                    }
                }
                if self.execution_deadline_reached() {
                    let frame = self.frames.last().expect("frame exists");
                    return Err(RuntimeError::new(format!(
                        "execution timeout exceeded at {}:{} in {} ({:?})",
                        frame.code.filename, frame.last_ip, frame.code.name, instr.opcode
                    )));
                }
                let step_result = self.execute_instruction(instr);

                match step_result {
                    Ok(Some(value)) => return Ok(value),
                    Ok(None) => {}
                    Err(err) => match self.handle_runtime_error(err) {
                        Ok(()) => {}
                        Err(err) => return Err(err),
                    },
                }
                let safe_for_gc_auto = self
                    .frames
                    .last()
                    .map(|frame| {
                        frame.code.name != "__del__"
                            && frame.generator_owner.is_none()
                            && self.active_generator_resume.is_none()
                    })
                    .unwrap_or(false);
                if self.gc_auto_collect_enabled
                    && !self.running_pending_del_finalizers
                    && safe_for_gc_auto
                {
                    self.maybe_gc_collect_automatic();
                }
                if !self.pending_del_instances.is_empty() || !self.weakref_finalizers.is_empty() {
                    if self.trace_flags.disable_pending_finalizers {
                        continue;
                    }
                    // Keep __del__ suppressed only while an active exception is being processed.
                    // Refcount-style cleanup in CPython can happen while ordinary operands are live,
                    // and several stdlib paths (tempfile/shutil) rely on that eagerness.
                    let safe_for_pending_finalizers = self
                        .frames
                        .last()
                        .map(|frame| {
                            frame.active_exception.is_none()
                                && frame.code.name != "__del__"
                                && frame.generator_owner.is_none()
                                && self.active_generator_resume.is_none()
                        })
                        .unwrap_or(false);
                    if safe_for_pending_finalizers {
                        self.run_pending_del_finalizers(false);
                    }
                }
            }
        })();
        self.active_run_depth = self.active_run_depth.saturating_sub(1);
        result
    }

    #[inline]
    fn execute_instruction(&mut self, instr: Instruction) -> Result<Option<Value>, RuntimeError> {
        let _debug_depth_guard =
            DebugDepthGuard::enter_for_vm(self, &DEBUG_EXEC_INSTR_DEPTH, "execute_instruction");
        match instr.opcode {
            Opcode::Send => self.execute_instruction_send(instr),
            Opcode::YieldFrom => self.execute_instruction_yield_from(instr),
            _ => self.execute_instruction_slow(instr),
        }
    }

    #[inline]
    fn execute_instruction_send(
        &mut self,
        instr: Instruction,
    ) -> Result<Option<Value>, RuntimeError> {
        let target = instr
            .arg
            .ok_or_else(|| RuntimeError::new("missing send target"))? as usize;
        let sent = self.pop_value()?;
        let iter = self.pop_value()?;
        if sent == Value::None
            && let Value::Generator(delegate) = &iter
        {
            let (running, closed) = match &*delegate.kind() {
                Object::Generator(state) => (state.running, state.closed),
                _ => (false, false),
            };
            if running {
                return Err(RuntimeError::value_error("generator already executing"));
            }
            if !closed {
                let mut delegated_frame = self
                    .generator_states
                    .remove(&delegate.id())
                    .ok_or_else(|| RuntimeError::new("generator has no suspended frame"))?;
                delegated_frame.generator_resume_value = Some(Value::None);
                delegated_frame.generator_pending_throw = None;
                delegated_frame.generator_resume_kind = Some(GeneratorResumeKind::Next);
                self.set_generator_running(delegate, true)?;
                self.set_generator_started(delegate, true)?;
                {
                    let frame = self.frames.last_mut().expect("frame exists");
                    frame.stack.push(iter.clone());
                    frame.send_delegate_state = Some((delegate.id(), target));
                }
                self.push_frame_checked(delegated_frame)?;
                return Ok(None);
            }
        }
        match self.delegate_yield_from(&iter, sent, None, GeneratorResumeKind::Next)? {
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
        Ok(None)
    }

    #[inline]
    fn execute_instruction_yield_from(
        &mut self,
        _instr: Instruction,
    ) -> Result<Option<Value>, RuntimeError> {
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
                Some(
                    frame
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow (Send source)"))?,
                )
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
            match self.to_iterator_value(source_opt.expect("source present")) {
                Ok(iter) => iter,
                Err(err) => {
                    let exc = self.runtime_error_to_exception_value(err);
                    self.raise_exception(exc)?;
                    return Ok(None);
                }
            }
        };
        if sent == Value::None
            && thrown.is_none()
            && resume_kind == GeneratorResumeKind::Next
            && let Value::Generator(delegate) = &iter
        {
            let (running, closed) = match &*delegate.kind() {
                Object::Generator(state) => (state.running, state.closed),
                _ => (false, false),
            };
            if running {
                return Err(RuntimeError::value_error("generator already executing"));
            }
            if !closed {
                let mut delegated_frame = self
                    .generator_states
                    .remove(&delegate.id())
                    .ok_or_else(|| RuntimeError::new("generator has no suspended frame"))?;
                delegated_frame.generator_resume_value = Some(Value::None);
                delegated_frame.generator_pending_throw = None;
                delegated_frame.generator_resume_kind = Some(GeneratorResumeKind::Next);
                self.set_generator_running(delegate, true)?;
                self.set_generator_started(delegate, true)?;
                {
                    let frame = self.frames.last_mut().expect("frame exists");
                    frame.ip = frame.ip.saturating_sub(1);
                    frame.yield_from_iter = Some(iter.clone());
                    frame.generator_awaiting_resume_value = false;
                    frame.generator_resume_value = None;
                    frame.generator_pending_throw = None;
                    // Remember parent resume intent so child-yield handling can preserve
                    // close/next parity without recursive resume.
                    frame.generator_resume_kind = Some(GeneratorResumeKind::Next);
                }
                self.push_frame_checked(delegated_frame)?;
                return Ok(None);
            }
        }
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
                    return Err(RuntimeError::new("generator ignored GeneratorExit"));
                }
                if self.active_generator_resume == Some(owner_id) {
                    self.generator_resume_outcome = Some(GeneratorResumeOutcome::Yield(value));
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
        Ok(None)
    }

    #[inline]
    fn execute_instruction_slow(
        &mut self,
        instr: Instruction,
    ) -> Result<Option<Value>, RuntimeError> {
        match instr.opcode {
            Opcode::Nop => {}
            Opcode::MakeCell => {
                let idx = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing MAKE_CELL argument"))?
                    as usize;
                let frame = self.frames.last_mut().expect("frame exists");
                let name = frame
                    .code
                    .names
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("name index out of range"))?;
                let cell_idx = frame
                    .code
                    .cellvar_to_index
                    .get(&name)
                    .copied()
                    .ok_or_else(|| RuntimeError::new("MAKE_CELL expected cellvar slot"))?;
                let initial_value = frame
                    .fast_locals
                    .get_mut(idx)
                    .and_then(Option::take)
                    .or_else(|| frame.locals.get(&name).cloned());
                let cell = frame
                    .cells
                    .get(cell_idx)
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("cell index out of range"))?;
                if let Object::Cell(cell_data) = &mut *cell.kind_mut() {
                    if cell_data.value.is_none() {
                        cell_data.value = initial_value;
                    }
                }
                if let Some(slot) = frame.fast_locals.get_mut(idx) {
                    *slot = Some(Value::Cell(cell));
                }
            }
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
                let site_index = self.current_site_index();
                let (value, cacheable, cache_hit, globals_module_id, globals_version) = {
                    let frame = self.frames.last().expect("frame exists");
                    let name = frame
                        .code
                        .names
                        .get(idx)
                        .ok_or_else(|| RuntimeError::new("name index out of range"))?;
                    let globals_module_id = frame.module.id();
                    let globals_version = frame.function_globals_version;
                    let cacheable = frame.is_module
                        && frame.locals.is_empty()
                        && frame.locals_fallback.is_none()
                        && frame.globals_fallback.is_none()
                        && frame.module_locals_dict.is_none();
                    if cacheable {
                        if let Some(Some(cached)) = frame.load_global_inline_cache.get(site_index) {
                            if cached.globals_module_id == globals_module_id
                                && cached.globals_version == globals_version
                                && cached.builtins_version == self.builtins_version
                            {
                                (
                                    cached.value.clone(),
                                    true,
                                    true,
                                    globals_module_id,
                                    globals_version,
                                )
                            } else {
                                (
                                    self.lookup_name_with_index(idx, name)?,
                                    true,
                                    false,
                                    globals_module_id,
                                    globals_version,
                                )
                            }
                        } else {
                            (
                                self.lookup_name_with_index(idx, name)?,
                                true,
                                false,
                                globals_module_id,
                                globals_version,
                            )
                        }
                    } else {
                        (
                            self.lookup_name_with_index(idx, name)?,
                            false,
                            false,
                            globals_module_id,
                            globals_version,
                        )
                    }
                };
                if cacheable
                    && !cache_hit
                    && let Some(frame) = self.frames.last_mut()
                    && let Some(slot) = frame.load_global_inline_cache.get_mut(site_index)
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
                        fused_direct_func: None,
                        fused_direct_func_epoch: 0,
                        fused_direct_code: None,
                        fused_direct_module: None,
                        fused_direct_owner_class: None,
                    });
                }
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
            Opcode::LoadFromDictOrGlobals => {
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
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("name index out of range"))?
                };
                let mapping = self.pop_value()?;
                let key = Value::Str(name.clone());
                let mapping_hit = match mapping {
                    Value::Dict(dict) => dict_get_value(&dict, &key),
                    other => match self.getitem_value(other, key.clone()) {
                        Ok(value) => Some(value),
                        Err(err) if runtime_error_matches_exception(&err, "KeyError") => None,
                        Err(err) => return Err(err),
                    },
                };
                let value = if let Some(value) = mapping_hit {
                    value
                } else if let Some(value) =
                    self.frames
                        .last()
                        .and_then(|frame| match &*frame.function_globals.kind() {
                            Object::Module(module_data) => module_data.globals.get(&name).cloned(),
                            _ => None,
                        })
                {
                    value
                } else if let Some(value) = self.lookup_builtin_global(&name) {
                    value
                } else {
                    return Err(RuntimeError::new(format!("name '{name}' is not defined")));
                };
                self.push_value(value);
            }
            Opcode::LoadFromDictOrDeref => {
                let idx = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing deref argument"))?
                    as usize;
                let name = {
                    let frame = self.frames.last().expect("frame exists");
                    deref_name(&frame.code, idx)
                        .or_else(|| frame.code.cellvars.get(idx).map(String::as_str))
                        .unwrap_or("<cell>")
                        .to_string()
                };
                let mapping = self.pop_value()?;
                let key = Value::Str(name);
                let mapping_hit = match mapping {
                    Value::Dict(dict) => dict_get_value(&dict, &key),
                    other => match self.getitem_value(other, key.clone()) {
                        Ok(value) => Some(value),
                        Err(err) if runtime_error_matches_exception(&err, "KeyError") => None,
                        Err(err) => return Err(err),
                    },
                };
                let value = if let Some(value) = mapping_hit {
                    value
                } else {
                    self.load_deref(idx)?
                };
                self.push_value(value);
            }
            Opcode::LoadFast => {
                let idx = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing local argument"))?
                    as usize;
                if self.trace_flags.fast_cell
                    && let Some(frame) = self.frames.last()
                    && frame.code.name == "deprecated"
                    && frame.code.filename.ends_with("_py_warnings.py")
                    && idx == 1
                {
                    let slot_type = frame
                        .fast_locals
                        .get(idx)
                        .and_then(|slot| slot.as_ref())
                        .map(|value| self.value_type_name_for_error(value))
                        .unwrap_or_else(|| "<unset>".to_string());
                    let local_name = frame
                        .code
                        .names
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| "<out-of-range>".to_string());
                    eprintln!(
                        "[trace-fast-cell] fn={} file={} idx={} name={} slot_type={} fast_len={}",
                        frame.code.name,
                        frame.code.filename,
                        idx,
                        local_name,
                        slot_type,
                        frame.fast_locals.len()
                    );
                }
                #[cfg(not(debug_assertions))]
                {
                    let fast_return = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .code
                            .instructions
                            .get(frame.ip)
                            .map(|next| next.opcode == Opcode::ReturnValue)
                            .unwrap_or(false)
                            && frame.simple_one_arg_no_cells
                            && idx == 0
                            && frame.code.plain_positional_arg0_slot == Some(0)
                            && frame.fast_locals.len() == 1
                            && frame.blocks.is_empty()
                            && frame.stack.is_empty()
                            && frame.active_exception.is_none()
                            && !frame.discard_result
                            && frame.generator_owner.is_none()
                    };
                    if fast_return {
                        let mut frame = self.frames.pop().expect("frame exists");
                        let value = frame
                            .fast_locals
                            .get_mut(0)
                            .and_then(Option::take)
                            .unwrap_or(Value::None);
                        if let Some(caller) = self.frames.last_mut() {
                            caller.stack.push(value);
                            if frame.owner_class.is_none() {
                                self.recycle_simple_frame_clean_slot0_unchecked(frame);
                            } else {
                                self.recycle_simple_frame(frame);
                            }
                            return Ok(None);
                        }
                        if frame.owner_class.is_none() {
                            self.recycle_simple_frame_clean_slot0_unchecked(frame);
                        } else {
                            self.recycle_simple_frame(frame);
                        }
                        return Ok(Some(value));
                    }
                }
                #[cfg(not(debug_assertions))]
                let mut fused_compare_jump = false;
                #[cfg(not(debug_assertions))]
                let mut plain_site_load_handled = false;
                #[cfg(not(debug_assertions))]
                {
                    let site_index = self.current_site_index();
                    let site_kind = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .quickened_sites
                            .get(site_index)
                            .copied()
                            .unwrap_or(QuickenedSiteKind::None)
                    };

                    let value_to_i64 = |value: &Value| match value {
                        Value::Int(integer) => Some(*integer),
                        Value::Bool(flag) => Some(if *flag { 1 } else { 0 }),
                        _ => None,
                    };

                    if site_kind == QuickenedSiteKind::LoadFastCompareLtConstJump {
                        let mut clear_cached_site = false;
                        let fused = {
                            let frame = self.frames.last().expect("frame exists");
                            if idx >= frame.fast_locals.len() {
                                clear_cached_site = true;
                                None
                            } else if let Some(left) = frame.fast_locals[idx].as_ref() {
                                if let Some(left_int) = value_to_i64(left) {
                                    if let Some(Some(cache)) =
                                        frame.load_fast_inline_cache.get(site_index)
                                    {
                                        Some((left_int < cache.compare_rhs_int, cache.jump_target))
                                    } else {
                                        clear_cached_site = true;
                                        None
                                    }
                                } else {
                                    clear_cached_site = true;
                                    None
                                }
                            } else {
                                clear_cached_site = true;
                                None
                            }
                        };
                        if clear_cached_site {
                            if let Some(frame) = self.frames.last_mut() {
                                if let Some(slot) = frame.load_fast_inline_cache.get_mut(site_index)
                                {
                                    *slot = None;
                                }
                            }
                            self.mark_quickened_site(site_index, QuickenedSiteKind::LoadFastPlain);
                        }
                        if let Some((truthy, target)) = fused {
                            let frame = self.frames.last_mut().expect("frame exists");
                            if truthy {
                                frame.ip += 2;
                            } else {
                                frame.ip = target;
                            }
                            fused_compare_jump = true;
                        }
                    } else if site_kind == QuickenedSiteKind::LoadFastPlain {
                        let fast_hit = {
                            let frame = self.frames.last_mut().expect("frame exists");
                            if idx < frame.fast_locals.len() {
                                if let Some(value) = &frame.fast_locals[idx] {
                                    frame.stack.push(Self::clone_fast_local_stack_value(value));
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
                        plain_site_load_handled = true;
                    } else {
                        let mut cache_to_store: Option<LoadFastSiteCacheEntry> = None;
                        let mut mark_plain = false;
                        let fused = {
                            let frame = self.frames.last().expect("frame exists");
                            if idx >= frame.fast_locals.len() {
                                None
                            } else if let Some(left) = frame.fast_locals[idx].as_ref() {
                                let left_int = value_to_i64(left);
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
                                                if let (Some(left_int), Some(right_int)) = (
                                                    left_int,
                                                    value_to_i64(&frame.code.constants[const_idx]),
                                                ) {
                                                    cache_to_store = Some(LoadFastSiteCacheEntry {
                                                        compare_rhs_int: right_int,
                                                        jump_target: target as usize,
                                                    });
                                                    Some((left_int < right_int, target as usize))
                                                } else {
                                                    mark_plain = true;
                                                    None
                                                }
                                            } else {
                                                mark_plain = true;
                                                None
                                            }
                                        } else {
                                            mark_plain = true;
                                            None
                                        }
                                    } else {
                                        mark_plain = true;
                                        None
                                    }
                                } else {
                                    mark_plain = true;
                                    None
                                }
                            } else {
                                None
                            }
                        };
                        if let Some(cache) = cache_to_store {
                            if let Some(frame) = self.frames.last_mut() {
                                if let Some(slot) = frame.load_fast_inline_cache.get_mut(site_index)
                                {
                                    *slot = Some(cache);
                                }
                            }
                            self.mark_quickened_site(
                                site_index,
                                QuickenedSiteKind::LoadFastCompareLtConstJump,
                            );
                        } else if mark_plain {
                            self.mark_quickened_site(site_index, QuickenedSiteKind::LoadFastPlain);
                        }
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
                }
                #[cfg(not(debug_assertions))]
                if !fused_compare_jump && !plain_site_load_handled {
                    let fast_hit = {
                        let frame = self.frames.last_mut().expect("frame exists");
                        if idx < frame.fast_locals.len() {
                            if let Some(value) = &frame.fast_locals[idx] {
                                frame.stack.push(Self::clone_fast_local_stack_value(value));
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
                                frame.stack.push(Self::clone_fast_local_stack_value(value));
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
                        frame.fast_locals[first]
                            .as_ref()
                            .map(Self::clone_fast_local_stack_value)
                    } else {
                        None
                    };
                    let second_value = if second < frame.fast_locals.len() {
                        frame.fast_locals[second]
                            .as_ref()
                            .map(Self::clone_fast_local_stack_value)
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
                let maybe_value = {
                    let frame = self.frames.last_mut().expect("frame exists");
                    let slot = frame
                        .fast_locals
                        .get_mut(idx)
                        .ok_or_else(|| RuntimeError::new("name index out of range"))?;
                    slot.take()
                };
                let value = maybe_value.unwrap_or_else(|| self.fast_local_unbound_marker.clone());
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
                let mut fused_direct_cached_func: Option<ObjRef> = None;
                #[cfg(not(debug_assertions))]
                let mut fused_direct_cached_epoch: u64 = 0;
                #[cfg(not(debug_assertions))]
                let mut fused_direct_cached_code: Option<Rc<CodeObject>> = None;
                #[cfg(not(debug_assertions))]
                let mut fused_direct_cached_module: Option<ObjRef> = None;
                #[cfg(not(debug_assertions))]
                let mut fused_direct_cached_owner_class: Option<ObjRef> = None;
                #[cfg(not(debug_assertions))]
                let mut fused_const_small_int: Option<i64> = None;
                #[cfg(not(debug_assertions))]
                let mut cached_direct_local_idx = 0usize;
                #[cfg(not(debug_assertions))]
                let mut cached_direct_const_idx = 0usize;
                #[cfg(not(debug_assertions))]
                let mut cached_direct_small_int: Option<i64> = None;
                #[cfg(not(debug_assertions))]
                let mut cached_direct_code: Option<Rc<CodeObject>> = None;
                #[cfg(not(debug_assertions))]
                let mut cached_direct_module: Option<ObjRef> = None;
                #[cfg(not(debug_assertions))]
                let mut cached_direct_owner_class: Option<ObjRef> = None;
                if let Some(frame) = self.frames.last()
                    && let Some(entry) = frame.load_global_inline_cache.get(site_index)
                    && let Some(cached) = entry
                    && cached.globals_module_id == globals_module_id
                    && cached.globals_version == globals_version
                    && cached.builtins_version == self.builtins_version
                {
                    #[cfg(not(debug_assertions))]
                    {
                        if !push_null && cached.fused_direct_one_arg_no_cells {
                            if let (
                                Some(local_idx),
                                Some(const_idx),
                                Some(func),
                                Some(code),
                                Some(module),
                            ) = (
                                cached.fused_local_idx,
                                cached.fused_const_idx,
                                cached.fused_direct_func.as_ref(),
                                cached.fused_direct_code.as_ref(),
                                cached.fused_direct_module.as_ref(),
                            ) {
                                let direct_ok = {
                                    let func_kind = func.kind();
                                    match &*func_kind {
                                        Object::Function(func_data) => {
                                            func_data.call_cache_epoch
                                                == cached.fused_direct_func_epoch
                                        }
                                        _ => false,
                                    }
                                };
                                if direct_ok {
                                    cached_direct_local_idx = local_idx as usize;
                                    cached_direct_const_idx = const_idx as usize;
                                    cached_direct_small_int = cached.fused_const_small_int;
                                    cached_direct_code = Some(code.clone());
                                    cached_direct_module = Some(module.clone());
                                    cached_direct_owner_class =
                                        cached.fused_direct_owner_class.clone();
                                }
                            }
                        }
                        if cached_direct_code.is_none() {
                            value = Some(cached.value.clone());
                            if let (Some(local_idx), Some(const_idx)) =
                                (cached.fused_local_idx, cached.fused_const_idx)
                            {
                                fused_candidate = Some((local_idx as usize, const_idx as usize));
                            }
                            fused_direct_one_arg_no_cells = cached.fused_direct_one_arg_no_cells;
                            fused_const_small_int = cached.fused_const_small_int;
                            if let Some(func) = cached.fused_direct_func.as_ref() {
                                fused_direct_cached_func = Some(func.clone());
                                fused_direct_cached_epoch = cached.fused_direct_func_epoch;
                            }
                            fused_direct_cached_code = cached.fused_direct_code.clone();
                            fused_direct_cached_module = cached.fused_direct_module.clone();
                            fused_direct_cached_owner_class =
                                cached.fused_direct_owner_class.clone();
                        }
                    }
                    #[cfg(debug_assertions)]
                    {
                        value = Some(cached.value.clone());
                    }
                }
                #[cfg(not(debug_assertions))]
                if let (Some(code), Some(module)) = (cached_direct_code, cached_direct_module) {
                    let arg = if let Some(right_int) = cached_direct_small_int {
                        let left_small = {
                            let frame = self.frames.last().expect("frame exists");
                            frame
                                .fast_locals
                                .get(cached_direct_local_idx)
                                .and_then(Option::as_ref)
                                .and_then(|value| match value {
                                    Value::Int(integer) => Some(*integer),
                                    Value::Bool(flag) => Some(if *flag { 1 } else { 0 }),
                                    _ => None,
                                })
                        };
                        if let Some(left_int) = left_small {
                            match left_int.checked_sub(right_int) {
                                Some(diff) => Value::Int(diff),
                                None => self.binary_sub_runtime(
                                    Value::Int(left_int),
                                    Value::Int(right_int),
                                )?,
                            }
                        } else {
                            self.fused_fast_local_sub_small_int_arg(
                                cached_direct_local_idx,
                                right_int,
                            )?
                        }
                    } else {
                        self.fused_fast_local_sub_const_arg(
                            cached_direct_local_idx,
                            cached_direct_const_idx,
                        )?
                    };
                    {
                        let caller = self.frames.last_mut().expect("frame exists");
                        caller.ip += 3;
                    }
                    self.push_simple_positional_function_frame_one_arg_no_cells_cached_ref(
                        &code,
                        &module,
                        cached_direct_owner_class.as_ref(),
                        arg,
                    )?;
                    return Ok(None);
                }
                #[cfg(not(debug_assertions))]
                let value = if let Some(value) = value {
                    value
                } else {
                    if !push_null {
                        fused_candidate = self.fused_global_fast_sub_call_one_arg_pattern();
                    }
                    let (value, cacheable, globals_module_id, globals_version) =
                        self.resolve_load_global_value(idx)?;
                    if cacheable {
                        if fused_candidate.is_some() {
                            if let Some(metadata) =
                                self.fused_direct_one_arg_no_cells_metadata(&value)
                            {
                                fused_direct_one_arg_no_cells = true;
                                fused_direct_cached_func = Some(metadata.func);
                                fused_direct_cached_epoch = metadata.func_epoch;
                                fused_direct_cached_code = Some(metadata.code);
                                fused_direct_cached_module = Some(metadata.module);
                                fused_direct_cached_owner_class = metadata.owner_class;
                            } else {
                                fused_direct_one_arg_no_cells = false;
                                fused_direct_cached_func = None;
                                fused_direct_cached_epoch = 0;
                                fused_direct_cached_code = None;
                                fused_direct_cached_module = None;
                                fused_direct_cached_owner_class = None;
                            }
                            fused_const_small_int = fused_candidate.and_then(|(_, const_idx)| {
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
                            if let Some(slot) = frame.load_global_inline_cache.get_mut(site_index) {
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
                                    fused_direct_func: fused_direct_cached_func.clone(),
                                    fused_direct_func_epoch: fused_direct_cached_epoch,
                                    fused_direct_code: fused_direct_cached_code.clone(),
                                    fused_direct_module: fused_direct_cached_module.clone(),
                                    fused_direct_owner_class: fused_direct_cached_owner_class
                                        .clone(),
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
                    if cacheable
                        && let Some(frame) = self.frames.last_mut()
                        && let Some(slot) = frame.load_global_inline_cache.get_mut(site_index)
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
                            fused_direct_func: None,
                            fused_direct_func_epoch: 0,
                            fused_direct_code: None,
                            fused_direct_module: None,
                            fused_direct_owner_class: None,
                        });
                    }
                    value
                };
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
                                            Value::Bool(flag) => Some(if *flag { 1 } else { 0 }),
                                            _ => None,
                                        })
                                };
                                if let Some(left_int) = left_small {
                                    match left_int.checked_sub(right_int) {
                                        Some(diff) => Value::Int(diff),
                                        None => self.binary_sub_runtime(
                                            Value::Int(left_int),
                                            Value::Int(right_int),
                                        )?,
                                    }
                                } else {
                                    self.fused_fast_local_sub_small_int_arg(local_idx, right_int)?
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
                                    self.push_simple_positional_function_frame_one_arg_no_cells_from_func(
                                        func_obj, arg,
                                    )?;
                                } else {
                                    self.push_function_call_one_arg_from_obj(func_obj, arg)?;
                                }
                                return Ok(None);
                            }
                        }
                    }
                }
                self.push_value(value);
                if push_null {
                    self.push_value(Value::None);
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
                let site_index = self.current_site_index();
                let value = self.pop_value()?;
                if self.trace_flags.startswith_attr
                    && attr_name == "startswith"
                    && let Some(frame) = self.frames.last()
                {
                    let location = frame.code.locations.get(frame.last_ip);
                    eprintln!(
                        "[startswith-attr] file={} fn={} line={} col={} value_type={} value={}",
                        frame.code.filename,
                        frame.code.name,
                        location.map(|loc| loc.line).unwrap_or(0),
                        location.map(|loc| loc.column).unwrap_or(0),
                        self.value_type_name_for_error(&value),
                        format_repr(&value)
                    );
                }
                let attr = if attr_name == "__doc__"
                    && !matches!(
                        value,
                        Value::Module(_)
                            | Value::Class(_)
                            | Value::Instance(_)
                            | Value::Super(_)
                            | Value::Builtin(_)
                            | Value::Function(_)
                            | Value::BoundMethod(_)
                            | Value::Code(_)
                            | Value::Exception(_)
                            | Value::ExceptionType(_)
                    ) {
                    self.load_runtime_value_doc_attr(&value)?
                } else if attr_name == "__class__" {
                    self.load_dunder_class_attr(&value)?
                } else {
                    match value {
                        Value::Module(module) => self.load_attr_module(&module, &attr_name)?,
                        Value::Class(class) => match self.load_attr_class(&class, &attr_name)? {
                            AttrAccessOutcome::Value(attr) => attr,
                            AttrAccessOutcome::ExceptionHandled => {
                                return Err(self.runtime_error_from_active_exception(
                                    "attribute access failed",
                                ));
                            }
                        },
                        Value::Instance(instance) => {
                            if let Some(cached_attr) = self.try_load_attr_instance_site_cache(
                                site_index, &instance, &attr_name,
                            )? {
                                cached_attr
                            } else {
                                let (loaded_outcome, site_cache_entry) =
                                    self.load_attr_instance_with_site_cache(&instance, &attr_name)?;
                                let loaded = match loaded_outcome {
                                    AttrAccessOutcome::Value(attr) => attr,
                                    AttrAccessOutcome::ExceptionHandled => {
                                        return Err(self.runtime_error_from_active_exception(
                                            "attribute access failed",
                                        ));
                                    }
                                };
                                if let Some(entry) = site_cache_entry {
                                    self.insert_load_attr_instance_site_cache_entry(
                                        site_index, entry,
                                    );
                                } else {
                                    self.clear_load_attr_site_cache(site_index);
                                }
                                loaded
                            }
                        }
                        Value::Super(super_obj) => {
                            match self.load_attr_super(&super_obj, &attr_name)? {
                                AttrAccessOutcome::Value(attr) => attr,
                                AttrAccessOutcome::ExceptionHandled => {
                                    return Err(self.runtime_error_from_active_exception(
                                        "attribute access failed",
                                    ));
                                }
                            }
                        }
                        Value::List(list) => self.load_attr_list_method(list, &attr_name)?,
                        Value::Tuple(tuple) => self.load_attr_tuple_method(tuple, &attr_name)?,
                        Value::Int(value) => {
                            self.load_attr_int_method(Value::Int(value), &attr_name)?
                        }
                        Value::BigInt(value) => {
                            self.load_attr_int_method(Value::BigInt(value), &attr_name)?
                        }
                        Value::Bool(value) => {
                            self.load_attr_int_method(Value::Bool(value), &attr_name)?
                        }
                        Value::Float(value) => self.load_attr_float_method(value, &attr_name)?,
                        Value::Complex { real, imag } => match attr_name.as_str() {
                            "__reduce_ex__" | "__reduce__" => {
                                let wrapper = match self.heap.alloc_module(ModuleObject::new(
                                    "__complex_reduce_ex__".to_string(),
                                )) {
                                    Value::Module(obj) => obj,
                                    _ => unreachable!(),
                                };
                                if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                                    module_data
                                        .globals
                                        .insert("value".to_string(), Value::Complex { real, imag });
                                }
                                self.alloc_native_bound_method(
                                    NativeMethodKind::ComplexReduceEx,
                                    wrapper,
                                )
                            }
                            "real" => Value::Float(real),
                            "imag" => Value::Float(imag),
                            _ => {
                                return Err(RuntimeError::attribute_error(format!(
                                    "complex has no attribute '{}'",
                                    attr_name
                                )));
                            }
                        },
                        Value::Str(text) => self.load_attr_str_method(text, &attr_name)?,
                        Value::Bytes(obj) => {
                            let is_bytes = matches!(&*obj.kind(), Object::Bytes(_));
                            if !is_bytes {
                                return Err(RuntimeError::attribute_error(format!(
                                    "{} has no attribute '{}'",
                                    self.value_type_name_for_error(&Value::Bytes(obj)),
                                    attr_name
                                )));
                            }
                            self.load_attr_bytes_method(Value::Bytes(obj), &attr_name)?
                        }
                        Value::ByteArray(obj) => {
                            let is_bytearray = matches!(&*obj.kind(), Object::ByteArray(_));
                            if !is_bytearray {
                                return Err(RuntimeError::attribute_error(format!(
                                    "{} has no attribute '{}'",
                                    self.value_type_name_for_error(&Value::ByteArray(obj)),
                                    attr_name
                                )));
                            }
                            self.load_attr_bytes_method(Value::ByteArray(obj), &attr_name)?
                        }
                        Value::Iterator(iterator) => {
                            self.load_attr_iterator(iterator, &attr_name)?
                        }
                        Value::MemoryView(view) => self.load_attr_memoryview(view, &attr_name)?,
                        Value::Slice(slice) => self.load_attr_slice(&slice, &attr_name)?,
                        Value::Set(set) => self.load_attr_set_method(set, &attr_name)?,
                        Value::FrozenSet(set) => self.load_attr_set_method(set, &attr_name)?,
                        Value::Dict(dict) => self.load_attr_dict_method(dict, &attr_name)?,
                        Value::DictKeys(view)
                        | Value::DictValues(view)
                        | Value::DictItems(view) => self.load_attr_dict_view(view, &attr_name)?,
                        Value::Cell(cell) => self.load_attr_cell(cell, &attr_name)?,
                        Value::None => match attr_name.as_str() {
                            "__doc__" => Value::Str("None".to_string()),
                            "__new__" => {
                                let none_type = self
                                    .types_module_class("NoneType")
                                    .unwrap_or_else(|| self.fallback_none_type_class());
                                self.alloc_builtin_bound_method(
                                    BuiltinFunction::ObjectNew,
                                    none_type,
                                )
                            }
                            _ => {
                                return Err(RuntimeError::attribute_error(format!(
                                    "NoneType has no attribute '{}'",
                                    attr_name
                                )));
                            }
                        },
                        Value::Builtin(builtin) => self.load_attr_builtin(builtin, &attr_name)?,
                        Value::Function(func) => self.load_attr_function(&func, &attr_name)?,
                        Value::BoundMethod(method) => {
                            self.load_attr_bound_method(&method, &attr_name)?
                        }
                        Value::Code(code) => self.load_attr_code(&code, &attr_name)?,
                        Value::Generator(generator) => {
                            if let Some(value) =
                                self.load_attr_generator_property(&generator, &attr_name)
                            {
                                value
                            } else {
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
                                                return Err(RuntimeError::attribute_error(
                                                    format!(
                                                        "async_generator has no attribute '{}'",
                                                        attr_name
                                                    ),
                                                ));
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
                                                return Err(RuntimeError::attribute_error(
                                                    format!(
                                                        "coroutine has no attribute '{}'",
                                                        attr_name
                                                    ),
                                                ));
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
                                            return Err(RuntimeError::attribute_error(format!(
                                                "generator has no attribute '{}'",
                                                attr_name
                                            )));
                                        }
                                    },
                                    _ => {
                                        return Err(RuntimeError::attribute_error(format!(
                                            "{} has no attribute '{}'",
                                            self.value_type_name_for_error(&Value::Generator(
                                                generator.clone()
                                            )),
                                            attr_name
                                        )));
                                    }
                                };
                                let native =
                                    self.heap.alloc_native_method(NativeMethodObject::new(kind));
                                let bound = BoundMethod::new(native, generator);
                                self.heap.alloc_bound_method(bound)
                            }
                        }
                        Value::Exception(exception) => {
                            self.load_attr_exception_value(&exception, &attr_name)?
                        }
                        Value::ExceptionType(name) => {
                            self.load_attr_exception_type(&name, &attr_name)?
                        }
                    }
                };
                let frame = self
                    .frames
                    .get_mut(caller_idx)
                    .ok_or_else(|| RuntimeError::new("attribute caller frame missing"))?;
                frame.stack.push(attr);
                if push_null {
                    frame.stack.push(Value::None);
                }
            }
            Opcode::LoadSuperAttr => {
                let raw = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing LOAD_SUPER_ATTR argument"))?
                    as usize;
                let name_idx = raw >> 2;
                let push_null = (raw & 0b01) != 0;
                let attr_name = {
                    let frame = self.frames.last().expect("frame exists");
                    frame
                        .code
                        .names
                        .get(name_idx)
                        .ok_or_else(|| RuntimeError::new("name index out of range"))?
                        .clone()
                };
                let object_value = self.pop_value()?;
                let type_value = self.pop_value()?;
                let super_callable = self.pop_value()?;
                if !matches!(super_callable, Value::Builtin(BuiltinFunction::Super)) {
                    return Err(RuntimeError::type_error(
                        "LOAD_SUPER_ATTR expected builtin super",
                    ));
                }
                let super_value =
                    self.builtin_super(vec![type_value, object_value], HashMap::new())?;
                let attr = match super_value {
                    Value::Super(super_obj) => {
                        match self.load_attr_super(&super_obj, &attr_name)? {
                            AttrAccessOutcome::Value(value) => value,
                            AttrAccessOutcome::ExceptionHandled => {
                                return Err(self.runtime_error_from_active_exception(
                                    "LOAD_SUPER_ATTR attribute lookup failed",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "super() did not return a super object",
                        ));
                    }
                };
                let frame = self.frames.last_mut().expect("frame exists");
                frame.stack.push(attr);
                if push_null {
                    frame.stack.push(Value::None);
                }
            }
            Opcode::LoadSpecial => {
                let oparg = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing LOAD_SPECIAL argument"))?;
                let (method_name, error_suffix) =
                    Self::load_special_spec(oparg).ok_or_else(|| {
                        RuntimeError::new(format!("unsupported LOAD_SPECIAL arg {oparg}"))
                    })?;
                let receiver = self.pop_value()?;
                let maybe_method = self.load_special_method(&receiver, method_name)?;
                let method = maybe_method.ok_or_else(|| {
                    RuntimeError::new(format!(
                        "'{}' {}",
                        self.value_type_name_for_error(&receiver),
                        error_suffix
                    ))
                })?;
                self.push_value(method);
                self.push_value(Value::None);
            }
            Opcode::StoreName => {
                let idx = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing name argument"))?
                    as usize;
                let value = {
                    let frame = self.frames.last_mut().expect("frame exists");
                    frame
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow (StoreName)"))?
                };
                self.store_name_by_index(idx, value)?;
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
                let module_slot_idx = self.frames.last().and_then(|frame| {
                    if frame.is_module {
                        frame.code.name_to_index.get(&name).copied()
                    } else {
                        None
                    }
                });
                let mut touched_module_version: Option<(u64, u64)> = None;
                let mut removed_from_module_locals_dict: Option<ObjRef> = None;
                if let Some(frame) = self.frames.last_mut() {
                    if !frame.is_module {
                        if let Some(slot_idx) = frame.code.name_to_index.get(&name).copied()
                            && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
                        {
                            removed = slot.take().is_some();
                        }
                        if removed {
                            frame.locals.remove(&name);
                        } else {
                            removed = frame.locals.remove(&name).is_some();
                        }
                    }
                    if !removed && let Some(dict) = frame.module_locals_dict.clone() {
                        removed = dict_remove_value(&dict, &Value::Str(name.clone())).is_some();
                        if removed && frame.is_module && !frame.return_class {
                            removed_from_module_locals_dict = Some(dict);
                        }
                    }
                    if !removed {
                        let deref_idx = frame
                            .code
                            .cellvars
                            .iter()
                            .position(|cell| cell == &name)
                            .or_else(|| {
                                frame
                                    .code
                                    .freevars
                                    .iter()
                                    .position(|free| free == &name)
                                    .map(|idx| frame.code.cellvars.len() + idx)
                            });
                        if let Some(idx) = deref_idx
                            && let Some(cell) = frame.cells.get(idx).cloned()
                            && let Object::Cell(cell_data) = &mut *cell.kind_mut()
                        {
                            removed = cell_data.value.take().is_some();
                        }
                    }
                    if !removed && let Object::Module(module_data) = &mut *frame.module.kind_mut() {
                        removed = module_data.globals.remove(&name).is_some();
                        if removed {
                            module_data.touch_globals_version();
                            touched_module_version =
                                Some((frame.module.id(), module_data.globals_version));
                        }
                    }
                    if frame.is_module
                        && !removed
                        && let Some(slot_idx) = module_slot_idx
                        && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
                    {
                        removed = slot.take().is_some();
                    }
                    if removed
                        && let Some(slot_idx) = module_slot_idx
                        && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
                    {
                        *slot = None;
                    }
                }
                if !removed {
                    return Err(RuntimeError::new(format!("name '{}' is not defined", name)));
                }
                if let Some(dict) = removed_from_module_locals_dict {
                    self.sync_module_global_from_locals_dict_write(
                        &dict,
                        &Value::Str(name.clone()),
                        None,
                    );
                }
                if let Some((module_id, version)) = touched_module_version {
                    self.propagate_module_globals_version(module_id, version);
                }
            }
            Opcode::DeleteFast => {
                let idx = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing local argument"))?
                    as usize;
                let _ = self.take_fast_local(idx)?;
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
                // CPython stack contract for STORE_FAST_STORE_FAST is
                // (value2, value1 --), so TOS (first pop) is stored to
                // the first local, then the next value to the second local.
                let value_for_first = self.pop_value()?;
                let value_for_second = self.pop_value()?;
                self.store_fast_local(first, value_for_first)?;
                self.store_fast_local(second, value_for_second)?;
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
                                ));
                            }
                        }
                    }
                    Value::Class(class) => {
                        let mut handled_by_descriptor = false;
                        let metaclass_descriptor = self
                            .class_of_value(&Value::Class(class.clone()))
                            .and_then(|metaclass| class_attr_lookup(&metaclass, &attr_name));
                        if let Some(descriptor) = metaclass_descriptor {
                            let (_getter, setter, _deleter) = self.descriptor_hooks(&descriptor)?;
                            if let Some(setter) = setter {
                                match self.call_internal(
                                    setter,
                                    vec![Value::Class(class.clone()), value.clone()],
                                    HashMap::new(),
                                )? {
                                    InternalCallOutcome::Value(_) => {
                                        handled_by_descriptor = true;
                                    }
                                    InternalCallOutcome::CallerExceptionHandled => {
                                        return Err(self.runtime_error_from_active_exception(
                                            "attribute assignment failed",
                                        ));
                                    }
                                }
                            }
                            if let Value::Instance(descriptor_instance) = &descriptor
                                && let Object::Instance(instance_data) =
                                    &*descriptor_instance.kind()
                                && let Object::Class(class_data) = &*instance_data.class.kind()
                                && class_data.name == "property"
                            {
                                return Err(RuntimeError::attribute_error("readonly attribute"));
                            }
                        }
                        if !handled_by_descriptor {
                            let (flags, class_name) = match &*class.kind() {
                                Object::Class(class_data) => (
                                    class_data.attrs.get("__flags__").and_then(
                                        |value| match value {
                                            Value::Int(flags) => Some(*flags),
                                            _ => None,
                                        },
                                    ),
                                    class_data.name.clone(),
                                ),
                                _ => (None, "type".to_string()),
                            };
                            let flags = flags.or_else(|| self.cpython_proxy_type_flags(&class));
                            if let Some(flags) = flags
                                && ((flags & PY_TPFLAGS_HEAPTYPE) == 0
                                    || (flags & PY_TPFLAGS_IMMUTABLETYPE) != 0)
                            {
                                return Err(RuntimeError::type_error(format!(
                                    "cannot set attribute '{}' of immutable type '{}'",
                                    attr_name, class_name
                                )));
                            }
                            if attr_name == "__bases__" {
                                self.update_class_bases_attr(&class, value)?;
                            } else {
                                self.attach_owner_class_to_value(&value, &class);
                                if let Object::Class(class_data) = &mut *class.kind_mut() {
                                    class_data.attrs.insert(attr_name.clone(), value);
                                }
                                self.normalize_class_annotations_after_attr_set(&class, &attr_name);
                            }
                            self.touch_class_attr_version(&class);
                        }
                    }
                    Value::Function(func) => self.store_attr_function(&func, attr_name, value)?,
                    Value::BoundMethod(method) => {
                        self.store_attr_bound_method(&method, &attr_name, value)?
                    }
                    Value::Cell(cell) => self.store_attr_cell(&cell, &attr_name, value)?,
                    Value::Exception(mut exception) => {
                        self.store_attr_exception(&mut exception, &attr_name, value)?
                    }
                    Value::Builtin(builtin) => {
                        self.store_attr_builtin(builtin, &attr_name, value)?
                    }
                    _ => {
                        if self.trace_flags.store_attr {
                            if let Some(frame) = self.frames.last() {
                                let location = frame.code.locations.get(frame.last_ip);
                                eprintln!(
                                    "[store-attr] file={} line={} col={}",
                                    frame.code.filename,
                                    location.map(|loc| loc.line).unwrap_or(0),
                                    location.map(|loc| loc.column).unwrap_or(0),
                                );
                            }
                            eprintln!(
                                "[store-attr] unsupported target={} attr={}",
                                self.value_type_name_for_error(&target),
                                attr_name
                            );
                        }
                        return Err(RuntimeError::type_error(
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
                                ));
                            }
                        }
                    }
                    Value::Class(class) => {
                        let mut handled_by_descriptor = false;
                        let metaclass_descriptor = self
                            .class_of_value(&Value::Class(class.clone()))
                            .and_then(|metaclass| class_attr_lookup(&metaclass, &attr_name));
                        if let Some(descriptor) = metaclass_descriptor {
                            let (_getter, setter, _deleter) = self.descriptor_hooks(&descriptor)?;
                            if let Some(setter) = setter {
                                match self.call_internal(
                                    setter,
                                    vec![Value::Class(class.clone()), value.clone()],
                                    HashMap::new(),
                                )? {
                                    InternalCallOutcome::Value(_) => {
                                        handled_by_descriptor = true;
                                    }
                                    InternalCallOutcome::CallerExceptionHandled => {
                                        return Err(self.runtime_error_from_active_exception(
                                            "attribute assignment failed",
                                        ));
                                    }
                                }
                            }
                            if let Value::Instance(descriptor_instance) = &descriptor
                                && let Object::Instance(instance_data) =
                                    &*descriptor_instance.kind()
                                && let Object::Class(class_data) = &*instance_data.class.kind()
                                && class_data.name == "property"
                            {
                                return Err(RuntimeError::attribute_error("readonly attribute"));
                            }
                        }
                        if !handled_by_descriptor {
                            let (flags, class_name) = match &*class.kind() {
                                Object::Class(class_data) => (
                                    class_data.attrs.get("__flags__").and_then(
                                        |value| match value {
                                            Value::Int(flags) => Some(*flags),
                                            _ => None,
                                        },
                                    ),
                                    class_data.name.clone(),
                                ),
                                _ => (None, "type".to_string()),
                            };
                            let flags = flags.or_else(|| self.cpython_proxy_type_flags(&class));
                            if let Some(flags) = flags
                                && ((flags & PY_TPFLAGS_HEAPTYPE) == 0
                                    || (flags & PY_TPFLAGS_IMMUTABLETYPE) != 0)
                            {
                                return Err(RuntimeError::type_error(format!(
                                    "cannot delete attribute '{}' of immutable type '{}'",
                                    attr_name, class_name
                                )));
                            }
                            if attr_name == "__bases__" {
                                self.update_class_bases_attr(&class, value)?;
                            } else {
                                self.attach_owner_class_to_value(&value, &class);
                                if let Object::Class(class_data) = &mut *class.kind_mut() {
                                    class_data.attrs.insert(attr_name.clone(), value);
                                }
                                self.normalize_class_annotations_after_attr_set(&class, &attr_name);
                            }
                            self.touch_class_attr_version(&class);
                        }
                    }
                    Value::Function(func) => self.store_attr_function(&func, attr_name, value)?,
                    Value::BoundMethod(method) => {
                        self.store_attr_bound_method(&method, &attr_name, value)?
                    }
                    Value::Cell(cell) => self.store_attr_cell(&cell, &attr_name, value)?,
                    Value::Exception(mut exception) => {
                        self.store_attr_exception(&mut exception, &attr_name, value)?
                    }
                    Value::Builtin(builtin) => {
                        self.store_attr_builtin(builtin, &attr_name, value)?
                    }
                    _ => {
                        if self.trace_flags.store_attr {
                            if let Some(frame) = self.frames.last() {
                                let location = frame.code.locations.get(frame.last_ip);
                                eprintln!(
                                    "[store-attr-cpython] file={} line={} col={}",
                                    frame.code.filename,
                                    location.map(|loc| loc.line).unwrap_or(0),
                                    location.map(|loc| loc.column).unwrap_or(0),
                                );
                            }
                            eprintln!(
                                "[store-attr-cpython] unsupported target={} attr={}",
                                self.value_type_name_for_error(&target),
                                attr_name
                            );
                        }
                        return Err(RuntimeError::type_error(
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
                        let (flags, class_name) = match &*class.kind() {
                            Object::Class(class_data) => (
                                class_data
                                    .attrs
                                    .get("__flags__")
                                    .and_then(|value| match value {
                                        Value::Int(flags) => Some(*flags),
                                        _ => None,
                                    }),
                                class_data.name.clone(),
                            ),
                            _ => (None, "type".to_string()),
                        };
                        let flags = flags.or_else(|| self.cpython_proxy_type_flags(&class));
                        if let Some(flags) = flags
                            && ((flags & PY_TPFLAGS_HEAPTYPE) == 0
                                || (flags & PY_TPFLAGS_IMMUTABLETYPE) != 0)
                        {
                            return Err(RuntimeError::type_error(format!(
                                "cannot set attribute '{}' of immutable type '{}'",
                                attr_name, class_name
                            )));
                        }
                        if let Object::Class(class_data) = &mut *class.kind_mut()
                            && class_data.attrs.remove(&attr_name).is_none()
                        {
                            return Err(RuntimeError::new(format!(
                                "class attribute '{}' does not exist",
                                attr_name
                            )));
                        }
                        self.touch_class_attr_version(&class);
                    }
                    Value::Instance(instance) => {
                        match self.delete_attr_instance(&instance, &attr_name)? {
                            AttrMutationOutcome::Done => {}
                            AttrMutationOutcome::ExceptionHandled => {
                                return Err(self.runtime_error_from_active_exception(
                                    "attribute deletion failed",
                                ));
                            }
                        }
                    }
                    Value::Function(func) => {
                        self.delete_attr_function(&func, &attr_name)?;
                    }
                    Value::Cell(cell) => {
                        self.delete_attr_cell(&cell, &attr_name)?;
                    }
                    Value::Exception(exception) => {
                        self.delete_attr_exception(&exception, &attr_name)?;
                    }
                    Value::Builtin(builtin) => {
                        self.delete_attr_builtin(builtin, &attr_name)?;
                    }
                    _ => {
                        if self.trace_flags.delete_attr {
                            if let Some(frame) = self.frames.last() {
                                let location = frame.code.locations.get(frame.last_ip);
                                eprintln!(
                                    "[delete-attr] file={} line={} col={} target_type={} attr={}",
                                    frame.code.filename,
                                    location.map(|loc| loc.line).unwrap_or(0),
                                    location.map(|loc| loc.column).unwrap_or(0),
                                    self.value_type_name_for_error(&target),
                                    attr_name
                                );
                            }
                        }
                        return Err(RuntimeError::type_error(
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
                let value = match (left, right) {
                    (Value::Int(a), Value::Int(b)) => match a.checked_add(b) {
                        Some(sum) => Value::Int(sum),
                        None => self.binary_add_runtime(Value::Int(a), Value::Int(b))?,
                    },
                    (left, right) => self.binary_add_runtime(left, right)?,
                };
                #[cfg(not(debug_assertions))]
                let value = match self.try_fast_terminal_return_simple_no_cells(value) {
                    Ok(()) => return Ok(None),
                    Err(value) => value,
                };
                self.frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(value);
            }
            Opcode::InplaceAdd => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let value = self.binary_inplace_add_runtime(left, right)?;
                self.push_value(value);
            }
            Opcode::BinarySub => {
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
                let value = match (left, right) {
                    (Value::Int(a), Value::Int(b)) => match a.checked_sub(b) {
                        Some(diff) => Value::Int(diff),
                        None => self.binary_sub_runtime(Value::Int(a), Value::Int(b))?,
                    },
                    (left, right) => self.binary_sub_runtime(left, right)?,
                };
                #[cfg(not(debug_assertions))]
                let value = match self.try_fast_terminal_return_simple_no_cells(value) {
                    Ok(()) => return Ok(None),
                    Err(value) => value,
                };
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
                    let left = frame
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow (BinarySubConst lhs)"))?;
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
                        None => self.binary_sub_runtime(Value::Int(a), Value::Int(b))?,
                    },
                    (left, Some(b), _) => self.binary_sub_runtime(left, Value::Int(b))?,
                    (left, None, Some(right)) => self.binary_sub_runtime(left, right)?,
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
                let value = self.binary_mul_runtime(left, right)?;
                #[cfg(not(debug_assertions))]
                let value = match self.try_fast_terminal_return_simple_no_cells(value) {
                    Ok(()) => return Ok(None),
                    Err(value) => value,
                };
                self.push_value(value);
            }
            Opcode::BinaryMatMul => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let value = self.binary_matmul_runtime(left, right)?;
                self.push_value(value);
            }
            Opcode::BinaryDiv => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let value = self.binary_div_runtime(left, right)?;
                #[cfg(not(debug_assertions))]
                let value = match self.try_fast_terminal_return_simple_no_cells(value) {
                    Ok(()) => return Ok(None),
                    Err(value) => value,
                };
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
                let value = self.binary_floor_div_runtime(left, right)?;
                #[cfg(not(debug_assertions))]
                let value = match self.try_fast_terminal_return_simple_no_cells(value) {
                    Ok(()) => return Ok(None),
                    Err(value) => value,
                };
                self.push_value(value);
            }
            Opcode::BinaryMod => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let value = self.binary_mod_runtime(left, right)?;
                #[cfg(not(debug_assertions))]
                let value = match self.try_fast_terminal_return_simple_no_cells(value) {
                    Ok(()) => return Ok(None),
                    Err(value) => value,
                };
                self.push_value(value);
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
                let value = self.binary_xor_runtime(left, right)?;
                self.push_value(value);
            }
            Opcode::BinaryOr => {
                let right = self.pop_value()?;
                let left = self.pop_value()?;
                let value = self.binary_or_runtime(left, right)?;
                self.push_value(value);
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
                let jump_target = self.next_jump_if_false_target();
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
                if let Some(target) = jump_target {
                    let (truthy, can_quicken) = if quickened_int {
                        match (left, right) {
                            (Value::Int(a), Value::Int(b)) => (a < b, false),
                            (left, right) => {
                                self.clear_quickened_site(site_index);
                                let result = self.compare_lt_runtime(left, right)?;
                                (self.truthy_from_value(&result)?, false)
                            }
                        }
                    } else {
                        match (left, right) {
                            (Value::Int(a), Value::Int(b)) => (a < b, true),
                            (left, right) => {
                                let result = self.compare_lt_runtime(left, right)?;
                                (self.truthy_from_value(&result)?, false)
                            }
                        }
                    };
                    if can_quicken {
                        self.mark_quickened_site(site_index, QuickenedSiteKind::CompareLtInt);
                    }
                    let frame = self.frames.last_mut().expect("frame exists");
                    if truthy {
                        frame.ip += 1;
                    } else {
                        frame.ip = target;
                    }
                } else {
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
                let jump_target = self.next_jump_if_false_target();
                let (left, right_int, right_value) = {
                    let frame = self.frames.last_mut().expect("frame exists");
                    if idx >= frame.code.constants.len() {
                        return Err(RuntimeError::new("constant index out of range"));
                    }
                    let left = frame
                        .stack
                        .pop()
                        .ok_or_else(|| RuntimeError::new("stack underflow (CompareLtConst lhs)"))?;
                    let (right_int, right_value) = match &frame.code.constants[idx] {
                        Value::Int(value) => (Some(*value), None),
                        Value::Bool(flag) => (Some(if *flag { 1 } else { 0 }), None),
                        value => (None, Some(value.clone())),
                    };
                    (left, right_int, right_value)
                };
                if let Some(target) = jump_target {
                    let truthy = match (left, right_int, right_value) {
                        (Value::Int(a), Some(b), _) => a < b,
                        (left, Some(b), _) => {
                            let result = self.compare_lt_runtime(left, Value::Int(b))?;
                            self.truthy_from_value(&result)?
                        }
                        (left, None, Some(right)) => {
                            let result = self.compare_lt_runtime(left, right)?;
                            self.truthy_from_value(&result)?
                        }
                        (_, None, None) => {
                            return Err(RuntimeError::new("invalid constant for CompareLtConst"));
                        }
                    };
                    let frame = self.frames.last_mut().expect("frame exists");
                    if truthy {
                        frame.ip += 1;
                    } else {
                        frame.ip = target;
                    }
                } else {
                    let result = match (left, right_int, right_value) {
                        (Value::Int(a), Some(b), _) => Value::Bool(a < b),
                        (left, Some(b), _) => self.compare_lt_runtime(left, Value::Int(b))?,
                        (left, None, Some(right)) => self.compare_lt_runtime(left, right)?,
                        (_, None, None) => {
                            return Err(RuntimeError::new("invalid constant for CompareLtConst"));
                        }
                    };
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
                let result = self.unary_neg_runtime(value)?;
                self.push_value(result);
            }
            Opcode::UnaryNot => {
                let value = self.pop_value()?;
                let truthy = self.truthy_from_value(&value)?;
                self.push_value(Value::Bool(!truthy));
            }
            Opcode::UnaryPos => {
                let value = self.pop_value()?;
                let result = self.unary_pos_runtime(value)?;
                self.push_value(result);
            }
            Opcode::UnaryInvert => {
                let value = self.pop_value()?;
                let result = self.unary_invert_runtime(value)?;
                self.push_value(result);
            }
            Opcode::ConvertValue => {
                let value = self.pop_value()?;
                let converted = match instr.arg.unwrap_or(0) {
                    1 => self.builtin_str(vec![value], HashMap::new())?,
                    2 => self.builtin_repr(vec![value], HashMap::new())?,
                    3 => self.builtin_ascii(vec![value], HashMap::new())?,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "unsupported CONVERT_VALUE arg {other}"
                        )));
                    }
                };
                self.push_value(converted);
            }
            Opcode::CallIntrinsic1 => {
                let intrinsic = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing CALL_INTRINSIC_1 argument"))?;
                let value = self.pop_value()?;
                let result = match intrinsic {
                    2 => {
                        let caller_idx = self.frames.len().saturating_sub(1);
                        self.import_star_into_caller_scope(caller_idx, value)?;
                        Value::None
                    }
                    5 => pos_value(value)?,
                    6 => match value {
                        Value::List(list) => match &*list.kind() {
                            Object::List(values) => self.heap.alloc_tuple(values.to_vec()),
                            _ => {
                                return Err(RuntimeError::new(
                                    "INTRINSIC_LIST_TO_TUPLE expects list",
                                ));
                            }
                        },
                        _ => {
                            return Err(RuntimeError::new("INTRINSIC_LIST_TO_TUPLE expects list"));
                        }
                    },
                    3 => self.intrinsic_stopiteration_error(value)?,
                    7 => self.intrinsic_make_typing_param(BuiltinFunction::TypingTypeVar, value)?,
                    8 => {
                        self.intrinsic_make_typing_param(BuiltinFunction::TypingParamSpec, value)?
                    }
                    9 => self
                        .intrinsic_make_typing_param(BuiltinFunction::TypingTypeVarTuple, value)?,
                    10 => self.intrinsic_subscript_generic(value)?,
                    11 => self.intrinsic_make_type_alias(value)?,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "unsupported CALL_INTRINSIC_1 arg {other}"
                        )));
                    }
                };
                self.push_value(result);
            }
            Opcode::CallIntrinsic2 => {
                let intrinsic = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing CALL_INTRINSIC_2 argument"))?;
                let value1 = self.pop_value()?;
                let value2 = self.pop_value()?;
                let result = match intrinsic {
                    1 => value2,
                    2 => self.intrinsic_make_typevar_with_bound(value2, value1)?,
                    3 => self.intrinsic_make_typevar_with_constraints(value2, value1)?,
                    4 => match value2 {
                        Value::Function(function) => {
                            self.store_attr_function(
                                &function,
                                "__type_params__".to_string(),
                                value1,
                            )?;
                            Value::Function(function)
                        }
                        _ => {
                            return Err(RuntimeError::type_error(
                                "INTRINSIC_SET_FUNCTION_TYPE_PARAMS expects function",
                            ));
                        }
                    },
                    5 => self.intrinsic_set_typeparam_default(value2, value1)?,
                    other => {
                        return Err(RuntimeError::new(format!(
                            "unsupported CALL_INTRINSIC_2 arg {other}"
                        )));
                    }
                };
                self.push_value(result);
            }
            Opcode::FormatSimple => {
                let value = self.pop_value()?;
                let rendered = self.builtin_format(vec![value], HashMap::new())?;
                match rendered {
                    Value::Str(_) => self.push_value(rendered),
                    other => {
                        return Err(RuntimeError::new(format!(
                            "format() returned non-string '{}'",
                            self.value_type_name_for_error(&other)
                        )));
                    }
                }
            }
            Opcode::FormatWithSpec => {
                let fmt_spec = self.pop_value()?;
                let value = self.pop_value()?;
                let rendered = self.builtin_format(vec![value, fmt_spec], HashMap::new())?;
                match rendered {
                    Value::Str(_) => self.push_value(rendered),
                    other => {
                        return Err(RuntimeError::new(format!(
                            "format() returned non-string '{}'",
                            self.value_type_name_for_error(&other)
                        )));
                    }
                }
            }
            Opcode::ToBool => {
                let value = self.pop_value()?;
                let truthy = self.truthy_from_value(&value)?;
                self.push_value(Value::Bool(truthy));
            }
            Opcode::GetLen => {
                let value = {
                    let frame = self.frames.last().expect("frame exists");
                    frame
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("GET_LEN stack underflow"))?
                };
                let len = self.builtin_len(vec![value], HashMap::new())?;
                self.push_value(len);
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
            Opcode::BuildSet => {
                let count = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing set size"))?
                    as usize;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(self.pop_value()?);
                }
                values.reverse();
                let deduped = self.dedup_hashable_values_runtime(values)?;
                let set_value = self.heap.alloc_set(deduped);
                self.push_value(set_value);
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
            Opcode::BuildString => {
                let count = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing string build size"))?
                    as usize;
                let mut pieces = Vec::with_capacity(count);
                for _ in 0..count {
                    pieces.push(self.pop_value()?);
                }
                pieces.reverse();
                let mut out = String::new();
                for piece in pieces {
                    match piece {
                        Value::Str(text) => out.push_str(&text),
                        other => {
                            return Err(RuntimeError::new(format!(
                                "BUILD_STRING expects str, got '{}'",
                                self.value_type_name_for_error(&other)
                            )));
                        }
                    }
                }
                self.push_value(Value::Str(out));
            }
            Opcode::BuildInterpolation => {
                let raw = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing BUILD_INTERPOLATION argument"))?;
                let has_format = (raw & 1) != 0;
                let conversion = raw >> 2;
                let format_spec = if has_format {
                    self.pop_value()?
                } else {
                    Value::Str(String::new())
                };
                let expression = self.pop_value()?;
                let value = self.pop_value()?;
                let interpolation = self.build_template_interpolation_value(
                    value,
                    expression,
                    conversion,
                    format_spec,
                )?;
                self.push_value(interpolation);
            }
            Opcode::BuildTemplate => {
                let interpolations = self.pop_value()?;
                let strings = self.pop_value()?;
                let template = self.build_template_value(strings, interpolations)?;
                self.push_value(template);
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
                    values.push((key, value));
                }
                values.reverse();
                let dict = match self.heap.alloc_dict(Vec::new()) {
                    Value::Dict(dict) => dict,
                    _ => unreachable!("heap.alloc_dict must return dict value"),
                };
                for (key, value) in values {
                    self.dict_set_value_checked_runtime(&dict, key, value)?;
                }
                self.push_value(Value::Dict(dict));
            }
            Opcode::UnpackSequence | Opcode::UnpackSequenceCpython => {
                let cpython_unpack = matches!(instr.opcode, Opcode::UnpackSequenceCpython);
                let count = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing unpack size"))?
                    as usize;
                let value = self.pop_value()?;
                match value {
                    Value::List(obj) => {
                        let kind = obj.kind();
                        let Object::List(values) = &*kind else {
                            return Err(RuntimeError::type_error("unpack expects iterable"));
                        };
                        if values.len() < count {
                            return Err(RuntimeError::new(format!(
                                "not enough values to unpack (expected {count}, got {})",
                                values.len()
                            )));
                        }
                        if values.len() > count {
                            return Err(RuntimeError::new(format!(
                                "too many values to unpack (expected {count})"
                            )));
                        }
                        if cpython_unpack {
                            for item in values.iter().rev() {
                                self.push_value(item.clone());
                            }
                        } else {
                            for item in values {
                                self.push_value(item.clone());
                            }
                        }
                    }
                    Value::Tuple(obj) => {
                        let kind = obj.kind();
                        let Object::Tuple(values) = &*kind else {
                            return Err(RuntimeError::type_error("unpack expects iterable"));
                        };
                        if values.len() < count {
                            return Err(RuntimeError::new(format!(
                                "not enough values to unpack (expected {count}, got {})",
                                values.len()
                            )));
                        }
                        if values.len() > count {
                            return Err(RuntimeError::new(format!(
                                "too many values to unpack (expected {count})"
                            )));
                        }
                        if cpython_unpack {
                            for item in values.iter().rev() {
                                self.push_value(item.clone());
                            }
                        } else {
                            for item in values {
                                self.push_value(item.clone());
                            }
                        }
                    }
                    other => {
                        let items = self
                            .collect_iterable_values(other)
                            .map_err(|_| RuntimeError::type_error("unpack expects iterable"))?;
                        if items.len() < count {
                            return Err(RuntimeError::new(format!(
                                "not enough values to unpack (expected {count}, got {})",
                                items.len()
                            )));
                        }
                        if items.len() > count {
                            return Err(RuntimeError::new(format!(
                                "too many values to unpack (expected {count})"
                            )));
                        }
                        if cpython_unpack {
                            for item in items.into_iter().rev() {
                                self.push_value(item);
                            }
                        } else {
                            for item in items {
                                self.push_value(item);
                            }
                        }
                    }
                }
            }
            Opcode::UnpackEx | Opcode::UnpackExCpython => {
                let cpython_unpack = matches!(instr.opcode, Opcode::UnpackExCpython);
                let packed = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing unpack sizes"))?;
                let (before, after) = if cpython_unpack {
                    // CPython oparg packs counts as:
                    //   low byte = mandatory values before star target
                    //   high bytes = mandatory values after star target
                    ((packed & 0xFF) as usize, (packed >> 8) as usize)
                } else {
                    // pyrs source compiler uses a wider local packing.
                    ((packed & 0xFFFF) as usize, (packed >> 16) as usize)
                };
                let value = self.pop_value()?;
                match value {
                    Value::List(obj) => {
                        let kind = obj.kind();
                        let Object::List(values) = &*kind else {
                            return Err(RuntimeError::type_error("unpack expects iterable"));
                        };
                        if values.len() < before + after {
                            return Err(RuntimeError::new(format!(
                                "not enough values to unpack (expected at least {}, got {})",
                                before + after,
                                values.len()
                            )));
                        }
                        let split_after = values.len() - after;
                        if cpython_unpack {
                            for item in values[split_after..].iter().rev() {
                                self.push_value(item.clone());
                            }
                            let middle: Vec<Value> = values[before..split_after].to_vec();
                            self.push_value(self.heap.alloc_list(middle));
                            for item in values[..before].iter().rev() {
                                self.push_value(item.clone());
                            }
                        } else {
                            for item in &values[..before] {
                                self.push_value(item.clone());
                            }
                            let middle: Vec<Value> = values[before..split_after].to_vec();
                            self.push_value(self.heap.alloc_list(middle));
                            for item in &values[split_after..] {
                                self.push_value(item.clone());
                            }
                        }
                    }
                    Value::Tuple(obj) => {
                        let kind = obj.kind();
                        let Object::Tuple(values) = &*kind else {
                            return Err(RuntimeError::type_error("unpack expects iterable"));
                        };
                        if values.len() < before + after {
                            return Err(RuntimeError::new(format!(
                                "not enough values to unpack (expected at least {}, got {})",
                                before + after,
                                values.len()
                            )));
                        }
                        let split_after = values.len() - after;
                        if cpython_unpack {
                            for item in values[split_after..].iter().rev() {
                                self.push_value(item.clone());
                            }
                            let middle: Vec<Value> = values[before..split_after].to_vec();
                            self.push_value(self.heap.alloc_list(middle));
                            for item in values[..before].iter().rev() {
                                self.push_value(item.clone());
                            }
                        } else {
                            for item in &values[..before] {
                                self.push_value(item.clone());
                            }
                            let middle: Vec<Value> = values[before..split_after].to_vec();
                            self.push_value(self.heap.alloc_list(middle));
                            for item in &values[split_after..] {
                                self.push_value(item.clone());
                            }
                        }
                    }
                    other => {
                        let mut items = self
                            .collect_iterable_values(other)
                            .map_err(|_| RuntimeError::type_error("unpack expects iterable"))?;
                        if items.len() < before + after {
                            return Err(RuntimeError::new(format!(
                                "not enough values to unpack (expected at least {}, got {})",
                                before + after,
                                items.len()
                            )));
                        }
                        let trailing = items.split_off(items.len() - after);
                        let middle = items.split_off(before);
                        if cpython_unpack {
                            for item in trailing.into_iter().rev() {
                                self.push_value(item);
                            }
                            self.push_value(self.heap.alloc_list(middle));
                            for item in items.into_iter().rev() {
                                self.push_value(item);
                            }
                        } else {
                            for item in items {
                                self.push_value(item);
                            }
                            self.push_value(self.heap.alloc_list(middle));
                            for item in trailing {
                                self.push_value(item);
                            }
                        }
                    }
                }
            }
            Opcode::ListAppend => {
                let oparg = instr.arg.map(|value| value as usize).unwrap_or(1usize);
                if oparg == 0 {
                    return Err(RuntimeError::new("LIST_APPEND expects oparg >= 1"));
                }
                let value = self.pop_value()?;
                let list = self.stack_list_target(oparg, "LIST_APPEND")?;
                if let Object::List(values) = &mut *list.kind_mut() {
                    values.push(value);
                } else {
                    return Err(RuntimeError::new("list append expects list"));
                }
            }
            Opcode::SetAdd => {
                let oparg = instr.arg.map(|value| value as usize).unwrap_or(1usize);
                if oparg == 0 {
                    return Err(RuntimeError::new("SET_ADD expects oparg >= 1"));
                }
                let value = self.pop_value()?;
                let set = self.stack_set_target(oparg, "SET_ADD")?;
                self.set_insert_checked_runtime(&set, value)?;
            }
            Opcode::ListExtend => {
                let oparg = instr.arg.map(|value| value as usize).unwrap_or(1usize);
                if oparg == 0 {
                    return Err(RuntimeError::new("LIST_EXTEND expects oparg >= 1"));
                }
                let other = self.pop_value()?;
                let list = self.stack_list_target(oparg, "LIST_EXTEND")?;
                let extra = self
                    .collect_iterable_values(other)
                    .map_err(|_| RuntimeError::new("list extend expects iterable"))?;
                if let Object::List(values) = &mut *list.kind_mut() {
                    values.extend(extra);
                } else {
                    return Err(RuntimeError::new("list extend expects list"));
                }
            }
            Opcode::SetUpdate => {
                let oparg = instr.arg.map(|value| value as usize).unwrap_or(1usize);
                if oparg == 0 {
                    return Err(RuntimeError::new("SET_UPDATE expects oparg >= 1"));
                }
                let other = self.pop_value()?;
                let set = self.stack_set_target(oparg, "SET_UPDATE")?;
                let extra = self
                    .collect_iterable_values(other)
                    .map_err(|_| RuntimeError::new("set update expects iterable"))?;
                for item in extra {
                    self.set_insert_checked_runtime(&set, item)?;
                }
            }
            Opcode::DictSet => {
                let value = self.pop_value()?;
                let key = self.pop_value()?;
                let dict = self.pop_value()?;
                match dict {
                    Value::Dict(obj) => {
                        self.dict_set_value_checked_runtime(&obj, key, value)?;
                        self.push_value(Value::Dict(obj));
                    }
                    _ => return Err(RuntimeError::new("dict set expects dict")),
                }
            }
            Opcode::MapAdd => {
                let oparg = instr.arg.map(|value| value as usize).unwrap_or(1usize);
                if oparg == 0 {
                    return Err(RuntimeError::new("MAP_ADD expects oparg >= 1"));
                }
                let value = self.pop_value()?;
                let key = self.pop_value()?;
                let dict = self.stack_dict_target_for_merge(oparg, "MAP_ADD")?;
                self.dict_set_value_checked_runtime(&dict, key, value)?;
            }
            Opcode::DictUpdate => {
                let oparg = instr.arg.map(|value| value as usize).unwrap_or(1usize);
                if oparg == 0 {
                    return Err(RuntimeError::new("DICT_UPDATE expects oparg >= 1"));
                }
                let update = self.pop_value()?;
                self.apply_dict_stack_merge(oparg, update, false, "DICT_UPDATE")?;
            }
            Opcode::DictMerge => {
                let oparg = instr.arg.map(|value| value as usize).unwrap_or(1usize);
                if oparg == 0 {
                    return Err(RuntimeError::new("DICT_MERGE expects oparg >= 1"));
                }
                let update = self.pop_value()?;
                if self.trace_flags.dict_merge {
                    eprintln!(
                        "[dict-merge] oparg={} source={}",
                        oparg,
                        self.value_type_name_for_error(&update)
                    );
                }
                self.apply_dict_stack_merge(oparg, update, true, "DICT_MERGE")?;
            }
            Opcode::BuildSlice => {
                let count = instr.arg.unwrap_or(3);
                let (lower, upper, step) = match count {
                    2 => {
                        let upper = self.pop_value()?;
                        let lower = self.pop_value()?;
                        (lower, upper, Value::None)
                    }
                    3 => {
                        let step = self.pop_value()?;
                        let upper = self.pop_value()?;
                        let lower = self.pop_value()?;
                        (lower, upper, step)
                    }
                    _ => {
                        return Err(RuntimeError::new(format!(
                            "invalid BUILD_SLICE arg {}",
                            count
                        )));
                    }
                };
                let lower = value_to_optional_index(lower)?;
                let upper = value_to_optional_index(upper)?;
                let step = value_to_optional_index(step)?;
                self.push_value(Value::Slice(Box::new(SliceValue::new(lower, upper, step))));
            }
            Opcode::BinarySlice => {
                let upper = self.pop_value()?;
                let lower = self.pop_value()?;
                let target = self.pop_value()?;
                let lower = value_to_optional_index(lower)?;
                let upper = value_to_optional_index(upper)?;
                let slice = Value::Slice(Box::new(SliceValue::new(lower, upper, None)));
                let result = self.getitem_value(target, slice)?;
                self.push_value(result);
            }
            Opcode::Subscript => {
                let index = self.pop_value()?;
                let value = self.pop_value()?;
                let trace_subscript_error = self.trace_flags.subscript_error;
                let trace_value = if trace_subscript_error {
                    Some(value.clone())
                } else {
                    None
                };
                let trace_index = if trace_subscript_error {
                    Some(index.clone())
                } else {
                    None
                };
                if self.trace_flags.subscript
                    && matches!(value, Value::Tuple(_))
                    && matches!(index, Value::Tuple(_))
                {
                    if let Some(frame) = self.frames.last() {
                        let location = frame.code.locations.get(frame.last_ip);
                        eprintln!(
                            "[subscript] file={} line={} col={} value={} index={}",
                            frame.code.filename,
                            location.map(|loc| loc.line).unwrap_or(0),
                            location.map(|loc| loc.column).unwrap_or(0),
                            format_value(&value),
                            format_value(&index)
                        );
                    }
                }
                let result = match self.getitem_value(value, index) {
                    Ok(value) => value,
                    Err(err) => {
                        if trace_subscript_error && let Some(frame) = self.frames.last() {
                            let location = frame.code.locations.get(frame.last_ip);
                            let value_tag = trace_value
                                .as_ref()
                                .map(|v| self.value_type_name_for_error(v))
                                .unwrap_or_else(|| "<missing>".to_string());
                            let index_tag = trace_index
                                .as_ref()
                                .map(|v| self.value_type_name_for_error(v))
                                .unwrap_or_else(|| "<missing>".to_string());
                            let value_repr = trace_value
                                .as_ref()
                                .map(format_repr)
                                .unwrap_or_else(|| "<missing>".to_string());
                            let index_repr = trace_index
                                .as_ref()
                                .map(format_repr)
                                .unwrap_or_else(|| "<missing>".to_string());
                            eprintln!(
                                "[subscript-error] file={} fn={} line={} col={} err={} value_type={} value={} index_type={} index={}",
                                frame.code.filename,
                                frame.code.name,
                                location.map(|loc| loc.line).unwrap_or(0),
                                location.map(|loc| loc.column).unwrap_or(0),
                                err.message,
                                value_tag,
                                value_repr,
                                index_tag,
                                index_repr
                            );
                        }
                        return Err(err);
                    }
                };
                self.push_value(result);
            }
            Opcode::StoreSubscript => {
                let cpython_order = instr.arg == Some(1);
                let discard_result = cpython_order;
                let (value, index, target) = if cpython_order {
                    let index = self.pop_value()?;
                    let target = self.pop_value()?;
                    let value = self.pop_value()?;
                    (value, index, target)
                } else {
                    let value = self.pop_value()?;
                    let index = self.pop_value()?;
                    let target = self.pop_value()?;
                    (value, index, target)
                };
                match target {
                    Value::List(obj) => match index {
                        Value::Slice(slice) => {
                            let lower = slice.lower;
                            let upper = slice.upper;
                            let step = slice.step;
                            let replacement = self
                                .collect_iterable_values(value)
                                .map_err(|_| RuntimeError::new("can only assign an iterable"))?;
                            if let Object::List(values) = &mut *obj.kind_mut() {
                                let step_value = step.unwrap_or(1);
                                if step_value == 1 {
                                    let (start, stop) =
                                        slice_bounds_for_step_one(values.len(), lower, upper);
                                    values.splice(start..stop, replacement);
                                } else {
                                    let indices = slice_indices(values.len(), lower, upper, step)?;
                                    if indices.len() != replacement.len() {
                                        return Err(RuntimeError::new(format!(
                                            "attempt to assign sequence of size {} to extended slice of size {}",
                                            replacement.len(),
                                            indices.len()
                                        )));
                                    }
                                    for (idx, item) in indices.into_iter().zip(replacement) {
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
                                    return Err(RuntimeError::index_error(
                                        "list index out of range",
                                    ));
                                }
                                values[idx as usize] = value;
                            }
                            self.push_value(Value::List(obj));
                        }
                    },
                    Value::ByteArray(obj) => match index {
                        Value::Slice(slice) => {
                            let lower = slice.lower;
                            let upper = slice.upper;
                            let step = slice.step;
                            let replacement = self.value_to_bytes_payload(value)?;
                            let has_exports =
                                self.heap.count_live_buffer_exports_for_source(&obj) > 0;
                            if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                let step_value = step.unwrap_or(1);
                                if step_value == 1 {
                                    let (start, stop) =
                                        slice_bounds_for_step_one(values.len(), lower, upper);
                                    if has_exports
                                        && replacement.len() != stop.saturating_sub(start)
                                    {
                                        return Err(RuntimeError::new(
                                            "BufferError: Existing exports of data: object cannot be re-sized",
                                        ));
                                    }
                                    values.splice(start..stop, replacement);
                                } else {
                                    let indices = slice_indices(values.len(), lower, upper, step)?;
                                    if indices.len() != replacement.len() {
                                        return Err(RuntimeError::new(format!(
                                            "attempt to assign sequence of size {} to extended slice of size {}",
                                            replacement.len(),
                                            indices.len()
                                        )));
                                    }
                                    for (idx, item) in indices.into_iter().zip(replacement) {
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
                                    return Err(RuntimeError::index_error("index out of range"));
                                }
                                let byte = value_to_int(value)?;
                                if !(0..=255).contains(&byte) {
                                    return Err(RuntimeError::value_error(
                                        "byte must be in range(0, 256)",
                                    ));
                                }
                                values[idx as usize] = byte as u8;
                            }
                            self.push_value(Value::ByteArray(obj));
                        }
                    },
                    Value::Instance(instance) => match index {
                        Value::Slice(_) => {
                            let target_value = Value::Instance(instance.clone());
                            if let Some(setitem) =
                                self.lookup_bound_special_method(&target_value, "__setitem__")?
                            {
                                let caller_idx = self.frames.len().saturating_sub(1);
                                if self.dispatch_call_no_kwargs_ignoring_result(
                                    caller_idx,
                                    setitem,
                                    vec![index, value],
                                    Some(target_value.clone()),
                                )? {
                                    return Ok(None);
                                }
                            } else if let Some(proxy_result) = self.cpython_proxy_set_item(
                                &target_value,
                                index.clone(),
                                value.clone(),
                            ) {
                                proxy_result?;
                                self.push_value(target_value);
                            } else if let Some(backing_list) = self.instance_backing_list(&instance)
                            {
                                let Value::Slice(slice) = index else {
                                    unreachable!();
                                };
                                let lower = slice.lower;
                                let upper = slice.upper;
                                let step = slice.step;
                                let replacement =
                                    self.collect_iterable_values(value).map_err(|_| {
                                        RuntimeError::new("can only assign an iterable")
                                    })?;
                                if let Object::List(values) = &mut *backing_list.kind_mut() {
                                    let step_value = step.unwrap_or(1);
                                    if step_value == 1 {
                                        let (start, stop) =
                                            slice_bounds_for_step_one(values.len(), lower, upper);
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
                                        for (idx, item) in indices.into_iter().zip(replacement) {
                                            values[idx] = item;
                                        }
                                    }
                                }
                                self.push_value(target_value);
                            } else {
                                if self.instance_backing_dict(&instance).is_some() {
                                    return Err(RuntimeError::new("slicing unsupported for dict"));
                                }
                                return Err(RuntimeError::new("slice assignment not supported"));
                            }
                        }
                        index => {
                            let target_value = Value::Instance(instance.clone());
                            if let Some(setitem) =
                                self.lookup_bound_special_method(&target_value, "__setitem__")?
                            {
                                let caller_idx = self.frames.len().saturating_sub(1);
                                if self.dispatch_call_no_kwargs_ignoring_result(
                                    caller_idx,
                                    setitem,
                                    vec![index, value],
                                    Some(target_value.clone()),
                                )? {
                                    return Ok(None);
                                }
                            } else if let Some(proxy_result) = self.cpython_proxy_set_item(
                                &target_value,
                                index.clone(),
                                value.clone(),
                            ) {
                                proxy_result?;
                                self.push_value(target_value);
                            } else if let Some(backing_list) = self.instance_backing_list(&instance)
                            {
                                if let Object::List(values) = &mut *backing_list.kind_mut() {
                                    let mut idx = value_to_int(index)? as isize;
                                    if idx < 0 {
                                        idx += values.len() as isize;
                                    }
                                    if idx < 0 || idx as usize >= values.len() {
                                        return Err(RuntimeError::index_error(
                                            "list index out of range",
                                        ));
                                    }
                                    values[idx as usize] = value;
                                }
                                self.push_value(target_value);
                            } else if let Some(backing_dict) = self.instance_backing_dict(&instance)
                            {
                                self.dict_set_value_checked_runtime(&backing_dict, index, value)?;
                                self.push_value(Value::Instance(instance));
                            } else {
                                if let Object::Instance(instance_data) = &*instance.kind()
                                    && instance_data.attrs.contains_key(MAPPING_PROXY_STORAGE_ATTR)
                                {
                                    return Err(RuntimeError::type_error(
                                        "'mappingproxy' object does not support item assignment",
                                    ));
                                }
                                if self.trace_flags.store_subscript {
                                    let target_value = Value::Instance(instance.clone());
                                    eprintln!(
                                        "[store-subscript] unsupported instance target_type={} index_type={} value_type={} target={} index={} value={}",
                                        self.value_type_name_for_error(&target_value),
                                        self.value_type_name_for_error(&index),
                                        self.value_type_name_for_error(&value),
                                        format_repr(&target_value),
                                        format_repr(&index),
                                        format_repr(&value),
                                    );
                                }
                                return Err(RuntimeError::type_error(format!(
                                    "'{}' object does not support item assignment",
                                    self.value_type_name_for_error(&Value::Instance(
                                        instance.clone()
                                    ))
                                )));
                            }
                        }
                    },
                    target => match (target, index) {
                        (Value::MemoryView(obj), Value::Slice(slice)) => {
                            let (
                                source,
                                view_start,
                                view_length,
                                view_itemsize,
                                view_shape,
                                view_strides,
                            ) = match &*obj.kind() {
                                Object::MemoryView(view) => (
                                    view.source.clone(),
                                    view.start,
                                    view.length,
                                    view.itemsize,
                                    view.shape.clone(),
                                    view.strides.clone(),
                                ),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "store subscript unsupported type",
                                    ));
                                }
                            };
                            if view_shape.as_ref().is_some_and(|shape| shape.len() > 1) {
                                return Err(RuntimeError::new(
                                    "NotImplementedError: memoryview slice assignments are currently restricted to ndim = 1",
                                ));
                            }
                            let replacement = self.value_to_bytes_payload(value)?;
                            match &mut *source.kind_mut() {
                                Object::ByteArray(values) => {
                                    if let Some((origin, logical_len, stride, itemsize)) =
                                        memoryview_layout_1d_from_parts(
                                            view_start,
                                            view_length,
                                            view_itemsize,
                                            view_shape.as_ref(),
                                            view_strides.as_ref(),
                                            values.len(),
                                        )
                                        && itemsize == 1
                                    {
                                        let indices = slice_indices(
                                            logical_len,
                                            slice.lower,
                                            slice.upper,
                                            slice.step,
                                        )?;
                                        if indices.len() != replacement.len() {
                                            return Err(RuntimeError::new(
                                                "memoryview assignment: lvalue and rvalue have different structures",
                                            ));
                                        }
                                        for (idx, item) in indices.into_iter().zip(replacement) {
                                            let offset = memoryview_element_offset(
                                                origin,
                                                logical_len,
                                                stride,
                                                idx as isize,
                                            )
                                            .ok_or_else(|| {
                                                RuntimeError::index_error("index out of range")
                                            })?;
                                            values[offset] = item;
                                        }
                                    } else {
                                        let (range_start, range_end) = memoryview_bounds(
                                            view_start,
                                            view_length,
                                            values.len(),
                                        );
                                        let range_len = range_end.saturating_sub(range_start);
                                        let lower = slice.lower;
                                        let upper = slice.upper;
                                        let step = slice.step;
                                        let step_value = step.unwrap_or(1);
                                        if step_value == 1 {
                                            let (start_rel, stop_rel) =
                                                slice_bounds_for_step_one(range_len, lower, upper);
                                            let start = range_start.saturating_add(start_rel);
                                            let stop = range_start.saturating_add(stop_rel);
                                            if replacement.len() != stop.saturating_sub(start) {
                                                return Err(RuntimeError::new(
                                                    "memoryview assignment: lvalue and rvalue have different structures",
                                                ));
                                            }
                                            values[start..stop].copy_from_slice(&replacement);
                                        } else {
                                            let indices =
                                                slice_indices(range_len, lower, upper, step)?;
                                            if indices.len() != replacement.len() {
                                                return Err(RuntimeError::new(
                                                    "memoryview assignment: lvalue and rvalue have different structures",
                                                ));
                                            }
                                            for (idx, item) in indices.into_iter().zip(replacement)
                                            {
                                                values[range_start + idx] = item;
                                            }
                                        }
                                    }
                                }
                                Object::Module(module_data) if module_data.name == "__array__" => {
                                    let Some(Value::List(values_obj)) =
                                        module_data.globals.get_mut("values")
                                    else {
                                        return Err(RuntimeError::new(
                                            "store subscript unsupported type",
                                        ));
                                    };
                                    let Object::List(values) = &mut *values_obj.kind_mut() else {
                                        return Err(RuntimeError::new(
                                            "store subscript unsupported type",
                                        ));
                                    };
                                    if let Some((origin, logical_len, stride, itemsize)) =
                                        memoryview_layout_1d_from_parts(
                                            view_start,
                                            view_length,
                                            view_itemsize,
                                            view_shape.as_ref(),
                                            view_strides.as_ref(),
                                            values.len(),
                                        )
                                        && itemsize == 1
                                    {
                                        let indices = slice_indices(
                                            logical_len,
                                            slice.lower,
                                            slice.upper,
                                            slice.step,
                                        )?;
                                        if indices.len() != replacement.len() {
                                            return Err(RuntimeError::new(
                                                "memoryview assignment: lvalue and rvalue have different structures",
                                            ));
                                        }
                                        for (idx, item) in indices.into_iter().zip(replacement) {
                                            let offset = memoryview_element_offset(
                                                origin,
                                                logical_len,
                                                stride,
                                                idx as isize,
                                            )
                                            .ok_or_else(|| {
                                                RuntimeError::index_error("index out of range")
                                            })?;
                                            values[offset] = Value::Int(item as i64);
                                        }
                                    } else {
                                        let (range_start, range_end) = memoryview_bounds(
                                            view_start,
                                            view_length,
                                            values.len(),
                                        );
                                        let range_len = range_end.saturating_sub(range_start);
                                        let lower = slice.lower;
                                        let upper = slice.upper;
                                        let step = slice.step;
                                        let step_value = step.unwrap_or(1);
                                        if step_value == 1 {
                                            let (start_rel, stop_rel) =
                                                slice_bounds_for_step_one(range_len, lower, upper);
                                            let start = range_start.saturating_add(start_rel);
                                            let stop = range_start.saturating_add(stop_rel);
                                            if replacement.len() != stop.saturating_sub(start) {
                                                return Err(RuntimeError::new(
                                                    "memoryview assignment: lvalue and rvalue have different structures",
                                                ));
                                            }
                                            for (offset, item) in replacement.iter().enumerate() {
                                                values[start + offset] = Value::Int(*item as i64);
                                            }
                                        } else {
                                            let indices =
                                                slice_indices(range_len, lower, upper, step)?;
                                            if indices.len() != replacement.len() {
                                                return Err(RuntimeError::new(
                                                    "memoryview assignment: lvalue and rvalue have different structures",
                                                ));
                                            }
                                            for (idx, item) in indices.into_iter().zip(replacement)
                                            {
                                                values[range_start + idx] = Value::Int(item as i64);
                                            }
                                        }
                                    }
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
                        (Value::Dict(obj), index) => {
                            let sync_key = index.clone();
                            let sync_value = value.clone();
                            self.dict_set_value_checked_runtime(&obj, index, value)?;
                            self.sync_module_global_from_locals_dict_write(
                                &obj,
                                &sync_key,
                                Some(sync_value.clone()),
                            );
                            self.sync_warnings_module_from_sys_modules_write(
                                &obj,
                                &sync_key,
                                Some(&sync_value),
                            );
                            self.push_value(Value::Dict(obj));
                        }
                        (Value::MemoryView(obj), index) => {
                            let (
                                source,
                                view_start,
                                view_length,
                                view_itemsize,
                                view_shape,
                                view_strides,
                                view_format,
                            ) = match &*obj.kind() {
                                Object::MemoryView(view) => (
                                    view.source.clone(),
                                    view.start,
                                    view.length,
                                    view.itemsize,
                                    view.shape.clone(),
                                    view.strides.clone(),
                                    view.format.clone(),
                                ),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "store subscript unsupported type",
                                    ));
                                }
                            };
                            if view_shape.as_ref().is_some_and(|shape| shape.len() > 1) {
                                return Err(RuntimeError::new(
                                    "NotImplementedError: sub-views are not implemented",
                                ));
                            }
                            let itemsize = view_itemsize.max(1);
                            let cast_format =
                                memoryview_format_for_view(itemsize, view_format.as_deref())?;
                            let replacement =
                                memoryview_encode_element(value, cast_format, itemsize)?;
                            let idx = value_to_int(index)? as isize;
                            match &mut *source.kind_mut() {
                                Object::ByteArray(values) => {
                                    let offset = if let Some((origin, logical_len, stride, _)) =
                                        memoryview_layout_1d_from_parts(
                                            view_start,
                                            view_length,
                                            view_itemsize,
                                            view_shape.as_ref(),
                                            view_strides.as_ref(),
                                            values.len(),
                                        ) {
                                        memoryview_element_offset(origin, logical_len, stride, idx)
                                            .ok_or_else(|| {
                                                RuntimeError::index_error("index out of range")
                                            })?
                                    } else {
                                        let (range_start, range_end) = memoryview_bounds(
                                            view_start,
                                            view_length,
                                            values.len(),
                                        );
                                        let span_len = range_end.saturating_sub(range_start);
                                        if span_len % itemsize != 0 {
                                            return Err(RuntimeError::new(
                                                "memoryview length is not a multiple of itemsize",
                                            ));
                                        }
                                        let logical_len = span_len / itemsize;
                                        let mut normalized = idx;
                                        if normalized < 0 {
                                            normalized += logical_len as isize;
                                        }
                                        if normalized < 0 || normalized as usize >= logical_len {
                                            return Err(RuntimeError::index_error(
                                                "index out of range",
                                            ));
                                        }
                                        range_start + (normalized as usize).saturating_mul(itemsize)
                                    };
                                    let end = offset.checked_add(itemsize).ok_or_else(|| {
                                        RuntimeError::index_error("index out of range")
                                    })?;
                                    let target = values.get_mut(offset..end).ok_or_else(|| {
                                        RuntimeError::index_error("index out of range")
                                    })?;
                                    target.copy_from_slice(&replacement);
                                }
                                Object::Module(module_data) if module_data.name == "__array__" => {
                                    let Some(Value::List(values_obj)) =
                                        module_data.globals.get_mut("values")
                                    else {
                                        return Err(RuntimeError::new(
                                            "store subscript unsupported type",
                                        ));
                                    };
                                    let Object::List(values) = &mut *values_obj.kind_mut() else {
                                        return Err(RuntimeError::new(
                                            "store subscript unsupported type",
                                        ));
                                    };
                                    let offset = if let Some((origin, logical_len, stride, _)) =
                                        memoryview_layout_1d_from_parts(
                                            view_start,
                                            view_length,
                                            view_itemsize,
                                            view_shape.as_ref(),
                                            view_strides.as_ref(),
                                            values.len(),
                                        ) {
                                        memoryview_element_offset(origin, logical_len, stride, idx)
                                            .ok_or_else(|| {
                                                RuntimeError::index_error("index out of range")
                                            })?
                                    } else {
                                        let (range_start, range_end) = memoryview_bounds(
                                            view_start,
                                            view_length,
                                            values.len(),
                                        );
                                        let span_len = range_end.saturating_sub(range_start);
                                        if span_len % itemsize != 0 {
                                            return Err(RuntimeError::new(
                                                "memoryview length is not a multiple of itemsize",
                                            ));
                                        }
                                        let logical_len = span_len / itemsize;
                                        let mut normalized = idx;
                                        if normalized < 0 {
                                            normalized += logical_len as isize;
                                        }
                                        if normalized < 0 || normalized as usize >= logical_len {
                                            return Err(RuntimeError::index_error(
                                                "index out of range",
                                            ));
                                        }
                                        range_start + (normalized as usize).saturating_mul(itemsize)
                                    };
                                    let end = offset.checked_add(itemsize).ok_or_else(|| {
                                        RuntimeError::index_error("index out of range")
                                    })?;
                                    if end > values.len() {
                                        return Err(RuntimeError::index_error(
                                            "index out of range",
                                        ));
                                    }
                                    for (byte_offset, byte) in replacement.iter().enumerate() {
                                        values[offset + byte_offset] = Value::Int(*byte as i64);
                                    }
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
                        (target, index) => {
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
                            } else if let Some(proxy_result) = self.cpython_proxy_set_item(
                                &target_value,
                                index.clone(),
                                value.clone(),
                            ) {
                                proxy_result?;
                                self.push_value(target_value);
                            } else {
                                if self.trace_flags.store_subscript {
                                    eprintln!(
                                        "[store-subscript] unsupported target_type={} index_type={} value_type={} target={} index={} value={}",
                                        self.value_type_name_for_error(&target),
                                        self.value_type_name_for_error(&index),
                                        self.value_type_name_for_error(&value),
                                        format_repr(&target),
                                        format_repr(&index),
                                        format_repr(&value),
                                    );
                                }
                                return Err(RuntimeError::type_error(format!(
                                    "'{}' object does not support item assignment",
                                    self.value_type_name_for_error(&target_value)
                                )));
                            }
                        }
                    },
                };
                if discard_result {
                    let _ = self.pop_value()?;
                }
            }
            Opcode::StoreSlice => {
                let upper = self.pop_value()?;
                let lower = self.pop_value()?;
                let target = self.pop_value()?;
                let value = self.pop_value()?;
                let lower = value_to_optional_index(lower)?;
                let upper = value_to_optional_index(upper)?;
                let slice = Value::Slice(Box::new(SliceValue::new(lower, upper, None)));
                // Reuse STORE_SUBSCR CPython stack convention so all target/setitem
                // semantics stay centralized in one implementation path.
                self.push_value(value);
                self.push_value(target);
                self.push_value(slice);
                let _ =
                    self.execute_instruction(Instruction::new(Opcode::StoreSubscript, Some(1)))?;
            }
            Opcode::DeleteSubscript => {
                let index = self.pop_value()?;
                let target = self.pop_value()?;
                match target {
                    Value::List(obj) => match index {
                        Value::Slice(slice) => {
                            let lower = slice.lower;
                            let upper = slice.upper;
                            let step = slice.step;
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
                                    return Err(RuntimeError::index_error(
                                        "list index out of range",
                                    ));
                                }
                                values.remove(idx as usize);
                            }
                        }
                    },
                    Value::ByteArray(obj) => match index {
                        Value::Slice(slice) => {
                            let lower = slice.lower;
                            let upper = slice.upper;
                            let step = slice.step;
                            let has_exports =
                                self.heap.count_live_buffer_exports_for_source(&obj) > 0;
                            if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                let step_value = step.unwrap_or(1);
                                if step_value == 1 {
                                    let (start, stop) =
                                        slice_bounds_for_step_one(values.len(), lower, upper);
                                    if has_exports && stop > start {
                                        return Err(RuntimeError::new(
                                            "BufferError: Existing exports of data: object cannot be re-sized",
                                        ));
                                    }
                                    values.drain(start..stop);
                                } else {
                                    let mut indices =
                                        slice_indices(values.len(), lower, upper, step)?;
                                    if has_exports && !indices.is_empty() {
                                        return Err(RuntimeError::new(
                                            "BufferError: Existing exports of data: object cannot be re-sized",
                                        ));
                                    }
                                    indices.sort_unstable();
                                    for idx in indices.into_iter().rev() {
                                        values.remove(idx);
                                    }
                                }
                            }
                        }
                        index => {
                            if self.heap.count_live_buffer_exports_for_source(&obj) > 0 {
                                return Err(RuntimeError::new(
                                    "BufferError: Existing exports of data: object cannot be re-sized",
                                ));
                            }
                            if let Object::ByteArray(values) = &mut *obj.kind_mut() {
                                let mut idx = value_to_int(index)? as isize;
                                if idx < 0 {
                                    idx += values.len() as isize;
                                }
                                if idx < 0 || idx as usize >= values.len() {
                                    return Err(RuntimeError::index_error("index out of range"));
                                }
                                values.remove(idx as usize);
                            }
                        }
                    },
                    Value::Instance(instance) => match index {
                        Value::Slice(_) => {
                            let target_value = Value::Instance(instance.clone());
                            if let Some(delitem) =
                                self.lookup_bound_special_method(&target_value, "__delitem__")?
                            {
                                let caller_idx = self.frames.len().saturating_sub(1);
                                if self.dispatch_call_no_kwargs_ignoring_result(
                                    caller_idx,
                                    delitem,
                                    vec![index],
                                    None,
                                )? {
                                    return Ok(None);
                                }
                            } else {
                                if self.instance_backing_dict(&instance).is_some() {
                                    return Err(RuntimeError::new("slice deletion not supported"));
                                }
                                return Err(RuntimeError::new("slice deletion not supported"));
                            }
                        }
                        index => {
                            let target_value = Value::Instance(instance.clone());
                            if let Some(delitem) =
                                self.lookup_bound_special_method(&target_value, "__delitem__")?
                            {
                                let caller_idx = self.frames.len().saturating_sub(1);
                                if self.dispatch_call_no_kwargs_ignoring_result(
                                    caller_idx,
                                    delitem,
                                    vec![index],
                                    None,
                                )? {
                                    return Ok(None);
                                }
                            } else if let Some(backing_dict) = self.instance_backing_dict(&instance)
                            {
                                if self
                                    .dict_remove_value_runtime(&backing_dict, &index)?
                                    .is_none()
                                {
                                    return Err(RuntimeError::key_error("key not found"));
                                }
                            } else {
                                return Err(RuntimeError::new("delete subscript unsupported type"));
                            }
                        }
                    },
                    target => match index {
                        Value::Slice(_) => {
                            return Err(RuntimeError::new("slice deletion not supported"));
                        }
                        index => match target {
                            Value::Dict(obj) => {
                                let sync_key = index.clone();
                                if self.dict_remove_value_runtime(&obj, &index)?.is_none() {
                                    return Err(RuntimeError::key_error("key not found"));
                                }
                                self.sync_module_global_from_locals_dict_write(
                                    &obj, &sync_key, None,
                                );
                            }
                            _ => {
                                if let Some(delitem) =
                                    self.lookup_bound_special_method(&target, "__delitem__")?
                                {
                                    let caller_idx = self.frames.len().saturating_sub(1);
                                    if self.dispatch_call_no_kwargs_ignoring_result(
                                        caller_idx,
                                        delitem,
                                        vec![index],
                                        None,
                                    )? {
                                        return Ok(None);
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
                let value = {
                    let frame = self.frames.last().expect("frame exists");
                    frame
                        .code
                        .constants
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("constant index out of range"))?
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
                            return Err(RuntimeError::new("expected defaults tuple for function"));
                        }
                    },
                    _ => {
                        return Err(RuntimeError::new("expected defaults tuple for function"));
                    }
                };
                let (module, defined_in_class_body) = {
                    let frame = self.frames.last().expect("frame exists");
                    (
                        if frame.return_class && is_comprehension_code(&code) {
                            frame.module.clone()
                        } else {
                            frame.function_globals.clone()
                        },
                        frame.return_class,
                    )
                };
                let function_qualname = self.current_frame_function_qualname(&code.name);
                let func = FunctionObject::new(
                    code,
                    module,
                    defaults,
                    kwonly_defaults,
                    Vec::new(),
                    None,
                    defined_in_class_body,
                );
                let func_value = self.heap.alloc_function(func);
                if let Value::Function(function_obj) = &func_value {
                    let function_dict = self.ensure_function_dict(function_obj)?;
                    self.dict_set_str_key(
                        &function_dict,
                        "__qualname__".to_string(),
                        Value::Str(function_qualname),
                    )?;
                }
                self.push_value(func_value);
            }
            Opcode::BuildClass => {
                let idx = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing class code argument"))?
                    as usize;
                let value = {
                    let frame = self.frames.last().expect("frame exists");
                    frame
                        .code
                        .constants
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("constant index out of range"))?
                };
                let code = match value {
                    Value::Code(code) => code,
                    _ => {
                        return Err(RuntimeError::new("expected code object for class body"));
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
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => values.clone(),
                        _ => return Err(RuntimeError::new("class bases must be a tuple")),
                    },
                    _ => return Err(RuntimeError::new("class bases must be a tuple")),
                };
                let orig_bases_tuple = self.heap.alloc_tuple(bases.clone());
                let trace_build_class = self.trace_flags.build_class;
                let trace_this_class = trace_build_class && class_name == "_TagInfo";
                let mut resolved_bases = Vec::new();
                let mut used_mro_entries = false;
                for base in bases {
                    if trace_this_class {
                        eprintln!(
                            "[build-class-op] name={} raw_base={}",
                            class_name,
                            format_repr(&base)
                        );
                    }
                    let maybe_mro_entries = if matches!(base, Value::Class(_)) {
                        None
                    } else {
                        match self.builtin_getattr(
                            vec![base.clone(), Value::Str("__mro_entries__".to_string())],
                            HashMap::new(),
                        ) {
                            Ok(callable) => Some(callable),
                            Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {
                                None
                            }
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
                            InternalCallOutcome::CallerExceptionHandled => return Ok(None),
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
                    let resolved = resolved_bases
                        .iter()
                        .map(format_repr)
                        .collect::<Vec<_>>()
                        .join(", ");
                    eprintln!(
                        "[build-class-op] name={} resolved_bases=[{}]",
                        class_name, resolved
                    );
                }
                let mut base_classes = Vec::new();
                for base in resolved_bases {
                    match self.class_from_base_value(base.clone()) {
                        Ok(class) => base_classes.push(class),
                        Err(err) => {
                            if self.trace_flags.class_base
                                && runtime_error_matches_exception(&err, "TypeError")
                                && let Some(frame) = self.frames.last()
                            {
                                let location = frame.code.locations.get(frame.last_ip);
                                eprintln!(
                                    "[class-base] build-class file={} line={} col={} base={}",
                                    frame.code.filename,
                                    location.map(|loc| loc.line).unwrap_or(0),
                                    location.map(|loc| loc.column).unwrap_or(0),
                                    format_value(&base)
                                );
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
                    eprintln!(
                        "[build-class-op] name={} base_classes=[{}]",
                        class_name, base_names
                    );
                }

                let class_declared_global = self
                    .frames
                    .last()
                    .is_some_and(|frame| self.class_assignment_is_global(frame, &class_name));

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
                            return Some(format!("{outer_qualname}.{class_name}"));
                        }
                        if frame.is_module || class_declared_global {
                            return None;
                        }
                        let mut outer_qualname = frame.code.name.clone();
                        let owner_value =
                            Self::frame_trace(frame).local_values.into_iter().find_map(
                                |(name, value)| (name == "self" || name == "cls").then_some(value),
                            );
                        if let Some(owner) = owner_value {
                            match owner {
                                Value::Instance(instance) => {
                                    if let Object::Instance(instance_data) = &*instance.kind()
                                        && let Object::Class(class_data) =
                                            &*instance_data.class.kind()
                                    {
                                        outer_qualname =
                                            format!("{}.{}", class_data.name, outer_qualname);
                                    }
                                }
                                Value::Class(class) => {
                                    if let Object::Class(class_data) = &*class.kind() {
                                        outer_qualname =
                                            format!("{}.{}", class_data.name, outer_qualname);
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(format!("{outer_qualname}.<locals>.{class_name}"))
                    })
                    .unwrap_or_else(|| class_name.clone());

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
                                class_name, meta_name, callable_type, callable_repr
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
                            vec![Value::Str(class_name.clone()), bases_tuple],
                            class_keywords.clone(),
                        )? {
                            InternalCallOutcome::Value(value) => value,
                            InternalCallOutcome::CallerExceptionHandled => return Ok(None),
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
                let module_name = self
                    .lookup_name_with_index(usize::MAX, "__name__")
                    .ok()
                    .and_then(|value| match value {
                        Value::Str(name) => Some(name),
                        _ => None,
                    })
                    .unwrap_or_else(|| "__main__".to_string());
                if self
                    .class_namespace_lookup_name(&prepared_namespace, "__module__")
                    .is_none()
                {
                    self.class_namespace_set_name(
                        &prepared_namespace,
                        "__module__".to_string(),
                        Value::Str(module_name),
                    )?;
                }
                if self
                    .class_namespace_lookup_name(&prepared_namespace, "__qualname__")
                    .is_none()
                {
                    self.class_namespace_set_name(
                        &prepared_namespace,
                        "__qualname__".to_string(),
                        Value::Str(class_qualname.clone()),
                    )?;
                }

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
                let mut frame = self.acquire_frame(code, class_module, true, false, cells, None);
                frame.function_globals = outer_globals.clone();
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
                frame.class_keywords = class_keywords;
                self.push_frame_checked(frame)?;
            }
            Opcode::MakeFunctionStack => {
                let value = self.pop_value()?;
                let code = match value {
                    Value::Code(code) => code,
                    _ => {
                        return Err(RuntimeError::new("expected code object for function"));
                    }
                };
                let (module, defined_in_class_body) = {
                    let frame = self.frames.last().expect("frame exists");
                    (
                        if frame.return_class && is_comprehension_code(&code) {
                            frame.module.clone()
                        } else {
                            frame.function_globals.clone()
                        },
                        frame.return_class,
                    )
                };
                let function_qualname = self.current_frame_function_qualname(&code.name);
                let func = FunctionObject::new(
                    code,
                    module,
                    Vec::new(),
                    HashMap::new(),
                    Vec::new(),
                    None,
                    defined_in_class_body,
                );
                let func_value = self.heap.alloc_function(func);
                if let Value::Function(function_obj) = &func_value {
                    let function_dict = self.ensure_function_dict(function_obj)?;
                    self.dict_set_str_key(
                        &function_dict,
                        "__qualname__".to_string(),
                        Value::Str(function_qualname),
                    )?;
                }
                self.push_value(func_value);
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
                                    return Err(RuntimeError::new("defaults must be tuple"));
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
                                    return Err(RuntimeError::new("kwonly defaults must be dict"));
                                }
                            },
                            _ => {
                                return Err(RuntimeError::new("kwonly defaults must be dict"));
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
                        if let Some(annotation_locals) = self.current_frame_annotation_locals() {
                            let entries = annotation_locals
                                .into_iter()
                                .map(|(name, value)| (Value::Str(name), value))
                                .collect::<Vec<_>>();
                            let annotation_locals_dict = self.heap.alloc_dict(entries);
                            let dict = self.ensure_function_dict(&func)?;
                            self.dict_set_str_key(
                                &dict,
                                "__pyrs_annotation_locals__".to_string(),
                                annotation_locals_dict,
                            )?;
                        }
                    }
                    0x10 => {
                        if !matches!(attr, Value::None) && !self.is_callable_value(&attr) {
                            return Err(RuntimeError::new("annotate must be callable"));
                        }
                        let dict = self.ensure_function_dict(&func)?;
                        self.dict_set_str_key(&dict, "__annotate__".to_string(), attr)?;
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
                                                if self.trace_flags.closure_shape {
                                                    if let Some(frame) = self.frames.last() {
                                                        eprintln!(
                                                            "[closure-shape] file={} fn={} attr_kind=0x08 entry_type={}",
                                                            frame.code.filename,
                                                            frame.code.name,
                                                            self.value_type_name_for_error(value)
                                                        );
                                                    }
                                                }
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
                if argc == 0 {
                    let site_index = self.current_site_index();
                    let quickened_zero_arg =
                        self.is_quickened_site(site_index, QuickenedSiteKind::CallFunctionZeroArg);
                    let func = {
                        let frame = self.frames.last_mut().expect("frame exists");
                        frame.stack.pop().ok_or_else(|| {
                            RuntimeError::new("stack underflow (CallFunction func)")
                        })?
                    };
                    match func {
                        Value::Function(func_obj) => {
                            if !quickened_zero_arg {
                                self.mark_quickened_site(
                                    site_index,
                                    QuickenedSiteKind::CallFunctionZeroArg,
                                );
                            }
                            self.push_function_call_from_obj(
                                &func_obj,
                                Vec::new(),
                                HashMap::new(),
                            )?;
                        }
                        Value::BoundMethod(method_obj) => {
                            if quickened_zero_arg {
                                self.clear_quickened_site(site_index);
                            }
                            self.push_bound_method_call_zero_args_from_obj(&method_obj)?;
                        }
                        Value::Builtin(builtin) => {
                            if quickened_zero_arg {
                                self.clear_quickened_site(site_index);
                            }
                            if let Some(result) = self.try_fast_builtin_zero_arg_no_kwargs(builtin)
                            {
                                self.push_value(result);
                            } else {
                                let caller_depth = self.frames.len();
                                let caller_idx = caller_depth.saturating_sub(1);
                                let caller_ip = self
                                    .frames
                                    .get(caller_idx)
                                    .map(|frame| frame.ip)
                                    .unwrap_or(0);
                                let call_result =
                                    self.call_builtin(builtin, Vec::new(), HashMap::new());
                                self.finalize_builtin_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                        }
                        other => {
                            if quickened_zero_arg {
                                self.clear_quickened_site(site_index);
                            }
                            self.dispatch_call_no_kwargs(other, Vec::new())?
                        }
                    }
                } else if argc == 1 {
                    let site_index = self.current_site_index();
                    let quickened_one_arg =
                        self.is_quickened_site(site_index, QuickenedSiteKind::CallFunctionOneArg);
                    let (func, arg0) = {
                        let frame = self.frames.last_mut().expect("frame exists");
                        let arg0 = frame.stack.pop().ok_or_else(|| {
                            RuntimeError::new("stack underflow (CallFunction arg0)")
                        })?;
                        let func = frame.stack.pop().ok_or_else(|| {
                            RuntimeError::new("stack underflow (CallFunction func)")
                        })?;
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
                        Value::BoundMethod(method_obj) => {
                            if quickened_one_arg {
                                self.clear_quickened_site(site_index);
                            }
                            self.push_bound_method_call_one_arg_from_obj(&method_obj, arg0)?;
                        }
                        Value::Builtin(builtin) => {
                            if quickened_one_arg {
                                self.clear_quickened_site(site_index);
                            }
                            if let Some(result) =
                                self.try_fast_builtin_single_arg_no_kwargs(builtin, &arg0)?
                            {
                                self.push_value(result);
                            } else {
                                let caller_depth = self.frames.len();
                                let caller_idx = caller_depth.saturating_sub(1);
                                let caller_ip = self
                                    .frames
                                    .get(caller_idx)
                                    .map(|frame| frame.ip)
                                    .unwrap_or(0);
                                let call_result =
                                    self.call_builtin(builtin, vec![arg0], HashMap::new());
                                self.finalize_builtin_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                        }
                        other => {
                            if quickened_one_arg {
                                self.clear_quickened_site(site_index);
                            }
                            self.dispatch_call_no_kwargs(other, vec![arg0])?
                        }
                    }
                } else if argc == 2 {
                    let site_index = self.current_site_index();
                    let quickened_two_arg =
                        self.is_quickened_site(site_index, QuickenedSiteKind::CallFunctionTwoArg);
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
                            if !quickened_two_arg {
                                self.mark_quickened_site(
                                    site_index,
                                    QuickenedSiteKind::CallFunctionTwoArg,
                                );
                            }
                            self.push_function_call_two_args_from_obj(&func_obj, arg0, arg1)?;
                        }
                        Value::BoundMethod(method_obj) => {
                            if quickened_two_arg {
                                self.clear_quickened_site(site_index);
                            }
                            self.push_bound_method_call_two_args_from_obj(&method_obj, arg0, arg1)?;
                        }
                        other => {
                            if quickened_two_arg {
                                self.clear_quickened_site(site_index);
                            }
                            self.dispatch_call_no_kwargs(other, vec![arg0, arg1])?
                        }
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
                        Value::BoundMethod(method_obj) => {
                            self.push_bound_method_call_three_args_from_obj(
                                &method_obj,
                                arg0,
                                arg1,
                                arg2,
                            )?;
                        }
                        other => self.dispatch_call_no_kwargs(other, vec![arg0, arg1, arg2])?,
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
                let (func, arg0) =
                    {
                        let frame = self.frames.last_mut().expect("frame exists");
                        let arg0 = frame.stack.pop().ok_or_else(|| {
                            RuntimeError::new("stack underflow (CallFunction1 arg0)")
                        })?;
                        let func = frame.stack.pop().ok_or_else(|| {
                            RuntimeError::new("stack underflow (CallFunction1 func)")
                        })?;
                        (func, arg0)
                    };
                match func {
                    Value::Function(func_obj) => {
                        self.push_function_call_one_arg_from_obj(&func_obj, arg0)?;
                    }
                    Value::BoundMethod(method_obj) => {
                        self.push_bound_method_call_one_arg_from_obj(&method_obj, arg0)?;
                    }
                    Value::Builtin(builtin) => {
                        if let Some(result) =
                            self.try_fast_builtin_single_arg_no_kwargs(builtin, &arg0)?
                        {
                            self.push_value(result);
                        } else {
                            let caller_depth = self.frames.len();
                            let caller_idx = caller_depth.saturating_sub(1);
                            let caller_ip = self
                                .frames
                                .get(caller_idx)
                                .map(|frame| frame.ip)
                                .unwrap_or(0);
                            let call_result =
                                self.call_builtin(builtin, vec![arg0], HashMap::new());
                            self.finalize_builtin_opcode_call(
                                caller_depth,
                                caller_ip,
                                call_result,
                            )?;
                        }
                    }
                    other => self.dispatch_call_no_kwargs(other, vec![arg0])?,
                }
            }
            Opcode::CallCpython => {
                let arg = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing call argument"))?;
                let pos_count = (arg & 0xFFFF) as usize;
                let kw_idx = (arg >> 16) as u16;
                let kw_names =
                    if kw_idx == u16::MAX {
                        None
                    } else {
                        let idx = kw_idx as usize;
                        let value =
                            {
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
                                return Err(RuntimeError::new("kw_names must be tuple of strings"));
                            }
                        },
                        Value::None => None,
                        _ => {
                            return Err(RuntimeError::new("kw_names must be tuple of strings"));
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
                let mut kwargs_order = Vec::with_capacity(kw_count);
                for idx in (0..kw_count).rev() {
                    let value = self.pop_value()?;
                    let name = kw_names
                        .as_ref()
                        .expect("kw names")
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("kw name index out of range"))?;
                    kwargs.insert(name.clone(), value);
                    kwargs_order.push(name);
                }
                kwargs_order.reverse();
                let mut args = Vec::with_capacity(pos_count - kw_count);
                for _ in 0..(pos_count - kw_count) {
                    args.push(self.pop_value()?);
                }
                args.reverse();
                let top = self.pop_value()?;
                let below = self.pop_value()?;
                let (func, self_or_null) =
                    if matches!(below, Value::None) && !matches!(top, Value::None) {
                        (top, Value::None)
                    } else {
                        (below, top)
                    };
                let func = func;
                if !matches!(self_or_null, Value::None) {
                    args.insert(0, self_or_null);
                }

                let mut fast_dispatched = false;
                if kwargs.is_empty() {
                    fast_dispatched = self.dispatch_small_arity_no_kwargs_call(&func, &mut args)?;
                }
                if kwargs.is_empty() {
                    if !fast_dispatched {
                        self.dispatch_call_no_kwargs(func, args)?;
                    }
                } else {
                    let kwargs_order_opt = if kwargs_order.is_empty() {
                        None
                    } else {
                        Some(kwargs_order.clone())
                    };
                    match func {
                        Value::Function(func) => {
                            self.push_function_call_from_obj_with_kwarg_order(
                                &func,
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )?;
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
                                    bound_args.push(self.receiver_value(&method_data.receiver)?);
                                    bound_args.extend(args);
                                    self.push_function_call_from_obj_with_kwarg_order(
                                        &method_data.function,
                                        bound_args,
                                        kwargs,
                                        kwargs_order_opt,
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
                                    let call_result = self.call_native_method_with_kwarg_order(
                                        native.kind,
                                        method_data.receiver.clone(),
                                        args,
                                        kwargs,
                                        kwargs_order_opt,
                                    );
                                    self.finalize_native_opcode_call(
                                        caller_depth,
                                        caller_ip,
                                        call_result,
                                    )?;
                                }
                                _ => {
                                    match self.call_internal_with_kwarg_order(
                                        Value::BoundMethod(method.clone()),
                                        args,
                                        kwargs,
                                        kwargs_order_opt,
                                    )? {
                                        InternalCallOutcome::Value(value) => {
                                            self.push_value(value);
                                        }
                                        InternalCallOutcome::CallerExceptionHandled => {}
                                    }
                                }
                            }
                        }
                        Value::Class(class) => {
                            match self.call_internal_with_kwarg_order(
                                Value::Class(class),
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )? {
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
                            let call_result = self.call_builtin_with_kwarg_order(
                                builtin,
                                args,
                                kwargs,
                                kwargs_order_opt,
                            );
                            self.finalize_builtin_opcode_call(
                                caller_depth,
                                caller_ip,
                                call_result,
                            )?;
                        }
                        Value::Instance(instance) => {
                            match self.call_internal_with_kwarg_order(
                                Value::Instance(instance),
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )? {
                                InternalCallOutcome::Value(value) => self.push_value(value),
                                InternalCallOutcome::CallerExceptionHandled => {}
                            }
                        }
                        Value::ExceptionType(name) => {
                            let value = self.instantiate_exception_type(&name, &args, &kwargs)?;
                            self.push_value(value);
                        }
                        other => {
                            return Err(RuntimeError::new(format!(
                                "attempted to call non-function: {}",
                                format_value(&other)
                            )));
                        }
                    }
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
                                        return Err(RuntimeError::new("kw names must be strings"));
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
                let mut kwargs_order = Vec::with_capacity(kw_count);
                for idx in (0..kw_count).rev() {
                    let value = self.pop_value()?;
                    let name = kw_names
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("kw name index out of range"))?;
                    kwargs.insert(name.clone(), value);
                    kwargs_order.push(name);
                }
                kwargs_order.reverse();
                let mut args = Vec::with_capacity(pos_total - kw_count);
                for _ in 0..(pos_total - kw_count) {
                    args.push(self.pop_value()?);
                }
                args.reverse();
                let top = self.pop_value()?;
                let below = self.pop_value()?;
                let (func, self_or_null) =
                    if matches!(below, Value::None) && !matches!(top, Value::None) {
                        (top, Value::None)
                    } else {
                        (below, top)
                    };
                let func = func;
                if !matches!(self_or_null, Value::None) {
                    args.insert(0, self_or_null);
                }
                let mut fast_dispatched = false;
                if kwargs.is_empty() {
                    fast_dispatched = self.dispatch_small_arity_no_kwargs_call(&func, &mut args)?;
                }
                if kwargs.is_empty() {
                    if !fast_dispatched {
                        self.dispatch_call_no_kwargs(func, args)?;
                    }
                } else {
                    let kwargs_order_opt = if kwargs_order.is_empty() {
                        None
                    } else {
                        Some(kwargs_order.clone())
                    };
                    match func {
                        Value::Function(func) => {
                            self.push_function_call_from_obj_with_kwarg_order(
                                &func,
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )?;
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
                                    bound_args.push(self.receiver_value(&method_data.receiver)?);
                                    bound_args.extend(args);
                                    self.push_function_call_from_obj_with_kwarg_order(
                                        &method_data.function,
                                        bound_args,
                                        kwargs,
                                        kwargs_order_opt,
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
                                    let call_result = self.call_native_method_with_kwarg_order(
                                        native.kind,
                                        method_data.receiver.clone(),
                                        args,
                                        kwargs,
                                        kwargs_order_opt,
                                    );
                                    self.finalize_native_opcode_call(
                                        caller_depth,
                                        caller_ip,
                                        call_result,
                                    )?;
                                }
                                _ => {
                                    match self.call_internal_with_kwarg_order(
                                        Value::BoundMethod(method.clone()),
                                        args,
                                        kwargs,
                                        kwargs_order_opt,
                                    )? {
                                        InternalCallOutcome::Value(value) => {
                                            self.push_value(value);
                                        }
                                        InternalCallOutcome::CallerExceptionHandled => {}
                                    }
                                }
                            }
                        }
                        Value::Class(class) => {
                            match self.call_internal_with_kwarg_order(
                                Value::Class(class),
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )? {
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
                            let call_result = self.call_builtin_with_kwarg_order(
                                builtin,
                                args,
                                kwargs,
                                kwargs_order_opt,
                            );
                            self.finalize_builtin_opcode_call(
                                caller_depth,
                                caller_ip,
                                call_result,
                            )?;
                        }
                        Value::Instance(instance) => {
                            match self.call_internal_with_kwarg_order(
                                Value::Instance(instance),
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )? {
                                InternalCallOutcome::Value(value) => self.push_value(value),
                                InternalCallOutcome::CallerExceptionHandled => {}
                            }
                        }
                        Value::ExceptionType(name) => {
                            let value = self.instantiate_exception_type(&name, &args, &kwargs)?;
                            self.push_value(value);
                        }
                        other => {
                            return Err(RuntimeError::new(format!(
                                "attempted to call non-function: {}",
                                format_value(&other)
                            )));
                        }
                    }
                }
            }
            Opcode::CallFunctionKw => {
                let arg = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing call argument"))?;
                let (pos_count, kw_count) = decode_call_counts(arg);
                let mut kwargs = HashMap::new();
                let mut kwargs_order = Vec::with_capacity(kw_count);
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
                    kwargs.insert(name.clone(), value);
                    kwargs_order.push(name);
                }
                kwargs_order.reverse();
                let mut args = Vec::with_capacity(pos_count);
                for _ in 0..pos_count {
                    args.push(self.pop_value()?);
                }
                args.reverse();
                let func = self.pop_value()?;
                let mut fast_dispatched = false;
                if kwargs.is_empty() {
                    fast_dispatched = self.dispatch_small_arity_no_kwargs_call(&func, &mut args)?;
                }
                if !fast_dispatched {
                    let kwargs_order_opt = if kwargs_order.is_empty() {
                        None
                    } else {
                        Some(kwargs_order.clone())
                    };
                    match func {
                        Value::Function(func) => {
                            self.push_function_call_from_obj_with_kwarg_order(
                                &func,
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )?;
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
                                    bound_args.push(self.receiver_value(&method_data.receiver)?);
                                    bound_args.extend(args);
                                    self.push_function_call_from_obj_with_kwarg_order(
                                        &method_data.function,
                                        bound_args,
                                        kwargs,
                                        kwargs_order_opt,
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
                                    let call_result = self.call_native_method_with_kwarg_order(
                                        native.kind,
                                        method_data.receiver.clone(),
                                        args,
                                        kwargs,
                                        kwargs_order_opt,
                                    );
                                    self.finalize_native_opcode_call(
                                        caller_depth,
                                        caller_ip,
                                        call_result,
                                    )?;
                                }
                                _ => {
                                    match self.call_internal(
                                        Value::BoundMethod(method.clone()),
                                        args,
                                        kwargs,
                                    )? {
                                        InternalCallOutcome::Value(value) => {
                                            self.push_value(value);
                                        }
                                        InternalCallOutcome::CallerExceptionHandled => {}
                                    }
                                }
                            }
                        }
                        Value::Class(class) => {
                            match self.call_internal_with_kwarg_order(
                                Value::Class(class),
                                args,
                                kwargs,
                                kwargs_order_opt,
                            )? {
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
                            let call_result = self.call_builtin_with_kwarg_order(
                                builtin,
                                args,
                                kwargs,
                                kwargs_order_opt,
                            );
                            self.finalize_builtin_opcode_call(
                                caller_depth,
                                caller_ip,
                                call_result,
                            )?;
                        }
                        Value::Instance(instance) => {
                            match self.call_internal(Value::Instance(instance), args, kwargs)? {
                                InternalCallOutcome::Value(value) => self.push_value(value),
                                InternalCallOutcome::CallerExceptionHandled => {}
                            }
                        }
                        Value::ExceptionType(name) => {
                            let value = self.instantiate_exception_type(&name, &args, &kwargs)?;
                            self.push_value(value);
                        }
                        other => {
                            return Err(RuntimeError::new(format!(
                                "attempted to call non-function: {}",
                                format_value(&other)
                            )));
                        }
                    }
                }
            }
            Opcode::CallFunctionVar => {
                let kwargs_value = self.pop_value()?;
                let args_value = self.pop_value()?;
                let func = self.pop_value()?;
                let (kwargs, kwargs_order) = match kwargs_value {
                    Value::Dict(obj) => match &*obj.kind() {
                        Object::Dict(entries) => {
                            let mut map = HashMap::new();
                            let mut order = Vec::with_capacity(entries.len());
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
                                    return Err(RuntimeError::new("duplicate keyword argument"));
                                }
                                map.insert(key.clone(), value.clone());
                                order.push(key);
                            }
                            (map, order)
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
                        let kwargs_order_opt = if kwargs_order.is_empty() {
                            None
                        } else {
                            Some(kwargs_order.clone())
                        };
                        self.push_function_call_from_obj_with_kwarg_order(
                            &func,
                            args,
                            kwargs,
                            kwargs_order_opt,
                        )?;
                    }
                    Value::BoundMethod(method) => {
                        let method_data = match &*method.kind() {
                            Object::BoundMethod(data) => data.clone(),
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "attempted to call non-function",
                                ));
                            }
                        };
                        match &*method_data.function.kind() {
                            Object::Function(_) => {
                                let mut bound_args = Vec::with_capacity(args.len() + 1);
                                bound_args.push(self.receiver_value(&method_data.receiver)?);
                                bound_args.extend(args);
                                let kwargs_order_opt = if kwargs_order.is_empty() {
                                    None
                                } else {
                                    Some(kwargs_order.clone())
                                };
                                self.push_function_call_from_obj_with_kwarg_order(
                                    &method_data.function,
                                    bound_args,
                                    kwargs,
                                    kwargs_order_opt,
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
                                let kwargs_order_opt = if kwargs_order.is_empty() {
                                    None
                                } else {
                                    Some(kwargs_order.clone())
                                };
                                let call_result = self.call_native_method_with_kwarg_order(
                                    native.kind,
                                    method_data.receiver.clone(),
                                    args,
                                    kwargs,
                                    kwargs_order_opt,
                                );
                                self.finalize_native_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                            _ => {
                                match self.call_internal(
                                    Value::BoundMethod(method.clone()),
                                    args,
                                    kwargs,
                                )? {
                                    InternalCallOutcome::Value(value) => {
                                        self.push_value(value);
                                    }
                                    InternalCallOutcome::CallerExceptionHandled => {}
                                }
                            }
                        }
                    }
                    Value::Class(class) => {
                        let kwargs_order_opt = if kwargs_order.is_empty() {
                            None
                        } else {
                            Some(kwargs_order.clone())
                        };
                        match self.call_internal_with_kwarg_order(
                            Value::Class(class),
                            args,
                            kwargs,
                            kwargs_order_opt,
                        )? {
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
                        let kwargs_order_opt = if kwargs_order.is_empty() {
                            None
                        } else {
                            Some(kwargs_order.clone())
                        };
                        let call_result = self.call_builtin_with_kwarg_order(
                            builtin,
                            args,
                            kwargs,
                            kwargs_order_opt,
                        );
                        self.finalize_builtin_opcode_call(caller_depth, caller_ip, call_result)?;
                    }
                    Value::Instance(instance) => {
                        match self.call_internal(Value::Instance(instance), args, kwargs)? {
                            InternalCallOutcome::Value(value) => self.push_value(value),
                            InternalCallOutcome::CallerExceptionHandled => {}
                        }
                    }
                    Value::ExceptionType(name) => {
                        let value = self.instantiate_exception_type(&name, &args, &kwargs)?;
                        self.push_value(value);
                    }
                    other => {
                        return Err(RuntimeError::new(format!(
                            "attempted to call non-function: {}",
                            format_value(&other)
                        )));
                    }
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
                if self.defer_dotted_import_until_parent_ready(&name, caller_idx)? {
                    return Ok(None);
                }
                let module = self.import_module_object_with_policy(
                    &name,
                    ImportReturnPolicy::DeferredWhenFramesQueued,
                )?;
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
                    frame
                        .code
                        .names
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("import name index out of range"))?
                };
                let level = {
                    let frame = self.frames.get(caller_idx).ok_or_else(|| {
                        RuntimeError::new("stack underflow (ImportNameCpython caller)")
                    })?;
                    let stack_len = frame.stack.len();
                    let level_value = frame
                        .stack
                        .get(stack_len.saturating_sub(2))
                        .cloned()
                        .ok_or_else(|| {
                            RuntimeError::new("stack underflow (ImportNameCpython level)")
                        })?;
                    value_to_int(level_value)?
                };
                if level < 0 {
                    return Err(RuntimeError::new("negative import level"));
                }
                let resolved_name = self.resolve_import_name(&name, level as usize)?;
                if self.defer_dotted_import_until_parent_ready(&resolved_name, caller_idx)? {
                    return Ok(None);
                }
                let fromlist = {
                    let frame = self.frames.get(caller_idx).ok_or_else(|| {
                        RuntimeError::new("stack underflow (ImportNameCpython fromlist)")
                    })?;
                    frame.stack.last().cloned().ok_or_else(|| {
                        RuntimeError::new("stack underflow (ImportNameCpython fromlist)")
                    })?
                };
                let module = self.import_module_object_with_policy(
                    &resolved_name,
                    ImportReturnPolicy::DeferredWhenFramesQueued,
                )?;
                let has_fromlist = self.fromlist_requested(&fromlist);
                if has_fromlist {
                    if self.frames.len() > caller_idx.saturating_add(1) {
                        if let Some(frame) = self.frames.get_mut(caller_idx) {
                            frame.ip = frame.last_ip;
                        }
                        return Ok(None);
                    }
                    if let Some(names) = self.simple_fromlist_names(&fromlist) {
                        for entry_name in names {
                            let has_attr = match self.load_attr_module(&module, &entry_name) {
                                Ok(_) => true,
                                Err(err) if is_missing_attribute_error(&err) => false,
                                Err(err) => return Err(err),
                            };
                            if has_attr {
                                continue;
                            }
                            let frames_before = self.frames.len();
                            let _ = self.load_submodule_with_policy(
                                &module,
                                &entry_name,
                                ImportReturnPolicy::DeferredWhenFramesQueued,
                            )?;
                            if self.frames.len() > frames_before {
                                if let Some(frame) = self.frames.get_mut(caller_idx) {
                                    frame.ip = frame.last_ip;
                                }
                                return Ok(None);
                            }
                        }
                    } else {
                        let frames_before = self.frames.len();
                        self.handle_import_fromlist(&module, fromlist.clone(), false)?;
                        if self.frames.len() > frames_before {
                            if let Some(frame) = self.frames.get_mut(caller_idx) {
                                frame.ip = frame.last_ip;
                            }
                            return Ok(None);
                        }
                    }
                }
                let _ = self.pop_value_from_frame(caller_idx, "ImportNameCpython fromlist")?;
                let _ = self.pop_value_from_frame(caller_idx, "ImportNameCpython level")?;
                let result_module = if has_fromlist {
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
                    frame
                        .code
                        .names
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("import name index out of range"))?
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
                        let module_name = match &*module_obj.kind() {
                            Object::Module(module_data) => module_data.name.clone(),
                            _ => String::new(),
                        };
                        let module_obj = if module_name.is_empty() {
                            module_obj
                        } else {
                            self.canonical_imported_module_for_name(&module_name, module_obj)
                        };
                        if let Some(frame) = self.frames.get_mut(caller_idx)
                            && let Some(slot) = frame.stack.last_mut()
                        {
                            *slot = Value::Module(module_obj.clone());
                        }
                        if attr_name == "*" {
                            self.import_star_into_caller_scope(
                                caller_idx,
                                Value::Module(module_obj),
                            )?;
                            self.push_value_to_caller_frame(caller_idx, Value::None)?;
                            return Ok(None);
                        }
                        let attr = self.import_from_resolve_attr(&module_obj, &attr_name)?;
                        self.push_value_to_caller_frame(caller_idx, attr)?;
                    }
                    _ => {
                        return Err(RuntimeError::new("import from expects module object"));
                    }
                }
            }
            Opcode::CallFunctionEx => {
                let kwargs_value = self.pop_value()?;
                let args_value = self.pop_value()?;
                let mut null_sentinel = self.pop_value()?;
                let mut func = self.pop_value()?;
                if matches!(func, Value::None) && !matches!(null_sentinel, Value::None) {
                    std::mem::swap(&mut func, &mut null_sentinel);
                }
                let (kwargs, kwargs_order) = match kwargs_value {
                    Value::None => (HashMap::new(), Vec::new()),
                    Value::Dict(obj) => match &*obj.kind() {
                        Object::Dict(entries) => {
                            let mut map = HashMap::new();
                            let mut order = Vec::with_capacity(entries.len());
                            for (key, value) in entries {
                                let key = match key {
                                    Value::Str(name) => name.clone(),
                                    _ => {
                                        return Err(RuntimeError::type_error(
                                            "keyword name must be string",
                                        ));
                                    }
                                };
                                if map.contains_key(&key) {
                                    return Err(RuntimeError::type_error(
                                        "duplicate keyword argument",
                                    ));
                                }
                                map.insert(key.clone(), value.clone());
                                order.push(key);
                            }
                            (map, order)
                        }
                        _ => return Err(RuntimeError::type_error("call kwargs must be dict")),
                    },
                    _ => return Err(RuntimeError::type_error("call kwargs must be dict")),
                };
                let args = match args_value {
                    Value::Tuple(obj) => match &*obj.kind() {
                        Object::Tuple(values) => values.clone(),
                        _ => return Err(RuntimeError::type_error("call args must be tuple")),
                    },
                    Value::List(obj) => match &*obj.kind() {
                        Object::List(values) => values.clone(),
                        _ => return Err(RuntimeError::type_error("call args must be tuple")),
                    },
                    other => self.collect_iterable_values(other).map_err(|err| {
                        if runtime_error_matches_exception(&err, "TypeError") {
                            RuntimeError::type_error("argument after * must be an iterable")
                        } else {
                            err
                        }
                    })?,
                };
                match func {
                    Value::Function(func) => {
                        let kwargs_order_opt = if kwargs_order.is_empty() {
                            None
                        } else {
                            Some(kwargs_order.clone())
                        };
                        self.push_function_call_from_obj_with_kwarg_order(
                            &func,
                            args,
                            kwargs,
                            kwargs_order_opt,
                        )?;
                    }
                    Value::BoundMethod(method) => {
                        let method_data = match &*method.kind() {
                            Object::BoundMethod(data) => data.clone(),
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "attempted to call non-function",
                                ));
                            }
                        };
                        match &*method_data.function.kind() {
                            Object::Function(_) => {
                                let mut bound_args = Vec::with_capacity(args.len() + 1);
                                bound_args.push(self.receiver_value(&method_data.receiver)?);
                                bound_args.extend(args);
                                let kwargs_order_opt = if kwargs_order.is_empty() {
                                    None
                                } else {
                                    Some(kwargs_order.clone())
                                };
                                self.push_function_call_from_obj_with_kwarg_order(
                                    &method_data.function,
                                    bound_args,
                                    kwargs,
                                    kwargs_order_opt,
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
                                let kwargs_order_opt = if kwargs_order.is_empty() {
                                    None
                                } else {
                                    Some(kwargs_order.clone())
                                };
                                let call_result = self.call_native_method_with_kwarg_order(
                                    native.kind,
                                    method_data.receiver.clone(),
                                    args,
                                    kwargs,
                                    kwargs_order_opt,
                                );
                                self.finalize_native_opcode_call(
                                    caller_depth,
                                    caller_ip,
                                    call_result,
                                )?;
                            }
                            _ => match self.call_internal(
                                Value::BoundMethod(method.clone()),
                                args,
                                kwargs,
                            )? {
                                InternalCallOutcome::Value(value) => {
                                    self.push_value(value);
                                }
                                InternalCallOutcome::CallerExceptionHandled => {}
                            },
                        }
                    }
                    Value::Class(class) => {
                        let kwargs_order_opt = if kwargs_order.is_empty() {
                            None
                        } else {
                            Some(kwargs_order.clone())
                        };
                        match self.call_internal_with_kwarg_order(
                            Value::Class(class),
                            args,
                            kwargs,
                            kwargs_order_opt,
                        )? {
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
                        let kwargs_order_opt = if kwargs_order.is_empty() {
                            None
                        } else {
                            Some(kwargs_order.clone())
                        };
                        let call_result = self.call_builtin_with_kwarg_order(
                            builtin,
                            args,
                            kwargs,
                            kwargs_order_opt,
                        );
                        self.finalize_builtin_opcode_call(caller_depth, caller_ip, call_result)?;
                    }
                    Value::Instance(instance) => {
                        match self.call_internal(Value::Instance(instance), args, kwargs)? {
                            InternalCallOutcome::Value(value) => self.push_value(value),
                            InternalCallOutcome::CallerExceptionHandled => {}
                        }
                    }
                    Value::ExceptionType(name) => {
                        let value = self.instantiate_exception_type(&name, &args, &kwargs)?;
                        self.push_value(value);
                    }
                    other => {
                        return Err(RuntimeError::new(format!(
                            "attempted to call non-function: {}",
                            format_value(&other)
                        )));
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
            Opcode::Copy => {
                let depth = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing COPY depth"))?
                    as usize;
                if depth == 0 {
                    return Err(RuntimeError::new("COPY expects depth >= 1"));
                }
                let value = self
                    .frames
                    .last()
                    .and_then(|frame| frame.stack.get(frame.stack.len().checked_sub(depth)?))
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("stack underflow (COPY)"))?;
                self.push_value(value);
            }
            Opcode::Swap => {
                let depth = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing SWAP depth"))?
                    as usize;
                if depth < 2 {
                    return Err(RuntimeError::new("SWAP expects depth >= 2"));
                }
                let frame = self.frames.last_mut().expect("frame exists");
                let top_index = frame.stack.len().saturating_sub(1);
                let bottom_index = frame
                    .stack
                    .len()
                    .checked_sub(depth)
                    .ok_or_else(|| RuntimeError::new("stack underflow (SWAP)"))?;
                frame.stack.swap(top_index, bottom_index);
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
                    .map_err(|_| RuntimeError::type_error("object is not iterable"))?;
                self.push_value(iterator);
            }
            Opcode::ForIter => {
                let target = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing jump target"))?
                    as usize;
                let iterator_value = self
                    .frames
                    .last()
                    .and_then(|frame| frame.stack.last())
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("stack underflow (FOR_ITER)"))?;
                match iterator_value {
                    Value::Generator(obj) => match self.generator_for_iter_next(&obj)? {
                        GeneratorResumeOutcome::Yield(value) => {
                            self.push_value(value);
                        }
                        GeneratorResumeOutcome::Complete(_) => {
                            let _ = self.pop_value()?;
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
                            self.push_value(value);
                        } else {
                            let _ = self.pop_value()?;
                            let frame = self.frames.last_mut().expect("frame exists");
                            frame.ip = target;
                        }
                    }
                    Value::Instance(instance) => {
                        let iterator = Value::Instance(instance.clone());
                        match self.next_from_iterator_value(&iterator)? {
                            GeneratorResumeOutcome::Yield(value) => {
                                self.push_value(value);
                            }
                            GeneratorResumeOutcome::Complete(_) => {
                                let _ = self.pop_value()?;
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
                                    return Err(self.runtime_error_from_active_exception(
                                        "iterator __next__ failed",
                                    ));
                                }
                                return Err(RuntimeError::new("iterator __next__ failed"));
                            }
                        }
                    }
                    _ => {
                        if self.trace_flags.for_iter_fail {
                            let (filename, function_name, line, column, ip) = self
                                .frames
                                .last()
                                .map(|frame| {
                                    let location = frame.code.locations.get(frame.last_ip);
                                    (
                                        frame.code.filename.clone(),
                                        frame.code.name.clone(),
                                        location.map(|loc| loc.line).unwrap_or(0),
                                        location.map(|loc| loc.column).unwrap_or(0),
                                        frame.last_ip,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    (
                                        "<no-frame>".to_string(),
                                        "<no-function>".to_string(),
                                        0,
                                        0,
                                        0,
                                    )
                                });
                            eprintln!(
                                "[for-iter-fail] value_type={} at {}:{}:{} ip={} fn={}",
                                self.value_type_name_for_error(&iterator_value),
                                filename,
                                line,
                                column,
                                ip,
                                function_name
                            );
                        }
                        return Err(RuntimeError::new("FOR_ITER expects iterator"));
                    }
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
                let yielded_value = yielded;
                let mut propagated_owner = owner;
                loop {
                    if let Some(caller) = self.frames.last_mut()
                        && let Some((delegated_owner_id, _target)) = caller.send_delegate_state
                        && delegated_owner_id == propagated_owner.id()
                    {
                        caller.send_delegate_state = None;
                        caller.stack.push(yielded_value);
                        return Ok(None);
                    }

                    let delegated_parent = self.frames.last().and_then(|caller| {
                        let delegated_child_matches = matches!(
                            caller.yield_from_iter.as_ref(),
                            Some(Value::Generator(child)) if child.id() == propagated_owner.id()
                        );
                        if !delegated_child_matches {
                            return None;
                        }
                        caller.generator_owner.as_ref().map(|parent_owner| {
                            (
                                parent_owner.clone(),
                                caller
                                    .generator_resume_kind
                                    .unwrap_or(GeneratorResumeKind::Next),
                            )
                        })
                    });
                    if let Some((parent_owner, parent_resume_kind)) = delegated_parent {
                        let parent_owner_id = parent_owner.id();
                        let mut parent_frame = self.frames.pop().expect("frame exists");
                        parent_frame.generator_awaiting_resume_value = false;
                        parent_frame.generator_resume_value = None;
                        parent_frame.generator_pending_throw = None;
                        parent_frame.generator_resume_kind = None;
                        self.set_generator_running(&parent_owner, false)?;
                        self.set_generator_started(&parent_owner, true)?;
                        self.generator_states.insert(parent_owner_id, parent_frame);
                        if parent_resume_kind == GeneratorResumeKind::Close {
                            return Err(RuntimeError::new("generator ignored GeneratorExit"));
                        }
                        propagated_owner = parent_owner;
                        continue;
                    }

                    if self.active_generator_resume == Some(propagated_owner.id()) {
                        self.generator_resume_outcome =
                            Some(GeneratorResumeOutcome::Yield(yielded_value));
                    } else if let Some(caller) = self.frames.last_mut() {
                        caller.stack.push(yielded_value);
                    } else {
                        return Ok(Some(Value::None));
                    }
                    return Ok(None);
                }
            }
            Opcode::Send => {
                let target = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing send target"))?
                    as usize;
                let sent = self.pop_value()?;
                let iter = self.pop_value()?;
                match self.delegate_yield_from(&iter, sent, None, GeneratorResumeKind::Next)? {
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
                    let source =
                        if frame.yield_from_iter.is_some() {
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
                    match self.to_iterator_value(source_opt.expect("source present")) {
                        Ok(iter) => iter,
                        Err(err) => {
                            let exc = self.runtime_error_to_exception_value(err);
                            self.raise_exception(exc)?;
                            return Ok(None);
                        }
                    }
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
                            return Err(RuntimeError::new("generator ignored GeneratorExit"));
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
                self.store_name("__annotations__", dict)?;
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
                        let (value, except_star_anchor) = {
                            let nearest = self.nearest_active_exception();
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
                                .or_else(|| nearest.as_ref().map(|(exc, _)| exc.clone()))
                                .ok_or_else(|| {
                                    let location = frame.code.locations.get(frame.last_ip);
                                    let line = location.map(|loc| loc.line).unwrap_or(0);
                                    let column = location.map(|loc| loc.column).unwrap_or(0);
                                    RuntimeError::new(format!(
                                        "no active exception to reraise at {}:{}:{} in {}",
                                        frame.code.filename, line, column, frame.code.name
                                    ))
                                })?;
                            let anchor = frame
                                .except_star_match_lasti
                                .or_else(|| nearest.and_then(|(_, lasti)| lasti));
                            (value, anchor)
                        };
                        if let Some(anchor_ip) = except_star_anchor
                            && let Value::Exception(exc) = &value
                            && self.exception_inherits(&exc.name, "BaseExceptionGroup")
                        {
                            if let Some(frame) = self.frames.last_mut() {
                                frame.reraise_lasti_override = Some(anchor_ip);
                            }
                        }
                        self.reraise_exception(value)?;
                    }
                    1 => {
                        if let Some(frame) = self.frames.last_mut() {
                            frame.except_star_match_lasti = None;
                        }
                        let value = self.pop_value()?;
                        self.raise_exception(value)?;
                    }
                    2 => {
                        if let Some(frame) = self.frames.last_mut() {
                            frame.except_star_match_lasti = None;
                        }
                        let cause = self.pop_value()?;
                        let value = self.pop_value()?;
                        self.raise_exception_with_cause(value, Some(cause))?;
                    }
                    _ => {
                        return Err(RuntimeError::new("invalid raise mode"));
                    }
                }
            }
            Opcode::Reraise => {
                let oparg = instr.arg.unwrap_or(0) as usize;
                if oparg > 2 {
                    return Err(RuntimeError::new("RERAISE oparg out of range"));
                }
                let exc = self.pop_value()?;
                if oparg > 0 {
                    let lasti_index = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .stack
                            .len()
                            .checked_sub(oparg)
                            .ok_or_else(|| RuntimeError::new("RERAISE stack underflow"))?
                    };
                    let lasti = {
                        let frame = self.frames.last().expect("frame exists");
                        frame
                            .stack
                            .get(lasti_index)
                            .cloned()
                            .ok_or_else(|| RuntimeError::new("RERAISE stack underflow"))?
                    };
                    let lasti = value_to_int(lasti)
                        .map_err(|_| RuntimeError::new("RERAISE expects integer lasti value"))?;
                    if lasti < 0 {
                        return Err(RuntimeError::new("RERAISE lasti must be non-negative"));
                    }
                    let frame = self.frames.last_mut().expect("frame exists");
                    frame.reraise_lasti_override = Some(lasti as usize);
                    if self.trace_flags.exception_table {
                        eprintln!(
                            "[reraise] oparg={} lasti={} current_last_ip={} next_ip={}",
                            oparg, lasti, frame.last_ip, frame.ip
                        );
                    }
                }
                let exc = self.normalize_exception_value(exc)?;
                self.unwind_exception_preserving_traceback(exc)?;
            }
            Opcode::MatchException => {
                let handler_type = self.pop_value()?;
                let mut exception = self.pop_value()?;
                let active_exception = self
                    .frames
                    .last()
                    .and_then(|frame| frame.active_exception.clone());
                let mut stack_recovered_exception: Option<Value> = None;
                if !self.value_is_exception_like(&exception)
                    && active_exception
                        .as_ref()
                        .is_none_or(|value| !self.value_is_exception_like(value))
                    && let Some(frame) = self.frames.last()
                {
                    for candidate in frame.stack.iter().rev() {
                        if self.value_is_exception_like(candidate) {
                            stack_recovered_exception = Some(candidate.clone());
                            break;
                        }
                    }
                }
                if self.trace_flags.check_exc {
                    let active_tag = active_exception
                        .as_ref()
                        .map(|value| self.value_type_name_for_error(value))
                        .unwrap_or_else(|| "None".to_string());
                    eprintln!(
                        "[match-exc-enter] exception={} handler={} active={}",
                        self.value_type_name_for_error(&exception),
                        self.value_type_name_for_error(&handler_type),
                        active_tag
                    );
                }
                if !self.value_is_exception_like(&exception)
                    && let Some(active) = active_exception
                    && self.value_is_exception_like(&active)
                {
                    exception = active;
                    if let Some(frame) = self.frames.last_mut() {
                        let top_is_exceptionish = frame
                            .stack
                            .last()
                            .map(|value| {
                                matches!(value, Value::Exception(_) | Value::ExceptionType(_))
                            })
                            .unwrap_or(false);
                        let second_is_exceptionish = frame
                            .stack
                            .get(frame.stack.len().saturating_sub(2))
                            .map(|value| {
                                matches!(value, Value::Exception(_) | Value::ExceptionType(_))
                            })
                            .unwrap_or(false);
                        let should_drop_junk = frame.stack.len() >= 2
                            && !top_is_exceptionish
                            && second_is_exceptionish;
                        if should_drop_junk {
                            frame.stack.pop();
                        }
                    }
                }
                if !self.value_is_exception_like(&exception)
                    && let Some(recovered) = stack_recovered_exception
                {
                    exception = recovered;
                }
                let replace_stack_top_exception = if self.value_is_exception_like(&exception) {
                    self.frames
                        .last()
                        .and_then(|frame| frame.stack.last())
                        .is_some_and(|top| !self.value_is_exception_like(top))
                } else {
                    false
                };
                if replace_stack_top_exception
                    && let Some(frame) = self.frames.last_mut()
                    && let Some(top) = frame.stack.last_mut()
                {
                    *top = exception.clone();
                }
                if self.trace_flags.check_exc {
                    eprintln!(
                        "[match-exc-before] exception={} handler={}",
                        self.value_type_name_for_error(&exception),
                        self.value_type_name_for_error(&handler_type)
                    );
                }
                let matches = self.exception_matches(&exception, &handler_type)?;
                self.push_value(Value::Bool(matches));
            }
            Opcode::MatchClass => {
                let positional_count = instr
                    .arg
                    .ok_or_else(|| RuntimeError::new("missing MATCH_CLASS argument"))?
                    as usize;
                let names_value = self.pop_value()?;
                let class_value = self.pop_value()?;
                let subject = self.pop_value()?;

                let keyword_names =
                    self.match_class_attr_names_from_tuple(&names_value, "MATCH_CLASS names")?;
                let isinstance_value = self.builtin_isinstance(
                    vec![subject.clone(), class_value.clone()],
                    HashMap::new(),
                )?;
                if !is_truthy(&isinstance_value) {
                    self.push_value(Value::None);
                } else {
                    let mut positional_attr_names = Vec::new();
                    let mut use_match_self = false;
                    if positional_count > 0 {
                        if let Some(match_args_value) =
                            self.optional_getattr_value(class_value.clone(), "__match_args__")?
                        {
                            let match_args = self.match_class_attr_names_from_tuple(
                                &match_args_value,
                                "__match_args__",
                            )?;
                            if positional_count > match_args.len() {
                                return Err(RuntimeError::type_error(
                                    "too many positional sub-patterns for class pattern",
                                ));
                            }
                            positional_attr_names
                                .extend(match_args.into_iter().take(positional_count));
                        } else if positional_count == 1
                            && Self::class_pattern_supports_match_self(&class_value)
                        {
                            use_match_self = true;
                        } else {
                            return Err(RuntimeError::type_error(
                                "positional sub-patterns are not supported for this class pattern",
                            ));
                        }
                    }

                    let mut seen_attrs = HashSet::new();
                    let mut matched_attrs =
                        Vec::with_capacity(positional_count + keyword_names.len());
                    let mut missing_attribute = false;

                    if use_match_self {
                        matched_attrs.push(subject.clone());
                    } else {
                        for attr_name in positional_attr_names {
                            if !seen_attrs.insert(attr_name.clone()) {
                                return Err(RuntimeError::type_error(
                                    "class pattern has duplicate attribute name",
                                ));
                            }
                            match self.optional_getattr_value(subject.clone(), &attr_name)? {
                                Some(value) => matched_attrs.push(value),
                                None => {
                                    missing_attribute = true;
                                    break;
                                }
                            }
                        }
                    }

                    if !missing_attribute {
                        for attr_name in keyword_names {
                            if !seen_attrs.insert(attr_name.clone()) {
                                return Err(RuntimeError::type_error(
                                    "class pattern has duplicate attribute name",
                                ));
                            }
                            match self.optional_getattr_value(subject.clone(), &attr_name)? {
                                Some(value) => matched_attrs.push(value),
                                None => {
                                    missing_attribute = true;
                                    break;
                                }
                            }
                        }
                    }

                    if missing_attribute {
                        self.push_value(Value::None);
                    } else {
                        self.push_value(self.heap.alloc_tuple(matched_attrs));
                    }
                }
            }
            Opcode::MatchKeys => {
                let keys = {
                    let frame = self.frames.last().expect("frame exists");
                    if frame.stack.len() < 2 {
                        return Err(RuntimeError::new("MATCH_KEYS stack underflow"));
                    }
                    frame.stack[frame.stack.len() - 1].clone()
                };
                let subject = {
                    let frame = self.frames.last().expect("frame exists");
                    frame.stack[frame.stack.len() - 2].clone()
                };
                let key_values = match keys {
                    Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                        Object::Tuple(values) => values.clone(),
                        _ => {
                            return Err(RuntimeError::type_error(
                                "MATCH_KEYS expects tuple[str] keys",
                            ));
                        }
                    },
                    _ => {
                        return Err(RuntimeError::type_error(
                            "MATCH_KEYS expects tuple[str] keys",
                        ));
                    }
                };
                let mut values = Vec::with_capacity(key_values.len());
                let mut missing_key = false;
                for key in key_values {
                    let value = match &subject {
                        Value::Dict(dict_obj) => self.dict_get_value_runtime(dict_obj, &key)?,
                        _ => match self.getitem_value(subject.clone(), key.clone()) {
                            Ok(value) => Some(value),
                            Err(err) if runtime_error_matches_exception(&err, "KeyError") => None,
                            Err(err) => return Err(err),
                        },
                    };
                    match value {
                        Some(value) => values.push(value),
                        None => {
                            missing_key = true;
                            break;
                        }
                    }
                }
                if missing_key {
                    self.push_value(Value::None);
                } else {
                    self.push_value(self.heap.alloc_tuple(values));
                }
            }
            Opcode::MatchMapping => {
                let subject = {
                    let frame = self.frames.last().expect("frame exists");
                    frame
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("MATCH_MAPPING stack underflow"))?
                };
                self.push_value(Value::Bool(Self::value_supports_mapping_pattern(&subject)));
            }
            Opcode::MatchSequence => {
                let subject = {
                    let frame = self.frames.last().expect("frame exists");
                    frame
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("MATCH_SEQUENCE stack underflow"))?
                };
                self.push_value(Value::Bool(Self::value_supports_sequence_pattern(&subject)));
            }
            Opcode::CheckExcMatch => {
                let handler_type = self.pop_value()?;
                let mut exception = self.pop_value()?;
                let active_exception = self
                    .frames
                    .last()
                    .and_then(|frame| frame.active_exception.clone());
                let mut stack_recovered_exception: Option<Value> = None;
                if !self.value_is_exception_like(&exception)
                    && active_exception
                        .as_ref()
                        .is_none_or(|value| !self.value_is_exception_like(value))
                    && let Some(frame) = self.frames.last()
                {
                    for candidate in frame.stack.iter().rev() {
                        if self.value_is_exception_like(candidate) {
                            stack_recovered_exception = Some(candidate.clone());
                            break;
                        }
                    }
                }
                if self.trace_flags.check_exc {
                    let active_tag = active_exception
                        .as_ref()
                        .map(|value| self.value_type_name_for_error(value))
                        .unwrap_or_else(|| "None".to_string());
                    eprintln!(
                        "[check-exc-enter] exception={} handler={} active={}",
                        self.value_type_name_for_error(&exception),
                        self.value_type_name_for_error(&handler_type),
                        active_tag
                    );
                }
                if !self.value_is_exception_like(&exception)
                    && let Some(active) = active_exception
                    && self.value_is_exception_like(&active)
                {
                    exception = active;
                    if let Some(frame) = self.frames.last_mut() {
                        let top_is_exceptionish = frame
                            .stack
                            .last()
                            .map(|value| {
                                matches!(value, Value::Exception(_) | Value::ExceptionType(_))
                            })
                            .unwrap_or(false);
                        let second_is_exceptionish = frame
                            .stack
                            .get(frame.stack.len().saturating_sub(2))
                            .map(|value| {
                                matches!(value, Value::Exception(_) | Value::ExceptionType(_))
                            })
                            .unwrap_or(false);
                        let should_drop_junk = frame.stack.len() >= 2
                            && !top_is_exceptionish
                            && second_is_exceptionish;
                        if should_drop_junk {
                            frame.stack.pop();
                        }
                    }
                }
                if !self.value_is_exception_like(&exception)
                    && let Some(recovered) = stack_recovered_exception
                {
                    exception = recovered;
                }
                if self.trace_flags.check_exc {
                    eprintln!(
                        "[check-exc-before-match] exception={} handler={}",
                        self.value_type_name_for_error(&exception),
                        self.value_type_name_for_error(&handler_type)
                    );
                }
                let matches = self.exception_matches(&exception, &handler_type)?;
                if self.trace_flags.exception_table {
                    eprintln!(
                        "[check-exc-match] exception={} handler={} -> {}",
                        self.value_type_name_for_error(&exception),
                        self.value_type_name_for_error(&handler_type),
                        matches
                    );
                }
                self.push_value(exception);
                self.push_value(Value::Bool(matches));
            }
            Opcode::MatchExceptionStar => {
                let handler_type = self.pop_value()?;
                let exception = self.pop_value()?;
                let (matched, remaining) =
                    self.exception_split_for_star(&exception, &handler_type)?;
                let matched_value = matched
                    .map(|exception| Value::Exception(Box::new(exception)))
                    .unwrap_or(Value::None);
                if let Some(frame) = self.frames.last_mut() {
                    frame.active_exception = match &matched_value {
                        Value::Exception(exc) => Some(Value::Exception(exc.clone())),
                        _ => frame.active_exception.clone(),
                    };
                    frame.except_star_match_lasti = match &matched_value {
                        Value::Exception(_) => Some(frame.last_ip),
                        _ => None,
                    };
                }
                self.push_value(matched_value);
                self.push_value(
                    remaining
                        .map(|exception| Value::Exception(Box::new(exception)))
                        .unwrap_or(Value::None),
                );
            }
            Opcode::ClearException => {
                if let Some(frame) = self.frames.last_mut() {
                    frame.active_exception = None;
                    frame.except_star_match_lasti = None;
                }
            }
            Opcode::PushExcInfo => {
                let exc = self.pop_value()?;
                let previous = if let Some(frame) = self.frames.last_mut() {
                    let previous = frame.active_exception.clone().unwrap_or(Value::None);
                    frame.active_exception = Some(exc.clone());
                    previous
                } else {
                    Value::None
                };
                self.push_value(previous);
                self.push_value(exc);
            }
            Opcode::PopExcept => {
                let exc_value = self.pop_value()?;
                if let Some(frame) = self.frames.last_mut() {
                    frame.active_exception = if matches!(exc_value, Value::None) {
                        None
                    } else {
                        Some(exc_value)
                    };
                    frame.except_star_match_lasti = None;
                }
            }
            Opcode::WithExceptStart => {
                let (exit_func, exit_self, value) = {
                    let frame = self.frames.last().expect("frame exists");
                    if frame.stack.len() < 5 {
                        return Err(RuntimeError::new("WITH_EXCEPT_START stack underflow"));
                    }
                    let top = frame.stack.len();
                    (
                        frame.stack[top - 5].clone(),
                        frame.stack[top - 4].clone(),
                        frame.stack[top - 1].clone(),
                    )
                };
                let exc_type = match &value {
                    Value::Exception(exc) => Value::ExceptionType(exc.name.clone()),
                    Value::ExceptionType(name) => Value::ExceptionType(name.clone()),
                    _ => {
                        return Err(RuntimeError::new(
                            "WITH_EXCEPT_START expects exception value on stack",
                        ));
                    }
                };
                let mut args = vec![exc_type, value, Value::None];
                if !matches!(exit_self, Value::None) {
                    args.insert(0, exit_self);
                }
                match self.call_internal(exit_func, args, HashMap::new())? {
                    InternalCallOutcome::Value(result) => self.push_value(result),
                    InternalCallOutcome::CallerExceptionHandled => {}
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
                let value = {
                    let frame = self.frames.last().expect("frame exists");
                    frame
                        .code
                        .constants
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| RuntimeError::new("constant index out of range"))?
                };
                let mut frame = self.frames.pop().expect("frame exists");
                if let Some(module_dict) = frame.module_locals_dict.take() {
                    self.sync_module_locals_dict_to_module(&frame.module, &module_dict);
                }
                self.finalize_module_frame_success(&frame);
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
                        frame.class_orig_bases,
                        frame.class_metaclass,
                        frame.class_keywords,
                        frame.class_namespace,
                        Some(frame.function_globals.clone()),
                        frame.locals_fallback.clone(),
                        frame.code.future_annotations_import,
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
                #[cfg(not(debug_assertions))]
                let simple_fast_return = {
                    if self.frames.len() <= 1 {
                        false
                    } else {
                        let frame = self.frames.last().expect("frame exists");
                        frame.simple_one_arg_no_cells
                            && frame.stack.len() == 1
                            && frame.locals.is_empty()
                            && frame.cells.is_empty()
                            && frame.blocks.is_empty()
                            && frame.class_bases.is_empty()
                            && frame.class_keywords.is_empty()
                            && frame.module_locals_dict.is_none()
                            && frame.globals_fallback.is_none()
                            && frame.locals_fallback.is_none()
                            && frame.class_metaclass.is_none()
                            && frame.class_namespace.is_none()
                            && frame.generator_owner.is_none()
                            && !frame.generator_awaiting_resume_value
                            && frame.generator_resume_value.is_none()
                            && frame.generator_pending_throw.is_none()
                            && frame.generator_resume_kind.is_none()
                            && frame.yield_from_iter.is_none()
                            && frame.reraise_lasti_override.is_none()
                            && frame.except_star_match_lasti.is_none()
                            && !frame.discard_result
                            && frame.active_exception.is_none()
                            && !frame.return_class
                            && frame.return_instance.is_none()
                            && !frame.return_module
                            && !frame.is_module
                            && !frame.expect_none_return
                    }
                };
                #[cfg(debug_assertions)]
                let simple_fast_return = {
                    if self.frames.len() <= 1 {
                        false
                    } else {
                        let frame = self.frames.last().expect("frame exists");
                        frame.simple_one_arg_no_cells
                            && frame.stack.len() == 1
                            && frame.locals.is_empty()
                            && frame.cells.is_empty()
                            && frame.blocks.is_empty()
                            && frame.class_bases.is_empty()
                            && frame.class_keywords.is_empty()
                            && frame.code.fast_local_count == 1
                            && frame.code.plain_positional_arg0_slot == Some(0)
                            && !frame.is_module
                            && !frame.discard_result
                            && frame.module_locals_dict.is_none()
                            && frame.globals_fallback.is_none()
                            && frame.locals_fallback.is_none()
                            && frame.active_exception.is_none()
                            && frame.except_star_match_lasti.is_none()
                            && frame.reraise_lasti_override.is_none()
                            && !frame.return_class
                            && frame.return_instance.is_none()
                            && !frame.return_module
                            && !frame.expect_none_return
                            && frame.class_metaclass.is_none()
                            && frame.class_namespace.is_none()
                            && !frame.generator_awaiting_resume_value
                            && frame.generator_resume_value.is_none()
                            && frame.generator_pending_throw.is_none()
                            && frame.generator_resume_kind.is_none()
                            && frame.yield_from_iter.is_none()
                            && frame.generator_owner.is_none()
                    }
                };
                if simple_fast_return {
                    #[cfg(debug_assertions)]
                    {
                        let frame = self.frames.last().expect("frame exists");
                        debug_assert!(frame.locals.is_empty());
                        debug_assert!(frame.cells.is_empty());
                        debug_assert!(frame.class_bases.is_empty());
                        debug_assert!(frame.class_keywords.is_empty());
                        debug_assert!(frame.globals_fallback.is_none());
                        debug_assert!(frame.locals_fallback.is_none());
                        debug_assert!(frame.class_metaclass.is_none());
                        debug_assert!(!frame.generator_awaiting_resume_value);
                        debug_assert!(frame.generator_resume_value.is_none());
                        debug_assert!(frame.generator_pending_throw.is_none());
                        debug_assert!(frame.generator_resume_kind.is_none());
                        debug_assert!(frame.yield_from_iter.is_none());
                    }
                    let value = {
                        let frame = self.frames.last_mut().expect("frame exists");
                        debug_assert!(frame.stack.len() == 1);
                        // Safety: guarded by `simple_fast_return` and debug assertion above.
                        unsafe { frame.stack.pop().unwrap_unchecked() }
                    };
                    let frame = self.frames.pop().expect("frame exists");
                    let caller = self.frames.last_mut().expect("caller frame exists");
                    caller.stack.push(value);
                    if frame.owner_class.is_none() {
                        self.recycle_simple_frame_clean_slot0_unchecked(frame);
                    } else {
                        self.recycle_simple_frame(frame);
                    }
                    return Ok(None);
                }
                let value = {
                    let frame = self.frames.last_mut().expect("frame exists");
                    frame.stack.pop().unwrap_or(Value::None)
                };
                let mut frame = self.frames.pop().expect("frame exists");
                if let Some(filter) = self.trace_text_filters.module_return_ip.as_ref()
                    && frame.is_module
                    && frame.code.filename.contains(filter)
                {
                    eprintln!(
                        "[module-return-op] file={} last_ip={} instr_len={} active_exc={} blocks={}",
                        frame.code.filename,
                        frame.last_ip,
                        frame.code.instructions.len(),
                        frame.active_exception.is_some(),
                        frame.blocks.len()
                    );
                }
                if let Some(module_dict) = frame.module_locals_dict.take() {
                    self.sync_module_locals_dict_to_module(&frame.module, &module_dict);
                }
                self.finalize_module_frame_success(&frame);
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
                        frame.class_orig_bases,
                        frame.class_metaclass,
                        frame.class_keywords,
                        frame.class_namespace,
                        Some(frame.function_globals.clone()),
                        frame.locals_fallback.clone(),
                        frame.code.future_annotations_import,
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
    }

    pub(super) fn raise_exception(&mut self, value: Value) -> Result<(), RuntimeError> {
        let _debug_depth_guard =
            DebugDepthGuard::enter_for_vm(self, &DEBUG_RAISE_DEPTH, "raise_exception");
        self.raise_exception_with_cause(value, None)
    }

    fn reraise_exception(&mut self, value: Value) -> Result<(), RuntimeError> {
        let exc = self.normalize_exception_value(value)?;
        self.unwind_exception_preserving_traceback(exc)
    }

    fn exception_handler_for_ip(
        frame: &Frame,
        instruction_ip: usize,
    ) -> Option<(usize, usize, bool)> {
        frame
            .code
            .exception_handlers
            .iter()
            .find(|entry| entry.start <= instruction_ip && instruction_ip < entry.end)
            .map(|entry| (entry.target, entry.depth, entry.push_lasti))
    }

    fn unwind_exception(&mut self, exc: Value) -> Result<(), RuntimeError> {
        self.unwind_exception_internal(exc, false)
    }

    fn unwind_exception_preserving_traceback(&mut self, exc: Value) -> Result<(), RuntimeError> {
        self.unwind_exception_internal(exc, true)
    }

    fn existing_traceback_frames(exception_value: &Value) -> Vec<TraceFrame> {
        let Value::Exception(exception) = exception_value else {
            return Vec::new();
        };
        exception
            .traceback_frames
            .iter()
            .map(|frame| TraceFrame {
                frame_id: frame.frame_id,
                filename: frame.filename.clone(),
                line: frame.line,
                column: frame.column,
                end_line: frame.end_line,
                end_column: frame.end_column,
                lasti: frame.lasti,
                name: frame.name.clone(),
                locals: frame.locals.clone(),
                local_values: frame.local_values.clone(),
                globals: frame.globals.clone(),
                self_local: frame.self_local.clone(),
            })
            .collect()
    }

    fn traceback_tail_matches_frame(traceback: &[TraceFrame], frame: &TraceFrame) -> bool {
        let Some(last) = traceback.last() else {
            return false;
        };
        if last.frame_id != 0 && frame.frame_id != 0 {
            return last.frame_id == frame.frame_id && last.lasti == frame.lasti;
        }
        last == frame
    }

    fn push_traceback_frame(traceback: &mut Vec<TraceFrame>, frame: TraceFrame) -> bool {
        if Self::traceback_tail_matches_frame(traceback, &frame) {
            return false;
        }
        traceback.push(frame);
        true
    }

    pub(super) fn traceback_value_from_frames(
        &mut self,
        frames: &[ExceptionTracebackFrame],
    ) -> Value {
        if frames.is_empty() {
            return Value::None;
        }
        let traceback_class =
            if let Some(class) = self.types_module_or_private_class("TracebackType") {
                class
            } else {
                match self
                    .heap
                    .alloc_class(ClassObject::new("traceback".to_string(), Vec::new()))
                {
                    Value::Class(class) => class,
                    _ => unreachable!(),
                }
            };
        let frame_class = if let Some(class) = self.types_module_or_private_class("FrameType") {
            class
        } else {
            match self
                .heap
                .alloc_class(ClassObject::new("frame".to_string(), Vec::new()))
            {
                Value::Class(class) => class,
                _ => unreachable!(),
            }
        };
        let mut next = Value::None;
        for frame in frames.iter() {
            let mut resolved_end_line = frame.end_line;
            let mut resolved_end_column = frame.end_column;
            if frame.line != 0
                && frame.column != 0
                && resolved_end_line == 0
                && resolved_end_column == 0
                && let Some(source_line) = self.traceback_source_line(&frame.filename, frame.line)
            {
                let start_col = frame.column.saturating_sub(1);
                if start_col <= source_line.len() && source_line.is_char_boundary(start_col) {
                    let segment = source_line[start_col..].trim_end();
                    let prefix = source_line[..start_col].trim_start();
                    let is_raise_expression = prefix == "raise" || prefix.starts_with("raise ");
                    let looks_like_call_span = segment.ends_with(')')
                        && !segment.contains('#')
                        && segment
                            .chars()
                            .next()
                            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic());
                    if looks_like_call_span && !is_raise_expression {
                        resolved_end_line = frame.line;
                        resolved_end_column = start_col
                            .saturating_add(segment.chars().count())
                            .saturating_add(1);
                    } else {
                        let inferred_end_line = self.infer_traceback_continuation_end_line(
                            &frame.filename,
                            frame.line,
                            &source_line,
                        );
                        if inferred_end_line > frame.line {
                            resolved_end_line = inferred_end_line;
                            if let Some(end_source_line) =
                                self.traceback_source_line(&frame.filename, inferred_end_line)
                            {
                                resolved_end_column =
                                    end_source_line.chars().count().saturating_add(1);
                            }
                        }
                    }
                    if resolved_end_line == 0 && resolved_end_column == 0 {
                        let has_operator = segment.chars().any(|ch| {
                            matches!(
                                ch,
                                '+' | '-' | '*' | '/' | '%' | '@' | '&' | '|' | '^' | '<' | '>'
                            )
                        });
                        if has_operator && !segment.contains('#') {
                            resolved_end_line = frame.line;
                            resolved_end_column = source_line.chars().count().saturating_add(1);
                        }
                    }
                }
            }
            if frame.line != 0
                && frame.column < resolved_end_column
                && resolved_end_line == frame.line
                && let Some(source_line) = self.traceback_source_line(&frame.filename, frame.line)
                && let Some(clipped_end) =
                    clip_short_circuit_end_column(&source_line, frame.column, resolved_end_column)
            {
                resolved_end_column = clipped_end;
            }
            if resolved_end_line > frame.line
                && resolved_end_column == 0
                && let Some(source_line) =
                    self.traceback_source_line(&frame.filename, resolved_end_line)
            {
                resolved_end_column = source_line.chars().count().saturating_add(1);
            }

            let mut code = CodeObject::new(frame.name.clone(), frame.filename.clone());
            code.first_line = frame.line.max(1);
            let instruction_count = frame.lasti.saturating_div(2).saturating_add(1).max(1);
            for _ in 0..instruction_count {
                code.instructions.push(Instruction::new(Opcode::Nop, None));
                code.locations.push(Location::with_end(
                    frame.line,
                    frame.column,
                    resolved_end_line,
                    resolved_end_column,
                ));
            }
            let code_rc = Rc::new(code);
            if !self.linecache_registered_sources.contains(&frame.filename)
                && let Some(lines) = self.source_text_cache.get(&frame.filename)
            {
                let source = lines.join("\n");
                self.register_source_in_linecache(code_rc.as_ref(), &source, &frame.filename);
            }
            let code_value = Value::Code(code_rc);

            let mut frame_instance = InstanceObject::new(frame_class.clone());
            let effective_local_values = if frame.frame_id != 0 {
                self.frames
                    .iter()
                    .find(|active| active.frame_id == frame.frame_id)
                    .map(|active| Self::frame_trace(active).local_values)
                    .unwrap_or_else(|| frame.local_values.clone())
            } else {
                frame.local_values.clone()
            };
            let mut seen_local_names = HashSet::new();
            let mut frame_locals = Vec::new();
            for (name, value) in effective_local_values {
                if seen_local_names.insert(name.clone()) {
                    frame_locals.push((Value::Str(name), value));
                }
            }
            for name in &frame.locals {
                if seen_local_names.insert(name.clone()) {
                    frame_locals.push((Value::Str(name.clone()), Value::None));
                }
            }
            let frame_globals = frame
                .globals
                .iter()
                .cloned()
                .map(|name| (Value::Str(name), Value::None))
                .collect::<Vec<_>>();
            if let Some(self_value) = &frame.self_local {
                let mut has_self = false;
                for (key, value) in &mut frame_locals {
                    if matches!(key, Value::Str(name) if name == "self") {
                        *value = self_value.clone();
                        has_self = true;
                        break;
                    }
                }
                if !has_self {
                    frame_locals.push((Value::Str("self".to_string()), self_value.clone()));
                }
            }
            frame_instance
                .attrs
                .insert("f_code".to_string(), code_value);
            frame_instance
                .attrs
                .insert("f_globals".to_string(), self.heap.alloc_dict(frame_globals));
            frame_instance
                .attrs
                .insert("f_locals".to_string(), self.heap.alloc_dict(frame_locals));
            frame_instance.attrs.insert(
                "f_builtins".to_string(),
                self.builtins_mapping_value_from_dunder_builtins(None),
            );
            frame_instance
                .attrs
                .insert("f_lineno".to_string(), Value::Int(frame.line as i64));
            frame_instance
                .attrs
                .insert("f_back".to_string(), Value::None);
            let frame_value = self.heap.alloc_instance(frame_instance);

            let mut instance = InstanceObject::new(traceback_class.clone());
            instance
                .attrs
                .insert("__pyrs_traceback_marker__".to_string(), Value::Bool(true));
            instance.attrs.insert(
                "__pyrs_tb_filename__".to_string(),
                Value::Str(frame.filename.clone()),
            );
            instance.attrs.insert(
                "__pyrs_tb_name__".to_string(),
                Value::Str(frame.name.clone()),
            );
            instance.attrs.insert(
                "__pyrs_tb_column__".to_string(),
                Value::Int(frame.column as i64),
            );
            instance.attrs.insert(
                "__pyrs_tb_end_line__".to_string(),
                Value::Int(resolved_end_line as i64),
            );
            instance.attrs.insert(
                "__pyrs_tb_end_column__".to_string(),
                Value::Int(resolved_end_column as i64),
            );
            instance.attrs.insert(
                "__pyrs_tb_frame_id__".to_string(),
                Value::Int(frame.frame_id as i64),
            );
            instance
                .attrs
                .insert("tb_lineno".to_string(), Value::Int(frame.line as i64));
            instance
                .attrs
                .insert("tb_lasti".to_string(), Value::Int(frame.lasti as i64));
            instance.attrs.insert("tb_frame".to_string(), frame_value);
            instance.attrs.insert("tb_next".to_string(), next.clone());
            next = self.heap.alloc_instance(instance);
        }
        next
    }

    pub(super) fn traceback_frames_from_value(
        &self,
        value: Value,
    ) -> Result<Option<Vec<ExceptionTracebackFrame>>, RuntimeError> {
        match value {
            Value::None => Ok(None),
            Value::Instance(instance) => {
                let mut frames = Vec::new();
                let mut current = Some(instance);
                let mut seen_ids = HashSet::new();
                while let Some(node) = current {
                    let node_id = node.id();
                    if !seen_ids.insert(node_id) {
                        return Err(RuntimeError::type_error(
                            "__traceback__ must be a traceback or None",
                        ));
                    }
                    let node_ref = node.kind();
                    let Object::Instance(node_data) = &*node_ref else {
                        return Err(RuntimeError::type_error(
                            "__traceback__ must be a traceback or None",
                        ));
                    };
                    match node_data.attrs.get("__pyrs_traceback_marker__") {
                        Some(Value::Bool(true)) => {}
                        _ => {
                            return Err(RuntimeError::type_error(
                                "__traceback__ must be a traceback or None",
                            ));
                        }
                    }
                    let filename = match node_data.attrs.get("__pyrs_tb_filename__") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => "<unknown>".to_string(),
                    };
                    let name = match node_data.attrs.get("__pyrs_tb_name__") {
                        Some(Value::Str(value)) => value.clone(),
                        _ => "<module>".to_string(),
                    };
                    let line = match node_data.attrs.get("tb_lineno") {
                        Some(Value::Int(value)) if *value >= 0 => *value as usize,
                        _ => 0,
                    };
                    let column = match node_data.attrs.get("__pyrs_tb_column__") {
                        Some(Value::Int(value)) if *value >= 0 => *value as usize,
                        _ => 0,
                    };
                    let end_line = match node_data.attrs.get("__pyrs_tb_end_line__") {
                        Some(Value::Int(value)) if *value >= 0 => *value as usize,
                        _ => 0,
                    };
                    let end_column = match node_data.attrs.get("__pyrs_tb_end_column__") {
                        Some(Value::Int(value)) if *value >= 0 => *value as usize,
                        _ => 0,
                    };
                    let lasti = match node_data.attrs.get("tb_lasti") {
                        Some(Value::Int(value)) if *value >= 0 => *value as usize,
                        _ => 0,
                    };
                    let frame_id = match node_data.attrs.get("__pyrs_tb_frame_id__") {
                        Some(Value::Int(value)) if *value >= 0 => *value as usize,
                        _ => 0,
                    };
                    let frame_attr_items = |dict_name: &str| -> Vec<(String, Value)> {
                        match node_data.attrs.get("tb_frame") {
                            Some(Value::Instance(tb_frame)) => match &*tb_frame.kind() {
                                Object::Instance(tb_frame_data) => {
                                    match tb_frame_data.attrs.get(dict_name) {
                                        Some(Value::Dict(dict_obj)) => match &*dict_obj.kind() {
                                            Object::Dict(entries) => entries
                                                .iter()
                                                .filter_map(|(key, value)| match key {
                                                    Value::Str(name) => {
                                                        Some((name.clone(), value.clone()))
                                                    }
                                                    _ => None,
                                                })
                                                .collect(),
                                            _ => Vec::new(),
                                        },
                                        _ => Vec::new(),
                                    }
                                }
                                _ => Vec::new(),
                            },
                            _ => Vec::new(),
                        }
                    };
                    let local_values = frame_attr_items("f_locals");
                    let globals_items = frame_attr_items("f_globals");
                    let locals = local_values
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>();
                    let globals = globals_items
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>();
                    let self_local = local_values.iter().find_map(|(name, value)| {
                        if name == "self" {
                            Some(value.clone())
                        } else {
                            None
                        }
                    });
                    frames.push(ExceptionTracebackFrame {
                        frame_id,
                        filename,
                        line,
                        column,
                        end_line,
                        end_column,
                        lasti,
                        name,
                        locals,
                        local_values,
                        globals,
                        self_local,
                    });
                    current = match node_data.attrs.get("tb_next") {
                        Some(Value::None) | None => None,
                        Some(Value::Instance(next)) => Some(next.clone()),
                        _ => {
                            return Err(RuntimeError::type_error(
                                "__traceback__ must be a traceback or None",
                            ));
                        }
                    };
                }
                frames.reverse();
                Ok(Some(frames))
            }
            _ => Err(RuntimeError::type_error(
                "__traceback__ must be a traceback or None",
            )),
        }
    }

    /// Core exception propagation routine.
    ///
    /// Walks frame/block stacks until a handler is found, while incrementally
    /// extending traceback frames in CPython order. When no handler remains,
    /// produces an unhandled `RuntimeError` that preserves the exception object.
    fn unwind_exception_internal(
        &mut self,
        mut exc: Value,
        preserve_existing_traceback: bool,
    ) -> Result<(), RuntimeError> {
        let _debug_depth_guard =
            DebugDepthGuard::enter_for_vm(self, &DEBUG_UNWIND_DEPTH, "unwind_exception");
        let mut traceback = Self::existing_traceback_frames(&exc);
        let mut skip_current_frame_trace = preserve_existing_traceback && !traceback.is_empty();
        let mut traceback_dirty = false;
        loop {
            let frame_depth = self.frames.len();
            let Some(frame) = self.frames.last_mut() else {
                if traceback_dirty {
                    Self::attach_traceback_to_exception(&mut exc, &traceback);
                }
                let message = self.format_traceback(&traceback, &exc);
                return Err(self.runtime_error_from_unhandled_exception(message, &exc));
            };

            if let Some(stop_depth) = self.run_stop_depth
                && frame_depth <= stop_depth
            {
                if !skip_current_frame_trace {
                    let current_frame_trace = Self::frame_trace(frame);
                    traceback_dirty |=
                        Self::push_traceback_frame(&mut traceback, current_frame_trace);
                }
                if traceback_dirty {
                    Self::attach_traceback_to_exception(&mut exc, &traceback);
                }
                // Stop-depth calls (`run_pending_import_frames`) must not consume
                // caller handlers/blocks; leave the frame intact and bubble the
                // pending exception back to the outer run loop.
                frame.active_exception = Some(exc.clone());
                let message = self.format_traceback(&traceback, &exc);
                return Err(self.runtime_error_from_unhandled_exception(message, &exc));
            }

            if skip_current_frame_trace {
                skip_current_frame_trace = false;
            } else {
                traceback_dirty |=
                    Self::push_traceback_frame(&mut traceback, Self::frame_trace(frame));
            }

            if let Some(block) = frame.blocks.pop() {
                if traceback_dirty {
                    Self::attach_traceback_to_exception(&mut exc, &traceback);
                }
                if let Some(filter) = self.trace_text_filters.unwind.as_ref()
                    && frame.code.filename.contains(filter)
                {
                    let location = frame.code.locations.get(frame.last_ip);
                    eprintln!(
                        "[unwind-block] file={} fn={} line={} col={} ip={} handler={} exc={:?}",
                        frame.code.filename,
                        frame.code.name,
                        location.map(|loc| loc.line).unwrap_or(0),
                        location.map(|loc| loc.column).unwrap_or(0),
                        frame.last_ip,
                        block.handler,
                        exc
                    );
                }
                Self::clear_stale_yield_from_on_handler_entry(frame);
                frame.stack.truncate(block.stack_len);
                frame.stack.push(exc.clone());
                frame.ip = block.handler;
                frame.reraise_lasti_override = None;
                frame.active_exception = Some(exc.clone());
                frame.except_star_match_lasti = None;
                return Ok(());
            }

            if let Some((target, depth, push_lasti)) =
                Self::exception_handler_for_ip(frame, frame.last_ip)
            {
                if traceback_dirty {
                    Self::attach_traceback_to_exception(&mut exc, &traceback);
                }
                if self.trace_flags.exception_table {
                    eprintln!(
                        "[exc-table] last_ip={} ip={} -> target={} depth={} push_lasti={} stack_len={}",
                        frame.last_ip,
                        frame.ip,
                        target,
                        depth,
                        push_lasti,
                        frame.stack.len()
                    );
                }
                Self::clear_stale_yield_from_on_handler_entry(frame);
                if frame.stack.len() > depth {
                    frame.stack.truncate(depth);
                }
                let lasti_to_push = frame.reraise_lasti_override.take().unwrap_or(frame.last_ip);
                if push_lasti {
                    frame.stack.push(Value::Int(lasti_to_push as i64));
                }
                frame.stack.push(exc.clone());
                frame.active_exception = Some(exc.clone());
                frame.except_star_match_lasti = None;
                frame.ip = target;
                return Ok(());
            }

            if let Some(boundary) = self.active_generator_resume_boundary
                && frame_depth <= boundary
            {
                if traceback_dirty {
                    Self::attach_traceback_to_exception(&mut exc, &traceback);
                }
                self.pending_generator_exception = Some(exc.clone());
                self.generator_resume_outcome = Some(GeneratorResumeOutcome::PropagatedException);
                return Ok(());
            }

            let frame = self.frames.pop().expect("frame exists");
            if self.cleanup_failed_module_frame(&frame)? {
                return Ok(());
            }
            if let Some(owner) = frame.generator_owner {
                if let Some(caller) = self.frames.last_mut()
                    && let Some((delegated_owner_id, _)) = caller.send_delegate_state
                    && delegated_owner_id == owner.id()
                {
                    caller.send_delegate_state = None;
                }
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

    #[inline]
    fn clear_stale_yield_from_on_handler_entry(frame: &mut Frame) {
        let current_opcode = frame
            .code
            .instructions
            .get(frame.last_ip)
            .map(|instr| instr.opcode);
        // Only clear delegated-yield state when the exception was raised from
        // `YIELD_FROM`/`SEND` handling itself. This fixes stale await delegates
        // while avoiding unrelated handler-path regressions.
        if matches!(current_opcode, Some(Opcode::YieldFrom | Opcode::Send)) {
            frame.yield_from_iter = None;
        }
    }

    fn runtime_error_from_unhandled_exception(
        &self,
        message: String,
        exception_value: &Value,
    ) -> RuntimeError {
        let exception = match exception_value {
            Value::Exception(exception) => Some(Box::new((**exception).clone())),
            Value::ExceptionType(name) => Some(Box::new(ExceptionObject::new(name.clone(), None))),
            _ => None,
        };
        RuntimeError { message, exception }
    }

    fn attach_traceback_to_exception(exception_value: &mut Value, frames: &[TraceFrame]) {
        let Value::Exception(exception) = exception_value else {
            return;
        };
        exception.traceback_frames = frames
            .iter()
            .map(|frame| ExceptionTracebackFrame {
                frame_id: frame.frame_id,
                filename: frame.filename.clone(),
                line: frame.line,
                column: frame.column,
                end_line: frame.end_line,
                end_column: frame.end_column,
                lasti: frame.lasti,
                name: frame.name.clone(),
                locals: frame.locals.clone(),
                local_values: frame.local_values.clone(),
                globals: frame.globals.clone(),
                self_local: frame.self_local.clone(),
            })
            .collect::<Vec<_>>()
            .into();
        exception.attrs.borrow_mut().remove("__traceback__");
    }

    /// Raise an exception value, attaching implicit context and optional
    /// explicit cause per CPython chaining rules.
    pub(super) fn raise_exception_with_cause(
        &mut self,
        value: Value,
        explicit_cause: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let mut exc = self.normalize_exception_value(value)?;
        if self.trace_flags.assert_raise {
            let is_assertion = match &exc {
                Value::Exception(exception) => exception.name == "AssertionError",
                Value::ExceptionType(name) => name == "AssertionError",
                _ => false,
            };
            if is_assertion {
                let (filename, func_name, ip, line, column) =
                    if let Some(frame) = self.frames.last() {
                        let location = frame.code.locations.get(frame.last_ip);
                        (
                            frame.code.filename.clone(),
                            frame.code.name.clone(),
                            frame.last_ip as i64,
                            location.map(|loc| loc.line).unwrap_or(0),
                            location.map(|loc| loc.column).unwrap_or(0),
                        )
                    } else {
                        ("<no-frame>".to_string(), "<no-frame>".to_string(), 0, 0, 0)
                    };
                let summary = match &exc {
                    Value::Exception(exception) => exception
                        .message
                        .clone()
                        .unwrap_or_else(|| "<no-message>".to_string()),
                    _ => "<non-instance>".to_string(),
                };
                eprintln!(
                    "[assert-raise] file={} func={} ip={} line={} col={} msg={}",
                    filename, func_name, ip, line, column, summary
                );
            }
        }
        if let Value::Exception(exc_data) = &mut exc {
            let implicit_context = self
                .frames
                .last()
                .and_then(|frame| frame.active_exception.clone())
                .map(|current| self.normalize_exception_value(current))
                .transpose()?;
            if let Some(Value::Exception(context_data)) = implicit_context
                && exc_data.context.is_none()
                && context_data.object_id != exc_data.object_id
            {
                exc_data.context = Some(context_data);
            }
            if let Some(cause_value) = explicit_cause {
                if matches!(cause_value, Value::None) {
                    exc_data.suppress_context = true;
                    exc_data.cause = None;
                } else {
                    let cause = self.normalize_exception_value(cause_value)?;
                    if let Value::Exception(cause_data) = cause {
                        exc_data.cause = Some(cause_data);
                        exc_data.suppress_context = true;
                    }
                }
            }
        }
        self.unwind_exception(exc)
    }

    /// Convert internal `RuntimeError` values into VM exception propagation.
    ///
    /// If the runtime error already contains a concrete exception object it is
    /// reused; otherwise a `RuntimeError` instance is synthesized.
    pub(super) fn handle_runtime_error(&mut self, err: RuntimeError) -> Result<(), RuntimeError> {
        let _debug_depth_guard = DebugDepthGuard::enter_for_vm(
            self,
            &DEBUG_HANDLE_RUNTIME_DEPTH,
            "handle_runtime_error",
        );
        if self.frames.is_empty() {
            return Err(err);
        }
        let RuntimeError { message, exception } = err;
        if self.trace_flags.import_pending {
            let active = self
                .frames
                .last()
                .and_then(|frame| frame.active_exception.as_ref())
                .map(|value| self.value_type_name_for_error(value))
                .unwrap_or_else(|| "<none>".to_string());
            let (last_ip, handler) = self
                .frames
                .last()
                .map(|frame| {
                    (
                        frame.last_ip,
                        Self::exception_handler_for_ip(frame, frame.last_ip),
                    )
                })
                .unwrap_or((0, None));
            let (file, function_name, opcode_name) = self
                .frames
                .last()
                .map(|frame| {
                    let opcode_name = frame
                        .code
                        .instructions
                        .get(frame.last_ip)
                        .map(|instr| format!("{:?}", instr.opcode))
                        .unwrap_or_else(|| "<unknown>".to_string());
                    (
                        frame.code.filename.clone(),
                        frame.code.name.clone(),
                        opcode_name,
                    )
                })
                .unwrap_or_else(|| {
                    (
                        "<no-frame>".to_string(),
                        "<no-frame>".to_string(),
                        "<unknown>".to_string(),
                    )
                });
            let block_len = self
                .frames
                .last()
                .map(|frame| frame.blocks.len())
                .unwrap_or(0);
            let stack = self
                .frames
                .iter()
                .rev()
                .take(8)
                .map(|frame| {
                    let line = frame
                        .code
                        .locations
                        .get(frame.last_ip)
                        .map(|loc| loc.line)
                        .unwrap_or(0);
                    format!("{}:{}@{}", frame.code.name, line, frame.code.filename)
                })
                .collect::<Vec<_>>()
                .join(" <- ");
            eprintln!("[handle-runtime] msg={}", message);
            eprintln!("[handle-runtime] active={}", active);
            eprintln!(
                "[handle-runtime] file={} fn={} ip={} opcode={} handler={handler:?} blocks={block_len} frames={}",
                file,
                function_name,
                last_ip,
                opcode_name,
                self.frames.len(),
            );
            eprintln!("[handle-runtime] stack={stack}");
        }
        if let Some(exception) = exception {
            let exception = *exception;
            self.ensure_exception_default_attrs(&exception);
            return self.raise_exception(Value::Exception(Box::new(exception)));
        }
        let err = RuntimeError {
            message,
            exception: None,
        };
        let exception = self.runtime_error_to_exception_object(err);
        self.raise_exception(Value::Exception(Box::new(exception)))
    }

    pub(super) fn normalize_exception_value(&self, value: Value) -> Result<Value, RuntimeError> {
        match value {
            Value::Exception(_) => Ok(value),
            Value::ExceptionType(name) => {
                let exception = ExceptionObject::new(name, None);
                exception
                    .attrs
                    .borrow_mut()
                    .insert("args".to_string(), self.heap.alloc_tuple(Vec::new()));
                Ok(Value::Exception(Box::new(exception)))
            }
            Value::Class(class) => {
                if self.class_is_exception_class(&class) {
                    let class_name = match &*class.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "Exception".to_string(),
                    };
                    let exception = ExceptionObject::new(class_name, None);
                    exception
                        .attrs
                        .borrow_mut()
                        .insert("__class__".to_string(), Value::Class(class.clone()));
                    exception
                        .attrs
                        .borrow_mut()
                        .insert("args".to_string(), self.heap.alloc_tuple(Vec::new()));
                    Ok(Value::Exception(Box::new(exception)))
                } else {
                    Err(RuntimeError::new("can only raise Exception types"))
                }
            }
            Value::Instance(instance) => {
                let class_name = self
                    .exception_class_name_for_instance(&instance)
                    .ok_or_else(|| RuntimeError::new("can only raise Exception types"))?;
                let message = self.exception_message_for_instance(&instance);
                let mut exception = ExceptionObject::new(class_name.clone(), message);
                // Preserve exception identity across repeated raises of the same
                // exception instance (for traceback cycle detection semantics).
                exception.object_id = instance.id() | (1u64 << 63);
                if let Object::Instance(instance_data) = &*instance.kind()
                    && !instance_data.attrs.is_empty()
                {
                    exception
                        .attrs
                        .borrow_mut()
                        .extend(instance_data.attrs.clone());
                }
                if let Object::Instance(instance_data) = &*instance.kind() {
                    exception.attrs.borrow_mut().insert(
                        "__class__".to_string(),
                        Value::Class(instance_data.class.clone()),
                    );
                }
                if !exception.attrs.borrow().contains_key("args") {
                    let args = if let Some(message) = &exception.message {
                        self.heap.alloc_tuple(vec![Value::Str(message.clone())])
                    } else {
                        self.heap.alloc_tuple(Vec::new())
                    };
                    exception
                        .attrs
                        .borrow_mut()
                        .insert("args".to_string(), args);
                }
                if self.exception_inherits(class_name.as_str(), "BaseExceptionGroup") {
                    let (group_message, members_source) =
                        {
                            let attrs = exception.attrs.borrow();
                            let group_message = match attrs.get("args") {
                                Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
                                    Object::Tuple(items) if !items.is_empty() => {
                                        Some(format_value(&items[0]))
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };
                            let members_source = attrs.get("exceptions").cloned().or_else(|| {
                                match attrs.get("args") {
                                    Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
                                        Object::Tuple(items) => items.get(1).cloned(),
                                        _ => None,
                                    },
                                    _ => None,
                                }
                            });
                            (group_message, members_source)
                        };
                    if let Some(group_message) = group_message {
                        exception.message = Some(group_message);
                    }
                    let members = if let Some(source) = members_source {
                        self.exception_members_from_value(&source)?
                    } else {
                        Vec::new()
                    };
                    let member_values = members
                        .iter()
                        .cloned()
                        .map(|member| Value::Exception(Box::new(member)))
                        .collect::<Vec<_>>();
                    exception.exceptions = members;
                    exception.attrs.borrow_mut().insert(
                        "exceptions".to_string(),
                        self.heap.alloc_tuple(member_values),
                    );
                }
                Ok(Value::Exception(Box::new(exception)))
            }
            _ => Err(RuntimeError::new("can only raise Exception types")),
        }
    }

    fn value_is_exception_like(&self, value: &Value) -> bool {
        match value {
            Value::Exception(_) | Value::ExceptionType(_) => true,
            Value::Class(class) => self.class_is_exception_class(class),
            _ => false,
        }
    }

    pub(super) fn exception_matches(
        &self,
        exception: &Value,
        handler_type: &Value,
    ) -> Result<bool, RuntimeError> {
        let exception_name: std::borrow::Cow<'_, str> = if matches!(
            exception,
            Value::Exception(_) | Value::ExceptionType(_) | Value::Class(_)
        ) {
            match exception {
                Value::Exception(exc) => std::borrow::Cow::Borrowed(exc.name.as_str()),
                Value::ExceptionType(name) => std::borrow::Cow::Borrowed(name.as_str()),
                Value::Class(class) if self.class_is_exception_class(class) => {
                    let class_kind = class.kind();
                    let Object::Class(class_data) = &*class_kind else {
                        return Err(RuntimeError::type_error("expected exception instance"));
                    };
                    std::borrow::Cow::Owned(class_data.name.clone())
                }
                _ => {
                    return Err(RuntimeError::type_error("expected exception instance"));
                }
            }
        } else {
            let Some(active) = self
                .frames
                .last()
                .and_then(|frame| frame.active_exception.clone())
            else {
                return Err(RuntimeError::type_error("expected exception instance"));
            };
            match active {
                Value::Exception(exc) => std::borrow::Cow::Owned(exc.name.clone()),
                Value::ExceptionType(name) => std::borrow::Cow::Owned(name),
                Value::Class(class) if self.class_is_exception_class(&class) => {
                    let class_kind = class.kind();
                    let Object::Class(class_data) = &*class_kind else {
                        return Err(RuntimeError::type_error("expected exception instance"));
                    };
                    std::borrow::Cow::Owned(class_data.name.clone())
                }
                _ => return Err(RuntimeError::type_error("expected exception instance")),
            }
        };

        let handler_name = match handler_type {
            Value::Tuple(obj) => {
                let Object::Tuple(items) = &*obj.kind() else {
                    return Err(RuntimeError::type_error("except expects exception type"));
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
                    return Err(RuntimeError::type_error("except expects exception type"));
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
                    return Err(RuntimeError::type_error("except expects exception type"));
                }
                let class_kind = class.kind();
                let Object::Class(class_data) = &*class_kind else {
                    return Err(RuntimeError::type_error("except expects exception type"));
                };
                return Ok(self.exception_inherits(&exception_name, &class_data.name));
            }
            _ => return Err(RuntimeError::type_error("except expects exception type")),
        };

        Ok(self.exception_inherits(&exception_name, handler_name))
    }

    pub(super) fn exception_split_for_star(
        &self,
        exception: &Value,
        handler_type: &Value,
    ) -> Result<(Option<ExceptionObject>, Option<ExceptionObject>), RuntimeError> {
        let Value::Exception(exception_obj) = exception else {
            return Err(RuntimeError::type_error("expected exception instance"));
        };
        self.exception_split_for_star_object(exception_obj, handler_type)
    }

    pub(super) fn exception_split_for_star_object(
        &self,
        exception: &ExceptionObject,
        handler_type: &Value,
    ) -> Result<(Option<ExceptionObject>, Option<ExceptionObject>), RuntimeError> {
        self.exception_split_for_star_object_mode(exception, handler_type, true)
    }

    fn exception_split_for_star_object_mode(
        &self,
        exception: &ExceptionObject,
        handler_type: &Value,
        wrap_leaf_matches: bool,
    ) -> Result<(Option<ExceptionObject>, Option<ExceptionObject>), RuntimeError> {
        if self.exception_inherits(&exception.name, "BaseExceptionGroup") {
            let mut matched_members = Vec::new();
            let mut remaining_members = Vec::new();
            for member in &exception.exceptions {
                let (matched, remaining) =
                    self.exception_split_for_star_object_mode(member, handler_type, false)?;
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

        let matches =
            self.exception_matches(&Value::Exception(Box::new(exception.clone())), handler_type)?;
        if matches {
            if wrap_leaf_matches {
                Ok((Some(self.wrap_naked_exception_for_star(exception)), None))
            } else {
                Ok((Some(exception.clone()), None))
            }
        } else {
            Ok((None, Some(exception.clone())))
        }
    }

    fn wrap_naked_exception_for_star(&self, exception: &ExceptionObject) -> ExceptionObject {
        let group_name = if self.exception_inherits(&exception.name, "Exception") {
            "ExceptionGroup"
        } else {
            "BaseExceptionGroup"
        };
        let wrapped = ExceptionObject::with_members(
            group_name.to_string(),
            Some(String::new()),
            vec![exception.clone()],
        );
        wrapped.attrs.borrow_mut().insert(
            "args".to_string(),
            self.heap.alloc_tuple(vec![
                Value::Str(String::new()),
                self.heap
                    .alloc_list(vec![Value::Exception(Box::new(exception.clone()))]),
            ]),
        );
        wrapped
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
        if exception_name == "ExceptionGroup" && handler_name == "Exception" {
            return true;
        }
        if exception_name == handler_name {
            return true;
        }
        let mut seen = HashSet::new();
        let mut current = self.exception_parent_name(exception_name);
        while let Some(name) = current {
            if !seen.insert(name.clone()) {
                break;
            }
            if name == "ExceptionGroup" && handler_name == "Exception" {
                return true;
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
        let exception_name = match &*instance_data.class.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => return None,
        };
        let args_value = instance_data.attrs.get("args")?;
        let Value::Tuple(args_obj) = args_value else {
            return None;
        };
        let Object::Tuple(args) = &*args_obj.kind() else {
            return None;
        };
        if args.is_empty() {
            return None;
        }
        if self.exception_inherits(exception_name.as_str(), "BaseExceptionGroup") {
            return Some(format_value(&args[0]));
        }
        if args.len() == 1 {
            return Some(format_value(&args[0]));
        }
        let parts = args.iter().map(format_value).collect::<Vec<_>>();
        Some(format!("({})", parts.join(", ")))
    }

    pub(super) fn populate_syntax_error_attrs(
        &self,
        attrs: &mut impl SyntaxErrorAttrStore,
        args: &[Value],
    ) {
        attrs.insert_attr("msg", args.first().cloned().unwrap_or(Value::None));
        let detail_items = args.get(1).and_then(|value| match value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(items) => Some(items.clone()),
                _ => None,
            },
            Value::List(list_obj) => match &*list_obj.kind() {
                Object::List(items) => Some(items.clone()),
                _ => None,
            },
            _ => None,
        });
        let mut filename = Value::None;
        let mut lineno = Value::None;
        let mut offset = Value::None;
        let mut text = Value::None;
        let mut end_lineno = Value::None;
        let mut end_offset = Value::None;
        if let Some(items) = detail_items {
            if let Some(value) = items.first() {
                filename = value.clone();
            }
            if let Some(value) = items.get(1) {
                lineno = value.clone();
            }
            if let Some(value) = items.get(2) {
                offset = value.clone();
            }
            if let Some(value) = items.get(3) {
                text = value.clone();
            }
            if let Some(value) = items.get(4) {
                end_lineno = value.clone();
            }
            if let Some(value) = items.get(5) {
                end_offset = value.clone();
            }
        }
        let print_file_and_line = !matches!(filename, Value::None);
        attrs.insert_attr("filename", filename);
        attrs.insert_attr("lineno", lineno);
        attrs.insert_attr("offset", offset);
        attrs.insert_attr("text", text);
        attrs.insert_attr("end_lineno", end_lineno);
        attrs.insert_attr("end_offset", end_offset);
        attrs.insert_attr("print_file_and_line", Value::Bool(print_file_and_line));
    }

    pub(super) fn instantiate_exception_type(
        &self,
        name: &str,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let call_args = args.to_vec();
        let mut remaining_kwargs = kwargs.clone();
        let mut import_error_name = Value::None;
        let mut import_error_path = Value::None;
        let mut name_error_name = Value::None;
        let mut attribute_error_name = Value::None;
        let mut attribute_error_obj = Value::None;

        let allows_named_kwargs = if is_import_error_family(name) {
            if let Some(value) = remaining_kwargs.remove("name") {
                import_error_name = value;
            }
            if let Some(value) = remaining_kwargs.remove("path") {
                import_error_path = value;
            }
            true
        } else if self.exception_inherits(name, "NameError") {
            if let Some(value) = remaining_kwargs.remove("name") {
                name_error_name = value;
            }
            true
        } else if self.exception_inherits(name, "AttributeError") {
            if let Some(value) = remaining_kwargs.remove("name") {
                attribute_error_name = value;
            }
            if let Some(value) = remaining_kwargs.remove("obj") {
                attribute_error_obj = value;
            }
            true
        } else {
            false
        };

        if !remaining_kwargs.is_empty() {
            let mut unexpected = remaining_kwargs.keys().cloned().collect::<Vec<_>>();
            unexpected.sort();
            let message = if allows_named_kwargs {
                format!(
                    "{name}() got an unexpected keyword argument '{}'",
                    unexpected[0]
                )
            } else {
                format!("{name}() takes no keyword arguments")
            };
            return Err(RuntimeError::type_error(message));
        }

        if self.exception_inherits(name, "BaseExceptionGroup") {
            let message = call_args.first().map(format_value);
            let members = if let Some(value) = call_args.get(1) {
                self.exception_members_from_value(value)?
            } else {
                Vec::new()
            };
            let exception = ExceptionObject::with_members(name.to_string(), message, members);
            exception
                .attrs
                .borrow_mut()
                .insert("args".to_string(), self.heap.alloc_tuple(call_args));
            return Ok(Value::Exception(Box::new(exception)));
        }

        let message = exception_message_from_call_args(&call_args);
        let exception = ExceptionObject::new(name.to_string(), message);
        {
            let mut attrs = exception.attrs.borrow_mut();
            attrs.insert("args".to_string(), self.heap.alloc_tuple(call_args.clone()));
            if matches!(name, "StopIteration" | "StopAsyncIteration") {
                let value = call_args.first().cloned().unwrap_or(Value::None);
                attrs.insert("value".to_string(), value);
            }
            if is_os_error_family(name) {
                if let Some(errno) = call_args
                    .first()
                    .and_then(|value| value_to_int(value.clone()).ok())
                {
                    attrs.insert("errno".to_string(), Value::Int(errno));
                }
                if let Some(strerror) = call_args.get(1) {
                    attrs.insert("strerror".to_string(), strerror.clone());
                }
                if let Some(filename) = call_args.get(2) {
                    if name == "BlockingIOError" {
                        attrs.insert("characters_written".to_string(), filename.clone());
                    } else {
                        attrs.insert("filename".to_string(), filename.clone());
                    }
                }
            }
            if is_import_error_family(name) {
                attrs.insert(
                    "msg".to_string(),
                    call_args.first().cloned().unwrap_or(Value::None),
                );
                attrs.insert("name".to_string(), import_error_name);
                attrs.insert("path".to_string(), import_error_path);
            }
            if self.exception_inherits(name, "NameError") {
                attrs.insert("name".to_string(), name_error_name);
            }
            if self.exception_inherits(name, "AttributeError") {
                attrs.insert("name".to_string(), attribute_error_name);
                attrs.insert("obj".to_string(), attribute_error_obj);
            }
            if self.exception_inherits(name, "SyntaxError") {
                self.populate_syntax_error_attrs(&mut attrs, &call_args);
            }
        }
        Ok(Value::Exception(Box::new(exception)))
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
            members.push(*exception);
        }
        Ok(members)
    }

    pub(super) fn frame_trace(frame: &Frame) -> TraceFrame {
        let trace_ip = frame.reraise_lasti_override.unwrap_or(frame.last_ip);
        let location = frame.code.locations.get(trace_ip);
        let line = location.map(|loc| loc.line).unwrap_or(0);
        let mut column = location.map(|loc| loc.column).unwrap_or(0);
        let mut end_line = location.map(|loc| loc.end_line).unwrap_or(0);
        let mut end_column = location.map(|loc| loc.end_column).unwrap_or(0);
        if frame.except_star_match_lasti.is_some()
            && frame.reraise_lasti_override.is_some()
            && let Some(last_loc) = frame.code.locations.get(frame.last_ip)
            && last_loc.line > line
        {
            end_line = last_loc.line;
            // CPython shows the full `except* ...` + `raise` source block for this case
            // without a caret anchor.
            column = column.saturating_sub(8);
            end_column = last_loc.end_column;
        }
        if matches!(
            frame
                .code
                .instructions
                .get(trace_ip)
                .map(|instr| instr.opcode),
            Some(Opcode::Raise | Opcode::Reraise)
        ) {
            end_line = 0;
            end_column = 0;
        }
        let mut seen = HashSet::new();
        let mut locals = Vec::new();
        let mut local_values = Vec::new();
        let mut self_local = None;
        let materialize_local_value = |value: Value| match value {
            Value::Cell(cell) => match &*cell.kind() {
                Object::Cell(cell_data) => cell_data.value.clone().unwrap_or(Value::None),
                _ => Value::None,
            },
            other => other,
        };
        if frame.is_module {
            if let Object::Module(module_data) = &*frame.module.kind() {
                self_local = module_data.globals.get("self").cloned();
                let mut module_names = module_data.globals.keys().cloned().collect::<Vec<_>>();
                module_names.sort();
                for name in module_names {
                    if seen.insert(name.clone()) {
                        locals.push(name.clone());
                        let value = module_data
                            .globals
                            .get(&name)
                            .cloned()
                            .map(materialize_local_value)
                            .unwrap_or(Value::None);
                        local_values.push((name, value));
                    }
                }
            }
        } else {
            self_local = frame.locals.get("self").cloned();
            for (idx, slot) in frame.fast_locals.iter().enumerate() {
                if slot.is_some()
                    && let Some(name) = frame.code.names.get(idx)
                {
                    if seen.insert(name.clone()) {
                        locals.push(name.clone());
                        if let Some(value) = slot {
                            local_values
                                .push((name.clone(), materialize_local_value(value.clone())));
                        }
                    }
                    if self_local.is_none() && name == "self" {
                        self_local = slot.clone().map(materialize_local_value);
                    }
                }
            }
            let mut extra_locals = frame.locals.keys().cloned().collect::<Vec<_>>();
            extra_locals.sort();
            for name in extra_locals {
                if seen.insert(name.clone()) {
                    locals.push(name.clone());
                    let value = frame
                        .locals
                        .get(&name)
                        .cloned()
                        .map(materialize_local_value)
                        .unwrap_or(Value::None);
                    local_values.push((name, value));
                }
            }
            if let Some(fallback_locals) = &frame.locals_fallback {
                let mut fallback_names = fallback_locals.keys().cloned().collect::<Vec<_>>();
                fallback_names.sort();
                for name in fallback_names {
                    if seen.insert(name.clone()) {
                        locals.push(name.clone());
                        let value = fallback_locals
                            .get(&name)
                            .cloned()
                            .map(materialize_local_value)
                            .unwrap_or(Value::None);
                        local_values.push((name, value));
                    }
                }
            }
            for (idx, name) in frame.code.cellvars.iter().enumerate() {
                if let Some(cell) = frame.cells.get(idx)
                    && let Object::Cell(cell_data) = &*cell.kind()
                    && let Some(value) = cell_data.value.clone()
                    && seen.insert(name.clone())
                {
                    let materialized = materialize_local_value(value);
                    locals.push(name.clone());
                    local_values.push((name.clone(), materialized.clone()));
                    if self_local.is_none() && name == "self" {
                        self_local = Some(materialized);
                    }
                }
            }
            let freevar_offset = frame.code.cellvars.len();
            for (idx, name) in frame.code.freevars.iter().enumerate() {
                if let Some(cell) = frame.cells.get(freevar_offset + idx)
                    && let Object::Cell(cell_data) = &*cell.kind()
                    && let Some(value) = cell_data.value.clone()
                    && seen.insert(name.clone())
                {
                    locals.push(name.clone());
                    local_values.push((name.clone(), materialize_local_value(value)));
                }
            }
        }
        let globals_source = frame
            .globals_fallback
            .as_ref()
            .unwrap_or(&frame.function_globals);
        let globals = if let Object::Module(module_data) = &*globals_source.kind() {
            let mut names = module_data.globals.keys().cloned().collect::<Vec<_>>();
            names.sort();
            names
        } else {
            Vec::new()
        };
        TraceFrame {
            frame_id: frame.frame_id,
            filename: frame.code.filename.clone(),
            line,
            column,
            end_line,
            end_column,
            lasti: trace_ip,
            name: frame.code.name.clone(),
            locals,
            local_values,
            globals,
            self_local,
        }
    }

    pub(super) fn format_traceback(&mut self, frames: &[TraceFrame], exc: &Value) -> String {
        match exc {
            Value::Exception(exception) => self.format_exception_chain(exception, 0, Some(frames)),
            _ => {
                let mut output = String::from("Traceback (most recent call last):\n");
                self.append_traceback_frames(&mut output, frames);
                output.push_str(&format_value(exc));
                output
            }
        }
    }

    fn format_exception_chain(
        &mut self,
        exception: &ExceptionObject,
        depth: usize,
        fallback_frames: Option<&[TraceFrame]>,
    ) -> String {
        // Guard against pathological __context__/__cause__ cycles.
        if depth >= 32 {
            return self.format_exception_with_traceback(exception, fallback_frames);
        }
        let mut output = String::new();
        if let Some(cause) = &exception.cause {
            let mut chain = self.format_exception_chain(cause, depth + 1, None);
            while chain.ends_with('\n') {
                chain.pop();
            }
            output.push_str(&chain);
            output.push_str(
                "\n\nThe above exception was the direct cause of the following exception:\n\n",
            );
        } else if !exception.suppress_context
            && let Some(context) = &exception.context
        {
            let mut chain = self.format_exception_chain(context, depth + 1, None);
            while chain.ends_with('\n') {
                chain.pop();
            }
            output.push_str(&chain);
            output.push_str(
                "\n\nDuring handling of the above exception, another exception occurred:\n\n",
            );
        }
        output.push_str(&self.format_exception_with_traceback(exception, fallback_frames));
        output
    }

    fn format_exception_with_traceback(
        &mut self,
        exception: &ExceptionObject,
        fallback_frames: Option<&[TraceFrame]>,
    ) -> String {
        let has_exception_frames = !exception.traceback_frames.is_empty();
        let has_fallback_frames = fallback_frames.is_some_and(|frames| !frames.is_empty());
        let mut output = String::new();
        if has_exception_frames || has_fallback_frames {
            output.push_str("Traceback (most recent call last):\n");
            if has_exception_frames {
                self.append_exception_traceback_frames(&mut output, exception);
            } else if let Some(frames) = fallback_frames {
                self.append_traceback_frames(&mut output, frames);
            }
        }
        output.push_str(&self.format_exception_object(exception));
        output
    }

    fn append_exception_traceback_frames(
        &mut self,
        output: &mut String,
        exception: &ExceptionObject,
    ) {
        for frame in exception.traceback_frames.iter().rev() {
            self.append_traceback_frame_line(
                output,
                &frame.filename,
                frame.line,
                frame.column,
                frame.end_line,
                frame.end_column,
                &frame.name,
                Some(&exception.name),
            );
        }
    }

    fn append_traceback_frames(&mut self, output: &mut String, frames: &[TraceFrame]) {
        for frame in frames.iter().rev() {
            self.append_traceback_frame_line(
                output,
                &frame.filename,
                frame.line,
                frame.column,
                frame.end_line,
                frame.end_column,
                &frame.name,
                None,
            );
        }
    }

    fn append_traceback_frame_line(
        &mut self,
        output: &mut String,
        filename: &str,
        line: usize,
        column: usize,
        end_line: usize,
        end_column: usize,
        name: &str,
        exception_name: Option<&str>,
    ) {
        output.push_str(&format!(
            "  File \"{}\", line {}, in {}\n",
            filename, line, name
        ));
        if line > 0
            && let Some(source_line) = self.traceback_source_line(filename, line)
        {
            output.push_str("    ");
            output.push_str(&source_line);
            output.push('\n');

            let mut resolved_end_line = end_line;
            if resolved_end_line <= line && end_column == 0 {
                let inferred =
                    self.infer_traceback_continuation_end_line(filename, line, &source_line);
                if inferred > line {
                    resolved_end_line = inferred;
                }
            }

            if resolved_end_line > line {
                for extra_line in (line + 1)..=resolved_end_line {
                    let Some(source) = self.traceback_source_line(filename, extra_line) else {
                        break;
                    };
                    output.push_str("    ");
                    output.push_str(&source);
                    output.push('\n');
                }
                return;
            }

            if self.traceback_caret_enabled
                && let Some(caret_line) = render_traceback_caret_line(
                    &source_line,
                    line,
                    column,
                    resolved_end_line,
                    end_column,
                )
            {
                if should_suppress_explicit_raise_caret(&source_line, column, exception_name) {
                    return;
                }
                output.push_str("    ");
                output.push_str(&caret_line);
                output.push('\n');
            }
        }
    }

    fn infer_traceback_continuation_end_line(
        &mut self,
        filename: &str,
        start_line: usize,
        first_line: &str,
    ) -> usize {
        let mut state = LineContinuationState::default();
        update_line_continuation_state(&mut state, first_line);
        let mut explicit = line_has_explicit_continuation(first_line);
        if state.delimiter_depth() == 0 && !explicit {
            return start_line;
        }

        let mut end_line = start_line;
        for _ in 0..32 {
            let next_line = end_line.saturating_add(1);
            let Some(source) = self.traceback_source_line(filename, next_line) else {
                break;
            };
            end_line = next_line;
            update_line_continuation_state(&mut state, &source);
            explicit = line_has_explicit_continuation(&source);
            if state.delimiter_depth() == 0 && !explicit {
                break;
            }
        }
        end_line
    }

    pub(super) fn format_exception_object(&self, exception: &ExceptionObject) -> String {
        let mut output = String::new();
        if let Some(location) = self.syntax_error_location_block(exception) {
            output.push_str(&location);
        }
        let display = self.exception_display_message(exception);
        if display.is_empty() {
            output.push_str(&exception.name);
        } else {
            output.push_str(&format!("{}: {}", exception.name, display));
        }
        output
    }

    fn exception_display_message(&self, exception: &ExceptionObject) -> String {
        let args = {
            let attrs = exception.attrs.borrow();
            let Some(Value::Tuple(tuple)) = attrs.get("args") else {
                return exception.message.clone().unwrap_or_default();
            };
            let Object::Tuple(values) = &*tuple.kind() else {
                return exception.message.clone().unwrap_or_default();
            };
            values.clone()
        };
        if args.is_empty() {
            return String::new();
        }
        if self.exception_inherits(exception.name.as_str(), "BaseExceptionGroup") {
            let message = format_value(&args[0]);
            let count = exception.exceptions.len();
            let suffix = if count == 1 {
                "1 sub-exception".to_string()
            } else {
                format!("{count} sub-exceptions")
            };
            return if message.is_empty() {
                format!(" ({suffix})")
            } else {
                format!("{message} ({suffix})")
            };
        }
        if self.exception_inherits(exception.name.as_str(), "SyntaxError") {
            return format_value(&args[0]);
        }
        if exception.name == "KeyError" && args.len() == 1 {
            return format_repr(&args[0]);
        }
        if args.len() == 1 {
            return format_value(&args[0]);
        }
        let parts = args.iter().map(format_repr).collect::<Vec<_>>();
        format!("({})", parts.join(", "))
    }

    fn syntax_error_location_block(&self, exception: &ExceptionObject) -> Option<String> {
        if !self.exception_inherits(exception.name.as_str(), "SyntaxError") {
            return None;
        }
        let (filename, lineno, text) = {
            let attrs = exception.attrs.borrow();
            (
                attrs.get("filename").cloned().unwrap_or(Value::None),
                attrs.get("lineno").cloned().unwrap_or(Value::None),
                attrs.get("text").cloned().unwrap_or(Value::None),
            )
        };
        if matches!(filename, Value::None) || matches!(lineno, Value::None) {
            return None;
        }
        let line_number = value_to_int(lineno).ok()?;
        let mut out = String::new();
        out.push_str(&format!(
            "  File \"{}\", line {}\n",
            format_value(&filename),
            line_number
        ));
        if !matches!(text, Value::None) {
            let text_value = format_value(&text);
            if !text_value.is_empty() {
                out.push_str("    ");
                out.push_str(text_value.trim_end_matches('\n'));
                out.push('\n');
            }
        }
        Some(out)
    }

    pub(super) fn class_namespace_backing_dict(&self, namespace: &Value) -> Option<ObjRef> {
        match namespace {
            Value::Dict(dict) => Some(dict.clone()),
            Value::Instance(instance) => self.instance_backing_dict(instance),
            _ => None,
        }
    }

    pub(super) fn class_namespace_attrs_map(
        &self,
        namespace: &Value,
    ) -> Result<HashMap<String, Value>, RuntimeError> {
        let Some(dict_obj) = self.class_namespace_backing_dict(namespace) else {
            return Err(RuntimeError::new("class namespace must be a mapping"));
        };
        let Object::Dict(entries) = &*dict_obj.kind() else {
            return Err(RuntimeError::new("class namespace must be a mapping"));
        };
        let mut attrs = HashMap::new();
        for (key, value) in entries {
            let Value::Str(name) = key else {
                return Err(RuntimeError::new("type() dict keys must be strings"));
            };
            attrs.insert(name.clone(), value.clone());
        }
        Ok(attrs)
    }

    pub(super) fn class_namespace_lookup_name(
        &self,
        namespace: &Value,
        name: &str,
    ) -> Option<Value> {
        let dict = self.class_namespace_backing_dict(namespace)?;
        dict_get_value(&dict, &Value::Str(name.to_string()))
    }

    pub(super) fn class_namespace_set_name(
        &mut self,
        namespace: &Value,
        name: String,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match namespace {
            Value::Dict(dict) => dict_set_value_checked(dict, Value::Str(name), value),
            Value::Instance(_) => {
                if let Some(setitem) = self.lookup_bound_special_method(namespace, "__setitem__")? {
                    match self.call_internal(
                        setitem,
                        vec![Value::Str(name), value],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(_)
                        | InternalCallOutcome::CallerExceptionHandled => Ok(()),
                    }
                } else if let Some(dict) = self.class_namespace_backing_dict(namespace) {
                    dict_set_value_checked(&dict, Value::Str(name), value)
                } else {
                    Err(RuntimeError::new(
                        "class namespace does not support item assignment",
                    ))
                }
            }
            _ => Err(RuntimeError::new(
                "class namespace does not support item assignment",
            )),
        }
    }

    fn maybe_promote_class_annotations_to_annotate(
        &mut self,
        namespace: &Value,
        class_body_module: &ObjRef,
        defining_globals: Option<&ObjRef>,
        defining_locals: Option<&HashMap<String, Value>>,
        future_annotations_import: bool,
    ) -> Result<(), RuntimeError> {
        let Some(namespace_dict) = self.class_namespace_backing_dict(namespace) else {
            return Ok(());
        };
        if dict_get_value(&namespace_dict, &Value::Str("__annotate__".to_string())).is_some()
            || dict_get_value(
                &namespace_dict,
                &Value::Str("__annotate_func__".to_string()),
            )
            .is_some()
        {
            return Ok(());
        }
        let annotations_key = Value::Str("__annotations__".to_string());
        let Some(Value::Dict(annotations_dict)) = dict_get_value(&namespace_dict, &annotations_key)
        else {
            return Ok(());
        };
        let has_deferred_strings = match &*annotations_dict.kind() {
            Object::Dict(entries) => entries
                .iter()
                .any(|(_key, value)| matches!(value, Value::Str(_))),
            _ => false,
        };
        if !has_deferred_strings {
            return Ok(());
        }

        let annotation_module = defining_globals
            .cloned()
            .or_else(|| {
                dict_get_value(&namespace_dict, &Value::Str("__module__".to_string())).and_then(
                    |value| match value {
                        Value::Str(module_name) => self.modules.get(&module_name).cloned(),
                        _ => None,
                    },
                )
            })
            .unwrap_or_else(|| class_body_module.clone());
        let mut annotate_code = CodeObject::new("__annotate__", "<class_annotations>");
        annotate_code.params.push("format".to_string());
        annotate_code.rebuild_layout_indexes();
        annotate_code.fast_local_count = 1;
        annotate_code.future_annotations_import = future_annotations_import;
        let annotate_function = match self.heap.alloc_function(FunctionObject::new(
            Rc::new(annotate_code),
            annotation_module,
            Vec::new(),
            HashMap::new(),
            Vec::new(),
            None,
            false,
        )) {
            Value::Function(function) => function,
            _ => unreachable!(),
        };
        if let Object::Function(function_data) = &mut *annotate_function.kind_mut() {
            function_data.annotations = Some(annotations_dict);
        }
        let function_dict = self.ensure_function_dict(&annotate_function)?;
        let mut annotation_locals_entries = match &*namespace_dict.kind() {
            Object::Dict(entries) => entries.to_vec(),
            _ => Vec::new(),
        };
        let mut annotation_local_names = annotation_locals_entries
            .iter()
            .filter_map(|(key, _value)| match key {
                Value::Str(name) => Some(name.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        if let Some(type_params) =
            dict_get_value(&namespace_dict, &Value::Str("__type_params__".to_string()))
        {
            let type_param_items = match type_params {
                Value::Tuple(tuple) => match &*tuple.kind() {
                    Object::Tuple(items) => Some(items.clone()),
                    _ => None,
                },
                Value::List(list) => match &*list.kind() {
                    Object::List(items) => Some(items.clone()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(type_param_items) = type_param_items {
                for type_param in type_param_items {
                    let name = self
                        .optional_getattr_value(type_param.clone(), "__name__")?
                        .and_then(|value| match value {
                            Value::Str(name) => Some(name),
                            _ => None,
                        });
                    if let Some(name) = name
                        && !annotation_local_names.contains(&name)
                    {
                        annotation_local_names.insert(name.clone());
                        annotation_locals_entries.push((Value::Str(name), type_param));
                    }
                }
            }
        }
        if let Some(defining_locals) = defining_locals {
            for (name, value) in defining_locals {
                if annotation_local_names.contains(name) {
                    continue;
                }
                annotation_local_names.insert(name.clone());
                annotation_locals_entries.push((Value::Str(name.clone()), value.clone()));
            }
        }
        let annotation_locals = self.heap.alloc_dict(annotation_locals_entries);
        self.dict_set_str_key(
            &function_dict,
            "__pyrs_annotation_locals__".to_string(),
            annotation_locals,
        )?;
        let annotate_receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__class_annotate__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *annotate_receiver.kind_mut() {
            module_data
                .globals
                .insert("function".to_string(), Value::Function(annotate_function));
        }
        dict_set_value(
            &namespace_dict,
            Value::Str("__annotate__".to_string()),
            self.alloc_native_bound_method(NativeMethodKind::FunctionAnnotate, annotate_receiver),
        );
        let _ = dict_remove_value(&namespace_dict, &annotations_key);
        Ok(())
    }

    fn seed_class_namespace_type_params_from_orig_bases(
        &mut self,
        namespace: &Value,
        class_orig_bases: Option<&Value>,
    ) -> Result<(), RuntimeError> {
        if self
            .class_namespace_lookup_name(namespace, "__type_params__")
            .is_some()
        {
            return Ok(());
        }
        let Some(class_orig_bases) = class_orig_bases else {
            return Ok(());
        };
        let base_values = match class_orig_bases {
            Value::Tuple(tuple) => match &*tuple.kind() {
                Object::Tuple(items) => items.clone(),
                _ => return Ok(()),
            },
            Value::List(list) => match &*list.kind() {
                Object::List(items) => items.clone(),
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        let mut seen_type_params = HashSet::new();
        let mut type_params = Vec::new();
        for base in base_values {
            let Some(parameters_value) = self.optional_getattr_value(base, "__parameters__")?
            else {
                continue;
            };
            let param_items = match parameters_value {
                Value::Tuple(tuple) => match &*tuple.kind() {
                    Object::Tuple(items) => items.clone(),
                    _ => continue,
                },
                Value::List(list) => match &*list.kind() {
                    Object::List(items) => items.clone(),
                    _ => continue,
                },
                _ => continue,
            };
            for param in param_items {
                if !self.is_type_parameter_value(&param) {
                    continue;
                }
                let Value::Instance(instance) = &param else {
                    continue;
                };
                if !seen_type_params.insert(instance.id()) {
                    continue;
                }
                type_params.push(param);
            }
        }
        if type_params.is_empty() {
            return Ok(());
        }
        self.class_namespace_set_name(
            namespace,
            "__type_params__".to_string(),
            self.heap.alloc_tuple(type_params),
        )?;
        Ok(())
    }

    pub(super) fn class_value_from_module(
        &mut self,
        module: &ObjRef,
        bases: Vec<ObjRef>,
        class_orig_bases: Option<Value>,
        metaclass: Option<Value>,
        class_keywords: HashMap<String, Value>,
        class_namespace: Option<Value>,
        defining_globals: Option<ObjRef>,
        defining_locals: Option<HashMap<String, Value>>,
        future_annotations_import: bool,
    ) -> Result<ClassBuildOutcome, RuntimeError> {
        let (name, module_attrs) = match &*module.kind() {
            Object::Module(module_data) => (module_data.name.clone(), module_data.globals.clone()),
            _ => ("<class>".to_string(), HashMap::new()),
        };
        let namespace_value = class_namespace.unwrap_or_else(|| {
            self.heap.alloc_dict(
                module_attrs
                    .iter()
                    .map(|(key, value)| (Value::Str(key.clone()), value.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        if let Some(orig_bases) = class_orig_bases.clone() {
            self.class_namespace_set_name(
                &namespace_value,
                "__orig_bases__".to_string(),
                orig_bases,
            )?;
        }
        self.seed_class_namespace_type_params_from_orig_bases(
            &namespace_value,
            class_orig_bases.as_ref(),
        )?;
        self.maybe_promote_class_annotations_to_annotate(
            &namespace_value,
            module,
            defining_globals.as_ref(),
            defining_locals.as_ref(),
            future_annotations_import,
        )?;
        let attrs = self.class_namespace_attrs_map(&namespace_value)?;
        let default_bases = if bases.is_empty() {
            if let Some(Value::Class(object_class)) = self.builtins.get("object") {
                vec![object_class.clone()]
            } else {
                Vec::new()
            }
        } else {
            bases.clone()
        };
        let resolved_metaclass =
            self.resolve_class_metaclass(&default_bases, metaclass.as_ref())?;
        let explicit_metaclass = metaclass.clone();
        let effective_metaclass =
            metaclass.or_else(|| resolved_metaclass.clone().map(Value::Class));
        let plain_type_metaclass_id = self.default_type_metaclass().map(|meta| meta.id());
        let uses_plain_type = matches!(
            &effective_metaclass,
            Some(Value::Builtin(BuiltinFunction::Type))
        ) || matches!(
            (&effective_metaclass, plain_type_metaclass_id),
            (Some(Value::Class(meta)), Some(type_id)) if meta.id() == type_id
        );

        if !uses_plain_type {
            let Some(meta) = effective_metaclass else {
                let class_value = self.build_default_class_value(
                    name,
                    attrs,
                    default_bases,
                    resolved_metaclass,
                    Some(&namespace_value),
                )?;
                if let Value::Class(class_ref) = &class_value
                    && self.call_init_subclass_hook(class_ref, &class_keywords)?
                {
                    return Ok(ClassBuildOutcome::ExceptionHandled);
                }
                return Ok(ClassBuildOutcome::Value(class_value));
            };
            if self.trace_flags.build_class && name == "_TagInfo" {
                let base_debug = bases
                    .iter()
                    .map(|base| match &*base.kind() {
                        Object::Class(class_data) => format!("{}#{}", class_data.name, base.id()),
                        _ => format!("<non-class>#{}", base.id()),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let typing_namedtuple = self
                    .modules
                    .get("typing")
                    .and_then(|typing| match &*typing.kind() {
                        Object::Module(module_data) => {
                            module_data.globals.get("_NamedTuple").cloned()
                        }
                        _ => None,
                    })
                    .and_then(|value| match value {
                        Value::Class(class) => Some(class),
                        _ => None,
                    })
                    .map(|class| {
                        format!(
                            "{}#{}",
                            match &*class.kind() {
                                Object::Class(class_data) => class_data.name.clone(),
                                _ => "<non-class>".to_string(),
                            },
                            class.id()
                        )
                    })
                    .unwrap_or_else(|| "<missing>".to_string());
                eprintln!(
                    "[build-class-meta] name={} bases=[{}] typing._NamedTuple={}",
                    name, base_debug, typing_namedtuple
                );
            }
            let bases_tuple = self
                .heap
                .alloc_tuple(bases.iter().cloned().map(Value::Class).collect::<Vec<_>>());
            return match self.call_internal(
                meta,
                vec![Value::Str(name), bases_tuple, namespace_value],
                class_keywords,
            )? {
                InternalCallOutcome::Value(value) => {
                    if let Value::Class(class) = &value {
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

        let class_value = self.build_default_class_value(
            name,
            attrs,
            default_bases,
            resolved_metaclass,
            Some(&namespace_value),
        )?;
        if let Value::Class(class_ref) = &class_value {
            if self.call_init_subclass_hook(class_ref, &class_keywords)? {
                return Ok(ClassBuildOutcome::ExceptionHandled);
            }
            if let Some(Value::Class(meta)) = explicit_metaclass
                && let Object::Class(class_data) = &mut *class_ref.kind_mut()
            {
                class_data.metaclass = Some(meta);
            }
            self.record_exception_parent_for_class(class_ref);
        }
        Ok(ClassBuildOutcome::Value(class_value))
    }

    pub(super) fn call_init_subclass_hook(
        &mut self,
        class: &ObjRef,
        class_keywords: &HashMap<String, Value>,
    ) -> Result<bool, RuntimeError> {
        let trace_init_subclass = self.trace_flags.init_subclass;
        let mro = self.class_mro_entries(class);
        let init_subclass = mro
            .into_iter()
            .skip(1)
            .find_map(|candidate| class_attr_lookup_direct(&candidate, "__init_subclass__"));
        let Some(raw_init_subclass) = init_subclass else {
            return Ok(false);
        };
        let (init_subclass, init_subclass_args) =
            if let Some(bound) = self.bind_classmethod_attr(class, &raw_init_subclass) {
                (bound, Vec::new())
            } else if let Some(unwrapped) = self.unwrap_staticmethod_attr(&raw_init_subclass) {
                (unwrapped, Vec::new())
            } else {
                (raw_init_subclass, vec![Value::Class(class.clone())])
            };
        if trace_init_subclass {
            let class_name = match &*class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            let hook_tag = match &init_subclass {
                Value::Builtin(builtin) => format!("builtin::{builtin:?}"),
                Value::Function(_) => "function".to_string(),
                Value::BoundMethod(_) => "bound_method".to_string(),
                Value::Class(class_ref) => match &*class_ref.kind() {
                    Object::Class(class_data) => format!("class::{}", class_data.name),
                    _ => "class::<non-class>".to_string(),
                },
                Value::Instance(_) => "instance".to_string(),
                _ => format!("{init_subclass:?}"),
            };
            eprintln!(
                "[init-subclass] depth={} class={} hook={} kwargs={}",
                self.frames.len(),
                class_name,
                hook_tag,
                class_keywords.len()
            );
        }
        match self.call_internal(init_subclass, init_subclass_args, class_keywords.clone())? {
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

        if winner.is_none() {
            winner = self.default_type_metaclass();
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
        namespace: Option<&Value>,
    ) -> Result<Value, RuntimeError> {
        self.ensure_unique_base_classes(&bases)?;
        for base in &bases {
            let Object::Class(class_data) = &*base.kind() else {
                continue;
            };
            if !matches!(
                class_data.attrs.get("__pyrs_disallow_subclassing__"),
                Some(Value::Bool(true))
            ) {
                continue;
            }
            let module_name = match class_data.attrs.get("__module__") {
                Some(Value::Str(module_name)) => module_name.as_str(),
                _ => "builtins",
            };
            return Err(RuntimeError::type_error(format!(
                "type '{}.{}' is not an acceptable base type",
                module_name, class_data.name
            )));
        }
        let mut attrs = attrs;
        let class_cell = attrs.remove("__classcell__");
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
            if let Some(class_cell) = class_cell {
                match class_cell {
                    Value::Cell(cell) => {
                        if let Object::Cell(cell_data) = &mut *cell.kind_mut() {
                            cell_data.value = Some(Value::Class(class_ref.clone()));
                        }
                    }
                    other => {
                        return Err(RuntimeError::type_error(format!(
                            "__classcell__ must be a cell, got {}",
                            self.value_type_name_for_error(&other)
                        )));
                    }
                }
            }
            let mut slot_descriptor_names = Vec::new();
            if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                class_data.attrs.extend(attrs);
                class_data.metaclass = metaclass;
                if matches!(
                    class_data.name.as_str(),
                    "Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" | "ReprEnum"
                ) {
                    let default_use_args =
                        matches!(class_data.name.as_str(), "IntEnum" | "StrEnum" | "IntFlag");
                    class_data
                        .attrs
                        .entry("_use_args_".to_string())
                        .or_insert(Value::Bool(default_use_args));
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
                if let Some(slots_value) = class_data.attrs.get("__slots__").cloned()
                    && let Some(slot_names) = slot_names_from_value(Some(slots_value.clone()))
                {
                    slot_descriptor_names = slot_names
                        .iter()
                        .filter(|slot_name| {
                            !matches!(slot_name.as_str(), "__dict__" | "__weakref__")
                                && !class_data.attrs.contains_key(slot_name.as_str())
                        })
                        .cloned()
                        .collect();
                    class_data.slots = Some(slot_names);
                    // Preserve the declared __slots__ object shape (str/list/tuple/etc.)
                    // while retaining normalized slot names in ClassObject::slots.
                    class_data
                        .attrs
                        .insert("__slots__".to_string(), slots_value);
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
                    .entry("__doc__".to_string())
                    .or_insert(Value::None);
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
            if !slot_descriptor_names.is_empty() {
                let slot_descriptors = slot_descriptor_names
                    .into_iter()
                    .map(|slot_name| {
                        let descriptor = self.slot_member_descriptor_value(class_ref, &slot_name);
                        (slot_name, descriptor)
                    })
                    .collect::<Vec<_>>();
                if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
                    for (slot_name, descriptor) in slot_descriptors {
                        class_data.attrs.entry(slot_name).or_insert(descriptor);
                    }
                }
            }
            self.update_class_annotate_owner(class_ref);
            self.attach_owner_class_to_attrs(class_ref);
            if let Ok(mro) = self.build_class_mro(class_ref, &bases)
                && let Object::Class(class_data) = &mut *class_ref.kind_mut()
            {
                class_data.mro = mro.clone();
                let mro_values = mro.into_iter().map(Value::Class).collect::<Vec<_>>();
                class_data
                    .attrs
                    .insert("__mro__".to_string(), self.heap.alloc_tuple(mro_values));
            }
            self.call_class_set_name_hooks(class_ref, namespace)?;
            self.record_exception_parent_for_class(class_ref);
        }
        Ok(class_value)
    }

    fn update_class_annotate_owner(&mut self, class_ref: &ObjRef) {
        let annotate = match &*class_ref.kind() {
            Object::Class(class_data) => class_data
                .attrs
                .get("__annotate__")
                .or_else(|| class_data.attrs.get("__annotate_func__"))
                .cloned(),
            _ => None,
        };
        let Some(Value::BoundMethod(bound_method)) = annotate else {
            return;
        };
        let (function, receiver) = match &*bound_method.kind() {
            Object::BoundMethod(bound_data) => {
                (bound_data.function.clone(), bound_data.receiver.clone())
            }
            _ => return,
        };
        let is_function_annotate = matches!(
            &*function.kind(),
            Object::NativeMethod(native_data)
                if native_data.kind == NativeMethodKind::FunctionAnnotate
        );
        if !is_function_annotate {
            return;
        }
        if let Object::Module(module_data) = &mut *receiver.kind_mut()
            && module_data.name == "__class_annotate__"
        {
            module_data
                .globals
                .insert("owner".to_string(), Value::Class(class_ref.clone()));
        }
    }

    pub(super) fn call_class_set_name_hooks(
        &mut self,
        class_ref: &ObjRef,
        namespace: Option<&Value>,
    ) -> Result<(), RuntimeError> {
        let attrs = if let Some(namespace) = namespace
            && let Some(dict_obj) = self.class_namespace_backing_dict(namespace)
            && let Object::Dict(entries) = &*dict_obj.kind()
        {
            entries
                .iter()
                .filter_map(|(key, value)| match key {
                    Value::Str(name) => Some((name.clone(), value.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        } else {
            match &*class_ref.kind() {
                Object::Class(class_data) => class_data
                    .attrs
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect::<Vec<_>>(),
                _ => return Ok(()),
            }
        };
        for (name, value) in attrs {
            let set_name = match self.call_internal_preserving_caller(
                Value::Builtin(BuiltinFunction::GetAttr),
                vec![value.clone(), Value::Str("__set_name__".to_string())],
                HashMap::new(),
            ) {
                Ok(InternalCallOutcome::Value(value)) => Some(value),
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    if self.active_exception_is("AttributeError") {
                        self.clear_active_exception();
                        None
                    } else {
                        return Err(self.runtime_error_from_active_exception(
                            "__set_name__ lookup during class creation failed",
                        ));
                    }
                }
                Err(err) => {
                    if runtime_error_matches_exception(&err, "AttributeError") {
                        None
                    } else {
                        return Err(err);
                    }
                }
            };
            let Some(set_name) = set_name else {
                continue;
            };
            match self.call_internal_preserving_caller(
                set_name,
                vec![Value::Class(class_ref.clone()), Value::Str(name.clone())],
                HashMap::new(),
            ) {
                Ok(InternalCallOutcome::Value(_)) => {}
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    self.add_set_name_failure_note_to_active_exception(class_ref, &name, &value);
                    return Err(self.runtime_error_from_active_exception(
                        "__set_name__ during class creation failed",
                    ));
                }
                Err(mut err) => {
                    self.add_set_name_failure_note_to_runtime_error(
                        class_ref, &name, &value, &mut err,
                    );
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    fn set_name_failure_note(
        &self,
        class_ref: &ObjRef,
        attr_name: &str,
        descriptor: &Value,
    ) -> String {
        let class_name = match &*class_ref.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => "<class>".to_string(),
        };
        let descriptor_type = self.value_type_name_for_error(descriptor);
        let attr_repr = format_repr(&Value::Str(attr_name.to_string()));
        format!(
            "Error calling __set_name__ on '{}' instance {} in '{}'",
            descriptor_type, attr_repr, class_name
        )
    }

    fn add_set_name_failure_note_to_active_exception(
        &mut self,
        class_ref: &ObjRef,
        attr_name: &str,
        descriptor: &Value,
    ) {
        let note = self.set_name_failure_note(class_ref, attr_name, descriptor);
        let note_value = Value::Str(note.clone());
        let default_notes_list = self.heap.alloc_list(vec![note_value.clone()]);
        let Some(exception) =
            self.frames
                .iter_mut()
                .rev()
                .find_map(|frame| match frame.active_exception.as_mut() {
                    Some(Value::Exception(exception)) => Some(exception),
                    _ => None,
                })
        else {
            return;
        };
        let mut note_applied = false;
        {
            let mut attrs = exception.attrs.borrow_mut();
            match attrs.get_mut("__notes__") {
                Some(Value::List(notes_obj)) => {
                    if let Object::List(notes) = &mut *notes_obj.kind_mut() {
                        notes.push(note_value);
                        note_applied = true;
                    }
                }
                Some(_) => {}
                None => {
                    attrs.insert("__notes__".to_string(), default_notes_list);
                    note_applied = true;
                }
            }
        }
        if note_applied {
            exception.notes.push(note);
        }
    }

    fn add_set_name_failure_note_to_runtime_error(
        &mut self,
        class_ref: &ObjRef,
        attr_name: &str,
        descriptor: &Value,
        err: &mut RuntimeError,
    ) {
        let Some(exception) = err.exception.as_mut() else {
            self.add_set_name_failure_note_to_active_exception(class_ref, attr_name, descriptor);
            return;
        };
        let note = self.set_name_failure_note(class_ref, attr_name, descriptor);
        let note_value = Value::Str(note.clone());
        let default_notes_list = self.heap.alloc_list(vec![note_value.clone()]);
        let mut note_applied = false;
        {
            let mut attrs = exception.attrs.borrow_mut();
            match attrs.get_mut("__notes__") {
                Some(Value::List(notes_obj)) => {
                    if let Object::List(notes) = &mut *notes_obj.kind_mut() {
                        notes.push(note_value);
                        note_applied = true;
                    }
                }
                Some(_) => {}
                None => {
                    attrs.insert("__notes__".to_string(), default_notes_list);
                    note_applied = true;
                }
            }
        }
        if note_applied {
            exception.notes.push(note);
        }
    }

    pub(super) fn attach_owner_class_to_attrs(&mut self, class_ref: &ObjRef) {
        let attrs = {
            let class_ref_borrow = class_ref.kind();
            let Object::Class(class_data) = &*class_ref_borrow else {
                return;
            };
            class_data.attrs.values().cloned().collect::<Vec<_>>()
        };
        for value in attrs {
            self.attach_owner_class_to_value(&value, class_ref);
        }
    }

    fn current_frame_function_qualname(&self, function_name: &str) -> String {
        self.frames
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
                    return Some(format!("{outer_qualname}.{function_name}"));
                }
                if frame.is_module {
                    return None;
                }
                let mut outer_qualname = frame.code.name.clone();
                let owner_value = Self::frame_trace(frame)
                    .local_values
                    .into_iter()
                    .find_map(|(name, value)| (name == "self" || name == "cls").then_some(value));
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
                Some(format!("{outer_qualname}.<locals>.{function_name}"))
            })
            .unwrap_or_else(|| function_name.to_string())
    }

    fn function_defined_in_class_body(&self, func: &ObjRef) -> bool {
        let Object::Function(func_data) = &*func.kind() else {
            return false;
        };
        func_data.defined_in_class_body
    }

    pub(super) fn attach_owner_class_to_value(&mut self, value: &Value, owner: &ObjRef) {
        match value {
            Value::Function(func) => {
                if self.function_defined_in_class_body(func) {
                    self.set_function_owner_class(func, owner);
                }
            }
            Value::Instance(instance) => {
                if let Some((fget, fset, fdel, _doc, _explicit_name)) =
                    self.property_descriptor_parts(instance)
                {
                    self.attach_owner_class_to_value(&fget, owner);
                    self.attach_owner_class_to_value(&fset, owner);
                    self.attach_owner_class_to_value(&fdel, owner);
                }
                if let Some((func, _attr_name, _doc)) =
                    self.cached_property_descriptor_parts(instance)
                {
                    self.attach_owner_class_to_value(&func, owner);
                }
            }
            Value::Module(module) => {
                let Object::Module(module_data) = &*module.kind() else {
                    return;
                };
                if (module_data.name == "__classmethod__" || module_data.name == "__staticmethod__")
                    && let Some(Value::Function(func)) = module_data.globals.get("__func__")
                {
                    if self.function_defined_in_class_body(func) {
                        self.set_function_owner_class(func, owner);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn set_function_owner_class(&mut self, func: &ObjRef, owner: &ObjRef) {
        if let Object::Function(func_data) = &mut *func.kind_mut() {
            if func_data.owner_class.is_none() {
                func_data.owner_class = Some(owner.clone());
            }
        }
    }

    pub(super) fn class_mro_entries(&self, class: &ObjRef) -> Vec<ObjRef> {
        let trace_debug_mro_depth = self.host.env_var_os("PYRS_DEBUG_MRO_DEPTH").is_some();

        fn seen_insert_class(class: &ObjRef, seen: &mut HashSet<u64>) -> bool {
            if !seen.insert(class.id()) {
                return false;
            }
            if let Some(proxy_ptr) =
                Vm::cpython_proxy_raw_ptr_from_value(&Value::Class(class.clone()))
            {
                let proxy_key = proxy_ptr as usize as u64;
                seen.insert(proxy_key | (1u64 << 63));
            }
            true
        }

        fn seen_contains_class(class: &ObjRef, seen: &HashSet<u64>) -> bool {
            if seen.contains(&class.id()) {
                return true;
            }
            if let Some(proxy_ptr) =
                Vm::cpython_proxy_raw_ptr_from_value(&Value::Class(class.clone()))
            {
                let proxy_key = proxy_ptr as usize as u64;
                return seen.contains(&(proxy_key | (1u64 << 63)));
            }
            false
        }

        fn collect_mro_entries(
            class: &ObjRef,
            seen: &mut HashSet<u64>,
            depth: usize,
            trace_debug_mro_depth: bool,
        ) -> Vec<ObjRef> {
            if trace_debug_mro_depth && depth > 256 {
                let class_name = match &*class.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<non-class>".to_string(),
                };
                panic!(
                    "class_mro_entries recursion depth exceeded: depth={depth} class={class_name} id={}",
                    class.id()
                );
            }
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Vec::new();
            };
            if !class_data.mro.is_empty() {
                let mut entries = Vec::new();
                for entry in &class_data.mro {
                    if !seen_contains_class(entry, seen) && seen_insert_class(entry, seen) {
                        entries.push(entry.clone());
                    }
                }
                return entries;
            }
            if !seen_insert_class(class, seen) {
                return Vec::new();
            }
            let mut entries = vec![class.clone()];
            for base in &class_data.bases {
                for candidate in
                    collect_mro_entries(base, seen, depth.saturating_add(1), trace_debug_mro_depth)
                {
                    let duplicate = entries.iter().any(|entry| {
                        entry.id() == candidate.id()
                            || (Vm::cpython_proxy_raw_ptr_from_value(&Value::Class(entry.clone()))
                                .zip(Vm::cpython_proxy_raw_ptr_from_value(&Value::Class(
                                    candidate.clone(),
                                )))
                                .is_some_and(|(left, right)| left == right))
                    });
                    if !duplicate {
                        entries.push(candidate);
                    }
                }
            }
            entries
        }

        let mut seen: HashSet<u64> = HashSet::new();
        let mut entries = collect_mro_entries(class, &mut seen, 0, trace_debug_mro_depth);
        if let Some(object_idx) = entries.iter().position(|entry| {
            matches!(&*entry.kind(), Object::Class(class_data) if class_data.name == "object")
        })
            && object_idx + 1 != entries.len() {
                let object_entry = entries.remove(object_idx);
                entries.push(object_entry);
            }
        entries
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

    #[inline(always)]
    pub(super) fn pop_value(&mut self) -> Result<Value, RuntimeError> {
        {
            let frame = self.frames.last_mut().expect("frame exists");
            if let Some(value) = frame.stack.pop() {
                return Ok(value);
            }
        }
        let frame = self.frames.last_mut().expect("frame exists");
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

    #[inline(always)]
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

    pub(super) fn pop_value_from_frame(
        &mut self,
        frame_idx: usize,
        context: &'static str,
    ) -> Result<Value, RuntimeError> {
        let Some(frame) = self.frames.get_mut(frame_idx) else {
            return Err(RuntimeError::new(format!("{context} frame missing")));
        };
        frame
            .stack
            .pop()
            .ok_or_else(|| RuntimeError::new(format!("stack underflow ({context})")))
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
                cell_data.value = Some(value.clone());
                if idx < frame.code.cellvars.len()
                    && let Some(name) = frame.code.cellvars.get(idx)
                    && let Some(slot_idx) = frame.code.name_to_index.get(name).copied()
                {
                    if let Some(slot) = frame.fast_locals.get_mut(slot_idx) {
                        match slot {
                            Some(Value::Cell(existing_cell)) if existing_cell.id() == cell.id() => {
                            }
                            Some(Value::Cell(_)) => {
                                *slot = Some(Value::Cell(cell.clone()));
                            }
                            Some(_) | None => {
                                *slot = Some(Value::Cell(cell.clone()));
                            }
                        }
                    }
                    if let Some(existing) = frame.locals.get_mut(name) {
                        *existing = value;
                    }
                }
                Ok(())
            }
            _ => Err(RuntimeError::new("invalid cell object")),
        }
    }

    #[inline(always)]
    pub(super) fn load_fast_local(&mut self, idx: usize) -> Result<Value, RuntimeError> {
        let cached = {
            let frame = self.frames.last().expect("frame exists");
            if idx < frame.fast_locals.len() {
                frame.fast_locals[idx].clone()
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
            let value = frame
                .locals
                .get(&name)
                .cloned()
                .or_else(|| {
                    frame
                        .code
                        .cellvar_to_index
                        .get(&name)
                        .and_then(|cell_idx| frame.cells.get(*cell_idx))
                        .and_then(|cell| match &*cell.kind() {
                            Object::Cell(cell_data) => cell_data.value.clone(),
                            _ => None,
                        })
                })
                .or_else(|| {
                    frame
                        .code
                        .freevars
                        .iter()
                        .position(|free_name| free_name == &name)
                        .map(|free_idx| frame.code.cellvars.len() + free_idx)
                        .and_then(|cell_idx| frame.cells.get(cell_idx))
                        .map(|cell| Value::Cell(cell.clone()))
                });
            (name, value)
        };

        let value = value.ok_or_else(|| {
            if self.trace_flags.fast_local_unbound
                && let Some(frame) = self.frames.last()
            {
                let location = frame.code.locations.get(frame.last_ip);
                eprintln!(
                    "[fast-local-unbound] fn={} file={} line={} col={} ip={} name={}",
                    frame.code.name,
                    frame.code.filename,
                    location.map(|loc| loc.line).unwrap_or(0),
                    location.map(|loc| loc.column).unwrap_or(0),
                    frame.last_ip,
                    name
                );
            }
            RuntimeError::new(format!("local '{name}' not set"))
        })?;
        if let Some(frame) = self.frames.last_mut()
            && let Some(slot) = frame.fast_locals.get_mut(idx)
        {
            Self::write_fast_local_slot(slot, value.clone());
        }
        Ok(value)
    }

    #[inline(always)]
    pub(super) fn store_fast_local(
        &mut self,
        idx: usize,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let name = {
            let frame = self.frames.last().expect("frame exists");
            frame
                .code
                .names
                .get(idx)
                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                .clone()
        };
        let is_unbound_marker = self.is_fast_local_unbound_marker(&value);
        let frame = self.frames.last_mut().expect("frame exists");
        if is_unbound_marker {
            if let Some(slot) = frame.fast_locals.get_mut(idx) {
                *slot = None;
            } else {
                return Err(RuntimeError::new("name index out of range"));
            }
            if let Some(cell_idx) = frame.code.cellvar_to_index.get(&name).copied()
                && let Some(cell) = frame.cells.get(cell_idx)
                && let Object::Cell(cell_data) = &mut *cell.kind_mut()
            {
                cell_data.value = None;
            }
            if let Some(free_idx) = frame.code.freevars.iter().position(|free| free == &name)
                && let Some(cell) = frame.cells.get(frame.code.cellvars.len() + free_idx)
                && let Object::Cell(cell_data) = &mut *cell.kind_mut()
            {
                cell_data.value = None;
            }
            frame.locals.remove(&name);
            return Ok(());
        }
        if let Some(slot) = frame.fast_locals.get_mut(idx) {
            // CPython STORE_FAST writes directly to the locals-plus slot, even when
            // that slot temporarily backs a cellvar (for example LOAD_FAST_AND_CLEAR
            // save/restore patterns in comprehensions).
            Self::write_fast_local_slot(slot, value.clone());
        } else {
            return Err(RuntimeError::new("name index out of range"));
        }
        if let Some(existing) = frame.locals.get_mut(&name) {
            *existing = value;
        }
        Ok(())
    }

    fn take_fast_local_optional(
        &mut self,
        idx: usize,
    ) -> Result<(String, Option<Value>), RuntimeError> {
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
        let mut value = if let Some(slot) = frame.fast_locals.get_mut(idx) {
            match slot {
                Some(Value::Cell(cell)) if frame.code.cellvar_to_index.contains_key(&name) => {
                    let mut cell_value = None;
                    if let Object::Cell(cell_data) = &mut *cell.kind_mut() {
                        cell_value = cell_data.value.take();
                    }
                    *slot = Some(Value::Cell(cell.clone()));
                    cell_value
                }
                _ => slot.take(),
            }
        } else {
            None
        };
        if value.is_none() {
            value = frame.locals.remove(&name);
        }
        if value.is_none()
            && let Some(cell_idx) = frame.code.cellvar_to_index.get(&name).copied()
            && let Some(cell) = frame.cells.get(cell_idx)
            && let Object::Cell(cell_data) = &mut *cell.kind_mut()
        {
            value = cell_data.value.take();
        }
        if value.is_none()
            && let Some(free_idx) = frame.code.freevars.iter().position(|free| free == &name)
            && let Some(cell) = frame.cells.get(frame.code.cellvars.len() + free_idx)
            && let Object::Cell(cell_data) = &mut *cell.kind_mut()
        {
            value = cell_data.value.take();
        }
        frame.locals.remove(&name);
        Ok((name, value))
    }

    pub(super) fn take_fast_local(&mut self, idx: usize) -> Result<Value, RuntimeError> {
        let (name, value) = self.take_fast_local_optional(idx)?;
        value.ok_or_else(|| {
            if self.trace_flags.fast_local_unbound
                && let Some(frame) = self.frames.last()
            {
                let location = frame.code.locations.get(frame.last_ip);
                eprintln!(
                    "[fast-local-unbound] fn={} file={} line={} col={} ip={} name={}",
                    frame.code.name,
                    frame.code.filename,
                    location.map(|loc| loc.line).unwrap_or(0),
                    location.map(|loc| loc.column).unwrap_or(0),
                    frame.last_ip,
                    name
                );
            }
            RuntimeError::new(format!("local '{name}' not set"))
        })
    }

    pub(super) fn ensure_frame_module_locals_dict(&mut self, frame_index: usize) -> ObjRef {
        if let Some(existing) = self.frames[frame_index].module_locals_dict.clone() {
            return existing;
        }
        let module = self.frames[frame_index].module.clone();
        if let Object::Module(module_data) = &*module.kind()
            && let Some(existing) = module_data.dict.clone()
        {
            self.frames[frame_index].module_locals_dict = Some(existing.clone());
            return existing;
        }
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
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data.dict = Some(dict.clone());
        }
        self.frames[frame_index].module_locals_dict = Some(dict.clone());
        dict
    }

    pub(super) fn active_module_frame_index(&self, module: &ObjRef) -> Option<usize> {
        if self
            .frames
            .last()
            .is_some_and(|frame| frame.is_module && frame.module.id() == module.id())
        {
            Some(self.frames.len().saturating_sub(1))
        } else {
            self.frames
                .iter()
                .rposition(|frame| frame.is_module && frame.module.id() == module.id())
        }
    }

    fn module_namespace_key_name(&mut self, key: &Value) -> Option<String> {
        match key {
            Value::Str(name) => Some(name.clone()),
            // Preserve CPython-like behavior for str-like object keys written via
            // globals()[key] by coercing non-primitive object keys through str().
            // Primitive non-string keys remain non-name entries and are ignored.
            Value::None
            | Value::Bool(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Float(_)
            | Value::Complex { .. } => None,
            other => {
                if self.frames.is_empty() {
                    return None;
                }
                match self.builtin_str(vec![other.clone()], HashMap::new()) {
                    Ok(Value::Str(name)) => Some(name),
                    _ => None,
                }
            }
        }
    }

    pub(super) fn sync_module_locals_dict_to_module(&mut self, module: &ObjRef, dict: &ObjRef) {
        let mut map = HashMap::new();
        if let Object::Dict(entries) = &*dict.kind() {
            for (key, value) in entries {
                if let Some(name) = self.module_namespace_key_name(key) {
                    map.insert(name.clone(), value.clone());
                }
            }
        }
        let mut version = None;
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data.globals = map;
            module_data.dict = Some(dict.clone());
            module_data.touch_globals_version();
            version = Some(module_data.globals_version);
        }
        if let Some(version) = version {
            self.propagate_module_globals_version(module.id(), version);
        }
    }

    pub(super) fn module_namespace_lookup(&self, frame: &Frame, name: &str) -> Option<Value> {
        if frame.return_class
            && let Some(namespace) = &frame.class_namespace
            && let Some(value) = self.class_namespace_lookup_name(namespace, name)
        {
            return Some(value);
        }
        if let Some(dict) = &frame.module_locals_dict {
            return dict_get_value(dict, &Value::Str(name.to_string()));
        }
        if let Object::Module(module_data) = &*frame.module.kind() {
            return module_data.globals.get(name).cloned();
        }
        None
    }

    pub(super) fn frame_local_value(frame: &Frame, name: &str) -> Option<Value> {
        if let Some(idx) = frame.code.name_to_index.get(name).copied()
            && idx < frame.fast_locals.len()
            && let Some(value) = &frame.fast_locals[idx]
        {
            return Some(value.clone());
        }
        if let Some(value) = frame.locals.get(name) {
            return Some(value.clone());
        }
        None
    }

    pub(super) fn lookup_name_with_index(
        &self,
        name_index: usize,
        name: &str,
    ) -> Result<Value, RuntimeError> {
        if let Some(frame) = self.frames.last() {
            if let Some(value) = frame.fast_locals.get(name_index).and_then(Option::as_ref) {
                return Ok(value.clone());
            }
            if let Some(value) = frame.locals.get(name) {
                return Ok(value.clone());
            }
            if let Some(fallback) = &frame.locals_fallback
                && let Some(value) = fallback.get(name)
            {
                return Ok(value.clone());
            }
            if let Some(value) = self.module_namespace_lookup(frame, name) {
                return Ok(value);
            }
            if let Some(fallback) = &frame.globals_fallback
                && let Object::Module(module_data) = &*fallback.kind()
                && let Some(value) = module_data.globals.get(name)
            {
                return Ok(value.clone());
            }
        }
        if let Some(value) = self.builtins.get(name).cloned() {
            return Ok(value);
        }
        if let Some(module) = self.modules.get("builtins")
            && let Object::Module(module_data) = &*module.kind()
            && let Some(value) = module_data.globals.get(name)
        {
            return Ok(value.clone());
        }
        Err(RuntimeError::new(format!("name '{name}' is not defined")))
    }

    pub(super) fn store_name_by_index(
        &mut self,
        name_index: usize,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let mut touched_module_version: Option<(u64, u64)> = None;
        let mut class_namespace_store: Option<(Value, String, Value)> = None;
        if let Some(frame) = self.frames.last_mut() {
            let name = frame
                .code
                .names
                .get(name_index)
                .ok_or_else(|| RuntimeError::new("name index out of range"))?;
            let name = name.clone();
            let has_fast_slot = name_index < frame.fast_locals.len();
            if frame.is_module {
                if has_fast_slot && let Some(slot) = frame.fast_locals.get_mut(name_index) {
                    Self::write_fast_local_slot(slot, value.clone());
                }
                if frame.return_class {
                    frame.locals.insert(name.clone(), value.clone());
                    if let Some(namespace) = frame.class_namespace.clone() {
                        class_namespace_store = Some((namespace, name, value));
                    } else if let Some(dict) = frame.module_locals_dict.clone() {
                        dict_set_value(&dict, Value::Str(name), value);
                    }
                } else {
                    if let Some(dict) = frame.module_locals_dict.clone() {
                        dict_set_value(&dict, Value::Str(name.clone()), value.clone());
                    }
                    if let Object::Module(module_data) = &mut *frame.module.kind_mut() {
                        if let Some(existing) = module_data.globals.get_mut(name.as_str()) {
                            *existing = value;
                        } else {
                            module_data.globals.insert(name, value);
                        }
                        module_data.touch_globals_version();
                        touched_module_version =
                            Some((frame.module.id(), module_data.globals_version));
                    }
                }
            } else {
                if has_fast_slot && let Some(slot) = frame.fast_locals.get_mut(name_index) {
                    Self::write_fast_local_slot(slot, value.clone());
                }
                if let Some(existing) = frame.locals.get_mut(name.as_str()) {
                    *existing = value;
                } else {
                    // Keep fast locals authoritative; only retain truly dynamic names here.
                    if !has_fast_slot {
                        frame.locals.insert(name, value);
                    }
                }
            }
        }
        if let Some((namespace, name, value)) = class_namespace_store {
            self.class_namespace_set_name(&namespace, name, value)?;
        }
        if let Some((module_id, module_version)) = touched_module_version {
            self.propagate_module_globals_version(module_id, module_version);
        }
        Ok(())
    }

    pub(super) fn store_name(&mut self, name: &str, value: Value) -> Result<(), RuntimeError> {
        let mut module_write: Option<(ObjRef, Value)> = None;
        let mut class_namespace_store: Option<(Value, String, Value)> = None;
        if let Some(frame) = self.frames.last_mut() {
            if frame.is_module {
                if let Some(slot_idx) = frame.code.name_to_index.get(name).copied()
                    && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
                {
                    Self::write_fast_local_slot(slot, value.clone());
                }
                if frame.return_class {
                    frame.locals.insert(name.to_string(), value.clone());
                    if let Some(namespace) = frame.class_namespace.clone() {
                        class_namespace_store = Some((namespace, name.to_string(), value));
                    } else if let Some(dict) = frame.module_locals_dict.clone() {
                        dict_set_value(&dict, Value::Str(name.to_string()), value);
                    }
                } else {
                    if let Some(dict) = frame.module_locals_dict.clone() {
                        dict_set_value(&dict, Value::Str(name.to_string()), value.clone());
                    }
                    module_write = Some((frame.module.clone(), value));
                }
            } else {
                if let Some(slot_idx) = frame.code.name_to_index.get(name).copied()
                    && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
                {
                    Self::write_fast_local_slot(slot, value.clone());
                }
                if let Some(existing) = frame.locals.get_mut(name) {
                    *existing = value;
                } else {
                    // Keep fast locals authoritative; only retain truly dynamic names here.
                    if !frame.code.name_to_index.contains_key(name) {
                        frame.locals.insert(name.to_string(), value);
                    }
                }
            }
        }
        if let Some((namespace, key, value)) = class_namespace_store {
            self.class_namespace_set_name(&namespace, key, value)?;
        }
        if let Some((module, value)) = module_write {
            self.upsert_module_global(&module, name, value);
        }
        Ok(())
    }

    #[inline]
    pub(super) fn upsert_module_global(&mut self, module: &ObjRef, name: &str, value: Value) {
        let slot_value = value.clone();
        let mut version = None;
        let mut namespace_dict = None;
        let mut is_builtins_module = false;
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            is_builtins_module = module_data.name == "builtins";
            if let Some(existing) = module_data.globals.get_mut(name) {
                *existing = value;
            } else {
                module_data.globals.insert(name.to_string(), value);
            }
            namespace_dict = module_data.dict.clone();
            module_data.touch_globals_version();
            version = Some(module_data.globals_version);
        }
        if let Some(dict) = namespace_dict {
            dict_set_value(&dict, Value::Str(name.to_string()), slot_value.clone());
        }
        if is_builtins_module {
            self.builtins.insert(name.to_string(), slot_value.clone());
            self.touch_builtins_version();
        }
        self.sync_module_frame_fast_local(module.id(), name, Some(slot_value));
        if let Some(version) = version {
            self.propagate_module_globals_version(module.id(), version);
        }
    }

    fn sync_module_frame_fast_local(&mut self, module_id: u64, name: &str, value: Option<Value>) {
        for frame in self.frames.iter_mut().rev() {
            if frame.is_module && frame.module.id() == module_id {
                let dict_key = Value::Str(name.to_string());
                if let Some(dict) = frame.module_locals_dict.as_ref() {
                    if let Some(stored) = &value {
                        dict_set_value(dict, dict_key.clone(), stored.clone());
                    } else {
                        let _ = dict_remove_value(dict, &dict_key);
                    }
                }
                if let Some(slot_idx) = frame.code.name_to_index.get(name).copied()
                    && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
                {
                    *slot = value.clone();
                }
                break;
            }
        }
    }

    pub(super) fn module_for_namespace_dict(&self, dict: &ObjRef) -> Option<ObjRef> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| {
                if frame.is_module
                    && let Some(module_dict) = frame.module_locals_dict.as_ref()
                    && module_dict.id() == dict.id()
                {
                    Some(frame.module.clone())
                } else {
                    None
                }
            })
            .or_else(|| {
                self.modules.values().find_map(|module| {
                    if let Object::Module(module_data) = &*module.kind()
                        && let Some(module_dict) = module_data.dict.as_ref()
                        && module_dict.id() == dict.id()
                    {
                        Some(module.clone())
                    } else {
                        None
                    }
                })
            })
    }

    pub(super) fn remove_module_global(&mut self, module: &ObjRef, name: &str) {
        let mut removed = false;
        let mut version = None;
        let mut namespace_dict = None;
        let mut is_builtins_module = false;
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            is_builtins_module = module_data.name == "builtins";
            removed = module_data.globals.remove(name).is_some();
            if removed {
                namespace_dict = module_data.dict.clone();
                module_data.touch_globals_version();
                version = Some(module_data.globals_version);
            }
        }
        if removed {
            if let Some(dict) = namespace_dict {
                let _ = dict_remove_value(&dict, &Value::Str(name.to_string()));
            }
            if is_builtins_module {
                self.builtins.remove(name);
                self.touch_builtins_version();
            }
            self.sync_module_frame_fast_local(module.id(), name, None);
            if let Some(version) = version {
                self.propagate_module_globals_version(module.id(), version);
            }
        }
    }

    fn sync_module_global_from_locals_dict_write(
        &mut self,
        dict: &ObjRef,
        key: &Value,
        value: Option<Value>,
    ) {
        let Some(module) = self.module_for_namespace_dict(dict) else {
            return;
        };
        let Some(name) = self.module_namespace_key_name(key) else {
            return;
        };
        match value {
            Some(value) => self.upsert_module_global(&module, &name, value),
            None => self.remove_module_global(&module, &name),
        }
    }

    fn sync_warnings_module_from_sys_modules_write(
        &mut self,
        dict: &ObjRef,
        key: &Value,
        value: Option<&Value>,
    ) {
        let Some(sys_modules) = self.sys_dict_obj("modules") else {
            return;
        };
        if dict.id() != sys_modules.id() {
            return;
        }
        if !matches!(key, Value::Str(name) if name == "warnings") {
            return;
        }
        let Some(Value::Module(warnings_module)) = value else {
            return;
        };
        let Ok(set_module) = self.load_attr_module(warnings_module, "_set_module") else {
            self.clear_active_exception();
            return;
        };
        let _ = self.call_internal_preserving_caller(
            set_module,
            vec![Value::Module(warnings_module.clone())],
            HashMap::new(),
        );
        self.clear_active_exception();
    }

    #[inline]
    fn current_site_index(&self) -> usize {
        let frame = self.frames.last().expect("frame exists");
        frame.last_ip
    }

    #[inline]
    fn clear_load_attr_site_cache(&mut self, site_index: usize) {
        if let Some(frame) = self.frames.last_mut()
            && let Some(slot) = frame.load_attr_inline_cache.get_mut(site_index)
        {
            *slot = [None, None];
        }
    }

    pub(super) fn instance_has_attr_shadow(&self, instance: &ObjRef, attr_name: &str) -> bool {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return false;
        };
        if instance_data.attrs.contains_key(attr_name) {
            return true;
        }
        if let Some(Value::Dict(dict_obj)) = instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR) {
            return dict_get_value(dict_obj, &Value::Str(attr_name.to_string())).is_some();
        }
        false
    }

    fn try_load_attr_instance_site_cache(
        &mut self,
        site_index: usize,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return Ok(None),
        };
        if self.instance_has_attr_shadow(instance, attr_name) {
            return Ok(None);
        }

        let ways = self
            .frames
            .last()
            .and_then(|frame| frame.load_attr_inline_cache.get(site_index))
            .cloned();
        let Some(ways) = ways else {
            return Ok(None);
        };
        for (way_idx, cached) in ways.into_iter().enumerate() {
            let Some(cached) = cached else {
                continue;
            };
            if class_ref.id() != cached.class_id {
                continue;
            }
            if self.class_attr_version(&class_ref) != cached.class_version {
                if let Some(frame) = self.frames.last_mut()
                    && let Some(slot) = frame.load_attr_inline_cache.get_mut(site_index)
                {
                    slot[way_idx] = None;
                }
                continue;
            }
            if cached.owner_class.id() != class_ref.id()
                && self.class_attr_version(&cached.owner_class) != cached.owner_class_version
            {
                if let Some(frame) = self.frames.last_mut()
                    && let Some(slot) = frame.load_attr_inline_cache.get_mut(site_index)
                {
                    slot[way_idx] = None;
                }
                continue;
            }

            let value = match cached.kind {
                LoadAttrSiteCacheKind::InstanceValue { value } => value.clone(),
                LoadAttrSiteCacheKind::InstanceFunction { function } => self
                    .heap
                    .alloc_bound_method(BoundMethod::new(function, instance.clone())),
                LoadAttrSiteCacheKind::InstanceBuiltin { builtin } => {
                    self.alloc_builtin_bound_method(builtin, instance.clone())
                }
                LoadAttrSiteCacheKind::InstanceClassMethod { descriptor } => {
                    match self.bind_classmethod_attr(&class_ref, &Value::Module(descriptor)) {
                        Some(bound) => bound,
                        None => continue,
                    }
                }
                LoadAttrSiteCacheKind::InstanceStaticMethod { descriptor } => {
                    match self.unwrap_staticmethod_attr(&Value::Module(descriptor)) {
                        Some(unwrapped) => unwrapped,
                        None => continue,
                    }
                }
            };
            return Ok(Some(value));
        }
        Ok(None)
    }

    #[inline]
    pub(super) fn is_load_attr_cacheable_plain_value(value: &Value) -> bool {
        match value {
            Value::None
            | Value::Bool(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Float(_)
            | Value::Complex { .. }
            | Value::Str(_)
            | Value::Tuple(_)
            | Value::Dict(_)
            | Value::DictKeys(_)
            | Value::DictValues(_)
            | Value::DictItems(_)
            | Value::Set(_)
            | Value::FrozenSet(_)
            | Value::Bytes(_)
            | Value::ByteArray(_)
            | Value::MemoryView(_)
            | Value::Iterator(_)
            | Value::Generator(_)
            | Value::Exception(_)
            | Value::ExceptionType(_)
            | Value::Slice(_)
            | Value::Code(_)
            | Value::Builtin(_)
            | Value::Cell(_) => true,
            Value::List(_)
            | Value::Module(_)
            | Value::Class(_)
            | Value::Instance(_)
            | Value::Super(_)
            | Value::Function(_)
            | Value::BoundMethod(_) => false,
        }
    }

    fn insert_load_attr_instance_site_cache_entry(
        &mut self,
        site_index: usize,
        entry: LoadAttrSiteCacheEntry,
    ) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        let Some(slot) = frame.load_attr_inline_cache.get_mut(site_index) else {
            return;
        };
        if let Some(existing) = slot[0].as_ref() {
            if existing.class_id == entry.class_id
                && existing.owner_class.id() == entry.owner_class.id()
            {
                slot[0] = Some(entry);
                return;
            }
        } else {
            slot[0] = Some(entry);
            return;
        }
        if let Some(existing) = slot[1].as_ref() {
            if existing.class_id == entry.class_id
                && existing.owner_class.id() == entry.owner_class.id()
            {
                slot[1] = Some(entry);
                return;
            }
        } else {
            slot[1] = Some(entry);
            return;
        }
        // Preserve the first way as most-recently-hot and use way-1 as replacement.
        slot[1] = Some(entry);
    }

    #[inline]
    fn mark_quickened_site(&mut self, site_index: usize, kind: QuickenedSiteKind) {
        if let Some(frame) = self.frames.last_mut()
            && let Some(slot) = frame.quickened_sites.get_mut(site_index)
        {
            *slot = kind;
        }
    }

    #[inline]
    fn clear_quickened_site(&mut self, site_index: usize) {
        if let Some(frame) = self.frames.last_mut()
            && let Some(slot) = frame.quickened_sites.get_mut(site_index)
        {
            *slot = QuickenedSiteKind::None;
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
        &mut self,
        name_idx: usize,
    ) -> Result<(Value, bool, u64, u64), RuntimeError> {
        let (name, mut value, globals_mapping) = {
            let frame = self.frames.last().expect("frame exists");
            let name = frame
                .code
                .names
                .get(name_idx)
                .ok_or_else(|| RuntimeError::new("name index out of range"))?
                .clone();
            let (value, mapping) =
                if let Object::Module(module_data) = &*frame.function_globals.kind() {
                    (
                        module_data.globals.get(&name).cloned(),
                        module_data
                            .globals
                            .get(Vm::FUNCTION_GLOBALS_MAPPING_KEY)
                            .cloned(),
                    )
                } else {
                    (None, None)
                };
            (name, value, mapping)
        };
        if value.is_none()
            && let Some(mapping) = globals_mapping.as_ref()
        {
            match self.getitem_value(mapping.clone(), Value::Str(name.clone())) {
                Ok(found) => value = Some(found),
                Err(err)
                    if runtime_error_matches_exception(&err, "KeyError")
                        || runtime_error_matches_exception(&err, "AttributeError") => {}
                Err(err) => return Err(err),
            }
        }
        let value = value.or_else(|| {
            let frame = self.frames.last().expect("frame exists");
            if let Some(fallback) = &frame.locals_fallback
                && let Some(value) = fallback.get(&name)
            {
                return Some(value.clone());
            }
            if let Some(fallback) = &frame.globals_fallback
                && let Object::Module(module_data) = &*fallback.kind()
            {
                return module_data.globals.get(&name).cloned();
            }
            None
        });
        let value = value
            .or_else(|| self.lookup_builtin_global(&name))
            .ok_or_else(|| RuntimeError::new(format!("name '{name}' is not defined")))?;
        let frame = self.frames.last().expect("frame exists");
        let cacheable = frame.locals_fallback.is_none()
            && frame.globals_fallback.is_none()
            && frame.function_globals_version != 0
            && globals_mapping.is_none();
        Ok((
            value,
            cacheable,
            frame.function_globals.id(),
            frame.function_globals_version,
        ))
    }

    fn lookup_builtin_global(&mut self, name: &str) -> Option<Value> {
        if let Some(value) = self.builtins.get(name).cloned() {
            return Some(value);
        }
        let value = self.modules.get("builtins").and_then(|module| {
            if let Object::Module(module_data) = &*module.kind() {
                module_data.globals.get(name).cloned()
            } else {
                None
            }
        });
        if let Some(value) = value.clone() {
            self.builtins.insert(name.to_string(), value.clone());
            self.touch_builtins_version();
        }
        value
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
    #[inline(always)]
    fn try_fast_terminal_return_simple_no_cells(&mut self, value: Value) -> Result<(), Value> {
        let can_fast_return = if self.frames.len() <= 1 {
            false
        } else {
            let frame = self.frames.last().expect("frame exists");
            frame.simple_one_arg_no_cells
                && frame.stack.is_empty()
                && !frame.discard_result
                && frame.active_exception.is_none()
                && !frame.expect_none_return
                && matches!(
                    frame.code.instructions.get(frame.ip),
                    Some(next) if next.opcode == Opcode::ReturnValue
                )
        };
        if !can_fast_return {
            return Err(value);
        }
        let frame = self.frames.pop().expect("frame exists");
        let caller = self.frames.last_mut().expect("caller frame exists");
        caller.stack.push(value);
        if frame.owner_class.is_none()
            && frame.code.fast_local_count == 1
            && frame.code.plain_positional_arg0_slot == Some(0)
        {
            self.recycle_simple_frame_clean_slot0_unchecked(frame);
        } else {
            self.recycle_simple_frame(frame);
        }
        Ok(())
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
    ) -> Option<FusedDirectOneArgNoCellsMetadata> {
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
            || Self::code_returns_generator_like(code)
            || code.is_comprehension
        {
            return None;
        }
        Some(FusedDirectOneArgNoCellsMetadata {
            func: func.clone(),
            func_epoch: func_data.call_cache_epoch,
            code: func_data.code.clone(),
            module: func_data.module.clone(),
            owner_class: func_data.owner_class.clone(),
        })
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
                        None => {
                            self.binary_sub_runtime(Value::Int(left_int), Value::Int(right_int))
                        }
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
                    None => self.binary_sub_runtime(Value::Int(left_int), Value::Int(right)),
                },
                other => self.binary_sub_runtime(other, Value::Int(right)),
            },
            Value::Bool(flag) => {
                let right = if flag { 1 } else { 0 };
                match left {
                    Value::Int(left_int) => match left_int.checked_sub(right) {
                        Some(diff) => Ok(Value::Int(diff)),
                        None => self.binary_sub_runtime(Value::Int(left_int), Value::Int(right)),
                    },
                    other => self.binary_sub_runtime(other, Value::Int(right)),
                }
            }
            right => self.binary_sub_runtime(left, right),
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
                None => self.binary_sub_runtime(Value::Int(left_int), Value::Int(right_int)),
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
        self.binary_sub_runtime(left, Value::Int(right_int))
    }

    fn dispatch_call_no_kwargs(
        &mut self,
        func: Value,
        args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        match func {
            Value::Function(func) => {
                self.push_function_call_from_obj(&func, args, HashMap::new())?;
            }
            Value::BoundMethod(method) => {
                let (function, receiver, dispatch_kind) = match &*method.kind() {
                    Object::BoundMethod(data) => (
                        data.function.clone(),
                        data.receiver.clone(),
                        data.dispatch_kind.clone(),
                    ),
                    _ => return Err(RuntimeError::type_error("attempted to call non-function")),
                };
                match dispatch_kind {
                    BoundMethodDispatchKind::Python => {
                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                        bound_args.push(self.receiver_value(&receiver)?);
                        bound_args.extend(args);
                        self.push_function_call_from_obj(&function, bound_args, HashMap::new())?;
                    }
                    BoundMethodDispatchKind::Native(native_kind) => {
                        let caller_depth = self.frames.len();
                        let caller_idx = caller_depth.saturating_sub(1);
                        let caller_ip = self
                            .frames
                            .get(caller_idx)
                            .map(|frame| frame.ip)
                            .unwrap_or(0);
                        let call_result =
                            self.call_native_method(native_kind, receiver, args, HashMap::new());
                        self.finalize_native_opcode_call(caller_depth, caller_ip, call_result)?;
                    }
                    BoundMethodDispatchKind::Generic => {
                        self.call_bound_method_via_call_internal(
                            function,
                            receiver,
                            args,
                            HashMap::new(),
                        )?;
                    }
                }
            }
            Value::Class(class) => {
                match self.call_internal(Value::Class(class), args, HashMap::new())? {
                    InternalCallOutcome::Value(value) => self.push_value(value),
                    InternalCallOutcome::CallerExceptionHandled => {}
                }
            }
            Value::Builtin(BuiltinFunction::BuildClass) => {
                let class_value = self.call_build_class(args, HashMap::new())?;
                if let Some(value) = class_value {
                    self.push_value(value);
                }
            }
            Value::Builtin(builtin) => {
                if let Some(value) = self.try_fast_builtin_no_kwargs(builtin, args.as_slice())? {
                    self.push_value(value);
                    return Ok(());
                }
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
                if Self::cpython_proxy_raw_ptr_from_value(&receiver).is_some() {
                    match self.call_internal(receiver, args, HashMap::new())? {
                        InternalCallOutcome::Value(value) => self.push_value(value),
                        InternalCallOutcome::CallerExceptionHandled => {}
                    }
                    return Ok(());
                }
                match self.load_attr_instance(&instance, "__call__") {
                    Ok(AttrAccessOutcome::Value(call_target)) => {
                        self.dispatch_call_no_kwargs(call_target, args)?;
                        return Ok(());
                    }
                    Ok(AttrAccessOutcome::ExceptionHandled) => return Ok(()),
                    Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {}
                    Err(err) => return Err(err),
                }
                if let Some(call_target) =
                    self.lookup_bound_special_method(&receiver, "__call__")?
                {
                    self.dispatch_call_no_kwargs(call_target, args)?;
                    return Ok(());
                }
                return Err(RuntimeError::type_error("attempted to call non-function"));
            }
            Value::ExceptionType(name) => {
                let value = self.instantiate_exception_type(&name, &args, &HashMap::new())?;
                self.push_value(value);
            }
            _ => return Err(RuntimeError::type_error("attempted to call non-function")),
        }
        Ok(())
    }

    fn dispatch_call_no_kwargs_ignoring_result(
        &mut self,
        caller_idx: usize,
        func: Value,
        args: Vec<Value>,
        completion_value: Option<Value>,
    ) -> Result<bool, RuntimeError> {
        let depth_before = self.frames.len();
        self.dispatch_call_no_kwargs(func, args)?;
        if self.frames.len() > depth_before {
            if let Some(frame) = self.frames.last_mut() {
                frame.discard_result = true;
            }
            if let Some(value) = completion_value {
                self.push_value_to_caller_frame(caller_idx, value)?;
            }
            return Ok(true);
        }
        let _ = self.pop_value()?;
        if let Some(value) = completion_value {
            self.push_value(value);
        }
        Ok(false)
    }

    #[inline]
    fn dispatch_small_arity_no_kwargs_call(
        &mut self,
        func: &Value,
        args: &mut Vec<Value>,
    ) -> Result<bool, RuntimeError> {
        match func {
            Value::Builtin(builtin) => {
                if let Some(result) = self.try_fast_builtin_no_kwargs(*builtin, args.as_slice())? {
                    args.clear();
                    self.push_value(result);
                    return Ok(true);
                }
                Ok(false)
            }
            Value::Function(func_obj) => match args.len() {
                0 => {
                    self.push_function_call_from_obj(func_obj, Vec::new(), HashMap::new())?;
                    Ok(true)
                }
                1 => {
                    let arg0 = args.pop().expect("len checked");
                    self.push_function_call_one_arg_from_obj(func_obj, arg0)?;
                    Ok(true)
                }
                2 => {
                    let arg1 = args.pop().expect("len checked");
                    let arg0 = args.pop().expect("len checked");
                    self.push_function_call_two_args_from_obj(func_obj, arg0, arg1)?;
                    Ok(true)
                }
                3 => {
                    let arg2 = args.pop().expect("len checked");
                    let arg1 = args.pop().expect("len checked");
                    let arg0 = args.pop().expect("len checked");
                    self.push_function_call_three_args_from_obj(func_obj, arg0, arg1, arg2)?;
                    Ok(true)
                }
                _ => Ok(false),
            },
            Value::BoundMethod(method_obj) => match args.len() {
                0 => {
                    self.push_bound_method_call_zero_args_from_obj(method_obj)?;
                    Ok(true)
                }
                1 => {
                    let arg0 = args.pop().expect("len checked");
                    self.push_bound_method_call_one_arg_from_obj(method_obj, arg0)?;
                    Ok(true)
                }
                2 => {
                    let arg1 = args.pop().expect("len checked");
                    let arg0 = args.pop().expect("len checked");
                    self.push_bound_method_call_two_args_from_obj(method_obj, arg0, arg1)?;
                    Ok(true)
                }
                3 => {
                    let arg2 = args.pop().expect("len checked");
                    let arg1 = args.pop().expect("len checked");
                    let arg0 = args.pop().expect("len checked");
                    self.push_bound_method_call_three_args_from_obj(method_obj, arg0, arg1, arg2)?;
                    Ok(true)
                }
                _ => Ok(false),
            },
            _ => Ok(false),
        }
    }

    #[inline]
    fn try_fast_builtin_no_kwargs(
        &mut self,
        builtin: BuiltinFunction,
        args: &[Value],
    ) -> Result<Option<Value>, RuntimeError> {
        match args {
            [] => Ok(self.try_fast_builtin_zero_arg_no_kwargs(builtin)),
            [arg0] => self.try_fast_builtin_single_arg_no_kwargs(builtin, arg0),
            _ => Ok(None),
        }
    }

    #[inline]
    fn try_fast_builtin_single_arg_no_kwargs(
        &mut self,
        builtin: BuiltinFunction,
        arg0: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        if builtin == BuiltinFunction::Bool {
            return Ok(Some(Value::Bool(self.truthy_from_value(arg0)?)));
        }
        if builtin != BuiltinFunction::Len {
            return Ok(None);
        }
        Ok(match arg0 {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => Some(Value::Int(values.len() as i64)),
                _ => None,
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => Some(Value::Int(values.len() as i64)),
                _ => None,
            },
            Value::Dict(obj) => match &*obj.kind() {
                Object::Dict(values) => Some(Value::Int(values.len() as i64)),
                _ => None,
            },
            Value::Set(obj) => match &*obj.kind() {
                Object::Set(values) => Some(Value::Int(values.len() as i64)),
                _ => None,
            },
            Value::FrozenSet(obj) => match &*obj.kind() {
                Object::FrozenSet(values) => Some(Value::Int(values.len() as i64)),
                _ => None,
            },
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(values) => Some(Value::Int(values.len() as i64)),
                _ => None,
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(values) => Some(Value::Int(values.len() as i64)),
                _ => None,
            },
            Value::DictKeys(view_obj)
            | Value::DictValues(view_obj)
            | Value::DictItems(view_obj) => {
                if let Object::DictView(view) = &*view_obj.kind()
                    && let Object::Dict(values) = &*view.dict.kind()
                {
                    return Ok(Some(Value::Int(values.len() as i64)));
                }
                None
            }
            Value::Instance(instance) => {
                if let Some(values) = self.namedtuple_instance_values(instance) {
                    return Ok(Some(Value::Int(values.len() as i64)));
                }
                if let Some(backing_list) = self.instance_backing_list(instance)
                    && let Object::List(values) = &*backing_list.kind()
                {
                    return Ok(Some(Value::Int(values.len() as i64)));
                }
                None
            }
            Value::Iterator(iterator) => {
                if let Some((start, stop, step)) = self.range_object_parts(iterator) {
                    return Ok(Some(value_from_bigint(
                        self.range_object_len_bigint(&start, &stop, &step),
                    )));
                }
                None
            }
            _ => None,
        })
    }

    #[inline]
    fn try_fast_builtin_zero_arg_no_kwargs(&self, builtin: BuiltinFunction) -> Option<Value> {
        if builtin == BuiltinFunction::Bool {
            return Some(Value::Bool(false));
        }
        None
    }

    #[inline]
    pub(crate) fn cpython_proxy_callable_has_bound_self(&mut self, callable: &Value) -> bool {
        if Self::cpython_proxy_raw_ptr_from_value(callable).is_none() {
            return false;
        }
        let saved_active_exception = self
            .frames
            .last()
            .and_then(|frame| frame.active_exception.clone());
        let resolved = self.load_cpython_proxy_attr_for_value(callable, "__self__");
        if resolved.is_none() {
            if let Some(frame) = self.frames.last_mut() {
                frame.active_exception = saved_active_exception;
            }
            // Optional `__self__` probe: clear both VM-side and C-API error state when absent.
            unsafe { PyErr_Clear() };
        }
        resolved.is_some_and(|value| !matches!(value, Value::None))
    }

    #[inline]
    pub(crate) fn bind_cpython_proxy_descriptor_callable(
        &mut self,
        callable: &Value,
        receiver: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        if Self::cpython_proxy_raw_ptr_from_value(callable).is_none()
            || Self::cpython_proxy_raw_ptr_from_value(receiver).is_none()
            || self.value_type_name_for_error(callable) != "cython_function_or_method"
        {
            return Ok(None);
        }
        let Some(getter) = self.load_cpython_proxy_attr_for_value(callable, "__get__") else {
            // Optional descriptor probe: clear error state when attribute is absent.
            self.clear_active_exception();
            unsafe { PyErr_Clear() };
            return Ok(None);
        };
        let owner = self
            .class_of_value(receiver)
            .map(Value::Class)
            .unwrap_or(Value::None);
        match self.call_internal(getter, vec![receiver.clone(), owner], HashMap::new())? {
            InternalCallOutcome::Value(bound_callable) => Ok(Some(bound_callable)),
            InternalCallOutcome::CallerExceptionHandled => Ok(None),
        }
    }

    #[inline]
    fn call_bound_method_via_call_internal(
        &mut self,
        function: ObjRef,
        receiver: ObjRef,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        let Some(callable) = value_from_object_ref(function.clone()) else {
            return Err(RuntimeError::type_error("attempted to call non-function"));
        };
        let receiver_value = self.receiver_value(&receiver)?;
        let callable_is_proxy = Self::cpython_proxy_raw_ptr_from_value(&callable).is_some();
        let receiver_is_proxy = Self::cpython_proxy_raw_ptr_from_value(&receiver_value).is_some();
        let callable_type_name = self.value_type_name_for_error(&callable);
        let callable_has_bound_self =
            callable_is_proxy && self.cpython_proxy_callable_has_bound_self(&callable);
        if callable_is_proxy
            && receiver_is_proxy
            && callable_type_name == "cython_function_or_method"
            && let Some(bound_callable) =
                self.bind_cpython_proxy_descriptor_callable(&callable, &receiver_value)?
        {
            match self.call_internal(bound_callable, args, kwargs)? {
                InternalCallOutcome::Value(value) => {
                    self.push_value(value);
                    return Ok(());
                }
                InternalCallOutcome::CallerExceptionHandled => return Ok(()),
            }
        }
        let proxy_callable_is_already_bound = callable_is_proxy
            && receiver_is_proxy
            && (matches!(
                callable_type_name.as_str(),
                "builtin_function_or_method" | "method"
            ) || callable_has_bound_self);
        if self
            .host
            .env_var_os("PYRS_TRACE_PROXY_BOUND_CALL")
            .is_some()
        {
            eprintln!(
                "[proxy-bound-call] helper callable_type={} callable_is_proxy={} receiver_is_proxy={} has_bound_self={} already_bound={} args={} kwargs={}",
                callable_type_name,
                callable_is_proxy,
                receiver_is_proxy,
                callable_has_bound_self,
                proxy_callable_is_already_bound,
                args.len(),
                kwargs.len()
            );
        }
        let call_args = if proxy_callable_is_already_bound {
            args
        } else {
            let mut bound_args = Vec::with_capacity(args.len() + 1);
            bound_args.push(receiver_value);
            bound_args.append(&mut args);
            bound_args
        };
        match self.call_internal(callable, call_args, kwargs)? {
            InternalCallOutcome::Value(value) => {
                self.push_value(value);
                Ok(())
            }
            InternalCallOutcome::CallerExceptionHandled => Ok(()),
        }
    }

    #[inline]
    pub(super) fn push_bound_method_call_zero_args_from_obj(
        &mut self,
        method: &ObjRef,
    ) -> Result<(), RuntimeError> {
        let (function, receiver, dispatch_kind) = {
            let method_kind = method.kind();
            let method_data = match &*method_kind {
                Object::BoundMethod(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            (
                method_data.function.clone(),
                method_data.receiver.clone(),
                method_data.dispatch_kind.clone(),
            )
        };
        match dispatch_kind {
            BoundMethodDispatchKind::Python => {
                let receiver_value = self.receiver_value(&receiver)?;
                self.push_function_call_one_arg_from_obj(&function, receiver_value)
            }
            BoundMethodDispatchKind::Native(native_kind) => {
                let caller_depth = self.frames.len();
                let caller_idx = caller_depth.saturating_sub(1);
                let caller_ip = self
                    .frames
                    .get(caller_idx)
                    .map(|frame| frame.ip)
                    .unwrap_or(0);
                let call_result =
                    self.call_native_method(native_kind, receiver, Vec::new(), HashMap::new());
                self.finalize_native_opcode_call(caller_depth, caller_ip, call_result)
            }
            BoundMethodDispatchKind::Generic => self.call_bound_method_via_call_internal(
                function,
                receiver,
                Vec::new(),
                HashMap::new(),
            ),
        }
    }

    #[inline]
    pub(super) fn push_bound_method_call_one_arg_from_obj(
        &mut self,
        method: &ObjRef,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        let (function, receiver, dispatch_kind) = {
            let method_kind = method.kind();
            let method_data = match &*method_kind {
                Object::BoundMethod(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            (
                method_data.function.clone(),
                method_data.receiver.clone(),
                method_data.dispatch_kind.clone(),
            )
        };
        match dispatch_kind {
            BoundMethodDispatchKind::Python => {
                let receiver_value = self.receiver_value(&receiver)?;
                self.push_function_call_two_args_from_obj(&function, receiver_value, arg0)
            }
            BoundMethodDispatchKind::Native(native_kind) => {
                let caller_depth = self.frames.len();
                let caller_idx = caller_depth.saturating_sub(1);
                let caller_ip = self
                    .frames
                    .get(caller_idx)
                    .map(|frame| frame.ip)
                    .unwrap_or(0);
                let call_result =
                    self.call_native_method(native_kind, receiver, vec![arg0], HashMap::new());
                self.finalize_native_opcode_call(caller_depth, caller_ip, call_result)
            }
            BoundMethodDispatchKind::Generic => self.call_bound_method_via_call_internal(
                function,
                receiver,
                vec![arg0],
                HashMap::new(),
            ),
        }
    }

    #[inline]
    pub(super) fn push_bound_method_call_two_args_from_obj(
        &mut self,
        method: &ObjRef,
        arg0: Value,
        arg1: Value,
    ) -> Result<(), RuntimeError> {
        let (function, receiver, dispatch_kind) = {
            let method_kind = method.kind();
            let method_data = match &*method_kind {
                Object::BoundMethod(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            (
                method_data.function.clone(),
                method_data.receiver.clone(),
                method_data.dispatch_kind.clone(),
            )
        };
        match dispatch_kind {
            BoundMethodDispatchKind::Python => {
                let receiver_value = self.receiver_value(&receiver)?;
                self.push_function_call_three_args_from_obj(&function, receiver_value, arg0, arg1)
            }
            BoundMethodDispatchKind::Native(native_kind) => {
                let caller_depth = self.frames.len();
                let caller_idx = caller_depth.saturating_sub(1);
                let caller_ip = self
                    .frames
                    .get(caller_idx)
                    .map(|frame| frame.ip)
                    .unwrap_or(0);
                let call_result = self.call_native_method(
                    native_kind,
                    receiver,
                    vec![arg0, arg1],
                    HashMap::new(),
                );
                self.finalize_native_opcode_call(caller_depth, caller_ip, call_result)
            }
            BoundMethodDispatchKind::Generic => self.call_bound_method_via_call_internal(
                function,
                receiver,
                vec![arg0, arg1],
                HashMap::new(),
            ),
        }
    }

    #[inline]
    pub(super) fn push_bound_method_call_three_args_from_obj(
        &mut self,
        method: &ObjRef,
        arg0: Value,
        arg1: Value,
        arg2: Value,
    ) -> Result<(), RuntimeError> {
        let (function, receiver, dispatch_kind) = {
            let method_kind = method.kind();
            let method_data = match &*method_kind {
                Object::BoundMethod(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            (
                method_data.function.clone(),
                method_data.receiver.clone(),
                method_data.dispatch_kind.clone(),
            )
        };
        match dispatch_kind {
            BoundMethodDispatchKind::Python => {
                let receiver_value = self.receiver_value(&receiver)?;
                self.push_function_call_from_obj(
                    &function,
                    vec![receiver_value, arg0, arg1, arg2],
                    HashMap::new(),
                )
            }
            BoundMethodDispatchKind::Native(native_kind) => {
                let caller_depth = self.frames.len();
                let caller_idx = caller_depth.saturating_sub(1);
                let caller_ip = self
                    .frames
                    .get(caller_idx)
                    .map(|frame| frame.ip)
                    .unwrap_or(0);
                let call_result = self.call_native_method(
                    native_kind,
                    receiver,
                    vec![arg0, arg1, arg2],
                    HashMap::new(),
                );
                self.finalize_native_opcode_call(caller_depth, caller_ip, call_result)
            }
            BoundMethodDispatchKind::Generic => self.call_bound_method_via_call_internal(
                function,
                receiver,
                vec![arg0, arg1, arg2],
                HashMap::new(),
            ),
        }
    }

    pub(super) fn push_function_call_one_arg_from_obj(
        &mut self,
        func: &ObjRef,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        enum CachedCallAction {
            SimpleNoCells {
                code: Rc<CodeObject>,
                module: ObjRef,
                owner_class: Option<ObjRef>,
            },
            SimpleNoCellsFromFunc,
            SimplePositional {
                code: Rc<CodeObject>,
                module: ObjRef,
                owner_class: Option<ObjRef>,
                closure: Vec<ObjRef>,
            },
            SimplePositionalFromFunc,
            Generic,
        }

        let site_index = self.current_site_index();
        let mut clear_cached = false;
        let mut cached_action = None;
        if let Some(frame) = self.frames.last()
            && let Some(slot) = frame.one_arg_inline_cache.get(site_index)
            && let Some(entry) = slot.as_ref()
        {
            if entry.func_id != func.id() {
                clear_cached = true;
            } else {
                // For stable no-cells hot paths we can trust cached metadata directly.
                if entry.hot_path == OneArgCallHotPath::SimplePositionalNoCells
                    && let (Some(code), Some(module)) =
                        (entry.cached_code.as_ref(), entry.cached_module.as_ref())
                {
                    cached_action = Some(CachedCallAction::SimpleNoCells {
                        code: code.clone(),
                        module: module.clone(),
                        owner_class: entry.cached_owner_class.clone(),
                    });
                }
                if cached_action.is_none() {
                    let valid = {
                        let func_kind = func.kind();
                        match &*func_kind {
                            Object::Function(data) => data.call_cache_epoch == entry.func_epoch,
                            _ => false,
                        }
                    };
                    if !valid {
                        clear_cached = true;
                    } else {
                        cached_action = match entry.hot_path {
                            OneArgCallHotPath::SimplePositionalNoCells => {
                                if let (Some(code), Some(module)) =
                                    (entry.cached_code.as_ref(), entry.cached_module.as_ref())
                                {
                                    Some(CachedCallAction::SimpleNoCells {
                                        code: code.clone(),
                                        module: module.clone(),
                                        owner_class: entry.cached_owner_class.clone(),
                                    })
                                } else {
                                    Some(CachedCallAction::SimpleNoCellsFromFunc)
                                }
                            }
                            OneArgCallHotPath::SimplePositional => {
                                if let (Some(code), Some(module), Some(closure)) = (
                                    entry.cached_code.as_ref(),
                                    entry.cached_module.as_ref(),
                                    entry.cached_closure.as_ref(),
                                ) {
                                    Some(CachedCallAction::SimplePositional {
                                        code: code.clone(),
                                        module: module.clone(),
                                        owner_class: entry.cached_owner_class.clone(),
                                        closure: closure.clone(),
                                    })
                                } else {
                                    Some(CachedCallAction::SimplePositionalFromFunc)
                                }
                            }
                            OneArgCallHotPath::Generic => Some(CachedCallAction::Generic),
                        };
                    }
                }
            }
        }
        if let Some(action) = cached_action {
            return match action {
                CachedCallAction::SimpleNoCells {
                    code,
                    module,
                    owner_class,
                } => self.push_simple_positional_function_frame_one_arg_no_cells_cached_ref(
                    &code,
                    &module,
                    owner_class.as_ref(),
                    arg0,
                ),
                CachedCallAction::SimpleNoCellsFromFunc => self
                    .push_simple_positional_function_frame_one_arg_no_cells_from_func(func, arg0),
                CachedCallAction::SimplePositional {
                    code,
                    module,
                    owner_class,
                    closure,
                } => self.push_simple_positional_function_frame_one_arg(
                    code,
                    module,
                    owner_class,
                    closure,
                    arg0,
                ),
                CachedCallAction::SimplePositionalFromFunc => {
                    self.push_simple_positional_function_frame_one_arg_from_func(func, arg0)
                }
                CachedCallAction::Generic => {
                    self.push_function_call_from_obj(func, vec![arg0], HashMap::new())
                }
            };
        }
        if clear_cached
            && let Some(frame) = self.frames.last_mut()
            && let Some(slot) = frame.one_arg_inline_cache.get_mut(site_index)
        {
            *slot = None;
        }

        let (code, module, owner_class, simple_positional_path, no_cells_hot, func_epoch) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            let code = func_data.code.clone();
            let simple_positional_path = func_data.plain_positional_call_arity == Some(1);
            let no_cells_hot = simple_positional_path
                && code.plain_positional_arg0_cell.is_none()
                && code.cellvars.is_empty()
                && func_data.closure.is_empty()
                && !Self::code_returns_generator_like(&code)
                && !code.is_comprehension;
            (
                code,
                func_data.module.clone(),
                func_data.owner_class.clone(),
                simple_positional_path,
                no_cells_hot,
                func_data.call_cache_epoch,
            )
        };
        if simple_positional_path {
            let closure = if no_cells_hot {
                None
            } else {
                let func_kind = func.kind();
                let func_data = match &*func_kind {
                    Object::Function(data) => data,
                    _ => return Err(RuntimeError::type_error("attempted to call non-function")),
                };
                Some(func_data.closure.clone())
            };
            if let Some(frame) = self.frames.last_mut()
                && let Some(slot) = frame.one_arg_inline_cache.get_mut(site_index)
            {
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
                    cached_closure: if no_cells_hot { None } else { closure.clone() },
                });
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
                closure.unwrap_or_default(),
                arg0,
            );
        }
        if let Some(frame) = self.frames.last_mut()
            && let Some(slot) = frame.one_arg_inline_cache.get_mut(site_index)
        {
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
        self.push_function_call_from_obj(func, vec![arg0], HashMap::new())
    }

    #[inline(always)]
    fn push_simple_positional_function_frame_one_arg_no_cells_from_func(
        &mut self,
        func: &ObjRef,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        let func_kind = func.kind();
        let func_data = match &*func_kind {
            Object::Function(data) => data,
            _ => return Err(RuntimeError::type_error("attempted to call non-function")),
        };
        if func_data.code.fast_local_count == 1
            && func_data.code.plain_positional_arg0_slot == Some(0)
        {
            self.push_simple_positional_function_frame_one_arg_slot0_no_cells_ref(
                &func_data.code,
                &func_data.module,
                func_data.owner_class.as_ref(),
                arg0,
            )
        } else {
            self.push_simple_positional_function_frame_one_arg_no_cells_ref(
                &func_data.code,
                &func_data.module,
                func_data.owner_class.as_ref(),
                arg0,
            )
        }
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
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
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
        if let Some(caller) = self.frames.last()
            && let Some(active_exception) = caller.active_exception.as_ref()
        {
            frame.active_exception = Some(Self::clone_active_exception_for_call(active_exception));
        }
        if slot_idx == Some(0) && frame.fast_locals.len() == 1 {
            frame.fast_locals[0] = Some(arg0);
            self.push_frame_checked(frame)?;
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
        self.push_frame_checked(frame)?;
        Ok(())
    }

    #[inline(always)]
    fn push_simple_positional_function_frame_one_arg_slot0_no_cells_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        owner_class: Option<&ObjRef>,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        debug_assert!(code.plain_positional_arg0_slot == Some(0));
        debug_assert!(code.fast_local_count == 1);
        let globals_version = self
            .frames
            .last()
            .map(|caller| caller.function_globals_version)
            .unwrap_or_else(|| module_globals_version(module));
        let mut frame = if owner_class.is_none() {
            self.acquire_simple_frame_slot0_no_cells_fast_ref(code, module, globals_version)
        } else {
            let mut frame = self.acquire_simple_frame_no_cells_ref(code, module, owner_class);
            frame.function_globals_version = globals_version;
            frame
        };
        if let Some(caller) = self.frames.last()
            && let Some(active_exception) = caller.active_exception.as_ref()
        {
            frame.active_exception = Some(Self::clone_active_exception_for_call(active_exception));
        }
        frame.fast_locals[0] = Some(arg0);
        self.push_frame_checked(frame)?;
        Ok(())
    }

    #[inline(always)]
    fn push_simple_positional_function_frame_one_arg_no_cells_cached_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        owner_class: Option<&ObjRef>,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        if code.fast_local_count == 1 && code.plain_positional_arg0_slot == Some(0) {
            self.push_simple_positional_function_frame_one_arg_slot0_no_cells_ref(
                code,
                module,
                owner_class,
                arg0,
            )
        } else {
            self.push_simple_positional_function_frame_one_arg_no_cells_ref(
                code,
                module,
                owner_class,
                arg0,
            )
        }
    }

    fn push_simple_positional_function_frame_one_arg_no_cells(
        &mut self,
        code: Rc<CodeObject>,
        module: ObjRef,
        owner_class: Option<ObjRef>,
        arg0: Value,
    ) -> Result<(), RuntimeError> {
        self.push_simple_positional_function_frame_one_arg_no_cells_cached_ref(
            &code,
            &module,
            owner_class.as_ref(),
            arg0,
        )
    }

    pub(super) fn push_function_call_two_args_from_obj(
        &mut self,
        func: &ObjRef,
        arg0: Value,
        arg1: Value,
    ) -> Result<(), RuntimeError> {
        let (simple_positional_path, no_cells_hot) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            let code = &func_data.code;
            let simple_positional_path = func_data.plain_positional_call_arity == Some(2);
            let no_cells_hot = simple_positional_path
                && code.plain_positional_arg0_cell.is_none()
                && code.plain_positional_arg1_cell.is_none()
                && code.cellvars.is_empty()
                && func_data.closure.is_empty()
                && !Self::code_returns_generator_like(&code)
                && !code.is_comprehension;
            (simple_positional_path, no_cells_hot)
        };
        if simple_positional_path {
            if no_cells_hot {
                return self.push_simple_positional_function_frame_two_args_no_cells_from_func(
                    func, arg0, arg1,
                );
            }
            let (code, module, closure, owner_class) = {
                let func_kind = func.kind();
                let func_data = match &*func_kind {
                    Object::Function(data) => data,
                    _ => return Err(RuntimeError::type_error("attempted to call non-function")),
                };
                (
                    func_data.code.clone(),
                    func_data.module.clone(),
                    func_data.closure.clone(),
                    func_data.owner_class.clone(),
                )
            };
            return self.push_simple_positional_function_frame_two_args(
                code,
                module,
                owner_class,
                closure,
                arg0,
                arg1,
            );
        }
        self.push_function_call_from_obj(func, vec![arg0, arg1], HashMap::new())
    }

    pub(super) fn push_function_call_three_args_from_obj(
        &mut self,
        func: &ObjRef,
        arg0: Value,
        arg1: Value,
        arg2: Value,
    ) -> Result<(), RuntimeError> {
        let (simple_positional_path, no_cells_hot) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            let code = &func_data.code;
            let simple_positional_path = func_data.plain_positional_call_arity == Some(3);
            let no_cells_hot = simple_positional_path
                && code.plain_positional_arg0_cell.is_none()
                && code.plain_positional_arg1_cell.is_none()
                && code.plain_positional_arg2_cell.is_none()
                && code.cellvars.is_empty()
                && func_data.closure.is_empty()
                && !Self::code_returns_generator_like(&code)
                && !code.is_comprehension;
            (simple_positional_path, no_cells_hot)
        };
        if simple_positional_path {
            if no_cells_hot {
                return self.push_simple_positional_function_frame_three_args_no_cells_from_func(
                    func, arg0, arg1, arg2,
                );
            }
            let (code, module, closure, owner_class) = {
                let func_kind = func.kind();
                let func_data = match &*func_kind {
                    Object::Function(data) => data,
                    _ => return Err(RuntimeError::type_error("attempted to call non-function")),
                };
                (
                    func_data.code.clone(),
                    func_data.module.clone(),
                    func_data.closure.clone(),
                    func_data.owner_class.clone(),
                )
            };
            return self.push_simple_positional_function_frame_three_args(
                code,
                module,
                owner_class,
                closure,
                arg0,
                arg1,
                arg2,
            );
        }
        self.push_function_call_from_obj(func, vec![arg0, arg1, arg2], HashMap::new())
    }

    #[inline]
    fn push_simple_positional_function_frame_two_args_no_cells_from_func(
        &mut self,
        func: &ObjRef,
        arg0: Value,
        arg1: Value,
    ) -> Result<(), RuntimeError> {
        let func_kind = func.kind();
        let func_data = match &*func_kind {
            Object::Function(data) => data,
            _ => return Err(RuntimeError::type_error("attempted to call non-function")),
        };
        self.push_simple_positional_function_frame_two_args_no_cells_ref(
            &func_data.code,
            &func_data.module,
            func_data.owner_class.as_ref(),
            arg0,
            arg1,
        )
    }

    #[inline]
    fn push_simple_positional_function_frame_three_args_no_cells_from_func(
        &mut self,
        func: &ObjRef,
        arg0: Value,
        arg1: Value,
        arg2: Value,
    ) -> Result<(), RuntimeError> {
        let func_kind = func.kind();
        let func_data = match &*func_kind {
            Object::Function(data) => data,
            _ => return Err(RuntimeError::type_error("attempted to call non-function")),
        };
        self.push_simple_positional_function_frame_three_args_no_cells_ref(
            &func_data.code,
            &func_data.module,
            func_data.owner_class.as_ref(),
            arg0,
            arg1,
            arg2,
        )
    }

    #[inline]
    fn code_returns_generator_like(code: &CodeObject) -> bool {
        code.is_generator || code.is_coroutine || code.is_async_generator
    }

    #[inline]
    fn push_simple_positional_function_frame_two_args_no_cells_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        owner_class: Option<&ObjRef>,
        arg0: Value,
        arg1: Value,
    ) -> Result<(), RuntimeError> {
        debug_assert!(!Self::code_returns_generator_like(code));
        debug_assert!(!code.is_comprehension);
        let mut frame = self.acquire_simple_frame_no_cells_ref(code, module, owner_class);
        if let Some(caller) = self.frames.last()
            && let Some(active_exception) = caller.active_exception.as_ref()
        {
            frame.active_exception = Some(Self::clone_active_exception_for_call(active_exception));
        }
        if frame.fast_locals.len() == 2
            && code.plain_positional_arg0_slot == Some(0)
            && code.plain_positional_arg1_slot == Some(1)
        {
            frame.fast_locals[0] = Some(arg0);
            frame.fast_locals[1] = Some(arg1);
            self.push_frame_checked(frame)?;
            return Ok(());
        }
        self.store_fast_positional_arg(code, &mut frame, 0, arg0);
        self.store_fast_positional_arg(code, &mut frame, 1, arg1);
        self.push_frame_checked(frame)?;
        Ok(())
    }

    #[inline]
    fn push_simple_positional_function_frame_three_args_no_cells_ref(
        &mut self,
        code: &Rc<CodeObject>,
        module: &ObjRef,
        owner_class: Option<&ObjRef>,
        arg0: Value,
        arg1: Value,
        arg2: Value,
    ) -> Result<(), RuntimeError> {
        debug_assert!(!Self::code_returns_generator_like(code));
        debug_assert!(!code.is_comprehension);
        let mut frame = self.acquire_simple_frame_no_cells_ref(code, module, owner_class);
        if let Some(caller) = self.frames.last()
            && let Some(active_exception) = caller.active_exception.as_ref()
        {
            frame.active_exception = Some(Self::clone_active_exception_for_call(active_exception));
        }
        if frame.fast_locals.len() == 3
            && code.plain_positional_arg0_slot == Some(0)
            && code.plain_positional_arg1_slot == Some(1)
            && code.plain_positional_arg2_slot == Some(2)
        {
            frame.fast_locals[0] = Some(arg0);
            frame.fast_locals[1] = Some(arg1);
            frame.fast_locals[2] = Some(arg2);
            self.push_frame_checked(frame)?;
            return Ok(());
        }
        self.store_fast_positional_arg(code, &mut frame, 0, arg0);
        self.store_fast_positional_arg(code, &mut frame, 1, arg1);
        self.store_fast_positional_arg(code, &mut frame, 2, arg2);
        self.push_frame_checked(frame)?;
        Ok(())
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

        if Self::code_returns_generator_like(&code) {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.clone(),
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
        self.push_frame_checked(frame)?;
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
            0 => (
                code.plain_positional_arg0_slot,
                code.plain_positional_arg0_cell,
            ),
            1 => (
                code.plain_positional_arg1_slot,
                code.plain_positional_arg1_cell,
            ),
            2 => (
                code.plain_positional_arg2_slot,
                code.plain_positional_arg2_cell,
            ),
            _ => (
                code.positional_param_slot_indexes
                    .get(arg_index)
                    .and_then(|idx| *idx),
                code.positional_param_cell_indexes
                    .get(arg_index)
                    .and_then(|idx| *idx),
            ),
        };
        if let Some(cell_idx) = cell_idx
            && let Some(cell) = frame.cells.get(cell_idx)
            && let Object::Cell(cell_data) = &mut *cell.kind_mut()
        {
            cell_data.value = Some(value);
            return;
        }
        if let Some(slot_idx) = slot_idx
            && let Some(slot) = frame.fast_locals.get_mut(slot_idx)
        {
            Self::write_fast_local_slot(slot, value);
            return;
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
            .and_then(|frame| frame.active_exception.as_ref())
            .map(Self::clone_active_exception_for_call);
        let module_id = module.id();
        let mut frame = self.acquire_frame(code.clone(), module, false, false, cells, owner_class);
        frame.active_exception = caller_active_exception;
        if code.is_comprehension
            && let Some(caller) = self.frames.last()
            && caller.return_class
            && caller.module.id() == module_id
        {
            frame.globals_fallback = Some(caller.function_globals.clone());
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

        if Self::code_returns_generator_like(&code) {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.clone(),
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
        self.push_frame_checked(frame)?;
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

        if Self::code_returns_generator_like(&code) {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.clone(),
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
        self.push_frame_checked(frame)?;
        Ok(())
    }

    pub(super) fn push_function_call_from_obj(
        &mut self,
        func: &ObjRef,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        self.push_function_call_from_obj_with_kwarg_order(func, args, kwargs, None)
    }

    pub(super) fn push_function_call_from_obj_with_kwarg_order(
        &mut self,
        func: &ObjRef,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        kwargs_order: Option<Vec<String>>,
    ) -> Result<(), RuntimeError> {
        let (code, module, closure, owner_class, simple_positional_path) = {
            let func_kind = func.kind();
            let func_data = match &*func_kind {
                Object::Function(data) => data,
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
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
                _ => return Err(RuntimeError::type_error("attempted to call non-function")),
            };
            match bind_arguments(func_data, &self.heap, args, kwargs, kwargs_order) {
                Ok(bindings) => bindings,
                Err(err) => {
                    if self.host.env_var_os("PYRS_TRACE_BIND_ARGS_STACK").is_some()
                        && err.message.contains("argument count mismatch")
                    {
                        let stack = self
                            .frames
                            .iter()
                            .rev()
                            .take(12)
                            .map(|frame| {
                                format!(
                                    "{}@{}:{}",
                                    frame.code.name,
                                    frame.code.filename,
                                    frame
                                        .code
                                        .locations
                                        .get(frame.last_ip)
                                        .map(|loc| loc.line)
                                        .unwrap_or(0)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" <- ");
                        eprintln!(
                            "[bind-args-stack] failing_fn={} file={} stack={}",
                            func_data.code.name, func_data.code.filename, stack
                        );
                        if self.host.env_var_os("PYRS_TRACE_BIND_ARGS_BT").is_some() {
                            eprintln!(
                                "[bind-args-bt] failing_fn={} bt={}",
                                func_data.code.name,
                                std::backtrace::Backtrace::force_capture()
                            );
                        }
                    }
                    return Err(err);
                }
            }
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

        if Self::code_returns_generator_like(&code) {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.clone(),
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
        self.push_frame_checked(frame)?;
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
        if Self::code_returns_generator_like(&code) {
            let generator = match self.heap.alloc_generator(GeneratorObject::new(
                code.clone(),
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
        self.push_frame_checked(frame)?;
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
            Object::Iterator(_) => Ok(Value::Iterator(receiver.clone())),
            Object::DictView(view) => Ok(match view.kind {
                DictViewKind::Keys => Value::DictKeys(receiver.clone()),
                DictViewKind::Values => Value::DictValues(receiver.clone()),
                DictViewKind::Items => Value::DictItems(receiver.clone()),
            }),
            _ => Err(RuntimeError::new("unsupported bound method receiver")),
        }
    }

    pub(super) fn bound_method_reduce_receiver_value(
        &self,
        receiver: &ObjRef,
    ) -> Result<Value, RuntimeError> {
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
            NativeMethodKind::ListEq => Some("__eq__"),
            NativeMethodKind::ListNe => Some("__ne__"),
            NativeMethodKind::TupleCount => Some("count"),
            NativeMethodKind::TupleIndex => Some("index"),
            NativeMethodKind::TupleEq => Some("__eq__"),
            NativeMethodKind::TupleNe => Some("__ne__"),
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
            | Value::MemoryView(obj)
            | Value::Iterator(obj)
            | Value::DictKeys(obj)
            | Value::DictValues(obj)
            | Value::DictItems(obj) => Ok(obj.clone()),
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
            Value::Builtin(builtin) if builtin_type_name_info(*builtin).is_some() => {
                self.default_type_metaclass()
            }
            Value::ExceptionType(_) => self.default_type_metaclass(),
            _ => None,
        }
    }

    pub(super) fn default_type_metaclass(&self) -> Option<ObjRef> {
        if let Some(Value::Class(type_class)) = self.builtins.get("type") {
            return Some(type_class.clone());
        }
        let Value::Class(object_class) = self.builtins.get("object")? else {
            return None;
        };
        let Object::Class(class_data) = &*object_class.kind() else {
            return None;
        };
        class_data.metaclass.clone()
    }

    pub(super) fn alloc_native_bound_method(
        &self,
        kind: NativeMethodKind,
        receiver: ObjRef,
    ) -> Value {
        let native = self.heap.alloc_native_method(NativeMethodObject::new(kind));
        let bound = BoundMethod::new(native, receiver);
        self.heap.alloc_bound_method(bound)
    }

    pub(super) fn alloc_builtin_bound_method(
        &self,
        builtin: BuiltinFunction,
        receiver: ObjRef,
    ) -> Value {
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

    pub(super) fn alloc_native_unbound_method(
        &self,
        wrapper_name: &str,
        owner: Value,
        kind: NativeMethodKind,
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
        self.alloc_native_bound_method(kind, receiver)
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

    pub(super) fn load_dunder_class_attr(&mut self, value: &Value) -> Result<Value, RuntimeError> {
        if let Value::Module(module) = value
            && let Object::Module(module_data) = &*module.kind()
            && let Some(Value::Class(class)) = module_data.globals.get("__class__")
        {
            return Ok(Value::Class(class.clone()));
        }
        if let Some(class) = self.class_of_value(value) {
            return Ok(Value::Class(class));
        }
        if let Some(runtime_type) = self.iterator_runtime_type_value(value) {
            return Ok(runtime_type);
        }
        if let Some(runtime_type) = self.callable_runtime_type_value(value) {
            return Ok(runtime_type);
        }
        if let Value::Code(_) = value
            && let Some(class) = self.types_module_or_private_class("CodeType")
        {
            return Ok(Value::Class(class));
        }
        if let Value::None = value
            && let Some(class) = self.types_module_or_private_class("NoneType")
        {
            return Ok(Value::Class(class));
        }
        let type_value = BuiltinFunction::Type.call(&self.heap, vec![value.clone()])?;
        Ok(self.normalize_runtime_type_value(type_value))
    }

    fn load_runtime_value_doc_attr(&mut self, value: &Value) -> Result<Value, RuntimeError> {
        let Value::Class(class) = self.load_dunder_class_attr(value)? else {
            return Ok(Value::None);
        };
        match self.load_attr_class(&class, "__doc__")? {
            AttrAccessOutcome::Value(doc) => Ok(doc),
            AttrAccessOutcome::ExceptionHandled => Ok(Value::None),
        }
    }

    pub(super) fn property_descriptor_parts(
        &self,
        descriptor: &ObjRef,
    ) -> Option<(Value, Value, Value, Value, Option<Value>)> {
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
        let explicit_name = instance_data.attrs.get("__name__").cloned();
        Some((fget, fset, fdel, doc, explicit_name))
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
        explicit_name: Option<Value>,
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
        if let Some(name) = explicit_name {
            instance.attrs.insert("__name__".to_string(), name);
        }
        self.heap.alloc_instance(instance)
    }

    pub(super) fn clone_property_descriptor_with(
        &self,
        descriptor: &ObjRef,
        fget: Option<Value>,
        fset: Option<Value>,
        fdel: Option<Value>,
        doc: Option<Value>,
        explicit_name: Option<Option<Value>>,
    ) -> Result<Value, RuntimeError> {
        let Some((current_get, current_set, current_del, current_doc, current_name)) =
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
            explicit_name.unwrap_or(current_name),
        ))
    }

    pub(super) fn optional_getattr_value(
        &mut self,
        target: Value,
        attr_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let outcome = self.call_internal_preserving_caller(
            Value::Builtin(BuiltinFunction::GetAttr),
            vec![target, Value::Str(attr_name.to_string())],
            HashMap::new(),
        );
        match outcome {
            Ok(InternalCallOutcome::Value(value)) => Ok(Some(value)),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                if self.active_exception_is("AttributeError") {
                    self.clear_active_exception();
                    Ok(None)
                } else {
                    Err(self
                        .runtime_error_from_active_exception("property attribute lookup failed"))
                }
            }
            Err(err) => {
                if runtime_error_matches_exception(&err, "AttributeError") {
                    Ok(None)
                } else {
                    Err(err)
                }
            }
        }
    }

    fn attr_access_outcome_to_option(
        &mut self,
        outcome: AttrAccessOutcome,
        error_context: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        match outcome {
            AttrAccessOutcome::Value(value) => Ok(Some(value)),
            AttrAccessOutcome::ExceptionHandled => {
                if self.active_exception_is("AttributeError") {
                    self.clear_active_exception();
                    Ok(None)
                } else {
                    Err(self.runtime_error_from_active_exception(error_context))
                }
            }
        }
    }

    pub(super) fn optional_internal_getattr_value(
        &mut self,
        target: Value,
        attr_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        match target {
            Value::Builtin(builtin) => match self.load_attr_builtin(builtin, attr_name) {
                Ok(value) => Ok(Some(value)),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            Value::Function(func) => match self.load_attr_function(&func, attr_name) {
                Ok(value) => Ok(Some(value)),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            Value::BoundMethod(method) => match self.load_attr_bound_method(&method, attr_name) {
                Ok(value) => Ok(Some(value)),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            Value::Class(class) => match self.load_attr_class(&class, attr_name) {
                Ok(outcome) => {
                    self.attr_access_outcome_to_option(outcome, "class attribute lookup failed")
                }
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            Value::Instance(instance) => match self.load_attr_instance(&instance, attr_name) {
                Ok(outcome) => {
                    self.attr_access_outcome_to_option(outcome, "instance attribute lookup failed")
                }
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            Value::Module(module) => match self.load_attr_module(&module, attr_name) {
                Ok(value) => Ok(Some(value)),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            Value::ExceptionType(name) => match self.load_attr_exception_type(&name, attr_name) {
                Ok(value) => Ok(Some(value)),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            Value::Code(code) => match self.load_attr_code(&code, attr_name) {
                Ok(value) => Ok(Some(value)),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => Ok(None),
                Err(err) => Err(err),
            },
            other => self.optional_getattr_value(other, attr_name),
        }
    }

    pub(super) fn object_is_abstract(&mut self, value: &Value) -> Result<bool, RuntimeError> {
        let Some(flag) =
            self.optional_internal_getattr_value(value.clone(), "__isabstractmethod__")?
        else {
            return Ok(false);
        };
        self.truthy_from_value(&flag)
    }

    fn match_class_attr_names_from_tuple(
        &self,
        value: &Value,
        context: &str,
    ) -> Result<Vec<String>, RuntimeError> {
        let values = match value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::type_error(format!(
                        "{context} must be a tuple of strings",
                    )));
                }
            },
            _ => {
                return Err(RuntimeError::type_error(format!(
                    "{context} must be a tuple of strings",
                )));
            }
        };
        let mut names = Vec::with_capacity(values.len());
        for entry in values {
            match entry {
                Value::Str(name) => names.push(name),
                _ => {
                    return Err(RuntimeError::type_error(format!(
                        "{context} entries must be strings",
                    )));
                }
            }
        }
        Ok(names)
    }

    fn class_pattern_supports_match_self(class_value: &Value) -> bool {
        match class_value {
            Value::Builtin(builtin) => matches!(
                builtin,
                BuiltinFunction::Bool
                    | BuiltinFunction::Int
                    | BuiltinFunction::Float
                    | BuiltinFunction::Complex
                    | BuiltinFunction::Str
                    | BuiltinFunction::Bytes
                    | BuiltinFunction::ByteArray
                    | BuiltinFunction::List
                    | BuiltinFunction::Tuple
                    | BuiltinFunction::Dict
                    | BuiltinFunction::Set
                    | BuiltinFunction::FrozenSet
            ),
            _ => false,
        }
    }

    fn value_supports_mapping_pattern(value: &Value) -> bool {
        matches!(value, Value::Dict(_))
    }

    fn value_supports_sequence_pattern(value: &Value) -> bool {
        matches!(value, Value::List(_) | Value::Tuple(_))
    }

    pub(super) fn typing_param_caller_module_attr(&self) -> Value {
        let module_name = self
            .frames
            .last()
            .and_then(|frame| match &*frame.module.kind() {
                Object::Module(module_data) => {
                    module_data.globals.get("__name__").and_then(|value| {
                        if let Value::Str(name) = value {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                }
                _ => None,
            });
        module_name.map(Value::Str).unwrap_or(Value::None)
    }

    fn intrinsic_make_typing_param(
        &mut self,
        builtin: BuiltinFunction,
        name: Value,
    ) -> Result<Value, RuntimeError> {
        let marker = self.call_builtin(builtin, vec![name], HashMap::new())?;
        let module_attr = self.typing_param_caller_module_attr();
        if let Value::Instance(instance) = &marker
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data
                .attrs
                .insert("__module__".to_string(), module_attr);
        }
        Ok(marker)
    }

    fn intrinsic_set_attr_for_value(
        &mut self,
        target: &Value,
        attr_name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match target {
            Value::Instance(instance) => {
                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                    instance_data.attrs.insert(attr_name.to_string(), value);
                }
                Ok(())
            }
            Value::Function(function) => {
                self.store_attr_function(function, attr_name.to_string(), value)
            }
            Value::Class(class) => {
                self.attach_owner_class_to_value(&value, class);
                if let Object::Class(class_data) = &mut *class.kind_mut() {
                    class_data.attrs.insert(attr_name.to_string(), value);
                }
                self.normalize_class_annotations_after_attr_set(class, attr_name);
                self.touch_class_attr_version(class);
                Ok(())
            }
            _ => Err(RuntimeError::type_error(format!(
                "cannot set intrinsic attribute '{}' on {}",
                attr_name,
                self.value_type_name_for_error(target),
            ))),
        }
    }

    fn intrinsic_stopiteration_error(&mut self, exc: Value) -> Result<Value, RuntimeError> {
        let Value::Instance(instance) = &exc else {
            return Ok(exc);
        };
        let Some(exception_name) = self.exception_class_name_for_instance(instance) else {
            return Ok(exc);
        };

        let frame_flags = self.frames.last().map(|frame| {
            (
                frame.code.is_generator,
                frame.code.is_coroutine,
                frame.code.is_async_generator,
            )
        });
        let (is_generator, is_coroutine, is_async_generator) =
            frame_flags.unwrap_or((false, false, false));

        let message = match exception_name.as_str() {
            "StopIteration" => {
                if is_async_generator {
                    Some("async generator raised StopIteration")
                } else if is_coroutine {
                    Some("coroutine raised StopIteration")
                } else if is_generator {
                    Some("generator raised StopIteration")
                } else {
                    None
                }
            }
            "StopAsyncIteration" if is_async_generator => {
                Some("async generator raised StopAsyncIteration")
            }
            _ => None,
        };

        let Some(message) = message else {
            return Ok(exc);
        };

        let wrapped = self.instantiate_exception_type(
            "RuntimeError",
            &[Value::Str(message.to_string())],
            &HashMap::new(),
        )?;
        self.intrinsic_set_attr_for_value(&wrapped, "__cause__", exc.clone())?;
        self.intrinsic_set_attr_for_value(&wrapped, "__context__", exc)?;
        self.intrinsic_set_attr_for_value(&wrapped, "__suppress_context__", Value::Bool(true))?;
        Ok(wrapped)
    }

    fn intrinsic_make_typevar_with_bound(
        &mut self,
        name: Value,
        evaluate_bound: Value,
    ) -> Result<Value, RuntimeError> {
        let marker = self.intrinsic_make_typing_param(BuiltinFunction::TypingTypeVar, name)?;
        self.intrinsic_set_attr_for_value(&marker, "__bound__", evaluate_bound)?;
        Ok(marker)
    }

    fn intrinsic_make_typevar_with_constraints(
        &mut self,
        name: Value,
        evaluate_constraints: Value,
    ) -> Result<Value, RuntimeError> {
        let marker = self.intrinsic_make_typing_param(BuiltinFunction::TypingTypeVar, name)?;
        self.intrinsic_set_attr_for_value(&marker, "__constraints__", evaluate_constraints)?;
        Ok(marker)
    }

    fn intrinsic_set_typeparam_default(
        &mut self,
        type_param: Value,
        default: Value,
    ) -> Result<Value, RuntimeError> {
        self.intrinsic_set_attr_for_value(&type_param, "__default__", default)?;
        Ok(type_param)
    }

    fn intrinsic_subscript_generic(&mut self, params: Value) -> Result<Value, RuntimeError> {
        let generic = self
            .modules
            .get("typing")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("Generic").cloned(),
                _ => None,
            })
            .or_else(|| {
                self.modules
                    .get("_typing")
                    .and_then(|module| match &*module.kind() {
                        Object::Module(module_data) => module_data.globals.get("Generic").cloned(),
                        _ => None,
                    })
            })
            .ok_or_else(|| RuntimeError::new("Cannot find Generic type"))?;
        match self.getitem_value(generic.clone(), params.clone()) {
            Ok(value) => Ok(value),
            Err(_) => Ok(self.alloc_generic_alias_instance(generic, params)),
        }
    }

    fn intrinsic_make_type_alias(&mut self, value: Value) -> Result<Value, RuntimeError> {
        let entries = match value {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(entries) => entries.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "INTRINSIC_TYPEALIAS expects a tuple",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::type_error(
                    "INTRINSIC_TYPEALIAS expects a tuple",
                ));
            }
        };
        if entries.len() != 3 {
            return Err(RuntimeError::type_error(
                "INTRINSIC_TYPEALIAS expects (name, type_params, value)",
            ));
        }

        let name = match &entries[0] {
            Value::Str(name) => name.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "INTRINSIC_TYPEALIAS name must be str",
                ));
            }
        };
        let type_params = entries[1].clone();
        let alias_value = entries[2].clone();
        let alias = self.intrinsic_make_typing_param(
            BuiltinFunction::TypingTypeAliasType,
            Value::Str(name.clone()),
        )?;
        self.intrinsic_set_attr_for_value(&alias, "__name__", Value::Str(name.clone()))?;
        self.intrinsic_set_attr_for_value(&alias, "__qualname__", Value::Str(name))?;
        self.intrinsic_set_attr_for_value(&alias, "__type_params__", type_params)?;
        self.intrinsic_set_attr_for_value(&alias, "__value__", alias_value)?;
        Ok(alias)
    }

    fn template_literal_class(&mut self, cache_key: &str, class_name: &str) -> ObjRef {
        if let Some(existing) = self.synthetic_builtin_classes.get(cache_key).cloned() {
            return existing;
        }

        let object_base = self.builtins.get("object").and_then(|value| match value {
            Value::Class(class) => Some(class.clone()),
            _ => None,
        });
        let mut bases = Vec::new();
        if let Some(base) = object_base {
            bases.push(base);
        }
        let class = match self
            .heap
            .alloc_class(ClassObject::new(class_name.to_string(), bases.clone()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
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
        let default_meta = self.default_type_metaclass();
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data.bases = bases.clone();
            class_data.mro = mro.clone();
            if class_data.metaclass.is_none() {
                class_data.metaclass = default_meta;
            }
            class_data
                .attrs
                .insert("__name__".to_string(), Value::Str(class_name.to_string()));
            class_data.attrs.insert(
                "__qualname__".to_string(),
                Value::Str(class_name.to_string()),
            );
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str("string.templatelib".to_string()),
            );
            class_data
                .attrs
                .insert("__flags__".to_string(), Value::Int(0));
            class_data.attrs.insert(
                "__bases__".to_string(),
                self.heap
                    .alloc_tuple(bases.iter().cloned().map(Value::Class).collect()),
            );
            class_data.attrs.insert(
                "__mro__".to_string(),
                self.heap
                    .alloc_tuple(mro.iter().cloned().map(Value::Class).collect()),
            );
        }
        self.synthetic_builtin_classes
            .insert(cache_key.to_string(), class.clone());
        class
    }

    fn build_template_interpolation_value(
        &mut self,
        value: Value,
        expression: Value,
        conversion: u32,
        format_spec: Value,
    ) -> Result<Value, RuntimeError> {
        let expression_value = match expression {
            Value::Str(text) => text,
            _ => {
                return Err(RuntimeError::type_error(
                    "BUILD_INTERPOLATION expects str expression",
                ));
            }
        };
        let format_spec_value = match format_spec {
            Value::Str(text) => text,
            _ => {
                return Err(RuntimeError::type_error(
                    "BUILD_INTERPOLATION expects str format spec",
                ));
            }
        };
        let conversion_value = match conversion {
            0 => Value::None,
            1 => Value::Str("s".to_string()),
            2 => Value::Str("r".to_string()),
            3 => Value::Str("a".to_string()),
            other => {
                return Err(RuntimeError::type_error(format!(
                    "invalid interpolation conversion code {other}",
                )));
            }
        };

        let interpolation_class = self.template_literal_class(
            "__pyrs_template_literal_interpolation_class__",
            "Interpolation",
        );
        let interpolation = match self
            .heap
            .alloc_instance(InstanceObject::new(interpolation_class))
        {
            Value::Instance(instance) => instance,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *interpolation.kind_mut() {
            instance_data.attrs.insert("value".to_string(), value);
            instance_data
                .attrs
                .insert("expression".to_string(), Value::Str(expression_value));
            instance_data
                .attrs
                .insert("conversion".to_string(), conversion_value);
            instance_data
                .attrs
                .insert("format_spec".to_string(), Value::Str(format_spec_value));
        }
        Ok(Value::Instance(interpolation))
    }

    fn build_template_value(
        &mut self,
        strings: Value,
        interpolations: Value,
    ) -> Result<Value, RuntimeError> {
        let string_items = match strings {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(items) => items.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "BUILD_TEMPLATE expects tuple[str] strings",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::type_error(
                    "BUILD_TEMPLATE expects tuple[str] strings",
                ));
            }
        };
        for item in &string_items {
            if !matches!(item, Value::Str(_)) {
                return Err(RuntimeError::type_error(
                    "BUILD_TEMPLATE strings must be str entries",
                ));
            }
        }
        let interpolation_items = match interpolations {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(items) => items.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "BUILD_TEMPLATE expects tuple interpolations",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::type_error(
                    "BUILD_TEMPLATE expects tuple interpolations",
                ));
            }
        };

        let strings_tuple = self.heap.alloc_tuple(string_items.clone());
        let interpolations_tuple = self.heap.alloc_tuple(interpolation_items.clone());

        let mut parts = Vec::with_capacity(string_items.len() + interpolation_items.len());
        let max_len = string_items.len().max(interpolation_items.len());
        for idx in 0..max_len {
            if let Some(item) = string_items.get(idx) {
                parts.push(item.clone());
            }
            if let Some(item) = interpolation_items.get(idx) {
                parts.push(item.clone());
            }
        }

        let template_class =
            self.template_literal_class("__pyrs_template_literal_template_class__", "Template");
        let template = match self
            .heap
            .alloc_instance(InstanceObject::new(template_class))
        {
            Value::Instance(instance) => instance,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *template.kind_mut() {
            instance_data
                .attrs
                .insert("strings".to_string(), strings_tuple);
            instance_data
                .attrs
                .insert("interpolations".to_string(), interpolations_tuple);
            instance_data
                .attrs
                .insert("__parts__".to_string(), self.heap.alloc_tuple(parts));
        }
        Ok(Value::Instance(template))
    }

    pub(super) fn property_descriptor_name(
        &mut self,
        fget: &Value,
        explicit_name: Option<&Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        if let Some(name) = explicit_name {
            return Ok(Some(name.clone()));
        }
        if matches!(fget, Value::None) {
            return Ok(None);
        }
        self.optional_getattr_value(fget.clone(), "__name__")
    }

    pub(super) fn property_descriptor_is_abstract(
        &mut self,
        fget: &Value,
        fset: &Value,
        fdel: &Value,
    ) -> Result<bool, RuntimeError> {
        for value in [fget, fset, fdel] {
            if self.object_is_abstract(value)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn load_attr_property_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some((fget, fset, fdel, doc, explicit_name)) = self.property_descriptor_parts(instance)
        else {
            return Ok(None);
        };
        match attr_name {
            "fget" => Ok(Some(fget)),
            "fset" => Ok(Some(fset)),
            "fdel" => Ok(Some(fdel)),
            "__doc__" => Ok(Some(doc)),
            "__name__" => self.property_descriptor_name(&fget, explicit_name.as_ref()),
            "__isabstractmethod__" => {
                let abstract_flag = self.property_descriptor_is_abstract(&fget, &fset, &fdel)?;
                Ok(Some(Value::Bool(abstract_flag)))
            }
            "__get__" => Ok(Some(self.alloc_native_bound_method(
                NativeMethodKind::PropertyGet,
                instance.clone(),
            ))),
            "__set__" => Ok(Some(self.alloc_native_bound_method(
                NativeMethodKind::PropertySet,
                instance.clone(),
            ))),
            "__delete__" => Ok(Some(self.alloc_native_bound_method(
                NativeMethodKind::PropertyDelete,
                instance.clone(),
            ))),
            "__set_name__" => Ok(Some(self.alloc_native_bound_method(
                NativeMethodKind::PropertySetName,
                instance.clone(),
            ))),
            "getter" => Ok(Some(self.alloc_native_bound_method(
                NativeMethodKind::PropertyGetter,
                instance.clone(),
            ))),
            "setter" => Ok(Some(self.alloc_native_bound_method(
                NativeMethodKind::PropertySetter,
                instance.clone(),
            ))),
            "deleter" => Ok(Some(self.alloc_native_bound_method(
                NativeMethodKind::PropertyDeleter,
                instance.clone(),
            ))),
            _ => Ok(None),
        }
    }

    pub(super) fn load_attr_cached_property_instance(
        &self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Option<Value> {
        let (func, attr_name_value, doc) = self.cached_property_descriptor_parts(instance)?;
        let module_name = if let Object::Instance(instance_data) = &*instance.kind() {
            instance_data
                .attrs
                .get("__module__")
                .cloned()
                .unwrap_or(Value::None)
        } else {
            Value::None
        };
        match attr_name {
            "func" => Some(func),
            "attrname" => Some(attr_name_value.map(Value::Str).unwrap_or(Value::None)),
            "__doc__" => Some(doc),
            "__module__" => Some(module_name),
            "__isabstractmethod__" => Some(Value::Bool(false)),
            "__get__" => {
                Some(self.alloc_native_bound_method(
                    NativeMethodKind::CachedPropertyGet,
                    instance.clone(),
                ))
            }
            "__set_name__" => Some(self.alloc_native_bound_method(
                NativeMethodKind::CachedPropertySetName,
                instance.clone(),
            )),
            _ => None,
        }
    }
}

fn render_traceback_caret_line(
    source_line: &str,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
) -> Option<String> {
    if source_line.is_empty() {
        return None;
    }
    if start_column == 0 {
        // CPython may still render an empty caret line for explicit 0..0 offsets.
        return if end_column == 0 {
            Some(String::new())
        } else {
            None
        };
    }
    if end_line != 0 && start_line != 0 && end_line < start_line {
        return None;
    }
    let char_count = source_line.chars().count();
    if char_count == 0 {
        return None;
    }
    let start = start_column
        .saturating_sub(1)
        .min(char_count.saturating_sub(1));
    let fallback = infer_traceback_caret_end(source_line, start);
    let mut end_exclusive = if end_column > 0 {
        end_column.saturating_sub(1)
    } else {
        match fallback {
            CaretInference::Suppress => return None,
            CaretInference::End(end) => end,
            CaretInference::Unknown => start.saturating_add(1),
        }
    };
    if end_line > start_line && start_line != 0 {
        end_exclusive = char_count;
    }
    if end_exclusive <= start {
        end_exclusive = match fallback {
            CaretInference::End(end) if end > start => end,
            _ => start.saturating_add(1),
        };
    }
    end_exclusive = end_exclusive.min(char_count);
    if end_column > 0 && end_exclusive > start {
        let span = source_line
            .chars()
            .skip(start)
            .take(end_exclusive.saturating_sub(start))
            .collect::<String>();
        if !span.contains('#')
            && span.ends_with(')')
            && let Some(open) = span.find('(')
            && open > 0
        {
            let call_head = open;
            let call_tail = end_exclusive
                .saturating_sub(start)
                .saturating_sub(call_head);
            return Some(format!(
                "{}{}{}",
                " ".repeat(start),
                "~".repeat(call_head),
                "^".repeat(call_tail.max(1))
            ));
        }
    }
    let width = end_exclusive.saturating_sub(start).max(1);
    Some(format!("{}{}", " ".repeat(start), "^".repeat(width)))
}

#[derive(Default)]
struct LineContinuationState {
    paren_depth: usize,
    bracket_depth: usize,
    brace_depth: usize,
    in_single_quote: bool,
    in_double_quote: bool,
    escaped: bool,
}

impl LineContinuationState {
    fn delimiter_depth(&self) -> usize {
        self.paren_depth + self.bracket_depth + self.brace_depth
    }
}

fn update_line_continuation_state(state: &mut LineContinuationState, line: &str) {
    for ch in line.chars() {
        if state.escaped {
            state.escaped = false;
            continue;
        }
        if state.in_single_quote {
            match ch {
                '\\' => state.escaped = true,
                '\'' => state.in_single_quote = false,
                _ => {}
            }
            continue;
        }
        if state.in_double_quote {
            match ch {
                '\\' => state.escaped = true,
                '"' => state.in_double_quote = false,
                _ => {}
            }
            continue;
        }
        if ch == '#' {
            break;
        }
        match ch {
            '\'' => state.in_single_quote = true,
            '"' => state.in_double_quote = true,
            '(' => state.paren_depth = state.paren_depth.saturating_add(1),
            ')' => state.paren_depth = state.paren_depth.saturating_sub(1),
            '[' => state.bracket_depth = state.bracket_depth.saturating_add(1),
            ']' => state.bracket_depth = state.bracket_depth.saturating_sub(1),
            '{' => state.brace_depth = state.brace_depth.saturating_add(1),
            '}' => state.brace_depth = state.brace_depth.saturating_sub(1),
            _ => {}
        }
    }
}

fn line_has_explicit_continuation(line: &str) -> bool {
    line.trim_end().ends_with('\\')
}

fn clip_short_circuit_end_column(
    source_line: &str,
    start_column: usize,
    end_column: usize,
) -> Option<usize> {
    if start_column == 0 || end_column <= start_column {
        return None;
    }
    let start = start_column.saturating_sub(1);
    let end = end_column.saturating_sub(1).min(source_line.len());
    if start > source_line.len()
        || !source_line.is_char_boundary(start)
        || !source_line.is_char_boundary(end)
    {
        return None;
    }
    let segment = &source_line[start..end];
    let has_operator = segment
        .chars()
        .any(|ch| matches!(ch, '+' | '-' | '*' | '/' | '%' | '@' | '&' | '|' | '^'));
    if !has_operator {
        return None;
    }
    for marker in [" and ", " or "] {
        if let Some(index) = segment.find(marker) {
            return Some(start + index + 1);
        }
    }
    None
}

enum CaretInference {
    Suppress,
    End(usize),
    Unknown,
}

fn infer_traceback_caret_end(source_line: &str, start: usize) -> CaretInference {
    let chars: Vec<char> = source_line.chars().collect();
    if start >= chars.len() {
        return CaretInference::Unknown;
    }
    let ch = chars[start];
    if ch.is_whitespace() {
        return CaretInference::Unknown;
    }
    if is_identifier_start(ch) {
        let mut idx = start + 1;
        while idx < chars.len() && is_identifier_continue(chars[idx]) {
            idx += 1;
        }
        let first_ident: String = chars[start..idx].iter().collect();
        if is_statement_keyword(&first_ident) {
            return CaretInference::Suppress;
        }
        // Keep dotted names together: e.g. np.float
        loop {
            if idx >= chars.len() || chars[idx] != '.' {
                break;
            }
            let next = idx + 1;
            if next >= chars.len() || !is_identifier_start(chars[next]) {
                break;
            }
            idx = next + 1;
            while idx < chars.len() && is_identifier_continue(chars[idx]) {
                idx += 1;
            }
        }
        return CaretInference::End(idx);
    }
    let mut idx = start + 1;
    while idx < chars.len() {
        let current = chars[idx];
        if current.is_whitespace() || current == ',' || current == ';' || current == '#' {
            break;
        }
        idx += 1;
    }
    CaretInference::End(idx)
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_statement_keyword(word: &str) -> bool {
    matches!(
        word,
        "raise"
            | "return"
            | "import"
            | "from"
            | "def"
            | "class"
            | "for"
            | "while"
            | "if"
            | "elif"
            | "else"
            | "try"
            | "except"
            | "finally"
            | "with"
            | "yield"
            | "assert"
            | "del"
            | "pass"
            | "break"
            | "continue"
            | "global"
            | "nonlocal"
            | "async"
            | "await"
    )
}

fn should_suppress_explicit_raise_caret(
    source_line: &str,
    start_column: usize,
    exception_name: Option<&str>,
) -> bool {
    let Some(exception_name) = exception_name else {
        return false;
    };
    if start_column == 0 {
        return false;
    }
    let trimmed = source_line.trim_start();
    let Some(after_raise) = trimmed.strip_prefix("raise") else {
        return false;
    };
    if !after_raise.chars().next().is_some_and(char::is_whitespace) {
        return false;
    }
    let after_raise_trimmed = after_raise.trim_start();
    if after_raise_trimmed.is_empty() {
        return false;
    }
    let raised_expr_head: String = after_raise_trimmed
        .chars()
        .take_while(|ch| is_identifier_continue(*ch) || *ch == '.')
        .collect();
    if raised_expr_head.is_empty() {
        return false;
    }
    let raised_type_name = raised_expr_head.rsplit('.').next().unwrap_or_default();
    if raised_type_name != exception_name {
        return false;
    }

    let leading_ws = source_line
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .count();
    let start = start_column.saturating_sub(1);
    let raise_start = leading_ws;
    let raise_end = raise_start + "raise".chars().count();
    if start >= raise_start && start < raise_end {
        return true;
    }
    let raise_gap = after_raise
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .count();
    let expr_start = leading_ws + "raise".chars().count() + raise_gap;
    let expr_end = expr_start + raised_expr_head.chars().count();
    start >= expr_start && start < expr_end
}
use std::cell::Cell;
