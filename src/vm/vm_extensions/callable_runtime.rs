use std::collections::HashMap;
use std::ffi::CString;

use crate::vm::ExtensionCallableEntry;

use super::cpython_context_runtime::ActiveCpythonContextGuard;
use super::{
    BoundMethod, CpythonMethodDef, ExtensionCallableKind, ModuleCapiContext, NativeCallResult,
    NativeMethodKind, NativeMethodObject, ObjRef, Object, PyrsApiV1, PyrsObjectHandle,
    RuntimeError, Value, Vm, cpython_invoke_method_from_values,
};

impl Vm {
    pub(in crate::vm) fn register_extension_callable(
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
            ExtensionCallableEntry {
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

    pub(in crate::vm) fn call_extension_callable(
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
                let mut arg_handles = Vec::with_capacity(args.len());
                for arg in args {
                    arg_handles.push(call_ctx.alloc_object(arg));
                }
                // SAFETY: callback pointer comes from extension registration and the API/context
                // pointers remain valid for the duration of this call.
                unsafe {
                    callback(
                        &api as *const PyrsApiV1,
                        (std::ptr::addr_of_mut!(call_ctx)).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
            ExtensionCallableKind::WithKeywords(callback) => {
                let mut arg_handles = Vec::with_capacity(args.len());
                for arg in args {
                    arg_handles.push(call_ctx.alloc_object(arg));
                }
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
                        (std::ptr::addr_of_mut!(call_ctx)).cast(),
                        arg_handles.len(),
                        arg_handles.as_ptr(),
                        kw_name_ptrs.len(),
                        kw_name_ptrs.as_ptr(),
                        kw_value_handles.as_ptr(),
                        &mut result_handle as *mut PyrsObjectHandle,
                    )
                }
            }
            ExtensionCallableKind::CpythonMethod { method_def } => {
                if std::env::var_os("PYRS_TRACE_CPY_EXT_CALL").is_some() {
                    let module_name = match &*entry.module.kind() {
                        Object::Module(module_data) => module_data.name.clone(),
                        _ => "<extension>".to_string(),
                    };
                    eprintln!(
                        "[cpy-ext-call] id={} module={} name={} method_def={:p} args_len={} kwargs_len={}",
                        function_id,
                        module_name,
                        entry.name,
                        method_def as *mut CpythonMethodDef,
                        args.len(),
                        kwargs.len()
                    );
                }
                if std::env::var_os("PYRS_TRACE_COPYTO_CALL").is_some() && entry.name == "copyto" {
                    let module_name = match &*entry.module.kind() {
                        Object::Module(module_data) => module_data.name.clone(),
                        _ => "<extension>".to_string(),
                    };
                    eprintln!(
                        "[copyto-entry] function_id={} module={} method_def={:p}",
                        function_id, module_name, method_def as *mut CpythonMethodDef
                    );
                }
                let _active_context_guard =
                    ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
                let self_obj =
                    call_ctx.alloc_cpython_ptr_for_value(Value::Module(entry.module.clone()));
                let result_ptr = cpython_invoke_method_from_values(
                    &mut call_ctx,
                    method_def as *mut CpythonMethodDef,
                    self_obj,
                    std::ptr::null_mut(),
                    args,
                    kwargs,
                );
                if result_ptr.is_null() {
                    -1
                } else if let Some(result_value) = call_ctx.cpython_value_from_owned_ptr(result_ptr)
                {
                    result_handle = call_ctx.alloc_object(result_value);
                    0
                } else {
                    call_ctx.set_error("CPython method call returned unknown object pointer");
                    -1
                }
            }
        };
        if status != 0 {
            if let Some(detail) = call_ctx.last_error.clone() {
                return Err(RuntimeError::new(detail));
            }
            return Err(RuntimeError::new(format!(
                "RuntimeError: extension function '{}.{}' failed with status {}",
                match &*entry.module.kind() {
                    Object::Module(module_data) => module_data.name.clone(),
                    _ => "<extension>".to_string(),
                },
                entry.name,
                status
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
}
