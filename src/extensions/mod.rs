use std::ffi::{CString, c_char, c_void};
use std::fs;
use std::path::{Path, PathBuf};

pub const PYRS_EXTENSION_MANIFEST_SUFFIX: &str = ".pyrs-ext";
pub const PYRS_EXTENSION_ABI_TAG: &str = "pyrs314";
pub const PYRS_CAPI_ABI_VERSION: u32 = 1;
pub const PYRS_DYNAMIC_INIT_SYMBOL_V1: &str = "pyrs_extension_init_v1";

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
    pub module_set_int:
        unsafe extern "C" fn(module_ctx: *mut c_void, name: *const c_char, value: i64) -> i32,
    pub module_set_bool:
        unsafe extern "C" fn(module_ctx: *mut c_void, name: *const c_char, value: i32) -> i32,
    pub module_set_string: unsafe extern "C" fn(
        module_ctx: *mut c_void,
        name: *const c_char,
        value: *const c_char,
    ) -> i32,
}

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
        ExtensionEntrypoint, PYRS_EXTENSION_ABI_TAG, parse_extension_manifest,
        shared_library_suffixes,
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
}
