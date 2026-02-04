use pyrs::{compiler, parser, runtime::Value, vm::Vm};

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
    assert_eq!(vm.get_global("x"), Some(&Value::Int(5)));
}

#[test]
fn executes_binary_expression_assignment() {
    let module = parser::parse_module("x = 1 + 2 * 3").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(7)));
}

#[test]
fn executes_comparison_assignment() {
    let module = parser::parse_module("x = 1 < 2").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Bool(true)));
}

#[test]
fn executes_comparison_variants() {
    let source = "a = 1 != 2\nb = 2 <= 2\nc = 3 > 2\nd = 2 >= 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(&Value::Bool(true)));
    assert_eq!(vm.get_global("b"), Some(&Value::Bool(true)));
    assert_eq!(vm.get_global("c"), Some(&Value::Bool(true)));
    assert_eq!(vm.get_global("d"), Some(&Value::Bool(false)));
}

#[test]
fn executes_unary_minus_assignment() {
    let module = parser::parse_module("x = -1").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(-1)));
}

#[test]
fn executes_boolean_literal_assignment() {
    let module = parser::parse_module("x = True").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Bool(true)));
}

#[test]
fn executes_function_definition_and_call() {
    let source = "def add(a, b):\n    return a + b\nx = add(2, 3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(5)));
}

#[test]
fn executes_function_without_return() {
    let source = "def noop():\n    x = 1\ny = noop()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(&Value::None));
}

#[test]
fn function_locals_do_not_override_globals() {
    let source = "x = 1\ndef f():\n    x = 2\nf()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(1)));
}

#[test]
fn executes_builtin_len() {
    let source = "x = len('hello')";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(5)));
}

#[test]
fn executes_list_literal_and_subscript() {
    let source = "x = [1, 2, 3]\ny = x[1]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(&Value::Int(2)));
}

#[test]
fn executes_negative_indexing() {
    let source = "x = [1, 2, 3]\ny = x[-1]\nz = (1, 2, 3)[-2]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(&Value::Int(3)));
    assert_eq!(vm.get_global("z"), Some(&Value::Int(2)));
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
        Some(&Value::List(vec![Value::Int(2), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("b"),
        Some(&Value::List(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(
        vm.get_global("c"),
        Some(&Value::List(vec![Value::Int(1), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("d"),
        Some(&Value::List(vec![
            Value::Int(4),
            Value::Int(3),
            Value::Int(2),
            Value::Int(1)
        ]))
    );
    assert_eq!(
        vm.get_global("u"),
        Some(&Value::Tuple(vec![
            Value::Int(2),
            Value::Int(3),
            Value::Int(4)
        ]))
    );
    assert_eq!(
        vm.get_global("v"),
        Some(&Value::Str("bc".to_string()))
    );
}

#[test]
fn executes_len_on_list() {
    let source = "x = len([1, 2, 3])";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(3)));
}

#[test]
fn executes_tuple_and_dict() {
    let source = "t = (1, 2)\nfirst = t[0]\nd = {'a': 1, 'b': 2}\nval = d['b']\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("first"), Some(&Value::Int(1)));
    assert_eq!(vm.get_global("val"), Some(&Value::Int(2)));
}

#[test]
fn executes_string_indexing() {
    let source = "x = 'cat'\ny = x[1]\nz = x[-1]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(&Value::Str("a".to_string())));
    assert_eq!(vm.get_global("z"), Some(&Value::Str("t".to_string())));
}

#[test]
fn executes_subscript_assignment() {
    let source = "x = [1, 2]\nx[0] = 5\nd = {'a': 1}\nd['a'] = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::List(vec![Value::Int(5), Value::Int(2)])));
    assert_eq!(
        vm.get_global("d"),
        Some(&Value::Dict(vec![(
            Value::Str("a".to_string()),
            Value::Int(3)
        )]))
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
    assert_eq!(vm.get_global("x"), Some(&Value::Int(8)));
}

#[test]
fn executes_modulo() {
    let source = "x = 5 % 2\ny = 9 % 4\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(1)));
    assert_eq!(vm.get_global("y"), Some(&Value::Int(1)));
}

#[test]
fn executes_multiplication_and_concat() {
    let source = "a = 'hi' * 3\nb = [1] * 2\nc = (1,) * 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(&Value::Str("hihihi".to_string())));
    assert_eq!(vm.get_global("b"), Some(&Value::List(vec![Value::Int(1), Value::Int(1)])));
    assert_eq!(
        vm.get_global("c"),
        Some(&Value::Tuple(vec![
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
        Some(&Value::List(vec![Value::Int(0), Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(
        vm.get_global("b"),
        Some(&Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]))
    );
    assert_eq!(
        vm.get_global("c"),
        Some(&Value::List(vec![Value::Int(5), Value::Int(3), Value::Int(1)]))
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
    assert_eq!(vm.get_global("x"), Some(&Value::Int(3)));
    assert_eq!(vm.get_global("y"), Some(&Value::Int(1)));
}

#[test]
fn executes_for_loop_over_list() {
    let source = "x = 0\nfor i in [1, 2, 3]:\n    x = x + i\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(6)));
}

#[test]
fn executes_for_loop_over_range() {
    let source = "x = 0\nfor i in range(3):\n    x = x + i\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(3)));
}

#[test]
fn executes_break_in_while_loop() {
    let source = "x = 0\nwhile 1:\n    x = x + 1\n    break\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(1)));
}

#[test]
fn executes_continue_in_while_loop() {
    let source = "x = 0\nwhile x < 3:\n    x = x + 1\n    continue\n    x = 100\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(3)));
}

#[test]
fn executes_boolean_operators() {
    let source = "x = not False\ny = 0 or 5\nz = 1 and 2\nw = 0 and 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Bool(true)));
    assert_eq!(vm.get_global("y"), Some(&Value::Int(5)));
    assert_eq!(vm.get_global("z"), Some(&Value::Int(2)));
    assert_eq!(vm.get_global("w"), Some(&Value::Int(0)));
}

#[test]
fn executes_in_operator() {
    let source = "a = 2 in [1, 2, 3]\nb = 'a' in 'cat'\nc = 'b' not in 'cat'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(&Value::Bool(true)));
    assert_eq!(vm.get_global("b"), Some(&Value::Bool(true)));
    assert_eq!(vm.get_global("c"), Some(&Value::Bool(true)));
}

#[test]
fn executes_if_expression() {
    let source = "x = 1 if 0 else 2\ny = 3 if 1 else 4\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(2)));
    assert_eq!(vm.get_global("y"), Some(&Value::Int(3)));
}

#[test]
fn executes_if_elif_else_statement() {
    let source = "x = 0\nif 0:\n    x = 1\nelif 1:\n    x = 2\nelse:\n    x = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(2)));
}

#[test]
fn executes_if_else_statement() {
    let source = "if 0:\n    x = 1\nelse:\n    x = 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(2)));
}

#[test]
fn executes_if_true_statement() {
    let source = "if 1:\n    x = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(3)));
}

#[test]
fn executes_while_loop() {
    let source = "x = 3\nwhile x:\n    x = x - 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(&Value::Int(0)));
}
