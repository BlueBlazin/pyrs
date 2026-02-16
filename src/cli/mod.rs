//! CLI entry point and argument handling.

mod repl;

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::VERSION;
use crate::compiler;
use crate::parser;
use crate::runtime::Value;
use crate::stdlib;
use crate::vm::Vm;

const HELP: &str = "pyrs (CPython 3.14 compatible)\n\nUsage:\n  pyrs                    Start interactive REPL (or read from stdin when piped)\n  pyrs <file.py>          Run a Python file\n  pyrs <file.pyc>         Run a CPython .pyc file\n  pyrs -S <file.py>       Run without importing site on startup\n  pyrs --ast <file.py>    Print parsed AST\n  pyrs --bytecode <file.py>  Print bytecode disassembly\n  pyrs --version          Print version\n  pyrs --help             Show help\n";

pub fn run() -> i32 {
    let mut args = env::args().skip(1).peekable();
    let mut import_site = true;

    // Parse a small subset of CPython-style startup flags used by stdlib tests.
    loop {
        let Some(flag) = args.peek().cloned() else {
            break;
        };
        match flag.as_str() {
            "-X" => {
                args.next();
                if args.next().is_none() {
                    eprintln!("error: -X expects an option");
                    return 2;
                }
            }
            "-S" | "--no-site" => {
                import_site = false;
                args.next();
            }
            _ => break,
        }
    }

    match args.next() {
        None => match repl::run_repl(import_site) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("error: {err}");
                2
            }
        },
        Some(flag) if flag == "-h" || flag == "--help" => {
            print_help();
            0
        }
        Some(flag) if flag == "--ast" => match args.next() {
            Some(path) => match run_ast(&path) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            },
            None => {
                eprintln!("error: --ast expects a file path");
                2
            }
        },
        Some(flag) if flag == "--bytecode" => match args.next() {
            Some(path) => match run_bytecode(&path) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            },
            None => {
                eprintln!("error: --bytecode expects a file path");
                2
            }
        },
        Some(flag) if flag == "-V" || flag == "--version" => {
            println!("pyrs {VERSION}");
            0
        }
        Some(flag) if flag == "-c" => match args.next() {
            Some(source) => match run_command(&source, import_site) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            },
            None => {
                eprintln!("error: -c expects command string");
                2
            }
        },
        Some(path) => match run_file(&path, import_site) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("error: {err}");
                2
            }
        },
    }
}

fn print_help() {
    println!("{HELP}");
}

fn run_file(path: &str, import_site: bool) -> Result<(), String> {
    let mut vm = Vm::new();
    configure_vm_for_execution(&mut vm, path, import_site)?;
    if path.ends_with(".pyc") {
        let exec_result = vm.execute_pyc_file(path);
        let shutdown_result = vm.run_shutdown_hooks();
        exec_result.map_err(|err| format!("runtime error: {}", err.message))?;
        shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;
        return Ok(());
    }

    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;

    stdlib::initialize();

    let module = parser::parse_module(&source).map_err(|err| {
        format!(
            "parse error at {} (line {}, column {}): {}",
            err.offset, err.line, err.column, err.message
        )
    })?;

    let code = compiler::compile_module_with_filename(&module, path)
        .map_err(|err| format!("compile error: {}", err.message))?;

    let exec_result = vm.execute(&code);
    let shutdown_result = vm.run_shutdown_hooks();
    exec_result.map_err(|err| format!("runtime error: {}", err.message))?;
    shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;

    Ok(())
}

fn run_command(source: &str, import_site: bool) -> Result<(), String> {
    let mut vm = Vm::new();
    configure_vm_for_command(&mut vm, import_site)?;

    stdlib::initialize();

    let module = parser::parse_module(source).map_err(|err| {
        format!(
            "parse error at {} (line {}, column {}): {}",
            err.offset, err.line, err.column, err.message
        )
    })?;

    let code = compiler::compile_module_with_filename(&module, "<string>")
        .map_err(|err| format!("compile error: {}", err.message))?;

    let exec_result = vm.execute(&code);
    let shutdown_result = vm.run_shutdown_hooks();
    exec_result.map_err(|err| format!("runtime error: {}", err.message))?;
    shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;

    Ok(())
}

fn configure_vm_for_execution(
    vm: &mut Vm,
    script_path: &str,
    import_site: bool,
) -> Result<(), String> {
    configure_vm_for_command(vm, import_site)?;
    if let Some(parent) = Path::new(script_path).parent()
        && !parent.as_os_str().is_empty()
    {
        vm.add_module_path_front(parent.to_path_buf());
    }
    Ok(())
}

fn configure_vm_for_command(vm: &mut Vm, import_site: bool) -> Result<(), String> {
    let pythonpath_entries = detect_pythonpath_entries();
    let (stdlib_paths, strict_site_import) = detect_cpython_stdlib_paths();
    for path in pythonpath_entries {
        vm.add_module_path(path);
    }
    vm.set_sys_no_site_flag(!import_site);
    for stdlib_path in &stdlib_paths {
        vm.add_module_path(stdlib_path.clone());
    }
    if import_site
        && !stdlib_paths.is_empty()
        && let Err(err) = vm.import_module("site")
        && strict_site_import
    {
        return Err(format!("startup site import failed: {}", err.message));
    }
    Ok(())
}

fn detect_cpython_stdlib_paths() -> (Vec<PathBuf>, bool) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut strict_site_import = false;

    let mut register = |candidate: PathBuf| {
        if candidate.as_os_str().is_empty() {
            return;
        }
        let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        if !normalized.join("site.py").is_file() {
            return;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    };

    if let Ok(path) = env::var("PYRS_CPYTHON_LIB") {
        strict_site_import = true;
        register(PathBuf::from(path));
    }
    if let Ok(home) = env::var("PYTHONHOME") {
        register(PathBuf::from(home).join("lib").join("python3.14"));
    }

    for candidate in [
        "/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
        "/opt/homebrew/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
        "/usr/local/lib/python3.14",
        "/usr/lib/python3.14",
    ] {
        let candidate_path = PathBuf::from(candidate);
        let normalized = std::fs::canonicalize(&candidate_path).unwrap_or(candidate_path);
        if normalized.join("site.py").is_file() {
            if seen.insert(normalized.clone()) {
                out.push(normalized);
            }
            break;
        }
    }

    (out, strict_site_import)
}

fn detect_pythonpath_entries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let Some(path) = env::var_os("PYTHONPATH") else {
        return out;
    };
    for entry in env::split_paths(&path) {
        if entry.as_os_str().is_empty() {
            continue;
        }
        let normalized = std::fs::canonicalize(&entry).unwrap_or(entry);
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn run_ast(path: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let module = parser::parse_module(&source).map_err(|err| {
        format!(
            "parse error at {} (line {}, column {}): {}",
            err.offset, err.line, err.column, err.message
        )
    })?;
    println!("{module:#?}");
    Ok(())
}

fn run_bytecode(path: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let module = parser::parse_module(&source).map_err(|err| {
        format!(
            "parse error at {} (line {}, column {}): {}",
            err.offset, err.line, err.column, err.message
        )
    })?;
    let code = compiler::compile_module_with_filename(&module, path)
        .map_err(|err| format!("compile error: {}", err.message))?;
    print_code_recursive(&code, 0);
    Ok(())
}

fn print_code_recursive(code: &crate::bytecode::CodeObject, depth: usize) {
    let indent = "  ".repeat(depth);
    println!("{indent}code {}:", code.name);
    if std::env::var_os("PYRS_BYTECODE_SHOW_TABLES").is_some() {
        println!("{indent}  names:");
        for (idx, name) in code.names.iter().enumerate() {
            println!("{indent}    {idx:04}: {name}");
        }
        println!("{indent}  constants:");
        for (idx, value) in code.constants.iter().enumerate() {
            println!("{indent}    {idx:04}: {value:?}");
        }
    }
    for line in code.disassemble().lines() {
        println!("{indent}  {line}");
    }
    for constant in &code.constants {
        if let Value::Code(code_obj) = constant {
            print_code_recursive(code_obj, depth + 1);
        }
    }
}
