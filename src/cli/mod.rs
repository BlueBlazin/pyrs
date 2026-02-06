//! CLI entry point and argument handling.

use std::env;

use crate::VERSION;
use crate::compiler;
use crate::parser;
use crate::stdlib;
use crate::vm::Vm;

const HELP: &str = "pyrs (CPython 3.14 compatible)\n\nUsage:\n  pyrs <file.py>          Run a Python file\n  pyrs <file.pyc>         Run a CPython .pyc file\n  pyrs --ast <file.py>    Print parsed AST\n  pyrs --bytecode <file.py>  Print bytecode disassembly\n  pyrs --version          Print version\n  pyrs --help             Show help\n";

pub fn run() -> i32 {
    let mut args = env::args().skip(1);
    match args.next() {
        None => {
            print_help();
            0
        }
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
        Some(path) => match run_file(&path) {
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

fn run_file(path: &str) -> Result<(), String> {
    if path.ends_with(".pyc") {
        let mut vm = Vm::new();
        vm.execute_pyc_file(path)
            .map_err(|err| format!("runtime error: {}", err.message))?;
        return Ok(());
    }

    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;

    stdlib::initialize();

    let module = parser::parse_module(&source)
        .map_err(|err| format!("parse error at {}: {}", err.offset, err.message))?;

    let code = compiler::compile_module_with_filename(&module, path)
        .map_err(|err| format!("compile error: {}", err.message))?;

    let mut vm = Vm::new();
    vm.execute(&code)
        .map_err(|err| format!("runtime error: {}", err.message))?;

    Ok(())
}

fn run_ast(path: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let module = parser::parse_module(&source)
        .map_err(|err| format!("parse error at {}: {}", err.offset, err.message))?;
    println!("{module:#?}");
    Ok(())
}

fn run_bytecode(path: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let module = parser::parse_module(&source)
        .map_err(|err| format!("parse error at {}: {}", err.offset, err.message))?;
    let code = compiler::compile_module_with_filename(&module, path)
        .map_err(|err| format!("compile error: {}", err.message))?;
    println!("{}", code.disassemble());
    Ok(())
}
