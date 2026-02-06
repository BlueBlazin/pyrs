use std::fs;
use std::path::{Path, PathBuf};

use pyrs::{compiler, parser, vm::Vm};

fn read_subset(path: &str) -> Vec<String> {
    let data = fs::read_to_string(path).unwrap_or_else(|_| String::new());
    data.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
}

fn module_path(lib: &Path, name: &str) -> Option<PathBuf> {
    if name.ends_with(".py") {
        let candidate = lib.join(name);
        return candidate.exists().then_some(candidate);
    }
    let test_path = lib.join("test").join(format!("{name}.py"));
    if test_path.exists() {
        return Some(test_path);
    }
    None
}

fn run_subset_file(subset_file: &str) {
    let lib = match std::env::var("PYRS_CPYTHON_LIB") {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            eprintln!("PYRS_CPYTHON_LIB not set; skipping CPython Lib/test harness");
            return;
        }
    };
    if !lib.exists() {
        eprintln!("PYRS_CPYTHON_LIB path missing; skipping");
        return;
    }

    let tests = read_subset(subset_file);
    if tests.is_empty() {
        return;
    }

    let mut failures = Vec::new();
    for test in tests {
        let path = match module_path(&lib, &test) {
            Some(path) => path,
            None => {
                failures.push(format!("{test}: missing"));
                continue;
            }
        };
        let source = match fs::read_to_string(&path) {
            Ok(src) => src,
            Err(err) => {
                failures.push(format!("{test}: read error {err}"));
                continue;
            }
        };
        let module = match parser::parse_module(&source) {
            Ok(module) => module,
            Err(err) => {
                failures.push(format!("{test}: parse error {}", err.message));
                continue;
            }
        };
        let code = match compiler::compile_module(&module) {
            Ok(code) => code,
            Err(err) => {
                failures.push(format!("{test}: compile error {}", err.message));
                continue;
            }
        };
        let mut vm = Vm::new();
        vm.add_module_path(&lib);
        if let Err(err) = vm.execute(&code) {
            failures.push(format!("{test}: runtime error {}", err.message));
        }
    }

    if !failures.is_empty() {
        panic!("CPython harness failures:\n{}", failures.join("\n"));
    }
}

#[test]
#[ignore = "Requires PYRS_CPYTHON_LIB pointing to CPython Lib/ and broader stdlib support"]
fn runs_cpython_lib_subset() {
    run_subset_file("tests/cpython_subset.txt");
}

#[test]
#[ignore = "Requires PYRS_CPYTHON_LIB pointing to CPython Lib/ and broader stdlib support"]
fn runs_cpython_import_subset() {
    run_subset_file("tests/cpython_subset_imports.txt");
}
