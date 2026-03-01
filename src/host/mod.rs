use std::ffi::OsString;
use std::path::PathBuf;

/// Host boundary for runtime setup and environment probes.
///
/// This is introduced as a non-invasive seam: default native behavior remains
/// unchanged while allowing alternate host adapters (for example wasm/web)
/// to be introduced incrementally.
pub trait VmHost: Send + Sync {
    fn current_dir(&self) -> Result<PathBuf, String>;
    fn env_var(&self, name: &str) -> Option<String>;
    fn env_var_os(&self, name: &str) -> Option<OsString>;

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
}

#[cfg(test)]
mod tests {
    use super::{NativeHost, VmHost, WasmHost};

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
}

