#![cfg(not(target_arch = "wasm32"))]

use pyrs::{compiler, parser, runtime::Value, vm::Vm};

#[test]
fn try_finally_runs_on_return() {
    let source = "x = 0\ndef f():\n    global x\n    try:\n        return 7\n    finally:\n        x = 2\nresult = f()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("result"), Some(Value::Int(7)));
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn try_except_finally_runs_on_return() {
    let source = "x = 0\ndef f():\n    global x\n    try:\n        raise ValueError('bad')\n    except ValueError:\n        return 11\n    finally:\n        x = 5\nresult = f()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("result"), Some(Value::Int(11)));
    assert_eq!(vm.get_global("x"), Some(Value::Int(5)));
}

#[test]
fn finally_return_overrides_try_return() {
    let source =
        "def f():\n    try:\n        return 1\n    finally:\n        return 2\nresult = f()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("result"), Some(Value::Int(2)));
}
