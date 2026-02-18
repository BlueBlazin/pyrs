use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::runtime::{Object, Value};

use super::{
    CPYTHON_ARGC, CPYTHON_ARGV, CPYTHON_ATEXIT_CALLBACKS, CPYTHON_HEAP_TYPE_REGISTRY,
    CPYTHON_INTERNED_UNICODE_REGISTRY, CPYTHON_INTERPRETER_STATE_ALLOCATIONS,
    CPYTHON_PENDING_CALLS, CPYTHON_STRUCTSEQ_TYPE_REGISTRY, CPYTHON_THREAD_LOCK_REGISTRY,
    CPYTHON_THREAD_STATE_ALLOCATIONS, CPYTHON_THREAD_TLS_KEY_REGISTRY, CPYTHON_THREAD_TLS_VALUES,
    CPYTHON_THREAD_TSS_REGISTRY, CPYTHON_THREAD_TSS_VALUES, CURRENT_THREAD_STATE_PTR,
    CpythonHeapTypeInfo, CpythonInternedUnicodeRegistry, CpythonPendingCall,
    CpythonStructSeqTypeInfo, CpythonThreadStateCompat, Cwchar, MAIN_INTERPRETER_STATE_TOKEN,
    MAIN_THREAD_STATE_STORAGE, cpython_string_to_wide_units, cpython_sys_module_obj,
    vm_current_thread_ident, with_active_cpython_context_mut,
};

pub(super) fn cpython_init_thread_state_compat(
    state: *mut CpythonThreadStateCompat,
    interp: *mut c_void,
) -> *mut CpythonThreadStateCompat {
    if state.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provides a writable thread-state compatibility pointer.
    unsafe {
        (*state).interp = interp;
        (*state).exc_state.exc_value = std::ptr::null_mut();
        (*state).exc_state.previous_item = std::ptr::null_mut();
        (*state).exc_info = std::ptr::addr_of_mut!((*state).exc_state);
    }
    state
}

pub(super) fn cpython_main_thread_state_ptr() -> usize {
    cpython_init_thread_state_compat(
        std::ptr::addr_of_mut!(MAIN_THREAD_STATE_STORAGE),
        cpython_main_interpreter_state_ptr() as *mut c_void,
    );
    (&raw mut MAIN_THREAD_STATE_STORAGE) as usize
}

pub(super) fn cpython_thread_state_allocations() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_STATE_ALLOCATIONS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn cpython_interned_unicode_registry() -> &'static Mutex<CpythonInternedUnicodeRegistry>
{
    CPYTHON_INTERNED_UNICODE_REGISTRY
        .get_or_init(|| Mutex::new(CpythonInternedUnicodeRegistry::default()))
}

pub(super) fn cpython_lookup_interned_unicode_ptr(text: &str) -> Option<*mut c_void> {
    cpython_interned_unicode_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.by_text.get(text).copied())
        .map(|raw| raw as *mut c_void)
}

pub(super) fn cpython_lookup_interned_unicode_text(ptr: *mut c_void) -> Option<String> {
    cpython_interned_unicode_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.by_ptr.get(&(ptr as usize)).cloned())
}

pub(super) fn cpython_register_interned_unicode(text: &str, ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    if let Ok(mut registry) = cpython_interned_unicode_registry().lock() {
        registry.by_text.insert(text.to_string(), ptr as usize);
        registry.by_ptr.insert(ptr as usize, text.to_string());
    }
}

pub(super) fn cpython_is_interned_unicode_ptr(ptr: *mut c_void) -> bool {
    if ptr.is_null() {
        return false;
    }
    cpython_interned_unicode_registry()
        .lock()
        .ok()
        .is_some_and(|registry| registry.by_ptr.contains_key(&(ptr as usize)))
}

pub(super) fn cpython_thread_lock_registry() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_LOCK_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn cpython_thread_tls_key_registry() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_TLS_KEY_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn cpython_thread_tls_values() -> &'static Mutex<HashMap<(u64, usize), usize>> {
    CPYTHON_THREAD_TLS_VALUES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn cpython_thread_tss_registry() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_THREAD_TSS_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn cpython_thread_tss_values() -> &'static Mutex<HashMap<(u64, usize), usize>> {
    CPYTHON_THREAD_TSS_VALUES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn cpython_pending_calls() -> &'static Mutex<VecDeque<CpythonPendingCall>> {
    CPYTHON_PENDING_CALLS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(super) fn cpython_atexit_callbacks() -> &'static Mutex<Vec<unsafe extern "C" fn()>> {
    CPYTHON_ATEXIT_CALLBACKS.get_or_init(|| Mutex::new(Vec::new()))
}

pub(super) fn cpython_leak_wide_string(text: &str) -> *mut Cwchar {
    let mut units = cpython_string_to_wide_units(text);
    units.push(0);
    let boxed = units.into_boxed_slice();
    Box::into_raw(boxed).cast::<Cwchar>()
}

pub(super) fn cpython_set_wide_storage(storage: &AtomicUsize, text: &str) -> *mut Cwchar {
    let pointer = cpython_leak_wide_string(text);
    storage.store(pointer as usize, Ordering::Relaxed);
    pointer
}

pub(super) fn cpython_get_or_init_wide_storage(
    storage: &AtomicUsize,
    fallback: impl FnOnce() -> String,
) -> *mut Cwchar {
    let current = storage.load(Ordering::Relaxed) as *mut Cwchar;
    if !current.is_null() {
        return current;
    }
    cpython_set_wide_storage(storage, &fallback())
}

pub(super) fn cpython_read_sys_string(name: &str) -> Option<String> {
    let mut value = None;
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(data) = &*sys_module.kind()
            && let Some(Value::Str(text)) = data.globals.get(name)
        {
            value = Some(text.clone());
        }
    });
    value
}

pub(super) fn cpython_read_sys_path_string() -> Option<String> {
    let mut value = None;
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(data) = &*sys_module.kind()
            && let Some(Value::List(path_list)) = data.globals.get("path")
            && let Object::List(entries) = &*path_list.kind()
        {
            #[cfg(windows)]
            let delimiter = ';';
            #[cfg(not(windows))]
            let delimiter = ':';
            let joined = entries
                .iter()
                .filter_map(|entry| match entry {
                    Value::Str(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(&delimiter.to_string());
            value = Some(joined);
        }
    });
    value
}

pub(super) fn cpython_store_argv_wide(arguments: &[String]) {
    let argc = arguments.len() as i64;
    let pointers: Vec<*mut Cwchar> = arguments
        .iter()
        .map(|arg| cpython_leak_wide_string(arg))
        .collect();
    let argv = if pointers.is_empty() {
        std::ptr::null_mut()
    } else {
        let boxed = pointers.into_boxed_slice();
        Box::into_raw(boxed).cast::<*mut Cwchar>()
    };
    CPYTHON_ARGC.store(argc, Ordering::Relaxed);
    CPYTHON_ARGV.store(argv as usize, Ordering::Relaxed);
}

pub(super) fn cpython_collect_sys_argv() -> Option<Vec<String>> {
    let mut argv = None;
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(data) = &*sys_module.kind()
            && let Some(Value::List(list_obj)) = data.globals.get("argv")
            && let Object::List(entries) = &*list_obj.kind()
        {
            let mut extracted = Vec::with_capacity(entries.len());
            for entry in entries {
                if let Value::Str(text) = entry {
                    extracted.push(text.clone());
                }
            }
            argv = Some(extracted);
        }
    });
    argv
}

pub(super) fn cpython_get_or_init_constant_ptr(
    storage: &AtomicUsize,
    init: impl FnOnce() -> *mut c_void,
) -> *mut c_void {
    let current = storage.load(Ordering::Relaxed) as *mut c_void;
    if !current.is_null() {
        return current;
    }
    let value = init();
    if value.is_null() {
        return std::ptr::null_mut();
    }
    storage.store(value as usize, Ordering::Relaxed);
    value
}

pub(super) fn cpython_main_interpreter_state_ptr() -> usize {
    std::ptr::addr_of!(MAIN_INTERPRETER_STATE_TOKEN) as usize
}

pub(super) fn cpython_interpreter_state_allocations() -> &'static Mutex<HashSet<usize>> {
    CPYTHON_INTERPRETER_STATE_ALLOCATIONS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn cpython_structseq_registry()
-> &'static Mutex<HashMap<usize, CpythonStructSeqTypeInfo>> {
    CPYTHON_STRUCTSEQ_TYPE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn cpython_heap_type_registry() -> &'static Mutex<HashMap<usize, CpythonHeapTypeInfo>> {
    CPYTHON_HEAP_TYPE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn cpython_is_known_interpreter_state_ptr(ptr: usize) -> bool {
    if ptr == 0 || ptr == cpython_main_interpreter_state_ptr() {
        return ptr != 0;
    }
    cpython_interpreter_state_allocations()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&ptr))
}

pub(super) fn cpython_current_thread_state_ptr() -> usize {
    let current = CURRENT_THREAD_STATE_PTR.load(Ordering::Relaxed);
    if current != 0 {
        return current;
    }
    let main_ptr = cpython_main_thread_state_ptr();
    CURRENT_THREAD_STATE_PTR
        .compare_exchange(0, main_ptr, Ordering::Relaxed, Ordering::Relaxed)
        .ok();
    CURRENT_THREAD_STATE_PTR.load(Ordering::Relaxed)
}

pub(super) fn cpython_is_known_thread_state_ptr(ptr: usize) -> bool {
    if ptr == 0 || ptr == cpython_main_thread_state_ptr() {
        return ptr != 0;
    }
    cpython_thread_state_allocations()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&ptr))
}

pub(super) fn cpython_current_thread_ident_u64() -> u64 {
    let ident = vm_current_thread_ident();
    if ident >= 0 {
        ident as u64
    } else {
        ident.unsigned_abs()
    }
}
