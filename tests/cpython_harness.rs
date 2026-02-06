use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use pyrs::{compiler, parser, vm::Vm};

const ALLOWLIST_FILE: &str = "tests/cpython_allowlist.txt";
const LANGUAGE_SUITE: &str = "tests/cpython_suite_language.txt";
const IMPORT_SUITE: &str = "tests/cpython_suite_imports.txt";

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
        let test = parts
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let category = parts
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let owner = parts
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
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
        let package = lib.join("test").join(entry.replace('.', "/")).join("__init__.py");
        if package.is_file() {
            return Some(package);
        }
    }
    None
}

fn run_entry(lib: &Path, entry: &str) -> Result<(), String> {
    let path = module_path(lib, entry).ok_or_else(|| "missing module".to_string())?;
    let source = fs::read_to_string(&path).map_err(|err| format!("read error {err}"))?;
    let module =
        parser::parse_module(&source).map_err(|err| format!("parse error {}", err.message))?;
    let code = compiler::compile_module_with_filename(&module, &path.to_string_lossy())
        .map_err(|err| format!("compile error {}", err.message))?;
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code)
        .map(|_| ())
        .map_err(|err| format!("runtime error {}", err.message))
}

fn run_suite_file(suite_file: &str) {
    let lib = cpython_lib_or_panic();
    if lib.as_os_str().is_empty() {
        return;
    }
    let suite = read_list(suite_file);
    let allow = read_allowlist(ALLOWLIST_FILE);

    let mut unexpected_failures = Vec::new();
    let mut stale_allowlist = Vec::new();
    let mut passed = 0usize;
    let mut allowed = 0usize;

    for entry in &suite {
        match run_entry(&lib, entry) {
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
    let allow = read_allowlist(ALLOWLIST_FILE);
    let suite_entries: HashSet<String> = read_list(LANGUAGE_SUITE)
        .into_iter()
        .chain(read_list(IMPORT_SUITE))
        .collect();
    let mut unused = Vec::new();
    for key in allow.keys() {
        if !suite_entries.contains(key) {
            unused.push(key.clone());
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
    run_suite_file(LANGUAGE_SUITE);
}

#[test]
fn runs_cpython_import_suite() {
    run_suite_file(IMPORT_SUITE);
}
