//! Runtime bridge for CPython proxy objects and type-backed operations.
//!
//! Proxy helpers here map pyrs values to raw CPython pointers and route selected
//! operations through CPython slots/call conventions when a proxy boundary is hit.

use crate::runtime::format_repr;
use std::collections::HashMap;
use std::ffi::{CString, c_void};

use super::cpython_context_runtime::ActiveCpythonContextGuard;
use super::{
    CPY_PROXY_PTR_ATTR, CpythonNumberMethods, CpythonObjectHead, CpythonTypeObject,
    ModuleCapiContext, ObjRef, Object, ProxyAttrLookupReentryGuard, PyErr_GivenExceptionMatches,
    PyExc_IndexError, PyExc_TypeError, PyNumber_Add, PyNumber_Float, PyNumber_Invert,
    PyNumber_Long, PyNumber_MatrixMultiply, PyNumber_Multiply, PyNumber_Negative,
    PyNumber_Positive, PyNumber_Subtract, PyNumber_TrueDivide, PyObject_CallObject,
    PyObject_GetAttrString, PyObject_GetItem, PyObject_IsTrue, PyObject_Repr, PyObject_RichCompare,
    PyObject_RichCompareBool, PyObject_SetItem, PyObject_Size, PyObject_Str, RuntimeError, Value,
    Vm, c_name_to_string, cpython_is_type_object_ptr, cpython_resolve_vectorcall,
    cpython_type_tp_getattro, cpython_valid_type_ptr, cpython_value_debug_tag,
    is_cpython_proxy_class,
};
const PY_TPFLAGS_DISALLOW_INSTANTIATION: i64 = 1 << 7;

impl Vm {
    fn cpython_proxy_text_from_unicode_result_ptr(
        call_ctx: &ModuleCapiContext,
        result_ptr: *mut c_void,
    ) -> Option<String> {
        if result_ptr.is_null() {
            return None;
        }
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        let raw = result_ptr as usize;
        if raw < MIN_VALID_PTR || raw % std::mem::align_of::<CpythonObjectHead>() != 0 {
            return None;
        }
        // SAFETY: guarded by null/address/alignment checks above.
        let type_ptr = unsafe {
            result_ptr
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if !cpython_valid_type_ptr(type_ptr) {
            return None;
        }
        // SAFETY: `type_ptr` was validated by `cpython_valid_type_ptr`.
        let type_name = unsafe { c_name_to_string((*type_ptr).tp_name).ok() }?;
        if type_name != "str" {
            return None;
        }
        call_ctx.unicode_text_from_raw_storage(result_ptr)
    }

    pub(in crate::vm) fn cpython_proxy_raw_ptr_is_callable(raw_ptr: *mut c_void) -> bool {
        if raw_ptr.is_null() {
            return false;
        }
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        if (raw_ptr as usize) < MIN_VALID_PTR
            || (raw_ptr as usize) % std::mem::align_of::<usize>() != 0
        {
            return false;
        }
        // SAFETY: guarded by pointer/null/alignment checks.
        let type_ptr = unsafe {
            raw_ptr
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if !cpython_valid_type_ptr(type_ptr) {
            return false;
        }
        // SAFETY: `type_ptr` was validated by `cpython_valid_type_ptr`.
        if unsafe { !(*type_ptr).tp_call.is_null() } {
            return true;
        }
        // SAFETY: only inspects vectorcall metadata on candidate CPython object.
        unsafe { cpython_resolve_vectorcall(raw_ptr).is_some() }
    }

    fn cpython_proxy_class_repr_text(class_data: &crate::runtime::ClassObject) -> String {
        let qualname = class_data
            .attrs
            .get("__qualname__")
            .or_else(|| class_data.attrs.get("__name__"))
            .and_then(|value| match value {
                Value::Str(text) => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| class_data.name.clone());
        let module = class_data
            .attrs
            .get("__module__")
            .and_then(|value| match value {
                Value::Str(text) if !text.is_empty() => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "builtins".to_string());
        if module == "builtins" {
            format!("<class '{qualname}'>")
        } else {
            format!("<class '{module}.{qualname}'>")
        }
    }

    /// Extract the raw CPython pointer carried by a proxy class/instance value.
    pub(in crate::vm) fn cpython_proxy_raw_ptr_from_value(value: &Value) -> Option<*mut c_void> {
        match value {
            Value::Class(class_obj) => {
                let Object::Class(class_data) = &*class_obj.kind() else {
                    return None;
                };
                if !is_cpython_proxy_class(class_data) {
                    return None;
                }
                match class_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                    Some(Value::Int(raw)) if *raw >= 0 => Some(*raw as usize as *mut c_void),
                    _ => None,
                }
            }
            Value::Instance(instance_obj) => {
                let Object::Instance(instance_data) = &*instance_obj.kind() else {
                    return None;
                };
                match instance_data.attrs.get(CPY_PROXY_PTR_ATTR) {
                    Some(Value::Int(raw)) if *raw >= 0 => Some(*raw as usize as *mut c_void),
                    _ => {
                        let Object::Class(class_data) = &*instance_data.class.kind() else {
                            return None;
                        };
                        if !is_cpython_proxy_class(class_data) {
                            return None;
                        }
                        None
                    }
                }
            }
            _ => None,
        }
    }

    /// Call a CPython proxy object or proxy type constructor.
    ///
    /// Executes through a temporary `ModuleCapiContext` so pointer ownership and
    /// error-state translation remain consistent with normal C-API call paths.
    pub(in crate::vm) fn call_cpython_proxy_object(
        &mut self,
        proxy_value: &Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if let Value::Class(class) = proxy_value {
            let proxy_disallow = self
                .cpython_proxy_type_flags(class)
                .map(|flags| (flags & PY_TPFLAGS_DISALLOW_INSTANTIATION) != 0)
                .unwrap_or(false);
            if proxy_disallow {
                let (module_name, class_name) = match &*class.kind() {
                    Object::Class(class_data) => {
                        let module_name = match class_data.attrs.get("__module__") {
                            Some(Value::Str(name)) => name.clone(),
                            _ => "builtins".to_string(),
                        };
                        (module_name, class_data.name.clone())
                    }
                    _ => ("builtins".to_string(), "type".to_string()),
                };
                let qualified_name = if module_name == "builtins" {
                    class_name
                } else {
                    format!("{module_name}.{class_name}")
                };
                return Err(RuntimeError::type_error(format!(
                    "cannot create '{}' instances",
                    qualified_name
                )));
            }
        }
        let Some(raw_ptr) = Self::cpython_proxy_raw_ptr_from_value(proxy_value) else {
            return Err(RuntimeError::new(
                "internal error: proxy call target missing raw pointer",
            ));
        };
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let result_ptr = call_ctx
            .try_native_tp_call(raw_ptr, &args, &kwargs)
            .unwrap_or(std::ptr::null_mut());
        if result_ptr.is_null() {
            if let Some(err) = call_ctx.runtime_error_from_current_error_state("proxy call failed")
            {
                return Err(err);
            }
            if std::env::var_os("PYRS_TRACE_PROXY_CALL_FAIL").is_some() {
                const MIN_VALID_PTR: usize = 0x1_0000_0000;
                let valid_object_ptr = (raw_ptr as usize) >= MIN_VALID_PTR
                    && (raw_ptr as usize) % std::mem::align_of::<usize>() == 0;
                let (type_ptr, type_name, tp_call, tp_vectorcall_offset) = if valid_object_ptr {
                    // SAFETY: guarded by non-null + minimum-address + alignment checks.
                    unsafe {
                        let head = raw_ptr.cast::<CpythonObjectHead>();
                        let type_ptr = head
                            .as_ref()
                            .map(|h| h.ob_type.cast::<CpythonTypeObject>())
                            .unwrap_or(std::ptr::null_mut());
                        let valid_type_ptr = (type_ptr as usize) >= MIN_VALID_PTR
                            && (type_ptr as usize) % std::mem::align_of::<usize>() == 0;
                        if !valid_type_ptr {
                            (
                                type_ptr,
                                "<invalid-type-ptr>".to_string(),
                                std::ptr::null_mut(),
                                0isize,
                            )
                        } else {
                            (
                                type_ptr,
                                c_name_to_string((*type_ptr).tp_name)
                                    .unwrap_or_else(|_| "<invalid>".to_string()),
                                (*type_ptr).tp_call,
                                (*type_ptr).tp_vectorcall_offset,
                            )
                        }
                    }
                } else {
                    (
                        std::ptr::null_mut(),
                        "<invalid-object-ptr>".to_string(),
                        std::ptr::null_mut(),
                        0isize,
                    )
                };
                let owns_ptr = call_ctx.owns_cpython_allocation_ptr(raw_ptr);
                let mapped_tag = call_ctx
                    .cpython_value_from_ptr(raw_ptr)
                    .map(|value| {
                        let base_tag = cpython_value_debug_tag(&value);
                        match value {
                            Value::Instance(instance_obj) => match &*instance_obj.kind() {
                                Object::Instance(instance_data) => match &*instance_data
                                    .class
                                    .kind()
                                {
                                    Object::Class(class_data) => {
                                        let is_proxy = is_cpython_proxy_class(class_data);
                                        format!("{base_tag}({} proxy={is_proxy})", class_data.name)
                                    }
                                    _ => base_tag,
                                },
                                _ => base_tag,
                            },
                            Value::Class(class_obj) => match &*class_obj.kind() {
                                Object::Class(class_data) => {
                                    let is_proxy = is_cpython_proxy_class(class_data);
                                    format!("{base_tag}({} proxy={is_proxy})", class_data.name)
                                }
                                _ => base_tag,
                            },
                            _ => base_tag,
                        }
                    })
                    .unwrap_or_else(|| "<none>".to_string());
                let mapped_expected_type_ptr = call_ctx
                    .cpython_value_from_ptr(raw_ptr)
                    .and_then(|value| match value {
                        Value::Instance(instance_obj) => match &*instance_obj.kind() {
                            Object::Instance(instance_data) => {
                                ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&Value::Class(
                                    instance_data.class.clone(),
                                ))
                            }
                            _ => None,
                        },
                        Value::Class(class_obj) => {
                            ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&Value::Class(
                                class_obj,
                            ))
                        }
                        _ => None,
                    })
                    .unwrap_or(std::ptr::null_mut());
                eprintln!(
                    "[cpy-proxy-call-fail] proxy={} ptr={:p} type={:p} type_name={} tp_call={:p} tp_vectorcall_offset={} args={} kwargs={} owns_ptr={} mapped={} mapped_expected_type={:p}",
                    cpython_value_debug_tag(proxy_value),
                    raw_ptr,
                    type_ptr,
                    type_name,
                    tp_call,
                    tp_vectorcall_offset,
                    args.len(),
                    kwargs.len(),
                    owns_ptr,
                    mapped_tag,
                    mapped_expected_type_ptr
                );
            }
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy object is not callable".to_string());
            return Err(RuntimeError::new(detail));
        }
        let converted = call_ctx.cpython_value_from_owned_ptr(result_ptr);
        if converted.is_none() && std::env::var_os("PYRS_TRACE_CPY_UNKNOWN_PTR").is_some() {
            let mut type_ptr: *mut CpythonTypeObject = std::ptr::null_mut();
            let mut type_name = "<invalid-ptr>".to_string();
            const MIN_VALID_PTR: usize = 0x1_0000_0000;
            let result_addr = result_ptr as usize;
            if result_addr >= MIN_VALID_PTR
                && result_addr % std::mem::align_of::<CpythonObjectHead>() == 0
            {
                // SAFETY: pointer shape checked above; this remains best-effort diagnostics only.
                unsafe {
                    if let Some(head) = result_ptr.cast::<CpythonObjectHead>().as_ref() {
                        type_ptr = head.ob_type.cast::<CpythonTypeObject>();
                        let type_addr = type_ptr as usize;
                        if !type_ptr.is_null()
                            && type_addr >= MIN_VALID_PTR
                            && type_addr % std::mem::align_of::<CpythonTypeObject>() == 0
                            && !(*type_ptr).tp_name.is_null()
                        {
                            type_name = c_name_to_string((*type_ptr).tp_name)
                                .unwrap_or_else(|_| "<invalid-type-name>".to_string());
                        } else if !type_ptr.is_null() {
                            type_name = "<invalid-type-ptr>".to_string();
                        } else {
                            type_name = "<null-type>".to_string();
                        }
                    }
                }
            }
            eprintln!(
                "[cpy-proxy-unknown-result] proxy={} result_ptr={:p} type_ptr={:p} type_name={}",
                cpython_value_debug_tag(proxy_value),
                result_ptr,
                type_ptr,
                type_name
            );
        }
        converted.ok_or_else(|| RuntimeError::new("proxy call returned unknown object pointer"))
    }

    pub(in crate::vm) fn cpython_proxy_get_iter(
        &mut self,
        proxy_value: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(proxy_value)?;
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        if (raw_ptr as usize) < MIN_VALID_PTR
            || (raw_ptr as usize) % std::mem::align_of::<usize>() != 0
        {
            return Some(Err(RuntimeError::new(
                "proxy iterator received invalid object pointer",
            )));
        }
        if !ModuleCapiContext::is_probable_external_cpython_object_ptr(raw_ptr) {
            return Some(Err(RuntimeError::type_error("object is not iterable")));
        }
        if self.capi_ptr_is_owned_compat(raw_ptr as usize) {
            return Some(Err(RuntimeError::type_error("object is not iterable")));
        }
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        // SAFETY: raw pointer validated above and points to a CPython-compatible object header.
        let type_ptr = unsafe {
            raw_ptr
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        let iter_ptr = if type_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: `type_ptr` was derived from the validated object header above.
            let tp_iter_raw = unsafe { (*type_ptr).tp_iter };
            if tp_iter_raw.is_null() {
                std::ptr::null_mut()
            } else {
                let tp_iter: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
                    // SAFETY: `tp_iter` follows CPython unary slot ABI.
                    unsafe { std::mem::transmute(tp_iter_raw) };
                // SAFETY: calling external type's iterator slot with its object pointer.
                unsafe { tp_iter(raw_ptr) }
            }
        };
        if iter_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "object is not iterable".to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(
            call_ctx
                .cpython_value_from_owned_ptr(iter_ptr)
                .ok_or_else(|| RuntimeError::new("proxy iterator returned unknown object pointer"))
                .and_then(|iter_value| {
                    if Vm::cpython_proxy_raw_ptr_from_value(&iter_value)
                        .is_some_and(|iter_ptr| iter_ptr == raw_ptr)
                    {
                        Err(RuntimeError::new("proxy iterator recursion detected"))
                    } else {
                        Ok(iter_value)
                    }
                }),
        )
    }

    pub(in crate::vm) fn cpython_proxy_has_iternext(proxy_value: &Value) -> Option<bool> {
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(proxy_value)?;
        const MIN_VALID_PTR: usize = 0x1_0000_0000;
        if (raw_ptr as usize) < MIN_VALID_PTR
            || (raw_ptr as usize) % std::mem::align_of::<usize>() != 0
        {
            return Some(false);
        }
        // SAFETY: pointer validity checked above for non-null/min-address/alignment.
        let type_ptr = unsafe {
            raw_ptr
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null() {
            return Some(false);
        }
        // SAFETY: `type_ptr` was derived from validated object header above.
        Some(unsafe { !(*type_ptr).tp_iternext.is_null() })
    }

    fn cpython_proxy_unary_numeric_op(
        &mut self,
        value: &Value,
        op: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        failure_label: &str,
        unknown_ptr_label: &str,
    ) -> Option<Result<Value, RuntimeError>> {
        Self::cpython_proxy_raw_ptr_from_value(value)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let value_ptr = call_ctx.alloc_cpython_ptr_for_value(value.clone());
        let result_ptr = if value_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: pointer was materialized in the active C-API context above.
            unsafe { op(value_ptr) }
        };
        if value_ptr.is_null() {
            return Some(Err(RuntimeError::new(format!(
                "{failure_label} to materialize operand",
            ))));
        }
        if result_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| failure_label.to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(
            call_ctx
                .cpython_value_from_owned_ptr(result_ptr)
                .ok_or_else(|| RuntimeError::new(unknown_ptr_label)),
        )
    }

    fn cpython_proxy_binary_numeric_op(
        &mut self,
        left: &Value,
        right: &Value,
        op: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
        failure_label: &str,
        unknown_ptr_label: &str,
    ) -> Option<Result<Value, RuntimeError>> {
        if Self::cpython_proxy_raw_ptr_from_value(left).is_none()
            && Self::cpython_proxy_raw_ptr_from_value(right).is_none()
        {
            return None;
        }
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let left_ptr = call_ctx.alloc_cpython_ptr_for_value(left.clone());
        let right_ptr = call_ctx.alloc_cpython_ptr_for_value(right.clone());
        let result_ptr = if left_ptr.is_null() || right_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: pointers were materialized in the active C-API context above.
            unsafe { op(left_ptr, right_ptr) }
        };
        if left_ptr.is_null() || right_ptr.is_null() {
            return Some(Err(RuntimeError::new(format!(
                "{failure_label} to materialize operands"
            ))));
        }
        if result_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| failure_label.to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(
            call_ctx
                .cpython_value_from_owned_ptr(result_ptr)
                .ok_or_else(|| RuntimeError::new(unknown_ptr_label)),
        )
    }

    pub(in crate::vm) fn cpython_proxy_subtract(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_binary_numeric_op(
            left,
            right,
            PyNumber_Subtract,
            "proxy subtraction failed",
            "proxy subtraction returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_long(
        &mut self,
        value: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        thread_local! {
            static CPYTHON_PROXY_LONG_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        let already_active = CPYTHON_PROXY_LONG_ACTIVE.with(|active| {
            if active.get() {
                true
            } else {
                active.set(true);
                false
            }
        });
        if already_active {
            return Some(Err(RuntimeError::type_error("int() unsupported type")));
        }
        struct CpythonProxyLongGuard;
        impl Drop for CpythonProxyLongGuard {
            fn drop(&mut self) {
                CPYTHON_PROXY_LONG_ACTIVE.with(|active| active.set(false));
            }
        }
        let _reentry_guard = CpythonProxyLongGuard;
        let raw_ptr = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(value)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let result = if raw_ptr.is_null() {
            Err(RuntimeError::type_error("int() unsupported type"))
        } else {
            // SAFETY: `raw_ptr` is a candidate CPython object pointer.
            let type_ptr = unsafe {
                raw_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if !cpython_valid_type_ptr(type_ptr) {
                Err(RuntimeError::type_error("int() unsupported type"))
            } else {
                // SAFETY: `type_ptr` is validated above.
                let number_methods = unsafe {
                    (*type_ptr)
                        .tp_as_number
                        .cast::<CpythonNumberMethods>()
                        .as_ref()
                };
                let converter =
                    number_methods.and_then(|methods| methods.nb_int.or(methods.nb_index));
                if let Some(converter) = converter {
                    if (converter as *const () as usize) == (PyNumber_Long as *const () as usize) {
                        Err(RuntimeError::type_error("int() unsupported type"))
                    } else {
                        // SAFETY: converter slot comes from validated number methods table.
                        let result_ptr = unsafe { converter(raw_ptr) };
                        if result_ptr.is_null() {
                            Err(RuntimeError::new(
                                call_ctx
                                    .last_error
                                    .clone()
                                    .unwrap_or_else(|| "int() unsupported type".to_string()),
                            ))
                        } else {
                            call_ctx
                                .cpython_value_from_owned_ptr(result_ptr)
                                .ok_or_else(|| {
                                    RuntimeError::new(
                                        "proxy int conversion returned unknown object pointer",
                                    )
                                })
                        }
                    }
                } else {
                    Err(RuntimeError::type_error("int() unsupported type"))
                }
            }
        };
        Some(result)
    }

    pub(in crate::vm) fn cpython_proxy_float(
        &mut self,
        value: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        thread_local! {
            static CPYTHON_PROXY_FLOAT_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        let already_active = CPYTHON_PROXY_FLOAT_ACTIVE.with(|active| {
            if active.get() {
                true
            } else {
                active.set(true);
                false
            }
        });
        if already_active {
            return Some(Err(RuntimeError::type_error("float() unsupported type")));
        }
        struct CpythonProxyFloatGuard;
        impl Drop for CpythonProxyFloatGuard {
            fn drop(&mut self) {
                CPYTHON_PROXY_FLOAT_ACTIVE.with(|active| active.set(false));
            }
        }
        let _reentry_guard = CpythonProxyFloatGuard;
        let raw_ptr = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(value)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let result = if raw_ptr.is_null() {
            Err(RuntimeError::type_error("float() unsupported type"))
        } else {
            // SAFETY: `raw_ptr` is a candidate CPython object pointer.
            let type_ptr = unsafe {
                raw_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut())
            };
            if !cpython_valid_type_ptr(type_ptr) {
                Err(RuntimeError::type_error("float() unsupported type"))
            } else {
                // SAFETY: `type_ptr` is validated above.
                let number_methods = unsafe {
                    (*type_ptr)
                        .tp_as_number
                        .cast::<CpythonNumberMethods>()
                        .as_ref()
                };
                let converter = number_methods
                    .and_then(|methods| (!methods.nb_float.is_null()).then_some(methods.nb_float));
                if let Some(converter) = converter {
                    if (converter as *const ()) == (PyNumber_Float as *const ()) {
                        Err(RuntimeError::type_error("float() unsupported type"))
                    } else {
                        // SAFETY: converter slot comes from validated number methods table.
                        let converter: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
                            unsafe { std::mem::transmute(converter) };
                        // SAFETY: slot signature matches CPython nb_float ABI.
                        let result_ptr = unsafe { converter(raw_ptr) };
                        if result_ptr.is_null() {
                            Err(RuntimeError::new(
                                call_ctx
                                    .last_error
                                    .clone()
                                    .unwrap_or_else(|| "float() unsupported type".to_string()),
                            ))
                        } else {
                            match call_ctx.cpython_value_from_owned_ptr(result_ptr) {
                                Some(Value::Float(value)) => Ok(Value::Float(value)),
                                Some(Value::Int(value)) => Ok(Value::Float(value as f64)),
                                Some(Value::Bool(flag)) => {
                                    Ok(Value::Float(if flag { 1.0 } else { 0.0 }))
                                }
                                Some(Value::BigInt(value)) => Ok(Value::Float(value.to_f64())),
                                Some(_) | None => {
                                    Err(RuntimeError::new("__float__ returned non-float"))
                                }
                            }
                        }
                    }
                } else {
                    Err(RuntimeError::type_error("float() unsupported type"))
                }
            }
        };
        Some(result)
    }

    pub(in crate::vm) fn cpython_proxy_negative(
        &mut self,
        value: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_unary_numeric_op(
            value,
            PyNumber_Negative,
            "proxy unary negative failed",
            "proxy unary negative returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_positive(
        &mut self,
        value: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_unary_numeric_op(
            value,
            PyNumber_Positive,
            "proxy unary positive failed",
            "proxy unary positive returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_invert(
        &mut self,
        value: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_unary_numeric_op(
            value,
            PyNumber_Invert,
            "proxy unary invert failed",
            "proxy unary invert returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_add(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_binary_numeric_op(
            left,
            right,
            PyNumber_Add,
            "proxy addition failed",
            "proxy addition returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_multiply(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_binary_numeric_op(
            left,
            right,
            PyNumber_Multiply,
            "proxy multiply failed",
            "proxy multiply returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_matmul(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_binary_numeric_op(
            left,
            right,
            PyNumber_MatrixMultiply,
            "proxy matrix multiply failed",
            "proxy matrix multiply returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_true_divide(
        &mut self,
        left: &Value,
        right: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        self.cpython_proxy_binary_numeric_op(
            left,
            right,
            PyNumber_TrueDivide,
            "proxy true divide failed",
            "proxy true divide returned unknown object pointer",
        )
    }

    pub(in crate::vm) fn cpython_proxy_set_item(
        &mut self,
        target: &Value,
        key: Value,
        value: Value,
    ) -> Option<Result<(), RuntimeError>> {
        Self::cpython_proxy_raw_ptr_from_value(target)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let target_ptr = call_ctx.alloc_cpython_ptr_for_value(target.clone());
        let key_ptr = call_ctx.alloc_cpython_ptr_for_value(key);
        let value_ptr = call_ctx.alloc_cpython_ptr_for_value(value);
        let status = if target_ptr.is_null() || key_ptr.is_null() || value_ptr.is_null() {
            -1
        } else {
            // SAFETY: pointers were materialized in the active C-API context above.
            unsafe { PyObject_SetItem(target_ptr, key_ptr, value_ptr) }
        };
        if target_ptr.is_null() || key_ptr.is_null() || value_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy setitem failed to materialize operands",
            )));
        }
        if status == 0 {
            return Some(Ok(()));
        }
        let detail = call_ctx
            .last_error
            .clone()
            .unwrap_or_else(|| "proxy setitem failed".to_string());
        Some(Err(RuntimeError::new(detail)))
    }

    pub(in crate::vm) fn cpython_proxy_str(
        &mut self,
        target: &Value,
    ) -> Option<Result<String, RuntimeError>> {
        if let Value::Class(class_obj) = target
            && let Object::Class(class_data) = &*class_obj.kind()
            && is_cpython_proxy_class(class_data)
        {
            return Some(Ok(Self::cpython_proxy_class_repr_text(class_data)));
        }
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(target)?;
        let _guard = ProxyAttrLookupReentryGuard::enter(raw_ptr as usize, "__str__", false)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        // Use the already-resolved external PyObject* for proxy values directly.
        let target_ptr = raw_ptr;
        let result_ptr = if target_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: target pointer is a live external PyObject*.
            unsafe { PyObject_Str(target_ptr) }
        };
        if target_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy str() failed to materialize operand",
            )));
        }
        if result_ptr.is_null() {
            return Some(Err(RuntimeError::new("proxy str() failed")));
        }
        Some(match call_ctx.cpython_value_from_owned_ptr(result_ptr) {
            Some(Value::Str(text)) => Ok(text),
            Some(_) => Self::cpython_proxy_text_from_unicode_result_ptr(&call_ctx, result_ptr)
                .ok_or_else(|| RuntimeError::new("proxy str() returned non-string")),
            None => Self::cpython_proxy_text_from_unicode_result_ptr(&call_ctx, result_ptr)
                .ok_or_else(|| RuntimeError::new("proxy str() returned unknown object pointer")),
        })
    }

    pub(in crate::vm) fn cpython_proxy_repr(
        &mut self,
        target: &Value,
    ) -> Option<Result<String, RuntimeError>> {
        if let Value::Class(class_obj) = target
            && let Object::Class(class_data) = &*class_obj.kind()
            && is_cpython_proxy_class(class_data)
        {
            return Some(Ok(Self::cpython_proxy_class_repr_text(class_data)));
        }
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(target)?;
        let _guard = ProxyAttrLookupReentryGuard::enter(raw_ptr as usize, "__repr__", false)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        // Use the already-resolved external PyObject* for proxy values directly.
        let target_ptr = raw_ptr;
        let result_ptr = if target_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: target pointer is a live external PyObject*.
            unsafe { PyObject_Repr(target_ptr) }
        };
        if target_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy repr() failed to materialize operand",
            )));
        }
        if result_ptr.is_null() {
            return Some(Err(RuntimeError::new("proxy repr() failed")));
        }
        Some(match call_ctx.cpython_value_from_owned_ptr(result_ptr) {
            Some(Value::Str(text)) => Ok(text),
            Some(_) => Self::cpython_proxy_text_from_unicode_result_ptr(&call_ctx, result_ptr)
                .ok_or_else(|| RuntimeError::new("proxy repr() returned non-string")),
            None => Self::cpython_proxy_text_from_unicode_result_ptr(&call_ctx, result_ptr)
                .ok_or_else(|| RuntimeError::new("proxy repr() returned unknown object pointer")),
        })
    }

    pub(in crate::vm) fn cpython_proxy_format(
        &mut self,
        target: &Value,
        spec: &str,
    ) -> Option<Result<String, RuntimeError>> {
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(target)?;
        let _guard = ProxyAttrLookupReentryGuard::enter(raw_ptr as usize, "__format__", false)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let target_ptr = call_ctx.alloc_cpython_ptr_for_value(target.clone());
        let c_format = CString::new("__format__").ok()?;
        let format_method_ptr = if target_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: pointer was materialized in the active C-API context above.
            unsafe { PyObject_GetAttrString(target_ptr, c_format.as_ptr()) }
        };
        let spec_args_ptr = if call_ctx.vm.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            let vm = unsafe { &mut *call_ctx.vm };
            let spec_args = vm.heap.alloc_tuple(vec![Value::Str(spec.to_string())]);
            call_ctx.alloc_cpython_ptr_for_value(spec_args)
        };
        let result_ptr = if format_method_ptr.is_null() || spec_args_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: pointers were materialized in the active C-API context above.
            unsafe { PyObject_CallObject(format_method_ptr, spec_args_ptr) }
        };
        if target_ptr.is_null() || format_method_ptr.is_null() || spec_args_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy format() failed to materialize operands",
            )));
        }
        if result_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy format() failed".to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(match call_ctx.cpython_value_from_owned_ptr(result_ptr) {
            Some(Value::Str(text)) => Ok(text),
            Some(_) => Err(RuntimeError::new("proxy format() returned non-string")),
            None => Err(RuntimeError::new(
                "proxy format() returned unknown object pointer",
            )),
        })
    }

    pub(in crate::vm) fn cpython_proxy_len(
        &mut self,
        target: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        Self::cpython_proxy_raw_ptr_from_value(target)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let target_ptr = call_ctx.alloc_cpython_ptr_for_value(target.clone());
        let size = if target_ptr.is_null() {
            -1
        } else {
            // SAFETY: pointer was materialized in the active C-API context above.
            unsafe { PyObject_Size(target_ptr) }
        };
        if target_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy len() failed to materialize operand",
            )));
        }
        if size < 0 {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy len() failed".to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(Ok(Value::Int(size as i64)))
    }

    pub(in crate::vm) fn cpython_proxy_get_item(
        &mut self,
        target: &Value,
        key: Value,
    ) -> Option<Result<Value, RuntimeError>> {
        let key_for_trace = key.clone();
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(target)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        if call_ctx.owns_cpython_allocation_ptr(raw_ptr) {
            return None;
        }
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let target_ptr = call_ctx.alloc_cpython_ptr_for_value(target.clone());
        let key_ptr = call_ctx.alloc_cpython_ptr_for_value(key);
        let result_ptr = if target_ptr.is_null() || key_ptr.is_null() {
            std::ptr::null_mut()
        } else {
            // SAFETY: pointers were materialized in the active C-API context above.
            unsafe { PyObject_GetItem(target_ptr, key_ptr) }
        };
        if target_ptr.is_null() || key_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy getitem failed to materialize operands",
            )));
        }
        if result_ptr.is_null() {
            let mut detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy getitem failed".to_string());
            if std::env::var_os("PYRS_TRACE_PROXY_GETITEM").is_some() {
                eprintln!(
                    "[proxy-getitem] target={} key={} raw_ptr={:p} error={}",
                    format_repr(target),
                    format_repr(&key_for_trace),
                    raw_ptr,
                    detail
                );
            }
            let is_index_error = call_ctx.current_error.as_ref().is_some_and(|state| {
                if state.ptype.is_null() {
                    return false;
                }
                // SAFETY: ptype originates from the active C-API error state.
                unsafe { PyErr_GivenExceptionMatches(state.ptype, PyExc_IndexError) != 0 }
            });
            if is_index_error && !detail.contains("IndexError") {
                detail = format!("IndexError: {detail}");
            }
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(
            call_ctx
                .cpython_value_from_owned_ptr(result_ptr)
                .ok_or_else(|| RuntimeError::new("proxy getitem returned unknown object pointer")),
        )
    }

    pub(in crate::vm) fn cpython_proxy_richcmp_bool(
        &mut self,
        left: &Value,
        right: &Value,
        op: i32,
    ) -> Option<Result<bool, RuntimeError>> {
        if Self::cpython_proxy_raw_ptr_from_value(left).is_none()
            && Self::cpython_proxy_raw_ptr_from_value(right).is_none()
        {
            return None;
        }
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let left_ptr = call_ctx.alloc_cpython_ptr_for_value(left.clone());
        let right_ptr = call_ctx.alloc_cpython_ptr_for_value(right.clone());
        let result = if left_ptr.is_null() || right_ptr.is_null() {
            -1
        } else {
            // SAFETY: pointers were materialized in the active C-API context above.
            unsafe { PyObject_RichCompareBool(left_ptr, right_ptr, op) }
        };
        if left_ptr.is_null() || right_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy comparison failed to materialize operands",
            )));
        }
        if result < 0 {
            let type_error = call_ctx.current_error.as_ref().is_some_and(|state| {
                if state.ptype.is_null() {
                    return false;
                }
                // SAFETY: ptype originates from CPython error state and is a candidate exception.
                unsafe { PyErr_GivenExceptionMatches(state.ptype, PyExc_TypeError) != 0 }
            });
            if type_error {
                return Some(Ok(false));
            }
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy comparison failed".to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(Ok(result != 0))
    }

    pub(in crate::vm) fn cpython_proxy_richcmp_value(
        &mut self,
        left: &Value,
        right: &Value,
        op: i32,
    ) -> Option<Result<Value, RuntimeError>> {
        if Self::cpython_proxy_raw_ptr_from_value(left).is_none()
            && Self::cpython_proxy_raw_ptr_from_value(right).is_none()
        {
            return None;
        }
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let left_ptr = call_ctx.alloc_cpython_ptr_for_value(left.clone());
        let right_ptr = call_ctx.alloc_cpython_ptr_for_value(right.clone());
        if left_ptr.is_null() || right_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy comparison failed to materialize operands",
            )));
        }
        // SAFETY: pointers were materialized in the active C-API context above.
        let result_ptr = unsafe { PyObject_RichCompare(left_ptr, right_ptr, op) };
        if result_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy comparison failed".to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(
            call_ctx
                .cpython_value_from_owned_ptr(result_ptr)
                .ok_or_else(|| {
                    RuntimeError::new("proxy comparison returned unknown object pointer")
                }),
        )
    }

    pub(in crate::vm) fn cpython_proxy_truthy(
        &mut self,
        value: &Value,
    ) -> Option<Result<bool, RuntimeError>> {
        if Self::cpython_proxy_raw_ptr_from_value(value).is_none() {
            return None;
        }
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let value_ptr = call_ctx.alloc_cpython_ptr_for_value(value.clone());
        if value_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy truthiness failed to materialize operand",
            )));
        }
        // SAFETY: pointer was materialized in the active C-API context above.
        let result = unsafe { PyObject_IsTrue(value_ptr) };
        if result < 0 {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy truthiness failed".to_string());
            return Some(Err(RuntimeError::new(detail)));
        }
        Some(Ok(result != 0))
    }

    pub(in crate::vm) fn load_cpython_proxy_attr_for_value(
        &mut self,
        proxy_value: &Value,
        attr_name: &str,
    ) -> Option<Value> {
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(proxy_value)?;
        let is_proxy_type_object = matches!(proxy_value, Value::Class(_));
        let _reentry_guard = if matches!(attr_name, "__repr__" | "__str__") {
            Some(ProxyAttrLookupReentryGuard::enter(
                raw_ptr as usize,
                attr_name,
                is_proxy_type_object,
            )?)
        } else {
            None
        };
        let c_name = CString::new(attr_name).ok()?;
        let trace_proxy_attr = std::env::var_os("PYRS_TRACE_PROXY_ATTR").is_some()
            && matches!(
                attr_name,
                "base" | "identity" | "newbyteorder" | "__ge__" | "char"
            );
        let trace_proxy_max =
            attr_name == "max" && std::env::var_os("PYRS_TRACE_PROXY_MAX").is_some();
        let trace_type_attr =
            attr_name == "type" && std::env::var_os("PYRS_TRACE_PROXY_TYPE_ATTR").is_some();
        if trace_type_attr {
            let (raw_type, raw_type_name) = unsafe {
                let raw_type = raw_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type)
                    .unwrap_or(std::ptr::null_mut());
                let raw_type_name = raw_type
                    .cast::<CpythonTypeObject>()
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string());
                (raw_type, raw_type_name)
            };
            eprintln!(
                "[cpy-proxy-attr] lookup ptr={:p} type={:p} type_name={} attr={}",
                raw_ptr, raw_type, raw_type_name, attr_name
            );
        }
        if trace_proxy_max {
            eprintln!(
                "[proxy-max] start target={} raw_ptr={:p} is_type_object={}",
                cpython_value_debug_tag(proxy_value),
                raw_ptr,
                is_proxy_type_object
            );
        }
        if trace_proxy_attr {
            let (type_ptr, type_name, methods_ptr, getset_ptr, members_ptr, base_ptr) = unsafe {
                let type_ptr = raw_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type)
                    .unwrap_or(std::ptr::null_mut());
                let type_name = type_ptr
                    .cast::<CpythonTypeObject>()
                    .as_ref()
                    .and_then(|ty| c_name_to_string(ty.tp_name).ok())
                    .unwrap_or_else(|| "<unknown>".to_string());
                if is_proxy_type_object {
                    let type_obj = raw_ptr.cast::<CpythonTypeObject>();
                    (
                        type_ptr,
                        type_name,
                        (*type_obj).tp_methods,
                        (*type_obj).tp_getset,
                        (*type_obj).tp_members,
                        (*type_obj).tp_base,
                    )
                } else {
                    (
                        type_ptr,
                        type_name,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                }
            };
            eprintln!(
                "[proxy-attr] begin attr={} is_type_obj={} raw_ptr={:p} type_ptr={:p} type_name={} methods={:p} getset={:p} members={:p} base={:p}",
                attr_name,
                is_proxy_type_object,
                raw_ptr,
                type_ptr,
                type_name,
                methods_ptr,
                getset_ptr,
                members_ptr,
                base_ptr
            );
        }
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let slot_dunder_fastpath = matches!(
            attr_name,
            "__lt__"
                | "__le__"
                | "__eq__"
                | "__ne__"
                | "__gt__"
                | "__ge__"
                | "__repr__"
                | "__str__"
                | "__bool__"
                | "__int__"
                | "__float__"
                | "__index__"
                | "__getitem__"
                | "__len__"
                | "__iter__"
                | "__setitem__"
                | "__get__"
                | "__set__"
                | "__delete__"
                | "__init__"
        );
        if !is_proxy_type_object
            && (slot_dunder_fastpath
                || std::env::var_os("PYRS_ENABLE_PROXY_TP_DICT_FASTPATH").is_some())
            && let Some(attr_ptr) = call_ctx.lookup_type_attr_via_tp_dict(raw_ptr, attr_name)
            && !attr_ptr.is_null()
        {
            if std::env::var_os("PYRS_TRACE_PROXY_ATTR_CALL").is_some() {
                eprintln!(
                    "[proxy-attr-map] source=tp_dict target={:p} attr={} value_ptr={:p}",
                    raw_ptr, attr_name, attr_ptr
                );
            }
            let mapped = call_ctx.cpython_value_from_borrowed_ptr(attr_ptr);
            if mapped.is_none() && std::env::var_os("PYRS_TRACE_PROXY_ATTR_CALL").is_some() {
                let probable = ModuleCapiContext::is_probable_external_cpython_object_ptr(attr_ptr);
                let owned = call_ctx.owns_cpython_allocation_ptr(attr_ptr);
                let local_handle = call_ctx.cpython_handle_from_ptr(attr_ptr);
                let vm_cache = if call_ctx.vm.is_null() {
                    false
                } else {
                    // SAFETY: VM pointer is valid for active context lifetime.
                    let vm = unsafe { &*call_ctx.vm };
                    vm.extension_cpython_ptr_values
                        .contains_key(&(attr_ptr as usize))
                };
                // SAFETY: best-effort pointer diagnostics for unknown CPython value mapping.
                let (refcnt, type_ptr, type_refcnt) = unsafe {
                    let head = attr_ptr
                        .cast::<CpythonObjectHead>()
                        .as_ref()
                        .map(|head| (head.ob_refcnt, head.ob_type.cast::<CpythonObjectHead>()))
                        .unwrap_or((0, std::ptr::null_mut()));
                    let type_refcnt = head
                        .1
                        .as_ref()
                        .map(|type_head| type_head.ob_refcnt)
                        .unwrap_or(0);
                    (head.0, head.1, type_refcnt)
                };
                eprintln!(
                    "[proxy-attr-map] source=tp_dict_unmapped target={:p} attr={} value_ptr={:p} probable={} owned={} local_handle={local_handle:?} vm_cache={} refcnt={} type_ptr={:p} type_refcnt={}",
                    raw_ptr,
                    attr_name,
                    attr_ptr,
                    probable,
                    owned,
                    vm_cache,
                    refcnt,
                    type_ptr,
                    type_refcnt
                );
            }
            if trace_proxy_max {
                let mapped_tag = mapped
                    .as_ref()
                    .map(cpython_value_debug_tag)
                    .unwrap_or_else(|| "<none>".to_string());
                eprintln!(
                    "[proxy-max] tp_dict fastpath attr=max raw_ptr={:p} mapped={}",
                    raw_ptr, mapped_tag
                );
            }
            return mapped;
        }
        if is_proxy_type_object {
            let name_ptr = call_ctx.alloc_cpython_ptr_for_value(Value::Str(attr_name.to_string()));
            if name_ptr.is_null() {
                return None;
            }
            let _active_context_guard =
                ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
            let mut attr_ptr = unsafe { cpython_type_tp_getattro(raw_ptr, name_ptr) };
            if attr_ptr.is_null() {
                // Some extension metatypes resolve attributes correctly only through their
                // generic attribute path; fall back before treating this as a soft miss.
                call_ctx.clear_error();
                attr_ptr = unsafe { PyObject_GetAttrString(raw_ptr, c_name.as_ptr()) };
            }
            if attr_ptr.is_null() {
                call_ctx.clear_error();
                return None;
            }
            return call_ctx.cpython_value_from_owned_ptr(attr_ptr);
        }
        // Guard fallback `PyObject_GetAttrString` dispatch against same-target/same-attr
        // re-entry loops. `__repr__`/`__str__` already hold a guard above, so avoid
        // double-entering the same key in this path.
        let _fallback_reentry_guard = if _reentry_guard.is_some() {
            None
        } else {
            Some(ProxyAttrLookupReentryGuard::enter(
                raw_ptr as usize,
                attr_name,
                is_proxy_type_object,
            )?)
        };
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let attr_ptr = unsafe { PyObject_GetAttrString(raw_ptr, c_name.as_ptr()) };
        if attr_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "<no-error>".to_string());
            // `load_cpython_proxy_attr_for_value` treats missing attributes as a soft miss.
            // Clear transient C-API error state in this temporary context before returning.
            call_ctx.clear_error();
            if trace_proxy_max {
                eprintln!(
                    "[proxy-max] getattr miss raw_ptr={:p} err={}",
                    raw_ptr, detail
                );
            }
            if trace_proxy_attr {
                eprintln!(
                    "[proxy-attr] PyObject_GetAttrString miss attr={} err={}",
                    attr_name, detail
                );
            }
            if !is_proxy_type_object
                && let Some(fallback_ptr) =
                    call_ctx.lookup_type_attr_via_tp_dict(raw_ptr, attr_name)
                && !fallback_ptr.is_null()
            {
                if trace_proxy_attr {
                    eprintln!(
                        "[proxy-attr] tp_dict fallback hit attr={} ptr={:p}",
                        attr_name, fallback_ptr
                    );
                }
                if std::env::var_os("PYRS_TRACE_PROXY_ATTR_CALL").is_some() {
                    eprintln!(
                        "[proxy-attr-map] source=tp_dict_fallback target={:p} attr={} value_ptr={:p}",
                        raw_ptr, attr_name, fallback_ptr
                    );
                }
                return call_ctx.cpython_value_from_borrowed_ptr(fallback_ptr);
            }
            if trace_type_attr {
                eprintln!(
                    "[cpy-proxy-attr] lookup miss ptr={:p} attr={}",
                    raw_ptr, attr_name
                );
            }
            return None;
        }
        if trace_proxy_max {
            eprintln!(
                "[proxy-max] getattr hit raw_ptr={:p} attr_ptr={:p}",
                raw_ptr, attr_ptr
            );
        }
        if trace_type_attr {
            eprintln!(
                "[cpy-proxy-attr] lookup hit ptr={:p} attr={} result_ptr={:p}",
                raw_ptr, attr_name, attr_ptr
            );
        }
        if trace_proxy_attr {
            eprintln!(
                "[proxy-attr] PyObject_GetAttrString hit attr={} ptr={:p}",
                attr_name, attr_ptr
            );
        }
        if std::env::var_os("PYRS_TRACE_PROXY_ATTR_CALL").is_some() {
            let proxy_tag = cpython_value_debug_tag(proxy_value);
            let (attr_type_ptr, attr_type_name) = unsafe {
                let type_ptr = attr_ptr
                    .cast::<CpythonObjectHead>()
                    .as_ref()
                    .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                    .unwrap_or(std::ptr::null_mut());
                let type_name = if type_ptr.is_null() {
                    "<null>".to_string()
                } else {
                    c_name_to_string((*type_ptr).tp_name)
                        .unwrap_or_else(|_| "<invalid>".to_string())
                };
                (type_ptr, type_name)
            };
            eprintln!(
                "[proxy-attr-map] source=getattr target={:p} target_tag={} attr={} value_ptr={:p} type={:p} type_name={}",
                raw_ptr, proxy_tag, attr_name, attr_ptr, attr_type_ptr, attr_type_name
            );
        }
        let mapped = call_ctx.cpython_value_from_owned_ptr(attr_ptr);
        if mapped.is_none()
            && !is_proxy_type_object
            && let Some(fallback_ptr) = call_ctx.lookup_type_attr_via_tp_dict(raw_ptr, attr_name)
            && !fallback_ptr.is_null()
        {
            if std::env::var_os("PYRS_TRACE_PROXY_ATTR_CALL").is_some() {
                eprintln!(
                    "[proxy-attr-map] source=getattr_unmapped_tp_dict_fallback target={:p} attr={} value_ptr={:p}",
                    raw_ptr, attr_name, fallback_ptr
                );
            }
            return call_ctx.cpython_value_from_borrowed_ptr(fallback_ptr);
        }
        if trace_proxy_max {
            let mapped_tag = mapped
                .as_ref()
                .map(cpython_value_debug_tag)
                .unwrap_or_else(|| "<none>".to_string());
            eprintln!(
                "[proxy-max] mapped attr=max raw_ptr={:p} value={}",
                raw_ptr, mapped_tag
            );
        }
        if std::env::var_os("PYRS_TRACE_PROXY_ATTR_CALL").is_some() {
            let proxy_tag = cpython_value_debug_tag(proxy_value);
            let mapped_tag = mapped
                .as_ref()
                .map(cpython_value_debug_tag)
                .unwrap_or_else(|| "<none>".to_string());
            eprintln!(
                "[proxy-attr-map] source=getattr_mapped target={:p} target_tag={} attr={} value_ptr={:p} mapped={}",
                raw_ptr, proxy_tag, attr_name, attr_ptr, mapped_tag
            );
        }
        mapped
    }

    pub(in crate::vm) fn bind_cpython_descriptor_for_super(
        &mut self,
        descriptor_value: &Value,
        receiver_value: &Value,
        owner_value: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        let descriptor_ptr = Self::cpython_proxy_raw_ptr_from_value(descriptor_value)?;
        let mut call_ctx = ModuleCapiContext::new(self as *mut Vm, self.main_module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(call_ctx));
        let receiver_ptr = call_ctx.alloc_cpython_ptr_for_value(receiver_value.clone());
        if receiver_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy descriptor binding failed to materialize receiver",
            )));
        }
        let owner_ptr = call_ctx.alloc_cpython_ptr_for_value(owner_value.clone());
        if owner_ptr.is_null() {
            return Some(Err(RuntimeError::new(
                "proxy descriptor binding failed to materialize owner",
            )));
        }
        let bound_ptr = call_ctx
            .resolve_descriptor_attr_ptr(
                descriptor_ptr,
                receiver_ptr,
                owner_ptr.cast::<CpythonTypeObject>(),
                false,
            )
            .or_else(|| {
                call_ctx.bind_generic_descriptor_attr_ptr(
                    descriptor_ptr,
                    receiver_ptr,
                    owner_ptr,
                    false,
                )
            })?;
        if bound_ptr.is_null() {
            let detail = call_ctx
                .last_error
                .clone()
                .unwrap_or_else(|| "proxy descriptor binding failed".to_string());
            call_ctx.clear_error();
            return Some(Err(RuntimeError::new(detail)));
        }
        let value = call_ctx
            .cpython_value_from_owned_ptr(bound_ptr)
            .or_else(|| call_ctx.cpython_value_from_borrowed_ptr(bound_ptr))
            .ok_or_else(|| RuntimeError::new("proxy descriptor binding produced unknown value"));
        Some(value)
    }

    pub(in crate::vm) fn load_cpython_proxy_attr(
        &mut self,
        proxy_class: &ObjRef,
        attr_name: &str,
    ) -> Option<Value> {
        self.load_cpython_proxy_attr_for_value(&Value::Class(proxy_class.clone()), attr_name)
    }

    pub(in crate::vm) fn cpython_proxy_type_flags(&self, proxy_class: &ObjRef) -> Option<i64> {
        let raw_ptr = Self::cpython_proxy_raw_ptr_from_value(&Value::Class(proxy_class.clone()))?;
        if !cpython_is_type_object_ptr(raw_ptr) {
            return None;
        }
        // SAFETY: `raw_ptr` is verified as a type object pointer and `tp_flags` is a plain field read.
        let flags = unsafe { (*raw_ptr.cast::<CpythonTypeObject>()).tp_flags };
        i64::try_from(flags).ok()
    }
}
