use pyrs::{compiler, parser, vm::Vm};

fn compile_source(source: &str) -> pyrs::bytecode::CodeObject {
    let module = parser::parse_module(source).expect("parse should succeed");
    compiler::compile_module(&module).expect("compile should succeed")
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
