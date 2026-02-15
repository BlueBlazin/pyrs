use super::{
    BigInt, BuiltinFunction, ClassObject, HashMap, InstanceObject, ModuleObject, ObjRef, Object,
    RuntimeError, Value, Vm, dict_set_value,
};
use std::ffi::{CStr, CString, c_char, c_int, c_longlong, c_void};
use std::path::PathBuf;

type PyObject = c_void;
type PySsizeT = isize;

type PyIsInitializedFn = unsafe extern "C" fn() -> c_int;
type PyInitializeExFn = unsafe extern "C" fn(c_int);
type PyGILStateEnsureFn = unsafe extern "C" fn() -> c_int;
type PyGILStateReleaseFn = unsafe extern "C" fn(c_int);
type PyImportImportModuleFn = unsafe extern "C" fn(*const c_char) -> *mut PyObject;
type PyObjectGetAttrStringFn = unsafe extern "C" fn(*mut PyObject, *const c_char) -> *mut PyObject;
type PyObjectCallFn =
    unsafe extern "C" fn(*mut PyObject, *mut PyObject, *mut PyObject) -> *mut PyObject;
type PyObjectStrFn = unsafe extern "C" fn(*mut PyObject) -> *mut PyObject;
type PyObjectReprFn = unsafe extern "C" fn(*mut PyObject) -> *mut PyObject;
type PyObjectIsTrueFn = unsafe extern "C" fn(*mut PyObject) -> c_int;
type PyObjectIsInstanceFn = unsafe extern "C" fn(*mut PyObject, *mut PyObject) -> c_int;
type PyCallableCheckFn = unsafe extern "C" fn(*mut PyObject) -> c_int;
type PyErrOccurredFn = unsafe extern "C" fn() -> *mut PyObject;
type PyErrFetchFn =
    unsafe extern "C" fn(*mut *mut PyObject, *mut *mut PyObject, *mut *mut PyObject);
type PyErrNormalizeExceptionFn =
    unsafe extern "C" fn(*mut *mut PyObject, *mut *mut PyObject, *mut *mut PyObject);
type PyErrClearFn = unsafe extern "C" fn();
type PyIncRefFn = unsafe extern "C" fn(*mut PyObject);
type PyDecRefFn = unsafe extern "C" fn(*mut PyObject);
type PyBoolFromLongFn = unsafe extern "C" fn(c_int) -> *mut PyObject;
type PyLongFromLongLongFn = unsafe extern "C" fn(c_longlong) -> *mut PyObject;
type PyLongFromStringFn =
    unsafe extern "C" fn(*const c_char, *mut *mut c_char, c_int) -> *mut PyObject;
type PyLongAsLongLongAndOverflowFn = unsafe extern "C" fn(*mut PyObject, *mut c_int) -> c_longlong;
type PyFloatFromDoubleFn = unsafe extern "C" fn(f64) -> *mut PyObject;
type PyFloatAsDoubleFn = unsafe extern "C" fn(*mut PyObject) -> f64;
type PyNumberLongFn = unsafe extern "C" fn(*mut PyObject) -> *mut PyObject;
type PyNumberFloatFn = unsafe extern "C" fn(*mut PyObject) -> *mut PyObject;
type PyUnicodeFromStringAndSizeFn = unsafe extern "C" fn(*const c_char, PySsizeT) -> *mut PyObject;
type PyUnicodeAsUtf8AndSizeFn = unsafe extern "C" fn(*mut PyObject, *mut PySsizeT) -> *const c_char;
type PyBytesFromStringAndSizeFn = unsafe extern "C" fn(*const c_char, PySsizeT) -> *mut PyObject;
type PyBytesAsStringAndSizeFn =
    unsafe extern "C" fn(*mut PyObject, *mut *mut c_char, *mut PySsizeT) -> c_int;
type PyTupleNewFn = unsafe extern "C" fn(PySsizeT) -> *mut PyObject;
type PyTupleSetItemFn = unsafe extern "C" fn(*mut PyObject, PySsizeT, *mut PyObject) -> c_int;
type PyTupleSizeFn = unsafe extern "C" fn(*mut PyObject) -> PySsizeT;
type PyTupleGetItemFn = unsafe extern "C" fn(*mut PyObject, PySsizeT) -> *mut PyObject;
type PyListNewFn = unsafe extern "C" fn(PySsizeT) -> *mut PyObject;
type PyListSetItemFn = unsafe extern "C" fn(*mut PyObject, PySsizeT, *mut PyObject) -> c_int;
type PyListSizeFn = unsafe extern "C" fn(*mut PyObject) -> PySsizeT;
type PyListGetItemFn = unsafe extern "C" fn(*mut PyObject, PySsizeT) -> *mut PyObject;
type PyDictNewFn = unsafe extern "C" fn() -> *mut PyObject;
type PyDictSetItemFn = unsafe extern "C" fn(*mut PyObject, *mut PyObject, *mut PyObject) -> c_int;
type PyDictNextFn = unsafe extern "C" fn(
    *mut PyObject,
    *mut PySsizeT,
    *mut *mut PyObject,
    *mut *mut PyObject,
) -> c_int;

const CPYTHON_PROXY_ID_ATTR: &str = "__pyrs_cpython_proxy_id__";
const CPYTHON_PROXY_CLASS_NAME: &str = "__pyrs_cpython_proxy__";
const CPYTHON_PROXY_MODE_ENV: &str = "PYRS_ENABLE_CPYTHON_ABI_BRIDGE";
const CPYTHON_PROXY_MODULES_ENV: &str = "PYRS_CPYTHON_ABI_BRIDGE_MODULES";
const CPYTHON_PROXY_DEFAULT_MODULE_PREFIXES: &[&str] = &["numpy", "scipy", "pandas", "matplotlib"];

#[cfg(target_os = "macos")]
const RTLD_GLOBAL: i32 = 0x8;
#[cfg(not(target_os = "macos"))]
const RTLD_GLOBAL: i32 = 0x100;
const RTLD_NOW: i32 = 2;

pub(super) struct CpythonAbiBridge {
    _lib: BridgeLibraryHandle,
    py_is_initialized: PyIsInitializedFn,
    py_initialize_ex: PyInitializeExFn,
    pygil_ensure: PyGILStateEnsureFn,
    pygil_release: PyGILStateReleaseFn,
    py_import_import_module: PyImportImportModuleFn,
    py_object_get_attr_string: PyObjectGetAttrStringFn,
    py_object_call: PyObjectCallFn,
    py_object_str: PyObjectStrFn,
    py_object_repr: PyObjectReprFn,
    py_object_is_true: PyObjectIsTrueFn,
    py_object_is_instance: PyObjectIsInstanceFn,
    py_callable_check: PyCallableCheckFn,
    py_err_occurred: PyErrOccurredFn,
    py_err_fetch: PyErrFetchFn,
    py_err_normalize_exception: PyErrNormalizeExceptionFn,
    py_err_clear: PyErrClearFn,
    py_inc_ref: PyIncRefFn,
    py_dec_ref: PyDecRefFn,
    py_bool_from_long: PyBoolFromLongFn,
    py_long_from_longlong: PyLongFromLongLongFn,
    py_long_from_string: PyLongFromStringFn,
    py_long_as_longlong_and_overflow: PyLongAsLongLongAndOverflowFn,
    py_float_from_double: PyFloatFromDoubleFn,
    py_float_as_double: PyFloatAsDoubleFn,
    py_number_long: PyNumberLongFn,
    py_number_float: PyNumberFloatFn,
    py_unicode_from_string_and_size: PyUnicodeFromStringAndSizeFn,
    py_unicode_as_utf8_and_size: PyUnicodeAsUtf8AndSizeFn,
    py_bytes_from_string_and_size: PyBytesFromStringAndSizeFn,
    py_bytes_as_string_and_size: PyBytesAsStringAndSizeFn,
    py_tuple_new: PyTupleNewFn,
    py_tuple_set_item: PyTupleSetItemFn,
    py_tuple_size: PyTupleSizeFn,
    py_tuple_get_item: PyTupleGetItemFn,
    py_list_new: PyListNewFn,
    py_list_set_item: PyListSetItemFn,
    py_list_size: PyListSizeFn,
    py_list_get_item: PyListGetItemFn,
    py_dict_new: PyDictNewFn,
    py_dict_set_item: PyDictSetItemFn,
    py_dict_next: PyDictNextFn,
    py_none: *mut PyObject,
    py_true: *mut PyObject,
    py_false: *mut PyObject,
    py_bool_type: *mut PyObject,
    py_long_type: *mut PyObject,
    py_float_type: *mut PyObject,
    py_unicode_type: *mut PyObject,
    py_bytes_type: *mut PyObject,
    py_list_type: *mut PyObject,
    py_tuple_type: *mut PyObject,
    py_dict_type: *mut PyObject,
}

impl CpythonAbiBridge {
    fn load() -> Result<Self, String> {
        let candidates = libpython_candidates();
        let mut last_err = String::new();
        for candidate in candidates {
            match BridgeLibraryHandle::open(&candidate) {
                Ok(lib) => {
                    return Self::from_library(lib);
                }
                Err(err) => {
                    last_err = format!("{} ({err})", candidate.display());
                }
            }
        }
        Err(format!(
            "failed to load libpython 3.14 dynamic library: {}",
            if last_err.is_empty() {
                "no candidate paths".to_string()
            } else {
                last_err
            }
        ))
    }

    fn from_library(lib: BridgeLibraryHandle) -> Result<Self, String> {
        macro_rules! sym {
            ($name:literal, $ty:ty) => {
                lib.symbol::<$ty>($name)?
            };
        }

        let py_none = lib.symbol_raw("_Py_NoneStruct")? as *mut PyObject;
        let py_true = lib.symbol_raw("_Py_TrueStruct")? as *mut PyObject;
        let py_false = lib.symbol_raw("_Py_FalseStruct")? as *mut PyObject;
        let py_bool_type = lib.symbol_raw("PyBool_Type")? as *mut PyObject;
        let py_long_type = lib.symbol_raw("PyLong_Type")? as *mut PyObject;
        let py_float_type = lib.symbol_raw("PyFloat_Type")? as *mut PyObject;
        let py_unicode_type = lib.symbol_raw("PyUnicode_Type")? as *mut PyObject;
        let py_bytes_type = lib.symbol_raw("PyBytes_Type")? as *mut PyObject;
        let py_list_type = lib.symbol_raw("PyList_Type")? as *mut PyObject;
        let py_tuple_type = lib.symbol_raw("PyTuple_Type")? as *mut PyObject;
        let py_dict_type = lib.symbol_raw("PyDict_Type")? as *mut PyObject;

        Ok(Self {
            py_is_initialized: sym!("Py_IsInitialized", PyIsInitializedFn),
            py_initialize_ex: sym!("Py_InitializeEx", PyInitializeExFn),
            pygil_ensure: sym!("PyGILState_Ensure", PyGILStateEnsureFn),
            pygil_release: sym!("PyGILState_Release", PyGILStateReleaseFn),
            py_import_import_module: sym!("PyImport_ImportModule", PyImportImportModuleFn),
            py_object_get_attr_string: sym!("PyObject_GetAttrString", PyObjectGetAttrStringFn),
            py_object_call: sym!("PyObject_Call", PyObjectCallFn),
            py_object_str: sym!("PyObject_Str", PyObjectStrFn),
            py_object_repr: sym!("PyObject_Repr", PyObjectReprFn),
            py_object_is_true: sym!("PyObject_IsTrue", PyObjectIsTrueFn),
            py_object_is_instance: sym!("PyObject_IsInstance", PyObjectIsInstanceFn),
            py_callable_check: sym!("PyCallable_Check", PyCallableCheckFn),
            py_err_occurred: sym!("PyErr_Occurred", PyErrOccurredFn),
            py_err_fetch: sym!("PyErr_Fetch", PyErrFetchFn),
            py_err_normalize_exception: sym!("PyErr_NormalizeException", PyErrNormalizeExceptionFn),
            py_err_clear: sym!("PyErr_Clear", PyErrClearFn),
            py_inc_ref: sym!("Py_IncRef", PyIncRefFn),
            py_dec_ref: sym!("Py_DecRef", PyDecRefFn),
            py_bool_from_long: sym!("PyBool_FromLong", PyBoolFromLongFn),
            py_long_from_longlong: sym!("PyLong_FromLongLong", PyLongFromLongLongFn),
            py_long_from_string: sym!("PyLong_FromString", PyLongFromStringFn),
            py_long_as_longlong_and_overflow: sym!(
                "PyLong_AsLongLongAndOverflow",
                PyLongAsLongLongAndOverflowFn
            ),
            py_float_from_double: sym!("PyFloat_FromDouble", PyFloatFromDoubleFn),
            py_float_as_double: sym!("PyFloat_AsDouble", PyFloatAsDoubleFn),
            py_number_long: sym!("PyNumber_Long", PyNumberLongFn),
            py_number_float: sym!("PyNumber_Float", PyNumberFloatFn),
            py_unicode_from_string_and_size: sym!(
                "PyUnicode_FromStringAndSize",
                PyUnicodeFromStringAndSizeFn
            ),
            py_unicode_as_utf8_and_size: sym!("PyUnicode_AsUTF8AndSize", PyUnicodeAsUtf8AndSizeFn),
            py_bytes_from_string_and_size: sym!(
                "PyBytes_FromStringAndSize",
                PyBytesFromStringAndSizeFn
            ),
            py_bytes_as_string_and_size: sym!("PyBytes_AsStringAndSize", PyBytesAsStringAndSizeFn),
            py_tuple_new: sym!("PyTuple_New", PyTupleNewFn),
            py_tuple_set_item: sym!("PyTuple_SetItem", PyTupleSetItemFn),
            py_tuple_size: sym!("PyTuple_Size", PyTupleSizeFn),
            py_tuple_get_item: sym!("PyTuple_GetItem", PyTupleGetItemFn),
            py_list_new: sym!("PyList_New", PyListNewFn),
            py_list_set_item: sym!("PyList_SetItem", PyListSetItemFn),
            py_list_size: sym!("PyList_Size", PyListSizeFn),
            py_list_get_item: sym!("PyList_GetItem", PyListGetItemFn),
            py_dict_new: sym!("PyDict_New", PyDictNewFn),
            py_dict_set_item: sym!("PyDict_SetItem", PyDictSetItemFn),
            py_dict_next: sym!("PyDict_Next", PyDictNextFn),
            py_none,
            py_true,
            py_false,
            py_bool_type,
            py_long_type,
            py_float_type,
            py_unicode_type,
            py_bytes_type,
            py_list_type,
            py_tuple_type,
            py_dict_type,
            _lib: lib,
        })
    }

    fn ensure_initialized(&self) {
        // SAFETY: CPython runtime API pointers are validated during bridge load.
        unsafe {
            if (self.py_is_initialized)() == 0 {
                (self.py_initialize_ex)(0);
            }
        }
    }

    fn with_gil<T>(&self, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.ensure_initialized();
        // SAFETY: CPython runtime API pointers are validated during bridge load.
        unsafe {
            let state = (self.pygil_ensure)();
            let result = f();
            (self.pygil_release)(state);
            result
        }
    }

    fn decref_owned(&self, ptr: *mut PyObject) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: ptr is expected to be a valid CPython object pointer.
        unsafe {
            (self.py_dec_ref)(ptr);
        }
    }

    fn incref_owned(&self, ptr: *mut PyObject) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: ptr is expected to be a valid CPython object pointer.
        unsafe {
            (self.py_inc_ref)(ptr);
        }
    }

    fn last_exception_message(&self, prefix: &str) -> String {
        // SAFETY: CPython runtime API pointers are validated during bridge load.
        unsafe {
            if (self.py_err_occurred)().is_null() {
                return prefix.to_string();
            }
            let mut ptype: *mut PyObject = std::ptr::null_mut();
            let mut pvalue: *mut PyObject = std::ptr::null_mut();
            let mut ptrace: *mut PyObject = std::ptr::null_mut();
            (self.py_err_fetch)(&mut ptype, &mut pvalue, &mut ptrace);
            (self.py_err_normalize_exception)(&mut ptype, &mut pvalue, &mut ptrace);
            let message = if pvalue.is_null() {
                format!("{prefix}: <unknown python error>")
            } else {
                let rendered = (self.py_object_str)(pvalue);
                let rendered_message = if rendered.is_null() {
                    format!("{prefix}: <unrenderable python error>")
                } else {
                    let mut size = 0isize;
                    let bytes = (self.py_unicode_as_utf8_and_size)(rendered, &mut size);
                    if bytes.is_null() {
                        format!("{prefix}: <utf8 decode error>")
                    } else {
                        let slice = std::slice::from_raw_parts(bytes.cast::<u8>(), size as usize);
                        let text = String::from_utf8_lossy(slice).into_owned();
                        format!("{prefix}: {text}")
                    }
                };
                self.decref_owned(rendered);
                rendered_message
            };
            self.decref_owned(ptype);
            self.decref_owned(pvalue);
            self.decref_owned(ptrace);
            (self.py_err_clear)();
            message
        }
    }

    fn is_instance_of(&self, obj: *mut PyObject, ty: *mut PyObject) -> Result<bool, String> {
        // SAFETY: pointers are CPython object pointers while GIL is held.
        unsafe {
            let status = (self.py_object_is_instance)(obj, ty);
            if status < 0 {
                return Err(self.last_exception_message("PyObject_IsInstance failed"));
            }
            Ok(status == 1)
        }
    }

    fn import_module(&self, name: &str) -> Result<*mut PyObject, String> {
        let c_name = CString::new(name)
            .map_err(|_| format!("module name '{name}' contains interior NUL byte"))?;
        // SAFETY: c_name is NUL-terminated and valid for the call.
        unsafe {
            let module = (self.py_import_import_module)(c_name.as_ptr());
            if module.is_null() {
                return Err(self.last_exception_message(&format!(
                    "CPython import failed for module '{name}'"
                )));
            }
            Ok(module)
        }
    }

    fn object_get_attr(
        &self,
        obj: *mut PyObject,
        attr_name: &str,
    ) -> Result<*mut PyObject, String> {
        let c_name = CString::new(attr_name)
            .map_err(|_| format!("attribute name '{attr_name}' contains interior NUL byte"))?;
        // SAFETY: pointers are valid while GIL is held.
        unsafe {
            let attr = (self.py_object_get_attr_string)(obj, c_name.as_ptr());
            if attr.is_null() {
                return Err(self.last_exception_message(&format!(
                    "CPython attribute lookup failed for '{attr_name}'"
                )));
            }
            Ok(attr)
        }
    }

    fn object_call(
        &self,
        callable: *mut PyObject,
        args: *mut PyObject,
        kwargs: *mut PyObject,
    ) -> Result<*mut PyObject, String> {
        // SAFETY: pointers are valid CPython objects while GIL is held.
        unsafe {
            let value = (self.py_object_call)(callable, args, kwargs);
            if value.is_null() {
                return Err(self.last_exception_message("CPython callable invocation failed"));
            }
            Ok(value)
        }
    }
}

#[cfg(unix)]
struct BridgeLibraryHandle {
    raw: *mut c_void,
}

#[cfg(unix)]
impl BridgeLibraryHandle {
    fn open(path: &PathBuf) -> Result<Self, String> {
        use std::os::unix::ffi::OsStrExt;

        let path_c = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            format!(
                "invalid libpython path '{}': contains NUL byte",
                path.display()
            )
        })?;
        let flags = RTLD_NOW | RTLD_GLOBAL;
        // SAFETY: path_c is a valid C string for dlopen.
        let raw = unsafe { dlopen(path_c.as_ptr(), flags) };
        if raw.is_null() {
            return Err(last_dl_error());
        }
        Ok(Self { raw })
    }

    fn symbol<T>(&self, name: &str) -> Result<T, String>
    where
        T: Copy,
    {
        let ptr = self.symbol_raw(name)?;
        // SAFETY: caller chooses T to match the target symbol signature.
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }

    fn symbol_raw(&self, name: &str) -> Result<*mut c_void, String> {
        let symbol_c = CString::new(name)
            .map_err(|_| format!("invalid symbol name '{name}': contains NUL byte"))?;
        // SAFETY: clear previous thread-local loader error.
        unsafe {
            let _ = dlerror();
        }
        // SAFETY: self.raw and symbol name are valid.
        let ptr = unsafe { dlsym(self.raw, symbol_c.as_ptr()) };
        // SAFETY: dlerror returns null or a valid C string.
        let err_ptr = unsafe { dlerror() };
        if !err_ptr.is_null() {
            // SAFETY: err_ptr is a valid C string as returned by dlerror.
            let err = unsafe { CStr::from_ptr(err_ptr) }
                .to_string_lossy()
                .into_owned();
            return Err(format!("failed to resolve symbol '{name}': {err}"));
        }
        if ptr.is_null() {
            return Err(format!("symbol '{name}' resolved to null"));
        }
        Ok(ptr)
    }
}

#[cfg(unix)]
impl Drop for BridgeLibraryHandle {
    fn drop(&mut self) {
        // SAFETY: handle was returned by dlopen when BridgeLibraryHandle was created.
        unsafe {
            let _ = dlclose(self.raw);
        }
    }
}

#[cfg(unix)]
fn last_dl_error() -> String {
    // SAFETY: dlerror returns null or thread-local C string.
    let ptr = unsafe { dlerror() };
    if ptr.is_null() {
        "unknown dynamic loader error".to_string()
    } else {
        // SAFETY: pointer from dlerror is valid NUL-terminated bytes.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
    fn dlerror() -> *const c_char;
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn parse_module_allowlist(raw: &str) -> Vec<String> {
    let mut modules = Vec::new();
    for token in raw.split(',') {
        let normalized = token.trim();
        if normalized.is_empty() {
            continue;
        }
        if modules
            .iter()
            .any(|existing: &String| existing == normalized)
        {
            continue;
        }
        modules.push(normalized.to_string());
    }
    modules
}

fn cpython_bridge_module_allowlist() -> Vec<String> {
    if let Ok(raw) = std::env::var(CPYTHON_PROXY_MODULES_ENV) {
        let parsed = parse_module_allowlist(&raw);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    CPYTHON_PROXY_DEFAULT_MODULE_PREFIXES
        .iter()
        .map(|module| module.to_string())
        .collect()
}

fn module_name_in_allowlist(module_name: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|prefix| {
        module_name == prefix
            || module_name
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('.'))
    })
}

fn libpython_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(explicit) = std::env::var_os("PYRS_CPYTHON_LIBPYTHON") {
        out.push(PathBuf::from(explicit));
    }
    out.push(PathBuf::from(
        "/Library/Frameworks/Python.framework/Versions/3.14/Python",
    ));
    out.push(PathBuf::from(
        "/Library/Frameworks/Python.framework/Versions/3.14/lib/libpython3.14.dylib",
    ));
    out.push(PathBuf::from("libpython3.14.dylib"));
    out.push(PathBuf::from("libpython3.14.so.1.0"));
    out.push(PathBuf::from("libpython3.14.so"));
    out
}

impl Vm {
    pub(super) fn cpython_abi_bridge_enabled_for_module(&self, module_name: &str) -> bool {
        if !env_flag_enabled(CPYTHON_PROXY_MODE_ENV) {
            return false;
        }
        let allowlist = cpython_bridge_module_allowlist();
        module_name_in_allowlist(module_name, &allowlist)
    }

    pub(super) fn ensure_cpython_abi_bridge(&mut self) -> Result<(), RuntimeError> {
        if self.cpython_abi_bridge.is_some() {
            return Ok(());
        }
        let bridge = CpythonAbiBridge::load().map_err(RuntimeError::new)?;
        self.cpython_abi_bridge = Some(bridge);
        Ok(())
    }

    pub(super) fn release_cpython_proxy_registry(&mut self) {
        let pointers = self.take_cpython_proxy_registry_pointers();
        let Some(bridge) = self.cpython_abi_bridge.as_ref() else {
            return;
        };
        let _ = bridge.with_gil(|| {
            for ptr in pointers {
                bridge.decref_owned(ptr as *mut PyObject);
            }
            Ok(())
        });
    }

    fn take_cpython_proxy_registry_pointers(&mut self) -> Vec<usize> {
        let pointers = self
            .cpython_proxy_registry
            .values()
            .copied()
            .collect::<Vec<_>>();
        self.cpython_proxy_registry.clear();
        pointers
    }

    fn ensure_cpython_proxy_class(&mut self) -> ObjRef {
        if let Some(class) = self.cpython_proxy_class.clone() {
            return class;
        }
        let class = match self.heap.alloc_class(ClassObject::new(
            CPYTHON_PROXY_CLASS_NAME.to_string(),
            Vec::new(),
        )) {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *class.kind_mut() {
            class_data
                .attrs
                .insert("__module__".to_string(), Value::Str("builtins".to_string()));
            class_data.attrs.insert(
                "__getattribute__".to_string(),
                Value::Builtin(BuiltinFunction::CpythonProxyGetAttribute),
            );
            class_data.attrs.insert(
                "__call__".to_string(),
                Value::Builtin(BuiltinFunction::CpythonProxyCall),
            );
            class_data.attrs.insert(
                "__repr__".to_string(),
                Value::Builtin(BuiltinFunction::CpythonProxyRepr),
            );
            class_data.attrs.insert(
                "__str__".to_string(),
                Value::Builtin(BuiltinFunction::CpythonProxyStr),
            );
            class_data.attrs.insert(
                "__bool__".to_string(),
                Value::Builtin(BuiltinFunction::CpythonProxyBool),
            );
            class_data.attrs.insert(
                "__int__".to_string(),
                Value::Builtin(BuiltinFunction::CpythonProxyInt),
            );
            class_data.attrs.insert(
                "__float__".to_string(),
                Value::Builtin(BuiltinFunction::CpythonProxyFloat),
            );
        }
        self.cpython_proxy_class = Some(class.clone());
        class
    }

    fn register_cpython_proxy_object(&mut self, ptr: *mut PyObject) -> Result<u64, RuntimeError> {
        let id = self.next_cpython_proxy_id;
        self.next_cpython_proxy_id = self.next_cpython_proxy_id.saturating_add(1);
        if id == 0 {
            return Err(RuntimeError::new("cpython proxy id overflow"));
        }
        self.cpython_proxy_registry.insert(id, ptr as usize);
        Ok(id)
    }

    fn cpython_proxy_ptr_from_value(&self, value: &Value) -> Option<*mut PyObject> {
        let proxy_id = match value {
            Value::Instance(instance) => {
                let Object::Instance(instance_data) = &*instance.kind() else {
                    return None;
                };
                match instance_data.attrs.get(CPYTHON_PROXY_ID_ATTR) {
                    Some(Value::Int(id)) if *id > 0 => *id as u64,
                    _ => return None,
                }
            }
            Value::Module(module) => {
                let Object::Module(module_data) = &*module.kind() else {
                    return None;
                };
                match module_data.globals.get(CPYTHON_PROXY_ID_ATTR) {
                    Some(Value::Int(id)) if *id > 0 => *id as u64,
                    _ => return None,
                }
            }
            _ => return None,
        };
        self.cpython_proxy_registry
            .get(&proxy_id)
            .copied()
            .map(|ptr| ptr as *mut PyObject)
    }

    fn wrap_cpython_proxy_value(&mut self, ptr: *mut PyObject) -> Result<Value, RuntimeError> {
        let proxy_id = self.register_cpython_proxy_object(ptr)?;
        let class = self.ensure_cpython_proxy_class();
        let mut instance = InstanceObject::new(class);
        instance.attrs.insert(
            CPYTHON_PROXY_ID_ATTR.to_string(),
            Value::Int(proxy_id as i64),
        );
        Ok(self.heap.alloc_instance(instance))
    }

    fn cpython_obj_to_value_owned(
        &mut self,
        bridge: &CpythonAbiBridge,
        ptr: *mut PyObject,
        depth: usize,
    ) -> Result<Value, RuntimeError> {
        if depth > 64 {
            bridge.decref_owned(ptr);
            return Err(RuntimeError::new(
                "cpython object conversion exceeded maximum recursion depth",
            ));
        }
        if ptr.is_null() {
            return Err(RuntimeError::new("null cpython object conversion"));
        }
        if ptr == bridge.py_none {
            bridge.decref_owned(ptr);
            return Ok(Value::None);
        }
        if ptr == bridge.py_true {
            bridge.decref_owned(ptr);
            return Ok(Value::Bool(true));
        }
        if ptr == bridge.py_false {
            bridge.decref_owned(ptr);
            return Ok(Value::Bool(false));
        }

        if bridge
            .is_instance_of(ptr, bridge.py_long_type)
            .map_err(RuntimeError::new)?
            && !bridge
                .is_instance_of(ptr, bridge.py_bool_type)
                .map_err(RuntimeError::new)?
        {
            // SAFETY: ptr is a live python int object while GIL is held.
            unsafe {
                let mut overflow = 0;
                let value = (bridge.py_long_as_longlong_and_overflow)(ptr, &mut overflow);
                let had_error = !(bridge.py_err_occurred)().is_null();
                if !had_error && overflow == 0 {
                    bridge.decref_owned(ptr);
                    return Ok(Value::Int(value));
                }
                if had_error {
                    (bridge.py_err_clear)();
                }
                let rendered = (bridge.py_object_str)(ptr);
                if rendered.is_null() {
                    bridge.decref_owned(ptr);
                    return Err(RuntimeError::new(
                        bridge.last_exception_message("failed to render cpython int"),
                    ));
                }
                let mut size = 0isize;
                let raw = (bridge.py_unicode_as_utf8_and_size)(rendered, &mut size);
                if raw.is_null() {
                    bridge.decref_owned(rendered);
                    bridge.decref_owned(ptr);
                    return Err(RuntimeError::new(
                        bridge.last_exception_message("failed to decode cpython int"),
                    ));
                }
                let slice = std::slice::from_raw_parts(raw.cast::<u8>(), size as usize);
                let text = String::from_utf8_lossy(slice).into_owned();
                bridge.decref_owned(rendered);
                bridge.decref_owned(ptr);
                let (negative, digits) = if let Some(rest) = text.strip_prefix('-') {
                    (true, rest)
                } else if let Some(rest) = text.strip_prefix('+') {
                    (false, rest)
                } else {
                    (false, text.as_str())
                };
                let mut big = BigInt::from_str_radix(digits, 10).ok_or_else(|| {
                    RuntimeError::new(format!(
                        "failed to parse cpython int literal from bridge: '{text}'"
                    ))
                })?;
                if negative {
                    big = big.negated();
                }
                return Ok(Value::BigInt(Box::new(big)));
            }
        }

        if bridge
            .is_instance_of(ptr, bridge.py_float_type)
            .map_err(RuntimeError::new)?
        {
            // SAFETY: ptr is a live python float object while GIL is held.
            unsafe {
                let value = (bridge.py_float_as_double)(ptr);
                if !(bridge.py_err_occurred)().is_null() {
                    let message = bridge.last_exception_message("failed to decode cpython float");
                    bridge.decref_owned(ptr);
                    return Err(RuntimeError::new(message));
                }
                bridge.decref_owned(ptr);
                return Ok(Value::Float(value));
            }
        }

        if bridge
            .is_instance_of(ptr, bridge.py_unicode_type)
            .map_err(RuntimeError::new)?
        {
            // SAFETY: ptr is a live python unicode object while GIL is held.
            unsafe {
                let mut size = 0isize;
                let raw = (bridge.py_unicode_as_utf8_and_size)(ptr, &mut size);
                if raw.is_null() {
                    let message =
                        bridge.last_exception_message("failed to decode cpython unicode object");
                    bridge.decref_owned(ptr);
                    return Err(RuntimeError::new(message));
                }
                let slice = std::slice::from_raw_parts(raw.cast::<u8>(), size as usize);
                let value = String::from_utf8_lossy(slice).into_owned();
                bridge.decref_owned(ptr);
                return Ok(Value::Str(value));
            }
        }

        if bridge
            .is_instance_of(ptr, bridge.py_bytes_type)
            .map_err(RuntimeError::new)?
        {
            // SAFETY: ptr is a live python bytes object while GIL is held.
            unsafe {
                let mut raw: *mut c_char = std::ptr::null_mut();
                let mut size = 0isize;
                let status = (bridge.py_bytes_as_string_and_size)(ptr, &mut raw, &mut size);
                if status != 0 || raw.is_null() {
                    let message =
                        bridge.last_exception_message("failed to decode cpython bytes object");
                    bridge.decref_owned(ptr);
                    return Err(RuntimeError::new(message));
                }
                let data = std::slice::from_raw_parts(raw.cast::<u8>(), size as usize).to_vec();
                bridge.decref_owned(ptr);
                return Ok(self.heap.alloc_bytes(data));
            }
        }

        if bridge
            .is_instance_of(ptr, bridge.py_list_type)
            .map_err(RuntimeError::new)?
        {
            // SAFETY: ptr is a live python list object while GIL is held.
            unsafe {
                let size = (bridge.py_list_size)(ptr);
                if size < 0 {
                    let message = bridge.last_exception_message("failed to read cpython list size");
                    bridge.decref_owned(ptr);
                    return Err(RuntimeError::new(message));
                }
                let mut values = Vec::with_capacity(size as usize);
                for index in 0..size {
                    let item = (bridge.py_list_get_item)(ptr, index);
                    if item.is_null() {
                        let message =
                            bridge.last_exception_message("failed to read cpython list item");
                        bridge.decref_owned(ptr);
                        return Err(RuntimeError::new(message));
                    }
                    bridge.incref_owned(item);
                    values.push(self.cpython_obj_to_value_owned(bridge, item, depth + 1)?);
                }
                bridge.decref_owned(ptr);
                return Ok(self.heap.alloc_list(values));
            }
        }

        if bridge
            .is_instance_of(ptr, bridge.py_tuple_type)
            .map_err(RuntimeError::new)?
        {
            // SAFETY: ptr is a live python tuple object while GIL is held.
            unsafe {
                let size = (bridge.py_tuple_size)(ptr);
                if size < 0 {
                    let message =
                        bridge.last_exception_message("failed to read cpython tuple size");
                    bridge.decref_owned(ptr);
                    return Err(RuntimeError::new(message));
                }
                let mut values = Vec::with_capacity(size as usize);
                for index in 0..size {
                    let item = (bridge.py_tuple_get_item)(ptr, index);
                    if item.is_null() {
                        let message =
                            bridge.last_exception_message("failed to read cpython tuple item");
                        bridge.decref_owned(ptr);
                        return Err(RuntimeError::new(message));
                    }
                    bridge.incref_owned(item);
                    values.push(self.cpython_obj_to_value_owned(bridge, item, depth + 1)?);
                }
                bridge.decref_owned(ptr);
                return Ok(self.heap.alloc_tuple(values));
            }
        }

        if bridge
            .is_instance_of(ptr, bridge.py_dict_type)
            .map_err(RuntimeError::new)?
        {
            // SAFETY: ptr is a live python dict object while GIL is held.
            unsafe {
                let mut position = 0isize;
                let mut entries = Vec::new();
                loop {
                    let mut key: *mut PyObject = std::ptr::null_mut();
                    let mut value: *mut PyObject = std::ptr::null_mut();
                    let status = (bridge.py_dict_next)(ptr, &mut position, &mut key, &mut value);
                    if status == 0 {
                        break;
                    }
                    if key.is_null() || value.is_null() {
                        continue;
                    }
                    bridge.incref_owned(key);
                    bridge.incref_owned(value);
                    let key_value = self.cpython_obj_to_value_owned(bridge, key, depth + 1)?;
                    let value_value = self.cpython_obj_to_value_owned(bridge, value, depth + 1)?;
                    entries.push((key_value, value_value));
                }
                bridge.decref_owned(ptr);
                return Ok(self.heap.alloc_dict(entries));
            }
        }

        self.wrap_cpython_proxy_value(ptr)
    }

    fn value_to_cpython_object(
        &mut self,
        bridge: &CpythonAbiBridge,
        value: &Value,
        depth: usize,
    ) -> Result<*mut PyObject, RuntimeError> {
        if depth > 64 {
            return Err(RuntimeError::new(
                "cpython object conversion exceeded maximum recursion depth",
            ));
        }
        if let Some(ptr) = self.cpython_proxy_ptr_from_value(value) {
            bridge.incref_owned(ptr);
            return Ok(ptr);
        }
        // SAFETY: CPython API pointers are valid and called while GIL is held.
        unsafe {
            match value {
                Value::None => {
                    bridge.incref_owned(bridge.py_none);
                    Ok(bridge.py_none)
                }
                Value::Bool(flag) => {
                    let object = (bridge.py_bool_from_long)(if *flag { 1 } else { 0 });
                    if object.is_null() {
                        Err(RuntimeError::new(bridge.last_exception_message(
                            "failed to convert bool to cpython object",
                        )))
                    } else {
                        Ok(object)
                    }
                }
                Value::Int(integer) => {
                    let object = (bridge.py_long_from_longlong)(*integer);
                    if object.is_null() {
                        Err(RuntimeError::new(bridge.last_exception_message(
                            "failed to convert int to cpython object",
                        )))
                    } else {
                        Ok(object)
                    }
                }
                Value::BigInt(big) => {
                    let decimal = big.to_string();
                    let c_decimal = CString::new(decimal.as_str()).map_err(|_| {
                        RuntimeError::new("bigint decimal conversion contains NUL byte")
                    })?;
                    let mut endptr: *mut c_char = std::ptr::null_mut();
                    let object = (bridge.py_long_from_string)(c_decimal.as_ptr(), &mut endptr, 10);
                    if object.is_null() {
                        Err(RuntimeError::new(bridge.last_exception_message(
                            "failed to convert bigint to cpython object",
                        )))
                    } else {
                        Ok(object)
                    }
                }
                Value::Float(number) => {
                    let object = (bridge.py_float_from_double)(*number);
                    if object.is_null() {
                        Err(RuntimeError::new(bridge.last_exception_message(
                            "failed to convert float to cpython object",
                        )))
                    } else {
                        Ok(object)
                    }
                }
                Value::Str(text) => {
                    let c_text = CString::new(text.as_str()).map_err(|_| {
                        RuntimeError::new("string conversion contains interior NUL byte")
                    })?;
                    let object = (bridge.py_unicode_from_string_and_size)(
                        c_text.as_ptr(),
                        text.len() as isize,
                    );
                    if object.is_null() {
                        Err(RuntimeError::new(bridge.last_exception_message(
                            "failed to convert str to cpython object",
                        )))
                    } else {
                        Ok(object)
                    }
                }
                Value::Bytes(bytes_obj) => {
                    let Object::Bytes(bytes) = &*bytes_obj.kind() else {
                        return Err(RuntimeError::new("bytes conversion expected bytes object"));
                    };
                    let object = (bridge.py_bytes_from_string_and_size)(
                        bytes.as_ptr().cast::<c_char>(),
                        bytes.len() as isize,
                    );
                    if object.is_null() {
                        Err(RuntimeError::new(bridge.last_exception_message(
                            "failed to convert bytes to cpython object",
                        )))
                    } else {
                        Ok(object)
                    }
                }
                Value::List(list_obj) => {
                    let Object::List(items) = &*list_obj.kind() else {
                        return Err(RuntimeError::new("list conversion expected list object"));
                    };
                    let list = (bridge.py_list_new)(items.len() as isize);
                    if list.is_null() {
                        return Err(RuntimeError::new(
                            bridge.last_exception_message("failed to allocate cpython list"),
                        ));
                    }
                    for (index, item) in items.iter().enumerate() {
                        let item_obj = self.value_to_cpython_object(bridge, item, depth + 1)?;
                        let status = (bridge.py_list_set_item)(list, index as isize, item_obj);
                        if status != 0 {
                            bridge.decref_owned(item_obj);
                            bridge.decref_owned(list);
                            return Err(RuntimeError::new(bridge.last_exception_message(
                                "failed to append list item to cpython list",
                            )));
                        }
                    }
                    Ok(list)
                }
                Value::Tuple(tuple_obj) => {
                    let Object::Tuple(items) = &*tuple_obj.kind() else {
                        return Err(RuntimeError::new("tuple conversion expected tuple object"));
                    };
                    let tuple = (bridge.py_tuple_new)(items.len() as isize);
                    if tuple.is_null() {
                        return Err(RuntimeError::new(
                            bridge.last_exception_message("failed to allocate cpython tuple"),
                        ));
                    }
                    for (index, item) in items.iter().enumerate() {
                        let item_obj = self.value_to_cpython_object(bridge, item, depth + 1)?;
                        let status = (bridge.py_tuple_set_item)(tuple, index as isize, item_obj);
                        if status != 0 {
                            bridge.decref_owned(item_obj);
                            bridge.decref_owned(tuple);
                            return Err(RuntimeError::new(bridge.last_exception_message(
                                "failed to append tuple item to cpython tuple",
                            )));
                        }
                    }
                    Ok(tuple)
                }
                Value::Dict(dict_obj) => {
                    let Object::Dict(entries) = &*dict_obj.kind() else {
                        return Err(RuntimeError::new("dict conversion expected dict object"));
                    };
                    let dict = (bridge.py_dict_new)();
                    if dict.is_null() {
                        return Err(RuntimeError::new(
                            bridge.last_exception_message("failed to allocate cpython dict"),
                        ));
                    }
                    for (key, value) in entries.iter() {
                        let py_key = self.value_to_cpython_object(bridge, key, depth + 1)?;
                        let py_value = self.value_to_cpython_object(bridge, value, depth + 1)?;
                        let status = (bridge.py_dict_set_item)(dict, py_key, py_value);
                        bridge.decref_owned(py_key);
                        bridge.decref_owned(py_value);
                        if status != 0 {
                            bridge.decref_owned(dict);
                            return Err(RuntimeError::new(
                                bridge.last_exception_message("failed to set cpython dict item"),
                            ));
                        }
                    }
                    Ok(dict)
                }
                _ => Err(RuntimeError::new(format!(
                    "cpython bridge conversion for type '{}' is not implemented",
                    self.value_type_name_for_error(value)
                ))),
            }
        }
    }

    pub(super) fn import_module_via_cpython_abi_bridge(
        &mut self,
        name: &str,
    ) -> Result<ObjRef, RuntimeError> {
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        let (module_ptr, module_file, module_package, module_path) = bridge
            .with_gil(|| {
                let module = bridge.import_module(name)?;
                let file = match bridge.object_get_attr(module, "__file__") {
                    Ok(obj) => Some(obj),
                    Err(_) => None,
                };
                let package = match bridge.object_get_attr(module, "__package__") {
                    Ok(obj) => Some(obj),
                    Err(_) => None,
                };
                let path = match bridge.object_get_attr(module, "__path__") {
                    Ok(obj) => Some(obj),
                    Err(_) => None,
                };
                Ok((module, file, package, path))
            })
            .map_err(RuntimeError::new)?;

        let module_obj = match self.heap.alloc_module(ModuleObject::new(name)) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        self.set_module_metadata(
            &module_obj,
            name,
            None,
            Some("pyrs.CPythonAbiBridgeLoader"),
            module_path.is_some(),
            Vec::new(),
            false,
        );
        if let Object::Module(module_data) = &mut *module_obj.kind_mut() {
            let proxy_id = self.register_cpython_proxy_object(module_ptr)?;
            module_data.globals.insert(
                CPYTHON_PROXY_ID_ATTR.to_string(),
                Value::Int(proxy_id as i64),
            );
            module_data.globals.insert(
                "__getattr__".to_string(),
                self.alloc_builtin_bound_method(
                    BuiltinFunction::CpythonProxyModuleGetAttr,
                    module_obj.clone(),
                ),
            );
            module_data.globals.insert(
                "__pyrs_extension_symbol_family__".to_string(),
                Value::Str("cpython-abi-bridge".to_string()),
            );
            module_data.globals.insert(
                "__pyrs_extension_entrypoint__".to_string(),
                Value::Str("PyImport_ImportModule".to_string()),
            );
            module_data
                .globals
                .insert("__name__".to_string(), Value::Str(name.to_string()));
        }

        if let Some(file_ptr) = module_file {
            let value = bridge
                .with_gil(|| {
                    self.cpython_obj_to_value_owned(bridge, file_ptr, 0)
                        .map_err(|err| err.message)
                })
                .map_err(RuntimeError::new)?;
            if let Object::Module(module_data) = &mut *module_obj.kind_mut() {
                module_data.globals.insert("__file__".to_string(), value);
            }
        }

        if let Some(package_ptr) = module_package {
            let value = bridge
                .with_gil(|| {
                    self.cpython_obj_to_value_owned(bridge, package_ptr, 0)
                        .map_err(|err| err.message)
                })
                .map_err(RuntimeError::new)?;
            if let Object::Module(module_data) = &mut *module_obj.kind_mut() {
                module_data.globals.insert("__package__".to_string(), value);
            }
        }
        if let Some(path_ptr) = module_path {
            let value = bridge
                .with_gil(|| {
                    self.cpython_obj_to_value_owned(bridge, path_ptr, 0)
                        .map_err(|err| err.message)
                })
                .map_err(RuntimeError::new)?;
            if let Object::Module(module_data) = &mut *module_obj.kind_mut() {
                module_data.globals.insert("__path__".to_string(), value);
            }
        }

        self.register_module(name, module_obj.clone());
        self.link_module_chain(name, module_obj.clone());
        if let Some(modules_dict) = self.sys_dict_obj("modules") {
            dict_set_value(
                &modules_dict,
                Value::Str(name.to_string()),
                Value::Module(module_obj.clone()),
            );
        }
        Ok(module_obj)
    }

    fn cpython_proxy_get_attr_inner(
        &mut self,
        receiver: &Value,
        attr_name: &str,
    ) -> Result<Value, RuntimeError> {
        let ptr = self
            .cpython_proxy_ptr_from_value(receiver)
            .ok_or_else(|| RuntimeError::new("cpython proxy receiver is not valid"))?;
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        bridge
            .with_gil(|| {
                let attr = bridge.object_get_attr(ptr, attr_name)?;
                self.cpython_obj_to_value_owned(bridge, attr, 0)
                    .map_err(|err| err.message)
            })
            .map_err(RuntimeError::new)
    }

    pub(super) fn builtin_cpython_proxy_module_getattr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__getattr__() got unexpected keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new(
                "__getattr__() expects module and attribute name",
            ));
        }
        let module = args
            .first()
            .cloned()
            .ok_or_else(|| RuntimeError::new("__getattr__() missing module receiver"))?;
        let attr_name = match args.get(1) {
            Some(Value::Str(name)) => name.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "__getattr__() attribute name must be str",
                ));
            }
        };
        let value = self.cpython_proxy_get_attr_inner(&module, &attr_name)?;
        if let Value::Module(module_obj) = module
            && let Object::Module(module_data) = &mut *module_obj.kind_mut()
        {
            module_data.globals.insert(attr_name, value.clone());
        }
        Ok(value)
    }

    pub(super) fn builtin_cpython_proxy_getattribute(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__getattribute__() got unexpected keyword arguments",
            ));
        }
        if args.len() != 2 {
            return Err(RuntimeError::new(
                "__getattribute__() expects object and attribute name",
            ));
        }
        let receiver = args
            .first()
            .cloned()
            .ok_or_else(|| RuntimeError::new("__getattribute__() missing receiver"))?;
        let attr_name = match args.get(1) {
            Some(Value::Str(name)) => name.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "__getattribute__() attribute name must be str",
                ));
            }
        };
        if attr_name == CPYTHON_PROXY_ID_ATTR || attr_name == "__class__" || attr_name == "__dict__"
        {
            return self.builtin_object_getattribute(args, HashMap::new());
        }
        self.cpython_proxy_get_attr_inner(&receiver, &attr_name)
    }

    pub(super) fn builtin_cpython_proxy_call(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (receiver, call_args) = args
            .split_first()
            .ok_or_else(|| RuntimeError::new("__call__() missing receiver"))?;
        let ptr = self
            .cpython_proxy_ptr_from_value(receiver)
            .ok_or_else(|| RuntimeError::new("invalid cpython proxy receiver"))?;
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        bridge
            .with_gil(|| {
                // SAFETY: all CPython API calls occur while GIL is held.
                unsafe {
                    if (bridge.py_callable_check)(ptr) != 1 {
                        return Err("cpython proxy target is not callable".to_string());
                    }
                }
                let tuple = unsafe { (bridge.py_tuple_new)(call_args.len() as isize) };
                if tuple.is_null() {
                    return Err(bridge.last_exception_message(
                        "failed to allocate call tuple in cpython bridge",
                    ));
                }
                for (index, arg) in call_args.iter().enumerate() {
                    let py_arg = self
                        .value_to_cpython_object(bridge, arg, 0)
                        .map_err(|err| err.message)?;
                    // SAFETY: tuple is valid and index in-bounds.
                    let set_status =
                        unsafe { (bridge.py_tuple_set_item)(tuple, index as isize, py_arg) };
                    if set_status != 0 {
                        bridge.decref_owned(py_arg);
                        bridge.decref_owned(tuple);
                        return Err(bridge
                            .last_exception_message("failed to set cpython call tuple argument"));
                    }
                }

                let kwargs_obj =
                    if kwargs.is_empty() {
                        std::ptr::null_mut()
                    } else {
                        let dict = unsafe { (bridge.py_dict_new)() };
                        if dict.is_null() {
                            bridge.decref_owned(tuple);
                            return Err(bridge
                                .last_exception_message("failed to allocate cpython kwargs dict"));
                        }
                        for (name, value) in kwargs.iter() {
                            let key = self
                                .value_to_cpython_object(bridge, &Value::Str(name.clone()), 0)
                                .map_err(|err| err.message)?;
                            let py_value = self
                                .value_to_cpython_object(bridge, value, 0)
                                .map_err(|err| err.message)?;
                            // SAFETY: dict/key/value are valid CPython objects.
                            let status = unsafe { (bridge.py_dict_set_item)(dict, key, py_value) };
                            bridge.decref_owned(key);
                            bridge.decref_owned(py_value);
                            if status != 0 {
                                bridge.decref_owned(dict);
                                bridge.decref_owned(tuple);
                                return Err(bridge
                                    .last_exception_message("failed to set cpython kwargs item"));
                            }
                        }
                        dict
                    };

                let value = bridge.object_call(ptr, tuple, kwargs_obj);
                bridge.decref_owned(tuple);
                bridge.decref_owned(kwargs_obj);
                let value = value?;
                self.cpython_obj_to_value_owned(bridge, value, 0)
                    .map_err(|err| err.message)
            })
            .map_err(RuntimeError::new)
    }

    pub(super) fn builtin_cpython_proxy_repr(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__repr__() got unexpected keyword arguments",
            ));
        }
        let receiver = args
            .first()
            .ok_or_else(|| RuntimeError::new("__repr__() missing receiver"))?;
        let ptr = self
            .cpython_proxy_ptr_from_value(receiver)
            .ok_or_else(|| RuntimeError::new("invalid cpython proxy receiver"))?;
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        bridge
            .with_gil(|| {
                // SAFETY: pointer and API are valid while GIL is held.
                let value = unsafe { (bridge.py_object_repr)(ptr) };
                if value.is_null() {
                    return Err(bridge.last_exception_message("cpython __repr__ failed"));
                }
                self.cpython_obj_to_value_owned(bridge, value, 0)
                    .map_err(|err| err.message)
            })
            .map_err(RuntimeError::new)
    }

    pub(super) fn builtin_cpython_proxy_str(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__str__() got unexpected keyword arguments",
            ));
        }
        let receiver = args
            .first()
            .ok_or_else(|| RuntimeError::new("__str__() missing receiver"))?;
        let ptr = self
            .cpython_proxy_ptr_from_value(receiver)
            .ok_or_else(|| RuntimeError::new("invalid cpython proxy receiver"))?;
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        bridge
            .with_gil(|| {
                // SAFETY: pointer and API are valid while GIL is held.
                let value = unsafe { (bridge.py_object_str)(ptr) };
                if value.is_null() {
                    return Err(bridge.last_exception_message("cpython __str__ failed"));
                }
                self.cpython_obj_to_value_owned(bridge, value, 0)
                    .map_err(|err| err.message)
            })
            .map_err(RuntimeError::new)
    }

    pub(super) fn builtin_cpython_proxy_bool(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__bool__() got unexpected keyword arguments",
            ));
        }
        let receiver = args
            .first()
            .ok_or_else(|| RuntimeError::new("__bool__() missing receiver"))?;
        let ptr = self
            .cpython_proxy_ptr_from_value(receiver)
            .ok_or_else(|| RuntimeError::new("invalid cpython proxy receiver"))?;
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        let truth = bridge
            .with_gil(|| {
                // SAFETY: pointer and API are valid while GIL is held.
                unsafe {
                    let status = (bridge.py_object_is_true)(ptr);
                    if status < 0 {
                        Err(bridge.last_exception_message("cpython __bool__ failed"))
                    } else {
                        Ok(status == 1)
                    }
                }
            })
            .map_err(RuntimeError::new)?;
        Ok(Value::Bool(truth))
    }

    pub(super) fn builtin_cpython_proxy_int(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__int__() got unexpected keyword arguments",
            ));
        }
        let receiver = args
            .first()
            .ok_or_else(|| RuntimeError::new("__int__() missing receiver"))?;
        let ptr = self
            .cpython_proxy_ptr_from_value(receiver)
            .ok_or_else(|| RuntimeError::new("invalid cpython proxy receiver"))?;
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        bridge
            .with_gil(|| {
                // SAFETY: pointer and API are valid while GIL is held.
                let value = unsafe { (bridge.py_number_long)(ptr) };
                if value.is_null() {
                    return Err(bridge.last_exception_message("cpython __int__ conversion failed"));
                }
                self.cpython_obj_to_value_owned(bridge, value, 0)
                    .map_err(|err| err.message)
            })
            .map_err(RuntimeError::new)
    }

    pub(super) fn builtin_cpython_proxy_float(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "__float__() got unexpected keyword arguments",
            ));
        }
        let receiver = args
            .first()
            .ok_or_else(|| RuntimeError::new("__float__() missing receiver"))?;
        let ptr = self
            .cpython_proxy_ptr_from_value(receiver)
            .ok_or_else(|| RuntimeError::new("invalid cpython proxy receiver"))?;
        self.ensure_cpython_abi_bridge()?;
        let bridge_ptr = self
            .cpython_abi_bridge
            .as_ref()
            .map(|bridge| bridge as *const CpythonAbiBridge)
            .ok_or_else(|| RuntimeError::new("cpython bridge is not initialized"))?;
        // SAFETY: bridge_ptr points to self-owned bridge storage that remains valid for
        // the duration of this method.
        let bridge = unsafe { &*bridge_ptr };
        bridge
            .with_gil(|| {
                // SAFETY: pointer and API are valid while GIL is held.
                let value = unsafe { (bridge.py_number_float)(ptr) };
                if value.is_null() {
                    return Err(
                        bridge.last_exception_message("cpython __float__ conversion failed")
                    );
                }
                self.cpython_obj_to_value_owned(bridge, value, 0)
                    .map_err(|err| err.message)
            })
            .map_err(RuntimeError::new)
    }
}

#[cfg(test)]
mod tests {
    use super::{Vm, module_name_in_allowlist, parse_module_allowlist};

    #[test]
    fn parse_module_allowlist_trims_and_deduplicates_entries() {
        let parsed = parse_module_allowlist(" numpy, scipy ,numpy, , pandas ,matplotlib ");
        assert_eq!(parsed, vec!["numpy", "scipy", "pandas", "matplotlib"]);
    }

    #[test]
    fn module_name_in_allowlist_matches_exact_and_submodule_names() {
        let allowlist = vec!["numpy".to_string(), "pandas".to_string()];
        assert!(module_name_in_allowlist("numpy", &allowlist));
        assert!(module_name_in_allowlist("numpy.linalg", &allowlist));
        assert!(module_name_in_allowlist("pandas.core.series", &allowlist));
        assert!(!module_name_in_allowlist("num", &allowlist));
        assert!(!module_name_in_allowlist("panda", &allowlist));
        assert!(!module_name_in_allowlist("pandasx.core", &allowlist));
    }

    #[test]
    fn taking_proxy_registry_pointers_clears_registry_state() {
        let mut vm = Vm::new();
        vm.cpython_proxy_registry.insert(1, 0x11);
        vm.cpython_proxy_registry.insert(2, 0x22);

        let mut pointers = vm.take_cpython_proxy_registry_pointers();
        pointers.sort_unstable();

        assert_eq!(pointers, vec![0x11, 0x22]);
        assert!(vm.cpython_proxy_registry.is_empty());
    }
}
