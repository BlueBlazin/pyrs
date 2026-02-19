use std::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::atomic::Ordering;

use crate::runtime::{ModuleObject, Value};

use super::{
    _Py_EllipsisObject, _Py_FalseStruct, _Py_NoneStruct, _Py_NotImplementedStruct, _Py_TrueStruct,
    CPYTHON_CONSTANT_EMPTY_BYTES_PTR, CPYTHON_CONSTANT_EMPTY_STR_PTR,
    CPYTHON_CONSTANT_EMPTY_TUPLE_PTR, CPYTHON_CONSTANT_ONE_PTR, CPYTHON_CONSTANT_ZERO_PTR,
    CPYTHON_IS_INITIALIZED, CPYTHON_THREAD_STATE_COMPAT_SIZE, CURRENT_THREAD_STATE_PTR,
    CpythonModuleDef, CpythonObjectHead, CpythonThreadStateCompat, Object, Py_IncRef,
    PyBytes_FromStringAndSize, PyErr_BadInternalCall, PyExc_SystemError, PyLong_FromLong,
    PyTuple_New, PyUnicode_FromStringAndSize, calloc, cpython_bind_module_def,
    cpython_current_thread_state_ptr, cpython_get_or_init_constant_ptr,
    cpython_init_thread_state_compat, cpython_interpreter_state_allocations,
    cpython_is_known_interpreter_state_ptr, cpython_is_known_thread_state_ptr,
    cpython_main_interpreter_state_ptr, cpython_main_thread_state_ptr, cpython_set_error,
    cpython_set_typed_error, cpython_thread_state_allocations, free, vm_current_thread_ident,
    with_active_cpython_context_mut,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_Get() -> *mut c_void {
    cpython_current_thread_state_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_GetUnchecked() -> *mut c_void {
    cpython_current_thread_state_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_New(interp: *mut c_void) -> *mut c_void {
    if interp.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyThreadState_New requires interpreter",
        );
        return std::ptr::null_mut();
    }
    // SAFETY: allocate a writable thread-state compatibility block so native code
    // that reads/writes PyThreadState fields does not access past a token-sized buffer.
    let raw = unsafe { calloc(1, CPYTHON_THREAD_STATE_COMPAT_SIZE) } as usize;
    if raw == 0 {
        cpython_set_error("PyThreadState_New failed to allocate thread state");
        return std::ptr::null_mut();
    }
    let state = cpython_init_thread_state_compat(raw as *mut CpythonThreadStateCompat, interp);
    if state.is_null() {
        cpython_set_error("PyThreadState_New failed to initialize thread state");
        return std::ptr::null_mut();
    }
    if let Ok(mut set) = cpython_thread_state_allocations().lock() {
        set.insert(raw);
    }
    raw as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyThreadState_Prealloc(interp: *mut c_void) -> *mut c_void {
    unsafe { PyThreadState_New(interp) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyThreadState_Init(_thread_state: *mut c_void) {
    cpython_set_typed_error(
        unsafe { PyExc_SystemError },
        "_PyThreadState_Init() is for internal use only",
    );
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_Swap(new_thread_state: *mut c_void) -> *mut c_void {
    let previous = cpython_current_thread_state_ptr() as *mut c_void;
    if !new_thread_state.is_null() {
        let new_ptr = new_thread_state as usize;
        if !cpython_is_known_thread_state_ptr(new_ptr) {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "PyThreadState_Swap received unknown thread state",
            );
            return std::ptr::null_mut();
        }
    }
    CURRENT_THREAD_STATE_PTR.store(new_thread_state as usize, Ordering::Relaxed);
    previous
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_Clear(thread_state: *mut c_void) {
    if thread_state.is_null() {
        return;
    }
    if !cpython_is_known_thread_state_ptr(thread_state as usize) {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyThreadState_Clear received unknown thread state",
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_Delete(thread_state: *mut c_void) {
    if thread_state.is_null() {
        return;
    }
    let ptr = thread_state as usize;
    if ptr == cpython_main_thread_state_ptr() {
        return;
    }
    let removed = cpython_thread_state_allocations()
        .lock()
        .ok()
        .is_some_and(|mut set| set.remove(&ptr));
    if removed {
        if CURRENT_THREAD_STATE_PTR.load(Ordering::Relaxed) == ptr {
            CURRENT_THREAD_STATE_PTR.store(cpython_main_thread_state_ptr(), Ordering::Relaxed);
        }
        // SAFETY: pointer was allocated in PyThreadState_New and removed from registry once.
        unsafe {
            free(thread_state);
        }
    } else {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyThreadState_Delete received unknown thread state",
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_DeleteCurrent() {
    let current = cpython_current_thread_state_ptr();
    if current == cpython_main_thread_state_ptr() {
        return;
    }
    unsafe { PyThreadState_Delete(current as *mut c_void) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_SetAsyncExc(id: u64, _exc: *mut c_void) -> i32 {
    if id == vm_current_thread_ident() as u64 {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_GetFrame(state: *mut c_void) -> *mut c_void {
    if state.is_null() {
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(active) = vm.frames.last() else {
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(Value::Code(active.code.clone()))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_New(
    _state: *mut c_void,
    code: *mut c_void,
    globals: *mut c_void,
    locals: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyFrame_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        let frame_obj = match vm
            .heap
            .alloc_module(ModuleObject::new("__pyframe__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *frame_obj.kind_mut() {
            let code_value = context
                .cpython_value_from_ptr_or_proxy(code)
                .unwrap_or(Value::None);
            let globals_value = context
                .cpython_value_from_ptr_or_proxy(globals)
                .unwrap_or(Value::None);
            let locals_value = context
                .cpython_value_from_ptr_or_proxy(locals)
                .unwrap_or(Value::None);
            module_data.globals.insert("f_code".to_string(), code_value);
            module_data
                .globals
                .insert("f_globals".to_string(), globals_value);
            module_data
                .globals
                .insert("f_locals".to_string(), locals_value);
        }
        context.alloc_cpython_ptr_for_value(Value::Module(frame_obj))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetCode(frame: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if frame.is_null() {
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyFrame_GetCode missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let current_frame_ptr = unsafe { PyThreadState_Get() };
        if frame == current_frame_ptr {
            let Some(active) = vm.frames.last() else {
                return std::ptr::null_mut();
            };
            return context.alloc_cpython_ptr_for_value(Value::Code(active.code.clone()));
        }
        if let Some(Value::Code(code_obj)) = context.cpython_value_from_ptr_or_proxy(frame) {
            return context.alloc_cpython_ptr_for_value(Value::Code(code_obj));
        }
        context.set_error("PyFrame_GetCode expected frame object");
        std::ptr::null_mut()
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetBack(frame: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if frame.is_null() {
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let current_frame_ptr = unsafe { PyThreadState_Get() };
        if frame == current_frame_ptr {
            if vm.frames.len() < 2 {
                return std::ptr::null_mut();
            }
            let back_frame = &vm.frames[vm.frames.len() - 2];
            return context.alloc_cpython_ptr_for_value(Value::Code(back_frame.code.clone()));
        }
        if let Some(Value::Module(frame_obj)) = context.cpython_value_from_ptr_or_proxy(frame)
            && let Object::Module(module_data) = &*frame_obj.kind()
            && let Some(back) = module_data.globals.get("f_back").cloned()
            && !matches!(back, Value::None)
        {
            return context.alloc_cpython_ptr_for_value(back);
        }
        std::ptr::null_mut()
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrame_GetLineNumber(frame: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        if frame.is_null() {
            return 0;
        }
        if context.vm.is_null() {
            return 0;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let current_frame_ptr = unsafe { PyThreadState_Get() };
        if frame == current_frame_ptr {
            let Some(active) = vm.frames.last() else {
                return 0;
            };
            let ip = if active.last_ip < active.code.locations.len() {
                active.last_ip
            } else {
                active.code.locations.len().saturating_sub(1)
            };
            return active
                .code
                .locations
                .get(ip)
                .map(|loc| loc.line as i32)
                .unwrap_or(0);
        }
        if let Some(Value::Code(code_obj)) = context.cpython_value_from_ptr_or_proxy(frame) {
            return code_obj
                .locations
                .first()
                .map(|loc| loc.line as i32)
                .unwrap_or(0);
        }
        0
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_Get() -> *mut c_void {
    cpython_main_interpreter_state_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_New() -> *mut c_void {
    // SAFETY: allocate opaque interpreter-state token for CPython-ABI compatibility.
    let raw = unsafe { calloc(1, 1) } as usize;
    if raw == 0 {
        cpython_set_error("PyInterpreterState_New failed to allocate interpreter state");
        return std::ptr::null_mut();
    }
    if let Ok(mut set) = cpython_interpreter_state_allocations().lock() {
        set.insert(raw);
    }
    raw as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_Clear(interp: *mut c_void) {
    if interp.is_null() {
        return;
    }
    if !cpython_is_known_interpreter_state_ptr(interp as usize) {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyInterpreterState_Clear received unknown interpreter state",
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_Delete(interp: *mut c_void) {
    if interp.is_null() {
        return;
    }
    let ptr = interp as usize;
    if ptr == cpython_main_interpreter_state_ptr() {
        return;
    }
    let removed = cpython_interpreter_state_allocations()
        .lock()
        .ok()
        .is_some_and(|mut set| set.remove(&ptr));
    if removed {
        // SAFETY: pointer was allocated in PyInterpreterState_New and removed once.
        unsafe {
            free(interp);
        }
    } else {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyInterpreterState_Delete received unknown interpreter state",
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_GetID(interp: *mut c_void) -> i64 {
    if interp.is_null() {
        return -1;
    }
    let raw = interp as usize;
    if !cpython_is_known_interpreter_state_ptr(raw) {
        return -1;
    }
    if raw == cpython_main_interpreter_state_ptr() {
        return 1;
    }
    raw as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInterpreterState_GetDict(interp: *mut c_void) -> *mut c_void {
    if interp.is_null() {
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| context.ensure_interpreter_state_dict_pointer())
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            std::ptr::null_mut()
        })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyState_AddModule(module: *mut c_void, module_def: *mut c_void) -> i32 {
    if module_def.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyState_AddModule requires module definition",
        );
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let module_def = module_def.cast::<CpythonModuleDef>();
        // SAFETY: `module_def` was validated non-null above.
        if unsafe { !(*module_def).m_slots.is_null() } {
            context.set_error("SystemError: PyState_AddModule called on module with slots");
            return -1;
        }
        let Some(module_value) = context.cpython_value_from_ptr_or_proxy(module) else {
            context.set_error("PyState_AddModule received unknown module pointer");
            return -1;
        };
        let Value::Module(module_obj) = module_value else {
            context.set_error("PyState_AddModule expected module object");
            return -1;
        };
        if let Err(err) = cpython_bind_module_def(context, &module_obj, module_def) {
            context.set_error(err);
            return -1;
        }
        context
            .state_modules_by_def
            .insert(module_def as usize, module as usize);
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyState_AddModule(
    _thread_state: *mut c_void,
    module: *mut c_void,
    module_def: *mut c_void,
) -> i32 {
    unsafe { PyState_AddModule(module, module_def) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyState_FindModule(module_def: *mut c_void) -> *mut c_void {
    if module_def.is_null() {
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        let module_def = module_def.cast::<CpythonModuleDef>();
        // SAFETY: `module_def` is non-null and points to extension-provided def storage.
        if unsafe { !(*module_def).m_slots.is_null() } {
            return std::ptr::null_mut();
        }
        if let Some(module_ptr) = context
            .state_modules_by_def
            .get(&(module_def as usize))
            .copied()
        {
            return module_ptr as *mut c_void;
        }
        if context.vm.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some((module_id, _)) = vm
            .extension_module_def_registry
            .iter()
            .find(|(_, def_ptr)| **def_ptr == module_def as usize)
        else {
            return std::ptr::null_mut();
        };
        let module_obj = vm
            .modules
            .values()
            .find(|module| module.id() == *module_id)
            .cloned();
        let Some(module_obj) = module_obj else {
            return std::ptr::null_mut();
        };
        let module_ptr = context.alloc_cpython_ptr_for_value(Value::Module(module_obj));
        if !module_ptr.is_null() {
            context
                .state_modules_by_def
                .insert(module_def as usize, module_ptr as usize);
        }
        module_ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyState_RemoveModule(module_def: *mut c_void) -> i32 {
    if module_def.is_null() {
        return -1;
    }
    with_active_cpython_context_mut(|context| {
        let module_def = module_def.cast::<CpythonModuleDef>();
        // SAFETY: `module_def` is non-null and points to extension-provided def storage.
        if unsafe { !(*module_def).m_slots.is_null() } {
            context.set_error("SystemError: PyState_RemoveModule called on module with slots");
            return -1;
        }
        context.state_modules_by_def.remove(&(module_def as usize));
        if context.vm.is_null() {
            return 0;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        if let Some((module_id, _)) = vm
            .extension_module_def_registry
            .iter()
            .find(|(_, def_ptr)| **def_ptr == module_def as usize)
            .map(|(module_id, def_ptr)| (*module_id, *def_ptr))
        {
            let _ = module_id;
            vm.extension_module_def_registry.remove(&module_id);
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_GetInterpreter(state: *mut c_void) -> *mut c_void {
    if state.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { PyInterpreterState_Get() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_GetID(state: *mut c_void) -> u64 {
    if state.is_null() {
        return u64::MAX;
    }
    vm_current_thread_ident() as u64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyThreadState_GetDict() -> *mut c_void {
    with_active_cpython_context_mut(|context| context.ensure_thread_state_dict_pointer())
        .unwrap_or_else(|err| {
            cpython_set_error(err);
            std::ptr::null_mut()
        })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceMalloc_Track(_domain: usize, _ptr: usize, _size: usize) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyTraceMalloc_Untrack(_domain: usize, _ptr: usize) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_EnterRecursiveCall(_where: *const c_char) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_LeaveRecursiveCall() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsInitialized() -> i32 {
    i32::from(CPYTHON_IS_INITIALIZED.load(Ordering::Relaxed) != 0)
}

const PY_CONSTANT_NONE: u32 = 0;
const PY_CONSTANT_FALSE: u32 = 1;
const PY_CONSTANT_TRUE: u32 = 2;
const PY_CONSTANT_ELLIPSIS: u32 = 3;
const PY_CONSTANT_NOT_IMPLEMENTED: u32 = 4;
const PY_CONSTANT_ZERO: u32 = 5;
const PY_CONSTANT_ONE: u32 = 6;
const PY_CONSTANT_EMPTY_STR: u32 = 7;
const PY_CONSTANT_EMPTY_BYTES: u32 = 8;
const PY_CONSTANT_EMPTY_TUPLE: u32 = 9;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetConstant(constant_id: c_uint) -> *mut c_void {
    match constant_id {
        PY_CONSTANT_NONE => std::ptr::addr_of_mut!(_Py_NoneStruct).cast(),
        PY_CONSTANT_FALSE => std::ptr::addr_of_mut!(_Py_FalseStruct).cast(),
        PY_CONSTANT_TRUE => std::ptr::addr_of_mut!(_Py_TrueStruct).cast(),
        PY_CONSTANT_ELLIPSIS => std::ptr::addr_of_mut!(_Py_EllipsisObject).cast(),
        PY_CONSTANT_NOT_IMPLEMENTED => std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast(),
        PY_CONSTANT_ZERO => {
            cpython_get_or_init_constant_ptr(&CPYTHON_CONSTANT_ZERO_PTR, || unsafe {
                PyLong_FromLong(0)
            })
        }
        PY_CONSTANT_ONE => cpython_get_or_init_constant_ptr(&CPYTHON_CONSTANT_ONE_PTR, || unsafe {
            PyLong_FromLong(1)
        }),
        PY_CONSTANT_EMPTY_STR => {
            cpython_get_or_init_constant_ptr(&CPYTHON_CONSTANT_EMPTY_STR_PTR, || unsafe {
                PyUnicode_FromStringAndSize(std::ptr::null(), 0)
            })
        }
        PY_CONSTANT_EMPTY_BYTES => {
            cpython_get_or_init_constant_ptr(&CPYTHON_CONSTANT_EMPTY_BYTES_PTR, || unsafe {
                PyBytes_FromStringAndSize(std::ptr::null(), 0)
            })
        }
        PY_CONSTANT_EMPTY_TUPLE => {
            cpython_get_or_init_constant_ptr(&CPYTHON_CONSTANT_EMPTY_TUPLE_PTR, || unsafe {
                PyTuple_New(0)
            })
        }
        _ => {
            unsafe { PyErr_BadInternalCall() };
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_GetConstantBorrowed(constant_id: c_uint) -> *mut c_void {
    unsafe { Py_GetConstant(constant_id) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_Is(left: *mut c_void, right: *mut c_void) -> c_int {
    i32::from(std::ptr::eq(left, right))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsNone(object: *mut c_void) -> c_int {
    i32::from(std::ptr::eq(
        object,
        std::ptr::addr_of_mut!(_Py_NoneStruct).cast::<c_void>(),
    ))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsTrue(object: *mut c_void) -> c_int {
    i32::from(std::ptr::eq(
        object,
        std::ptr::addr_of_mut!(_Py_TrueStruct).cast::<c_void>(),
    ))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IsFalse(object: *mut c_void) -> c_int {
    i32::from(std::ptr::eq(
        object,
        std::ptr::addr_of_mut!(_Py_FalseStruct).cast::<c_void>(),
    ))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_NewRef(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    unsafe { Py_IncRef(object) };
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_XNewRef(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { Py_IncRef(object) };
    object
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_REFCNT(object: *mut c_void) -> isize {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return 0;
    }
    // SAFETY: caller is expected to provide a valid PyObject*.
    unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_refcnt)
            .unwrap_or(0)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_TYPE(object: *mut c_void) -> *mut c_void {
    if object.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    // SAFETY: caller is expected to provide a valid PyObject*.
    unsafe {
        object
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map_or(std::ptr::null_mut(), |head| head.ob_type)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyVectorcall_NARGS(nargsf: usize) -> usize {
    let offset_mask: usize = 1usize << (usize::BITS - 1);
    nargsf & !offset_mask
}
