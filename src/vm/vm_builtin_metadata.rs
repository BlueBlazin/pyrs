use std::cell::Cell;

use super::{
    AttrAccessOutcome, AttrMutationOutcome, BYTES_BACKING_STORAGE_ATTR, Block, BoundMethod,
    BuiltinFunction, COMPLEX_BACKING_STORAGE_ATTR, CodeObject, DICT_BACKING_STORAGE_ATTR,
    ExceptionObject, FLOAT_BACKING_STORAGE_ATTR, FROZENSET_BACKING_STORAGE_ATTR, Frame, HashMap,
    INSTANCE_DICT_STORAGE_ATTR, INT_BACKING_STORAGE_ATTR, InstanceObject, InternalCallOutcome,
    IteratorKind, LIST_BACKING_STORAGE_ATTR, MAPPING_PROXY_STORAGE_ATTR, ModuleObject,
    NativeCallResult, NativeMethodKind, ObjRef, Object, Opcode, PY_TPFLAGS_HEAPTYPE, Rc,
    RuntimeError, SET_BACKING_STORAGE_ATTR, STR_BACKING_STORAGE_ATTR, TUPLE_BACKING_STORAGE_ATTR,
    Value, Vm, apply_bindings, bind_arguments, bytes_like_source_is_readonly, class_attr_lookup,
    class_attr_lookup_direct, class_attr_walk, class_inherits_dynamic_instance_dict,
    class_name_for_instance, collect_slot_names, dict_get_value, dict_remove_value, dict_set_value,
    format_repr, memoryview_bounds, runtime_error_matches_exception, value_from_bigint,
    value_from_object_ref, with_bytes_like_source,
};

thread_local! {
    static CALL_INTERNAL_DEPTH: Cell<usize> = const { Cell::new(0) };
    static LOAD_ATTR_SUPER_DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct CallInternalDepthGuard;

impl CallInternalDepthGuard {
    fn enter() -> (Self, usize) {
        let depth = CALL_INTERNAL_DEPTH.with(|counter| {
            let depth = counter.get().saturating_add(1);
            counter.set(depth);
            depth
        });
        (Self, depth)
    }
}

impl Drop for CallInternalDepthGuard {
    fn drop(&mut self) {
        CALL_INTERNAL_DEPTH.with(|counter| {
            let next = counter.get().saturating_sub(1);
            counter.set(next);
        });
    }
}

struct LoadAttrSuperDepthGuard;

impl LoadAttrSuperDepthGuard {
    fn enter() -> Option<(Self, usize)> {
        let trace_enabled = std::env::var_os("PYRS_TRACE_LOAD_ATTR_SUPER").is_some();
        let limit = std::env::var("PYRS_DEBUG_LOAD_ATTR_SUPER_DEPTH_LIMIT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|limit| *limit > 0)?;
        let depth = LOAD_ATTR_SUPER_DEPTH.with(|counter| {
            let depth = counter.get().saturating_add(1);
            counter.set(depth);
            depth
        });
        if trace_enabled {
            eprintln!("[load-attr-super-depth] depth={} limit={}", depth, limit);
        }
        if depth > limit {
            panic!(
                "load_attr_super recursion depth exceeded: depth={} limit={}",
                depth, limit
            );
        }
        Some((Self, depth))
    }
}

impl Drop for LoadAttrSuperDepthGuard {
    fn drop(&mut self) {
        LOAD_ATTR_SUPER_DEPTH.with(|counter| {
            let next = counter.get().saturating_sub(1);
            counter.set(next);
        });
    }
}

fn memoryview_shape_and_strides(
    view: &crate::runtime::MemoryViewObject,
    byte_len: usize,
) -> (Vec<isize>, Vec<isize>) {
    if let (Some(shape), Some(strides)) = (&view.shape, &view.strides)
        && shape.len() == strides.len()
        && !shape.is_empty()
    {
        return (shape.clone(), strides.clone());
    }
    let itemsize = view.itemsize.max(1);
    let logical_len = if byte_len % itemsize == 0 {
        byte_len / itemsize
    } else {
        byte_len
    };
    (vec![logical_len as isize], vec![itemsize as isize])
}

fn memoryview_contiguity(
    shape: &[isize],
    strides: &[isize],
    itemsize: isize,
) -> (bool, bool, bool) {
    if shape.len() != strides.len() {
        return (false, false, false);
    }
    if shape.iter().any(|dim| *dim < 0) {
        return (false, false, false);
    }
    if shape.iter().any(|dim| *dim == 0) {
        return (true, true, true);
    }
    let mut c_expected = itemsize;
    let mut c_contiguous = true;
    for index in (0..shape.len()).rev() {
        if shape[index] <= 0 {
            return (false, false, false);
        }
        if shape[index] != 1 {
            if strides[index] != c_expected {
                c_contiguous = false;
                break;
            }
            let Some(next_expected) = c_expected.checked_mul(shape[index]) else {
                c_contiguous = false;
                break;
            };
            c_expected = next_expected;
        }
    }
    let mut f_expected = itemsize;
    let mut f_contiguous = true;
    for index in 0..shape.len() {
        if shape[index] <= 0 {
            return (false, false, false);
        }
        if shape[index] != 1 {
            if strides[index] != f_expected {
                f_contiguous = false;
                break;
            }
            let Some(next_expected) = f_expected.checked_mul(shape[index]) else {
                f_contiguous = false;
                break;
            };
            f_expected = next_expected;
        }
    }
    let contiguous = c_contiguous || f_contiguous;
    (contiguous, c_contiguous, f_contiguous)
}

fn function_docstring_from_code(code: &CodeObject) -> Option<Value> {
    if let Some(Value::Str(doc)) = code.constants.first() {
        return Some(Value::Str(doc.clone()));
    }
    let mut index = 0usize;
    while index < code.instructions.len() {
        match code.instructions[index].opcode {
            Opcode::Nop | Opcode::MakeCell => {
                index += 1;
            }
            Opcode::LoadConst => {
                let Some(const_index) = code.instructions[index].arg.map(|idx| idx as usize) else {
                    return None;
                };
                let Some(Value::Str(doc)) = code.constants.get(const_index) else {
                    return None;
                };
                let next_index = index + 1;
                if next_index < code.instructions.len()
                    && matches!(code.instructions[next_index].opcode, Opcode::PopTop)
                {
                    return Some(Value::Str(doc.clone()));
                }
                return None;
            }
            _ => return None,
        }
    }
    None
}

impl Vm {
    fn builtin_type_has_none_hash(&self, builtin: BuiltinFunction) -> bool {
        matches!(
            builtin,
            BuiltinFunction::List
                | BuiltinFunction::Dict
                | BuiltinFunction::CollectionsDefaultDict
                | BuiltinFunction::CollectionsOrderedDict
                | BuiltinFunction::Set
                | BuiltinFunction::ByteArray
        )
    }

    fn builtin_module_binding(&self, builtin: BuiltinFunction) -> Option<(String, String)> {
        for (module_name, module) in &self.modules {
            let Object::Module(module_data) = &*module.kind() else {
                continue;
            };
            for (name, value) in &module_data.globals {
                if matches!(value, Value::Builtin(candidate) if *candidate == builtin) {
                    return Some((module_name.clone(), name.clone()));
                }
            }
        }
        None
    }

    pub(super) fn builtin_type_name(&self, builtin: BuiltinFunction) -> &'static str {
        match builtin {
            BuiltinFunction::Type => "type",
            BuiltinFunction::Ascii => "ascii",
            BuiltinFunction::Bool => "bool",
            BuiltinFunction::Int => "int",
            BuiltinFunction::Float => "float",
            BuiltinFunction::Str => "str",
            BuiltinFunction::List => "list",
            BuiltinFunction::Tuple => "tuple",
            BuiltinFunction::Dict => "dict",
            BuiltinFunction::CollectionsOrderedDict => "OrderedDict",
            BuiltinFunction::Set => "set",
            BuiltinFunction::FrozenSet => "frozenset",
            BuiltinFunction::Bytes => "bytes",
            BuiltinFunction::ByteArray => "bytearray",
            BuiltinFunction::MemoryView => "memoryview",
            BuiltinFunction::Complex => "complex",
            BuiltinFunction::Slice => "slice",
            BuiltinFunction::GeneratorType => "generator",
            BuiltinFunction::CoroutineType => "coroutine",
            BuiltinFunction::AsyncGeneratorType => "async_generator",
            BuiltinFunction::ClassMethod => "classmethod",
            BuiltinFunction::StaticMethod => "staticmethod",
            BuiltinFunction::Property => "property",
            BuiltinFunction::FunctoolsCachedProperty => "cached_property",
            BuiltinFunction::CodecsEncode => "encode",
            BuiltinFunction::CodecsDecode => "decode",
            BuiltinFunction::CodecsEscapeDecode => "escape_decode",
            BuiltinFunction::CodecsMakeIdentityDict => "make_identity_dict",
            BuiltinFunction::CodecsLookup => "lookup",
            BuiltinFunction::CodecsRegister => "register",
            BuiltinFunction::CodecsCodecInfoInit => "__init__",
            BuiltinFunction::CodecsGetIncrementalEncoder => "getincrementalencoder",
            BuiltinFunction::CodecsGetIncrementalDecoder => "getincrementaldecoder",
            BuiltinFunction::CodecsIncrementalEncoderInit => "__init__",
            BuiltinFunction::CodecsIncrementalEncoderEncode => "encode",
            BuiltinFunction::CodecsIncrementalEncoderReset => "reset",
            BuiltinFunction::CodecsIncrementalEncoderGetState => "getstate",
            BuiltinFunction::CodecsIncrementalEncoderSetState => "setstate",
            BuiltinFunction::CodecsIncrementalDecoderInit => "__init__",
            BuiltinFunction::CodecsIncrementalDecoderDecode => "decode",
            BuiltinFunction::CodecsIncrementalDecoderReset => "reset",
            BuiltinFunction::CodecsIncrementalDecoderGetState => "getstate",
            BuiltinFunction::CodecsIncrementalDecoderSetState => "setstate",
            BuiltinFunction::CollectionsDefaultDict => "defaultdict",
            _ => "builtin",
        }
    }

    pub(super) fn builtin_runtime_name(&self, builtin: BuiltinFunction) -> String {
        for (name, value) in &self.builtins {
            if matches!(value, Value::Builtin(candidate) if *candidate == builtin) {
                return name.clone();
            }
        }
        if let Some((_module_name, name)) = self.builtin_module_binding(builtin) {
            return name;
        }
        self.builtin_type_name(builtin).to_string()
    }

    pub(super) fn builtin_attribute_name(&self, builtin: BuiltinFunction) -> String {
        match builtin {
            BuiltinFunction::DictFromKeys => "fromkeys".to_string(),
            BuiltinFunction::SetReduce => "__reduce__".to_string(),
            BuiltinFunction::BytesMakeTrans | BuiltinFunction::StrMakeTrans => {
                "maketrans".to_string()
            }
            BuiltinFunction::JsonEncodeBaseString => "encode_basestring".to_string(),
            BuiltinFunction::JsonEncodeBaseStringAscii => "encode_basestring_ascii".to_string(),
            BuiltinFunction::JsonMakeEncoder => "make_encoder".to_string(),
            BuiltinFunction::JsonMakeEncoderCall => "_iterencode".to_string(),
            BuiltinFunction::JsonScannerMakeScanner => "make_scanner".to_string(),
            BuiltinFunction::JsonScannerPyMakeScanner => "py_make_scanner".to_string(),
            BuiltinFunction::JsonScannerScanOnce => "scan_once".to_string(),
            BuiltinFunction::JsonDecoderScanString => "scanstring".to_string(),
            BuiltinFunction::SqliteConnect => "connect".to_string(),
            BuiltinFunction::SqliteCompleteStatement => "complete_statement".to_string(),
            BuiltinFunction::SqliteRegisterAdapter => "register_adapter".to_string(),
            BuiltinFunction::SqliteRegisterConverter => "register_converter".to_string(),
            BuiltinFunction::SqliteEnableCallbackTracebacks => {
                "enable_callback_tracebacks".to_string()
            }
            BuiltinFunction::ObjectFormat => "__format__".to_string(),
            BuiltinFunction::ObjectInitSubclass => "__init_subclass__".to_string(),
            BuiltinFunction::SysAudit => "audit".to_string(),
            BuiltinFunction::SysAddAuditHook => "addaudithook".to_string(),
            BuiltinFunction::SysClearTypeDescriptors => "_clear_type_descriptors".to_string(),
            BuiltinFunction::SysCallTracing => "call_tracing".to_string(),
            BuiltinFunction::SysDisplayHook => "displayhook".to_string(),
            BuiltinFunction::SysCurrentFrames => "_current_frames".to_string(),
            BuiltinFunction::SysGetFrameModuleName => "_getframemodulename".to_string(),
            BuiltinFunction::SysUnraisableHook => "unraisablehook".to_string(),
            BuiltinFunction::SysBreakpointHook => "breakpointhook".to_string(),
            BuiltinFunction::SysIntern => "intern".to_string(),
            BuiltinFunction::SysIsGilEnabled => "_is_gil_enabled".to_string(),
            BuiltinFunction::SysMonitoringGetTool => "get_tool".to_string(),
            BuiltinFunction::SysMonitoringUseToolId => "use_tool_id".to_string(),
            BuiltinFunction::SysMonitoringClearToolId => "clear_tool_id".to_string(),
            BuiltinFunction::SysMonitoringFreeToolId => "free_tool_id".to_string(),
            BuiltinFunction::SysMonitoringRegisterCallback => "register_callback".to_string(),
            BuiltinFunction::SysMonitoringGetEvents => "get_events".to_string(),
            BuiltinFunction::SysMonitoringSetEvents => "set_events".to_string(),
            BuiltinFunction::SysMonitoringGetLocalEvents => "get_local_events".to_string(),
            BuiltinFunction::SysMonitoringSetLocalEvents => "set_local_events".to_string(),
            BuiltinFunction::SysMonitoringRestartEvents => "restart_events".to_string(),
            BuiltinFunction::SqliteConnectionInit => "__init__".to_string(),
            BuiltinFunction::SqliteConnectionDel => "__del__".to_string(),
            BuiltinFunction::SqliteConnectionGetAttribute => "__getattribute__".to_string(),
            BuiltinFunction::SqliteConnectionSetAttribute => "__setattr__".to_string(),
            BuiltinFunction::SqliteConnectionDelAttribute => "__delattr__".to_string(),
            BuiltinFunction::SqliteConnectionCursor => "cursor".to_string(),
            BuiltinFunction::SqliteConnectionClose => "close".to_string(),
            BuiltinFunction::SqliteConnectionEnter => "__enter__".to_string(),
            BuiltinFunction::SqliteConnectionExit => "__exit__".to_string(),
            BuiltinFunction::SqliteConnectionExecute => "execute".to_string(),
            BuiltinFunction::SqliteConnectionExecuteMany => "executemany".to_string(),
            BuiltinFunction::SqliteConnectionExecuteScript => "executescript".to_string(),
            BuiltinFunction::SqliteConnectionCommit => "commit".to_string(),
            BuiltinFunction::SqliteConnectionRollback => "rollback".to_string(),
            BuiltinFunction::SqliteConnectionInterrupt => "interrupt".to_string(),
            BuiltinFunction::SqliteConnectionIterDump => "iterdump".to_string(),
            BuiltinFunction::SqliteConnectionCreateFunction => "create_function".to_string(),
            BuiltinFunction::SqliteConnectionCreateAggregate => "create_aggregate".to_string(),
            BuiltinFunction::SqliteConnectionCreateWindowFunction => {
                "create_window_function".to_string()
            }
            BuiltinFunction::SqliteConnectionSetTraceCallback => "set_trace_callback".to_string(),
            BuiltinFunction::SqliteConnectionCreateCollation => "create_collation".to_string(),
            BuiltinFunction::SqliteConnectionSetAuthorizer => "set_authorizer".to_string(),
            BuiltinFunction::SqliteConnectionSetProgressHandler => {
                "set_progress_handler".to_string()
            }
            BuiltinFunction::SqliteConnectionGetLimit => "getlimit".to_string(),
            BuiltinFunction::SqliteConnectionSetLimit => "setlimit".to_string(),
            BuiltinFunction::SqliteConnectionGetConfig => "getconfig".to_string(),
            BuiltinFunction::SqliteConnectionSetConfig => "setconfig".to_string(),
            BuiltinFunction::SqliteConnectionBlobOpen => "blobopen".to_string(),
            BuiltinFunction::SqliteConnectionBackup => "backup".to_string(),
            BuiltinFunction::SqliteCursorInit => "__init__".to_string(),
            BuiltinFunction::SqliteCursorSetAttribute => "__setattr__".to_string(),
            BuiltinFunction::SqliteCursorSetInputSizes => "setinputsizes".to_string(),
            BuiltinFunction::SqliteCursorSetOutputSize => "setoutputsize".to_string(),
            BuiltinFunction::SqliteCursorExecute => "execute".to_string(),
            BuiltinFunction::SqliteCursorExecuteMany => "executemany".to_string(),
            BuiltinFunction::SqliteCursorExecuteScript => "executescript".to_string(),
            BuiltinFunction::SqliteCursorFetchOne => "fetchone".to_string(),
            BuiltinFunction::SqliteCursorFetchMany => "fetchmany".to_string(),
            BuiltinFunction::SqliteCursorFetchAll => "fetchall".to_string(),
            BuiltinFunction::SqliteCursorClose => "close".to_string(),
            BuiltinFunction::SqliteCursorIter => "__iter__".to_string(),
            BuiltinFunction::SqliteCursorNext => "__next__".to_string(),
            BuiltinFunction::SqliteBlobClose => "close".to_string(),
            BuiltinFunction::SqliteBlobRead => "read".to_string(),
            BuiltinFunction::SqliteBlobWrite => "write".to_string(),
            BuiltinFunction::SqliteBlobSeek => "seek".to_string(),
            BuiltinFunction::SqliteBlobTell => "tell".to_string(),
            BuiltinFunction::SqliteBlobEnter => "__enter__".to_string(),
            BuiltinFunction::SqliteBlobExit => "__exit__".to_string(),
            BuiltinFunction::SqliteBlobLen => "__len__".to_string(),
            BuiltinFunction::SqliteBlobGetItem => "__getitem__".to_string(),
            BuiltinFunction::SqliteBlobSetItem => "__setitem__".to_string(),
            BuiltinFunction::SqliteBlobDelItem => "__delitem__".to_string(),
            BuiltinFunction::SqliteBlobIter => "__iter__".to_string(),
            BuiltinFunction::SqliteRowInit => "__init__".to_string(),
            BuiltinFunction::SqliteRowKeys => "keys".to_string(),
            BuiltinFunction::SqliteRowLen => "__len__".to_string(),
            BuiltinFunction::SqliteRowGetItem => "__getitem__".to_string(),
            BuiltinFunction::SqliteRowIter => "__iter__".to_string(),
            BuiltinFunction::SqliteRowEq => "__eq__".to_string(),
            BuiltinFunction::SqliteRowHash => "__hash__".to_string(),
            BuiltinFunction::SreCompile => "compile".to_string(),
            BuiltinFunction::SreTemplate => "template".to_string(),
            BuiltinFunction::SreAsciiIsCased => "ascii_iscased".to_string(),
            BuiltinFunction::SreAsciiToLower => "ascii_tolower".to_string(),
            BuiltinFunction::SreUnicodeIsCased => "unicode_iscased".to_string(),
            BuiltinFunction::SreUnicodeToLower => "unicode_tolower".to_string(),
            BuiltinFunction::ZlibCompress => "compress".to_string(),
            BuiltinFunction::ZlibDecompress => "decompress".to_string(),
            BuiltinFunction::ZlibCompressObj => "compressobj".to_string(),
            BuiltinFunction::ZlibDecompressObj => "decompressobj".to_string(),
            BuiltinFunction::ZlibCrc32 => "crc32".to_string(),
            BuiltinFunction::ZlibCompressObjectCompress => "compress".to_string(),
            BuiltinFunction::ZlibCompressObjectFlush => "flush".to_string(),
            BuiltinFunction::ZlibDecompressObjectDecompress => "decompress".to_string(),
            BuiltinFunction::ZlibDecompressObjectFlush => "flush".to_string(),
            BuiltinFunction::Bz2CompressorInit => "__init__".to_string(),
            BuiltinFunction::Bz2CompressorCompress => "compress".to_string(),
            BuiltinFunction::Bz2CompressorFlush => "flush".to_string(),
            BuiltinFunction::Bz2DecompressorInit => "__init__".to_string(),
            BuiltinFunction::Bz2DecompressorDecompress => "decompress".to_string(),
            BuiltinFunction::LzmaCompressorInit => "__init__".to_string(),
            BuiltinFunction::LzmaCompressorCompress => "compress".to_string(),
            BuiltinFunction::LzmaCompressorFlush => "flush".to_string(),
            BuiltinFunction::LzmaDecompressorInit => "__init__".to_string(),
            BuiltinFunction::LzmaDecompressorDecompress => "decompress".to_string(),
            BuiltinFunction::LzmaIsCheckSupported => "is_check_supported".to_string(),
            BuiltinFunction::LzmaEncodeFilterProperties => "_encode_filter_properties".to_string(),
            BuiltinFunction::LzmaDecodeFilterProperties => "_decode_filter_properties".to_string(),
            BuiltinFunction::SslTxt2Obj => "txt2obj".to_string(),
            BuiltinFunction::SslNid2Obj => "nid2obj".to_string(),
            BuiltinFunction::SslRandStatus => "RAND_status".to_string(),
            BuiltinFunction::SslRandAdd => "RAND_add".to_string(),
            BuiltinFunction::SslRandBytes => "RAND_bytes".to_string(),
            BuiltinFunction::SslRandEgd => "RAND_egd".to_string(),
            BuiltinFunction::SslContextNew => "__new__".to_string(),
            BuiltinFunction::SslContextInit => "__init__".to_string(),
            BuiltinFunction::SslCreateDefaultContext => "create_default_context".to_string(),
            BuiltinFunction::PyExpatParserCreate => "ParserCreate".to_string(),
            BuiltinFunction::PyExpatParserParse => "Parse".to_string(),
            BuiltinFunction::PyExpatParserGetReparseDeferralEnabled => {
                "GetReparseDeferralEnabled".to_string()
            }
            BuiltinFunction::PyExpatParserSetReparseDeferralEnabled => {
                "SetReparseDeferralEnabled".to_string()
            }
            BuiltinFunction::ThreadingRegisterAtexit => "_register_atexit".to_string(),
            BuiltinFunction::OperatorContains => "contains".to_string(),
            BuiltinFunction::FunctoolsReduce => "reduce".to_string(),
            _ => self.builtin_runtime_name(builtin),
        }
    }

    pub(super) fn builtin_attribute_qualname(&self, builtin: BuiltinFunction) -> String {
        match builtin {
            BuiltinFunction::DictFromKeys => "dict.fromkeys".to_string(),
            BuiltinFunction::SetReduce => "set.__reduce__".to_string(),
            BuiltinFunction::BytesMakeTrans => "bytearray.maketrans".to_string(),
            BuiltinFunction::StrMakeTrans => "str.maketrans".to_string(),
            BuiltinFunction::JsonEncodeBaseString => "_json.encode_basestring".to_string(),
            BuiltinFunction::JsonEncodeBaseStringAscii => {
                "_json.encode_basestring_ascii".to_string()
            }
            BuiltinFunction::JsonMakeEncoder => "_json.make_encoder".to_string(),
            BuiltinFunction::JsonMakeEncoderCall => "_json._iterencode".to_string(),
            BuiltinFunction::JsonScannerMakeScanner => "_json.make_scanner".to_string(),
            BuiltinFunction::JsonScannerPyMakeScanner => "json.scanner.py_make_scanner".to_string(),
            BuiltinFunction::JsonScannerScanOnce => "_json.scan_once".to_string(),
            BuiltinFunction::JsonDecoderScanString => "_json.scanstring".to_string(),
            BuiltinFunction::SqliteConnect => "_sqlite3.connect".to_string(),
            BuiltinFunction::SqliteCompleteStatement => "_sqlite3.complete_statement".to_string(),
            BuiltinFunction::SqliteRegisterAdapter => "_sqlite3.register_adapter".to_string(),
            BuiltinFunction::SqliteRegisterConverter => "_sqlite3.register_converter".to_string(),
            BuiltinFunction::SqliteEnableCallbackTracebacks => {
                "_sqlite3.enable_callback_tracebacks".to_string()
            }
            BuiltinFunction::SqliteConnectionInit => "_sqlite3.Connection.__init__".to_string(),
            BuiltinFunction::SqliteConnectionDel => "_sqlite3.Connection.__del__".to_string(),
            BuiltinFunction::SqliteConnectionGetAttribute => {
                "_sqlite3.Connection.__getattribute__".to_string()
            }
            BuiltinFunction::SqliteConnectionSetAttribute => {
                "_sqlite3.Connection.__setattr__".to_string()
            }
            BuiltinFunction::SqliteConnectionDelAttribute => {
                "_sqlite3.Connection.__delattr__".to_string()
            }
            BuiltinFunction::SqliteConnectionCursor => "_sqlite3.Connection.cursor".to_string(),
            BuiltinFunction::SqliteConnectionClose => "_sqlite3.Connection.close".to_string(),
            BuiltinFunction::SqliteConnectionEnter => "_sqlite3.Connection.__enter__".to_string(),
            BuiltinFunction::SqliteConnectionExit => "_sqlite3.Connection.__exit__".to_string(),
            BuiltinFunction::SqliteConnectionExecute => "_sqlite3.Connection.execute".to_string(),
            BuiltinFunction::SqliteConnectionExecuteMany => {
                "_sqlite3.Connection.executemany".to_string()
            }
            BuiltinFunction::SqliteConnectionExecuteScript => {
                "_sqlite3.Connection.executescript".to_string()
            }
            BuiltinFunction::SqliteConnectionCommit => "_sqlite3.Connection.commit".to_string(),
            BuiltinFunction::SqliteConnectionRollback => "_sqlite3.Connection.rollback".to_string(),
            BuiltinFunction::SqliteConnectionInterrupt => {
                "_sqlite3.Connection.interrupt".to_string()
            }
            BuiltinFunction::SqliteConnectionIterDump => "_sqlite3.Connection.iterdump".to_string(),
            BuiltinFunction::SqliteConnectionCreateFunction => {
                "_sqlite3.Connection.create_function".to_string()
            }
            BuiltinFunction::SqliteConnectionCreateAggregate => {
                "_sqlite3.Connection.create_aggregate".to_string()
            }
            BuiltinFunction::SqliteConnectionCreateWindowFunction => {
                "_sqlite3.Connection.create_window_function".to_string()
            }
            BuiltinFunction::SqliteConnectionSetTraceCallback => {
                "_sqlite3.Connection.set_trace_callback".to_string()
            }
            BuiltinFunction::SqliteConnectionCreateCollation => {
                "_sqlite3.Connection.create_collation".to_string()
            }
            BuiltinFunction::SqliteConnectionSetAuthorizer => {
                "_sqlite3.Connection.set_authorizer".to_string()
            }
            BuiltinFunction::SqliteConnectionSetProgressHandler => {
                "_sqlite3.Connection.set_progress_handler".to_string()
            }
            BuiltinFunction::SqliteConnectionGetLimit => "_sqlite3.Connection.getlimit".to_string(),
            BuiltinFunction::SqliteConnectionSetLimit => "_sqlite3.Connection.setlimit".to_string(),
            BuiltinFunction::SqliteConnectionGetConfig => {
                "_sqlite3.Connection.getconfig".to_string()
            }
            BuiltinFunction::SqliteConnectionSetConfig => {
                "_sqlite3.Connection.setconfig".to_string()
            }
            BuiltinFunction::SqliteConnectionBlobOpen => "_sqlite3.Connection.blobopen".to_string(),
            BuiltinFunction::SqliteConnectionBackup => "_sqlite3.Connection.backup".to_string(),
            BuiltinFunction::SqliteCursorInit => "_sqlite3.Cursor.__init__".to_string(),
            BuiltinFunction::SqliteCursorSetAttribute => "_sqlite3.Cursor.__setattr__".to_string(),
            BuiltinFunction::SqliteCursorSetInputSizes => {
                "_sqlite3.Cursor.setinputsizes".to_string()
            }
            BuiltinFunction::SqliteCursorSetOutputSize => {
                "_sqlite3.Cursor.setoutputsize".to_string()
            }
            BuiltinFunction::SqliteCursorExecute => "_sqlite3.Cursor.execute".to_string(),
            BuiltinFunction::SqliteCursorExecuteMany => "_sqlite3.Cursor.executemany".to_string(),
            BuiltinFunction::SqliteCursorExecuteScript => {
                "_sqlite3.Cursor.executescript".to_string()
            }
            BuiltinFunction::SqliteCursorFetchOne => "_sqlite3.Cursor.fetchone".to_string(),
            BuiltinFunction::SqliteCursorFetchMany => "_sqlite3.Cursor.fetchmany".to_string(),
            BuiltinFunction::SqliteCursorFetchAll => "_sqlite3.Cursor.fetchall".to_string(),
            BuiltinFunction::SqliteCursorClose => "_sqlite3.Cursor.close".to_string(),
            BuiltinFunction::SqliteCursorIter => "_sqlite3.Cursor.__iter__".to_string(),
            BuiltinFunction::SqliteCursorNext => "_sqlite3.Cursor.__next__".to_string(),
            BuiltinFunction::SqliteBlobClose => "_sqlite3.Blob.close".to_string(),
            BuiltinFunction::SqliteBlobRead => "_sqlite3.Blob.read".to_string(),
            BuiltinFunction::SqliteBlobWrite => "_sqlite3.Blob.write".to_string(),
            BuiltinFunction::SqliteBlobSeek => "_sqlite3.Blob.seek".to_string(),
            BuiltinFunction::SqliteBlobTell => "_sqlite3.Blob.tell".to_string(),
            BuiltinFunction::SqliteBlobEnter => "_sqlite3.Blob.__enter__".to_string(),
            BuiltinFunction::SqliteBlobExit => "_sqlite3.Blob.__exit__".to_string(),
            BuiltinFunction::SqliteBlobLen => "_sqlite3.Blob.__len__".to_string(),
            BuiltinFunction::SqliteBlobGetItem => "_sqlite3.Blob.__getitem__".to_string(),
            BuiltinFunction::SqliteBlobSetItem => "_sqlite3.Blob.__setitem__".to_string(),
            BuiltinFunction::SqliteBlobDelItem => "_sqlite3.Blob.__delitem__".to_string(),
            BuiltinFunction::SqliteBlobIter => "_sqlite3.Blob.__iter__".to_string(),
            BuiltinFunction::SqliteRowInit => "_sqlite3.Row.__init__".to_string(),
            BuiltinFunction::SqliteRowKeys => "_sqlite3.Row.keys".to_string(),
            BuiltinFunction::SqliteRowLen => "_sqlite3.Row.__len__".to_string(),
            BuiltinFunction::SqliteRowGetItem => "_sqlite3.Row.__getitem__".to_string(),
            BuiltinFunction::SqliteRowIter => "_sqlite3.Row.__iter__".to_string(),
            BuiltinFunction::SqliteRowEq => "_sqlite3.Row.__eq__".to_string(),
            BuiltinFunction::SqliteRowHash => "_sqlite3.Row.__hash__".to_string(),
            BuiltinFunction::ZlibCompress => "zlib.compress".to_string(),
            BuiltinFunction::ZlibDecompress => "zlib.decompress".to_string(),
            BuiltinFunction::ZlibCompressObj => "zlib.compressobj".to_string(),
            BuiltinFunction::ZlibDecompressObj => "zlib.decompressobj".to_string(),
            BuiltinFunction::ZlibCrc32 => "zlib.crc32".to_string(),
            BuiltinFunction::ZlibCompressObjectCompress => "zlib.Compress.compress".to_string(),
            BuiltinFunction::ZlibCompressObjectFlush => "zlib.Compress.flush".to_string(),
            BuiltinFunction::ZlibDecompressObjectDecompress => {
                "zlib.Decompress.decompress".to_string()
            }
            BuiltinFunction::ZlibDecompressObjectFlush => "zlib.Decompress.flush".to_string(),
            BuiltinFunction::Bz2CompressorInit => "_bz2.BZ2Compressor.__init__".to_string(),
            BuiltinFunction::Bz2CompressorCompress => "_bz2.BZ2Compressor.compress".to_string(),
            BuiltinFunction::Bz2CompressorFlush => "_bz2.BZ2Compressor.flush".to_string(),
            BuiltinFunction::Bz2DecompressorInit => "_bz2.BZ2Decompressor.__init__".to_string(),
            BuiltinFunction::Bz2DecompressorDecompress => {
                "_bz2.BZ2Decompressor.decompress".to_string()
            }
            BuiltinFunction::LzmaCompressorInit => "_lzma.LZMACompressor.__init__".to_string(),
            BuiltinFunction::LzmaCompressorCompress => "_lzma.LZMACompressor.compress".to_string(),
            BuiltinFunction::LzmaCompressorFlush => "_lzma.LZMACompressor.flush".to_string(),
            BuiltinFunction::LzmaDecompressorInit => "_lzma.LZMADecompressor.__init__".to_string(),
            BuiltinFunction::LzmaDecompressorDecompress => {
                "_lzma.LZMADecompressor.decompress".to_string()
            }
            BuiltinFunction::LzmaIsCheckSupported => "_lzma.is_check_supported".to_string(),
            BuiltinFunction::LzmaEncodeFilterProperties => {
                "_lzma._encode_filter_properties".to_string()
            }
            BuiltinFunction::LzmaDecodeFilterProperties => {
                "_lzma._decode_filter_properties".to_string()
            }
            BuiltinFunction::SslTxt2Obj => "_ssl.txt2obj".to_string(),
            BuiltinFunction::SslNid2Obj => "_ssl.nid2obj".to_string(),
            BuiltinFunction::SslRandStatus => "_ssl.RAND_status".to_string(),
            BuiltinFunction::SslRandAdd => "_ssl.RAND_add".to_string(),
            BuiltinFunction::SslRandBytes => "_ssl.RAND_bytes".to_string(),
            BuiltinFunction::SslRandEgd => "_ssl.RAND_egd".to_string(),
            BuiltinFunction::SslContextNew => "_ssl._SSLContext.__new__".to_string(),
            BuiltinFunction::SslContextInit => "ssl.SSLContext.__init__".to_string(),
            BuiltinFunction::SslCreateDefaultContext => "ssl.create_default_context".to_string(),
            BuiltinFunction::PyExpatParserCreate => "pyexpat.ParserCreate".to_string(),
            BuiltinFunction::PyExpatParserParse => "pyexpat.xmlparser.Parse".to_string(),
            BuiltinFunction::PyExpatParserGetReparseDeferralEnabled => {
                "pyexpat.xmlparser.GetReparseDeferralEnabled".to_string()
            }
            BuiltinFunction::PyExpatParserSetReparseDeferralEnabled => {
                "pyexpat.xmlparser.SetReparseDeferralEnabled".to_string()
            }
            BuiltinFunction::ThreadingRegisterAtexit => "threading._register_atexit".to_string(),
            BuiltinFunction::SreCompile => "_sre.compile".to_string(),
            BuiltinFunction::SreTemplate => "_sre.template".to_string(),
            BuiltinFunction::SreAsciiIsCased => "_sre.ascii_iscased".to_string(),
            BuiltinFunction::SreAsciiToLower => "_sre.ascii_tolower".to_string(),
            BuiltinFunction::SreUnicodeIsCased => "_sre.unicode_iscased".to_string(),
            BuiltinFunction::SreUnicodeToLower => "_sre.unicode_tolower".to_string(),
            BuiltinFunction::OperatorContains => "operator.contains".to_string(),
            BuiltinFunction::FunctoolsReduce => "reduce".to_string(),
            _ => self.builtin_attribute_name(builtin),
        }
    }

    pub(super) fn builtin_type_dict_entries(
        &self,
        builtin: BuiltinFunction,
    ) -> Vec<(Value, Value)> {
        let mut entries = Vec::new();
        if self.builtin_type_has_none_hash(builtin) {
            entries.push((Value::Str("__hash__".to_string()), Value::None));
        }
        if matches!(
            builtin,
            BuiltinFunction::List
                | BuiltinFunction::Tuple
                | BuiltinFunction::Dict
                | BuiltinFunction::Set
                | BuiltinFunction::FrozenSet
                | BuiltinFunction::Str
                | BuiltinFunction::Bytes
                | BuiltinFunction::ByteArray
        ) {
            entries.push((
                Value::Str("__iter__".to_string()),
                Value::Builtin(BuiltinFunction::Iter),
            ));
        }
        if builtin == BuiltinFunction::Dict {
            entries.push((
                Value::Str("fromkeys".to_string()),
                Value::Builtin(BuiltinFunction::DictFromKeys),
            ));
        } else if builtin == BuiltinFunction::Int {
            entries.push((
                Value::Str("from_bytes".to_string()),
                Value::Builtin(BuiltinFunction::IntFromBytes),
            ));
        } else if builtin == BuiltinFunction::ByteArray {
            entries.push((
                Value::Str("maketrans".to_string()),
                Value::Builtin(BuiltinFunction::BytesMakeTrans),
            ));
        } else if builtin == BuiltinFunction::Type {
            let descriptor = match self.heap.alloc_module(ModuleObject::new(
                "__type_annotations_descriptor__".to_string(),
            )) {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *descriptor.kind_mut() {
                module_data.globals.insert(
                    "__get__".to_string(),
                    Value::Builtin(BuiltinFunction::TypeAnnotationsGet),
                );
            }
            entries.push((
                Value::Str("__annotations__".to_string()),
                Value::Module(descriptor),
            ));
        }
        entries
    }

    pub(super) fn load_attr_builtin(
        &self,
        builtin: BuiltinFunction,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if let Some(overrides) = self.builtin_attr_overrides.get(&builtin)
            && let Some(value) = overrides.get(attr_name)
        {
            return Ok(value.clone());
        }
        let mut builtin_module_name = match builtin {
            BuiltinFunction::CsvReader
            | BuiltinFunction::CsvWriter
            | BuiltinFunction::CsvWriterRow
            | BuiltinFunction::CsvWriterRows
            | BuiltinFunction::CsvRegisterDialect
            | BuiltinFunction::CsvUnregisterDialect
            | BuiltinFunction::CsvGetDialect
            | BuiltinFunction::CsvListDialects
            | BuiltinFunction::CsvFieldSizeLimit
            | BuiltinFunction::CsvDialectValidate
            | BuiltinFunction::CsvReaderIter
            | BuiltinFunction::CsvReaderNext => "_csv",
            BuiltinFunction::PickleDump
            | BuiltinFunction::PickleDumps
            | BuiltinFunction::PickleLoad
            | BuiltinFunction::PickleLoads
            | BuiltinFunction::PickleModuleGetAttr
            | BuiltinFunction::PicklePicklerInit
            | BuiltinFunction::PicklePicklerDump
            | BuiltinFunction::PickleUnpicklerInit
            | BuiltinFunction::PickleUnpicklerLoad
            | BuiltinFunction::PickleBufferInit
            | BuiltinFunction::PickleBufferRaw
            | BuiltinFunction::PickleBufferRelease => "_pickle",
            BuiltinFunction::CopyregReconstructor
            | BuiltinFunction::CopyregNewObj
            | BuiltinFunction::CopyregNewObjEx => "copyreg",
            BuiltinFunction::JsonScannerMakeScanner
            | BuiltinFunction::JsonMakeEncoder
            | BuiltinFunction::JsonMakeEncoderCall
            | BuiltinFunction::JsonEncodeBaseString
            | BuiltinFunction::JsonEncodeBaseStringAscii
            | BuiltinFunction::JsonScannerScanOnce
            | BuiltinFunction::JsonDecoderScanString => "_json",
            BuiltinFunction::SqliteConnect
            | BuiltinFunction::SqliteCompleteStatement
            | BuiltinFunction::SqliteRegisterAdapter
            | BuiltinFunction::SqliteRegisterConverter
            | BuiltinFunction::SqliteEnableCallbackTracebacks
            | BuiltinFunction::SqliteConnectionInit
            | BuiltinFunction::SqliteConnectionDel
            | BuiltinFunction::SqliteConnectionGetAttribute
            | BuiltinFunction::SqliteConnectionSetAttribute
            | BuiltinFunction::SqliteConnectionDelAttribute
            | BuiltinFunction::SqliteConnectionCursor
            | BuiltinFunction::SqliteConnectionClose
            | BuiltinFunction::SqliteConnectionEnter
            | BuiltinFunction::SqliteConnectionExit
            | BuiltinFunction::SqliteConnectionExecute
            | BuiltinFunction::SqliteConnectionExecuteMany
            | BuiltinFunction::SqliteConnectionExecuteScript
            | BuiltinFunction::SqliteConnectionCommit
            | BuiltinFunction::SqliteConnectionRollback
            | BuiltinFunction::SqliteConnectionInterrupt
            | BuiltinFunction::SqliteConnectionIterDump
            | BuiltinFunction::SqliteConnectionCreateFunction
            | BuiltinFunction::SqliteConnectionCreateAggregate
            | BuiltinFunction::SqliteConnectionCreateWindowFunction
            | BuiltinFunction::SqliteConnectionSetTraceCallback
            | BuiltinFunction::SqliteConnectionCreateCollation
            | BuiltinFunction::SqliteConnectionSetAuthorizer
            | BuiltinFunction::SqliteConnectionSetProgressHandler
            | BuiltinFunction::SqliteConnectionGetLimit
            | BuiltinFunction::SqliteConnectionSetLimit
            | BuiltinFunction::SqliteConnectionGetConfig
            | BuiltinFunction::SqliteConnectionSetConfig
            | BuiltinFunction::SqliteConnectionBlobOpen
            | BuiltinFunction::SqliteConnectionBackup
            | BuiltinFunction::SqliteCursorInit
            | BuiltinFunction::SqliteCursorSetAttribute
            | BuiltinFunction::SqliteCursorSetInputSizes
            | BuiltinFunction::SqliteCursorSetOutputSize
            | BuiltinFunction::SqliteCursorExecute
            | BuiltinFunction::SqliteCursorExecuteMany
            | BuiltinFunction::SqliteCursorExecuteScript
            | BuiltinFunction::SqliteCursorFetchOne
            | BuiltinFunction::SqliteCursorFetchMany
            | BuiltinFunction::SqliteCursorFetchAll
            | BuiltinFunction::SqliteCursorClose
            | BuiltinFunction::SqliteCursorIter
            | BuiltinFunction::SqliteCursorNext
            | BuiltinFunction::SqliteBlobClose
            | BuiltinFunction::SqliteBlobRead
            | BuiltinFunction::SqliteBlobWrite
            | BuiltinFunction::SqliteBlobSeek
            | BuiltinFunction::SqliteBlobTell
            | BuiltinFunction::SqliteBlobEnter
            | BuiltinFunction::SqliteBlobExit
            | BuiltinFunction::SqliteBlobLen
            | BuiltinFunction::SqliteBlobGetItem
            | BuiltinFunction::SqliteBlobSetItem
            | BuiltinFunction::SqliteBlobDelItem
            | BuiltinFunction::SqliteBlobIter
            | BuiltinFunction::SqliteRowInit
            | BuiltinFunction::SqliteRowKeys
            | BuiltinFunction::SqliteRowLen
            | BuiltinFunction::SqliteRowGetItem
            | BuiltinFunction::SqliteRowIter
            | BuiltinFunction::SqliteRowEq
            | BuiltinFunction::SqliteRowHash => "_sqlite3",
            BuiltinFunction::ZlibCompress
            | BuiltinFunction::ZlibDecompress
            | BuiltinFunction::ZlibCompressObj
            | BuiltinFunction::ZlibDecompressObj
            | BuiltinFunction::ZlibCrc32
            | BuiltinFunction::ZlibCompressObjectCompress
            | BuiltinFunction::ZlibCompressObjectFlush
            | BuiltinFunction::ZlibDecompressObjectDecompress
            | BuiltinFunction::ZlibDecompressObjectFlush => "zlib",
            BuiltinFunction::Bz2CompressorInit
            | BuiltinFunction::Bz2CompressorCompress
            | BuiltinFunction::Bz2CompressorFlush
            | BuiltinFunction::Bz2DecompressorInit
            | BuiltinFunction::Bz2DecompressorDecompress => "_bz2",
            BuiltinFunction::LzmaCompressorInit
            | BuiltinFunction::LzmaCompressorCompress
            | BuiltinFunction::LzmaCompressorFlush
            | BuiltinFunction::LzmaDecompressorInit
            | BuiltinFunction::LzmaDecompressorDecompress
            | BuiltinFunction::LzmaIsCheckSupported
            | BuiltinFunction::LzmaEncodeFilterProperties
            | BuiltinFunction::LzmaDecodeFilterProperties => "_lzma",
            BuiltinFunction::SslTxt2Obj
            | BuiltinFunction::SslNid2Obj
            | BuiltinFunction::SslRandStatus
            | BuiltinFunction::SslRandAdd
            | BuiltinFunction::SslRandBytes
            | BuiltinFunction::SslRandEgd
            | BuiltinFunction::SslContextNew => "_ssl",
            BuiltinFunction::SslContextInit | BuiltinFunction::SslCreateDefaultContext => "ssl",
            BuiltinFunction::PyExpatParserCreate
            | BuiltinFunction::PyExpatParserParse
            | BuiltinFunction::PyExpatParserGetReparseDeferralEnabled
            | BuiltinFunction::PyExpatParserSetReparseDeferralEnabled => "pyexpat",
            BuiltinFunction::ThreadingRegisterAtexit => "threading",
            BuiltinFunction::SreCompile
            | BuiltinFunction::SreTemplate
            | BuiltinFunction::SreAsciiIsCased
            | BuiltinFunction::SreAsciiToLower
            | BuiltinFunction::SreUnicodeIsCased
            | BuiltinFunction::SreUnicodeToLower => "_sre",
            BuiltinFunction::JsonScannerPyMakeScanner => "json.scanner",
            BuiltinFunction::OperatorContains => "operator",
            BuiltinFunction::FunctoolsReduce => "functools",
            BuiltinFunction::CollectionsDefaultDict => "collections",
            BuiltinFunction::CodecsEncode
            | BuiltinFunction::CodecsDecode
            | BuiltinFunction::CodecsEscapeDecode
            | BuiltinFunction::CodecsMakeIdentityDict
            | BuiltinFunction::CodecsLookup
            | BuiltinFunction::CodecsRegister
            | BuiltinFunction::CodecsCodecInfoInit
            | BuiltinFunction::CodecsGetIncrementalEncoder
            | BuiltinFunction::CodecsGetIncrementalDecoder
            | BuiltinFunction::CodecsIncrementalEncoderInit
            | BuiltinFunction::CodecsIncrementalEncoderEncode
            | BuiltinFunction::CodecsIncrementalEncoderReset
            | BuiltinFunction::CodecsIncrementalEncoderGetState
            | BuiltinFunction::CodecsIncrementalEncoderSetState
            | BuiltinFunction::CodecsIncrementalDecoderInit
            | BuiltinFunction::CodecsIncrementalDecoderDecode
            | BuiltinFunction::CodecsIncrementalDecoderReset
            | BuiltinFunction::CodecsIncrementalDecoderGetState
            | BuiltinFunction::CodecsIncrementalDecoderSetState => "codecs",
            _ => "builtins",
        }
        .to_string();
        if builtin_module_name == "builtins" {
            let in_builtins = self
                .builtins
                .values()
                .any(|value| matches!(value, Value::Builtin(candidate) if *candidate == builtin));
            if !in_builtins && let Some((module_name, _)) = self.builtin_module_binding(builtin) {
                builtin_module_name = module_name;
            }
        }
        match attr_name {
            "__dict__" => {
                let mut entries = self.builtin_type_dict_entries(builtin);
                if self.builtin_is_type_object(builtin)
                    && !entries
                        .iter()
                        .any(|(name, _)| matches!(name, Value::Str(key) if key == "__new__"))
                {
                    entries.push((Value::Str("__new__".to_string()), Value::Builtin(builtin)));
                }
                let dict_value = self.heap.alloc_dict(entries);
                if self.builtin_is_type_object(builtin)
                    && let Some(mappingproxy_class) = self
                        .mappingproxy_type_class
                        .clone()
                        .or_else(|| self.types_module_class("__pyrs_mappingproxy_type__"))
                {
                    let mappingproxy = match self
                        .heap
                        .alloc_instance(InstanceObject::new(mappingproxy_class))
                    {
                        Value::Instance(obj) => obj,
                        _ => unreachable!(),
                    };
                    if let Object::Instance(instance_data) = &mut *mappingproxy.kind_mut() {
                        instance_data
                            .attrs
                            .insert(MAPPING_PROXY_STORAGE_ATTR.to_string(), dict_value);
                    }
                    return Ok(Value::Instance(mappingproxy));
                }
                Ok(dict_value)
            }
            "__name__" => Ok(Value::Str(self.builtin_attribute_name(builtin))),
            "__qualname__" => Ok(Value::Str(self.builtin_attribute_qualname(builtin))),
            "__base__" if self.builtin_is_type_object(builtin) => {
                if builtin == BuiltinFunction::ObjectNew {
                    Ok(Value::None)
                } else {
                    Ok(self
                        .builtins
                        .get("object")
                        .cloned()
                        .unwrap_or(Value::Builtin(BuiltinFunction::ObjectNew)))
                }
            }
            "__bases__" if self.builtin_is_type_object(builtin) => {
                if builtin == BuiltinFunction::ObjectNew {
                    Ok(self.heap.alloc_tuple(Vec::new()))
                } else {
                    Ok(self.heap.alloc_tuple(vec![
                        self.builtins
                            .get("object")
                            .cloned()
                            .unwrap_or(Value::Builtin(BuiltinFunction::ObjectNew)),
                    ]))
                }
            }
            "__mro__" if self.builtin_is_type_object(builtin) => {
                let mut entries = vec![Value::Builtin(builtin)];
                if builtin != BuiltinFunction::ObjectNew {
                    entries.push(
                        self.builtins
                            .get("object")
                            .cloned()
                            .unwrap_or(Value::Builtin(BuiltinFunction::ObjectNew)),
                    );
                }
                Ok(self.heap.alloc_tuple(entries))
            }
            "__module__" => Ok(Value::Str(builtin_module_name)),
            "__self__" => Ok(Value::Builtin(builtin)),
            "__flags__" => Ok(Value::Int(0)),
            "__basicsize__" if self.builtin_is_type_object(builtin) => {
                let basicsize = match builtin {
                    BuiltinFunction::Type => 936,
                    BuiltinFunction::ObjectNew => 16,
                    BuiltinFunction::Tuple => 32,
                    BuiltinFunction::List => 40,
                    BuiltinFunction::Dict => 48,
                    BuiltinFunction::Set | BuiltinFunction::FrozenSet => 200,
                    BuiltinFunction::Bytes => 33,
                    BuiltinFunction::ByteArray => 56,
                    BuiltinFunction::Str => 64,
                    BuiltinFunction::Int | BuiltinFunction::Bool => 24,
                    BuiltinFunction::Float => 24,
                    BuiltinFunction::Complex => 32,
                    _ => 16,
                };
                Ok(Value::Int(basicsize))
            }
            "__itemsize__" if self.builtin_is_type_object(builtin) => {
                let itemsize = match builtin {
                    BuiltinFunction::Type => 40,
                    BuiltinFunction::Tuple => 8,
                    BuiltinFunction::Bytes => 1,
                    BuiltinFunction::Int | BuiltinFunction::Bool => 4,
                    _ => 0,
                };
                Ok(Value::Int(itemsize))
            }
            "__new__"
                if builtin != BuiltinFunction::Type && self.builtin_is_type_object(builtin) =>
            {
                Ok(Value::Builtin(BuiltinFunction::ObjectNew))
            }
            "__new__" => Ok(Value::Builtin(builtin)),
            "__init__" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::ObjectInit))
            }
            "__hash__" if self.builtin_type_has_none_hash(builtin) => Ok(Value::None),
            "__hash__" => {
                let wrapper = match self
                    .heap
                    .alloc_module(ModuleObject::new("__builtin_hash_receiver__".to_string()))
                {
                    Value::Module(module) => module,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                    module_data
                        .globals
                        .insert("value".to_string(), Value::Builtin(builtin));
                }
                Ok(self.alloc_builtin_bound_method(BuiltinFunction::Id, wrapper))
            }
            "__eq__" if builtin == BuiltinFunction::ObjectNew => {
                Ok(Value::Builtin(BuiltinFunction::OperatorEq))
            }
            "__ne__" if builtin == BuiltinFunction::ObjectNew => {
                Ok(Value::Builtin(BuiltinFunction::OperatorNe))
            }
            "__lt__" if matches!(builtin, BuiltinFunction::ObjectNew | BuiltinFunction::Int) => {
                Ok(Value::Builtin(BuiltinFunction::OperatorLt))
            }
            "__getformat__" if builtin == BuiltinFunction::Float => {
                Ok(Value::Builtin(BuiltinFunction::Str))
            }
            "from_iterable" if builtin == BuiltinFunction::ItertoolsChain => {
                Ok(Value::Builtin(BuiltinFunction::ItertoolsChainFromIterable))
            }
            "fromhex" if builtin == BuiltinFunction::Float => {
                Ok(Value::Builtin(BuiltinFunction::FloatFromHex))
            }
            "fromhex" if builtin == BuiltinFunction::Bytes => {
                Ok(Value::Builtin(BuiltinFunction::BytesFromHex))
            }
            "fromhex" if builtin == BuiltinFunction::ByteArray => {
                Ok(Value::Builtin(BuiltinFunction::ByteArrayFromHex))
            }
            "hex" if builtin == BuiltinFunction::Float => {
                Ok(Value::Builtin(BuiltinFunction::FloatHex))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::Dict => {
                Ok(Value::Builtin(BuiltinFunction::DictTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::List => {
                Ok(Value::Builtin(BuiltinFunction::ListTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::Tuple => {
                Ok(Value::Builtin(BuiltinFunction::TupleTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::Set => {
                Ok(Value::Builtin(BuiltinFunction::SetTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::FrozenSet => {
                Ok(Value::Builtin(BuiltinFunction::FrozenSetTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::Str => {
                Ok(Value::Builtin(BuiltinFunction::StrTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::Bytes => {
                Ok(Value::Builtin(BuiltinFunction::BytesTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::ByteArray => {
                Ok(Value::Builtin(BuiltinFunction::ByteArrayTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::TypesMappingProxy => {
                Ok(Value::Builtin(BuiltinFunction::MappingProxyTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::CollectionsDefaultDict => Ok(
                Value::Builtin(BuiltinFunction::CollectionsDefaultDictTypeRepr),
            ),
            "__repr__" | "__str__" if builtin == BuiltinFunction::CollectionsOrderedDict => Ok(
                Value::Builtin(BuiltinFunction::CollectionsOrderedDictTypeRepr),
            ),
            "__repr__" | "__str__" if builtin == BuiltinFunction::CollectionsCounter => {
                Ok(Value::Builtin(BuiltinFunction::CollectionsCounterTypeRepr))
            }
            "__repr__" | "__str__" if builtin == BuiltinFunction::CollectionsDeque => {
                Ok(Value::Builtin(BuiltinFunction::CollectionsDequeTypeRepr))
            }
            "__format__" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::Format))
            }
            "__format__" if builtin == BuiltinFunction::Float => {
                Ok(Value::Builtin(BuiltinFunction::Format))
            }
            "__format__" if builtin == BuiltinFunction::Str => {
                Ok(Value::Builtin(BuiltinFunction::Format))
            }
            "__repr__" | "__str__" => Ok(Value::Builtin(BuiltinFunction::Repr)),
            "__format__" => Ok(Value::Builtin(BuiltinFunction::ObjectFormat)),
            "__reduce_ex__" | "__reduce__" => {
                Ok(self.alloc_reduce_ex_bound_method(Value::Builtin(builtin)))
            }
            "bit_length" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::IntBitLength))
            }
            "__add__" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::OperatorAdd))
            }
            "from_bytes" if builtin == BuiltinFunction::Int => {
                Ok(Value::Builtin(BuiltinFunction::IntFromBytes))
            }
            "append" if builtin == BuiltinFunction::List => {
                Ok(Value::Builtin(BuiltinFunction::ListAppendDescriptor))
            }
            "pop" if builtin == BuiltinFunction::List => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__list_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::List));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::ListPop, receiver))
            }
            "__getitem__" if matches!(builtin, BuiltinFunction::List | BuiltinFunction::Tuple) => {
                Ok(Value::Builtin(BuiltinFunction::OperatorGetItem))
            }
            "__contains__" if matches!(builtin, BuiltinFunction::List | BuiltinFunction::Tuple) => {
                Ok(Value::Builtin(BuiltinFunction::OperatorContains))
            }
            "__len__"
                if matches!(
                    builtin,
                    BuiltinFunction::List
                        | BuiltinFunction::Tuple
                        | BuiltinFunction::Dict
                        | BuiltinFunction::Set
                        | BuiltinFunction::FrozenSet
                        | BuiltinFunction::Str
                        | BuiltinFunction::Bytes
                        | BuiltinFunction::ByteArray
                ) =>
            {
                Ok(Value::Builtin(BuiltinFunction::Len))
            }
            "__iter__"
                if matches!(
                    builtin,
                    BuiltinFunction::List
                        | BuiltinFunction::Tuple
                        | BuiltinFunction::Dict
                        | BuiltinFunction::Set
                        | BuiltinFunction::FrozenSet
                        | BuiltinFunction::Str
                        | BuiltinFunction::Bytes
                        | BuiltinFunction::ByteArray
                ) =>
            {
                Ok(Value::Builtin(BuiltinFunction::Iter))
            }
            "maketrans" if builtin == BuiltinFunction::Bytes => Ok(self
                .alloc_builtin_unbound_method(
                    "__bytes_unbound_method__",
                    Value::Builtin(BuiltinFunction::Bytes),
                    BuiltinFunction::BytesMakeTrans,
                )),
            "maketrans" if builtin == BuiltinFunction::ByteArray => Ok(self
                .alloc_builtin_unbound_method(
                    "__bytearray_unbound_method__",
                    Value::Builtin(BuiltinFunction::ByteArray),
                    BuiltinFunction::BytesMakeTrans,
                )),
            "fromkeys" if builtin == BuiltinFunction::Dict => Ok(self
                .alloc_builtin_unbound_method(
                    "__dict_unbound_method__",
                    Value::Builtin(BuiltinFunction::Dict),
                    BuiltinFunction::DictFromKeys,
                )),
            "keys" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictKeys, receiver))
            }
            "update" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictUpdateMethod, receiver))
            }
            "setdefault" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictSetDefault, receiver))
            }
            "get" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictGet, receiver))
            }
            "pop" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictPop, receiver))
            }
            "copy" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictCopy, receiver))
            }
            "items" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictItems, receiver))
            }
            "values" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictValues, receiver))
            }
            "clear" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictClear, receiver))
            }
            "__getitem__" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictGetItem, receiver))
            }
            "__setitem__" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictSetItem, receiver))
            }
            "__delitem__" if builtin == BuiltinFunction::Dict => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__dict_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Dict));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::DictDelItem, receiver))
            }
            "__contains__" if builtin == BuiltinFunction::Dict => {
                Ok(Value::Builtin(BuiltinFunction::OperatorContains))
            }
            "__contains__"
                if matches!(builtin, BuiltinFunction::Set | BuiltinFunction::FrozenSet) =>
            {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__set_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(builtin));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetContains, receiver))
            }
            "maketrans" if builtin == BuiltinFunction::Str => Ok(self
                .alloc_builtin_unbound_method(
                    "__str_unbound_method__",
                    Value::Builtin(BuiltinFunction::Str),
                    BuiltinFunction::StrMakeTrans,
                )),
            "count" if builtin == BuiltinFunction::Tuple => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__tuple_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Tuple));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::TupleCount, receiver))
            }
            "index" if builtin == BuiltinFunction::Tuple => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__tuple_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Tuple));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::TupleIndex, receiver))
            }
            "count" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrCount, receiver))
            }
            "index" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrIndex, receiver))
            }
            "translate" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrTranslate, receiver))
            }
            "title" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrTitle, receiver))
            }
            "lower" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrLower, receiver))
            }
            "swapcase" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrSwapCase, receiver))
            }
            "capitalize" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrCapitalize, receiver))
            }
            "upper" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrUpper, receiver))
            }
            "join" if builtin == BuiltinFunction::Str => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::StrJoin, receiver))
            }
            attr_name
                if builtin == BuiltinFunction::Str
                    && matches!(
                        attr_name,
                        "isupper"
                            | "islower"
                            | "isascii"
                            | "isalpha"
                            | "isalnum"
                            | "isdigit"
                            | "isspace"
                            | "isidentifier"
                    ) =>
            {
                let kind = match attr_name {
                    "isupper" => NativeMethodKind::StrIsUpper,
                    "islower" => NativeMethodKind::StrIsLower,
                    "isascii" => NativeMethodKind::StrIsAscii,
                    "isalpha" => NativeMethodKind::StrIsAlpha,
                    "isalnum" => NativeMethodKind::StrIsAlNum,
                    "isdigit" => NativeMethodKind::StrIsDigit,
                    "isspace" => NativeMethodKind::StrIsSpace,
                    "isidentifier" => NativeMethodKind::StrIsIdentifier,
                    _ => unreachable!(),
                };
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
                }
                Ok(self.alloc_native_bound_method(kind, receiver))
            }
            "__instancecheck__" if self.builtin_is_type_object(builtin) => Ok(self
                .alloc_builtin_unbound_method(
                    "__builtin_unbound_method__",
                    Value::Builtin(builtin),
                    BuiltinFunction::TypeInstanceCheck,
                )),
            "__get__"
                if matches!(
                    builtin,
                    BuiltinFunction::ObjectInit
                        | BuiltinFunction::ObjectNew
                        | BuiltinFunction::OperatorLt
                        | BuiltinFunction::ListAppendDescriptor
                        | BuiltinFunction::DictFromKeys
                        | BuiltinFunction::IntFromBytes
                ) =>
            {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__builtin_descriptor__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("builtin".to_string(), Value::Builtin(builtin));
                }
                Ok(self
                    .alloc_native_bound_method(NativeMethodKind::FunctionDescriptorGet, receiver))
            }
            "__subclasscheck__" if self.builtin_is_type_object(builtin) => Ok(self
                .alloc_builtin_unbound_method(
                    "__builtin_unbound_method__",
                    Value::Builtin(builtin),
                    BuiltinFunction::TypeSubclassCheck,
                )),
            "__prepare__" if self.builtin_is_type_object(builtin) => Ok(self
                .alloc_builtin_unbound_method(
                    "__builtin_unbound_method__",
                    Value::Builtin(builtin),
                    BuiltinFunction::TypePrepare,
                )),
            _ => Err(RuntimeError::attribute_error(format!(
                "builtin has no attribute '{}'",
                attr_name
            ))),
        }
    }

    fn builtin_attr_is_overridable(attr_name: &str) -> bool {
        matches!(
            attr_name,
            "__name__"
                | "__qualname__"
                | "__module__"
                | "__doc__"
                | "__annotate__"
                | "__defaults__"
                | "__kwdefaults__"
        )
    }

    pub(super) fn store_attr_builtin(
        &mut self,
        builtin: BuiltinFunction,
        attr_name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if !Self::builtin_attr_is_overridable(attr_name) {
            return Err(RuntimeError::attribute_error(format!(
                "builtin has no writable attribute '{}'",
                attr_name
            )));
        }
        self.builtin_attr_overrides
            .entry(builtin)
            .or_default()
            .insert(attr_name.to_string(), value);
        Ok(())
    }

    pub(super) fn delete_attr_builtin(
        &mut self,
        builtin: BuiltinFunction,
        attr_name: &str,
    ) -> Result<(), RuntimeError> {
        if !Self::builtin_attr_is_overridable(attr_name) {
            return Err(RuntimeError::attribute_error(format!(
                "builtin has no deletable attribute '{}'",
                attr_name
            )));
        }
        let deleted = self
            .builtin_attr_overrides
            .get_mut(&builtin)
            .and_then(|overrides| overrides.remove(attr_name))
            .is_some();
        if !deleted {
            return Err(RuntimeError::attribute_error(format!(
                "builtin has no attribute '{}'",
                attr_name
            )));
        }
        if self
            .builtin_attr_overrides
            .get(&builtin)
            .is_some_and(HashMap::is_empty)
        {
            self.builtin_attr_overrides.remove(&builtin);
        }
        Ok(())
    }

    pub(super) fn load_attr_class_builtin_base_method(
        &self,
        class: &ObjRef,
        attr_name: &str,
    ) -> Option<Value> {
        if (self.class_has_builtin_int_base(class)
            || self.class_has_builtin_float_base(class)
            || self.class_has_builtin_str_base(class))
            && attr_name == "__format__"
        {
            return Some(Value::Builtin(BuiltinFunction::Format));
        }
        if (self.class_has_builtin_int_base(class)
            || self.class_has_builtin_float_base(class)
            || self.class_has_builtin_str_base(class))
            && attr_name == "__str__"
        {
            return Some(Value::Builtin(BuiltinFunction::Str));
        }
        if (self.class_has_builtin_int_base(class)
            || self.class_has_builtin_float_base(class)
            || self.class_has_builtin_str_base(class))
            && attr_name == "__repr__"
        {
            return Some(Value::Builtin(BuiltinFunction::Repr));
        }
        if (self.class_has_builtin_int_base(class)
            || self.class_has_builtin_float_base(class)
            || self.class_has_builtin_str_base(class))
            && (attr_name == "__reduce_ex__" || attr_name == "__reduce__")
        {
            return Some(Value::Builtin(BuiltinFunction::ObjectReduceEx));
        }
        if self.class_has_builtin_tuple_base(class) && attr_name == "count" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__tuple_unbound_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data
                    .globals
                    .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Tuple));
            }
            return Some(self.alloc_native_bound_method(NativeMethodKind::TupleCount, receiver));
        }
        if self.class_has_builtin_tuple_base(class) && attr_name == "index" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__tuple_unbound_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data
                    .globals
                    .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Tuple));
            }
            return Some(self.alloc_native_bound_method(NativeMethodKind::TupleIndex, receiver));
        }
        if self.class_has_builtin_str_base(class) && attr_name == "count" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__str_unbound_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data
                    .globals
                    .insert("owner".to_string(), Value::Builtin(BuiltinFunction::Str));
            }
            return Some(self.alloc_native_bound_method(NativeMethodKind::StrCount, receiver));
        }
        None
    }

    pub(super) fn load_attr_list_method(
        &self,
        list: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__len__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Len, list));
        }
        if attr_name == "__getitem__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::OperatorGetItem, list));
        }
        if attr_name == "__contains__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::OperatorContains, list));
        }
        if attr_name == "__iter__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Iter, list));
        }
        let kind = match attr_name {
            "__init__" => NativeMethodKind::ListInit,
            "__eq__" => NativeMethodKind::ListEq,
            "__ne__" => NativeMethodKind::ListNe,
            "append" => NativeMethodKind::ListAppend,
            "extend" => NativeMethodKind::ListExtend,
            "insert" => NativeMethodKind::ListInsert,
            "remove" => NativeMethodKind::ListRemove,
            "pop" => NativeMethodKind::ListPop,
            "count" => NativeMethodKind::ListCount,
            "copy" => NativeMethodKind::ListCopy,
            "clear" => NativeMethodKind::ListClear,
            "index" => NativeMethodKind::ListIndex,
            "reverse" => NativeMethodKind::ListReverse,
            "sort" => NativeMethodKind::ListSort,
            _ => {
                return Err(RuntimeError::attribute_error(format!(
                    "list has no attribute '{}'",
                    attr_name
                )));
            }
        };
        Ok(self.alloc_native_bound_method(kind, list))
    }

    pub(super) fn load_attr_tuple_method(
        &self,
        tuple: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__len__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Len, tuple));
        }
        if attr_name == "__getitem__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::OperatorGetItem, tuple));
        }
        if attr_name == "__contains__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::OperatorContains, tuple));
        }
        if attr_name == "__iter__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Iter, tuple));
        }
        let kind = match attr_name {
            "__eq__" => NativeMethodKind::TupleEq,
            "__ne__" => NativeMethodKind::TupleNe,
            "count" => NativeMethodKind::TupleCount,
            "index" => NativeMethodKind::TupleIndex,
            _ => {
                return Err(RuntimeError::attribute_error(format!(
                    "tuple has no attribute '{}'",
                    attr_name
                )));
            }
        };
        Ok(self.alloc_native_bound_method(kind, tuple))
    }

    pub(super) fn load_attr_cell(
        &self,
        cell: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__doc__" {
            return Ok(Value::None);
        }
        let cell_kind = cell.kind();
        let Object::Cell(cell_data) = &*cell_kind else {
            return Err(RuntimeError::attribute_error(format!(
                "cell has no attribute '{}'",
                attr_name
            )));
        };
        match attr_name {
            "cell_contents" => cell_data
                .value
                .clone()
                .ok_or_else(|| RuntimeError::new("Cell is empty")),
            _ => Err(RuntimeError::attribute_error(format!(
                "cell has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_int_method(
        &self,
        value: Value,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        match attr_name {
            "numerator" | "real" => return Ok(value.clone()),
            "denominator" => return Ok(Value::Int(1)),
            "imag" => return Ok(Value::Int(0)),
            _ => {}
        }
        if attr_name == "__new__" {
            return Ok(Value::Builtin(BuiltinFunction::ObjectNew));
        }
        if attr_name == "__format__" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__int_format_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data.globals.insert("value".to_string(), value);
            }
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Format, receiver));
        }
        if attr_name == "__lt__" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__int_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data.globals.insert("value".to_string(), value);
            }
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::OperatorLt, receiver));
        }
        if attr_name == "__doc__" {
            return Ok(Value::None);
        }
        let kind = match attr_name {
            "to_bytes" => NativeMethodKind::IntToBytes,
            "bit_length" => NativeMethodKind::IntBitLengthMethod,
            "__index__" | "__int__" => NativeMethodKind::IntIndexMethod,
            _ => {
                return Err(RuntimeError::attribute_error(format!(
                    "int has no attribute '{}'",
                    attr_name
                )));
            }
        };
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__int_method__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data.globals.insert("value".to_string(), value);
        }
        Ok(self.alloc_native_bound_method(kind, receiver))
    }

    pub(super) fn load_attr_str_method(
        &self,
        text: String,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__doc__" {
            return Ok(Value::None);
        }
        if attr_name == "__iter__" {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__str_iter_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data
                    .globals
                    .insert("value".to_string(), Value::Str(text));
            }
            return Ok(self.alloc_native_bound_method(
                NativeMethodKind::Builtin(BuiltinFunction::Iter),
                receiver,
            ));
        }
        let kind = match attr_name {
            "startswith" => NativeMethodKind::StrStartsWith,
            "endswith" => NativeMethodKind::StrEndsWith,
            "replace" => NativeMethodKind::StrReplace,
            "upper" => NativeMethodKind::StrUpper,
            "lower" => NativeMethodKind::StrLower,
            "swapcase" => NativeMethodKind::StrSwapCase,
            "capitalize" => NativeMethodKind::StrCapitalize,
            "title" => NativeMethodKind::StrTitle,
            "encode" => NativeMethodKind::StrEncode,
            "decode" => NativeMethodKind::StrDecode,
            "removeprefix" => NativeMethodKind::StrRemovePrefix,
            "removesuffix" => NativeMethodKind::StrRemoveSuffix,
            "format" => NativeMethodKind::StrFormat,
            "isupper" => NativeMethodKind::StrIsUpper,
            "islower" => NativeMethodKind::StrIsLower,
            "isascii" => NativeMethodKind::StrIsAscii,
            "isalpha" => NativeMethodKind::StrIsAlpha,
            "isalnum" => NativeMethodKind::StrIsAlNum,
            "isdigit" => NativeMethodKind::StrIsDigit,
            "isspace" => NativeMethodKind::StrIsSpace,
            "isidentifier" => NativeMethodKind::StrIsIdentifier,
            "join" => NativeMethodKind::StrJoin,
            "split" => NativeMethodKind::StrSplit,
            "splitlines" => NativeMethodKind::StrSplitLines,
            "rsplit" => NativeMethodKind::StrRSplit,
            "partition" => NativeMethodKind::StrPartition,
            "rpartition" => NativeMethodKind::StrRPartition,
            "count" => NativeMethodKind::StrCount,
            "find" => NativeMethodKind::StrFind,
            "translate" => NativeMethodKind::StrTranslate,
            "index" => NativeMethodKind::StrIndex,
            "rfind" => NativeMethodKind::StrRFind,
            "lstrip" => NativeMethodKind::StrLStrip,
            "rstrip" => NativeMethodKind::StrRStrip,
            "strip" => NativeMethodKind::StrStrip,
            "ljust" => NativeMethodKind::StrLJust,
            "center" => NativeMethodKind::StrCenter,
            "expandtabs" => NativeMethodKind::StrExpandTabs,
            _ => {
                return Err(RuntimeError::attribute_error(format!(
                    "str has no attribute '{}'",
                    attr_name
                )));
            }
        };
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__str_method__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            module_data
                .globals
                .insert("value".to_string(), Value::Str(text));
        }
        Ok(self.alloc_native_bound_method(kind, receiver))
    }

    pub(super) fn load_attr_bytes_method(
        &self,
        receiver_value: Value,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__doc__" {
            return Ok(Value::None);
        }
        if attr_name == "__iter__" {
            match receiver_value {
                Value::Bytes(bytes) | Value::ByteArray(bytes) => {
                    return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Iter, bytes));
                }
                _ => return Err(RuntimeError::type_error("bytes receiver is invalid")),
            }
        }
        let type_name = if matches!(receiver_value, Value::ByteArray(_)) {
            "bytearray"
        } else {
            "bytes"
        };
        let kind = match attr_name {
            "decode" => NativeMethodKind::BytesDecode,
            "hex" => NativeMethodKind::BytesHex,
            "startswith" => NativeMethodKind::BytesStartsWith,
            "endswith" => NativeMethodKind::BytesEndsWith,
            "count" => NativeMethodKind::BytesCount,
            "find" => NativeMethodKind::BytesFind,
            "index" => NativeMethodKind::BytesIndex,
            "split" => NativeMethodKind::BytesSplit,
            "splitlines" => NativeMethodKind::BytesSplitLines,
            "translate" => NativeMethodKind::BytesTranslate,
            "join" => NativeMethodKind::BytesJoin,
            "ljust" => NativeMethodKind::BytesLJust,
            "lstrip" => NativeMethodKind::BytesLStrip,
            "strip" => NativeMethodKind::BytesStrip,
            "rstrip" => NativeMethodKind::BytesRStrip,
            "append" if matches!(receiver_value, Value::ByteArray(_)) => {
                NativeMethodKind::ByteArrayAppend
            }
            "extend" if matches!(receiver_value, Value::ByteArray(_)) => {
                NativeMethodKind::ByteArrayExtend
            }
            "clear" if matches!(receiver_value, Value::ByteArray(_)) => {
                NativeMethodKind::ByteArrayClear
            }
            "resize" if matches!(receiver_value, Value::ByteArray(_)) => {
                NativeMethodKind::ByteArrayResize
            }
            _ => {
                return Err(RuntimeError::new(format!(
                    "{} has no attribute '{}'",
                    type_name, attr_name
                )));
            }
        };
        let receiver = match self
            .heap
            .alloc_module(ModuleObject::new("__bytes_method__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *receiver.kind_mut() {
            match receiver_value {
                Value::Bytes(_) | Value::ByteArray(_) => {
                    module_data
                        .globals
                        .insert("value".to_string(), receiver_value);
                }
                _ => {
                    return Err(RuntimeError::type_error("bytes receiver is invalid"));
                }
            }
        }
        Ok(self.alloc_native_bound_method(kind, receiver))
    }

    pub(super) fn load_attr_iterator(
        &self,
        iterator: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let (type_name, range_start, range_stop, range_step, allow_reduce, allow_next) =
            match &*iterator.kind() {
                Object::Iterator(state) => match &state.kind {
                    IteratorKind::RangeObject { start, stop, step } => (
                        "range",
                        Some(start.clone()),
                        Some(stop.clone()),
                        Some(step.clone()),
                        true,
                        false,
                    ),
                    IteratorKind::Map { .. } => ("map", None, None, None, true, true),
                    IteratorKind::Zip { .. } => ("zip", None, None, None, true, true),
                    IteratorKind::Range { .. } => ("range_iterator", None, None, None, false, true),
                    IteratorKind::List(_) => ("list_iterator", None, None, None, false, true),
                    IteratorKind::Tuple(_) => ("tuple_iterator", None, None, None, false, true),
                    IteratorKind::Str(_) => ("str_iterator", None, None, None, false, true),
                    IteratorKind::Dict(_) => ("dict_keyiterator", None, None, None, false, true),
                    IteratorKind::Set(_) => ("set_iterator", None, None, None, false, true),
                    IteratorKind::Bytes(_) => ("bytes_iterator", None, None, None, false, true),
                    IteratorKind::ByteArray(_) => {
                        ("bytearray_iterator", None, None, None, false, true)
                    }
                    IteratorKind::MemoryView(_) => {
                        ("memoryview_iterator", None, None, None, false, true)
                    }
                    IteratorKind::Cycle { .. } => ("cycle", None, None, None, false, true),
                    IteratorKind::Count { .. } => ("count", None, None, None, false, true),
                    IteratorKind::SequenceGetItem { .. } => {
                        ("iterator", None, None, None, false, true)
                    }
                    IteratorKind::CpythonSequence { .. } => {
                        ("iterator", None, None, None, false, true)
                    }
                    IteratorKind::CallIter { .. } => {
                        ("callable_iterator", None, None, None, false, true)
                    }
                },
                _ => {
                    return Err(RuntimeError::attribute_error(
                        "attribute access unsupported type",
                    ));
                }
            };
        match attr_name {
            "__iter__" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::IteratorIter, iterator))
            }
            "__next__" if allow_next => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::IteratorNext, iterator))
            }
            "__reduce_ex__" | "__reduce__" if allow_reduce => {
                Ok(self.alloc_reduce_ex_bound_method(Value::Iterator(iterator)))
            }
            "start" if range_start.is_some() => Ok(value_from_bigint(
                range_start.expect("range start is present"),
            )),
            "stop" if range_stop.is_some() => Ok(value_from_bigint(
                range_stop.expect("range stop is present"),
            )),
            "step" if range_step.is_some() => Ok(value_from_bigint(
                range_step.expect("range step is present"),
            )),
            _ => Err(RuntimeError::new(format!(
                "{type_name} has no attribute '{attr_name}'"
            ))),
        }
    }

    pub(super) fn load_attr_memoryview(
        &self,
        view: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        match attr_name {
            "__enter__" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewEnter, view))
            }
            "__exit__" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewExit, view))
            }
            "toreadonly" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewToReadOnly, view))
            }
            "cast" => Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewCast, view)),
            "tolist" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewToList, view))
            }
            "release" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::MemoryViewRelease, view))
            }
            "tobytes" => Ok(self.alloc_builtin_bound_method(BuiltinFunction::Bytes, view)),
            "contiguous" | "c_contiguous" | "f_contiguous" => match &*view.kind() {
                Object::MemoryView(view_data) => {
                    with_bytes_like_source(&view_data.source, |values| {
                        let (start, end) =
                            memoryview_bounds(view_data.start, view_data.length, values.len());
                        let byte_len = end.saturating_sub(start);
                        let (shape, strides) = memoryview_shape_and_strides(view_data, byte_len);
                        let (_, c_contiguous, f_contiguous) = if view_data.contiguous {
                            memoryview_contiguity(
                                &shape,
                                &strides,
                                view_data.itemsize.max(1) as isize,
                            )
                        } else {
                            (false, false, false)
                        };
                        let contiguous = c_contiguous || f_contiguous;
                        Value::Bool(match attr_name {
                            "c_contiguous" => c_contiguous,
                            "f_contiguous" => f_contiguous,
                            _ => contiguous,
                        })
                    })
                    .ok_or_else(|| RuntimeError::type_error("memoryview receiver is invalid"))
                }
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "readonly" => match &*view.kind() {
                Object::MemoryView(view_data) => bytes_like_source_is_readonly(&view_data.source)
                    .map(Value::Bool)
                    .ok_or_else(|| RuntimeError::type_error("memoryview receiver is invalid")),
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "obj" => match &*view.kind() {
                Object::MemoryView(view_data) => match &*view_data.source.kind() {
                    Object::Bytes(_) => Ok(Value::Bytes(view_data.source.clone())),
                    Object::ByteArray(_) => Ok(Value::ByteArray(view_data.source.clone())),
                    Object::Instance(_) => Ok(Value::Instance(view_data.source.clone())),
                    _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
                },
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "itemsize" => match &*view.kind() {
                Object::MemoryView(view_data) => Ok(Value::Int(view_data.itemsize as i64)),
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "format" => match &*view.kind() {
                Object::MemoryView(view_data) => Ok(Value::Str(
                    view_data.format.clone().unwrap_or_else(|| "B".to_string()),
                )),
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "ndim" => match &*view.kind() {
                Object::MemoryView(view_data) => {
                    with_bytes_like_source(&view_data.source, |values| {
                        let (start, end) =
                            memoryview_bounds(view_data.start, view_data.length, values.len());
                        let byte_len = end.saturating_sub(start);
                        let (shape, _strides) = memoryview_shape_and_strides(view_data, byte_len);
                        Value::Int(shape.len() as i64)
                    })
                    .ok_or_else(|| RuntimeError::type_error("memoryview receiver is invalid"))
                }
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "shape" => match &*view.kind() {
                Object::MemoryView(view_data) => {
                    with_bytes_like_source(&view_data.source, |values| {
                        let (start, end) =
                            memoryview_bounds(view_data.start, view_data.length, values.len());
                        let byte_len = end.saturating_sub(start);
                        let (shape, _strides) = memoryview_shape_and_strides(view_data, byte_len);
                        let tuple_values = shape
                            .into_iter()
                            .map(|dim| Value::Int(dim as i64))
                            .collect::<Vec<Value>>();
                        self.heap.alloc_tuple(tuple_values)
                    })
                    .ok_or_else(|| RuntimeError::type_error("memoryview receiver is invalid"))
                }
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "strides" => match &*view.kind() {
                Object::MemoryView(view_data) => {
                    with_bytes_like_source(&view_data.source, |values| {
                        let (start, end) =
                            memoryview_bounds(view_data.start, view_data.length, values.len());
                        let byte_len = end.saturating_sub(start);
                        let (_shape, strides) = memoryview_shape_and_strides(view_data, byte_len);
                        let tuple_values = strides
                            .into_iter()
                            .map(|dim| Value::Int(dim as i64))
                            .collect::<Vec<Value>>();
                        self.heap.alloc_tuple(tuple_values)
                    })
                    .ok_or_else(|| RuntimeError::type_error("memoryview receiver is invalid"))
                }
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            "nbytes" => match &*view.kind() {
                Object::MemoryView(view_data) => {
                    with_bytes_like_source(&view_data.source, |values| {
                        let (start, end) =
                            memoryview_bounds(view_data.start, view_data.length, values.len());
                        let byte_len = end.saturating_sub(start);
                        let (shape, _strides) = memoryview_shape_and_strides(view_data, byte_len);
                        let mut elements = 1usize;
                        for dim in shape {
                            if dim < 0 {
                                return Value::Int(0);
                            }
                            let Ok(dim_usize) = usize::try_from(dim) else {
                                return Value::Int(0);
                            };
                            let Some(next) = elements.checked_mul(dim_usize) else {
                                return Value::Int(0);
                            };
                            elements = next;
                        }
                        let nbytes = elements.saturating_mul(view_data.itemsize.max(1));
                        Value::Int(nbytes as i64)
                    })
                    .ok_or_else(|| RuntimeError::type_error("memoryview receiver is invalid"))
                }
                _ => Err(RuntimeError::type_error("memoryview receiver is invalid")),
            },
            _ => Err(RuntimeError::attribute_error(format!(
                "memoryview has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_set_method(
        &self,
        set: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let (type_name, is_frozenset) = match &*set.kind() {
            Object::Set(_) => ("set", false),
            Object::FrozenSet(_) => ("frozenset", true),
            _ => {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            }
        };
        match attr_name {
            "__reduce__" => {
                Ok(self.alloc_builtin_bound_method(BuiltinFunction::SetReduce, set.clone()))
            }
            "__iter__" => Ok(self.alloc_builtin_bound_method(BuiltinFunction::Iter, set)),
            "__reduce_ex__" => match &*set.kind() {
                Object::Set(_) => Ok(self.alloc_reduce_ex_bound_method(Value::Set(set.clone()))),
                Object::FrozenSet(_) => {
                    Ok(self.alloc_reduce_ex_bound_method(Value::FrozenSet(set.clone())))
                }
                _ => Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                )),
            },
            "__contains__" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetContains, set))
            }
            "issuperset" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetIsSuperset, set))
            }
            "issubset" => Ok(self.alloc_native_bound_method(NativeMethodKind::SetIsSubset, set)),
            "isdisjoint" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetIsDisjoint, set))
            }
            "union" => Ok(self.alloc_native_bound_method(NativeMethodKind::SetUnion, set)),
            "intersection" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetIntersection, set))
            }
            "difference" => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetDifference, set))
            }
            "add" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetAdd, set))
            }
            "discard" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetDiscard, set))
            }
            "remove" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetRemove, set))
            }
            "pop" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetPop, set))
            }
            "update" if !is_frozenset => {
                Ok(self.alloc_native_bound_method(NativeMethodKind::SetUpdate, set))
            }
            _ => Err(RuntimeError::new(format!(
                "{type_name} has no attribute '{attr_name}'"
            ))),
        }
    }

    pub(super) fn load_attr_dict_method(
        &self,
        dict: ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        self.load_attr_dict_method_with_owner(dict, None, attr_name)
    }

    pub(super) fn load_attr_dict_method_with_owner(
        &self,
        dict: ObjRef,
        owner: Option<Value>,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        if attr_name == "__len__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Len, dict));
        }
        if attr_name == "__contains__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::OperatorContains, dict));
        }
        if attr_name == "__iter__" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Iter, dict));
        }
        if attr_name == "__reduce_ex__" || attr_name == "__reduce__" {
            return Ok(self.alloc_reduce_ex_bound_method(Value::Dict(dict)));
        }
        let is_contextvar_storage =
            dict_get_value(&dict, &Value::Str("__pyrs_contextvar__".to_string()))
                .is_some_and(|value| matches!(value, Value::Bool(true)));
        if is_contextvar_storage {
            let contextvar_kind = match attr_name {
                "get" => Some(NativeMethodKind::ContextVarGetMethod),
                "set" => Some(NativeMethodKind::ContextVarSetMethod),
                "reset" => Some(NativeMethodKind::ContextVarResetMethod),
                _ => None,
            };
            if let Some(kind) = contextvar_kind {
                return Ok(self.alloc_native_bound_method(kind, dict));
            }
        }
        let kind = match attr_name {
            "__init__" => NativeMethodKind::DictInit,
            "keys" => NativeMethodKind::DictKeys,
            "values" => NativeMethodKind::DictValues,
            "items" => NativeMethodKind::DictItems,
            "clear" => NativeMethodKind::DictClear,
            "copy" => NativeMethodKind::DictCopy,
            "update" => NativeMethodKind::DictUpdateMethod,
            "setdefault" => NativeMethodKind::DictSetDefault,
            "get" => NativeMethodKind::DictGet,
            "__getitem__" => NativeMethodKind::DictGetItem,
            "__setitem__" => NativeMethodKind::DictSetItem,
            "__delitem__" => NativeMethodKind::DictDelItem,
            "pop" => NativeMethodKind::DictPop,
            _ => {
                if attr_name == "_member_names"
                    && std::env::var_os("PYRS_TRACE_ENUM_MEMBER_NAMES").is_some()
                {
                    eprintln!("[enum-member-names] attr lookup on dict");
                    for frame in self.frames.iter().rev().take(12) {
                        let location = frame.code.locations.get(frame.last_ip);
                        eprintln!(
                            "  fn={} file={} line={} col={} ip={}",
                            frame.code.name,
                            frame.code.filename,
                            location.map(|loc| loc.line).unwrap_or(0),
                            location.map(|loc| loc.column).unwrap_or(0),
                            frame.last_ip
                        );
                    }
                }
                return Err(RuntimeError::attribute_error(format!(
                    "dict has no attribute '{}'",
                    attr_name
                )));
            }
        };
        if matches!(kind, NativeMethodKind::DictGetItem) && owner.is_some() {
            let receiver = match self
                .heap
                .alloc_module(ModuleObject::new("__dict_method__".to_string()))
            {
                Value::Module(obj) => obj,
                _ => unreachable!(),
            };
            if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                module_data
                    .globals
                    .insert("dict".to_string(), Value::Dict(dict));
                if let Some(owner) = owner {
                    module_data.globals.insert("owner".to_string(), owner);
                }
            }
            Ok(self.alloc_native_bound_method(kind, receiver))
        } else {
            Ok(self.alloc_native_bound_method(kind, dict))
        }
    }

    pub(super) fn dict_lookup_str_key(
        &self,
        dict: &ObjRef,
        key: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let string_key = Value::Str(key.to_string());
        if !matches!(&*dict.kind(), Object::Dict(_)) {
            return Err(RuntimeError::new("function attribute dict is invalid"));
        }
        Ok(dict_get_value(dict, &string_key))
    }

    pub(super) fn dict_set_str_key(
        &self,
        dict: &ObjRef,
        key: String,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if !matches!(&*dict.kind(), Object::Dict(_)) {
            return Err(RuntimeError::new("function attribute dict is invalid"));
        }
        dict_set_value(dict, Value::Str(key), value);
        Ok(())
    }

    pub(super) fn dict_remove_str_key(
        &self,
        dict: &ObjRef,
        key: &str,
    ) -> Result<bool, RuntimeError> {
        if !matches!(&*dict.kind(), Object::Dict(_)) {
            return Err(RuntimeError::new("function attribute dict is invalid"));
        }
        Ok(dict_remove_value(dict, &Value::Str(key.to_string())).is_some())
    }

    pub(super) fn ensure_function_annotations(
        &mut self,
        func: &ObjRef,
    ) -> Result<ObjRef, RuntimeError> {
        let (existing_annotations, function_dict, future_annotations_import) = {
            let func_ref = func.kind();
            let Object::Function(func_data) = &*func_ref else {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            };
            (
                func_data.annotations.clone(),
                func_data.dict.clone(),
                func_data.code.future_annotations_import,
            )
        };
        let annotations_need_resolution = existing_annotations
            .as_ref()
            .map(|dict| {
                if let Object::Dict(entries) = &*dict.kind() {
                    entries
                        .iter()
                        .any(|(_key, value)| matches!(value, Value::Str(_)))
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if let Some(obj) = existing_annotations.clone()
            && (!annotations_need_resolution || future_annotations_import)
        {
            return Ok(obj);
        }

        let annotate_from_dict = function_dict
            .as_ref()
            .and_then(|dict| dict_get_value(dict, &Value::Str("__annotate__".to_string())))
            .filter(|value| !matches!(value, Value::None))
            .filter(|value| self.is_callable_value(value));
        let annotate_callable = annotate_from_dict.or_else(|| {
            self.load_attr_function(func, "__annotate__")
                .ok()
                .filter(|value| !matches!(value, Value::None))
                .filter(|value| self.is_callable_value(value))
        });
        if (existing_annotations.is_none() || annotations_need_resolution)
            && let Some(annotate_callable) = annotate_callable
        {
            match self.call_internal(annotate_callable, vec![Value::Int(1)], HashMap::new())? {
                InternalCallOutcome::Value(Value::Dict(dict)) => {
                    let mut func_ref = func.kind_mut();
                    let Object::Function(func_data) = &mut *func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    func_data.annotations = Some(dict.clone());
                    return Ok(dict);
                }
                InternalCallOutcome::Value(other) => {
                    return Err(RuntimeError::type_error(format!(
                        "__annotate__ returned non-dict of type '{}'",
                        self.value_type_name_for_error(&other)
                    )));
                }
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(
                        self.runtime_error_from_active_exception("function.__annotate__ failed")
                    );
                }
            }
        }
        if let Some(obj) = existing_annotations {
            return Ok(obj);
        }

        let mut func_ref = func.kind_mut();
        let Object::Function(func_data) = &mut *func_ref else {
            return Err(RuntimeError::attribute_error(
                "attribute access unsupported type",
            ));
        };
        let dict = self.heap.alloc_dict(Vec::new());
        let obj = match dict {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };
        func_data.annotations = Some(obj.clone());
        Ok(obj)
    }

    pub(super) fn ensure_function_dict(&mut self, func: &ObjRef) -> Result<ObjRef, RuntimeError> {
        let mut func_ref = func.kind_mut();
        let Object::Function(func_data) = &mut *func_ref else {
            return Err(RuntimeError::attribute_error(
                "attribute access unsupported type",
            ));
        };
        if let Some(obj) = &func_data.dict {
            return Ok(obj.clone());
        }
        let dict = self.heap.alloc_dict(Vec::new());
        let obj = match dict {
            Value::Dict(obj) => obj,
            _ => unreachable!(),
        };
        func_data.dict = Some(obj.clone());
        Ok(obj)
    }

    pub(super) fn function_module_name(&self, module: &ObjRef) -> String {
        match &*module.kind() {
            Object::Module(module_data) => module_data
                .globals
                .get("__name__")
                .and_then(|value| match value {
                    Value::Str(name) => Some(name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| module_data.name.clone()),
            _ => "__main__".to_string(),
        }
    }

    pub(super) fn load_attr_function(
        &mut self,
        func: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let function_dict = {
            let func_ref = func.kind();
            let Object::Function(func_data) = &*func_ref else {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            };
            func_data.dict.clone()
        };
        if let Some(dict) = &function_dict
            && let Some(value) = self.dict_lookup_str_key(dict, attr_name)?
        {
            return Ok(value);
        }

        match attr_name {
            "__annotations__" => Ok(Value::Dict(self.ensure_function_annotations(func)?)),
            "__dict__" => Ok(Value::Dict(self.ensure_function_dict(func)?)),
            "__name__" => {
                let name = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    func_data.code.name.clone()
                };
                Ok(Value::Str(name))
            }
            "__qualname__" => {
                let qualname = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    let base_name = func_data.code.name.clone();
                    if let Some(owner_class) = &func_data.owner_class {
                        if let Object::Class(class_data) = &*owner_class.kind() {
                            let owner_qualname = class_data
                                .attrs
                                .get("__qualname__")
                                .and_then(|value| match value {
                                    Value::Str(name) => Some(name.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| class_data.name.clone());
                            format!("{owner_qualname}.{base_name}")
                        } else {
                            base_name
                        }
                    } else {
                        base_name
                    }
                };
                Ok(Value::Str(qualname))
            }
            "__module__" => {
                let module_name = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    self.function_module_name(&func_data.module)
                };
                Ok(Value::Str(module_name))
            }
            "__code__" => {
                let code = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    func_data.code.clone()
                };
                Ok(Value::Code(code))
            }
            "__globals__" => {
                let module = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    func_data.module.clone()
                };
                if let Object::Module(module_data) = &*module.kind() {
                    let entries = module_data
                        .globals
                        .iter()
                        .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                        .collect::<Vec<_>>();
                    Ok(self.heap.alloc_dict(entries))
                } else {
                    Ok(self.heap.alloc_dict(Vec::new()))
                }
            }
            "__doc__" => {
                let doc = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    function_docstring_from_code(&func_data.code)
                };
                Ok(doc.unwrap_or(Value::None))
            }
            "__call__" => Ok(Value::Function(func.clone())),
            "__func__" => Ok(Value::Function(func.clone())),
            "__get__" => Ok(self
                .alloc_native_bound_method(NativeMethodKind::FunctionDescriptorGet, func.clone())),
            "__annotate__" => {
                let has_annotations = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    func_data
                        .annotations
                        .as_ref()
                        .and_then(|annotations| match &*annotations.kind() {
                            Object::Dict(entries) => Some(!entries.is_empty()),
                            _ => None,
                        })
                        .unwrap_or(false)
                };
                if !has_annotations {
                    return Ok(Value::None);
                }
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__function_annotate__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("function".to_string(), Value::Function(func.clone()));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::FunctionAnnotate, receiver))
            }
            "__defaults__" => {
                let defaults = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    func_data.defaults.clone()
                };
                if defaults.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(self.heap.alloc_tuple(defaults))
                }
            }
            "__kwdefaults__" => {
                let entries = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    if func_data.kwonly_defaults.is_empty() {
                        return Ok(Value::None);
                    }
                    func_data
                        .kwonly_defaults
                        .iter()
                        .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                        .collect::<Vec<_>>()
                };
                Ok(self.heap.alloc_dict(entries))
            }
            "__closure__" => {
                let closure = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    func_data.closure.clone()
                };
                if closure.is_empty() {
                    Ok(Value::None)
                } else {
                    let values = closure.into_iter().map(Value::Cell).collect::<Vec<_>>();
                    Ok(self.heap.alloc_tuple(values))
                }
            }
            "__type_params__" => Ok(self.heap.alloc_tuple(Vec::new())),
            "__builtins__" => {
                let module_builtins = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::attribute_error(
                            "attribute access unsupported type",
                        ));
                    };
                    match &*func_data.module.kind() {
                        Object::Module(module_data) => {
                            module_data.globals.get("__builtins__").cloned()
                        }
                        _ => None,
                    }
                };
                let resolved = match module_builtins {
                    Some(Value::Dict(dict)) => Value::Dict(dict),
                    Some(Value::Module(module)) => {
                        if let Object::Module(module_data) = &*module.kind() {
                            let entries = module_data
                                .globals
                                .iter()
                                .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                                .collect::<Vec<_>>();
                            self.heap.alloc_dict(entries)
                        } else {
                            Value::None
                        }
                    }
                    Some(other) => other,
                    None => {
                        let entries = self
                            .builtins
                            .iter()
                            .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                            .collect::<Vec<_>>();
                        self.heap.alloc_dict(entries)
                    }
                };
                Ok(resolved)
            }
            _ => Err(RuntimeError::attribute_error(format!(
                "function has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_bound_method(
        &mut self,
        method: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let (function, receiver) = {
            let method_ref = method.kind();
            let Object::BoundMethod(method_data) = &*method_ref else {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            };
            (method_data.function.clone(), method_data.receiver.clone())
        };
        enum BoundFunctionKind {
            Function,
            Module,
            Class,
            NativeMethod(NativeMethodKind),
            Unsupported,
        }
        let function_kind = {
            let function_ref = function.kind();
            match &*function_ref {
                Object::Function(_) => BoundFunctionKind::Function,
                Object::Module(_) => BoundFunctionKind::Module,
                Object::Class(_) => BoundFunctionKind::Class,
                Object::NativeMethod(native_method) => {
                    BoundFunctionKind::NativeMethod(native_method.kind)
                }
                _ => BoundFunctionKind::Unsupported,
            }
        };
        let as_value = |kind: &BoundFunctionKind, obj: &ObjRef| match kind {
            BoundFunctionKind::Function => Some(Value::Function(obj.clone())),
            BoundFunctionKind::Module => Some(Value::Module(obj.clone())),
            BoundFunctionKind::Class => Some(Value::Class(obj.clone())),
            BoundFunctionKind::NativeMethod(_) => None,
            BoundFunctionKind::Unsupported => None,
        };
        if matches!(
            attr_name,
            "__name__"
                | "__qualname__"
                | "__module__"
                | "__doc__"
                | "__annotate__"
                | "__type_params__"
        ) && let Some(overrides) = self.callable_attr_overrides.get(&function.id())
            && let Some(value) = overrides.get(attr_name)
        {
            return Ok(value.clone());
        }
        let native_method_default_name = |kind: &NativeMethodKind| -> String {
            match kind {
                NativeMethodKind::ExtensionFunctionCall(function_id) => self
                    .extension_callable_registry
                    .get(function_id)
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| "method".to_string()),
                _ => "method".to_string(),
            }
        };
        let native_method_default_module = |kind: &NativeMethodKind| -> Value {
            if let NativeMethodKind::ExtensionFunctionCall(function_id) = kind
                && let Some(entry) = self.extension_callable_registry.get(function_id)
                && let Object::Module(module_data) = &*entry.module.kind()
            {
                return Value::Str(module_data.name.clone());
            }
            match self.receiver_value(&receiver) {
                Ok(Value::Module(module)) => {
                    if let Object::Module(module_data) = &*module.kind() {
                        Value::Str(module_data.name.clone())
                    } else {
                        Value::None
                    }
                }
                _ => Value::None,
            }
        };
        let receiver_is_unbound_method_descriptor = matches!(
            &*receiver.kind(),
            Object::Module(module_data) if module_data.name.ends_with("_unbound_method__")
        );
        match attr_name {
            "__call__" => Ok(Value::BoundMethod(method.clone())),
            "__get__" if receiver_is_unbound_method_descriptor => Ok(self
                .alloc_native_bound_method(
                    NativeMethodKind::BoundMethodDescriptorGet,
                    method.clone(),
                )),
            "__reduce_ex__" | "__reduce__" => {
                let wrapper = match self
                    .heap
                    .alloc_module(ModuleObject::new("__bound_method_reduce_ex__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *wrapper.kind_mut() {
                    module_data
                        .globals
                        .insert("method".to_string(), Value::BoundMethod(method.clone()));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::BoundMethodReduceEx, wrapper))
            }
            "__self__" => self.receiver_value(&receiver),
            "__func__" => as_value(&function_kind, &function)
                .ok_or_else(|| RuntimeError::attribute_error("attribute access unsupported type")),
            "__name__" | "__qualname__" | "__module__" | "__doc__" | "__annotate__"
            | "__type_params__" | "__builtins__" | "__globals__" => {
                if let BoundFunctionKind::NativeMethod(kind) = &function_kind {
                    if matches!(
                        attr_name,
                        "__annotate__" | "__type_params__" | "__builtins__" | "__globals__"
                    ) {
                        return Err(RuntimeError::attribute_error(format!(
                            "method has no attribute '{}'",
                            attr_name
                        )));
                    }
                    return match attr_name {
                        "__name__" | "__qualname__" => {
                            Ok(Value::Str(native_method_default_name(kind)))
                        }
                        "__module__" => Ok(native_method_default_module(kind)),
                        "__doc__" => Ok(Value::None),
                        _ => unreachable!(),
                    };
                }
                let function_value = as_value(&function_kind, &function).ok_or_else(|| {
                    RuntimeError::attribute_error("attribute access unsupported type")
                })?;
                self.builtin_getattr(
                    vec![function_value, Value::Str(attr_name.to_string())],
                    HashMap::new(),
                )
            }
            _ => Err(RuntimeError::attribute_error(format!(
                "method has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn store_attr_bound_method(
        &mut self,
        method: &ObjRef,
        attr_name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let function = {
            let method_ref = method.kind();
            let Object::BoundMethod(method_data) = &*method_ref else {
                return Err(RuntimeError::type_error(
                    "attribute assignment unsupported type",
                ));
            };
            method_data.function.clone()
        };
        match attr_name {
            "__name__" | "__qualname__" | "__module__" | "__doc__" | "__annotate__"
            | "__type_params__" => {}
            _ => {
                return Err(RuntimeError::attribute_error(format!(
                    "method has no writable attribute '{}'",
                    attr_name
                )));
            }
        }
        let function_kind = function.kind();
        match &*function_kind {
            Object::Function(_) => {
                self.store_attr_function(&function, attr_name.to_string(), value)
            }
            Object::NativeMethod(_) => {
                self.callable_attr_overrides
                    .entry(function.id())
                    .or_default()
                    .insert(attr_name.to_string(), value);
                Ok(())
            }
            _ => Err(RuntimeError::type_error(
                "attribute assignment unsupported type",
            )),
        }
    }

    pub(super) fn load_attr_exception_type(
        &self,
        exception_name: &str,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        match attr_name {
            "__name__" | "__qualname__" => Ok(Value::Str(exception_name.to_string())),
            "__init__" => Ok(Value::Builtin(BuiltinFunction::ExceptionTypeInit)),
            "__module__" => {
                let module_name = if exception_name == "Error" {
                    "_csv"
                } else if matches!(
                    exception_name,
                    "PickleError" | "PicklingError" | "UnpicklingError"
                ) {
                    "_pickle"
                } else {
                    "builtins"
                };
                Ok(Value::Str(module_name.to_string()))
            }
            "__doc__" => Ok(Value::None),
            _ => Err(RuntimeError::attribute_error(format!(
                "exception type has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_code(
        &self,
        code: &Rc<CodeObject>,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let mut varnames = Vec::new();
        varnames.extend(code.posonly_params.iter().cloned());
        varnames.extend(code.params.iter().cloned());
        varnames.extend(code.kwonly_params.iter().cloned());
        if let Some(vararg) = &code.vararg {
            varnames.push(vararg.clone());
        }
        if let Some(kwarg) = &code.kwarg {
            varnames.push(kwarg.clone());
        }

        let mut flags = 0x0001 | 0x0002;
        if code.vararg.is_some() {
            flags |= 0x0004;
        }
        if code.kwarg.is_some() {
            flags |= 0x0008;
        }
        if code.is_generator {
            flags |= 0x0020;
        }
        if code.is_coroutine {
            flags |= 0x0080;
        }
        if code.is_iterable_coroutine {
            flags |= 0x0100;
        }
        if code.is_async_generator {
            flags |= 0x0200;
        }

        let first_line = code.first_line.max(1) as i64;

        match attr_name {
            "replace" => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__code_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("value".to_string(), Value::Code(code.clone()));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::CodeReplace, receiver))
            }
            "co_positions" => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__code_positions_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("value".to_string(), Value::Code(code.clone()));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::CodeCoPositions, receiver))
            }
            "co_lines" => {
                let receiver = match self
                    .heap
                    .alloc_module(ModuleObject::new("__code_lines_method__".to_string()))
                {
                    Value::Module(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Module(module_data) = &mut *receiver.kind_mut() {
                    module_data
                        .globals
                        .insert("value".to_string(), Value::Code(code.clone()));
                }
                Ok(self.alloc_native_bound_method(NativeMethodKind::CodeCoLines, receiver))
            }
            "co_name" | "co_qualname" => Ok(Value::Str(code.name.clone())),
            "co_filename" => Ok(Value::Str(code.filename.clone())),
            "co_argcount" => Ok(Value::Int(
                (code.posonly_params.len() + code.params.len()) as i64,
            )),
            "co_posonlyargcount" => Ok(Value::Int(code.posonly_params.len() as i64)),
            "co_kwonlyargcount" => Ok(Value::Int(code.kwonly_params.len() as i64)),
            "co_nlocals" => Ok(Value::Int(varnames.len() as i64)),
            "co_stacksize" => Ok(Value::Int(0)),
            "co_flags" => Ok(Value::Int(flags)),
            "co_firstlineno" => Ok(Value::Int(first_line)),
            "co_consts" => Ok(self.heap.alloc_tuple(code.constants.clone())),
            "co_names" => Ok(self.heap.alloc_tuple(
                code.names
                    .iter()
                    .map(|name| Value::Str(name.clone()))
                    .collect::<Vec<_>>(),
            )),
            "co_varnames" => Ok(self
                .heap
                .alloc_tuple(varnames.into_iter().map(Value::Str).collect::<Vec<_>>())),
            "co_cellvars" => Ok(self.heap.alloc_tuple(
                code.cellvars
                    .iter()
                    .map(|name| Value::Str(name.clone()))
                    .collect::<Vec<_>>(),
            )),
            "co_freevars" => Ok(self.heap.alloc_tuple(
                code.freevars
                    .iter()
                    .map(|name| Value::Str(name.clone()))
                    .collect::<Vec<_>>(),
            )),
            "co_code" | "co_lnotab" | "co_exceptiontable" => Ok(self.heap.alloc_bytes(Vec::new())),
            _ => Err(RuntimeError::attribute_error(format!(
                "code has no attribute '{}'",
                attr_name
            ))),
        }
    }

    pub(super) fn load_attr_generator_property(
        &self,
        generator: &ObjRef,
        attr_name: &str,
    ) -> Option<Value> {
        let suspended = |started: bool, running: bool, closed: bool| started && !running && !closed;
        match &*generator.kind() {
            Object::Generator(state) if state.is_async_generator => match attr_name {
                "__name__" | "__qualname__" => Some(Value::Str(state.code.name.clone())),
                "ag_code" => Some(Value::Code(state.code.clone())),
                "ag_running" => Some(Value::Bool(state.running)),
                "ag_frame" => Some(Value::None),
                "ag_await" => Some(Value::None),
                _ => None,
            },
            Object::Generator(state) if state.is_coroutine => match attr_name {
                "__name__" | "__qualname__" => Some(Value::Str(state.code.name.clone())),
                "cr_code" => Some(Value::Code(state.code.clone())),
                "cr_running" => Some(Value::Bool(state.running)),
                "cr_frame" => Some(Value::None),
                "cr_await" => Some(Value::None),
                "cr_suspended" => Some(Value::Bool(suspended(
                    state.started,
                    state.running,
                    state.closed,
                ))),
                _ => None,
            },
            Object::Generator(state) => match attr_name {
                "__name__" | "__qualname__" => Some(Value::Str(state.code.name.clone())),
                "gi_code" => Some(Value::Code(state.code.clone())),
                "gi_running" => Some(Value::Bool(state.running)),
                "gi_frame" => Some(Value::None),
                "gi_yieldfrom" => Some(Value::None),
                "gi_suspended" => Some(Value::Bool(suspended(
                    state.started,
                    state.running,
                    state.closed,
                ))),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn store_attr_function(
        &mut self,
        func: &ObjRef,
        attr_name: String,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match attr_name.as_str() {
            "__annotations__" => {
                let annotations = match value {
                    Value::Dict(obj) => obj,
                    _ => return Err(RuntimeError::new("function __annotations__ must be dict")),
                };
                let mut func_ref = func.kind_mut();
                let Object::Function(func_data) = &mut *func_ref else {
                    return Err(RuntimeError::type_error(
                        "attribute assignment unsupported type",
                    ));
                };
                func_data.annotations = Some(annotations);
                Ok(())
            }
            "__dict__" => {
                let dict = match value {
                    Value::Dict(obj) => obj,
                    _ => return Err(RuntimeError::new("function __dict__ must be dict")),
                };
                let mut func_ref = func.kind_mut();
                let Object::Function(func_data) = &mut *func_ref else {
                    return Err(RuntimeError::type_error(
                        "attribute assignment unsupported type",
                    ));
                };
                func_data.dict = Some(dict);
                Ok(())
            }
            "__code__" => {
                let code = match value {
                    Value::Code(code) => code,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "__code__ must be set to a code object",
                        ));
                    }
                };
                let function_dict = {
                    let mut func_ref = func.kind_mut();
                    let Object::Function(func_data) = &mut *func_ref else {
                        return Err(RuntimeError::type_error(
                            "attribute assignment unsupported type",
                        ));
                    };
                    func_data.code = code;
                    func_data.refresh_plain_positional_call_arity();
                    func_data.dict.clone()
                };
                if let Some(dict) = function_dict {
                    self.dict_remove_str_key(&dict, "__code__")?;
                }
                Ok(())
            }
            "__defaults__" => {
                let defaults = match value {
                    Value::None => Vec::new(),
                    Value::Tuple(tuple) => match &*tuple.kind() {
                        Object::Tuple(values) => values.clone(),
                        _ => {
                            return Err(RuntimeError::type_error(
                                "__defaults__ must be set to a tuple object",
                            ));
                        }
                    },
                    _ => {
                        return Err(RuntimeError::type_error(
                            "__defaults__ must be set to a tuple object",
                        ));
                    }
                };
                let function_dict = {
                    let mut func_ref = func.kind_mut();
                    let Object::Function(func_data) = &mut *func_ref else {
                        return Err(RuntimeError::type_error(
                            "attribute assignment unsupported type",
                        ));
                    };
                    func_data.defaults = defaults;
                    func_data.refresh_plain_positional_call_arity();
                    func_data.dict.clone()
                };
                if let Some(dict) = function_dict {
                    self.dict_remove_str_key(&dict, "__defaults__")?;
                }
                Ok(())
            }
            "__kwdefaults__" => {
                let kwonly_defaults = match value {
                    Value::None => HashMap::new(),
                    Value::Dict(dict_obj) => match &*dict_obj.kind() {
                        Object::Dict(entries) => {
                            let mut defaults = HashMap::with_capacity(entries.len());
                            for (key, entry_value) in entries {
                                let Value::Str(name) = key else {
                                    return Err(RuntimeError::type_error(
                                        "__kwdefaults__ dict keys must be strings",
                                    ));
                                };
                                defaults.insert(name.clone(), entry_value.clone());
                            }
                            defaults
                        }
                        _ => {
                            return Err(RuntimeError::type_error(
                                "__kwdefaults__ must be set to a dict object",
                            ));
                        }
                    },
                    _ => {
                        return Err(RuntimeError::type_error(
                            "__kwdefaults__ must be set to a dict object",
                        ));
                    }
                };
                let function_dict = {
                    let mut func_ref = func.kind_mut();
                    let Object::Function(func_data) = &mut *func_ref else {
                        return Err(RuntimeError::type_error(
                            "attribute assignment unsupported type",
                        ));
                    };
                    func_data.kwonly_defaults = kwonly_defaults;
                    func_data.refresh_plain_positional_call_arity();
                    func_data.dict.clone()
                };
                if let Some(dict) = function_dict {
                    self.dict_remove_str_key(&dict, "__kwdefaults__")?;
                }
                Ok(())
            }
            "__builtins__" => Err(RuntimeError::attribute_error("readonly attribute")),
            "__type_params__" => match value {
                Value::Tuple(_) => {
                    let dict = self.ensure_function_dict(func)?;
                    self.dict_set_str_key(&dict, attr_name, value)
                }
                _ => Err(RuntimeError::type_error(
                    "__type_params__ must be set to a tuple",
                )),
            },
            "__annotate__" => {
                if !matches!(value, Value::None) && !self.is_callable_value(&value) {
                    return Err(RuntimeError::type_error(
                        "__annotate__ must be callable or None",
                    ));
                }
                let dict = self.ensure_function_dict(func)?;
                self.dict_set_str_key(&dict, attr_name, value)
            }
            _ => {
                let dict = self.ensure_function_dict(func)?;
                self.dict_set_str_key(&dict, attr_name, value)
            }
        }
    }

    pub(super) fn store_attr_exception(
        &mut self,
        exception: &mut ExceptionObject,
        attr_name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match attr_name {
            "__cause__" => match value {
                Value::None => {
                    exception.cause = None;
                    Ok(())
                }
                Value::Exception(cause) => {
                    exception.cause = Some(cause);
                    Ok(())
                }
                _ => Err(RuntimeError::new("__cause__ must be an exception or None")),
            },
            "__context__" => match value {
                Value::None => {
                    exception.context = None;
                    Ok(())
                }
                Value::Exception(context) => {
                    exception.context = Some(context);
                    Ok(())
                }
                _ => Err(RuntimeError::new(
                    "__context__ must be an exception or None",
                )),
            },
            "__suppress_context__" => match value {
                Value::Bool(flag) => {
                    exception.suppress_context = flag;
                    Ok(())
                }
                _ => Err(RuntimeError::new(
                    "__suppress_context__ must be set to bool",
                )),
            },
            "__traceback__" => {
                let frames = self.traceback_frames_from_value(value.clone())?;
                exception.traceback_frames = frames.unwrap_or_default();
                exception
                    .attrs
                    .borrow_mut()
                    .insert("__traceback__".to_string(), value);
                Ok(())
            }
            _ => {
                exception
                    .attrs
                    .borrow_mut()
                    .insert(attr_name.to_string(), value);
                Ok(())
            }
        }
    }

    pub(super) fn delete_attr_exception(
        &mut self,
        exception: &ExceptionObject,
        attr_name: &str,
    ) -> Result<(), RuntimeError> {
        match attr_name {
            "__cause__" | "__context__" | "__suppress_context__" | "__traceback__" => Err(
                RuntimeError::new(format!("cannot delete exception attribute '{}'", attr_name)),
            ),
            _ => {
                if exception.attrs.borrow_mut().remove(attr_name).is_some() {
                    Ok(())
                } else {
                    Err(RuntimeError::attribute_error(format!(
                        "exception has no attribute '{}'",
                        attr_name
                    )))
                }
            }
        }
    }

    pub(super) fn delete_attr_function(
        &mut self,
        func: &ObjRef,
        attr_name: &str,
    ) -> Result<(), RuntimeError> {
        match attr_name {
            "__annotations__" => {
                let mut func_ref = func.kind_mut();
                let Object::Function(func_data) = &mut *func_ref else {
                    return Err(RuntimeError::type_error(
                        "attribute deletion unsupported type",
                    ));
                };
                if func_data.annotations.take().is_none() {
                    return Err(RuntimeError::new(format!(
                        "function attribute '{}' does not exist",
                        attr_name
                    )));
                }
                Ok(())
            }
            "__dict__" => Err(RuntimeError::new(
                "cannot delete function attribute '__dict__'",
            )),
            _ => {
                let dict = {
                    let func_ref = func.kind();
                    let Object::Function(func_data) = &*func_ref else {
                        return Err(RuntimeError::type_error(
                            "attribute deletion unsupported type",
                        ));
                    };
                    func_data.dict.clone()
                };
                let Some(dict) = dict else {
                    return Err(RuntimeError::new(format!(
                        "function attribute '{}' does not exist",
                        attr_name
                    )));
                };
                if self.dict_remove_str_key(&dict, attr_name)? {
                    Ok(())
                } else {
                    Err(RuntimeError::new(format!(
                        "function attribute '{}' does not exist",
                        attr_name
                    )))
                }
            }
        }
    }

    pub(super) fn store_attr_cell(
        &self,
        cell: &ObjRef,
        attr_name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if attr_name != "cell_contents" {
            return Err(RuntimeError::attribute_error(format!(
                "cell has no attribute '{}'",
                attr_name
            )));
        }
        let mut cell_kind = cell.kind_mut();
        let Object::Cell(cell_data) = &mut *cell_kind else {
            return Err(RuntimeError::new("cell assignment receiver must be cell"));
        };
        cell_data.value = Some(value);
        Ok(())
    }

    pub(super) fn delete_attr_cell(
        &self,
        cell: &ObjRef,
        attr_name: &str,
    ) -> Result<(), RuntimeError> {
        if attr_name != "cell_contents" {
            return Err(RuntimeError::attribute_error(format!(
                "cell has no attribute '{}'",
                attr_name
            )));
        }
        let mut cell_kind = cell.kind_mut();
        let Object::Cell(cell_data) = &mut *cell_kind else {
            return Err(RuntimeError::new("cell deletion receiver must be cell"));
        };
        if cell_data.value.is_none() {
            return Err(RuntimeError::new("Cell is empty"));
        }
        cell_data.value = None;
        Ok(())
    }

    pub(super) fn bind_descriptor_method(
        &mut self,
        method: Value,
        receiver: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let owner_class = match receiver {
            Value::Class(class) => Some(class.clone()),
            _ => self.class_of_value(receiver),
        };
        if let Some(owner_class) = owner_class {
            if let Some(bound) = self.bind_classmethod_attr(&owner_class, &method) {
                return Ok(Some(bound));
            }
            if let Some(unwrapped) = self.unwrap_staticmethod_attr(&method) {
                return Ok(Some(unwrapped));
            }
        }
        match method {
            Value::Function(func) => {
                let receiver_ref = self.receiver_from_value(receiver)?;
                Ok(Some(
                    self.heap
                        .alloc_bound_method(BoundMethod::new(func, receiver_ref)),
                ))
            }
            Value::Builtin(builtin) => {
                let receiver_ref = self.receiver_from_value(receiver)?;
                Ok(Some(self.alloc_builtin_bound_method(builtin, receiver_ref)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn lookup_bound_special_method(
        &mut self,
        receiver: &Value,
        method_name: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(class_ref) = self.class_of_value(receiver) else {
            return Ok(None);
        };
        if let Value::Instance(instance) = receiver {
            let instance_special = match &*instance.kind() {
                Object::Instance(instance_data) => {
                    let is_unittest_mock = matches!(
                        &*instance_data.class.kind(),
                        Object::Class(class_data)
                            if matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module_name)) if module_name == "unittest.mock"
                            )
                    );
                    if !is_unittest_mock {
                        None
                    } else {
                        instance_data.attrs.get(method_name).cloned()
                    }
                }
                _ => None,
            };
            if let Some(method) = instance_special {
                if self.is_callable_value(&method) {
                    return Ok(Some(method));
                }
            }
        }
        if let Some(method) = class_attr_lookup(&class_ref, method_name) {
            return self.bind_descriptor_method(method, receiver);
        }
        Ok(None)
    }

    pub(super) fn descriptor_hooks(
        &mut self,
        descriptor: &Value,
    ) -> Result<(Option<Value>, Option<Value>, Option<Value>), RuntimeError> {
        if matches!(descriptor, Value::Function(_)) {
            return Ok((None, None, None));
        }

        if let Value::Instance(instance) = descriptor {
            if self.property_descriptor_parts(instance).is_some() {
                return Ok((
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::PropertyGet,
                        instance.clone(),
                    )),
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::PropertySet,
                        instance.clone(),
                    )),
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::PropertyDelete,
                        instance.clone(),
                    )),
                ));
            }
            if self.cached_property_descriptor_parts(instance).is_some() {
                return Ok((
                    Some(self.alloc_native_bound_method(
                        NativeMethodKind::CachedPropertyGet,
                        instance.clone(),
                    )),
                    None,
                    None,
                ));
            }
        }

        let Some(class_ref) = self.class_of_value(descriptor) else {
            return Ok((None, None, None));
        };
        let mut get = class_attr_lookup(&class_ref, "__get__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        let mut set = class_attr_lookup(&class_ref, "__set__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        let mut delete = class_attr_lookup(&class_ref, "__delete__")
            .map(|method| self.bind_descriptor_method(method, descriptor))
            .transpose()?
            .flatten();
        if (get.is_none() || set.is_none() || delete.is_none())
            && Vm::cpython_proxy_raw_ptr_from_value(descriptor).is_some()
        {
            if get.is_none()
                && let Some(method) = self.load_cpython_proxy_attr_for_value(descriptor, "__get__")
            {
                if let Some(bound) = self.bind_descriptor_method(method.clone(), descriptor)? {
                    get = Some(bound);
                } else {
                    get = Some(method);
                }
            }
            if set.is_none()
                && let Some(method) = self.load_cpython_proxy_attr_for_value(descriptor, "__set__")
            {
                if let Some(bound) = self.bind_descriptor_method(method.clone(), descriptor)? {
                    set = Some(bound);
                } else {
                    set = Some(method);
                }
            }
            if delete.is_none()
                && let Some(method) =
                    self.load_cpython_proxy_attr_for_value(descriptor, "__delete__")
            {
                if let Some(bound) = self.bind_descriptor_method(method.clone(), descriptor)? {
                    delete = Some(bound);
                } else {
                    delete = Some(method);
                }
            }
        }
        Ok((get, set, delete))
    }

    pub(super) fn unwrap_staticmethod_attr(&self, value: &Value) -> Option<Value> {
        let Value::Module(module) = value else {
            return None;
        };
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        if module_data.name != "__staticmethod__" {
            return None;
        }
        module_data.globals.get("__func__").cloned()
    }

    pub(super) fn unwrap_classmethod_attr(&self, value: &Value) -> Option<Value> {
        let Value::Module(module) = value else {
            return None;
        };
        let Object::Module(module_data) = &*module.kind() else {
            return None;
        };
        if module_data.name != "__classmethod__" {
            return None;
        }
        module_data.globals.get("__func__").cloned()
    }

    pub(super) fn bind_classmethod_attr(
        &self,
        owner_class: &ObjRef,
        value: &Value,
    ) -> Option<Value> {
        let unwrapped = self.unwrap_classmethod_attr(value)?;
        match unwrapped {
            Value::Function(func) => Some(
                self.heap
                    .alloc_bound_method(BoundMethod::new(func, owner_class.clone())),
            ),
            Value::Builtin(builtin) => {
                Some(self.alloc_builtin_bound_method(builtin, owner_class.clone()))
            }
            Value::Class(callable)
            | Value::Instance(callable)
            | Value::Module(callable)
            | Value::Generator(callable)
            | Value::List(callable)
            | Value::Tuple(callable)
            | Value::Dict(callable)
            | Value::Set(callable)
            | Value::FrozenSet(callable)
            | Value::Bytes(callable)
            | Value::ByteArray(callable)
            | Value::MemoryView(callable) => Some(
                self.heap
                    .alloc_bound_method(BoundMethod::new(callable, owner_class.clone())),
            ),
            _ => Some(unwrapped),
        }
    }

    pub(super) fn resolve_metaclass_call_target(
        &mut self,
        class: &ObjRef,
    ) -> Result<Option<Value>, RuntimeError> {
        let metaclass = match &*class.kind() {
            Object::Class(class_data) => class_data.metaclass.clone(),
            _ => None,
        };
        let Some(metaclass) = metaclass else {
            return Ok(None);
        };
        let Some(attr) = class_attr_lookup(&metaclass, "__call__") else {
            return Ok(None);
        };
        if let Some(bound) = self.bind_classmethod_attr(class, &attr) {
            return Ok(Some(bound));
        }
        if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
            return Ok(Some(unwrapped));
        }
        if let Value::Function(func) = attr.clone() {
            let bound = BoundMethod::new(func, class.clone());
            return Ok(Some(self.heap.alloc_bound_method(bound)));
        }
        if let Value::Builtin(builtin) = attr.clone() {
            return Ok(Some(
                self.alloc_builtin_bound_method(builtin, class.clone()),
            ));
        }
        let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
        if let Some(getter) = getter {
            return Ok(
                match self.call_internal(
                    getter,
                    vec![Value::Class(class.clone()), Value::Class(metaclass)],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => Some(value),
                    InternalCallOutcome::CallerExceptionHandled => None,
                },
            );
        }
        Ok(Some(attr))
    }

    pub(super) fn caller_exception_handled(&self, caller_depth: usize, caller_ip: usize) -> bool {
        if self.frames.len() < caller_depth {
            return true;
        }
        self.frames
            .get(caller_depth.saturating_sub(1))
            .map(|frame| frame.ip != caller_ip)
            .unwrap_or(false)
    }

    pub(super) fn finalize_builtin_opcode_call(
        &mut self,
        caller_depth: usize,
        caller_ip: usize,
        result: Result<Value, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        match result {
            Ok(value) => {
                if !self.caller_exception_handled(caller_depth, caller_ip) {
                    let caller_idx = caller_depth.saturating_sub(1);
                    self.push_value_to_caller_frame(caller_idx, value)?;
                }
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn finalize_native_opcode_call(
        &mut self,
        caller_depth: usize,
        caller_ip: usize,
        result: Result<NativeCallResult, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        match result {
            Ok(NativeCallResult::Value(value)) => {
                if !self.caller_exception_handled(caller_depth, caller_ip) {
                    let caller_idx = caller_depth.saturating_sub(1);
                    self.push_value_to_caller_frame(caller_idx, value)?;
                }
                Ok(())
            }
            Ok(NativeCallResult::PropagatedException) => {
                self.propagate_pending_generator_exception()?;
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn call_builtin_with_kwarg_order(
        &mut self,
        builtin: BuiltinFunction,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        kwargs_order: Option<Vec<String>>,
    ) -> Result<Value, RuntimeError> {
        match builtin {
            BuiltinFunction::Dict => self.builtin_dict_with_order(args, kwargs, kwargs_order),
            BuiltinFunction::CollectionsOrderedDict => {
                self.builtin_collections_ordereddict_with_order(args, kwargs, kwargs_order)
            }
            BuiltinFunction::SimpleNamespaceInit => {
                self.builtin_types_simplenamespace_init_with_order(args, kwargs, kwargs_order)
            }
            _ => self.call_builtin(builtin, args, kwargs),
        }
    }

    pub(super) fn call_internal(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<InternalCallOutcome, RuntimeError> {
        self.call_internal_with_kwarg_order(callable, args, kwargs, None)
    }

    #[inline]
    fn active_exception_fingerprint(
        value: Option<&Value>,
    ) -> Option<(u64, usize, Option<u64>, Option<u64>, usize)> {
        match value {
            Some(Value::Exception(exception)) => Some((
                exception.object_id,
                exception.traceback_frames.len(),
                exception.context.as_ref().map(|context| context.object_id),
                exception.cause.as_ref().map(|cause| cause.object_id),
                exception.notes.len(),
            )),
            Some(_) => Some((u64::MAX, 0, None, None, 0)),
            None => None,
        }
    }

    pub(super) fn call_internal_with_kwarg_order(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        kwargs_order: Option<Vec<String>>,
    ) -> Result<InternalCallOutcome, RuntimeError> {
        if std::env::var_os("PYRS_TRACE_CALLABLE_NONE_BT").is_some()
            && matches!(callable, Value::None)
        {
            let args_summary = args.iter().map(format_repr).collect::<Vec<_>>().join(", ");
            let mut kw_entries = kwargs.iter().collect::<Vec<_>>();
            kw_entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let kwargs_summary = kw_entries
                .into_iter()
                .map(|(name, value)| format!("{name}={}", format_repr(value)))
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!(
                "[call-none-entry] positional=[{}] kwargs=[{}]\n{:?}",
                args_summary,
                kwargs_summary,
                std::backtrace::Backtrace::force_capture()
            );
        }
        let (call_depth_guard, call_depth) = CallInternalDepthGuard::enter();
        let _call_depth_guard = call_depth_guard;
        let caller_depth = self.frames.len();
        if caller_depth == 0 {
            return Err(RuntimeError::new(
                "internal call requires an active execution frame",
            ));
        }
        let hard_limit = std::env::var("PYRS_DEBUG_CALL_DEPTH_HARD_LIMIT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|limit| *limit > 0)
            .unwrap_or_else(|| self.recursion_limit.max(1) as usize * 4);
        if call_depth > hard_limit {
            return Err(self.recursion_limit_error());
        }
        self.ensure_can_push_python_frame()?;
        let (caller_ip, caller_active_exception_fingerprint) = self
            .frames
            .last()
            .map(|frame| {
                (
                    frame.ip,
                    Self::active_exception_fingerprint(frame.active_exception.as_ref()),
                )
            })
            .unwrap_or((0, None));
        let trace_class_call = std::env::var_os("PYRS_TRACE_CLASS_CALL_RUNTIME").is_some();
        let callable_was_class = matches!(&callable, Value::Class(_));
        let trace_callable_repr =
            (trace_class_call && callable_was_class).then(|| format_repr(&callable));

        let needs_run = match callable {
            Value::Function(func) => {
                let depth_before = self.frames.len();
                if kwargs.is_empty() {
                    let mut args = args;
                    match args.len() {
                        0 => self.push_function_call_from_obj(&func, Vec::new(), HashMap::new())?,
                        1 => {
                            let arg0 = args.pop().expect("len checked");
                            self.push_function_call_one_arg_from_obj(&func, arg0)?
                        }
                        2 => {
                            let arg1 = args.pop().expect("len checked");
                            let arg0 = args.pop().expect("len checked");
                            self.push_function_call_two_args_from_obj(&func, arg0, arg1)?
                        }
                        3 => {
                            let arg2 = args.pop().expect("len checked");
                            let arg1 = args.pop().expect("len checked");
                            let arg0 = args.pop().expect("len checked");
                            self.push_function_call_three_args_from_obj(&func, arg0, arg1, arg2)?
                        }
                        _ => {
                            self.push_function_call_from_obj(&func, args, HashMap::new())?;
                        }
                    }
                } else {
                    self.push_function_call_from_obj_with_kwarg_order(
                        &func,
                        args,
                        kwargs,
                        kwargs_order.clone(),
                    )?;
                }
                self.frames.len() > depth_before
            }
            Value::BoundMethod(method) => {
                let method_data = match &*method.kind() {
                    Object::BoundMethod(data) => data.clone(),
                    _ => return Err(RuntimeError::type_error("attempted to call non-function")),
                };
                match &*method_data.function.kind() {
                    Object::Function(_) => {
                        let depth_before = self.frames.len();
                        let mut bound_args = Vec::with_capacity(args.len() + 1);
                        bound_args.push(self.receiver_value(&method_data.receiver)?);
                        bound_args.extend(args);
                        self.push_function_call_from_obj_with_kwarg_order(
                            &method_data.function,
                            bound_args,
                            kwargs,
                            kwargs_order.clone(),
                        )?;
                        self.frames.len() > depth_before
                    }
                    Object::NativeMethod(native) => {
                        let native_call = self.call_native_method(
                            native.kind,
                            method_data.receiver.clone(),
                            args,
                            kwargs,
                        );
                        match native_call {
                            Ok(NativeCallResult::Value(result)) => {
                                return Ok(InternalCallOutcome::Value(result));
                            }
                            Ok(NativeCallResult::PropagatedException) => {
                                self.propagate_pending_generator_exception()?;
                                return Ok(InternalCallOutcome::CallerExceptionHandled);
                            }
                            Err(err) => {
                                if self.caller_exception_handled(caller_depth, caller_ip) {
                                    return Ok(InternalCallOutcome::CallerExceptionHandled);
                                }
                                return Err(err);
                            }
                        }
                    }
                    _ => {
                        let Some(callable) = value_from_object_ref(method_data.function.clone())
                        else {
                            return Err(RuntimeError::type_error("attempted to call non-function"));
                        };
                        let receiver_value = self.receiver_value(&method_data.receiver)?;
                        let callable_is_proxy =
                            Self::cpython_proxy_raw_ptr_from_value(&callable).is_some();
                        let receiver_is_proxy =
                            Self::cpython_proxy_raw_ptr_from_value(&receiver_value).is_some();
                        let callable_type_name = self.value_type_name_for_error(&callable);
                        if callable_is_proxy
                            && receiver_is_proxy
                            && callable_type_name == "cython_function_or_method"
                            && let Some(bound_callable) = self
                                .bind_cpython_proxy_descriptor_callable(
                                    &callable,
                                    &receiver_value,
                                )?
                        {
                            return self.call_internal_with_kwarg_order(
                                bound_callable,
                                args,
                                kwargs,
                                kwargs_order.clone(),
                            );
                        }
                        let proxy_callable_is_already_bound = callable_is_proxy
                            && receiver_is_proxy
                            && (matches!(
                                callable_type_name.as_str(),
                                "builtin_function_or_method" | "method"
                            ) || self.cpython_proxy_callable_has_bound_self(&callable));
                        if std::env::var_os("PYRS_TRACE_PROXY_BOUND_CALL").is_some() {
                            let receiver_tag = format_repr(&receiver_value);
                            let receiver_type = self.value_type_name_for_error(&receiver_value);
                            let receiver_ptr =
                                Vm::cpython_proxy_raw_ptr_from_value(&receiver_value)
                                    .map(|ptr| format!("{:p}", ptr))
                                    .unwrap_or_else(|| "<none>".to_string());
                            let callable_repr = format_repr(&callable);
                            let callable_ptr = Vm::cpython_proxy_raw_ptr_from_value(&callable)
                                .map(|ptr| format!("{:p}", ptr))
                                .unwrap_or_else(|| "<none>".to_string());
                            let callable_name = self
                                .load_cpython_proxy_attr_for_value(&callable, "__qualname__")
                                .map(|value| format_repr(&value))
                                .unwrap_or_else(|| "<missing>".to_string());
                            let callable_has_self =
                                self.cpython_proxy_callable_has_bound_self(&callable);
                            eprintln!(
                                "[proxy-bound-call] inline callable_type={} callable_is_proxy={} receiver_is_proxy={} already_bound={} args={} kwargs={} receiver_type={} receiver_ptr={} receiver_tag={} callable_ptr={} callable_repr={} callable_qualname={} callable_has_self={}",
                                callable_type_name,
                                callable_is_proxy,
                                receiver_is_proxy,
                                proxy_callable_is_already_bound,
                                args.len(),
                                kwargs.len(),
                                receiver_type,
                                receiver_ptr,
                                receiver_tag,
                                callable_ptr,
                                callable_repr,
                                callable_name,
                                callable_has_self
                            );
                        }
                        let call_args = if proxy_callable_is_already_bound {
                            args
                        } else {
                            let mut bound_args = Vec::with_capacity(args.len() + 1);
                            bound_args.push(receiver_value);
                            bound_args.extend(args);
                            bound_args
                        };
                        return self.call_internal_with_kwarg_order(
                            callable,
                            call_args,
                            kwargs,
                            kwargs_order.clone(),
                        );
                    }
                }
            }
            Value::Builtin(builtin) => {
                return match self.call_builtin_with_kwarg_order(builtin, args, kwargs, kwargs_order)
                {
                    Ok(result) => {
                        if self.caller_exception_handled(caller_depth, caller_ip) {
                            Ok(InternalCallOutcome::CallerExceptionHandled)
                        } else {
                            Ok(InternalCallOutcome::Value(result))
                        }
                    }
                    Err(err) => {
                        if self.caller_exception_handled(caller_depth, caller_ip) {
                            Ok(InternalCallOutcome::CallerExceptionHandled)
                        } else {
                            Err(err)
                        }
                    }
                };
            }
            Value::Instance(instance) => {
                let receiver = Value::Instance(instance.clone());
                if Self::cpython_proxy_raw_ptr_from_value(&receiver).is_some() {
                    let value = self.call_cpython_proxy_object(&receiver, args, kwargs)?;
                    return Ok(InternalCallOutcome::Value(value));
                }
                match self.load_attr_instance(&instance, "__call__") {
                    Ok(AttrAccessOutcome::Value(call_target)) => {
                        return self.call_internal_with_kwarg_order(
                            call_target,
                            args,
                            kwargs,
                            kwargs_order.clone(),
                        );
                    }
                    Ok(AttrAccessOutcome::ExceptionHandled) => {
                        return Ok(InternalCallOutcome::CallerExceptionHandled);
                    }
                    Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {}
                    Err(err) => return Err(err),
                }
                if let Some(call_target) =
                    self.lookup_bound_special_method(&receiver, "__call__")?
                {
                    return self.call_internal_with_kwarg_order(
                        call_target,
                        args,
                        kwargs,
                        kwargs_order.clone(),
                    );
                }
                return Err(RuntimeError::type_error("attempted to call non-function"));
            }
            Value::Class(class) => {
                let (class_name, metaclass_name) = match &*class.kind() {
                    Object::Class(class_data) => {
                        let metaclass_name = class_data.metaclass.as_ref().map(|meta| match &*meta
                            .kind()
                        {
                            Object::Class(meta_data) => meta_data.name.clone(),
                            _ => "<non-class-meta>".to_string(),
                        });
                        (class_data.name.clone(), metaclass_name)
                    }
                    _ => ("<non-class>".to_string(), None),
                };
                if trace_class_call {
                    eprintln!(
                        "[class-call] enter class={} metaclass={} args={} kwargs={}",
                        class_name,
                        metaclass_name.unwrap_or_else(|| "<none>".to_string()),
                        args.len(),
                        kwargs.len()
                    );
                }
                let proxy_value = Value::Class(class.clone());
                if Self::cpython_proxy_raw_ptr_from_value(&proxy_value).is_some() {
                    let value = self.call_cpython_proxy_object(&proxy_value, args, kwargs)?;
                    if trace_class_call {
                        eprintln!(
                            "[class-call] proxy-return class={} type={}",
                            class_name,
                            self.value_type_name_for_error(&value)
                        );
                    }
                    return Ok(InternalCallOutcome::Value(value));
                }
                if self.suppress_metaclass_dispatch_depth == 0 {
                    if let Some(call_target) = self.resolve_metaclass_call_target(&class)? {
                        if trace_class_call {
                            eprintln!(
                                "[class-call] metaclass-dispatch class={} target_type={} target_repr={}",
                                class_name,
                                self.value_type_name_for_error(&call_target),
                                format_repr(&call_target)
                            );
                        }
                        return self.call_internal_with_kwarg_order(
                            call_target,
                            args,
                            kwargs,
                            kwargs_order.clone(),
                        );
                    }
                }
                if let Some(message) = self.class_disallow_instantiation_message(&class) {
                    return Err(RuntimeError::type_error(message));
                }
                if self.class_is_exact_types_generic_alias(&class) {
                    let value =
                        self.instantiate_generic_alias_class(class.clone(), args, kwargs)?;
                    return Ok(InternalCallOutcome::Value(value));
                }
                if self.class_has_builtin_type_base(&class) {
                    let class_value =
                        self.instantiate_type_derived_class(class.clone(), args, kwargs)?;
                    self.push_value(class_value);
                    false
                } else {
                    let class_value = Value::Class(class.clone());
                    let mut instance = self.alloc_instance_for_class(&class);
                    let mut used_custom_new = false;
                    if let Some(raw_new_callable) = class_attr_lookup(&class, "__new__") {
                        let new_callable = self
                            .unwrap_staticmethod_attr(&raw_new_callable)
                            .unwrap_or(raw_new_callable);
                        if !matches!(new_callable, Value::Builtin(BuiltinFunction::ObjectNew)) {
                            if trace_class_call {
                                eprintln!(
                                    "[class-call] __new__ class={} callable_type={} callable_repr={}",
                                    class_name,
                                    self.value_type_name_for_error(&new_callable),
                                    format_repr(&new_callable)
                                );
                            }
                            used_custom_new = true;
                            let prepend_class_arg = !matches!(new_callable, Value::BoundMethod(_));
                            let mut new_args = Vec::with_capacity(
                                args.len() + if prepend_class_arg { 1 } else { 0 },
                            );
                            if prepend_class_arg {
                                new_args.push(class_value.clone());
                            }
                            new_args.extend(args.clone());
                            match self.call_internal_with_kwarg_order(
                                new_callable,
                                new_args,
                                kwargs.clone(),
                                kwargs_order.clone(),
                            )? {
                                InternalCallOutcome::Value(value) => {
                                    if !self.value_is_instance_of(&value, &class_value)? {
                                        if trace_class_call {
                                            eprintln!(
                                                "[class-call] __new__ non-instance class={} value_type={} value_repr={}",
                                                class_name,
                                                self.value_type_name_for_error(&value),
                                                format_repr(&value)
                                            );
                                        }
                                        return Ok(InternalCallOutcome::Value(value));
                                    }
                                    let Value::Instance(created_instance) = value else {
                                        if trace_class_call {
                                            eprintln!(
                                                "[class-call] __new__ non-instance-variant class={} value_type={}",
                                                class_name,
                                                self.value_type_name_for_error(&value)
                                            );
                                        }
                                        return Ok(InternalCallOutcome::Value(value));
                                    };
                                    instance = created_instance;
                                }
                                InternalCallOutcome::CallerExceptionHandled => {
                                    return Ok(InternalCallOutcome::CallerExceptionHandled);
                                }
                            }
                        }
                    }
                    let is_typing_paramspec_attr_class = match &*class.kind() {
                        Object::Class(class_data) => {
                            matches!(
                                class_data.name.as_str(),
                                "ParamSpecArgs" | "ParamSpecKwargs"
                            ) && matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module_name))
                                    if module_name == "typing" || module_name == "_typing"
                            )
                        }
                        _ => false,
                    };
                    if is_typing_paramspec_attr_class
                        && !used_custom_new
                        && kwargs.is_empty()
                        && args.len() == 1
                    {
                        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                            instance_data
                                .attrs
                                .insert("__origin__".to_string(), args[0].clone());
                        }
                        return Ok(InternalCallOutcome::Value(Value::Instance(instance)));
                    }
                    let mut init = class_attr_lookup(&class, "__init__");
                    if matches!(init, Some(Value::Builtin(BuiltinFunction::ObjectInit)))
                        && !used_custom_new
                        && (self.class_has_builtin_list_base(&class)
                            || self.class_has_builtin_tuple_base(&class)
                            || self.class_has_builtin_str_base(&class)
                            || self.class_has_builtin_bytes_base(&class)
                            || self.class_has_builtin_bytearray_base(&class)
                            || self.class_has_builtin_int_base(&class)
                            || self.class_has_builtin_float_base(&class)
                            || self.class_has_builtin_complex_base(&class)
                            || self.class_has_builtin_dict_base(&class)
                            || self.class_has_builtin_set_base(&class)
                            || self.class_has_builtin_frozenset_base(&class)
                            || self.class_has_builtin_property_base(&class))
                    {
                        // For builtin-backed subclasses we need the constructor fallback path
                        // below to hydrate backing storage from user-provided args/kwargs.
                        init = None;
                    }
                    if let Some(init_callable) = init {
                        if trace_class_call {
                            eprintln!(
                                "[class-call] __init__ class={} callable_type={} callable_repr={}",
                                class_name,
                                self.value_type_name_for_error(&init_callable),
                                format_repr(&init_callable)
                            );
                        }
                        if matches!(init_callable, Value::Builtin(BuiltinFunction::ObjectInit)) {
                            if used_custom_new {
                                if trace_class_call {
                                    eprintln!(
                                        "[class-call] return-used-custom-new class={} type=instance",
                                        class_name
                                    );
                                }
                                return Ok(InternalCallOutcome::Value(Value::Instance(instance)));
                            }
                            if let Some(fields) = self.class_namedtuple_fields(&class) {
                                self.bind_namedtuple_instance_fields(
                                    &instance,
                                    &fields,
                                    args.clone(),
                                    kwargs.clone(),
                                )?;
                                if trace_class_call {
                                    eprintln!(
                                        "[class-call] return-namedtuple-bind class={} type=instance",
                                        class_name
                                    );
                                }
                                return Ok(InternalCallOutcome::Value(Value::Instance(instance)));
                            }
                        }
                        if let Value::Function(init_func) = init_callable {
                            let func_data = match &*init_func.kind() {
                                Object::Function(data) => data.clone(),
                                _ => {
                                    return Err(RuntimeError::new(
                                        "attempted to call non-function",
                                    ));
                                }
                            };
                            let mut init_args = Vec::with_capacity(args.len() + 1);
                            init_args.push(Value::Instance(instance.clone()));
                            init_args.extend(args);
                            let bindings = match bind_arguments(
                                &func_data,
                                &self.heap,
                                init_args,
                                kwargs,
                                kwargs_order.clone(),
                            ) {
                                Ok(bindings) => bindings,
                                Err(err) => {
                                    if std::env::var_os("PYRS_TRACE_BIND_ARGS_STACK").is_some()
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
                                        if std::env::var_os("PYRS_TRACE_BIND_ARGS_BT").is_some() {
                                            eprintln!(
                                                "[bind-args-bt] failing_fn={} bt={}",
                                                func_data.code.name,
                                                std::backtrace::Backtrace::force_capture()
                                            );
                                        }
                                    }
                                    return Err(err);
                                }
                            };
                            let cells =
                                self.build_cells(&func_data.code, func_data.closure.clone());
                            let mut frame = Frame::new(
                                func_data.code.clone(),
                                func_data.module.clone(),
                                false,
                                false,
                                cells,
                                func_data.owner_class.clone(),
                            );
                            frame.active_exception = self
                                .frames
                                .last()
                                .and_then(|caller| caller.active_exception.as_ref())
                                .map(Self::clone_active_exception_for_call);
                            frame.return_instance = Some(instance);
                            frame.expect_none_return = true;
                            apply_bindings(&mut frame, &func_data.code, bindings, &self.heap);
                            let depth_before = self.frames.len();
                            self.push_frame_checked(Box::new(frame))?;
                            self.frames.len() > depth_before
                        } else {
                            let mut init_args = Vec::with_capacity(args.len() + 1);
                            init_args.push(Value::Instance(instance.clone()));
                            init_args.extend(args);
                            match self.call_internal_with_kwarg_order(
                                init_callable,
                                init_args,
                                kwargs,
                                kwargs_order.clone(),
                            )? {
                                InternalCallOutcome::Value(Value::None) => {
                                    if trace_class_call {
                                        eprintln!(
                                            "[class-call] return-init-none class={} type=instance",
                                            class_name
                                        );
                                    }
                                    self.push_value(Value::Instance(instance));
                                    false
                                }
                                InternalCallOutcome::Value(_) => {
                                    return Err(RuntimeError::new("__init__() should return None"));
                                }
                                InternalCallOutcome::CallerExceptionHandled => false,
                            }
                        }
                    } else {
                        if used_custom_new {
                            if trace_class_call {
                                eprintln!(
                                    "[class-call] return-custom-new-no-init class={} type=instance",
                                    class_name
                                );
                            }
                            self.push_value(Value::Instance(instance));
                            false
                        } else if let Some(fields) = self.class_namedtuple_fields(&class) {
                            self.bind_namedtuple_instance_fields(&instance, &fields, args, kwargs)?;
                            if trace_class_call {
                                eprintln!(
                                    "[class-call] return-namedtuple-no-init class={} type=instance",
                                    class_name
                                );
                            }
                            self.push_value(Value::Instance(instance));
                            false
                        } else {
                            if self.class_is_exception_class(&class) {
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        "args".to_string(),
                                        self.heap.alloc_tuple(args.clone()),
                                    );
                                    for (name, value) in kwargs {
                                        instance_data.attrs.insert(name, value);
                                    }
                                } else {
                                    return Err(RuntimeError::new(
                                        "exception instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_list_base(&class) {
                                let list_value =
                                    self.call_builtin(BuiltinFunction::List, args, kwargs)?;
                                let Value::List(_) = list_value else {
                                    return Err(RuntimeError::new(
                                        "list constructor returned non-list",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(LIST_BACKING_STORAGE_ATTR.to_string(), list_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "list instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_tuple_base(&class) {
                                let tuple_value =
                                    self.call_builtin(BuiltinFunction::Tuple, args, kwargs)?;
                                let Value::Tuple(_) = tuple_value else {
                                    return Err(RuntimeError::new(
                                        "tuple constructor returned non-tuple",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        TUPLE_BACKING_STORAGE_ATTR.to_string(),
                                        tuple_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "tuple instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_str_base(&class) {
                                let str_value =
                                    self.call_builtin(BuiltinFunction::Str, args, kwargs)?;
                                let Value::Str(_) = str_value else {
                                    return Err(RuntimeError::new(
                                        "str constructor returned non-str",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(STR_BACKING_STORAGE_ATTR.to_string(), str_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "str instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_bytes_base(&class) {
                                let bytes_value =
                                    self.call_builtin(BuiltinFunction::Bytes, args, kwargs)?;
                                let Value::Bytes(_) = bytes_value else {
                                    return Err(RuntimeError::new(
                                        "bytes constructor returned non-bytes",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        BYTES_BACKING_STORAGE_ATTR.to_string(),
                                        bytes_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "bytes instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_bytearray_base(&class) {
                                let bytearray_value =
                                    self.call_builtin(BuiltinFunction::ByteArray, args, kwargs)?;
                                let Value::ByteArray(_) = bytearray_value else {
                                    return Err(RuntimeError::new(
                                        "bytearray constructor returned non-bytearray",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        BYTES_BACKING_STORAGE_ATTR.to_string(),
                                        bytearray_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "bytearray instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_int_base(&class) {
                                let int_value = self.builtin_int(args, kwargs)?;
                                let (Value::Int(_) | Value::BigInt(_) | Value::Bool(_)) = int_value
                                else {
                                    return Err(RuntimeError::new(
                                        "int constructor returned non-int",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(INT_BACKING_STORAGE_ATTR.to_string(), int_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "int instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_float_base(&class) {
                                let float_value = self.builtin_float(args, kwargs)?;
                                let Value::Float(_) = float_value else {
                                    return Err(RuntimeError::new(
                                        "float constructor returned non-float",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        FLOAT_BACKING_STORAGE_ATTR.to_string(),
                                        float_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "float instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_complex_base(&class) {
                                let complex_value = self.builtin_complex(args, kwargs)?;
                                let Value::Complex { .. } = complex_value else {
                                    return Err(RuntimeError::new(
                                        "complex constructor returned non-complex",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        COMPLEX_BACKING_STORAGE_ATTR.to_string(),
                                        complex_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "complex instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_dict_base(&class) {
                                let dict_value =
                                    self.call_builtin(BuiltinFunction::Dict, args, kwargs)?;
                                let Value::Dict(_) = dict_value else {
                                    return Err(RuntimeError::new(
                                        "dict constructor returned non-dict",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(DICT_BACKING_STORAGE_ATTR.to_string(), dict_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "dict instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_set_base(&class) {
                                let set_value =
                                    self.call_builtin(BuiltinFunction::Set, args, kwargs)?;
                                let Value::Set(_) = set_value else {
                                    return Err(RuntimeError::new(
                                        "set constructor returned non-set",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data
                                        .attrs
                                        .insert(SET_BACKING_STORAGE_ATTR.to_string(), set_value);
                                } else {
                                    return Err(RuntimeError::new(
                                        "set instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_frozenset_base(&class) {
                                let frozenset_value =
                                    self.call_builtin(BuiltinFunction::FrozenSet, args, kwargs)?;
                                let Value::FrozenSet(_) = frozenset_value else {
                                    return Err(RuntimeError::new(
                                        "frozenset constructor returned non-frozenset",
                                    ));
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.insert(
                                        FROZENSET_BACKING_STORAGE_ATTR.to_string(),
                                        frozenset_value,
                                    );
                                } else {
                                    return Err(RuntimeError::new(
                                        "frozenset instance construction failed",
                                    ));
                                }
                            } else if self.class_has_builtin_property_base(&class) {
                                let descriptor_value =
                                    self.call_builtin(BuiltinFunction::Property, args, kwargs)?;
                                let Value::Instance(descriptor_instance) = descriptor_value else {
                                    return Err(RuntimeError::new(
                                        "property constructor returned non-property",
                                    ));
                                };
                                let descriptor_attrs = match &*descriptor_instance.kind() {
                                    Object::Instance(descriptor_data) => {
                                        descriptor_data.attrs.clone()
                                    }
                                    _ => {
                                        return Err(RuntimeError::new(
                                            "property descriptor construction failed",
                                        ));
                                    }
                                };
                                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                                    instance_data.attrs.extend(descriptor_attrs);
                                } else {
                                    return Err(RuntimeError::new(
                                        "property instance construction failed",
                                    ));
                                }
                            } else if !kwargs.is_empty() || !args.is_empty() {
                                if std::env::var_os("PYRS_TRACE_CLASS_CTOR_NOARGS").is_some() {
                                    let class_name = match &*class.kind() {
                                        Object::Class(data) => data.name.clone(),
                                        _ => "<non-class>".to_string(),
                                    };
                                    let (filename, function_name, line, column) = self
                                        .frames
                                        .last()
                                        .map(|frame| {
                                            let location = frame.code.locations.get(frame.last_ip);
                                            (
                                                frame.code.filename.clone(),
                                                frame.code.name.clone(),
                                                location.map(|loc| loc.line).unwrap_or(0),
                                                location.map(|loc| loc.column).unwrap_or(0),
                                            )
                                        })
                                        .unwrap_or_else(|| {
                                            (
                                                "<no-frame>".to_string(),
                                                "<no-function>".to_string(),
                                                0,
                                                0,
                                            )
                                        });
                                    eprintln!(
                                        "[class-ctor-noargs] class={} args_len={} kwargs_len={} at {}:{}:{} in {}",
                                        class_name,
                                        args.len(),
                                        kwargs.len(),
                                        filename,
                                        line,
                                        column,
                                        function_name
                                    );
                                }
                                return Err(RuntimeError::new(
                                    "class constructor takes no arguments",
                                ));
                            }
                            if trace_class_call {
                                eprintln!(
                                    "[class-call] return-default class={} type=instance",
                                    class_name
                                );
                            }
                            self.push_value(Value::Instance(instance));
                            false
                        }
                    }
                }
            }
            other => {
                if std::env::var_os("PYRS_TRACE_CALL_NON_FUNCTION").is_some() {
                    if let Some(frame) = self.frames.last() {
                        let location = frame.code.locations.get(frame.last_ip);
                        let opcode = frame
                            .code
                            .instructions
                            .get(frame.last_ip)
                            .map(|instr| format!("{:?}", instr.opcode))
                            .unwrap_or_else(|| "<unknown>".to_string());
                        eprintln!(
                            "[call-non-function] file={} func={} line={} col={} ip={} opcode={} value={}",
                            frame.code.filename,
                            frame.code.name,
                            location.map(|loc| loc.line).unwrap_or(0),
                            location.map(|loc| loc.column).unwrap_or(0),
                            frame.last_ip,
                            opcode,
                            format_repr(&other),
                        );
                        if std::env::var_os("PYRS_TRACE_CALL_NON_FUNCTION_ARGS").is_some() {
                            let args_summary =
                                args.iter().map(format_repr).collect::<Vec<_>>().join(", ");
                            let mut kw_entries = kwargs.iter().collect::<Vec<_>>();
                            kw_entries.sort_by(|(left, _), (right, _)| left.cmp(right));
                            let kwargs_summary = kw_entries
                                .into_iter()
                                .map(|(key, value)| format!("{key}={}", format_repr(value)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            eprintln!(
                                "[call-non-function-args] positional=[{}] kwargs=[{}]",
                                args_summary, kwargs_summary
                            );
                        }
                        if std::env::var_os("PYRS_TRACE_CALL_NON_FUNCTION_BT").is_some() {
                            eprintln!(
                                "[call-non-function-bt]\n{:?}",
                                std::backtrace::Backtrace::force_capture()
                            );
                        }
                    } else {
                        eprintln!("[call-non-function] value={}", format_repr(&other));
                    }
                }
                return Err(RuntimeError::type_error("attempted to call non-function"));
            }
        };

        if !needs_run {
            let value = self.pop_value()?;
            return Ok(InternalCallOutcome::Value(value));
        }

        let previous_stop = self.run_stop_depth;
        self.run_stop_depth = Some(caller_depth);
        let run_result = self.run();
        self.run_stop_depth = previous_stop;
        run_result?;

        if self.frames.len() < caller_depth {
            if trace_class_call
                && callable_was_class
                && let Some(callable_repr) = &trace_callable_repr
            {
                eprintln!(
                    "[class-call] caller-frame-dropped callable={} depth_before={} depth_now={}",
                    callable_repr,
                    caller_depth,
                    self.frames.len()
                );
            }
            return Ok(InternalCallOutcome::CallerExceptionHandled);
        }

        let caller = self
            .frames
            .get(caller_depth - 1)
            .ok_or_else(|| RuntimeError::new("caller frame missing"))?;
        if caller.ip != caller_ip {
            if trace_class_call
                && callable_was_class
                && let Some(callable_repr) = &trace_callable_repr
            {
                eprintln!(
                    "[class-call] caller-ip-mismatch callable={} before={} after={}",
                    callable_repr, caller_ip, caller.ip
                );
            }
            return Ok(InternalCallOutcome::CallerExceptionHandled);
        }
        if Self::active_exception_fingerprint(caller.active_exception.as_ref())
            != caller_active_exception_fingerprint
        {
            if trace_class_call
                && callable_was_class
                && let Some(callable_repr) = &trace_callable_repr
            {
                eprintln!(
                    "[class-call] caller-active-exception-changed callable={} before={:?} after={:?}",
                    callable_repr,
                    caller_active_exception_fingerprint,
                    Self::active_exception_fingerprint(caller.active_exception.as_ref())
                );
            }
            return Ok(InternalCallOutcome::CallerExceptionHandled);
        }

        let value = self.pop_value()?;
        if trace_class_call
            && callable_was_class
            && let Some(callable_repr) = &trace_callable_repr
        {
            eprintln!(
                "[class-call] return callable={} value_type={} value_repr={}",
                callable_repr,
                self.value_type_name_for_error(&value),
                format_repr(&value)
            );
        }
        Ok(InternalCallOutcome::Value(value))
    }

    pub(super) fn call_internal_preserving_caller(
        &mut self,
        callable: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<InternalCallOutcome, RuntimeError> {
        let caller_depth = self.frames.len();
        let (caller_ip, caller_stack, caller_blocks, caller_active_exception) = self
            .frames
            .last()
            .map(|frame| {
                (
                    frame.ip,
                    frame.stack.clone(),
                    (!frame.blocks.is_empty()).then(|| frame.blocks.clone()),
                    frame
                        .active_exception
                        .as_ref()
                        .map(Self::clone_active_exception_for_call),
                )
            })
            .unwrap_or((0, Vec::new(), None, None));

        let outcome = self.call_internal(callable, args, kwargs);
        match outcome {
            Ok(InternalCallOutcome::Value(value)) => {
                self.restore_internal_call_caller_state(
                    caller_depth,
                    caller_ip,
                    &caller_stack,
                    &caller_blocks,
                    caller_active_exception.clone(),
                );
                Ok(InternalCallOutcome::Value(value))
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                let active_exception = self
                    .frames
                    .last()
                    .and_then(|frame| frame.active_exception.as_ref())
                    .map(Self::clone_active_exception_for_call);
                self.restore_internal_call_caller_state(
                    caller_depth,
                    caller_ip,
                    &caller_stack,
                    &caller_blocks,
                    active_exception,
                );
                Ok(InternalCallOutcome::CallerExceptionHandled)
            }
            Err(err) => {
                self.restore_internal_call_caller_state(
                    caller_depth,
                    caller_ip,
                    &caller_stack,
                    &caller_blocks,
                    caller_active_exception,
                );
                Err(err)
            }
        }
    }

    pub(super) fn restore_internal_call_caller_state(
        &mut self,
        caller_depth: usize,
        caller_ip: usize,
        caller_stack: &[Value],
        caller_blocks: &Option<Vec<Block>>,
        active_exception: Option<Value>,
    ) {
        if self.frames.len() == caller_depth
            && let Some(frame) = self.frames.last_mut()
        {
            frame.ip = caller_ip;
            frame.stack = caller_stack.to_vec();
            if let Some(blocks) = caller_blocks {
                frame.blocks = blocks.clone();
            } else {
                frame.blocks.clear();
            }
            frame.active_exception = active_exception;
        }
    }

    pub(super) fn load_attr_class(
        &mut self,
        class: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let (class_name, class_metaclass, is_cpython_proxy_class) = match &*class.kind() {
            Object::Class(class_data) => (
                class_data.name.clone(),
                class_data.metaclass.clone(),
                matches!(
                    class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                    Some(Value::Bool(true))
                ),
            ),
            _ => ("<class>".to_string(), None, false),
        };
        if is_cpython_proxy_class
            && attr_name == "__flags__"
            && let Some(proxy_flags) = self.cpython_proxy_type_flags(class)
        {
            return Ok(AttrAccessOutcome::Value(Value::Int(proxy_flags)));
        }
        let mut descriptor_owner: Option<ObjRef> = None;
        let proxy_base_attr = if is_cpython_proxy_class {
            None
        } else {
            let mut value = None;
            for candidate in class_attr_walk(class) {
                let is_proxy_class = matches!(
                    &*candidate.kind(),
                    Object::Class(class_data)
                        if matches!(
                            class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                            Some(Value::Bool(true))
                        )
                );
                if !is_proxy_class {
                    continue;
                }
                if let Some(proxy_attr) = self.load_cpython_proxy_attr(&candidate, attr_name) {
                    value = Some(proxy_attr);
                    break;
                }
            }
            value
        };
        let attr = if let Some(attr) = class_attr_lookup(class, attr_name) {
            attr
        } else if let Some(attr) = proxy_base_attr {
            attr
        } else if attr_name == "__name__" || attr_name == "__qualname__" {
            Value::Str(class_name.clone())
        } else if attr_name == "__base__" {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            };
            class_data
                .bases
                .first()
                .cloned()
                .map(Value::Class)
                .unwrap_or(Value::None)
        } else if attr_name == "__mro__" {
            let mro_values = self
                .class_mro_entries(class)
                .into_iter()
                .map(Value::Class)
                .collect::<Vec<_>>();
            self.heap.alloc_tuple(mro_values)
        } else if attr_name == "mro" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::TypeMro,
                class.clone(),
            )));
        } else if attr_name == "__prepare__" && self.class_has_builtin_type_base(class) {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::TypePrepare,
                class.clone(),
            )));
        } else if attr_name == "__module__" {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            };
            class_data
                .attrs
                .get("__module__")
                .cloned()
                .unwrap_or(Value::None)
        } else if attr_name == "__type_params__" {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            };
            class_data
                .attrs
                .get("__type_params__")
                .cloned()
                .unwrap_or_else(|| self.heap.alloc_tuple(Vec::new()))
        } else if attr_name == "__basicsize__" {
            let basicsize = if self.class_has_builtin_type_base(class) {
                936
            } else if self.class_has_builtin_module_base(class) {
                56
            } else if self.class_has_builtin_tuple_base(class) {
                32
            } else if self.class_has_builtin_list_base(class) {
                40
            } else if self.class_has_builtin_dict_base(class) {
                48
            } else if self.class_has_builtin_set_base(class) {
                200
            } else if self.class_has_builtin_frozenset_base(class) {
                200
            } else if self.class_has_builtin_str_base(class) {
                64
            } else if self.class_has_builtin_bytes_base(class) {
                33
            } else if self.class_has_builtin_bytearray_base(class) {
                56
            } else if self.class_has_builtin_int_base(class) {
                24
            } else if self.class_has_builtin_float_base(class) {
                24
            } else if self.class_has_builtin_complex_base(class) {
                32
            } else {
                16
            };
            Value::Int(basicsize)
        } else if attr_name == "__itemsize__" {
            let itemsize = if self.class_has_builtin_type_base(class) {
                40
            } else if self.class_has_builtin_tuple_base(class) {
                8
            } else if self.class_has_builtin_bytes_base(class) {
                1
            } else if self.class_has_builtin_int_base(class) {
                4
            } else {
                0
            };
            Value::Int(itemsize)
        } else if attr_name == "__dict__" {
            let class_kind = class.kind();
            let Object::Class(class_data) = &*class_kind else {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            };
            let mut entries = class_data
                .attrs
                .iter()
                .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                .collect::<Vec<_>>();
            let is_user_class = matches!(
                class_data.attrs.get("__pyrs_user_class__"),
                Some(Value::Bool(true))
            );
            if !is_user_class
                && !entries
                    .iter()
                    .any(|(name, _)| matches!(name, Value::Str(key) if key == "__new__"))
            {
                entries.push((
                    Value::Str("__new__".to_string()),
                    Value::Builtin(BuiltinFunction::ObjectNew),
                ));
            }
            let dict_value = self.heap.alloc_readonly_dict(entries);
            if let Some(mappingproxy_class) = self
                .mappingproxy_type_class
                .clone()
                .or_else(|| self.types_module_class("__pyrs_mappingproxy_type__"))
            {
                let mappingproxy = match self
                    .heap
                    .alloc_instance(InstanceObject::new(mappingproxy_class))
                {
                    Value::Instance(obj) => obj,
                    _ => unreachable!(),
                };
                if let Object::Instance(instance_data) = &mut *mappingproxy.kind_mut() {
                    instance_data
                        .attrs
                        .insert(MAPPING_PROXY_STORAGE_ATTR.to_string(), dict_value);
                }
                Value::Instance(mappingproxy)
            } else {
                dict_value
            }
        } else if attr_name == "__new__" {
            if is_cpython_proxy_class
                && let Some(proxy_attr) = self.load_cpython_proxy_attr(class, attr_name)
            {
                return Ok(AttrAccessOutcome::Value(proxy_attr));
            }
            if self.class_namedtuple_fields(class).is_some() {
                return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                    BuiltinFunction::ObjectNew,
                    class.clone(),
                )));
            }
            Value::Builtin(BuiltinFunction::ObjectNew)
        } else if attr_name == "__init__" {
            if is_cpython_proxy_class
                && let Some(proxy_attr) = self.load_cpython_proxy_attr(class, attr_name)
            {
                return Ok(AttrAccessOutcome::Value(proxy_attr));
            }
            Value::Builtin(BuiltinFunction::ObjectInit)
        } else if attr_name == "__getstate__" {
            Value::Builtin(BuiltinFunction::ObjectGetState)
        } else if attr_name == "__repr__" && !is_cpython_proxy_class {
            if let Some(meta) = class_metaclass.clone()
                && let Some(meta_attr) = class_attr_lookup(&meta, attr_name)
            {
                descriptor_owner = Some(meta);
                meta_attr
            } else {
                return Ok(AttrAccessOutcome::Value(
                    self.alloc_builtin_bound_method(BuiltinFunction::Repr, class.clone()),
                ));
            }
        } else if attr_name == "__str__" && !is_cpython_proxy_class {
            return Ok(AttrAccessOutcome::Value(
                self.alloc_builtin_bound_method(BuiltinFunction::Str, class.clone()),
            ));
        } else if attr_name == "__format__" && !is_cpython_proxy_class {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectFormat,
                class.clone(),
            )));
        } else if attr_name == "__reduce_ex__" || attr_name == "__reduce__" {
            Value::Builtin(BuiltinFunction::ObjectReduceEx)
        } else if attr_name == "__doc__" {
            Value::None
        } else if attr_name == "__flags__" {
            Value::Int(PY_TPFLAGS_HEAPTYPE)
        } else if let Some(inherited) = self.load_attr_class_builtin_base_method(class, attr_name) {
            inherited
        } else if let Some(meta) = class_metaclass {
            if let Some(meta_attr) = class_attr_lookup(&meta, attr_name) {
                descriptor_owner = Some(meta);
                meta_attr
            } else if let Some(proxy_attr) = self.load_cpython_proxy_attr(class, attr_name) {
                proxy_attr
            } else {
                if std::env::var_os("PYRS_TRACE_PROXY_CLASS_ATTR_MISS").is_some() {
                    let class_kind = class.kind();
                    if let Object::Class(class_data) = &*class_kind {
                        let mut keys = class_data.attrs.keys().cloned().collect::<Vec<_>>();
                        keys.sort();
                        let raw_ptr_present =
                            class_data.attrs.contains_key("__pyrs_cpython_proxy_ptr__");
                        let raw_ptr_detail = class_data
                            .attrs
                            .get("__pyrs_cpython_proxy_ptr__")
                            .and_then(|value| match value {
                                Value::Int(raw) if *raw > 0 => Some(*raw as usize),
                                _ => None,
                            })
                            .map(|raw_ptr| format!(" raw_ptr=0x{raw_ptr:x}"))
                            .unwrap_or_default();
                        eprintln!(
                            "[proxy-class-miss] class={} attr={} raw_ptr_present={} attrs={keys:?}{}",
                            class_name, attr_name, raw_ptr_present, raw_ptr_detail
                        );
                    }
                }
                return Err(RuntimeError::attribute_error(format!(
                    "class '{}' has no attribute '{}'",
                    class_name, attr_name
                )));
            }
        } else if let Some(proxy_attr) = self.load_cpython_proxy_attr(class, attr_name) {
            proxy_attr
        } else {
            if std::env::var_os("PYRS_TRACE_PROXY_CLASS_ATTR_MISS").is_some() {
                let class_kind = class.kind();
                if let Object::Class(class_data) = &*class_kind {
                    let mut keys = class_data.attrs.keys().cloned().collect::<Vec<_>>();
                    keys.sort();
                    let raw_ptr_present =
                        class_data.attrs.contains_key("__pyrs_cpython_proxy_ptr__");
                    let raw_ptr_detail = class_data
                        .attrs
                        .get("__pyrs_cpython_proxy_ptr__")
                        .and_then(|value| match value {
                            Value::Int(raw) if *raw > 0 => Some(*raw as usize),
                            _ => None,
                        })
                        .map(|raw_ptr| format!(" raw_ptr=0x{raw_ptr:x}"))
                        .unwrap_or_default();
                    eprintln!(
                        "[proxy-class-miss] class={} attr={} raw_ptr_present={} attrs={keys:?}{}",
                        class_name, attr_name, raw_ptr_present, raw_ptr_detail
                    );
                }
            }
            return Err(RuntimeError::attribute_error(format!(
                "class '{}' has no attribute '{}'",
                class_name, attr_name
            )));
        };

        let builtin_marker_for_class = |candidate: &ObjRef| -> Option<Value> {
            let candidate_kind = candidate.kind();
            let Object::Class(class_data) = &*candidate_kind else {
                return None;
            };
            if !matches!(
                class_data.attrs.get("__module__"),
                Some(Value::Str(module_name)) if module_name == "builtins"
            ) {
                return None;
            }
            for builtin in [
                BuiltinFunction::Type,
                BuiltinFunction::ObjectNew,
                BuiltinFunction::Bool,
                BuiltinFunction::Int,
                BuiltinFunction::Float,
                BuiltinFunction::Complex,
                BuiltinFunction::Str,
                BuiltinFunction::List,
                BuiltinFunction::Tuple,
                BuiltinFunction::Dict,
                BuiltinFunction::Set,
                BuiltinFunction::FrozenSet,
                BuiltinFunction::Bytes,
                BuiltinFunction::ByteArray,
                BuiltinFunction::MemoryView,
                BuiltinFunction::Range,
                BuiltinFunction::Slice,
                BuiltinFunction::Enumerate,
                BuiltinFunction::Zip,
                BuiltinFunction::Map,
                BuiltinFunction::Filter,
                BuiltinFunction::Super,
                BuiltinFunction::ClassMethod,
                BuiltinFunction::StaticMethod,
                BuiltinFunction::Property,
            ] {
                if self.builtin_attribute_name(builtin) == class_data.name {
                    return Some(Value::Builtin(builtin));
                }
            }
            None
        };

        if attr_name == "__base__"
            && let Value::Class(base_class) = &attr
            && let Some(marker) = builtin_marker_for_class(base_class)
        {
            return Ok(AttrAccessOutcome::Value(marker));
        }
        if matches!(attr_name, "__bases__" | "__mro__")
            && let Value::Tuple(tuple_obj) = &attr
            && let Object::Tuple(items) = &*tuple_obj.kind()
        {
            let normalized = items
                .iter()
                .map(|item| match item {
                    Value::Class(class_obj) => {
                        builtin_marker_for_class(class_obj).unwrap_or_else(|| item.clone())
                    }
                    _ => item.clone(),
                })
                .collect::<Vec<_>>();
            return Ok(AttrAccessOutcome::Value(self.heap.alloc_tuple(normalized)));
        }

        if let Some(bound) = self.bind_classmethod_attr(class, &attr) {
            return Ok(AttrAccessOutcome::Value(bound));
        }

        if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
            return Ok(AttrAccessOutcome::Value(unwrapped));
        }

        if descriptor_owner.is_some()
            && let Value::Function(func) = attr.clone()
        {
            let bound = BoundMethod::new(func, class.clone());
            return Ok(AttrAccessOutcome::Value(
                self.heap.alloc_bound_method(bound),
            ));
        }
        if matches!(attr_name, "__repr__" | "__str__" | "__format__")
            && let Value::Builtin(builtin) = attr.clone()
        {
            return Ok(AttrAccessOutcome::Value(
                self.alloc_builtin_bound_method(builtin, class.clone()),
            ));
        }
        let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
        if let Some(getter) = getter {
            let getter_args = if let Some(owner) = descriptor_owner {
                vec![Value::Class(class.clone()), Value::Class(owner)]
            } else {
                vec![Value::None, Value::Class(class.clone())]
            };
            return Ok(
                match self.call_internal(getter, getter_args, HashMap::new())? {
                    InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrAccessOutcome::ExceptionHandled
                    }
                },
            );
        }

        Ok(AttrAccessOutcome::Value(attr))
    }

    pub(super) fn class_has_builtin_list_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "list",
                _ => false,
            })
    }

    pub(super) fn bind_namedtuple_instance_fields(
        &mut self,
        instance: &ObjRef,
        fields: &[String],
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        let mut bound_values: Vec<Option<Value>> = vec![None; fields.len()];
        if args.len() > fields.len() {
            return Err(RuntimeError::new("namedtuple() argument count mismatch"));
        }
        for (index, value) in args.into_iter().enumerate() {
            bound_values[index] = Some(value);
        }
        for (key, value) in kwargs {
            let Some(index) = fields.iter().position(|name| name == &key) else {
                return Err(RuntimeError::new(
                    "namedtuple() got unexpected keyword argument",
                ));
            };
            if bound_values[index].is_some() {
                return Err(RuntimeError::new(
                    "namedtuple() got multiple values for field",
                ));
            }
            bound_values[index] = Some(value);
        }
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            let mut tuple_values = Vec::with_capacity(fields.len());
            for (index, name) in fields.iter().enumerate() {
                let Some(value) = bound_values[index].clone() else {
                    return Err(RuntimeError::new(format!(
                        "namedtuple() missing value for field '{}'",
                        name
                    )));
                };
                instance_data.attrs.insert(name.clone(), value.clone());
                tuple_values.push(value);
            }
            instance_data.attrs.insert(
                TUPLE_BACKING_STORAGE_ATTR.to_string(),
                self.heap.alloc_tuple(tuple_values),
            );
            Ok(())
        } else {
            Err(RuntimeError::new(
                "namedtuple() instance construction failed",
            ))
        }
    }

    pub(super) fn class_has_builtin_tuple_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => {
                    class_data.name == "tuple"
                        || matches!(
                            class_data.attrs.get("__pyrs_tuple_backed_type__"),
                            Some(Value::Bool(true))
                        )
                }
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_str_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "str",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_bytes_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "bytes",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_bytearray_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "bytearray",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_int_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "int",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_float_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "float",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_complex_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "complex",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_dict_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "dict",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_defaultdict_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "defaultdict",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_ordereddict_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "OrderedDict",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_set_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "set",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_frozenset_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "frozenset",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_property_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "property",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_module_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "module",
                _ => false,
            })
    }

    pub(super) fn class_has_builtin_type_base(&self, class: &ObjRef) -> bool {
        self.class_mro_entries(class)
            .iter()
            .any(|entry| match &*entry.kind() {
                Object::Class(class_data) => class_data.name == "type",
                _ => false,
            })
    }

    pub(super) fn instantiate_type_derived_class(
        &mut self,
        metaclass: ObjRef,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 3 {
            return Err(RuntimeError::type_error(format!(
                "type.__new__() takes exactly 3 arguments ({} given)",
                args.len()
            )));
        }
        let resolve_metaclass_ctor = |name: &str, vm: &mut Vm| -> Option<Value> {
            let raw = class_attr_lookup_direct(&metaclass, name)?;
            if let Some(bound) = vm.bind_classmethod_attr(&metaclass, &raw) {
                return Some(bound);
            }
            if let Some(unwrapped) = vm.unwrap_staticmethod_attr(&raw) {
                return Some(unwrapped);
            }
            Some(raw)
        };
        let custom_new = resolve_metaclass_ctor("__new__", self)
            .filter(|callable| !matches!(callable, Value::Builtin(BuiltinFunction::Type)));
        if let Some(new_callable) = custom_new {
            if std::env::var_os("PYRS_TRACE_METACLASS_NEW_FAIL").is_some() {
                eprintln!(
                    "[metaclass-new] metaclass={} callable_type={} callable_repr={}",
                    match &*metaclass.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "<non-class>".to_string(),
                    },
                    self.value_type_name_for_error(&new_callable),
                    format_repr(&new_callable)
                );
            }
            let prepend_meta_arg = !matches!(new_callable, Value::BoundMethod(_));
            let mut new_args = Vec::with_capacity(args.len() + usize::from(prepend_meta_arg));
            if prepend_meta_arg {
                new_args.push(Value::Class(metaclass.clone()));
            }
            new_args.extend(args.clone());
            let created = match self.call_internal(new_callable, new_args, kwargs.clone())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception("metaclass call failed"));
                }
            };
            if !matches!(created, Value::Class(_)) {
                if std::env::var_os("PYRS_TRACE_METACLASS_NEW_FAIL").is_some() {
                    let meta_name = match &*metaclass.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "<non-class>".to_string(),
                    };
                    let class_name = match args.first() {
                        Some(Value::Str(name)) => name.clone(),
                        Some(value) => format!("<{}>", self.value_type_name_for_error(value)),
                        None => "<missing>".to_string(),
                    };
                    eprintln!(
                        "[metaclass-new-fail] metaclass={} class_name={} returned_type={}",
                        meta_name,
                        class_name,
                        self.value_type_name_for_error(&created)
                    );
                }
                return Err(RuntimeError::new(
                    "metaclass __new__ must return a class object",
                ));
            }
            if let Some(init_callable) = resolve_metaclass_ctor("__init__", self)
                .filter(|callable| !matches!(callable, Value::Builtin(BuiltinFunction::NoOp)))
            {
                let init_result = self.call_internal(
                    init_callable,
                    vec![
                        created.clone(),
                        args[0].clone(),
                        args[1].clone(),
                        args[2].clone(),
                    ],
                    kwargs,
                )?;
                match init_result {
                    InternalCallOutcome::Value(Value::None) => {}
                    InternalCallOutcome::Value(_) => {
                        return Err(RuntimeError::new("__init__() should return None"));
                    }
                    InternalCallOutcome::CallerExceptionHandled => {
                        return Err(
                            self.runtime_error_from_active_exception("metaclass __init__ failed")
                        );
                    }
                }
            }
            return Ok(created);
        }
        let created = self.call_builtin(
            BuiltinFunction::Type,
            vec![
                Value::Class(metaclass.clone()),
                args[0].clone(),
                args[1].clone(),
                args[2].clone(),
            ],
            kwargs.clone(),
        )?;
        if let Some(init_callable) = resolve_metaclass_ctor("__init__", self)
            .filter(|callable| !matches!(callable, Value::Builtin(BuiltinFunction::NoOp)))
        {
            let init_result = self.call_internal(
                init_callable,
                vec![
                    created.clone(),
                    args[0].clone(),
                    args[1].clone(),
                    args[2].clone(),
                ],
                kwargs,
            )?;
            match init_result {
                InternalCallOutcome::Value(Value::None) => {}
                InternalCallOutcome::Value(_) => {
                    return Err(RuntimeError::new("__init__() should return None"));
                }
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(
                        self.runtime_error_from_active_exception("metaclass __init__ failed")
                    );
                }
            }
        }
        Ok(created)
    }

    pub(super) fn alloc_instance_for_class(&mut self, class: &ObjRef) -> ObjRef {
        let instance = match self.heap.alloc_instance(InstanceObject::new(class.clone())) {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if self.class_has_builtin_list_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    LIST_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_list(Vec::new()),
                );
            }
        } else if self.class_has_builtin_tuple_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    TUPLE_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_tuple(Vec::new()),
                );
            }
        } else if self.class_has_builtin_str_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data.attrs.insert(
                STR_BACKING_STORAGE_ATTR.to_string(),
                Value::Str(String::new()),
            );
        }
        if self.class_has_builtin_bytes_base(class) {
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.attrs.insert(
                    BYTES_BACKING_STORAGE_ATTR.to_string(),
                    self.heap.alloc_bytes(Vec::new()),
                );
            }
        } else if self.class_has_builtin_bytearray_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data.attrs.insert(
                BYTES_BACKING_STORAGE_ATTR.to_string(),
                self.heap.alloc_bytearray(Vec::new()),
            );
        }
        if self.class_has_builtin_int_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data
                .attrs
                .insert(INT_BACKING_STORAGE_ATTR.to_string(), Value::Int(0));
        }
        if self.class_has_builtin_float_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data
                .attrs
                .insert(FLOAT_BACKING_STORAGE_ATTR.to_string(), Value::Float(0.0));
        }
        if self.class_has_builtin_complex_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data.attrs.insert(
                COMPLEX_BACKING_STORAGE_ATTR.to_string(),
                Value::Complex {
                    real: 0.0,
                    imag: 0.0,
                },
            );
        }
        if self.class_has_builtin_dict_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data.attrs.insert(
                DICT_BACKING_STORAGE_ATTR.to_string(),
                self.heap.alloc_dict(Vec::new()),
            );
        }
        if self.class_has_builtin_set_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data.attrs.insert(
                SET_BACKING_STORAGE_ATTR.to_string(),
                self.heap.alloc_set(Vec::new()),
            );
        }
        if self.class_has_builtin_frozenset_base(class)
            && let Object::Instance(instance_data) = &mut *instance.kind_mut()
        {
            instance_data.attrs.insert(
                FROZENSET_BACKING_STORAGE_ATTR.to_string(),
                self.heap.alloc_frozenset(Vec::new()),
            );
        }
        self.track_instance_del_candidate(class, &instance);
        instance
    }

    pub(super) fn instance_backing_list(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(LIST_BACKING_STORAGE_ATTR) {
            Some(Value::List(list)) => Some(list.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_tuple(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(TUPLE_BACKING_STORAGE_ATTR) {
            Some(Value::Tuple(tuple)) => Some(tuple.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_str(&self, instance: &ObjRef) -> Option<String> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(STR_BACKING_STORAGE_ATTR) {
            Some(Value::Str(text)) => Some(text.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_int(&self, instance: &ObjRef) -> Option<Value> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(INT_BACKING_STORAGE_ATTR) {
            Some(Value::Int(value)) => Some(Value::Int(*value)),
            Some(Value::BigInt(value)) => Some(Value::BigInt(value.clone())),
            Some(Value::Bool(value)) => Some(Value::Bool(*value)),
            _ => None,
        }
    }

    pub(super) fn instance_backing_float(&self, instance: &ObjRef) -> Option<f64> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(FLOAT_BACKING_STORAGE_ATTR) {
            Some(Value::Float(value)) => Some(*value),
            _ => None,
        }
    }

    pub(super) fn instance_backing_complex(&self, instance: &ObjRef) -> Option<(f64, f64)> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(COMPLEX_BACKING_STORAGE_ATTR) {
            Some(Value::Complex { real, imag }) => Some((*real, *imag)),
            _ => None,
        }
    }

    pub(super) fn instance_backing_dict(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(DICT_BACKING_STORAGE_ATTR) {
            Some(Value::Dict(dict)) => Some(dict.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_set(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(SET_BACKING_STORAGE_ATTR) {
            Some(Value::Set(set)) => Some(set.clone()),
            _ => None,
        }
    }

    pub(super) fn instance_backing_frozenset(&self, instance: &ObjRef) -> Option<ObjRef> {
        let Object::Instance(instance_data) = &*instance.kind() else {
            return None;
        };
        match instance_data.attrs.get(FROZENSET_BACKING_STORAGE_ATTR) {
            Some(Value::FrozenSet(set)) => Some(set.clone()),
            _ => None,
        }
    }

    fn alloc_paramspec_attr_instance(
        &mut self,
        origin: &ObjRef,
        preferred_module: Option<&str>,
        attr_name: &str,
    ) -> Option<Value> {
        let class_name = match attr_name {
            "args" => "ParamSpecArgs",
            "kwargs" => "ParamSpecKwargs",
            _ => return None,
        };

        let mut module_names: Vec<String> = Vec::new();
        if let Some(module_name) = preferred_module {
            module_names.push(module_name.to_string());
        }
        if !module_names.iter().any(|name| name == "typing") {
            module_names.push("typing".to_string());
        }
        if !module_names.iter().any(|name| name == "_typing") {
            module_names.push("_typing".to_string());
        }

        for module_name in module_names {
            let Some(module_ref) = self.modules.get(module_name.as_str()).cloned() else {
                continue;
            };
            let class_ref = {
                let module_kind = module_ref.kind();
                let Object::Module(module_data) = &*module_kind else {
                    continue;
                };
                match module_data.globals.get(class_name) {
                    Some(Value::Class(class_ref)) => class_ref.clone(),
                    _ => continue,
                }
            };
            let Value::Instance(attr_obj) =
                self.heap.alloc_instance(InstanceObject::new(class_ref))
            else {
                continue;
            };
            if let Object::Instance(attr_data) = &mut *attr_obj.kind_mut() {
                attr_data
                    .attrs
                    .insert("__origin__".to_string(), Value::Instance(origin.clone()));
            }
            return Some(Value::Instance(attr_obj));
        }

        None
    }

    pub(super) fn instance_dict_entries(instance_data: &InstanceObject) -> Vec<(Value, Value)> {
        instance_data
            .attrs
            .iter()
            .filter_map(|(name, value)| match name.as_str() {
                LIST_BACKING_STORAGE_ATTR
                | TUPLE_BACKING_STORAGE_ATTR
                | STR_BACKING_STORAGE_ATTR
                | BYTES_BACKING_STORAGE_ATTR
                | INT_BACKING_STORAGE_ATTR
                | FLOAT_BACKING_STORAGE_ATTR
                | COMPLEX_BACKING_STORAGE_ATTR
                | DICT_BACKING_STORAGE_ATTR
                | SET_BACKING_STORAGE_ATTR
                | FROZENSET_BACKING_STORAGE_ATTR
                | INSTANCE_DICT_STORAGE_ATTR => None,
                _ => Some((Value::Str(name.clone()), value.clone())),
            })
            .collect()
    }

    fn instance_missing_attribute_error(
        &self,
        instance: &ObjRef,
        class_name: &str,
        attr_name: &str,
    ) -> RuntimeError {
        let message = format!("'{}' object has no attribute '{}'", class_name, attr_name);
        let exception = ExceptionObject::new("AttributeError", Some(message.clone()));
        {
            let mut attrs = exception.attrs.borrow_mut();
            attrs.insert(
                "args".to_string(),
                self.heap.alloc_tuple(vec![Value::Str(message)]),
            );
            attrs.insert("name".to_string(), Value::Str(attr_name.to_string()));
            attrs.insert("obj".to_string(), Value::Instance(instance.clone()));
        }
        RuntimeError::from_exception(exception)
    }

    fn annotate_runtime_attribute_error_context(
        &self,
        err: &mut RuntimeError,
        instance: &ObjRef,
        attr_name: &str,
    ) {
        let Some(exception) = err.exception.as_mut() else {
            return;
        };
        if !self.exception_inherits(&exception.name, "AttributeError") {
            return;
        }
        let mut attrs = exception.attrs.borrow_mut();
        attrs.insert("name".to_string(), Value::Str(attr_name.to_string()));
        attrs.insert("obj".to_string(), Value::Instance(instance.clone()));
    }

    fn annotate_active_attribute_error_context(&mut self, instance: &ObjRef, attr_name: &str) {
        let Some(active) = self
            .frames
            .last_mut()
            .and_then(|frame| frame.active_exception.as_mut())
        else {
            return;
        };
        let Value::Exception(exception) = active else {
            return;
        };
        if exception.name != "AttributeError" {
            return;
        }
        let mut attrs = exception.attrs.borrow_mut();
        attrs.insert("name".to_string(), Value::Str(attr_name.to_string()));
        attrs.insert("obj".to_string(), Value::Instance(instance.clone()));
    }

    pub(super) fn load_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            }
        };
        let is_cpython_proxy_instance = matches!(
            &*class_ref.kind(),
            Object::Class(class_data)
                if class_data.name == "__pyrs_cpython_proxy__"
                    || matches!(
                        class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                        Some(Value::Bool(true))
                    )
        );

        // CPython routes the default object-attribute path through a native slot
        // rather than an ordinary Python-level method call each access.
        // Mirror that behavior by bypassing generic bound-method invocation when
        // __getattribute__ resolves to the builtin object implementation.
        if matches!(
            class_attr_lookup(&class_ref, "__getattribute__"),
            Some(Value::Builtin(BuiltinFunction::ObjectGetAttribute))
        ) {
            return self.load_attr_instance_default(instance, attr_name, true);
        }

        let receiver = Value::Instance(instance.clone());
        if let Some(getattribute_method) =
            self.lookup_bound_special_method(&receiver, "__getattribute__")?
        {
            let mut getattribute_runtime_attribute_error: Option<RuntimeError> = None;
            let getattribute_outcome = self.call_internal_preserving_caller(
                getattribute_method,
                vec![Value::Str(attr_name.to_string())],
                HashMap::new(),
            );
            match getattribute_outcome {
                Ok(InternalCallOutcome::Value(value)) => {
                    return Ok(AttrAccessOutcome::Value(value));
                }
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    if !self.active_exception_is("AttributeError") {
                        return Ok(AttrAccessOutcome::ExceptionHandled);
                    }
                }
                Err(err) => {
                    if !runtime_error_matches_exception(&err, "AttributeError") {
                        return Err(err);
                    }
                    getattribute_runtime_attribute_error = Some(err);
                }
            }

            if let Some(getattr_method) =
                self.lookup_bound_special_method(&receiver, "__getattr__")?
            {
                if self.active_exception_is("AttributeError") {
                    self.clear_active_exception();
                }
                return Ok(
                    match self.call_internal_preserving_caller(
                        getattr_method,
                        vec![Value::Str(attr_name.to_string())],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrAccessOutcome::ExceptionHandled
                        }
                    },
                );
            }

            if !is_cpython_proxy_instance {
                if self.active_exception_is("AttributeError") {
                    return Ok(AttrAccessOutcome::ExceptionHandled);
                }
                if let Some(err) = getattribute_runtime_attribute_error {
                    return Err(err);
                }
            } else if self.active_exception_is("AttributeError") {
                self.clear_active_exception();
            }

            if is_cpython_proxy_instance {
                let proxy_fallback = self.load_attr_instance_default(instance, attr_name, false);
                match proxy_fallback {
                    Ok(AttrAccessOutcome::Value(value)) => {
                        return Ok(AttrAccessOutcome::Value(value));
                    }
                    Ok(AttrAccessOutcome::ExceptionHandled) => {
                        return Ok(AttrAccessOutcome::ExceptionHandled);
                    }
                    Err(err) => {
                        if !runtime_error_matches_exception(&err, "AttributeError") {
                            return Err(err);
                        }
                    }
                }
                if let Some(err) = getattribute_runtime_attribute_error {
                    return Err(err);
                }
            }

            let class_name = match &*instance.kind() {
                Object::Instance(instance_data) => match &*instance_data.class.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<class>".to_string(),
                },
                _ => "<class>".to_string(),
            };
            return Err(self.instance_missing_attribute_error(instance, &class_name, attr_name));
        }

        self.load_attr_instance_default(instance, attr_name, true)
    }

    pub(super) fn load_attr_instance_default(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
        allow_getattr_fallback: bool,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            }
        };
        let is_cpython_proxy_instance = matches!(
            &*class_ref.kind(),
            Object::Class(class_data)
                if class_data.name == "__pyrs_cpython_proxy__"
                    || matches!(
                        class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                        Some(Value::Bool(true))
                    )
        );

        if attr_name == "__class__" {
            return Ok(AttrAccessOutcome::Value(Value::Class(class_ref)));
        }

        if attr_name == "__dict__" {
            let slot_layout = collect_slot_names(&class_ref);
            let inherited_slot_names = if slot_layout.is_none() {
                let mut inherited = Vec::new();
                for candidate in class_attr_walk(&class_ref).into_iter().skip(1) {
                    if let Object::Class(class_data) = &*candidate.kind()
                        && let Some(slots) = &class_data.slots
                    {
                        for slot in slots {
                            if !inherited.iter().any(|existing| existing == slot) {
                                inherited.push(slot.clone());
                            }
                        }
                    }
                }
                inherited
            } else {
                Vec::new()
            };
            let has_dynamic_dict =
                Self::class_supports_dynamic_instance_dict(&class_ref, slot_layout.as_ref());
            if !has_dynamic_dict {
                let class_name = match &*class_ref.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "<class>".to_string(),
                };
                return Err(RuntimeError::new(format!(
                    "'{}' object has no attribute '__dict__'",
                    class_name
                )));
            }
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                if let Some(Value::Dict(dict_obj)) =
                    instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                {
                    return Ok(AttrAccessOutcome::Value(Value::Dict(dict_obj.clone())));
                }
                let dict_entries = match &slot_layout {
                    Some(allowed_slots) => instance_data
                        .attrs
                        .iter()
                        .filter_map(|(name, value)| {
                            if matches!(
                                name.as_str(),
                                LIST_BACKING_STORAGE_ATTR
                                    | TUPLE_BACKING_STORAGE_ATTR
                                    | STR_BACKING_STORAGE_ATTR
                                    | BYTES_BACKING_STORAGE_ATTR
                                    | INT_BACKING_STORAGE_ATTR
                                    | FLOAT_BACKING_STORAGE_ATTR
                                    | COMPLEX_BACKING_STORAGE_ATTR
                                    | DICT_BACKING_STORAGE_ATTR
                                    | SET_BACKING_STORAGE_ATTR
                                    | FROZENSET_BACKING_STORAGE_ATTR
                                    | INSTANCE_DICT_STORAGE_ATTR
                            ) {
                                return None;
                            }
                            if allowed_slots.iter().any(|slot| slot == name) {
                                return None;
                            }
                            Some((Value::Str(name.clone()), value.clone()))
                        })
                        .collect(),
                    None if inherited_slot_names.is_empty() => {
                        Self::instance_dict_entries(instance_data)
                    }
                    None => instance_data
                        .attrs
                        .iter()
                        .filter_map(|(name, value)| {
                            if matches!(
                                name.as_str(),
                                LIST_BACKING_STORAGE_ATTR
                                    | TUPLE_BACKING_STORAGE_ATTR
                                    | STR_BACKING_STORAGE_ATTR
                                    | BYTES_BACKING_STORAGE_ATTR
                                    | INT_BACKING_STORAGE_ATTR
                                    | FLOAT_BACKING_STORAGE_ATTR
                                    | COMPLEX_BACKING_STORAGE_ATTR
                                    | DICT_BACKING_STORAGE_ATTR
                                    | SET_BACKING_STORAGE_ATTR
                                    | FROZENSET_BACKING_STORAGE_ATTR
                                    | INSTANCE_DICT_STORAGE_ATTR
                            ) {
                                return None;
                            }
                            if inherited_slot_names.iter().any(|slot| slot == name) {
                                return None;
                            }
                            Some((Value::Str(name.clone()), value.clone()))
                        })
                        .collect(),
                };
                let dict_value = self.heap.alloc_dict(dict_entries);
                instance_data
                    .attrs
                    .insert(INSTANCE_DICT_STORAGE_ATTR.to_string(), dict_value.clone());
                return Ok(AttrAccessOutcome::Value(dict_value));
            }
            return Err(RuntimeError::attribute_error(
                "attribute access unsupported type",
            ));
        }

        if let Some(attr) = self.load_attr_property_instance(instance, attr_name)? {
            return Ok(AttrAccessOutcome::Value(attr));
        }
        if let Some(attr) = self.load_attr_cached_property_instance(instance, attr_name) {
            return Ok(AttrAccessOutcome::Value(attr));
        }
        if is_cpython_proxy_instance {
            let proxy_attr = self
                .load_cpython_proxy_attr_for_value(&Value::Instance(instance.clone()), attr_name);
            if let Some(proxy_attr) = proxy_attr {
                return Ok(AttrAccessOutcome::Value(proxy_attr));
            }
        }

        let mut class_attr_owner: Option<ObjRef> = None;
        let mut class_attr = class_attr_walk(&class_ref)
            .into_iter()
            .find_map(|candidate| {
                class_attr_lookup_direct(&candidate, attr_name).inspect(|_value| {
                    class_attr_owner = Some(candidate);
                })
            });
        if class_attr.is_none() {
            for candidate in class_attr_walk(&class_ref) {
                let is_proxy_class = matches!(
                    &*candidate.kind(),
                    Object::Class(class_data)
                        if matches!(
                            class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                            Some(Value::Bool(true))
                        )
                );
                if !is_proxy_class {
                    continue;
                }
                if let Some(proxy_attr) = self.load_cpython_proxy_attr(&candidate, attr_name) {
                    class_attr_owner = Some(candidate);
                    class_attr = Some(proxy_attr);
                    break;
                }
            }
        }
        if let Some(attr) = class_attr.clone() {
            let (getter, setter, deleter) = self.descriptor_hooks(&attr)?;
            if setter.is_some() || deleter.is_some() {
                if let Some(getter) = getter {
                    return Ok(
                        match self.call_internal_preserving_caller(
                            getter,
                            vec![
                                Value::Instance(instance.clone()),
                                Value::Class(class_ref.clone()),
                            ],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                AttrAccessOutcome::ExceptionHandled
                            }
                        },
                    );
                }
                return Ok(AttrAccessOutcome::Value(attr));
            }
        }

        if let Object::Instance(instance_data) = &*instance.kind() {
            let slot_layout = collect_slot_names(&class_ref);
            let has_dynamic_dict =
                Self::class_supports_dynamic_instance_dict(&class_ref, slot_layout.as_ref());
            let mut attr_is_declared_slot = false;
            if let Some(allowed_slots) = &slot_layout {
                attr_is_declared_slot = allowed_slots.iter().any(|name| name == attr_name);
            }
            if let Some(allowed_slots) = &slot_layout
                && has_dynamic_dict
                && !attr_is_declared_slot
                && !allowed_slots.is_empty()
            {
                if let Some(Value::Dict(dict_obj)) =
                    instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                    && let Some(attr) = dict_get_value(dict_obj, &Value::Str(attr_name.to_string()))
                {
                    return Ok(AttrAccessOutcome::Value(attr));
                }
            } else {
                if let Some(attr) = instance_data.attrs.get(attr_name).cloned() {
                    return Ok(AttrAccessOutcome::Value(attr));
                }
                if let Some(Value::Dict(dict_obj)) =
                    instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                    && let Some(attr) = dict_get_value(dict_obj, &Value::Str(attr_name.to_string()))
                {
                    return Ok(AttrAccessOutcome::Value(attr));
                }
            }
        }

        if attr_name == "__init__" {
            let enum_like = class_attr_walk(&class_ref).into_iter().any(|candidate| {
                matches!(
                    &*candidate.kind(),
                    Object::Class(class_data)
                        if matches!(
                            class_data.name.as_str(),
                            "Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" | "ReprEnum"
                        )
                )
            });
            if enum_like
                || self.class_has_builtin_int_base(&class_ref)
                || self.class_has_builtin_float_base(&class_ref)
                || self.class_has_builtin_str_base(&class_ref)
            {
                return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                    BuiltinFunction::NoOp,
                    instance.clone(),
                )));
            }
        }

        if attr_name == "closed" {
            let is_iobase_instance = class_attr_walk(&class_ref).into_iter().any(|candidate| {
                matches!(&*candidate.kind(), Object::Class(class_data) if class_data.name == "IOBase")
            });
            if is_iobase_instance && let Object::Instance(instance_data) = &*instance.kind() {
                let closed = matches!(instance_data.attrs.get("closed"), Some(Value::Bool(true)))
                    || matches!(
                        instance_data.attrs.get("__IOBase_closed"),
                        Some(Value::Bool(true))
                    )
                    || matches!(instance_data.attrs.get("_closed"), Some(Value::Bool(true)));
                return Ok(AttrAccessOutcome::Value(Value::Bool(closed)));
            }
        }

        let reduce_ex_attr = attr_name == "__reduce_ex__";
        let reduce_attr = reduce_ex_attr || attr_name == "__reduce__";
        let is_type_parameter_instance =
            self.is_type_parameter_value(&Value::Instance(instance.clone()));
        let is_generic_alias_instance = self
            .generic_alias_parts_from_value(&Value::Instance(instance.clone()))
            .is_some();
        let is_types_generic_alias_instance =
            self.is_types_generic_alias_value(&Value::Instance(instance.clone()));
        if is_type_parameter_instance {
            if attr_name == "__repr__" || attr_name == "__str__" {
                return Ok(AttrAccessOutcome::Value(self.alloc_native_bound_method(
                    NativeMethodKind::TypeParamRepr,
                    instance.clone(),
                )));
            }
            if attr_name == "__copy__" || attr_name == "__deepcopy__" {
                return Ok(AttrAccessOutcome::Value(self.alloc_native_bound_method(
                    NativeMethodKind::TypeParamCopy,
                    instance.clone(),
                )));
            }
            if reduce_attr {
                return Ok(AttrAccessOutcome::Value(self.alloc_native_bound_method(
                    NativeMethodKind::TypeParamReduceEx,
                    instance.clone(),
                )));
            }
            if attr_name == "args" || attr_name == "kwargs" {
                let preferred_module = match &*class_ref.kind() {
                    Object::Class(class_data)
                        if class_data.name == "ParamSpec"
                            && matches!(
                                class_data.attrs.get("__module__"),
                                Some(Value::Str(module_name))
                                    if module_name == "typing" || module_name == "_typing"
                            ) =>
                    {
                        match class_data.attrs.get("__module__") {
                            Some(Value::Str(module_name)) => Some(module_name.clone()),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(value) = self.alloc_paramspec_attr_instance(
                    instance,
                    preferred_module.as_deref(),
                    attr_name,
                ) {
                    return Ok(AttrAccessOutcome::Value(value));
                }
            }
        }
        if is_types_generic_alias_instance && attr_name == "__mro_entries__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_native_bound_method(
                NativeMethodKind::GenericAliasMroEntries,
                instance.clone(),
            )));
        }
        if is_types_generic_alias_instance && attr_name == "__call__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_native_bound_method(
                NativeMethodKind::GenericAliasCall,
                instance.clone(),
            )));
        }
        if reduce_ex_attr && is_generic_alias_instance {
            return Ok(AttrAccessOutcome::Value(self.alloc_native_bound_method(
                NativeMethodKind::GenericAliasReduceEx,
                instance.clone(),
            )));
        }
        if is_types_generic_alias_instance
            && !attr_name.starts_with("__")
            && let Some((origin, _args)) =
                self.generic_alias_parts_from_value(&Value::Instance(instance.clone()))
        {
            match self.builtin_getattr(
                vec![origin, Value::Str(attr_name.to_string())],
                HashMap::new(),
            ) {
                Ok(value) => return Ok(AttrAccessOutcome::Value(value)),
                Err(err) if runtime_error_matches_exception(&err, "AttributeError") => {}
                Err(err) => return Err(err),
            }
        }
        if class_attr.is_none()
            && let Some(backing_list) = self.instance_backing_list(instance)
            && !reduce_attr
            && let Ok(bound_method) = self.load_attr_list_method(backing_list, attr_name)
        {
            return Ok(AttrAccessOutcome::Value(bound_method));
        }
        if class_attr.is_none()
            && let Some(backing_tuple) = self.instance_backing_tuple(instance)
            && !reduce_attr
            && let Ok(bound_method) = self.load_attr_tuple_method(backing_tuple, attr_name)
        {
            return Ok(AttrAccessOutcome::Value(bound_method));
        }
        if class_attr.is_none()
            && let Some(backing_str) = self.instance_backing_str(instance)
            && !reduce_attr
            && let Ok(bound_method) = self.load_attr_str_method(backing_str, attr_name)
        {
            return Ok(AttrAccessOutcome::Value(bound_method));
        }
        if class_attr.is_none()
            && let Some(backing_dict) = self.instance_backing_dict(instance)
            && !reduce_attr
        {
            let is_exact_dict = matches!(
                &*class_ref.kind(),
                Object::Class(class_data) if class_data.name == "dict"
            );
            let owner = if attr_name == "__getitem__" && !is_exact_dict {
                Some(Value::Instance(instance.clone()))
            } else {
                None
            };
            if let Ok(bound_method) =
                self.load_attr_dict_method_with_owner(backing_dict, owner, attr_name)
            {
                return Ok(AttrAccessOutcome::Value(bound_method));
            }
        }
        if class_attr.is_none()
            && let Some(backing_set) = self.instance_backing_set(instance)
            && !reduce_attr
            && let Ok(bound_method) = self.load_attr_set_method(backing_set, attr_name)
        {
            return Ok(AttrAccessOutcome::Value(bound_method));
        }
        if class_attr.is_none()
            && let Some(backing_frozenset) = self.instance_backing_frozenset(instance)
            && !reduce_attr
            && let Ok(bound_method) = self.load_attr_set_method(backing_frozenset, attr_name)
        {
            return Ok(AttrAccessOutcome::Value(bound_method));
        }

        if let Some(attr) = class_attr {
            if let Some(bound) = self.bind_classmethod_attr(&class_ref, &attr) {
                return Ok(AttrAccessOutcome::Value(bound));
            }

            if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
                return Ok(AttrAccessOutcome::Value(unwrapped));
            }
            if let Value::Function(func) = attr.clone() {
                let bound = BoundMethod::new(func, instance.clone());
                return Ok(AttrAccessOutcome::Value(
                    self.heap.alloc_bound_method(bound),
                ));
            }
            if let Value::Builtin(builtin) = attr.clone() {
                let owner_is_user_class = class_attr_owner
                    .as_ref()
                    .is_some_and(|owner| matches!(&*owner.kind(), Object::Class(class_data)
                        if matches!(class_data.attrs.get("__pyrs_user_class__"), Some(Value::Bool(true)))));
                if owner_is_user_class {
                    return Ok(AttrAccessOutcome::Value(Value::Builtin(builtin)));
                }
                return Ok(AttrAccessOutcome::Value(
                    self.alloc_builtin_bound_method(builtin, instance.clone()),
                ));
            }
            let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
            if let Some(getter) = getter {
                return Ok(
                    match self.call_internal_preserving_caller(
                        getter,
                        vec![
                            Value::Instance(instance.clone()),
                            Value::Class(class_ref.clone()),
                        ],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrAccessOutcome::ExceptionHandled
                        }
                    },
                );
            }
            return Ok(AttrAccessOutcome::Value(attr));
        }

        if allow_getattr_fallback
            && let Some(getattr_method) =
                self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__getattr__")?
        {
            let getattr_outcome = self.call_internal_preserving_caller(
                getattr_method,
                vec![Value::Str(attr_name.to_string())],
                HashMap::new(),
            );
            return match getattr_outcome {
                Ok(InternalCallOutcome::Value(value)) => Ok(AttrAccessOutcome::Value(value)),
                Ok(InternalCallOutcome::CallerExceptionHandled) => {
                    if self.active_exception_is("AttributeError") {
                        self.annotate_active_attribute_error_context(instance, attr_name);
                    }
                    Ok(AttrAccessOutcome::ExceptionHandled)
                }
                Err(mut err) => {
                    if runtime_error_matches_exception(&err, "AttributeError") {
                        self.annotate_runtime_attribute_error_context(
                            &mut err, instance, attr_name,
                        );
                    }
                    Err(err)
                }
            };
        }

        if attr_name == "__getstate__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectGetState,
                instance.clone(),
            )));
        }
        if attr_name == "__setstate__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectSetState,
                instance.clone(),
            )));
        }
        if attr_name == "__reduce_ex__" || attr_name == "__reduce__" {
            return Ok(AttrAccessOutcome::Value(
                self.alloc_reduce_ex_bound_method(Value::Instance(instance.clone())),
            ));
        }
        if attr_name == "__doc__" {
            return Ok(AttrAccessOutcome::Value(Value::None));
        }
        if let Some(proxy_attr) =
            self.load_cpython_proxy_attr_for_value(&Value::Instance(instance.clone()), attr_name)
        {
            return Ok(AttrAccessOutcome::Value(proxy_attr));
        }

        let class_name = match &*class_ref.kind() {
            Object::Class(class_data) => class_data.name.clone(),
            _ => "<class>".to_string(),
        };
        if std::env::var_os("PYRS_TRACE_PROXY_INSTANCE_ATTR_MISS").is_some()
            && class_name == "__pyrs_cpython_proxy__"
        {
            let raw_ptr = match &*instance.kind() {
                Object::Instance(instance_data) => instance_data
                    .attrs
                    .get("__pyrs_cpython_proxy_ptr__")
                    .cloned(),
                _ => None,
            };
            eprintln!(
                "[proxy-instance-miss] class={} attr={} raw_ptr={raw_ptr:?}",
                class_name, attr_name
            );
        }
        Err(self.instance_missing_attribute_error(instance, &class_name, attr_name))
    }

    pub(super) fn load_attr_super(
        &mut self,
        super_ref: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrAccessOutcome, RuntimeError> {
        let _super_depth_guard = LoadAttrSuperDepthGuard::enter();
        let super_depth = LOAD_ATTR_SUPER_DEPTH.with(|counter| counter.get());
        let (start_class, receiver, object_type) = match &*super_ref.kind() {
            Object::Super(data) => (
                data.start_class.clone(),
                data.object.clone(),
                data.object_type.clone(),
            ),
            _ => {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            }
        };
        if std::env::var_os("PYRS_TRACE_LOAD_ATTR_SUPER").is_some() && super_depth > 1 {
            let start_name = match &*start_class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            let object_type_name = match &*object_type.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            if super_depth <= 2 {
                let mro = self.class_mro_entries(&object_type);
                let mro_summary = mro
                    .iter()
                    .map(|entry| match &*entry.kind() {
                        Object::Class(class_data) => format!("{}#{}", class_data.name, entry.id()),
                        _ => format!("<non-class>#{}", entry.id()),
                    })
                    .collect::<Vec<_>>()
                    .join(" -> ");
                eprintln!("[load-attr-super] mro={}", mro_summary);
            }
            eprintln!(
                "[load-attr-super] depth={} attr={} start={}#{} object_type={}#{}",
                super_depth,
                attr_name,
                start_name,
                start_class.id(),
                object_type_name,
                object_type.id()
            );
        }

        let receiver_value = self.receiver_value(&receiver)?;
        let owner_value = Value::Class(object_type.clone());
        let mro = self.class_mro_entries(&object_type);
        let class_name = |class: &ObjRef| match &*class.kind() {
            Object::Class(class_data) => Some(class_data.name.clone()),
            _ => None,
        };
        let start_name = class_name(&start_class);
        let mut start_idx = mro
            .iter()
            .position(|entry| entry.id() == start_class.id())
            .map(|idx| idx + 1);
        if start_idx.is_none()
            && let Some(start_name) = start_name
        {
            start_idx = mro
                .iter()
                .position(|entry| class_name(entry).as_deref() == Some(start_name.as_str()))
                .map(|idx| idx + 1);
        }
        let start_idx = start_idx.unwrap_or_else(|| usize::from(!mro.is_empty()));
        if std::env::var_os("PYRS_TRACE_SUPER_DTYPE").is_some() && attr_name == "dtype" {
            let start_name = match &*start_class.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            let object_type_name = match &*object_type.kind() {
                Object::Class(class_data) => class_data.name.clone(),
                _ => "<non-class>".to_string(),
            };
            let mro_names = mro
                .iter()
                .map(|entry| match &*entry.kind() {
                    Object::Class(class_data) => format!("{}#{}", class_data.name, entry.id()),
                    _ => format!("<non-class>#{}", entry.id()),
                })
                .collect::<Vec<_>>()
                .join(" -> ");
            eprintln!(
                "[super-dtype] start={}#{} object_type={}#{} start_idx={} mro={}",
                start_name,
                start_class.id(),
                object_type_name,
                object_type.id(),
                start_idx,
                mro_names
            );
        }

        for class in mro.into_iter().skip(start_idx) {
            let class_attr = class_attr_lookup_direct(&class, attr_name)
                .or_else(|| self.load_cpython_proxy_attr(&class, attr_name));
            if let Some(attr) = class_attr {
                if std::env::var_os("PYRS_TRACE_LOAD_ATTR_SUPER").is_some() && super_depth > 1 {
                    let owner_name = match &*class.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "<non-class>".to_string(),
                    };
                    eprintln!(
                        "[load-attr-super] resolved owner={}#{} attr={} value_type={}",
                        owner_name,
                        class.id(),
                        attr_name,
                        self.value_type_name_for_error(&attr)
                    );
                }
                if std::env::var_os("PYRS_TRACE_SUPER_DTYPE").is_some() && attr_name == "dtype" {
                    let class_name = match &*class.kind() {
                        Object::Class(class_data) => class_data.name.clone(),
                        _ => "<non-class>".to_string(),
                    };
                    eprintln!(
                        "[super-dtype] owner={} attr_type={} proxy_ptr={:?} repr={}",
                        class_name,
                        self.value_type_name_for_error(&attr),
                        Self::cpython_proxy_raw_ptr_from_value(&attr),
                        format_repr(&attr)
                    );
                }
                if let Some(bound) = self.bind_classmethod_attr(&object_type, &attr) {
                    return Ok(AttrAccessOutcome::Value(bound));
                }
                if let Some(unwrapped) = self.unwrap_staticmethod_attr(&attr) {
                    return Ok(AttrAccessOutcome::Value(unwrapped));
                }
                if let Value::Function(func) = attr.clone() {
                    if attr_name == "__new__" {
                        return Ok(AttrAccessOutcome::Value(Value::Function(func)));
                    }
                    let bound = BoundMethod::new(func, receiver.clone());
                    return Ok(AttrAccessOutcome::Value(
                        self.heap.alloc_bound_method(bound),
                    ));
                }
                if let Value::Builtin(builtin) = attr.clone() {
                    if attr_name == "__new__" {
                        return Ok(AttrAccessOutcome::Value(Value::Builtin(builtin)));
                    }
                    return Ok(AttrAccessOutcome::Value(
                        self.alloc_builtin_bound_method(builtin, receiver.clone()),
                    ));
                }
                if Self::cpython_proxy_raw_ptr_from_value(&attr).is_some()
                    && let Some(bound_result) =
                        self.bind_cpython_descriptor_for_super(&attr, &receiver_value, &owner_value)
                {
                    return Ok(AttrAccessOutcome::Value(bound_result?));
                }
                let (getter, _setter, _deleter) = self.descriptor_hooks(&attr)?;
                if std::env::var_os("PYRS_TRACE_SUPER_DTYPE").is_some() && attr_name == "dtype" {
                    let getter_tag = getter
                        .as_ref()
                        .map(|value| {
                            format!(
                                "{} {}",
                                self.value_type_name_for_error(value),
                                format_repr(value)
                            )
                        })
                        .unwrap_or_else(|| "<none>".to_string());
                    eprintln!("[super-dtype] descriptor getter={getter_tag}");
                }
                if let Some(getter) = getter {
                    return Ok(
                        match self.call_internal(
                            getter,
                            vec![receiver_value.clone(), owner_value.clone()],
                            HashMap::new(),
                        )? {
                            InternalCallOutcome::Value(value) => AttrAccessOutcome::Value(value),
                            InternalCallOutcome::CallerExceptionHandled => {
                                AttrAccessOutcome::ExceptionHandled
                            }
                        },
                    );
                }
                return Ok(AttrAccessOutcome::Value(attr));
            }
            if self.class_has_builtin_dict_base(&class) {
                let dict_receiver = match &receiver_value {
                    Value::Dict(dict) => Some(dict.clone()),
                    Value::Instance(instance) => self.instance_backing_dict(instance),
                    _ => None,
                };
                if let Some(dict_receiver) = dict_receiver
                    && let Ok(method) = self.load_attr_dict_method_with_owner(
                        dict_receiver,
                        Some(owner_value.clone()),
                        attr_name,
                    )
                {
                    return Ok(AttrAccessOutcome::Value(method));
                }
            }
            if self.class_has_builtin_list_base(&class) {
                let list_receiver = match &receiver_value {
                    Value::List(list) => Some(list.clone()),
                    Value::Instance(instance) => self.instance_backing_list(instance),
                    _ => None,
                };
                if let Some(list_receiver) = list_receiver
                    && let Ok(method) = self.load_attr_list_method(list_receiver, attr_name)
                {
                    return Ok(AttrAccessOutcome::Value(method));
                }
            }
        }

        // Synthetic builtin base classes used by class inheritance currently
        // may not carry explicit `__new__`/`__init__` attrs in their class dict.
        // CPython still resolves these through `super(...)` in paths like
        // `super(Subclass, cls).__new__(cls, value)`.
        if attr_name == "__new__" {
            return Ok(AttrAccessOutcome::Value(Value::Builtin(
                BuiltinFunction::ObjectNew,
            )));
        }
        if attr_name == "__prepare__" && self.class_has_builtin_type_base(&object_type) {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::TypePrepare,
                receiver.clone(),
            )));
        }
        if attr_name == "__instancecheck__" && self.class_has_builtin_type_base(&object_type) {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::TypeInstanceCheck,
                receiver.clone(),
            )));
        }
        if attr_name == "__subclasscheck__" && self.class_has_builtin_type_base(&object_type) {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::TypeSubclassCheck,
                receiver.clone(),
            )));
        }
        if attr_name == "__init__" {
            return Ok(AttrAccessOutcome::Value(self.alloc_builtin_bound_method(
                BuiltinFunction::ObjectInit,
                receiver,
            )));
        }

        Err(RuntimeError::attribute_error(format!(
            "super object has no attribute '{}'",
            attr_name
        )))
    }

    pub(super) fn load_attr_module(
        &mut self,
        module: &ObjRef,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        const MODULE_INITIALIZING_FLAG: &str = "__pyrs_module_initializing__";
        const FRAME_PROXY_FLAG: &str = "__pyrs_frame_proxy__";
        let is_frame_proxy = match &*module.kind() {
            Object::Module(module_data) => module_data
                .globals
                .get(FRAME_PROXY_FLAG)
                .is_some_and(|value| matches!(value, Value::Bool(true))),
            _ => false,
        };
        if is_frame_proxy {
            self.refresh_frame_proxy_cache_if_active(module);
        }
        let module_is_initializing = match &*module.kind() {
            Object::Module(module_data) => module_data
                .globals
                .get(MODULE_INITIALIZING_FLAG)
                .is_some_and(|value| matches!(value, Value::Bool(true))),
            _ => false,
        };
        let active_frame_module_dict = if module_is_initializing {
            if self
                .frames
                .last()
                .is_some_and(|frame| frame.is_module && frame.module.id() == module.id())
            {
                Some(self.ensure_frame_module_locals_dict(self.frames.len().saturating_sub(1)))
            } else {
                self.frames
                    .iter()
                    .rposition(|frame| frame.is_module && frame.module.id() == module.id())
                    .map(|frame_index| self.ensure_frame_module_locals_dict(frame_index))
            }
        } else {
            None
        };
        let (module_name, attr, module_getattr, module_is_package) = match &*module.kind() {
            Object::Module(module_data) => {
                let attr_key = Value::Str(attr_name.to_string());
                let getattr_key = Value::Str("__getattr__".to_string());
                let path_key = Value::Str("__path__".to_string());
                let attr = active_frame_module_dict
                    .as_ref()
                    .and_then(|dict| dict_get_value(dict, &attr_key))
                    .or_else(|| module_data.globals.get(attr_name).cloned());
                let module_getattr = active_frame_module_dict
                    .as_ref()
                    .and_then(|dict| dict_get_value(dict, &getattr_key))
                    .or_else(|| module_data.globals.get("__getattr__").cloned());
                let module_name = module_data.name.clone();
                let module_is_package = active_frame_module_dict
                    .as_ref()
                    .and_then(|dict| dict_get_value(dict, &path_key))
                    .is_some()
                    || module_data.globals.contains_key("__path__");
                (module_name, attr, module_getattr, module_is_package)
            }
            _ => {
                return Err(RuntimeError::attribute_error(
                    "attribute access unsupported type",
                ));
            }
        };
        if let Some(attr) = attr {
            return Ok(attr);
        }
        if (module_name == "__classmethod__" || module_name == "__staticmethod__")
            && (attr_name == "__reduce_ex__" || attr_name == "__reduce__")
        {
            return Ok(self.alloc_native_bound_method(
                NativeMethodKind::DescriptorReduceTypeError,
                module.clone(),
            ));
        }
        if module_name == "__classmethod__" && attr_name == "__get__" {
            return Ok(self.alloc_native_bound_method(
                NativeMethodKind::ClassMethodDescriptorGet,
                module.clone(),
            ));
        }
        if module_name == "__staticmethod__" && attr_name == "__get__" {
            return Ok(self.alloc_native_bound_method(
                NativeMethodKind::StaticMethodDescriptorGet,
                module.clone(),
            ));
        }
        if module_name == "unittest" && attr_name == "IsolatedAsyncioTestCase" {
            let test_case = if let Some(dict) = &active_frame_module_dict {
                dict_get_value(dict, &Value::Str("TestCase".to_string()))
                    .or_else(|| dict_get_value(dict, &Value::Str("Case".to_string())))
            } else {
                match &*module.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("TestCase")
                        .cloned()
                        .or_else(|| module_data.globals.get("Case").cloned()),
                    _ => None,
                }
            };
            if let Some(test_case) = test_case {
                return Ok(test_case);
            }
            return Ok(Value::Class(
                self.alloc_synthetic_class("unittest.IsolatedAsyncioTestCase"),
            ));
        }
        if module_name == "__array__" && attr_name == "tobytes" {
            return Ok(self.alloc_builtin_bound_method(BuiltinFunction::Bytes, module.clone()));
        }
        if attr_name == "__dict__" {
            if let Some(dict) = active_frame_module_dict {
                return Ok(Value::Dict(dict));
            }
            if let Object::Module(module_data) = &*module.kind() {
                let globals_snapshot = module_data
                    .globals
                    .iter()
                    .map(|(name, value)| (Value::Str(name.clone()), value.clone()))
                    .collect::<Vec<_>>();
                return Ok(self.heap.alloc_dict(globals_snapshot));
            }
            return Ok(self.heap.alloc_dict(Vec::new()));
        }
        if module_name == "__re_pattern__" {
            let kind = match attr_name {
                "search" => Some(NativeMethodKind::RePatternSearch),
                "match" => Some(NativeMethodKind::RePatternMatch),
                "fullmatch" => Some(NativeMethodKind::RePatternFullMatch),
                "sub" => Some(NativeMethodKind::RePatternSub),
                "findall" => Some(NativeMethodKind::Builtin(BuiltinFunction::RePatternFindAll)),
                "finditer" => Some(NativeMethodKind::Builtin(
                    BuiltinFunction::RePatternFindIter,
                )),
                "split" => Some(NativeMethodKind::Builtin(BuiltinFunction::RePatternSplit)),
                _ => None,
            };
            if let Some(kind) = kind {
                return Ok(self.alloc_native_bound_method(kind, module.clone()));
            }
        }
        if module_name == "__re_match__" {
            let kind = match attr_name {
                "group" => Some(NativeMethodKind::ReMatchGroup),
                "__getitem__" => Some(NativeMethodKind::ReMatchGroup),
                "groups" => Some(NativeMethodKind::ReMatchGroups),
                "groupdict" => Some(NativeMethodKind::ReMatchGroupDict),
                "start" => Some(NativeMethodKind::ReMatchStart),
                "end" => Some(NativeMethodKind::ReMatchEnd),
                "span" => Some(NativeMethodKind::ReMatchSpan),
                _ => None,
            };
            if let Some(kind) = kind {
                return Ok(self.alloc_native_bound_method(kind, module.clone()));
            }
        }
        if let Some(attr) = module_name.split('.').next_back().and_then(|suffix| {
            if suffix == attr_name {
                Some(Value::Module(module.clone()))
            } else {
                None
            }
        }) {
            return Ok(attr);
        }
        if module_is_package && let Some(submodule) = self.load_submodule(module, attr_name) {
            return Ok(Value::Module(submodule));
        }
        if super::env_var_present_cached("PYRS_TRACE_NUMPY_DTYPE_RESOLVE")
            && module_name == "numpy.dtypes"
        {
            if let Object::Module(module_data) = &*module.kind() {
                let mut keys = module_data.globals.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                eprintln!(
                    "[numpy-dtypes] resolve attr={} has___getattr__={} is_package={} globals_len={} keys={:?}",
                    attr_name,
                    module_getattr.is_some(),
                    module_is_package,
                    keys.len(),
                    keys
                );
            }
        }
        if attr_name != "__getattr__"
            && let Some(module_getattr) = module_getattr
        {
            return match self.call_internal(
                module_getattr,
                vec![Value::Str(attr_name.to_string())],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => Ok(value),
                InternalCallOutcome::CallerExceptionHandled => {
                    Err(RuntimeError::new("module __getattr__ failed"))
                }
            };
        }
        Err(RuntimeError::attribute_error(format!(
            "module '{}' has no attribute '{}'",
            module_name, attr_name
        )))
    }

    pub(super) fn store_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
        value: Value,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        if let Some(setattr_method) =
            self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__setattr__")?
        {
            return Ok(
                match self.call_internal(
                    setattr_method,
                    vec![Value::Str(attr_name.to_string()), value],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrMutationOutcome::ExceptionHandled
                    }
                },
            );
        }

        self.store_attr_instance_direct(instance, attr_name, value)
    }

    fn class_is_cpickle_type(class_ref: &ObjRef, expected_name: &str) -> bool {
        let Object::Class(class_data) = &*class_ref.kind() else {
            return false;
        };
        if class_data.name != expected_name {
            return false;
        }
        matches!(
            class_data.attrs.get("__module__"),
            Some(Value::Str(module_name)) if module_name == "_pickle"
        )
    }

    fn class_assignment_layout_signature(class_ref: &ObjRef) -> Option<(Vec<String>, bool)> {
        let Object::Class(_) = &*class_ref.kind() else {
            return None;
        };
        let slot_layout = collect_slot_names(class_ref);
        let mut effective_slots = slot_layout.clone().unwrap_or_default();
        if slot_layout.is_none() {
            for candidate in class_attr_walk(class_ref).into_iter().skip(1) {
                if let Object::Class(class_data) = &*candidate.kind()
                    && let Some(slots) = &class_data.slots
                {
                    for slot in slots {
                        if !effective_slots.iter().any(|existing| existing == slot) {
                            effective_slots.push(slot.clone());
                        }
                    }
                }
            }
        }
        let has_dynamic_dict =
            Self::class_supports_dynamic_instance_dict(class_ref, slot_layout.as_ref());
        Some((effective_slots, has_dynamic_dict))
    }

    fn class_supports_dynamic_instance_dict(
        class_ref: &ObjRef,
        slot_layout: Option<&Vec<String>>,
    ) -> bool {
        let explicit_dynamic = match slot_layout {
            Some(allowed_slots) => allowed_slots.iter().any(|name| name == "__dict__"),
            None => matches!(
                &*class_ref.kind(),
                Object::Class(class_data)
                    if matches!(
                        class_data.attrs.get("__pyrs_user_class__"),
                        Some(Value::Bool(true))
                    )
            ),
        };
        explicit_dynamic || class_inherits_dynamic_instance_dict(class_ref)
    }

    fn class_assignment_layout_compatible(old_class: &ObjRef, new_class: &ObjRef) -> bool {
        let Some(old_layout) = Self::class_assignment_layout_signature(old_class) else {
            return false;
        };
        let Some(new_layout) = Self::class_assignment_layout_signature(new_class) else {
            return false;
        };
        old_layout == new_layout
    }

    fn validate_cpickle_unpickler_memo_assignment(value: &Value) -> Result<(), RuntimeError> {
        let Value::Dict(dict_obj) = value else {
            return Err(RuntimeError::new(
                "TypeError: unpickler memo must be a dict",
            ));
        };
        let Object::Dict(entries) = &*dict_obj.kind() else {
            return Err(RuntimeError::new(
                "TypeError: unpickler memo must be a dict",
            ));
        };
        for (key, _) in entries.iter() {
            let is_negative_index = match key {
                Value::Int(index) => *index < 0,
                Value::BigInt(index) => index.is_negative(),
                Value::Bool(_) => false,
                _ => {
                    return Err(RuntimeError::type_error("memo keys must be integers"));
                }
            };
            if is_negative_index {
                return Err(RuntimeError::value_error("memo key out of range"));
            }
        }
        Ok(())
    }

    pub(super) fn store_attr_instance_direct(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
        value: Value,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "attribute assignment unsupported type",
                ));
            }
        };
        let mro_class_names = class_attr_walk(&class_ref)
            .into_iter()
            .filter_map(|candidate| match &*candidate.kind() {
                Object::Class(class_data) => Some(class_data.name.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let raw_is_readonly = attr_name == "raw"
            && mro_class_names
                .iter()
                .any(|name| name == "BufferedIOBase" || name == "BufferedReader");
        let buffer_is_readonly =
            attr_name == "buffer" && mro_class_names.iter().any(|name| name == "TextIOWrapper");
        if raw_is_readonly || buffer_is_readonly {
            return Err(RuntimeError::attribute_error("readonly attribute"));
        }
        if matches!(
            &*class_ref.kind(),
            Object::Class(class_data) if class_data.name == "__csv_dialect__"
        ) {
            return Err(RuntimeError::new("csv dialect attributes are read-only"));
        }
        if attr_name == "memo" && Self::class_is_cpickle_type(&class_ref, "Unpickler") {
            Self::validate_cpickle_unpickler_memo_assignment(&value)?;
        }

        if attr_name == "__class__" {
            let Value::Class(new_class) = value else {
                return Err(RuntimeError::type_error(format!(
                    "__class__ must be set to a class, not '{}' object",
                    self.value_type_name_for_error(&value)
                )));
            };
            if class_ref.id() == new_class.id() {
                return Ok(AttrMutationOutcome::Done);
            }
            if !Self::class_assignment_layout_compatible(&class_ref, &new_class) {
                let old_name = match &*class_ref.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "object".to_string(),
                };
                let new_name = match &*new_class.kind() {
                    Object::Class(class_data) => class_data.name.clone(),
                    _ => "object".to_string(),
                };
                return Err(RuntimeError::type_error(format!(
                    "__class__ assignment: '{}' object layout differs from '{}'",
                    new_name, old_name
                )));
            }
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                instance_data.class = new_class;
            }
            return Ok(AttrMutationOutcome::Done);
        }

        if let Some(descriptor) = class_attr_lookup(&class_ref, attr_name) {
            let (_getter, setter, _deleter) = self.descriptor_hooks(&descriptor)?;
            if let Some(setter) = setter {
                return Ok(
                    match self.call_internal(
                        setter,
                        vec![Value::Instance(instance.clone()), value],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrMutationOutcome::ExceptionHandled
                        }
                    },
                );
            }
        }

        if attr_name == "__dict__" {
            let Value::Dict(new_dict) = value else {
                return Err(RuntimeError::type_error("__dict__ must be set to a dict"));
            };
            let slot_layout = collect_slot_names(&class_ref);
            let inherited_slot_names = if slot_layout.is_none() {
                let mut inherited = Vec::new();
                for candidate in class_attr_walk(&class_ref).into_iter().skip(1) {
                    if let Object::Class(class_data) = &*candidate.kind()
                        && let Some(slots) = &class_data.slots
                    {
                        for slot in slots {
                            if !inherited.iter().any(|existing| existing == slot) {
                                inherited.push(slot.clone());
                            }
                        }
                    }
                }
                inherited
            } else {
                Vec::new()
            };
            let has_dynamic_dict =
                Self::class_supports_dynamic_instance_dict(&class_ref, slot_layout.as_ref());
            if !has_dynamic_dict {
                return Err(RuntimeError::attribute_error(format!(
                    "'{}' object has no attribute '__dict__'",
                    class_name_for_instance(instance).unwrap_or_else(|| "object".to_string())
                )));
            }
            if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                let values_to_mirror = match &*new_dict.kind() {
                    Object::Dict(entries) => Some(entries.clone()),
                    _ => None,
                };
                instance_data.attrs.insert(
                    INSTANCE_DICT_STORAGE_ATTR.to_string(),
                    Value::Dict(new_dict.clone()),
                );

                // Clear dynamic attributes from the inline attr map to avoid stale
                // cross-thread/object dict leakage; keep slots and storage attrs.
                instance_data.attrs.retain(|name, _| {
                    if matches!(
                        name.as_str(),
                        LIST_BACKING_STORAGE_ATTR
                            | TUPLE_BACKING_STORAGE_ATTR
                            | STR_BACKING_STORAGE_ATTR
                            | BYTES_BACKING_STORAGE_ATTR
                            | INT_BACKING_STORAGE_ATTR
                            | FLOAT_BACKING_STORAGE_ATTR
                            | COMPLEX_BACKING_STORAGE_ATTR
                            | DICT_BACKING_STORAGE_ATTR
                            | SET_BACKING_STORAGE_ATTR
                            | FROZENSET_BACKING_STORAGE_ATTR
                            | INSTANCE_DICT_STORAGE_ATTR
                    ) {
                        return true;
                    }
                    if let Some(allowed_slots) = &slot_layout {
                        return allowed_slots.iter().any(|slot| slot == name);
                    }
                    inherited_slot_names.iter().any(|slot| slot == name)
                });

                // Slot-less classes keep a mirrored inline attr map; slot classes
                // with dynamic dict use INSTANCE_DICT_STORAGE_ATTR as source-of-truth
                // for non-slot attributes.
                if slot_layout.is_none()
                    && inherited_slot_names.is_empty()
                    && let Some(entries) = values_to_mirror
                {
                    for (key, dict_value) in entries {
                        if let Value::Str(name) = key {
                            instance_data.attrs.insert(name, dict_value);
                        }
                    }
                }
            }
            return Ok(AttrMutationOutcome::Done);
        }

        if let Some(allowed_slots) = collect_slot_names(&class_ref) {
            let has_dynamic_dict = allowed_slots.iter().any(|name| name == "__dict__")
                || class_inherits_dynamic_instance_dict(&class_ref);
            if has_dynamic_dict {
                let attr_is_declared_slot = allowed_slots.iter().any(|name| name == attr_name);
                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                    if attr_is_declared_slot {
                        instance_data.attrs.insert(attr_name.to_string(), value);
                    } else {
                        let dict_obj = match instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR) {
                            Some(Value::Dict(dict_obj)) => dict_obj.clone(),
                            _ => {
                                let dict_obj = self.heap.alloc_dict(Vec::new());
                                instance_data.attrs.insert(
                                    INSTANCE_DICT_STORAGE_ATTR.to_string(),
                                    dict_obj.clone(),
                                );
                                match dict_obj {
                                    Value::Dict(dict_ref) => dict_ref,
                                    _ => unreachable!(),
                                }
                            }
                        };
                        dict_set_value(&dict_obj, Value::Str(attr_name.to_string()), value);
                    }
                }
                return Ok(AttrMutationOutcome::Done);
            }
            let allowed = allowed_slots.iter().any(|name| name == attr_name);
            if !allowed {
                return Err(RuntimeError::new(format!(
                    "'{}' object has no attribute '{}'",
                    class_name_for_instance(instance).unwrap_or_else(|| "object".to_string()),
                    attr_name
                )));
            }
        }

        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            if let Some(Value::Dict(dict_obj)) = instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
            {
                dict_set_value(dict_obj, Value::Str(attr_name.to_string()), value.clone());
            }
            instance_data.attrs.insert(attr_name.to_string(), value);
        }
        Ok(AttrMutationOutcome::Done)
    }

    pub(super) fn delete_attr_instance(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        if let Some(delattr_method) =
            self.lookup_bound_special_method(&Value::Instance(instance.clone()), "__delattr__")?
        {
            return Ok(
                match self.call_internal(
                    delattr_method,
                    vec![Value::Str(attr_name.to_string())],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                    InternalCallOutcome::CallerExceptionHandled => {
                        AttrMutationOutcome::ExceptionHandled
                    }
                },
            );
        }

        self.delete_attr_instance_direct(instance, attr_name)
    }

    pub(super) fn delete_attr_instance_direct(
        &mut self,
        instance: &ObjRef,
        attr_name: &str,
    ) -> Result<AttrMutationOutcome, RuntimeError> {
        let class_ref = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "attribute deletion unsupported type",
                ));
            }
        };
        if attr_name == "_CHUNK_SIZE"
            && matches!(
                &*class_ref.kind(),
                Object::Class(class_data) if class_data.name == "TextIOWrapper"
            )
        {
            return Err(RuntimeError::attribute_error("cannot delete attribute"));
        }
        if matches!(
            &*class_ref.kind(),
            Object::Class(class_data) if class_data.name == "__csv_dialect__"
        ) {
            return Err(RuntimeError::new("csv dialect attributes are read-only"));
        }

        if let Some(descriptor) = class_attr_lookup(&class_ref, attr_name) {
            let (_getter, _setter, deleter) = self.descriptor_hooks(&descriptor)?;
            if let Some(deleter) = deleter {
                return Ok(
                    match self.call_internal(
                        deleter,
                        vec![Value::Instance(instance.clone())],
                        HashMap::new(),
                    )? {
                        InternalCallOutcome::Value(_) => AttrMutationOutcome::Done,
                        InternalCallOutcome::CallerExceptionHandled => {
                            AttrMutationOutcome::ExceptionHandled
                        }
                    },
                );
            }
        }

        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            if instance_data.attrs.remove(attr_name).is_some() {
                if let Some(Value::Dict(dict_obj)) =
                    instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                {
                    dict_remove_value(dict_obj, &Value::Str(attr_name.to_string()));
                }
                return Ok(AttrMutationOutcome::Done);
            }
            if let Some(Value::Dict(dict_obj)) = instance_data.attrs.get(INSTANCE_DICT_STORAGE_ATTR)
                && dict_remove_value(dict_obj, &Value::Str(attr_name.to_string())).is_some()
            {
                return Ok(AttrMutationOutcome::Done);
            }
        }

        Err(RuntimeError::new(format!(
            "AttributeError: attribute '{}' does not exist",
            attr_name
        )))
    }
}
