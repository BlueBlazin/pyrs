use pyrs::{
    compiler,
    parser,
    runtime::{ExceptionObject, ModuleObject, Value},
    vm::Vm,
};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn executes_constant_expression() {
    let module = parser::parse_module("42").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
}

#[test]
fn executes_name_expression_with_global() {
    let module = parser::parse_module("x").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.set_global("x", Value::Int(7));
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
}

#[test]
fn executes_assignment_statement() {
    let module = parser::parse_module("x = 5\nx").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(5)));
}

#[test]
fn executes_binary_expression_assignment() {
    let module = parser::parse_module("x = 1 + 2 * 3").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(7)));
}

#[test]
fn executes_comparison_assignment() {
    let module = parser::parse_module("x = 1 < 2").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Bool(true)));
}

#[test]
fn executes_comparison_variants() {
    let source = "a = 1 != 2\nb = 2 <= 2\nc = 3 > 2\nd = 2 >= 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("b"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("c"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("d"), Some(Value::Bool(false)));
}

#[test]
fn executes_bool_int_numeric_ops() {
    let source = "a = True + 1\nb = False + 2\nc = True * 3\nd = True == 1\ne = True < 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("c"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("d"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("e"), Some(Value::Bool(true)));
}

#[test]
fn executes_unary_minus_assignment() {
    let module = parser::parse_module("x = -1").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(-1)));
}

#[test]
fn executes_unary_plus_assignment() {
    let module = parser::parse_module("x = +1\ny = +True").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(1)));
}

#[test]
fn executes_boolean_literal_assignment() {
    let module = parser::parse_module("x = True").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Bool(true)));
}

#[test]
fn executes_function_definition_and_call() {
    let source = "def add(a, b):\n    return a + b\nx = add(2, 3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(5)));
}

#[test]
fn executes_global_assignment() {
    let source = "x = 1\ndef f():\n    global x\n    x = 3\nf()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_global_augmented_assignment() {
    let source = "x = 2\ndef f():\n    global x\n    x += 4\nf()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(6)));
}

#[test]
fn executes_function_without_return() {
    let source = "def noop():\n    x = 1\ny = noop()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::None));
}

#[test]
fn function_locals_do_not_override_globals() {
    let source = "x = 1\ndef f():\n    x = 2\nf()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
}

#[test]
fn executes_builtin_len() {
    let source = "x = len('hello')";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(5)));
}

#[test]
fn executes_list_literal_and_subscript() {
    let source = "x = [1, 2, 3]\ny = x[1]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(2)));
}

#[test]
fn executes_negative_indexing() {
    let source = "x = [1, 2, 3]\ny = x[-1]\nz = (1, 2, 3)[-2]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("z"), Some(Value::Int(2)));
}

#[test]
fn executes_slicing() {
    let source = "x = [1, 2, 3, 4]\n\
a = x[1:3]\n\
b = x[:2]\n\
c = x[::2]\n\
d = x[::-1]\n\
t = (1, 2, 3, 4)\n\
u = t[1:]\n\
s = 'abcd'\n\
v = s[1:3]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("a"),
        Some(Value::List(vec![Value::Int(2), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("b"),
        Some(Value::List(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(
        vm.get_global("c"),
        Some(Value::List(vec![Value::Int(1), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("d"),
        Some(Value::List(vec![
            Value::Int(4),
            Value::Int(3),
            Value::Int(2),
            Value::Int(1)
        ]))
    );
    assert_eq!(
        vm.get_global("u"),
        Some(Value::Tuple(vec![
            Value::Int(2),
            Value::Int(3),
            Value::Int(4)
        ]))
    );
    assert_eq!(
        vm.get_global("v"),
        Some(Value::Str("bc".to_string()))
    );
}

#[test]
fn executes_slice_builtin() {
    let source = "x = [1, 2, 3, 4]\n\
y = x[slice(1, 3)]\n\
z = x[slice(None, None, -1)]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("y"),
        Some(Value::List(vec![Value::Int(2), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("z"),
        Some(Value::List(vec![
            Value::Int(4),
            Value::Int(3),
            Value::Int(2),
            Value::Int(1)
        ]))
    );
}

#[test]
fn executes_bool_int_str_builtins() {
    let source = "a = bool([])\n\
b = bool(1)\n\
c = int(True)\n\
d = int('5')\n\
e = str(3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("b"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("c"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("d"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("e"), Some(Value::Str("3".to_string())));
}

#[test]
fn executes_abs_builtin() {
    let source = "a = abs(-3)\n\
b = abs(True)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(1)));
}

#[test]
fn executes_sum_builtin() {
    let source = "a = sum([1, 2, 3])\n\
b = sum((1, 2), 5)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(6)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(8)));
}

#[test]
fn executes_min_max_builtins() {
    let source = "a = min(3, 1, 2)\n\
b = max([1, 5, 2])\n\
c = min('b', 'a')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("c"), Some(Value::Str("a".to_string())));
}

#[test]
fn executes_all_any_builtins() {
    let source = "a = all([1, 2, 0])\n\
b = any([0, 0, 3])\n\
c = all((1, 2, 3))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("b"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("c"), Some(Value::Bool(true)));
}

#[test]
fn executes_pow_builtin() {
    let source = "a = pow(2, 3)\n\
b = pow(2, 3, 5)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(8)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(3)));
}

#[test]
fn executes_list_tuple_builtins() {
    let source = "a = list()\n\
b = list((1, 2))\n\
c = tuple([1, 2])\n\
d = list('ab')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::List(vec![])));
    assert_eq!(
        vm.get_global("b"),
        Some(Value::List(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(
        vm.get_global("c"),
        Some(Value::Tuple(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(
        vm.get_global("d"),
        Some(Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string())
        ]))
    );
}

#[test]
fn executes_divmod_builtin() {
    let source = "a = divmod(7, 3)\n\
b = divmod(-7, 3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("a"),
        Some(Value::Tuple(vec![Value::Int(2), Value::Int(1)]))
    );
    assert_eq!(
        vm.get_global("b"),
        Some(Value::Tuple(vec![Value::Int(-3), Value::Int(2)]))
    );
}

#[test]
fn executes_sorted_builtin() {
    let source = "a = sorted([3, 1, 2])\n\
b = sorted(('b', 'a'))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("a"),
        Some(Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("b"),
        Some(Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string())
        ]))
    );
}

#[test]
fn executes_try_except_statement() {
    let source =
        "try:\n    raise ValueError('bad')\nexcept ValueError as err:\n    x = 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
    assert_eq!(
        vm.get_global("err"),
        Some(Value::Exception(ExceptionObject {
            name: "ValueError".to_string(),
            message: Some("bad".to_string()),
        }))
    );
}

#[test]
fn executes_try_except_else_statement() {
    let source = "try:\n    x = 1\nexcept Exception:\n    x = 2\nelse:\n    x = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_try_except_runtime_error() {
    let source = "try:\n    x = 1 // 0\nexcept RuntimeError:\n    x = 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
}

#[test]
fn executes_assert_statement() {
    let source = "assert 1\nx = 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn assert_raises_on_false() {
    let source = "assert 0, 'nope'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("assert should raise");
    assert!(err.message.contains("AssertionError"));
}

#[test]
fn executes_is_operator() {
    let source = "a = None\nb = None\nc = 1\nd = 2\nx = (a is b)\ny = (c is d)\nz = (c is not d)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("y"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("z"), Some(Value::Bool(true)));
}

#[test]
fn executes_lambda_expression() {
    let source = "f = lambda x: x + 1\nx = f(2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_lambda_multiple_args() {
    let source = "g = lambda a, b: a * b\nx = g(3, 4)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(12)));
}

#[test]
fn executes_module_attribute_access() {
    let module = parser::parse_module("y = mod.x").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let module_obj = Rc::new(ModuleObject::new("mod"));
    module_obj
        .globals
        .borrow_mut()
        .insert("x".to_string(), Value::Int(42));
    vm.set_global("mod", Value::Module(module_obj));
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(42)));
}

#[test]
fn executes_module_attribute_assignment() {
    let module = parser::parse_module("mod.x = 7").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let module_obj = Rc::new(ModuleObject::new("mod"));
    vm.set_global("mod", Value::Module(module_obj.clone()));
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);

    let stored = vm.get_global("mod").expect("module exists");
    match stored {
        Value::Module(module) => {
            let globals = module.globals.borrow();
            assert_eq!(globals.get("x"), Some(&Value::Int(7)));
        }
        other => panic!("expected module, got {other:?}"),
    }
}

#[test]
fn executes_import_statement() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_import_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let module_path = temp_dir.join("mod.py");
    std::fs::write(&module_path, "value = 10\n").expect("write module");

    let source = "import mod\ny = mod.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(10)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_import_alias() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_import_alias_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let module_path = temp_dir.join("mod.py");
    std::fs::write(&module_path, "value = 11\n").expect("write module");

    let source = "import mod as m\ny = m.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(11)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_from_import_statement() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_from_import_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let module_path = temp_dir.join("mod.py");
    std::fs::write(&module_path, "value = 5\nother = 7\n").expect("write module");

    let source = "from mod import value, other\nx = value\ny = other\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(7)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_from_import_alias() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_from_import_alias_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let module_path = temp_dir.join("mod.py");
    std::fs::write(&module_path, "value = 21\n").expect("write module");

    let source = "from mod import value as v\nx = v\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(21)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_len_on_list() {
    let source = "x = len([1, 2, 3])";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_tuple_and_dict() {
    let source = "t = (1, 2)\nfirst = t[0]\nd = {'a': 1, 'b': 2}\nval = d['b']\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("first"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("val"), Some(Value::Int(2)));
}

#[test]
fn executes_string_indexing() {
    let source = "x = 'cat'\ny = x[1]\nz = x[-1]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Str("a".to_string())));
    assert_eq!(vm.get_global("z"), Some(Value::Str("t".to_string())));
}

#[test]
fn executes_subscript_assignment() {
    let source = "x = [1, 2]\nx[0] = 5\nd = {'a': 1}\nd['a'] = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::List(vec![Value::Int(5), Value::Int(2)])));
    assert_eq!(
        vm.get_global("d"),
        Some(Value::Dict(vec![(
            Value::Str("a".to_string()),
            Value::Int(3)
        )]))
    );
}

#[test]
fn executes_negative_index_assignment() {
    let source = "x = [1, 2, 3]\nx[-1] = 9\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("x"),
        Some(Value::List(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(9)
        ]))
    );
}

#[test]
fn executes_augmented_assignment() {
    let source = "x = 1\nx += 2\nx *= 3\nx -= 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(8)));
}

#[test]
fn executes_modulo() {
    let source = "x = 5 % 2\ny = 9 % 4\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(1)));
}

#[test]
fn executes_floor_division() {
    let source = "x = 7 // 2\ny = -3 // 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(-2)));
}

#[test]
fn executes_multiplication_and_concat() {
    let source = "a = 'hi' * 3\nb = [1] * 2\nc = (1,) * 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Str("hihihi".to_string())));
    assert_eq!(vm.get_global("b"), Some(Value::List(vec![Value::Int(1), Value::Int(1)])));
    assert_eq!(
        vm.get_global("c"),
        Some(Value::Tuple(vec![
            Value::Int(1),
            Value::Int(1),
            Value::Int(1)
        ]))
    );
}

#[test]
fn executes_range_variants() {
    let source = "a = range(3)\nb = range(1, 4)\nc = range(5, 0, -2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("a"),
        Some(Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(
        vm.get_global("b"),
        Some(Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("c"),
        Some(Value::List(vec![Value::Int(5), Value::Int(3), Value::Int(1)]))
    );
}

#[test]
fn executes_len_on_tuple_dict() {
    let source = "x = len((1, 2, 3))\ny = len({'a': 1})\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(1)));
}

#[test]
fn executes_for_loop_over_list() {
    let source = "x = 0\nfor i in [1, 2, 3]:\n    x = x + i\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(6)));
}

#[test]
fn executes_for_loop_over_range() {
    let source = "x = 0\nfor i in range(3):\n    x = x + i\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_break_in_while_loop() {
    let source = "x = 0\nwhile 1:\n    x = x + 1\n    break\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
}

#[test]
fn executes_continue_in_while_loop() {
    let source = "x = 0\nwhile x < 3:\n    x = x + 1\n    continue\n    x = 100\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_boolean_operators() {
    let source = "x = not False\ny = 0 or 5\nz = 1 and 2\nw = 0 and 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("z"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("w"), Some(Value::Int(0)));
}

#[test]
fn executes_in_operator() {
    let source = "a = 2 in [1, 2, 3]\nb = 'a' in 'cat'\nc = 'b' not in 'cat'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("b"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("c"), Some(Value::Bool(true)));
}

#[test]
fn executes_if_expression() {
    let source = "x = 1 if 0 else 2\ny = 3 if 1 else 4\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(3)));
}

#[test]
fn executes_if_elif_else_statement() {
    let source = "x = 0\nif 0:\n    x = 1\nelif 1:\n    x = 2\nelse:\n    x = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn executes_if_else_statement() {
    let source = "if 0:\n    x = 1\nelse:\n    x = 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn executes_if_true_statement() {
    let source = "if 1:\n    x = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_while_loop() {
    let source = "x = 3\nwhile x:\n    x = x - 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(0)));
}

#[test]
fn executes_while_else_clause() {
    let source = "x = 0\nwhile x < 2:\n    x += 1\nelse:\n    y = 5\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(5)));
}

#[test]
fn executes_for_else_clause() {
    let source = "y = 0\nfor i in [1, 2]:\n    pass\nelse:\n    y = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(3)));
}
