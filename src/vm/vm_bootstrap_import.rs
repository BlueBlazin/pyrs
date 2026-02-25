use super::{
    AtomicOrdering, BUILTIN_MODULE_LOADER, BuiltinFunction, ClassObject, DEFAULT_META_PATH_FINDER,
    DEFAULT_PATH_HOOK, DefaultHasher, EXTENSION_FILE_LOADER, Frame, Hash, HashMap, HashSet, Hasher,
    ImportDirCacheEntry, InstanceObject, LOCAL_SHIM_MODULES, LOCAL_SHIM_PRECEDENCE_MODULES,
    ModuleObject, ModuleSourceInfo, NAMESPACE_LOADER, ObjRef, Object,
    PURE_STDLIB_COLLECTIONS_MODULES, PURE_STDLIB_DECIMAL_MODULES, PURE_STDLIB_JSON_MODULES,
    PURE_STDLIB_PATHLIB_MODULES, PURE_STDLIB_PICKLE_MODULES, PURE_STDLIB_RE_MODULES,
    PURE_STDLIB_TYPES_MODULES, Path, PathBuf, Rc, RuntimeError, SIGNAL_DEFAULT, SIGNAL_IGNORE,
    SIGNAL_SIGINT, SIGNAL_SIGTERM, SOURCE_FILE_LOADER, SOURCELESS_FILE_LOADER,
    SUBMODULE_TRACE_COUNT, Value, Vm, cached_module_path, compiler, cpython, dict_get_value,
    dict_remove_value, dict_set_value, matches_finder_kind, parse_uuid_like_string, parser,
    source_path_from_cache_path,
};
use crate::extensions::{
    PYRS_EXTENSION_MANIFEST_SUFFIX, find_shared_library_for_module, find_shared_library_for_package,
};

const PYRS_MODULE_INITIALIZING_FLAG: &str = "__pyrs_module_initializing__";

impl Vm {
    fn alloc_tuple_backed_builtin_class(&mut self, name: &str) -> Value {
        let mut bases = Vec::new();
        if let Some(Value::Class(tuple_class)) = self.builtins.get("tuple") {
            bases.push(tuple_class.clone());
        }
        let class = self
            .heap
            .alloc_class(ClassObject::new(name.to_string(), bases));
        if let Value::Class(class_obj) = &class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data
                .attrs
                .insert("__pyrs_tuple_backed_type__".to_string(), Value::Bool(true));
        }
        class
    }

    fn configure_bootstrap_ast_class(
        &mut self,
        class_name: &str,
        fields: &[&str],
        attributes: &[&str],
    ) {
        let Some(ast_module) = self.modules.get("_ast").cloned() else {
            return;
        };
        let class_ref = {
            let module_kind = ast_module.kind();
            let module_data = match &*module_kind {
                Object::Module(module_data) => module_data,
                _ => return,
            };
            match module_data.globals.get(class_name) {
                Some(Value::Class(class_ref)) => class_ref.clone(),
                _ => return,
            }
        };

        let field_tuple = self.heap.alloc_tuple(
            fields
                .iter()
                .map(|entry| Value::Str((*entry).to_string()))
                .collect(),
        );
        let attributes_tuple = self.heap.alloc_tuple(
            attributes
                .iter()
                .map(|entry| Value::Str((*entry).to_string()))
                .collect(),
        );
        if let Object::Class(class_data) = &mut *class_ref.kind_mut() {
            class_data
                .attrs
                .insert("_fields".to_string(), field_tuple.clone());
            class_data
                .attrs
                .insert("__match_args__".to_string(), field_tuple);
            class_data
                .attrs
                .insert("_attributes".to_string(), attributes_tuple);
        }
    }

    fn configure_bootstrap_ast_metadata(&mut self) {
        const LOC_ATTRS: [&str; 4] = ["lineno", "col_offset", "end_lineno", "end_col_offset"];
        self.configure_bootstrap_ast_class("AST", &[], &[]);
        self.configure_bootstrap_ast_class("mod", &[], &[]);
        self.configure_bootstrap_ast_class("stmt", &[], &[]);
        self.configure_bootstrap_ast_class("expr", &[], &[]);
        self.configure_bootstrap_ast_class("expr_context", &[], &[]);
        self.configure_bootstrap_ast_class("operator", &[], &[]);
        self.configure_bootstrap_ast_class("unaryop", &[], &[]);
        self.configure_bootstrap_ast_class("boolop", &[], &[]);
        self.configure_bootstrap_ast_class("cmpop", &[], &[]);

        self.configure_bootstrap_ast_class("Module", &["body", "type_ignores"], &[]);
        self.configure_bootstrap_ast_class("Expression", &["body"], &[]);
        self.configure_bootstrap_ast_class(
            "FunctionDef",
            &[
                "name",
                "args",
                "body",
                "decorator_list",
                "returns",
                "type_comment",
                "type_params",
            ],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "AsyncFunctionDef",
            &[
                "name",
                "args",
                "body",
                "decorator_list",
                "returns",
                "type_comment",
                "type_params",
            ],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "ClassDef",
            &[
                "name",
                "bases",
                "keywords",
                "body",
                "decorator_list",
                "type_params",
            ],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "TypeAlias",
            &["name", "type_params", "value"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "Assign",
            &["targets", "value", "type_comment"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("AugAssign", &["target", "op", "value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class(
            "AnnAssign",
            &["target", "annotation", "value", "simple"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("Delete", &["targets"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Return", &["value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Raise", &["exc", "cause"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Assert", &["test", "msg"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Expr", &["value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Pass", &[], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Break", &[], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Continue", &[], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Match", &["subject", "cases"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("If", &["test", "body", "orelse"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("While", &["test", "body", "orelse"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class(
            "For",
            &["target", "iter", "body", "orelse", "type_comment"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "AsyncFor",
            &["target", "iter", "body", "orelse", "type_comment"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("Import", &["names"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("ImportFrom", &["module", "names", "level"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Global", &["names"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Nonlocal", &["names"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("With", &["items", "body", "type_comment"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class(
            "AsyncWith",
            &["items", "body", "type_comment"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "Try",
            &["body", "handlers", "orelse", "finalbody"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "TryStar",
            &["body", "handlers", "orelse", "finalbody"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("excepthandler", &[], &[]);
        self.configure_bootstrap_ast_class("ExceptHandler", &["type", "name", "body"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("alias", &["name", "asname"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("withitem", &["context_expr", "optional_vars"], &[]);
        self.configure_bootstrap_ast_class(
            "arguments",
            &[
                "posonlyargs",
                "args",
                "vararg",
                "kwonlyargs",
                "kw_defaults",
                "kwarg",
                "defaults",
            ],
            &[],
        );
        self.configure_bootstrap_ast_class(
            "arg",
            &["arg", "annotation", "type_comment"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("match_case", &["pattern", "guard", "body"], &[]);
        self.configure_bootstrap_ast_class("pattern", &[], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("MatchValue", &["value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("MatchSingleton", &["value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("MatchSequence", &["patterns"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class(
            "MatchMapping",
            &["keys", "patterns", "rest"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class(
            "MatchClass",
            &["cls", "patterns", "kwd_attrs", "kwd_patterns"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("MatchStar", &["name"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("MatchAs", &["pattern", "name"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("MatchOr", &["patterns"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("type_param", &[], &LOC_ATTRS);
        self.configure_bootstrap_ast_class(
            "TypeVar",
            &["name", "bound", "default_value"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("ParamSpec", &["name", "default_value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("TypeVarTuple", &["name", "default_value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Name", &["id", "ctx"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Call", &["func", "args", "keywords"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("keyword", &["arg", "value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Attribute", &["value", "attr", "ctx"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Subscript", &["value", "slice", "ctx"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Tuple", &["elts", "ctx"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("List", &["elts", "ctx"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Dict", &["keys", "values"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Constant", &["value", "kind"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Starred", &["value", "ctx"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Slice", &["lower", "upper", "step"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("BinOp", &["left", "op", "right"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("UnaryOp", &["op", "operand"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Compare", &["left", "ops", "comparators"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class(
            "Interpolation",
            &["value", "str", "conversion", "format_spec"],
            &LOC_ATTRS,
        );
        self.configure_bootstrap_ast_class("TemplateStr", &["values"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("BoolOp", &["op", "values"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("IfExp", &["test", "body", "orelse"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("NamedExpr", &["target", "value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Lambda", &["args", "body"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Await", &["value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("ListComp", &["elt", "generators"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("SetComp", &["elt", "generators"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("DictComp", &["key", "value", "generators"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("GeneratorExp", &["elt", "generators"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("Yield", &["value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class("YieldFrom", &["value"], &LOC_ATTRS);
        self.configure_bootstrap_ast_class(
            "comprehension",
            &["target", "iter", "ifs", "is_async"],
            &[],
        );

        self.configure_bootstrap_ast_class("Load", &[], &[]);
        self.configure_bootstrap_ast_class("Store", &[], &[]);
        self.configure_bootstrap_ast_class("Del", &[], &[]);
        self.configure_bootstrap_ast_class("And", &[], &[]);
        self.configure_bootstrap_ast_class("Or", &[], &[]);
        self.configure_bootstrap_ast_class("Not", &[], &[]);
        self.configure_bootstrap_ast_class("Add", &[], &[]);
        self.configure_bootstrap_ast_class("Sub", &[], &[]);
        self.configure_bootstrap_ast_class("Mult", &[], &[]);
        self.configure_bootstrap_ast_class("MatMult", &[], &[]);
        self.configure_bootstrap_ast_class("Div", &[], &[]);
        self.configure_bootstrap_ast_class("FloorDiv", &[], &[]);
        self.configure_bootstrap_ast_class("Mod", &[], &[]);
        self.configure_bootstrap_ast_class("Pow", &[], &[]);
        self.configure_bootstrap_ast_class("LShift", &[], &[]);
        self.configure_bootstrap_ast_class("RShift", &[], &[]);
        self.configure_bootstrap_ast_class("BitAnd", &[], &[]);
        self.configure_bootstrap_ast_class("BitOr", &[], &[]);
        self.configure_bootstrap_ast_class("BitXor", &[], &[]);
        self.configure_bootstrap_ast_class("UAdd", &[], &[]);
        self.configure_bootstrap_ast_class("USub", &[], &[]);
        self.configure_bootstrap_ast_class("Invert", &[], &[]);
        self.configure_bootstrap_ast_class("Eq", &[], &[]);
        self.configure_bootstrap_ast_class("NotEq", &[], &[]);
        self.configure_bootstrap_ast_class("Lt", &[], &[]);
        self.configure_bootstrap_ast_class("LtE", &[], &[]);
        self.configure_bootstrap_ast_class("Gt", &[], &[]);
        self.configure_bootstrap_ast_class("GtE", &[], &[]);
        self.configure_bootstrap_ast_class("In", &[], &[]);
        self.configure_bootstrap_ast_class("NotIn", &[], &[]);
        self.configure_bootstrap_ast_class("Is", &[], &[]);
        self.configure_bootstrap_ast_class("IsNot", &[], &[]);
    }

    fn sys_str_value(&self, name: &str) -> Option<String> {
        let sys_module = self.modules.get("sys")?.clone();
        let module_kind = sys_module.kind();
        let module_data = match &*module_kind {
            Object::Module(module_data) => module_data,
            _ => return None,
        };
        match module_data.globals.get(name) {
            Some(Value::Str(text)) => Some(text.clone()),
            _ => None,
        }
    }

    fn sys_list_obj(&self, name: &str) -> Option<ObjRef> {
        let sys_module = self.modules.get("sys")?.clone();
        let module_kind = sys_module.kind();
        let module_data = match &*module_kind {
            Object::Module(module_data) => module_data,
            _ => return None,
        };
        match module_data.globals.get(name) {
            Some(Value::List(list)) => Some(list.clone()),
            _ => None,
        }
    }

    fn list_has_default_finder(&self, list_name: &str, default_kind: &str) -> bool {
        let Some(list_obj) = self.sys_list_obj(list_name) else {
            return false;
        };
        let list_kind = list_obj.kind();
        let Object::List(values) = &*list_kind else {
            return false;
        };
        for value in values {
            if matches_finder_kind(value, default_kind) {
                return true;
            }
        }
        false
    }

    fn list_signature(values: &[Value]) -> u64 {
        let mut hasher = DefaultHasher::new();
        values.len().hash(&mut hasher);
        for value in values {
            match value {
                Value::Str(path) => {
                    1u8.hash(&mut hasher);
                    path.hash(&mut hasher);
                }
                other => {
                    0u8.hash(&mut hasher);
                    std::mem::discriminant(other).hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    fn sys_list_signature(&self, name: &str) -> Option<u64> {
        let list_obj = self.sys_list_obj(name)?;
        let list_kind = list_obj.kind();
        let Object::List(values) = &*list_kind else {
            return None;
        };
        Some(Self::list_signature(values))
    }

    pub(super) fn refresh_import_resolver_state(&mut self) {
        self.sync_module_paths_from_sys();
        match self.sys_list_signature("meta_path") {
            Some(signature) if signature != self.import_meta_path_signature => {
                self.import_meta_path_signature = signature;
                self.import_meta_path_has_default_finder =
                    self.list_has_default_finder("meta_path", DEFAULT_META_PATH_FINDER);
            }
            None => {
                self.import_meta_path_signature = 0;
                self.import_meta_path_has_default_finder = false;
            }
            _ => {}
        }
        match self.sys_list_signature("path_hooks") {
            Some(signature) if signature != self.import_path_hooks_signature => {
                self.import_path_hooks_signature = signature;
                self.import_path_hooks_has_default_hook =
                    self.list_has_default_finder("path_hooks", DEFAULT_PATH_HOOK);
            }
            None => {
                self.import_path_hooks_signature = 0;
                self.import_path_hooks_has_default_hook = false;
            }
            _ => {}
        }
    }

    fn source_timestamp_and_size(path: &Path) -> Option<(u32, u32)> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified = metadata.modified().ok()?;
        let timestamp = modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| u32::try_from(duration.as_secs()).ok())?;
        let size = u32::try_from(metadata.len()).ok()?;
        Some((timestamp, size))
    }

    fn directory_mtime_ns(path: &Path) -> Option<u128> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified = metadata.modified().ok()?;
        let duration = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(duration.as_secs() as u128 * 1_000_000_000 + duration.subsec_nanos() as u128)
    }

    fn directory_contains_entry_cached(
        &mut self,
        directory: &Path,
        entry: &std::ffi::OsStr,
    ) -> bool {
        let mtime_ns = Self::directory_mtime_ns(directory);
        let refresh = self
            .import_dir_cache
            .get(directory)
            .is_none_or(|cached| cached.mtime_ns != mtime_ns);
        if refresh {
            let entries = match std::fs::read_dir(directory) {
                Ok(read_dir) => read_dir
                    .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
                    .collect::<HashSet<_>>(),
                Err(_) => {
                    self.import_dir_cache.remove(directory);
                    return false;
                }
            };
            self.import_dir_cache.insert(
                directory.to_path_buf(),
                ImportDirCacheEntry { mtime_ns, entries },
            );
        }
        self.import_dir_cache
            .get(directory)
            .is_some_and(|cached| cached.entries.contains(entry))
    }

    fn cached_path_is_file(&mut self, path: &Path) -> bool {
        let Some(file_name) = path.file_name() else {
            return false;
        };
        let Some(parent) = path.parent() else {
            return false;
        };
        if !self.directory_contains_entry_cached(parent, file_name) {
            return false;
        }
        path.is_file()
    }

    fn cached_path_is_dir(&mut self, path: &Path) -> bool {
        let Some(dir_name) = path.file_name() else {
            return false;
        };
        let Some(parent) = path.parent() else {
            return false;
        };
        if !self.directory_contains_entry_cached(parent, dir_name) {
            return false;
        }
        path.is_dir()
    }

    fn pyc_matches_source(pyc_path: &Path, source_path: &Path) -> bool {
        let (source_timestamp, source_size) = match Self::source_timestamp_and_size(source_path) {
            Some(value) => value,
            None => return false,
        };
        let mut header = [0u8; 16];
        let Ok(mut file) = std::fs::File::open(pyc_path) else {
            return false;
        };
        use std::io::Read;
        if file.read_exact(&mut header).is_err() {
            return false;
        }
        let parsed = match crate::bytecode::pyc::parse_pyc_header(&header) {
            Ok((parsed, _)) => parsed,
            Err(_) => return false,
        };
        if parsed.bitfield & 0x01 != 0 {
            return false;
        }
        matches!(
            (parsed.timestamp, parsed.source_size),
            (Some(ts), Some(sz)) if ts == source_timestamp && sz == source_size
        )
    }

    pub(super) fn queue_source_module_execution(
        &mut self,
        module: &ObjRef,
        name: &str,
        source_path: &Path,
    ) -> Result<(), RuntimeError> {
        if self.import_perf_enabled {
            self.import_perf_counters.fs_source_compiles = self
                .import_perf_counters
                .fs_source_compiles
                .saturating_add(1);
        }
        let source = std::fs::read_to_string(source_path)
            .map_err(|err| RuntimeError::new(format!("failed to read module '{name}': {err}")))?;
        let source_filename = source_path.to_string_lossy().to_string();
        self.cache_source_text(&source_filename, &source);

        let module_ast = parser::parse_module(&source).map_err(|err| {
            RuntimeError::new(format!(
                "parse error in module '{name}' at {}: {}",
                err.offset, err.message
            ))
        })?;
        let code = compiler::compile_module_with_filename(&module_ast, &source_filename).map_err(
            |err| {
                let detail = if let Some(span) = err.span {
                    format!("{} at {}:{}", err.message, span.line, span.column)
                } else {
                    err.message
                };
                RuntimeError::new(format!("compile error in module '{name}': {detail}"))
            },
        )?;
        self.mark_module_initializing(module);
        let code = Rc::new(code);
        let cells = self.build_cells(&code, Vec::new());
        let mut frame = Frame::new(code, module.clone(), true, false, cells, None);
        frame.discard_result = true;
        self.frames.push(Box::new(frame));
        Ok(())
    }

    pub(super) fn set_module_class_bases(
        &mut self,
        module_name: &str,
        class_name: &str,
        base_names: &[&str],
    ) -> Result<(), RuntimeError> {
        let module = self.modules.get(module_name).cloned().ok_or_else(|| {
            RuntimeError::module_not_found_error(format!("module '{module_name}' not found"))
        })?;
        let (class_ref, base_refs) = {
            let Object::Module(module_data) = &*module.kind() else {
                return Err(RuntimeError::new(format!(
                    "module '{module_name}' is invalid"
                )));
            };
            let class_ref = match module_data.globals.get(class_name) {
                Some(Value::Class(class)) => class.clone(),
                _ => {
                    return Err(RuntimeError::new(format!(
                        "module '{module_name}' has no class '{class_name}'",
                    )));
                }
            };
            let mut base_refs = Vec::new();
            for base_name in base_names {
                if let Some(Value::Class(base)) = module_data.globals.get(*base_name) {
                    base_refs.push(base.clone());
                    continue;
                }
                if let Some(Value::Class(base)) = self.builtins.get(*base_name) {
                    base_refs.push(base.clone());
                    continue;
                }
                return Err(RuntimeError::new(format!(
                    "module '{module_name}' has no class '{base_name}'",
                )));
            }
            (class_ref, base_refs)
        };
        let Object::Class(class_data) = &mut *class_ref.kind_mut() else {
            return Err(RuntimeError::new("target class object is invalid"));
        };
        class_data.bases = base_refs;
        class_data.mro.clear();
        Ok(())
    }

    pub(super) fn wire_io_class_hierarchy(&mut self) {
        for module in ["io", "_io"] {
            let _ = self.set_module_class_bases(module, "RawIOBase", &["IOBase"]);
            let _ = self.set_module_class_bases(module, "BufferedIOBase", &["IOBase"]);
            let _ = self.set_module_class_bases(module, "TextIOBase", &["IOBase"]);
            let _ = self.set_module_class_bases(module, "FileIO", &["RawIOBase"]);
            let _ = self.set_module_class_bases(module, "BufferedReader", &["BufferedIOBase"]);
            let _ = self.set_module_class_bases(module, "BufferedWriter", &["BufferedIOBase"]);
            let _ = self.set_module_class_bases(module, "BufferedRandom", &["BufferedIOBase"]);
            let _ = self.set_module_class_bases(module, "BufferedRWPair", &["BufferedIOBase"]);
            let _ = self.set_module_class_bases(module, "BytesIO", &["BufferedIOBase"]);
            let _ = self.set_module_class_bases(module, "StringIO", &["TextIOBase"]);
            let _ = self.set_module_class_bases(module, "TextIOWrapper", &["TextIOBase"]);
            if let Some(module_ref) = self.modules.get(module)
                && let Object::Module(module_data) = &mut *module_ref.kind_mut()
            {
                for (alias, canonical) in [
                    ("_IOBase", "IOBase"),
                    ("_RawIOBase", "RawIOBase"),
                    ("_BufferedIOBase", "BufferedIOBase"),
                    ("_TextIOBase", "TextIOBase"),
                ] {
                    if let Some(value) = module_data.globals.get(canonical).cloned() {
                        module_data.globals.insert(alias.to_string(), value);
                    }
                }
            }
        }
    }

    fn wire_ast_class_hierarchy(&mut self) {
        let _ = self.set_module_class_bases("_ast", "mod", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "stmt", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "expr", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "expr_context", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "operator", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "unaryop", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "boolop", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "cmpop", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "pattern", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "excepthandler", &["AST"]);

        let _ = self.set_module_class_bases("_ast", "Module", &["mod"]);
        let _ = self.set_module_class_bases("_ast", "Expression", &["mod"]);

        for class_name in [
            "FunctionDef",
            "AsyncFunctionDef",
            "ClassDef",
            "TypeAlias",
            "Assign",
            "AugAssign",
            "AnnAssign",
            "Delete",
            "Return",
            "Raise",
            "Assert",
            "Expr",
            "Pass",
            "Break",
            "Continue",
            "If",
            "While",
            "For",
            "AsyncFor",
            "Import",
            "ImportFrom",
            "Global",
            "Nonlocal",
            "With",
            "AsyncWith",
            "Try",
            "TryStar",
            "Match",
        ] {
            let _ = self.set_module_class_bases("_ast", class_name, &["stmt"]);
        }

        for class_name in [
            "Name",
            "Call",
            "Attribute",
            "Subscript",
            "Tuple",
            "List",
            "Dict",
            "Constant",
            "Starred",
            "Slice",
            "BinOp",
            "UnaryOp",
            "Compare",
            "BoolOp",
            "IfExp",
            "NamedExpr",
            "Lambda",
            "Await",
            "ListComp",
            "SetComp",
            "DictComp",
            "GeneratorExp",
            "Yield",
            "YieldFrom",
            "Interpolation",
            "TemplateStr",
        ] {
            let _ = self.set_module_class_bases("_ast", class_name, &["expr"]);
        }
        let _ = self.set_module_class_bases("_ast", "keyword", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "alias", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "withitem", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "arguments", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "arg", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "comprehension", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "type_param", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "TypeVar", &["type_param"]);
        let _ = self.set_module_class_bases("_ast", "ParamSpec", &["type_param"]);
        let _ = self.set_module_class_bases("_ast", "TypeVarTuple", &["type_param"]);
        let _ = self.set_module_class_bases("_ast", "match_case", &["AST"]);
        let _ = self.set_module_class_bases("_ast", "MatchValue", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "MatchSingleton", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "MatchSequence", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "MatchMapping", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "MatchClass", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "MatchStar", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "MatchAs", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "MatchOr", &["pattern"]);
        let _ = self.set_module_class_bases("_ast", "ExceptHandler", &["excepthandler"]);

        for class_name in ["Load", "Store", "Del"] {
            let _ = self.set_module_class_bases("_ast", class_name, &["expr_context"]);
        }
        for class_name in [
            "Add", "Sub", "Mult", "MatMult", "Div", "FloorDiv", "Mod", "Pow", "LShift", "RShift",
            "BitAnd", "BitOr", "BitXor",
        ] {
            let _ = self.set_module_class_bases("_ast", class_name, &["operator"]);
        }
        for class_name in ["UAdd", "USub", "Invert", "Not"] {
            let _ = self.set_module_class_bases("_ast", class_name, &["unaryop"]);
        }
        for class_name in ["And", "Or"] {
            let _ = self.set_module_class_bases("_ast", class_name, &["boolop"]);
        }
        for class_name in [
            "Eq", "NotEq", "Lt", "LtE", "Gt", "GtE", "In", "NotIn", "Is", "IsNot",
        ] {
            let _ = self.set_module_class_bases("_ast", class_name, &["cmpop"]);
        }
    }

    pub(super) fn install_stdlib_modules(&mut self) {
        let platform = match std::env::consts::OS {
            "macos" => "darwin",
            other => other,
        };
        self.install_builtin_module(
            "math",
            &[
                ("sqrt", BuiltinFunction::MathSqrt),
                ("copysign", BuiltinFunction::MathCopySign),
                ("ldexp", BuiltinFunction::MathLdExp),
                ("hypot", BuiltinFunction::MathHypot),
                ("fabs", BuiltinFunction::MathFAbs),
                ("exp", BuiltinFunction::MathExp),
                ("erfc", BuiltinFunction::MathErfc),
                ("log", BuiltinFunction::MathLog),
                ("log2", BuiltinFunction::MathLog2),
                ("lgamma", BuiltinFunction::MathLGamma),
                ("fsum", BuiltinFunction::MathFSum),
                ("sumprod", BuiltinFunction::MathSumProd),
                ("cos", BuiltinFunction::MathCos),
                ("sin", BuiltinFunction::MathSin),
                ("tan", BuiltinFunction::MathTan),
                ("cosh", BuiltinFunction::MathCosh),
                ("asin", BuiltinFunction::MathAsin),
                ("atan", BuiltinFunction::MathAtan),
                ("acos", BuiltinFunction::MathAcos),
                ("floor", BuiltinFunction::MathFloor),
                ("ceil", BuiltinFunction::MathCeil),
                ("trunc", BuiltinFunction::MathTrunc),
                ("isfinite", BuiltinFunction::MathIsFinite),
                ("isinf", BuiltinFunction::MathIsInf),
                ("isnan", BuiltinFunction::MathIsNaN),
                ("isclose", BuiltinFunction::MathIsClose),
                ("factorial", BuiltinFunction::MathFactorial),
                ("gcd", BuiltinFunction::MathGcd),
            ],
            vec![
                ("pi", Value::Float(std::f64::consts::PI)),
                ("e", Value::Float(std::f64::consts::E)),
                ("tau", Value::Float(std::f64::consts::TAU)),
                ("inf", Value::Float(f64::INFINITY)),
                ("nan", Value::Float(f64::NAN)),
            ],
        );
        let decimal_class = self
            .heap
            .alloc_class(ClassObject::new("Decimal".to_string(), Vec::new()));
        let decimal_context_class = self
            .heap
            .alloc_class(ClassObject::new("Context".to_string(), Vec::new()));
        if let Value::Class(class) = &decimal_context_class
            && let Object::Class(class_data) = &mut *class.kind_mut()
        {
            class_data.attrs.insert(
                "__enter__".to_string(),
                Value::Builtin(BuiltinFunction::DecimalContextEnter),
            );
            class_data.attrs.insert(
                "__exit__".to_string(),
                Value::Builtin(BuiltinFunction::DecimalContextExit),
            );
        }
        let decimal_default_context = match &decimal_context_class {
            Value::Class(class) => self.heap.alloc_instance(InstanceObject::new(class.clone())),
            _ => Value::None,
        };
        self.install_builtin_module(
            "decimal",
            &[
                ("getcontext", BuiltinFunction::DecimalGetContext),
                ("setcontext", BuiltinFunction::DecimalSetContext),
                ("localcontext", BuiltinFunction::DecimalLocalContext),
            ],
            vec![
                ("Decimal", decimal_class),
                ("Context", decimal_context_class),
                ("ROUND_HALF_EVEN", Value::Str("ROUND_HALF_EVEN".to_string())),
                ("_context", decimal_default_context),
            ],
        );
        self.install_builtin_module(
            "_pylong",
            &[
                (
                    "int_to_decimal_string",
                    BuiltinFunction::PyLongIntToDecimalString,
                ),
                ("int_divmod", BuiltinFunction::PyLongIntDivMod),
                ("int_from_string", BuiltinFunction::PyLongIntFromString),
                ("compute_powers", BuiltinFunction::PyLongComputePowers),
                (
                    "_dec_str_to_int_inner",
                    BuiltinFunction::PyLongDecStrToIntInner,
                ),
            ],
            vec![
                ("_spread", self.heap.alloc_dict(Vec::new())),
                ("_LOG_10_BASE_256", Value::Int(1)),
                ("_DIV_LIMIT", Value::Int(1)),
                ("_DIV_LIMIT_MAX", Value::Int(1)),
            ],
        );
        let time_struct_time_class = self.alloc_tuple_backed_builtin_class("struct_time");
        self.install_builtin_module(
            "time",
            &[
                ("time", BuiltinFunction::TimeTime),
                ("time_ns", BuiltinFunction::TimeTimeNs),
                ("localtime", BuiltinFunction::TimeLocalTime),
                ("gmtime", BuiltinFunction::TimeGmTime),
                ("strftime", BuiltinFunction::TimeStrFTime),
                ("monotonic", BuiltinFunction::TimeMonotonic),
                ("perf_counter", BuiltinFunction::TimeMonotonic),
                ("perf_counter_ns", BuiltinFunction::TimeTimeNs),
                ("sleep", BuiltinFunction::TimeSleep),
            ],
            vec![
                ("struct_time", time_struct_time_class),
                ("timezone", Value::Int(0)),
                ("altzone", Value::Int(0)),
                ("daylight", Value::Int(0)),
                (
                    "tzname",
                    self.heap.alloc_tuple(vec![
                        Value::Str("UTC".to_string()),
                        Value::Str("UTC".to_string()),
                    ]),
                ),
            ],
        );
        self.install_builtin_module(
            "platform",
            &[
                ("system", BuiltinFunction::SysGetFilesystemEncoding),
                ("release", BuiltinFunction::SysGetFilesystemEncoding),
                ("version", BuiltinFunction::SysGetFilesystemEncoding),
                ("machine", BuiltinFunction::SysGetFilesystemEncoding),
                ("processor", BuiltinFunction::SysGetFilesystemEncoding),
                ("node", BuiltinFunction::SysGetFilesystemEncoding),
                ("platform", BuiltinFunction::SysGetFilesystemEncoding),
                ("python_version", BuiltinFunction::SysGetFilesystemEncoding),
                (
                    "python_implementation",
                    BuiltinFunction::SysGetFilesystemEncoding,
                ),
                ("libc_ver", BuiltinFunction::PlatformLibcVer),
                ("win32_is_iot", BuiltinFunction::PlatformWin32IsIot),
                ("uname", BuiltinFunction::Tuple),
            ],
            Vec::new(),
        );
        let os_stat_result_class = self.alloc_tuple_backed_builtin_class("stat_result");
        let os_terminal_size_class = self.alloc_tuple_backed_builtin_class("terminal_size");
        if let Value::Class(class_obj) = &os_terminal_size_class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("os".to_string()));
            class_data.attrs.insert(
                "__new__".to_string(),
                Value::Builtin(BuiltinFunction::OsTerminalSize),
            );
            class_data.attrs.insert(
                "_fields".to_string(),
                self.heap.alloc_tuple(vec![
                    Value::Str("columns".to_string()),
                    Value::Str("lines".to_string()),
                ]),
            );
            class_data
                .attrs
                .insert("n_fields".to_string(), Value::Int(2));
            class_data
                .attrs
                .insert("n_sequence_fields".to_string(), Value::Int(2));
            class_data
                .attrs
                .insert("n_unnamed_fields".to_string(), Value::Int(0));
        }
        self.install_builtin_module(
            "os",
            &[
                ("getpid", BuiltinFunction::OsGetPid),
                ("getcwd", BuiltinFunction::OsGetCwd),
                ("uname", BuiltinFunction::OsUname),
                ("getenv", BuiltinFunction::OsGetEnv),
                ("putenv", BuiltinFunction::OsPutEnv),
                ("unsetenv", BuiltinFunction::OsUnsetEnv),
                ("get_terminal_size", BuiltinFunction::OsGetTerminalSize),
                ("open", BuiltinFunction::OsOpen),
                ("pipe", BuiltinFunction::OsPipe),
                ("read", BuiltinFunction::OsRead),
                ("readinto", BuiltinFunction::OsReadInto),
                ("write", BuiltinFunction::OsWrite),
                ("dup", BuiltinFunction::OsDup),
                ("lseek", BuiltinFunction::OsLSeek),
                ("ftruncate", BuiltinFunction::OsFTruncate),
                ("close", BuiltinFunction::OsClose),
                ("kill", BuiltinFunction::OsKill),
                ("isatty", BuiltinFunction::OsIsATty),
                ("set_inheritable", BuiltinFunction::OsSetInheritable),
                ("get_inheritable", BuiltinFunction::OsGetInheritable),
                ("urandom", BuiltinFunction::OsURandom),
                ("stat", BuiltinFunction::OsStat),
                ("fstat", BuiltinFunction::OsStat),
                ("lstat", BuiltinFunction::OsLStat),
                ("mkdir", BuiltinFunction::OsMkdir),
                ("chmod", BuiltinFunction::OsChmod),
                ("rmdir", BuiltinFunction::OsRmdir),
                ("utime", BuiltinFunction::OsUTime),
                ("scandir", BuiltinFunction::OsScandir),
                ("walk", BuiltinFunction::OsWalk),
                ("listdir", BuiltinFunction::OsListDir),
                ("access", BuiltinFunction::OsAccess),
                ("fsencode", BuiltinFunction::OsFsEncode),
                ("fsdecode", BuiltinFunction::OsFsDecode),
                (
                    "waitstatus_to_exitcode",
                    BuiltinFunction::OsWaitStatusToExitCode,
                ),
                ("waitpid", BuiltinFunction::OsWaitPid),
                ("path_exists", BuiltinFunction::OsPathExists),
                ("path_join", BuiltinFunction::OsPathJoin),
                ("_get_exports_list", BuiltinFunction::Dir),
                ("fspath", BuiltinFunction::OsFspath),
                ("unlink", BuiltinFunction::OsRemove),
                ("remove", BuiltinFunction::OsRemove),
                ("popen", BuiltinFunction::OsPopen),
            ],
            vec![
                ("sep", Value::Str(std::path::MAIN_SEPARATOR.to_string())),
                (
                    "pathsep",
                    Value::Str(if cfg!(windows) { ";" } else { ":" }.to_string()),
                ),
                (
                    "altsep",
                    if cfg!(windows) {
                        Value::Str("/".to_string())
                    } else {
                        Value::None
                    },
                ),
                ("curdir", Value::Str(".".to_string())),
                ("pardir", Value::Str("..".to_string())),
                ("extsep", Value::Str(".".to_string())),
                (
                    "linesep",
                    Value::Str(if cfg!(windows) { "\r\n" } else { "\n" }.to_string()),
                ),
                (
                    "defpath",
                    Value::Str(
                        if cfg!(windows) {
                            ".;C:\\\\"
                        } else {
                            "/bin:/usr/bin"
                        }
                        .to_string(),
                    ),
                ),
                (
                    "devnull",
                    Value::Str(if cfg!(windows) { "NUL" } else { "/dev/null" }.to_string()),
                ),
                (
                    "name",
                    Value::Str(if cfg!(windows) { "nt" } else { "posix" }.to_string()),
                ),
                ("_walk_symlinks_as_files", Value::Bool(false)),
                ("supports_bytes_environ", Value::Bool(!cfg!(windows))),
                (
                    "PathLike",
                    self.heap
                        .alloc_class(ClassObject::new("PathLike".to_string(), Vec::new())),
                ),
                (
                    "environ",
                    self.heap.alloc_dict(
                        std::env::vars()
                            .map(|(name, value)| (Value::Str(name), Value::Str(value)))
                            .collect::<Vec<_>>(),
                    ),
                ),
                ("stat_result", os_stat_result_class),
                ("O_RDONLY", Value::Int(0)),
                ("O_WRONLY", Value::Int(1)),
                ("O_RDWR", Value::Int(2)),
                ("O_CREAT", Value::Int(64)),
                ("O_EXCL", Value::Int(128)),
                ("O_TRUNC", Value::Int(512)),
                ("O_APPEND", Value::Int(1024)),
                ("O_CLOEXEC", Value::Int(0)),
                ("O_DIRECTORY", Value::Int(0)),
                ("F_OK", Value::Int(0)),
                ("R_OK", Value::Int(4)),
                ("W_OK", Value::Int(2)),
                ("X_OK", Value::Int(1)),
                ("SEEK_SET", Value::Int(0)),
                ("SEEK_CUR", Value::Int(1)),
                ("SEEK_END", Value::Int(2)),
                ("terminal_size", os_terminal_size_class),
                ("supports_dir_fd", self.heap.alloc_set(Vec::new())),
                ("supports_fd", self.heap.alloc_set(Vec::new())),
                ("supports_follow_symlinks", self.heap.alloc_set(Vec::new())),
                ("WNOHANG", Value::Int(1)),
                ("WIFSTOPPED", Value::Builtin(BuiltinFunction::OsWIfStopped)),
                ("WSTOPSIG", Value::Builtin(BuiltinFunction::OsWStopSig)),
                (
                    "WIFSIGNALED",
                    Value::Builtin(BuiltinFunction::OsWIfSignaled),
                ),
                ("WTERMSIG", Value::Builtin(BuiltinFunction::OsWTermSig)),
                ("WIFEXITED", Value::Builtin(BuiltinFunction::OsWIfExited)),
                (
                    "WEXITSTATUS",
                    Value::Builtin(BuiltinFunction::OsWExitStatus),
                ),
            ],
        );
        let posix_stat_result_class = self.alloc_tuple_backed_builtin_class("stat_result");
        self.install_builtin_module(
            "posix",
            &[
                ("getpid", BuiltinFunction::OsGetPid),
                ("getcwd", BuiltinFunction::OsGetCwd),
                ("uname", BuiltinFunction::OsUname),
                ("getenv", BuiltinFunction::OsGetEnv),
                ("putenv", BuiltinFunction::OsPutEnv),
                ("unsetenv", BuiltinFunction::OsUnsetEnv),
                ("open", BuiltinFunction::OsOpen),
                ("pipe", BuiltinFunction::OsPipe),
                ("read", BuiltinFunction::OsRead),
                ("readinto", BuiltinFunction::OsReadInto),
                ("write", BuiltinFunction::OsWrite),
                ("dup", BuiltinFunction::OsDup),
                ("lseek", BuiltinFunction::OsLSeek),
                ("ftruncate", BuiltinFunction::OsFTruncate),
                ("close", BuiltinFunction::OsClose),
                ("kill", BuiltinFunction::OsKill),
                ("isatty", BuiltinFunction::OsIsATty),
                ("set_inheritable", BuiltinFunction::OsSetInheritable),
                ("get_inheritable", BuiltinFunction::OsGetInheritable),
                ("urandom", BuiltinFunction::OsURandom),
                ("listdir", BuiltinFunction::OsListDir),
                ("access", BuiltinFunction::OsAccess),
                (
                    "waitstatus_to_exitcode",
                    BuiltinFunction::OsWaitStatusToExitCode,
                ),
                ("waitpid", BuiltinFunction::OsWaitPid),
                ("stat", BuiltinFunction::OsStat),
                ("lstat", BuiltinFunction::OsLStat),
                ("mkdir", BuiltinFunction::OsMkdir),
                ("chmod", BuiltinFunction::OsChmod),
                ("rmdir", BuiltinFunction::OsRmdir),
                ("utime", BuiltinFunction::OsUTime),
                ("scandir", BuiltinFunction::OsScandir),
                ("_path_normpath", BuiltinFunction::OsPathNormPath),
                ("_path_splitroot_ex", BuiltinFunction::OsPathSplitRootEx),
            ],
            vec![
                ("sep", Value::Str("/".to_string())),
                ("pathsep", Value::Str(":".to_string())),
                ("altsep", Value::None),
                ("environ", self.heap.alloc_dict(Vec::new())),
                ("WNOHANG", Value::Int(1)),
                ("WIFSTOPPED", Value::Builtin(BuiltinFunction::OsWIfStopped)),
                ("WSTOPSIG", Value::Builtin(BuiltinFunction::OsWStopSig)),
                (
                    "WIFSIGNALED",
                    Value::Builtin(BuiltinFunction::OsWIfSignaled),
                ),
                ("WTERMSIG", Value::Builtin(BuiltinFunction::OsWTermSig)),
                ("WIFEXITED", Value::Builtin(BuiltinFunction::OsWIfExited)),
                (
                    "WEXITSTATUS",
                    Value::Builtin(BuiltinFunction::OsWExitStatus),
                ),
                ("stat_result", posix_stat_result_class),
            ],
        );
        let prefix = self
            .sys_str_value("prefix")
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| ".".to_string());
        let exec_prefix = self
            .sys_str_value("exec_prefix")
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| prefix.clone());
        let base_prefix = self
            .sys_str_value("base_prefix")
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| prefix.clone());
        let base_exec_prefix = self
            .sys_str_value("base_exec_prefix")
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| exec_prefix.clone());
        let cc = std::env::var("CC").unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "cl".to_string()
            } else {
                "cc".to_string()
            }
        });
        let cflags = std::env::var("CFLAGS").unwrap_or_default();
        let cppflags = std::env::var("CPPFLAGS").unwrap_or_default();
        let ldflags = std::env::var("LDFLAGS").unwrap_or_default();
        let ldshared = std::env::var("LDSHARED").unwrap_or_else(|_| {
            if cfg!(target_os = "macos") {
                format!("{cc} -bundle -undefined dynamic_lookup")
            } else if cfg!(target_os = "windows") {
                cc.clone()
            } else {
                format!("{cc} -shared")
            }
        });
        let bldshared = std::env::var("BLDSHARED").unwrap_or_else(|_| ldshared.clone());
        let ar = std::env::var("AR").unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "lib".to_string()
            } else {
                "ar".to_string()
            }
        });
        let arflags = std::env::var("ARFLAGS").unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "/NOLOGO".to_string()
            } else {
                "rcs".to_string()
            }
        });
        let ccshared = std::env::var("CCSHARED").unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "/LD".to_string()
            } else {
                "-fPIC".to_string()
            }
        });
        let shlib_suffix = if cfg!(target_os = "windows") {
            ".pyd"
        } else {
            ".so"
        };
        let soabi = format!("pyrs-314-{platform}");
        let ext_suffix = format!(".{soabi}{shlib_suffix}");
        let ldversion = "3.14".to_string();
        let library = if cfg!(target_os = "windows") {
            "python314.lib".to_string()
        } else {
            "libpython3.14.a".to_string()
        };
        let includepy = if cfg!(target_os = "windows") {
            format!("{prefix}\\include")
        } else {
            format!("{prefix}/include")
        };
        let libdir = if cfg!(target_os = "windows") {
            format!("{prefix}\\libs")
        } else {
            format!("{prefix}/lib")
        };
        let build_time_vars = vec![
            (Value::Str("prefix".to_string()), Value::Str(prefix.clone())),
            (
                Value::Str("exec_prefix".to_string()),
                Value::Str(exec_prefix.clone()),
            ),
            (
                Value::Str("host_prefix".to_string()),
                Value::Str(prefix.clone()),
            ),
            (
                Value::Str("host_exec_prefix".to_string()),
                Value::Str(exec_prefix.clone()),
            ),
            (
                Value::Str("installed_base".to_string()),
                Value::Str(base_prefix.clone()),
            ),
            (
                Value::Str("installed_platbase".to_string()),
                Value::Str(base_exec_prefix.clone()),
            ),
            (Value::Str("base".to_string()), Value::Str(prefix.clone())),
            (
                Value::Str("platbase".to_string()),
                Value::Str(exec_prefix.clone()),
            ),
            (
                Value::Str("ABIFLAGS".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("MULTIARCH".to_string()),
                Value::Str(String::new()),
            ),
            (Value::Str("SOABI".to_string()), Value::Str(soabi.clone())),
            (
                Value::Str("EXT_SUFFIX".to_string()),
                Value::Str(ext_suffix.clone()),
            ),
            (Value::Str("SO".to_string()), Value::Str(ext_suffix)),
            (
                Value::Str("SHLIB_SUFFIX".to_string()),
                Value::Str(shlib_suffix.to_string()),
            ),
            (Value::Str("CC".to_string()), Value::Str(cc.clone())),
            (Value::Str("AR".to_string()), Value::Str(ar)),
            (Value::Str("ARFLAGS".to_string()), Value::Str(arflags)),
            (Value::Str("CCSHARED".to_string()), Value::Str(ccshared)),
            (Value::Str("LDSHARED".to_string()), Value::Str(ldshared)),
            (Value::Str("BLDSHARED".to_string()), Value::Str(bldshared)),
            (Value::Str("CFLAGS".to_string()), Value::Str(cflags)),
            (Value::Str("CPPFLAGS".to_string()), Value::Str(cppflags)),
            (Value::Str("LDFLAGS".to_string()), Value::Str(ldflags)),
            (Value::Str("LIBRARY".to_string()), Value::Str(library)),
            (Value::Str("LDVERSION".to_string()), Value::Str(ldversion)),
            (
                Value::Str("LDLIBRARY".to_string()),
                Value::Str(String::new()),
            ),
            (Value::Str("LIBDIR".to_string()), Value::Str(libdir)),
            (
                Value::Str("LIBPL".to_string()),
                Value::Str(if cfg!(target_os = "windows") {
                    format!("{prefix}\\libs")
                } else {
                    format!("{prefix}/lib")
                }),
            ),
            (
                Value::Str("INCLUDEDIR".to_string()),
                Value::Str(if cfg!(target_os = "windows") {
                    format!("{prefix}\\include")
                } else {
                    format!("{prefix}/include")
                }),
            ),
            (
                Value::Str("INCLUDEPY".to_string()),
                Value::Str(includepy.clone()),
            ),
            (
                Value::Str("CONFINCLUDEPY".to_string()),
                Value::Str(includepy),
            ),
            (Value::Str("Py_GIL_DISABLED".to_string()), Value::Int(0)),
            (Value::Str("Py_DEBUG".to_string()), Value::Int(0)),
            (Value::Str("Py_ENABLE_SHARED".to_string()), Value::Int(1)),
            (
                Value::Str("VERSION".to_string()),
                Value::Str("314".to_string()),
            ),
            (
                Value::Str("py_version".to_string()),
                Value::Str("3.14".to_string()),
            ),
            (
                Value::Str("py_version_short".to_string()),
                Value::Str("3.14".to_string()),
            ),
            (
                Value::Str("py_version_nodot".to_string()),
                Value::Str("314".to_string()),
            ),
        ];
        let sysconfigdata_name = format!("_sysconfigdata__{platform}_");
        self.install_builtin_module(
            &sysconfigdata_name,
            &[],
            vec![(
                "build_time_vars",
                self.heap.alloc_dict(build_time_vars.clone()),
            )],
        );
        let legacy_sysconfigdata_name = format!("_sysconfigdata__{platform}");
        self.install_builtin_module(
            &legacy_sysconfigdata_name,
            &[],
            vec![("build_time_vars", self.heap.alloc_dict(build_time_vars))],
        );
        let pathlib_path_class = match self
            .heap
            .alloc_class(ClassObject::new("Path".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *pathlib_path_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::PathlibPathInit),
            );
            class_data.attrs.insert(
                "joinpath".to_string(),
                Value::Builtin(BuiltinFunction::PathlibPathJoinPath),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::PathlibPathStr),
            );
            class_data.attrs.insert(
                "__fspath__".to_string(),
                Value::Builtin(BuiltinFunction::PathlibPathStr),
            );
        }
        self.install_builtin_module(
            "pathlib",
            &[
                ("joinpath", BuiltinFunction::OsPathJoin),
                ("exists", BuiltinFunction::OsPathExists),
            ],
            vec![("Path", Value::Class(pathlib_path_class))],
        );
        self.install_builtin_module(
            "os.path",
            &[
                ("join", BuiltinFunction::OsPathJoin),
                ("exists", BuiltinFunction::OsPathExists),
                ("lexists", BuiltinFunction::OsPathExists),
                ("normpath", BuiltinFunction::OsPathNormPath),
                ("normcase", BuiltinFunction::OsPathNormCase),
                ("splitdrive", BuiltinFunction::OsPathSplitDrive),
                ("abspath", BuiltinFunction::OsPathAbsPath),
                ("expanduser", BuiltinFunction::OsPathExpandUser),
                ("realpath", BuiltinFunction::OsPathRealPath),
                ("relpath", BuiltinFunction::OsPathRelPath),
                ("dirname", BuiltinFunction::OsPathDirName),
                ("basename", BuiltinFunction::OsPathBaseName),
                ("split", BuiltinFunction::OsPathSplit),
                ("isabs", BuiltinFunction::OsPathIsAbs),
                ("isdir", BuiltinFunction::OsPathIsDir),
                ("isfile", BuiltinFunction::OsPathIsFile),
                ("islink", BuiltinFunction::OsPathIsLink),
                ("isjunction", BuiltinFunction::OsPathIsJunction),
                ("splitext", BuiltinFunction::OsPathSplitExt),
                ("commonprefix", BuiltinFunction::OsPathCommonPrefix),
            ],
            vec![
                ("sep", Value::Str("/".to_string())),
                ("pathsep", Value::Str(":".to_string())),
            ],
        );
        self.install_builtin_module(
            "_osx_support",
            &[("customize_config_vars", BuiltinFunction::TypingIdFunc)],
            Vec::new(),
        );
        self.install_builtin_module(
            "select",
            &[("select", BuiltinFunction::SelectSelect)],
            vec![
                ("POLLIN", Value::Int(1)),
                ("POLLOUT", Value::Int(4)),
                ("POLLERR", Value::Int(8)),
                ("POLLHUP", Value::Int(16)),
                ("POLLNVAL", Value::Int(32)),
            ],
        );
        if let (Some(os_module), Some(os_path_module)) = (
            self.modules.get("os").cloned(),
            self.modules.get("os.path").cloned(),
        ) && let Object::Module(module_data) = &mut *os_module.kind_mut()
        {
            module_data
                .globals
                .insert("path".to_string(), Value::Module(os_path_module));
        }
        self.install_builtin_module(
            "json",
            &[
                ("dumps", BuiltinFunction::JsonDumps),
                ("loads", BuiltinFunction::JsonLoads),
            ],
            vec![(
                "JSONDecodeError",
                Value::ExceptionType("ValueError".to_string()),
            )],
        );
        let zlib_compress_type = match self
            .heap
            .alloc_class(ClassObject::new("Compress".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *zlib_compress_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("zlib".to_string()));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
            class_data.attrs.insert(
                "compress".to_string(),
                Value::Builtin(BuiltinFunction::ZlibCompressObjectCompress),
            );
            class_data.attrs.insert(
                "flush".to_string(),
                Value::Builtin(BuiltinFunction::ZlibCompressObjectFlush),
            );
        }
        let zlib_decompress_type = match self
            .heap
            .alloc_class(ClassObject::new("Decompress".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *zlib_decompress_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("zlib".to_string()));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
            class_data.attrs.insert(
                "decompress".to_string(),
                Value::Builtin(BuiltinFunction::ZlibDecompressObjectDecompress),
            );
            class_data.attrs.insert(
                "flush".to_string(),
                Value::Builtin(BuiltinFunction::ZlibDecompressObjectFlush),
            );
        }
        self.install_builtin_module(
            "zlib",
            &[
                ("compress", BuiltinFunction::ZlibCompress),
                ("decompress", BuiltinFunction::ZlibDecompress),
                ("compressobj", BuiltinFunction::ZlibCompressObj),
                ("decompressobj", BuiltinFunction::ZlibDecompressObj),
                ("crc32", BuiltinFunction::ZlibCrc32),
            ],
            vec![
                ("Compress", Value::Class(zlib_compress_type)),
                ("Decompress", Value::Class(zlib_decompress_type)),
                ("error", Value::ExceptionType("Exception".to_string())),
                (
                    "ZLIB_VERSION",
                    Value::Str(
                        self.zlib_version_string()
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                ),
                ("MAX_WBITS", Value::Int(15)),
                ("DEFLATED", Value::Int(8)),
                ("DEF_MEM_LEVEL", Value::Int(8)),
                ("Z_NO_FLUSH", Value::Int(0)),
                ("Z_SYNC_FLUSH", Value::Int(2)),
                ("Z_FINISH", Value::Int(4)),
                ("Z_DEFAULT_COMPRESSION", Value::Int(-1)),
                ("Z_BEST_SPEED", Value::Int(1)),
                ("Z_BEST_COMPRESSION", Value::Int(9)),
                ("Z_DEFAULT_STRATEGY", Value::Int(0)),
            ],
        );
        let bz2_compressor_type = match self
            .heap
            .alloc_class(ClassObject::new("BZ2Compressor".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *bz2_compressor_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_bz2".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::Bz2CompressorInit),
            );
            class_data.attrs.insert(
                "compress".to_string(),
                Value::Builtin(BuiltinFunction::Bz2CompressorCompress),
            );
            class_data.attrs.insert(
                "flush".to_string(),
                Value::Builtin(BuiltinFunction::Bz2CompressorFlush),
            );
        }
        let bz2_decompressor_type = match self
            .heap
            .alloc_class(ClassObject::new("BZ2Decompressor".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *bz2_decompressor_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_bz2".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::Bz2DecompressorInit),
            );
            class_data.attrs.insert(
                "decompress".to_string(),
                Value::Builtin(BuiltinFunction::Bz2DecompressorDecompress),
            );
        }
        self.install_builtin_module(
            "_bz2",
            &[],
            vec![
                ("BZ2Compressor", Value::Class(bz2_compressor_type)),
                ("BZ2Decompressor", Value::Class(bz2_decompressor_type)),
            ],
        );
        let lzma_compressor_type = match self
            .heap
            .alloc_class(ClassObject::new("LZMACompressor".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *lzma_compressor_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_lzma".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::LzmaCompressorInit),
            );
            class_data.attrs.insert(
                "compress".to_string(),
                Value::Builtin(BuiltinFunction::LzmaCompressorCompress),
            );
            class_data.attrs.insert(
                "flush".to_string(),
                Value::Builtin(BuiltinFunction::LzmaCompressorFlush),
            );
        }
        let lzma_decompressor_type = match self
            .heap
            .alloc_class(ClassObject::new("LZMADecompressor".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *lzma_decompressor_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_lzma".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::LzmaDecompressorInit),
            );
            class_data.attrs.insert(
                "decompress".to_string(),
                Value::Builtin(BuiltinFunction::LzmaDecompressorDecompress),
            );
        }
        let mut lzma_values = vec![
            ("LZMACompressor", Value::Class(lzma_compressor_type)),
            ("LZMADecompressor", Value::Class(lzma_decompressor_type)),
            ("LZMAError", Value::ExceptionType("Exception".to_string())),
        ];
        lzma_values.extend(Self::lzma_constants());
        self.install_builtin_module(
            "_lzma",
            &[
                ("is_check_supported", BuiltinFunction::LzmaIsCheckSupported),
                (
                    "_encode_filter_properties",
                    BuiltinFunction::LzmaEncodeFilterProperties,
                ),
                (
                    "_decode_filter_properties",
                    BuiltinFunction::LzmaDecodeFilterProperties,
                ),
            ],
            lzma_values,
        );
        let ssl_context_type = match self
            .heap
            .alloc_class(ClassObject::new("_SSLContext".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *ssl_context_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_ssl".to_string()));
            class_data.attrs.insert(
                "__new__".to_string(),
                Value::Builtin(BuiltinFunction::SslContextNew),
            );
        }
        let ssl_memory_bio_type = match self
            .heap
            .alloc_class(ClassObject::new("MemoryBIO".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *ssl_memory_bio_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_ssl".to_string()));
        }
        let ssl_session_type = match self
            .heap
            .alloc_class(ClassObject::new("SSLSession".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *ssl_session_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_ssl".to_string()));
        }
        let mut ssl_values = vec![
            ("_SSLContext", Value::Class(ssl_context_type)),
            ("MemoryBIO", Value::Class(ssl_memory_bio_type)),
            ("SSLSession", Value::Class(ssl_session_type)),
            ("SSLError", Value::ExceptionType("SSLError".to_string())),
            (
                "SSLZeroReturnError",
                Value::ExceptionType("SSLZeroReturnError".to_string()),
            ),
            (
                "SSLWantReadError",
                Value::ExceptionType("SSLWantReadError".to_string()),
            ),
            (
                "SSLWantWriteError",
                Value::ExceptionType("SSLWantWriteError".to_string()),
            ),
            (
                "SSLSyscallError",
                Value::ExceptionType("SSLSyscallError".to_string()),
            ),
            (
                "SSLEOFError",
                Value::ExceptionType("SSLEOFError".to_string()),
            ),
            (
                "SSLCertVerificationError",
                Value::ExceptionType("SSLCertVerificationError".to_string()),
            ),
        ];
        ssl_values.extend(self.ssl_module_constants());
        self.install_builtin_module(
            "_ssl",
            &[
                ("txt2obj", BuiltinFunction::SslTxt2Obj),
                ("nid2obj", BuiltinFunction::SslNid2Obj),
                ("RAND_status", BuiltinFunction::SslRandStatus),
                ("RAND_add", BuiltinFunction::SslRandAdd),
                ("RAND_bytes", BuiltinFunction::SslRandBytes),
                ("RAND_egd", BuiltinFunction::SslRandEgd),
            ],
            ssl_values,
        );
        let ssl_public_context_type = match self
            .heap
            .alloc_class(ClassObject::new("SSLContext".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *ssl_public_context_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("ssl".to_string()));
            class_data.attrs.insert(
                "__new__".to_string(),
                Value::Builtin(BuiltinFunction::SslContextNew),
            );
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::SslContextInit),
            );
        }
        let ssl_socket_type = match self
            .heap
            .alloc_class(ClassObject::new("SSLSocket".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *ssl_socket_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("ssl".to_string()));
        }
        let ssl_purpose_type = match self
            .heap
            .alloc_class(ClassObject::new("Purpose".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *ssl_purpose_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("ssl".to_string()));
            class_data.attrs.insert(
                "SERVER_AUTH".to_string(),
                Value::Str("1.3.6.1.5.5.7.3.1".to_string()),
            );
            class_data.attrs.insert(
                "CLIENT_AUTH".to_string(),
                Value::Str("1.3.6.1.5.5.7.3.2".to_string()),
            );
        }
        self.install_builtin_module(
            "ssl",
            &[
                (
                    "create_default_context",
                    BuiltinFunction::SslCreateDefaultContext,
                ),
                (
                    "_create_stdlib_context",
                    BuiltinFunction::SslCreateDefaultContext,
                ),
            ],
            vec![
                ("SSLContext", Value::Class(ssl_public_context_type)),
                ("SSLSocket", Value::Class(ssl_socket_type)),
                ("Purpose", Value::Class(ssl_purpose_type)),
                ("PROTOCOL_TLS", Value::Int(2)),
                ("PROTOCOL_TLS_CLIENT", Value::Int(16)),
                ("PROTOCOL_TLS_SERVER", Value::Int(17)),
                ("CERT_NONE", Value::Int(0)),
                ("CERT_OPTIONAL", Value::Int(1)),
                ("CERT_REQUIRED", Value::Int(2)),
                ("VERIFY_DEFAULT", Value::Int(0)),
                ("VERIFY_X509_STRICT", Value::Int(32)),
                ("VERIFY_X509_PARTIAL_CHAIN", Value::Int(0x80000)),
                ("SSLError", Value::ExceptionType("SSLError".to_string())),
                (
                    "SSLZeroReturnError",
                    Value::ExceptionType("SSLZeroReturnError".to_string()),
                ),
                (
                    "SSLWantReadError",
                    Value::ExceptionType("SSLWantReadError".to_string()),
                ),
                (
                    "SSLWantWriteError",
                    Value::ExceptionType("SSLWantWriteError".to_string()),
                ),
                (
                    "SSLSyscallError",
                    Value::ExceptionType("SSLSyscallError".to_string()),
                ),
                (
                    "SSLEOFError",
                    Value::ExceptionType("SSLEOFError".to_string()),
                ),
                (
                    "SSLCertVerificationError",
                    Value::ExceptionType("SSLCertVerificationError".to_string()),
                ),
            ],
        );
        let pyexpat_parser_type = match self
            .heap
            .alloc_class(ClassObject::new("xmlparser".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *pyexpat_parser_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("pyexpat".to_string()));
            class_data.attrs.insert(
                "Parse".to_string(),
                Value::Builtin(BuiltinFunction::PyExpatParserParse),
            );
            class_data.attrs.insert(
                "GetReparseDeferralEnabled".to_string(),
                Value::Builtin(BuiltinFunction::PyExpatParserGetReparseDeferralEnabled),
            );
            class_data.attrs.insert(
                "SetReparseDeferralEnabled".to_string(),
                Value::Builtin(BuiltinFunction::PyExpatParserSetReparseDeferralEnabled),
            );
        }
        let pyexpat_model_module = match self.heap.alloc_module(ModuleObject::new("pyexpat.model"))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &pyexpat_model_module,
            "pyexpat.model",
            None,
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        self.register_module("pyexpat.model", pyexpat_model_module.clone());
        let pyexpat_errors_module =
            match self.heap.alloc_module(ModuleObject::new("pyexpat.errors")) {
                Value::Module(module) => module,
                _ => unreachable!(),
            };
        self.set_module_metadata(
            &pyexpat_errors_module,
            "pyexpat.errors",
            None,
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        self.register_module("pyexpat.errors", pyexpat_errors_module.clone());
        self.install_builtin_module(
            "pyexpat",
            &[("ParserCreate", BuiltinFunction::PyExpatParserCreate)],
            vec![
                ("xmlparser", Value::Class(pyexpat_parser_type.clone())),
                ("XMLParserType", Value::Class(pyexpat_parser_type)),
                ("ExpatError", Value::ExceptionType("ExpatError".to_string())),
                ("error", Value::ExceptionType("ExpatError".to_string())),
                ("model", Value::Module(pyexpat_model_module)),
                ("errors", Value::Module(pyexpat_errors_module)),
                (
                    "version_info",
                    self.heap
                        .alloc_tuple(vec![Value::Int(2), Value::Int(6), Value::Int(0)]),
                ),
            ],
        );
        self.install_builtin_module(
            "_json",
            &[
                ("encode_basestring", BuiltinFunction::JsonEncodeBaseString),
                (
                    "encode_basestring_ascii",
                    BuiltinFunction::JsonEncodeBaseStringAscii,
                ),
                ("make_encoder", BuiltinFunction::JsonMakeEncoder),
                ("make_scanner", BuiltinFunction::JsonScannerMakeScanner),
                ("scanstring", BuiltinFunction::JsonDecoderScanString),
            ],
            Vec::new(),
        );
        let build_hash_type = |module_name: &str, class_name: &str| {
            let class = match self
                .heap
                .alloc_class(ClassObject::new(class_name.to_string(), Vec::new()))
            {
                Value::Class(class) => class,
                _ => unreachable!(),
            };
            if let Object::Class(class_data) = &mut *class.kind_mut() {
                class_data.attrs.insert(
                    "__module__".to_string(),
                    Value::Str(module_name.to_string()),
                );
                class_data
                    .attrs
                    .insert("__flags__".to_string(), Value::Int(0));
                class_data.attrs.insert(
                    "__pyrs_disallow_instantiation__".to_string(),
                    Value::Bool(true),
                );
                class_data.attrs.insert(
                    "update".to_string(),
                    Value::Builtin(BuiltinFunction::HashlibHashUpdate),
                );
                class_data.attrs.insert(
                    "digest".to_string(),
                    Value::Builtin(BuiltinFunction::HashlibHashDigest),
                );
                class_data.attrs.insert(
                    "hexdigest".to_string(),
                    Value::Builtin(BuiltinFunction::HashlibHashHexDigest),
                );
                class_data.attrs.insert(
                    "copy".to_string(),
                    Value::Builtin(BuiltinFunction::HashlibHashCopy),
                );
            }
            class
        };
        let md5_type = build_hash_type("_md5", "md5");
        let sha1_type = build_hash_type("_sha1", "sha1");
        let sha224_type = build_hash_type("_sha2", "SHA224Type");
        let sha256_type = build_hash_type("_sha2", "SHA256Type");
        let sha384_type = build_hash_type("_sha2", "SHA384Type");
        let sha512_type = build_hash_type("_sha2", "SHA512Type");
        let blake2b_type = build_hash_type("_blake2", "blake2b");
        let blake2s_type = build_hash_type("_blake2", "blake2s");
        let sha3_224_type = build_hash_type("_sha3", "sha3_224");
        let sha3_256_type = build_hash_type("_sha3", "sha3_256");
        let sha3_384_type = build_hash_type("_sha3", "sha3_384");
        let sha3_512_type = build_hash_type("_sha3", "sha3_512");
        let shake128_type = build_hash_type("_sha3", "shake_128");
        let shake256_type = build_hash_type("_sha3", "shake_256");
        let hmac_type = match self
            .heap
            .alloc_class(ClassObject::new("HMAC".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *hmac_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_hashlib".to_string()));
            class_data
                .attrs
                .insert("__flags__".to_string(), Value::Int(0));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
            class_data.attrs.insert(
                "update".to_string(),
                Value::Builtin(BuiltinFunction::HashlibHmacUpdate),
            );
            class_data.attrs.insert(
                "digest".to_string(),
                Value::Builtin(BuiltinFunction::HashlibHmacObjDigest),
            );
            class_data.attrs.insert(
                "hexdigest".to_string(),
                Value::Builtin(BuiltinFunction::HashlibHmacObjHexDigest),
            );
            class_data.attrs.insert(
                "copy".to_string(),
                Value::Builtin(BuiltinFunction::HashlibHmacCopy),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::HashlibHmacRepr),
            );
        }
        self.synthetic_builtin_classes
            .insert("__hashlib_hmac_type__".to_string(), hmac_type.clone());
        self.install_builtin_module(
            "_md5",
            &[("md5", BuiltinFunction::HashlibMd5)],
            vec![
                ("MD5Type", Value::Class(md5_type)),
                ("_GIL_MINSIZE", Value::Int(2048)),
            ],
        );
        self.install_builtin_module(
            "_sha1",
            &[("sha1", BuiltinFunction::HashlibSha1)],
            vec![
                ("SHA1Type", Value::Class(sha1_type)),
                ("_GIL_MINSIZE", Value::Int(2048)),
            ],
        );
        self.install_builtin_module(
            "_sha2",
            &[
                ("sha224", BuiltinFunction::HashlibSha224),
                ("sha256", BuiltinFunction::HashlibSha256),
                ("sha384", BuiltinFunction::HashlibSha384),
                ("sha512", BuiltinFunction::HashlibSha512),
            ],
            vec![
                ("SHA224Type", Value::Class(sha224_type)),
                ("SHA256Type", Value::Class(sha256_type)),
                ("SHA384Type", Value::Class(sha384_type)),
                ("SHA512Type", Value::Class(sha512_type)),
                ("_GIL_MINSIZE", Value::Int(2048)),
            ],
        );
        self.install_builtin_module(
            "_blake2",
            &[
                ("blake2b", BuiltinFunction::HashlibBlake2b),
                ("blake2s", BuiltinFunction::HashlibBlake2s),
            ],
            vec![
                ("_BLAKE2bType", Value::Class(blake2b_type)),
                ("_BLAKE2sType", Value::Class(blake2s_type)),
                ("BLAKE2B_MAX_DIGEST_SIZE", Value::Int(64)),
                ("BLAKE2B_MAX_KEY_SIZE", Value::Int(64)),
                ("BLAKE2B_SALT_SIZE", Value::Int(16)),
                ("BLAKE2B_PERSON_SIZE", Value::Int(16)),
                ("BLAKE2S_MAX_DIGEST_SIZE", Value::Int(32)),
                ("BLAKE2S_MAX_KEY_SIZE", Value::Int(32)),
                ("BLAKE2S_SALT_SIZE", Value::Int(8)),
                ("BLAKE2S_PERSON_SIZE", Value::Int(8)),
                ("_GIL_MINSIZE", Value::Int(2048)),
            ],
        );
        self.builtin_attr_overrides.insert(
            BuiltinFunction::HashlibBlake2b,
            HashMap::from([
                ("SALT_SIZE".to_string(), Value::Int(16)),
                ("PERSON_SIZE".to_string(), Value::Int(16)),
                ("MAX_DIGEST_SIZE".to_string(), Value::Int(64)),
                ("MAX_KEY_SIZE".to_string(), Value::Int(64)),
            ]),
        );
        self.builtin_attr_overrides.insert(
            BuiltinFunction::HashlibBlake2s,
            HashMap::from([
                ("SALT_SIZE".to_string(), Value::Int(8)),
                ("PERSON_SIZE".to_string(), Value::Int(8)),
                ("MAX_DIGEST_SIZE".to_string(), Value::Int(32)),
                ("MAX_KEY_SIZE".to_string(), Value::Int(32)),
            ]),
        );
        self.install_builtin_module(
            "_sha3",
            &[
                ("sha3_224", BuiltinFunction::HashlibSha3_224),
                ("sha3_256", BuiltinFunction::HashlibSha3_256),
                ("sha3_384", BuiltinFunction::HashlibSha3_384),
                ("sha3_512", BuiltinFunction::HashlibSha3_512),
                ("shake_128", BuiltinFunction::HashlibShake128),
                ("shake_256", BuiltinFunction::HashlibShake256),
            ],
            vec![
                ("_SHA3_224Type", Value::Class(sha3_224_type)),
                ("_SHA3_256Type", Value::Class(sha3_256_type)),
                ("_SHA3_384Type", Value::Class(sha3_384_type)),
                ("_SHA3_512Type", Value::Class(sha3_512_type)),
                ("_SHAKE128Type", Value::Class(shake128_type)),
                ("_SHAKE256Type", Value::Class(shake256_type)),
                ("_GIL_MINSIZE", Value::Int(2048)),
            ],
        );
        self.install_builtin_module(
            "_hashlib",
            &[
                ("new", BuiltinFunction::HashlibNew),
                ("pbkdf2_hmac", BuiltinFunction::HashlibPbkdf2Hmac),
                ("scrypt", BuiltinFunction::HashlibScrypt),
                ("hmac_new", BuiltinFunction::HashlibHmacNew),
                ("hmac_digest", BuiltinFunction::HashlibHmacDigest),
                ("compare_digest", BuiltinFunction::OperatorCompareDigest),
                ("openssl_md5", BuiltinFunction::HashlibMd5),
                ("openssl_sha1", BuiltinFunction::HashlibSha1),
                ("openssl_sha224", BuiltinFunction::HashlibSha224),
                ("openssl_sha256", BuiltinFunction::HashlibSha256),
                ("openssl_sha384", BuiltinFunction::HashlibSha384),
                ("openssl_sha512", BuiltinFunction::HashlibSha512),
                ("openssl_blake2b", BuiltinFunction::HashlibBlake2b),
                ("openssl_blake2s", BuiltinFunction::HashlibBlake2s),
                ("openssl_sha3_224", BuiltinFunction::HashlibSha3_224),
                ("openssl_sha3_256", BuiltinFunction::HashlibSha3_256),
                ("openssl_sha3_384", BuiltinFunction::HashlibSha3_384),
                ("openssl_sha3_512", BuiltinFunction::HashlibSha3_512),
                ("openssl_shake_128", BuiltinFunction::HashlibShake128),
                ("openssl_shake_256", BuiltinFunction::HashlibShake256),
            ],
            vec![
                (
                    "openssl_md_meth_names",
                    self.heap.alloc_frozenset(vec![
                        Value::Str("md5".to_string()),
                        Value::Str("sha1".to_string()),
                        Value::Str("sha224".to_string()),
                        Value::Str("sha256".to_string()),
                        Value::Str("sha384".to_string()),
                        Value::Str("sha512".to_string()),
                        Value::Str("blake2b".to_string()),
                        Value::Str("blake2s".to_string()),
                        Value::Str("sha3_224".to_string()),
                        Value::Str("sha3_256".to_string()),
                        Value::Str("sha3_384".to_string()),
                        Value::Str("sha3_512".to_string()),
                        Value::Str("shake_128".to_string()),
                        Value::Str("shake_256".to_string()),
                    ]),
                ),
                (
                    "_constructors",
                    self.heap.alloc_dict(vec![
                        (
                            Value::Builtin(BuiltinFunction::HashlibMd5),
                            Value::Str("md5".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha1),
                            Value::Str("sha1".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha224),
                            Value::Str("sha224".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha256),
                            Value::Str("sha256".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha384),
                            Value::Str("sha384".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha512),
                            Value::Str("sha512".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha3_224),
                            Value::Str("sha3_224".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha3_256),
                            Value::Str("sha3_256".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha3_384),
                            Value::Str("sha3_384".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibSha3_512),
                            Value::Str("sha3_512".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibShake128),
                            Value::Str("shake_128".to_string()),
                        ),
                        (
                            Value::Builtin(BuiltinFunction::HashlibShake256),
                            Value::Str("shake_256".to_string()),
                        ),
                    ]),
                ),
                (
                    "UnsupportedDigestmodError",
                    Value::ExceptionType("UnsupportedDigestmodError".to_string()),
                ),
                ("HMAC", Value::Class(hmac_type)),
                ("_GIL_MINSIZE", Value::Int(2048)),
            ],
        );
        self.exception_parents.insert(
            "UnsupportedDigestmodError".to_string(),
            "ValueError".to_string(),
        );
        let pickle_buffer_class = match self
            .heap
            .alloc_class(ClassObject::new("PickleBuffer".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *pickle_buffer_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_pickle".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::PickleBufferInit),
            );
            class_data.attrs.insert(
                "raw".to_string(),
                Value::Builtin(BuiltinFunction::PickleBufferRaw),
            );
            class_data.attrs.insert(
                "release".to_string(),
                Value::Builtin(BuiltinFunction::PickleBufferRelease),
            );
        }
        let pickler_class = match self
            .heap
            .alloc_class(ClassObject::new("Pickler".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *pickler_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_pickle".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::PicklePicklerInit),
            );
            class_data.attrs.insert(
                "dump".to_string(),
                Value::Builtin(BuiltinFunction::PicklePicklerDump),
            );
            class_data.attrs.insert(
                "clear_memo".to_string(),
                Value::Builtin(BuiltinFunction::PicklePicklerClearMemo),
            );
            class_data.attrs.insert(
                "persistent_id".to_string(),
                Value::Builtin(BuiltinFunction::PicklePicklerPersistentId),
            );
        }
        let unpickler_class = match self
            .heap
            .alloc_class(ClassObject::new("Unpickler".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *unpickler_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_pickle".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::PickleUnpicklerInit),
            );
            class_data.attrs.insert(
                "load".to_string(),
                Value::Builtin(BuiltinFunction::PickleUnpicklerLoad),
            );
            class_data.attrs.insert(
                "persistent_load".to_string(),
                Value::Builtin(BuiltinFunction::PickleUnpicklerPersistentLoad),
            );
        }
        self.install_builtin_module(
            "_pickle",
            &[
                ("dump", BuiltinFunction::PickleDump),
                ("dumps", BuiltinFunction::PickleDumps),
                ("load", BuiltinFunction::PickleLoad),
                ("loads", BuiltinFunction::PickleLoads),
                ("__getattr__", BuiltinFunction::PickleModuleGetAttr),
            ],
            vec![
                ("Pickler", Value::Class(pickler_class)),
                ("Unpickler", Value::Class(unpickler_class)),
                ("PickleBuffer", Value::Class(pickle_buffer_class)),
                (
                    "PickleError",
                    Value::ExceptionType("PickleError".to_string()),
                ),
                (
                    "PicklingError",
                    Value::ExceptionType("PicklingError".to_string()),
                ),
                (
                    "UnpicklingError",
                    Value::ExceptionType("UnpicklingError".to_string()),
                ),
            ],
        );
        self.install_builtin_module(
            "copyreg",
            &[
                ("_reconstructor", BuiltinFunction::CopyregReconstructor),
                ("__newobj__", BuiltinFunction::CopyregNewObj),
                ("__newobj_ex__", BuiltinFunction::CopyregNewObjEx),
            ],
            vec![("dispatch_table", self.heap.alloc_dict(Vec::new()))],
        );
        if let (Some(json_module), Value::Module(decoder_module), Value::Module(scanner_module)) = (
            self.modules.get("json").cloned(),
            self.heap
                .alloc_module(ModuleObject::new("json.decoder".to_string())),
            self.heap
                .alloc_module(ModuleObject::new("json.scanner".to_string())),
        ) {
            self.set_module_metadata(
                &decoder_module,
                "json.decoder",
                None,
                None,
                Some(BUILTIN_MODULE_LOADER),
                false,
                Vec::new(),
                false,
            );
            self.set_module_metadata(
                &scanner_module,
                "json.scanner",
                None,
                None,
                Some(BUILTIN_MODULE_LOADER),
                false,
                Vec::new(),
                false,
            );
            if let Object::Module(module_data) = &mut *decoder_module.kind_mut() {
                module_data.globals.insert(
                    "JSONDecodeError".to_string(),
                    Value::ExceptionType("ValueError".to_string()),
                );
                module_data.globals.insert(
                    "scanstring".to_string(),
                    Value::Builtin(BuiltinFunction::JsonDecoderScanString),
                );
                module_data.globals.insert(
                    "c_scanstring".to_string(),
                    Value::Builtin(BuiltinFunction::JsonDecoderScanString),
                );
                module_data.globals.insert(
                    "py_scanstring".to_string(),
                    Value::Builtin(BuiltinFunction::JsonDecoderScanString),
                );
            }
            if let Object::Module(module_data) = &mut *scanner_module.kind_mut() {
                module_data.globals.insert(
                    "make_scanner".to_string(),
                    Value::Builtin(BuiltinFunction::JsonScannerMakeScanner),
                );
                module_data.globals.insert(
                    "py_make_scanner".to_string(),
                    Value::Builtin(BuiltinFunction::JsonScannerPyMakeScanner),
                );
                module_data.globals.insert(
                    "c_make_scanner".to_string(),
                    Value::Builtin(BuiltinFunction::JsonScannerMakeScanner),
                );
            }
            if let Object::Module(module_data) = &mut *json_module.kind_mut() {
                module_data
                    .globals
                    .insert("decoder".to_string(), Value::Module(decoder_module.clone()));
                module_data
                    .globals
                    .insert("scanner".to_string(), Value::Module(scanner_module.clone()));
            }
            self.register_module("json.decoder", decoder_module);
            self.register_module("json.scanner", scanner_module);
        }
        self.install_builtin_module(
            "marshal",
            &[
                ("loads", BuiltinFunction::MarshalLoads),
                ("dumps", BuiltinFunction::MarshalDumps),
            ],
            vec![("version", Value::Int(5))],
        );
        let codec_info_class = self
            .heap
            .alloc_class(ClassObject::new("CodecInfo".to_string(), Vec::new()));
        let incremental_decoder_class = self.heap.alloc_class(ClassObject::new(
            "IncrementalDecoder".to_string(),
            Vec::new(),
        ));
        let incremental_encoder_class = self.heap.alloc_class(ClassObject::new(
            "IncrementalEncoder".to_string(),
            Vec::new(),
        ));
        let codec_class = self
            .heap
            .alloc_class(ClassObject::new("Codec".to_string(), Vec::new()));
        if let Value::Class(class_obj) = &codec_class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("codecs".to_string()));
        }
        if let Value::Class(class_obj) = &codec_info_class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("codecs".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::CodecsCodecInfoInit),
            );
        }
        if let Value::Class(class_obj) = &incremental_decoder_class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("codecs".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalDecoderInit),
            );
            class_data.attrs.insert(
                "decode".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalDecoderDecode),
            );
            class_data.attrs.insert(
                "reset".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalDecoderReset),
            );
            class_data.attrs.insert(
                "getstate".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalDecoderGetState),
            );
            class_data.attrs.insert(
                "setstate".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalDecoderSetState),
            );
        }
        if let Value::Class(class_obj) = &incremental_encoder_class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("codecs".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalEncoderInit),
            );
            class_data.attrs.insert(
                "encode".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalEncoderEncode),
            );
            class_data.attrs.insert(
                "reset".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalEncoderReset),
            );
            class_data.attrs.insert(
                "getstate".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalEncoderGetState),
            );
            class_data.attrs.insert(
                "setstate".to_string(),
                Value::Builtin(BuiltinFunction::CodecsIncrementalEncoderSetState),
            );
        }
        let stream_reader_class = self
            .heap
            .alloc_class(ClassObject::new("StreamReader".to_string(), Vec::new()));
        let stream_writer_class = self
            .heap
            .alloc_class(ClassObject::new("StreamWriter".to_string(), Vec::new()));
        self.install_builtin_module(
            "codecs",
            &[
                ("encode", BuiltinFunction::CodecsEncode),
                ("decode", BuiltinFunction::CodecsDecode),
                ("escape_decode", BuiltinFunction::CodecsEscapeDecode),
                (
                    "make_identity_dict",
                    BuiltinFunction::CodecsMakeIdentityDict,
                ),
                ("lookup", BuiltinFunction::CodecsLookup),
                ("register", BuiltinFunction::CodecsRegister),
                ("unregister", BuiltinFunction::CodecsUnregister),
                (
                    "getincrementalencoder",
                    BuiltinFunction::CodecsGetIncrementalEncoder,
                ),
                (
                    "getincrementaldecoder",
                    BuiltinFunction::CodecsGetIncrementalDecoder,
                ),
            ],
            vec![
                ("BOM_UTF8", self.heap.alloc_bytes(vec![0xEF, 0xBB, 0xBF])),
                ("Codec", codec_class),
                ("CodecInfo", codec_info_class),
                ("IncrementalDecoder", incremental_decoder_class),
                ("IncrementalEncoder", incremental_encoder_class),
                ("StreamReader", stream_reader_class),
                ("StreamWriter", stream_writer_class),
            ],
        );
        self.install_module_alias_from_existing("_codecs", "codecs");
        self.install_builtin_module(
            "unicodedata",
            &[
                ("normalize", BuiltinFunction::UnicodedataNormalize),
                (
                    "east_asian_width",
                    BuiltinFunction::UnicodedataEastAsianWidth,
                ),
                ("category", BuiltinFunction::UnicodedataCategory),
                ("bidirectional", BuiltinFunction::UnicodedataBidirectional),
            ],
            Vec::new(),
        );
        if let Some(unicodedata_module) = self.modules.get("unicodedata").cloned() {
            if let Object::Module(module_data) = &mut *unicodedata_module.kind_mut() {
                module_data.globals.insert(
                    "unidata_version".to_string(),
                    Value::Str("16.0.0".to_string()),
                );
                let legacy = match self
                    .heap
                    .alloc_module(ModuleObject::new("unicodedata.ucd_3_2_0".to_string()))
                {
                    Value::Module(module) => module,
                    _ => unreachable!(),
                };
                self.set_module_metadata(
                    &legacy,
                    "unicodedata.ucd_3_2_0",
                    None,
                    None,
                    Some(BUILTIN_MODULE_LOADER),
                    false,
                    Vec::new(),
                    false,
                );
                if let Object::Module(legacy_data) = &mut *legacy.kind_mut() {
                    legacy_data.globals.insert(
                        "unidata_version".to_string(),
                        Value::Str("3.2.0".to_string()),
                    );
                    legacy_data.globals.insert(
                        "normalize".to_string(),
                        Value::Builtin(BuiltinFunction::UnicodedataNormalize),
                    );
                    legacy_data.globals.insert(
                        "east_asian_width".to_string(),
                        Value::Builtin(BuiltinFunction::UnicodedataEastAsianWidth),
                    );
                    legacy_data.globals.insert(
                        "category".to_string(),
                        Value::Builtin(BuiltinFunction::UnicodedataLegacyCategory),
                    );
                    legacy_data.globals.insert(
                        "bidirectional".to_string(),
                        Value::Builtin(BuiltinFunction::UnicodedataLegacyBidirectional),
                    );
                }
                module_data
                    .globals
                    .insert("ucd_3_2_0".to_string(), Value::Module(legacy));
            }
        }
        self.install_builtin_module(
            "binascii",
            &[
                ("crc32", BuiltinFunction::BinasciiCrc32),
                ("b2a_base64", BuiltinFunction::BinasciiB2aBase64),
                ("a2b_base64", BuiltinFunction::BinasciiA2bBase64),
                ("hexlify", BuiltinFunction::BinasciiHexlify),
                ("b2a_hex", BuiltinFunction::BinasciiHexlify),
                ("unhexlify", BuiltinFunction::BinasciiUnhexlify),
                ("a2b_hex", BuiltinFunction::BinasciiUnhexlify),
            ],
            vec![
                ("Error", Value::ExceptionType("Exception".to_string())),
                ("Incomplete", Value::ExceptionType("Exception".to_string())),
            ],
        );
        let csv_reader_class = match self
            .heap
            .alloc_class(ClassObject::new("Reader".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *csv_reader_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_csv".to_string()));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
        }
        let csv_writer_class = match self
            .heap
            .alloc_class(ClassObject::new("Writer".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *csv_writer_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_csv".to_string()));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
        }
        self.install_builtin_module(
            "_csv",
            &[
                ("reader", BuiltinFunction::CsvReader),
                ("writer", BuiltinFunction::CsvWriter),
                ("register_dialect", BuiltinFunction::CsvRegisterDialect),
                ("unregister_dialect", BuiltinFunction::CsvUnregisterDialect),
                ("get_dialect", BuiltinFunction::CsvGetDialect),
                ("list_dialects", BuiltinFunction::CsvListDialects),
                ("field_size_limit", BuiltinFunction::CsvFieldSizeLimit),
                ("Dialect", BuiltinFunction::CsvDialectValidate),
            ],
            vec![
                ("Error", Value::ExceptionType("Error".to_string())),
                ("Reader", Value::Class(csv_reader_class)),
                ("Writer", Value::Class(csv_writer_class)),
                ("QUOTE_MINIMAL", Value::Int(0)),
                ("QUOTE_ALL", Value::Int(1)),
                ("QUOTE_NONNUMERIC", Value::Int(2)),
                ("QUOTE_NONE", Value::Int(3)),
                ("QUOTE_STRINGS", Value::Int(4)),
                ("QUOTE_NOTNULL", Value::Int(5)),
            ],
        );
        let sqlite_connection_class = match self
            .heap
            .alloc_class(ClassObject::new("Connection".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *sqlite_connection_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_sqlite3".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionInit),
            );
            class_data.attrs.insert(
                "__del__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionDel),
            );
            class_data.attrs.insert(
                "__getattribute__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionGetAttribute),
            );
            class_data.attrs.insert(
                "__setattr__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionSetAttribute),
            );
            class_data.attrs.insert(
                "__delattr__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionDelAttribute),
            );
            class_data.attrs.insert(
                "Warning".to_string(),
                Value::ExceptionType("Warning".to_string()),
            );
            class_data.attrs.insert(
                "Error".to_string(),
                Value::ExceptionType("Error".to_string()),
            );
            class_data.attrs.insert(
                "InterfaceError".to_string(),
                Value::ExceptionType("InterfaceError".to_string()),
            );
            class_data.attrs.insert(
                "DatabaseError".to_string(),
                Value::ExceptionType("DatabaseError".to_string()),
            );
            class_data.attrs.insert(
                "DataError".to_string(),
                Value::ExceptionType("DataError".to_string()),
            );
            class_data.attrs.insert(
                "OperationalError".to_string(),
                Value::ExceptionType("OperationalError".to_string()),
            );
            class_data.attrs.insert(
                "IntegrityError".to_string(),
                Value::ExceptionType("IntegrityError".to_string()),
            );
            class_data.attrs.insert(
                "InternalError".to_string(),
                Value::ExceptionType("InternalError".to_string()),
            );
            class_data.attrs.insert(
                "ProgrammingError".to_string(),
                Value::ExceptionType("ProgrammingError".to_string()),
            );
            class_data.attrs.insert(
                "NotSupportedError".to_string(),
                Value::ExceptionType("NotSupportedError".to_string()),
            );
            class_data.attrs.insert(
                "cursor".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionCursor),
            );
            class_data.attrs.insert(
                "close".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionClose),
            );
            class_data.attrs.insert(
                "__enter__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionEnter),
            );
            class_data.attrs.insert(
                "__exit__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionExit),
            );
            class_data.attrs.insert(
                "execute".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionExecute),
            );
            class_data.attrs.insert(
                "executemany".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionExecuteMany),
            );
            class_data.attrs.insert(
                "executescript".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionExecuteScript),
            );
            class_data.attrs.insert(
                "__call__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionExecute),
            );
            class_data.attrs.insert(
                "commit".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionCommit),
            );
            class_data.attrs.insert(
                "rollback".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionRollback),
            );
            class_data.attrs.insert(
                "interrupt".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionInterrupt),
            );
            class_data.attrs.insert(
                "iterdump".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionIterDump),
            );
            class_data.attrs.insert(
                "create_function".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionCreateFunction),
            );
            class_data.attrs.insert(
                "create_aggregate".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionCreateAggregate),
            );
            class_data.attrs.insert(
                "create_window_function".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionCreateWindowFunction),
            );
            class_data.attrs.insert(
                "set_trace_callback".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionSetTraceCallback),
            );
            class_data.attrs.insert(
                "create_collation".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionCreateCollation),
            );
            class_data.attrs.insert(
                "set_authorizer".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionSetAuthorizer),
            );
            class_data.attrs.insert(
                "set_progress_handler".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionSetProgressHandler),
            );
            class_data.attrs.insert(
                "getlimit".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionGetLimit),
            );
            class_data.attrs.insert(
                "setlimit".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionSetLimit),
            );
            class_data.attrs.insert(
                "getconfig".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionGetConfig),
            );
            class_data.attrs.insert(
                "setconfig".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionSetConfig),
            );
            class_data.attrs.insert(
                "blobopen".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionBlobOpen),
            );
            class_data.attrs.insert(
                "backup".to_string(),
                Value::Builtin(BuiltinFunction::SqliteConnectionBackup),
            );
        }
        let sqlite_cursor_class = match self
            .heap
            .alloc_class(ClassObject::new("Cursor".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *sqlite_cursor_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_sqlite3".to_string()));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorInit),
            );
            class_data.attrs.insert(
                "__setattr__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorSetAttribute),
            );
            class_data.attrs.insert(
                "setinputsizes".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorSetInputSizes),
            );
            class_data.attrs.insert(
                "setoutputsize".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorSetOutputSize),
            );
            class_data.attrs.insert(
                "execute".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorExecute),
            );
            class_data.attrs.insert(
                "executemany".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorExecuteMany),
            );
            class_data.attrs.insert(
                "executescript".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorExecuteScript),
            );
            class_data.attrs.insert(
                "fetchone".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorFetchOne),
            );
            class_data.attrs.insert(
                "fetchmany".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorFetchMany),
            );
            class_data.attrs.insert(
                "fetchall".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorFetchAll),
            );
            class_data.attrs.insert(
                "close".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorClose),
            );
            class_data.attrs.insert(
                "__iter__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorIter),
            );
            class_data.attrs.insert(
                "__next__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteCursorNext),
            );
        }
        let sqlite_blob_class = match self
            .heap
            .alloc_class(ClassObject::new("Blob".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *sqlite_blob_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_sqlite3".to_string()));
            class_data.attrs.insert(
                "__pyrs_disallow_instantiation__".to_string(),
                Value::Bool(true),
            );
            class_data.attrs.insert(
                "close".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobClose),
            );
            class_data.attrs.insert(
                "read".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobRead),
            );
            class_data.attrs.insert(
                "write".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobWrite),
            );
            class_data.attrs.insert(
                "seek".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobSeek),
            );
            class_data.attrs.insert(
                "tell".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobTell),
            );
            class_data.attrs.insert(
                "__enter__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobEnter),
            );
            class_data.attrs.insert(
                "__exit__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobExit),
            );
            class_data.attrs.insert(
                "__len__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobLen),
            );
            class_data.attrs.insert(
                "__getitem__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobGetItem),
            );
            class_data.attrs.insert(
                "__setitem__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobSetItem),
            );
            class_data.attrs.insert(
                "__delitem__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobDelItem),
            );
            class_data.attrs.insert(
                "__iter__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteBlobIter),
            );
        }
        let sqlite_row_class = self
            .heap
            .alloc_class(ClassObject::new("Row".to_string(), Vec::new()));
        if let Value::Class(class) = &sqlite_row_class
            && let Object::Class(class_data) = &mut *class.kind_mut()
        {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_sqlite3".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteRowInit),
            );
            class_data.attrs.insert(
                "keys".to_string(),
                Value::Builtin(BuiltinFunction::SqliteRowKeys),
            );
            class_data.attrs.insert(
                "__len__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteRowLen),
            );
            class_data.attrs.insert(
                "__getitem__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteRowGetItem),
            );
            class_data.attrs.insert(
                "__iter__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteRowIter),
            );
            class_data.attrs.insert(
                "__eq__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteRowEq),
            );
            class_data.attrs.insert(
                "__hash__".to_string(),
                Value::Builtin(BuiltinFunction::SqliteRowHash),
            );
        }
        let sqlite_prepare_protocol_class = self
            .heap
            .alloc_class(ClassObject::new("PrepareProtocol".to_string(), Vec::new()));
        if let Value::Class(class) = &sqlite_prepare_protocol_class
            && let Object::Class(class_data) = &mut *class.kind_mut()
        {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_sqlite3".to_string()));
        }
        self.install_builtin_module(
            "_sqlite3",
            &[
                ("connect", BuiltinFunction::SqliteConnect),
                (
                    "complete_statement",
                    BuiltinFunction::SqliteCompleteStatement,
                ),
                ("register_adapter", BuiltinFunction::SqliteRegisterAdapter),
                (
                    "register_converter",
                    BuiltinFunction::SqliteRegisterConverter,
                ),
                (
                    "enable_callback_tracebacks",
                    BuiltinFunction::SqliteEnableCallbackTracebacks,
                ),
            ],
            vec![
                ("Connection", Value::Class(sqlite_connection_class)),
                ("Cursor", Value::Class(sqlite_cursor_class)),
                ("Row", sqlite_row_class),
                ("PrepareProtocol", sqlite_prepare_protocol_class),
                ("Warning", Value::ExceptionType("Warning".to_string())),
                ("Error", Value::ExceptionType("Error".to_string())),
                (
                    "InterfaceError",
                    Value::ExceptionType("InterfaceError".to_string()),
                ),
                (
                    "DatabaseError",
                    Value::ExceptionType("DatabaseError".to_string()),
                ),
                ("DataError", Value::ExceptionType("DataError".to_string())),
                (
                    "OperationalError",
                    Value::ExceptionType("OperationalError".to_string()),
                ),
                (
                    "IntegrityError",
                    Value::ExceptionType("IntegrityError".to_string()),
                ),
                (
                    "InternalError",
                    Value::ExceptionType("InternalError".to_string()),
                ),
                (
                    "ProgrammingError",
                    Value::ExceptionType("ProgrammingError".to_string()),
                ),
                (
                    "NotSupportedError",
                    Value::ExceptionType("NotSupportedError".to_string()),
                ),
                ("PARSE_DECLTYPES", Value::Int(1)),
                ("PARSE_COLNAMES", Value::Int(2)),
                ("LEGACY_TRANSACTION_CONTROL", Value::Int(-1)),
                ("threadsafety", Value::Int(3)),
                (
                    "sqlite_version",
                    Value::Str(self.sqlite_libversion_string()),
                ),
                ("SQLITE_OK", Value::Int(0)),
                ("SQLITE_DENY", Value::Int(1)),
                ("SQLITE_IGNORE", Value::Int(2)),
                ("SQLITE_CREATE_INDEX", Value::Int(1)),
                ("SQLITE_CREATE_TABLE", Value::Int(2)),
                ("SQLITE_CREATE_TEMP_INDEX", Value::Int(3)),
                ("SQLITE_CREATE_TEMP_TABLE", Value::Int(4)),
                ("SQLITE_CREATE_TEMP_TRIGGER", Value::Int(5)),
                ("SQLITE_CREATE_TEMP_VIEW", Value::Int(6)),
                ("SQLITE_CREATE_TRIGGER", Value::Int(7)),
                ("SQLITE_CREATE_VIEW", Value::Int(8)),
                ("SQLITE_DELETE", Value::Int(9)),
                ("SQLITE_DROP_INDEX", Value::Int(10)),
                ("SQLITE_DROP_TABLE", Value::Int(11)),
                ("SQLITE_DROP_TEMP_INDEX", Value::Int(12)),
                ("SQLITE_DROP_TEMP_TABLE", Value::Int(13)),
                ("SQLITE_DROP_TEMP_TRIGGER", Value::Int(14)),
                ("SQLITE_DROP_TEMP_VIEW", Value::Int(15)),
                ("SQLITE_DROP_TRIGGER", Value::Int(16)),
                ("SQLITE_DROP_VIEW", Value::Int(17)),
                ("SQLITE_INSERT", Value::Int(18)),
                ("SQLITE_PRAGMA", Value::Int(19)),
                ("SQLITE_READ", Value::Int(20)),
                ("SQLITE_SELECT", Value::Int(21)),
                ("SQLITE_TRANSACTION", Value::Int(22)),
                ("SQLITE_UPDATE", Value::Int(23)),
                ("SQLITE_ATTACH", Value::Int(24)),
                ("SQLITE_DETACH", Value::Int(25)),
                ("SQLITE_ALTER_TABLE", Value::Int(26)),
                ("SQLITE_REINDEX", Value::Int(27)),
                ("SQLITE_ANALYZE", Value::Int(28)),
                ("SQLITE_CREATE_VTABLE", Value::Int(29)),
                ("SQLITE_DROP_VTABLE", Value::Int(30)),
                ("SQLITE_FUNCTION", Value::Int(31)),
                ("SQLITE_SAVEPOINT", Value::Int(32)),
                ("SQLITE_RECURSIVE", Value::Int(33)),
                ("SQLITE_LIMIT_LENGTH", Value::Int(0)),
                ("SQLITE_LIMIT_SQL_LENGTH", Value::Int(1)),
                ("SQLITE_LIMIT_COLUMN", Value::Int(2)),
                ("SQLITE_LIMIT_EXPR_DEPTH", Value::Int(3)),
                ("SQLITE_LIMIT_COMPOUND_SELECT", Value::Int(4)),
                ("SQLITE_LIMIT_VDBE_OP", Value::Int(5)),
                ("SQLITE_LIMIT_FUNCTION_ARG", Value::Int(6)),
                ("SQLITE_LIMIT_ATTACHED", Value::Int(7)),
                ("SQLITE_LIMIT_LIKE_PATTERN_LENGTH", Value::Int(8)),
                ("SQLITE_LIMIT_VARIABLE_NUMBER", Value::Int(9)),
                ("SQLITE_LIMIT_TRIGGER_DEPTH", Value::Int(10)),
                ("SQLITE_LIMIT_WORKER_THREADS", Value::Int(11)),
                ("SQLITE_DBCONFIG_ENABLE_FKEY", Value::Int(1002)),
                ("SQLITE_DBCONFIG_ENABLE_TRIGGER", Value::Int(1003)),
                ("SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER", Value::Int(1004)),
                ("SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION", Value::Int(1005)),
                ("SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE", Value::Int(1006)),
                ("SQLITE_DBCONFIG_ENABLE_QPSG", Value::Int(1007)),
                ("SQLITE_DBCONFIG_TRIGGER_EQP", Value::Int(1008)),
                ("SQLITE_DBCONFIG_RESET_DATABASE", Value::Int(1009)),
                ("SQLITE_DBCONFIG_DEFENSIVE", Value::Int(1010)),
                ("SQLITE_DBCONFIG_WRITABLE_SCHEMA", Value::Int(1011)),
                ("SQLITE_DBCONFIG_LEGACY_ALTER_TABLE", Value::Int(1012)),
                ("SQLITE_DBCONFIG_DQS_DML", Value::Int(1013)),
                ("SQLITE_DBCONFIG_DQS_DDL", Value::Int(1014)),
                ("SQLITE_DBCONFIG_ENABLE_VIEW", Value::Int(1015)),
                ("SQLITE_DBCONFIG_LEGACY_FILE_FORMAT", Value::Int(1016)),
                ("SQLITE_DBCONFIG_TRUSTED_SCHEMA", Value::Int(1017)),
                ("SQLITE_ABORT", Value::Int(4)),
                ("SQLITE_ABORT_ROLLBACK", Value::Int(516)),
                ("SQLITE_AUTH", Value::Int(23)),
                ("SQLITE_AUTH_USER", Value::Int(279)),
                ("SQLITE_BUSY", Value::Int(5)),
                ("SQLITE_BUSY_RECOVERY", Value::Int(261)),
                ("SQLITE_BUSY_SNAPSHOT", Value::Int(517)),
                ("SQLITE_BUSY_TIMEOUT", Value::Int(773)),
                ("SQLITE_CANTOPEN", Value::Int(14)),
                ("SQLITE_CANTOPEN_CONVPATH", Value::Int(1038)),
                ("SQLITE_CANTOPEN_DIRTYWAL", Value::Int(1294)),
                ("SQLITE_CANTOPEN_FULLPATH", Value::Int(782)),
                ("SQLITE_CANTOPEN_ISDIR", Value::Int(526)),
                ("SQLITE_CANTOPEN_NOTEMPDIR", Value::Int(270)),
                ("SQLITE_CANTOPEN_SYMLINK", Value::Int(1550)),
                ("SQLITE_CONSTRAINT", Value::Int(19)),
                ("SQLITE_CONSTRAINT_CHECK", Value::Int(275)),
                ("SQLITE_CONSTRAINT_COMMITHOOK", Value::Int(531)),
                ("SQLITE_CONSTRAINT_FOREIGNKEY", Value::Int(787)),
                ("SQLITE_CONSTRAINT_FUNCTION", Value::Int(1043)),
                ("SQLITE_CONSTRAINT_NOTNULL", Value::Int(1299)),
                ("SQLITE_CONSTRAINT_PINNED", Value::Int(2835)),
                ("SQLITE_CONSTRAINT_PRIMARYKEY", Value::Int(1555)),
                ("SQLITE_CONSTRAINT_ROWID", Value::Int(2579)),
                ("SQLITE_CONSTRAINT_TRIGGER", Value::Int(1811)),
                ("SQLITE_CONSTRAINT_UNIQUE", Value::Int(2067)),
                ("SQLITE_CONSTRAINT_VTAB", Value::Int(2323)),
                ("SQLITE_CORRUPT", Value::Int(11)),
                ("SQLITE_CORRUPT_INDEX", Value::Int(779)),
                ("SQLITE_CORRUPT_SEQUENCE", Value::Int(523)),
                ("SQLITE_CORRUPT_VTAB", Value::Int(267)),
                ("SQLITE_DONE", Value::Int(101)),
                ("SQLITE_EMPTY", Value::Int(16)),
                ("SQLITE_ERROR", Value::Int(1)),
                ("SQLITE_ERROR_MISSING_COLLSEQ", Value::Int(257)),
                ("SQLITE_ERROR_RETRY", Value::Int(513)),
                ("SQLITE_ERROR_SNAPSHOT", Value::Int(769)),
                ("SQLITE_FORMAT", Value::Int(24)),
                ("SQLITE_FULL", Value::Int(13)),
                ("SQLITE_INTERNAL", Value::Int(2)),
                ("SQLITE_INTERRUPT", Value::Int(9)),
                ("SQLITE_IOERR", Value::Int(10)),
                ("SQLITE_IOERR_ACCESS", Value::Int(3338)),
                ("SQLITE_IOERR_AUTH", Value::Int(7178)),
                ("SQLITE_IOERR_BEGIN_ATOMIC", Value::Int(7434)),
                ("SQLITE_IOERR_BLOCKED", Value::Int(2826)),
                ("SQLITE_IOERR_CHECKRESERVEDLOCK", Value::Int(3594)),
                ("SQLITE_IOERR_CLOSE", Value::Int(4106)),
                ("SQLITE_IOERR_COMMIT_ATOMIC", Value::Int(7690)),
                ("SQLITE_IOERR_CONVPATH", Value::Int(6666)),
                ("SQLITE_IOERR_CORRUPTFS", Value::Int(8458)),
                ("SQLITE_IOERR_DATA", Value::Int(8202)),
                ("SQLITE_IOERR_DELETE", Value::Int(2570)),
                ("SQLITE_IOERR_DELETE_NOENT", Value::Int(5898)),
                ("SQLITE_IOERR_DIR_CLOSE", Value::Int(4362)),
                ("SQLITE_IOERR_DIR_FSYNC", Value::Int(1290)),
                ("SQLITE_IOERR_FSTAT", Value::Int(1802)),
                ("SQLITE_IOERR_FSYNC", Value::Int(1034)),
                ("SQLITE_IOERR_GETTEMPPATH", Value::Int(6410)),
                ("SQLITE_IOERR_LOCK", Value::Int(3850)),
                ("SQLITE_IOERR_MMAP", Value::Int(6154)),
                ("SQLITE_IOERR_NOMEM", Value::Int(3082)),
                ("SQLITE_IOERR_RDLOCK", Value::Int(2314)),
                ("SQLITE_IOERR_READ", Value::Int(266)),
                ("SQLITE_IOERR_ROLLBACK_ATOMIC", Value::Int(7946)),
                ("SQLITE_IOERR_SEEK", Value::Int(5642)),
                ("SQLITE_IOERR_SHMLOCK", Value::Int(5130)),
                ("SQLITE_IOERR_SHMMAP", Value::Int(5386)),
                ("SQLITE_IOERR_SHMOPEN", Value::Int(4618)),
                ("SQLITE_IOERR_SHMSIZE", Value::Int(4874)),
                ("SQLITE_IOERR_SHORT_READ", Value::Int(522)),
                ("SQLITE_IOERR_TRUNCATE", Value::Int(1546)),
                ("SQLITE_IOERR_UNLOCK", Value::Int(2058)),
                ("SQLITE_IOERR_VNODE", Value::Int(6922)),
                ("SQLITE_IOERR_WRITE", Value::Int(778)),
                ("SQLITE_LOCKED", Value::Int(6)),
                ("SQLITE_LOCKED_SHAREDCACHE", Value::Int(262)),
                ("SQLITE_LOCKED_VTAB", Value::Int(518)),
                ("SQLITE_MISMATCH", Value::Int(20)),
                ("SQLITE_MISUSE", Value::Int(21)),
                ("SQLITE_NOLFS", Value::Int(22)),
                ("SQLITE_NOMEM", Value::Int(7)),
                ("SQLITE_NOTADB", Value::Int(26)),
                ("SQLITE_NOTFOUND", Value::Int(12)),
                ("SQLITE_NOTICE", Value::Int(27)),
                ("SQLITE_NOTICE_RECOVER_ROLLBACK", Value::Int(539)),
                ("SQLITE_NOTICE_RECOVER_WAL", Value::Int(283)),
                ("SQLITE_OK_LOAD_PERMANENTLY", Value::Int(256)),
                ("SQLITE_OK_SYMLINK", Value::Int(512)),
                ("SQLITE_PERM", Value::Int(3)),
                ("SQLITE_PROTOCOL", Value::Int(15)),
                ("SQLITE_RANGE", Value::Int(25)),
                ("SQLITE_READONLY", Value::Int(8)),
                ("SQLITE_READONLY_CANTINIT", Value::Int(1288)),
                ("SQLITE_READONLY_CANTLOCK", Value::Int(520)),
                ("SQLITE_READONLY_DBMOVED", Value::Int(1032)),
                ("SQLITE_READONLY_DIRECTORY", Value::Int(1544)),
                ("SQLITE_READONLY_RECOVERY", Value::Int(264)),
                ("SQLITE_READONLY_ROLLBACK", Value::Int(776)),
                ("SQLITE_ROW", Value::Int(100)),
                ("SQLITE_SCHEMA", Value::Int(17)),
                ("SQLITE_TOOBIG", Value::Int(18)),
                ("SQLITE_WARNING", Value::Int(28)),
                ("SQLITE_WARNING_AUTOINDEX", Value::Int(284)),
                ("adapters", self.heap.alloc_dict(Vec::new())),
                ("converters", self.heap.alloc_dict(Vec::new())),
                ("Blob", Value::Class(sqlite_blob_class)),
            ],
        );
        self.exception_parents
            .insert("InterfaceError".to_string(), "Error".to_string());
        self.exception_parents
            .insert("DatabaseError".to_string(), "Error".to_string());
        self.exception_parents
            .insert("DataError".to_string(), "DatabaseError".to_string());
        self.exception_parents
            .insert("OperationalError".to_string(), "DatabaseError".to_string());
        self.exception_parents
            .insert("IntegrityError".to_string(), "DatabaseError".to_string());
        self.exception_parents
            .insert("InternalError".to_string(), "DatabaseError".to_string());
        self.exception_parents
            .insert("ProgrammingError".to_string(), "DatabaseError".to_string());
        self.exception_parents
            .insert("NotSupportedError".to_string(), "DatabaseError".to_string());
        // CPython accelerator shim for Lib/re package.
        // Reference: Python-3.14.3 Modules/_sre/sre.c and Lib/re/_compiler.py.
        self.install_builtin_module(
            "_sre",
            &[
                ("compile", BuiltinFunction::SreCompile),
                ("template", BuiltinFunction::SreTemplate),
                ("ascii_iscased", BuiltinFunction::SreAsciiIsCased),
                ("ascii_tolower", BuiltinFunction::SreAsciiToLower),
                ("unicode_iscased", BuiltinFunction::SreUnicodeIsCased),
                ("unicode_tolower", BuiltinFunction::SreUnicodeToLower),
            ],
            vec![
                ("MAGIC", Value::Int(20230612)),
                ("CODESIZE", Value::Int(4)),
                ("MAXREPEAT", Value::Int(i32::MAX as i64)),
                ("MAXGROUPS", Value::Int(i32::MAX as i64)),
            ],
        );
        self.install_builtin_module(
            "re",
            &[
                ("search", BuiltinFunction::ReSearch),
                ("match", BuiltinFunction::ReMatch),
                ("fullmatch", BuiltinFunction::ReFullMatch),
                ("compile", BuiltinFunction::ReCompile),
                ("escape", BuiltinFunction::ReEscape),
            ],
            vec![
                ("TEMPLATE", Value::Int(1)),
                ("T", Value::Int(1)),
                ("IGNORECASE", Value::Int(2)),
                ("I", Value::Int(2)),
                ("LOCALE", Value::Int(4)),
                ("L", Value::Int(4)),
                ("MULTILINE", Value::Int(8)),
                ("M", Value::Int(8)),
                ("DOTALL", Value::Int(16)),
                ("S", Value::Int(16)),
                ("UNICODE", Value::Int(32)),
                ("U", Value::Int(32)),
                ("VERBOSE", Value::Int(64)),
                ("X", Value::Int(64)),
                ("DEBUG", Value::Int(128)),
                ("ASCII", Value::Int(256)),
                ("A", Value::Int(256)),
                (
                    "Scanner",
                    self.heap
                        .alloc_class(ClassObject::new("Scanner".to_string(), Vec::new())),
                ),
                (
                    "Pattern",
                    self.heap
                        .alloc_class(ClassObject::new("Pattern".to_string(), Vec::new())),
                ),
                (
                    "Match",
                    self.heap
                        .alloc_class(ClassObject::new("Match".to_string(), Vec::new())),
                ),
            ],
        );
        self.install_builtin_module(
            "operator",
            &[
                ("add", BuiltinFunction::OperatorAdd),
                ("sub", BuiltinFunction::OperatorSub),
                ("mul", BuiltinFunction::OperatorMul),
                ("mod", BuiltinFunction::OperatorMod),
                ("pow", BuiltinFunction::Pow),
                ("and_", BuiltinFunction::OperatorAnd),
                ("or_", BuiltinFunction::OperatorOr),
                ("xor", BuiltinFunction::OperatorXor),
                ("lshift", BuiltinFunction::OperatorLShift),
                ("rshift", BuiltinFunction::OperatorRShift),
                ("matmul", BuiltinFunction::OperatorMatMul),
                ("neg", BuiltinFunction::OperatorNeg),
                ("pos", BuiltinFunction::OperatorPos),
                ("inv", BuiltinFunction::OperatorInvert),
                ("invert", BuiltinFunction::OperatorInvert),
                ("truediv", BuiltinFunction::OperatorTrueDiv),
                ("floordiv", BuiltinFunction::OperatorFloorDiv),
                ("index", BuiltinFunction::OperatorIndex),
                ("eq", BuiltinFunction::OperatorEq),
                ("ne", BuiltinFunction::OperatorNe),
                ("lt", BuiltinFunction::OperatorLt),
                ("le", BuiltinFunction::OperatorLe),
                ("gt", BuiltinFunction::OperatorGt),
                ("ge", BuiltinFunction::OperatorGe),
                ("contains", BuiltinFunction::OperatorContains),
                ("getitem", BuiltinFunction::OperatorGetItem),
                ("itemgetter", BuiltinFunction::OperatorItemGetter),
                ("attrgetter", BuiltinFunction::OperatorAttrGetter),
                ("methodcaller", BuiltinFunction::OperatorMethodCaller),
                ("_compare_digest", BuiltinFunction::OperatorCompareDigest),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "_operator",
            &[("_compare_digest", BuiltinFunction::OperatorCompareDigest)],
            Vec::new(),
        );
        let mmap_type = self
            .heap
            .alloc_class(ClassObject::new("mmap".to_string(), Vec::new()));
        self.install_builtin_module(
            "mmap",
            &[],
            vec![
                ("mmap", mmap_type),
                ("ACCESS_READ", Value::Int(1)),
                ("ACCESS_WRITE", Value::Int(2)),
                ("ACCESS_COPY", Value::Int(3)),
                ("PAGESIZE", Value::Int(4096)),
                ("ALLOCATIONGRANULARITY", Value::Int(4096)),
            ],
        );
        self.install_builtin_module(
            "_string",
            &[
                ("formatter_parser", BuiltinFunction::StringFormatterParser),
                (
                    "formatter_field_name_split",
                    BuiltinFunction::StringFormatterFieldNameSplit,
                ),
            ],
            Vec::new(),
        );
        let ansi_colors = match self
            .heap
            .alloc_module(ModuleObject::new("__colorize_ansi__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *ansi_colors.kind_mut() {
            for name in [
                "RESET",
                "RED",
                "GREEN",
                "YELLOW",
                "BLUE",
                "MAGENTA",
                "CYAN",
                "WHITE",
                "BOLD",
                "BOLD_RED",
                "BOLD_GREEN",
                "BOLD_YELLOW",
                "BOLD_BLUE",
                "BOLD_MAGENTA",
                "BOLD_CYAN",
                "BOLD_WHITE",
            ] {
                module_data
                    .globals
                    .insert(name.to_string(), Value::Str(String::new()));
            }
        }
        let traceback_theme = match self
            .heap
            .alloc_module(ModuleObject::new("__colorize_traceback__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *traceback_theme.kind_mut() {
            for name in [
                "type",
                "message",
                "filename",
                "line_no",
                "frame",
                "error_highlight",
                "error_range",
                "reset",
            ] {
                module_data
                    .globals
                    .insert(name.to_string(), Value::Str(String::new()));
            }
        }
        let unittest_theme = match self
            .heap
            .alloc_module(ModuleObject::new("__colorize_unittest__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *unittest_theme.kind_mut() {
            for name in ["passed", "warn", "fail", "fail_info", "reset"] {
                module_data
                    .globals
                    .insert(name.to_string(), Value::Str(String::new()));
            }
        }
        let syntax_theme = match self
            .heap
            .alloc_module(ModuleObject::new("__colorize_syntax__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *syntax_theme.kind_mut() {
            for name in [
                "prompt",
                "keyword",
                "keyword_constant",
                "builtin",
                "comment",
                "string",
                "number",
                "op",
                "definition",
                "soft_keyword",
                "reset",
            ] {
                module_data
                    .globals
                    .insert(name.to_string(), Value::Str(String::new()));
            }
        }
        let argparse_theme = match self
            .heap
            .alloc_module(ModuleObject::new("__colorize_argparse__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *argparse_theme.kind_mut() {
            for name in [
                "usage",
                "prog",
                "prog_extra",
                "heading",
                "summary_long_option",
                "summary_short_option",
                "summary_label",
                "summary_action",
                "long_option",
                "short_option",
                "label",
                "action",
                "reset",
            ] {
                module_data
                    .globals
                    .insert(name.to_string(), Value::Str(String::new()));
            }
        }
        let color_theme = match self
            .heap
            .alloc_module(ModuleObject::new("__colorize_theme__".to_string()))
        {
            Value::Module(module) => module,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *color_theme.kind_mut() {
            module_data
                .globals
                .insert("argparse".to_string(), Value::Module(argparse_theme));
            module_data
                .globals
                .insert("syntax".to_string(), Value::Module(syntax_theme));
            module_data
                .globals
                .insert("traceback".to_string(), Value::Module(traceback_theme));
            module_data
                .globals
                .insert("unittest".to_string(), Value::Module(unittest_theme));
        }
        self.install_builtin_module(
            "_colorize",
            &[
                ("can_colorize", BuiltinFunction::ColorizeCanColorize),
                ("get_theme", BuiltinFunction::ColorizeGetTheme),
                ("get_colors", BuiltinFunction::ColorizeGetColors),
                ("set_theme", BuiltinFunction::ColorizeSetTheme),
                ("decolor", BuiltinFunction::ColorizeDecolor),
            ],
            vec![
                ("COLORIZE", Value::Bool(false)),
                ("ANSIColors", Value::Module(ansi_colors.clone())),
                ("NoColors", Value::Module(ansi_colors.clone())),
                ("default_theme", Value::Module(color_theme.clone())),
                ("_theme", Value::Module(color_theme)),
                ("_ansi", Value::Module(ansi_colors)),
            ],
        );
        self.install_builtin_module(
            "itertools",
            &[
                ("accumulate", BuiltinFunction::ItertoolsAccumulate),
                ("chain", BuiltinFunction::ItertoolsChain),
                ("combinations", BuiltinFunction::ItertoolsCombinations),
                (
                    "combinations_with_replacement",
                    BuiltinFunction::ItertoolsCombinationsWithReplacement,
                ),
                ("compress", BuiltinFunction::ItertoolsCompress),
                ("count", BuiltinFunction::ItertoolsCount),
                ("cycle", BuiltinFunction::ItertoolsCycle),
                ("dropwhile", BuiltinFunction::ItertoolsDropWhile),
                ("filterfalse", BuiltinFunction::ItertoolsFilterFalse),
                ("groupby", BuiltinFunction::ItertoolsGroupBy),
                ("islice", BuiltinFunction::ItertoolsISlice),
                ("pairwise", BuiltinFunction::ItertoolsPairwise),
                ("repeat", BuiltinFunction::ItertoolsRepeat),
                ("starmap", BuiltinFunction::ItertoolsStarMap),
                ("takewhile", BuiltinFunction::ItertoolsTakeWhile),
                ("tee", BuiltinFunction::ItertoolsTee),
                ("zip_longest", BuiltinFunction::ItertoolsZipLongest),
                ("batched", BuiltinFunction::ItertoolsBatched),
                ("permutations", BuiltinFunction::ItertoolsPermutations),
                ("product", BuiltinFunction::ItertoolsProduct),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "functools",
            &[
                ("reduce", BuiltinFunction::FunctoolsReduce),
                ("wraps", BuiltinFunction::FunctoolsWraps),
                ("partial", BuiltinFunction::FunctoolsPartial),
                ("partialmethod", BuiltinFunction::FunctoolsPartial),
                ("cmp_to_key", BuiltinFunction::FunctoolsCmpToKey),
                ("lru_cache", BuiltinFunction::FunctoolsLruCache),
                ("cache", BuiltinFunction::FunctoolsLruCache),
                ("cached_property", BuiltinFunction::FunctoolsCachedProperty),
                ("total_ordering", BuiltinFunction::TypingIdFunc),
                ("singledispatch", BuiltinFunction::FunctoolsSingleDispatch),
                (
                    "singledispatchmethod",
                    BuiltinFunction::FunctoolsSingleDispatchMethod,
                ),
            ],
            vec![
                (
                    "WRAPPER_ASSIGNMENTS",
                    self.heap.alloc_tuple(vec![
                        Value::Str("__module__".to_string()),
                        Value::Str("__name__".to_string()),
                        Value::Str("__qualname__".to_string()),
                        Value::Str("__doc__".to_string()),
                        Value::Str("__annotations__".to_string()),
                    ]),
                ),
                (
                    "WRAPPER_UPDATES",
                    self.heap
                        .alloc_tuple(vec![Value::Str("__dict__".to_string())]),
                ),
            ],
        );
        self.install_module_alias_from_existing("_functools", "functools");
        let typing_placeholder = self.heap.alloc_class(ClassObject::new(
            "typing.placeholder".to_string(),
            Vec::new(),
        ));
        let typing_generic_alias_class =
            self.alloc_bootstrap_class_value("_GenericAlias", "typing");
        self.install_builtin_module(
            "typing",
            &[
                ("assert_never", BuiltinFunction::TypingIdFunc),
                ("overload", BuiltinFunction::TypingIdFunc),
                ("final", BuiltinFunction::TypingIdFunc),
                ("assert_type", BuiltinFunction::TypingIdFunc),
                ("cast", BuiltinFunction::TypingIdFunc),
                ("runtime_checkable", BuiltinFunction::TypingIdFunc),
                ("override", BuiltinFunction::TypingIdFunc),
                ("reveal_type", BuiltinFunction::TypingIdFunc),
                ("dataclass_transform", BuiltinFunction::TypingIdFunc),
                ("no_type_check", BuiltinFunction::TypingIdFunc),
                ("no_type_check_decorator", BuiltinFunction::TypingIdFunc),
                ("TypeVar", BuiltinFunction::TypingTypeVar),
                ("ParamSpec", BuiltinFunction::TypingParamSpec),
                ("TypeVarTuple", BuiltinFunction::TypingTypeVarTuple),
                ("TypeAliasType", BuiltinFunction::TypingTypeAliasType),
                ("get_type_hints", BuiltinFunction::Dict),
                ("get_origin", BuiltinFunction::TypingIdFunc),
                ("get_args", BuiltinFunction::Tuple),
                ("get_protocol_members", BuiltinFunction::Tuple),
                ("get_overloads", BuiltinFunction::List),
                ("clear_overloads", BuiltinFunction::Print),
                ("is_typeddict", BuiltinFunction::Bool),
                ("is_protocol", BuiltinFunction::Bool),
            ],
            vec![
                ("TYPE_CHECKING", Value::Bool(false)),
                ("_cleanups", self.heap.alloc_list(Vec::new())),
                ("_ASSERT_NEVER_REPR_MAX_LENGTH", Value::Int(100)),
                ("Any", typing_placeholder.clone()),
                ("NoReturn", typing_placeholder.clone()),
                ("Never", typing_placeholder.clone()),
                ("Text", typing_placeholder.clone()),
                ("AnyStr", typing_placeholder.clone()),
                ("T", typing_placeholder.clone()),
                ("KT", typing_placeholder.clone()),
                ("VT", typing_placeholder.clone()),
                ("Union", typing_placeholder.clone()),
                ("Optional", typing_placeholder.clone()),
                ("Literal", typing_placeholder.clone()),
                ("Tuple", typing_placeholder.clone()),
                ("List", typing_placeholder.clone()),
                ("Dict", typing_placeholder.clone()),
                ("DefaultDict", typing_placeholder.clone()),
                ("MutableMapping", typing_placeholder.clone()),
                ("Callable", typing_placeholder.clone()),
                ("Iterable", typing_placeholder.clone()),
                ("Iterator", typing_placeholder.clone()),
                ("Collection", typing_placeholder.clone()),
                ("Generic", typing_placeholder.clone()),
                ("ClassVar", typing_placeholder.clone()),
                ("Final", typing_placeholder.clone()),
                ("Protocol", typing_placeholder.clone()),
                ("Type", typing_placeholder.clone()),
                ("NamedTuple", typing_placeholder.clone()),
                ("NotRequired", typing_placeholder.clone()),
                ("Required", typing_placeholder.clone()),
                ("ReadOnly", typing_placeholder.clone()),
                ("TypedDict", typing_placeholder.clone()),
                ("IO", typing_placeholder.clone()),
                ("TextIO", typing_placeholder.clone()),
                ("BinaryIO", typing_placeholder.clone()),
                ("Pattern", typing_placeholder.clone()),
                ("Match", typing_placeholder.clone()),
                ("Annotated", typing_placeholder.clone()),
                ("ForwardRef", typing_placeholder.clone()),
                ("Self", typing_placeholder.clone()),
                ("LiteralString", typing_placeholder.clone()),
                ("TypeAlias", typing_placeholder.clone()),
                ("_GenericAlias", typing_generic_alias_class.clone()),
                ("ParamSpecArgs", typing_placeholder.clone()),
                ("ParamSpecKwargs", typing_placeholder.clone()),
                ("Concatenate", typing_placeholder.clone()),
                ("Unpack", typing_placeholder.clone()),
                ("TypeGuard", typing_placeholder.clone()),
                ("TypeIs", typing_placeholder.clone()),
                ("NoDefault", typing_placeholder.clone()),
            ],
        );
        // Do not shadow CPython's pure-Python dataclasses implementation with a partial
        // built-in shim; we import from Lib/dataclasses.py for correctness.
        let deque_class = match self
            .heap
            .alloc_class(ClassObject::new("deque".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *deque_class.kind_mut() {
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str("collections".to_string()),
            );
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeInit),
            );
            class_data.attrs.insert(
                "append".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeAppend),
            );
            class_data.attrs.insert(
                "appendleft".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeAppendLeft),
            );
            class_data.attrs.insert(
                "pop".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequePop),
            );
            class_data.attrs.insert(
                "popleft".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequePopleft),
            );
            class_data.attrs.insert(
                "clear".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeClear),
            );
            class_data.attrs.insert(
                "extend".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeExtend),
            );
            class_data.attrs.insert(
                "extendleft".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeExtendLeft),
            );
            class_data.attrs.insert(
                "__len__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeLen),
            );
            class_data.attrs.insert(
                "__iter__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsDequeIter),
            );
        }
        let chain_map_class = match self
            .heap
            .alloc_class(ClassObject::new("ChainMap".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *chain_map_class.kind_mut() {
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str("collections".to_string()),
            );
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapInit),
            );
            class_data.attrs.insert(
                "new_child".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapNewChild),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapRepr),
            );
            class_data.attrs.insert(
                "items".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapItems),
            );
            class_data.attrs.insert(
                "get".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapGet),
            );
            class_data.attrs.insert(
                "__getitem__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapGetItem),
            );
            class_data.attrs.insert(
                "__setitem__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapSetItem),
            );
            class_data.attrs.insert(
                "__delitem__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsChainMapDelItem),
            );
        }
        let user_dict_class = match self
            .heap
            .alloc_class(ClassObject::new("UserDict".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *user_dict_class.kind_mut() {
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str("collections".to_string()),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsUserDictTypeRepr),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsUserDictTypeRepr),
            );
        }
        let user_list_bases = self
            .modules
            .get("builtins")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("list").cloned(),
                _ => None,
            })
            .and_then(|value| match value {
                Value::Class(class_obj) => Some(vec![class_obj]),
                _ => None,
            })
            .unwrap_or_else(|| {
                match self
                    .heap
                    .alloc_class(ClassObject::new("list".to_string(), Vec::new()))
                {
                    Value::Class(class_obj) => vec![class_obj],
                    _ => Vec::new(),
                }
            });
        let user_list_class = match self
            .heap
            .alloc_class(ClassObject::new("UserList".to_string(), user_list_bases))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *user_list_class.kind_mut() {
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str("collections".to_string()),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsUserListTypeRepr),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsUserListTypeRepr),
            );
        }
        let user_string_class = match self
            .heap
            .alloc_class(ClassObject::new("UserString".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *user_string_class.kind_mut() {
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str("collections".to_string()),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsUserStringTypeRepr),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::CollectionsUserStringTypeRepr),
            );
        }
        self.install_builtin_module(
            "collections",
            &[
                ("Counter", BuiltinFunction::CollectionsCounter),
                ("namedtuple", BuiltinFunction::CollectionsNamedTuple),
                ("defaultdict", BuiltinFunction::CollectionsDefaultDict),
                ("_count_elements", BuiltinFunction::CollectionsCountElements),
            ],
            vec![
                ("deque", Value::Class(deque_class)),
                ("ChainMap", Value::Class(chain_map_class)),
                (
                    "OrderedDict",
                    Value::Builtin(BuiltinFunction::CollectionsOrderedDict),
                ),
                ("UserDict", Value::Class(user_dict_class)),
                ("UserList", Value::Class(user_list_class)),
                ("UserString", Value::Class(user_string_class)),
            ],
        );
        self.install_module_alias_from_existing("_collections", "collections");
        self.install_builtin_module(
            "collections.abc",
            &[],
            vec![
                (
                    "Awaitable",
                    self.heap
                        .alloc_class(ClassObject::new("Awaitable".to_string(), Vec::new())),
                ),
                (
                    "Coroutine",
                    self.heap
                        .alloc_class(ClassObject::new("Coroutine".to_string(), Vec::new())),
                ),
                (
                    "AsyncIterator",
                    self.heap
                        .alloc_class(ClassObject::new("AsyncIterator".to_string(), Vec::new())),
                ),
                (
                    "AsyncIterable",
                    self.heap
                        .alloc_class(ClassObject::new("AsyncIterable".to_string(), Vec::new())),
                ),
                (
                    "AsyncGenerator",
                    self.heap
                        .alloc_class(ClassObject::new("AsyncGenerator".to_string(), Vec::new())),
                ),
                (
                    "Iterable",
                    self.heap
                        .alloc_class(ClassObject::new("Iterable".to_string(), Vec::new())),
                ),
                (
                    "Iterator",
                    self.heap
                        .alloc_class(ClassObject::new("Iterator".to_string(), Vec::new())),
                ),
                (
                    "Generator",
                    self.heap
                        .alloc_class(ClassObject::new("Generator".to_string(), Vec::new())),
                ),
                (
                    "Reversible",
                    self.heap
                        .alloc_class(ClassObject::new("Reversible".to_string(), Vec::new())),
                ),
                (
                    "Mapping",
                    self.heap
                        .alloc_class(ClassObject::new("Mapping".to_string(), Vec::new())),
                ),
                (
                    "MutableMapping",
                    self.heap
                        .alloc_class(ClassObject::new("MutableMapping".to_string(), Vec::new())),
                ),
                (
                    "KeysView",
                    self.heap
                        .alloc_class(ClassObject::new("KeysView".to_string(), Vec::new())),
                ),
                (
                    "ItemsView",
                    self.heap
                        .alloc_class(ClassObject::new("ItemsView".to_string(), Vec::new())),
                ),
                (
                    "ValuesView",
                    self.heap
                        .alloc_class(ClassObject::new("ValuesView".to_string(), Vec::new())),
                ),
                (
                    "Sequence",
                    self.heap
                        .alloc_class(ClassObject::new("Sequence".to_string(), Vec::new())),
                ),
                (
                    "MutableSequence",
                    self.heap
                        .alloc_class(ClassObject::new("MutableSequence".to_string(), Vec::new())),
                ),
                (
                    "Set",
                    self.heap
                        .alloc_class(ClassObject::new("Set".to_string(), Vec::new())),
                ),
                (
                    "MutableSet",
                    self.heap
                        .alloc_class(ClassObject::new("MutableSet".to_string(), Vec::new())),
                ),
                (
                    "Callable",
                    self.heap
                        .alloc_class(ClassObject::new("Callable".to_string(), Vec::new())),
                ),
                (
                    "Collection",
                    self.heap
                        .alloc_class(ClassObject::new("Collection".to_string(), Vec::new())),
                ),
                (
                    "Hashable",
                    self.heap
                        .alloc_class(ClassObject::new("Hashable".to_string(), Vec::new())),
                ),
                (
                    "Container",
                    self.heap
                        .alloc_class(ClassObject::new("Container".to_string(), Vec::new())),
                ),
                (
                    "Sized",
                    self.heap
                        .alloc_class(ClassObject::new("Sized".to_string(), Vec::new())),
                ),
                (
                    "ByteString",
                    self.heap
                        .alloc_class(ClassObject::new("ByteString".to_string(), Vec::new())),
                ),
                (
                    "Buffer",
                    self.heap
                        .alloc_class(ClassObject::new("Buffer".to_string(), Vec::new())),
                ),
            ],
        );
        let simple_namespace_class = self
            .heap
            .alloc_class(ClassObject::new("SimpleNamespace".to_string(), Vec::new()));
        let function_type_class = self
            .heap
            .alloc_class(ClassObject::new("function".to_string(), Vec::new()));
        let builtin_function_type_class = self.heap.alloc_class(ClassObject::new(
            "builtin_function_or_method".to_string(),
            Vec::new(),
        ));
        let code_type_class = self
            .heap
            .alloc_class(ClassObject::new("code".to_string(), Vec::new()));
        let generic_alias_class = self.ensure_generic_alias_class();
        if let Value::Class(class_obj) = &simple_namespace_class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::SimpleNamespaceTypeRepr),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::SimpleNamespaceTypeRepr),
            );
        }
        if let Value::Class(class_obj) = &function_type_class
            && let Object::Class(class_data) = &mut *class_obj.kind_mut()
        {
            class_data.attrs.insert(
                "__new__".to_string(),
                Value::Builtin(BuiltinFunction::TypesFunctionType),
            );
        }
        self.install_builtin_module(
            "types",
            &[
                ("ModuleType", BuiltinFunction::TypesModuleType),
                ("MappingProxyType", BuiltinFunction::TypesMappingProxy),
                ("MethodType", BuiltinFunction::TypesMethodType),
                ("new_class", BuiltinFunction::TypesNewClass),
                ("coroutine", BuiltinFunction::TypesCoroutine),
            ],
            vec![
                (
                    "DynamicClassAttribute",
                    Value::Builtin(BuiltinFunction::Property),
                ),
                ("FunctionType", function_type_class.clone()),
                ("LambdaType", function_type_class),
                ("BuiltinFunctionType", builtin_function_type_class.clone()),
                ("BuiltinMethodType", builtin_function_type_class),
                ("CodeType", code_type_class),
                (
                    "NoneType",
                    self.heap
                        .alloc_class(ClassObject::new("NoneType".to_string(), Vec::new())),
                ),
                ("EllipsisType", self.heap.ellipsis_type()),
                (
                    "NotImplementedType",
                    self.heap.alloc_class(ClassObject::new(
                        "NotImplementedType".to_string(),
                        Vec::new(),
                    )),
                ),
                ("SimpleNamespace", simple_namespace_class),
                ("GenericAlias", Value::Class(generic_alias_class.clone())),
                (
                    "UnionType",
                    self.heap
                        .alloc_class(ClassObject::new("UnionType".to_string(), Vec::new())),
                ),
                (
                    "MemberDescriptorType",
                    self.heap.alloc_class(ClassObject::new(
                        "member_descriptor".to_string(),
                        Vec::new(),
                    )),
                ),
                (
                    "CellType",
                    self.heap
                        .alloc_class(ClassObject::new("cell".to_string(), Vec::new())),
                ),
                (
                    "GeneratorType",
                    Value::Builtin(BuiltinFunction::GeneratorType),
                ),
                (
                    "CoroutineType",
                    Value::Builtin(BuiltinFunction::CoroutineType),
                ),
                (
                    "AsyncGeneratorType",
                    Value::Builtin(BuiltinFunction::AsyncGeneratorType),
                ),
                (
                    "WrapperDescriptorType",
                    self.heap.alloc_class(ClassObject::new(
                        "wrapper_descriptor".to_string(),
                        Vec::new(),
                    )),
                ),
                (
                    "MethodWrapperType",
                    self.heap
                        .alloc_class(ClassObject::new("method-wrapper".to_string(), Vec::new())),
                ),
                (
                    "MethodDescriptorType",
                    self.heap.alloc_class(ClassObject::new(
                        "method_descriptor".to_string(),
                        Vec::new(),
                    )),
                ),
                (
                    "ClassMethodDescriptorType",
                    self.heap.alloc_class(ClassObject::new(
                        "classmethod_descriptor".to_string(),
                        Vec::new(),
                    )),
                ),
                (
                    "TracebackType",
                    self.heap
                        .alloc_class(ClassObject::new("traceback".to_string(), Vec::new())),
                ),
                (
                    "FrameType",
                    self.heap
                        .alloc_class(ClassObject::new("frame".to_string(), Vec::new())),
                ),
                (
                    "GetSetDescriptorType",
                    self.heap.alloc_class(ClassObject::new(
                        "getset_descriptor".to_string(),
                        Vec::new(),
                    )),
                ),
            ],
        );
        if let Some(types_module) = self.modules.get("types").cloned() {
            let types_alias = match self.heap.alloc_module(ModuleObject::new("_types")) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            self.set_module_metadata(
                &types_alias,
                "_types",
                None,
                None,
                Some(BUILTIN_MODULE_LOADER),
                false,
                Vec::new(),
                false,
            );
            let exported = if let Object::Module(types_data) = &*types_module.kind() {
                Some(types_data.globals.clone())
            } else {
                None
            };
            if let Some(exported) = exported
                && let Object::Module(alias_data) = &mut *types_alias.kind_mut()
            {
                alias_data.globals.extend(exported);
                alias_data
                    .globals
                    .insert("__name__".to_string(), Value::Str("_types".to_string()));
            }
            self.register_module("_types", types_alias);
        }
        self.install_builtin_module(
            "_thread",
            &[
                ("RLock", BuiltinFunction::ThreadRLock),
                ("allocate_lock", BuiltinFunction::ThreadRLock),
                ("get_ident", BuiltinFunction::ThreadingGetIdent),
                ("_count", BuiltinFunction::ThreadingActiveCount),
                ("start_new_thread", BuiltinFunction::ThreadStartNewThread),
            ],
            vec![("TIMEOUT_MAX", Value::Float(f64::MAX))],
        );
        self.install_builtin_module(
            "__future__",
            &[],
            vec![
                ("all_feature_names", self.heap.alloc_list(Vec::new())),
                (
                    "__all__",
                    self.heap
                        .alloc_list(vec![Value::Str("all_feature_names".to_string())]),
                ),
                ("annotations", Value::None),
                ("nested_scopes", Value::None),
                ("generators", Value::None),
                ("division", Value::None),
                ("absolute_import", Value::None),
                ("with_statement", Value::None),
                ("print_function", Value::None),
                ("unicode_literals", Value::None),
                ("generator_stop", Value::None),
                ("barry_as_FLUFL", Value::None),
            ],
        );
        self.install_builtin_module(
            "_contextvars",
            &[
                ("ContextVar", BuiltinFunction::ContextVar),
                ("copy_context", BuiltinFunction::ContextCopyContext),
            ],
            vec![
                (
                    "Context",
                    self.heap
                        .alloc_class(ClassObject::new("Context".to_string(), Vec::new())),
                ),
                (
                    "Token",
                    self.heap
                        .alloc_class(ClassObject::new("Token".to_string(), Vec::new())),
                ),
            ],
        );
        self.install_builtin_module(
            "atexit",
            &[
                ("register", BuiltinFunction::AtexitRegister),
                ("unregister", BuiltinFunction::AtexitUnregister),
                ("_run_exitfuncs", BuiltinFunction::AtexitRunExitFuncs),
                ("_clear", BuiltinFunction::AtexitClear),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "_tokenize",
            &[("TokenizerIter", BuiltinFunction::TokenizeTokenizerIter)],
            Vec::new(),
        );
        let struct_class = match self
            .heap
            .alloc_class(ClassObject::new("Struct".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *struct_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::StructClassInit),
            );
            class_data.attrs.insert(
                "pack".to_string(),
                Value::Builtin(BuiltinFunction::StructClassPack),
            );
            class_data.attrs.insert(
                "unpack".to_string(),
                Value::Builtin(BuiltinFunction::StructClassUnpack),
            );
            class_data.attrs.insert(
                "iter_unpack".to_string(),
                Value::Builtin(BuiltinFunction::StructClassIterUnpack),
            );
            class_data.attrs.insert(
                "pack_into".to_string(),
                Value::Builtin(BuiltinFunction::StructClassPackInto),
            );
            class_data.attrs.insert(
                "unpack_from".to_string(),
                Value::Builtin(BuiltinFunction::StructClassUnpackFrom),
            );
        }
        self.install_builtin_module(
            "_struct",
            &[
                ("calcsize", BuiltinFunction::StructCalcSize),
                ("pack", BuiltinFunction::StructPack),
                ("unpack", BuiltinFunction::StructUnpack),
                ("iter_unpack", BuiltinFunction::StructIterUnpack),
                ("pack_into", BuiltinFunction::StructPackInto),
                ("unpack_from", BuiltinFunction::StructUnpackFrom),
                ("_clearcache", BuiltinFunction::StructClearCache),
            ],
            vec![
                ("Struct", Value::Class(struct_class)),
                ("error", Value::ExceptionType("Exception".to_string())),
                ("__doc__", Value::Str("pyrs _struct stub".to_string())),
            ],
        );
        self.install_builtin_module(
            "_imp",
            &[
                ("acquire_lock", BuiltinFunction::ImpAcquireLock),
                ("release_lock", BuiltinFunction::ImpReleaseLock),
                ("lock_held", BuiltinFunction::ImpLockHeld),
                ("is_builtin", BuiltinFunction::ImpIsBuiltin),
                ("is_frozen", BuiltinFunction::ImpIsFrozen),
                ("is_frozen_package", BuiltinFunction::ImpIsFrozenPackage),
                ("find_frozen", BuiltinFunction::ImpFindFrozen),
                ("get_frozen_object", BuiltinFunction::ImpGetFrozenObject),
                ("create_builtin", BuiltinFunction::ImpCreateBuiltin),
                ("exec_builtin", BuiltinFunction::ImpExecBuiltin),
                ("create_dynamic", BuiltinFunction::ImpCreateDynamic),
                ("exec_dynamic", BuiltinFunction::ImpExecDynamic),
                ("extension_suffixes", BuiltinFunction::ImpExtensionSuffixes),
                ("source_hash", BuiltinFunction::ImpSourceHash),
                ("_fix_co_filename", BuiltinFunction::ImpFixCoFilename),
                (
                    "_override_frozen_modules_for_tests",
                    BuiltinFunction::ImpOverrideFrozenModulesForTests,
                ),
                (
                    "_override_multi_interp_extensions_check",
                    BuiltinFunction::ImpOverrideMultiInterpExtensionsCheck,
                ),
                (
                    "_frozen_module_names",
                    BuiltinFunction::ImpFrozenModuleNames,
                ),
            ],
            vec![
                ("pyc_magic_number_token", Value::Int(3600)),
                ("check_hash_based_pycs", Value::Str("default".to_string())),
            ],
        );
        let typing_paramspec_args_class =
            self.alloc_bootstrap_class_value("ParamSpecArgs", "_typing");
        let typing_paramspec_kwargs_class =
            self.alloc_bootstrap_class_value("ParamSpecKwargs", "_typing");
        let typing_generic_class = self.alloc_bootstrap_class_value("Generic", "_typing");
        let typing_union_class = self.alloc_bootstrap_class_value("Union", "_typing");
        let typing_nodefault_class = self.alloc_bootstrap_class_value("NoDefault", "_typing");
        self.install_builtin_module(
            "_typing",
            &[
                ("_idfunc", BuiltinFunction::TypingIdFunc),
                ("TypeVar", BuiltinFunction::TypingTypeVar),
                ("ParamSpec", BuiltinFunction::TypingParamSpec),
                ("TypeVarTuple", BuiltinFunction::TypingTypeVarTuple),
                ("TypeAliasType", BuiltinFunction::TypingTypeAliasType),
            ],
            vec![
                ("ParamSpecArgs", typing_paramspec_args_class),
                ("ParamSpecKwargs", typing_paramspec_kwargs_class),
                ("Generic", typing_generic_class),
                ("Union", typing_union_class),
                ("NoDefault", typing_nodefault_class),
            ],
        );
        self.install_builtin_module(
            "_ast",
            &[],
            vec![
                (
                    "AST",
                    self.heap
                        .alloc_class(ClassObject::new("AST".to_string(), Vec::new())),
                ),
                (
                    "Expression",
                    self.heap
                        .alloc_class(ClassObject::new("Expression".to_string(), Vec::new())),
                ),
                (
                    "Module",
                    self.heap
                        .alloc_class(ClassObject::new("Module".to_string(), Vec::new())),
                ),
                (
                    "FunctionDef",
                    self.heap
                        .alloc_class(ClassObject::new("FunctionDef".to_string(), Vec::new())),
                ),
                (
                    "AsyncFunctionDef",
                    self.heap
                        .alloc_class(ClassObject::new("AsyncFunctionDef".to_string(), Vec::new())),
                ),
                (
                    "ClassDef",
                    self.heap
                        .alloc_class(ClassObject::new("ClassDef".to_string(), Vec::new())),
                ),
                (
                    "TypeAlias",
                    self.heap
                        .alloc_class(ClassObject::new("TypeAlias".to_string(), Vec::new())),
                ),
                (
                    "mod",
                    self.heap
                        .alloc_class(ClassObject::new("mod".to_string(), Vec::new())),
                ),
                (
                    "stmt",
                    self.heap
                        .alloc_class(ClassObject::new("stmt".to_string(), Vec::new())),
                ),
                (
                    "expr",
                    self.heap
                        .alloc_class(ClassObject::new("expr".to_string(), Vec::new())),
                ),
                (
                    "expr_context",
                    self.heap
                        .alloc_class(ClassObject::new("expr_context".to_string(), Vec::new())),
                ),
                (
                    "operator",
                    self.heap
                        .alloc_class(ClassObject::new("operator".to_string(), Vec::new())),
                ),
                (
                    "unaryop",
                    self.heap
                        .alloc_class(ClassObject::new("unaryop".to_string(), Vec::new())),
                ),
                (
                    "boolop",
                    self.heap
                        .alloc_class(ClassObject::new("boolop".to_string(), Vec::new())),
                ),
                (
                    "cmpop",
                    self.heap
                        .alloc_class(ClassObject::new("cmpop".to_string(), Vec::new())),
                ),
                (
                    "arguments",
                    self.heap
                        .alloc_class(ClassObject::new("arguments".to_string(), Vec::new())),
                ),
                (
                    "arg",
                    self.heap
                        .alloc_class(ClassObject::new("arg".to_string(), Vec::new())),
                ),
                (
                    "type_param",
                    self.heap
                        .alloc_class(ClassObject::new("type_param".to_string(), Vec::new())),
                ),
                (
                    "TypeVar",
                    self.heap
                        .alloc_class(ClassObject::new("TypeVar".to_string(), Vec::new())),
                ),
                (
                    "ParamSpec",
                    self.heap
                        .alloc_class(ClassObject::new("ParamSpec".to_string(), Vec::new())),
                ),
                (
                    "TypeVarTuple",
                    self.heap
                        .alloc_class(ClassObject::new("TypeVarTuple".to_string(), Vec::new())),
                ),
                (
                    "Expr",
                    self.heap
                        .alloc_class(ClassObject::new("Expr".to_string(), Vec::new())),
                ),
                (
                    "Assign",
                    self.heap
                        .alloc_class(ClassObject::new("Assign".to_string(), Vec::new())),
                ),
                (
                    "AugAssign",
                    self.heap
                        .alloc_class(ClassObject::new("AugAssign".to_string(), Vec::new())),
                ),
                (
                    "AnnAssign",
                    self.heap
                        .alloc_class(ClassObject::new("AnnAssign".to_string(), Vec::new())),
                ),
                (
                    "Delete",
                    self.heap
                        .alloc_class(ClassObject::new("Delete".to_string(), Vec::new())),
                ),
                (
                    "Return",
                    self.heap
                        .alloc_class(ClassObject::new("Return".to_string(), Vec::new())),
                ),
                (
                    "Raise",
                    self.heap
                        .alloc_class(ClassObject::new("Raise".to_string(), Vec::new())),
                ),
                (
                    "Assert",
                    self.heap
                        .alloc_class(ClassObject::new("Assert".to_string(), Vec::new())),
                ),
                (
                    "If",
                    self.heap
                        .alloc_class(ClassObject::new("If".to_string(), Vec::new())),
                ),
                (
                    "While",
                    self.heap
                        .alloc_class(ClassObject::new("While".to_string(), Vec::new())),
                ),
                (
                    "For",
                    self.heap
                        .alloc_class(ClassObject::new("For".to_string(), Vec::new())),
                ),
                (
                    "AsyncFor",
                    self.heap
                        .alloc_class(ClassObject::new("AsyncFor".to_string(), Vec::new())),
                ),
                (
                    "With",
                    self.heap
                        .alloc_class(ClassObject::new("With".to_string(), Vec::new())),
                ),
                (
                    "AsyncWith",
                    self.heap
                        .alloc_class(ClassObject::new("AsyncWith".to_string(), Vec::new())),
                ),
                (
                    "Try",
                    self.heap
                        .alloc_class(ClassObject::new("Try".to_string(), Vec::new())),
                ),
                (
                    "TryStar",
                    self.heap
                        .alloc_class(ClassObject::new("TryStar".to_string(), Vec::new())),
                ),
                (
                    "ExceptHandler",
                    self.heap
                        .alloc_class(ClassObject::new("ExceptHandler".to_string(), Vec::new())),
                ),
                (
                    "excepthandler",
                    self.heap
                        .alloc_class(ClassObject::new("excepthandler".to_string(), Vec::new())),
                ),
                (
                    "Import",
                    self.heap
                        .alloc_class(ClassObject::new("Import".to_string(), Vec::new())),
                ),
                (
                    "ImportFrom",
                    self.heap
                        .alloc_class(ClassObject::new("ImportFrom".to_string(), Vec::new())),
                ),
                (
                    "Global",
                    self.heap
                        .alloc_class(ClassObject::new("Global".to_string(), Vec::new())),
                ),
                (
                    "Nonlocal",
                    self.heap
                        .alloc_class(ClassObject::new("Nonlocal".to_string(), Vec::new())),
                ),
                (
                    "Constant",
                    self.heap
                        .alloc_class(ClassObject::new("Constant".to_string(), Vec::new())),
                ),
                (
                    "Pass",
                    self.heap
                        .alloc_class(ClassObject::new("Pass".to_string(), Vec::new())),
                ),
                (
                    "Break",
                    self.heap
                        .alloc_class(ClassObject::new("Break".to_string(), Vec::new())),
                ),
                (
                    "Continue",
                    self.heap
                        .alloc_class(ClassObject::new("Continue".to_string(), Vec::new())),
                ),
                (
                    "Match",
                    self.heap
                        .alloc_class(ClassObject::new("Match".to_string(), Vec::new())),
                ),
                (
                    "match_case",
                    self.heap
                        .alloc_class(ClassObject::new("match_case".to_string(), Vec::new())),
                ),
                (
                    "pattern",
                    self.heap
                        .alloc_class(ClassObject::new("pattern".to_string(), Vec::new())),
                ),
                (
                    "MatchValue",
                    self.heap
                        .alloc_class(ClassObject::new("MatchValue".to_string(), Vec::new())),
                ),
                (
                    "MatchSingleton",
                    self.heap
                        .alloc_class(ClassObject::new("MatchSingleton".to_string(), Vec::new())),
                ),
                (
                    "MatchSequence",
                    self.heap
                        .alloc_class(ClassObject::new("MatchSequence".to_string(), Vec::new())),
                ),
                (
                    "MatchMapping",
                    self.heap
                        .alloc_class(ClassObject::new("MatchMapping".to_string(), Vec::new())),
                ),
                (
                    "MatchClass",
                    self.heap
                        .alloc_class(ClassObject::new("MatchClass".to_string(), Vec::new())),
                ),
                (
                    "MatchStar",
                    self.heap
                        .alloc_class(ClassObject::new("MatchStar".to_string(), Vec::new())),
                ),
                (
                    "MatchAs",
                    self.heap
                        .alloc_class(ClassObject::new("MatchAs".to_string(), Vec::new())),
                ),
                (
                    "MatchOr",
                    self.heap
                        .alloc_class(ClassObject::new("MatchOr".to_string(), Vec::new())),
                ),
                (
                    "alias",
                    self.heap
                        .alloc_class(ClassObject::new("alias".to_string(), Vec::new())),
                ),
                (
                    "withitem",
                    self.heap
                        .alloc_class(ClassObject::new("withitem".to_string(), Vec::new())),
                ),
                (
                    "Tuple",
                    self.heap
                        .alloc_class(ClassObject::new("Tuple".to_string(), Vec::new())),
                ),
                (
                    "List",
                    self.heap
                        .alloc_class(ClassObject::new("List".to_string(), Vec::new())),
                ),
                (
                    "Set",
                    self.heap
                        .alloc_class(ClassObject::new("Set".to_string(), Vec::new())),
                ),
                (
                    "Dict",
                    self.heap
                        .alloc_class(ClassObject::new("Dict".to_string(), Vec::new())),
                ),
                (
                    "Call",
                    self.heap
                        .alloc_class(ClassObject::new("Call".to_string(), Vec::new())),
                ),
                (
                    "Name",
                    self.heap
                        .alloc_class(ClassObject::new("Name".to_string(), Vec::new())),
                ),
                (
                    "Load",
                    self.heap
                        .alloc_class(ClassObject::new("Load".to_string(), Vec::new())),
                ),
                (
                    "Store",
                    self.heap
                        .alloc_class(ClassObject::new("Store".to_string(), Vec::new())),
                ),
                (
                    "Del",
                    self.heap
                        .alloc_class(ClassObject::new("Del".to_string(), Vec::new())),
                ),
                (
                    "Attribute",
                    self.heap
                        .alloc_class(ClassObject::new("Attribute".to_string(), Vec::new())),
                ),
                (
                    "BinOp",
                    self.heap
                        .alloc_class(ClassObject::new("BinOp".to_string(), Vec::new())),
                ),
                (
                    "UnaryOp",
                    self.heap
                        .alloc_class(ClassObject::new("UnaryOp".to_string(), Vec::new())),
                ),
                (
                    "BoolOp",
                    self.heap
                        .alloc_class(ClassObject::new("BoolOp".to_string(), Vec::new())),
                ),
                (
                    "IfExp",
                    self.heap
                        .alloc_class(ClassObject::new("IfExp".to_string(), Vec::new())),
                ),
                (
                    "NamedExpr",
                    self.heap
                        .alloc_class(ClassObject::new("NamedExpr".to_string(), Vec::new())),
                ),
                (
                    "Lambda",
                    self.heap
                        .alloc_class(ClassObject::new("Lambda".to_string(), Vec::new())),
                ),
                (
                    "Await",
                    self.heap
                        .alloc_class(ClassObject::new("Await".to_string(), Vec::new())),
                ),
                (
                    "ListComp",
                    self.heap
                        .alloc_class(ClassObject::new("ListComp".to_string(), Vec::new())),
                ),
                (
                    "SetComp",
                    self.heap
                        .alloc_class(ClassObject::new("SetComp".to_string(), Vec::new())),
                ),
                (
                    "DictComp",
                    self.heap
                        .alloc_class(ClassObject::new("DictComp".to_string(), Vec::new())),
                ),
                (
                    "GeneratorExp",
                    self.heap
                        .alloc_class(ClassObject::new("GeneratorExp".to_string(), Vec::new())),
                ),
                (
                    "Yield",
                    self.heap
                        .alloc_class(ClassObject::new("Yield".to_string(), Vec::new())),
                ),
                (
                    "YieldFrom",
                    self.heap
                        .alloc_class(ClassObject::new("YieldFrom".to_string(), Vec::new())),
                ),
                (
                    "Subscript",
                    self.heap
                        .alloc_class(ClassObject::new("Subscript".to_string(), Vec::new())),
                ),
                (
                    "Slice",
                    self.heap
                        .alloc_class(ClassObject::new("Slice".to_string(), Vec::new())),
                ),
                (
                    "Starred",
                    self.heap
                        .alloc_class(ClassObject::new("Starred".to_string(), Vec::new())),
                ),
                (
                    "Compare",
                    self.heap
                        .alloc_class(ClassObject::new("Compare".to_string(), Vec::new())),
                ),
                (
                    "Interpolation",
                    self.heap
                        .alloc_class(ClassObject::new("Interpolation".to_string(), Vec::new())),
                ),
                (
                    "TemplateStr",
                    self.heap
                        .alloc_class(ClassObject::new("TemplateStr".to_string(), Vec::new())),
                ),
                (
                    "keyword",
                    self.heap
                        .alloc_class(ClassObject::new("keyword".to_string(), Vec::new())),
                ),
                (
                    "comprehension",
                    self.heap
                        .alloc_class(ClassObject::new("comprehension".to_string(), Vec::new())),
                ),
                (
                    "And",
                    self.heap
                        .alloc_class(ClassObject::new("And".to_string(), Vec::new())),
                ),
                (
                    "Or",
                    self.heap
                        .alloc_class(ClassObject::new("Or".to_string(), Vec::new())),
                ),
                (
                    "Not",
                    self.heap
                        .alloc_class(ClassObject::new("Not".to_string(), Vec::new())),
                ),
                (
                    "Add",
                    self.heap
                        .alloc_class(ClassObject::new("Add".to_string(), Vec::new())),
                ),
                (
                    "Sub",
                    self.heap
                        .alloc_class(ClassObject::new("Sub".to_string(), Vec::new())),
                ),
                (
                    "Mult",
                    self.heap
                        .alloc_class(ClassObject::new("Mult".to_string(), Vec::new())),
                ),
                (
                    "MatMult",
                    self.heap
                        .alloc_class(ClassObject::new("MatMult".to_string(), Vec::new())),
                ),
                (
                    "Div",
                    self.heap
                        .alloc_class(ClassObject::new("Div".to_string(), Vec::new())),
                ),
                (
                    "FloorDiv",
                    self.heap
                        .alloc_class(ClassObject::new("FloorDiv".to_string(), Vec::new())),
                ),
                (
                    "Mod",
                    self.heap
                        .alloc_class(ClassObject::new("Mod".to_string(), Vec::new())),
                ),
                (
                    "Pow",
                    self.heap
                        .alloc_class(ClassObject::new("Pow".to_string(), Vec::new())),
                ),
                (
                    "LShift",
                    self.heap
                        .alloc_class(ClassObject::new("LShift".to_string(), Vec::new())),
                ),
                (
                    "RShift",
                    self.heap
                        .alloc_class(ClassObject::new("RShift".to_string(), Vec::new())),
                ),
                (
                    "BitAnd",
                    self.heap
                        .alloc_class(ClassObject::new("BitAnd".to_string(), Vec::new())),
                ),
                (
                    "BitOr",
                    self.heap
                        .alloc_class(ClassObject::new("BitOr".to_string(), Vec::new())),
                ),
                (
                    "BitXor",
                    self.heap
                        .alloc_class(ClassObject::new("BitXor".to_string(), Vec::new())),
                ),
                (
                    "UAdd",
                    self.heap
                        .alloc_class(ClassObject::new("UAdd".to_string(), Vec::new())),
                ),
                (
                    "USub",
                    self.heap
                        .alloc_class(ClassObject::new("USub".to_string(), Vec::new())),
                ),
                (
                    "Invert",
                    self.heap
                        .alloc_class(ClassObject::new("Invert".to_string(), Vec::new())),
                ),
                (
                    "Is",
                    self.heap
                        .alloc_class(ClassObject::new("Is".to_string(), Vec::new())),
                ),
                (
                    "IsNot",
                    self.heap
                        .alloc_class(ClassObject::new("IsNot".to_string(), Vec::new())),
                ),
                (
                    "In",
                    self.heap
                        .alloc_class(ClassObject::new("In".to_string(), Vec::new())),
                ),
                (
                    "NotIn",
                    self.heap
                        .alloc_class(ClassObject::new("NotIn".to_string(), Vec::new())),
                ),
                (
                    "Eq",
                    self.heap
                        .alloc_class(ClassObject::new("Eq".to_string(), Vec::new())),
                ),
                (
                    "NotEq",
                    self.heap
                        .alloc_class(ClassObject::new("NotEq".to_string(), Vec::new())),
                ),
                (
                    "Lt",
                    self.heap
                        .alloc_class(ClassObject::new("Lt".to_string(), Vec::new())),
                ),
                (
                    "LtE",
                    self.heap
                        .alloc_class(ClassObject::new("LtE".to_string(), Vec::new())),
                ),
                (
                    "Gt",
                    self.heap
                        .alloc_class(ClassObject::new("Gt".to_string(), Vec::new())),
                ),
                (
                    "GtE",
                    self.heap
                        .alloc_class(ClassObject::new("GtE".to_string(), Vec::new())),
                ),
                ("PyCF_ONLY_AST", Value::Int(1024)),
                ("PyCF_TYPE_COMMENTS", Value::Int(4096)),
                ("PyCF_OPTIMIZED_AST", Value::Int(32768)),
            ],
        );
        self.configure_bootstrap_ast_metadata();
        self.wire_ast_class_hierarchy();
        self.install_builtin_module(
            "_opcode",
            &[
                ("stack_effect", BuiltinFunction::OpcodeStackEffect),
                ("has_arg", BuiltinFunction::OpcodeHasArg),
                ("has_const", BuiltinFunction::OpcodeHasConst),
                ("has_name", BuiltinFunction::OpcodeHasName),
                ("has_jump", BuiltinFunction::OpcodeHasJump),
                ("has_free", BuiltinFunction::OpcodeHasFree),
                ("has_local", BuiltinFunction::OpcodeHasLocal),
                ("has_exc", BuiltinFunction::OpcodeHasExc),
                ("get_intrinsic1_descs", BuiltinFunction::List),
                ("get_intrinsic2_descs", BuiltinFunction::List),
                ("get_special_method_names", BuiltinFunction::List),
                ("get_nb_ops", BuiltinFunction::List),
                ("get_executor", BuiltinFunction::OpcodeGetExecutor),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "pkgutil",
            &[
                ("get_data", BuiltinFunction::PkgutilGetData),
                ("iter_modules", BuiltinFunction::PkgutilIterModules),
                ("walk_packages", BuiltinFunction::PkgutilWalkPackages),
                ("resolve_name", BuiltinFunction::PkgutilResolveName),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "_abc",
            &[
                ("get_cache_token", BuiltinFunction::AbcGetCacheToken),
                ("_abc_init", BuiltinFunction::AbcInit),
                ("_abc_register", BuiltinFunction::AbcRegister),
                ("_abc_instancecheck", BuiltinFunction::AbcInstanceCheck),
                ("_abc_subclasscheck", BuiltinFunction::AbcSubclassCheck),
                ("_get_dump", BuiltinFunction::AbcGetDump),
                ("_reset_registry", BuiltinFunction::AbcResetRegistry),
                ("_reset_caches", BuiltinFunction::AbcResetCaches),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "gc",
            &[
                ("collect", BuiltinFunction::GcCollect),
                ("enable", BuiltinFunction::GcEnable),
                ("disable", BuiltinFunction::GcDisable),
                ("isenabled", BuiltinFunction::GcIsEnabled),
                ("get_threshold", BuiltinFunction::GcGetThreshold),
                ("set_threshold", BuiltinFunction::GcSetThreshold),
                ("get_count", BuiltinFunction::GcGetCount),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "_weakref",
            &[
                ("ref", BuiltinFunction::WeakRefRef),
                ("proxy", BuiltinFunction::WeakRefProxy),
                ("getweakrefcount", BuiltinFunction::WeakRefGetWeakRefCount),
                ("getweakrefs", BuiltinFunction::WeakRefGetWeakRefs),
                ("_remove_dead_weakref", BuiltinFunction::WeakRefRemoveDead),
            ],
            vec![
                ("ReferenceType", Value::Builtin(BuiltinFunction::Type)),
                ("ProxyType", Value::Builtin(BuiltinFunction::Type)),
                ("CallableProxyType", Value::Builtin(BuiltinFunction::Type)),
            ],
        );
        self.install_builtin_module(
            "weakref",
            &[
                ("ref", BuiltinFunction::WeakRefRef),
                ("proxy", BuiltinFunction::WeakRefProxy),
                ("finalize", BuiltinFunction::WeakRefFinalize),
                ("getweakrefcount", BuiltinFunction::WeakRefGetWeakRefCount),
                ("getweakrefs", BuiltinFunction::WeakRefGetWeakRefs),
            ],
            vec![
                ("ReferenceType", Value::Builtin(BuiltinFunction::Type)),
                ("ProxyType", Value::Builtin(BuiltinFunction::Type)),
                ("CallableProxyType", Value::Builtin(BuiltinFunction::Type)),
                ("WeakSet", Value::Builtin(BuiltinFunction::Set)),
                ("WeakKeyDictionary", Value::Builtin(BuiltinFunction::Dict)),
                ("WeakValueDictionary", Value::Builtin(BuiltinFunction::Dict)),
                (
                    "ProxyTypes",
                    self.heap.alloc_tuple(vec![
                        Value::Builtin(BuiltinFunction::Type),
                        Value::Builtin(BuiltinFunction::Type),
                    ]),
                ),
            ],
        );
        self.install_builtin_module(
            "_weakrefset",
            &[],
            vec![("WeakSet", Value::Builtin(BuiltinFunction::Set))],
        );
        self.install_builtin_module(
            "array",
            &[("array", BuiltinFunction::ArrayArray)],
            vec![("typecodes", Value::Str("bBuhHiIlLqQfdw".to_string()))],
        );
        let errno_constants = vec![
            ("EPERM", 1),
            ("ENOENT", 2),
            ("ESRCH", 3),
            ("EINTR", 4),
            ("EIO", 5),
            ("ENXIO", 6),
            ("E2BIG", 7),
            ("ENOEXEC", 8),
            ("EBADF", 9),
            ("ECHILD", 10),
            ("EDEADLK", 11),
            ("ENOMEM", 12),
            ("EACCES", 13),
            ("EFAULT", 14),
            ("ENOTBLK", 15),
            ("EBUSY", 16),
            ("EEXIST", 17),
            ("EXDEV", 18),
            ("ENODEV", 19),
            ("ENOTDIR", 20),
            ("EISDIR", 21),
            ("EINVAL", 22),
            ("ENFILE", 23),
            ("EMFILE", 24),
            ("ENOTTY", 25),
            ("ETXTBSY", 26),
            ("EFBIG", 27),
            ("ENOSPC", 28),
            ("ESPIPE", 29),
            ("EROFS", 30),
            ("EMLINK", 31),
            ("EPIPE", 32),
            ("EDOM", 33),
            ("ERANGE", 34),
            ("EAGAIN", 35),
            ("EWOULDBLOCK", 35),
            ("EINPROGRESS", 36),
            ("EALREADY", 37),
            ("ENOTSOCK", 38),
            ("EDESTADDRREQ", 39),
            ("EMSGSIZE", 40),
            ("EPROTOTYPE", 41),
            ("ENOPROTOOPT", 42),
            ("EPROTONOSUPPORT", 43),
            ("ESOCKTNOSUPPORT", 44),
            ("ENOTSUP", 45),
            ("EPFNOSUPPORT", 46),
            ("EAFNOSUPPORT", 47),
            ("EADDRINUSE", 48),
            ("EADDRNOTAVAIL", 49),
            ("ENETDOWN", 50),
            ("ENETUNREACH", 51),
            ("ENETRESET", 52),
            ("ECONNABORTED", 53),
            ("ECONNRESET", 54),
            ("ENOBUFS", 55),
            ("EISCONN", 56),
            ("ENOTCONN", 57),
            ("ESHUTDOWN", 58),
            ("ETOOMANYREFS", 59),
            ("ETIMEDOUT", 60),
            ("ECONNREFUSED", 61),
            ("ELOOP", 62),
            ("ENAMETOOLONG", 63),
            ("EHOSTDOWN", 64),
            ("EHOSTUNREACH", 65),
            ("ENOTEMPTY", 66),
            ("EPROCLIM", 67),
            ("EUSERS", 68),
            ("EDQUOT", 69),
            ("ESTALE", 70),
            ("EREMOTE", 71),
            ("EBADRPC", 72),
            ("ERPCMISMATCH", 73),
            ("EPROGUNAVAIL", 74),
            ("EPROGMISMATCH", 75),
            ("EPROCUNAVAIL", 76),
            ("ENOLCK", 77),
            ("ENOSYS", 78),
            ("EFTYPE", 79),
            ("EAUTH", 80),
            ("ENEEDAUTH", 81),
            ("EPWROFF", 82),
            ("EDEVERR", 83),
            ("EOVERFLOW", 84),
            ("EBADEXEC", 85),
            ("EBADARCH", 86),
            ("ESHLIBVERS", 87),
            ("EBADMACHO", 88),
            ("ECANCELED", 89),
            ("EIDRM", 90),
            ("ENOMSG", 91),
            ("EILSEQ", 92),
            ("ENOATTR", 93),
            ("EBADMSG", 94),
            ("EMULTIHOP", 95),
            ("ENODATA", 96),
            ("ENOLINK", 97),
            ("ENOSR", 98),
            ("ENOSTR", 99),
            ("EPROTO", 100),
            ("ETIME", 101),
            ("EOPNOTSUPP", 102),
            ("ENOPOLICY", 103),
            ("ENOTRECOVERABLE", 104),
            ("EOWNERDEAD", 105),
            ("EQFULL", 106),
            ("ENOTCAPABLE", 107),
        ];
        let mut errno_values = Vec::new();
        let mut errorcode_entries = Vec::new();
        for (name, value) in &errno_constants {
            errno_values.push((*name, Value::Int(*value)));
            errorcode_entries.push((Value::Int(*value), Value::Str((*name).to_string())));
        }
        errno_values.push(("errorcode", self.heap.alloc_dict(errorcode_entries)));
        self.install_builtin_module("errno", &[], errno_values);
        let inspect_sentinel = {
            let sentinel_class = match self.heap.alloc_class(ClassObject::new(
                "_inspect_sentinel".to_string(),
                Vec::new(),
            )) {
                Value::Class(obj) => obj,
                _ => unreachable!(),
            };
            self.heap
                .alloc_instance(InstanceObject::new(sentinel_class))
        };
        let inspect_signature_class = match self
            .heap
            .alloc_class(ClassObject::new("Signature".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        let inspect_parameter_class = match self
            .heap
            .alloc_class(ClassObject::new("Parameter".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        let inspect_bound_arguments_class = match self
            .heap
            .alloc_class(ClassObject::new("BoundArguments".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *inspect_signature_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("inspect".to_string()));
            class_data
                .attrs
                .insert("empty".to_string(), inspect_sentinel.clone());
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureInit),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureStr),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureRepr),
            );
            class_data.attrs.insert(
                "replace".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureReplace),
            );
            class_data.attrs.insert(
                "bind".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureBind),
            );
            class_data.attrs.insert(
                "bind_partial".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureBindPartial),
            );
            class_data.attrs.insert(
                "from_callable".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignature),
            );
        }
        if let Object::Class(class_data) = &mut *inspect_parameter_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("inspect".to_string()));
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::InspectParameterInit),
            );
            class_data.attrs.insert(
                "replace".to_string(),
                Value::Builtin(BuiltinFunction::InspectParameterReplace),
            );
            class_data
                .attrs
                .insert("empty".to_string(), inspect_sentinel.clone());
            class_data
                .attrs
                .insert("POSITIONAL_ONLY".to_string(), Value::Int(0));
            class_data
                .attrs
                .insert("POSITIONAL_OR_KEYWORD".to_string(), Value::Int(1));
            class_data
                .attrs
                .insert("VAR_POSITIONAL".to_string(), Value::Int(2));
            class_data
                .attrs
                .insert("KEYWORD_ONLY".to_string(), Value::Int(3));
            class_data
                .attrs
                .insert("VAR_KEYWORD".to_string(), Value::Int(4));
        }
        if let Object::Class(class_data) = &mut *inspect_bound_arguments_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("inspect".to_string()));
        }
        self.install_builtin_module(
            "inspect",
            &[
                ("signature", BuiltinFunction::InspectSignature),
                ("getmodule", BuiltinFunction::InspectGetModule),
                ("getfile", BuiltinFunction::InspectGetFile),
                ("getdoc", BuiltinFunction::InspectGetDoc),
                ("getsourcefile", BuiltinFunction::InspectGetSourceFile),
                ("cleandoc", BuiltinFunction::InspectCleanDoc),
                ("isabstract", BuiltinFunction::InspectIsAbstract),
                ("isfunction", BuiltinFunction::InspectIsFunction),
                ("ismethod", BuiltinFunction::InspectIsMethod),
                ("isroutine", BuiltinFunction::InspectIsRoutine),
                (
                    "ismethoddescriptor",
                    BuiltinFunction::InspectIsMethodDescriptor,
                ),
                ("isdatadescriptor", BuiltinFunction::InspectIsDataDescriptor),
                ("ismethodwrapper", BuiltinFunction::InspectIsMethodWrapper),
                ("istraceback", BuiltinFunction::InspectIsTraceback),
                ("isframe", BuiltinFunction::InspectIsFrame),
                ("iscode", BuiltinFunction::InspectIsCode),
                ("unwrap", BuiltinFunction::InspectUnwrap),
                ("isclass", BuiltinFunction::InspectIsClass),
                ("ismodule", BuiltinFunction::InspectIsModule),
                ("isgenerator", BuiltinFunction::InspectIsGenerator),
                ("isgeneratorfunction", BuiltinFunction::InspectIsGenerator),
                ("iscoroutine", BuiltinFunction::InspectIsCoroutine),
                ("iscoroutinefunction", BuiltinFunction::InspectIsCoroutine),
                ("isawaitable", BuiltinFunction::InspectIsAwaitable),
                ("isasyncgen", BuiltinFunction::InspectIsAsyncGen),
                ("isasyncgenfunction", BuiltinFunction::InspectIsAsyncGen),
                ("_static_getmro", BuiltinFunction::InspectStaticGetMro),
                (
                    "_get_dunder_dict_of_class",
                    BuiltinFunction::InspectGetDunderDictOfClass,
                ),
            ],
            vec![
                ("_sentinel", inspect_sentinel.clone()),
                ("_empty", inspect_sentinel.clone()),
                ("Signature", Value::Class(inspect_signature_class)),
                ("Parameter", Value::Class(inspect_parameter_class)),
                (
                    "BoundArguments",
                    Value::Class(inspect_bound_arguments_class),
                ),
                ("CO_VARARGS", Value::Int(0x04)),
                ("CO_VARKEYWORDS", Value::Int(0x08)),
                ("CO_GENERATOR", Value::Int(0x20)),
                ("CO_COROUTINE", Value::Int(0x80)),
                ("CO_ASYNC_GENERATOR", Value::Int(0x200)),
            ],
        );
        self.install_builtin_module(
            "io",
            &[
                ("open", BuiltinFunction::IoOpen),
                ("read_text", BuiltinFunction::IoReadText),
                ("write_text", BuiltinFunction::IoWriteText),
                ("text_encoding", BuiltinFunction::IoTextEncoding),
            ],
            vec![
                {
                    let textio = self
                        .heap
                        .alloc_class(ClassObject::new("TextIOWrapper".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &textio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoTextIOWrapperInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileRead),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileReadLine),
                        );
                        class_data.attrs.insert(
                            "readlines".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileReadLines),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileWrite),
                        );
                        class_data.attrs.insert(
                            "writelines".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileWriteLines),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileTruncate),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileTell),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileClose),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileFlush),
                        );
                        class_data.attrs.insert(
                            "__iter__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileIter),
                        );
                        class_data.attrs.insert(
                            "__next__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileNext),
                        );
                        class_data.attrs.insert(
                            "__enter__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileEnter),
                        );
                        class_data.attrs.insert(
                            "__exit__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileExit),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileFileno),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileDetach),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileSeekable),
                        );
                    }
                    ("TextIOWrapper", textio)
                },
                {
                    let fileio = self
                        .heap
                        .alloc_class(ClassObject::new("FileIO".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &fileio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_io_file_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileInit),
                        );
                    }
                    ("FileIO", fileio)
                },
                {
                    let stringio = self
                        .heap
                        .alloc_class(ClassObject::new("StringIO".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &stringio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_stringio_methods(class_data);
                    }
                    ("StringIO", stringio)
                },
                {
                    let bytesio = self
                        .heap
                        .alloc_class(ClassObject::new("BytesIO".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &bytesio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_bytesio_methods(class_data);
                    }
                    ("BytesIO", bytesio)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedReader".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead1),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadLine),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedClose),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedDetach),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedPeek),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedReader", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedWriter".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead1),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadLine),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedClose),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedDetach),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedPeek),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedWriter", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedRandom".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead1),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadLine),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedClose),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedDetach),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedPeek),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedRandom", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedRWPair".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairRead),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadLine),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairRead1),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadInto1),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairClose),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairWritable),
                        );
                        class_data.attrs.insert(
                            "isatty".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairIsAtty),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairSeekable),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairDetach),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairPeek),
                        );
                    }
                    ("BufferedRWPair", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("IOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                    }
                    ("IOBase", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("RawIOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoRawRead),
                        );
                        class_data.attrs.insert(
                            "readall".to_string(),
                            Value::Builtin(BuiltinFunction::IoRawReadAll),
                        );
                    }
                    ("RawIOBase", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedIOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedPeek),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedIOBase", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("TextIOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                    }
                    ("TextIOBase", class)
                },
                (
                    "Reader",
                    self.heap
                        .alloc_class(ClassObject::new("Reader".to_string(), Vec::new())),
                ),
                (
                    "Writer",
                    self.heap
                        .alloc_class(ClassObject::new("Writer".to_string(), Vec::new())),
                ),
                {
                    let class = self.heap.alloc_class(ClassObject::new(
                        "IncrementalNewlineDecoder".to_string(),
                        Vec::new(),
                    ));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderInit),
                        );
                        class_data.attrs.insert(
                            "decode".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderDecode),
                        );
                        class_data.attrs.insert(
                            "getstate".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderGetState),
                        );
                        class_data.attrs.insert(
                            "setstate".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderSetState),
                        );
                        class_data.attrs.insert(
                            "reset".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderReset),
                        );
                    }
                    ("IncrementalNewlineDecoder", class)
                },
                (
                    "UnsupportedOperation",
                    Value::ExceptionType("UnsupportedOperation".to_string()),
                ),
                (
                    "BlockingIOError",
                    Value::ExceptionType("BlockingIOError".to_string()),
                ),
                (
                    "__all__",
                    self.heap.alloc_list(vec![
                        Value::Str("open".to_string()),
                        Value::Str("TextIOWrapper".to_string()),
                        Value::Str("FileIO".to_string()),
                        Value::Str("StringIO".to_string()),
                        Value::Str("BytesIO".to_string()),
                        Value::Str("BufferedReader".to_string()),
                        Value::Str("BufferedWriter".to_string()),
                        Value::Str("BufferedRandom".to_string()),
                        Value::Str("BufferedRWPair".to_string()),
                        Value::Str("IOBase".to_string()),
                        Value::Str("RawIOBase".to_string()),
                        Value::Str("BufferedIOBase".to_string()),
                        Value::Str("TextIOBase".to_string()),
                        Value::Str("Reader".to_string()),
                        Value::Str("Writer".to_string()),
                        Value::Str("IncrementalNewlineDecoder".to_string()),
                        Value::Str("UnsupportedOperation".to_string()),
                        Value::Str("BlockingIOError".to_string()),
                        Value::Str("DEFAULT_BUFFER_SIZE".to_string()),
                        Value::Str("SEEK_SET".to_string()),
                        Value::Str("SEEK_CUR".to_string()),
                        Value::Str("SEEK_END".to_string()),
                    ]),
                ),
                ("DEFAULT_BUFFER_SIZE", Value::Int(8192)),
                ("SEEK_SET", Value::Int(0)),
                ("SEEK_CUR", Value::Int(1)),
                ("SEEK_END", Value::Int(2)),
            ],
        );
        self.install_builtin_module(
            "_io",
            &[("open", BuiltinFunction::IoOpen)],
            vec![
                {
                    let textio = self
                        .heap
                        .alloc_class(ClassObject::new("TextIOWrapper".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &textio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoTextIOWrapperInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileRead),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileReadLine),
                        );
                        class_data.attrs.insert(
                            "readlines".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileReadLines),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileWrite),
                        );
                        class_data.attrs.insert(
                            "writelines".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileWriteLines),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileTruncate),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileTell),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileClose),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileFlush),
                        );
                        class_data.attrs.insert(
                            "__iter__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileIter),
                        );
                        class_data.attrs.insert(
                            "__next__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileNext),
                        );
                        class_data.attrs.insert(
                            "__enter__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileEnter),
                        );
                        class_data.attrs.insert(
                            "__exit__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileExit),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileFileno),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileDetach),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileSeekable),
                        );
                    }
                    ("TextIOWrapper", textio)
                },
                {
                    let fileio = self
                        .heap
                        .alloc_class(ClassObject::new("FileIO".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &fileio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_io_file_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoFileInit),
                        );
                    }
                    ("FileIO", fileio)
                },
                {
                    let bytesio = self
                        .heap
                        .alloc_class(ClassObject::new("BytesIO".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &bytesio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_bytesio_methods(class_data);
                    }
                    ("BytesIO", bytesio)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedReader".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead1),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadLine),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedClose),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedDetach),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedPeek),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedReader", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedWriter".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead1),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadLine),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedClose),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedDetach),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedPeek),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedWriter", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedRandom".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRead1),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadLine),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedClose),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedDetach),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedRandom", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedRWPair".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairInit),
                        );
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairRead),
                        );
                        class_data.attrs.insert(
                            "readline".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadLine),
                        );
                        class_data.attrs.insert(
                            "read1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairRead1),
                        );
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadInto1),
                        );
                        class_data.attrs.insert(
                            "write".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairWrite),
                        );
                        class_data.attrs.insert(
                            "flush".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairFlush),
                        );
                        class_data.attrs.insert(
                            "close".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairClose),
                        );
                        class_data.attrs.insert(
                            "fileno".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedFileno),
                        );
                        class_data.attrs.insert(
                            "seek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeek),
                        );
                        class_data.attrs.insert(
                            "tell".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTell),
                        );
                        class_data.attrs.insert(
                            "truncate".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedTruncate),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairWritable),
                        );
                        class_data.attrs.insert(
                            "isatty".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairIsAtty),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairSeekable),
                        );
                        class_data.attrs.insert(
                            "detach".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairDetach),
                        );
                        class_data.attrs.insert(
                            "peek".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedRWPairPeek),
                        );
                    }
                    ("BufferedRWPair", class)
                },
                ("StringIO", {
                    let stringio = self
                        .heap
                        .alloc_class(ClassObject::new("StringIO".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &stringio
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_stringio_methods(class_data);
                    }
                    stringio
                }),
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("IOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                    }
                    ("IOBase", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("RawIOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "read".to_string(),
                            Value::Builtin(BuiltinFunction::IoRawRead),
                        );
                        class_data.attrs.insert(
                            "readall".to_string(),
                            Value::Builtin(BuiltinFunction::IoRawReadAll),
                        );
                    }
                    ("RawIOBase", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("BufferedIOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                        class_data.attrs.insert(
                            "readinto".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto),
                        );
                        class_data.attrs.insert(
                            "readinto1".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadInto1),
                        );
                        class_data.attrs.insert(
                            "readable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedReadable),
                        );
                        class_data.attrs.insert(
                            "writable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedWritable),
                        );
                        class_data.attrs.insert(
                            "seekable".to_string(),
                            Value::Builtin(BuiltinFunction::IoBufferedSeekable),
                        );
                    }
                    ("BufferedIOBase", class)
                },
                {
                    let class = self
                        .heap
                        .alloc_class(ClassObject::new("TextIOBase".to_string(), Vec::new()));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        Self::install_iobase_methods(class_data);
                    }
                    ("TextIOBase", class)
                },
                {
                    let class = self.heap.alloc_class(ClassObject::new(
                        "IncrementalNewlineDecoder".to_string(),
                        Vec::new(),
                    ));
                    if let Value::Class(class_ref) = &class
                        && let Object::Class(class_data) = &mut *class_ref.kind_mut()
                    {
                        class_data.attrs.insert(
                            "__init__".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderInit),
                        );
                        class_data.attrs.insert(
                            "decode".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderDecode),
                        );
                        class_data.attrs.insert(
                            "getstate".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderGetState),
                        );
                        class_data.attrs.insert(
                            "setstate".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderSetState),
                        );
                        class_data.attrs.insert(
                            "reset".to_string(),
                            Value::Builtin(BuiltinFunction::IoIncrementalNewlineDecoderReset),
                        );
                    }
                    ("IncrementalNewlineDecoder", class)
                },
                (
                    "UnsupportedOperation",
                    Value::ExceptionType("UnsupportedOperation".to_string()),
                ),
                (
                    "BlockingIOError",
                    Value::ExceptionType("BlockingIOError".to_string()),
                ),
                ("DEFAULT_BUFFER_SIZE", Value::Int(8192)),
                ("SEEK_SET", Value::Int(0)),
                ("SEEK_CUR", Value::Int(1)),
                ("SEEK_END", Value::Int(2)),
            ],
        );
        self.wire_io_class_hierarchy();
        self.install_builtin_module(
            "resource",
            &[("getrlimit", BuiltinFunction::Range)],
            vec![
                ("RLIMIT_STACK", Value::Int(2)),
                ("RLIM_INFINITY", Value::Int(-1)),
            ],
        );
        self.install_builtin_module(
            "_posixsubprocess",
            &[("fork_exec", BuiltinFunction::PosixSubprocessForkExec)],
            vec![(
                "__doc__",
                Value::Str("pyrs _posixsubprocess stub".to_string()),
            )],
        );
        let subprocess_pipe_class = match self
            .heap
            .alloc_class(ClassObject::new("_PyrsPipe".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *subprocess_pipe_class.kind_mut() {
            class_data.attrs.insert(
                "read".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPipeRead),
            );
            class_data.attrs.insert(
                "readline".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPipeReadline),
            );
            class_data.attrs.insert(
                "write".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPipeWrite),
            );
            class_data.attrs.insert(
                "flush".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPipeFlush),
            );
            class_data.attrs.insert(
                "close".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPipeClose),
            );
        }
        let subprocess_popen_class = match self
            .heap
            .alloc_class(ClassObject::new("Popen".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *subprocess_popen_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPopenInit),
            );
            class_data.attrs.insert(
                "communicate".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPopenCommunicate),
            );
            class_data.attrs.insert(
                "wait".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPopenWait),
            );
            class_data.attrs.insert(
                "kill".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPopenKill),
            );
            class_data.attrs.insert(
                "poll".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPopenPoll),
            );
            class_data.attrs.insert(
                "__enter__".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPopenEnter),
            );
            class_data.attrs.insert(
                "__exit__".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessPopenExit),
            );
        }
        let subprocess_completed_process_class = match self
            .heap
            .alloc_class(ClassObject::new("CompletedProcess".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *subprocess_completed_process_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::SubprocessCompletedProcessInit),
            );
        }
        self.install_builtin_module(
            "subprocess",
            &[
                ("_cleanup", BuiltinFunction::SubprocessCleanup),
                ("run", BuiltinFunction::SubprocessRun),
                ("check_call", BuiltinFunction::SubprocessCheckCall),
                ("_args_from_interpreter_flags", BuiltinFunction::List),
            ],
            vec![
                ("PIPE", Value::Int(-1)),
                ("STDOUT", Value::Int(-2)),
                ("DEVNULL", Value::Int(-3)),
                ("_PyrsPipe", Value::Class(subprocess_pipe_class)),
                ("Popen", Value::Class(subprocess_popen_class)),
                (
                    "CompletedProcess",
                    Value::Class(subprocess_completed_process_class),
                ),
                (
                    "CalledProcessError",
                    Value::ExceptionType("CalledProcessError".to_string()),
                ),
                (
                    "SubprocessError",
                    Value::ExceptionType("SubprocessError".to_string()),
                ),
                (
                    "TimeoutExpired",
                    Value::ExceptionType("TimeoutExpired".to_string()),
                ),
            ],
        );
        self.install_builtin_module(
            "_testsinglephase",
            &[],
            vec![(
                "__doc__",
                Value::Str("pyrs _testsinglephase stub".to_string()),
            )],
        );
        self.install_builtin_module(
            "_testmultiphase",
            &[],
            vec![(
                "__doc__",
                Value::Str("pyrs _testmultiphase stub".to_string()),
            )],
        );
        let datetime_class = match self
            .heap
            .alloc_class(ClassObject::new("datetime".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *datetime_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeInit),
            );
            class_data.attrs.insert(
                "now".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeNow),
            );
            class_data.attrs.insert(
                "today".to_string(),
                Value::Builtin(BuiltinFunction::DateToday),
            );
            class_data.attrs.insert(
                "fromtimestamp".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeFromTimestamp),
            );
            class_data.attrs.insert(
                "fromisocalendar".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeFromIsoCalendar),
            );
            class_data.attrs.insert(
                "astimezone".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeAstimezone),
            );
            class_data.attrs.insert(
                "replace".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeReplace),
            );
            class_data.attrs.insert(
                "strftime".to_string(),
                Value::Builtin(BuiltinFunction::DateStrFTime),
            );
            class_data.attrs.insert(
                "isoformat".to_string(),
                Value::Builtin(BuiltinFunction::DateIsoFormat),
            );
            class_data.attrs.insert(
                "toordinal".to_string(),
                Value::Builtin(BuiltinFunction::DateToOrdinal),
            );
            class_data.attrs.insert(
                "weekday".to_string(),
                Value::Builtin(BuiltinFunction::DateWeekday),
            );
            class_data.attrs.insert(
                "isoweekday".to_string(),
                Value::Builtin(BuiltinFunction::DateIsoWeekday),
            );
        }
        let date_class = match self
            .heap
            .alloc_class(ClassObject::new("date".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *date_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::DateInit),
            );
            class_data.attrs.insert(
                "today".to_string(),
                Value::Builtin(BuiltinFunction::DateToday),
            );
            class_data.attrs.insert(
                "fromisocalendar".to_string(),
                Value::Builtin(BuiltinFunction::DateFromIsoCalendar),
            );
            class_data.attrs.insert(
                "replace".to_string(),
                Value::Builtin(BuiltinFunction::DateReplace),
            );
            class_data.attrs.insert(
                "strftime".to_string(),
                Value::Builtin(BuiltinFunction::DateStrFTime),
            );
            class_data.attrs.insert(
                "isoformat".to_string(),
                Value::Builtin(BuiltinFunction::DateIsoFormat),
            );
            class_data.attrs.insert(
                "toordinal".to_string(),
                Value::Builtin(BuiltinFunction::DateToOrdinal),
            );
            class_data.attrs.insert(
                "weekday".to_string(),
                Value::Builtin(BuiltinFunction::DateWeekday),
            );
            class_data.attrs.insert(
                "isoweekday".to_string(),
                Value::Builtin(BuiltinFunction::DateIsoWeekday),
            );
        }
        let timedelta_class = match self
            .heap
            .alloc_class(ClassObject::new("timedelta".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *timedelta_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeDeltaInit),
            );
        }
        let time_class = match self
            .heap
            .alloc_class(ClassObject::new("time".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *time_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::TimeInit),
            );
            class_data.attrs.insert(
                "replace".to_string(),
                Value::Builtin(BuiltinFunction::TimeReplace),
            );
        }
        let tzinfo_class = match self
            .heap
            .alloc_class(ClassObject::new("tzinfo".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        let timezone_class = match self.heap.alloc_class(ClassObject::new(
            "timezone".to_string(),
            vec![tzinfo_class.clone()],
        )) {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *timezone_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeTimezoneInit),
            );
        }
        let timezone_utc = match self
            .heap
            .alloc_instance(InstanceObject::new(timezone_class.clone()))
        {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *timezone_utc.kind_mut() {
            instance_data
                .attrs
                .insert("offset".to_string(), Value::Int(0));
            instance_data
                .attrs
                .insert("name".to_string(), Value::Str("UTC".to_string()));
        }
        if let Object::Class(class_data) = &mut *timezone_class.kind_mut() {
            class_data
                .attrs
                .insert("utc".to_string(), Value::Instance(timezone_utc.clone()));
        }
        self.install_builtin_module(
            "datetime",
            &[
                ("now", BuiltinFunction::DateTimeNow),
                ("today", BuiltinFunction::DateToday),
            ],
            vec![
                ("datetime", Value::Class(datetime_class)),
                ("date", Value::Class(date_class)),
                ("timedelta", Value::Class(timedelta_class)),
                ("time", Value::Class(time_class)),
                ("tzinfo", Value::Class(tzinfo_class)),
                ("timezone", Value::Class(timezone_class)),
                ("UTC", Value::Instance(timezone_utc)),
            ],
        );
        self.install_module_alias_from_existing("_datetime", "datetime");
        let uuid_class = match self
            .heap
            .alloc_class(ClassObject::new("UUID".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *uuid_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::UuidClassInit),
            );
            class_data
                .attrs
                .insert("__str__".to_string(), Value::Builtin(BuiltinFunction::Repr));
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::Repr),
            );
        }
        let alloc_uuid_constant = |vm: &mut Vm, text: &str| -> Value {
            let bytes = parse_uuid_like_string(text).expect("static UUID constant must be valid");
            let instance = match vm
                .heap
                .alloc_instance(InstanceObject::new(uuid_class.clone()))
            {
                Value::Instance(obj) => obj,
                _ => unreachable!(),
            };
            vm.populate_uuid_instance(&instance, bytes)
                .expect("UUID constant population must succeed");
            Value::Instance(instance)
        };
        let uuid_namespace_dns = alloc_uuid_constant(self, "6ba7b810-9dad-11d1-80b4-00c04fd430c8");
        let uuid_namespace_url = alloc_uuid_constant(self, "6ba7b811-9dad-11d1-80b4-00c04fd430c8");
        let uuid_namespace_oid = alloc_uuid_constant(self, "6ba7b812-9dad-11d1-80b4-00c04fd430c8");
        let uuid_namespace_x500 = alloc_uuid_constant(self, "6ba7b814-9dad-11d1-80b4-00c04fd430c8");
        let uuid_nil = alloc_uuid_constant(self, "00000000-0000-0000-0000-000000000000");
        let uuid_max = alloc_uuid_constant(self, "ffffffff-ffff-ffff-ffff-ffffffffffff");
        self.install_builtin_module(
            "uuid",
            &[
                ("uuid1", BuiltinFunction::Uuid1),
                ("uuid3", BuiltinFunction::Uuid3),
                ("uuid4", BuiltinFunction::Uuid4),
                ("uuid5", BuiltinFunction::Uuid5),
                ("uuid6", BuiltinFunction::Uuid6),
                ("uuid7", BuiltinFunction::Uuid7),
                ("uuid8", BuiltinFunction::Uuid8),
                ("getnode", BuiltinFunction::UuidGetNode),
            ],
            vec![
                ("UUID", Value::Class(uuid_class)),
                ("NAMESPACE_DNS", uuid_namespace_dns),
                ("NAMESPACE_URL", uuid_namespace_url),
                ("NAMESPACE_OID", uuid_namespace_oid),
                ("NAMESPACE_X500", uuid_namespace_x500),
                ("NIL", uuid_nil),
                ("MAX", uuid_max),
            ],
        );
        self.install_builtin_module(
            "asyncio",
            &[
                ("run", BuiltinFunction::AsyncioRun),
                ("sleep", BuiltinFunction::AsyncioSleep),
                ("create_task", BuiltinFunction::AsyncioCreateTask),
                ("gather", BuiltinFunction::AsyncioGather),
            ],
            Vec::new(),
        );
        let thread_class = match self
            .heap
            .alloc_class(ClassObject::new("Thread".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *thread_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadClassInit),
            );
            class_data.attrs.insert(
                "start".to_string(),
                Value::Builtin(BuiltinFunction::ThreadClassStart),
            );
            class_data.attrs.insert(
                "join".to_string(),
                Value::Builtin(BuiltinFunction::ThreadClassJoin),
            );
            class_data.attrs.insert(
                "is_alive".to_string(),
                Value::Builtin(BuiltinFunction::ThreadClassIsAlive),
            );
        }
        let event_class = match self
            .heap
            .alloc_class(ClassObject::new("Event".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *event_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadEventInit),
            );
            class_data.attrs.insert(
                "set".to_string(),
                Value::Builtin(BuiltinFunction::ThreadEventSet),
            );
            class_data.attrs.insert(
                "clear".to_string(),
                Value::Builtin(BuiltinFunction::ThreadEventClear),
            );
            class_data.attrs.insert(
                "wait".to_string(),
                Value::Builtin(BuiltinFunction::ThreadEventWait),
            );
            class_data.attrs.insert(
                "is_set".to_string(),
                Value::Builtin(BuiltinFunction::ThreadEventIsSet),
            );
        }
        let condition_class = match self
            .heap
            .alloc_class(ClassObject::new("Condition".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *condition_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionInit),
            );
            class_data.attrs.insert(
                "acquire".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionAcquire),
            );
            class_data.attrs.insert(
                "__enter__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionEnter),
            );
            class_data.attrs.insert(
                "release".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionRelease),
            );
            class_data.attrs.insert(
                "__exit__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionExit),
            );
            class_data.attrs.insert(
                "wait".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionWait),
            );
            class_data.attrs.insert(
                "notify".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionNotify),
            );
            class_data.attrs.insert(
                "notify_all".to_string(),
                Value::Builtin(BuiltinFunction::ThreadConditionNotifyAll),
            );
        }
        let semaphore_class = match self
            .heap
            .alloc_class(ClassObject::new("Semaphore".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *semaphore_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadSemaphoreInit),
            );
            class_data.attrs.insert(
                "acquire".to_string(),
                Value::Builtin(BuiltinFunction::ThreadSemaphoreAcquire),
            );
            class_data.attrs.insert(
                "release".to_string(),
                Value::Builtin(BuiltinFunction::ThreadSemaphoreRelease),
            );
        }
        let bounded_semaphore_class = match self
            .heap
            .alloc_class(ClassObject::new("BoundedSemaphore".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *bounded_semaphore_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadBoundedSemaphoreInit),
            );
            class_data.attrs.insert(
                "acquire".to_string(),
                Value::Builtin(BuiltinFunction::ThreadSemaphoreAcquire),
            );
            class_data.attrs.insert(
                "release".to_string(),
                Value::Builtin(BuiltinFunction::ThreadSemaphoreRelease),
            );
        }
        let barrier_class = match self
            .heap
            .alloc_class(ClassObject::new("Barrier".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *barrier_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::ThreadBarrierInit),
            );
            class_data.attrs.insert(
                "wait".to_string(),
                Value::Builtin(BuiltinFunction::ThreadBarrierWait),
            );
            class_data.attrs.insert(
                "reset".to_string(),
                Value::Builtin(BuiltinFunction::ThreadBarrierReset),
            );
            class_data.attrs.insert(
                "abort".to_string(),
                Value::Builtin(BuiltinFunction::ThreadBarrierAbort),
            );
        }
        let thread_local_class = match self
            .heap
            .alloc_class(ClassObject::new("local".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Some(thread_module) = self.modules.get("_thread").cloned()
            && let Object::Module(module_data) = &mut *thread_module.kind_mut()
        {
            module_data.globals.insert(
                "_local".to_string(),
                Value::Class(thread_local_class.clone()),
            );
        }
        self.install_builtin_module(
            "threading",
            &[
                ("RLock", BuiltinFunction::ThreadRLock),
                ("_PyRLock", BuiltinFunction::ThreadRLock),
                ("_CRLock", BuiltinFunction::ThreadRLock),
                ("Lock", BuiltinFunction::ThreadRLock),
                ("excepthook", BuiltinFunction::ThreadingExcepthook),
                ("__excepthook__", BuiltinFunction::ThreadingExcepthook),
                ("get_ident", BuiltinFunction::ThreadingGetIdent),
                ("current_thread", BuiltinFunction::ThreadingCurrentThread),
                ("main_thread", BuiltinFunction::ThreadingMainThread),
                ("active_count", BuiltinFunction::ThreadingActiveCount),
                ("_register_atexit", BuiltinFunction::ThreadingRegisterAtexit),
            ],
            vec![
                ("TIMEOUT_MAX", Value::Float(f64::MAX)),
                ("Thread", Value::Class(thread_class)),
                ("Event", Value::Class(event_class)),
                ("Condition", Value::Class(condition_class)),
                ("Semaphore", Value::Class(semaphore_class)),
                ("BoundedSemaphore", Value::Class(bounded_semaphore_class)),
                ("Barrier", Value::Class(barrier_class)),
                ("local", Value::Class(thread_local_class)),
                (
                    "ThreadError",
                    Value::ExceptionType("RuntimeError".to_string()),
                ),
                ("_dangling", self.heap.alloc_set(Vec::new())),
            ],
        );
        self.install_builtin_module(
            "signal",
            &[
                ("signal", BuiltinFunction::SignalSignal),
                ("getsignal", BuiltinFunction::SignalGetSignal),
                ("raise_signal", BuiltinFunction::SignalRaiseSignal),
            ],
            vec![
                ("SIG_DFL", Value::Int(SIGNAL_DEFAULT)),
                ("SIG_IGN", Value::Int(SIGNAL_IGNORE)),
                ("SIGINT", Value::Int(SIGNAL_SIGINT)),
                ("SIGTERM", Value::Int(SIGNAL_SIGTERM)),
            ],
        );
        self.install_module_alias_from_existing("_signal", "signal");
        self.install_builtin_module(
            "_suggestions",
            &[("_generate_suggestions", BuiltinFunction::NoOp)],
            Vec::new(),
        );
        self.install_builtin_module(
            "_symtable",
            &[("symtable", BuiltinFunction::NoOp)],
            vec![
                ("USE", Value::Int(16)),
                ("DEF_GLOBAL", Value::Int(1)),
                ("DEF_NONLOCAL", Value::Int(8)),
                ("DEF_LOCAL", Value::Int(2)),
                ("DEF_PARAM", Value::Int(4)),
                ("DEF_TYPE_PARAM", Value::Int(1024)),
                ("DEF_FREE_CLASS", Value::Int(64)),
                ("DEF_IMPORT", Value::Int(128)),
                ("DEF_BOUND", Value::Int(134)),
                ("DEF_ANNOT", Value::Int(256)),
                ("DEF_COMP_ITER", Value::Int(512)),
                ("DEF_COMP_CELL", Value::Int(2048)),
                ("SCOPE_OFF", Value::Int(12)),
                ("SCOPE_MASK", Value::Int(15)),
                ("FREE", Value::Int(4)),
                ("LOCAL", Value::Int(1)),
                ("GLOBAL_IMPLICIT", Value::Int(3)),
                ("GLOBAL_EXPLICIT", Value::Int(2)),
                ("CELL", Value::Int(5)),
                ("TYPE_FUNCTION", Value::Int(0)),
                ("TYPE_CLASS", Value::Int(1)),
                ("TYPE_MODULE", Value::Int(2)),
                ("TYPE_ANNOTATION", Value::Int(3)),
                ("TYPE_TYPE_ALIAS", Value::Int(4)),
                ("TYPE_TYPE_PARAMETERS", Value::Int(5)),
                ("TYPE_TYPE_VARIABLE", Value::Int(6)),
            ],
        );
        self.install_builtin_module(
            "_tracemalloc",
            &[
                ("start", BuiltinFunction::NoOp),
                ("stop", BuiltinFunction::NoOp),
                ("is_tracing", BuiltinFunction::Bool),
                ("get_traceback_limit", BuiltinFunction::Int),
                ("get_traced_memory", BuiltinFunction::Tuple),
                ("get_tracemalloc_memory", BuiltinFunction::Int),
                ("reset_peak", BuiltinFunction::NoOp),
                ("clear_traces", BuiltinFunction::NoOp),
                ("_get_traces", BuiltinFunction::List),
                ("_get_object_traceback", BuiltinFunction::NoOp),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "faulthandler",
            &[
                ("enable", BuiltinFunction::NoOp),
                ("disable", BuiltinFunction::Bool),
                ("is_enabled", BuiltinFunction::Bool),
                ("dump_traceback", BuiltinFunction::NoOp),
                ("dump_traceback_later", BuiltinFunction::NoOp),
                ("cancel_dump_traceback_later", BuiltinFunction::NoOp),
                ("register", BuiltinFunction::NoOp),
                ("unregister", BuiltinFunction::Bool),
                ("dump_c_stack", BuiltinFunction::NoOp),
                ("_read_null", BuiltinFunction::NoOp),
                ("_sigsegv", BuiltinFunction::NoOp),
                ("_sigabrt", BuiltinFunction::NoOp),
                ("_sigfpe", BuiltinFunction::NoOp),
                ("_stack_overflow", BuiltinFunction::NoOp),
                ("_fatal_error_c_thread", BuiltinFunction::NoOp),
            ],
            Vec::new(),
        );
        let pwd_struct_passwd_class = self.alloc_tuple_backed_builtin_class("struct_passwd");
        self.install_builtin_module(
            "pwd",
            &[
                ("getpwall", BuiltinFunction::PwdGetPwAll),
                ("getpwnam", BuiltinFunction::PwdGetPwNam),
                ("getpwuid", BuiltinFunction::PwdGetPwUid),
            ],
            vec![("struct_passwd", pwd_struct_passwd_class)],
        );
        self.install_builtin_module(
            "_locale",
            &[
                ("setlocale", BuiltinFunction::LocaleSetLocale),
                ("localeconv", BuiltinFunction::LocaleLocaleConv),
                ("strxfrm", BuiltinFunction::Str),
                ("strcoll", BuiltinFunction::NoOp),
                ("nl_langinfo", BuiltinFunction::NoOp),
                ("getencoding", BuiltinFunction::SysGetFilesystemEncoding),
            ],
            vec![
                ("CHAR_MAX", Value::Int(127)),
                ("LC_CTYPE", Value::Int(0)),
                ("LC_NUMERIC", Value::Int(1)),
                ("LC_TIME", Value::Int(2)),
                ("LC_COLLATE", Value::Int(3)),
                ("LC_MONETARY", Value::Int(4)),
                ("LC_MESSAGES", Value::Int(5)),
                ("LC_ALL", Value::Int(6)),
                ("Error", Value::ExceptionType("Error".to_string())),
                ("_pyrs_current_locale", Value::Str("C".to_string())),
            ],
        );
        self.install_builtin_module(
            "_stat",
            &[],
            vec![
                ("ST_MODE", Value::Int(0)),
                ("ST_INO", Value::Int(1)),
                ("ST_DEV", Value::Int(2)),
                ("ST_NLINK", Value::Int(3)),
                ("ST_UID", Value::Int(4)),
                ("ST_GID", Value::Int(5)),
                ("ST_SIZE", Value::Int(6)),
                ("ST_ATIME", Value::Int(7)),
                ("ST_MTIME", Value::Int(8)),
                ("ST_CTIME", Value::Int(9)),
                ("S_ISUID", Value::Int(0o4000)),
                ("S_ISGID", Value::Int(0o2000)),
                ("S_ISVTX", Value::Int(0o1000)),
                ("S_IREAD", Value::Int(0o400)),
                ("S_IWRITE", Value::Int(0o200)),
                ("S_IEXEC", Value::Int(0o100)),
                ("S_IRWXU", Value::Int(0o700)),
                ("S_IRUSR", Value::Int(0o400)),
                ("S_IWUSR", Value::Int(0o200)),
                ("S_IXUSR", Value::Int(0o100)),
                ("S_IRWXG", Value::Int(0o070)),
                ("S_IRGRP", Value::Int(0o040)),
                ("S_IWGRP", Value::Int(0o020)),
                ("S_IXGRP", Value::Int(0o010)),
                ("S_IRWXO", Value::Int(0o007)),
                ("S_IROTH", Value::Int(0o004)),
                ("S_IWOTH", Value::Int(0o002)),
                ("S_IXOTH", Value::Int(0o001)),
                ("S_IFMT", Value::Int(0o170000)),
                ("S_IFDIR", Value::Int(0o040000)),
                ("S_IFCHR", Value::Int(0o020000)),
                ("S_IFBLK", Value::Int(0o060000)),
                ("S_IFREG", Value::Int(0o100000)),
                ("S_IFIFO", Value::Int(0o010000)),
                ("S_IFLNK", Value::Int(0o120000)),
                ("S_IFSOCK", Value::Int(0o140000)),
                ("S_IFDOOR", Value::Int(0)),
                ("S_IFPORT", Value::Int(0)),
                ("S_IFWHT", Value::Int(0o160000)),
                ("UF_SETTABLE", Value::Int(0x0000ffff)),
                ("UF_NODUMP", Value::Int(0x00000001)),
                ("UF_IMMUTABLE", Value::Int(0x00000002)),
                ("UF_APPEND", Value::Int(0x00000004)),
                ("UF_OPAQUE", Value::Int(0x00000008)),
                ("UF_NOUNLINK", Value::Int(0x00000010)),
                ("UF_COMPRESSED", Value::Int(0x00000020)),
                ("UF_TRACKED", Value::Int(0x00000040)),
                ("UF_DATAVAULT", Value::Int(0x00000080)),
                ("UF_HIDDEN", Value::Int(0x00008000)),
                ("SF_SETTABLE", Value::Int(0xffff0000u32 as i64)),
                ("SF_ARCHIVED", Value::Int(0x00010000)),
                ("SF_IMMUTABLE", Value::Int(0x00020000)),
                ("SF_APPEND", Value::Int(0x00040000)),
                ("SF_RESTRICTED", Value::Int(0x00080000)),
                ("SF_NOUNLINK", Value::Int(0x00100000)),
                ("SF_SNAPSHOT", Value::Int(0x00200000)),
                ("SF_FIRMLINK", Value::Int(0x00800000)),
                ("SF_DATALESS", Value::Int(0x40000000)),
            ],
        );
        let socket_class = match self
            .heap
            .alloc_class(ClassObject::new("socket".to_string(), Vec::new()))
        {
            Value::Class(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *socket_class.kind_mut() {
            class_data.attrs.insert(
                "__init__".to_string(),
                Value::Builtin(BuiltinFunction::SocketObjectInit),
            );
            class_data.attrs.insert(
                "close".to_string(),
                Value::Builtin(BuiltinFunction::SocketObjectClose),
            );
            class_data.attrs.insert(
                "fileno".to_string(),
                Value::Builtin(BuiltinFunction::SocketObjectFileno),
            );
            class_data.attrs.insert(
                "detach".to_string(),
                Value::Builtin(BuiltinFunction::SocketObjectDetach),
            );
        }
        self.install_builtin_module(
            "_socket",
            &[
                ("gethostname", BuiltinFunction::SocketGetHostName),
                ("gethostbyname", BuiltinFunction::SocketGetHostByName),
                ("getaddrinfo", BuiltinFunction::SocketGetAddrInfo),
                ("fromfd", BuiltinFunction::SocketFromFd),
                (
                    "getdefaulttimeout",
                    BuiltinFunction::SocketGetDefaultTimeout,
                ),
                (
                    "setdefaulttimeout",
                    BuiltinFunction::SocketSetDefaultTimeout,
                ),
                ("ntohs", BuiltinFunction::SocketNtoHs),
                ("ntohl", BuiltinFunction::SocketNtoHl),
                ("htons", BuiltinFunction::SocketHtoNs),
                ("htonl", BuiltinFunction::SocketHtoNl),
            ],
            vec![
                ("socket", Value::Class(socket_class)),
                ("error", Value::ExceptionType("Exception".to_string())),
                ("herror", Value::ExceptionType("Exception".to_string())),
                ("gaierror", Value::ExceptionType("Exception".to_string())),
                ("timeout", Value::ExceptionType("Exception".to_string())),
                ("has_ipv6", Value::Bool(false)),
                ("AF_UNSPEC", Value::Int(0)),
                ("AF_UNIX", Value::Int(1)),
                ("AF_INET", Value::Int(2)),
                ("AF_INET6", Value::Int(10)),
                ("SOCK_STREAM", Value::Int(1)),
                ("SOCK_DGRAM", Value::Int(2)),
                ("SOCK_RAW", Value::Int(3)),
                ("SOL_SOCKET", Value::Int(1)),
                ("SO_TYPE", Value::Int(3)),
                ("SCM_RIGHTS", Value::Int(1)),
                ("AI_PASSIVE", Value::Int(1)),
                ("AI_CANONNAME", Value::Int(2)),
                ("AI_NUMERICHOST", Value::Int(4)),
                ("AI_ADDRCONFIG", Value::Int(32)),
                ("AI_NUMERICSERV", Value::Int(1024)),
                ("_GLOBAL_DEFAULT_TIMEOUT", Value::None),
            ],
        );
        self.install_builtin_module(
            "_scproxy",
            &[
                (
                    "_get_proxy_settings",
                    BuiltinFunction::ScproxyGetProxySettings,
                ),
                ("_get_proxies", BuiltinFunction::ScproxyGetProxies),
            ],
            vec![],
        );
        self.install_builtin_module(
            "_warnings",
            &[
                ("warn", BuiltinFunction::WarningsWarn),
                ("warn_explicit", BuiltinFunction::WarningsWarnExplicit),
                ("_filters_mutated", BuiltinFunction::WarningsFiltersMutated),
                (
                    "_filters_mutated_lock_held",
                    BuiltinFunction::WarningsFiltersMutated,
                ),
                ("_acquire_lock", BuiltinFunction::WarningsAcquireLock),
                ("_release_lock", BuiltinFunction::WarningsReleaseLock),
            ],
            vec![
                ("_defaultaction", Value::Str("default".to_string())),
                ("_onceregistry", self.heap.alloc_dict(Vec::new())),
                ("_warnings_context", Value::None),
                ("filters", self.heap.alloc_list(Vec::new())),
            ],
        );
    }

    pub(super) fn sync_sys_path_from_module_paths(&mut self) {
        let sys_module = match self.modules.get("sys").cloned() {
            Some(module) => module,
            None => return,
        };
        let values = self
            .module_paths
            .iter()
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .collect::<Vec<_>>();
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("path".to_string(), self.heap.alloc_list(values));
        }
    }

    pub(super) fn sync_module_paths_from_sys(&mut self) {
        let Some(path_list) = self.sys_list_obj("path") else {
            return;
        };
        let list_kind = path_list.kind();
        let Object::List(values) = &*list_kind else {
            return;
        };
        let signature = Self::list_signature(values);
        if signature == self.import_sys_path_signature {
            return;
        }
        self.import_sys_path_signature = signature;
        let mut new_paths = Vec::new();
        for value in values {
            if let Value::Str(path) = value {
                new_paths.push(PathBuf::from(path));
            }
        }
        if new_paths == self.module_paths {
            return;
        }
        self.module_paths = new_paths;
        self.module_source_positive_cache.clear();
        self.import_dir_cache.clear();
        self.preferred_filesystem_module_cache.clear();
        self.maybe_prefer_cpython_pure_stdlib_modules();
    }

    pub(super) fn refresh_sys_modules_dict(&mut self) {
        let sys_module = match self.modules.get("sys").cloned() {
            Some(module) => module,
            None => return,
        };
        let existing_modules = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("modules").cloned(),
            _ => None,
        };
        let mut preserved_entries: HashMap<String, Value> = HashMap::new();
        if let Some(Value::Dict(existing)) = &existing_modules
            && let Object::Dict(existing_entries) = &*existing.kind()
        {
            for (key, value) in existing_entries.iter() {
                let Value::Str(name) = key else {
                    continue;
                };
                let preserve = match value {
                    // Preserve explicit import blockers and user overrides.
                    Value::None => true,
                    // Preserve sys.modules module entries unknown to `self.modules`
                    // plus narrowly-scoped replacement cases that must survive cache
                    // refresh (decimal aliasing and in-flight module init replacement).
                    Value::Module(existing_module) => match self.modules.get(name) {
                        None => true,
                        Some(cached) => {
                            if cached.id() == existing_module.id() {
                                false
                            } else {
                                self.should_preserve_sys_modules_module_override(
                                    name,
                                    cached,
                                    existing_module,
                                )
                            }
                        }
                    },
                    // Preserve non-module sentinels/extensions installed by user code.
                    _ => true,
                };
                if preserve {
                    preserved_entries.insert(name.clone(), value.clone());
                }
            }
        }
        let mut entries = Vec::with_capacity(self.modules.len() + preserved_entries.len());
        for (name, module) in self.modules.iter() {
            if preserved_entries.contains_key(name) {
                continue;
            }
            entries.push((Value::Str(name.clone()), Value::Module(module.clone())));
        }
        for (name, value) in preserved_entries {
            entries.push((Value::Str(name), value));
        }
        if let Some(Value::Dict(existing)) = existing_modules {
            if let Object::Dict(existing_entries) = &mut *existing.kind_mut() {
                *existing_entries = crate::runtime::DictObject::new(entries);
                return;
            }
        }
        let modules_dict = self.heap.alloc_dict(entries);
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("modules".to_string(), modules_dict);
        }
    }

    fn module_runtime_name(module: &ObjRef) -> Option<String> {
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return None;
        };
        Some(module_data.name.clone())
    }

    fn is_decimal_alias_override(name: &str, replacement: &ObjRef) -> bool {
        if name != "decimal" {
            return false;
        }
        matches!(
            Self::module_runtime_name(replacement).as_deref(),
            Some("_pydecimal")
        )
    }

    fn is_active_initializing_module_frame(&self, name: &str, module: &ObjRef) -> bool {
        self.frames.iter().rev().any(|frame| {
            frame.is_module
                && frame.module.id() == module.id()
                && matches!(
                    Self::module_runtime_name(&frame.module).as_deref(),
                    Some(module_name) if module_name == name
                )
        })
    }

    fn should_preserve_sys_modules_module_override(
        &self,
        name: &str,
        cached: &ObjRef,
        replacement: &ObjRef,
    ) -> bool {
        if Self::is_decimal_alias_override(name, replacement) {
            return true;
        }
        // During extension module initialization (Py_mod_exec), module code can
        // intentionally replace sys.modules[name]. Preserve only for the active
        // module frame and only while the cached module is still initializing.
        Self::module_is_initializing(cached)
            && !Self::module_is_initializing(replacement)
            && self.is_active_initializing_module_frame(name, cached)
    }

    pub(super) fn unregister_module(&mut self, name: &str) {
        if std::env::var_os("PYRS_TRACE_MODULE_CTYPES").is_some()
            && (name == "ctypes" || name == "_ctypes")
        {
            eprintln!(
                "[module-unregister] name={} frames={} stack={}",
                name,
                self.frames.len(),
                self.frames
                    .iter()
                    .rev()
                    .take(8)
                    .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                    .collect::<Vec<_>>()
                    .join(" <- ")
            );
        }
        self.modules.remove(name);
        if matches!(name, "pickle" | "_pickle" | "copyreg") {
            self.pickle_symbol_cache.clear();
            self.pickle_copyreg_cache.clear();
        }
        let Some(sys_module) = self.modules.get("sys").cloned() else {
            return;
        };
        let modules_dict = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("modules").cloned(),
            _ => None,
        };
        let Some(Value::Dict(modules_dict)) = modules_dict else {
            return;
        };
        if let Object::Dict(entries) = &mut *modules_dict.kind_mut() {
            entries.retain(|(key, _)| match key {
                Value::Str(entry_name) => entry_name != name,
                _ => true,
            });
        }
    }

    pub(super) fn has_cpython_pure_module_on_module_path(&self, module_name: &str) -> bool {
        let mut candidates = vec![module_name.to_string()];
        if let Some(alias_name) = Self::module_source_alias(module_name) {
            candidates.push(alias_name.to_string());
        }
        candidates.into_iter().any(|candidate| {
            let rel = candidate.replace('.', "/");
            self.module_paths.iter().any(|root| {
                root.join(format!("{rel}.py")).is_file()
                    || root.join(&rel).join("__init__.py").is_file()
            })
        })
    }

    pub(super) fn has_local_shim_module(&self, module_name: &str) -> bool {
        if !LOCAL_SHIM_MODULES.contains(&module_name) {
            return false;
        }
        let rel = module_name.replace('.', "/");
        let Some(shim_root) = Self::local_shim_root() else {
            return false;
        };
        shim_root.join(format!("{rel}.py")).is_file()
            || shim_root.join(rel).join("__init__.py").is_file()
    }

    pub(super) fn has_preferred_filesystem_module(&mut self, module_name: &str) -> bool {
        if let Some(cached) = self.preferred_filesystem_module_cache.get(module_name) {
            return *cached;
        }
        let present = self.has_cpython_pure_module_on_module_path(module_name)
            || self.has_local_shim_module(module_name);
        self.preferred_filesystem_module_cache
            .insert(module_name.to_string(), present);
        present
    }

    fn module_source_alias(module_name: &str) -> Option<&'static str> {
        match module_name {
            "collections.abc" => Some("_collections_abc"),
            _ => None,
        }
    }

    pub(super) fn maybe_prefer_cpython_pure_stdlib_modules(&mut self) {
        if self.prefer_pure_json_when_available {
            for module_name in PURE_STDLIB_JSON_MODULES {
                if self.has_preferred_filesystem_module(module_name)
                    && self.module_preference_requires_unload(module_name)
                {
                    self.unregister_module(module_name);
                }
            }
        }
        if self.prefer_pure_pickle_when_available {
            for module_name in PURE_STDLIB_PICKLE_MODULES {
                if self.has_preferred_filesystem_module(module_name)
                    && self.module_preference_requires_unload(module_name)
                {
                    self.unregister_module(module_name);
                }
            }
        }
        if self.prefer_pure_re_when_available {
            for module_name in PURE_STDLIB_RE_MODULES {
                if self.has_preferred_filesystem_module(module_name)
                    && self.module_preference_requires_unload(module_name)
                {
                    self.unregister_module(module_name);
                }
            }
        }
        for module_name in PURE_STDLIB_PATHLIB_MODULES {
            if self.has_preferred_filesystem_module(module_name)
                && self.module_preference_requires_unload(module_name)
            {
                self.unregister_module(module_name);
            }
        }
        for module_name in PURE_STDLIB_COLLECTIONS_MODULES {
            if self.has_preferred_filesystem_module(module_name)
                && self.module_preference_requires_unload(module_name)
            {
                self.unregister_module(module_name);
            }
        }
        for module_name in PURE_STDLIB_DECIMAL_MODULES {
            if self.has_preferred_filesystem_module(module_name)
                && self.module_preference_requires_unload(module_name)
            {
                self.unregister_module(module_name);
            }
        }
        for module_name in PURE_STDLIB_TYPES_MODULES {
            if self.has_preferred_filesystem_module(module_name)
                && self.module_preference_requires_unload(module_name)
            {
                self.unregister_module(module_name);
            }
        }
    }

    pub(super) fn module_preference_requires_unload(&self, module_name: &str) -> bool {
        let Some(module) = self.modules.get(module_name) else {
            return false;
        };
        if Self::module_loader_name(module).as_deref() == Some(BUILTIN_MODULE_LOADER) {
            return true;
        }
        Self::module_is_local_shim(module)
    }

    pub(super) fn local_shim_root() -> Option<PathBuf> {
        let repo_shim_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shims");
        if repo_shim_root.is_dir() {
            return Some(repo_shim_root);
        }
        let cwd_shim_root = std::env::current_dir().ok()?.join("shims");
        if cwd_shim_root.is_dir() {
            Some(cwd_shim_root)
        } else {
            None
        }
    }

    pub(super) fn module_origin_path(module: &ObjRef) -> Option<PathBuf> {
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return None;
        };
        match module_data.globals.get("__file__") {
            Some(Value::Str(path)) => Some(PathBuf::from(path)),
            _ => None,
        }
    }

    pub(super) fn module_is_local_shim(module: &ObjRef) -> bool {
        let Some(shim_root) = Self::local_shim_root() else {
            return false;
        };
        let Some(origin) = Self::module_origin_path(module) else {
            return false;
        };
        origin.starts_with(shim_root)
    }

    pub(super) fn register_module(&mut self, name: &str, module: ObjRef) {
        if std::env::var_os("PYRS_TRACE_MODULE_CTYPES").is_some()
            && (name == "ctypes" || name == "_ctypes")
        {
            eprintln!(
                "[module-register] name={} frames={} stack={}",
                name,
                self.frames.len(),
                self.frames
                    .iter()
                    .rev()
                    .take(8)
                    .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                    .collect::<Vec<_>>()
                    .join(" <- ")
            );
        }
        self.modules.insert(name.to_string(), module);
        self.refresh_sys_modules_dict();
    }

    fn install_module_alias_from_existing(&mut self, alias: &str, source_name: &str) {
        if self.modules.contains_key(alias) {
            return;
        }
        let Some(source_module) = self.modules.get(source_name).cloned() else {
            return;
        };
        let alias_module = match self.heap.alloc_module(ModuleObject::new(alias.to_string())) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &alias_module,
            alias,
            None,
            None,
            Some(BUILTIN_MODULE_LOADER),
            false,
            Vec::new(),
            false,
        );
        let exported = if let Object::Module(data) = &*source_module.kind() {
            Some(data.globals.clone())
        } else {
            None
        };
        if let Some(exported) = exported
            && let Object::Module(alias_data) = &mut *alias_module.kind_mut()
        {
            alias_data.globals.extend(exported);
            alias_data
                .globals
                .insert("__name__".to_string(), Value::Str(alias.to_string()));
        }
        self.register_module(alias, alias_module);
    }

    fn install_abc_fallback_module(&mut self) -> Result<(), RuntimeError> {
        if self.modules.contains_key("abc") {
            return Ok(());
        }
        let type_class = self
            .builtins
            .get("type")
            .cloned()
            .ok_or_else(|| RuntimeError::new("type class is unavailable"))
            .and_then(|value| self.class_from_base_value(value))?;
        let object_class = self
            .builtins
            .get("object")
            .cloned()
            .ok_or_else(|| RuntimeError::new("object class is unavailable"))
            .and_then(|value| self.class_from_base_value(value))?;

        let mut abc_meta_attrs = HashMap::new();
        abc_meta_attrs.insert("__module__".to_string(), Value::Str("abc".to_string()));
        abc_meta_attrs.insert(
            "register".to_string(),
            Value::Builtin(BuiltinFunction::AbcRegister),
        );
        abc_meta_attrs.insert(
            "__instancecheck__".to_string(),
            Value::Builtin(BuiltinFunction::AbcInstanceCheck),
        );
        abc_meta_attrs.insert(
            "__subclasscheck__".to_string(),
            Value::Builtin(BuiltinFunction::AbcSubclassCheck),
        );

        let abc_meta = match self.build_default_class_value(
            "ABCMeta".to_string(),
            abc_meta_attrs,
            vec![type_class],
            self.default_type_metaclass(),
            None,
        )? {
            Value::Class(class) => class,
            _ => return Err(RuntimeError::new("failed to create abc.ABCMeta")),
        };

        let mut abc_attrs = HashMap::new();
        abc_attrs.insert("__module__".to_string(), Value::Str("abc".to_string()));
        let abc_value = self.build_default_class_value(
            "ABC".to_string(),
            abc_attrs,
            vec![object_class],
            Some(abc_meta.clone()),
            None,
        )?;

        self.install_builtin_module(
            "abc",
            &[
                ("get_cache_token", BuiltinFunction::AbcGetCacheToken),
                ("abstractmethod", BuiltinFunction::AbcAbstractMethod),
                (
                    "update_abstractmethods",
                    BuiltinFunction::AbcUpdateAbstractMethods,
                ),
            ],
            vec![
                ("ABCMeta", Value::Class(abc_meta)),
                ("ABC", abc_value),
                (
                    "abstractclassmethod",
                    Value::Builtin(BuiltinFunction::AbcAbstractMethod),
                ),
                (
                    "abstractstaticmethod",
                    Value::Builtin(BuiltinFunction::AbcAbstractMethod),
                ),
                (
                    "abstractproperty",
                    Value::Builtin(BuiltinFunction::AbcAbstractMethod),
                ),
            ],
        );
        Ok(())
    }

    fn install_sysconfig_fallback_module(&mut self) {
        if self.modules.contains_key("sysconfig") {
            return;
        }
        self.install_builtin_module(
            "sysconfig",
            &[(
                "_get_sysconfigdata_name",
                BuiltinFunction::SysconfigGetDataName,
            )],
            Vec::new(),
        );
        self.install_module_alias_from_existing("_sysconfig", "sysconfig");
    }

    fn install_socket_fallback_module(&mut self) -> Result<(), RuntimeError> {
        if self.modules.contains_key("socket") {
            return Ok(());
        }
        let object_class = self
            .builtins
            .get("object")
            .cloned()
            .ok_or_else(|| RuntimeError::new("object class is unavailable"))
            .and_then(|value| self.class_from_base_value(value))?;
        let default_timeout = self.heap.alloc_instance(InstanceObject::new(object_class));

        let mut constants = vec![("_GLOBAL_DEFAULT_TIMEOUT", default_timeout)];
        if let Some(socket_module) = self.modules.get("_socket").cloned()
            && let Object::Module(module_data) = &*socket_module.kind()
        {
            for name in [
                "socket",
                "error",
                "timeout",
                "gaierror",
                "AF_INET",
                "AF_INET6",
                "SOCK_STREAM",
                "SOCK_DGRAM",
                "SOCK_RAW",
            ] {
                if let Some(value) = module_data.globals.get(name).cloned() {
                    constants.push((name, value));
                }
            }
        }
        self.install_builtin_module(
            "socket",
            &[("fromfd", BuiltinFunction::SocketFromFd)],
            constants,
        );
        Ok(())
    }

    fn install_builtin_import_fallback(&mut self, name: &str) -> Result<bool, RuntimeError> {
        match name {
            "abc" => {
                self.install_abc_fallback_module()?;
                Ok(true)
            }
            "sysconfig" => {
                self.install_sysconfig_fallback_module();
                Ok(true)
            }
            "_sysconfig" => {
                self.install_sysconfig_fallback_module();
                Ok(self.modules.contains_key("_sysconfig"))
            }
            "socket" => {
                self.install_socket_fallback_module()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn load_module(&mut self, name: &str) -> Result<ObjRef, RuntimeError> {
        if std::env::var_os("PYRS_TRACE_MODULE_LOAD").is_some() {
            eprintln!("[module-load] {name}");
        }
        if let Some(module) = self.modules.get(name).cloned() {
            if !self.module_requires_realization(name, &module) {
                return Ok(module);
            }
            self.remove_module_entry_and_parent_binding(name);
        }

        if let Some((parent, _)) = name.rsplit_once('.') {
            let parent_needs_load = self
                .modules
                .get(parent)
                .cloned()
                .is_none_or(|parent_module| {
                    self.module_requires_realization(parent, &parent_module)
                });
            if parent_needs_load {
                let parent_caller_depth = self.frames.len();
                let _ = self.load_module(parent)?;
                self.run_pending_import_frames(parent_caller_depth)?;
                if self
                    .modules
                    .get(parent)
                    .is_some_and(Self::module_is_initializing)
                {
                    self.run_pending_import_frames_force(parent_caller_depth)?;
                }
                // Parent package initialization may import this child module as a side effect.
                // Re-check caches before creating/executing the module a second time.
                if let Some(module) = self.modules.get(name).cloned() {
                    return Ok(module);
                }
                if let Some(modules_dict) = self.sys_dict_obj("modules")
                    && let Some(Value::Module(module)) =
                        dict_get_value(&modules_dict, &Value::Str(name.to_string()))
                {
                    self.modules.insert(name.to_string(), module.clone());
                    return Ok(module);
                }
            }
        }

        let source_info = match self.find_module_source(name) {
            Some(info) => info,
            None => {
                if self.install_builtin_import_fallback(name)?
                    && let Some(module) = self.modules.get(name).cloned()
                {
                    return Ok(module);
                }
                return Err(RuntimeError::module_not_found_error(format!(
                    "module '{name}' not found"
                )));
            }
        };
        let loader_name = if source_info.is_namespace {
            NAMESPACE_LOADER
        } else if source_info.is_extension {
            EXTENSION_FILE_LOADER
        } else if source_info.is_bytecode {
            SOURCELESS_FILE_LOADER
        } else {
            SOURCE_FILE_LOADER
        };

        let module = self.create_module_for_loader(name, loader_name)?;
        let (metadata_origin, metadata_cached) = self.module_origin_and_cached_paths(&source_info);
        self.set_module_metadata(
            &module,
            name,
            metadata_origin.as_ref(),
            metadata_cached.as_ref(),
            Some(loader_name),
            source_info.is_package,
            source_info.package_dirs.clone(),
            source_info.is_namespace,
        );

        self.register_module(name, module.clone());
        self.link_module_chain(name, module.clone());
        if let Err(err) = self.exec_module_for_loader(&module, name, loader_name, &source_info) {
            self.remove_module_entry_and_parent_binding(name);
            return Err(err);
        }
        Ok(module)
    }

    pub(super) fn create_module_for_loader(
        &mut self,
        name: &str,
        loader_name: &str,
    ) -> Result<ObjRef, RuntimeError> {
        match loader_name {
            SOURCE_FILE_LOADER
            | SOURCELESS_FILE_LOADER
            | NAMESPACE_LOADER
            | EXTENSION_FILE_LOADER => match self.heap.alloc_module(ModuleObject::new(name)) {
                Value::Module(obj) => Ok(obj),
                _ => unreachable!(),
            },
            _ => Err(RuntimeError::new(format!(
                "unsupported loader for module creation: {loader_name}"
            ))),
        }
    }

    pub(super) fn module_origin_and_cached_paths(
        &self,
        source_info: &ModuleSourceInfo,
    ) -> (Option<PathBuf>, Option<PathBuf>) {
        if source_info.is_namespace {
            return (None, None);
        }
        if source_info.is_bytecode {
            let cached = Some(source_info.path.clone());
            let source_path = PathBuf::from(source_path_from_cache_path(
                &source_info.path.to_string_lossy(),
            ));
            if source_path.is_file() {
                return (Some(source_path), cached);
            }
            return (Some(source_info.path.clone()), cached);
        }
        (Some(source_info.path.clone()), None)
    }

    pub(super) fn exec_module_for_loader(
        &mut self,
        module: &ObjRef,
        name: &str,
        loader_name: &str,
        source_info: &ModuleSourceInfo,
    ) -> Result<(), RuntimeError> {
        match loader_name {
            NAMESPACE_LOADER => Ok(()),
            SOURCE_FILE_LOADER => {
                self.queue_source_module_execution(module, name, &source_info.path)
            }
            EXTENSION_FILE_LOADER => {
                self.mark_module_initializing(module);
                let result = self.exec_extension_module(module, name, &source_info.path);
                self.clear_module_initializing(module);
                result
            }
            SOURCELESS_FILE_LOADER => {
                if self.import_perf_enabled {
                    self.import_perf_counters.pyc_load_attempts = self
                        .import_perf_counters
                        .pyc_load_attempts
                        .saturating_add(1);
                }
                let bytes = std::fs::read(&source_info.path).map_err(|err| {
                    RuntimeError::new(format!("failed to read module '{name}': {err}"))
                })?;
                let pyc_result = cpython::load_pyc(&bytes)
                    .map_err(|err| RuntimeError::new(format!("pyc load error: {}", err.message)))
                    .and_then(|pyc| {
                        cpython::translate_code(&pyc, &mut self.heap).map_err(|err| {
                            RuntimeError::new(format!("pyc translate error: {}", err.message))
                        })
                    });
                match pyc_result {
                    Ok(code) => {
                        self.mark_module_initializing(module);
                        let code = Rc::new(code);
                        let cells = self.build_cells(&code, Vec::new());
                        let mut frame = Frame::new(code, module.clone(), true, false, cells, None);
                        frame.discard_result = true;
                        self.frames.push(Box::new(frame));
                        Ok(())
                    }
                    Err(pyc_err) => {
                        let source_path = PathBuf::from(source_path_from_cache_path(
                            &source_info.path.to_string_lossy(),
                        ));
                        if std::env::var_os("PYRS_IMPORT_PERF_VERBOSE").is_some() {
                            eprintln!(
                                "[import-perf] pyc-fallback module={name} pyc={} source={} source_exists={} reason={}",
                                source_info.path.display(),
                                source_path.display(),
                                source_path.is_file(),
                                pyc_err.message
                            );
                        }
                        if source_path.is_file() {
                            if self.import_perf_enabled {
                                self.import_perf_counters.pyc_load_fallback_to_source = self
                                    .import_perf_counters
                                    .pyc_load_fallback_to_source
                                    .saturating_add(1);
                            }
                            self.set_module_metadata(
                                module,
                                name,
                                Some(&source_path),
                                None,
                                Some(SOURCE_FILE_LOADER),
                                source_info.is_package,
                                source_info.package_dirs.clone(),
                                source_info.is_namespace,
                            );
                            self.queue_source_module_execution(module, name, &source_path)
                        } else {
                            Err(pyc_err)
                        }
                    }
                }
            }
            _ => Err(RuntimeError::new(format!(
                "unsupported loader for module execution: {loader_name}"
            ))),
        }
    }

    pub(super) fn find_module_source(&mut self, name: &str) -> Option<ModuleSourceInfo> {
        self.refresh_import_resolver_state();
        if !self.import_meta_path_has_default_finder {
            return None;
        }
        self.path_finder_find_spec(name)
    }

    pub(super) fn path_finder_find_spec(&mut self, name: &str) -> Option<ModuleSourceInfo> {
        if LOCAL_SHIM_PRECEDENCE_MODULES.contains(&name)
            && let Some(shim_source) = self.preferred_local_shim_source(name)
        {
            return Some(shim_source);
        }
        if let Some((parent_name, child_name)) = name.rsplit_once('.')
            && let Some(parent_paths) = self.package_search_paths(parent_name)
            && let Some(source) = self.find_module_source_in_roots(child_name, &parent_paths)
        {
            return Some(source);
        }
        let roots = self.module_paths.clone();
        if let Some(source) = self.find_module_source_in_roots(name, &roots) {
            return Some(source);
        }
        if let Some(alias_name) = Self::module_source_alias(name)
            && let Some(source) = self.find_module_source_in_roots(alias_name, &roots)
        {
            return Some(source);
        }
        if !self.local_shim_fallback_enabled {
            return None;
        }
        // Only fall back to local shims when normal path resolution fails.
        self.preferred_local_shim_source(name)
    }

    pub(super) fn preferred_local_shim_source(&mut self, name: &str) -> Option<ModuleSourceInfo> {
        if !LOCAL_SHIM_MODULES.contains(&name) {
            return None;
        }
        let shim_root = Self::local_shim_root()?;
        self.find_module_source_in_single_root(name, &shim_root)
    }

    pub(super) fn package_search_paths(&self, package_name: &str) -> Option<Vec<PathBuf>> {
        let package = self.modules.get(package_name)?.clone();
        let package_kind = package.kind();
        let module_data = match &*package_kind {
            Object::Module(module) => module,
            _ => return None,
        };
        let path_value = module_data.globals.get("__path__")?;
        let path_list = match path_value {
            Value::List(list) => list.clone(),
            _ => return None,
        };
        let list_kind = path_list.kind();
        let values = match &*list_kind {
            Object::List(values) => values,
            _ => return None,
        };
        let mut roots = Vec::new();
        for value in values {
            if let Value::Str(path) = value {
                roots.push(PathBuf::from(path));
            }
        }
        if roots.is_empty() { None } else { Some(roots) }
    }

    pub(super) fn find_module_source_in_roots(
        &mut self,
        module_name: &str,
        roots: &[PathBuf],
    ) -> Option<ModuleSourceInfo> {
        let mut namespace_dirs = Vec::new();
        let mut bytecode_fallback: Option<ModuleSourceInfo> = None;
        for root in roots {
            let importer = match self.path_importer_for_root(root) {
                Some(importer) => importer,
                None => continue,
            };
            if let Some(spec) = self.find_module_source_with_importer(&importer, module_name) {
                if spec.is_namespace {
                    namespace_dirs.extend(spec.package_dirs);
                    continue;
                }
                if spec.is_bytecode {
                    if bytecode_fallback.is_none() {
                        bytecode_fallback = Some(spec);
                    }
                    continue;
                }
                return Some(spec);
            }
        }
        if let Some(spec) = bytecode_fallback {
            return Some(spec);
        }
        if !namespace_dirs.is_empty() {
            return Some(ModuleSourceInfo {
                path: namespace_dirs[0].clone(),
                is_package: true,
                package_dirs: namespace_dirs,
                is_namespace: true,
                is_bytecode: false,
                is_extension: false,
            });
        }
        None
    }

    pub(super) fn path_importer_for_root(&mut self, root: &std::path::Path) -> Option<Value> {
        let key = Value::Str(root.to_string_lossy().to_string());
        if let Some(cache_dict) = self.sys_dict_obj("path_importer_cache") {
            if let Some(cached) = dict_get_value(&cache_dict, &key) {
                return if matches!(cached, Value::None) {
                    None
                } else {
                    Some(cached)
                };
            }

            let importer = self.run_path_hooks_for_root(root);
            let cached_value = importer.clone().unwrap_or(Value::None);
            dict_set_value(&cache_dict, key, cached_value.clone());
            return if matches!(cached_value, Value::None) {
                None
            } else {
                Some(cached_value)
            };
        }
        self.run_path_hooks_for_root(root)
    }

    pub(super) fn run_path_hooks_for_root(&mut self, root: &std::path::Path) -> Option<Value> {
        if self.import_path_hooks_has_default_hook {
            Some(self.make_file_finder_importer(root))
        } else {
            None
        }
    }

    pub(super) fn make_file_finder_importer(&self, root: &std::path::Path) -> Value {
        self.heap.alloc_dict(vec![
            (
                Value::Str("kind".to_string()),
                Value::Str(DEFAULT_PATH_HOOK.to_string()),
            ),
            (
                Value::Str("path".to_string()),
                Value::Str(root.to_string_lossy().to_string()),
            ),
        ])
    }

    pub(super) fn find_module_source_with_importer(
        &mut self,
        importer: &Value,
        module_name: &str,
    ) -> Option<ModuleSourceInfo> {
        let importer_dict = match importer {
            Value::Dict(dict) => dict.clone(),
            _ => return None,
        };
        let kind = match dict_get_value(&importer_dict, &Value::Str("kind".to_string())) {
            Some(Value::Str(kind)) => kind,
            _ => return None,
        };
        if kind != DEFAULT_PATH_HOOK {
            None
        } else {
            let root = match dict_get_value(&importer_dict, &Value::Str("path".to_string())) {
                Some(Value::Str(path)) => PathBuf::from(path),
                _ => return None,
            };
            self.find_module_source_in_single_root(module_name, &root)
        }
    }

    fn cache_module_source_positive(
        &mut self,
        cache_key: &(PathBuf, String),
        spec: ModuleSourceInfo,
    ) -> ModuleSourceInfo {
        self.module_source_positive_cache
            .insert(cache_key.clone(), spec.clone());
        spec
    }

    pub(super) fn find_module_source_in_single_root(
        &mut self,
        module_name: &str,
        root: &std::path::Path,
    ) -> Option<ModuleSourceInfo> {
        let cache_key = (root.to_path_buf(), module_name.to_string());
        if let Some(cached) = self.module_source_positive_cache.get(&cache_key) {
            return Some(cached.clone());
        }
        let rel_name = module_name.replace('.', "/");
        let candidate = root.join(format!("{rel_name}.py"));
        let pyc_candidate = cached_module_path(root, &rel_name);
        let direct_pyc = root.join(format!("{rel_name}.pyc"));
        if self.cached_path_is_file(&candidate) {
            if self.prefer_pyc_when_source_available
                && self.cached_path_is_file(&pyc_candidate)
                && Self::pyc_matches_source(&pyc_candidate, &candidate)
            {
                return Some(self.cache_module_source_positive(
                    &cache_key,
                    ModuleSourceInfo {
                        path: pyc_candidate,
                        is_package: false,
                        package_dirs: Vec::new(),
                        is_namespace: false,
                        is_bytecode: true,
                        is_extension: false,
                    },
                ));
            }
            if self.prefer_pyc_when_source_available
                && self.cached_path_is_file(&direct_pyc)
                && Self::pyc_matches_source(&direct_pyc, &candidate)
            {
                return Some(self.cache_module_source_positive(
                    &cache_key,
                    ModuleSourceInfo {
                        path: direct_pyc,
                        is_package: false,
                        package_dirs: Vec::new(),
                        is_namespace: false,
                        is_bytecode: true,
                        is_extension: false,
                    },
                ));
            }
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: candidate,
                    is_package: false,
                    package_dirs: Vec::new(),
                    is_namespace: false,
                    is_bytecode: false,
                    is_extension: false,
                },
            ));
        }
        if self.cached_path_is_file(&direct_pyc) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: direct_pyc,
                    is_package: false,
                    package_dirs: Vec::new(),
                    is_namespace: false,
                    is_bytecode: true,
                    is_extension: false,
                },
            ));
        }
        if self.cached_path_is_file(&pyc_candidate) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: pyc_candidate,
                    is_package: false,
                    package_dirs: Vec::new(),
                    is_namespace: false,
                    is_bytecode: true,
                    is_extension: false,
                },
            ));
        }
        if let Some(library_candidate) = find_shared_library_for_module(root, &rel_name) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: library_candidate,
                    is_package: false,
                    package_dirs: Vec::new(),
                    is_namespace: false,
                    is_bytecode: false,
                    is_extension: true,
                },
            ));
        }
        let extension_manifest = root.join(format!("{rel_name}{PYRS_EXTENSION_MANIFEST_SUFFIX}"));
        if self.cached_path_is_file(&extension_manifest) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: extension_manifest,
                    is_package: false,
                    package_dirs: Vec::new(),
                    is_namespace: false,
                    is_bytecode: false,
                    is_extension: true,
                },
            ));
        }
        let package_dir = root.join(&rel_name);
        let package_init = package_dir.join("__init__.py");
        let package_init_pyc = package_dir
            .join("__pycache__")
            .join("__init__.cpython-314.pyc");
        let direct_package_init_pyc = package_dir.join("__init__.pyc");
        if self.cached_path_is_file(&package_init) {
            if self.prefer_pyc_when_source_available
                && self.cached_path_is_file(&package_init_pyc)
                && Self::pyc_matches_source(&package_init_pyc, &package_init)
            {
                return Some(self.cache_module_source_positive(
                    &cache_key,
                    ModuleSourceInfo {
                        path: package_init_pyc,
                        is_package: true,
                        package_dirs: vec![package_dir],
                        is_namespace: false,
                        is_bytecode: true,
                        is_extension: false,
                    },
                ));
            }
            if self.prefer_pyc_when_source_available
                && self.cached_path_is_file(&direct_package_init_pyc)
                && Self::pyc_matches_source(&direct_package_init_pyc, &package_init)
            {
                return Some(self.cache_module_source_positive(
                    &cache_key,
                    ModuleSourceInfo {
                        path: direct_package_init_pyc,
                        is_package: true,
                        package_dirs: vec![package_dir],
                        is_namespace: false,
                        is_bytecode: true,
                        is_extension: false,
                    },
                ));
            }
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: package_init,
                    is_package: true,
                    package_dirs: vec![package_dir],
                    is_namespace: false,
                    is_bytecode: false,
                    is_extension: false,
                },
            ));
        }
        if self.cached_path_is_file(&direct_package_init_pyc) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: direct_package_init_pyc,
                    is_package: true,
                    package_dirs: vec![package_dir],
                    is_namespace: false,
                    is_bytecode: true,
                    is_extension: false,
                },
            ));
        }
        if self.cached_path_is_file(&package_init_pyc) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: package_init_pyc,
                    is_package: true,
                    package_dirs: vec![package_dir],
                    is_namespace: false,
                    is_bytecode: true,
                    is_extension: false,
                },
            ));
        }
        if let Some(library_candidate) = find_shared_library_for_package(&package_dir) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: library_candidate,
                    is_package: true,
                    package_dirs: vec![package_dir],
                    is_namespace: false,
                    is_bytecode: false,
                    is_extension: true,
                },
            ));
        }
        let package_extension_manifest =
            package_dir.join(format!("__init__{PYRS_EXTENSION_MANIFEST_SUFFIX}"));
        if self.cached_path_is_file(&package_extension_manifest) {
            return Some(self.cache_module_source_positive(
                &cache_key,
                ModuleSourceInfo {
                    path: package_extension_manifest,
                    is_package: true,
                    package_dirs: vec![package_dir],
                    is_namespace: false,
                    is_bytecode: false,
                    is_extension: true,
                },
            ));
        }
        if self.cached_path_is_dir(&package_dir) {
            return Some(ModuleSourceInfo {
                path: package_dir.clone(),
                is_package: true,
                package_dirs: vec![package_dir],
                is_namespace: true,
                is_bytecode: false,
                is_extension: false,
            });
        }
        None
    }

    pub(super) fn sys_dict_obj(&self, name: &str) -> Option<ObjRef> {
        let sys_module = self.modules.get("sys")?.clone();
        let module_kind = sys_module.kind();
        let module_data = match &*module_kind {
            Object::Module(module_data) => module_data,
            _ => return None,
        };
        match module_data.globals.get(name) {
            Some(Value::Dict(dict)) => Some(dict.clone()),
            _ => None,
        }
    }

    pub(super) fn find_module_file(&mut self, name: &str) -> Option<PathBuf> {
        self.find_module_source(name).map(|info| info.path)
    }

    pub(super) fn load_submodule_with_error(
        &mut self,
        parent: &ObjRef,
        attr_name: &str,
    ) -> Result<Option<ObjRef>, RuntimeError> {
        let parent_name = match &*parent.kind() {
            Object::Module(module) => module.name.clone(),
            _ => return Ok(None),
        };
        if std::env::var_os("PYRS_TRACE_SUBMODULE").is_some() {
            let seen = SUBMODULE_TRACE_COUNT.fetch_add(1, AtomicOrdering::Relaxed);
            if seen < 200 {
                eprintln!("[submodule] parent={parent_name} attr={attr_name}");
            } else if seen == 200 {
                eprintln!("[submodule] trace limit reached; suppressing further output");
            }
        }
        let full_name = format!("{}.{}", parent_name, attr_name);
        let key = Value::Str(full_name.clone());
        let mut missing_from_sys_modules = false;
        if let Some(modules_dict) = self.sys_dict_obj("modules") {
            match dict_get_value(&modules_dict, &key) {
                Some(Value::Module(module)) => {
                    self.modules.insert(full_name.clone(), module.clone());
                    return Ok(Some(module));
                }
                Some(Value::None) => {
                    self.modules.remove(&full_name);
                    return Ok(None);
                }
                Some(_) => {
                    self.modules.remove(&full_name);
                    return Ok(None);
                }
                None => {
                    missing_from_sys_modules = true;
                }
            }
        }
        if missing_from_sys_modules {
            self.modules.remove(&full_name);
        } else if let Some(module) = self.modules.get(&full_name).cloned() {
            return Ok(Some(module));
        }
        if self.find_module_file(&full_name).is_some() {
            let caller_depth = self.frames.len();
            let module = self.import_module_object(&full_name)?;
            self.run_pending_import_frames(caller_depth)?;
            let module = self.canonical_imported_module_for_name(&full_name, module);
            self.upsert_module_global(parent, attr_name, Value::Module(module.clone()));
            return Ok(Some(module));
        }
        Ok(None)
    }

    pub(super) fn load_submodule(&mut self, parent: &ObjRef, attr_name: &str) -> Option<ObjRef> {
        self.load_submodule_with_error(parent, attr_name)
            .ok()
            .flatten()
    }

    pub(super) fn ensure_module(&mut self, name: &str) -> ObjRef {
        if let Some(module) = self.modules.get(name).cloned() {
            return module;
        }
        let module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(&module, name, None, None, None, false, Vec::new(), false);
        self.register_module(name, module.clone());
        module
    }

    pub(super) fn set_module_metadata(
        &mut self,
        module: &ObjRef,
        name: &str,
        origin: Option<&PathBuf>,
        cached: Option<&PathBuf>,
        loader_name: Option<&str>,
        is_package: bool,
        package_dirs: Vec<PathBuf>,
        is_namespace: bool,
    ) {
        let package_name = if is_package {
            name.to_string()
        } else {
            name.rsplit_once('.')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default()
        };
        let loader_value = loader_name
            .map(|loader| Value::Str(loader.to_string()))
            .unwrap_or(Value::None);
        let origin_value = origin
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .unwrap_or(Value::None);
        let cached_value = cached
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .unwrap_or(Value::None);
        let submodule_locations = if is_package {
            let mut entries = Vec::new();
            for dir in package_dirs.iter() {
                entries.push(Value::Str(dir.to_string_lossy().to_string()));
            }
            self.heap.alloc_list(entries)
        } else {
            Value::None
        };
        let spec_value = self.build_module_spec_value(
            name,
            origin,
            cached,
            loader_name,
            is_package,
            package_dirs.as_slice(),
            is_namespace,
        );

        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name.to_string()));
            module_data
                .globals
                .entry("__doc__".to_string())
                .or_insert(Value::None);
            module_data
                .globals
                .insert("__package__".to_string(), Value::Str(package_name));
            module_data
                .globals
                .insert("__loader__".to_string(), loader_value);
            module_data
                .globals
                .insert("__spec__".to_string(), spec_value);
            if origin.is_some() {
                module_data
                    .globals
                    .insert("__file__".to_string(), origin_value);
            }
            match cached_value {
                Value::None => {
                    module_data.globals.remove("__cached__");
                }
                _ => {
                    module_data
                        .globals
                        .insert("__cached__".to_string(), cached_value);
                }
            }
            if is_package {
                module_data
                    .globals
                    .insert("__path__".to_string(), submodule_locations);
            }
            if name == "test.support" {
                // `test.support` can be imported recursively by helper modules.
                // Seed platform flags so early accesses during cycle handling work.
                module_data
                    .globals
                    .entry("is_apple".to_string())
                    .or_insert(Value::Bool(false));
                module_data
                    .globals
                    .entry("is_apple_mobile".to_string())
                    .or_insert(Value::Bool(false));
                module_data
                    .globals
                    .entry("is_wasi".to_string())
                    .or_insert(Value::Bool(false));
                module_data
                    .globals
                    .entry("is_emscripten".to_string())
                    .or_insert(Value::Bool(false));
            }
        }
    }

    pub(super) fn loader_spec_value(&mut self, loader_name: Option<&str>) -> Value {
        let Some(loader_name) = loader_name else {
            return Value::None;
        };
        let class_name = loader_name
            .rsplit_once('.')
            .map(|(_, name)| name)
            .unwrap_or(loader_name);
        let class = match self
            .heap
            .alloc_class(ClassObject::new(class_name.to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data
                .attrs
                .insert("__name__".to_string(), Value::Str(class_name.to_string()));
            class_data.attrs.insert(
                "__qualname__".to_string(),
                Value::Str(class_name.to_string()),
            );
            class_data.attrs.insert(
                "__module__".to_string(),
                Value::Str("importlib.machinery".to_string()),
            );
        }
        self.heap.alloc_instance(InstanceObject::new(class))
    }

    pub(super) fn build_module_spec_value(
        &mut self,
        name: &str,
        origin: Option<&PathBuf>,
        cached: Option<&PathBuf>,
        loader_name: Option<&str>,
        is_package: bool,
        package_dirs: &[PathBuf],
        is_namespace: bool,
    ) -> Value {
        let parent = name
            .rsplit_once('.')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
        let loader_value = self.loader_spec_value(loader_name);
        let origin_value = origin
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .unwrap_or(Value::None);
        let cached_value = cached
            .map(|path| Value::Str(path.to_string_lossy().to_string()))
            .unwrap_or(Value::None);
        let submodule_locations = if is_package {
            let mut entries = Vec::new();
            for dir in package_dirs {
                entries.push(Value::Str(dir.to_string_lossy().to_string()));
            }
            self.heap.alloc_list(entries)
        } else {
            Value::None
        };

        let spec_class = self
            .modules
            .get("_frozen_importlib")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("ModuleSpec").cloned(),
                _ => None,
            })
            .and_then(|value| match value {
                Value::Class(class) => Some(class),
                _ => None,
            })
            .unwrap_or_else(|| {
                let class_value = self
                    .heap
                    .alloc_class(ClassObject::new("ModuleSpec".to_string(), Vec::new()));
                match class_value {
                    Value::Class(class) => class,
                    _ => unreachable!(),
                }
            });
        let spec = match self.heap.alloc_instance(InstanceObject::new(spec_class)) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *spec.kind_mut() {
            instance_data
                .attrs
                .insert("name".to_string(), Value::Str(name.to_string()));
            instance_data
                .attrs
                .insert("origin".to_string(), origin_value);
            instance_data
                .attrs
                .insert("loader".to_string(), loader_value);
            instance_data
                .attrs
                .insert("parent".to_string(), Value::Str(parent));
            instance_data.attrs.insert(
                "submodule_search_locations".to_string(),
                submodule_locations,
            );
            instance_data
                .attrs
                .insert("is_package".to_string(), Value::Bool(is_package));
            instance_data
                .attrs
                .insert("is_namespace".to_string(), Value::Bool(is_namespace));
            instance_data
                .attrs
                .insert("has_location".to_string(), Value::Bool(origin.is_some()));
            instance_data
                .attrs
                .insert("cached".to_string(), cached_value);
        }
        Value::Instance(spec)
    }

    pub(super) fn set_module_spec_field(&self, spec: &Value, field: &str, value: Value) {
        match spec {
            Value::Module(spec_obj) => {
                if let Object::Module(module_data) = &mut *spec_obj.kind_mut() {
                    module_data.globals.insert(field.to_string(), value);
                }
            }
            Value::Dict(spec_obj) => {
                dict_set_value(spec_obj, Value::Str(field.to_string()), value);
            }
            Value::Instance(spec_obj) => {
                if let Object::Instance(instance_data) = &mut *spec_obj.kind_mut() {
                    instance_data.attrs.insert(field.to_string(), value);
                }
            }
            _ => {}
        }
    }

    pub(super) fn link_module_chain(&mut self, name: &str, module: ObjRef) {
        let parts: Vec<&str> = name.split('.').collect();
        if parts.len() <= 1 {
            return;
        }

        let mut current_name = parts[0].to_string();
        let mut current_module = self.ensure_module(&current_name);

        for part in parts.iter().skip(1) {
            let child_name = format!("{current_name}.{part}");
            let child_module = if child_name == name {
                module.clone()
            } else {
                self.ensure_module(&child_name)
            };
            self.upsert_module_global(&current_module, part, Value::Module(child_module.clone()));
            current_module = child_module;
            current_name = child_name;
        }
    }

    pub(super) fn import_module_object(&mut self, name: &str) -> Result<ObjRef, RuntimeError> {
        self.sync_module_paths_from_sys();
        let caller_depth = self.frames.len();
        let existing_modules: HashSet<String> = self.modules.keys().cloned().collect();
        let key = Value::Str(name.to_string());
        let mut present_in_sys_modules = false;
        if let Some(modules_dict) = self.sys_dict_obj("modules") {
            self.prune_module_cache_for_removed_sys_modules(&modules_dict);
            let sys_entry = dict_get_value(&modules_dict, &key);
            match sys_entry {
                Some(Value::Module(module)) => {
                    present_in_sys_modules = true;
                    if self.should_prefer_filesystem_module(name, &module) {
                        self.modules.remove(name);
                        let _ = dict_remove_value(&modules_dict, &key);
                    } else {
                        self.modules.insert(name.to_string(), module.clone());
                        return self.return_imported_module(module, caller_depth);
                    }
                }
                Some(Value::None) => {
                    self.modules.remove(name);
                    return Err(RuntimeError::module_not_found_error(format!(
                        "No module named '{}'",
                        name
                    )));
                }
                Some(value) => {
                    present_in_sys_modules = true;
                    if let Some(module) =
                        self.coerce_sys_modules_entry_to_module(name, &value, &modules_dict)
                    {
                        self.modules.insert(name.to_string(), module.clone());
                        return self.return_imported_module(module, caller_depth);
                    }
                }
                None => {}
            }
        }
        if !present_in_sys_modules {
            let keep_cached_builtin = if let Some(module) = self.modules.get(name).cloned() {
                Self::module_loader_name(&module).as_deref() == Some(BUILTIN_MODULE_LOADER)
                    && !self.should_prefer_filesystem_module(name, &module)
            } else {
                false
            };
            let keep_cached_initializing = self
                .modules
                .get(name)
                .is_some_and(Self::module_is_initializing);
            if !keep_cached_builtin && !keep_cached_initializing {
                self.modules.remove(name);
            }
        }
        if let Some(module) = self.modules.get(name).cloned() {
            if self.should_prefer_filesystem_module(name, &module) {
                self.modules.remove(name);
                if let Some(modules_dict) = self.sys_dict_obj("modules") {
                    let _ = dict_remove_value(&modules_dict, &key);
                }
            } else {
                if !present_in_sys_modules && let Some(modules_dict) = self.sys_dict_obj("modules")
                {
                    dict_set_value(
                        &modules_dict,
                        Value::Str(name.to_string()),
                        Value::Module(module.clone()),
                    );
                }
                return self.return_imported_module(module, caller_depth);
            }
        }
        if let Some(module) = self.import_module_via_meta_path(name, caller_depth)? {
            return Ok(module);
        }
        match self.load_module(name) {
            Ok(module) => match self.return_imported_module(module, caller_depth) {
                Ok(module) => Ok(module),
                Err(err) => {
                    self.cleanup_failed_import(name, &existing_modules);
                    Err(err)
                }
            },
            Err(load_err) => {
                if let Some((parent, _)) = name.rsplit_once('.') {
                    let _ = self.import_module_object(parent)?;
                    // Do not "rescue" modules that were first introduced during this failed
                    // import attempt. Returning such partially-initialized modules masks the
                    // underlying exception and causes silent stdlib/third-party corruption.
                    if existing_modules.contains(name) {
                        if let Some(module) = self.modules.get(name).cloned() {
                            if let Some(modules_dict) = self.sys_dict_obj("modules") {
                                dict_set_value(
                                    &modules_dict,
                                    Value::Str(name.to_string()),
                                    Value::Module(module.clone()),
                                );
                            }
                            return self.return_imported_module(module, caller_depth);
                        }
                        if let Some(modules_dict) = self.sys_dict_obj("modules") {
                            let key = Value::Str(name.to_string());
                            match dict_get_value(&modules_dict, &key) {
                                Some(Value::Module(module)) => {
                                    self.modules.insert(name.to_string(), module.clone());
                                    return self.return_imported_module(module, caller_depth);
                                }
                                Some(Value::None) => {
                                    return Err(RuntimeError::module_not_found_error(format!(
                                        "No module named '{}'",
                                        name
                                    )));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                self.cleanup_failed_import(name, &existing_modules);
                Err(load_err)
            }
        }
    }

    pub(super) fn prune_module_cache_for_removed_sys_modules(&mut self, modules_dict: &ObjRef) {
        let Object::Dict(entries) = &*modules_dict.kind() else {
            return;
        };
        let mut present = HashSet::with_capacity(entries.len());
        for (key, _) in entries.iter() {
            if let Value::Str(name) = key {
                present.insert(name.clone());
            }
        }
        let module_entries = self
            .modules
            .iter()
            .map(|(name, module)| (name.clone(), module.clone()))
            .collect::<Vec<_>>();
        let stale = module_entries
            .iter()
            .filter_map(|(name, module)| {
                if present.contains(name) {
                    return None;
                }
                let is_builtin =
                    Self::module_loader_name(module).as_deref() == Some(BUILTIN_MODULE_LOADER);
                let preserve_builtin =
                    is_builtin && !self.should_prefer_filesystem_module(name, module);
                if preserve_builtin {
                    None
                } else {
                    Some(name.clone())
                }
            })
            .collect::<Vec<_>>();
        for name in stale {
            self.modules.remove(&name);
        }
    }

    pub(super) fn remove_module_entry_and_parent_binding(&mut self, name: &str) {
        self.unregister_module(name);
        if let Some((parent, child)) = name.rsplit_once('.')
            && let Some(parent_module) = self.modules.get(parent)
            && let Object::Module(parent_data) = &mut *parent_module.kind_mut()
        {
            parent_data.globals.remove(child);
        }
    }

    pub(super) fn cleanup_failed_import(
        &mut self,
        failed_name: &str,
        existing_modules: &HashSet<String>,
    ) {
        let failed_prefix = format!("{failed_name}.");
        let failed_new_modules: Vec<String> = self
            .modules
            .keys()
            .filter_map(|name| {
                if existing_modules.contains(name) {
                    return None;
                }
                if name == failed_name || name.starts_with(&failed_prefix) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        for name in failed_new_modules {
            self.remove_module_entry_and_parent_binding(&name);
            self.extension_init_failures.remove(&name);
        }
        self.extension_init_failures.remove(failed_name);
        self.cleanup_partial_modules(existing_modules);
    }

    fn module_requires_realization(&mut self, name: &str, module: &ObjRef) -> bool {
        if Self::module_is_initializing(module) {
            return false;
        }
        if Self::module_loader_name(module).is_some() {
            return false;
        }
        self.find_module_source(name).is_some()
    }

    fn coerce_sys_modules_entry_to_module(
        &mut self,
        name: &str,
        value: &Value,
        modules_dict: &ObjRef,
    ) -> Option<ObjRef> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let (class, instance_attrs) = match &*instance.kind() {
            Object::Instance(instance_data) => {
                (instance_data.class.clone(), instance_data.attrs.clone())
            }
            _ => return None,
        };
        if !self.class_has_builtin_module_base(&class) {
            return None;
        }
        let class_attrs = match &*class.kind() {
            Object::Class(class_data) => class_data.attrs.clone(),
            _ => HashMap::new(),
        };
        let module = self.ensure_module(name);
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            for (attr, value) in instance_attrs {
                module_data.globals.insert(attr, value);
            }
            for (attr, value) in class_attrs {
                if matches!(
                    attr.as_str(),
                    "__dict__" | "__weakref__" | "__mro__" | "__bases__" | "__new__" | "__init__"
                ) {
                    continue;
                }
                module_data.globals.entry(attr).or_insert(value);
            }
            module_data
                .globals
                .entry("__name__".to_string())
                .or_insert_with(|| Value::Str(name.to_string()));
        }
        dict_set_value(
            modules_dict,
            Value::Str(name.to_string()),
            Value::Module(module.clone()),
        );
        Some(module)
    }

    fn import_module_via_meta_path(
        &mut self,
        name: &str,
        caller_depth: usize,
    ) -> Result<Option<ObjRef>, RuntimeError> {
        let trace_meta_path = std::env::var_os("PYRS_TRACE_META_PATH").is_some();
        let Some(meta_path_obj) = self.sys_list_obj("meta_path") else {
            return Ok(None);
        };
        let finders = match &*meta_path_obj.kind() {
            Object::List(values) => values.clone(),
            _ => return Ok(None),
        };
        let name_value = Value::Str(name.to_string());

        for finder in finders {
            if matches_finder_kind(&finder, DEFAULT_META_PATH_FINDER)
                || matches!(finder, Value::Str(_))
            {
                continue;
            }
            if trace_meta_path {
                eprintln!(
                    "[meta-path] probe name={} finder={}",
                    name,
                    self.value_type_name_for_error(&finder)
                );
            }

            let find_spec = match self.builtin_getattr(
                vec![finder.clone(), Value::Str("find_spec".to_string())],
                HashMap::new(),
            ) {
                Ok(callable) => callable,
                Err(_) => {
                    if trace_meta_path {
                        eprintln!("[meta-path] finder has no find_spec");
                    }
                    continue;
                }
            };
            let spec = match self.call_internal(
                find_spec,
                vec![name_value.clone(), Value::None, Value::None],
                HashMap::new(),
            ) {
                Ok(super::InternalCallOutcome::Value(value)) => value,
                Ok(super::InternalCallOutcome::CallerExceptionHandled) => {
                    if trace_meta_path {
                        eprintln!("[meta-path] find_spec handled exception");
                    }
                    continue;
                }
                Err(err) => {
                    if trace_meta_path {
                        eprintln!("[meta-path] find_spec error: {}", err.message);
                    }
                    continue;
                }
            };
            if matches!(spec, Value::None) {
                if trace_meta_path {
                    eprintln!("[meta-path] spec=None");
                }
                continue;
            }
            if trace_meta_path {
                eprintln!(
                    "[meta-path] spec found type={}",
                    self.value_type_name_for_error(&spec)
                );
            }

            let loader = self
                .builtin_getattr(
                    vec![spec.clone(), Value::Str("loader".to_string())],
                    HashMap::new(),
                )
                .ok()
                .filter(|value| !matches!(value, Value::None))
                .unwrap_or_else(|| finder.clone());

            let create_module = self
                .builtin_getattr(
                    vec![loader.clone(), Value::Str("create_module".to_string())],
                    HashMap::new(),
                )
                .ok();
            let load_module = self
                .builtin_getattr(
                    vec![loader.clone(), Value::Str("load_module".to_string())],
                    HashMap::new(),
                )
                .ok();
            let exec_module = self
                .builtin_getattr(
                    vec![loader.clone(), Value::Str("exec_module".to_string())],
                    HashMap::new(),
                )
                .ok();

            let mut created_module: Option<Value> = None;
            if let Some(create_callable) = create_module {
                match self.call_internal(create_callable, vec![spec.clone()], HashMap::new()) {
                    Ok(super::InternalCallOutcome::Value(value)) => {
                        if !matches!(value, Value::None) {
                            created_module = Some(value);
                        }
                    }
                    Ok(super::InternalCallOutcome::CallerExceptionHandled) => {
                        if trace_meta_path {
                            eprintln!("[meta-path] create_module handled exception");
                        }
                        continue;
                    }
                    Err(err) => {
                        if trace_meta_path {
                            eprintln!("[meta-path] create_module error: {}", err.message);
                        }
                        continue;
                    }
                }
            }
            if created_module.is_none()
                && let Some(load_callable) = load_module
            {
                match self.call_internal(load_callable, vec![name_value.clone()], HashMap::new()) {
                    Ok(super::InternalCallOutcome::Value(value)) => {
                        if !matches!(value, Value::None) {
                            created_module = Some(value);
                        }
                    }
                    Ok(super::InternalCallOutcome::CallerExceptionHandled) => {
                        if trace_meta_path {
                            eprintln!("[meta-path] load_module handled exception");
                        }
                        continue;
                    }
                    Err(err) => {
                        if trace_meta_path {
                            eprintln!("[meta-path] load_module error: {}", err.message);
                        }
                        continue;
                    }
                }
            }
            if let (Some(exec_callable), Some(module_value)) = (exec_module, created_module.clone())
            {
                let _ = self.call_internal(exec_callable, vec![module_value], HashMap::new());
            }

            self.run_pending_import_frames(caller_depth)?;
            let Some(modules_dict) = self.sys_dict_obj("modules") else {
                continue;
            };
            let key = Value::Str(name.to_string());
            if let Some(Value::Module(module)) = dict_get_value(&modules_dict, &key) {
                if trace_meta_path {
                    eprintln!("[meta-path] sys.modules module hit name={}", name);
                }
                self.modules.insert(name.to_string(), module.clone());
                return Ok(Some(module));
            }
            if let Some(entry) = dict_get_value(&modules_dict, &key)
                && let Some(module) =
                    self.coerce_sys_modules_entry_to_module(name, &entry, &modules_dict)
            {
                if trace_meta_path {
                    eprintln!("[meta-path] sys.modules coerced module hit name={}", name);
                }
                self.modules.insert(name.to_string(), module.clone());
                return Ok(Some(module));
            }
            if let Some(Value::Module(module)) = created_module {
                if trace_meta_path {
                    eprintln!("[meta-path] created-module hit name={}", name);
                }
                dict_set_value(
                    &modules_dict,
                    Value::Str(name.to_string()),
                    Value::Module(module.clone()),
                );
                self.modules.insert(name.to_string(), module.clone());
                return Ok(Some(module));
            }
        }
        Ok(None)
    }

    pub(super) fn cleanup_partial_modules(&mut self, existing_modules: &HashSet<String>) {
        let added: Vec<String> = self
            .modules
            .keys()
            .filter(|name| !existing_modules.contains(*name))
            .cloned()
            .collect();
        for name in added {
            let should_remove = self
                .modules
                .get(&name)
                .map(Self::module_is_uninitialized)
                .unwrap_or(false);
            if !should_remove {
                continue;
            }
            self.remove_module_entry_and_parent_binding(&name);
        }
    }

    pub(super) fn module_is_uninitialized(module: &ObjRef) -> bool {
        if Self::module_is_initializing(module) {
            return true;
        }
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return false;
        };
        module_data.globals.keys().all(|key| {
            matches!(
                key.as_str(),
                "__name__"
                    | "__doc__"
                    | "__package__"
                    | "__loader__"
                    | "__spec__"
                    | "__file__"
                    | "__path__"
            )
        })
    }

    pub(super) fn mark_module_initializing(&self, module: &ObjRef) {
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert(PYRS_MODULE_INITIALIZING_FLAG.to_string(), Value::Bool(true));
        }
    }

    pub(super) fn clear_module_initializing(&self, module: &ObjRef) {
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data.globals.remove(PYRS_MODULE_INITIALIZING_FLAG);
        }
    }

    pub(super) fn module_is_initializing(module: &ObjRef) -> bool {
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return false;
        };
        matches!(
            module_data.globals.get(PYRS_MODULE_INITIALIZING_FLAG),
            Some(Value::Bool(true))
        )
    }

    pub(super) fn module_loader_name(module: &ObjRef) -> Option<String> {
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return None;
        };
        match module_data.globals.get("__loader__") {
            Some(Value::Str(name)) => Some(name.clone()),
            _ => None,
        }
    }

    pub(super) fn should_prefer_filesystem_module(&mut self, name: &str, module: &ObjRef) -> bool {
        let is_json_stack = matches!(
            name,
            "json" | "json.decoder" | "json.scanner" | "json.encoder" | "_json"
        );
        let is_pickle_stack = matches!(name, "pickle" | "pickletools" | "copyreg");
        let is_re_stack = matches!(
            name,
            "re" | "re._compiler" | "re._constants" | "re._parser" | "re._casefix"
        );
        let is_collections_stack = matches!(name, "collections.abc");
        let is_decimal_stack = name == "decimal";
        let is_functools_stack = name == "functools";
        let is_types_stack = matches!(name, "types" | "typing");
        let is_random_stack = name == "random";
        if !is_json_stack
            && !is_pickle_stack
            && !is_re_stack
            && !is_collections_stack
            && !is_decimal_stack
            && !is_functools_stack
            && !is_types_stack
            && !is_random_stack
        {
            return false;
        }
        if is_json_stack && !self.prefer_pure_json_when_available {
            return false;
        }
        if is_pickle_stack && !self.prefer_pure_pickle_when_available {
            return false;
        }
        if is_re_stack && !self.prefer_pure_re_when_available {
            return false;
        }
        if !self.has_preferred_filesystem_module(name) {
            return false;
        }
        if Self::module_loader_name(module).as_deref() == Some(BUILTIN_MODULE_LOADER) {
            return true;
        }
        if Self::module_is_local_shim(module) {
            return true;
        }
        false
    }

    pub(super) fn module_for_plain_import(&mut self, name: &str, module: ObjRef) -> ObjRef {
        if let Some((root, _)) = name.split_once('.') {
            self.link_module_chain(name, module);
            self.ensure_module(root)
        } else {
            module
        }
    }

    pub(super) fn canonical_imported_module_for_name(
        &mut self,
        name: &str,
        fallback: ObjRef,
    ) -> ObjRef {
        if !name.is_empty() {
            if let Some(modules_dict) = self.sys_dict_obj("modules") {
                let key = Value::Str(name.to_string());
                if let Some(Value::Module(module)) = dict_get_value(&modules_dict, &key) {
                    self.modules.insert(name.to_string(), module.clone());
                    self.apply_post_import_fixups(name, &module);
                    return module;
                }
            }
            if let Some(module) = self.modules.get(name).cloned() {
                self.apply_post_import_fixups(name, &module);
                return module;
            }
        }
        self.apply_post_import_fixups(name, &fallback);
        fallback
    }

    fn apply_post_import_fixups(&mut self, name: &str, module: &ObjRef) {
        if name == "_threading_local" {
            let local_class = match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("local").cloned(),
                _ => None,
            };
            if let Some(local_class) = local_class
                && let Some(thread_module) = self.modules.get("_thread").cloned()
                && let Object::Module(module_data) = &mut *thread_module.kind_mut()
            {
                module_data
                    .globals
                    .insert("_local".to_string(), local_class);
            }
        }
    }

    pub(super) fn fromlist_requested(&self, fromlist: &Value) -> bool {
        match fromlist {
            Value::None => false,
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => !values.is_empty(),
                _ => true,
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => !values.is_empty(),
                _ => true,
            },
            _ => true,
        }
    }

    pub(super) fn import_package_context(&self) -> Option<String> {
        let frame = self.frames.last()?;
        let module_ref = frame.module.kind();
        let module = match &*module_ref {
            Object::Module(module) => module,
            _ => return None,
        };
        if let Some(Value::Str(package)) = module.globals.get("__package__") {
            return Some(package.clone());
        }
        if module.globals.contains_key("__path__") {
            return Some(module.name.clone());
        }
        Some(
            module
                .name
                .rsplit_once('.')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default(),
        )
    }

    pub(super) fn resolve_import_name(
        &self,
        requested: &str,
        level: usize,
    ) -> Result<String, RuntimeError> {
        if level == 0 {
            return Ok(requested.to_string());
        }

        let package = self
            .import_package_context()
            .ok_or_else(|| RuntimeError::new("relative import outside module context"))?;
        if package.is_empty() {
            return Err(RuntimeError::new(
                "attempted relative import with no known parent package",
            ));
        }

        self.resolve_import_name_from_package(&package, requested, level)
    }

    pub(super) fn resolve_import_name_from_package(
        &self,
        package: &str,
        requested: &str,
        level: usize,
    ) -> Result<String, RuntimeError> {
        if level == 0 {
            return Ok(requested.to_string());
        }

        let mut parts: Vec<&str> = package.split('.').collect();
        let trim = level.saturating_sub(1);
        if trim > parts.len() {
            return Err(RuntimeError::new(
                "attempted relative import beyond top-level package",
            ));
        }
        parts.truncate(parts.len() - trim);

        let mut resolved = parts.join(".");
        if !requested.is_empty() {
            if !resolved.is_empty() {
                resolved.push('.');
            }
            resolved.push_str(requested);
        }
        Ok(resolved)
    }
}
