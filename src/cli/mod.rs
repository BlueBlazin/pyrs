//! CLI entry point and argument handling.

use std::env;

use crate::compiler;
use crate::parser;
use crate::stdlib;
use crate::vm::Vm;
use crate::VERSION;

const HELP: &str = "pyrs (CPython 3.14 compatible)\n\nUsage:\n  pyrs <file.py>   Run a Python file\n  pyrs --version   Print version\n  pyrs --help      Show help\n";

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
    let source = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {path}: {err}"))?;

    stdlib::initialize();

    let module = parser::parse_module(&source)
        .map_err(|err| format!("parse error at {}: {}", err.offset, err.message))?;

    let code = compiler::compile_module(&module)
        .map_err(|err| format!("compile error: {}", err.message))?;

    let mut vm = Vm::new();
    vm.execute(&code)
        .map_err(|err| format!("runtime error: {}", err.message))?;

    Ok(())
}
