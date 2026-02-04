use pyrs::{compiler, parser, runtime::Value, vm::Vm};

#[test]
fn empty_source_executes() {
    let module = parser::parse_module("").expect("empty source should parse");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
}
