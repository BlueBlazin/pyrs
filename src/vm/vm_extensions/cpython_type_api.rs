use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_uint, c_void};

use crate::runtime::{Object, Value};

use super::{
    CpythonHeapTypeInfo, CpythonObjectHead, CpythonTypeObject, CpythonTypeSpec, ModuleCapiContext,
    PY_TPFLAGS_BASETYPE, PY_TPFLAGS_BYTES_SUBCLASS,
    PY_TPFLAGS_DICT_SUBCLASS, PY_TPFLAGS_HEAPTYPE, PY_TPFLAGS_IMMUTABLETYPE,
    PY_TPFLAGS_LIST_SUBCLASS, PY_TPFLAGS_LONG_SUBCLASS, PY_TPFLAGS_READY,
    PY_TPFLAGS_TUPLE_SUBCLASS, PY_TPFLAGS_TYPE_SUBCLASS, PY_TPFLAGS_UNICODE_SUBCLASS,
    PY_TYPE_SLOT_MAX, PY_TYPE_SLOT_TP_ALLOC, PY_TYPE_SLOT_TP_BASE, PY_TYPE_SLOT_TP_BASES,
    PY_TYPE_SLOT_TP_CALL, PY_TYPE_SLOT_TP_CLEAR, PY_TYPE_SLOT_TP_DEALLOC, PY_TYPE_SLOT_TP_DEL,
    PY_TYPE_SLOT_TP_DESCR_GET, PY_TYPE_SLOT_TP_DESCR_SET, PY_TYPE_SLOT_TP_DOC,
    PY_TYPE_SLOT_TP_FINALIZE, PY_TYPE_SLOT_TP_FREE, PY_TYPE_SLOT_TP_GETATTR,
    PY_TYPE_SLOT_TP_GETATTRO, PY_TYPE_SLOT_TP_GETSET, PY_TYPE_SLOT_TP_HASH,
    PY_TYPE_SLOT_TP_INIT, PY_TYPE_SLOT_TP_IS_GC, PY_TYPE_SLOT_TP_ITER,
    PY_TYPE_SLOT_TP_ITERNEXT, PY_TYPE_SLOT_TP_MEMBERS, PY_TYPE_SLOT_TP_METHODS,
    PY_TYPE_SLOT_TP_NEW, PY_TYPE_SLOT_TP_REPR, PY_TYPE_SLOT_TP_RICHCOMPARE,
    PY_TYPE_SLOT_TP_SETATTR, PY_TYPE_SLOT_TP_SETATTRO, PY_TYPE_SLOT_TP_STR,
    PY_TYPE_SLOT_TP_TOKEN, PY_TYPE_SLOT_TP_TRAVERSE, PY_TYPE_SLOT_TP_VECTORCALL,
    PyBaseObject_Type, PyDict_New, PyErr_BadInternalCall, PyExc_SystemError,
    PyExc_TypeError, PyExc_MemoryError, PyModule_GetState, PyObject_Free, PyTuple_GetItem,
    PyTuple_New, PyTuple_SetItem, PyTuple_Size, PyType_Type, Py_DecRef, Py_IncRef, Py_XIncRef,
    _PyObject_New, _PyObject_NewVar, c_name_to_string, cpython_builtin_type_ptr_for_class_name,
    cpython_heap_type_registry, cpython_keyword_args_from_dict_object, cpython_new_ptr_for_value,
    cpython_positional_args_from_tuple_object, cpython_set_error, cpython_set_typed_error,
    cpython_value_from_ptr, free, with_active_cpython_context_mut,
};

unsafe extern "C" {
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
}

pub(super) unsafe extern "C" fn cpython_type_tp_call(
    callable: *mut c_void,
    args: *mut c_void,
    kwargs: *mut c_void,
) -> *mut c_void {
    let trace_calls = std::env::var_os("PYRS_TRACE_CPY_CALLS").is_some();
    if callable.is_null() {
        cpython_set_error("type call received null callable");
        return std::ptr::null_mut();
    }
    if callable == (&raw mut PyType_Type).cast() {
        let positional = match cpython_positional_args_from_tuple_object(args) {
            Ok(values) => values,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        let keywords = match cpython_keyword_args_from_dict_object(kwargs) {
            Ok(values) => values,
            Err(err) => {
                cpython_set_error(err);
                return std::ptr::null_mut();
            }
        };
        if positional.len() == 1 && keywords.is_empty() {
            let ptr = cpython_new_ptr_for_value(positional[0].clone());
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            // SAFETY: object pointer was materialized by `cpython_new_ptr_for_value`.
            let ty = unsafe { (*ptr.cast::<CpythonObjectHead>()).ob_type };
            unsafe { Py_XIncRef(ty) };
            return ty;
        }
        if positional.len() != 3 {
            cpython_set_error("TypeError: type() takes 1 or 3 arguments");
            return std::ptr::null_mut();
        }
    }
    let ty = callable.cast::<CpythonTypeObject>();
    // SAFETY: callable points to a PyTypeObject-compatible struct.
    let new_slot = unsafe { (*ty).tp_new };
    if new_slot.is_null() {
        let type_name =
            unsafe { c_name_to_string((*ty).tp_name) }.unwrap_or_else(|_| "<unnamed>".to_string());
        cpython_set_error(format!("TypeError: cannot create '{type_name}' instances"));
        return std::ptr::null_mut();
    }
    if trace_calls {
        // SAFETY: callable points to a PyTypeObject-compatible struct.
        let init_slot = unsafe { (*ty).tp_init };
        eprintln!(
            "[cpy-type-call] callable={:p} tp_new={:p} tp_init={:p} args_ptr={:p} kwargs_ptr={:p}",
            callable, new_slot, init_slot, args, kwargs
        );
    }
    let new_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
        // SAFETY: tp_new follows CPython `newfunc` signature.
        unsafe { std::mem::transmute(new_slot) };
    let object = unsafe { new_fn(callable, args, kwargs) };
    if trace_calls {
        let object_type = if object.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: object returned by tp_new is expected to be PyObject-compatible.
            unsafe { (*object.cast::<CpythonObjectHead>()).ob_type }
        };
        eprintln!(
            "[cpy-type-call] tp_new_result object={:p} object_type={:p}",
            object, object_type
        );
    }
    if object.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: object returned by tp_new must be PyObject-compatible.
    let object_type = unsafe { (*object.cast::<CpythonObjectHead>()).ob_type };
    let should_init = unsafe { PyType_IsSubtype(object_type, callable) != 0 };
    if !should_init {
        return object;
    }
    let init_slot = unsafe {
        object_type
            .cast::<CpythonTypeObject>()
            .as_ref()
            .map(|object_type| object_type.tp_init)
            .unwrap_or(std::ptr::null_mut())
    };
    if init_slot.is_null() {
        return object;
    }
    let init_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 =
        // SAFETY: tp_init follows CPython `initproc` signature.
        unsafe { std::mem::transmute(init_slot) };
    let status = unsafe { init_fn(object, args, kwargs) };
    if status < 0 {
        unsafe { Py_DecRef(object) };
        return std::ptr::null_mut();
    }
    if trace_calls {
        eprintln!(
            "[cpy-type-call] init complete object={:p} object_type={:p} tp_init={:p}",
            object, object_type, init_slot
        );
    }
    if trace_calls {
        let callable_type_name =
            unsafe { c_name_to_string((*ty).tp_name) }.unwrap_or_else(|_| "<unnamed>".to_string());
        let object_type_name = unsafe {
            object_type
                .cast::<CpythonTypeObject>()
                .as_ref()
                .map(|raw| {
                    c_name_to_string(raw.tp_name).unwrap_or_else(|_| "<unnamed>".to_string())
                })
                .unwrap_or_else(|| "<null>".to_string())
        };
        eprintln!(
            "[cpy-type-call] callable_type={} object_type={} should_init={}",
            callable_type_name, object_type_name, should_init
        );
    }
    object
}

pub(super) fn cpython_is_type_object_ptr(ptr: *mut c_void) -> bool {
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    if ptr.is_null() {
        return false;
    }
    if (ptr as usize) < MIN_VALID_PTR {
        return false;
    }
    if (ptr as usize) % std::mem::align_of::<CpythonObjectHead>() != 0 {
        return false;
    }
    let type_type = std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>();
    if ptr == type_type {
        return true;
    }
    // SAFETY: `ptr` is assumed to reference a CPython object header.
    let object_type = unsafe {
        ptr.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type)
            .unwrap_or(std::ptr::null_mut())
    };
    if object_type.is_null() {
        return ModuleCapiContext::is_probable_type_object_without_metatype(ptr);
    }
    if object_type == type_type {
        return true;
    }
    // SAFETY: object_type and type_type are validated non-null pointers.
    unsafe { PyType_IsSubtype(object_type, type_type) != 0 }
}

fn cpython_type_ptr_from_value(value: &Value) -> Option<*mut CpythonTypeObject> {
    if let Some(raw) = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(value)
        && cpython_is_type_object_ptr(raw)
    {
        return Some(raw.cast::<CpythonTypeObject>());
    }
    match value {
        Value::Class(class_obj) => {
            let Object::Class(class_data) = &*class_obj.kind() else {
                return None;
            };
            cpython_builtin_type_ptr_for_class_name(&class_data.name)
                .filter(|ptr| cpython_is_type_object_ptr(*ptr))
                .map(|ptr| ptr.cast::<CpythonTypeObject>())
        }
        _ => None,
    }
}

fn cpython_resolve_type_base_from_arg(
    bases: *mut c_void,
) -> Result<*mut CpythonTypeObject, String> {
    let default = std::ptr::addr_of_mut!(PyBaseObject_Type);
    if bases.is_null() {
        return Ok(default);
    }
    if cpython_is_type_object_ptr(bases) {
        return Ok(bases.cast::<CpythonTypeObject>());
    }
    let value = cpython_value_from_ptr(bases)?;
    match value {
        Value::Tuple(tuple_obj) => {
            let Object::Tuple(items) = &*tuple_obj.kind() else {
                return Ok(default);
            };
            for item in items {
                if let Some(base) = cpython_type_ptr_from_value(item) {
                    return Ok(base);
                }
            }
            Ok(default)
        }
        _ => Err("bases must be a type or tuple of types".to_string()),
    }
}

fn cpython_split_type_name(full_name: &str) -> (String, String) {
    if let Some((module_name, qualname)) = full_name.rsplit_once('.')
        && !module_name.is_empty()
        && !qualname.is_empty()
    {
        return (module_name.to_string(), qualname.to_string());
    }
    ("builtins".to_string(), full_name.to_string())
}

fn cpython_type_name_from_tp_name(type_ptr: *mut CpythonTypeObject) -> String {
    // SAFETY: `type_ptr` is expected to reference a type object.
    let full = unsafe { c_name_to_string((*type_ptr).tp_name) }
        .unwrap_or_else(|_| "<unnamed>".to_string());
    full.rsplit('.').next().unwrap_or(full.as_str()).to_string()
}

fn cpython_type_is_heap_type(type_ptr: *mut CpythonTypeObject) -> bool {
    if type_ptr.is_null() {
        return false;
    }
    // SAFETY: caller ensures a non-null type pointer.
    unsafe { ((*type_ptr).tp_flags & PY_TPFLAGS_HEAPTYPE) != 0 }
}

fn cpython_type_qualname_from_tp_name(type_ptr: *mut CpythonTypeObject) -> String {
    cpython_type_name_from_tp_name(type_ptr)
}

fn cpython_type_module_name_from_tp_name(type_ptr: *mut CpythonTypeObject) -> String {
    // SAFETY: `type_ptr` is expected to reference a type object.
    let full = unsafe { c_name_to_string((*type_ptr).tp_name) }
        .unwrap_or_else(|_| "<unnamed>".to_string());
    full.rsplit_once('.')
        .map(|(module_name, _)| module_name.to_string())
        .unwrap_or_else(|| "builtins".to_string())
}

fn cpython_apply_type_slot(
    ty: &mut CpythonTypeObject,
    slot: c_int,
    pfunc: *mut c_void,
    token: &mut usize,
) -> Result<(), String> {
    match slot {
        PY_TYPE_SLOT_TP_ALLOC => ty.tp_alloc = pfunc,
        PY_TYPE_SLOT_TP_BASE => ty.tp_base = pfunc.cast::<CpythonTypeObject>(),
        PY_TYPE_SLOT_TP_BASES => ty.tp_bases = pfunc,
        PY_TYPE_SLOT_TP_CALL => ty.tp_call = pfunc,
        PY_TYPE_SLOT_TP_CLEAR => ty.tp_clear = pfunc,
        PY_TYPE_SLOT_TP_DEALLOC => ty.tp_dealloc = pfunc,
        PY_TYPE_SLOT_TP_DEL => ty.tp_del = pfunc,
        PY_TYPE_SLOT_TP_DESCR_GET => ty.tp_descr_get = pfunc,
        PY_TYPE_SLOT_TP_DESCR_SET => ty.tp_descr_set = pfunc,
        PY_TYPE_SLOT_TP_DOC => ty.tp_doc = pfunc.cast::<c_char>(),
        PY_TYPE_SLOT_TP_GETATTR => ty.tp_getattr = pfunc,
        PY_TYPE_SLOT_TP_GETATTRO => ty.tp_getattro = pfunc,
        PY_TYPE_SLOT_TP_HASH => ty.tp_hash = pfunc,
        PY_TYPE_SLOT_TP_INIT => ty.tp_init = pfunc,
        PY_TYPE_SLOT_TP_IS_GC => ty.tp_is_gc = pfunc,
        PY_TYPE_SLOT_TP_ITER => ty.tp_iter = pfunc,
        PY_TYPE_SLOT_TP_ITERNEXT => ty.tp_iternext = pfunc,
        PY_TYPE_SLOT_TP_METHODS => ty.tp_methods = pfunc,
        PY_TYPE_SLOT_TP_NEW => ty.tp_new = pfunc,
        PY_TYPE_SLOT_TP_REPR => ty.tp_repr = pfunc,
        PY_TYPE_SLOT_TP_RICHCOMPARE => ty.tp_richcompare = pfunc,
        PY_TYPE_SLOT_TP_SETATTR => ty.tp_setattr = pfunc,
        PY_TYPE_SLOT_TP_SETATTRO => ty.tp_setattro = pfunc,
        PY_TYPE_SLOT_TP_STR => ty.tp_str = pfunc,
        PY_TYPE_SLOT_TP_TRAVERSE => ty.tp_traverse = pfunc,
        PY_TYPE_SLOT_TP_MEMBERS => ty.tp_members = pfunc,
        PY_TYPE_SLOT_TP_GETSET => ty.tp_getset = pfunc,
        PY_TYPE_SLOT_TP_FREE => ty.tp_free = pfunc,
        PY_TYPE_SLOT_TP_FINALIZE => ty.tp_finalize = pfunc,
        PY_TYPE_SLOT_TP_VECTORCALL => ty.tp_vectorcall = pfunc,
        PY_TYPE_SLOT_TP_TOKEN => *token = pfunc as usize,
        1..=PY_TYPE_SLOT_MAX => {}
        _ => return Err(format!("invalid type slot id {slot}")),
    }
    Ok(())
}

fn cpython_type_slot_from_type_object(
    type_ptr: *mut CpythonTypeObject,
    slot: c_int,
) -> *mut c_void {
    if type_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller provides a valid type pointer.
    let ty = unsafe { &*type_ptr };
    match slot {
        PY_TYPE_SLOT_TP_ALLOC => ty.tp_alloc,
        PY_TYPE_SLOT_TP_BASE => ty.tp_base.cast::<c_void>(),
        PY_TYPE_SLOT_TP_BASES => ty.tp_bases,
        PY_TYPE_SLOT_TP_CALL => ty.tp_call,
        PY_TYPE_SLOT_TP_CLEAR => ty.tp_clear,
        PY_TYPE_SLOT_TP_DEALLOC => ty.tp_dealloc,
        PY_TYPE_SLOT_TP_DEL => ty.tp_del,
        PY_TYPE_SLOT_TP_DESCR_GET => ty.tp_descr_get,
        PY_TYPE_SLOT_TP_DESCR_SET => ty.tp_descr_set,
        PY_TYPE_SLOT_TP_DOC => ty.tp_doc as *mut c_void,
        PY_TYPE_SLOT_TP_GETATTR => ty.tp_getattr,
        PY_TYPE_SLOT_TP_GETATTRO => ty.tp_getattro,
        PY_TYPE_SLOT_TP_HASH => ty.tp_hash,
        PY_TYPE_SLOT_TP_INIT => ty.tp_init,
        PY_TYPE_SLOT_TP_IS_GC => ty.tp_is_gc,
        PY_TYPE_SLOT_TP_ITER => ty.tp_iter,
        PY_TYPE_SLOT_TP_ITERNEXT => ty.tp_iternext,
        PY_TYPE_SLOT_TP_METHODS => ty.tp_methods,
        PY_TYPE_SLOT_TP_NEW => ty.tp_new,
        PY_TYPE_SLOT_TP_REPR => ty.tp_repr,
        PY_TYPE_SLOT_TP_RICHCOMPARE => ty.tp_richcompare,
        PY_TYPE_SLOT_TP_SETATTR => ty.tp_setattr,
        PY_TYPE_SLOT_TP_SETATTRO => ty.tp_setattro,
        PY_TYPE_SLOT_TP_STR => ty.tp_str,
        PY_TYPE_SLOT_TP_TRAVERSE => ty.tp_traverse,
        PY_TYPE_SLOT_TP_MEMBERS => ty.tp_members,
        PY_TYPE_SLOT_TP_GETSET => ty.tp_getset,
        PY_TYPE_SLOT_TP_FREE => ty.tp_free,
        PY_TYPE_SLOT_TP_FINALIZE => ty.tp_finalize,
        PY_TYPE_SLOT_TP_VECTORCALL => ty.tp_vectorcall,
        _ => std::ptr::null_mut(),
    }
}

fn cpython_module_name_and_def_for_type_creation(
    module_ptr: *mut c_void,
    fallback_module_name: String,
) -> Result<(String, usize), String> {
    if module_ptr.is_null() {
        return Ok((fallback_module_name, 0));
    }
    with_active_cpython_context_mut(|context| {
        let Some(value) = context.cpython_value_from_ptr_or_proxy(module_ptr) else {
            return Err("invalid module object".to_string());
        };
        let Value::Module(module_obj) = value else {
            return Err("module argument must be a module object".to_string());
        };
        let module_name = match &*module_obj.kind() {
            Object::Module(module_data) => module_data.name.clone(),
            _ => fallback_module_name,
        };
        let mut module_def_ptr = 0usize;
        if !context.vm.is_null() {
            // SAFETY: VM pointer is valid for active context lifetime.
            let vm = unsafe { &mut *context.vm };
            module_def_ptr = vm
                .extension_module_def_registry
                .get(&module_obj.id())
                .copied()
                .unwrap_or(0);
        }
        Ok((module_name, module_def_ptr))
    })
    .unwrap_or_else(|err| Err(err.to_string()))
}

fn cpython_type_from_spec_impl(
    metaclass: *mut c_void,
    module: *mut c_void,
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    if spec.is_null() {
        cpython_set_typed_error(unsafe { PyExc_SystemError }, "type spec cannot be NULL");
        return std::ptr::null_mut();
    }
    // SAFETY: pointer is validated non-null above.
    let spec = unsafe { &*(spec.cast::<CpythonTypeSpec>()) };
    if spec.name.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "type spec name cannot be NULL",
        );
        return std::ptr::null_mut();
    }
    let full_name = match unsafe { c_name_to_string(spec.name) } {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("invalid type spec name: {err}"));
            return std::ptr::null_mut();
        }
    };
    let metaclass_ptr = if metaclass.is_null() {
        std::ptr::addr_of_mut!(PyType_Type).cast::<c_void>()
    } else if cpython_is_type_object_ptr(metaclass) {
        metaclass
    } else {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            "metaclass must be a type object",
        );
        return std::ptr::null_mut();
    };
    let base = match cpython_resolve_type_base_from_arg(bases) {
        Ok(base) => base,
        Err(err) => {
            cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
            return std::ptr::null_mut();
        }
    };
    let (default_module_name, qualname) = cpython_split_type_name(&full_name);
    let (module_name, module_def_ptr) =
        match cpython_module_name_and_def_for_type_creation(module, default_module_name) {
            Ok(parts) => parts,
            Err(err) => {
                cpython_set_typed_error(unsafe { PyExc_TypeError }, err);
                return std::ptr::null_mut();
            }
        };
    let owned_name = match CString::new(full_name.clone()) {
        Ok(name) => name,
        Err(err) => {
            cpython_set_error(format!("invalid type spec name: {err}"));
            return std::ptr::null_mut();
        }
    };
    // SAFETY: base pointer is validated and can be copied by value as a template.
    let mut type_value = unsafe { std::ptr::read(base) };
    if std::env::var_os("PYRS_TRACE_EXT_SLOTS").is_some() {
        // SAFETY: `base` is validated type pointer for type creation.
        let base_name = unsafe { c_name_to_string((*base).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        // SAFETY: `base` is validated type pointer for type creation.
        let base_size = unsafe { (*base).tp_basicsize };
        eprintln!(
            "[cpy-type-spec] name={} spec_basicsize={} spec_itemsize={} base={} base_basicsize={}",
            full_name, spec.basicsize, spec.itemsize, base_name, base_size
        );
    }
    type_value.ob_refcnt = 1;
    type_value.ob_type = metaclass_ptr;
    type_value.ob_size = 0;
    type_value.tp_name = owned_name.as_ptr();
    if spec.basicsize > 0 {
        type_value.tp_basicsize = spec.basicsize as isize;
    }
    if spec.itemsize >= 0 {
        type_value.tp_itemsize = spec.itemsize as isize;
    }
    type_value.tp_base = base;
    type_value.tp_dict = std::ptr::null_mut();
    type_value.tp_flags |= PY_TPFLAGS_HEAPTYPE | PY_TPFLAGS_BASETYPE;
    type_value.tp_flags |= spec.flags as usize;
    type_value.tp_flags &= !PY_TPFLAGS_READY;

    let mut token = 0usize;
    let mut slot_map: HashMap<c_int, usize> = HashMap::new();
    if !spec.slots.is_null() {
        let mut slot = spec.slots;
        let mut guard = 0usize;
        while guard < 8192 {
            // SAFETY: slots is a contiguous slot table terminated by slot==0.
            let slot_entry = unsafe { &*slot };
            if slot_entry.slot == 0 {
                break;
            }
            let slot_pfunc =
                if slot_entry.slot == PY_TYPE_SLOT_TP_TOKEN && slot_entry.pfunc.is_null() {
                    spec as *const CpythonTypeSpec as *mut c_void
                } else {
                    slot_entry.pfunc
                };
            slot_map.insert(slot_entry.slot, slot_pfunc as usize);
            if let Err(err) =
                cpython_apply_type_slot(&mut type_value, slot_entry.slot, slot_pfunc, &mut token)
            {
                cpython_set_typed_error(unsafe { PyExc_SystemError }, err);
                return std::ptr::null_mut();
            }
            // SAFETY: move to next entry in contiguous table.
            slot = unsafe { slot.add(1) };
            guard += 1;
        }
        if guard == 8192 {
            cpython_set_typed_error(
                unsafe { PyExc_SystemError },
                "type slot table is not terminated",
            );
            return std::ptr::null_mut();
        }
    }

    let type_data_size = (type_value.tp_basicsize - unsafe { (*base).tp_basicsize }).max(0);
    let heap_type_size = unsafe { PyType_Type.tp_basicsize.max(0) as usize }
        .max(std::mem::size_of::<CpythonTypeObject>());
    // SAFETY: allocate heap-type storage large enough for CPython heap-type layout expectations.
    let type_ptr = unsafe { calloc(1, heap_type_size) }.cast::<CpythonTypeObject>();
    if type_ptr.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_MemoryError },
            "failed to allocate heap type object",
        );
        return std::ptr::null_mut();
    }
    // SAFETY: destination points to writable heap block with room for CpythonTypeObject header.
    unsafe {
        std::ptr::write(type_ptr, type_value);
    }
    if unsafe { PyType_Ready(type_ptr.cast::<c_void>()) } != 0 {
        // SAFETY: type pointer allocated above and not published on failure.
        unsafe {
            free(type_ptr.cast());
        }
        return std::ptr::null_mut();
    }
    let type_key = type_ptr as usize;
    match cpython_heap_type_registry().lock() {
        Ok(mut registry) => {
            registry.insert(
                type_key,
                CpythonHeapTypeInfo {
                    _owned_name: owned_name,
                    qualname,
                    module_name,
                    module_ptr: module as usize,
                    module_def_ptr,
                    token,
                    type_data_size,
                    slots: slot_map,
                },
            );
        }
        Err(_) => {
            std::mem::forget(owned_name);
        }
    }
    type_ptr.cast::<c_void>()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromMetaclass(
    metaclass: *mut c_void,
    module: *mut c_void,
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    cpython_type_from_spec_impl(metaclass, module, spec, bases)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromModuleAndSpec(
    module: *mut c_void,
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    cpython_type_from_spec_impl(std::ptr::null_mut(), module, spec, bases)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromSpecWithBases(
    spec: *mut c_void,
    bases: *mut c_void,
) -> *mut c_void {
    cpython_type_from_spec_impl(std::ptr::null_mut(), std::ptr::null_mut(), spec, bases)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromSpec(spec: *mut c_void) -> *mut c_void {
    cpython_type_from_spec_impl(
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        spec,
        std::ptr::null_mut(),
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Str(cpython_type_name_from_tp_name(
        ty.cast::<CpythonTypeObject>(),
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetQualName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_key = ty as usize;
    let qualname = cpython_heap_type_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.get(&type_key).map(|info| info.qualname.clone()))
        .unwrap_or_else(|| cpython_type_qualname_from_tp_name(ty.cast::<CpythonTypeObject>()));
    cpython_new_ptr_for_value(Value::Str(qualname))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_key = ty as usize;
    let module_name = cpython_heap_type_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.get(&type_key).map(|info| info.module_name.clone()))
        .unwrap_or_else(|| cpython_type_module_name_from_tp_name(ty.cast::<CpythonTypeObject>()));
    cpython_new_ptr_for_value(Value::Str(module_name))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFullyQualifiedName(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_ptr = ty.cast::<CpythonTypeObject>();
    if !cpython_type_is_heap_type(type_ptr) {
        let full = unsafe { c_name_to_string((*type_ptr).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        return cpython_new_ptr_for_value(Value::Str(full));
    }
    let type_key = ty as usize;
    let (module_name, qualname) = cpython_heap_type_registry()
        .lock()
        .ok()
        .and_then(|registry| {
            registry.get(&type_key).map(|info| {
                let module = info.module_name.clone();
                let qual = info.qualname.clone();
                (module, qual)
            })
        })
        .unwrap_or_else(|| {
            (
                cpython_type_module_name_from_tp_name(ty.cast::<CpythonTypeObject>()),
                cpython_type_qualname_from_tp_name(ty.cast::<CpythonTypeObject>()),
            )
        });
    let full_name =
        if module_name.is_empty() || module_name == "builtins" || module_name == "__main__" {
            qualname
        } else {
            format!("{module_name}.{qualname}")
        };
    cpython_new_ptr_for_value(Value::Str(full_name))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetSlot(ty: *mut c_void, slot: c_int) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if !(1..=PY_TYPE_SLOT_MAX).contains(&slot) {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_ptr = ty.cast::<CpythonTypeObject>();
    let type_key = type_ptr as usize;
    if let Ok(registry) = cpython_heap_type_registry().lock()
        && let Some(info) = registry.get(&type_key)
    {
        if slot == PY_TYPE_SLOT_TP_TOKEN {
            return info.token as *mut c_void;
        }
        if let Some(raw) = info.slots.get(&slot)
            && *raw != 0
        {
            return *raw as *mut c_void;
        }
    }
    if slot == PY_TYPE_SLOT_TP_TOKEN {
        return std::ptr::null_mut();
    }
    cpython_type_slot_from_type_object(type_ptr, slot)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyType_Lookup(ty: *mut c_void, name: *mut c_void) -> *mut c_void {
    if ty.is_null() || name.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        let Some(name_value) = context.cpython_value_from_ptr_or_proxy(name) else {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        };
        let Value::Str(attr_name) = name_value else {
            unsafe { PyErr_BadInternalCall() };
            return std::ptr::null_mut();
        };
        // _PyType_Lookup() must not leave a new error behind when lookup misses.
        let saved_current_error = context.current_error;
        let saved_last_error = context.last_error.clone();
        let saved_first_error = context.first_error.clone();
        let result = context
            .lookup_type_attr_via_tp_dict(ty, &attr_name)
            .or_else(|| context.lookup_type_attr_via_runtime_mro(ty, &attr_name))
            .unwrap_or(std::ptr::null_mut());
        context.current_error = saved_current_error;
        context.last_error = saved_last_error;
        context.first_error = saved_first_error;
        result
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModule(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    let type_ptr = ty.cast::<CpythonTypeObject>();
    let type_key = type_ptr as usize;
    let Some((module_ptr, type_name)) =
        cpython_heap_type_registry()
            .lock()
            .ok()
            .and_then(|registry| {
                registry
                    .get(&type_key)
                    .map(|info| (info.module_ptr, cpython_type_name_from_tp_name(type_ptr)))
            })
    else {
        let type_name = unsafe { c_name_to_string((*type_ptr).tp_name) }
            .unwrap_or_else(|_| "<unnamed>".to_string());
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            format!("PyType_GetModule: Type '{type_name}' is not a heap type"),
        );
        return std::ptr::null_mut();
    };
    if module_ptr == 0 {
        cpython_set_typed_error(
            unsafe { PyExc_TypeError },
            format!(
                "PyType_GetModule: Type '{}' has no associated module",
                type_name
            ),
        );
        return std::ptr::null_mut();
    }
    module_ptr as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleState(ty: *mut c_void) -> *mut c_void {
    let module = unsafe { PyType_GetModule(ty) };
    if module.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { PyModule_GetState(module) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleByDef(
    ty: *mut c_void,
    module_def: *mut c_void,
) -> *mut c_void {
    if ty.is_null() || module_def.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return std::ptr::null_mut();
    }
    if !cpython_is_type_object_ptr(ty) {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected type object");
        return std::ptr::null_mut();
    }
    let mut current = ty.cast::<CpythonTypeObject>();
    while !current.is_null() {
        let key = current as usize;
        if let Ok(registry) = cpython_heap_type_registry().lock()
            && let Some(info) = registry.get(&key)
            && info.module_ptr != 0
            && info.module_def_ptr != 0
            && info.module_def_ptr == module_def as usize
        {
            return info.module_ptr as *mut c_void;
        }
        // SAFETY: current pointer is valid in MRO walk.
        current = unsafe { (*current).tp_base };
    }
    // SAFETY: `ty` is non-null and expected to be a type object.
    let type_name = unsafe { c_name_to_string((*ty.cast::<CpythonTypeObject>()).tp_name) }
        .unwrap_or_else(|_| "<unnamed>".to_string());
    cpython_set_typed_error(
        unsafe { PyExc_TypeError },
        format!("PyType_GetModuleByDef: No superclass of '{type_name}' has the given module"),
    );
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetTypeDataSize(ty: *mut c_void) -> isize {
    if ty.is_null() {
        return 0;
    }
    let key = ty as usize;
    if let Ok(registry) = cpython_heap_type_registry().lock()
        && let Some(info) = registry.get(&key)
    {
        return info.type_data_size.max(0);
    }
    // SAFETY: caller provided type pointer.
    let ty = ty.cast::<CpythonTypeObject>();
    // SAFETY: pointer validated non-null above.
    let base = unsafe { (*ty).tp_base };
    if base.is_null() {
        return 0;
    }
    // SAFETY: type and base pointers are valid for read.
    let delta = unsafe { (*ty).tp_basicsize - (*base).tp_basicsize };
    delta.max(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetBaseByToken(
    ty: *mut c_void,
    token: *mut c_void,
    result: *mut *mut c_void,
) -> c_int {
    if !result.is_null() {
        // SAFETY: output pointer is caller-provided and writable.
        unsafe { *result = std::ptr::null_mut() };
    }
    if token.is_null() {
        cpython_set_typed_error(
            unsafe { PyExc_SystemError },
            "PyType_GetBaseByToken called with token=NULL",
        );
        return -1;
    }
    if ty.is_null() || !cpython_is_type_object_ptr(ty) {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected a type object");
        return -1;
    }
    let token_key = token as usize;
    let mut current = ty.cast::<CpythonTypeObject>();
    while !current.is_null() {
        let key = current as usize;
        if let Ok(registry) = cpython_heap_type_registry().lock()
            && let Some(info) = registry.get(&key)
            && info.token == token_key
        {
            if !result.is_null() {
                // SAFETY: output pointer is caller-provided and writable.
                unsafe { *result = current.cast::<c_void>() };
                // SAFETY: result pointer holds a type object pointer that must be returned as new reference.
                unsafe { Py_IncRef(*result) };
            }
            return 1;
        }
        // SAFETY: current pointer is valid in MRO walk.
        current = unsafe { (*current).tp_base };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_ClearCache() -> c_uint {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Modified(_ty: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Freeze(ty: *mut c_void) -> c_int {
    if ty.is_null() {
        unsafe { PyErr_BadInternalCall() };
        return -1;
    }
    if !cpython_is_type_object_ptr(ty) {
        cpython_set_typed_error(unsafe { PyExc_TypeError }, "expected type object");
        return -1;
    }
    // SAFETY: pointer is validated above.
    unsafe {
        (*ty.cast::<CpythonTypeObject>()).tp_flags |= PY_TPFLAGS_IMMUTABLETYPE;
    }
    unsafe { PyType_Modified(ty) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFlags(ty: *mut c_void) -> usize {
    if ty.is_null() {
        return 0;
    }
    // SAFETY: caller provided a type pointer.
    unsafe { ty.cast::<CpythonTypeObject>().as_ref() }
        .map(|ty| ty.tp_flags)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_IsSubtype(subtype: *mut c_void, ty: *mut c_void) -> i32 {
    if subtype.is_null() || ty.is_null() {
        return 0;
    }
    const MIN_VALID_PTR: usize = 0x1_0000_0000;
    const TYPE_ALIGN: usize = std::mem::align_of::<CpythonTypeObject>();
    if (subtype as usize) < MIN_VALID_PTR || (ty as usize) < MIN_VALID_PTR {
        return 0;
    }
    if (subtype as usize) % TYPE_ALIGN != 0 || (ty as usize) % TYPE_ALIGN != 0 {
        return 0;
    }
    let target = ty.cast::<CpythonTypeObject>();
    let mut current = subtype.cast::<CpythonTypeObject>();
    while !current.is_null() {
        if (current as usize) < MIN_VALID_PTR || (current as usize) % TYPE_ALIGN != 0 {
            return 0;
        }
        if current == target {
            return 1;
        }
        // SAFETY: current is checked non-null.
        let next = unsafe { (*current).tp_base };
        if next == current {
            break;
        }
        current = next;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Ready(ty: *mut c_void) -> i32 {
    if ty.is_null() {
        cpython_set_error("PyType_Ready received null type");
        return -1;
    }
    // SAFETY: caller provided non-null type pointer.
    let ty = ty.cast::<CpythonTypeObject>();
    // SAFETY: `ty` is valid for mutation during type ready.
    unsafe {
        if (*ty).ob_type.is_null() {
            (*ty).ob_type = (&raw mut PyType_Type).cast();
        }
        if (*ty).tp_base.is_null()
            && ty != (&raw mut PyBaseObject_Type)
            && ty != (&raw mut PyType_Type)
        {
            (*ty).tp_base = &raw mut PyBaseObject_Type;
        }
        let base = (*ty).tp_base;
        if (*ty).tp_basicsize <= 0 {
            if !base.is_null() && (*base).tp_basicsize > 0 {
                (*ty).tp_basicsize = (*base).tp_basicsize;
            } else {
                (*ty).tp_basicsize = std::mem::size_of::<CpythonObjectHead>() as isize;
            }
        }
        if (*ty).tp_call.is_null() && !base.is_null() {
            (*ty).tp_call = (*base).tp_call;
        }
        if (*ty).tp_init.is_null() && !base.is_null() {
            (*ty).tp_init = (*base).tp_init;
        }
        if (*ty).tp_alloc.is_null() && !base.is_null() {
            (*ty).tp_alloc = (*base).tp_alloc;
        }
        if (*ty).tp_new.is_null() && !base.is_null() {
            (*ty).tp_new = (*base).tp_new;
        }
        if (*ty).tp_free.is_null() && !base.is_null() {
            (*ty).tp_free = (*base).tp_free;
        }
        if (*ty).tp_getattro.is_null() && !base.is_null() {
            (*ty).tp_getattro = (*base).tp_getattro;
        }
        if (*ty).tp_setattro.is_null() && !base.is_null() {
            (*ty).tp_setattro = (*base).tp_setattro;
        }
        if (*ty).tp_repr.is_null() && !base.is_null() {
            (*ty).tp_repr = (*base).tp_repr;
        }
        if (*ty).tp_str.is_null() && !base.is_null() {
            (*ty).tp_str = (*base).tp_str;
        }
        if (*ty).tp_basicsize <= 0 {
            (*ty).tp_basicsize = std::mem::size_of::<CpythonObjectHead>() as isize;
        }
        if (*ty).tp_alloc.is_null() {
            (*ty).tp_alloc = PyType_GenericAlloc as *mut c_void;
        }
        if (*ty).tp_free.is_null() {
            (*ty).tp_free = PyObject_Free as *mut c_void;
        }
        if (*ty).tp_new.is_null() {
            (*ty).tp_new = PyType_GenericNew as *mut c_void;
        }
        if (*ty).tp_dict.is_null() {
            let dict_ptr = PyDict_New();
            if dict_ptr.is_null() {
                return -1;
            }
            (*ty).tp_dict = dict_ptr;
        }
        if (*ty).tp_bases.is_null() {
            let base_obj = if !base.is_null() {
                base.cast::<c_void>()
            } else {
                std::ptr::null_mut()
            };
            let bases_len = usize::from(!base_obj.is_null());
            let bases_tuple = PyTuple_New(bases_len as isize);
            if bases_tuple.is_null() {
                return -1;
            }
            if !base_obj.is_null() {
                Py_XIncRef(base_obj);
                if PyTuple_SetItem(bases_tuple, 0, base_obj) != 0 {
                    Py_DecRef(bases_tuple);
                    return -1;
                }
            }
            (*ty).tp_bases = bases_tuple;
        }
        if (*ty).tp_mro.is_null() {
            let type_obj = ty.cast::<c_void>();
            let mut base_entries: Vec<*mut c_void> = Vec::new();
            if !base.is_null() {
                let base_mro = (*base).tp_mro;
                if !base_mro.is_null() {
                    let base_mro_len = PyTuple_Size(base_mro).max(0) as usize;
                    if base_mro_len > 0 {
                        for index in 0..base_mro_len {
                            let entry = PyTuple_GetItem(base_mro, index as isize);
                            if !entry.is_null() {
                                base_entries.push(entry);
                            }
                        }
                    }
                }
                if base_entries.is_empty() {
                    base_entries.push(base.cast::<c_void>());
                }
            }
            let mro_tuple = PyTuple_New((1 + base_entries.len()) as isize);
            if mro_tuple.is_null() {
                return -1;
            }
            Py_XIncRef(type_obj);
            if PyTuple_SetItem(mro_tuple, 0, type_obj) != 0 {
                Py_DecRef(mro_tuple);
                return -1;
            }
            for (index, entry) in base_entries.into_iter().enumerate() {
                Py_XIncRef(entry);
                if PyTuple_SetItem(mro_tuple, (index + 1) as isize, entry) != 0 {
                    Py_DecRef(mro_tuple);
                    return -1;
                }
            }
            (*ty).tp_mro = mro_tuple;
        }
        if !base.is_null() {
            let inherited_subclass_bits = (*base).tp_flags
                & (PY_TPFLAGS_LONG_SUBCLASS
                    | PY_TPFLAGS_LIST_SUBCLASS
                    | PY_TPFLAGS_TUPLE_SUBCLASS
                    | PY_TPFLAGS_BYTES_SUBCLASS
                    | PY_TPFLAGS_UNICODE_SUBCLASS
                    | PY_TPFLAGS_DICT_SUBCLASS
                    | PY_TPFLAGS_TYPE_SUBCLASS);
            (*ty).tp_flags |= inherited_subclass_bits;
        }
        (*ty).tp_flags |= PY_TPFLAGS_READY;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GenericAlloc(subtype: *mut c_void, nitems: isize) -> *mut c_void {
    if subtype.is_null() {
        cpython_set_error("PyType_GenericAlloc received null subtype");
        return std::ptr::null_mut();
    }
    let ty = subtype.cast::<CpythonTypeObject>();
    // SAFETY: subtype is checked non-null.
    let itemsize = unsafe { (*ty).tp_itemsize };
    if itemsize > 0 || nitems > 0 {
        unsafe { _PyObject_NewVar(ty, nitems.max(0)) }
    } else {
        unsafe { _PyObject_New(ty) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GenericNew(
    subtype: *mut c_void,
    _args: *mut c_void,
    _kwargs: *mut c_void,
) -> *mut c_void {
    if subtype.is_null() {
        cpython_set_error("PyType_GenericNew received null subtype");
        return std::ptr::null_mut();
    }
    let ty = subtype.cast::<CpythonTypeObject>();
    // SAFETY: subtype is checked non-null.
    let alloc = unsafe { (*ty).tp_alloc };
    if alloc.is_null() {
        return unsafe { PyType_GenericAlloc(subtype, 0) };
    }
    let alloc_fn: unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void =
        // SAFETY: tp_alloc slot follows CPython allocfunc signature.
        unsafe { std::mem::transmute(alloc) };
    unsafe { alloc_fn(subtype, 0) }
}

