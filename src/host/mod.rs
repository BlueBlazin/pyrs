use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCapability {
    FilesystemRead,
    FilesystemWrite,
    EnvironmentRead,
    ProcessArgs,
    ProcessSpawn,
    DynamicLibraryLoad,
    InteractiveTerminal,
    NetworkSockets,
}

/// Host boundary for runtime setup and environment probes.
///
/// This is introduced as a non-invasive seam: default native behavior remains
/// unchanged while allowing alternate host adapters (for example wasm/web)
/// to be introduced incrementally.
pub trait VmHost: Send + Sync {
    fn current_dir(&self) -> Result<PathBuf, String>;
    fn env_var(&self, name: &str) -> Option<String>;
    fn env_var_os(&self, name: &str) -> Option<OsString>;
    fn process_args(&self) -> Vec<String>;
    fn current_exe(&self) -> Option<PathBuf>;
    fn os_name(&self) -> &'static str;
    fn supports(&self, capability: HostCapability) -> bool;

    fn env_flag_enabled(&self, name: &str) -> bool {
        let Some(raw) = self.env_var(name) else {
            return false;
        };
        let normalized = raw.trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    }

    fn env_flag_enabled_or_default(&self, name: &str, default: bool) -> bool {
        let Some(raw) = self.env_var(name) else {
            return default;
        };
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        }
    }
}

#[derive(Debug, Default)]
pub struct NativeHost;

impl VmHost for NativeHost {
    fn current_dir(&self) -> Result<PathBuf, String> {
        std::env::current_dir().map_err(|err| err.to_string())
    }

    fn env_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn env_var_os(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn process_args(&self) -> Vec<String> {
        std::env::args().collect()
    }

    fn current_exe(&self) -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    fn os_name(&self) -> &'static str {
        std::env::consts::OS
    }

    fn supports(&self, _capability: HostCapability) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct WasmHost;

impl VmHost for WasmHost {
    fn current_dir(&self) -> Result<PathBuf, String> {
        Err("current_dir is unavailable in wasm host".to_string())
    }

    fn env_var(&self, _name: &str) -> Option<String> {
        None
    }

    fn env_var_os(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn process_args(&self) -> Vec<String> {
        vec!["pyrs".to_string()]
    }

    fn current_exe(&self) -> Option<PathBuf> {
        None
    }

    fn os_name(&self) -> &'static str {
        "emscripten"
    }

    fn supports(&self, capability: HostCapability) -> bool {
        matches!(capability, HostCapability::ProcessArgs)
    }
}

#[cfg(test)]
mod tests {
    use super::{HostCapability, NativeHost, VmHost, WasmHost};

    #[test]
    fn native_host_can_read_current_dir() {
        let host = NativeHost;
        let path = host.current_dir().expect("native cwd");
        assert!(path.is_absolute());
    }

    #[test]
    fn wasm_host_reports_no_env() {
        let host = WasmHost;
        assert!(host.env_var("HOME").is_none());
        assert!(host.env_var_os("HOME").is_none());
    }

    #[test]
    fn wasm_host_reports_stubbed_process_metadata() {
        let host = WasmHost;
        assert_eq!(host.process_args(), vec!["pyrs".to_string()]);
        assert!(host.current_exe().is_none());
        assert_eq!(host.os_name(), "emscripten");
    }

    #[test]
    fn wasm_host_capability_matrix_is_explicit() {
        let host = WasmHost;
        assert!(host.supports(HostCapability::ProcessArgs));
        assert!(!host.supports(HostCapability::FilesystemRead));
        assert!(!host.supports(HostCapability::FilesystemWrite));
        assert!(!host.supports(HostCapability::EnvironmentRead));
        assert!(!host.supports(HostCapability::ProcessSpawn));
        assert!(!host.supports(HostCapability::DynamicLibraryLoad));
        assert!(!host.supports(HostCapability::InteractiveTerminal));
        assert!(!host.supports(HostCapability::NetworkSockets));
    }
}
