use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use pyrs::{compiler, parser, vm::Vm};

const ALLOWLIST_FILE: &str = "tests/cpython_allowlist.txt";
const STRICT_ALLOWLIST_FILE: &str = "tests/cpython_allowlist_strict.txt";
const LANGUAGE_SUITE: &str = "tests/cpython_suite_language.txt";
const IMPORT_SUITE: &str = "tests/cpython_suite_imports.txt";
const STRICT_STDLIB_SUITE: &str = "tests/cpython_suite_strict_stdlib.txt";

#[derive(Debug, Clone)]
struct AllowEntry {
    category: String,
    owner: String,
}

fn read_list(path: &str) -> Vec<String> {
    let data = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("failed to read {path}: {err}");
    });
    data.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

fn read_allowlist(path: &str) -> HashMap<String, AllowEntry> {
    let mut allow = HashMap::new();
    for line in read_list(path) {
        let mut parts = line.split('|');
        let test = parts.next().unwrap_or_default().trim().to_string();
        let category = parts.next().unwrap_or_default().trim().to_string();
        let owner = parts.next().unwrap_or_default().trim().to_string();
        assert!(
            !test.is_empty() && !category.is_empty() && !owner.is_empty(),
            "invalid allowlist row (need 3 pipe-delimited fields): {line}"
        );
        let replaced = allow.insert(test.clone(), AllowEntry { category, owner });
        assert!(
            replaced.is_none(),
            "duplicate allowlist entry for {test} in {path}"
        );
    }
    allow
}

fn detect_cpython_lib() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let path = PathBuf::from(path);
        if path.join("test").is_dir() {
            return Some(path);
        }
    }
    let candidates = [
        "/Users/$USER/Downloads/Python-3.14.3/Lib",
        "/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
    ];
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.join("test").is_dir() {
            return Some(path);
        }
    }
    None
}

fn cpython_lib_or_panic() -> PathBuf {
    if let Some(path) = detect_cpython_lib() {
        return path;
    }
    if std::env::var("PYRS_CPYTHON_OPTIONAL").as_deref() == Ok("1") {
        eprintln!("CPython Lib path not found; skipping harness due to PYRS_CPYTHON_OPTIONAL=1");
        return PathBuf::new();
    }
    panic!(
        "CPython Lib path not found. Set PYRS_CPYTHON_LIB (expected <...>/Lib with test/ directory)."
    );
}

fn module_path(lib: &Path, entry: &str) -> Option<PathBuf> {
    let candidate = lib.join(entry);
    if candidate.is_file() {
        return Some(candidate);
    }
    if !entry.ends_with(".py") {
        let test_module = lib.join("test").join(format!("{entry}.py"));
        if test_module.is_file() {
            return Some(test_module);
        }
        let package = lib
            .join("test")
            .join(entry.replace('.', "/"))
            .join("__init__.py");
        if package.is_file() {
            return Some(package);
        }
    }
    None
}

fn module_name(entry: &str) -> Option<String> {
    let without_suffix = entry.strip_suffix(".py")?;
    let without_init = without_suffix
        .strip_suffix("/__init__")
        .unwrap_or(without_suffix);
    let normalized = without_init.replace('/', ".");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn pyrs_bin() -> Option<PathBuf> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_pyrs") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_pyrs") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let from_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/pyrs");
    if from_manifest.is_file() {
        return Some(from_manifest);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(debug_dir) = exe.parent().and_then(|deps| deps.parent()) {
            let sibling = debug_dir.join("pyrs");
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    None
}

fn strict_unittest_timeout() -> Duration {
    let secs = std::env::var("PYRS_STRICT_HARNESS_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    Duration::from_secs(secs.max(1))
}

fn run_source_in_subprocess(bin: &Path, source: &str, timeout: Duration) -> Result<(), String> {
    let mut child = Command::new(bin)
        .arg("-c")
        .arg(source)
        .spawn()
        .map_err(|err| format!("failed to spawn subprocess harness: {err}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                return Err(format!("subprocess harness failed with status {status}"));
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "subprocess harness timed out after {}s",
                        timeout.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(format!("failed to poll subprocess harness: {err}")),
        }
    }
}

#[derive(Clone, Copy)]
enum SuiteMode {
    ImportOnly,
    StrictUnittest,
}

fn run_entry(lib: &Path, entry: &str, mode: SuiteMode) -> Result<(), String> {
    let _path = module_path(lib, entry).ok_or_else(|| "missing module".to_string())?;
    let import_name = module_name(entry).ok_or_else(|| "invalid module entry".to_string())?;
    let lib_path = lib.to_string_lossy();
    let executable_patch = pyrs_bin()
        .map(|path| format!("sys.executable = {:?}\n", path.to_string_lossy()))
        .unwrap_or_default();
    let source = match mode {
        SuiteMode::ImportOnly => format!(
            "import sys\nimport importlib\n{executable_patch}sys.path = [{lib_path:?}]\nimportlib.import_module({import_name:?})\n"
        ),
        SuiteMode::StrictUnittest => format!(
            "import sys\nimport importlib\nimport unittest\n{executable_patch}sys.path = [{lib_path:?}]\nmodule = importlib.import_module({import_name:?})\nloader = unittest.defaultTestLoader\nbefore_errors = len(getattr(loader, 'errors', []))\nsuite = loader.loadTestsFromModule(module)\nafter_errors = len(getattr(loader, 'errors', []))\nif after_errors > before_errors:\n    raise RuntimeError('strict unittest loader failed')\nresult = unittest.TextTestRunner(verbosity=0, failfast=True).run(suite)\nif not result.wasSuccessful():\n    raise RuntimeError('strict unittest suite failed')\n"
        ),
    };

    if matches!(mode, SuiteMode::StrictUnittest) {
        if let Some(bin) = pyrs_bin() {
            return run_source_in_subprocess(&bin, &source, strict_unittest_timeout());
        }
    }

    let module =
        parser::parse_module(&source).map_err(|err| format!("parse error {}", err.message))?;
    let code = compiler::compile_module_with_filename(&module, "<cpython_harness>")
        .map_err(|err| format!("compile error {}", err.message))?;
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code)
        .map(|_| ())
        .map_err(|err| format!("runtime error {}", err.message))
}

fn run_suite_file(suite_file: &str, allowlist_file: &str, mode: SuiteMode) {
    let lib = cpython_lib_or_panic();
    if lib.as_os_str().is_empty() {
        return;
    }
    let suite = read_list(suite_file);
    let allow = read_allowlist(allowlist_file);

    let mut unexpected_failures = Vec::new();
    let mut stale_allowlist = Vec::new();
    let mut passed = 0usize;
    let mut allowed = 0usize;

    for entry in &suite {
        match run_entry(&lib, entry, mode) {
            Ok(()) => {
                if let Some(allow_entry) = allow.get(entry) {
                    stale_allowlist.push(format!(
                        "{entry}: now passing; remove allowlist ({}/{})",
                        allow_entry.category, allow_entry.owner
                    ));
                } else {
                    passed += 1;
                }
            }
            Err(reason) => {
                if allow.contains_key(entry) {
                    allowed += 1;
                } else {
                    unexpected_failures.push(format!("{entry}: {reason}"));
                }
            }
        }
    }

    if !stale_allowlist.is_empty() || !unexpected_failures.is_empty() {
        let mut lines = Vec::new();
        lines.push(format!(
            "CPython harness parity mismatch for {suite_file} (pass={passed}, allowlisted_fail={allowed}, total={})",
            suite.len()
        ));
        if !unexpected_failures.is_empty() {
            lines.push("Unexpected failures:".to_string());
            lines.extend(unexpected_failures);
        }
        if !stale_allowlist.is_empty() {
            lines.push("Stale allowlist entries:".to_string());
            lines.extend(stale_allowlist);
        }
        panic!("{}", lines.join("\n"));
    }
}

#[test]
fn allowlist_entries_are_referenced_by_suites() {
    let suite_entries: HashSet<String> = read_list(LANGUAGE_SUITE)
        .into_iter()
        .chain(read_list(IMPORT_SUITE))
        .chain(read_list(STRICT_STDLIB_SUITE))
        .collect();
    let mut unused = Vec::new();
    for allowlist_path in [ALLOWLIST_FILE, STRICT_ALLOWLIST_FILE] {
        let allow = read_allowlist(allowlist_path);
        for key in allow.keys() {
            if !suite_entries.contains(key) {
                unused.push(format!("{allowlist_path}: {key}"));
            }
        }
    }
    if !unused.is_empty() {
        panic!(
            "allowlist entries not present in any suite:\n{}",
            unused.join("\n")
        );
    }
}

#[test]
fn runs_cpython_language_suite() {
    run_suite_file(LANGUAGE_SUITE, ALLOWLIST_FILE, SuiteMode::ImportOnly);
}

#[test]
fn runs_cpython_import_suite() {
    run_suite_file(IMPORT_SUITE, ALLOWLIST_FILE, SuiteMode::ImportOnly);
}

#[test]
fn runs_cpython_strict_stdlib_suite() {
    let handle = std::thread::Builder::new()
        .name("cpython-strict-stdlib".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            run_suite_file(
                STRICT_STDLIB_SUITE,
                STRICT_ALLOWLIST_FILE,
                SuiteMode::StrictUnittest,
            );
        })
        .expect("spawn strict stdlib harness thread");
    handle
        .join()
        .expect("strict stdlib harness thread should complete");
}
