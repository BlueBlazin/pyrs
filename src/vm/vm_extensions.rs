use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};

use crate::extensions::{
    ExtensionEntrypoint, PYRS_CAPI_ABI_VERSION, PYRS_DYNAMIC_INIT_SYMBOL_V1,
    PYRS_EXTENSION_ABI_TAG, PYRS_EXTENSION_MANIFEST_SUFFIX, PYRS_TYPE_BOOL, PYRS_TYPE_FLOAT,
    PYRS_TYPE_INT, PYRS_TYPE_NONE, PYRS_TYPE_STR, PyrsApiV1, PyrsCFunctionKwV1, PyrsCFunctionV1,
    PyrsObjectHandle, load_dynamic_initializer, parse_extension_manifest, path_is_shared_library,
};
use crate::runtime::{
    BoundMethod, NativeMethodKind, NativeMethodObject, Object, RuntimeError, Value,
};

use super::{ExtensionCallableKind, NativeCallResult, ObjRef, Vm};

struct CapiObjectSlot {
    value: Value,
    refcount: usize,
}

struct ModuleCapiContext {
    vm: *mut Vm,
    module: ObjRef,
    next_object_handle: PyrsObjectHandle,
    objects: HashMap<PyrsObjectHandle, CapiObjectSlot>,
    last_error: Option<String>,
    scratch_strings: Vec<CString>,
}

impl ModuleCapiContext {
    fn new(vm: *mut Vm, module: ObjRef) -> Self {
        Self {
            vm,
            module,
            next_object_handle: 1,
            objects: HashMap::new(),
            last_error: None,
            scratch_strings: Vec::new(),
        }
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.last_error = Some(message.into());
    }

    fn clear_error(&mut self) {
        self.last_error = None;
    }

    fn alloc_object(&mut self, value: Value) -> PyrsObjectHandle {
        let handle = self.next_object_handle;
        self.next_object_handle = self.next_object_handle.wrapping_add(1);
        if self.next_object_handle == 0 {
            self.next_object_handle = 1;
        }
        self.objects
            .insert(handle, CapiObjectSlot { value, refcount: 1 });
        handle
    }

    fn object_slot(&self, handle: PyrsObjectHandle) -> Option<&CapiObjectSlot> {
        self.objects.get(&handle)
    }

    fn object_value(&self, handle: PyrsObjectHandle) -> Option<Value> {
        self.object_slot(handle).map(|slot| slot.value.clone())
    }

    fn incref(&mut self, handle: PyrsObjectHandle) -> Result<(), String> {
        let Some(slot) = self.objects.get_mut(&handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        slot.refcount = slot.refcount.saturating_add(1);
        Ok(())
    }

    fn decref(&mut self, handle: PyrsObjectHandle) -> Result<(), String> {
        let Some(slot) = self.objects.get_mut(&handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        if slot.refcount == 0 {
            self.objects.remove(&handle);
            return Ok(());
        }
        slot.refcount -= 1;
        if slot.refcount == 0 {
            self.objects.remove(&handle);
        }
        Ok(())
    }

    fn object_type(&self, handle: PyrsObjectHandle) -> Result<i32, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        let ty = match slot.value {
            Value::None => PYRS_TYPE_NONE,
            Value::Bool(_) => PYRS_TYPE_BOOL,
            Value::Int(_) => PYRS_TYPE_INT,
            Value::Str(_) => PYRS_TYPE_STR,
            Value::Float(_) => PYRS_TYPE_FLOAT,
            _ => 0,
        };
        Ok(ty)
    }

    fn object_get_int(&self, handle: PyrsObjectHandle) -> Result<i64, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Int(value) => Ok(value),
            _ => Err(format!("object handle {} is not an int", handle)),
        }
    }

    fn object_get_bool(&self, handle: PyrsObjectHandle) -> Result<i32, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Bool(value) => Ok(if value { 1 } else { 0 }),
            _ => Err(format!("object handle {} is not a bool", handle)),
        }
    }

    fn object_get_float(&self, handle: PyrsObjectHandle) -> Result<f64, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        match slot.value {
            Value::Float(value) => Ok(value),
            _ => Err(format!("object handle {} is not a float", handle)),
        }
    }

    fn object_get_string_ptr(&mut self, handle: PyrsObjectHandle) -> Result<*const c_char, String> {
        let Some(slot) = self.object_slot(handle) else {
            return Err(format!("invalid object handle {}", handle));
        };
        let Value::Str(text) = &slot.value else {
            return Err(format!("object handle {} is not a str", handle));
        };
        let cstring = CString::new(text.as_str())
            .map_err(|_| "string contains interior NUL byte".to_string())?;
        self.scratch_strings.push(cstring);
        let ptr = self
            .scratch_strings
            .last()
            .map(|value| value.as_ptr())
            .unwrap_or(std::ptr::null());
        if ptr.is_null() {
            Err("failed to materialize string pointer".to_string())
        } else {
            Ok(ptr)
        }
    }
}

unsafe fn capi_context_mut<'a>(module_ctx: *mut c_void) -> Option<&'a mut ModuleCapiContext> {
    if module_ctx.is_null() {
        return None;
    }
    // SAFETY: caller guarantees `module_ctx` points to a valid `ModuleCapiContext`.
    Some(unsafe { &mut *(module_ctx as *mut ModuleCapiContext) })
}

unsafe fn c_name_to_string(name: *const c_char) -> Result<String, String> {
    if name.is_null() {
        return Err("received null C string pointer".to_string());
    }
    // SAFETY: caller ensures pointer is a valid NUL-terminated C string.
    let c_name = unsafe { CStr::from_ptr(name) };
    c_name
        .to_str()
        .map(|text| text.to_string())
        .map_err(|_| "received non-utf8 C string".to_string())
}

unsafe fn capi_module_insert_value(
    context: &mut ModuleCapiContext,
    name: *const c_char,
    value: Value,
) -> i32 {
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, value);
    0
}

unsafe extern "C" fn capi_module_set_int(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Int(value)) }
}

unsafe extern "C" fn capi_module_set_bool(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: i32,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, Value::Bool(value != 0)) }
}

unsafe extern "C" fn capi_module_set_string(
    module_ctx: *mut c_void,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let value = match unsafe { c_name_to_string(value) } {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    unsafe { capi_module_insert_value(context, name, Value::Str(value)) }
}

unsafe extern "C" fn capi_module_add_function(
    module_ctx: *mut c_void,
    name: *const c_char,
    callback: Option<PyrsCFunctionV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(callback) = callback else {
        context.set_error("module_add_function requires a non-null callback");
        return -1;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    if context.vm.is_null() {
        context.set_error("module_add_function missing VM context");
        return -1;
    }
    // SAFETY: VM pointer is set by `exec_extension_module` and valid during init callback.
    let vm = unsafe { &mut *context.vm };
    let callable = match vm.register_extension_callable(
        context.module.clone(),
        &name,
        ExtensionCallableKind::Positional(callback),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err.message);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, callable);
    0
}

unsafe extern "C" fn capi_module_add_function_kw(
    module_ctx: *mut c_void,
    name: *const c_char,
    callback: Option<PyrsCFunctionKwV1>,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(callback) = callback else {
        context.set_error("module_add_function_kw requires a non-null callback");
        return -1;
    };
    let name = match unsafe { c_name_to_string(name) } {
        Ok(name) => name,
        Err(err) => {
            context.set_error(err);
            return -1;
        }
    };
    if context.vm.is_null() {
        context.set_error("module_add_function_kw missing VM context");
        return -1;
    }
    // SAFETY: VM pointer is set by `exec_extension_module` and valid during init callback.
    let vm = unsafe { &mut *context.vm };
    let callable = match vm.register_extension_callable(
        context.module.clone(),
        &name,
        ExtensionCallableKind::WithKeywords(callback),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err.message);
            return -1;
        }
    };
    let Object::Module(module_data) = &mut *context.module.kind_mut() else {
        context.set_error("module context no longer points to a module");
        return -1;
    };
    module_data.globals.insert(name, callable);
    0
}

unsafe extern "C" fn capi_object_new_int(module_ctx: *mut c_void, value: i64) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Int(value))
}

unsafe extern "C" fn capi_object_new_none(module_ctx: *mut c_void) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::None)
}

unsafe extern "C" fn capi_object_new_bool(module_ctx: *mut c_void, value: i32) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Bool(value != 0))
}

unsafe extern "C" fn capi_object_new_float(
    module_ctx: *mut c_void,
    value: f64,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    context.alloc_object(Value::Float(value))
}

unsafe extern "C" fn capi_object_new_string(
    module_ctx: *mut c_void,
    value: *const c_char,
) -> PyrsObjectHandle {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    let value = match unsafe { c_name_to_string(value) } {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            return 0;
        }
    };
    context.alloc_object(Value::Str(value))
}

unsafe extern "C" fn capi_object_incref(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.incref(handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_decref(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match context.decref(handle) {
        Ok(()) => 0,
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_module_set_object(
    module_ctx: *mut c_void,
    name: *const c_char,
    handle: PyrsObjectHandle,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    let Some(value) = context.object_value(handle) else {
        context.set_error(format!("invalid object handle {}", handle));
        return -1;
    };
    unsafe { capi_module_insert_value(context, name, value) }
}

unsafe extern "C" fn capi_object_type(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 0;
    };
    match context.object_type(handle) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(err);
            0
        }
    }
}

unsafe extern "C" fn capi_object_get_int(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut i64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_int received null out pointer");
        return -1;
    }
    match context.object_get_int(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_float(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut f64,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_float received null out pointer");
        return -1;
    }
    match context.object_get_float(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_bool(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
    out: *mut i32,
) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    if out.is_null() {
        context.set_error("object_get_bool received null out pointer");
        return -1;
    }
    match context.object_get_bool(handle) {
        Ok(value) => {
            // SAFETY: caller provided non-null out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_object_get_string(
    module_ctx: *mut c_void,
    handle: PyrsObjectHandle,
) -> *const c_char {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return std::ptr::null();
    };
    match context.object_get_string_ptr(handle) {
        Ok(ptr) => ptr,
        Err(err) => {
            context.set_error(err);
            std::ptr::null()
        }
    }
}

unsafe extern "C" fn capi_error_set(module_ctx: *mut c_void, message: *const c_char) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    match unsafe { c_name_to_string(message) } {
        Ok(message) => {
            context.set_error(message);
            -1
        }
        Err(err) => {
            context.set_error(err);
            -1
        }
    }
}

unsafe extern "C" fn capi_error_clear(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return -1;
    };
    context.clear_error();
    0
}

unsafe extern "C" fn capi_error_occurred(module_ctx: *mut c_void) -> i32 {
    let Some(context) = (unsafe { capi_context_mut(module_ctx) }) else {
        return 1;
    };
    if context.last_error.is_some() { 1 } else { 0 }
}

enum ExtensionExecutionPlan {
    HelloExt,
    Dynamic {
        library_path: PathBuf,
        symbol: String,
    },
}

impl Vm {
    fn cpython_init_symbol_for_module(module_name: &str) -> String {
        let leaf = module_name
            .rsplit('.')
            .next()
            .unwrap_or(module_name)
            .replace('-', "_");
        format!("PyInit_{leaf}")
    }

    fn capi_api_v1(&self) -> PyrsApiV1 {
        PyrsApiV1 {
            abi_version: PYRS_CAPI_ABI_VERSION,
            module_set_int: capi_module_set_int,
            module_set_bool: capi_module_set_bool,
            module_set_string: capi_module_set_string,
            module_add_function: capi_module_add_function,
            module_add_function_kw: capi_module_add_function_kw,
            object_new_int: capi_object_new_int,
            object_new_none: capi_object_new_none,
            object_new_bool: capi_object_new_bool,
            object_new_float: capi_object_new_float,
            object_new_string: capi_object_new_string,
            object_incref: capi_object_incref,
            object_decref: capi_object_decref,
            module_set_object: capi_module_set_object,
            object_type: capi_object_type,
            object_get_int: capi_object_get_int,
            object_get_float: capi_object_get_float,
            object_get_bool: capi_object_get_bool,
            object_get_string: capi_object_get_string,
            error_set: capi_error_set,
            error_clear: capi_error_clear,
            error_occurred: capi_error_occurred,
        }
    }

    pub(super) fn register_extension_callable(
        &mut self,
        module: ObjRef,
        name: &str,
        kind: ExtensionCallableKind,
    ) -> Result<Value, RuntimeError> {
        let id = self.next_extension_callable_id;
        self.next_extension_callable_id = self.next_extension_callable_id.wrapping_add(1);
        if self.next_extension_callable_id == 0 {
            self.next_extension_callable_id = 1;
        }
        self.extension_callable_registry.insert(
            id,
            super::ExtensionCallableEntry {
                module: module.clone(),
                name: name.to_string(),
                kind,
            },
        );

        let native = self.heap.alloc_native_method(NativeMethodObject::new(
            NativeMethodKind::ExtensionFunctionCall(id),
        ));
        let bound = match self
            .heap
            .alloc_bound_method(BoundMethod::new(native, module))
        {
            Value::BoundMethod(obj) => obj,
            _ => unreachable!(),
        };
        Ok(Value::BoundMethod(bound))
    }

    pub(super) fn call_extension_callable(
        &mut self,
        function_id: u64,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<NativeCallResult, RuntimeError> {
        let Some(entry) = self.extension_callable_registry.get(&function_id).cloned() else {
            return Err(RuntimeError::new(format!(
                "unknown extension callable id {}",
                function_id
            )));
        };
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, entry.module.clone());
        let mut arg_handles = Vec::with_capacity(args.len());
        for arg in args {
            arg_handles.push(call_ctx.alloc_object(arg));
        }
        let api = self.capi_api_v1();
        let mut result_handle: PyrsObjectHandle = 0;
        let status = match entry.kind {
            ExtensionCallableKind::Positional(callback) => {
                if !kwargs.is_empty() {
                    return Err(RuntimeError::new(format!(
                        "extension function '{}.{}' does not accept keyword arguments",
                        match &*entry.module.kind() {
                            Object::Module(module_data) => module_data.name.clone(),
                            _ => "<extension>".to_string(),
                        },
                        entry.name
                    )));
                }
                // SAFETY: callback pointer comes from extension registration and the API/context
                // pointers remain valid for the duration of this call.
                unsafe {
                    callback(
                        &api as *const PyrsApiV1,
                        (&mut call_ctx as *mut ModuleCapiContext).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
            ExtensionCallableKind::WithKeywords(callback) => {
                let mut kw_name_storage = Vec::with_capacity(kwargs.len());
                let mut kw_name_ptrs = Vec::with_capacity(kwargs.len());
                let mut kw_value_handles = Vec::with_capacity(kwargs.len());
                for (name, value) in kwargs {
                    let c_name = CString::new(name.as_str()).map_err(|_| {
                        RuntimeError::new("extension call keyword name contains interior NUL byte")
                    })?;
                    kw_name_storage.push(c_name);
                    let ptr = kw_name_storage
                        .last()
                        .map(|name| name.as_ptr())
                        .unwrap_or(std::ptr::null());
                    kw_name_ptrs.push(ptr);
                    kw_value_handles.push(call_ctx.alloc_object(value));
                }
                // SAFETY: callback pointer comes from extension registration and the API/context
                // pointers remain valid for the duration of this call. Keyword C strings and
                // value handles remain alive for the callback duration.
                unsafe {
                    callback(
                        &api as *const PyrsApiV1,
                        (&mut call_ctx as *mut ModuleCapiContext).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        kw_name_ptrs.len(),
                        kw_name_ptrs.as_ptr(),
                        kw_value_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
        };
        if status != 0 {
            let detail = call_ctx
                .last_error
                .as_deref()
                .map(|text| format!(": {text}"))
                .unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' failed with status {}{}",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name,
                status,
                detail
            )));
        }
        if result_handle == 0 {
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' returned null handle",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name
            )));
        }
        let Some(result) = call_ctx.object_value(result_handle) else {
            return Err(RuntimeError::new(format!(
                "extension function '{}.{}' returned unknown handle {}",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name,
                result_handle
            )));
        };
        Ok(NativeCallResult::Value(result))
    }

    fn set_extension_metadata(
        &mut self,
        module: &ObjRef,
        abi_tag: &str,
        entrypoint: &str,
        origin: &Path,
    ) -> Result<(), RuntimeError> {
        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new("extension load target is not a module"));
        };
        module_data
            .globals
            .insert("__pyrs_extension__".to_string(), Value::Bool(true));
        module_data.globals.insert(
            "__pyrs_extension_abi__".to_string(),
            Value::Str(abi_tag.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_entrypoint__".to_string(),
            Value::Str(entrypoint.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_origin__".to_string(),
            Value::Str(origin.to_string_lossy().to_string()),
        );
        module_data.globals.insert(
            "__pyrs_capi_abi_version__".to_string(),
            Value::Int(PYRS_CAPI_ABI_VERSION as i64),
        );
        Ok(())
    }

    fn execute_dynamic_extension(
        &mut self,
        module: &ObjRef,
        module_name: &str,
        library_path: &Path,
        symbol: &str,
    ) -> Result<(), RuntimeError> {
        let (handle, initializer) = load_dynamic_initializer(library_path, symbol).map_err(|err| {
            if symbol == PYRS_DYNAMIC_INIT_SYMBOL_V1 {
                let cpython_symbol = Self::cpython_init_symbol_for_module(module_name);
                RuntimeError::new(format!(
                    "{err}; expected '{}'. CPython-style extension symbols such as '{}' are not supported yet",
                    PYRS_DYNAMIC_INIT_SYMBOL_V1, cpython_symbol
                ))
            } else {
                RuntimeError::new(err)
            }
        })?;
        let mut module_ctx = ModuleCapiContext::new(self as *mut Vm, module.clone());
        let api = self.capi_api_v1();
        // SAFETY: initializer is resolved from the shared object symbol with expected signature;
        // pointers are valid for the duration of the call.
        let status = unsafe {
            initializer(
                &api as *const PyrsApiV1,
                (&mut module_ctx as *mut ModuleCapiContext).cast(),
            )
        };
        if status != 0 {
            let message = module_ctx
                .last_error
                .as_deref()
                .map(|text| format!(": {text}"))
                .unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "extension '{}' initializer '{}' failed with status {}{}",
                module_name, symbol, status, message
            )));
        }
        if let Some(message) = module_ctx.last_error.as_deref() {
            return Err(RuntimeError::new(format!(
                "extension '{}' initializer '{}' reported error despite success: {}",
                module_name, symbol, message
            )));
        }

        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new(format!(
                "module '{}' invalid after extension init",
                module_name
            )));
        };
        module_data.globals.insert(
            "__pyrs_extension_library__".to_string(),
            Value::Str(library_path.to_string_lossy().to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_symbol__".to_string(),
            Value::Str(symbol.to_string()),
        );
        self.extension_libraries.push(handle);
        Ok(())
    }

    pub(super) fn exec_extension_module(
        &mut self,
        module: &ObjRef,
        name: &str,
        source_path: &Path,
    ) -> Result<(), RuntimeError> {
        let (abi_tag, entrypoint_name, plan) = if source_path
            .to_string_lossy()
            .ends_with(PYRS_EXTENSION_MANIFEST_SUFFIX)
        {
            let manifest =
                parse_extension_manifest(source_path, name).map_err(RuntimeError::new)?;
            let entrypoint_name = manifest.entrypoint.as_str();
            let plan = match manifest.entrypoint {
                ExtensionEntrypoint::HelloExt => ExtensionExecutionPlan::HelloExt,
                ExtensionEntrypoint::DynamicSymbol(ref symbol) => {
                    let library_path =
                        manifest.resolve_library_path(source_path).ok_or_else(|| {
                            RuntimeError::new(format!(
                                "extension manifest '{}' missing dynamic library path",
                                source_path.display()
                            ))
                        })?;
                    ExtensionExecutionPlan::Dynamic {
                        library_path,
                        symbol: symbol.clone(),
                    }
                }
            };
            (manifest.abi_tag, entrypoint_name, plan)
        } else if path_is_shared_library(source_path) {
            (
                PYRS_EXTENSION_ABI_TAG.to_string(),
                format!("dynamic:{PYRS_DYNAMIC_INIT_SYMBOL_V1}"),
                ExtensionExecutionPlan::Dynamic {
                    library_path: source_path.to_path_buf(),
                    symbol: PYRS_DYNAMIC_INIT_SYMBOL_V1.to_string(),
                },
            )
        } else {
            return Err(RuntimeError::new(format!(
                "unsupported extension module source '{}'",
                source_path.display()
            )));
        };

        self.set_extension_metadata(module, &abi_tag, &entrypoint_name, source_path)?;

        match plan {
            ExtensionExecutionPlan::HelloExt => {
                let Object::Module(module_data) = &mut *module.kind_mut() else {
                    return Err(RuntimeError::new(format!(
                        "module '{}' extension load target is invalid",
                        name
                    )));
                };
                module_data
                    .globals
                    .insert("EXTENSION_LOADED".to_string(), Value::Bool(true));
                module_data.globals.insert(
                    "ENTRYPOINT".to_string(),
                    Value::Str("hello_ext".to_string()),
                );
                module_data.globals.insert(
                    "MESSAGE".to_string(),
                    Value::Str("hello from hello_ext".to_string()),
                );
                Ok(())
            }
            ExtensionExecutionPlan::Dynamic {
                library_path,
                symbol,
            } => self.execute_dynamic_extension(module, name, &library_path, &symbol),
        }
    }
}
