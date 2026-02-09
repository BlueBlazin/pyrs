use super::super::*;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Default, Clone, Copy)]
struct PickleProfileStat {
    calls: u64,
    total_ns: u128,
    max_ns: u128,
}

struct PickleProfileGuard {
    name: &'static str,
    start: Option<Instant>,
}

impl Drop for PickleProfileGuard {
    fn drop(&mut self) {
        let Some(start) = self.start else {
            return;
        };
        pickle_profile_record(self.name, start.elapsed().as_nanos());
    }
}

static PICKLE_PROFILE_ENABLED: OnceLock<bool> = OnceLock::new();
static PICKLE_PROFILE_EMIT_EVERY: OnceLock<u64> = OnceLock::new();
static PICKLE_PROFILE_STATS: OnceLock<Mutex<BTreeMap<&'static str, PickleProfileStat>>> =
    OnceLock::new();
static PICKLE_PROFILE_EVENTS: AtomicU64 = AtomicU64::new(0);
const PICKLE_BUFFER_RELEASED_ATTR: &str = "__pyrs_picklebuffer_released__";
const PICKLE_DEFAULT_PROTOCOL: i64 = 5;
const PICKLE_MIN_FAST_PROTOCOL: i64 = 4;
const PICKLE_MAX_FAST_PROTOCOL: i64 = 5;
const PICKLE_FRAME_SIZE_TARGET: usize = 65_536;
const PICKLE_BATCH_SIZE: usize = 1_000;
const PICKLER_FILE_ATTR: &str = "__pyrs_pickle_file__";
const PICKLER_PROTOCOL_ATTR: &str = "__pyrs_pickle_protocol__";
const PICKLER_FIX_IMPORTS_ATTR: &str = "__pyrs_pickle_fix_imports__";
const PICKLER_BUFFER_CALLBACK_ATTR: &str = "__pyrs_pickle_buffer_callback__";
const PICKLER_FALLBACK_ATTR: &str = "__pyrs_pickle_fallback__";
const UNPICKLER_FILE_ATTR: &str = "__pyrs_unpickle_file__";
const UNPICKLER_FIX_IMPORTS_ATTR: &str = "__pyrs_unpickle_fix_imports__";
const UNPICKLER_ENCODING_ATTR: &str = "__pyrs_unpickle_encoding__";
const UNPICKLER_ERRORS_ATTR: &str = "__pyrs_unpickle_errors__";
const UNPICKLER_BUFFERS_ATTR: &str = "__pyrs_unpickle_buffers__";
const UNPICKLER_FALLBACK_ATTR: &str = "__pyrs_unpickle_fallback__";

#[derive(Default)]
struct FastPickleEncoder {
    payload: Vec<u8>,
    seen_container_ids: HashSet<u64>,
    depth: usize,
}

#[derive(Clone, Copy)]
enum PickleCallKind {
    Dump,
    Dumps,
    Load,
    Loads,
}

fn pickle_profile_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn pickle_profile_enabled() -> bool {
    *PICKLE_PROFILE_ENABLED.get_or_init(|| pickle_profile_flag("PYRS_PROFILE_PICKLE"))
}

fn pickle_profile_emit_every() -> u64 {
    *PICKLE_PROFILE_EMIT_EVERY.get_or_init(|| {
        std::env::var("PYRS_PROFILE_PICKLE_EMIT_EVERY")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1_000)
            .max(1)
    })
}

fn pickle_profile_scope(name: &'static str) -> PickleProfileGuard {
    if !pickle_profile_enabled() {
        return PickleProfileGuard { name, start: None };
    }
    PickleProfileGuard {
        name,
        start: Some(Instant::now()),
    }
}

fn pickle_profile_record(name: &'static str, elapsed_ns: u128) {
    let stats = PICKLE_PROFILE_STATS.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut guard) = stats.lock() {
        let entry = guard.entry(name).or_default();
        entry.calls += 1;
        entry.total_ns += elapsed_ns;
        if elapsed_ns > entry.max_ns {
            entry.max_ns = elapsed_ns;
        }
    }
    let event_count = PICKLE_PROFILE_EVENTS.fetch_add(1, AtomicOrdering::Relaxed) + 1;
    let emit_every = pickle_profile_emit_every();
    if event_count % emit_every == 0 {
        pickle_profile_emit_summary(event_count);
    }
}

fn pickle_profile_emit_summary(event_count: u64) {
    let Some(stats) = PICKLE_PROFILE_STATS.get() else {
        return;
    };
    let Ok(guard) = stats.lock() else {
        return;
    };
    let mut rows: Vec<(&'static str, PickleProfileStat)> =
        guard.iter().map(|(name, stat)| (*name, *stat)).collect();
    rows.sort_by(|a, b| b.1.total_ns.cmp(&a.1.total_ns));
    eprintln!("pickle-prof summary: events={event_count}");
    for (name, stat) in rows.into_iter().take(8) {
        let total_ms = stat.total_ns as f64 / 1_000_000.0;
        let avg_us = if stat.calls == 0 {
            0.0
        } else {
            (stat.total_ns as f64 / stat.calls as f64) / 1_000.0
        };
        let max_us = stat.max_ns as f64 / 1_000.0;
        eprintln!(
            "pickle-prof {name}: calls={} total_ms={:.3} avg_us={:.3} max_us={:.3}",
            stat.calls, total_ms, avg_us, max_us
        );
    }
}

impl FastPickleEncoder {
    fn new() -> Self {
        Self {
            payload: Vec::new(),
            seen_container_ids: HashSet::new(),
            depth: 0,
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        self.payload.extend_from_slice(bytes);
    }

    fn push_byte(&mut self, byte: u8) {
        self.payload.push(byte);
    }

    fn encode_long_i64(&mut self, value: i64) {
        let mut n = value as i128;
        let mut out = Vec::new();
        loop {
            out.push((n & 0xff) as u8);
            let sign_bit_set = out.last().copied().unwrap_or_default() & 0x80 != 0;
            n >>= 8;
            if (n == 0 && !sign_bit_set) || (n == -1 && sign_bit_set) {
                break;
            }
        }
        if out.len() < 256 {
            self.push_byte(0x8a); // LONG1
            self.push_byte(out.len() as u8);
        } else {
            self.push_byte(0x8b); // LONG4
            self.push_bytes(&(out.len() as u32).to_le_bytes());
        }
        self.push_bytes(&out);
    }

    fn encode_bytes_payload(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if bytes.len() < 256 {
            self.push_byte(b'C'); // SHORT_BINBYTES
            self.push_byte(bytes.len() as u8);
            self.push_bytes(bytes);
            return Ok(());
        }
        if bytes.len() <= u32::MAX as usize {
            self.push_byte(b'B'); // BINBYTES
            self.push_bytes(&(bytes.len() as u32).to_le_bytes());
            self.push_bytes(bytes);
            return Ok(());
        }
        Err(())
    }

    fn encode_unicode_payload(&mut self, text: &str) -> Result<(), ()> {
        let bytes = text.as_bytes();
        if bytes.len() < 256 {
            self.push_byte(0x8c); // SHORT_BINUNICODE
            self.push_byte(bytes.len() as u8);
            self.push_bytes(bytes);
            return Ok(());
        }
        if bytes.len() <= u32::MAX as usize {
            self.push_byte(b'X'); // BINUNICODE
            self.push_bytes(&(bytes.len() as u32).to_le_bytes());
            self.push_bytes(bytes);
            return Ok(());
        }
        Err(())
    }
}

impl Vm {
    fn object_reduce_ex_builtin_singleton_name(&self, value: &Value) -> Option<&'static str> {
        let Value::Instance(instance) = value else {
            return None;
        };
        let Some(Value::Instance(ellipsis)) = self.builtins.get("Ellipsis") else {
            return None;
        };
        if instance.id() == ellipsis.id() {
            return Some("Ellipsis");
        }
        let Some(Value::Instance(not_implemented)) = self.builtins.get("NotImplemented") else {
            return None;
        };
        if instance.id() == not_implemented.id() {
            return Some("NotImplemented");
        }
        None
    }

    fn pickle_resolve_pure_symbol(&mut self, symbol: &str) -> Result<Value, RuntimeError> {
        if let Some(cached) = self.pickle_symbol_cache.get(symbol) {
            return Ok(cached.clone());
        }
        let caller_depth = self.frames.len();
        let pickle_module = Value::Module(self.import_module_object("pickle")?);
        self.run_pending_import_frames(caller_depth)?;
        let resolved = self.builtin_getattr(
            vec![pickle_module, Value::Str(symbol.to_string())],
            HashMap::new(),
        )?;
        self.pickle_symbol_cache
            .insert(symbol.to_string(), resolved.clone());
        Ok(resolved)
    }

    fn pickle_call_pure_symbol(
        &mut self,
        symbol: &str,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
        fallback_message: &str,
    ) -> Result<Value, RuntimeError> {
        let callable = self.pickle_resolve_pure_symbol(symbol)?;
        match self.call_internal(callable, args, kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => Err(RuntimeError::new(fallback_message)),
        }
    }

    fn pickle_protocol_from_value(&self, value: Value) -> Result<i64, RuntimeError> {
        if matches!(value, Value::None) {
            return Ok(PICKLE_DEFAULT_PROTOCOL);
        }
        value_to_int(value)
    }

    fn pickle_extract_bool_kwarg(
        kwargs: &HashMap<String, Value>,
        name: &str,
        default: bool,
    ) -> bool {
        kwargs
            .get(name)
            .map(is_truthy)
            .unwrap_or(default)
    }

    fn pickle_extract_protocol(
        &self,
        args: &[Value],
        kwargs: &HashMap<String, Value>,
        positional_index: usize,
    ) -> Result<i64, RuntimeError> {
        if let Some(value) = kwargs.get("protocol") {
            return self.pickle_protocol_from_value(value.clone());
        }
        if args.len() > positional_index {
            return self.pickle_protocol_from_value(args[positional_index].clone());
        }
        Ok(PICKLE_DEFAULT_PROTOCOL)
    }

    fn pickle_kwargs_are_simple(kwargs: &HashMap<String, Value>, kind: PickleCallKind) -> bool {
        let mut allowed = HashSet::new();
        match kind {
            PickleCallKind::Dump | PickleCallKind::Dumps => {
                allowed.insert("protocol");
                allowed.insert("fix_imports");
                allowed.insert("buffer_callback");
            }
            PickleCallKind::Load | PickleCallKind::Loads => {
                allowed.insert("fix_imports");
                allowed.insert("encoding");
                allowed.insert("errors");
                allowed.insert("buffers");
            }
        }
        kwargs.keys().all(|key| allowed.contains(key.as_str()))
    }

    fn fast_pickle_decode_i64(bytes: &[u8]) -> Option<i64> {
        if bytes.is_empty() {
            return Some(0);
        }
        if bytes.len() > 16 {
            return None;
        }
        let mut value: i128 = 0;
        for (idx, byte) in bytes.iter().enumerate() {
            value |= (*byte as i128) << (idx * 8);
        }
        if bytes.last().copied().unwrap_or_default() & 0x80 != 0 {
            value -= 1_i128 << (bytes.len() * 8);
        }
        i64::try_from(value).ok()
    }

    fn fast_pickle_encode_value(
        &self,
        encoder: &mut FastPickleEncoder,
        value: &Value,
    ) -> Result<(), ()> {
        if encoder.depth > 512 {
            return Err(());
        }
        encoder.depth += 1;
        let result = match value {
            Value::None => {
                encoder.push_byte(b'N');
                Ok(())
            }
            Value::Bool(flag) => {
                encoder.push_byte(if *flag { 0x88 } else { 0x89 });
                Ok(())
            }
            Value::Int(number) => {
                if (0..=255).contains(number) {
                    encoder.push_byte(b'K'); // BININT1
                    encoder.push_byte(*number as u8);
                } else if i32::try_from(*number).is_ok() {
                    encoder.push_byte(b'J'); // BININT
                    encoder.push_bytes(&(*number as i32).to_le_bytes());
                } else {
                    encoder.encode_long_i64(*number);
                }
                Ok(())
            }
            Value::Str(text) => encoder.encode_unicode_payload(text),
            Value::Bytes(bytes_obj) => {
                let payload = match &*bytes_obj.kind() {
                    Object::Bytes(bytes) => bytes.clone(),
                    _ => return Err(()),
                };
                encoder.encode_bytes_payload(&payload)
            }
            Value::Tuple(tuple_obj) => {
                if !encoder.seen_container_ids.insert(tuple_obj.id()) {
                    return Err(());
                }
                let values = match &*tuple_obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => return Err(()),
                };
                match values.len() {
                    0 => encoder.push_byte(b')'), // EMPTY_TUPLE
                    1 => {
                        self.fast_pickle_encode_value(encoder, &values[0])?;
                        encoder.push_byte(0x85); // TUPLE1
                    }
                    2 => {
                        self.fast_pickle_encode_value(encoder, &values[0])?;
                        self.fast_pickle_encode_value(encoder, &values[1])?;
                        encoder.push_byte(0x86); // TUPLE2
                    }
                    3 => {
                        self.fast_pickle_encode_value(encoder, &values[0])?;
                        self.fast_pickle_encode_value(encoder, &values[1])?;
                        self.fast_pickle_encode_value(encoder, &values[2])?;
                        encoder.push_byte(0x87); // TUPLE3
                    }
                    _ => {
                        encoder.push_byte(b'('); // MARK
                        for item in values {
                            self.fast_pickle_encode_value(encoder, &item)?;
                        }
                        encoder.push_byte(b't'); // TUPLE
                    }
                }
                Ok(())
            }
            Value::List(list_obj) => {
                if !encoder.seen_container_ids.insert(list_obj.id()) {
                    return Err(());
                }
                let values = match &*list_obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => return Err(()),
                };
                encoder.push_byte(b']'); // EMPTY_LIST
                for chunk in values.chunks(PICKLE_BATCH_SIZE) {
                    if chunk.is_empty() {
                        continue;
                    }
                    encoder.push_byte(b'('); // MARK
                    for item in chunk {
                        self.fast_pickle_encode_value(encoder, item)?;
                    }
                    encoder.push_byte(b'e'); // APPENDS
                }
                Ok(())
            }
            Value::Dict(dict_obj) => {
                if !encoder.seen_container_ids.insert(dict_obj.id()) {
                    return Err(());
                }
                let pairs = match &*dict_obj.kind() {
                    Object::Dict(entries) => entries.iter().cloned().collect::<Vec<_>>(),
                    _ => return Err(()),
                };
                encoder.push_byte(b'}'); // EMPTY_DICT
                for chunk in pairs.chunks(PICKLE_BATCH_SIZE) {
                    if chunk.is_empty() {
                        continue;
                    }
                    encoder.push_byte(b'('); // MARK
                    for (key, value) in chunk {
                        self.fast_pickle_encode_value(encoder, key)?;
                        self.fast_pickle_encode_value(encoder, value)?;
                    }
                    encoder.push_byte(b'u'); // SETITEMS
                }
                Ok(())
            }
            _ => Err(()),
        };
        encoder.depth -= 1;
        result
    }

    fn fast_pickle_encode_chunks(
        &self,
        value: &Value,
        protocol: i64,
    ) -> Option<Vec<Vec<u8>>> {
        if !(PICKLE_MIN_FAST_PROTOCOL..=PICKLE_MAX_FAST_PROTOCOL).contains(&protocol) {
            return None;
        }
        let mut encoder = FastPickleEncoder::new();
        self.fast_pickle_encode_value(&mut encoder, value).ok()?;
        encoder.push_byte(b'.'); // STOP
        let boundaries = Self::fast_pickle_opcode_boundaries(&encoder.payload)?;
        let mut chunks = Vec::new();
        chunks.push(vec![0x80, protocol as u8]); // PROTO
        let mut frame_start = 0usize;
        while frame_start < encoder.payload.len() {
            let target_end = (frame_start + PICKLE_FRAME_SIZE_TARGET).min(encoder.payload.len());
            let mut frame_end = target_end;
            if target_end < encoder.payload.len() {
                let candidate = boundaries
                    .iter()
                    .copied()
                    .filter(|boundary| *boundary > frame_start && *boundary <= target_end)
                    .max();
                frame_end = if let Some(boundary) = candidate {
                    boundary
                } else {
                    boundaries
                        .iter()
                        .copied()
                        .find(|boundary| *boundary > frame_start)
                        .unwrap_or(encoder.payload.len())
                };
            }
            let frame_payload = &encoder.payload[frame_start..frame_end];
            let mut frame_header = Vec::with_capacity(9);
            frame_header.push(0x95); // FRAME
            frame_header.extend_from_slice(&(frame_payload.len() as u64).to_le_bytes());
            chunks.push(frame_header);
            chunks.push(frame_payload.to_vec());
            frame_start = frame_end;
        }
        Some(chunks)
    }

    fn fast_pickle_graph_has_alias(value: &Value, seen: &mut HashSet<u64>) -> bool {
        match value {
            Value::List(obj) => {
                if !seen.insert(obj.id()) {
                    return true;
                }
                let elements = match &*obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => return true,
                };
                elements
                    .iter()
                    .any(|element| Self::fast_pickle_graph_has_alias(element, seen))
            }
            Value::Tuple(obj) => {
                if !seen.insert(obj.id()) {
                    return true;
                }
                let elements = match &*obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => return true,
                };
                elements
                    .iter()
                    .any(|element| Self::fast_pickle_graph_has_alias(element, seen))
            }
            Value::Dict(obj) => {
                if !seen.insert(obj.id()) {
                    return true;
                }
                let entries = match &*obj.kind() {
                    Object::Dict(entries) => entries
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect::<Vec<_>>(),
                    _ => return true,
                };
                entries.iter().any(|(key, value)| {
                    Self::fast_pickle_graph_has_alias(key, seen)
                        || Self::fast_pickle_graph_has_alias(value, seen)
                })
            }
            Value::Bytes(obj) => !seen.insert(obj.id()),
            _ => false,
        }
    }

    fn fast_pickle_graph_is_alias_free(value: &Value) -> bool {
        let mut seen = HashSet::new();
        !Self::fast_pickle_graph_has_alias(value, &mut seen)
    }

    fn fast_pickle_graph_has_large_payload(value: &Value, seen: &mut HashSet<u64>) -> bool {
        match value {
            Value::Str(text) => text.len() >= PICKLE_FRAME_SIZE_TARGET,
            Value::Bytes(obj) => {
                if !seen.insert(obj.id()) {
                    return false;
                }
                match &*obj.kind() {
                    Object::Bytes(bytes) => bytes.len() >= PICKLE_FRAME_SIZE_TARGET,
                    _ => true,
                }
            }
            Value::List(obj) => {
                if !seen.insert(obj.id()) {
                    return false;
                }
                let values = match &*obj.kind() {
                    Object::List(values) => values.clone(),
                    _ => return true,
                };
                values
                    .iter()
                    .any(|item| Self::fast_pickle_graph_has_large_payload(item, seen))
            }
            Value::Tuple(obj) => {
                if !seen.insert(obj.id()) {
                    return false;
                }
                let values = match &*obj.kind() {
                    Object::Tuple(values) => values.clone(),
                    _ => return true,
                };
                values
                    .iter()
                    .any(|item| Self::fast_pickle_graph_has_large_payload(item, seen))
            }
            Value::Dict(obj) => {
                if !seen.insert(obj.id()) {
                    return false;
                }
                let entries = match &*obj.kind() {
                    Object::Dict(entries) => entries
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect::<Vec<_>>(),
                    _ => return true,
                };
                entries.iter().any(|(key, value)| {
                    Self::fast_pickle_graph_has_large_payload(key, seen)
                        || Self::fast_pickle_graph_has_large_payload(value, seen)
                })
            }
            _ => false,
        }
    }

    fn fast_pickle_graph_is_small_payload(value: &Value) -> bool {
        let mut seen = HashSet::new();
        !Self::fast_pickle_graph_has_large_payload(value, &mut seen)
    }

    fn fast_pickle_opcode_boundaries(payload: &[u8]) -> Option<Vec<usize>> {
        let mut boundaries = Vec::new();
        let mut idx = 0usize;
        boundaries.push(0);
        while idx < payload.len() {
            let opcode = payload[idx];
            idx += 1;
            match opcode {
                b'N' | 0x88 | 0x89 | b']' | b'}' | b')' | b'(' | b'e' | b'u' | b't' | 0x85
                | 0x86 | 0x87 | b'.' => {}
                b'K' => {
                    idx += 1;
                }
                b'J' => {
                    idx += 4;
                }
                0x8a => {
                    let len = *payload.get(idx)? as usize;
                    idx += 1 + len;
                }
                0x8b => {
                    let len_bytes: [u8; 4] = payload.get(idx..idx + 4)?.try_into().ok()?;
                    idx += 4 + u32::from_le_bytes(len_bytes) as usize;
                }
                0x8c => {
                    let len = *payload.get(idx)? as usize;
                    idx += 1 + len;
                }
                b'X' | b'B' => {
                    let len_bytes: [u8; 4] = payload.get(idx..idx + 4)?.try_into().ok()?;
                    idx += 4 + u32::from_le_bytes(len_bytes) as usize;
                }
                b'C' => {
                    let len = *payload.get(idx)? as usize;
                    idx += 1 + len;
                }
                _ => return None,
            }
            if idx > payload.len() {
                return None;
            }
            boundaries.push(idx);
        }
        Some(boundaries)
    }

    fn pickle_write_chunks_to_file(
        &mut self,
        file: Value,
        chunks: Vec<Vec<u8>>,
    ) -> Result<(), RuntimeError> {
        let write_method = self.builtin_getattr(
            vec![file, Value::Str("write".to_string())],
            HashMap::new(),
        )?;
        for chunk in chunks {
            let payload = self.heap.alloc_bytes(chunk);
            match self.call_internal(write_method.clone(), vec![payload], HashMap::new())? {
                InternalCallOutcome::Value(_) => {}
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("pickle write callback failed"));
                }
            }
        }
        Ok(())
    }

    fn pickle_extract_bytes_like(&self, value: &Value) -> Option<Vec<u8>> {
        match value {
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => Some(bytes.clone()),
                _ => None,
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(bytes) => Some(bytes.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn fast_pickle_decode_payload(&mut self, payload: &[u8]) -> Option<Value> {
        let mut stack: Vec<Value> = Vec::new();
        let mut marks: Vec<usize> = Vec::new();
        let mut idx = 0usize;
        while idx < payload.len() {
            let opcode = payload[idx];
            idx += 1;
            match opcode {
                b'N' => stack.push(Value::None),
                0x88 => stack.push(Value::Bool(true)),
                0x89 => stack.push(Value::Bool(false)),
                b'K' => {
                    let value = *payload.get(idx)? as i64;
                    idx += 1;
                    stack.push(Value::Int(value));
                }
                b'J' => {
                    let bytes: [u8; 4] = payload.get(idx..idx + 4)?.try_into().ok()?;
                    idx += 4;
                    stack.push(Value::Int(i32::from_le_bytes(bytes) as i64));
                }
                0x8a => {
                    let len = *payload.get(idx)? as usize;
                    idx += 1;
                    let data = payload.get(idx..idx + len)?;
                    idx += len;
                    stack.push(Value::Int(Self::fast_pickle_decode_i64(data)?));
                }
                0x8b => {
                    let len_bytes: [u8; 4] = payload.get(idx..idx + 4)?.try_into().ok()?;
                    idx += 4;
                    let len = u32::from_le_bytes(len_bytes) as usize;
                    let data = payload.get(idx..idx + len)?;
                    idx += len;
                    stack.push(Value::Int(Self::fast_pickle_decode_i64(data)?));
                }
                0x8c => {
                    let len = *payload.get(idx)? as usize;
                    idx += 1;
                    let data = payload.get(idx..idx + len)?;
                    idx += len;
                    stack.push(Value::Str(String::from_utf8(data.to_vec()).ok()?));
                }
                b'X' => {
                    let len_bytes: [u8; 4] = payload.get(idx..idx + 4)?.try_into().ok()?;
                    idx += 4;
                    let len = u32::from_le_bytes(len_bytes) as usize;
                    let data = payload.get(idx..idx + len)?;
                    idx += len;
                    stack.push(Value::Str(String::from_utf8(data.to_vec()).ok()?));
                }
                b'C' => {
                    let len = *payload.get(idx)? as usize;
                    idx += 1;
                    let data = payload.get(idx..idx + len)?;
                    idx += len;
                    stack.push(self.heap.alloc_bytes(data.to_vec()));
                }
                b'B' => {
                    let len_bytes: [u8; 4] = payload.get(idx..idx + 4)?.try_into().ok()?;
                    idx += 4;
                    let len = u32::from_le_bytes(len_bytes) as usize;
                    let data = payload.get(idx..idx + len)?;
                    idx += len;
                    stack.push(self.heap.alloc_bytes(data.to_vec()));
                }
                b']' => stack.push(self.heap.alloc_list(Vec::new())),
                b'}' => stack.push(self.heap.alloc_dict(Vec::new())),
                b')' => stack.push(self.heap.alloc_tuple(Vec::new())),
                b'(' => marks.push(stack.len()),
                0x85 => {
                    let one = stack.pop()?;
                    stack.push(self.heap.alloc_tuple(vec![one]));
                }
                0x86 => {
                    let two = stack.pop()?;
                    let one = stack.pop()?;
                    stack.push(self.heap.alloc_tuple(vec![one, two]));
                }
                0x87 => {
                    let three = stack.pop()?;
                    let two = stack.pop()?;
                    let one = stack.pop()?;
                    stack.push(self.heap.alloc_tuple(vec![one, two, three]));
                }
                b't' => {
                    let mark = marks.pop()?;
                    let items = stack.split_off(mark);
                    stack.push(self.heap.alloc_tuple(items));
                }
                b'e' => {
                    let mark = marks.pop()?;
                    let items = stack.split_off(mark);
                    let list_obj = match stack.last().cloned()? {
                        Value::List(obj) => obj,
                        _ => return None,
                    };
                    if let Object::List(values) = &mut *list_obj.kind_mut() {
                        values.extend(items);
                    } else {
                        return None;
                    }
                }
                b'u' => {
                    let mark = marks.pop()?;
                    let items = stack.split_off(mark);
                    if items.len() % 2 != 0 {
                        return None;
                    }
                    let dict_obj = match stack.last().cloned()? {
                        Value::Dict(obj) => obj,
                        _ => return None,
                    };
                    if let Object::Dict(entries) = &mut *dict_obj.kind_mut() {
                        for pair in items.chunks_exact(2) {
                            entries.insert(pair[0].clone(), pair[1].clone());
                        }
                    } else {
                        return None;
                    }
                }
                b'.' => {
                    if idx != payload.len() {
                        return None;
                    }
                    return stack.pop();
                }
                _ => return None,
            }
        }
        None
    }

    fn fast_pickle_decode_bytes(&mut self, data: &[u8]) -> Option<Value> {
        if data.len() < 2 || data[0] != 0x80 {
            return None;
        }
        let protocol = data[1] as i64;
        if !(PICKLE_MIN_FAST_PROTOCOL..=PICKLE_MAX_FAST_PROTOCOL).contains(&protocol) {
            return None;
        }
        let mut idx = 2usize;
        let mut payload = Vec::new();
        while idx < data.len() {
            if data[idx] != 0x95 {
                return None;
            }
            idx += 1;
            let len_bytes: [u8; 8] = data.get(idx..idx + 8)?.try_into().ok()?;
            idx += 8;
            let len = u64::from_le_bytes(len_bytes) as usize;
            let frame = data.get(idx..idx + len)?;
            idx += len;
            payload.extend_from_slice(frame);
        }
        self.fast_pickle_decode_payload(&payload)
    }

    fn pickle_store_instance_attr(
        instance: &ObjRef,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let mut kind = instance.kind_mut();
        let Object::Instance(instance_data) = &mut *kind else {
            return Err(RuntimeError::new("descriptor requires an instance"));
        };
        instance_data.attrs.insert(name.to_string(), value);
        Ok(())
    }

    fn pickle_get_instance_attr(instance: &ObjRef, name: &str) -> Option<Value> {
        let kind = instance.kind();
        let Object::Instance(instance_data) = &*kind else {
            return None;
        };
        instance_data.attrs.get(name).cloned()
    }

    fn pickle_get_pickler_dispatch_table(&mut self, instance: &ObjRef) -> Option<Value> {
        let instance_value = Value::Instance(instance.clone());
        let has_dispatch_table = self
            .builtin_hasattr(
                vec![
                    instance_value.clone(),
                    Value::Str("dispatch_table".to_string()),
                ],
                HashMap::new(),
            )
            .ok();
        if !matches!(has_dispatch_table, Some(Value::Bool(true))) {
            return None;
        }
        self.builtin_getattr(
            vec![instance_value, Value::Str("dispatch_table".to_string())],
            HashMap::new(),
        )
        .ok()
    }

    fn pickle_copyreg_callable(&mut self, attr_name: &str) -> Result<Value, RuntimeError> {
        let _profile = pickle_profile_scope("pickle_copyreg_callable");
        if let Some(cached) = self.pickle_copyreg_cache.get(attr_name) {
            return Ok(cached.clone());
        }
        let caller_depth = self.frames.len();
        let copyreg_module = Value::Module(self.import_module_object("copyreg")?);
        self.run_pending_import_frames(caller_depth)?;
        let resolved = self.builtin_getattr(
            vec![copyreg_module, Value::Str(attr_name.to_string())],
            HashMap::new(),
        )?;
        self.pickle_copyreg_cache
            .insert(attr_name.to_string(), resolved.clone());
        Ok(resolved)
    }

    fn picklebuffer_storage_from_value(&self, value: Value) -> Result<Value, RuntimeError> {
        match value {
            Value::Bytes(_) | Value::ByteArray(_) => Ok(value),
            Value::MemoryView(view) => match &*view.kind() {
                Object::MemoryView(view_data) => match &*view_data.source.kind() {
                    Object::Bytes(_) => Ok(Value::Bytes(view_data.source.clone())),
                    Object::ByteArray(_) => Ok(Value::ByteArray(view_data.source.clone())),
                    _ => Err(RuntimeError::new(
                        "PickleBuffer() argument must be a bytes-like object",
                    )),
                },
                _ => Err(RuntimeError::new(
                    "PickleBuffer() argument must be a bytes-like object",
                )),
            },
            Value::Instance(instance) => {
                let kind = instance.kind();
                let Object::Instance(instance_data) = &*kind else {
                    return Err(RuntimeError::new(
                        "PickleBuffer() argument must be a bytes-like object",
                    ));
                };
                match instance_data.attrs.get(BYTES_BACKING_STORAGE_ATTR) {
                    Some(Value::Bytes(storage)) => Ok(Value::Bytes(storage.clone())),
                    Some(Value::ByteArray(storage)) => Ok(Value::ByteArray(storage.clone())),
                    _ => Err(RuntimeError::new(
                        "PickleBuffer() argument must be a bytes-like object",
                    )),
                }
            }
            _ => Err(RuntimeError::new(
                "PickleBuffer() argument must be a bytes-like object",
            )),
        }
    }

    pub(in crate::vm) fn builtin_picklebuffer_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "PickleBuffer.__init__() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "PickleBuffer.__init__")?;
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "PickleBuffer.__init__() expects one argument",
            ));
        }
        let storage = self.picklebuffer_storage_from_value(args.remove(0))?;
        {
            let mut instance_kind = instance.kind_mut();
            let Object::Instance(instance_data) = &mut *instance_kind else {
                return Err(RuntimeError::new(
                    "PickleBuffer.__init__() descriptor requires an instance",
                ));
            };
            instance_data
                .attrs
                .insert(BYTES_BACKING_STORAGE_ATTR.to_string(), storage);
            instance_data
                .attrs
                .insert(PICKLE_BUFFER_RELEASED_ATTR.to_string(), Value::Bool(false));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_picklebuffer_release(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "PickleBuffer.release() takes no keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "PickleBuffer.release")?;
        if !args.is_empty() {
            return Err(RuntimeError::new("PickleBuffer.release() expects no arguments"));
        }
        {
            let mut instance_kind = instance.kind_mut();
            let Object::Instance(instance_data) = &mut *instance_kind else {
                return Err(RuntimeError::new(
                    "PickleBuffer.release() descriptor requires an instance",
                ));
            };
            instance_data.attrs.remove(BYTES_BACKING_STORAGE_ATTR);
            instance_data
                .attrs
                .insert(PICKLE_BUFFER_RELEASED_ATTR.to_string(), Value::Bool(true));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_pickle_module_getattr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_pickle.__getattr__() expects one attribute name",
            ));
        }
        let Value::Str(attr_name) = &args[0] else {
            return Err(RuntimeError::new(
                "_pickle.__getattr__() attribute name must be str",
            ));
        };

        let target_attr = match attr_name.as_str() {
            "Pickler" => "_Pickler",
            "Unpickler" => "_Unpickler",
            "dump" => "_dump",
            "dumps" => "_dumps",
            "load" => "_load",
            "loads" => "_loads",
            _ => {
                return Err(RuntimeError::new(format!(
                    "AttributeError: module '_pickle' has no attribute '{}'",
                    attr_name
                )))
            }
        };

        let caller_depth = self.frames.len();
        let pickle_module = Value::Module(self.import_module_object("pickle")?);
        self.run_pending_import_frames(caller_depth)?;
        self.builtin_getattr(
            vec![pickle_module, Value::Str(target_attr.to_string())],
            HashMap::new(),
        )
    }

    pub(in crate::vm) fn builtin_pickle_dump(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return self.pickle_call_pure_symbol(
                "_dump",
                args,
                kwargs,
                "pickle dump fallback failed",
            );
        }
        let raw_args = args.clone();
        let raw_kwargs = kwargs.clone();
        if !Self::pickle_kwargs_are_simple(&kwargs, PickleCallKind::Dump) {
            return self.pickle_call_pure_symbol(
                "_dump",
                raw_args,
                raw_kwargs,
                "pickle dump fallback failed",
            );
        }
        let protocol = self.pickle_extract_protocol(&args, &kwargs, 2)?;
        let fix_imports = Self::pickle_extract_bool_kwarg(&kwargs, "fix_imports", true);
        let buffer_callback = kwargs.get("buffer_callback").cloned().unwrap_or(Value::None);
        if fix_imports
            && matches!(buffer_callback, Value::None)
            && (PICKLE_MIN_FAST_PROTOCOL..=PICKLE_MAX_FAST_PROTOCOL).contains(&protocol)
            && Self::fast_pickle_graph_is_alias_free(&args[0])
            && Self::fast_pickle_graph_is_small_payload(&args[0])
        {
            if let Some(chunks) = self.fast_pickle_encode_chunks(&args[0], protocol) {
                self.pickle_write_chunks_to_file(args[1].clone(), chunks)?;
                return Ok(Value::None);
            }
        }
        self.pickle_call_pure_symbol("_dump", raw_args, raw_kwargs, "pickle dump fallback failed")
    }

    pub(in crate::vm) fn builtin_pickle_dumps(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return self.pickle_call_pure_symbol(
                "_dumps",
                args,
                kwargs,
                "pickle dumps fallback failed",
            );
        }
        let raw_args = args.clone();
        let raw_kwargs = kwargs.clone();
        if !Self::pickle_kwargs_are_simple(&kwargs, PickleCallKind::Dumps) {
            return self.pickle_call_pure_symbol(
                "_dumps",
                raw_args,
                raw_kwargs,
                "pickle dumps fallback failed",
            );
        }
        let protocol = self.pickle_extract_protocol(&args, &kwargs, 1)?;
        let fix_imports = Self::pickle_extract_bool_kwarg(&kwargs, "fix_imports", true);
        let buffer_callback = kwargs.get("buffer_callback").cloned().unwrap_or(Value::None);
        if fix_imports
            && matches!(buffer_callback, Value::None)
            && (PICKLE_MIN_FAST_PROTOCOL..=PICKLE_MAX_FAST_PROTOCOL).contains(&protocol)
            && Self::fast_pickle_graph_is_alias_free(&args[0])
            && Self::fast_pickle_graph_is_small_payload(&args[0])
        {
            if let Some(chunks) = self.fast_pickle_encode_chunks(&args[0], protocol) {
                let mut payload = Vec::new();
                for chunk in chunks {
                    payload.extend_from_slice(&chunk);
                }
                return Ok(self.heap.alloc_bytes(payload));
            }
        }
        self.pickle_call_pure_symbol(
            "_dumps",
            raw_args,
            raw_kwargs,
            "pickle dumps fallback failed",
        )
    }

    pub(in crate::vm) fn builtin_pickle_loads(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return self.pickle_call_pure_symbol(
                "_loads",
                args,
                kwargs,
                "pickle loads fallback failed",
            );
        }
        let raw_args = args.clone();
        let raw_kwargs = kwargs.clone();
        if Self::pickle_kwargs_are_simple(&kwargs, PickleCallKind::Loads)
            && Self::pickle_extract_bool_kwarg(&kwargs, "fix_imports", true)
            && kwargs
                .get("encoding")
                .is_none_or(|value| matches!(value, Value::Str(name) if name == "ASCII"))
            && kwargs
                .get("errors")
                .is_none_or(|value| matches!(value, Value::Str(name) if name == "strict"))
            && kwargs.get("buffers").is_none_or(|value| matches!(value, Value::None))
        {
            if let Some(bytes) = self.pickle_extract_bytes_like(&args[0]) {
                if let Some(value) = self.fast_pickle_decode_bytes(&bytes) {
                    return Ok(value);
                }
            }
        }
        self.pickle_call_pure_symbol(
            "_loads",
            raw_args,
            raw_kwargs,
            "pickle loads fallback failed",
        )
    }

    pub(in crate::vm) fn builtin_pickle_load(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return self.pickle_call_pure_symbol(
                "_load",
                args,
                kwargs,
                "pickle load fallback failed",
            );
        }
        let raw_args = args.clone();
        let raw_kwargs = kwargs.clone();
        if Self::pickle_kwargs_are_simple(&kwargs, PickleCallKind::Load)
            && Self::pickle_extract_bool_kwarg(&kwargs, "fix_imports", true)
            && kwargs
                .get("encoding")
                .is_none_or(|value| matches!(value, Value::Str(name) if name == "ASCII"))
            && kwargs
                .get("errors")
                .is_none_or(|value| matches!(value, Value::Str(name) if name == "strict"))
            && kwargs.get("buffers").is_none_or(|value| matches!(value, Value::None))
        {
            let file = args[0].clone();
            let tell_method = self.builtin_getattr(
                vec![file.clone(), Value::Str("tell".to_string())],
                HashMap::new(),
            );
            let seek_method = self.builtin_getattr(
                vec![file.clone(), Value::Str("seek".to_string())],
                HashMap::new(),
            );
            let read_method = self.builtin_getattr(
                vec![file.clone(), Value::Str("read".to_string())],
                HashMap::new(),
            );
            if let (Ok(tell_method), Ok(seek_method), Ok(read_method)) =
                (tell_method, seek_method, read_method)
            {
                let start = match self.call_internal(tell_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => Value::None,
                };
                let raw = match self.call_internal(read_method, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(value) => value,
                    InternalCallOutcome::CallerExceptionHandled => Value::None,
                };
                if let Some(bytes) = self.pickle_extract_bytes_like(&raw) {
                    if let Some(value) = self.fast_pickle_decode_bytes(&bytes) {
                        return Ok(value);
                    }
                }
                let _ = self.call_internal(seek_method, vec![start], HashMap::new());
            }
        }
        self.pickle_call_pure_symbol(
            "_load",
            raw_args,
            raw_kwargs,
            "pickle load fallback failed",
        )
    }

    fn pickle_pickler_build_fallback(&mut self, instance: &ObjRef) -> Result<Value, RuntimeError> {
        if let Some(existing) = Self::pickle_get_instance_attr(instance, PICKLER_FALLBACK_ATTR) {
            if !matches!(existing, Value::None) {
                if let Some(dispatch_table) = self.pickle_get_pickler_dispatch_table(instance) {
                    self.builtin_setattr(
                        vec![
                            existing.clone(),
                            Value::Str("dispatch_table".to_string()),
                            dispatch_table,
                        ],
                        HashMap::new(),
                    )?;
                }
                return Ok(existing);
            }
        }
        let file = Self::pickle_get_instance_attr(instance, PICKLER_FILE_ATTR)
            .ok_or_else(|| RuntimeError::new("Pickler.__init__() was not called by Pickler.__init__"))?;
        let protocol = Self::pickle_get_instance_attr(instance, PICKLER_PROTOCOL_ATTR)
            .ok_or_else(|| RuntimeError::new("Pickler.__init__() was not called by Pickler.__init__"))?;
        let fix_imports =
            Self::pickle_get_instance_attr(instance, PICKLER_FIX_IMPORTS_ATTR).unwrap_or(Value::Bool(true));
        let buffer_callback =
            Self::pickle_get_instance_attr(instance, PICKLER_BUFFER_CALLBACK_ATTR).unwrap_or(Value::None);
        let pickler_class = self.pickle_resolve_pure_symbol("_Pickler")?;
        let mut kwargs = HashMap::new();
        kwargs.insert("fix_imports".to_string(), fix_imports);
        kwargs.insert("buffer_callback".to_string(), buffer_callback);
        let pure = match self.call_internal(pickler_class, vec![file, protocol], kwargs)? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(RuntimeError::new("Pickler fallback construction failed"));
            }
        };
        if let Some(dispatch_table) = self.pickle_get_pickler_dispatch_table(instance) {
            self.builtin_setattr(
                vec![
                    pure.clone(),
                    Value::Str("dispatch_table".to_string()),
                    dispatch_table,
                ],
                HashMap::new(),
            )?;
        }
        Self::pickle_store_instance_attr(instance, PICKLER_FALLBACK_ATTR, pure.clone())?;
        Ok(pure)
    }

    fn pickle_unpickler_build_fallback(
        &mut self,
        instance: &ObjRef,
    ) -> Result<Value, RuntimeError> {
        if let Some(existing) = Self::pickle_get_instance_attr(instance, UNPICKLER_FALLBACK_ATTR) {
            if !matches!(existing, Value::None) {
                return Ok(existing);
            }
        }
        let file = Self::pickle_get_instance_attr(instance, UNPICKLER_FILE_ATTR)
            .ok_or_else(|| RuntimeError::new("Unpickler.__init__() was not called by Unpickler.__init__"))?;
        let fix_imports =
            Self::pickle_get_instance_attr(instance, UNPICKLER_FIX_IMPORTS_ATTR).unwrap_or(Value::Bool(true));
        let encoding = Self::pickle_get_instance_attr(instance, UNPICKLER_ENCODING_ATTR)
            .unwrap_or_else(|| Value::Str("ASCII".to_string()));
        let errors = Self::pickle_get_instance_attr(instance, UNPICKLER_ERRORS_ATTR)
            .unwrap_or_else(|| Value::Str("strict".to_string()));
        let buffers = Self::pickle_get_instance_attr(instance, UNPICKLER_BUFFERS_ATTR)
            .unwrap_or(Value::None);
        let unpickler_class = self.pickle_resolve_pure_symbol("_Unpickler")?;
        let mut kwargs = HashMap::new();
        kwargs.insert("fix_imports".to_string(), fix_imports);
        kwargs.insert("encoding".to_string(), encoding);
        kwargs.insert("errors".to_string(), errors);
        kwargs.insert("buffers".to_string(), buffers);
        let pure = match self.call_internal(unpickler_class, vec![file], kwargs)? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(RuntimeError::new("Unpickler fallback construction failed"));
            }
        };
        Self::pickle_store_instance_attr(instance, UNPICKLER_FALLBACK_ATTR, pure.clone())?;
        Ok(pure)
    }

    pub(in crate::vm) fn builtin_pickle_pickler_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "Pickler.__init__")?;
        if args.is_empty() {
            return Err(RuntimeError::new(
                "Pickler.__init__() missing required argument 'file'",
            ));
        }
        let file = args.remove(0);
        let protocol = if let Some(value) = kwargs.remove("protocol") {
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::Int(PICKLE_DEFAULT_PROTOCOL)
        };
        let fix_imports = kwargs
            .remove("fix_imports")
            .unwrap_or(Value::Bool(true));
        let buffer_callback = kwargs
            .remove("buffer_callback")
            .unwrap_or(Value::None);
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new("Pickler.__init__() received unexpected arguments"));
        }
        Self::pickle_store_instance_attr(&instance, PICKLER_FILE_ATTR, file)?;
        Self::pickle_store_instance_attr(&instance, PICKLER_PROTOCOL_ATTR, protocol)?;
        Self::pickle_store_instance_attr(&instance, PICKLER_FIX_IMPORTS_ATTR, fix_imports)?;
        Self::pickle_store_instance_attr(
            &instance,
            PICKLER_BUFFER_CALLBACK_ATTR,
            buffer_callback,
        )?;
        Self::pickle_store_instance_attr(&instance, PICKLER_FALLBACK_ATTR, Value::None)?;
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_pickle_pickler_dump(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("Pickler.dump() takes no keyword arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Pickler.dump")?;
        if args.len() != 1 {
            return Err(RuntimeError::new("Pickler.dump() expects one argument"));
        }
        let file = Self::pickle_get_instance_attr(&instance, PICKLER_FILE_ATTR)
            .ok_or_else(|| RuntimeError::new("Pickler.__init__() was not called by Pickler.__init__"))?;
        let protocol = Self::pickle_get_instance_attr(&instance, PICKLER_PROTOCOL_ATTR)
            .ok_or_else(|| RuntimeError::new("Pickler.__init__() was not called by Pickler.__init__"))?;
        let protocol = self.pickle_protocol_from_value(protocol)?;
        let fix_imports = Self::pickle_extract_bool_kwarg(
            &HashMap::from([(
                "fix_imports".to_string(),
                Self::pickle_get_instance_attr(&instance, PICKLER_FIX_IMPORTS_ATTR)
                    .unwrap_or(Value::Bool(true)),
            )]),
            "fix_imports",
            true,
        );
        let buffer_callback =
            Self::pickle_get_instance_attr(&instance, PICKLER_BUFFER_CALLBACK_ATTR).unwrap_or(Value::None);
        let has_dispatch_table = self.pickle_get_pickler_dispatch_table(&instance).is_some();
        if fix_imports
            && matches!(buffer_callback, Value::None)
            && (PICKLE_MIN_FAST_PROTOCOL..=PICKLE_MAX_FAST_PROTOCOL).contains(&protocol)
            && !has_dispatch_table
            && Self::fast_pickle_graph_is_alias_free(&args[0])
            && Self::fast_pickle_graph_is_small_payload(&args[0])
        {
            if let Some(chunks) = self.fast_pickle_encode_chunks(&args[0], protocol) {
                self.pickle_write_chunks_to_file(file, chunks)?;
                return Ok(Value::None);
            }
        }
        let fallback = self.pickle_pickler_build_fallback(&instance)?;
        let dump_method = self.builtin_getattr(
            vec![fallback, Value::Str("dump".to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(dump_method, vec![args.remove(0)], HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("Pickler.dump() fallback failed"))
            }
        }
    }

    pub(in crate::vm) fn builtin_pickle_unpickler_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "Unpickler.__init__")?;
        if args.is_empty() {
            return Err(RuntimeError::new(
                "Unpickler.__init__() missing required argument 'file'",
            ));
        }
        let file = args.remove(0);
        let fix_imports = kwargs
            .remove("fix_imports")
            .unwrap_or(Value::Bool(true));
        let encoding = kwargs
            .remove("encoding")
            .unwrap_or_else(|| Value::Str("ASCII".to_string()));
        let errors = kwargs
            .remove("errors")
            .unwrap_or_else(|| Value::Str("strict".to_string()));
        let buffers = kwargs.remove("buffers").unwrap_or(Value::None);
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "Unpickler.__init__() received unexpected arguments",
            ));
        }
        Self::pickle_store_instance_attr(&instance, UNPICKLER_FILE_ATTR, file)?;
        Self::pickle_store_instance_attr(&instance, UNPICKLER_FIX_IMPORTS_ATTR, fix_imports)?;
        Self::pickle_store_instance_attr(&instance, UNPICKLER_ENCODING_ATTR, encoding)?;
        Self::pickle_store_instance_attr(&instance, UNPICKLER_ERRORS_ATTR, errors)?;
        Self::pickle_store_instance_attr(&instance, UNPICKLER_BUFFERS_ATTR, buffers)?;
        Self::pickle_store_instance_attr(&instance, UNPICKLER_FALLBACK_ATTR, Value::None)?;
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_pickle_unpickler_load(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("Unpickler.load() takes no keyword arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Unpickler.load")?;
        if !args.is_empty() {
            return Err(RuntimeError::new("Unpickler.load() expects no arguments"));
        }
        let file = Self::pickle_get_instance_attr(&instance, UNPICKLER_FILE_ATTR)
            .ok_or_else(|| RuntimeError::new("Unpickler.__init__() was not called by Unpickler.__init__"))?;
        let fix_imports = Self::pickle_extract_bool_kwarg(
            &HashMap::from([(
                "fix_imports".to_string(),
                Self::pickle_get_instance_attr(&instance, UNPICKLER_FIX_IMPORTS_ATTR)
                    .unwrap_or(Value::Bool(true)),
            )]),
            "fix_imports",
            true,
        );
        let encoding = Self::pickle_get_instance_attr(&instance, UNPICKLER_ENCODING_ATTR)
            .unwrap_or_else(|| Value::Str("ASCII".to_string()));
        let errors = Self::pickle_get_instance_attr(&instance, UNPICKLER_ERRORS_ATTR)
            .unwrap_or_else(|| Value::Str("strict".to_string()));
        let buffers = Self::pickle_get_instance_attr(&instance, UNPICKLER_BUFFERS_ATTR)
            .unwrap_or(Value::None);
        if fix_imports
            && matches!(encoding, Value::Str(ref text) if text == "ASCII")
            && matches!(errors, Value::Str(ref text) if text == "strict")
            && matches!(buffers, Value::None)
        {
            let read_method = self.builtin_getattr(
                vec![file.clone(), Value::Str("read".to_string())],
                HashMap::new(),
            );
            if let Ok(read_method) = read_method {
                let bytes_value =
                    match self.call_internal(read_method, Vec::new(), HashMap::new())? {
                        InternalCallOutcome::Value(value) => value,
                        InternalCallOutcome::CallerExceptionHandled => Value::None,
                    };
                if let Some(bytes) = self.pickle_extract_bytes_like(&bytes_value) {
                    if let Some(value) = self.fast_pickle_decode_bytes(&bytes) {
                        return Ok(value);
                    }
                }
            }
        }
        let fallback = self.pickle_unpickler_build_fallback(&instance)?;
        let load_method = self.builtin_getattr(
            vec![fallback, Value::Str("load".to_string())],
            HashMap::new(),
        )?;
        match self.call_internal(load_method, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("Unpickler.load() fallback failed"))
            }
        }
    }

    pub(in crate::vm) fn builtin_copyreg_newobj(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() {
            return Err(RuntimeError::new("__newobj__() expects class and args"));
        }
        let class_value = args.remove(0);
        let new_method = self.builtin_getattr(
            vec![class_value.clone(), Value::Str("__new__".to_string())],
            HashMap::new(),
        )?;
        let mut new_args = Vec::with_capacity(1 + args.len());
        new_args.push(class_value);
        new_args.extend(args);
        match self.call_internal(new_method, new_args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("__newobj__() failed"))
            }
        }
    }

    pub(in crate::vm) fn builtin_copyreg_newobj_ex(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(RuntimeError::new(
                "__newobj_ex__() expects class, args tuple, kwargs dict",
            ));
        }
        let class_value = args.remove(0);
        let tuple_args = match args.remove(0) {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("__newobj_ex__ args must be tuple")),
            },
            _ => return Err(RuntimeError::new("__newobj_ex__ args must be tuple")),
        };
        let call_kwargs = match args.remove(0) {
            Value::Dict(obj) => match &*obj.kind() {
                Object::Dict(entries) => {
                    let mut out = HashMap::new();
                    for (key, value) in entries {
                        let Value::Str(name) = key else {
                            return Err(RuntimeError::new(
                                "__newobj_ex__ kwargs keys must be strings",
                            ));
                        };
                        out.insert(name.clone(), value.clone());
                    }
                    out
                }
                _ => return Err(RuntimeError::new("__newobj_ex__ kwargs must be dict")),
            },
            _ => return Err(RuntimeError::new("__newobj_ex__ kwargs must be dict")),
        };
        let new_method = self.builtin_getattr(
            vec![class_value.clone(), Value::Str("__new__".to_string())],
            HashMap::new(),
        )?;
        let mut new_args = Vec::with_capacity(1 + tuple_args.len());
        new_args.push(class_value);
        new_args.extend(tuple_args);
        match self.call_internal(new_method, new_args, call_kwargs)? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("__newobj_ex__() failed"))
            }
        }
    }

    pub(in crate::vm) fn builtin_copyreg_reconstructor(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(RuntimeError::new(
                "_reconstructor() expects class, base, and state",
            ));
        }
        let class_value = args.remove(0);
        let base = args.remove(0);
        let state = args.remove(0);

        let base_is_object = self
            .builtins
            .get("object")
            .is_some_and(|object_type| *object_type == base);

        let new_target = if base_is_object { class_value.clone() } else { base };
        let new_method = self.builtin_getattr(
            vec![new_target, Value::Str("__new__".to_string())],
            HashMap::new(),
        )?;
        let mut new_args = vec![class_value];
        if !base_is_object {
            new_args.push(state);
        }
        match self.call_internal(new_method, new_args, HashMap::new())? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(RuntimeError::new("_reconstructor() failed"))
            }
        }
    }

    fn object_reduce_ex_new_constructor_and_args(
        &mut self,
        value: &Value,
        protocol: i64,
    ) -> Result<Option<(Value, Value)>, RuntimeError> {
        let _profile = pickle_profile_scope("object_reduce_ex_new_constructor_and_args");
        let Value::Instance(_) = value else {
            return Ok(None);
        };
        let Some(class_obj) = self.class_of_value(value).map(Value::Class) else {
            return Ok(None);
        };

        if let Some(getnewargs_ex) = self.lookup_bound_special_method(value, "__getnewargs_ex__")?
        {
            let payload = match self.call_internal(getnewargs_ex, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("__getnewargs_ex__ callback failed"));
                }
            };
            let Value::Tuple(pair_obj) = payload else {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ should return a tuple of length 2",
                ));
            };
            let Object::Tuple(pair_values) = &*pair_obj.kind() else {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ should return a tuple of length 2",
                ));
            };
            if pair_values.len() != 2 {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ should return a tuple of length 2",
                ));
            }
            let (args_tuple, kwargs_dict) = match (&pair_values[0], &pair_values[1]) {
                (Value::Tuple(args_tuple), Value::Dict(kwargs_dict)) => {
                    (args_tuple.clone(), kwargs_dict.clone())
                }
                _ => {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ should return (tuple, dict)",
                    ))
                }
            };

            let tuple_values = match &*args_tuple.kind() {
                Object::Tuple(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ should return (tuple, dict)",
                    ))
                }
            };
            let kwargs_entries = match &*kwargs_dict.kind() {
                Object::Dict(entries) => entries.iter().cloned().collect::<Vec<_>>(),
                _ => {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ should return (tuple, dict)",
                    ))
                }
            };

            if protocol >= 4 {
                let constructor_args = self.heap.alloc_tuple(vec![
                    class_obj,
                    Value::Tuple(args_tuple),
                    Value::Dict(kwargs_dict),
                ]);
                return Ok(Some((
                    self.pickle_copyreg_callable("__newobj_ex__")?,
                    constructor_args,
                )));
            }

            // Protocols <4 lack NEWOBJ_EX. For int-subclass compatibility we lower
            // __getnewargs_ex__ to int(cls, *args, base?) positional form.
            if !matches!(value, Value::Instance(instance) if self.instance_backing_int(instance).is_some())
            {
                return Err(RuntimeError::new(
                    "__getnewargs_ex__ kwargs require protocol >= 4",
                ));
            }
            let mut constructor_args = Vec::with_capacity(1 + tuple_values.len() + kwargs_entries.len());
            constructor_args.push(class_obj);
            constructor_args.extend(tuple_values);
            for (key, value) in kwargs_entries {
                let Value::Str(name) = key else {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ kwargs keys must be strings",
                    ));
                };
                if name == "base" {
                    constructor_args.push(value);
                } else {
                    return Err(RuntimeError::new(
                        "__getnewargs_ex__ kwargs require protocol >= 4",
                    ));
                }
            }
            return Ok(Some((
                Value::Builtin(BuiltinFunction::Int),
                self.heap.alloc_tuple(constructor_args),
            )));
        }

        if let Some(getnewargs) = self.lookup_bound_special_method(value, "__getnewargs__")? {
            let payload = match self.call_internal(getnewargs, Vec::new(), HashMap::new())? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("__getnewargs__ callback failed"));
                }
            };
            let Value::Tuple(args_obj) = payload else {
                return Err(RuntimeError::new("__getnewargs__ should return a tuple"));
            };
            let Object::Tuple(args_values) = &*args_obj.kind() else {
                return Err(RuntimeError::new("__getnewargs__ should return a tuple"));
            };
            let mut constructor_args = Vec::with_capacity(args_values.len() + 1);
            constructor_args.push(class_obj);
            constructor_args.extend(args_values.clone());
            return Ok(Some((
                self.pickle_copyreg_callable("__newobj__")?,
                self.heap.alloc_tuple(constructor_args),
            )));
        }

        if let Value::Instance(instance) = value {
            if let Some(integer_value) = self.instance_backing_int(instance) {
                let int_value = BuiltinFunction::Int.call(&self.heap, vec![integer_value])?;
                return Ok(Some((
                    self.pickle_copyreg_callable("__newobj__")?,
                    self.heap.alloc_tuple(vec![class_obj, int_value]),
                )));
            }
            // Default protocol >=2 constructor path for user instances:
            // use __newobj__(cls, *args) so unpickling bypasses __init__.
            return Ok(Some((
                self.pickle_copyreg_callable("__newobj__")?,
                self.heap.alloc_tuple(vec![class_obj]),
            )));
        }

        Ok(None)
    }

    fn object_reduce_ex_legacy_constructor_and_args(
        &mut self,
        value: &Value,
    ) -> Result<Option<(Value, Value)>, RuntimeError> {
        let _profile = pickle_profile_scope("object_reduce_ex_legacy_constructor_and_args");
        let Value::Instance(instance) = value else {
            return Ok(None);
        };
        let Some(class_obj) = self.class_of_value(value).map(Value::Class) else {
            return Ok(None);
        };
        let constructor_args = if let Some(integer_value) = self.instance_backing_int(instance) {
            let int_value = BuiltinFunction::Int.call(&self.heap, vec![integer_value])?;
            self.heap.alloc_tuple(vec![
                class_obj,
                Value::Builtin(BuiltinFunction::Int),
                int_value,
            ])
        } else {
            // For protocol 0/1, regular user instances must use copyreg._reconstructor.
            // Emitting (Class, ()) here incorrectly routes through __init__ on load.
            let base = self
                .builtins
                .get("object")
                .cloned()
                .ok_or_else(|| RuntimeError::new("object type is unavailable"))?;
            self.heap.alloc_tuple(vec![class_obj, base, Value::None])
        };
        Ok(Some((
            self.pickle_copyreg_callable("_reconstructor")?,
            constructor_args,
        )))
    }

    fn instance_has_non_object_reduce(&self, instance: &ObjRef) -> bool {
        let _profile = pickle_profile_scope("instance_has_non_object_reduce");
        let class = match &*instance.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return false,
        };
        for entry in self.class_mro_entries(&class) {
            let Object::Class(class_data) = &*entry.kind() else {
                continue;
            };
            let Some(attr) = class_data.attrs.get("__reduce__") else {
                continue;
            };
            return !matches!(
                attr,
                Value::Builtin(BuiltinFunction::ObjectReduceEx | BuiltinFunction::ObjectReduce)
                    if class_data.name == "object"
            );
        }
        false
    }

    fn object_reduce_ex_custom_reduce(
        &mut self,
        value: &Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let _profile = pickle_profile_scope("object_reduce_ex_custom_reduce");
        let Value::Instance(instance) = value else {
            return Ok(None);
        };
        if !self.instance_has_non_object_reduce(instance) {
            return Ok(None);
        }
        let Some(reduce_callable) = self.lookup_bound_special_method(value, "__reduce__")? else {
            return Ok(None);
        };
        let reduced = match self.call_internal(reduce_callable, Vec::new(), HashMap::new())? {
            InternalCallOutcome::Value(value) => value,
            InternalCallOutcome::CallerExceptionHandled => {
                return Err(RuntimeError::new("__reduce__ callback failed"));
            }
        };
        if matches!(reduced, Value::Str(_)) {
            return Ok(Some(reduced));
        }
        if let Value::Tuple(obj) = &reduced {
            let tuple_len = {
                let Object::Tuple(values) = &*obj.kind() else {
                    return Err(RuntimeError::new("__reduce__ must return a tuple"));
                };
                values.len()
            };
            if !(2..=6).contains(&tuple_len) {
                return Err(RuntimeError::new(
                    "tuple returned by __reduce__ must contain 2 through 6 elements",
                ));
            }
            return Ok(Some(reduced));
        }
        Err(RuntimeError::new("__reduce__ must return a string or tuple"))
    }

    pub(in crate::vm) fn builtin_object_getstate(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "object.__getstate__() takes exactly one argument",
            ));
        }
        let Some(value) = args.first() else {
            return Ok(Value::None);
        };
        match value {
            Value::Instance(instance) => match &*instance.kind() {
                Object::Instance(instance_data) => {
                    let entries = Self::instance_dict_entries(instance_data);
                    if entries.is_empty() {
                        Ok(Value::None)
                    } else {
                        Ok(self.heap.alloc_dict(entries))
                    }
                }
                _ => Ok(Value::None),
            },
            _ => Ok(Value::None),
        }
    }

    pub(in crate::vm) fn builtin_object_setstate(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "object.__setstate__() takes exactly two arguments",
            ));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new(
                "object.__setstate__() requires an instance receiver",
            ));
        };
        let apply_state_dict =
            |instance: &ObjRef, state: &ObjRef| -> Result<(), RuntimeError> {
                let entries: Vec<(Value, Value)> = match &*state.kind() {
                    Object::Dict(entries) => entries.iter().cloned().collect(),
                    _ => {
                        return Err(RuntimeError::new(
                            "state dictionary must be a dict object",
                        ))
                    }
                };
                if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
                    for (key, value) in entries {
                        let Value::Str(name) = key else {
                            return Err(RuntimeError::new(
                                "state dictionary keys must be strings",
                            ));
                        };
                        instance_data.attrs.insert(name, value);
                    }
                    Ok(())
                } else {
                    Err(RuntimeError::new(
                        "object.__setstate__() requires an instance receiver",
                    ))
                }
            };

        match &args[1] {
            Value::None => Ok(Value::None),
            Value::Dict(dict) => {
                apply_state_dict(instance, dict)?;
                Ok(Value::None)
            }
            Value::Tuple(tuple_obj) => {
                let Object::Tuple(parts) = &*tuple_obj.kind() else {
                    return Err(RuntimeError::new("invalid state payload"));
                };
                if parts.len() != 2 {
                    return Err(RuntimeError::new("invalid state payload"));
                }
                match &parts[0] {
                    Value::None => {}
                    Value::Dict(dict) => apply_state_dict(instance, dict)?,
                    _ => return Err(RuntimeError::new("invalid state payload")),
                }
                match &parts[1] {
                    Value::None => {}
                    Value::Dict(dict) => apply_state_dict(instance, dict)?,
                    _ => return Err(RuntimeError::new("invalid state payload")),
                }
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new("invalid state payload")),
        }
    }

    fn reduce_ex_constructor_and_args(&self, value: &Value) -> (Value, Value) {
        match value {
            Value::Dict(dict_obj) => {
                if let Some(default_factory) = self.defaultdict_factories.get(&dict_obj.id()) {
                    let args = if matches!(default_factory, Value::None) {
                        Vec::new()
                    } else {
                        vec![default_factory.clone()]
                    };
                    return (
                        Value::Builtin(BuiltinFunction::CollectionsDefaultDict),
                        self.heap.alloc_tuple(args),
                    );
                }
                (
                    Value::Builtin(BuiltinFunction::Dict),
                    self.heap.alloc_tuple(vec![value.clone()]),
                )
            }
            Value::Bool(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Float(_)
            | Value::Complex { .. }
            | Value::Str(_)
            | Value::Bytes(_)
            | Value::List(_)
            | Value::Tuple(_)
            | Value::Set(_)
            | Value::FrozenSet(_) => {
                let constructor = self
                    .class_of_value(value)
                    .map(Value::Class)
                    .unwrap_or_else(|| match value {
                        Value::Bool(_) => Value::Builtin(BuiltinFunction::Bool),
                        Value::Int(_) | Value::BigInt(_) => Value::Builtin(BuiltinFunction::Int),
                        Value::Float(_) => Value::Builtin(BuiltinFunction::Float),
                        Value::Complex { .. } => Value::Builtin(BuiltinFunction::Complex),
                        Value::Str(_) => Value::Builtin(BuiltinFunction::Str),
                        Value::Bytes(_) => Value::Builtin(BuiltinFunction::Bytes),
                        Value::List(_) => Value::Builtin(BuiltinFunction::List),
                        Value::Tuple(_) => Value::Builtin(BuiltinFunction::Tuple),
                        Value::Set(_) => Value::Builtin(BuiltinFunction::Set),
                        Value::FrozenSet(_) => Value::Builtin(BuiltinFunction::FrozenSet),
                        _ => Value::Builtin(BuiltinFunction::ObjectNew),
                    });
                (
                    constructor,
                    self.heap.alloc_tuple(vec![value.clone()]),
                )
            }
            Value::Exception(exception) => {
                let args = exception
                    .attrs
                    .borrow()
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| {
                        if let Some(message) = &exception.message {
                            self.heap.alloc_tuple(vec![Value::Str(message.clone())])
                        } else {
                            self.heap.alloc_tuple(Vec::new())
                        }
                    });
                (Value::ExceptionType(exception.name.clone()), args)
            }
            Value::ByteArray(obj) => {
                let payload = match &*obj.kind() {
                    Object::ByteArray(values) => values.iter().map(|value| *value as char).collect(),
                    _ => String::new(),
                };
                let constructor = self
                    .class_of_value(value)
                    .map(Value::Class)
                    .unwrap_or(Value::Builtin(BuiltinFunction::ByteArray));
                (
                    constructor,
                    self.heap
                        .alloc_tuple(vec![Value::Str(payload), Value::Str("latin-1".to_string())]),
                )
            }
            Value::Iterator(obj) => match &*obj.kind() {
                Object::Iterator(IteratorObject {
                    kind:
                        IteratorKind::Map {
                            func,
                            sources,
                            ..
                        },
                    ..
                }) => {
                    let mut args = Vec::with_capacity(1 + sources.len());
                    args.push(func.clone());
                    args.extend(sources.clone());
                    (
                        Value::Builtin(BuiltinFunction::Map),
                        self.heap.alloc_tuple(args),
                    )
                }
                Object::Iterator(IteratorObject {
                    kind: IteratorKind::RangeObject { start, stop, step },
                    ..
                }) => (
                    Value::Builtin(BuiltinFunction::Range),
                    self.heap.alloc_tuple(vec![
                        value_from_bigint(start.clone()),
                        value_from_bigint(stop.clone()),
                        value_from_bigint(step.clone()),
                    ]),
                ),
                _ => {
                    let constructor = self
                        .class_of_value(value)
                        .map(Value::Class)
                        .unwrap_or(Value::Builtin(BuiltinFunction::ObjectNew));
                    (constructor, self.heap.alloc_tuple(Vec::new()))
                }
            },
            _ => {
                let constructor = self
                    .class_of_value(value)
                    .map(Value::Class)
                    .unwrap_or(Value::Builtin(BuiltinFunction::ObjectNew));
                (constructor, self.heap.alloc_tuple(Vec::new()))
            }
        }
    }

    pub(in crate::vm) fn object_reduce_ex_for_value(
        &mut self,
        value: Value,
        protocol: i64,
        allow_custom_reduce: bool,
    ) -> Result<Value, RuntimeError> {
        let _profile = pickle_profile_scope("object_reduce_ex_for_value");
        if let Value::Builtin(builtin) = &value {
            if matches!(
                builtin,
                BuiltinFunction::DictFromKeys
                    | BuiltinFunction::BytesMakeTrans
                    | BuiltinFunction::StrMakeTrans
            ) {
                return Err(RuntimeError::new(
                    "TypeError: cannot pickle method_descriptor objects",
                ));
            }
            return Ok(Value::Str(self.builtin_attribute_qualname(*builtin)));
        }
        if let Some(name) = self.object_reduce_ex_builtin_singleton_name(&value) {
            return Ok(Value::Str(name.to_string()));
        }
        if let Value::Instance(instance) = &value {
            if let Some(class_name) = class_name_for_instance(instance) {
                if class_name == "__csv_dialect__" {
                    return Err(RuntimeError::new("cannot pickle 'Dialect' instances"));
                }
            }
        }
        if allow_custom_reduce {
            if let Some(reduced) = self.object_reduce_ex_custom_reduce(&value)? {
                return Ok(reduced);
            }
        }

        let (constructor, constructor_args) = if protocol < 2 {
            match self.object_reduce_ex_legacy_constructor_and_args(&value)? {
                Some(pair) => pair,
                None => self.reduce_ex_constructor_and_args(&value),
            }
        } else {
            match self.object_reduce_ex_new_constructor_and_args(&value, protocol)? {
                Some(pair) => pair,
                None => self.reduce_ex_constructor_and_args(&value),
            }
        };
        let mut reduced_parts = vec![constructor, constructor_args];
        let state = match &value {
            Value::Instance(_) => {
                self.builtin_object_getstate(vec![value.clone()], HashMap::new())?
            }
            _ => Value::None,
        };
        reduced_parts.push(state);

        if let Value::Instance(instance) = &value {
            if let Some(list_backing) = self.instance_backing_list(instance) {
                let iter_value =
                    self.call_builtin(BuiltinFunction::Iter, vec![Value::List(list_backing)], HashMap::new())?;
                reduced_parts.push(iter_value);
            }
        }

        Ok(self.heap.alloc_tuple(reduced_parts))
    }

    pub(in crate::vm) fn builtin_object_reduce_ex(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let _profile = pickle_profile_scope("builtin_object_reduce_ex");
        if !kwargs.is_empty() || !(1..=2).contains(&args.len()) {
            return Err(RuntimeError::new(
                "object.__reduce_ex__() takes one or two arguments",
            ));
        }
        let value = args[0].clone();
        let mut protocol = 0;
        if args.len() == 2 {
            protocol = value_to_int(args[1].clone())?;
        }
        self.object_reduce_ex_for_value(value, protocol, true)
    }

    pub(in crate::vm) fn builtin_object_reduce(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let _profile = pickle_profile_scope("builtin_object_reduce");
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "object.__reduce__() takes exactly one argument",
            ));
        }
        self.object_reduce_ex_for_value(args[0].clone(), 0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler;
    use crate::parser;
    use crate::runtime::{ClassObject, InstanceObject, Object};

    fn tuple_values(value: &Value) -> Vec<Value> {
        let Value::Tuple(obj) = value else {
            panic!("expected tuple value, got {value:?}");
        };
        let kind = obj.kind();
        let Object::Tuple(values) = &*kind else {
            panic!("expected tuple object");
        };
        values.clone()
    }

    fn alloc_instance_with_attrs(
        vm: &mut Vm,
        class_name: &str,
        attrs: &[(&str, Value)],
    ) -> Value {
        let class = match vm
            .heap
            .alloc_class(ClassObject::new(class_name.to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            other => panic!("expected class allocation, got {other:?}"),
        };
        let mut instance = InstanceObject::new(class);
        for (name, value) in attrs {
            instance.attrs.insert((*name).to_string(), value.clone());
        }
        vm.heap.alloc_instance(instance)
    }

    #[test]
    fn object_getstate_returns_none_for_non_instance_values() {
        let vm = Vm::new();
        let state = vm
            .builtin_object_getstate(vec![Value::Int(7)], HashMap::new())
            .expect("object.__getstate__ should succeed");
        assert_eq!(state, Value::None);
    }

    #[test]
    fn object_getstate_returns_instance_dict_payload() {
        let mut vm = Vm::new();
        let instance = alloc_instance_with_attrs(
            &mut vm,
            "Point",
            &[("x", Value::Int(4)), ("y", Value::Int(9))],
        );
        let state = vm
            .builtin_object_getstate(vec![instance], HashMap::new())
            .expect("object.__getstate__ should return state");
        let Value::Dict(dict) = state else {
            panic!("expected dict state");
        };
        let kind = dict.kind();
        let Object::Dict(entries) = &*kind else {
            panic!("expected dict object");
        };
        assert_eq!(entries.find(&Value::Str("x".to_string())), Some(&Value::Int(4)));
        assert_eq!(entries.find(&Value::Str("y".to_string())), Some(&Value::Int(9)));
    }

    #[test]
    fn object_reduce_ex_returns_tuple_for_builtin_payload() {
        let mut vm = Vm::new();
        let reduced = vm
            .builtin_object_reduce_ex(vec![Value::Int(7), Value::Int(4)], HashMap::new())
            .expect("object.__reduce_ex__ should succeed");
        let parts = tuple_values(&reduced);
        assert_eq!(parts.len(), 3);
        let constructor_args = tuple_values(&parts[1]);
        assert_eq!(constructor_args, vec![Value::Int(7)]);
        assert_eq!(parts[2], Value::None);
    }

    #[test]
    fn object_reduce_ex_bytearray_uses_latin1_constructor_args() {
        let mut vm = Vm::new();
        let payload = vm.heap.alloc_bytearray(vec![0x78, 0x79, 0x7A, 0xFF]);
        let reduced = vm
            .builtin_object_reduce_ex(vec![payload, Value::Int(0)], HashMap::new())
            .expect("object.__reduce_ex__ should succeed");
        let parts = tuple_values(&reduced);
        let constructor_args = tuple_values(&parts[1]);
        assert_eq!(
            constructor_args,
            vec![
                Value::Str("xyz\u{ff}".to_string()),
                Value::Str("latin-1".to_string())
            ]
        );
        assert_eq!(parts[2], Value::None);
    }

    #[test]
    fn object_reduce_ex_protocol0_uses_reconstructor_for_instances() {
        let mut vm = Vm::new();
        if vm.import_module_object("copyreg").is_err() {
            eprintln!("skipping legacy protocol reduce_ex unit test (copyreg unavailable)");
            return;
        }
        let instance =
            alloc_instance_with_attrs(&mut vm, "NeedsArgs", &[("a", Value::Int(1))]);
        let class_obj = vm
            .class_of_value(&instance)
            .map(Value::Class)
            .expect("instance should have class");
        let reduced = vm
            .builtin_object_reduce_ex(vec![instance, Value::Int(0)], HashMap::new())
            .expect("object.__reduce_ex__ should succeed");
        let parts = tuple_values(&reduced);
        assert_eq!(parts.len(), 3);
        // Protocol 0/1 should use copyreg._reconstructor rather than direct class call.
        assert!(!matches!(parts[0], Value::Class(_)));
        let constructor_args = tuple_values(&parts[1]);
        assert_eq!(constructor_args.len(), 3);
        assert_eq!(constructor_args[0], class_obj);
        assert_eq!(constructor_args[2], Value::None);
        let object_type = vm
            .builtins
            .get("object")
            .cloned()
            .expect("object type should be installed");
        assert_eq!(constructor_args[1], object_type);
    }

    #[test]
    fn object_reduce_ex_protocol2_uses_newobj_for_instances() {
        let mut vm = Vm::new();
        if vm.import_module_object("copyreg").is_err() {
            eprintln!("skipping protocol-2 reduce_ex unit test (copyreg unavailable)");
            return;
        }
        let instance =
            alloc_instance_with_attrs(&mut vm, "NeedsArgs", &[("a", Value::Int(1))]);
        let class_obj = vm
            .class_of_value(&instance)
            .map(Value::Class)
            .expect("instance should have class");
        let reduced = vm
            .builtin_object_reduce_ex(vec![instance, Value::Int(2)], HashMap::new())
            .expect("object.__reduce_ex__ should succeed");
        let parts = tuple_values(&reduced);
        assert_eq!(parts.len(), 3);
        assert!(!matches!(parts[0], Value::Class(_)));
        let constructor_args = tuple_values(&parts[1]);
        assert_eq!(constructor_args, vec![class_obj]);
    }

    #[test]
    fn object_reduce_ex_caches_copyreg_callables() {
        let mut vm = Vm::new();
        if vm.import_module_object("copyreg").is_err() {
            eprintln!("skipping copyreg cache unit test (copyreg unavailable)");
            return;
        }
        let instance = alloc_instance_with_attrs(&mut vm, "NeedsArgs", &[("a", Value::Int(1))]);

        vm.builtin_object_reduce_ex(vec![instance.clone(), Value::Int(2)], HashMap::new())
            .expect("protocol-2 reduce should succeed");
        let cached_after_first = vm.pickle_copyreg_cache.len();
        assert!(
            vm.pickle_copyreg_cache.contains_key("__newobj__"),
            "expected __newobj__ callable in cache"
        );

        vm.builtin_object_reduce_ex(vec![instance, Value::Int(2)], HashMap::new())
            .expect("second protocol-2 reduce should succeed");
        assert_eq!(
            vm.pickle_copyreg_cache.len(),
            cached_after_first,
            "copyreg callable cache should be reused instead of growing"
        );
    }

    #[test]
    fn object_reduce_ex_validates_arity_protocol_and_dialect_instances() {
        let mut vm = Vm::new();
        let arity_err = vm
            .builtin_object_reduce_ex(Vec::new(), HashMap::new())
            .expect_err("missing self should fail");
        assert!(
            arity_err
                .message
                .contains("object.__reduce_ex__() takes one or two arguments")
        );

        vm.builtin_object_reduce_ex(
            vec![Value::Int(1), Value::Str("bad".to_string())],
            HashMap::new(),
        )
        .expect_err("non-integer protocol should fail");

        let dialect = alloc_instance_with_attrs(&mut vm, "__csv_dialect__", &[]);
        let dialect_err = vm
            .builtin_object_reduce_ex(vec![dialect, Value::Int(4)], HashMap::new())
            .expect_err("dialect pickling should fail");
        assert!(dialect_err.message.contains("cannot pickle 'Dialect' instances"));
    }

    #[test]
    fn object_reduce_ex_returns_names_for_builtin_singletons() {
        let mut vm = Vm::new();
        let ellipsis = vm
            .builtins
            .get("Ellipsis")
            .cloned()
            .expect("Ellipsis should be installed");
        let reduced_ellipsis = vm
            .builtin_object_reduce_ex(vec![ellipsis, Value::Int(4)], HashMap::new())
            .expect("Ellipsis reduce should succeed");
        assert_eq!(reduced_ellipsis, Value::Str("Ellipsis".to_string()));

        let not_implemented = vm
            .builtins
            .get("NotImplemented")
            .cloned()
            .expect("NotImplemented should be installed");
        let reduced_not_implemented = vm
            .builtin_object_reduce_ex(vec![not_implemented, Value::Int(4)], HashMap::new())
            .expect("NotImplemented reduce should succeed");
        assert_eq!(
            reduced_not_implemented,
            Value::Str("NotImplemented".to_string())
        );
    }

    #[test]
    fn pickle_module_getattr_maps_accelerator_names_to_pure_pickle_symbols() {
        let mut vm = Vm::new();
        let Ok(pickle_module) = vm.import_module_object("pickle") else {
            eprintln!("skipping _pickle getattr mapping test (pickle module unavailable)");
            return;
        };
        let c_pickle = vm
            .import_module_object("_pickle")
            .expect("_pickle module should import");

        let pickler_attr = vm
            .builtin_getattr(
                vec![Value::Module(c_pickle.clone()), Value::Str("Pickler".to_string())],
                HashMap::new(),
            )
            .expect("_pickle.Pickler should resolve");
        let expected_pickler = vm
            .builtin_getattr(
                vec![
                    Value::Module(pickle_module.clone()),
                    Value::Str("_Pickler".to_string()),
                ],
                HashMap::new(),
            )
            .expect("pickle._Pickler should resolve");
        assert_eq!(pickler_attr, expected_pickler);

        let dumps_attr = vm
            .builtin_getattr(
                vec![Value::Module(c_pickle), Value::Str("dumps".to_string())],
                HashMap::new(),
            )
            .expect("_pickle.dumps should resolve");
        let expected_dumps = vm
            .builtin_getattr(
                vec![Value::Module(pickle_module), Value::Str("_dumps".to_string())],
                HashMap::new(),
            )
            .expect("pickle._dumps should resolve");
        assert_eq!(dumps_attr, expected_dumps);
    }

    #[test]
    fn picklebuffer_raw_returns_memoryview_and_release_blocks_access() {
        let mut vm = Vm::new();
        let source = r#"import _pickle
pb = _pickle.PickleBuffer(b"abc")
raw = pb.raw().tobytes()
pb.release()
caught = False
try:
    pb.raw()
except ValueError:
    caught = True
ok = (raw == b"abc" and caught)
"#;
        let module = parser::parse_module(source).expect("parse should succeed");
        let code = compiler::compile_module(&module).expect("compile should succeed");
        vm.execute(&code).expect("execution should succeed");
        assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
    }
}
