use super::super::{Vm, RuntimeError, Value, HashMap, bytes_like_from_value, ObjRef, Object};
use std::ffi::CStr;
use std::mem;
use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr;

const Z_OK: c_int = 0;
const Z_STREAM_END: c_int = 1;
const Z_NEED_DICT: c_int = 2;
const Z_BUF_ERROR: c_int = -5;
const Z_NO_FLUSH: c_int = 0;
const Z_SYNC_FLUSH: c_int = 2;
const Z_FINISH: c_int = 4;
const Z_DEFLATED: c_int = 8;
const Z_DEFAULT_COMPRESSION: c_int = -1;
const Z_DEFAULT_STRATEGY: c_int = 0;
const Z_DEFAULT_WINDOW_BITS: c_int = 15;
const Z_DEFAULT_MEM_LEVEL: c_int = 8;
const Z_CHUNK_SIZE: usize = 32 * 1024;

#[derive(Clone)]
pub(in crate::vm) struct ZlibCompressObjectState {
    level: c_int,
    wbits: c_int,
    mem_level: c_int,
    strategy: c_int,
    buffer: Vec<u8>,
    finished: bool,
}

#[derive(Clone)]
pub(in crate::vm) struct ZlibDecompressObjectState {
    wbits: c_int,
    eof: bool,
    unused_data: Vec<u8>,
    unconsumed_tail: Vec<u8>,
}

type ZAlloc = Option<unsafe extern "C" fn(*mut c_void, c_uint, c_uint) -> *mut c_void>;
type ZFree = Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>;

#[repr(C)]
struct ZStream {
    next_in: *mut u8,
    avail_in: c_uint,
    total_in: c_ulong,
    next_out: *mut u8,
    avail_out: c_uint,
    total_out: c_ulong,
    msg: *mut c_char,
    state: *mut c_void,
    zalloc: ZAlloc,
    zfree: ZFree,
    opaque: *mut c_void,
    data_type: c_int,
    adler: c_ulong,
    reserved: c_ulong,
}

#[link(name = "z")]
unsafe extern "C" {
    fn zlibVersion() -> *const c_char;
    fn deflateInit2_(
        strm: *mut ZStream,
        level: c_int,
        method: c_int,
        window_bits: c_int,
        mem_level: c_int,
        strategy: c_int,
        version: *const c_char,
        stream_size: c_int,
    ) -> c_int;
    fn deflate(strm: *mut ZStream, flush: c_int) -> c_int;
    fn deflateEnd(strm: *mut ZStream) -> c_int;
    fn inflateInit2_(
        strm: *mut ZStream,
        window_bits: c_int,
        version: *const c_char,
        stream_size: c_int,
    ) -> c_int;
    fn inflate(strm: *mut ZStream, flush: c_int) -> c_int;
    fn inflateEnd(strm: *mut ZStream) -> c_int;
    fn crc32(crc: c_ulong, buf: *const u8, len: c_uint) -> c_ulong;
}

impl Vm {
    fn zlib_error_message(action: &str, code: c_int) -> RuntimeError {
        RuntimeError::new(format!("zlib.error: {action} failed with code {code}"))
    }

    fn zlib_parse_optional_int(value: Option<Value>, default: i64, name: &str) -> Result<i64, RuntimeError> {
        match value {
            None => Ok(default),
            Some(Value::Int(v)) => Ok(v),
            Some(Value::Bool(v)) => Ok(if v { 1 } else { 0 }),
            Some(_) => Err(RuntimeError::new(format!(
                "TypeError: integer argument expected for '{name}'"
            ))),
        }
    }

    fn zlib_parse_compress_kwargs(
        &self,
        args: &mut Vec<Value>,
        kwargs: &mut HashMap<String, Value>,
    ) -> Result<(Vec<u8>, c_int, c_int), RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new("TypeError: compress() missing required argument 'data'"));
        }
        if args.len() > 3 {
            return Err(RuntimeError::new(format!(
                "TypeError: compress() takes at most 3 positional arguments ({} given)",
                args.len()
            )));
        }
        let data_arg = args.remove(0);
        let level_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("level")
        };
        let wbits_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("wbits")
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: compress() got an unexpected keyword argument '{key}'"
            )));
        }

        let level = Self::zlib_parse_optional_int(level_arg, Z_DEFAULT_COMPRESSION as i64, "level")?;
        let wbits = Self::zlib_parse_optional_int(wbits_arg, Z_DEFAULT_WINDOW_BITS as i64, "wbits")?;
        let payload = bytes_like_from_value(data_arg)
            .map_err(|_| RuntimeError::new("TypeError: a bytes-like object is required"))?;
        Ok((payload, level as c_int, wbits as c_int))
    }

    fn zlib_deflate_bytes(
        &self,
        payload: &[u8],
        level: c_int,
        wbits: c_int,
        mem_level: c_int,
        strategy: c_int,
    ) -> Result<Vec<u8>, RuntimeError> {
        if payload.len() > c_uint::MAX as usize {
            return Err(RuntimeError::new("OverflowError: input is too large"));
        }

        let mut stream: ZStream = unsafe { mem::zeroed() };
        let version = unsafe { zlibVersion() };
        if version.is_null() {
            return Err(RuntimeError::new("zlib.error: zlibVersion() returned NULL"));
        }
        let init_code = unsafe {
            deflateInit2_(
                &mut stream,
                level,
                Z_DEFLATED,
                wbits,
                mem_level,
                strategy,
                version,
                mem::size_of::<ZStream>() as c_int,
            )
        };
        if init_code != Z_OK {
            return Err(Self::zlib_error_message("deflateInit2", init_code));
        }

        stream.next_in = if payload.is_empty() {
            ptr::null_mut()
        } else {
            payload.as_ptr() as *mut u8
        };
        stream.avail_in = payload.len() as c_uint;

        let mut out = Vec::with_capacity(payload.len().saturating_add(64));
        let mut chunk = [0u8; Z_CHUNK_SIZE];
        loop {
            stream.next_out = chunk.as_mut_ptr();
            stream.avail_out = chunk.len() as c_uint;
            let code = unsafe { deflate(&mut stream, Z_FINISH) };
            let written = chunk.len() - stream.avail_out as usize;
            if written > 0 {
                out.extend_from_slice(&chunk[..written]);
            }
            if code == Z_STREAM_END {
                break;
            }
            if code != Z_OK && code != Z_BUF_ERROR {
                unsafe {
                    deflateEnd(&mut stream);
                }
                return Err(Self::zlib_error_message("deflate", code));
            }
            if code == Z_BUF_ERROR && written == 0 {
                break;
            }
        }

        unsafe {
            deflateEnd(&mut stream);
        }
        Ok(out)
    }

    fn zlib_inflate_bytes(
        &self,
        payload: &[u8],
        wbits: c_int,
        require_eof: bool,
    ) -> Result<(Vec<u8>, bool, Vec<u8>), RuntimeError> {
        if payload.len() > c_uint::MAX as usize {
            return Err(RuntimeError::new("OverflowError: input is too large"));
        }

        let mut stream: ZStream = unsafe { mem::zeroed() };
        let version = unsafe { zlibVersion() };
        if version.is_null() {
            return Err(RuntimeError::new("zlib.error: zlibVersion() returned NULL"));
        }
        let init_code = unsafe {
            inflateInit2_(
                &mut stream,
                wbits,
                version,
                mem::size_of::<ZStream>() as c_int,
            )
        };
        if init_code != Z_OK {
            return Err(Self::zlib_error_message("inflateInit2", init_code));
        }

        stream.next_in = if payload.is_empty() {
            ptr::null_mut()
        } else {
            payload.as_ptr() as *mut u8
        };
        stream.avail_in = payload.len() as c_uint;

        let mut out = Vec::with_capacity(payload.len().saturating_mul(2).max(64));
        let mut chunk = [0u8; Z_CHUNK_SIZE];
        let mut eof = false;
        loop {
            stream.next_out = chunk.as_mut_ptr();
            stream.avail_out = chunk.len() as c_uint;
            let code = unsafe { inflate(&mut stream, Z_NO_FLUSH) };
            let written = chunk.len() - stream.avail_out as usize;
            if written > 0 {
                out.extend_from_slice(&chunk[..written]);
            }

            match code {
                Z_STREAM_END => {
                    eof = true;
                    break;
                }
                Z_OK => {
                    if stream.avail_in == 0 && written == 0 {
                        break;
                    }
                }
                Z_BUF_ERROR => {
                    if stream.avail_in == 0 {
                        break;
                    }
                }
                Z_NEED_DICT => {
                    unsafe {
                        inflateEnd(&mut stream);
                    }
                    return Err(RuntimeError::new(
                        "zlib.error: stream requires a dictionary",
                    ));
                }
                _ => {
                    unsafe {
                        inflateEnd(&mut stream);
                    }
                    return Err(Self::zlib_error_message("inflate", code));
                }
            }
        }

        let remaining = stream.avail_in as usize;
        unsafe {
            inflateEnd(&mut stream);
        }

        if require_eof && !eof {
            return Err(RuntimeError::new(
                "zlib.error: incomplete or truncated stream",
            ));
        }

        let consumed = payload.len().saturating_sub(remaining);
        let unused = if eof && consumed < payload.len() {
            payload[consumed..].to_vec()
        } else {
            Vec::new()
        };

        Ok((out, eof, unused))
    }

    fn zlib_compress_class(&self) -> Result<ObjRef, RuntimeError> {
        let module = self
            .modules
            .get("zlib")
            .ok_or_else(|| RuntimeError::new("module 'zlib' not found"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("invalid zlib module object"));
        };
        match module_data.globals.get("Compress") {
            Some(Value::Class(class)) => Ok(class.clone()),
            _ => Err(RuntimeError::new("zlib.Compress type is not available")),
        }
    }

    fn zlib_decompress_class(&self) -> Result<ObjRef, RuntimeError> {
        let module = self
            .modules
            .get("zlib")
            .ok_or_else(|| RuntimeError::new("module 'zlib' not found"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("invalid zlib module object"));
        };
        match module_data.globals.get("Decompress") {
            Some(Value::Class(class)) => Ok(class.clone()),
            _ => Err(RuntimeError::new("zlib.Decompress type is not available")),
        }
    }

    fn zlib_update_decompress_instance_attrs(&mut self, receiver: &ObjRef) {
        let Some(state) = self.zlib_decompress_objects.get(&receiver.id()).cloned() else {
            return;
        };
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .insert("eof".to_string(), Value::Bool(state.eof));
            instance_data
                .attrs
                .insert("unused_data".to_string(), self.heap.alloc_bytes(state.unused_data));
            instance_data.attrs.insert(
                "unconsumed_tail".to_string(),
                self.heap.alloc_bytes(state.unconsumed_tail),
            );
        }
    }

    pub(in crate::vm) fn builtin_zlib_compress(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (payload, level, wbits) = self.zlib_parse_compress_kwargs(&mut args, &mut kwargs)?;
        let out = self.zlib_deflate_bytes(
            &payload,
            level,
            wbits,
            Z_DEFAULT_MEM_LEVEL,
            Z_DEFAULT_STRATEGY,
        )?;
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_zlib_decompress(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: decompress() missing required argument 'data'",
            ));
        }
        if args.len() > 3 {
            return Err(RuntimeError::new(format!(
                "TypeError: decompress() takes at most 3 positional arguments ({} given)",
                args.len()
            )));
        }
        let payload = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::new("TypeError: a bytes-like object is required"))?;

        let wbits_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("wbits")
        };
        if !args.is_empty() {
            let _bufsize = args.remove(0);
        } else {
            let _ = kwargs.remove("bufsize");
        }
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: decompress() got an unexpected keyword argument '{key}'"
            )));
        }
        let wbits = Self::zlib_parse_optional_int(wbits_arg, Z_DEFAULT_WINDOW_BITS as i64, "wbits")?
            as c_int;
        let (out, _, _) = self.zlib_inflate_bytes(&payload, wbits, true)?;
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_zlib_crc32(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: crc32() missing required argument 'data'",
            ));
        }
        if args.len() > 2 {
            return Err(RuntimeError::new(format!(
                "TypeError: crc32() takes at most 2 positional arguments ({} given)",
                args.len()
            )));
        }
        let payload = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::new("TypeError: a bytes-like object is required"))?;
        let value_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("value")
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: crc32() got an unexpected keyword argument '{key}'"
            )));
        }
        let seed = Self::zlib_parse_optional_int(value_arg, 0, "value")? as u32;
        if payload.len() > c_uint::MAX as usize {
            return Err(RuntimeError::new("OverflowError: input is too large"));
        }
        let crc = unsafe {
            crc32(
                seed as c_ulong,
                if payload.is_empty() {
                    ptr::null()
                } else {
                    payload.as_ptr()
                },
                payload.len() as c_uint,
            )
        } as u32;
        Ok(Value::Int(i64::from(crc)))
    }

    pub(in crate::vm) fn builtin_zlib_compressobj(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 6 {
            return Err(RuntimeError::new(format!(
                "TypeError: compressobj() takes at most 6 positional arguments ({} given)",
                args.len()
            )));
        }

        let level = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("level")
        };
        let method = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("method")
        };
        let wbits = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("wbits")
        };
        let mem_level = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("memLevel")
        };
        let strategy = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("strategy")
        };
        let zdict = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("zdict")
        };

        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: compressobj() got an unexpected keyword argument '{key}'"
            )));
        }

        let level = Self::zlib_parse_optional_int(level, Z_DEFAULT_COMPRESSION as i64, "level")?
            as c_int;
        let method = Self::zlib_parse_optional_int(method, Z_DEFLATED as i64, "method")? as c_int;
        if method != Z_DEFLATED {
            return Err(RuntimeError::new("zlib.error: only DEFLATED method is supported"));
        }
        let wbits = Self::zlib_parse_optional_int(wbits, Z_DEFAULT_WINDOW_BITS as i64, "wbits")?
            as c_int;
        let mem_level =
            Self::zlib_parse_optional_int(mem_level, Z_DEFAULT_MEM_LEVEL as i64, "memLevel")?
                as c_int;
        let strategy =
            Self::zlib_parse_optional_int(strategy, Z_DEFAULT_STRATEGY as i64, "strategy")?
                as c_int;

        if let Some(zdict_value) = zdict
            && !matches!(zdict_value, Value::None) {
                return Err(RuntimeError::new(
                    "NotImplementedError: compressobj(zdict=...) is not implemented",
                ));
            }

        let class = self.zlib_compress_class()?;
        let instance = self.alloc_instance_for_class(&class);
        self.zlib_compress_objects.insert(
            instance.id(),
            ZlibCompressObjectState {
                level,
                wbits,
                mem_level,
                strategy,
                buffer: Vec::new(),
                finished: false,
            },
        );
        Ok(Value::Instance(instance))
    }

    pub(in crate::vm) fn builtin_zlib_decompressobj(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new(format!(
                "TypeError: decompressobj() takes at most 2 positional arguments ({} given)",
                args.len()
            )));
        }

        let wbits = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("wbits")
        };
        let zdict = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("zdict")
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: decompressobj() got an unexpected keyword argument '{key}'"
            )));
        }
        if let Some(zdict_value) = zdict
            && !matches!(zdict_value, Value::None) {
                return Err(RuntimeError::new(
                    "NotImplementedError: decompressobj(zdict=...) is not implemented",
                ));
            }

        let wbits = Self::zlib_parse_optional_int(wbits, Z_DEFAULT_WINDOW_BITS as i64, "wbits")?
            as c_int;
        let class = self.zlib_decompress_class()?;
        let instance = self.alloc_instance_for_class(&class);
        self.zlib_decompress_objects.insert(
            instance.id(),
            ZlibDecompressObjectState {
                wbits,
                eof: false,
                unused_data: Vec::new(),
                unconsumed_tail: Vec::new(),
            },
        );
        self.zlib_update_decompress_instance_attrs(&instance);
        Ok(Value::Instance(instance))
    }

    pub(in crate::vm) fn builtin_zlib_compress_object_compress(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: Compress.compress() does not accept keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new(format!(
                "TypeError: Compress.compress() takes exactly one data argument ({} given)",
                args.len().saturating_sub(1)
            )));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid Compress object")),
        };
        let payload = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::new("TypeError: a bytes-like object is required"))?;
        let Some(state) = self.zlib_compress_objects.get_mut(&receiver.id()) else {
            return Err(RuntimeError::new("TypeError: invalid Compress object"));
        };
        if state.finished {
            return Err(RuntimeError::new("zlib.error: inconsistent stream state"));
        }
        state.buffer.extend_from_slice(&payload);
        Ok(self.heap.alloc_bytes(Vec::new()))
    }

    pub(in crate::vm) fn builtin_zlib_compress_object_flush(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: Compress.flush() missing self argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid Compress object")),
        };
        let mode_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("mode")
        };
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: Compress.flush() received unexpected arguments",
            ));
        }
        let mode = Self::zlib_parse_optional_int(mode_arg, Z_FINISH as i64, "mode")? as c_int;
        if mode == Z_SYNC_FLUSH {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }
        if mode != Z_FINISH {
            return Err(RuntimeError::new(
                "NotImplementedError: only Z_SYNC_FLUSH and Z_FINISH modes are supported",
            ));
        }

        let (buffer, level, wbits, mem_level, strategy, already_finished) = {
            let Some(state) = self.zlib_compress_objects.get_mut(&receiver.id()) else {
                return Err(RuntimeError::new("TypeError: invalid Compress object"));
            };
            (
                state.buffer.clone(),
                state.level,
                state.wbits,
                state.mem_level,
                state.strategy,
                state.finished,
            )
        };
        if already_finished {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }
        let out = self.zlib_deflate_bytes(&buffer, level, wbits, mem_level, strategy)?;
        let Some(state) = self.zlib_compress_objects.get_mut(&receiver.id()) else {
            return Err(RuntimeError::new("TypeError: invalid Compress object"));
        };
        state.buffer.clear();
        state.finished = true;
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_zlib_decompress_object_decompress(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "TypeError: Decompress.decompress() missing required argument 'data'",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid Decompress object")),
        };
        let payload = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::new("TypeError: a bytes-like object is required"))?;

        let max_length_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("max_length")
        };
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: Decompress.decompress() received unexpected arguments",
            ));
        }
        let max_length = Self::zlib_parse_optional_int(max_length_arg, 0, "max_length")?;

        let state_wbits;
        {
            let Some(state) = self.zlib_decompress_objects.get_mut(&receiver.id()) else {
                return Err(RuntimeError::new("TypeError: invalid Decompress object"));
            };
            state_wbits = state.wbits;
            if state.eof {
                state.unused_data.extend_from_slice(&payload);
                self.zlib_update_decompress_instance_attrs(&receiver);
                return Ok(self.heap.alloc_bytes(Vec::new()));
            }
        }

        let (mut out, eof, unused) = self.zlib_inflate_bytes(&payload, state_wbits, false)?;
        if max_length > 0 && out.len() > max_length as usize {
            out.truncate(max_length as usize);
        }

        if let Some(state) = self.zlib_decompress_objects.get_mut(&receiver.id()) {
            state.eof = eof;
            state.unused_data = unused;
            state.unconsumed_tail = Vec::new();
        }
        self.zlib_update_decompress_instance_attrs(&receiver);

        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_zlib_decompress_object_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: Decompress.flush() does not accept keyword arguments",
            ));
        }
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: Decompress.flush() missing self argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid Decompress object")),
        };
        if !self.zlib_decompress_objects.contains_key(&receiver.id()) {
            return Err(RuntimeError::new("TypeError: invalid Decompress object"));
        }
        Ok(self.heap.alloc_bytes(Vec::new()))
    }

    pub(in crate::vm) fn zlib_version_string(&self) -> Option<String> {
        let ptr = unsafe { zlibVersion() };
        if ptr.is_null() {
            return None;
        }
        Some(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
    }
}
