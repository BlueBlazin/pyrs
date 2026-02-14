use super::super::{Vm, RuntimeError, Value, ObjRef, Object, HashMap, bytes_like_from_value};
use std::os::raw::{c_int, c_uint, c_void};

const LZMA_OK: c_uint = 0;
const LZMA_BUF_ERROR: c_uint = 10;

const FORMAT_AUTO: i64 = 0;
const FORMAT_XZ: i64 = 1;
const FORMAT_ALONE: i64 = 2;
const FORMAT_RAW: i64 = 3;

const CHECK_NONE: i64 = 0;
const CHECK_CRC32: i64 = 1;
const CHECK_CRC64: i64 = 4;
const CHECK_SHA256: i64 = 10;
const CHECK_ID_MAX: i64 = 15;
const CHECK_UNKNOWN: i64 = 16;

const PRESET_DEFAULT: i64 = 6;
const PRESET_EXTREME: i64 = 1 << 31;

#[derive(Clone)]
pub(in crate::vm) struct LzmaCompressorState {
    format: i64,
    check: i64,
    preset: c_uint,
    buffer: Vec<u8>,
    finished: bool,
}

#[derive(Clone)]
pub(in crate::vm) struct LzmaDecompressorState {
    format: i64,
    eof: bool,
    unused_data: Vec<u8>,
    needs_input: bool,
    check: i64,
}

#[link(name = "lzma")]
unsafe extern "C" {
    fn lzma_easy_buffer_encode(
        preset: c_uint,
        check: c_uint,
        allocator: *const c_void,
        input: *const u8,
        input_size: usize,
        output: *mut u8,
        output_pos: *mut usize,
        output_size: usize,
    ) -> c_uint;
    fn lzma_stream_buffer_decode(
        memlimit: *mut u64,
        flags: c_uint,
        allocator: *const c_void,
        input: *const u8,
        input_pos: *mut usize,
        input_size: usize,
        output: *mut u8,
        output_pos: *mut usize,
        output_size: usize,
    ) -> c_uint;
    fn lzma_check_is_supported(check: c_uint) -> c_int;
}

impl Vm {
    fn lzma_error(action: &str, code: c_uint) -> RuntimeError {
        RuntimeError::new(format!("_lzma error during {action}: code {code}"))
    }

    fn lzma_parse_optional_int(
        value: Option<Value>,
        default: i64,
        name: &str,
    ) -> Result<i64, RuntimeError> {
        match value {
            None => Ok(default),
            Some(Value::Int(v)) => Ok(v),
            Some(Value::Bool(v)) => Ok(if v { 1 } else { 0 }),
            Some(Value::None) if name == "preset" => Ok(default),
            Some(_) => Err(RuntimeError::new(format!(
                "TypeError: integer argument expected for '{name}'",
            ))),
        }
    }

    fn lzma_encode_xz(&self, payload: &[u8], check: i64, preset: c_uint) -> Result<Vec<u8>, RuntimeError> {
        let mut out_cap = payload.len().saturating_add(payload.len() / 3).saturating_add(256).max(64);
        loop {
            let mut out = vec![0u8; out_cap];
            let mut out_pos = 0usize;
            let code = unsafe {
                lzma_easy_buffer_encode(
                    preset,
                    check as c_uint,
                    std::ptr::null(),
                    if payload.is_empty() {
                        std::ptr::null()
                    } else {
                        payload.as_ptr()
                    },
                    payload.len(),
                    out.as_mut_ptr(),
                    &mut out_pos,
                    out.len(),
                )
            };
            if code == LZMA_OK {
                out.truncate(out_pos);
                return Ok(out);
            }
            if code == LZMA_BUF_ERROR {
                out_cap = out_cap.saturating_mul(2);
                continue;
            }
            return Err(Self::lzma_error("compress", code));
        }
    }

    fn lzma_decode_auto(
        &self,
        payload: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), RuntimeError> {
        let mut out_cap = payload.len().saturating_mul(6).saturating_add(1024).max(64);
        loop {
            let mut out = vec![0u8; out_cap];
            let mut out_pos = 0usize;
            let mut in_pos = 0usize;
            let mut memlimit = u64::MAX;
            let code = unsafe {
                lzma_stream_buffer_decode(
                    &mut memlimit,
                    0,
                    std::ptr::null(),
                    if payload.is_empty() {
                        std::ptr::null()
                    } else {
                        payload.as_ptr()
                    },
                    &mut in_pos,
                    payload.len(),
                    out.as_mut_ptr(),
                    &mut out_pos,
                    out.len(),
                )
            };
            if code == LZMA_OK {
                out.truncate(out_pos);
                let unused = if in_pos < payload.len() {
                    payload[in_pos..].to_vec()
                } else {
                    Vec::new()
                };
                return Ok((out, unused));
            }
            if code == LZMA_BUF_ERROR {
                out_cap = out_cap.saturating_mul(2);
                continue;
            }
            return Err(Self::lzma_error("decompress", code));
        }
    }

    fn lzma_update_decompressor_attrs(&mut self, receiver: &ObjRef) {
        let Some(state) = self.lzma_decompressors.get(&receiver.id()).cloned() else {
            return;
        };
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .insert("eof".to_string(), Value::Bool(state.eof));
            instance_data
                .attrs
                .insert("needs_input".to_string(), Value::Bool(state.needs_input));
            instance_data
                .attrs
                .insert("check".to_string(), Value::Int(state.check));
            instance_data
                .attrs
                .insert("unused_data".to_string(), self.heap.alloc_bytes(state.unused_data));
        }
    }

    pub(in crate::vm) fn builtin_lzma_compressor_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: LZMACompressor.__init__ missing self argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid LZMACompressor object")),
        };
        if args.len() > 4 {
            return Err(RuntimeError::new(format!(
                "TypeError: LZMACompressor.__init__ takes at most 4 positional arguments ({} given)",
                args.len()
            )));
        }
        let format_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("format")
        };
        let check_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("check")
        };
        let preset_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("preset")
        };
        let filters_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("filters")
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: LZMACompressor.__init__ got an unexpected keyword argument '{key}'",
            )));
        }

        let format = Self::lzma_parse_optional_int(format_arg, FORMAT_XZ, "format")?;
        if format != FORMAT_XZ {
            return Err(RuntimeError::new(
                "NotImplementedError: only FORMAT_XZ compression is supported",
            ));
        }
        if let Some(filters) = filters_arg
            && !matches!(filters, Value::None) {
                return Err(RuntimeError::new(
                    "NotImplementedError: LZMACompressor(filters=...) is not implemented",
                ));
            }

        let mut check = Self::lzma_parse_optional_int(check_arg, -1, "check")?;
        if check == -1 {
            check = CHECK_CRC64;
        }
        let preset = Self::lzma_parse_optional_int(preset_arg, PRESET_DEFAULT, "preset")?;
        if !(0..=(9 | PRESET_EXTREME)).contains(&preset) {
            return Err(RuntimeError::new(
                "ValueError: preset must be in range 0..9 (with optional PRESET_EXTREME)",
            ));
        }

        self.lzma_compressors.insert(
            receiver.id(),
            LzmaCompressorState {
                format,
                check,
                preset: preset as c_uint,
                buffer: Vec::new(),
                finished: false,
            },
        );
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_lzma_compressor_compress(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: LZMACompressor.compress() does not accept keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new(
                "TypeError: LZMACompressor.compress() takes exactly one data argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid LZMACompressor object")),
        };
        let payload = bytes_like_from_value(args.remove(0))
            .map_err(|_| RuntimeError::new("TypeError: a bytes-like object is required"))?;
        let Some(state) = self.lzma_compressors.get_mut(&receiver.id()) else {
            return Err(RuntimeError::new("TypeError: invalid LZMACompressor object"));
        };
        if state.finished {
            return Err(RuntimeError::new("ValueError: compressor object already flushed"));
        }
        state.buffer.extend_from_slice(&payload);
        Ok(self.heap.alloc_bytes(Vec::new()))
    }

    pub(in crate::vm) fn builtin_lzma_compressor_flush(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: LZMACompressor.flush() does not accept keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: LZMACompressor.flush() takes no arguments",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid LZMACompressor object")),
        };

        let (already_finished, format, check, preset, payload) = {
            let Some(state) = self.lzma_compressors.get(&receiver.id()) else {
                return Err(RuntimeError::new("TypeError: invalid LZMACompressor object"));
            };
            (
                state.finished,
                state.format,
                state.check,
                state.preset,
                state.buffer.clone(),
            )
        };
        if already_finished {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }
        if format != FORMAT_XZ {
            return Err(RuntimeError::new(
                "NotImplementedError: only FORMAT_XZ compression is supported",
            ));
        }
        let out = self.lzma_encode_xz(&payload, check, preset)?;
        if let Some(state) = self.lzma_compressors.get_mut(&receiver.id()) {
            state.finished = true;
            state.buffer.clear();
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_lzma_decompressor_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: LZMADecompressor.__init__ missing self argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid LZMADecompressor object")),
        };
        if args.len() > 3 {
            return Err(RuntimeError::new(format!(
                "TypeError: LZMADecompressor.__init__ takes at most 3 positional arguments ({} given)",
                args.len()
            )));
        }
        let format_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("format")
        };
        let _memlimit = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("memlimit")
        };
        let filters_arg = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            kwargs.remove("filters")
        };
        if !kwargs.is_empty() {
            let key = kwargs.keys().next().cloned().unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "TypeError: LZMADecompressor.__init__ got an unexpected keyword argument '{key}'",
            )));
        }

        let format = Self::lzma_parse_optional_int(format_arg, FORMAT_AUTO, "format")?;
        if format == FORMAT_RAW {
            return Err(RuntimeError::new(
                "NotImplementedError: FORMAT_RAW decompression is not supported",
            ));
        }
        if let Some(filters) = filters_arg
            && !matches!(filters, Value::None) {
                return Err(RuntimeError::new(
                    "NotImplementedError: LZMADecompressor(filters=...) is not implemented",
                ));
            }

        self.lzma_decompressors.insert(
            receiver.id(),
            LzmaDecompressorState {
                format,
                eof: false,
                unused_data: Vec::new(),
                needs_input: true,
                check: CHECK_UNKNOWN,
            },
        );
        self.lzma_update_decompressor_attrs(&receiver);
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_lzma_decompressor_decompress(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::new(
                "TypeError: LZMADecompressor.decompress() missing required argument 'data'",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => return Err(RuntimeError::new("TypeError: invalid LZMADecompressor object")),
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
                "TypeError: LZMADecompressor.decompress() received unexpected arguments",
            ));
        }
        let max_length = Self::lzma_parse_optional_int(max_length_arg, -1, "max_length")?;

        let (already_eof, format) = self
            .lzma_decompressors
            .get(&receiver.id())
            .map(|state| (state.eof, state.format))
            .unwrap_or((false, FORMAT_AUTO));
        if already_eof {
            if let Some(state) = self.lzma_decompressors.get_mut(&receiver.id()) {
                state.unused_data.extend_from_slice(&payload);
            }
            self.lzma_update_decompressor_attrs(&receiver);
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }
        if format == FORMAT_RAW {
            return Err(RuntimeError::new(
                "NotImplementedError: FORMAT_RAW decompression is not supported",
            ));
        }

        let (mut out, unused) = self.lzma_decode_auto(&payload)?;
        if max_length >= 0 && out.len() > max_length as usize {
            out.truncate(max_length as usize);
        }

        if let Some(state) = self.lzma_decompressors.get_mut(&receiver.id()) {
            state.eof = true;
            state.unused_data = unused;
            state.needs_input = true;
            state.check = CHECK_CRC64;
        }
        self.lzma_update_decompressor_attrs(&receiver);
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_lzma_is_check_supported(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "TypeError: is_check_supported() does not accept keyword arguments",
            ));
        }
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "TypeError: is_check_supported() takes exactly one argument",
            ));
        }
        let check = Self::lzma_parse_optional_int(Some(args.remove(0)), CHECK_NONE, "check")?;
        let supported = unsafe { lzma_check_is_supported(check as c_uint) } != 0;
        Ok(Value::Bool(supported))
    }

    pub(in crate::vm) fn builtin_lzma_encode_filter_properties(
        &mut self,
        _args: Vec<Value>,
        _kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        Err(RuntimeError::new(
            "NotImplementedError: _encode_filter_properties is not implemented",
        ))
    }

    pub(in crate::vm) fn builtin_lzma_decode_filter_properties(
        &mut self,
        _args: Vec<Value>,
        _kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        Err(RuntimeError::new(
            "NotImplementedError: _decode_filter_properties is not implemented",
        ))
    }

    pub(in crate::vm) fn lzma_constants() -> Vec<(&'static str, Value)> {
        vec![
            ("CHECK_NONE", Value::Int(CHECK_NONE)),
            ("CHECK_CRC32", Value::Int(CHECK_CRC32)),
            ("CHECK_CRC64", Value::Int(CHECK_CRC64)),
            ("CHECK_SHA256", Value::Int(CHECK_SHA256)),
            ("CHECK_ID_MAX", Value::Int(CHECK_ID_MAX)),
            ("CHECK_UNKNOWN", Value::Int(CHECK_UNKNOWN)),
            ("FILTER_LZMA1", Value::Int(0x4000_0000_0000_0001)),
            ("FILTER_LZMA2", Value::Int(0x21)),
            ("FILTER_DELTA", Value::Int(0x03)),
            ("FILTER_X86", Value::Int(0x04)),
            ("FILTER_IA64", Value::Int(0x05)),
            ("FILTER_ARM", Value::Int(0x07)),
            ("FILTER_ARMTHUMB", Value::Int(0x08)),
            ("FILTER_POWERPC", Value::Int(0x09)),
            ("FILTER_SPARC", Value::Int(0x0a)),
            ("FORMAT_AUTO", Value::Int(FORMAT_AUTO)),
            ("FORMAT_XZ", Value::Int(FORMAT_XZ)),
            ("FORMAT_ALONE", Value::Int(FORMAT_ALONE)),
            ("FORMAT_RAW", Value::Int(FORMAT_RAW)),
            ("MF_HC3", Value::Int(0x03)),
            ("MF_HC4", Value::Int(0x04)),
            ("MF_BT2", Value::Int(0x12)),
            ("MF_BT3", Value::Int(0x13)),
            ("MF_BT4", Value::Int(0x14)),
            ("MODE_FAST", Value::Int(0x01)),
            ("MODE_NORMAL", Value::Int(0x02)),
            ("PRESET_DEFAULT", Value::Int(PRESET_DEFAULT)),
            ("PRESET_EXTREME", Value::Int(PRESET_EXTREME)),
        ]
    }
}
