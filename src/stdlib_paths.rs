use std::collections::HashSet;
use std::path::PathBuf;

use crate::CPYTHON_STDLIB_VERSION;
use crate::host::VmHost;

#[derive(Debug, Clone)]
pub(crate) struct DetectedStdlibPaths {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) strict_site_import: bool,
}

pub(crate) fn detect_cpython_stdlib_paths(host: &dyn VmHost) -> DetectedStdlibPaths {
    fn register_unique_path(out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
        let normalized = std::fs::canonicalize(&path).unwrap_or(path);
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }

    fn register_stdlib_root(
        out: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        root: PathBuf,
        host: &dyn VmHost,
    ) -> Option<PathBuf> {
        let normalized = std::fs::canonicalize(&root).unwrap_or(root);
        if !host.path_is_dir(&normalized) || !normalized.join("site.py").is_file() {
            return None;
        }
        register_unique_path(out, seen, normalized.clone());
        Some(normalized)
    }

    fn register_dynload_for_root(
        out: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        root: &PathBuf,
    ) -> bool {
        let dynload = root.join("lib-dynload");
        if dynload.is_dir() {
            register_unique_path(out, seen, dynload);
            return true;
        }
        false
    }

    fn host_stdlib_roots() -> [PathBuf; 4] {
        [
            PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
            PathBuf::from("/opt/homebrew/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
            PathBuf::from("/usr/local/lib/python3.14"),
            PathBuf::from("/usr/lib/python3.14"),
        ]
    }

    fn register_host_dynload_fallback(out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
        for candidate in host_stdlib_roots() {
            let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
            if register_dynload_for_root(out, seen, &normalized) {
                break;
            }
        }
    }

    fn install_managed_stdlib_roots(host: &dyn VmHost) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let stdlib_suffix = PathBuf::from(format!("stdlib/{CPYTHON_STDLIB_VERSION}/Lib"));
        if let Some(executable_path) = host.current_exe()
            && let Some(bin_dir) = executable_path.parent()
        {
            roots.push(bin_dir.join("../share/pyrs").join(&stdlib_suffix));
            roots.push(bin_dir.join("../libexec").join(&stdlib_suffix));
            roots.push(bin_dir.join("../stdlib").join(&stdlib_suffix));
        }
        if let Some(xdg_data_home) = host.env_var_os("XDG_DATA_HOME") {
            roots.push(
                PathBuf::from(xdg_data_home)
                    .join("pyrs")
                    .join(&stdlib_suffix),
            );
        }
        if let Some(home) = host.env_var_os("HOME") {
            roots.push(
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("pyrs")
                    .join(&stdlib_suffix),
            );
        }
        roots.push(PathBuf::from(format!(
            ".local/Python-{CPYTHON_STDLIB_VERSION}/Lib"
        )));
        roots
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut strict_site_import = false;

    if let Some(path) = host.env_var("PYRS_CPYTHON_LIB") {
        strict_site_import = true;
        let mut has_local_dynload = false;
        if let Some(root) = register_stdlib_root(&mut out, &mut seen, PathBuf::from(path), host) {
            has_local_dynload = register_dynload_for_root(&mut out, &mut seen, &root);
        }
        if !has_local_dynload {
            register_host_dynload_fallback(&mut out, &mut seen);
        }
        return DetectedStdlibPaths {
            paths: out,
            strict_site_import,
        };
    }

    for root_candidate in install_managed_stdlib_roots(host) {
        if let Some(root) = register_stdlib_root(&mut out, &mut seen, root_candidate, host) {
            strict_site_import = true;
            if !register_dynload_for_root(&mut out, &mut seen, &root) {
                register_host_dynload_fallback(&mut out, &mut seen);
            }
            return DetectedStdlibPaths {
                paths: out,
                strict_site_import,
            };
        }
    }

    if let Some(home) = host.env_var("PYTHONHOME")
        && let Some(root) = register_stdlib_root(
            &mut out,
            &mut seen,
            PathBuf::from(home).join("lib").join("python3.14"),
            host,
        )
    {
        register_dynload_for_root(&mut out, &mut seen, &root);
    }

    for candidate in host_stdlib_roots() {
        let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        if let Some(root) = register_stdlib_root(&mut out, &mut seen, normalized, host) {
            register_dynload_for_root(&mut out, &mut seen, &root);
        }
    }

    DetectedStdlibPaths {
        paths: out,
        strict_site_import,
    }
}
