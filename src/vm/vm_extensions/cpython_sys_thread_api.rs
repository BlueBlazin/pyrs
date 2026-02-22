use std::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::runtime::{Object, Value};
use crate::vm::{ObjRef, dict_remove_value, dict_set_value_checked, vm_os_thread_ident};

use super::{
    CPYTHON_THREAD_NEXT_IDENT, CPYTHON_THREAD_STACK_SIZE, CPYTHON_THREAD_TLS_NEXT_KEY,
    CpythonThreadLock, CpythonThreadTss, Cwchar, ModuleCapiContext, PyExc_SystemError,
    PyExc_TypeError, PyExc_ValueError, c_name_to_string, c_wide_name_to_string,
    cpython_current_thread_ident_u64, cpython_mark_thread_runtime_initialized, cpython_set_error,
    cpython_set_typed_error, cpython_store_argv_wide, cpython_thread_lock_registry,
    cpython_thread_tls_key_registry, cpython_thread_tls_values, cpython_thread_tss_registry,
    cpython_thread_tss_values, with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetObject(name: *const c_char) -> *mut c_void {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySys_GetObject missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(sys_module) = vm.modules.get("sys") else {
            context.set_error("PySys_GetObject could not find sys module");
            return std::ptr::null_mut();
        };
        let Object::Module(data) = &*sys_module.kind() else {
            context.set_error("PySys_GetObject sys module invalid");
            return std::ptr::null_mut();
        };
        let Some(value) = data.globals.get(&name) else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(value.clone())
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(in crate::vm::vm_extensions) fn cpython_sys_module_obj(
    context: &mut ModuleCapiContext,
) -> Result<ObjRef, String> {
    if context.vm.is_null() {
        return Err("missing VM context".to_string());
    }
    // SAFETY: VM pointer is valid for active C-API context.
    let vm = unsafe { &mut *context.vm };
    vm.modules
        .get("sys")
        .cloned()
        .ok_or_else(|| "could not find sys module".to_string())
}

fn cpython_sys_warnoptions_list(context: &mut ModuleCapiContext) -> Result<ObjRef, String> {
    let sys_module = cpython_sys_module_obj(context)?;
    let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
        return Err("sys module object is invalid".to_string());
    };
    if let Some(Value::List(existing)) = sys_data.globals.get("warnoptions") {
        return Ok(existing.clone());
    }
    let list_obj = if context.vm.is_null() {
        return Err("missing VM context".to_string());
    } else {
        // SAFETY: VM pointer is valid for active C-API context.
        let vm = unsafe { &mut *context.vm };
        match vm.heap.alloc_list(Vec::new()) {
            Value::List(list_obj) => list_obj,
            _ => return Err("failed to allocate warnoptions list".to_string()),
        }
    };
    sys_data
        .globals
        .insert("warnoptions".to_string(), Value::List(list_obj.clone()));
    Ok(list_obj)
}

fn cpython_sys_xoptions_dict(context: &mut ModuleCapiContext) -> Result<ObjRef, String> {
    let sys_module = cpython_sys_module_obj(context)?;
    let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
        return Err("sys module object is invalid".to_string());
    };
    if let Some(Value::Dict(existing)) = sys_data.globals.get("_xoptions") {
        return Ok(existing.clone());
    }
    let dict_obj = if context.vm.is_null() {
        return Err("missing VM context".to_string());
    } else {
        // SAFETY: VM pointer is valid for active C-API context.
        let vm = unsafe { &mut *context.vm };
        match vm.heap.alloc_dict(Vec::new()) {
            Value::Dict(dict_obj) => dict_obj,
            _ => return Err("failed to allocate _xoptions dict".to_string()),
        }
    };
    sys_data
        .globals
        .insert("_xoptions".to_string(), Value::Dict(dict_obj.clone()));
    Ok(dict_obj)
}

fn cpython_sys_add_warn_option(
    context: &mut ModuleCapiContext,
    option: String,
) -> Result<(), String> {
    let warnoptions = cpython_sys_warnoptions_list(context)?;
    let Object::List(values) = &mut *warnoptions.kind_mut() else {
        return Err("warnoptions is not a list".to_string());
    };
    values.push(Value::Str(option));
    Ok(())
}

fn cpython_sys_set_global(
    context: &mut ModuleCapiContext,
    name: &str,
    value: Value,
) -> Result<(), String> {
    let sys_module = cpython_sys_module_obj(context)?;
    {
        let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
            return Err("sys module object is invalid".to_string());
        };
        sys_data.globals.insert(name.to_string(), value.clone());
    }
    context
        .sync_module_dict_set(&sys_module, name, &value)
        .map_err(|err| format!("failed syncing sys.{name}: {err}"))?;
    Ok(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetObject(name: *const c_char, value: *mut c_void) -> i32 {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        let sys_module = match cpython_sys_module_obj(context) {
            Ok(module) => module,
            Err(err) => {
                context.set_error(format!("PySys_SetObject {err}"));
                return -1;
            }
        };
        if value.is_null() {
            let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
                context.set_error("PySys_SetObject sys module invalid");
                return -1;
            };
            sys_data.globals.remove(&name);
            if let Some(dict_handle) = context.module_dict_handle_for_module(&sys_module)
                && let Some(slot) = context.objects.get(&dict_handle)
                && let Value::Dict(dict_obj) = &slot.value
            {
                let _ = dict_remove_value(dict_obj, &Value::Str(name.clone()));
            }
            return 0;
        }
        let Some(mapped) = context.cpython_value_from_ptr_or_proxy(value) else {
            context.set_error("PySys_SetObject received unknown value pointer");
            return -1;
        };
        let Object::Module(sys_data) = &mut *sys_module.kind_mut() else {
            context.set_error("PySys_SetObject sys module invalid");
            return -1;
        };
        sys_data.globals.insert(name.clone(), mapped.clone());
        if let Err(err) = context.sync_module_dict_set(&sys_module, &name, &mapped) {
            context.set_error(format!(
                "PySys_SetObject failed syncing module dict entry '{}': {}",
                name, err
            ));
            return -1;
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_GetXOptions() -> *mut c_void {
    with_active_cpython_context_mut(|context| match cpython_sys_xoptions_dict(context) {
        Ok(dict_obj) => context.alloc_cpython_ptr_for_value(Value::Dict(dict_obj)),
        Err(err) => {
            context.set_error(format!("PySys_GetXOptions {err}"));
            std::ptr::null_mut()
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AddXOption(option: *const Cwchar) {
    let option = match unsafe { c_wide_name_to_string(option) } {
        Ok(option) => option,
        Err(err) => {
            cpython_set_error(err);
            return;
        }
    };
    let _ = with_active_cpython_context_mut(|context| {
        let xoptions = match cpython_sys_xoptions_dict(context) {
            Ok(dict_obj) => dict_obj,
            Err(err) => {
                context.set_error(format!("PySys_AddXOption {err}"));
                return;
            }
        };
        let (key, value) = if let Some(eq) = option.find('=') {
            (
                option[..eq].to_string(),
                Value::Str(option[eq + 1..].to_string()),
            )
        } else {
            (option, Value::Bool(true))
        };
        let _ = dict_set_value_checked(&xoptions, Value::Str(key), value).map_err(|err| {
            context.set_error(format!("PySys_AddXOption {}", err.message));
        });
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_HasWarnOptions() -> i32 {
    with_active_cpython_context_mut(|context| match cpython_sys_warnoptions_list(context) {
        Ok(list_obj) => match &*list_obj.kind() {
            Object::List(values) => i32::from(!values.is_empty()),
            _ => 0,
        },
        Err(err) => {
            context.set_error(format!("PySys_HasWarnOptions {err}"));
            0
        }
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_ResetWarnOptions() {
    let _ = with_active_cpython_context_mut(|context| {
        let warnoptions = match cpython_sys_warnoptions_list(context) {
            Ok(warnoptions) => warnoptions,
            Err(err) => {
                context.set_error(format!("PySys_ResetWarnOptions {err}"));
                return;
            }
        };
        let Object::List(values) = &mut *warnoptions.kind_mut() else {
            context.set_error("PySys_ResetWarnOptions warnoptions is not a list");
            return;
        };
        values.clear();
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AddWarnOption(option: *const Cwchar) {
    let option = match unsafe { c_wide_name_to_string(option) } {
        Ok(option) => option,
        Err(err) => {
            cpython_set_error(err);
            return;
        }
    };
    let _ = with_active_cpython_context_mut(|context| {
        if let Err(err) = cpython_sys_add_warn_option(context, option) {
            context.set_error(format!("PySys_AddWarnOption {err}"));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AddWarnOptionUnicode(option: *const Cwchar) {
    unsafe { PySys_AddWarnOption(option) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_sys_write_stdout(text: *const c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid NUL-terminated string.
    let line = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    print!("{line}");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_sys_write_stderr(text: *const c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid NUL-terminated string.
    let line = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    eprint!("{line}");
}

fn cpython_sys_audit_dispatch_from_object(
    event: *const c_char,
    args: *mut c_void,
    require_tuple: bool,
) -> i32 {
    if event.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PySys_AuditTuple requires event name",
        );
        return -1;
    }

    let event_name = match unsafe { c_name_to_string(event) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };

    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySys_AuditTuple missing VM context");
            return -1;
        }
        let event_args = if args.is_null() {
            Vec::new()
        } else {
            let Some(arg_value) = context.cpython_value_from_ptr_or_proxy(args) else {
                cpython_set_error("PySys_AuditTuple received unknown args object");
                return -1;
            };
            match arg_value {
                Value::Tuple(items_obj) => match &*items_obj.kind() {
                    Object::Tuple(items) => items.clone(),
                    _ => {
                        cpython_set_error("PySys_AuditTuple tuple storage invalid");
                        return -1;
                    }
                },
                other if !require_tuple => vec![other],
                other => {
                    // SAFETY: VM pointer is valid for active context lifetime.
                    let type_name = unsafe { (&mut *context.vm).value_type_name_for_error(&other) };
                    cpython_set_typed_error(
                        unsafe { PyExc_TypeError },
                        format!("args must be tuple, got {type_name}"),
                    );
                    return -1;
                }
            }
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.dispatch_sys_audit_event(&event_name, event_args) {
            Ok(()) => 0,
            Err(err) => {
                context.set_error(err.message);
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err.to_string());
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_AuditTuple(event: *const c_char, args: *mut c_void) -> i32 {
    cpython_sys_audit_dispatch_from_object(event, args, true)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_sys_audit_object(
    event: *const c_char,
    args: *mut c_void,
) -> i32 {
    cpython_sys_audit_dispatch_from_object(event, args, false)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pyrs_capi_sys_audit_noargs(event: *const c_char) -> i32 {
    unsafe { PySys_AuditTuple(event, std::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetPath(path: *const Cwchar) {
    let path_text = match unsafe { c_wide_name_to_string(path) } {
        Ok(path) => path,
        Err(err) => {
            cpython_set_error(err);
            return;
        }
    };
    #[cfg(windows)]
    let delimiter = ';';
    #[cfg(not(windows))]
    let delimiter = ':';
    let entries: Vec<Value> = if path_text.is_empty() {
        Vec::new()
    } else {
        path_text
            .split(delimiter)
            .map(|entry| Value::Str(entry.to_string()))
            .collect()
    };
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySys_SetPath missing VM context");
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let path_list = unsafe { (&mut *context.vm).heap.alloc_list(entries) };
        if let Err(err) = cpython_sys_set_global(context, "path", path_list) {
            context.set_error(format!("PySys_SetPath {err}"));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetArgvEx(argc: i32, argv: *mut *mut Cwchar, updatepath: i32) {
    if argc < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PySys_SetArgvEx argc must be >= 0",
        );
        return;
    }
    let mut argv_values: Vec<Value> = Vec::new();
    for idx in 0..(argc as usize) {
        let arg_ptr = if argv.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: caller guarantees `argv` has `argc` entries when non-null.
            unsafe { *argv.add(idx) }
        };
        let arg = if arg_ptr.is_null() {
            String::new()
        } else {
            match unsafe { c_wide_name_to_string(arg_ptr) } {
                Ok(arg) => arg,
                Err(err) => {
                    cpython_set_error(format!("PySys_SetArgvEx invalid argument: {err}"));
                    return;
                }
            }
        };
        argv_values.push(Value::Str(arg));
    }
    let argv_strings: Vec<String> = argv_values
        .iter()
        .filter_map(|value| match value {
            Value::Str(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    cpython_store_argv_wide(&argv_strings);

    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PySys_SetArgvEx missing VM context");
            return;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let argv_list = vm.heap.alloc_list(argv_values.clone());
        if let Err(err) = cpython_sys_set_global(context, "argv", argv_list) {
            context.set_error(format!("PySys_SetArgvEx {err}"));
            return;
        }
        if updatepath == 0 {
            return;
        }
        let mut path_values: Vec<Value> = Vec::new();
        if let Some(Value::Str(program_path)) = argv_values.first() {
            let first_path = Path::new(program_path).parent().map_or_else(
                || "".to_string(),
                |parent| parent.to_string_lossy().to_string(),
            );
            path_values.push(Value::Str(first_path));
        }
        if let Ok(sys_module) = cpython_sys_module_obj(context)
            && let Object::Module(sys_data) = &*sys_module.kind()
            && let Some(Value::List(existing_list)) = sys_data.globals.get("path")
            && let Object::List(items) = &*existing_list.kind()
        {
            path_values.extend(items.iter().cloned());
        }
        let path_list = vm.heap.alloc_list(path_values);
        if let Err(err) = cpython_sys_set_global(context, "path", path_list) {
            context.set_error(format!("PySys_SetArgvEx {err}"));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySys_SetArgv(argc: i32, argv: *mut *mut Cwchar) {
    unsafe { PySys_SetArgvEx(argc, argv, 1) }
}

fn cpython_thread_lock_is_known(ptr: usize) -> bool {
    cpython_thread_lock_registry()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&ptr))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_init_thread() {
    cpython_mark_thread_runtime_initialized();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_start_new_thread(
    function: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
) -> c_ulong {
    let Some(function) = function else {
        return c_ulong::MAX;
    };
    let stack_size = CPYTHON_THREAD_STACK_SIZE.load(Ordering::Relaxed);
    let arg_bits = arg as usize;
    let ident = CPYTHON_THREAD_NEXT_IDENT.fetch_add(1, Ordering::Relaxed);
    let mut builder = std::thread::Builder::new();
    if stack_size > 0 {
        builder = builder.stack_size(stack_size);
    }
    match builder.spawn(move || {
        // SAFETY: C-API contract provides callable + argument pointer.
        unsafe { function(arg_bits as *mut c_void) };
    }) {
        Ok(handle) => {
            std::mem::drop(handle);
            ident as c_ulong
        }
        Err(_) => c_ulong::MAX,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_exit_thread() {
    loop {
        std::thread::park();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_thread_ident() -> c_ulong {
    vm_os_thread_ident() as c_ulong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_thread_native_id() -> c_ulong {
    vm_os_thread_ident() as c_ulong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_allocate_lock() -> *mut c_void {
    let raw = Box::into_raw(Box::new(CpythonThreadLock {
        state: Mutex::new(false),
        condvar: Condvar::new(),
    })) as usize;
    if let Ok(mut set) = cpython_thread_lock_registry().lock() {
        set.insert(raw);
    }
    raw as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_free_lock(lock: *mut c_void) {
    if lock.is_null() {
        return;
    }
    let ptr = lock as usize;
    let removed = cpython_thread_lock_registry()
        .lock()
        .ok()
        .is_some_and(|mut set| set.remove(&ptr));
    if removed {
        // SAFETY: pointer was produced by Box::into_raw in PyThread_allocate_lock and removed once.
        unsafe {
            drop(Box::from_raw(lock.cast::<CpythonThreadLock>()));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_acquire_lock(lock: *mut c_void, waitflag: c_int) -> c_int {
    let timeout = if waitflag != 0 { -1 } else { 0 };
    let status = unsafe { PyThread_acquire_lock_timed(lock, timeout, 0) };
    if status == 1 { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_acquire_lock_timed(
    lock: *mut c_void,
    microseconds: i64,
    _intr_flag: c_int,
) -> c_int {
    if lock.is_null() {
        return 0;
    }
    let lock_ptr = lock as usize;
    if !cpython_thread_lock_is_known(lock_ptr) {
        return 0;
    }
    // SAFETY: lock pointer validity is guarded by registry membership.
    let lock_ref = unsafe { &*lock.cast::<CpythonThreadLock>() };
    let mut state = match lock_ref.state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !*state {
        *state = true;
        return 1;
    }
    if microseconds == 0 {
        return 0;
    }
    if microseconds < 0 {
        while *state {
            state = match lock_ref.condvar.wait(state) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        *state = true;
        return 1;
    }
    let timeout = Duration::from_micros(microseconds as u64);
    let start = Instant::now();
    while *state {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return 0;
        }
        let remaining = timeout - elapsed;
        let result = lock_ref.condvar.wait_timeout(state, remaining);
        let (new_state, wait_outcome) = match result {
            Ok(value) => value,
            Err(poisoned) => poisoned.into_inner(),
        };
        state = new_state;
        if wait_outcome.timed_out() && *state {
            return 0;
        }
    }
    *state = true;
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_release_lock(lock: *mut c_void) {
    if lock.is_null() {
        return;
    }
    let lock_ptr = lock as usize;
    if !cpython_thread_lock_is_known(lock_ptr) {
        return;
    }
    // SAFETY: lock pointer validity is guarded by registry membership.
    let lock_ref = unsafe { &*lock.cast::<CpythonThreadLock>() };
    let mut state = match lock_ref.state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *state {
        *state = false;
        lock_ref.condvar.notify_one();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_stacksize() -> usize {
    CPYTHON_THREAD_STACK_SIZE.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_set_stacksize(size: usize) -> c_int {
    if size == 0 {
        CPYTHON_THREAD_STACK_SIZE.store(0, Ordering::Relaxed);
        return 0;
    }
    if size < 32 * 1024 {
        return -1;
    }
    CPYTHON_THREAD_STACK_SIZE.store(size, Ordering::Relaxed);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_GetInfo() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyThread_GetInfo missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let info = vm.heap.alloc_tuple(vec![
            Value::Str("pyrs".to_string()),
            Value::Str("mutex+cond".to_string()),
            Value::None,
        ]);
        context.alloc_cpython_ptr_for_value(info)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_create_key() -> c_int {
    let raw = CPYTHON_THREAD_TLS_NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    if raw > c_int::MAX as usize {
        return -1;
    }
    if let Ok(mut set) = cpython_thread_tls_key_registry().lock() {
        set.insert(raw);
    }
    raw as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_delete_key(key: c_int) {
    if key <= 0 {
        return;
    }
    let key_id = key as usize;
    if let Ok(mut set) = cpython_thread_tls_key_registry().lock() {
        set.remove(&key_id);
    }
    if let Ok(mut map) = cpython_thread_tls_values().lock() {
        map.retain(|(_, stored_key), _| *stored_key != key_id);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_set_key_value(key: c_int, value: *mut c_void) -> c_int {
    if key <= 0 {
        return -1;
    }
    let key_id = key as usize;
    let is_known = cpython_thread_tls_key_registry()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&key_id));
    if !is_known {
        return -1;
    }
    let thread_id = cpython_current_thread_ident_u64();
    if let Ok(mut map) = cpython_thread_tls_values().lock() {
        map.insert((thread_id, key_id), value as usize);
        0
    } else {
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_get_key_value(key: c_int) -> *mut c_void {
    if key <= 0 {
        return std::ptr::null_mut();
    }
    let key_id = key as usize;
    let thread_id = cpython_current_thread_ident_u64();
    cpython_thread_tls_values()
        .lock()
        .ok()
        .and_then(|map| map.get(&(thread_id, key_id)).copied())
        .unwrap_or(0) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_delete_key_value(key: c_int) {
    if key <= 0 {
        return;
    }
    let key_id = key as usize;
    let thread_id = cpython_current_thread_ident_u64();
    if let Ok(mut map) = cpython_thread_tls_values().lock() {
        map.remove(&(thread_id, key_id));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_ReInitTLS() {
    let current = cpython_current_thread_ident_u64();
    if let Ok(mut map) = cpython_thread_tls_values().lock() {
        map.retain(|(thread_id, _), _| *thread_id == current);
    }
    if let Ok(mut map) = cpython_thread_tss_values().lock() {
        map.retain(|(thread_id, _), _| *thread_id == current);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_alloc() -> *mut c_void {
    Box::into_raw(Box::new(CpythonThreadTss {
        initialized: 0,
        key: 0,
    })) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_free(key: *mut c_void) {
    if key.is_null() {
        return;
    }
    unsafe { PyThread_tss_delete(key) };
    // SAFETY: pointer was allocated by PyThread_tss_alloc.
    unsafe {
        drop(Box::from_raw(key.cast::<CpythonThreadTss>()));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_is_created(key: *mut c_void) -> c_int {
    if key.is_null() {
        return 0;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &*key.cast::<CpythonThreadTss>() };
    if key_ref.initialized != 0 { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_create(key: *mut c_void) -> c_int {
    if key.is_null() {
        return -1;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &mut *key.cast::<CpythonThreadTss>() };
    if key_ref.initialized != 0 {
        return 0;
    }
    let key_id = CPYTHON_THREAD_TLS_NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut set) = cpython_thread_tss_registry().lock() {
        set.insert(key_id);
    }
    key_ref.key = key_id;
    key_ref.initialized = 1;
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_delete(key: *mut c_void) {
    if key.is_null() {
        return;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &mut *key.cast::<CpythonThreadTss>() };
    if key_ref.initialized == 0 {
        return;
    }
    let key_id = key_ref.key;
    if let Ok(mut set) = cpython_thread_tss_registry().lock() {
        set.remove(&key_id);
    }
    if let Ok(mut map) = cpython_thread_tss_values().lock() {
        map.retain(|(_, stored_key), _| *stored_key != key_id);
    }
    key_ref.key = 0;
    key_ref.initialized = 0;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_set(key: *mut c_void, value: *mut c_void) -> c_int {
    if key.is_null() {
        return -1;
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &*key.cast::<CpythonThreadTss>() };
    if key_ref.initialized == 0 {
        return -1;
    }
    let key_id = key_ref.key;
    let is_known = cpython_thread_tss_registry()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(&key_id));
    if !is_known {
        return -1;
    }
    let thread_id = cpython_current_thread_ident_u64();
    if let Ok(mut map) = cpython_thread_tss_values().lock() {
        map.insert((thread_id, key_id), value as usize);
        0
    } else {
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThread_tss_get(key: *mut c_void) -> *mut c_void {
    if key.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provided pointer is expected to reference Py_tss_t compatible storage.
    let key_ref = unsafe { &*key.cast::<CpythonThreadTss>() };
    if key_ref.initialized == 0 {
        return std::ptr::null_mut();
    }
    let thread_id = cpython_current_thread_ident_u64();
    cpython_thread_tss_values()
        .lock()
        .ok()
        .and_then(|map| map.get(&(thread_id, key_ref.key)).copied())
        .unwrap_or(0) as *mut c_void
}
