use std::collections::HashMap;
use std::ffi::{CString, c_char, c_long, c_void};

use crate::runtime::{BuiltinFunction, Object, RuntimeError, Value};

use super::cpython_import_runtime::{
    CpythonInittabInitFunc, cpython_import_add_module_by_name, cpython_import_exec_code_in_module,
    cpython_import_from_inittab, cpython_inittab_registry,
};
use super::cpython_module_name_runtime::{
    cpython_module_name_from_object, cpython_optional_value_from_ptr,
};
use super::{
    InternalCallOutcome, PyErr_WarnEx, PyExc_DeprecationWarning, c_name_to_string,
    cpython_debug_compare_value, cpython_exception_ptr_for_name, cpython_set_error,
    cpython_value_from_ptr, dict_get_value, with_active_cpython_context_mut,
};

fn set_context_error_from_runtime_error(context: &mut super::ModuleCapiContext, err: RuntimeError) {
    let RuntimeError { message, exception } = err;
    if let Some(exception_obj) = exception {
        let exception_name = exception_obj.name.clone();
        let ptype = cpython_exception_ptr_for_name(&exception_name)
            .unwrap_or(unsafe { super::PyExc_RuntimeError });
        let pvalue = context.alloc_cpython_ptr_for_value(Value::Exception(exception_obj));
        if !pvalue.is_null() {
            context.set_error_state(ptype, pvalue, std::ptr::null_mut(), message);
            return;
        }
    }
    context.set_error(message);
}

const PYC_MAGIC_NUMBER_TOKEN: c_long = 0x0A0D0E2B;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_GetMagicNumber() -> c_long {
    PYC_MAGIC_NUMBER_TOKEN
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_GetMagicTag() -> *const c_char {
    c"cpython-314".as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportModule(name: *const c_char) -> *mut c_void {
    match unsafe { c_name_to_string(name) } {
        Ok(module_name) => {
            let trace_pyarrow_import = std::env::var_os("PYRS_TRACE_PYARROW_IMPORT").is_some()
                && module_name.contains("pyarrow");
            if trace_pyarrow_import {
                eprintln!("[pyarrow-import] PyImport_ImportModule name={module_name}");
            }
            let trace_ctypes = std::env::var_os("PYRS_TRACE_CPY_CTYPES_IMPORT").is_some()
                && module_name.contains("ctypes");
            if trace_ctypes {
                eprintln!("[cpy-ctypes-import] PyImport_ImportModule name={module_name}");
            }
            if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
                eprintln!("[cpy-api] PyImport_ImportModule name={module_name}");
            }
            with_active_cpython_context_mut(|context| {
                match cpython_import_from_inittab(context, &module_name) {
                    Ok(Some(value)) => {
                        if trace_ctypes {
                            eprintln!(
                                "[cpy-ctypes-import] inittab hit name={} value={}",
                                module_name,
                                cpython_debug_compare_value(&value)
                            );
                        }
                        return context.alloc_cpython_ptr_for_value(value);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        if trace_ctypes {
                            eprintln!(
                                "[cpy-ctypes-import] inittab err name={} err={}",
                                module_name, err
                            );
                        }
                        context.set_error(err);
                        return std::ptr::null_mut();
                    }
                }
                match context.module_import(&module_name) {
                    Ok(handle) => {
                        context.clear_error();
                        if trace_pyarrow_import {
                            eprintln!(
                                "[pyarrow-import] PyImport_ImportModule success name={} handle={}",
                                module_name, handle
                            );
                        }
                        if trace_ctypes {
                            eprintln!(
                                "[cpy-ctypes-import] module_import ok name={} handle={}",
                                module_name, handle
                            );
                        }
                        context.alloc_cpython_ptr_for_handle(handle)
                    }
                    Err(err) => {
                        if trace_pyarrow_import {
                            eprintln!(
                                "[pyarrow-import] PyImport_ImportModule error name={} err={}",
                                module_name, err
                            );
                        }
                        if trace_ctypes {
                            eprintln!(
                                "[cpy-ctypes-import] module_import err name={} err={}",
                                module_name, err
                            );
                        }
                        set_context_error_from_runtime_error(context, RuntimeError::new(err));
                        std::ptr::null_mut()
                    }
                }
            })
            .unwrap_or_else(|err| {
                cpython_set_error(err);
                std::ptr::null_mut()
            })
        }
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_Import(name: *mut c_void) -> *mut c_void {
    let module_name = match cpython_value_from_ptr(name) {
        Ok(Value::Str(name)) => name,
        Ok(_) => {
            cpython_set_error("PyImport_Import expects module name string");
            return std::ptr::null_mut();
        }
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let c_name = match CString::new(module_name) {
        Ok(name) => name,
        Err(_) => {
            cpython_set_error("PyImport_Import received module name with NUL byte");
            return std::ptr::null_mut();
        }
    };
    unsafe { PyImport_ImportModule(c_name.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_GetModuleDict() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyImport_GetModuleDict missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.refresh_sys_modules_dict();
        let Some(modules_dict) = vm.sys_dict_obj("modules") else {
            context.set_error("unable to get sys.modules");
            return std::ptr::null_mut();
        };
        context.alloc_cpython_ptr_for_value(Value::Dict(modules_dict))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_AddModuleRef(name: *const c_char) -> *mut c_void {
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    if std::env::var_os("PYRS_TRACE_CPY_CTYPES_IMPORT").is_some() && module_name.contains("ctypes")
    {
        eprintln!("[cpy-ctypes-import] PyImport_AddModuleRef name={module_name}");
    }
    with_active_cpython_context_mut(|context| {
        match cpython_import_add_module_by_name(context, &module_name) {
            Ok(module) => context.alloc_cpython_ptr_for_value(Value::Module(module)),
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyImport_AddModuleObject(name: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_name =
            match cpython_module_name_from_object(context, name, "PyImport_AddModuleObject") {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            };
        if std::env::var_os("PYRS_TRACE_CPY_CTYPES_IMPORT").is_some()
            && module_name.contains("ctypes")
        {
            eprintln!("[cpy-ctypes-import] PyImport_AddModuleObject name={module_name}");
        }
        match cpython_import_add_module_by_name(context, &module_name) {
            Ok(module) => context.alloc_cpython_ptr_for_value(Value::Module(module)),
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyImport_AddModule(name: *const c_char) -> *mut c_void {
    unsafe { PyImport_AddModuleRef(name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_AppendInittab(
    name: *const c_char,
    initfunc: Option<CpythonInittabInitFunc>,
) -> i32 {
    let Some(initfunc) = initfunc else {
        cpython_set_error("PyImport_AppendInittab received null initfunc");
        return -1;
    };
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let mut guard = match cpython_inittab_registry().lock() {
        Ok(guard) => guard,
        Err(_) => {
            cpython_set_error("PyImport_AppendInittab registry lock poisoned");
            return -1;
        }
    };
    if guard.contains_key(&module_name) {
        cpython_set_error(format!(
            "PyImport_AppendInittab duplicate module '{}'",
            module_name
        ));
        return -1;
    }
    guard.insert(module_name, initfunc);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportFrozenModule(name: *const c_char) -> i32 {
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    with_active_cpython_context_mut(|context| {
        match cpython_import_from_inittab(context, &module_name) {
            Ok(Some(_)) => return 1,
            Ok(None) => {}
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        }
        if context.vm.is_null() {
            context.set_error("PyImport_ImportFrozenModule missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_import_module(vec![Value::Str(module_name)], HashMap::new()) {
            Ok(_) => 1,
            Err(_) => {
                vm.clear_active_exception();
                0
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportFrozenModuleObject(name: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let module_name = match cpython_module_name_from_object(
            context,
            name,
            "PyImport_ImportFrozenModuleObject",
        ) {
            Ok(name) => name,
            Err(err) => {
                context.set_error(err);
                return -1;
            }
        };
        let c_name = match CString::new(module_name) {
            Ok(name) => name,
            Err(_) => {
                context.set_error(
                    "PyImport_ImportFrozenModuleObject module name contains interior NUL",
                );
                return -1;
            }
        };
        unsafe { PyImport_ImportFrozenModule(c_name.as_ptr()) }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ExecCodeModule(
    name: *const c_char,
    code: *mut c_void,
) -> *mut c_void {
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(code_value) = context.cpython_value_from_ptr_or_proxy(code) else {
            context.set_error("PyImport_ExecCodeModule received unknown code pointer");
            return std::ptr::null_mut();
        };
        let Value::Code(code_obj) = code_value else {
            context.set_error("PyImport_ExecCodeModule expected code object");
            return std::ptr::null_mut();
        };
        match cpython_import_exec_code_in_module(context, &module_name, code_obj, None) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyImport_ExecCodeModuleEx(
    name: *const c_char,
    code: *mut c_void,
    pathname: *const c_char,
) -> *mut c_void {
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    let pathname_value = if pathname.is_null() {
        None
    } else {
        match unsafe { c_name_to_string(pathname) } {
            Ok(path) => Some(Value::Str(path)),
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        }
    };
    with_active_cpython_context_mut(|context| {
        let Some(code_value) = context.cpython_value_from_ptr_or_proxy(code) else {
            context.set_error("PyImport_ExecCodeModuleEx received unknown code pointer");
            return std::ptr::null_mut();
        };
        let Value::Code(code_obj) = code_value else {
            context.set_error("PyImport_ExecCodeModuleEx expected code object");
            return std::ptr::null_mut();
        };
        match cpython_import_exec_code_in_module(context, &module_name, code_obj, pathname_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyImport_ExecCodeModuleObject(
    name: *mut c_void,
    code: *mut c_void,
    pathname: *mut c_void,
    _cpathname: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_name =
            match cpython_module_name_from_object(context, name, "PyImport_ExecCodeModuleObject") {
                Ok(name) => name,
                Err(err) => {
                    context.set_error(err);
                    return std::ptr::null_mut();
                }
            };
        let Some(code_value) = context.cpython_value_from_ptr_or_proxy(code) else {
            context.set_error("PyImport_ExecCodeModuleObject received unknown code pointer");
            return std::ptr::null_mut();
        };
        let Value::Code(code_obj) = code_value else {
            context.set_error("PyImport_ExecCodeModuleObject expected code object");
            return std::ptr::null_mut();
        };
        let pathname_value = if pathname.is_null() {
            None
        } else {
            match context.cpython_value_from_ptr_or_proxy(pathname) {
                Some(value) => Some(value),
                None => {
                    context.set_error(
                        "PyImport_ExecCodeModuleObject received unknown pathname pointer",
                    );
                    return std::ptr::null_mut();
                }
            }
        };
        match cpython_import_exec_code_in_module(context, &module_name, code_obj, pathname_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err);
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
pub unsafe extern "C" fn PyImport_ExecCodeModuleWithPathnames(
    name: *const c_char,
    code: *mut c_void,
    pathname: *const c_char,
    _cpathname: *const c_char,
) -> *mut c_void {
    unsafe { PyImport_ExecCodeModuleEx(name, code, pathname) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_GetModule(name: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_name = match cpython_module_name_from_object(context, name, "PyImport_GetModule")
        {
            Ok(name) => name,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        if context.vm.is_null() {
            context.set_error("PyImport_GetModule missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        vm.refresh_sys_modules_dict();
        let Some(modules_dict) = vm.sys_dict_obj("modules") else {
            context.set_error("unable to get sys.modules");
            return std::ptr::null_mut();
        };
        match dict_get_value(&modules_dict, &Value::Str(module_name)) {
            Some(value) => context.alloc_cpython_ptr_for_value(value),
            None => std::ptr::null_mut(),
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportModuleNoBlock(name: *const c_char) -> *mut c_void {
    const DEPRECATION_MESSAGE: &[u8] = b"PyImport_ImportModuleNoBlock() is deprecated and scheduled for removal in Python 3.15. Use PyImport_ImportModule() instead.\0";
    let warning_status = unsafe {
        PyErr_WarnEx(
            std::ptr::addr_of_mut!(PyExc_DeprecationWarning).cast(),
            DEPRECATION_MESSAGE.as_ptr().cast(),
            1,
        )
    };
    if warning_status != 0 {
        return std::ptr::null_mut();
    }
    unsafe { PyImport_ImportModule(name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_GetImporter(path: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyImport_GetImporter missing VM context");
            return std::ptr::null_mut();
        }
        let Some(path_value) = context.cpython_value_from_ptr_or_proxy(path) else {
            context.set_error("PyImport_GetImporter received unknown path pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let pkgutil = match vm
            .builtin_import_module(vec![Value::Str("pkgutil".to_string())], HashMap::new())
        {
            Ok(value) => value,
            Err(err) => {
                vm.clear_active_exception();
                if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
                    eprintln!(
                        "[cpy-api] PyImport_GetImporter fallback (pkgutil import failed): {}",
                        err.message
                    );
                }
                return context.alloc_cpython_ptr_for_value(Value::None);
            }
        };
        let get_importer = match vm.call_internal(
            Value::Builtin(BuiltinFunction::GetAttr),
            vec![pkgutil, Value::Str("get_importer".to_string())],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => value,
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                vm.clear_active_exception();
                return context.alloc_cpython_ptr_for_value(Value::None);
            }
            Err(err) => {
                vm.clear_active_exception();
                if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
                    eprintln!(
                        "[cpy-api] PyImport_GetImporter fallback (getattr failed): {}",
                        err.message
                    );
                }
                return context.alloc_cpython_ptr_for_value(Value::None);
            }
        };
        match vm.call_internal(get_importer, vec![path_value], HashMap::new()) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                vm.clear_active_exception();
                context.alloc_cpython_ptr_for_value(Value::None)
            }
            Err(err) => {
                vm.clear_active_exception();
                if std::env::var_os("PYRS_TRACE_CPY_API").is_some() {
                    eprintln!(
                        "[cpy-api] PyImport_GetImporter fallback (call failed): {}",
                        err.message
                    );
                }
                context.alloc_cpython_ptr_for_value(Value::None)
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ImportModuleLevelObject(
    name: *mut c_void,
    globals: *mut c_void,
    locals: *mut c_void,
    fromlist: *mut c_void,
    level: i32,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyImport_ImportModuleLevelObject missing VM context");
            return std::ptr::null_mut();
        }
        let module_name = match cpython_optional_value_from_ptr(context, name, "module name") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let globals_value = match cpython_optional_value_from_ptr(context, globals, "globals") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let locals_value = match cpython_optional_value_from_ptr(context, locals, "locals") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        let fromlist_value = match cpython_optional_value_from_ptr(context, fromlist, "fromlist") {
            Ok(value) => value,
            Err(err) => {
                context.set_error(err);
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for the active context lifetime.
        let vm = unsafe { &mut *context.vm };
        let args = vec![
            module_name,
            globals_value,
            locals_value,
            fromlist_value,
            Value::Int(level as i64),
        ];
        let trace_pyarrow_import = std::env::var_os("PYRS_TRACE_PYARROW_IMPORT").is_some()
            && matches!(args.first(), Some(Value::Str(name)) if name.contains("pyarrow"));
        if trace_pyarrow_import {
            let fromlist_desc = match args.get(3) {
                Some(Value::Tuple(tuple_obj)) => match &*tuple_obj.kind() {
                    Object::Tuple(items) => items
                        .iter()
                        .map(|item| match item {
                            Value::Str(text) => text.clone(),
                            other => cpython_debug_compare_value(other),
                        })
                        .collect::<Vec<_>>()
                        .join(","),
                    _ => "<tuple-storage-invalid>".to_string(),
                },
                Some(value) => cpython_debug_compare_value(value),
                None => "<none>".to_string(),
            };
            eprintln!(
                "[pyarrow-import] PyImport_ImportModuleLevelObject name={} level={} fromlist=[{}]",
                match args.first() {
                    Some(Value::Str(name)) => name.as_str(),
                    _ => "<non-str>",
                },
                level,
                fromlist_desc
            );
        }
        match vm.call_internal(
            Value::Builtin(BuiltinFunction::Import),
            args,
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(value)) => {
                context.clear_error();
                if trace_pyarrow_import {
                    eprintln!("[pyarrow-import] PyImport_ImportModuleLevelObject success");
                }
                context.alloc_cpython_ptr_for_value(value)
            }
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                let runtime_err =
                    vm.runtime_error_from_active_exception("import module level call failed");
                if trace_pyarrow_import {
                    eprintln!(
                        "[pyarrow-import] PyImport_ImportModuleLevelObject handled-exception"
                    );
                }
                set_context_error_from_runtime_error(context, runtime_err);
                std::ptr::null_mut()
            }
            Err(err) => {
                let detail_err = vm.runtime_error_from_active_exception(&err.message);
                if trace_pyarrow_import {
                    eprintln!(
                        "[pyarrow-import] PyImport_ImportModuleLevelObject error err={} detail={}",
                        err.message, detail_err.message
                    );
                }
                let detail_message = detail_err.message.clone();
                if detail_message.is_empty() {
                    set_context_error_from_runtime_error(context, err);
                } else {
                    set_context_error_from_runtime_error(context, detail_err);
                }
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
pub unsafe extern "C" fn PyImport_ImportModuleLevel(
    name: *const c_char,
    globals: *mut c_void,
    locals: *mut c_void,
    fromlist: *mut c_void,
    level: i32,
) -> *mut c_void {
    let module_name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(err);
            return std::ptr::null_mut();
        }
    };
    with_active_cpython_context_mut(|context| {
        let name_obj = context.alloc_cpython_ptr_for_value(Value::Str(module_name));
        unsafe { PyImport_ImportModuleLevelObject(name_obj, globals, locals, fromlist, level) }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyImport_ReloadModule(module: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if module.is_null() {
            context.set_error("PyImport_ReloadModule expected module object");
            return std::ptr::null_mut();
        }
        if context.vm.is_null() {
            context.set_error("PyImport_ReloadModule missing VM context");
            return std::ptr::null_mut();
        }
        let module_value = match context.cpython_value_from_ptr_or_proxy(module) {
            Some(value) => value,
            None => {
                context.set_error("PyImport_ReloadModule received unknown module pointer");
                return std::ptr::null_mut();
            }
        };
        let module_name = match &module_value {
            Value::Module(module_obj) => match &*module_obj.kind() {
                Object::Module(module_data) => module_data.name.clone(),
                _ => String::new(),
            },
            _ => {
                context.set_error("PyImport_ReloadModule expected module object");
                return std::ptr::null_mut();
            }
        };
        if module_name.is_empty() {
            context.set_error("PyImport_ReloadModule could not resolve module name");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.builtin_import_module(vec![Value::Str(module_name)], HashMap::new()) {
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
