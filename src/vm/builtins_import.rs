use super::*;

impl Vm {
    pub(super) fn builtin_import(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let caller_depth = self.frames.len();
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
        let module = self.import_module_object(&resolved_name)?;
        self.run_pending_import_frames(caller_depth)?;
        let module = self.canonical_imported_module_for_name(&resolved_name, module);
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
        let caller_depth = self.frames.len();
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
        let module = self.import_module_object(&resolved_name)?;
        self.run_pending_import_frames(caller_depth)?;
        let module = self.canonical_imported_module_for_name(&resolved_name, module);
        Ok(Value::Module(module))
    }

    pub(super) fn run_pending_import_frames(
        &mut self,
        caller_depth: usize,
    ) -> Result<(), RuntimeError> {
        if self.frames.len() <= caller_depth {
            return Ok(());
        }
        let previous_stop = self.run_stop_depth;
        self.run_stop_depth = Some(caller_depth);
        let run_result = self.run();
        self.run_stop_depth = previous_stop;
        run_result?;
        Ok(())
    }

    pub(super) fn return_imported_module(
        &mut self,
        module: ObjRef,
        caller_depth: usize,
    ) -> Result<ObjRef, RuntimeError> {
        // Only force immediate module execution for host-side imports
        // (no active Python frame). In-frame imports are executed by the
        // main VM loop and should not recurse into `run()`.
        if caller_depth == 0 {
            self.run_pending_import_frames(caller_depth)?;
        }
        let module_name = match &*module.kind() {
            Object::Module(module_data) => module_data.name.clone(),
            _ => String::new(),
        };
        let canonical = self.canonical_imported_module_for_name(&module_name, module);
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
            value = self.builtin_getattr(
                vec![value, Value::Str(part.to_string())],
                HashMap::new(),
            )?;
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

        if let Some(existing) = self.modules.get(&resolved_name).cloned() {
            if let Object::Module(module_data) = &*existing.kind() {
                if let Some(spec) = module_data.globals.get("__spec__").cloned() {
                    return Ok(spec);
                }
            }
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
        let origin = if source_info.is_namespace {
            None
        } else {
            Some(&source_info.path)
        };
        Ok(self.build_module_spec_value(
            &resolved_name,
            origin,
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
        let Some(sys_module) = self.modules.get("sys").cloned() else {
            return Ok(Value::None);
        };
        let Object::Module(module_data) = &mut *sys_module.kind_mut() else {
            return Ok(Value::None);
        };
        if let Some(Value::Dict(cache)) = module_data.globals.get("path_importer_cache").cloned() {
            if let Object::Dict(entries) = &mut *cache.kind_mut() {
                entries.clear();
            }
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
        let loader = kwargs
            .remove("loader")
            .unwrap_or(Value::Str(SOURCE_FILE_LOADER.to_string()));
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
        let location_path = PathBuf::from(&location);
        let spec =
            self.build_module_spec_value(&name, Some(&location_path), None, is_package, &[], false);
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
        self.builtin_opcode_has_flag(args, kwargs, "ESCAPES", "has_exc")
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
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "cache_from_source() expects one path argument",
            ));
        }
        let path = match args.remove(0) {
            Value::Str(path) => path,
            _ => return Err(RuntimeError::new("path must be string")),
        };
        let cache = cache_path_from_source_path(&path);
        Ok(Value::Str(cache))
    }
}
