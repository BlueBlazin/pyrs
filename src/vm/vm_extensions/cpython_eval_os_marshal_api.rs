use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_double, c_int, c_long, c_ulong, c_void};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::bytecode::cpython::{marshal_dump_object, marshal_load_object};
use crate::runtime::Value;

use super::{
    Py_DecRef, PyEval_GetFrameGlobals, PyEval_GetFrameLocals, PyExc_KeyboardInterrupt,
    PyExc_SystemError, PyExc_ValueError, PyFrame_GetCode, PyObject_Call, PyThreadState_Get,
    cpython_current_thread_state_ptr, cpython_current_thread_state_ptr_unchecked,
    cpython_gil_acquire_for_current_thread, cpython_gil_current_thread_holds,
    cpython_gil_release_for_current_thread, cpython_is_known_thread_state_ptr,
    cpython_main_interpreter_state_ptr, cpython_mark_thread_runtime_initialized,
    cpython_marshal_object_to_value, cpython_set_current_thread_state_ptr, cpython_set_error,
    cpython_set_typed_error, cpython_take_pending_interrupt_signum,
    cpython_thread_runtime_initialized, value_to_cpython_marshal_object,
    with_active_cpython_context_mut,
};

unsafe extern "C" {
    fn strtol(string: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_long;
    fn strtoul(string: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_ulong;
    fn strtod(string: *const c_char, endptr: *mut *mut c_char) -> c_double;
}

const PY_GILSTATE_LOCKED: i32 = 0;
const PY_GILSTATE_UNLOCKED: i32 = 1;
const PY_MUTEX_LOCKED_BIT: u8 = 0x01;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_CheckSignals() -> i32 {
    if cpython_take_pending_interrupt_signum().is_some() {
        cpython_set_typed_error(unsafe { PyExc_KeyboardInterrupt }, "KeyboardInterrupt");
        return -1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGILState_Ensure() -> i32 {
    cpython_mark_thread_runtime_initialized();
    let had_gil = cpython_gil_current_thread_holds();
    let state = cpython_current_thread_state_ptr();
    if state == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyGILState_Ensure could not resolve current thread state",
        );
        return PY_GILSTATE_UNLOCKED;
    }
    cpython_set_current_thread_state_ptr(state);
    cpython_gil_acquire_for_current_thread();
    if had_gil {
        PY_GILSTATE_LOCKED
    } else {
        PY_GILSTATE_UNLOCKED
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGILState_GetThisThreadState() -> *mut c_void {
    cpython_current_thread_state_ptr_unchecked() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyGILState_Release(_state: i32) {
    if !cpython_gil_release_for_current_thread() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyGILState_Release called without matching PyGILState_Ensure",
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_AcquireLock() {
    cpython_mark_thread_runtime_initialized();
    let state = cpython_current_thread_state_ptr();
    if state != 0 {
        cpython_set_current_thread_state_ptr(state);
    }
    cpython_gil_acquire_for_current_thread();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_ReleaseLock() {
    let _ = cpython_gil_release_for_current_thread();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_AcquireThread(state: *mut c_void) {
    if state.is_null() || !cpython_is_known_thread_state_ptr(state as usize) {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_AcquireThread received unknown thread state",
        );
        return;
    }
    cpython_mark_thread_runtime_initialized();
    cpython_set_current_thread_state_ptr(state as usize);
    cpython_gil_acquire_for_current_thread();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_ReleaseThread(state: *mut c_void) {
    if state.is_null() || !cpython_is_known_thread_state_ptr(state as usize) {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_ReleaseThread received unknown thread state",
        );
        return;
    }
    let current = cpython_current_thread_state_ptr_unchecked();
    if current != 0 && current != state as usize {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_ReleaseThread called with non-current thread state",
        );
        return;
    }
    cpython_set_current_thread_state_ptr(state as usize);
    if !cpython_gil_release_for_current_thread() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_ReleaseThread called while thread does not own the GIL",
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_InitThreads() {
    // PyEval_InitThreads() is a compatibility no-op in CPython 3.14.
    cpython_mark_thread_runtime_initialized();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_ThreadsInitialized() -> i32 {
    i32::from(cpython_thread_runtime_initialized())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_CallObjectWithKeywords(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    unsafe { PyObject_Call(callable, args, kwargs) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalCode(
    code: *mut c_void,
    globals: *mut c_void,
    locals: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyEval_EvalCode missing VM context");
            return std::ptr::null_mut();
        }
        let Some(code_value) = context.cpython_value_from_ptr_or_proxy(code) else {
            context.set_error("PyEval_EvalCode received unknown code pointer");
            return std::ptr::null_mut();
        };
        let Value::Code(code_obj) = code_value else {
            context.set_error("PyEval_EvalCode expected code object");
            return std::ptr::null_mut();
        };
        if globals.is_null() {
            context.set_error("PyEval_EvalCode globals must not be NULL");
            return std::ptr::null_mut();
        }
        let Some(globals_value) = context.cpython_value_from_ptr_or_proxy(globals) else {
            context.set_error("PyEval_EvalCode received unknown globals pointer");
            return std::ptr::null_mut();
        };
        if !matches!(globals_value, Value::Dict(_) | Value::Module(_)) {
            context.set_error("PyEval_EvalCode globals must be a dict or module");
            return std::ptr::null_mut();
        }
        let locals_value = if locals.is_null() {
            globals_value.clone()
        } else {
            let Some(value) = context.cpython_value_from_ptr_or_proxy(locals) else {
                context.set_error("PyEval_EvalCode received unknown locals pointer");
                return std::ptr::null_mut();
            };
            value
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_eval(
            vec![Value::Code(code_obj), globals_value, locals_value],
            HashMap::new(),
        ) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalCodeEx(
    code: *mut c_void,
    globals: *mut c_void,
    locals: *mut c_void,
    args: *const *mut c_void,
    argcount: i32,
    kws: *const *mut c_void,
    kwcount: i32,
    defs: *const *mut c_void,
    defcount: i32,
    kwdefs: *mut c_void,
    closure: *mut c_void,
) -> *mut c_void {
    let is_simple_call = args.is_null()
        && kws.is_null()
        && defs.is_null()
        && argcount == 0
        && kwcount == 0
        && defcount == 0
        && kwdefs.is_null()
        && closure.is_null();
    if is_simple_call {
        return unsafe { PyEval_EvalCode(code, globals, locals) };
    }
    with_active_cpython_context_mut(|context| {
        context.set_error("PyEval_EvalCodeEx extended args/kws/defs/closure are not yet supported");
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
    });
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalFrame(frame: *mut c_void) -> *mut c_void {
    if frame.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_EvalFrame requires frame",
        );
        return std::ptr::null_mut();
    }
    if frame == unsafe { PyThreadState_Get() } {
        cpython_set_error("PyEval_EvalFrame current-frame evaluation is not yet supported");
        return std::ptr::null_mut();
    }
    let code = unsafe { PyFrame_GetCode(frame) };
    if code.is_null() {
        return std::ptr::null_mut();
    }
    let globals_from_frame = unsafe { PyEval_GetFrameGlobals() };
    let globals = if globals_from_frame.is_null() {
        let fallback = with_active_cpython_context_mut(|context| {
            context.alloc_cpython_ptr_for_value(Value::Module(context.module.clone()))
        })
        .unwrap_or_else(|_| std::ptr::null_mut());
        fallback
    } else {
        globals_from_frame
    };
    if globals.is_null() {
        unsafe { Py_DecRef(code) };
        return std::ptr::null_mut();
    }
    let locals = unsafe { PyEval_GetFrameLocals() };
    let locals_arg = if locals.is_null() { globals } else { locals };
    let result = unsafe { PyEval_EvalCode(code, globals, locals_arg) };
    unsafe {
        Py_DecRef(code);
        Py_DecRef(globals);
        if !locals.is_null() && locals != globals {
            Py_DecRef(locals);
        }
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_EvalFrameEx(frame: *mut c_void, _throwflag: i32) -> *mut c_void {
    unsafe { PyEval_EvalFrame(frame) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_SaveThread() -> *mut c_void {
    let state = cpython_current_thread_state_ptr();
    if state == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_SaveThread missing current thread state",
        );
        return std::ptr::null_mut();
    }
    if !cpython_gil_release_for_current_thread() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_SaveThread called while thread does not own the GIL",
        );
        return std::ptr::null_mut();
    }
    state as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_RestoreThread(state: *mut c_void) {
    if state.is_null() || !cpython_is_known_thread_state_ptr(state as usize) {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyEval_RestoreThread received unknown thread state",
        );
        return;
    }
    cpython_mark_thread_runtime_initialized();
    cpython_set_current_thread_state_ptr(state as usize);
    cpython_gil_acquire_for_current_thread();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_Main() -> *mut c_void {
    cpython_main_interpreter_state_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMutex_Lock(mutex: *mut c_void) {
    if mutex.is_null() {
        return;
    }
    let bits = mutex.cast::<AtomicU8>();
    let mut spins: usize = 0;
    loop {
        // SAFETY: caller provides a valid pointer to PyMutex-compatible storage.
        let observed = unsafe { (*bits).load(Ordering::Acquire) };
        if observed & PY_MUTEX_LOCKED_BIT == 0 {
            let desired = observed | PY_MUTEX_LOCKED_BIT;
            // SAFETY: caller provides a valid pointer to PyMutex-compatible storage.
            let acquired = unsafe {
                (*bits)
                    .compare_exchange_weak(observed, desired, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            };
            if acquired {
                return;
            }
            continue;
        }
        spins = spins.saturating_add(1);
        if spins.is_multiple_of(64) {
            std::thread::yield_now();
        } else {
            std::hint::spin_loop();
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMutex_Unlock(mutex: *mut c_void) {
    if mutex.is_null() {
        return;
    }
    let bits = mutex.cast::<AtomicU8>();
    loop {
        // SAFETY: caller provides a valid pointer to PyMutex-compatible storage.
        let observed = unsafe { (*bits).load(Ordering::Acquire) };
        if observed & PY_MUTEX_LOCKED_BIT == 0 {
            return;
        }
        let desired = observed & !PY_MUTEX_LOCKED_BIT;
        // SAFETY: caller provides a valid pointer to PyMutex-compatible storage.
        let released = unsafe {
            (*bits)
                .compare_exchange_weak(observed, desired, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        };
        if released {
            return;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_strtol(
    string: *const c_char,
    endptr: *mut *mut c_char,
    base: i32,
) -> c_long {
    unsafe { strtol(string, endptr, base as c_int) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_strtoul(
    string: *const c_char,
    endptr: *mut *mut c_char,
    base: i32,
) -> c_ulong {
    unsafe { strtoul(string, endptr, base as c_int) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_string_to_double(
    string: *const c_char,
    endptr: *mut *mut c_char,
    _overflow_exception: *mut c_void,
) -> c_double {
    unsafe { strtod(string, endptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_snprintf(
    buffer: *mut c_char,
    size: usize,
    format: *const c_char,
) -> i32 {
    if buffer.is_null() || size == 0 {
        return 0;
    }
    let text = if format.is_null() {
        ""
    } else {
        // SAFETY: caller provides NUL-terminated format string.
        unsafe { CStr::from_ptr(format) }.to_str().unwrap_or("")
    };
    let bytes = text.as_bytes();
    let writable = size.saturating_sub(1).min(bytes.len());
    // SAFETY: caller provided writable output buffer with length `size`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.cast::<u8>(), writable);
        *buffer.add(writable) = 0;
    }
    writable as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMarshal_WriteObjectToString(
    object: *mut c_void,
    _version: i32,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyMarshal_WriteObjectToString missing VM context");
            return std::ptr::null_mut();
        }
        let Some(value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("PyMarshal_WriteObjectToString received unknown object");
            return std::ptr::null_mut();
        };
        let marshal_object = match value_to_cpython_marshal_object(&value) {
            Ok(value) => value,
            Err(err) => {
                context.set_error(format!("PyMarshal_WriteObjectToString {err}"));
                return std::ptr::null_mut();
            }
        };
        let encoded = match marshal_dump_object(&marshal_object) {
            Ok(encoded) => encoded,
            Err(err) => {
                context.set_error(format!(
                    "PyMarshal_WriteObjectToString failed to encode object: {}",
                    err.message
                ));
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let encoded_value = unsafe { (&mut *context.vm).heap.alloc_bytes(encoded) };
        context.alloc_cpython_ptr_for_value(encoded_value)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMarshal_ReadObjectFromString(
    data: *const c_char,
    len: isize,
) -> *mut c_void {
    if data.is_null() || len < 0 {
        cpython_set_typed_error(
            unsafe { PyExc_ValueError },
            "PyMarshal_ReadObjectFromString requires non-null data and non-negative length",
        );
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyMarshal_ReadObjectFromString missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: caller guarantees `data` points to at least `len` bytes.
        let payload = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) };
        let decoded = match marshal_load_object(payload, true) {
            Ok(decoded) => decoded,
            Err(err) => {
                context.set_error(format!(
                    "PyMarshal_ReadObjectFromString failed to decode payload: {}",
                    err.message
                ));
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match cpython_marshal_object_to_value(&decoded, vm) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(format!("PyMarshal_ReadObjectFromString {err}"));
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}
