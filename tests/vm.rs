use pyrs::{
    compiler,
    parser,
    runtime::{BuiltinFunction, ExceptionObject, Object, Value},
    vm::Vm,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn list_values(value: Option<Value>) -> Option<Vec<Value>> {
    value.and_then(|val| val.as_list())
}

fn tuple_values(value: Option<Value>) -> Option<Vec<Value>> {
    value.and_then(|val| val.as_tuple())
}

fn dict_entries(value: Option<Value>) -> Option<Vec<(Value, Value)>> {
    value.and_then(|val| val.as_dict())
}

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
fn executes_destructuring_assignment() {
    let module = parser::parse_module("a, b = [1, 2]").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(2)));
}

#[test]
fn executes_destructuring_assignment_nested() {
    let source = "a, (b, c) = (1, (2, 3))";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("c"), Some(Value::Int(3)));
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
fn executes_function_defaults() {
    let source = "def add(a, b=2):\n    return a + b\nx = add(3)\ny = add(3, 4)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(7)));
}

#[test]
fn executes_keyword_only_parameters() {
    let source = "def add(a, *, b=2):\n    return a + b\nx = add(1, b=3)\ny = add(1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(4)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(3)));
}

#[test]
fn executes_positional_only_parameters() {
    let source = "def add(a, /, b):\n    return a + b\nx = add(1, 2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_keyword_arguments() {
    let source = "def add(a, b):\n    return a + b\nx = add(b=2, a=1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_module_annotations() {
    let source = "x: int\ny: str = 'a'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        dict_entries(vm.get_global("__annotations__")),
        Some(vec![
            (
                Value::Str("x".to_string()),
                Value::Builtin(BuiltinFunction::Int)
            ),
            (
                Value::Str("y".to_string()),
                Value::Builtin(BuiltinFunction::Str)
            ),
        ])
    );
}

#[test]
fn executes_function_annotations() {
    let source = "def f(x: int, y: str = 'a') -> int:\n    z: int\n    return __annotations__\n\nout = f(1)\nann = f.__annotations__\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        dict_entries(vm.get_global("ann")),
        Some(vec![
            (
                Value::Str("x".to_string()),
                Value::Builtin(BuiltinFunction::Int)
            ),
            (
                Value::Str("y".to_string()),
                Value::Builtin(BuiltinFunction::Str)
            ),
            (
                Value::Str("return".to_string()),
                Value::Builtin(BuiltinFunction::Int)
            ),
        ])
    );
    assert_eq!(
        dict_entries(vm.get_global("out")),
        Some(vec![(
            Value::Str("z".to_string()),
            Value::Builtin(BuiltinFunction::Int)
        )])
    );
}

#[test]
fn executes_generator_for_loop() {
    let source = "def gen():\n    yield 1\n    yield 2\nvals = []\nfor x in gen():\n    vals += [x]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        list_values(vm.get_global("vals")),
        Some(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn executes_generator_send_next_and_close() {
    let source = "def gen():\n    yield 1\n    yield 2\ng = gen()\na = g.send(None)\nb = g.__next__()\nc = g.close()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("c"), Some(Value::None));
}

#[test]
fn executes_generator_lazily_on_iteration() {
    let source = "state = 0\ndef gen():\n    global state\n    state = state + 1\n    yield state\ng = gen()\ncreated = state\nfirst = g.__next__()\nafter = state\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("created"), Some(Value::Int(0)));
    assert_eq!(vm.get_global("first"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("after"), Some(Value::Int(1)));
}

#[test]
fn executes_generator_send_value_into_yield_expression() {
    let source = "def gen():\n    x = yield 1\n    yield x\ng = gen()\na = g.send(None)\nb = g.send(5)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(5)));
}

#[test]
fn executes_generator_throw_into_suspended_frame() {
    let source = "def gen():\n    try:\n        yield 1\n    except ValueError:\n        yield 2\ng = gen()\na = g.__next__()\nb = g.throw(ValueError)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(2)));
}

#[test]
fn executes_generator_reentrancy_error() {
    let source = "caught = False\ndef gen():\n    global caught\n    try:\n        g.__next__()\n    except RuntimeError:\n        caught = True\n    yield 1\ng = gen()\na = g.__next__()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("caught"), Some(Value::Bool(true)));
}

#[test]
fn executes_generator_close_runs_finally() {
    let source = "closed = False\ndef gen():\n    global closed\n    try:\n        yield 1\n    finally:\n        closed = True\ng = gen()\na = g.__next__()\nb = g.close()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::None));
    assert_eq!(vm.get_global("closed"), Some(Value::Bool(true)));
}

#[test]
fn executes_generator_yield_from() {
    let source = "def gen():\n    yield from [1, 2, 3]\nvals = []\nfor x in gen():\n    vals += [x]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        list_values(vm.get_global("vals")),
        Some(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn executes_generator_yield_from_send_delegation() {
    let source = "def sub():\n    x = yield 1\n    yield x\n\ndef outer():\n    yield from sub()\n\ng = outer()\na = g.send(None)\nb = g.send(9)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(9)));
}

#[test]
fn executes_generator_yield_from_send_non_none_to_plain_iterator() {
    let source = "def gen():\n    yield from [1, 2]\n\ng = gen()\na = g.__next__()\ncaught = False\ntry:\n    g.send(7)\nexcept AttributeError:\n    caught = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("caught"), Some(Value::Bool(true)));
}

#[test]
fn executes_generator_yield_from_throw_delegation() {
    let source = "def sub():\n    try:\n        yield 1\n    except ValueError:\n        yield 2\n\ndef outer():\n    yield from sub()\n\ng = outer()\na = g.__next__()\nb = g.throw(ValueError)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(2)));
}

#[test]
fn executes_generator_yield_from_throw_to_plain_iterator() {
    let source = "def outer():\n    yield from [1, 2]\n\ng = outer()\na = g.__next__()\ncaught = False\ntry:\n    g.throw(ValueError)\nexcept ValueError:\n    caught = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("caught"), Some(Value::Bool(true)));
}

#[test]
fn executes_generator_yield_from_return_value_propagation() {
    let source = "def inner():\n    return 42\n    yield 0\n\ndef outer():\n    x = yield from inner()\n    yield x\n\ng = outer()\nv = g.__next__()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("v"), Some(Value::Int(42)));
}

#[test]
fn executes_generator_yield_from_close_delegation() {
    let source = "closed = False\ndef sub():\n    global closed\n    try:\n        yield 1\n    finally:\n        closed = True\n\ndef outer():\n    yield from sub()\n\ng = outer()\na = g.__next__()\nb = g.close()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::None));
    assert_eq!(vm.get_global("closed"), Some(Value::Bool(true)));
}

#[test]
fn executes_generator_exit_not_caught_by_exception_handler() {
    let source = "flag = 0\ndef gen():\n    global flag\n    try:\n        yield 1\n    except Exception:\n        flag = flag + 1\n    finally:\n        flag = flag + 2\n\ng = gen()\na = g.__next__()\nb = g.close()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::None));
    assert_eq!(vm.get_global("flag"), Some(Value::Int(2)));
}

#[test]
fn executes_generator_yield_from_lazily() {
    let source = "state = 0\ndef inner():\n    global state\n    state = state + 1\n    yield state\ndef outer():\n    yield from inner()\ng = outer()\nbefore = state\nv = g.__next__()\nafter = state\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("before"), Some(Value::Int(0)));
    assert_eq!(vm.get_global("v"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("after"), Some(Value::Int(1)));
}

#[test]
fn executes_generator_throw_and_stop_iteration() {
    let source = "def gen():\n    yield 1\ng = gen()\nfirst = g.send(None)\nhandled = False\ntry:\n    g.throw(RuntimeError)\nexcept RuntimeError:\n    handled = True\nstopped = False\ntry:\n    g.send(None)\nexcept StopIteration:\n    stopped = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("first"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("handled"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("stopped"), Some(Value::Bool(true)));
}

#[test]
fn executes_star_args() {
    let source = "def add(a, b):\n    return a + b\nargs = [1, 2]\nx = add(*args)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_double_star_kwargs() {
    let source = "def add(a, b):\n    return a + b\nkwargs = {'a': 1, 'b': 2}\nx = add(**kwargs)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_varargs_definition() {
    let source = "def total(*args):\n    s = 0\n    for v in args:\n        s = s + v\n    return s\nx = total(1, 2, 3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(6)));
}

#[test]
fn executes_kwargs_definition() {
    let source = "def pick(**kw):\n    return kw['a']\nx = pick(a=3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_varargs_and_kwargs_definition() {
    let source = "def collect(a, *rest, **kw):\n    return len(rest) + kw['b'] + a\nx = collect(1, 2, 3, b=4)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(7)));
}

#[test]
fn executes_default_from_global() {
    let source = "x = 5\n\ndef f(a=x):\n    return a\n\ny = f()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(5)));
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
        list_values(vm.get_global("a")),
        Some(vec![Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        list_values(vm.get_global("b")),
        Some(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        list_values(vm.get_global("c")),
        Some(vec![Value::Int(1), Value::Int(3)])
    );
    assert_eq!(
        list_values(vm.get_global("d")),
        Some(vec![
            Value::Int(4),
            Value::Int(3),
            Value::Int(2),
            Value::Int(1)
        ])
    );
    assert_eq!(
        tuple_values(vm.get_global("u")),
        Some(vec![Value::Int(2), Value::Int(3), Value::Int(4)])
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
        list_values(vm.get_global("y")),
        Some(vec![Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        list_values(vm.get_global("z")),
        Some(vec![
            Value::Int(4),
            Value::Int(3),
            Value::Int(2),
            Value::Int(1)
        ])
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
fn executes_sum_with_keyword_start() {
    let source = "a = sum([1, 2, 3], start=4)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(10)));
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
    assert_eq!(list_values(vm.get_global("a")), Some(vec![]));
    assert_eq!(
        list_values(vm.get_global("b")),
        Some(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        tuple_values(vm.get_global("c")),
        Some(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        list_values(vm.get_global("d")),
        Some(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string())
        ])
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
        tuple_values(vm.get_global("a")),
        Some(vec![Value::Int(2), Value::Int(1)])
    );
    assert_eq!(
        tuple_values(vm.get_global("b")),
        Some(vec![Value::Int(-3), Value::Int(2)])
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
        list_values(vm.get_global("a")),
        Some(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        list_values(vm.get_global("b")),
        Some(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string())
        ])
    );
}

#[test]
fn executes_sorted_with_reverse() {
    let source = "a = sorted([3, 1, 2], reverse=True)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        list_values(vm.get_global("a")),
        Some(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
    );
}

#[test]
fn executes_enumerate_builtin() {
    let source = "a = enumerate([1, 2])\n\
b = enumerate('ab', start=1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        list_values(vm.get_global("a")),
        Some(vec![
            vm.alloc_tuple(vec![Value::Int(0), Value::Int(1)]),
            vm.alloc_tuple(vec![Value::Int(1), Value::Int(2)])
        ])
    );
    assert_eq!(
        list_values(vm.get_global("b")),
        Some(vec![
            vm.alloc_tuple(vec![Value::Int(1), Value::Str("a".to_string())]),
            vm.alloc_tuple(vec![Value::Int(2), Value::Str("b".to_string())])
        ])
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
    let source = "try:\n    x = 1 // 0\nexcept ZeroDivisionError:\n    x = 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
}

#[test]
fn executes_try_finally_statement() {
    let source = "try:\n    x = 1\nfinally:\n    x = 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn executes_try_finally_on_exception() {
    let source = "try:\n    x = 1 // 0\nfinally:\n    x = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should raise");
    assert!(err.message.contains("ZeroDivisionError"));
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
}

#[test]
fn executes_try_except_finally_statement() {
    let source = "try:\n    raise ValueError('bad')\nexcept ValueError:\n    x = 1\nfinally:\n    x = 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn executes_try_except_finally_unhandled_exception() {
    let source = "try:\n    x = 1 // 0\nexcept ValueError:\n    x = 1\nfinally:\n    x = 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should raise");
    assert!(err.message.contains("ZeroDivisionError"));
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn executes_class_definition_and_methods() {
    let source = "class Foo:\n    def __init__(self, x):\n        self.x = x\n    def get(self):\n        return self.x\n\nf = Foo(3)\ny = f.get()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(3)));
}

#[test]
fn executes_class_attribute_lookup() {
    let source = "class Bar:\n    kind = 'bar'\n\nb = Bar()\nx = b.kind\ny = Bar.kind\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Str("bar".to_string())));
    assert_eq!(vm.get_global("y"), Some(Value::Str("bar".to_string())));
}

#[test]
fn class_body_can_access_module_globals() {
    let source = "x = 5\nclass Baz:\n    y = x\n    def get(self):\n        return x\n\nb = Baz()\nval = b.get()\nattr = Baz.y\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("val"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("attr"), Some(Value::Int(5)));
}

#[test]
fn executes_class_inheritance() {
    let source = "class Base:\n    def __init__(self, x):\n        self.x = x\n    def get(self):\n        return self.x\n\nclass Child(Base):\n    pass\n\nc = Child(7)\nval = c.get()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("val"), Some(Value::Int(7)));
}

#[test]
fn init_returning_value_raises() {
    let source = "class Bad:\n    def __init__(self):\n        return 1\nBad()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("__init__() should return None"));
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
fn executes_lambda_defaults() {
    let source = "f = lambda x=2: x + 1\na = f()\nb = f(3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(4)));
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
    let module_value = vm.alloc_module("mod");
    if let Value::Module(obj) = &module_value {
        if let pyrs::runtime::Object::Module(module_data) = &mut *obj.kind_mut() {
            module_data
                .globals
                .insert("x".to_string(), Value::Int(42));
        }
    }
    vm.set_global("mod", module_value);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(42)));
}

#[test]
fn executes_module_attribute_assignment() {
    let module = parser::parse_module("mod.x = 7").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let module_value = vm.alloc_module("mod");
    vm.set_global("mod", module_value);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);

    let stored = vm.get_global("mod").expect("module exists");
    match stored {
        Value::Module(module) => {
            if let pyrs::runtime::Object::Module(module_data) = &*module.kind() {
                assert_eq!(module_data.globals.get("x"), Some(&Value::Int(7)));
            } else {
                panic!("expected module data");
            }
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
fn executes_dotted_import_statement() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_import_dotted_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");

    let module_path = pkg_dir.join("sub.py");
    std::fs::write(&module_path, "value = 13\n").expect("write module");

    let source = "import pkg.sub\nx = pkg.sub.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(13)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_dir(&pkg_dir);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_import_submodule_attribute() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_import_pkg_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");

    let module_path = pkg_dir.join("sub.py");
    std::fs::write(&module_path, "value = 19\n").expect("write module");

    let source = "import pkg\nx = pkg.sub.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(19)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&pkg_dir);
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
fn executes_from_dotted_import_statement() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_from_import_dotted_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");

    let module_path = pkg_dir.join("sub.py");
    std::fs::write(&module_path, "value = 17\n").expect("write module");

    let source = "from pkg.sub import value\nx = value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(17)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_dir(&pkg_dir);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_from_import_submodule() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_from_import_pkg_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");

    let module_path = pkg_dir.join("sub.py");
    std::fs::write(&module_path, "value = 23\n").expect("write module");

    let source = "from pkg import sub\nx = sub.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(23)));

    let _ = std::fs::remove_file(&module_path);
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&pkg_dir);
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
fn executes_relative_from_import_in_package_module() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_relative_import_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");
    std::fs::write(pkg_dir.join("sub.py"), "value = 31\n").expect("write sub module");
    std::fs::write(
        pkg_dir.join("mod.py"),
        "from .sub import value\nresult = value\n",
    )
    .expect("write package module");

    let source = "import pkg.mod\nx = pkg.mod.result\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(31)));

    let _ = std::fs::remove_file(pkg_dir.join("mod.py"));
    let _ = std::fs::remove_file(pkg_dir.join("sub.py"));
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&pkg_dir);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn sets_module_import_metadata_fields() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_import_metadata_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");
    std::fs::write(pkg_dir.join("sub.py"), "value = 7\n").expect("write sub module");

    let source = "\
import pkg.sub\n\
pkg_name = pkg.__name__\n\
pkg_package = pkg.__package__\n\
pkg_spec_name = pkg.__spec__['name']\n\
pkg_path_len = len(pkg.__path__)\n\
sub_package = pkg.sub.__package__\n\
sub_spec_parent = pkg.sub.__spec__['parent']\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("pkg_name"), Some(Value::Str("pkg".to_string())));
    assert_eq!(vm.get_global("pkg_package"), Some(Value::Str("pkg".to_string())));
    assert_eq!(
        vm.get_global("pkg_spec_name"),
        Some(Value::Str("pkg".to_string()))
    );
    assert_eq!(vm.get_global("pkg_path_len"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("sub_package"), Some(Value::Str("pkg".to_string())));
    assert_eq!(
        vm.get_global("sub_spec_parent"),
        Some(Value::Str("pkg".to_string()))
    );

    let _ = std::fs::remove_file(pkg_dir.join("sub.py"));
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&pkg_dir);
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
fn executes_len_with_keyword() {
    let source = "x = len(obj=[1, 2])";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
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
    assert_eq!(
        list_values(vm.get_global("x")),
        Some(vec![Value::Int(5), Value::Int(2)])
    );
    assert_eq!(
        dict_entries(vm.get_global("d")),
        Some(vec![(Value::Str("a".to_string()), Value::Int(3))])
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
        list_values(vm.get_global("x")),
        Some(vec![Value::Int(1), Value::Int(2), Value::Int(9)])
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
fn executes_augmented_assignment_variants() {
    let source = "x = 10\nx %= 4\nx //= 2\nx **= 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
}

#[test]
fn executes_for_with_tuple_target() {
    let source = r#"pairs = [(1, 2), (3, 4)]
total = 0
for a, b in pairs:
    total = total + a + b
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("total"), Some(Value::Int(10)));
}

#[test]
fn executes_with_statement_with_target() {
    let source = r#"class C:
    def __init__(self):
        self.state = 0
    def __enter__(self):
        self.state = 1
        return self
    def __exit__(self, exc_type, exc, tb):
        self.state = self.state + 1
        return False
with C() as c:
    c.state = c.state + 10
x = c.state
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(12)));
}

#[test]
fn executes_with_statement_without_target() {
    let source = r#"flag = 0
class C:
    def __enter__(self):
        return self
    def __exit__(self, exc_type, exc, tb):
        global flag
        flag = 1
        return False
with C():
    pass
x = flag
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
}

#[test]
fn executes_id_builtin_and_is_identity() {
    let source = "a = [1]\n\
b = a\n\
c = [1]\n\
x = id(a) == id(b)\n\
y = a is b\n\
z = a is c\n\
w = a == c\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("y"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("z"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("w"), Some(Value::Bool(true)));
}

#[test]
fn collects_self_referential_list_cycles() {
    let mut vm = Vm::new();
    let before = vm.heap_object_count();
    {
        let list_value = vm.alloc_list(Vec::new());
        let list_obj = match &list_value {
            Value::List(obj) => obj.clone(),
            _ => panic!("expected list"),
        };
        if let Object::List(values) = &mut *list_obj.kind_mut() {
            values.push(Value::List(list_obj.clone()));
        }
        vm.set_global("tmp", list_value);
    }
    vm.set_global("tmp", Value::None);
    vm.gc_collect();
    let after = vm.heap_object_count();
    assert_eq!(after, before);
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
fn executes_power_expression() {
    let source = "x = 2 ** 3 ** 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(512)));
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
    assert_eq!(
        list_values(vm.get_global("b")),
        Some(vec![Value::Int(1), Value::Int(1)])
    );
    assert_eq!(
        tuple_values(vm.get_global("c")),
        Some(vec![Value::Int(1), Value::Int(1), Value::Int(1)])
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
        list_values(vm.get_global("a")),
        Some(vec![Value::Int(0), Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        list_values(vm.get_global("b")),
        Some(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        list_values(vm.get_global("c")),
        Some(vec![Value::Int(5), Value::Int(3), Value::Int(1)])
    );
}

#[test]
fn executes_range_with_keywords() {
    let source =
        "a = range(stop=3)\nb = range(start=1, stop=4)\nc = range(start=1, stop=6, step=2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        list_values(vm.get_global("a")),
        Some(vec![Value::Int(0), Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        list_values(vm.get_global("b")),
        Some(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        list_values(vm.get_global("c")),
        Some(vec![Value::Int(1), Value::Int(3), Value::Int(5)])
    );
}

#[test]
fn executes_print_with_keywords() {
    let module =
        parser::parse_module("print(1, 2, sep='-', end='!')").expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
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
