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

impl HostCapability {
    pub fn key(self) -> &'static str {
        match self {
            Self::FilesystemRead => "filesystem_read",
            Self::FilesystemWrite => "filesystem_write",
            Self::EnvironmentRead => "environment_read",
            Self::ProcessArgs => "process_args",
            Self::ProcessSpawn => "process_spawn",
            Self::DynamicLibraryLoad => "dynamic_library_load",
            Self::InteractiveTerminal => "interactive_terminal",
            Self::NetworkSockets => "network_sockets",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::FilesystemRead => "read host filesystem paths",
            Self::FilesystemWrite => "write host filesystem paths",
            Self::EnvironmentRead => "read process environment variables",
            Self::ProcessArgs => "read process argv metadata",
            Self::ProcessSpawn => "spawn subprocesses",
            Self::DynamicLibraryLoad => "load dynamic libraries/extensions",
            Self::InteractiveTerminal => "interact with terminal/tty capabilities",
            Self::NetworkSockets => "open raw network sockets",
        }
    }

    pub fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "filesystem_read" => Some(Self::FilesystemRead),
            "filesystem_write" => Some(Self::FilesystemWrite),
            "environment_read" => Some(Self::EnvironmentRead),
            "process_args" => Some(Self::ProcessArgs),
            "process_spawn" => Some(Self::ProcessSpawn),
            "dynamic_library_load" => Some(Self::DynamicLibraryLoad),
            "interactive_terminal" => Some(Self::InteractiveTerminal),
            "network_sockets" => Some(Self::NetworkSockets),
            _ => None,
        }
    }
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

    fn unsupported_message(&self, capability: HostCapability) -> Option<String> {
        if self.supports(capability) {
            None
        } else {
            Some(format!(
                "unsupported capability '{}' ({}) in current host",
                capability.key(),
                capability.description()
            ))
        }
    }

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

    #[test]
    fn capability_key_roundtrip_is_stable() {
        let expected = [
            HostCapability::FilesystemRead,
            HostCapability::FilesystemWrite,
            HostCapability::EnvironmentRead,
            HostCapability::ProcessArgs,
            HostCapability::ProcessSpawn,
            HostCapability::DynamicLibraryLoad,
            HostCapability::InteractiveTerminal,
            HostCapability::NetworkSockets,
        ];
        for capability in expected {
            let key = capability.key();
            assert_eq!(HostCapability::from_key(key), Some(capability));
        }
        assert_eq!(HostCapability::from_key("unknown_capability"), None);
    }
}
