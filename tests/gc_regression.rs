#![cfg(not(target_arch = "wasm32"))]

use pyrs::{compiler, parser, vm::Vm};
use std::path::PathBuf;

fn compile_source(source: &str) -> pyrs::bytecode::CodeObject {
    let module = parser::parse_module(source).expect("parse should succeed");
    compiler::compile_module(&module).expect("compile should succeed")
}

fn cpython_lib_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let path = PathBuf::from(path);
        if path.join("argparse.py").is_file() {
            return Some(path);
        }
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        workspace.join(".local/Python-3.14.3/Lib"),
        PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
    ];
    for candidate in candidates {
        if candidate.join("argparse.py").is_file() {
            return Some(candidate);
        }
    }
    None
}

fn run_with_large_stack<F>(name: &str, f: F)
where
    F: FnOnce() + Send + 'static,
{
    let join = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(f)
        .expect("failed to spawn large-stack test thread");
    match join.join() {
        Ok(()) => {}
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

#[test]
fn gc_collect_reclaims_unreachable_cycles_created_in_loop() {
    let code = compile_source(
        r#"for _ in range(600):
    a = []
    a.append(a)
    b = {}
    b['self'] = b
    a = None
    b = None
"#,
    );
    let mut vm = Vm::new();
    let baseline = vm.heap_object_count();
    vm.execute(&code).expect("execution should succeed");
    let after_execute = vm.heap_object_count();
    assert!(
        after_execute > baseline + 200,
        "expected heap growth before gc (baseline={baseline}, after_execute={after_execute})"
    );

    vm.gc_collect();
    let after_gc = vm.heap_object_count();
    assert!(
        after_gc <= baseline + 32,
        "expected gc to reclaim cycles (baseline={baseline}, after_gc={after_gc}, after_execute={after_execute})"
    );
}

#[test]
fn repeated_execute_plus_gc_keeps_heap_growth_bounded() {
    let code = compile_source(
        r#"for _ in range(200):
    a = []
    a.append(a)
    b = {}
    b['self'] = b
    a = None
    b = None
"#,
    );
    let mut vm = Vm::new();
    let baseline = vm.heap_object_count();
    for _ in 0..12 {
        vm.execute(&code).expect("execution should succeed");
        vm.gc_collect();
    }
    let after = vm.heap_object_count();
    assert!(
        after <= baseline + 48,
        "heap grew unexpectedly across repeated execute/gc cycles (baseline={baseline}, after={after})"
    );
}

#[test]
fn repeated_stdlib_import_exec_plus_gc_stays_bounded_after_warmup() {
    run_with_large_stack("gc-regression-stdlib-import-loop", move || {
        let Some(lib) = cpython_lib_path() else {
            eprintln!("skipping stdlib gc regression (CPython Lib not available)");
            return;
        };
        let code = compile_source(
            r#"import importlib
mods = ("argparse", "json", "csv", "pickle", "re")
for _ in range(12):
    for name in mods:
        mod = importlib.import_module(name)
        _ = mod.__name__
"#,
        );
        let mut vm = Vm::new();
        vm.add_module_path(&lib);

        vm.execute(&code).expect("warmup execute should succeed");
        vm.gc_collect();
        let after_warmup = vm.heap_object_count();

        for _ in 0..6 {
            vm.execute(&code).expect("repeat execute should succeed");
            vm.gc_collect();
        }
        let after_repeats = vm.heap_object_count();
        assert!(
            after_repeats <= after_warmup + 256,
            "heap grew unexpectedly after stdlib warmup (warmup={after_warmup}, after={after_repeats})"
        );
    });
}
