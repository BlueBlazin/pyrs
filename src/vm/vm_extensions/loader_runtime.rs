use std::backtrace::Backtrace;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use crate::extensions::{
    CpythonExtensionInit, ExtensionEntrypoint, PYRS_CAPI_ABI_VERSION, PYRS_DYNAMIC_INIT_SYMBOL_V1,
    PYRS_EXTENSION_ABI_TAG, PYRS_EXTENSION_MANIFEST_SUFFIX, PyrsApiV1, keep_dynamic_library_loaded,
    load_dynamic_initializer, load_dynamic_symbol, parse_extension_manifest,
    path_is_shared_library,
};
use crate::runtime::{Object, RuntimeError, Value};
use crate::vm::ExtensionCapsuleRegistryEntry;

use super::cpython_context_runtime::ActiveCpythonContextGuard;
use super::cpython_module_runtime::cpython_bind_module_def;
use super::{
    _Py_NoneStruct, CpythonModuleDef, CpythonModuleDefSlot, CpythonObjectHead, CpythonTypeObject,
    ExtensionCallableKind, ExtensionInitScopeGuard, ModuleCapiContext, ObjRef, PYRS_DATETIME_CAPI,
    PYRS_DATETIME_CAPSULE_NAME, PYRS_DATETIME_DATE_TYPE, PYRS_DATETIME_DATETIME_TYPE,
    PYRS_DATETIME_DELTA_TYPE, PYRS_DATETIME_TIME_TYPE, PYRS_DATETIME_TZINFO_TYPE, Vm,
    c_name_to_string, initialize_datetime_capi_types, with_active_cpython_context_mut,
};

enum ExtensionExecutionPlan {
    HelloExt,
    Dynamic {
        library_path: PathBuf,
        symbol: String,
    },
}

impl Vm {
    pub(super) fn ensure_builtin_datetime_capi_capsule(&mut self) {
        if self
            .extension_capsule_registry
            .contains_key(PYRS_DATETIME_CAPSULE_NAME)
        {
            return;
        }
        let mut datetime_date_value: Option<Value> = None;
        let mut datetime_datetime_value: Option<Value> = None;
        let mut datetime_time_value: Option<Value> = None;
        let mut datetime_timedelta_value: Option<Value> = None;
        let mut datetime_tzinfo_value: Option<Value> = None;
        let mut datetime_timezone_utc_value: Option<Value> = None;
        if self.import_module("datetime").is_ok()
            && let Some(module) = self.modules.get("datetime")
            && let Object::Module(module_data) = &*module.kind()
        {
            datetime_date_value = module_data.globals.get("date").cloned();
            datetime_datetime_value = module_data.globals.get("datetime").cloned();
            datetime_time_value = module_data.globals.get("time").cloned();
            datetime_timedelta_value = module_data.globals.get("timedelta").cloned();
            datetime_tzinfo_value = module_data.globals.get("tzinfo").cloned();
            datetime_timezone_utc_value = module_data.globals.get("UTC").cloned().or_else(|| {
                module_data
                    .globals
                    .get("timezone")
                    .and_then(|timezone| match timezone {
                        Value::Class(class_obj) => match &*class_obj.kind() {
                            Object::Class(class_data) => class_data.attrs.get("utc").cloned(),
                            _ => None,
                        },
                        _ => None,
                    })
            });
        }

        let materialize_cpython_ptr = |value: Option<Value>| -> Option<*mut c_void> {
            let value = value?;
            with_active_cpython_context_mut(|context| {
                let ptr = context.alloc_cpython_ptr_for_value(value);
                (!ptr.is_null()).then_some(ptr)
            })
            .ok()
            .flatten()
        };

        let runtime_date_type_ptr = materialize_cpython_ptr(datetime_date_value);
        let runtime_datetime_type_ptr = materialize_cpython_ptr(datetime_datetime_value);
        let runtime_time_type_ptr = materialize_cpython_ptr(datetime_time_value);
        let runtime_timedelta_type_ptr = materialize_cpython_ptr(datetime_timedelta_value);
        let runtime_tzinfo_type_ptr = materialize_cpython_ptr(datetime_tzinfo_value);
        let runtime_timezone_utc_ptr = materialize_cpython_ptr(datetime_timezone_utc_value);
        // SAFETY: static capsule storage and exported type/singleton symbols live for
        // process lifetime; registry stores raw pointers as opaque capsule payloads.
        unsafe {
            initialize_datetime_capi_types();
            PYRS_DATETIME_CAPI.date_type = runtime_date_type_ptr
                .unwrap_or_else(|| std::ptr::addr_of_mut!(PYRS_DATETIME_DATE_TYPE).cast());
            PYRS_DATETIME_CAPI.datetime_type = runtime_datetime_type_ptr
                .unwrap_or_else(|| std::ptr::addr_of_mut!(PYRS_DATETIME_DATETIME_TYPE).cast());
            PYRS_DATETIME_CAPI.time_type = runtime_time_type_ptr
                .unwrap_or_else(|| std::ptr::addr_of_mut!(PYRS_DATETIME_TIME_TYPE).cast());
            PYRS_DATETIME_CAPI.delta_type = runtime_timedelta_type_ptr
                .unwrap_or_else(|| std::ptr::addr_of_mut!(PYRS_DATETIME_DELTA_TYPE).cast());
            PYRS_DATETIME_CAPI.tzinfo_type = runtime_tzinfo_type_ptr
                .unwrap_or_else(|| std::ptr::addr_of_mut!(PYRS_DATETIME_TZINFO_TYPE).cast());
            PYRS_DATETIME_CAPI.timezone_utc = runtime_timezone_utc_ptr
                .unwrap_or_else(|| std::ptr::addr_of_mut!(_Py_NoneStruct).cast());
            if super::super::env_var_present_cached("PYRS_TRACE_DATETIME_CAPSULE") {
                let capi_date = PYRS_DATETIME_CAPI.date_type;
                let capi_datetime = PYRS_DATETIME_CAPI.datetime_type;
                let capi_time = PYRS_DATETIME_CAPI.time_type;
                let capi_timedelta = PYRS_DATETIME_CAPI.delta_type;
                let capi_tzinfo = PYRS_DATETIME_CAPI.tzinfo_type;
                let capi_utc = PYRS_DATETIME_CAPI.timezone_utc;
                eprintln!(
                    "[datetime-capsule] date={:p} datetime={:p} time={:p} timedelta={:p} tzinfo={:p} utc={:p} runtime_date={:p} runtime_datetime={:p} runtime_time={:p} runtime_timedelta={:p} runtime_tzinfo={:p} runtime_utc={:p}",
                    capi_date,
                    capi_datetime,
                    capi_time,
                    capi_timedelta,
                    capi_tzinfo,
                    capi_utc,
                    runtime_date_type_ptr.unwrap_or(std::ptr::null_mut()),
                    runtime_datetime_type_ptr.unwrap_or(std::ptr::null_mut()),
                    runtime_time_type_ptr.unwrap_or(std::ptr::null_mut()),
                    runtime_timedelta_type_ptr.unwrap_or(std::ptr::null_mut()),
                    runtime_tzinfo_type_ptr.unwrap_or(std::ptr::null_mut()),
                    runtime_timezone_utc_ptr.unwrap_or(std::ptr::null_mut()),
                );
            }
            self.extension_capsule_registry.insert(
                PYRS_DATETIME_CAPSULE_NAME.to_string(),
                ExtensionCapsuleRegistryEntry {
                    pointer: std::ptr::addr_of_mut!(PYRS_DATETIME_CAPI) as usize,
                    context: 0,
                    destructor: None,
                },
            );
        }
    }

    pub(super) fn prune_extension_module_state_registry(&mut self) {
        let live_module_ids: std::collections::HashSet<u64> =
            self.modules.values().map(|module| module.id()).collect();
        self.extension_module_def_registry
            .retain(|module_id, _| live_module_ids.contains(module_id));
        let stale_ids: Vec<u64> = self
            .extension_module_state_registry
            .keys()
            .copied()
            .filter(|id| !live_module_ids.contains(id))
            .collect();
        for stale_id in stale_ids {
            if let Some(entry) = self.extension_module_state_registry.remove(&stale_id) {
                if entry.state != 0 {
                    if let Some(finalize_func) = entry.finalize_func {
                        // SAFETY: finalize function pointer was provided by extension code.
                        unsafe {
                            finalize_func(entry.state as *mut c_void);
                        }
                    }
                    if let Some(free_func) = entry.free_func {
                        // SAFETY: free function pointer was provided by extension code.
                        unsafe {
                            free_func(entry.state as *mut c_void);
                        }
                    }
                }
            }
        }
    }

    fn cpython_init_symbol_for_module(module_name: &str) -> String {
        let leaf = module_name
            .rsplit('.')
            .next()
            .unwrap_or(module_name)
            .replace('-', "_");
        format!("PyInit_{leaf}")
    }

    pub(super) fn register_cpython_module_methods_from_def(
        &mut self,
        module: &ObjRef,
        module_def: *mut CpythonModuleDef,
    ) -> Result<(), RuntimeError> {
        if module_def.is_null() {
            return Ok(());
        }
        // SAFETY: module_def points to extension-provided module definition.
        let methods_ptr = unsafe { (*module_def).m_methods };
        if super::super::env_var_present_cached("PYRS_TRACE_CPY_MODULE_METHODS") {
            let module_name = match &*module.kind() {
                Object::Module(module_data) => module_data.name.clone(),
                _ => "<non-module>".to_string(),
            };
            eprintln!(
                "[cpy-module-methods] module={} module_def={:p} methods_ptr={:p}",
                module_name, module_def, methods_ptr
            );
        }
        if methods_ptr.is_null() {
            return Ok(());
        }
        let mut method = methods_ptr;
        loop {
            // SAFETY: method table is terminated by null `ml_name`.
            let method_name_ptr = unsafe { (*method).ml_name };
            if method_name_ptr.is_null() {
                break;
            }
            // SAFETY: `ml_name` is NUL-terminated by PyMethodDef contract.
            let method_name =
                unsafe { c_name_to_string(method_name_ptr) }.map_err(RuntimeError::new)?;
            if super::super::env_var_present_cached("PYRS_TRACE_CPY_MODULE_METHODS") {
                // SAFETY: method points to valid PyMethodDef entry.
                let flags = unsafe { (*method).ml_flags };
                eprintln!(
                    "[cpy-module-methods] register method={} def_ptr={:p} flags={}",
                    method_name, method, flags
                );
            }
            let callable = self.register_extension_callable(
                module.clone(),
                &method_name,
                ExtensionCallableKind::CpythonMethod {
                    method_def: method as usize,
                },
            )?;
            let Object::Module(module_data) = &mut *module.kind_mut() else {
                return Err(RuntimeError::new(
                    "extension module target is not a module during method registration",
                ));
            };
            module_data.globals.insert(method_name, callable);
            // SAFETY: method table entries are contiguous.
            method = unsafe { method.add(1) };
        }
        Ok(())
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
        let import_error =
            |message: String| RuntimeError::with_exception("ImportError", Some(message));
        let trace_slots = super::super::env_var_present_cached("PYRS_TRACE_EXT_SLOTS");
        if trace_slots {
            eprintln!(
                "[ext-load] module={} begin initialized={} in_progress={}",
                module_name,
                self.extension_initialized_names.contains(module_name),
                self.extension_init_in_progress.contains(module_name)
            );
        }
        let module_flag_initialized = if let Object::Module(module_data) = &*module.kind() {
            matches!(
                module_data.globals.get("__pyrs_extension_initialized__"),
                Some(Value::Bool(true))
            )
        } else {
            false
        };
        if self.extension_init_in_progress.contains(module_name) {
            if trace_slots {
                eprintln!("[ext-load] module={} skip=init_in_progress", module_name);
            }
            return Ok(());
        }
        if self.extension_initialized_names.contains(module_name) {
            if trace_slots {
                eprintln!(
                    "[ext-load] module={} skip=already_initialized_name_guard",
                    module_name
                );
            }
            if let Some(existing) = self.modules.get(module_name).cloned() {
                if existing.id() != module.id() {
                    // Keep canonical cache authoritative. The caller resolves through
                    // `canonical_imported_module_for_name`, so preserve the initialized object.
                    self.modules
                        .insert(module_name.to_string(), existing.clone());
                    if let Object::Module(existing_data) = &*existing.kind()
                        && let Object::Module(current_data) = &mut *module.kind_mut()
                    {
                        current_data.globals = existing_data.globals.clone();
                    }
                }
                return Ok(());
            }
            if module_flag_initialized {
                return Ok(());
            }
            // CPython single-phase extension modules cannot be loaded more than once per process.
            return Err(import_error(
                "cannot load module more than once per process".to_string(),
            ));
        }
        if module_flag_initialized {
            if trace_slots {
                eprintln!(
                    "[ext-load] module={} skip=module_flag_initialized",
                    module_name
                );
            }
            return Ok(());
        }
        if let Some(message) = self.extension_init_failures.get(module_name).cloned() {
            return Err(RuntimeError::new(message));
        }
        if let Object::Module(module_data) = &*module.kind()
            && let Some(Value::Str(message)) =
                module_data.globals.get("__pyrs_extension_init_error__")
        {
            return Err(RuntimeError::new(message.clone()));
        }
        self.extension_init_in_progress
            .insert(module_name.to_string());
        let _init_scope_guard = ExtensionInitScopeGuard::new(self, module_name);

        enum ResolvedInit {
            Pyrs {
                initializer: crate::extensions::PyrsExtensionInitV1,
            },
            Cpython {
                initializer: CpythonExtensionInit,
            },
        }

        let mut module_ctx = ModuleCapiContext::new(self as *mut Vm, module.clone());
        let _active_context_guard =
            ActiveCpythonContextGuard::push(std::ptr::addr_of_mut!(module_ctx));
        let mut active_module = module.clone();
        let (resolved_symbol, resolved_init): (String, ResolvedInit) = {
            // Some extensions (notably pybind11-based modules) execute static initializers during
            // `dlopen` that query thread/interpreter state. Keep the module C-API context active
            // for the entire init flow so nested imports/calls observe the same synchronous
            // extension semantics as CPython extension init.
            if symbol.starts_with("PyInit_") {
                let (handle, init) =
                    load_dynamic_symbol::<CpythonExtensionInit>(library_path, symbol)
                        .map_err(import_error)?;
                keep_dynamic_library_loaded(library_path, handle);
                (
                    symbol.to_string(),
                    ResolvedInit::Cpython { initializer: init },
                )
            } else {
                match load_dynamic_initializer(library_path, symbol) {
                    Ok((handle, init)) => {
                        keep_dynamic_library_loaded(library_path, handle);
                        (symbol.to_string(), ResolvedInit::Pyrs { initializer: init })
                    }
                    Err(pyrs_err) if symbol == PYRS_DYNAMIC_INIT_SYMBOL_V1 => {
                        let cpython_symbol = Self::cpython_init_symbol_for_module(module_name);
                        match load_dynamic_symbol::<CpythonExtensionInit>(
                            library_path,
                            &cpython_symbol,
                        ) {
                            Ok((handle, init)) => {
                                keep_dynamic_library_loaded(library_path, handle);
                                (cpython_symbol, ResolvedInit::Cpython { initializer: init })
                            }
                            Err(cpython_err) => {
                                return Err(import_error(format!(
                                    "{pyrs_err}; fallback '{}' also failed: {cpython_err}",
                                    cpython_symbol
                                )));
                            }
                        }
                    }
                    Err(err) => return Err(import_error(err)),
                }
            }
        };
        if matches!(&resolved_init, ResolvedInit::Cpython { .. }) {
            module_ctx.run_capsule_destructors_on_drop = false;
            module_ctx.strict_capsule_refcount = false;
            module_ctx.keep_cpython_allocations_on_drop = true;
        }
        let init_result = match resolved_init {
            ResolvedInit::Pyrs { initializer } => {
                // The shared library was promoted to process-lifetime keepalive before init.
                // Unwind-time callback teardown can therefore safely invoke extension code
                // even if initializer execution fails partway through.
                let api = self.capi_api_v1();
                // SAFETY: initializer is resolved from the shared object symbol with expected signature;
                // pointers are valid for the duration of the call.
                let status = unsafe {
                    initializer(
                        &api as *const PyrsApiV1,
                        (std::ptr::addr_of_mut!(module_ctx)).cast(),
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
                        module_name, resolved_symbol, status, message
                    )));
                }
                if let Some(message) = module_ctx.last_error.as_deref() {
                    return Err(RuntimeError::new(format!(
                        "extension '{}' initializer '{}' reported error despite success: {}",
                        module_name, resolved_symbol, message
                    )));
                }
                std::ptr::null_mut()
            }
            ResolvedInit::Cpython { initializer } => {
                // SAFETY: symbol was resolved with `unsafe extern "C" fn() -> *mut c_void`.
                unsafe { initializer() }
            }
        };

        if resolved_symbol.starts_with("PyInit_") {
            if init_result.is_null() {
                let message = module_ctx
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "extension returned null module".to_string());
                return Err(RuntimeError::new(format!(
                    "extension '{}' initializer '{}' failed: {}",
                    module_name, resolved_symbol, message
                )));
            }
            let returned = if let Some(value) = module_ctx.cpython_value_from_ptr(init_result) {
                value
            } else {
                // CPython multi-phase extensions return `PyModuleDef*` from `PyInit_*`.
                // Our import path already created the target module object, so use that
                // module as the execution target and drive slot execution from `m_slots`.
                let mut module_ptr =
                    module_ctx.alloc_cpython_ptr_for_value(Value::Module(active_module.clone()));
                if !module_ptr.is_null() {
                    let module_def = init_result.cast::<CpythonModuleDef>();
                    if !module_def.is_null() {
                        if let Err(err) =
                            cpython_bind_module_def(&mut module_ctx, &active_module, module_def)
                        {
                            return Err(RuntimeError::new(format!(
                                "extension '{}' initializer '{}' failed to bind module definition: {}",
                                module_name, resolved_symbol, err
                            )));
                        }
                        self.register_cpython_module_methods_from_def(&active_module, module_def)?;
                        // SAFETY: module_def points to extension-provided PyModuleDef layout.
                        let slots_ptr = unsafe { (*module_def).m_slots };
                        if !slots_ptr.is_null() {
                            let mut slot_index = 0usize;
                            let mut cursor = slots_ptr.cast::<CpythonModuleDefSlot>();
                            let module_spec_ptr = match &*active_module.kind() {
                                Object::Module(module_data) => module_data
                                    .globals
                                    .get("__spec__")
                                    .cloned()
                                    .map(|spec| module_ctx.alloc_cpython_ptr_for_value(spec))
                                    .unwrap_or(std::ptr::null_mut()),
                                _ => std::ptr::null_mut(),
                            };
                            loop {
                                // SAFETY: slots array is terminated by {0, NULL}.
                                let slot = unsafe { (*cursor).slot };
                                // SAFETY: slots array is terminated by {0, NULL}.
                                let value = unsafe { (*cursor).value };
                                if slot == 0 {
                                    break;
                                }
                                if trace_slots {
                                    eprintln!(
                                        "[ext-slot] module={} symbol={} index={} slot={} value={:p}",
                                        module_name, resolved_symbol, slot_index, slot, value
                                    );
                                }
                                if slot == 1 && !value.is_null() {
                                    // Py_mod_create(module_spec, module_def) -> module object.
                                    let create: unsafe extern "C" fn(
                                        *mut c_void,
                                        *mut c_void,
                                    )
                                        -> *mut c_void = unsafe { std::mem::transmute(value) };
                                    let created = unsafe { create(module_spec_ptr, init_result) };
                                    if !created.is_null() {
                                        module_ptr = created;
                                        // Keep sys.modules aligned with the create-slot module
                                        // before running any exec slots so recursive imports
                                        // observe the active module instance.
                                        if let Some(Value::Module(created_module)) =
                                            module_ctx.cpython_value_from_ptr(created)
                                        {
                                            self.modules.insert(
                                                module_name.to_string(),
                                                created_module.clone(),
                                            );
                                            module_ctx.module = created_module.clone();
                                            active_module = created_module.clone();
                                            if let Err(err) = cpython_bind_module_def(
                                                &mut module_ctx,
                                                &active_module,
                                                module_def,
                                            ) {
                                                return Err(RuntimeError::new(format!(
                                                    "extension '{}' create-slot module bind failed: {}",
                                                    module_name, err
                                                )));
                                            }
                                            if created_module.id() != module.id()
                                                && let Object::Module(current_data) =
                                                    &*module.kind()
                                                && let Object::Module(created_data) =
                                                    &mut *created_module.kind_mut()
                                            {
                                                for (key, value) in &current_data.globals {
                                                    let force_metadata = matches!(
                                                        key.as_str(),
                                                        "__name__"
                                                            | "__package__"
                                                            | "__loader__"
                                                            | "__spec__"
                                                            | "__file__"
                                                            | "__path__"
                                                    );
                                                    if force_metadata {
                                                        created_data
                                                            .globals
                                                            .insert(key.clone(), value.clone());
                                                    } else {
                                                        created_data
                                                            .globals
                                                            .entry(key.clone())
                                                            .or_insert_with(|| value.clone());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else if slot == 2 && !value.is_null() {
                                    // Py_mod_exec(module) -> int status.
                                    let exec: unsafe extern "C" fn(*mut c_void) -> i32 =
                                        unsafe { std::mem::transmute(value) };
                                    if trace_slots {
                                        let (module_type, module_type_name) =
                                            if module_ptr.is_null() {
                                                (std::ptr::null_mut(), "<null>".to_string())
                                            } else {
                                                // SAFETY: best-effort diagnostics before exec slot call.
                                                unsafe {
                                                    let ty = module_ptr
                                                        .cast::<CpythonObjectHead>()
                                                        .as_ref()
                                                        .map(|head| {
                                                            head.ob_type.cast::<CpythonTypeObject>()
                                                        })
                                                        .unwrap_or(std::ptr::null_mut());
                                                    let ty_name = ty
                                                        .as_ref()
                                                        .and_then(|raw| {
                                                            c_name_to_string(raw.tp_name).ok()
                                                        })
                                                        .unwrap_or_else(|| "<unknown>".to_string());
                                                    (ty.cast::<c_void>(), ty_name)
                                                }
                                            };
                                        eprintln!(
                                            "[ext-slot] module={} slot=2 exec={:p} module_ptr={:p} module_type={:p} module_type_name={}",
                                            module_name,
                                            value,
                                            module_ptr,
                                            module_type,
                                            module_type_name
                                        );
                                    }
                                    let status = unsafe { exec(module_ptr) };
                                    if status != 0 {
                                        if module_ctx.last_error.is_none()
                                            && super::super::env_var_present_cached(
                                                "PYRS_IGNORE_SLOT_STATUS_NOERROR",
                                            )
                                        {
                                            if trace_slots {
                                                eprintln!(
                                                    "[ext-slot] module={} ignoring non-zero status={} due no last_error",
                                                    module_name, status
                                                );
                                            }
                                            // Continue slot execution in explicit probe mode.
                                            cursor = unsafe { cursor.add(1) };
                                            slot_index += 1;
                                            continue;
                                        }
                                        if module_ctx.last_error.is_none()
                                            && super::super::env_var_present_cached(
                                                "PYRS_TRACE_EXT_SLOT_BT",
                                            )
                                        {
                                            eprintln!(
                                                "[ext-slot] module={} status={} without last_error",
                                                module_name, status
                                            );
                                            eprintln!("{}", Backtrace::force_capture());
                                        }
                                        if super::super::env_var_present_cached(
                                            "PYRS_TRACE_CPY_ERRORS",
                                        ) {
                                            eprintln!(
                                                "[ext-slot] module={} slot_exec_status={} first_error={:?} last_error={:?} current_error_ptype={:p} current_error_pvalue={:p}",
                                                module_name,
                                                status,
                                                module_ctx.first_error,
                                                module_ctx.last_error,
                                                module_ctx
                                                    .current_error
                                                    .as_ref()
                                                    .map_or(std::ptr::null_mut(), |state| state
                                                        .ptype),
                                                module_ctx
                                                    .current_error
                                                    .as_ref()
                                                    .map_or(std::ptr::null_mut(), |state| state
                                                        .pvalue)
                                            );
                                        }
                                        let message = module_ctx
                                            .last_error
                                            .clone()
                                            .or_else(|| module_ctx.first_error.clone())
                                            .unwrap_or_else(|| "Py_mod_exec failed".to_string());
                                        let mut propagated_error = module_ctx
                                            .runtime_error_from_current_error_state(&message);
                                        let detailed_message =
                                            if let Some(err) = propagated_error.as_ref() {
                                                if err.message.is_empty() {
                                                    message.clone()
                                                } else {
                                                    err.message.clone()
                                                }
                                            } else if module_ctx.vm.is_null() {
                                                message.clone()
                                            } else {
                                                // SAFETY: module C-API context owns a valid VM pointer
                                                // for the duration of extension initialization.
                                                let vm = unsafe { &mut *module_ctx.vm };
                                                let err = vm
                                                    .runtime_error_from_active_exception(&message);
                                                let detail = if err.message.is_empty() {
                                                    message.clone()
                                                } else {
                                                    err.message.clone()
                                                };
                                                propagated_error = Some(err);
                                                detail
                                            };
                                        let full_error = format!(
                                            "extension '{}' initializer '{}' Py_mod_exec failed: {}",
                                            module_name, resolved_symbol, detailed_message
                                        );
                                        if super::super::env_var_present_cached(
                                            "PYRS_TRACE_EXT_SLOT_MODULE_KEYS",
                                        ) && let Object::Module(module_data) =
                                            &*active_module.kind()
                                        {
                                            let mut names: Vec<String> =
                                                module_data.globals.keys().cloned().collect();
                                            names.sort();
                                            let mut probe = Vec::new();
                                            for key in [
                                                "_ARRAY_API",
                                                "False_",
                                                "True_",
                                                "add",
                                                "matmul",
                                                "arange",
                                                "ndarray",
                                            ] {
                                                probe.push(format!(
                                                    "{}={}",
                                                    key,
                                                    module_data.globals.contains_key(key)
                                                ));
                                            }
                                            eprintln!(
                                                "[ext-slot] module={} keys={} probe=[{}] sample={:?}",
                                                module_name,
                                                names.len(),
                                                probe.join(", "),
                                                names.iter().take(24).collect::<Vec<_>>()
                                            );
                                        }
                                        if let Object::Module(module_data) =
                                            &mut *active_module.kind_mut()
                                        {
                                            module_data.globals.insert(
                                                "__pyrs_extension_init_error__".to_string(),
                                                Value::Str(full_error.clone()),
                                            );
                                        }
                                        self.extension_init_failures
                                            .insert(module_name.to_string(), full_error.clone());
                                        if trace_slots {
                                            let propagated_name = propagated_error
                                                .as_ref()
                                                .and_then(|err| err.exception_name())
                                                .unwrap_or("<none>");
                                            eprintln!(
                                                "[ext-load] module={} slot_exec_error={} propagated={}",
                                                module_name, detailed_message, propagated_name
                                            );
                                        }
                                        return Err(propagated_error
                                            .unwrap_or_else(|| RuntimeError::new(full_error)));
                                    }
                                }
                                // SAFETY: move to next slot entry.
                                cursor = unsafe { cursor.add(1) };
                                slot_index += 1;
                            }
                        }
                    }
                }
                if module_ptr.is_null() {
                    if trace_slots {
                        eprintln!("[ext-load] module={} unknown_module_ptr", module_name);
                    }
                    return Err(RuntimeError::new(format!(
                        "extension '{}' initializer '{}' returned unknown PyObject pointer",
                        module_name, resolved_symbol
                    )));
                }
                module_ctx
                    .cpython_value_from_ptr(module_ptr)
                    .ok_or_else(|| {
                        RuntimeError::new(format!(
                            "extension '{}' initializer '{}' returned unknown PyObject pointer",
                            module_name, resolved_symbol
                        ))
                    })?
            };
            let Value::Module(returned_module) = returned else {
                if trace_slots {
                    eprintln!("[ext-load] module={} non_module_return", module_name);
                }
                return Err(RuntimeError::new(format!(
                    "extension '{}' initializer '{}' did not return a module object",
                    module_name, resolved_symbol
                )));
            };
            if returned_module.id() != module.id() {
                if trace_slots {
                    eprintln!(
                        "[ext-load] module={} reconcile_module_instance returned_id={} expected_id={}",
                        module_name,
                        returned_module.id(),
                        active_module.id()
                    );
                }
                self.modules
                    .insert(module_name.to_string(), returned_module.clone());
                if let Object::Module(returned_data) = &*returned_module.kind()
                    && let Object::Module(current_data) = &mut *module.kind_mut()
                {
                    current_data.globals = returned_data.globals.clone();
                }
            }
            module_ctx.module = returned_module.clone();
            active_module = returned_module;
        }

        let Object::Module(module_data) = &mut *active_module.kind_mut() else {
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
            Value::Str(resolved_symbol.clone()),
        );
        module_data.globals.insert(
            "__pyrs_extension_entrypoint__".to_string(),
            Value::Str(format!("dynamic:{resolved_symbol}")),
        );
        let symbol_family = if resolved_symbol == PYRS_DYNAMIC_INIT_SYMBOL_V1 {
            "pyrs-v1"
        } else if resolved_symbol.starts_with("PyInit_") {
            "cpython"
        } else {
            "custom"
        };
        module_data.globals.insert(
            "__pyrs_extension_expected_symbol__".to_string(),
            Value::Str(resolved_symbol),
        );
        module_data.globals.insert(
            "__pyrs_extension_symbol_family__".to_string(),
            Value::Str(symbol_family.to_string()),
        );
        module_data.globals.insert(
            "__pyrs_extension_initialized__".to_string(),
            Value::Bool(true),
        );
        module_data.globals.remove("__pyrs_extension_init_error__");
        self.extension_init_failures.remove(module_name);
        if symbol_family == "cpython" {
            self.extension_initialized_names
                .insert(module_name.to_string());
        } else {
            self.extension_initialized_names.remove(module_name);
        }
        if trace_slots {
            eprintln!("[ext-load] module={} done", module_name);
        }
        Ok(())
    }

    pub(in crate::vm) fn exec_extension_module(
        &mut self,
        module: &ObjRef,
        name: &str,
        source_path: &Path,
    ) -> Result<(), RuntimeError> {
        // `_elementtree` hard-links to `pyexpat.expat_CAPI` in module init. Until the
        // capsule surface is implemented in pyrs, prefer CPython's pure-Python fallback
        // path in `xml.etree.ElementTree` by making this extension unavailable.
        if name == "_elementtree" {
            return Err(RuntimeError::import_error(
                "dynamic module '_elementtree' is unavailable",
            ));
        }
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
