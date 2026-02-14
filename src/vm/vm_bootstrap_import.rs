use super::{
    AtomicOrdering, BUILTIN_MODULE_LOADER, BuiltinFunction, ClassObject, DEFAULT_META_PATH_FINDER,
    DEFAULT_PATH_HOOK, Frame, HashMap, HashSet, InstanceObject, LOCAL_SHIM_MODULES, ModuleObject,
    ModuleSourceInfo, NAMESPACE_LOADER, ObjRef, Object, PURE_STDLIB_JSON_MODULES,
    PURE_STDLIB_PATHLIB_MODULES, PURE_STDLIB_PICKLE_MODULES, PURE_STDLIB_RE_MODULES, PathBuf, Rc,
    RuntimeError, SIGNAL_DEFAULT, SIGNAL_IGNORE, SIGNAL_SIGINT, SIGNAL_SIGTERM, SOURCE_FILE_LOADER,
    SOURCELESS_FILE_LOADER, SUBMODULE_TRACE_COUNT, Value, Vm, cached_module_path, compiler,
    cpython, dict_get_value, dict_remove_value, dict_set_value, matches_finder_kind,
    parse_uuid_like_string, parser,
};

impl Vm {
    pub(super) fn set_module_class_bases(
        &mut self,
        module_name: &str,
        class_name: &str,
        base_names: &[&str],
    ) -> Result<(), RuntimeError> {
        let module = self
            .modules
            .get(module_name)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("module '{module_name}' not found")))?;
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
            vec![(
                "struct_time",
                self.heap
                    .alloc_class(ClassObject::new("struct_time".to_string(), Vec::new())),
            )],
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
        self.install_builtin_module(
            "os",
            &[
                ("getpid", BuiltinFunction::OsGetPid),
                ("getcwd", BuiltinFunction::OsGetCwd),
                ("getenv", BuiltinFunction::OsGetEnv),
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
                (
                    "stat_result",
                    self.heap
                        .alloc_class(ClassObject::new("stat_result".to_string(), Vec::new())),
                ),
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
                (
                    "terminal_size",
                    Value::Builtin(BuiltinFunction::OsTerminalSize),
                ),
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
        self.install_builtin_module(
            "posix",
            &[
                ("getpid", BuiltinFunction::OsGetPid),
                ("getcwd", BuiltinFunction::OsGetCwd),
                ("getenv", BuiltinFunction::OsGetEnv),
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
                (
                    "stat_result",
                    self.heap
                        .alloc_class(ClassObject::new("stat_result".to_string(), Vec::new())),
                ),
            ],
        );
        let build_time_vars = vec![
            (Value::Str("prefix".to_string()), Value::Str(String::new())),
            (
                Value::Str("exec_prefix".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("ABIFLAGS".to_string()),
                Value::Str(String::new()),
            ),
            (
                Value::Str("MULTIARCH".to_string()),
                Value::Str(String::new()),
            ),
            (Value::Str("Py_GIL_DISABLED".to_string()), Value::Int(0)),
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
        self.install_builtin_module(
            "pathlib",
            &[
                ("Path", BuiltinFunction::OsPathJoin),
                ("joinpath", BuiltinFunction::OsPathJoin),
                ("exists", BuiltinFunction::OsPathExists),
            ],
            Vec::new(),
        );
        self.install_builtin_module(
            "os.path",
            &[
                ("join", BuiltinFunction::OsPathJoin),
                ("exists", BuiltinFunction::OsPathExists),
                ("lexists", BuiltinFunction::OsPathExists),
                ("normpath", BuiltinFunction::OsPathNormPath),
                ("normcase", BuiltinFunction::OsPathNormCase),
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
        let md5_type = match self
            .heap
            .alloc_class(ClassObject::new("md5".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *md5_type.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("_md5".to_string()));
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
        self.install_builtin_module(
            "_md5",
            &[("md5", BuiltinFunction::HashlibMd5)],
            vec![("MD5Type", Value::Class(md5_type))],
        );

        let build_sha2_type = |name: &str| {
            let class = match self
                .heap
                .alloc_class(ClassObject::new(name.to_string(), Vec::new()))
            {
                Value::Class(class) => class,
                _ => unreachable!(),
            };
            if let Object::Class(class_data) = &mut *class.kind_mut() {
                class_data
                    .attrs
                    .insert("__module__".to_string(), Value::Str("_sha2".to_string()));
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
        let sha224_type = build_sha2_type("SHA224Type");
        let sha256_type = build_sha2_type("SHA256Type");
        let sha384_type = build_sha2_type("SHA384Type");
        let sha512_type = build_sha2_type("SHA512Type");
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
            ],
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
                Some(BUILTIN_MODULE_LOADER),
                false,
                Vec::new(),
                false,
            );
            self.set_module_metadata(
                &scanner_module,
                "json.scanner",
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
                ("lookup", BuiltinFunction::CodecsLookup),
                ("register", BuiltinFunction::CodecsRegister),
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
                ("CodecInfo", codec_info_class),
                ("IncrementalDecoder", incremental_decoder_class),
                ("IncrementalEncoder", incremental_encoder_class),
                ("StreamReader", stream_reader_class),
                ("StreamWriter", stream_writer_class),
            ],
        );
        self.install_builtin_module(
            "unicodedata",
            &[("normalize", BuiltinFunction::UnicodedataNormalize)],
            Vec::new(),
        );
        self.install_builtin_module(
            "binascii",
            &[
                ("crc32", BuiltinFunction::BinasciiCrc32),
                ("b2a_base64", BuiltinFunction::BinasciiB2aBase64),
                ("a2b_base64", BuiltinFunction::BinasciiA2bBase64),
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
        let typing_placeholder = self.heap.alloc_class(ClassObject::new(
            "typing.placeholder".to_string(),
            Vec::new(),
        ));
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
        let user_list_class = match self
            .heap
            .alloc_class(ClassObject::new("UserList".to_string(), Vec::new()))
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
        self.install_builtin_module(
            "types",
            &[
                ("ModuleType", BuiltinFunction::TypesModuleType),
                ("MappingProxyType", BuiltinFunction::TypesMappingProxy),
                ("MethodType", BuiltinFunction::TypesMethodType),
                ("new_class", BuiltinFunction::TypesNewClass),
                ("coroutine", BuiltinFunction::TypingIdFunc),
            ],
            vec![
                (
                    "DynamicClassAttribute",
                    Value::Builtin(BuiltinFunction::Property),
                ),
                (
                    "FunctionType",
                    self.heap
                        .alloc_class(ClassObject::new("function".to_string(), Vec::new())),
                ),
                (
                    "BuiltinFunctionType",
                    self.heap.alloc_class(ClassObject::new(
                        "builtin_function_or_method".to_string(),
                        Vec::new(),
                    )),
                ),
                (
                    "CodeType",
                    self.heap
                        .alloc_class(ClassObject::new("code".to_string(), Vec::new())),
                ),
                (
                    "NoneType",
                    self.heap
                        .alloc_class(ClassObject::new("NoneType".to_string(), Vec::new())),
                ),
                (
                    "EllipsisType",
                    self.heap
                        .alloc_class(ClassObject::new("ellipsis".to_string(), Vec::new())),
                ),
                (
                    "NotImplementedType",
                    self.heap.alloc_class(ClassObject::new(
                        "NotImplementedType".to_string(),
                        Vec::new(),
                    )),
                ),
                ("SimpleNamespace", simple_namespace_class),
                (
                    "GenericAlias",
                    self.heap
                        .alloc_class(ClassObject::new("GenericAlias".to_string(), Vec::new())),
                ),
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
            ],
        );
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
                (
                    "ParamSpecArgs",
                    self.heap
                        .alloc_class(ClassObject::new("ParamSpecArgs".to_string(), Vec::new())),
                ),
                (
                    "ParamSpecKwargs",
                    self.heap
                        .alloc_class(ClassObject::new("ParamSpecKwargs".to_string(), Vec::new())),
                ),
                (
                    "Generic",
                    self.heap
                        .alloc_class(ClassObject::new("Generic".to_string(), Vec::new())),
                ),
                (
                    "Union",
                    self.heap
                        .alloc_class(ClassObject::new("Union".to_string(), Vec::new())),
                ),
                (
                    "NoDefault",
                    self.heap
                        .alloc_class(ClassObject::new("NoDefault".to_string(), Vec::new())),
                ),
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
                    "Constant",
                    self.heap
                        .alloc_class(ClassObject::new("Constant".to_string(), Vec::new())),
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
            "array",
            &[("array", BuiltinFunction::ArrayArray)],
            vec![("typecodes", Value::Str("bBuhHiIlLqQfdw".to_string()))],
        );
        let errno_constants = vec![
            ("EPERM", 1),
            ("ENOENT", 2),
            ("EINTR", 4),
            ("EBADF", 9),
            ("ECHILD", 10),
            ("EAGAIN", 11),
            ("EACCES", 13),
            ("EEXIST", 17),
            ("ENOTDIR", 20),
            ("EISDIR", 21),
            ("EINVAL", 22),
            ("ENOSYS", 38),
        ];
        let mut errno_values = Vec::new();
        let mut errorcode_entries = Vec::new();
        for (name, value) in &errno_constants {
            errno_values.push((*name, Value::Int(*value)));
            errorcode_entries.push((Value::Int(*value), Value::Str((*name).to_string())));
        }
        errno_values.push(("errorcode", self.heap.alloc_dict(errorcode_entries)));
        self.install_builtin_module("errno", &[], errno_values);
        let abc_base = self
            .heap
            .alloc_class(ClassObject::new("ABC".to_string(), Vec::new()));
        self.install_builtin_module(
            "abc",
            &[
                ("abstractmethod", BuiltinFunction::AbcAbstractMethod),
                (
                    "update_abstractmethods",
                    BuiltinFunction::AbcUpdateAbstractMethods,
                ),
                ("get_cache_token", BuiltinFunction::AbcGetCacheToken),
            ],
            vec![
                ("ABCMeta", Value::Builtin(BuiltinFunction::Type)),
                ("ABC", abc_base),
                (
                    "abstractclassmethod",
                    Value::Builtin(BuiltinFunction::ClassMethod),
                ),
                (
                    "abstractstaticmethod",
                    Value::Builtin(BuiltinFunction::StaticMethod),
                ),
                (
                    "abstractproperty",
                    Value::Builtin(BuiltinFunction::Property),
                ),
            ],
        );
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
        if let Object::Class(class_data) = &mut *inspect_signature_class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("inspect".to_string()));
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureStr),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::InspectSignatureRepr),
            );
        }
        self.install_builtin_module(
            "inspect",
            &[
                ("signature", BuiltinFunction::InspectSignature),
                ("getmodule", BuiltinFunction::InspectGetModule),
                ("getfile", BuiltinFunction::InspectGetFile),
                ("getsourcefile", BuiltinFunction::InspectGetSourceFile),
                ("isfunction", BuiltinFunction::InspectIsFunction),
                ("ismethod", BuiltinFunction::InspectIsMethod),
                ("isroutine", BuiltinFunction::InspectIsRoutine),
                (
                    "ismethoddescriptor",
                    BuiltinFunction::InspectIsMethodDescriptor,
                ),
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
                ("_sentinel", inspect_sentinel),
                ("Signature", Value::Class(inspect_signature_class)),
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
                (
                    "IncrementalNewlineDecoder",
                    self.heap.alloc_class(ClassObject::new(
                        "IncrementalNewlineDecoder".to_string(),
                        Vec::new(),
                    )),
                ),
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
                (
                    "IncrementalNewlineDecoder",
                    self.heap.alloc_class(ClassObject::new(
                        "IncrementalNewlineDecoder".to_string(),
                        Vec::new(),
                    )),
                ),
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
                "astimezone".to_string(),
                Value::Builtin(BuiltinFunction::DateTimeAstimezone),
            );
            class_data.attrs.insert(
                "strftime".to_string(),
                Value::Builtin(BuiltinFunction::DateStrFTime),
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
                "strftime".to_string(),
                Value::Builtin(BuiltinFunction::DateStrFTime),
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
        }
        let timezone_class = match self
            .heap
            .alloc_class(ClassObject::new("timezone".to_string(), Vec::new()))
        {
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
                ("timezone", Value::Class(timezone_class)),
                ("UTC", Value::Instance(timezone_utc)),
            ],
        );
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
        let sys_module = match self.modules.get("sys").cloned() {
            Some(module) => module,
            None => return,
        };
        let path_value = match &*sys_module.kind() {
            Object::Module(module_data) => module_data.globals.get("path").cloned(),
            _ => None,
        };
        let Some(Value::List(path_list)) = path_value else {
            return;
        };

        let mut new_paths = Vec::new();
        if let Object::List(values) = &*path_list.kind() {
            for value in values {
                if let Value::Str(path) = value {
                    new_paths.push(PathBuf::from(path));
                }
            }
        }
        self.module_paths = new_paths;
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
        if let Some(Value::Dict(existing)) = existing_modules
            && let Object::Dict(existing_entries) = &*existing.kind()
        {
            for (key, value) in existing_entries.iter() {
                let Value::Str(name) = key else {
                    continue;
                };
                let preserve = match value {
                    // Preserve explicit import blockers and user overrides.
                    Value::None => true,
                    // Preserve sys.modules module entries unknown to `self.modules`.
                    Value::Module(_) => !self.modules.contains_key(name),
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
        let modules_dict = self.heap.alloc_dict(entries);
        if let Object::Module(module_data) = &mut *sys_module.kind_mut() {
            module_data
                .globals
                .insert("modules".to_string(), modules_dict);
        }
    }

    pub(super) fn unregister_module(&mut self, name: &str) {
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
        let rel = module_name.replace('.', "/");
        self.module_paths.iter().any(|root| {
            root.join(format!("{rel}.py")).is_file()
                || root.join(&rel).join("__init__.py").is_file()
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

    pub(super) fn has_preferred_filesystem_module(&self, module_name: &str) -> bool {
        self.has_cpython_pure_module_on_module_path(module_name)
            || self.has_local_shim_module(module_name)
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
        self.modules.insert(name.to_string(), module);
        self.refresh_sys_modules_dict();
    }

    pub(super) fn load_module(&mut self, name: &str) -> Result<ObjRef, RuntimeError> {
        if std::env::var_os("PYRS_TRACE_MODULE_LOAD").is_some() {
            eprintln!("[module-load] {name}");
        }
        if let Some(module) = self.modules.get(name).cloned() {
            return Ok(module);
        }

        if let Some((parent, _)) = name.rsplit_once('.')
            && !self.modules.contains_key(parent)
        {
            let parent_caller_depth = self.frames.len();
            let _ = self.load_module(parent)?;
            self.run_pending_import_frames(parent_caller_depth)?;
        }

        let source_info = self
            .find_module_source(name)
            .ok_or_else(|| RuntimeError::new(format!("module '{name}' not found")))?;
        let loader_name = if source_info.is_namespace {
            NAMESPACE_LOADER
        } else if source_info.is_bytecode {
            SOURCELESS_FILE_LOADER
        } else {
            SOURCE_FILE_LOADER
        };

        let module = self.create_module_for_loader(name, loader_name)?;
        let origin = if source_info.is_namespace {
            None
        } else {
            Some(&source_info.path)
        };
        self.set_module_metadata(
            &module,
            name,
            origin,
            Some(loader_name),
            source_info.is_package,
            source_info.package_dirs.clone(),
            source_info.is_namespace,
        );

        self.register_module(name, module.clone());
        self.link_module_chain(name, module.clone());
        self.exec_module_for_loader(&module, name, loader_name, &source_info)?;
        Ok(module)
    }

    pub(super) fn create_module_for_loader(
        &mut self,
        name: &str,
        loader_name: &str,
    ) -> Result<ObjRef, RuntimeError> {
        match loader_name {
            SOURCE_FILE_LOADER | SOURCELESS_FILE_LOADER | NAMESPACE_LOADER => {
                match self.heap.alloc_module(ModuleObject::new(name)) {
                    Value::Module(obj) => Ok(obj),
                    _ => unreachable!(),
                }
            }
            _ => Err(RuntimeError::new(format!(
                "unsupported loader for module creation: {loader_name}"
            ))),
        }
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
                let source = std::fs::read_to_string(&source_info.path).map_err(|err| {
                    RuntimeError::new(format!("failed to read module '{name}': {err}"))
                })?;

                let module_ast = parser::parse_module(&source).map_err(|err| {
                    RuntimeError::new(format!(
                        "parse error in module '{name}' at {}: {}",
                        err.offset, err.message
                    ))
                })?;
                let code = compiler::compile_module_with_filename(
                    &module_ast,
                    &source_info.path.to_string_lossy(),
                )
                .map_err(|err| {
                    RuntimeError::new(format!("compile error in module '{name}': {}", err.message))
                })?;
                let code = Rc::new(code);
                let cells = self.build_cells(&code, Vec::new());
                let mut frame = Frame::new(code, module.clone(), true, false, cells, None);
                frame.discard_result = true;
                self.frames.push(Box::new(frame));
                Ok(())
            }
            SOURCELESS_FILE_LOADER => {
                let bytes = std::fs::read(&source_info.path).map_err(|err| {
                    RuntimeError::new(format!("failed to read module '{name}': {err}"))
                })?;
                let pyc = cpython::load_pyc(&bytes)
                    .map_err(|err| RuntimeError::new(format!("pyc load error: {}", err.message)))?;
                let code = cpython::translate_code(&pyc, &mut self.heap).map_err(|err| {
                    RuntimeError::new(format!("pyc translate error: {}", err.message))
                })?;
                let code = Rc::new(code);
                let cells = self.build_cells(&code, Vec::new());
                let mut frame = Frame::new(code, module.clone(), true, false, cells, None);
                frame.discard_result = true;
                self.frames.push(Box::new(frame));
                Ok(())
            }
            _ => Err(RuntimeError::new(format!(
                "unsupported loader for module execution: {loader_name}"
            ))),
        }
    }

    pub(super) fn find_module_source(&mut self, name: &str) -> Option<ModuleSourceInfo> {
        self.sync_module_paths_from_sys();
        let meta_path = self.sys_list_values("meta_path").unwrap_or_default();
        for finder in &meta_path {
            if let Some(source) = self.find_module_source_with_meta_finder(name, finder) {
                return Some(source);
            }
        }
        None
    }

    pub(super) fn find_module_source_with_meta_finder(
        &mut self,
        name: &str,
        finder: &Value,
    ) -> Option<ModuleSourceInfo> {
        if matches_finder_kind(finder, DEFAULT_META_PATH_FINDER) {
            return self.path_finder_find_spec(name);
        }
        None
    }

    pub(super) fn path_finder_find_spec(&mut self, name: &str) -> Option<ModuleSourceInfo> {
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
        let hooks = self.sys_list_values("path_hooks").unwrap_or_default();
        for hook in hooks {
            if matches_finder_kind(&hook, DEFAULT_PATH_HOOK) {
                return Some(self.make_file_finder_importer(root));
            }
        }
        None
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
        &self,
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

    pub(super) fn find_module_source_in_single_root(
        &self,
        module_name: &str,
        root: &std::path::Path,
    ) -> Option<ModuleSourceInfo> {
        let rel_name = module_name.replace('.', "/");
        let candidate = root.join(format!("{rel_name}.py"));
        if candidate.exists() {
            return Some(ModuleSourceInfo {
                path: candidate,
                is_package: false,
                package_dirs: Vec::new(),
                is_namespace: false,
                is_bytecode: false,
            });
        }
        let pyc_candidate = cached_module_path(root, &rel_name);
        if pyc_candidate.exists() {
            return Some(ModuleSourceInfo {
                path: pyc_candidate,
                is_package: false,
                package_dirs: Vec::new(),
                is_namespace: false,
                is_bytecode: true,
            });
        }
        let direct_pyc = root.join(format!("{rel_name}.pyc"));
        if direct_pyc.exists() {
            return Some(ModuleSourceInfo {
                path: direct_pyc,
                is_package: false,
                package_dirs: Vec::new(),
                is_namespace: false,
                is_bytecode: true,
            });
        }
        let package_dir = root.join(&rel_name);
        let package_init = package_dir.join("__init__.py");
        if package_init.exists() {
            return Some(ModuleSourceInfo {
                path: package_init,
                is_package: true,
                package_dirs: vec![package_dir],
                is_namespace: false,
                is_bytecode: false,
            });
        }
        let package_init_pyc = package_dir
            .join("__pycache__")
            .join("__init__.cpython-314.pyc");
        if package_init_pyc.exists() {
            return Some(ModuleSourceInfo {
                path: package_init_pyc,
                is_package: true,
                package_dirs: vec![package_dir],
                is_namespace: false,
                is_bytecode: true,
            });
        }
        let direct_package_init_pyc = package_dir.join("__init__.pyc");
        if direct_package_init_pyc.exists() {
            return Some(ModuleSourceInfo {
                path: direct_package_init_pyc,
                is_package: true,
                package_dirs: vec![package_dir],
                is_namespace: false,
                is_bytecode: true,
            });
        }
        if package_dir.is_dir() {
            return Some(ModuleSourceInfo {
                path: package_dir.clone(),
                is_package: true,
                package_dirs: vec![package_dir],
                is_namespace: true,
                is_bytecode: false,
            });
        }
        None
    }

    pub(super) fn sys_list_values(&self, name: &str) -> Option<Vec<Value>> {
        let sys_module = self.modules.get("sys")?.clone();
        let module_kind = sys_module.kind();
        let module_data = match &*module_kind {
            Object::Module(module_data) => module_data,
            _ => return None,
        };
        let list_obj = match module_data.globals.get(name) {
            Some(Value::List(list)) => list.clone(),
            _ => return None,
        };
        match &*list_obj.kind() {
            Object::List(values) => Some(values.clone()),
            _ => None,
        }
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

    pub(super) fn load_submodule(&mut self, parent: &ObjRef, attr_name: &str) -> Option<ObjRef> {
        let parent_name = match &*parent.kind() {
            Object::Module(module) => module.name.clone(),
            _ => return None,
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
                    return Some(module);
                }
                Some(Value::None) => {
                    self.modules.remove(&full_name);
                    return None;
                }
                Some(_) => {
                    self.modules.remove(&full_name);
                    return None;
                }
                None => {
                    missing_from_sys_modules = true;
                }
            }
        }
        if missing_from_sys_modules {
            self.modules.remove(&full_name);
        } else if let Some(module) = self.modules.get(&full_name).cloned() {
            return Some(module);
        }
        if self.find_module_file(&full_name).is_some()
            && let Ok(module) = self.import_module_object(&full_name)
        {
            self.upsert_module_global(parent, attr_name, Value::Module(module.clone()));
            return Some(module);
        }
        None
    }

    pub(super) fn ensure_module(&mut self, name: &str) -> ObjRef {
        if let Some(module) = self.modules.get(name).cloned() {
            return module;
        }
        let module = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(&module, name, None, None, false, Vec::new(), false);
        self.register_module(name, module.clone());
        module
    }

    pub(super) fn set_module_metadata(
        &mut self,
        module: &ObjRef,
        name: &str,
        origin: Option<&PathBuf>,
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

    pub(super) fn build_module_spec_value(
        &mut self,
        name: &str,
        origin: Option<&PathBuf>,
        loader_name: Option<&str>,
        is_package: bool,
        package_dirs: &[PathBuf],
        is_namespace: bool,
    ) -> Value {
        let parent = name
            .rsplit_once('.')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
        let loader_value = loader_name
            .map(|loader| Value::Str(loader.to_string()))
            .unwrap_or(Value::None);
        let origin_value = origin
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

        let spec = match self
            .heap
            .alloc_module(ModuleObject::new("__module_spec__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *spec.kind_mut() {
            module_data
                .globals
                .insert("name".to_string(), Value::Str(name.to_string()));
            module_data
                .globals
                .insert("origin".to_string(), origin_value);
            module_data
                .globals
                .insert("loader".to_string(), loader_value);
            module_data
                .globals
                .insert("parent".to_string(), Value::Str(parent));
            module_data.globals.insert(
                "submodule_search_locations".to_string(),
                submodule_locations,
            );
            module_data
                .globals
                .insert("is_package".to_string(), Value::Bool(is_package));
            module_data
                .globals
                .insert("is_namespace".to_string(), Value::Bool(is_namespace));
            module_data
                .globals
                .insert("has_location".to_string(), Value::Bool(origin.is_some()));
            module_data
                .globals
                .insert("cached".to_string(), Value::None);
        }
        Value::Module(spec)
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
                    return Err(RuntimeError::new(format!("No module named '{}'", name)));
                }
                Some(_) => {
                    present_in_sys_modules = true;
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
            if !keep_cached_builtin {
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
        match self.load_module(name) {
            Ok(module) => self.return_imported_module(module, caller_depth),
            Err(load_err) => {
                if let Some((parent, _)) = name.rsplit_once('.') {
                    let _ = self.import_module_object(parent)?;
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
                                return Err(RuntimeError::new(format!(
                                    "No module named '{}'",
                                    name
                                )));
                            }
                            _ => {}
                        }
                    }
                }
                self.cleanup_partial_modules(&existing_modules);
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
            self.modules.remove(&name);
            if let Some(modules_dict) = self.sys_dict_obj("modules") {
                let _ = dict_remove_value(&modules_dict, &Value::Str(name.clone()));
            }
            if let Some((parent, child)) = name.rsplit_once('.')
                && let Some(parent_module) = self.modules.get(parent)
                && let Object::Module(parent_data) = &mut *parent_module.kind_mut()
            {
                parent_data.globals.remove(child);
            }
        }
    }

    pub(super) fn module_is_uninitialized(module: &ObjRef) -> bool {
        let module_kind = module.kind();
        let Object::Module(module_data) = &*module_kind else {
            return false;
        };
        if module_data.globals.contains_key("__builtins__") {
            return false;
        }
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
        let is_decimal_stack = name == "decimal";
        if !is_json_stack && !is_pickle_stack && !is_re_stack && !is_decimal_stack {
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
                    return module;
                }
            }
            if let Some(module) = self.modules.get(name).cloned() {
                return module;
            }
        }
        fallback
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
