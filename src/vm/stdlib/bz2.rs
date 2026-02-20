use super::super::{HashMap, ObjRef, Object, RuntimeError, Value, Vm, bytes_like_from_value};
use std::os::raw::{c_char, c_int, c_uint};

const BZ_OK: c_int = 0;
const BZ_OUTBUFF_FULL: c_int = -8;

#[derive(Clone)]
pub(in crate::vm) struct Bz2CompressorState {
    level: c_int,
    buffer: Vec<u8>,
    finished: bool,
}

#[derive(Clone)]
pub(in crate::vm) struct Bz2DecompressorState {
    eof: bool,
    unused_data: Vec<u8>,
    needs_input: bool,
}

#[link(name = "bz2")]
unsafe extern "C" {
    fn BZ2_bzBuffToBuffCompress(
        dest: *mut c_char,
        dest_len: *mut c_uint,
        source: *mut c_char,
        source_len: c_uint,
        block_size_100k: c_int,
        verbosity: c_int,
        work_factor: c_int,
    ) -> c_int;
    fn BZ2_bzBuffToBuffDecompress(
        dest: *mut c_char,
        dest_len: *mut c_uint,
        source: *mut c_char,
        source_len: c_uint,
        small: c_int,
        verbosity: c_int,
    ) -> c_int;
}

impl Vm {
    fn bz2_error(action: &str, code: c_int) -> RuntimeError {
        RuntimeError::new(format!("_bz2 error during {action}: code {code}"))
    }

    fn bz2_parse_optional_int(
        value: Option<Value>,
        default: i64,
        name: &str,
    ) -> Result<i64, RuntimeError> {
        match value {
            None => Ok(default),
            Some(Value::Int(v)) => Ok(v),
            Some(Value::Bool(v)) => Ok(if v { 1 } else { 0 }),
            Some(_) => Err(RuntimeError::new(format!(
                "TypeError: integer argument expected for '{name}'",
            ))),
        }
    }

    fn bz2_compress_bytes(&self, payload: &[u8], level: c_int) -> Result<Vec<u8>, RuntimeError> {
        if payload.len() > c_uint::MAX as usize {
            return Err(RuntimeError::overflow_error("input is too large"));
        }
        let mut out_cap = payload
            .len()
            .saturating_add(payload.len() / 100)
            .saturating_add(601)
            .max(64);
        loop {
            if out_cap > c_uint::MAX as usize {
                return Err(RuntimeError::overflow_error("output buffer too large"));
            }
            let mut out = vec![0u8; out_cap];
            let mut out_len = out_cap as c_uint;
            let code = unsafe {
                BZ2_bzBuffToBuffCompress(
                    out.as_mut_ptr() as *mut c_char,
                    &mut out_len,
                    if payload.is_empty() {
                        std::ptr::null_mut()
                    } else {
                        payload.as_ptr() as *mut c_char
                    },
                    payload.len() as c_uint,
                    level,
                    0,
                    0,
                )
            };
            if code == BZ_OK {
                out.truncate(out_len as usize);
                return Ok(out);
            }
            if code == BZ_OUTBUFF_FULL {
                out_cap = out_cap.saturating_mul(2);
                continue;
            }
            return Err(Self::bz2_error("compress", code));
        }
    }

    fn bz2_decompress_bytes(&self, payload: &[u8]) -> Result<Vec<u8>, RuntimeError> {
        if payload.len() > c_uint::MAX as usize {
            return Err(RuntimeError::overflow_error("input is too large"));
        }
        let mut out_cap = payload.len().saturating_mul(6).saturating_add(1024).max(64);
        loop {
            if out_cap > c_uint::MAX as usize {
                return Err(RuntimeError::overflow_error("output buffer too large"));
            }
            let mut out = vec![0u8; out_cap];
            let mut out_len = out_cap as c_uint;
            let code = unsafe {
                BZ2_bzBuffToBuffDecompress(
                    out.as_mut_ptr() as *mut c_char,
                    &mut out_len,
                    if payload.is_empty() {
                        std::ptr::null_mut()
                    } else {
                        payload.as_ptr() as *mut c_char
                    },
                    payload.len() as c_uint,
                    0,
                    0,
                )
            };
            if code == BZ_OK {
                out.truncate(out_len as usize);
                return Ok(out);
            }
            if code == BZ_OUTBUFF_FULL {
                out_cap = out_cap.saturating_mul(2);
                continue;
            }
            return Err(Self::bz2_error("decompress", code));
        }
    }

    fn bz2_update_decompressor_attrs(&mut self, receiver: &ObjRef) {
        let Some(state) = self.bz2_decompressors.get(&receiver.id()).cloned() else {
            return;
        };
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .insert("eof".to_string(), Value::Bool(state.eof));
            instance_data
                .attrs
                .insert("needs_input".to_string(), Value::Bool(state.needs_input));
            instance_data.attrs.insert(
                "unused_data".to_string(),
                self.heap.alloc_bytes(state.unused_data),
            );
        }
    }

    pub(in crate::vm) fn builtin_bz2_compressor_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: BZ2Compressor.__init__ missing self argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::type_error("invalid BZ2Compressor object")),
        };
        if args.len() > 1 {
            return Err(RuntimeError::new(format!(
                "TypeError: BZ2Compressor.__init__ takes at most 1 positional argument ({} given)",
                args.len()
            )));
        }
        let level_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("compresslevel")
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: BZ2Compressor.__init__ got an unexpected keyword argument '{key}'",
            )));
        }
        let level = Self::bz2_parse_optional_int(level_arg, 9, "compresslevel")?;
        if !(1..=9).contains(&level) {
            return Err(RuntimeError::new(
                "ValueError: compresslevel must be between 1 and 9",
            ));
        }
        self.bz2_compressors.insert(
            receiver.id(),
            Bz2CompressorState {
                level: level as c_int,
                buffer: Vec::new(),
                finished: false,
            },
        );
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_bz2_compressor_compress(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: BZ2Compressor.compress() does not accept keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new(
                "TypeError: BZ2Compressor.compress() takes exactly one data argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::type_error("invalid BZ2Compressor object")),
        };
        let payload = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::type_error("a bytes-like object is required"))?;
        let Some(state) = self.bz2_compressors.get_mut(&receiver.id()) else {
            return Err(RuntimeError::type_error("invalid BZ2Compressor object"));
        };
        if state.finished {
            return Err(RuntimeError::new(
                "ValueError: compressor object already flushed",
            ));
        }
        state.buffer.extend_from_slice(&payload);
        Ok(self.heap.alloc_bytes(Vec::new()))
    }

    pub(in crate::vm) fn builtin_bz2_compressor_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: BZ2Compressor.flush() does not accept keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: BZ2Compressor.flush() takes no arguments",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::type_error("invalid BZ2Compressor object")),
        };

        let (already_finished, level, payload) = {
            let Some(state) = self.bz2_compressors.get(&receiver.id()) else {
                return Err(RuntimeError::type_error("invalid BZ2Compressor object"));
            };
            (state.finished, state.level, state.buffer.clone())
        };
        if already_finished {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }
        let out = self.bz2_compress_bytes(&payload, level)?;
        if let Some(state) = self.bz2_compressors.get_mut(&receiver.id()) {
            state.finished = true;
            state.buffer.clear();
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_bz2_decompressor_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: BZ2Decompressor.__init__ does not accept keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: BZ2Decompressor.__init__ takes no arguments",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: invalid BZ2Decompressor object",
                ));
            }
        };
        self.bz2_decompressors.insert(
            receiver.id(),
            Bz2DecompressorState {
                eof: false,
                unused_data: Vec::new(),
                needs_input: true,
            },
        );
        self.bz2_update_decompressor_attrs(&receiver);
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_bz2_decompressor_decompress(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "TypeError: BZ2Decompressor.decompress() missing required argument 'data'",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "TypeError: invalid BZ2Decompressor object",
                ));
            }
        };
        let payload = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::type_error("a bytes-like object is required"))?;
        let max_length_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("max_length")
        };
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: BZ2Decompressor.decompress() received unexpected arguments",
            ));
        }
        let max_length = Self::bz2_parse_optional_int(max_length_arg, -1, "max_length")?;

        let already_eof = self
            .bz2_decompressors
            .get(&receiver.id())
            .map(|state| state.eof)
            .unwrap_or(false);
        if already_eof {
            if let Some(state) = self.bz2_decompressors.get_mut(&receiver.id()) {
                state.unused_data.extend_from_slice(&payload);
            }
            self.bz2_update_decompressor_attrs(&receiver);
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }

        let mut out = self.bz2_decompress_bytes(&payload)?;
        if max_length >= 0 && out.len() > max_length as usize {
            out.truncate(max_length as usize);
        }

        if let Some(state) = self.bz2_decompressors.get_mut(&receiver.id()) {
            state.eof = true;
            state.needs_input = true;
            state.unused_data.clear();
        }
        self.bz2_update_decompressor_attrs(&receiver);
        Ok(self.heap.alloc_bytes(out))
    }
}
