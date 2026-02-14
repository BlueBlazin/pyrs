use std::ffi::{CString, c_char, c_void};
use std::fs;
use std::path::{Path, PathBuf};

pub const PYRS_EXTENSION_MANIFEST_SUFFIX: &str = ".pyrs-ext";
pub const PYRS_EXTENSION_ABI_TAG: &str = "pyrs314";
pub const PYRS_CAPI_ABI_VERSION: u32 = 1;
pub const PYRS_DYNAMIC_INIT_SYMBOL_V1: &str = "pyrs_extension_init_v1";
pub type PyrsObjectHandle = u64;
pub const PYRS_TYPE_NONE: i32 = 1;
pub const PYRS_TYPE_BOOL: i32 = 2;
pub const PYRS_TYPE_INT: i32 = 3;
pub const PYRS_TYPE_STR: i32 = 4;
pub const PYRS_TYPE_FLOAT: i32 = 5;
pub const PYRS_TYPE_BYTES: i32 = 6;
pub const PYRS_TYPE_TUPLE: i32 = 7;
pub const PYRS_TYPE_LIST: i32 = 8;
pub const PYRS_TYPE_DICT: i32 = 9;

#[repr(C)]
pub struct PyrsBufferViewV1 {
    pub data: *const u8,
    pub len: usize,
    pub readonly: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionEntrypoint {
    HelloExt,
    DynamicSymbol(String),
}

impl ExtensionEntrypoint {
    pub fn as_str(&self) -> String {
        match self {
            Self::HelloExt => "hello_ext".to_string(),
            Self::DynamicSymbol(symbol) => format!("dynamic:{symbol}"),
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        if raw == "hello_ext" {
            return Some(Self::HelloExt);
        }
        raw.strip_prefix("dynamic:")
            .map(|symbol| Self::DynamicSymbol(symbol.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifest {
    pub module_name: String,
    pub abi_tag: String,
    pub entrypoint: ExtensionEntrypoint,
    pub library_path: Option<String>,
}

impl ExtensionManifest {
    pub fn resolve_library_path(&self, manifest_path: &Path) -> Option<PathBuf> {
        let library = self.library_path.as_ref()?;
        let path = PathBuf::from(library);
        if path.is_absolute() {
            Some(path)
        } else {
            Some(
                manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path),
            )
        }
    }
}

pub fn parse_extension_manifest(
    path: &Path,
    expected_module_name: &str,
) -> Result<ExtensionManifest, String> {
    let raw = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read extension manifest '{}': {err}",
            path.display()
        )
    })?;
    let mut module_name: Option<String> = None;
    let mut abi_tag: Option<String> = None;
    let mut entrypoint: Option<ExtensionEntrypoint> = None;
    let mut library_path: Option<String> = None;

    for (line_number, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            return Err(format!(
                "invalid manifest line {} in '{}': expected key=value",
                line_number + 1,
                path.display()
            ));
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "module" => module_name = Some(value.to_string()),
            "abi" => abi_tag = Some(value.to_string()),
            "entrypoint" => {
                let parsed = ExtensionEntrypoint::parse(value).ok_or_else(|| {
                    format!(
                        "unsupported extension entrypoint '{}' in '{}'",
                        value,
                        path.display()
                    )
                })?;
                entrypoint = Some(parsed);
            }
            "library" => library_path = Some(value.to_string()),
            other => {
                return Err(format!(
                    "unsupported manifest key '{}' in '{}'",
                    other,
                    path.display()
                ));
            }
        }
    }

    let module_name = module_name.ok_or_else(|| {
        format!(
            "missing required 'module' key in extension manifest '{}'",
            path.display()
        )
    })?;
    if module_name != expected_module_name {
        return Err(format!(
            "manifest module '{}' does not match import target '{}'",
            module_name, expected_module_name
        ));
    }

    let abi_tag = abi_tag.ok_or_else(|| {
        format!(
            "missing required 'abi' key in extension manifest '{}'",
            path.display()
        )
    })?;
    if abi_tag != PYRS_EXTENSION_ABI_TAG {
        return Err(format!(
            "unsupported extension ABI '{}'; expected '{}'",
            abi_tag, PYRS_EXTENSION_ABI_TAG
        ));
    }

    let entrypoint = entrypoint.ok_or_else(|| {
        format!(
            "missing required 'entrypoint' key in extension manifest '{}'",
            path.display()
        )
    })?;

    match &entrypoint {
        ExtensionEntrypoint::HelloExt => {
            if library_path.is_some() {
                return Err(format!(
                    "manifest '{}' cannot set 'library' for hello_ext entrypoint",
                    path.display()
                ));
            }
        }
        ExtensionEntrypoint::DynamicSymbol(_) => {
            if library_path.is_none() {
                return Err(format!(
                    "manifest '{}' requires 'library' for dynamic entrypoint",
                    path.display()
                ));
            }
        }
    }

    Ok(ExtensionManifest {
        module_name,
        abi_tag,
        entrypoint,
        library_path,
    })
}

pub fn shared_library_suffixes() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &[".pyd", ".dll"]
    }
    #[cfg(target_os = "macos")]
    {
        &[".so", ".dylib"]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[".so"]
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        &[".so"]
    }
}

pub fn shared_library_module_candidates(root: &Path, rel_module_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for suffix in shared_library_suffixes() {
        out.push(root.join(format!("{rel_module_name}{suffix}")));
    }
    out
}

pub fn shared_library_package_candidates(package_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for suffix in shared_library_suffixes() {
        out.push(package_dir.join(format!("__init__{suffix}")));
    }
    out
}

fn file_matches_shared_library_stem(file_name: &str, stem: &str) -> bool {
    for suffix in shared_library_suffixes() {
        if file_name == format!("{stem}{suffix}") {
            return true;
        }
        if file_name.starts_with(&format!("{stem}."))
            && file_name.ends_with(suffix)
            && file_name.len() > stem.len() + suffix.len() + 1
        {
            return true;
        }
    }
    false
}

pub fn find_shared_library_for_module(root: &Path, rel_module_name: &str) -> Option<PathBuf> {
    let rel = Path::new(rel_module_name);
    let stem = rel.file_name()?.to_str()?;
    let parent = rel.parent().unwrap_or_else(|| Path::new(""));
    let search_dir = root.join(parent);
    if !search_dir.is_dir() {
        return None;
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&search_dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_matches_shared_library_stem(file_name, stem) {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

pub fn find_shared_library_for_package(package_dir: &Path) -> Option<PathBuf> {
    if !package_dir.is_dir() {
        return None;
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(package_dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_matches_shared_library_stem(file_name, "__init__") {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

pub fn path_is_shared_library(path: &Path) -> bool {
    let text = path.to_string_lossy();
    for suffix in shared_library_suffixes() {
        if text.ends_with(suffix) {
            return true;
        }
    }
    false
}

#[repr(C)]
pub struct PyrsApiV1 {
    pub abi_version: u32,
    pub api_has_capability:
        unsafe extern "C" fn(module_ctx: *mut c_void, name: *const c_char) -> i32,
    pub module_set_int:
        unsafe extern "C" fn(module_ctx: *mut c_void, name: *const c_char, value: i64) -> i32,
    pub module_set_bool:
        unsafe extern "C" fn(module_ctx: *mut c_void, name: *const c_char, value: i32) -> i32,
    pub module_set_string: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        name: *const c_char,
        value: *const c_char,
    ) -> i32,
    pub module_add_function: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        name: *const c_char,
        callback: Option<PyrsCFunctionV1>,
    ) -> i32,
    pub module_add_function_kw: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        name: *const c_char,
        callback: Option<PyrsCFunctionKwV1>,
    ) -> i32,
    pub object_new_int:
        unsafe extern "C" fn(module_ctx: *mut c_void, value: i64) -> PyrsObjectHandle,
    pub object_new_none: unsafe extern "C" fn(module_ctx: *mut c_void) -> PyrsObjectHandle,
    pub object_new_bool:
        unsafe extern "C" fn(module_ctx: *mut c_void, value: i32) -> PyrsObjectHandle,
    pub object_new_float:
        unsafe extern "C" fn(module_ctx: *mut c_void, value: f64) -> PyrsObjectHandle,
    pub object_new_bytes: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        data: *const u8,
        len: usize,
    ) -> PyrsObjectHandle,
    pub object_new_tuple: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        len: usize,
        items: *const PyrsObjectHandle,
    ) -> PyrsObjectHandle,
    pub object_new_list: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        len: usize,
        items: *const PyrsObjectHandle,
    ) -> PyrsObjectHandle,
    pub object_new_dict: unsafe extern "C" fn(module_ctx: *mut c_void) -> PyrsObjectHandle,
    pub object_new_string:
        unsafe extern "C" fn(module_ctx: *mut c_void, value: *const c_char) -> PyrsObjectHandle,
    pub object_incref:
        unsafe extern "C" fn(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32,
    pub object_decref:
        unsafe extern "C" fn(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32,
    pub module_set_object: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        name: *const c_char,
        handle: PyrsObjectHandle,
    ) -> i32,
    pub module_get_object: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        name: *const c_char,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub module_import: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        module_name: *const c_char,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub module_get_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        module_handle: PyrsObjectHandle,
        attr_name: *const c_char,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_type: unsafe extern "C" fn(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> i32,
    pub object_is_instance: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        classinfo_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_is_subclass: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        class_handle: PyrsObjectHandle,
        classinfo_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_get_int: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out: *mut i64,
    ) -> i32,
    pub object_get_float: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out: *mut f64,
    ) -> i32,
    pub object_get_bool: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out: *mut i32,
    ) -> i32,
    pub object_get_bytes: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out_data: *mut *const u8,
        out_len: *mut usize,
    ) -> i32,
    pub object_len: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out_len: *mut usize,
    ) -> i32,
    pub object_get_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_sequence_len: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out_len: *mut usize,
    ) -> i32,
    pub object_sequence_get_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        index: usize,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_get_iter: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_iter_next: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        iter_handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_list_append: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        list_handle: PyrsObjectHandle,
        item_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_list_set_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        list_handle: PyrsObjectHandle,
        index: usize,
        item_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_dict_len: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        handle: PyrsObjectHandle,
        out_len: *mut usize,
    ) -> i32,
    pub object_dict_set_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        value_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_dict_get_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_dict_contains: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_dict_del_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        dict_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_get_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        attr_name: *const c_char,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_set_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        attr_name: *const c_char,
        value_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_del_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        attr_name: *const c_char,
    ) -> i32,
    pub object_has_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        attr_name: *const c_char,
    ) -> i32,
    pub object_call_noargs: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        callable_handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_call_onearg: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        callable_handle: PyrsObjectHandle,
        arg_handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_call: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        callable_handle: PyrsObjectHandle,
        argc: usize,
        argv: *const PyrsObjectHandle,
        kwargc: usize,
        kwarg_names: *const *const c_char,
        kwarg_values: *const PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_get_string:
        unsafe extern "C" fn(module_ctx: *mut c_void, handle: PyrsObjectHandle) -> *const c_char,
    pub error_set: unsafe extern "C" fn(module_ctx: *mut c_void, message: *const c_char) -> i32,
    pub error_get_message: unsafe extern "C" fn(module_ctx: *mut c_void) -> *const c_char,
    pub error_clear: unsafe extern "C" fn(module_ctx: *mut c_void) -> i32,
    pub error_occurred: unsafe extern "C" fn(module_ctx: *mut c_void) -> i32,
    pub module_set_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        module_handle: PyrsObjectHandle,
        attr_name: *const c_char,
        value_handle: PyrsObjectHandle,
    ) -> i32,
    pub module_del_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        module_handle: PyrsObjectHandle,
        attr_name: *const c_char,
    ) -> i32,
    pub module_has_attr: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        module_handle: PyrsObjectHandle,
        attr_name: *const c_char,
    ) -> i32,
    pub object_set_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
        value_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_del_item: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        key_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_contains: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        needle_handle: PyrsObjectHandle,
    ) -> i32,
    pub object_dict_keys: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        dict_handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_dict_items: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        dict_handle: PyrsObjectHandle,
        out_handle: *mut PyrsObjectHandle,
    ) -> i32,
    pub object_get_buffer: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        object_handle: PyrsObjectHandle,
        out_view: *mut PyrsBufferViewV1,
    ) -> i32,
    pub object_release_buffer:
        unsafe extern "C" fn(module_ctx: *mut c_void, object_handle: PyrsObjectHandle) -> i32,
    pub capsule_new: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        pointer: *mut c_void,
        name: *const c_char,
    ) -> PyrsObjectHandle,
    pub capsule_get_pointer: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        capsule_handle: PyrsObjectHandle,
        name: *const c_char,
    ) -> *mut c_void,
    pub capsule_get_name: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        capsule_handle: PyrsObjectHandle,
    ) -> *const c_char,
    pub capsule_set_context: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        capsule_handle: PyrsObjectHandle,
        context: *mut c_void,
    ) -> i32,
    pub capsule_get_context: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        capsule_handle: PyrsObjectHandle,
    ) -> *mut c_void,
    pub capsule_set_destructor: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        capsule_handle: PyrsObjectHandle,
        destructor: Option<PyrsCapsuleDestructorV1>,
    ) -> i32,
}

pub type PyrsCFunctionV1 = unsafe extern "C" fn(
    api: *const PyrsApiV1,
    module_ctx: *mut c_void,
    argc: usize,
    argv: *const PyrsObjectHandle,
    result: *mut PyrsObjectHandle,
) -> i32;

pub type PyrsCFunctionKwV1 = unsafe extern "C" fn(
    api: *const PyrsApiV1,
    module_ctx: *mut c_void,
    argc: usize,
    argv: *const PyrsObjectHandle,
    kwargc: usize,
    kwarg_names: *const *const c_char,
    kwarg_values: *const PyrsObjectHandle,
    result: *mut PyrsObjectHandle,
) -> i32;

pub type PyrsCapsuleDestructorV1 = unsafe extern "C" fn(pointer: *mut c_void, context: *mut c_void);

pub type PyrsExtensionInitV1 =
    unsafe extern "C" fn(api: *const PyrsApiV1, module_ctx: *mut c_void) -> i32;

#[cfg(unix)]
pub struct SharedLibraryHandle {
    raw: *mut c_void,
}

#[cfg(unix)]
impl SharedLibraryHandle {
    fn open(path: &Path) -> Result<Self, String> {
        use std::os::unix::ffi::OsStrExt;

        let path_c = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            format!(
                "invalid shared library path '{}': contains NUL",
                path.display()
            )
        })?;
        // RTLD_NOW; RTLD_LOCAL is zero on some systems and non-zero on others.
        let flags: i32 = 2;
        // SAFETY: C string is NUL-terminated and valid for the duration of the call.
        let raw = unsafe { dlopen(path_c.as_ptr(), flags) };
        if raw.is_null() {
            return Err(format!(
                "failed to load shared library '{}': {}",
                path.display(),
                last_dl_error()
            ));
        }
        Ok(Self { raw })
    }

    fn symbol<T>(&self, name: &str) -> Result<T, String>
    where
        T: Copy,
    {
        let symbol_c =
            CString::new(name).map_err(|_| format!("invalid symbol '{}': contains NUL", name))?;
        // SAFETY: clear any existing error before dlsym.
        unsafe {
            let _ = dlerror();
        }
        // SAFETY: handle is valid while self lives and symbol_c is valid C string.
        let ptr = unsafe { dlsym(self.raw, symbol_c.as_ptr()) };
        // SAFETY: dlerror returns a thread-local error pointer or null.
        let err_ptr = unsafe { dlerror() };
        if !err_ptr.is_null() {
            // SAFETY: err_ptr from dlerror points to NUL-terminated bytes.
            let err = unsafe { std::ffi::CStr::from_ptr(err_ptr) }
                .to_string_lossy()
                .into_owned();
            return Err(format!("failed to resolve symbol '{}': {err}", name));
        }
        if ptr.is_null() {
            return Err(format!("symbol '{}' resolved to null", name));
        }
        // SAFETY: caller chooses T to match the symbol signature.
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }
}

#[cfg(unix)]
impl Drop for SharedLibraryHandle {
    fn drop(&mut self) {
        // SAFETY: raw was returned by dlopen and may be null only if construction failed,
        // in which case Drop is not run.
        unsafe {
            let _ = dlclose(self.raw);
        }
    }
}

#[cfg(unix)]
fn last_dl_error() -> String {
    // SAFETY: dlerror returns thread-local pointer or null.
    let ptr = unsafe { dlerror() };
    if ptr.is_null() {
        "unknown dynamic loader error".to_string()
    } else {
        // SAFETY: pointer from dlerror is a valid C string.
        unsafe { std::ffi::CStr::from_ptr(ptr) }
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

#[cfg(unix)]
pub fn load_dynamic_initializer(
    library_path: &Path,
    symbol: &str,
) -> Result<(SharedLibraryHandle, PyrsExtensionInitV1), String> {
    let handle = SharedLibraryHandle::open(library_path)?;
    let initializer = handle.symbol::<PyrsExtensionInitV1>(symbol)?;
    Ok((handle, initializer))
}

#[cfg(not(unix))]
#[derive(Default)]
pub struct SharedLibraryHandle;

#[cfg(not(unix))]
pub fn load_dynamic_initializer(
    library_path: &Path,
    _symbol: &str,
) -> Result<(SharedLibraryHandle, PyrsExtensionInitV1), String> {
    Err(format!(
        "dynamic extension loading is not supported on this target for '{}'",
        library_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        ExtensionEntrypoint, PYRS_EXTENSION_ABI_TAG, find_shared_library_for_module,
        find_shared_library_for_package, parse_extension_manifest, shared_library_suffixes,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_manifest_path(stem: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("pyrs_manifest_{stem}_{nanos}.pyrs-ext"))
    }

    #[test]
    fn parses_valid_static_manifest() {
        let path = temp_manifest_path("valid_static");
        fs::write(
            &path,
            format!("module=hello_ext\nabi={PYRS_EXTENSION_ABI_TAG}\nentrypoint=hello_ext\n"),
        )
        .expect("manifest write should succeed");

        let parsed = parse_extension_manifest(&path, "hello_ext").expect("manifest should parse");
        assert_eq!(parsed.module_name, "hello_ext");
        assert_eq!(parsed.abi_tag, PYRS_EXTENSION_ABI_TAG);
        assert_eq!(parsed.entrypoint, ExtensionEntrypoint::HelloExt);
        assert!(parsed.library_path.is_none());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn parses_valid_dynamic_manifest() {
        let path = temp_manifest_path("valid_dynamic");
        fs::write(
            &path,
            format!(
                "module=hello_ext\nabi={PYRS_EXTENSION_ABI_TAG}\nentrypoint=dynamic:init_symbol\nlibrary=libhello_ext.so\n"
            ),
        )
        .expect("manifest write should succeed");

        let parsed = parse_extension_manifest(&path, "hello_ext").expect("manifest should parse");
        assert_eq!(parsed.module_name, "hello_ext");
        assert_eq!(parsed.abi_tag, PYRS_EXTENSION_ABI_TAG);
        assert_eq!(
            parsed.entrypoint,
            ExtensionEntrypoint::DynamicSymbol("init_symbol".to_string())
        );
        assert_eq!(parsed.library_path.as_deref(), Some("libhello_ext.so"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_dynamic_manifest_missing_library() {
        let path = temp_manifest_path("missing_library");
        fs::write(
            &path,
            format!(
                "module=hello_ext\nabi={PYRS_EXTENSION_ABI_TAG}\nentrypoint=dynamic:init_symbol\n"
            ),
        )
        .expect("manifest write should succeed");

        let err = parse_extension_manifest(&path, "hello_ext").expect_err("manifest should fail");
        assert!(err.contains("requires 'library'"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_unexpected_module_name() {
        let path = temp_manifest_path("module_mismatch");
        fs::write(
            &path,
            format!("module=other\nabi={PYRS_EXTENSION_ABI_TAG}\nentrypoint=hello_ext\n"),
        )
        .expect("manifest write should succeed");

        let err = parse_extension_manifest(&path, "hello_ext").expect_err("manifest should fail");
        assert!(err.contains("does not match import target"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn shared_library_suffixes_are_non_empty() {
        assert!(!shared_library_suffixes().is_empty());
    }

    #[test]
    fn finds_tagged_shared_library_for_module() {
        let root = std::env::temp_dir().join(format!(
            "pyrs_find_so_module_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp root should be created");
        let tagged = if cfg!(target_os = "macos") {
            root.join("mymod.cpython-314-darwin.so")
        } else if cfg!(target_os = "windows") {
            root.join("mymod.cp314-win_amd64.pyd")
        } else {
            root.join("mymod.cpython-314-x86_64-linux-gnu.so")
        };
        fs::write(&tagged, b"").expect("tagged artifact should be created");
        let found = find_shared_library_for_module(&root, "mymod").expect("candidate expected");
        assert_eq!(found, tagged);
        let _ = fs::remove_file(&tagged);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finds_tagged_shared_library_for_package_init() {
        let root = std::env::temp_dir().join(format!(
            "pyrs_find_so_pkg_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        let package_dir = root.join("pkg");
        fs::create_dir_all(&package_dir).expect("package dir should be created");
        let tagged = if cfg!(target_os = "macos") {
            package_dir.join("__init__.cpython-314-darwin.so")
        } else if cfg!(target_os = "windows") {
            package_dir.join("__init__.cp314-win_amd64.pyd")
        } else {
            package_dir.join("__init__.cpython-314-x86_64-linux-gnu.so")
        };
        fs::write(&tagged, b"").expect("tagged artifact should be created");
        let found =
            find_shared_library_for_package(&package_dir).expect("package candidate expected");
        assert_eq!(found, tagged);
        let _ = fs::remove_file(&tagged);
        let _ = fs::remove_dir_all(root);
    }
}
