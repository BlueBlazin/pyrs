use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use crate::cli;
use crate::runtime::Value;
use crate::{compiler, parser};

use super::{
    CPYTHON_ARGC, CPYTHON_ARGV, CPYTHON_BUILD_INFO_TEXT, CPYTHON_COMPILER_TEXT,
    CPYTHON_EXEC_PREFIX_WIDE, CPYTHON_IS_FINALIZING, CPYTHON_IS_INITIALIZED, CPYTHON_PATH_WIDE,
    CPYTHON_PLATFORM_TEXT, CPYTHON_PREFIX_WIDE, CPYTHON_PROGRAM_FULL_PATH_WIDE,
    CPYTHON_PROGRAM_NAME_WIDE, CPYTHON_PYTHON_HOME_WIDE, CPYTHON_RECURSION_LIMIT,
    CPYTHON_REPR_STACK, CPYTHON_VERSION_TEXT, CpythonPendingCall, CpythonThreadStateCompat, Cwchar,
    PyErr_SetString, PyExc_SyntaxError, PyExc_SystemError, PyExc_TypeError,
    PyInterpreterState_Clear, PyInterpreterState_Delete, PyInterpreterState_New, PyMem_RawMalloc,
    PySys_SetPath, PyThreadState_Clear, PyThreadState_Delete, PyThreadState_New,
    PyThreadState_Swap, c_name_to_string, c_wide_name_to_string, cpython_atexit_callbacks,
    cpython_collect_sys_argv, cpython_current_thread_state_ptr_unchecked,
    cpython_get_or_init_wide_storage, cpython_is_known_interpreter_state_ptr,
    cpython_is_known_thread_state_ptr, cpython_main_interpreter_state_ptr, cpython_pending_calls,
    cpython_read_sys_path_string, cpython_read_sys_string, cpython_set_current_thread_state_ptr,
    cpython_set_error, cpython_set_typed_error, cpython_set_wide_storage, cpython_store_argv_wide,
    cpython_string_to_wide_units, cpython_thread_state_allocations, cpython_wide_ptr_to_string,
    with_active_cpython_context_mut,
};

const CPYTHON_PENDING_CALLS_MAX: usize = 32;
const CPYTHON_ATEXIT_CALLBACKS_MAX: usize = 32;

fn cpython_target_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    }
}

fn cpython_target_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn cpython_host_env_var(name: &str) -> Option<String> {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return None;
        }
        // SAFETY: active context holds a live VM pointer for this call.
        let vm = unsafe { &*context.vm };
        vm.host.env_var(name)
    })
    .ok()
    .flatten()
}

fn cpython_build_info_cstring() -> &'static CString {
    CPYTHON_BUILD_INFO_TEXT.get_or_init(|| {
        let package_version = env!("CARGO_PKG_VERSION");
        let target_arch = cpython_target_arch();
        let target_os = cpython_target_os();
        CString::new(format!("pyrs-{package_version}, {target_arch}-{target_os}"))
            .expect("build info should not contain interior NUL")
    })
}

fn cpython_compiler_cstring() -> &'static CString {
    CPYTHON_COMPILER_TEXT.get_or_init(|| {
        let rustc_version = option_env!("RUSTC_VERSION").unwrap_or("rustc");
        CString::new(format!("[Rust {rustc_version}]"))
            .expect("compiler info should not contain interior NUL")
    })
}

fn cpython_platform_cstring() -> &'static CString {
    CPYTHON_PLATFORM_TEXT
        .get_or_init(|| CString::new(cpython_target_os()).expect("platform should not contain interior NUL"))
}

fn cpython_version_cstring() -> &'static CString {
    CPYTHON_VERSION_TEXT.get_or_init(|| {
        let build = cpython_build_info_cstring().to_string_lossy();
        let compiler = cpython_compiler_cstring().to_string_lossy();
        CString::new(format!("3.14.0 ({build}) {compiler}"))
            .expect("version info should not contain interior NUL")
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_ReprEnter(object: *mut c_void) -> c_int {
    if object.is_null() {
        return 0;
    }
    CPYTHON_REPR_STACK.with(|stack| {
        let mut seen = stack.borrow_mut();
        let key = object as usize;
        if seen.contains(&key) {
            1
        } else {
            seen.push(key);
            0
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_ReprLeave(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    CPYTHON_REPR_STACK.with(|stack| {
        let mut seen = stack.borrow_mut();
        let key = object as usize;
        if let Some(index) = seen.iter().rposition(|entry| *entry == key) {
            seen.remove(index);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_AddPendingCall(
    func: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    arg: *mut c_void,
) -> c_int {
    let Some(func) = func else {
        return -1;
    };
    match cpython_pending_calls().lock() {
        Ok(mut queue) => {
            if queue.len() >= CPYTHON_PENDING_CALLS_MAX {
                return -1;
            }
            queue.push_back(CpythonPendingCall {
                func,
                arg: arg as usize,
            });
            0
        }
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_MakePendingCalls() -> c_int {
    for _ in 0..CPYTHON_PENDING_CALLS_MAX {
        let next = match cpython_pending_calls().lock() {
            Ok(mut queue) => queue.pop_front(),
            Err(_) => return -1,
        };
        let Some(pending) = next else {
            break;
        };
        // SAFETY: callback pointer was registered via `Py_AddPendingCall`.
        let status = unsafe { (pending.func)(pending.arg as *mut c_void) };
        if status != 0 {
            return -1;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_AtExit(func: Option<unsafe extern "C" fn()>) -> c_int {
    let Some(func) = func else {
        return -1;
    };
    match cpython_atexit_callbacks().lock() {
        Ok(mut callbacks) => {
            if callbacks.len() >= CPYTHON_ATEXIT_CALLBACKS_MAX {
                return -1;
            }
            callbacks.push(func);
            0
        }
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetRecursionLimit() -> c_int {
    let mut limit = CPYTHON_RECURSION_LIMIT.load(Ordering::Relaxed);
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active C-API context.
        let vm = unsafe { &mut *context.vm };
        limit = vm.recursion_limit;
    });
    CPYTHON_RECURSION_LIMIT.store(limit, Ordering::Relaxed);
    limit as c_int
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_SetRecursionLimit(new_limit: c_int) {
    let new_limit = new_limit as i64;
    CPYTHON_RECURSION_LIMIT.store(new_limit, Ordering::Relaxed);
    let _ = with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return;
        }
        // SAFETY: VM pointer is valid for active C-API context.
        unsafe {
            (*context.vm).recursion_limit = new_limit;
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetVersion() -> *const c_char {
    cpython_version_cstring().as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetBuildInfo() -> *const c_char {
    cpython_build_info_cstring().as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetCompiler() -> *const c_char {
    cpython_compiler_cstring().as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetPlatform() -> *const c_char {
    cpython_platform_cstring().as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetCopyright() -> *const c_char {
    c"Copyright (c) 2001 Python Software Foundation.\n\
All Rights Reserved.\n\
\n\
Copyright (c) 2000 BeOpen.com.\n\
All Rights Reserved.\n\
\n\
Copyright (c) 1995-2001 Corporation for National Research Initiatives.\n\
All Rights Reserved.\n\
\n\
Copyright (c) 1991-1995 Stichting Mathematisch Centrum, Amsterdam.\n\
All Rights Reserved."
        .as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_SetProgramName(name: *const Cwchar) {
    if name.is_null() {
        cpython_set_wide_storage(&CPYTHON_PROGRAM_NAME_WIDE, "");
        return;
    }
    if let Ok(decoded) = unsafe { c_wide_name_to_string(name) } {
        cpython_set_wide_storage(&CPYTHON_PROGRAM_NAME_WIDE, &decoded);
        if CPYTHON_PROGRAM_FULL_PATH_WIDE.load(Ordering::Relaxed) == 0 {
            cpython_set_wide_storage(&CPYTHON_PROGRAM_FULL_PATH_WIDE, &decoded);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetProgramName() -> *mut Cwchar {
    cpython_get_or_init_wide_storage(&CPYTHON_PROGRAM_NAME_WIDE, || {
        if let Some(executable) = cpython_read_sys_string("executable")
            && let Some(name) = Path::new(&executable).file_name()
        {
            return name.to_string_lossy().to_string();
        }
        "pyrs".to_string()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_SetPythonHome(home: *const Cwchar) {
    if home.is_null() {
        cpython_set_wide_storage(&CPYTHON_PYTHON_HOME_WIDE, "");
        return;
    }
    if let Ok(decoded) = unsafe { c_wide_name_to_string(home) } {
        cpython_set_wide_storage(&CPYTHON_PYTHON_HOME_WIDE, &decoded);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetPythonHome() -> *mut Cwchar {
    cpython_get_or_init_wide_storage(&CPYTHON_PYTHON_HOME_WIDE, || {
        cpython_host_env_var("PYTHONHOME")
            .or_else(|| cpython_read_sys_string("prefix"))
            .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_SetPath(path: *const Cwchar) {
    if path.is_null() {
        cpython_set_wide_storage(&CPYTHON_PATH_WIDE, "");
        return;
    }
    if let Ok(decoded) = unsafe { c_wide_name_to_string(path) } {
        cpython_set_wide_storage(&CPYTHON_PATH_WIDE, &decoded);
        let _ = with_active_cpython_context_mut(|_| {
            unsafe { PySys_SetPath(path) };
        });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetPath() -> *mut Cwchar {
    cpython_get_or_init_wide_storage(&CPYTHON_PATH_WIDE, || {
        cpython_read_sys_path_string().unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetPrefix() -> *mut Cwchar {
    cpython_get_or_init_wide_storage(&CPYTHON_PREFIX_WIDE, || {
        cpython_read_sys_string("prefix").unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetExecPrefix() -> *mut Cwchar {
    cpython_get_or_init_wide_storage(&CPYTHON_EXEC_PREFIX_WIDE, || {
        cpython_read_sys_string("exec_prefix").unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetProgramFullPath() -> *mut Cwchar {
    cpython_get_or_init_wide_storage(&CPYTHON_PROGRAM_FULL_PATH_WIDE, || {
        cpython_read_sys_string("executable").unwrap_or_else(|| "pyrs".to_string())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetArgcArgv(argc: *mut c_int, argv: *mut *mut *mut Cwchar) {
    if CPYTHON_ARGV.load(Ordering::Relaxed) == 0
        && let Some(collected) = cpython_collect_sys_argv()
    {
        cpython_store_argv_wide(&collected);
    }
    if !argc.is_null() {
        // SAFETY: caller provided output pointer.
        unsafe {
            *argc = CPYTHON_ARGC.load(Ordering::Relaxed) as c_int;
        }
    }
    if !argv.is_null() {
        // SAFETY: caller provided output pointer.
        unsafe {
            *argv = CPYTHON_ARGV.load(Ordering::Relaxed) as *mut *mut Cwchar;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_DecodeLocale(arg: *const c_char, wlen: *mut usize) -> *mut Cwchar {
    if arg.is_null() {
        if !wlen.is_null() {
            // SAFETY: caller provided output pointer.
            unsafe {
                *wlen = usize::MAX;
            }
        }
        return std::ptr::null_mut();
    }
    // SAFETY: caller guarantees a valid NUL-terminated bytes string.
    let bytes = unsafe { CStr::from_ptr(arg) }.to_bytes();
    let decoded = String::from_utf8_lossy(bytes).into_owned();
    let units = cpython_string_to_wide_units(&decoded);
    if !wlen.is_null() {
        // SAFETY: caller provided output pointer.
        unsafe {
            *wlen = units.len();
        }
    }
    let byte_len = match units
        .len()
        .checked_add(1)
        .and_then(|count| count.checked_mul(std::mem::size_of::<Cwchar>()))
    {
        Some(len) => len,
        None => {
            if !wlen.is_null() {
                // SAFETY: caller provided output pointer.
                unsafe {
                    *wlen = usize::MAX;
                }
            }
            return std::ptr::null_mut();
        }
    };
    let out = unsafe { PyMem_RawMalloc(byte_len) }.cast::<Cwchar>();
    if out.is_null() {
        if !wlen.is_null() {
            // SAFETY: caller provided output pointer.
            unsafe {
                *wlen = usize::MAX;
            }
        }
        return std::ptr::null_mut();
    }
    // SAFETY: destination buffer is sized for `units + trailing NUL`.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len());
        *out.add(units.len()) = 0;
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_EncodeLocale(
    text: *const Cwchar,
    error_pos: *mut usize,
) -> *mut c_char {
    if text.is_null() {
        if !error_pos.is_null() {
            // SAFETY: caller provided output pointer.
            unsafe {
                *error_pos = 0;
            }
        }
        return std::ptr::null_mut();
    }
    let decoded = match unsafe { cpython_wide_ptr_to_string(text, -1, "Py_EncodeLocale") } {
        Ok(value) => value,
        Err(_) => {
            if !error_pos.is_null() {
                // SAFETY: caller provided output pointer.
                unsafe {
                    *error_pos = 0;
                }
            }
            return std::ptr::null_mut();
        }
    };
    let bytes = decoded.into_bytes();
    let out = unsafe { PyMem_RawMalloc(bytes.len().saturating_add(1)) }.cast::<c_char>();
    if out.is_null() {
        if !error_pos.is_null() {
            // SAFETY: caller provided output pointer.
            unsafe {
                *error_pos = 0;
            }
        }
        return std::ptr::null_mut();
    }
    // SAFETY: destination has at least `bytes.len() + 1` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, bytes.len());
        *out.add(bytes.len()) = 0;
    }
    if !error_pos.is_null() {
        // SAFETY: caller provided output pointer.
        unsafe {
            *error_pos = usize::MAX;
        }
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_PACK_FULL_VERSION(
    major: c_int,
    minor: c_int,
    micro: c_int,
    level: c_int,
    serial: c_int,
) -> u32 {
    ((major as u32 & 0xff) << 24)
        | ((minor as u32 & 0xff) << 16)
        | ((micro as u32 & 0xff) << 8)
        | ((level as u32 & 0x0f) << 4)
        | (serial as u32 & 0x0f)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_PACK_VERSION(major: c_int, minor: c_int) -> u32 {
    unsafe { Py_PACK_FULL_VERSION(major, minor, 0, 0, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsFinalizing() -> c_int {
    c_int::from(CPYTHON_IS_FINALIZING.load(Ordering::Relaxed) != 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_Initialize() {
    CPYTHON_IS_FINALIZING.store(0, Ordering::Relaxed);
    CPYTHON_IS_INITIALIZED.store(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_InitializeEx(_initsigs: c_int) {
    unsafe { Py_Initialize() };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_FinalizeEx() -> c_int {
    if CPYTHON_IS_INITIALIZED.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    CPYTHON_IS_FINALIZING.store(1, Ordering::Relaxed);
    if let Ok(mut callbacks) = cpython_atexit_callbacks().lock() {
        while let Some(callback) = callbacks.pop() {
            // SAFETY: callback pointer was registered via `Py_AtExit`.
            unsafe { callback() };
        }
    }
    CPYTHON_IS_INITIALIZED.store(0, Ordering::Relaxed);
    CPYTHON_IS_FINALIZING.store(0, Ordering::Relaxed);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_Finalize() {
    let _ = unsafe { Py_FinalizeEx() };
}

const PY_SINGLE_INPUT: c_int = 256;
const PY_FILE_INPUT: c_int = 257;
const PY_EVAL_INPUT: c_int = 258;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_CompileString(
    source: *const c_char,
    filename: *const c_char,
    start: c_int,
) -> *mut c_void {
    if source.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: caller provides NUL-terminated source for non-null pointer.
    let source_text = match unsafe { c_name_to_string(source) } {
        Ok(text) => text,
        Err(err) => {
            cpython_set_error(format!("Py_CompileString {err}"));
            return std::ptr::null_mut();
        }
    };
    let filename_text = if filename.is_null() {
        "<string>".to_string()
    } else {
        // SAFETY: caller provides NUL-terminated filename for non-null pointer.
        match unsafe { c_name_to_string(filename) } {
            Ok(text) => text,
            Err(err) => {
                cpython_set_error(format!("Py_CompileString {err}"));
                return std::ptr::null_mut();
            }
        }
    };

    let compiled = match start {
        PY_FILE_INPUT | PY_SINGLE_INPUT => {
            let module = match parser::parse_module(&source_text) {
                Ok(module) => module,
                Err(err) => {
                    cpython_set_typed_error(
                        unsafe { PyExc_SyntaxError },
                        format!(
                            "parse error at {} (line {}, column {}): {}",
                            err.offset, err.line, err.column, err.message
                        ),
                    );
                    return std::ptr::null_mut();
                }
            };
            match compiler::compile_module_with_filename(&module, &filename_text) {
                Ok(code) => code,
                Err(err) => {
                    cpython_set_typed_error(
                        unsafe { PyExc_SyntaxError },
                        format!("compile error: {}", err.message),
                    );
                    return std::ptr::null_mut();
                }
            }
        }
        PY_EVAL_INPUT => {
            let expr = match parser::parse_expression(&source_text) {
                Ok(expr) => expr,
                Err(err) => {
                    cpython_set_typed_error(
                        unsafe { PyExc_SyntaxError },
                        format!(
                            "parse error at {} (line {}, column {}): {}",
                            err.offset, err.line, err.column, err.message
                        ),
                    );
                    return std::ptr::null_mut();
                }
            };
            match compiler::compile_expression_with_filename(&expr, &filename_text) {
                Ok(code) => code,
                Err(err) => {
                    cpython_set_typed_error(
                        unsafe { PyExc_SyntaxError },
                        format!("compile error: {}", err.message),
                    );
                    return std::ptr::null_mut();
                }
            }
        }
        _ => {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                format!("Py_CompileString received invalid start mode: {start}"),
            );
            return std::ptr::null_mut();
        }
    };

    with_active_cpython_context_mut(|context| {
        context.alloc_cpython_ptr_for_value(Value::Code(Rc::new(compiled)))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_Main(argc: c_int, argv: *mut *mut Cwchar) -> c_int {
    if argc < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Py_Main received negative argc",
        );
        return 2;
    }
    let argc_usize = argc as usize;
    let mut all_args = Vec::with_capacity(argc_usize);
    for idx in 0..argc_usize {
        let arg_ptr = if argv.is_null() {
            std::ptr::null()
        } else {
            // SAFETY: caller guarantees `argv` has `argc` entries when non-null.
            unsafe { *argv.add(idx) }
        };
        if arg_ptr.is_null() {
            all_args.push(String::new());
            continue;
        }
        // SAFETY: each argv entry is expected to be NUL-terminated.
        let arg = match unsafe { cpython_wide_ptr_to_string(arg_ptr, -1, "Py_Main") } {
            Ok(value) => value,
            Err(err) => {
                cpython_set_error(err);
                return 2;
            }
        };
        all_args.push(arg);
    }
    if let Some(program) = all_args.first() {
        cpython_set_wide_storage(&CPYTHON_PROGRAM_NAME_WIDE, program);
        cpython_set_wide_storage(&CPYTHON_PROGRAM_FULL_PATH_WIDE, program);
    }
    cpython_store_argv_wide(&all_args);
    let cli_args = all_args.into_iter().skip(1).collect();
    cli::run_with_args_vec(cli_args)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_BytesMain(argc: c_int, argv: *mut *mut c_char) -> c_int {
    if argc < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Py_BytesMain received negative argc",
        );
        return 2;
    }
    let argc_usize = argc as usize;
    let mut all_args = Vec::with_capacity(argc_usize);
    for idx in 0..argc_usize {
        let arg_ptr = if argv.is_null() {
            std::ptr::null()
        } else {
            // SAFETY: caller guarantees `argv` has `argc` entries when non-null.
            unsafe { *argv.add(idx) }
        };
        if arg_ptr.is_null() {
            all_args.push(String::new());
            continue;
        }
        // SAFETY: argv entries are NUL-terminated C strings.
        let arg = unsafe { CStr::from_ptr(arg_ptr) }
            .to_string_lossy()
            .to_string();
        all_args.push(arg);
    }
    if let Some(program) = all_args.first() {
        cpython_set_wide_storage(&CPYTHON_PROGRAM_NAME_WIDE, program);
        cpython_set_wide_storage(&CPYTHON_PROGRAM_FULL_PATH_WIDE, program);
    }
    cpython_store_argv_wide(&all_args);
    let cli_args = all_args.into_iter().skip(1).collect();
    cli::run_with_args_vec(cli_args)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_Exit(status: c_int) {
    let _ = unsafe { Py_FinalizeEx() };
    std::process::exit(status);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_FatalError(message: *const c_char) {
    let rendered = if message.is_null() {
        "Fatal Python error".to_string()
    } else {
        // SAFETY: caller provides NUL-terminated message.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .to_string()
    };
    eprintln!("{rendered}");
    std::process::abort();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_FatalErrorFunc(func: *const c_char, message: *const c_char) {
    let rendered_func = if func.is_null() {
        "<unknown>".to_string()
    } else {
        // SAFETY: caller provides a NUL-terminated function name.
        unsafe { CStr::from_ptr(func) }
            .to_string_lossy()
            .to_string()
    };
    let rendered_message = if message.is_null() {
        "Fatal Python error".to_string()
    } else {
        // SAFETY: caller provides a NUL-terminated message.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .to_string()
    };
    eprintln!("Fatal Python error: {rendered_func}: {rendered_message}");
    std::process::abort();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_NewInterpreter() -> *mut c_void {
    let interp = unsafe { PyInterpreterState_New() };
    if interp.is_null() {
        return std::ptr::null_mut();
    }
    let state = unsafe { PyThreadState_New(interp) };
    if state.is_null() {
        unsafe { PyInterpreterState_Delete(interp) };
        return std::ptr::null_mut();
    }
    unsafe { PyThreadState_Swap(state) };
    state
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_EndInterpreter(state: *mut c_void) {
    if state.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Py_EndInterpreter requires non-null thread state",
        );
        return;
    }
    let state_ptr = state as usize;
    if !cpython_is_known_thread_state_ptr(state_ptr) {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Py_EndInterpreter received unknown thread state",
        );
        return;
    }
    if cpython_current_thread_state_ptr_unchecked() != state_ptr {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Py_EndInterpreter requires current thread state",
        );
        return;
    }

    // SAFETY: `state_ptr` is validated as a known thread-state allocation above.
    let interp_ptr = unsafe { (*(state_ptr as *mut CpythonThreadStateCompat)).interp as usize };
    if interp_ptr == 0 || interp_ptr == cpython_main_interpreter_state_ptr() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Py_EndInterpreter cannot close main interpreter",
        );
        return;
    }
    if !cpython_is_known_interpreter_state_ptr(interp_ptr) {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "Py_EndInterpreter received thread state with unknown interpreter",
        );
        return;
    }

    let mut states_for_interp: Vec<usize> = match cpython_thread_state_allocations().lock() {
        Ok(set) => set
            .iter()
            .copied()
            .filter(|candidate| {
                let state_raw = *candidate as *mut CpythonThreadStateCompat;
                // SAFETY: pointers in `cpython_thread_state_allocations` come from
                // `PyThreadState_New` and have `CpythonThreadStateCompat` layout.
                unsafe { (*state_raw).interp as usize == interp_ptr }
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    if !states_for_interp.contains(&state_ptr) {
        states_for_interp.push(state_ptr);
    }
    states_for_interp.sort_unstable();
    states_for_interp.dedup();
    states_for_interp.sort_by_key(|candidate| usize::from(*candidate == state_ptr));

    unsafe { PyInterpreterState_Clear(interp_ptr as *mut c_void) };
    for candidate in states_for_interp {
        let raw = candidate as *mut c_void;
        unsafe { PyThreadState_Clear(raw) };
        unsafe { PyThreadState_Delete(raw) };
    }
    // CPython leaves no current thread-state after subinterpreter teardown; callers
    // are expected to restore a previous state explicitly.
    cpython_set_current_thread_state_ptr(0);
    unsafe { PyInterpreterState_Delete(interp_ptr as *mut c_void) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyErr_BadInternalCall(_filename: *const c_char, _lineno: i32) {
    cpython_set_error("bad internal call");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_BadInternalCall() {
    unsafe { _PyErr_BadInternalCall(std::ptr::null(), 0) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_BadArgument() -> i32 {
    // SAFETY: exception singletons are process-lifetime globals.
    unsafe {
        PyErr_SetString(
            PyExc_TypeError,
            c"bad argument type for built-in operation".as_ptr(),
        )
    };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Py_HashDouble(_inst: *mut c_void, value: f64) -> isize {
    if value.is_nan() {
        return 0;
    }
    let bits = value.to_bits() as i64;
    bits as isize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsWhitespace(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_whitespace()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsAlpha(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_alphabetic()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsDecimalDigit(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_ascii_digit()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_ToDecimalDigit(ch: u32) -> i32 {
    char::from_u32(ch)
        .and_then(|value| value.to_digit(10))
        .map(|value| value as i32)
        .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsDigit(ch: u32) -> i32 {
    unsafe { _PyUnicode_IsDecimalDigit(ch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsNumeric(ch: u32) -> i32 {
    unsafe { _PyUnicode_IsDecimalDigit(ch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsLowercase(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_lowercase()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsUppercase(ch: u32) -> i32 {
    char::from_u32(ch)
        .map(|value| i32::from(value.is_uppercase()))
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyUnicode_IsTitlecase(ch: u32) -> i32 {
    // Rust stdlib does not expose titlecase directly; use uppercase heuristic.
    unsafe { _PyUnicode_IsUppercase(ch) }
}
