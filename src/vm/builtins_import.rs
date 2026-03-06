use super::{
    BUILTIN_MODULE_LOADER, HashMap, ImportReturnPolicy, ModuleObject, NAMESPACE_LOADER,
    OPCODE_METADATA, ObjRef, Object, OpcodeMetadata, PathBuf, RuntimeError, SOURCE_FILE_LOADER,
    SOURCELESS_FILE_LOADER, Value, Vm, bytes_like_from_value, cache_path_from_source_path,
    cache_path_from_source_path_with_optimization, class_attr_lookup, fs, is_truthy,
    opcode_flags_contains, source_path_from_cache_path, split_relative_import_name, value_to_int,
    value_to_path,
};

impl Vm {
    fn import_active_exception_summary(&self, value: &Value) -> String {
        match value {
            Value::Exception(exception) => {
                if let Some(message) = exception.message.as_ref() {
                    let prefixed = format!("{}:", exception.name);
                    if message.starts_with(&prefixed) {
                        message.clone()
                    } else {
                        format!("{}: {}", exception.name, message)
                    }
                } else {
                    exception.name.clone()
                }
            }
            Value::ExceptionType(name) => name.clone(),
            other => self.value_type_name_for_error(other),
        }
    }

    pub(super) fn builtin_import(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 5 {
            return Err(RuntimeError::new("__import__() takes at most 5 arguments"));
        }

        let kw_name = kwargs.remove("name");
        let kw_globals = kwargs.remove("globals");
        let kw_locals = kwargs.remove("locals");
        let kw_fromlist = kwargs.remove("fromlist");
        let kw_level = kwargs.remove("level");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__import__() got an unexpected keyword argument",
            ));
        }

        let name_value = if let Some(value) = kw_name {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "__import__() got multiple values for argument 'name'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new(
                "__import__() missing required argument 'name'",
            ));
        };
        let name = match name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("__import__() name must be string")),
        };

        if kw_globals.is_some() && !args.is_empty() {
            return Err(RuntimeError::new(
                "__import__() got multiple values for argument 'globals'",
            ));
        }
        if kw_locals.is_some() && args.len() > 1 {
            return Err(RuntimeError::new(
                "__import__() got multiple values for argument 'locals'",
            ));
        }
        let fromlist = if let Some(value) = kw_fromlist {
            if args.len() > 2 {
                return Err(RuntimeError::new(
                    "__import__() got multiple values for argument 'fromlist'",
                ));
            }
            value
        } else if args.len() > 2 {
            args[2].clone()
        } else {
            Value::None
        };
        let level = if let Some(value) = kw_level {
            if args.len() > 3 {
                return Err(RuntimeError::new(
                    "__import__() got multiple values for argument 'level'",
                ));
            }
            value_to_int(value)?
        } else if args.len() > 3 {
            value_to_int(args[3].clone())?
        } else {
            0
        };
        if level < 0 {
            return Err(RuntimeError::new("level must be >= 0"));
        }

        let resolved_name = self.resolve_import_name(&name, level as usize)?;
        let module = self.import_module_object_with_policy(
            &resolved_name,
            ImportReturnPolicy::DeferredWhenFramesQueued,
        )?;
        let result = if self.fromlist_requested(&fromlist) {
            module
        } else {
            self.module_for_plain_import(&resolved_name, module)
        };
        Ok(Value::Module(result))
    }

    pub(super) fn builtin_import_module(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "import_module() takes at most 2 arguments",
            ));
        }
        let kw_name = kwargs.remove("name");
        let kw_package = kwargs.remove("package");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "import_module() got an unexpected keyword argument",
            ));
        }

        let name_value = if let Some(value) = kw_name {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "import_module() got multiple values for argument 'name'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new(
                "import_module() missing required argument 'name'",
            ));
        };
        let name = match name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("import_module() name must be string")),
        };

        let package_value = if let Some(value) = kw_package {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "import_module() got multiple values for argument 'package'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };
        let package = match package_value {
            Value::None => None,
            Value::Str(package) => Some(package),
            _ => {
                return Err(RuntimeError::new(
                    "import_module() package must be string or None",
                ));
            }
        };

        let (level, requested) = split_relative_import_name(&name);
        let resolved_name = if level == 0 {
            name
        } else {
            let package = package.ok_or_else(|| {
                RuntimeError::new("import_module() relative import requires package argument")
            })?;
            self.resolve_import_name_from_package(&package, &requested, level)?
        };
        let module = self.import_module_object_with_policy(
            &resolved_name,
            ImportReturnPolicy::DeferredWhenFramesQueued,
        )?;
        Ok(Value::Module(module))
    }

    pub(super) fn run_pending_import_frames(
        &mut self,
        caller_depth: usize,
    ) -> Result<(), RuntimeError> {
        self.run_pending_import_frames_impl(caller_depth, false)
    }

    pub(super) fn run_pending_import_frames_force(
        &mut self,
        caller_depth: usize,
    ) -> Result<(), RuntimeError> {
        self.run_pending_import_frames_impl(caller_depth, true)
    }

    fn run_pending_import_frames_impl(
        &mut self,
        caller_depth: usize,
        force_nested_sync: bool,
    ) -> Result<(), RuntimeError> {
        if self.frames.len() <= caller_depth {
            return Ok(());
        }
        if let Some(active_stop_depth) = self.run_stop_depth {
            let cpython_context_active = super::vm_extensions::cpython_active_context_is_set();
            // We're already inside a stop-depth run loop (e.g. import/eval/call
            // trampoline). Re-entering `run()` here can recurse one level per
            // nested import and eventually overflow the Rust stack for deep
            // extension import trees (NumPy random/scipy bring-up hit this).
            //
            // If the active loop is already draining to an equal-or-shallower
            // depth, we still need synchronous import semantics: the caller must
            // not observe a partially initialized module.
            //
            // If an outer pending-import drain is already running, nested
            // Python-side import paths can rely on it and skip another `run()`
            // re-entry. C-extension contexts still force local drain semantics.
            if active_stop_depth <= caller_depth {
                if self.trace_flags.import_pending {
                    let reason = if cpython_context_active {
                        "cpython-context"
                    } else if force_nested_sync {
                        "forced-sync"
                    } else {
                        "sync-semantic"
                    };
                    eprintln!(
                        "[import-pending-force] caller_depth={} active_stop_depth={} frames={} drain_depth={} reason={} force_nested_sync={}",
                        caller_depth,
                        active_stop_depth,
                        self.frames.len(),
                        self.pending_import_drain_depth,
                        reason,
                        force_nested_sync
                    );
                }
                if self.pending_import_drain_depth > 0 && !cpython_context_active {
                    // Nested Python import paths must not re-enter `run()` while an
                    // outer import-drain loop is already active; doing so creates
                    // one Rust stack frame per nested import and can overflow on
                    // stdlib import chains (for example `import os` -> `abc`).
                    //
                    // The active outer drain already targets an equal-or-shallower
                    // stop depth, so returning here preserves synchronous import
                    // semantics without recursive VM re-entry.
                    return Ok(());
                }
            } else {
                // Rare case: nested caller asks to drain farther than current stop
                // depth. Tighten the active stop depth in-place and let the running
                // loop honor it without introducing another `run()` re-entry.
                self.run_stop_depth = Some(caller_depth);
                if self.trace_flags.import_pending {
                    eprintln!(
                        "[import-pending-tighten] caller_depth={} previous_stop_depth={} frames={}",
                        caller_depth,
                        active_stop_depth,
                        self.frames.len()
                    );
                }
                return Ok(());
            }
        }
        let caller_active_exception_before = if caller_depth == 0 {
            None
        } else {
            self.frames
                .get(caller_depth.saturating_sub(1))
                .and_then(|frame| frame.active_exception.clone())
        };
        let previous_stop = self.run_stop_depth;
        self.run_stop_depth = Some(caller_depth);
        self.pending_import_drain_depth += 1;
        let run_result = self.run();
        self.pending_import_drain_depth = self
            .pending_import_drain_depth
            .checked_sub(1)
            .expect("import drain depth underflow");
        self.run_stop_depth = previous_stop;
        if let Err(err) = run_result {
            if self.trace_flags.import_pending {
                let caller_exc = self
                    .frames
                    .get(caller_depth.saturating_sub(1))
                    .and_then(|frame| frame.active_exception.as_ref())
                    .map(|value| self.value_type_name_for_error(value))
                    .unwrap_or_else(|| "<none>".to_string());
                eprintln!(
                    "[import-pending-err] caller_depth={} frames={} err={} caller_exc={}",
                    caller_depth,
                    self.frames.len(),
                    err.message,
                    caller_exc
                );
            }
            if caller_depth > 0
                && let Some(active) = self
                    .frames
                    .get(caller_depth.saturating_sub(1))
                    .and_then(|frame| frame.active_exception.as_ref())
            {
                return Err(RuntimeError::new(
                    self.import_active_exception_summary(active),
                ));
            }
            return Err(err);
        }
        if caller_depth > 0 {
            let caller_frame = self.frames.get(caller_depth.saturating_sub(1));
            let caller_active_exception_after =
                caller_frame.and_then(|frame| frame.active_exception.clone());
            if caller_active_exception_after != caller_active_exception_before {
                if self.trace_flags.import_pending {
                    let before = caller_active_exception_before
                        .as_ref()
                        .map(|value| self.value_type_name_for_error(value))
                        .unwrap_or_else(|| "<none>".to_string());
                    let after = caller_active_exception_after
                        .as_ref()
                        .map(|value| self.value_type_name_for_error(value))
                        .unwrap_or_else(|| "<none>".to_string());
                    eprintln!(
                        "[import-pending-active-exc] caller_depth={} before={} after={}",
                        caller_depth, before, after
                    );
                }
                if let Some(active) = caller_active_exception_after.as_ref() {
                    return Err(RuntimeError::new(
                        self.import_active_exception_summary(active),
                    ));
                }
                return Err(RuntimeError::runtime_error("import raised exception"));
            }
        }
        Ok(())
    }

    pub(super) fn return_imported_module(
        &mut self,
        module: ObjRef,
        caller_depth: usize,
    ) -> Result<ObjRef, RuntimeError> {
        self.return_imported_module_with_policy(
            module,
            caller_depth,
            ImportReturnPolicy::Synchronous,
        )
    }

    pub(super) fn return_imported_module_with_policy(
        &mut self,
        module: ObjRef,
        caller_depth: usize,
        return_policy: ImportReturnPolicy,
    ) -> Result<ObjRef, RuntimeError> {
        // Ensure queued module frames are executed before returning an import
        // result. CPython import semantics require a fully initialized module
        // unless this is a genuine re-entrant cycle returning an already
        // registered in-progress module.
        if self.frames.len() > caller_depth {
            let cpython_context_active = super::vm_extensions::cpython_active_context_is_set();
            match return_policy {
                ImportReturnPolicy::Synchronous => self.run_pending_import_frames(caller_depth)?,
                ImportReturnPolicy::DeferredWhenFramesQueued if !cpython_context_active => {
                    return Ok(module);
                }
                ImportReturnPolicy::DeferredWhenFramesQueued => {
                    self.run_pending_import_frames(caller_depth)?
                }
            }
        }
        let module_name = match &*module.kind() {
            Object::Module(module_data) => module_data.name.clone(),
            _ => String::new(),
        };
        let mut canonical = self.canonical_imported_module_for_name(&module_name, module);
        if Self::module_is_initializing(&canonical) {
            // Nested importlib flows can defer drain to an outer import loop.
            // If we would otherwise return an initializing placeholder, force a
            // local drain so import statements bind the finalized sys.modules entry.
            self.run_pending_import_frames_force(caller_depth)?;
            canonical = self.canonical_imported_module_for_name(&module_name, canonical);
        }
        self.sync_re_module_flag_aliases(&canonical);
        Ok(canonical)
    }

    pub(super) fn builtin_pkgutil_get_data(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("get_data() expects package and resource"));
        }
        let package = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("package must be string")),
        };
        let resource = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("resource must be string")),
        };

        let caller_depth = self.frames.len();
        let module = match self.import_module_object(&package) {
            Ok(module) => module,
            Err(_) => return Ok(Value::None),
        };
        let module = match self.return_imported_module(module, caller_depth) {
            Ok(module) => module,
            Err(_) => return Ok(Value::None),
        };
        let Object::Module(module_data) = &*module.kind() else {
            return Ok(Value::None);
        };

        let mut base_dir = None;
        if let Some(path_value) = module_data.globals.get("__path__") {
            let first = match path_value {
                Value::List(path_list) => match &*path_list.kind() {
                    Object::List(entries) => entries.first().cloned(),
                    _ => None,
                },
                Value::Tuple(path_tuple) => match &*path_tuple.kind() {
                    Object::Tuple(entries) => entries.first().cloned(),
                    _ => None,
                },
                _ => None,
            };
            if let Some(entry) = first {
                base_dir = Some(PathBuf::from(value_to_path(&entry)?));
            }
        }

        if base_dir.is_none()
            && let Some(Value::Str(origin)) = module_data.globals.get("__file__")
        {
            let path = PathBuf::from(origin);
            if let Some(parent) = path.parent() {
                base_dir = Some(parent.to_path_buf());
            }
        }

        let Some(base_dir) = base_dir else {
            return Ok(Value::None);
        };
        let path = base_dir.join(resource);
        let Ok(bytes) = fs::read(path) else {
            return Ok(Value::None);
        };
        Ok(self.heap.alloc_bytes(bytes))
    }

    pub(super) fn builtin_pkgutil_iter_modules(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "iter_modules() expects optional path and prefix",
            ));
        }
        let mut prefix = if args.len() > 1 {
            args[1].clone()
        } else {
            Value::Str(String::new())
        };
        if let Some(value) = kwargs.remove("prefix") {
            if args.len() > 1 {
                return Err(RuntimeError::new(
                    "iter_modules() got multiple values for argument 'prefix'",
                ));
            }
            prefix = value;
        }
        if kwargs.remove("path").is_some() && !args.is_empty() {
            return Err(RuntimeError::new(
                "iter_modules() got multiple values for argument 'path'",
            ));
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "iter_modules() got an unexpected keyword argument",
            ));
        }
        if !matches!(prefix, Value::Str(_)) {
            return Err(RuntimeError::new("prefix must be string"));
        }
        Ok(self.heap.alloc_list(Vec::new()))
    }

    pub(super) fn builtin_pkgutil_walk_packages(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "walk_packages() expects optional path, prefix, onerror",
            ));
        }
        let mut prefix = if args.len() > 1 {
            args[1].clone()
        } else {
            Value::Str(String::new())
        };
        if let Some(value) = kwargs.remove("prefix") {
            if args.len() > 1 {
                return Err(RuntimeError::new(
                    "walk_packages() got multiple values for argument 'prefix'",
                ));
            }
            prefix = value;
        }
        if kwargs.remove("path").is_some() && !args.is_empty() {
            return Err(RuntimeError::new(
                "walk_packages() got multiple values for argument 'path'",
            ));
        }
        if kwargs.remove("onerror").is_some() && args.len() > 2 {
            return Err(RuntimeError::new(
                "walk_packages() got multiple values for argument 'onerror'",
            ));
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "walk_packages() got an unexpected keyword argument",
            ));
        }
        if !matches!(prefix, Value::Str(_)) {
            return Err(RuntimeError::new("prefix must be string"));
        }
        Ok(self.heap.alloc_list(Vec::new()))
    }

    pub(super) fn builtin_pkgutil_resolve_name(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "resolve_name() expects name and optional package",
            ));
        }
        let name = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("name must be a string")),
        };
        let package = if let Some(value) = kwargs.remove("package") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "resolve_name() got multiple values for argument 'package'",
                ));
            }
            match value {
                Value::None => None,
                Value::Str(package) => Some(package),
                _ => return Err(RuntimeError::new("package must be string or None")),
            }
        } else if !args.is_empty() {
            match args.remove(0) {
                Value::None => None,
                Value::Str(package) => Some(package),
                _ => return Err(RuntimeError::new("package must be string or None")),
            }
        } else {
            None
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "resolve_name() got an unexpected keyword argument",
            ));
        }

        let (level, requested) = split_relative_import_name(&name);
        let target = if level == 0 {
            name
        } else {
            let package = package
                .ok_or_else(|| RuntimeError::new("relative resolve_name() requires package"))?;
            self.resolve_import_name_from_package(&package, &requested, level)?
        };

        let mut parts = target.splitn(2, ':');
        let module_name = parts.next().unwrap_or_default();
        let qualname = parts.next().unwrap_or_default();
        let caller_depth = self.frames.len();
        let module = self.import_module_object(module_name)?;
        self.run_pending_import_frames(caller_depth)?;
        let module = self.canonical_imported_module_for_name(module_name, module);
        if qualname.is_empty() {
            return Ok(Value::Module(module));
        }
        let mut value = Value::Module(module);
        for part in qualname.split('.') {
            value =
                self.builtin_getattr(vec![value, Value::Str(part.to_string())], HashMap::new())?;
        }
        Ok(value)
    }

    pub(super) fn sync_re_module_flag_aliases(&mut self, module: &ObjRef) {
        let regex_flag_class = {
            let module_kind = module.kind();
            let Object::Module(module_data) = &*module_kind else {
                return;
            };
            if module_data.name != "re" {
                return;
            }
            module_data.globals.get("RegexFlag").cloned()
        };

        let Some(Value::Class(regex_flag_class)) = regex_flag_class else {
            return;
        };

        let alias_names = [
            "NOFLAG",
            "ASCII",
            "A",
            "IGNORECASE",
            "I",
            "LOCALE",
            "L",
            "UNICODE",
            "U",
            "MULTILINE",
            "M",
            "DOTALL",
            "S",
            "VERBOSE",
            "X",
            "DEBUG",
        ];

        let mut pending = Vec::new();
        for name in alias_names {
            let needs_alias = {
                let module_kind = module.kind();
                let Object::Module(module_data) = &*module_kind else {
                    return;
                };
                !module_data.globals.contains_key(name)
            };
            if !needs_alias {
                continue;
            }
            if let Some(value) = class_attr_lookup(&regex_flag_class, name) {
                pending.push((name.to_string(), value));
            }
        }

        if pending.is_empty() {
            return;
        }
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            for (name, value) in pending {
                module_data.globals.insert(name, value);
            }
        }
    }

    pub(super) fn builtin_find_spec(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new("find_spec() takes at most 2 arguments"));
        }
        let kw_name = kwargs.remove("name");
        let kw_package = kwargs.remove("package");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "find_spec() got an unexpected keyword argument",
            ));
        }

        let name_value = if let Some(value) = kw_name {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "find_spec() got multiple values for argument 'name'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new(
                "find_spec() missing required argument 'name'",
            ));
        };
        let name = match name_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("find_spec() name must be string")),
        };

        let package_value = if let Some(value) = kw_package {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "find_spec() got multiple values for argument 'package'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };
        let package = match package_value {
            Value::None => None,
            Value::Str(package) => Some(package),
            _ => {
                return Err(RuntimeError::new(
                    "find_spec() package must be string or None",
                ));
            }
        };

        let (level, requested) = split_relative_import_name(&name);
        let resolved_name = if level == 0 {
            name
        } else {
            let package = package.ok_or_else(|| {
                RuntimeError::new("find_spec() relative import requires package argument")
            })?;
            self.resolve_import_name_from_package(&package, &requested, level)?
        };

        if let Some(existing) = self.modules.get(&resolved_name).cloned()
            && let Object::Module(module_data) = &*existing.kind()
            && let Some(spec) = module_data.globals.get("__spec__").cloned()
        {
            return Ok(spec);
        }

        let Some(source_info) = self.find_module_source(&resolved_name) else {
            return Ok(Value::None);
        };
        let loader_name = if source_info.is_namespace {
            NAMESPACE_LOADER
        } else if source_info.is_bytecode {
            SOURCELESS_FILE_LOADER
        } else {
            SOURCE_FILE_LOADER
        };
        let (origin, cached) = self.module_origin_and_cached_paths(&source_info);
        Ok(self.build_module_spec_value(
            &resolved_name,
            origin.as_ref(),
            cached.as_ref(),
            Some(loader_name),
            source_info.is_package,
            source_info.package_dirs.as_slice(),
            source_info.is_namespace,
        ))
    }

    pub(super) fn builtin_importlib_path_hook(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("path_hook() expects one path argument"));
        }
        let path = value_to_path(&args.remove(0))?;
        let root = PathBuf::from(path);
        Ok(self.make_file_finder_importer(&root))
    }

    pub(super) fn builtin_importlib_file_finder_find_spec(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "find_spec() missing required argument 'fullname'",
            ));
        }
        let finder = args.remove(0);
        let kw_fullname = kwargs.remove("fullname");
        let kw_target = kwargs.remove("target");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "find_spec() got an unexpected keyword argument",
            ));
        }

        let fullname_value = if let Some(value) = kw_fullname {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "find_spec() got multiple values for argument 'fullname'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new(
                "find_spec() missing required argument 'fullname'",
            ));
        };
        let fullname = match fullname_value {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("find_spec() fullname must be string")),
        };

        if let Some(_target) = kw_target {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "find_spec() got multiple values for argument 'target'",
                ));
            }
        } else if !args.is_empty() {
            args.remove(0);
        }
        if !args.is_empty() {
            return Err(RuntimeError::new("find_spec() takes at most 2 arguments"));
        }

        let module_name = fullname
            .rsplit_once('.')
            .map(|(_, tail)| tail)
            .unwrap_or(fullname.as_str());
        let Some(source_info) = self.find_module_source_with_importer(&finder, module_name) else {
            return Ok(Value::None);
        };
        let loader_name = if source_info.is_namespace {
            NAMESPACE_LOADER
        } else if source_info.is_bytecode {
            SOURCELESS_FILE_LOADER
        } else {
            SOURCE_FILE_LOADER
        };
        let (origin, cached) = self.module_origin_and_cached_paths(&source_info);
        Ok(self.build_module_spec_value(
            &fullname,
            origin.as_ref(),
            cached.as_ref(),
            Some(loader_name),
            source_info.is_package,
            source_info.package_dirs.as_slice(),
            source_info.is_namespace,
        ))
    }

    pub(super) fn builtin_importlib_invalidate_caches(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "invalidate_caches() expects no arguments",
            ));
        }
        // CPython keeps `sys.path_importer_cache` entries and forwards invalidation
        // to active finders instead of clearing the cache dict wholesale.
        Ok(Value::None)
    }

    pub(super) fn builtin_importlib_file_finder_invalidate_caches(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::type_error(
                "invalidate_caches() got an unexpected keyword argument",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::type_error(format!(
                "invalidate_caches() takes 1 positional argument but {} were given",
                args.len()
            )));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_importlib_spec_from_file_location(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 2 {
            return Err(RuntimeError::new(
                "spec_from_file_location() expects name and location",
            ));
        }
        let name = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("name must be string")),
        };
        let location = match args.remove(0) {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("location must be string")),
        };
        let location_path = PathBuf::from(&location);
        let loader = kwargs.remove("loader").unwrap_or_else(|| {
            self.loader_spec_value_for_module(
                &name,
                Some(&location_path),
                Some(SOURCE_FILE_LOADER),
                &[],
                false,
            )
        });
        let search_locations = kwargs.remove("submodule_search_locations");
        kwargs.remove("target");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "spec_from_file_location() got an unexpected keyword argument",
            ));
        }
        let normalized_search_locations = if let Some(value) = search_locations {
            if matches!(value, Value::None) {
                Value::None
            } else {
                let locations = self.collect_iterable_values(value)?;
                self.heap.alloc_list(locations)
            }
        } else {
            Value::None
        };
        let is_package = !matches!(normalized_search_locations, Value::None);
        let spec = self.build_module_spec_value(
            &name,
            Some(&location_path),
            None,
            None,
            is_package,
            &[],
            false,
        );
        self.set_module_spec_field(&spec, "loader", loader);
        self.set_module_spec_field(
            &spec,
            "cached",
            Value::Str(cache_path_from_source_path(&location)),
        );
        self.set_module_spec_field(
            &spec,
            "submodule_search_locations",
            normalized_search_locations,
        );
        self.set_module_spec_field(&spec, "__spec__", Value::None);
        Ok(spec)
    }

    pub(super) fn builtin_importlib_module_from_spec(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "module_from_spec() expects exactly one spec argument",
            ));
        }
        let spec = args.remove(0);
        let spec_field = |spec: &Value, field: &str| -> Option<Value> {
            match spec {
                Value::Module(obj) => match &*obj.kind() {
                    Object::Module(module_data) => module_data.globals.get(field).cloned(),
                    _ => None,
                },
                Value::Dict(obj) => match &*obj.kind() {
                    Object::Dict(entries) => entries.find(&Value::Str(field.to_string())).cloned(),
                    _ => None,
                },
                Value::Instance(obj) => match &*obj.kind() {
                    Object::Instance(instance_data) => instance_data.attrs.get(field).cloned(),
                    _ => None,
                },
                _ => None,
            }
        };
        let name = match spec_field(&spec, "name") {
            Some(Value::Str(value)) if !value.is_empty() => value,
            _ => return Err(RuntimeError::new("module_from_spec() requires spec.name")),
        };
        let loader = spec_field(&spec, "loader").unwrap_or(Value::None);
        let origin = spec_field(&spec, "origin").unwrap_or(Value::None);
        let cached = spec_field(&spec, "cached").unwrap_or(Value::None);
        let submodule_search_locations =
            spec_field(&spec, "submodule_search_locations").unwrap_or(Value::None);
        let package = if matches!(submodule_search_locations, Value::None) {
            spec_field(&spec, "parent").unwrap_or_else(|| {
                Value::Str(
                    name.rsplit_once('.')
                        .map(|(parent, _)| parent.to_string())
                        .unwrap_or_default(),
                )
            })
        } else {
            Value::Str(name.clone())
        };

        let module = match self.heap.alloc_module(ModuleObject::new(name.clone())) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name));
            module_data
                .globals
                .insert("__loader__".to_string(), loader.clone());
            module_data
                .globals
                .insert("__package__".to_string(), package);
            module_data
                .globals
                .insert("__spec__".to_string(), spec.clone());
            if !matches!(origin, Value::None) {
                module_data.globals.insert("__file__".to_string(), origin);
            }
            if !matches!(cached, Value::None) {
                module_data.globals.insert("__cached__".to_string(), cached);
            }
            if !matches!(submodule_search_locations, Value::None) {
                module_data
                    .globals
                    .insert("__path__".to_string(), submodule_search_locations);
            }
        }
        Ok(Value::Module(module))
    }

    pub(super) fn builtin_frozen_importlib_spec_from_loader(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 4 {
            return Err(RuntimeError::new(
                "spec_from_loader() expects name, loader, and optional origin/is_package",
            ));
        }
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(RuntimeError::new("spec_from_loader() name must be string")),
        };
        let loader = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("loader").unwrap_or(Value::None)
        };
        let origin = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("origin").unwrap_or(Value::None)
        };
        let is_package_value = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs.remove("is_package").unwrap_or(Value::Bool(false))
        };
        kwargs.remove("cached");
        kwargs.remove("submodule_search_locations");
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "spec_from_loader() got an unexpected keyword argument",
            ));
        }
        let is_package = match is_package_value {
            Value::Bool(value) => value,
            other => is_truthy(&other),
        };
        let spec = self.build_module_spec_value(
            &name,
            None,
            None,
            if matches!(loader, Value::None) {
                None
            } else {
                Some(BUILTIN_MODULE_LOADER)
            },
            is_package,
            &[],
            false,
        );
        self.set_module_spec_field(&spec, "loader", loader);
        self.set_module_spec_field(&spec, "origin", origin.clone());
        self.set_module_spec_field(
            &spec,
            "has_location",
            Value::Bool(!matches!(origin, Value::None)),
        );
        Ok(spec)
    }

    pub(super) fn builtin_frozen_importlib_verbose_message(
        &self,
        _args: Vec<Value>,
        _kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        Ok(Value::None)
    }

    pub(super) fn builtin_frozen_importlib_external_path_join(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "_path_join() got unexpected keyword argument",
            ));
        }
        let mut out = PathBuf::new();
        for part in args {
            out.push(value_to_path(&part)?);
        }
        Ok(Value::Str(out.to_string_lossy().to_string()))
    }

    pub(super) fn builtin_frozen_importlib_external_path_split(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_path_split() expects one path argument"));
        }
        let path = PathBuf::from(value_to_path(&args[0])?);
        let parent = path
            .parent()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        let tail = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(self
            .heap
            .alloc_tuple(vec![Value::Str(parent), Value::Str(tail)]))
    }

    pub(super) fn builtin_frozen_importlib_external_path_stat(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_path_stat() expects one path argument"));
        }
        let path = self.path_arg_to_string(args[0].clone())?;
        let metadata =
            fs::metadata(path).map_err(|err| RuntimeError::new(format!("stat failed: {err}")))?;
        self.build_stat_result(metadata, false)
    }

    pub(super) fn builtin_frozen_importlib_external_unpack_uint16(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_frozen_importlib_external_unpack_uint(args, kwargs, 2)
    }

    pub(super) fn builtin_frozen_importlib_external_pack_uint32(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_pack_uint32() expects one integer argument",
            ));
        }
        let value = value_to_int(args[0].clone())?;
        Ok(self.heap.alloc_bytes((value as u32).to_le_bytes().to_vec()))
    }

    pub(super) fn builtin_frozen_importlib_external_unpack_uint32(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_frozen_importlib_external_unpack_uint(args, kwargs, 4)
    }

    pub(super) fn builtin_frozen_importlib_external_unpack_uint64(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_frozen_importlib_external_unpack_uint(args, kwargs, 8)
    }

    pub(super) fn builtin_frozen_importlib_external_unpack_uint(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        width: usize,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_unpack_uint*() expects one bytes argument",
            ));
        }
        let bytes = bytes_like_from_value(args[0].clone())?;
        if bytes.len() < width {
            return Err(RuntimeError::new("_unpack_uint*() argument too short"));
        }
        let value = match width {
            2 => u16::from_le_bytes([bytes[0], bytes[1]]) as u64,
            4 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
            8 => u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            _ => unreachable!(),
        };
        if value > i64::MAX as u64 {
            return Err(RuntimeError::new(
                "_unpack_uint*() value exceeds runtime int range",
            ));
        }
        Ok(Value::Int(value as i64))
    }

    pub(super) fn builtin_opcode_stack_effect(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "stack_effect() expects opcode and optional oparg",
            ));
        }
        let _jump = kwargs.remove("jump");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "stack_effect() got an unexpected keyword argument",
            ));
        }
        let opcode = value_to_int(args.remove(0))?;
        if !args.is_empty() {
            let _ = value_to_int(args.remove(0))?;
        }
        let info = self
            .opcode_info_by_number(opcode)
            .ok_or_else(|| RuntimeError::new("stack_effect() unknown opcode"))?;
        Ok(Value::Int(i64::from(info.stack_effect)))
    }

    pub(super) fn builtin_opcode_has_arg(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_opcode_has_flag(args, kwargs, "ARG", "has_arg")
    }

    pub(super) fn builtin_opcode_has_const(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_opcode_has_flag(args, kwargs, "CONST", "has_const")
    }

    pub(super) fn builtin_opcode_has_name(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_opcode_has_flag(args, kwargs, "NAME", "has_name")
    }

    pub(super) fn builtin_opcode_has_jump(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_opcode_has_flag(args, kwargs, "JUMP", "has_jump")
    }

    pub(super) fn builtin_opcode_has_free(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_opcode_has_flag(args, kwargs, "FREE", "has_free")
    }

    pub(super) fn builtin_opcode_has_local(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_opcode_has_flag(args, kwargs, "LOCAL", "has_local")
    }

    pub(super) fn builtin_opcode_has_exc(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("has_exc() expects one opcode"));
        }
        let _ = value_to_int(args[0].clone())?;
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_opcode_get_intrinsic1_descs(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "get_intrinsic1_descs() expects no arguments",
            ));
        }
        Ok(self.heap.alloc_list(vec![
            Value::Str("INTRINSIC_1_INVALID".to_string()),
            Value::Str("INTRINSIC_PRINT".to_string()),
            Value::Str("INTRINSIC_IMPORT_STAR".to_string()),
            Value::Str("INTRINSIC_STOPITERATION_ERROR".to_string()),
            Value::Str("INTRINSIC_ASYNC_GEN_WRAP".to_string()),
            Value::Str("INTRINSIC_UNARY_POSITIVE".to_string()),
            Value::Str("INTRINSIC_LIST_TO_TUPLE".to_string()),
            Value::Str("INTRINSIC_TYPEVAR".to_string()),
            Value::Str("INTRINSIC_PARAMSPEC".to_string()),
            Value::Str("INTRINSIC_TYPEVARTUPLE".to_string()),
            Value::Str("INTRINSIC_SUBSCRIPT_GENERIC".to_string()),
            Value::Str("INTRINSIC_TYPEALIAS".to_string()),
        ]))
    }

    pub(super) fn builtin_opcode_get_intrinsic2_descs(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "get_intrinsic2_descs() expects no arguments",
            ));
        }
        Ok(self.heap.alloc_list(vec![
            Value::Str("INTRINSIC_2_INVALID".to_string()),
            Value::Str("INTRINSIC_PREP_RERAISE_STAR".to_string()),
            Value::Str("INTRINSIC_TYPEVAR_WITH_BOUND".to_string()),
            Value::Str("INTRINSIC_TYPEVAR_WITH_CONSTRAINTS".to_string()),
            Value::Str("INTRINSIC_SET_FUNCTION_TYPE_PARAMS".to_string()),
            Value::Str("INTRINSIC_SET_TYPEPARAM_DEFAULT".to_string()),
        ]))
    }

    pub(super) fn builtin_opcode_get_special_method_names(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "get_special_method_names() expects no arguments",
            ));
        }
        Ok(self.heap.alloc_list(vec![
            Value::Str("__enter__".to_string()),
            Value::Str("__exit__".to_string()),
            Value::Str("__aenter__".to_string()),
            Value::Str("__aexit__".to_string()),
        ]))
    }

    pub(super) fn builtin_opcode_get_nb_ops(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("get_nb_ops() expects no arguments"));
        }
        let mut rows = Vec::new();
        for (name, symbol) in [
            ("NB_ADD", "+"),
            ("NB_AND", "&"),
            ("NB_FLOOR_DIVIDE", "//"),
            ("NB_LSHIFT", "<<"),
            ("NB_MATRIX_MULTIPLY", "@"),
            ("NB_MULTIPLY", "*"),
            ("NB_REMAINDER", "%"),
            ("NB_OR", "|"),
            ("NB_POWER", "**"),
            ("NB_RSHIFT", ">>"),
            ("NB_SUBTRACT", "-"),
            ("NB_TRUE_DIVIDE", "/"),
            ("NB_XOR", "^"),
            ("NB_INPLACE_ADD", "+="),
            ("NB_INPLACE_AND", "&="),
            ("NB_INPLACE_FLOOR_DIVIDE", "//="),
            ("NB_INPLACE_LSHIFT", "<<="),
            ("NB_INPLACE_MATRIX_MULTIPLY", "@="),
            ("NB_INPLACE_MULTIPLY", "*="),
            ("NB_INPLACE_REMAINDER", "%="),
            ("NB_INPLACE_OR", "|="),
            ("NB_INPLACE_POWER", "**="),
            ("NB_INPLACE_RSHIFT", ">>="),
            ("NB_INPLACE_SUBTRACT", "-="),
            ("NB_INPLACE_TRUE_DIVIDE", "/="),
            ("NB_INPLACE_XOR", "^="),
            ("NB_SUBSCR", "[]"),
        ] {
            rows.push(self.heap.alloc_tuple(vec![
                Value::Str(name.to_string()),
                Value::Str(symbol.to_string()),
            ]));
        }
        Ok(self.heap.alloc_list(rows))
    }

    pub(super) fn builtin_opcode_get_executor(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "get_executor() expects code object and instruction offset",
            ));
        }
        if !matches!(args[0], Value::Code(_)) {
            return Err(RuntimeError::type_error(
                "get_executor() expected code object as first argument",
            ));
        }
        let _ = value_to_int(args[1].clone())?;
        Ok(Value::None)
    }

    pub(super) fn builtin_opcode_has_flag(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        flag: &str,
        name: &str,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(format!("{name}() expects one opcode")));
        }
        let opcode = value_to_int(args[0].clone())?;
        let value = self
            .opcode_info_by_number(opcode)
            .map(|info| opcode_flags_contains(&info.flags, flag))
            .unwrap_or(false);
        Ok(Value::Bool(value))
    }

    pub(super) fn opcode_metadata(&self) -> &OpcodeMetadata {
        OPCODE_METADATA.get_or_init(|| {
            OpcodeMetadata::load_default().unwrap_or_else(|_| OpcodeMetadata::empty())
        })
    }

    pub(super) fn opcode_info_by_number(
        &self,
        opcode: i64,
    ) -> Option<&crate::bytecode::metadata::OpcodeInfo> {
        if opcode < 0 || opcode > i64::from(u16::MAX) {
            return None;
        }
        let code = opcode as u16;
        self.opcode_metadata()
            .opcodes
            .iter()
            .find(|info| info.code == code)
    }

    pub(super) fn builtin_importlib_source_from_cache(
        &self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "source_from_cache() expects one path argument",
            ));
        }
        let path = match args.remove(0) {
            Value::Str(path) => path,
            _ => return Err(RuntimeError::new("path must be string")),
        };
        let source = source_path_from_cache_path(&path);
        Ok(Value::Str(source))
    }

    pub(super) fn builtin_importlib_cache_from_source(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let mut path = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = kwargs.remove("path") {
            value
        } else {
            return Err(RuntimeError::new(
                "cache_from_source() missing required argument 'path'",
            ));
        };
        if let Some(value) = kwargs.remove("path") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "cache_from_source() got multiple values for argument 'path'",
                ));
            }
            path = value;
        }

        let mut debug_override = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("debug_override") {
            if debug_override.is_some() {
                return Err(RuntimeError::new(
                    "cache_from_source() got multiple values for argument 'debug_override'",
                ));
            }
            debug_override = Some(value);
        }
        if !args.is_empty() {
            return Err(RuntimeError::new(
                "cache_from_source() takes at most 2 positional arguments",
            ));
        }

        let mut optimization = kwargs.remove("optimization");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "cache_from_source() got an unexpected keyword argument",
            ));
        }

        if let Some(debug_override) = debug_override
            && !matches!(debug_override, Value::None)
        {
            if optimization.is_some() {
                return Err(RuntimeError::new(
                    "debug_override or optimization must be set to None",
                ));
            }
            optimization = Some(if is_truthy(&debug_override) {
                Value::Str(String::new())
            } else {
                Value::Int(1)
            });
        }

        let path = match path {
            Value::Str(path) => path,
            _ => return Err(RuntimeError::new("path must be string")),
        };
        let optimization = match optimization {
            None | Some(Value::None) => String::new(),
            Some(Value::Str(text)) => text,
            Some(Value::Int(value)) => value.to_string(),
            Some(Value::BigInt(value)) => value.to_string(),
            Some(Value::Bool(value)) => {
                if value {
                    "True".to_string()
                } else {
                    "False".to_string()
                }
            }
            Some(other) => {
                return Err(RuntimeError::new(format!(
                    "optimization must be str, int, bool, or None, not {}",
                    self.value_type_name_for_error(&other)
                )));
            }
        };
        if !optimization.is_empty() && !optimization.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return Err(RuntimeError::new(format!(
                "{optimization:?} is not alphanumeric"
            )));
        }
        let cache = if optimization.is_empty() {
            cache_path_from_source_path(&path)
        } else {
            cache_path_from_source_path_with_optimization(&path, &optimization)
        };
        Ok(Value::Str(cache))
    }
}
