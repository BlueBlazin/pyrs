use std::backtrace::Backtrace;
use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::Ordering;

use crate::runtime::{Object, Value};

use super::{
    BuiltinFunction, CpythonMappingMethods, CpythonObjectHead, CpythonTypeObject,
    ModuleCapiContext, Py_DecRef, Py_IncRef, Py_XDecRef, PyErr_BadInternalCall, PyErr_Occurred,
    PyErr_SetObject, PyExc_KeyError, PyUnicode_FromString, TRACE_NUMPY_TYPEDICT_PTR,
    c_name_to_string, cpython_call_builtin, cpython_debug_compare_value,
    cpython_is_reduce_probe_name, cpython_new_ptr_for_value, cpython_safe_object_type_name,
    cpython_set_error, cpython_trace_numpy_reduce_enabled, cpython_value_debug_tag,
    cpython_value_from_ptr, with_active_cpython_context_mut,
};

fn cpython_dict_set_key_error_for_value(context: &mut ModuleCapiContext, key: Value) {
    let key_ptr = context.alloc_cpython_ptr_for_value(key);
    if key_ptr.is_null() {
        context.set_error("dict key lookup failed");
        return;
    }
    // SAFETY: `key_ptr` is a temporary strong reference owned by this context.
    unsafe {
        PyErr_SetObject(PyExc_KeyError, key_ptr);
        Py_DecRef(key_ptr);
    }
}

unsafe extern "C" fn cpython_dict_mp_length_slot(dict: *mut c_void) -> isize {
    // SAFETY: mapping slot ABI forwards validated args.
    unsafe { PyDict_Size(dict) }
}

unsafe extern "C" fn cpython_dict_mp_subscript_slot(
    dict: *mut c_void,
    key: *mut c_void,
) -> *mut c_void {
    let trace_slot = super::super::env_var_present_cached("PYRS_TRACE_DICT_SLOT");
    with_active_cpython_context_mut(|context| {
        let Some(dict_value) = context.cpython_value_from_ptr_or_proxy(dict) else {
            if trace_slot {
                eprintln!(
                    "[cpy-dict-slot] mp_subscript unknown dict ptr={:p} key={:p}",
                    dict, key
                );
            }
            context.set_error("dict mp_subscript received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = dict_value else {
            if trace_slot {
                eprintln!(
                    "[cpy-dict-slot] mp_subscript non-dict target ptr={:p} key={:p} tag={}",
                    dict,
                    key,
                    cpython_value_debug_tag(&dict_value)
                );
            }
            context.set_error("dict mp_subscript expected dict object");
            return std::ptr::null_mut();
        };
        if context.vm.is_null() {
            context.set_error("dict mp_subscript missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            if trace_slot {
                let key_type =
                    cpython_safe_object_type_name(key).unwrap_or_else(|| "<unknown>".to_string());
                eprintln!(
                    "[cpy-dict-slot] mp_subscript unknown key ptr={:p} key_type={} dict={:p}",
                    key, key_type, dict
                );
            }
            context.set_error("dict mp_subscript received unknown key pointer");
            return std::ptr::null_mut();
        };
        if trace_slot {
            eprintln!(
                "[cpy-dict-slot] mp_subscript dict={:p} key={}",
                dict,
                cpython_debug_compare_value(&key_value)
            );
        }
        match vm.dict_get_value_runtime(&dict_obj, &key_value) {
            Ok(Some(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(None) => {
                cpython_dict_set_key_error_for_value(context, key_value);
                std::ptr::null_mut()
            }
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

unsafe extern "C" fn cpython_dict_mp_ass_subscript_slot(
    dict: *mut c_void,
    key: *mut c_void,
    value: *mut c_void,
) -> c_int {
    let trace_slot = super::super::env_var_present_cached("PYRS_TRACE_DICT_SLOT");
    with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
        let Some(dict_value) = context.cpython_value_from_ptr_or_proxy(dict) else {
            if trace_slot {
                eprintln!(
                    "[cpy-dict-slot] mp_ass_subscript unknown dict ptr={:p} key={:p} value={:p}",
                    dict, key, value
                );
            }
            context.set_error("dict mp_ass_subscript received unknown dict pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = dict_value else {
            if trace_slot {
                eprintln!(
                    "[cpy-dict-slot] mp_ass_subscript non-dict target ptr={:p} key={:p} value={:p} tag={}",
                    dict,
                    key,
                    value,
                    cpython_value_debug_tag(&dict_value)
                );
            }
            context.set_error("dict mp_ass_subscript expected dict object");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("dict mp_ass_subscript missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            if trace_slot {
                let key_type =
                    cpython_safe_object_type_name(key).unwrap_or_else(|| "<unknown>".to_string());
                eprintln!(
                    "[cpy-dict-slot] mp_ass_subscript unknown key ptr={:p} key_type={} dict={:p}",
                    key, key_type, dict
                );
            }
            context.set_error("dict mp_ass_subscript received unknown key pointer");
            return -1;
        };
        if value.is_null() {
            match vm.dict_remove_value_runtime(&dict_obj, &key_value) {
                Ok(Some(_)) => {
                    if let Some(module_obj) = module_target
                        && let Value::Str(name) = &key_value
                        && let Object::Module(module_data) = &mut *module_obj.kind_mut()
                    {
                        module_data.globals.remove(name);
                    }
                    0
                }
                Ok(None) => {
                    cpython_dict_set_key_error_for_value(context, key_value);
                    -1
                }
                Err(err) => {
                    context.set_error(err.message);
                    -1
                }
            }
        } else {
            let Some(value_obj) = context.cpython_value_from_ptr_or_proxy(value) else {
                context.set_error("dict mp_ass_subscript received unknown value pointer");
                return -1;
            };
            match vm.dict_set_value_checked_runtime(&dict_obj, key_value.clone(), value_obj.clone()) {
                Ok(()) => {
                    if let Some(module_obj) = module_target
                        && let Value::Str(name) = key_value
                        && let Object::Module(module_data) = &mut *module_obj.kind_mut()
                    {
                        module_data.globals.insert(name, value_obj);
                    }
                    0
                }
                Err(err) => {
                    context.set_error(err.message);
                    -1
                }
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

pub(super) static mut PY_DICT_MAPPING_METHODS: CpythonMappingMethods = CpythonMappingMethods {
    mp_length: cpython_dict_mp_length_slot as *mut c_void,
    mp_subscript: cpython_dict_mp_subscript_slot as *mut c_void,
    mp_ass_subscript: cpython_dict_mp_ass_subscript_slot as *mut c_void,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_New() -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_New missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let dict = vm.heap.alloc_dict(Vec::new());
        context.alloc_cpython_ptr_for_value(dict)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyDict_NewPresized(_minused: isize) -> *mut c_void {
    unsafe { PyDict_New() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Size(dict: *mut c_void) -> isize {
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr_or_proxy(dict) else {
            context.set_error("PyDict_Size received unknown dict pointer");
            return -1;
        };
        if let Value::Dict(dict_obj) = target.clone() {
            return match &*dict_obj.kind() {
                Object::Dict(values) => values.len() as isize,
                _ => {
                    context.set_error("PyDict_Size encountered invalid dict storage");
                    -1
                }
            };
        }
        match cpython_call_builtin(BuiltinFunction::Len, vec![target]) {
            Ok(Value::Int(length)) => match isize::try_from(length) {
                Ok(length) => length,
                Err(_) => {
                    context.set_error("PyDict_Size length does not fit isize");
                    -1
                }
            },
            Ok(Value::BigInt(_)) => {
                context.set_error("PyDict_Size length does not fit isize");
                -1
            }
            Ok(other) => {
                context.set_error(format!(
                    "PyDict_Size expected dict-like length result, got {}",
                    cpython_value_debug_tag(&other)
                ));
                -1
            }
            Err(err) => {
                context.set_error(err);
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetItem(
    dict: *mut c_void,
    key: *mut c_void,
    value: *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
        if value.is_null() && super::super::env_var_present_cached("PYRS_TRACE_CPY_ERRORS") {
            eprintln!(
                "[cpy-err] PyDict_SetItem null value pointer dict={:p} key={:p}",
                dict, key
            );
            eprintln!("{:?}", std::backtrace::Backtrace::capture());
        }
        let target = if !context.vm.is_null()
            && unsafe { (&*context.vm).capi_ptr_is_owned_compat(dict as usize) }
        {
            context.cpython_value_from_ptr_or_proxy(dict)
        } else {
            context.cpython_value_from_ptr(dict)
        };
        if let Some(target) = target
            && let Value::Dict(dict_obj) = target
        {
            if context.vm.is_null() {
                context.set_error("PyDict_SetItem missing VM context");
                return -1;
            }
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            let module_trace = module_target
                .as_ref()
                .and_then(|module_obj| match &*module_obj.kind() {
                    Object::Module(module_data) => module_data
                        .globals
                        .get("__name__")
                        .and_then(|value| match value {
                            Value::Str(name) => Some(format!("{}#{}", name, module_obj.id())),
                            _ => Some(format!("<unnamed>#{}", module_obj.id())),
                        }),
                    _ => None,
                })
                .unwrap_or_else(|| "-".to_string());
            let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
                context.set_error("PyDict_SetItem received unknown key pointer");
                return -1;
            };
            let Some(item_value) = context.cpython_value_from_ptr_or_proxy(value) else {
                context.set_error("PyDict_SetItem received unknown value pointer");
                return -1;
            };
            if cpython_trace_numpy_reduce_enabled()
                && let Value::Str(name) = &key_value
                && cpython_is_reduce_probe_name(name)
            {
                eprintln!(
                    "[numpy-reduce] PyDict_SetItem dict={:p} key={} value_ptr={:p} value_tag={}",
                    dict,
                    name,
                    value,
                    cpython_value_debug_tag(&item_value)
                );
            }
            if super::super::env_var_present_cached("PYRS_TRACE_CPY_DICT") {
                eprintln!(
                    "[cpy-dict-set] dict={:p} module={} key_ptr={:p} key={} value_ptr={:p} value_tag={}",
                    dict,
                    module_trace,
                    key,
                    cpython_debug_compare_value(&key_value),
                    value,
                    cpython_value_debug_tag(&item_value)
                );
            }
            return match vm.dict_set_value_checked_runtime(&dict_obj, key_value.clone(), item_value.clone()) {
                Ok(()) => {
                    if let Some(module_obj) = module_target
                        && let Value::Str(name) = key_value
                        && let Object::Module(module_data) = &mut *module_obj.kind_mut()
                    {
                        module_data.globals.insert(name, item_value);
                    }
                    0
                }
                Err(err) => {
                    if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
                        eprintln!(
                            "[cpy-dict-err] PyDict_SetItem internal dict={:p} key_ptr={:p} key={} err={}",
                            dict,
                            key,
                            cpython_debug_compare_value(&key_value),
                            err.message
                        );
                    }
                    context.set_error(err.message);
                    -1
                }
            };
        }

        // External-dict fallback: native extensions can pass foreign dict pointers that are
        // not owned by this runtime's C-API object table.
        const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
        if dict.is_null()
            || key.is_null()
            || value.is_null()
            || (dict as usize) < MIN_VALID_PTR
            || (dict as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
        {
            context.set_error(format!(
                "PyDict_SetItem received unknown dict pointer {:p}",
                dict
            ));
            return -1;
        }
        // SAFETY: best-effort inspection of foreign object header.
        let type_ptr = unsafe {
            dict.cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null()
            || (type_ptr as usize) < MIN_VALID_PTR
            || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
        {
            context.set_error(format!(
                "PyDict_SetItem received unknown dict pointer {:p}",
                dict
            ));
            return -1;
        }
        // SAFETY: type pointer is validated for mapping slot reads.
        let mapping = unsafe { (*type_ptr).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if mapping.is_null()
            || (mapping as usize) < MIN_VALID_PTR
            || (mapping as usize) % std::mem::align_of::<CpythonMappingMethods>() != 0
        {
            context.set_error("PyDict_SetItem expected dict object");
            return -1;
        }
        // SAFETY: mapping slot table follows CPython ABI.
        let mp_ass_subscript = unsafe { (*mapping).mp_ass_subscript };
        if mp_ass_subscript.is_null() {
            context.set_error("PyDict_SetItem expected dict object");
            return -1;
        }
        let subscript: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int =
            // SAFETY: `mp_ass_subscript` follows CPython mapping ABI.
            unsafe { std::mem::transmute(mp_ass_subscript) };
        // SAFETY: foreign dict and key/value pointers are handed to native mapping slot.
        let status = unsafe { subscript(dict, key, value) };
        if status < 0 && unsafe { PyErr_Occurred() }.is_null() {
            if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
                eprintln!(
                    "[cpy-dict-err] PyDict_SetItem external dict={:p} key={:p} value={:p} err=no-exception",
                    dict, key, value
                );
            }
            context.set_error("PyDict_SetItem mapping slot failed without setting an exception");
            return -1;
        }
        if status < 0 && super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
            eprintln!(
                "[cpy-dict-err] PyDict_SetItem external dict={:p} key={:p} value={:p} err=with-exception",
                dict, key, value
            );
        }
        status
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetDefault(
    dict: *mut c_void,
    key: *mut c_void,
    default_value: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_SetDefault received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_SetDefault expected dict object");
            return std::ptr::null_mut();
        };
        if context.vm.is_null() {
            context.set_error("PyDict_SetDefault missing VM context");
            return std::ptr::null_mut();
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_SetDefault received unknown key pointer");
            return std::ptr::null_mut();
        };
        if let Ok(Some(existing)) = vm.dict_get_value_runtime(&dict_obj, &key_value) {
            return context.alloc_cpython_ptr_for_value(existing);
        }
        let Some(default_item) = context.cpython_value_from_ptr_or_proxy(default_value) else {
            if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
                let ty = cpython_safe_object_type_name(default_value)
                    .unwrap_or_else(|| "<unknown-type>".to_string());
                let probable = ModuleCapiContext::is_probable_external_cpython_object_ptr(
                    default_value,
                );
                let owned = context.owns_cpython_allocation_ptr(default_value);
                eprintln!(
                    "[pydict-setdefault-unknown-default] dict={:p} key={} default={:p} default_type={} owned={} probable={}",
                    dict,
                    cpython_value_debug_tag(&key_value),
                    default_value,
                    ty,
                    owned,
                    probable
                );
            }
            context.set_error("PyDict_SetDefault received unknown default pointer");
            return std::ptr::null_mut();
        };
        if let Err(err) = vm.dict_set_value_checked_runtime(&dict_obj, key_value.clone(), default_item.clone())
        {
            context.set_error(err.message);
            return std::ptr::null_mut();
        }
        if let Some(module_obj) = module_target
            && let Value::Str(name) = key_value
            && let Object::Module(module_data) = &mut *module_obj.kind_mut()
        {
            module_data.globals.insert(name, default_item.clone());
        }
        context.alloc_cpython_ptr_for_value(default_item)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetDefaultRef(
    dict: *mut c_void,
    key: *mut c_void,
    default_value: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        if !result.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe {
                *result = std::ptr::null_mut();
            }
        }
        let module_target = context.module_dict_module_for_ptr(dict);
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_SetDefaultRef received unknown dict pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_SetDefaultRef expected dict object");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("PyDict_SetDefaultRef missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_SetDefaultRef received unknown key pointer");
            return -1;
        };
        if let Ok(Some(existing)) = vm.dict_get_value_runtime(&dict_obj, &key_value) {
            if !result.is_null() {
                // SAFETY: caller provided writable output pointer.
                unsafe {
                    *result = context.alloc_cpython_ptr_for_value(existing);
                }
            }
            return 1;
        }
        let Some(default_item) = context.cpython_value_from_ptr_or_proxy(default_value) else {
            if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
                let ty = cpython_safe_object_type_name(default_value)
                    .unwrap_or_else(|| "<unknown-type>".to_string());
                let probable = ModuleCapiContext::is_probable_external_cpython_object_ptr(
                    default_value,
                );
                let owned = context.owns_cpython_allocation_ptr(default_value);
                eprintln!(
                    "[pydict-setdefaultref-unknown-default] dict={:p} key={} default={:p} default_type={} owned={} probable={} result_ptr={:p}",
                    dict,
                    cpython_value_debug_tag(&key_value),
                    default_value,
                    ty,
                    owned,
                    probable,
                    result
                );
            }
            context.set_error("PyDict_SetDefaultRef received unknown default pointer");
            return -1;
        };
        if let Err(err) = vm.dict_set_value_checked_runtime(&dict_obj, key_value.clone(), default_item.clone())
        {
            context.set_error(err.message);
            return -1;
        }
        if let Some(module_obj) = module_target
            && let Value::Str(name) = key_value
            && let Object::Module(module_data) = &mut *module_obj.kind_mut()
        {
            module_data.globals.insert(name, default_item.clone());
        }
        if !result.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe {
                *result = context.alloc_cpython_ptr_for_value(default_item);
            }
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        if !result.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe {
                *result = std::ptr::null_mut();
            }
        }
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItem(dict: *mut c_void, key: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        // CPython preserves/suppresses errors in PyDict_GetItem; snapshot state up-front.
        let saved_current_error = context.current_error;
        let saved_last_error = context.last_error.clone();
        let saved_first_error = context.first_error.clone();
        let module_target = context.module_dict_module_for_ptr(dict);

        if let Some(target) = context.cpython_value_from_ptr(dict) {
            if let Value::Dict(dict_obj) = target {
                if context.vm.is_null() {
                    context.current_error = saved_current_error;
                    context.last_error = saved_last_error.clone();
                    context.first_error = saved_first_error.clone();
                    return std::ptr::null_mut();
                }
                // SAFETY: VM pointer is valid for context lifetime.
                let vm = unsafe { &mut *context.vm };
                let module_trace = module_target
                    .as_ref()
                    .and_then(|module_obj| match &*module_obj.kind() {
                        Object::Module(module_data) => module_data
                            .globals
                            .get("__name__")
                            .and_then(|value| match value {
                                Value::Str(name) => Some(format!("{}#{}", name, module_obj.id())),
                                _ => Some(format!("<unnamed>#{}", module_obj.id())),
                            }),
                        _ => None,
                    })
                    .unwrap_or_else(|| "-".to_string());
                let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
                    context.current_error = saved_current_error;
                    context.last_error = saved_last_error.clone();
                    context.first_error = saved_first_error.clone();
                    return std::ptr::null_mut();
                };
                let trace_typedict_lookup = super::super::env_var_present_cached(
                    "PYRS_TRACE_NUMPY_TYPEDICT",
                ) && TRACE_NUMPY_TYPEDICT_PTR.load(Ordering::Relaxed)
                    == dict as usize
                    && matches!(
                        &key_value,
                        Value::Str(name) if name == "int8" || name == "bool" || name == "float64"
                    );
                if super::super::env_var_present_cached("PYRS_TRACE_NUMPY_BOOL_LOOKUP")
                    && matches!(&key_value, Value::Str(name) if name == "bool")
                {
                    eprintln!(
                        "[numpy-bool-lookup] runtime-dict dict={:p} key_ptr={:p}",
                        dict, key
                    );
                }
                if trace_typedict_lookup {
                    eprintln!(
                        "[numpy-typedict] lookup dict={:p} key_ptr={:p} key={}",
                        dict,
                        key,
                        cpython_debug_compare_value(&key_value)
                    );
                }
                if super::super::env_var_present_cached("PYRS_TRACE_CPY_DICT") {
                    eprintln!(
                        "[cpy-dict-get] dict={:p} module={} key_ptr={:p} key={}",
                        dict,
                        module_trace,
                        key,
                        cpython_debug_compare_value(&key_value)
                    );
                }
                let value = match vm.dict_get_value_runtime(&dict_obj, &key_value) {
                    Ok(value) => value,
                    Err(_) => {
                        context.current_error = saved_current_error;
                        context.last_error = saved_last_error.clone();
                        context.first_error = saved_first_error.clone();
                        return std::ptr::null_mut();
                    }
                };
                let Some(value) = value else {
                    if trace_typedict_lookup {
                        eprintln!(
                            "[numpy-typedict] miss key={}",
                            cpython_debug_compare_value(&key_value)
                        );
                    }
                    if super::super::env_var_present_cached("PYRS_TRACE_CPY_DICT") {
                        eprintln!("[cpy-dict-get] dict={:p} miss", dict);
                    }
                    return std::ptr::null_mut();
                };
                if trace_typedict_lookup {
                    eprintln!(
                        "[numpy-typedict] hit key={} value={}",
                        cpython_debug_compare_value(&key_value),
                        cpython_value_debug_tag(&value)
                    );
                }
                if super::super::env_var_present_cached("PYRS_TRACE_CPY_DICT") {
                    eprintln!(
                        "[cpy-dict-get] dict={:p} hit value_tag={}",
                        dict,
                        cpython_value_debug_tag(&value)
                    );
                }
                context.current_error = saved_current_error;
                context.last_error = saved_last_error.clone();
                context.first_error = saved_first_error.clone();
                return context.alloc_cpython_ptr_for_value(value);
            }
            if super::super::env_var_present_cached("PYRS_TRACE_CPY_DICT") {
                eprintln!(
                    "[cpy-dict-get] non-dict pointer={:p} tag={}",
                    dict,
                    cpython_value_debug_tag(&target)
                );
            }
            context.current_error = saved_current_error;
            context.last_error = saved_last_error.clone();
            context.first_error = saved_first_error.clone();
            return std::ptr::null_mut();
        }

        // External-dict fallback: NumPy and other native modules may call PyDict_GetItem
        // against foreign dict pointers not owned by this runtime.
        const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
        if dict.is_null()
            || key.is_null()
            || (dict as usize) < MIN_VALID_PTR
            || (dict as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
        {
            if super::super::env_var_present_cached("PYRS_TRACE_CPY_DICT") {
                eprintln!(
                    "[cpy-dict-get] unknown-precheck dict={:p} key={:p}\n{:?}",
                    dict,
                    key,
                    Backtrace::force_capture()
                );
            }
            context.current_error = saved_current_error;
            context.last_error = saved_last_error.clone();
            context.first_error = saved_first_error.clone();
            return std::ptr::null_mut();
        }
        // SAFETY: best-effort inspection of foreign object header.
        let type_ptr = unsafe {
            dict.cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null()
            || (type_ptr as usize) < MIN_VALID_PTR
            || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
        {
            if super::super::env_var_present_cached("PYRS_TRACE_CPY_DICT") {
                eprintln!(
                    "[cpy-dict-get] unknown-type dict={:p} key={:p} type_ptr={:p}\n{:?}",
                    dict,
                    key,
                    type_ptr,
                    Backtrace::force_capture()
                );
            }
            context.current_error = saved_current_error;
            context.last_error = saved_last_error.clone();
            context.first_error = saved_first_error.clone();
            return std::ptr::null_mut();
        }
        // SAFETY: `type_ptr` is a validated pointer to type metadata.
        let mapping = unsafe { (*type_ptr).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if mapping.is_null()
            || (mapping as usize) < MIN_VALID_PTR
            || (mapping as usize) % std::mem::align_of::<CpythonMappingMethods>() != 0
        {
            context.current_error = saved_current_error;
            context.last_error = saved_last_error.clone();
            context.first_error = saved_first_error.clone();
            return std::ptr::null_mut();
        }
        // SAFETY: mapping slot table follows CPython ABI.
        let mp_subscript = unsafe { (*mapping).mp_subscript };
        if mp_subscript.is_null() {
            context.current_error = saved_current_error;
            context.last_error = saved_last_error.clone();
            context.first_error = saved_first_error.clone();
            return std::ptr::null_mut();
        }
        let subscript: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            // SAFETY: `mp_subscript` follows CPython mapping ABI.
            unsafe { std::mem::transmute(mp_subscript) };
        // SAFETY: foreign dict and key pointers are handed to native mapping slot.
        let value_ptr = unsafe { subscript(dict, key) };
        if super::super::env_var_present_cached("PYRS_TRACE_NUMPY_BOOL_LOOKUP") {
            let key_is_bool = context
                .cpython_value_from_ptr_or_proxy(key)
                .is_some_and(|value| matches!(value, Value::Str(ref text) if text == "bool"));
            if key_is_bool {
                eprintln!(
                    "[numpy-bool-lookup] external-dict dict={:p} key_ptr={:p} value_ptr={:p}",
                    dict, key, value_ptr
                );
            }
        }
        // PyDict_GetItem suppresses lookup exceptions and preserves prior error state.
        context.current_error = saved_current_error;
        context.last_error = saved_last_error;
        context.first_error = saved_first_error;
        value_ptr
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemWithError(
    dict: *mut c_void,
    key: *mut c_void,
) -> *mut c_void {
    unsafe { PyDict_GetItem(dict, key) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyDict_GetItem_KnownHash(
    dict: *mut c_void,
    key: *mut c_void,
    _hash: isize,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if let Some(target) = context.cpython_value_from_ptr(dict) {
            let Value::Dict(dict_obj) = target else {
                unsafe { PyErr_BadInternalCall() };
                return std::ptr::null_mut();
            };
            if context.vm.is_null() {
                context.set_error("_PyDict_GetItem_KnownHash missing VM context");
                return std::ptr::null_mut();
            }
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
                return std::ptr::null_mut();
            };
            match vm.dict_get_value_runtime(&dict_obj, &key_value) {
                Ok(None) => std::ptr::null_mut(),
                Ok(Some(value)) => context.alloc_cpython_ptr_for_value(value),
                Err(err) => {
                    context.set_error(err.message);
                    std::ptr::null_mut()
                }
            }
        } else {
            unsafe { PyDict_GetItem(dict, key) }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Contains(dict: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Contains received unknown dict pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Contains expected dict object");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("PyDict_Contains missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_Contains received unknown key pointer");
            return -1;
        };
        match vm.dict_contains_key_checked_runtime(&dict_obj, &key_value) {
            Ok(true) => 1,
            Ok(false) => 0,
            Err(err) => {
                context.set_error(err.message);
                -1
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetItemString(
    dict: *mut c_void,
    key: *const c_char,
    value: *mut c_void,
) -> i32 {
    if value.is_null() && super::super::env_var_present_cached("PYRS_TRACE_CPY_ERRORS") {
        let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
        eprintln!("[cpy-err] PyDict_SetItemString null value key={key_name}");
    }
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
    let result = unsafe { PyDict_SetItem(dict, key_obj, value) };
    if super::super::env_var_present_cached("PYRS_TRACE_PYBIND11_ATTRS")
        && key_name.contains("__pybind11")
    {
        eprintln!(
            "[pybind11-dict] set key={} dict={:p} value={:p} result={}",
            key_name, dict, value, result
        );
    }
    if super::super::env_var_present_cached("PYRS_TRACE_NUMPY_INIT")
        && matches!(
            key_name.as_str(),
            "_ARRAY_API" | "_UFUNC_API" | "False_" | "True_"
        )
    {
        eprintln!(
            "[numpy-init] PyDict_SetItemString key={} value_ptr={:p} result={}",
            key_name, value, result
        );
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemString(
    dict: *mut c_void,
    key: *const c_char,
) -> *mut c_void {
    let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
    let trace_key = matches!(
        key_name.as_str(),
        "matmul" | "logical_and" | "logical_or" | "logical_xor"
    );
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        if super::super::env_var_present_cached("PYRS_TRACE_NUMPY_INIT") && trace_key {
            eprintln!(
                "[numpy-init] PyDict_GetItemString key={} key_obj=<null> dict={:p}",
                key_name, dict
            );
        }
        return std::ptr::null_mut();
    }
    let result = unsafe { PyDict_GetItem(dict, key_obj) };
    // SAFETY: `key_obj` is a temporary strong reference created above.
    unsafe { Py_DecRef(key_obj) };
    if super::super::env_var_present_cached("PYRS_TRACE_PYBIND11_ATTRS")
        && key_name.contains("__pybind11")
    {
        eprintln!(
            "[pybind11-dict] get key={} dict={:p} result={:p}",
            key_name, dict, result
        );
    }
    if super::super::env_var_present_cached("PYRS_TRACE_NUMPY_INIT") && trace_key {
        eprintln!(
            "[numpy-init] PyDict_GetItemString key={} dict={:p} result={:p}",
            key_name, dict, result
        );
    }
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemRef(
    dict: *mut c_void,
    key: *mut c_void,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() {
        cpython_set_error("PyDict_GetItemRef requires non-null out pointer");
        return -1;
    }
    let value = unsafe { PyDict_GetItemWithError(dict, key) };
    if value.is_null() && !unsafe { PyErr_Occurred() }.is_null() {
        // SAFETY: caller provided writable pointer.
        unsafe { *out = std::ptr::null_mut() };
        return -1;
    }
    if !value.is_null() {
        // SAFETY: successful lookup returns a borrowed reference; Ref variant returns strong ref.
        unsafe { Py_IncRef(value) };
    }
    // SAFETY: caller provided writable pointer.
    unsafe { *out = value };
    if value.is_null() { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemStringRef(
    dict: *mut c_void,
    key: *const c_char,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() {
        cpython_set_error("PyDict_GetItemStringRef requires non-null out pointer");
        return -1;
    }
    let key_name = unsafe { c_name_to_string(key) }.unwrap_or_else(|_| "<invalid>".to_string());
    let trace_key = matches!(
        key_name.as_str(),
        "matmul" | "logical_and" | "logical_or" | "logical_xor"
    );
    let value = unsafe { PyDict_GetItemString(dict, key) };
    if value.is_null() && !unsafe { PyErr_Occurred() }.is_null() {
        // SAFETY: caller provided writable pointer.
        unsafe { *out = std::ptr::null_mut() };
        return -1;
    }
    if !value.is_null() {
        // SAFETY: successful lookup returns a borrowed reference; Ref variant returns strong ref.
        unsafe { Py_IncRef(value) };
    }
    // SAFETY: caller provided writable pointer.
    unsafe { *out = value };
    let status = if value.is_null() { 0 } else { 1 };
    if super::super::env_var_present_cached("PYRS_TRACE_NUMPY_INIT") && trace_key {
        eprintln!(
            "[numpy-init] PyDict_GetItemStringRef key={} dict={:p} value={:p} status={}",
            key_name, dict, value, status
        );
    }
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Pop(
    dict: *mut c_void,
    key: *mut c_void,
    result: *mut *mut c_void,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        if !result.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe { *result = std::ptr::null_mut() };
        }
        let module_target = context.module_dict_module_for_ptr(dict);
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Pop received unknown dict pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Pop expected dict object");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("PyDict_Pop missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("PyDict_Pop received unknown key pointer");
            return -1;
        };
        let popped = match vm.dict_remove_value_runtime(&dict_obj, &key_value) {
            Ok(popped) => popped,
            Err(err) => {
                context.set_error(err.message);
                return -1;
            }
        };
        let Some(popped) = popped else {
            return 0;
        };
        if let Some(module_obj) = module_target
            && let Value::Str(name) = &key_value
            && let Object::Module(module_data) = &mut *module_obj.kind_mut()
        {
            module_data.globals.remove(name);
        }
        let popped_ptr = context.alloc_cpython_ptr_for_value(popped);
        if popped_ptr.is_null() {
            return -1;
        }
        if result.is_null() {
            // PyDict_Pop with null output still consumes one owned result reference.
            unsafe { Py_DecRef(popped_ptr) };
        } else {
            // SAFETY: caller provided writable output pointer.
            unsafe { *result = popped_ptr };
        }
        1
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        if !result.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe { *result = std::ptr::null_mut() };
        }
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_PopString(
    dict: *mut c_void,
    key: *const c_char,
    result: *mut *mut c_void,
) -> i32 {
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        if !result.is_null() {
            // SAFETY: caller provided writable output pointer.
            unsafe { *result = std::ptr::null_mut() };
        }
        return -1;
    }
    let status = unsafe { PyDict_Pop(dict, key_obj, result) };
    // SAFETY: `key_obj` is a temporary strong reference created above.
    unsafe { Py_DecRef(key_obj) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyDict_Pop(
    dict: *mut c_void,
    key: *mut c_void,
    default_value: *mut c_void,
) -> *mut c_void {
    let mut result = std::ptr::null_mut();
    let status = unsafe { PyDict_Pop(dict, key, std::ptr::addr_of_mut!(result)) };
    if status < 0 {
        return std::ptr::null_mut();
    }
    if status == 1 {
        return result;
    }
    if !default_value.is_null() {
        unsafe { Py_IncRef(default_value) };
        return default_value;
    }
    unsafe { PyErr_SetObject(PyExc_KeyError, key) };
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItem(dict: *mut c_void, key: *mut c_void) -> i32 {
    with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
        if let Some(target) = context.cpython_value_from_ptr(dict) {
            let Value::Dict(dict_obj) = target else {
                context.set_error("PyDict_DelItem expected dict object");
                return -1;
            };
            if context.vm.is_null() {
                context.set_error("PyDict_DelItem missing VM context");
                return -1;
            }
            // SAFETY: VM pointer is valid for context lifetime.
            let vm = unsafe { &mut *context.vm };
            let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
                context.set_error("PyDict_DelItem received unknown key pointer");
                return -1;
            };
            if cpython_trace_numpy_reduce_enabled()
                && let Value::Str(name) = &key_value
                && cpython_is_reduce_probe_name(name)
            {
                eprintln!("[numpy-reduce] PyDict_DelItem dict={:p} key={}", dict, name);
            }
            let removed = match vm.dict_remove_value_runtime(&dict_obj, &key_value) {
                Ok(removed) => removed,
                Err(err) => {
                    context.set_error(err.message);
                    return -1;
                }
            };
            if removed.is_some() {
                if let Some(module_obj) = module_target
                    && let Value::Str(name) = &key_value
                    && let Object::Module(module_data) = &mut *module_obj.kind_mut()
                {
                    module_data.globals.remove(name);
                }
                return 0;
            }
            if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
                eprintln!(
                    "[cpy-dict-err] PyDict_DelItem internal dict={:p} key_ptr={:p} key={} err=key-not-found",
                    dict,
                    key,
                    cpython_debug_compare_value(&key_value)
                );
            }
            context.set_error("PyDict_DelItem key not found");
            return -1;
        }

        // External-dict fallback: native code may pass foreign dict pointers not present
        // in the runtime-owned C-API object table.
        const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
        if dict.is_null()
            || key.is_null()
            || (dict as usize) < MIN_VALID_PTR
            || (dict as usize) % std::mem::align_of::<CpythonObjectHead>() != 0
        {
            context.set_error(format!(
                "PyDict_DelItem received unknown dict pointer {:p}",
                dict
            ));
            return -1;
        }
        // SAFETY: best-effort inspection of foreign object header.
        let type_ptr = unsafe {
            dict.cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null()
            || (type_ptr as usize) < MIN_VALID_PTR
            || (type_ptr as usize) % std::mem::align_of::<CpythonTypeObject>() != 0
        {
            context.set_error(format!(
                "PyDict_DelItem received unknown dict pointer {:p}",
                dict
            ));
            return -1;
        }
        // SAFETY: validated type pointer for slot reads.
        let mapping = unsafe { (*type_ptr).tp_as_mapping.cast::<CpythonMappingMethods>() };
        if mapping.is_null()
            || (mapping as usize) < MIN_VALID_PTR
            || (mapping as usize) % std::mem::align_of::<CpythonMappingMethods>() != 0
        {
            context.set_error("PyDict_DelItem expected dict object");
            return -1;
        }
        // SAFETY: mapping slot table follows CPython ABI.
        let mp_ass_subscript = unsafe { (*mapping).mp_ass_subscript };
        if mp_ass_subscript.is_null() {
            context.set_error("PyDict_DelItem expected dict object");
            return -1;
        }
        let subscript: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int =
            // SAFETY: `mp_ass_subscript` follows CPython mapping ABI.
            unsafe { std::mem::transmute(mp_ass_subscript) };
        if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_DEL_EXTERNAL") {
            eprintln!(
                "[cpy-dict-del-external] dict={:p} key={:p} type_ptr={:p} mapping={:p} mp_ass_subscript={:p}",
                dict, key, type_ptr, mapping, mp_ass_subscript
            );
        }
        // SAFETY: deletion uses `value=NULL` in CPython mapping slot convention.
        let status = unsafe { subscript(dict, key, std::ptr::null_mut()) };
        if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_DEL_EXTERNAL") {
            eprintln!(
                "[cpy-dict-del-external] status={} dict={:p} key={:p}",
                status, dict, key
            );
        }
        if status < 0 && unsafe { PyErr_Occurred() }.is_null() {
            if super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
                eprintln!(
                    "[cpy-dict-err] PyDict_DelItem external dict={:p} key={:p} err=no-exception",
                    dict, key
                );
            }
            context.set_error("PyDict_DelItem mapping slot failed without setting an exception");
            return -1;
        }
        if status < 0 && super::super::env_var_present_cached("PYRS_TRACE_PYDICT_ERRORS") {
            eprintln!(
                "[cpy-dict-err] PyDict_DelItem external dict={:p} key={:p} err=with-exception",
                dict, key
            );
        }
        status
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItemString(dict: *mut c_void, key: *const c_char) -> i32 {
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    unsafe { PyDict_DelItem(dict, key_obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_ContainsString(dict: *mut c_void, key: *const c_char) -> i32 {
    let key_obj = unsafe { PyUnicode_FromString(key) };
    if key_obj.is_null() {
        return -1;
    }
    unsafe { PyDict_Contains(dict, key_obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Copy(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Copy missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Copy received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Copy expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Copy encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let copied = vm.heap.alloc_dict(entries);
        context.alloc_cpython_ptr_for_value(copied)
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Clear(dict: *mut c_void) {
    let _ = with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Clear received unknown dict pointer");
            return;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Clear expected dict object");
            return;
        };
        let mut dict_kind = dict_obj.kind_mut();
        let Object::Dict(values) = &mut *dict_kind else {
            context.set_error("PyDict_Clear encountered invalid dict storage");
            return;
        };
        values.clear();
        if let Some(module_obj) = module_target
            && let Object::Module(module_data) = &mut *module_obj.kind_mut()
        {
            module_data.globals.clear();
        }
    })
    .map_err(cpython_set_error);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Merge(
    dict: *mut c_void,
    other: *mut c_void,
    override_existing: i32,
) -> i32 {
    with_active_cpython_context_mut(|context| {
        let module_target = context.module_dict_module_for_ptr(dict);
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Merge received unknown dict pointer");
            return -1;
        };
        let Some(source) = context.cpython_value_from_ptr(other) else {
            context.set_error("PyDict_Merge received unknown source pointer");
            return -1;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Merge expected target dict");
            return -1;
        };
        if context.vm.is_null() {
            context.set_error("PyDict_Merge missing VM context");
            return -1;
        }
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let Value::Dict(source_obj) = source else {
            context.set_error("PyDict_Merge expected source dict");
            return -1;
        };
        let source_entries = match &*source_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Merge encountered invalid source dict storage");
                return -1;
            }
        };
        let replace_existing = override_existing != 0;
        for (key, value) in source_entries {
            if !replace_existing {
                let should_skip = match vm.dict_contains_key_checked_runtime(&dict_obj, &key) {
                    Ok(contains) => contains,
                    Err(err) => {
                        context.set_error(err.message);
                        return -1;
                    }
                };
                if should_skip {
                    continue;
                }
            }
            let module_key = match &key {
                Value::Str(name) => Some(name.clone()),
                _ => None,
            };
            let module_value = value.clone();
            if let Err(err) = vm.dict_set_value_checked_runtime(&dict_obj, key, value) {
                context.set_error(err.message);
                return -1;
            }
            if let Some(module_obj) = module_target.as_ref()
                && let Some(name) = module_key.as_ref()
                && let Object::Module(module_data) = &mut *module_obj.kind_mut()
            {
                module_data
                    .globals
                    .insert(name.clone(), module_value.clone());
            }
        }
        0
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        -1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Update(dict: *mut c_void, other: *mut c_void) -> i32 {
    unsafe { PyDict_Merge(dict, other, 1) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_MergeFromSeq2(
    dict: *mut c_void,
    seq2: *mut c_void,
    override_existing: i32,
) -> i32 {
    let seq_value = match cpython_value_from_ptr(seq2) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let mapping_value = match cpython_call_builtin(BuiltinFunction::Dict, vec![seq_value]) {
        Ok(value) => value,
        Err(err) => {
            cpython_set_error(err);
            return -1;
        }
    };
    let mapping = cpython_new_ptr_for_value(mapping_value);
    if mapping.is_null() {
        return -1;
    }
    let status = unsafe { PyDict_Merge(dict, mapping, override_existing) };
    unsafe { Py_XDecRef(mapping) };
    status
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Keys(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Keys missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Keys received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Keys expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Keys encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let keys = entries.into_iter().map(|(key, _)| key).collect::<Vec<_>>();
        context.alloc_cpython_ptr_for_value(Value::List(vm.heap.alloc(Object::List(keys))))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Values(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Values missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Values received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Values expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Values encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let values = entries
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        context.alloc_cpython_ptr_for_value(Value::List(vm.heap.alloc(Object::List(values))))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Items(dict: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyDict_Items missing VM context");
            return std::ptr::null_mut();
        }
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Items received unknown dict pointer");
            return std::ptr::null_mut();
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Items expected dict object");
            return std::ptr::null_mut();
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Items encountered invalid dict storage");
                return std::ptr::null_mut();
            }
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let mut items = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let tuple = vm.heap.alloc(Object::Tuple(vec![key, value]));
            items.push(Value::Tuple(tuple));
        }
        context.alloc_cpython_ptr_for_value(Value::List(vm.heap.alloc(Object::List(items))))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Next(
    dict: *mut c_void,
    position: *mut isize,
    out_key: *mut *mut c_void,
    out_value: *mut *mut c_void,
) -> i32 {
    if position.is_null() {
        cpython_set_error("PyDict_Next requires non-null position pointer");
        return 0;
    }
    with_active_cpython_context_mut(|context| {
        let Some(target) = context.cpython_value_from_ptr(dict) else {
            context.set_error("PyDict_Next received unknown dict pointer");
            return 0;
        };
        let Value::Dict(dict_obj) = target else {
            context.set_error("PyDict_Next expected dict object");
            return 0;
        };
        let entries = match &*dict_obj.kind() {
            Object::Dict(values) => values.to_vec(),
            _ => {
                context.set_error("PyDict_Next encountered invalid dict storage");
                return 0;
            }
        };
        // SAFETY: caller-provided pointer is writable.
        let idx = unsafe { *position };
        if idx < 0 || idx as usize >= entries.len() {
            return 0;
        }
        let (key, value) = entries[idx as usize].clone();
        if !out_key.is_null() {
            // SAFETY: caller-provided pointer is writable.
            unsafe { *out_key = context.alloc_cpython_ptr_for_value(key) };
        }
        if !out_value.is_null() {
            // SAFETY: caller-provided pointer is writable.
            unsafe { *out_value = context.alloc_cpython_ptr_for_value(value) };
        }
        // SAFETY: caller-provided pointer is writable.
        unsafe { *position = idx + 1 };
        1
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDictProxy_New(dict: *mut c_void) -> *mut c_void {
    unsafe { PyDict_Copy(dict) }
}
