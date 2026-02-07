use pyrs::{
    compiler, parser,
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

fn set_values(value: Option<Value>) -> Option<Vec<Value>> {
    match value {
        Some(Value::Set(obj)) => match &*obj.kind() {
            Object::Set(values) => Some(values.clone()),
            _ => None,
        },
        Some(Value::FrozenSet(obj)) => match &*obj.kind() {
            Object::FrozenSet(values) => Some(values.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn bytes_values(value: Option<Value>) -> Option<Vec<u8>> {
    match value {
        Some(Value::Bytes(obj)) => match &*obj.kind() {
            Object::Bytes(values) => Some(values.clone()),
            _ => None,
        },
        Some(Value::ByteArray(obj)) => match &*obj.kind() {
            Object::ByteArray(values) => Some(values.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn assert_float_global(vm: &Vm, name: &str, expected: f64) {
    match vm.get_global(name) {
        Some(Value::Float(actual)) => {
            assert!(
                (actual - expected).abs() < 1e-12,
                "expected {name}={expected}, got {actual}"
            );
        }
        other => panic!("expected float global {name}, got {other:?}"),
    }
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
    let module = parser::parse_module(&source).expect("parse should succeed");
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
    let module = parser::parse_module(&source).expect("parse should succeed");
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
    let module = parser::parse_module(&source).expect("parse should succeed");
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
fn executes_float_numeric_ops() {
    let source = "\
a = 1 / 2\n\
b = 5.0 / 2\n\
c = 5 // 2.0\n\
d = 5.0 % 2\n\
e = 2 ** -1\n\
f = +3.5\n\
g = -3.5\n\
h = 1.0 == 1\n\
i = 1.0 < 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_float_global(&vm, "a", 0.5);
    assert_float_global(&vm, "b", 2.5);
    assert_float_global(&vm, "c", 2.0);
    assert_float_global(&vm, "d", 1.0);
    assert_float_global(&vm, "e", 0.5);
    assert_float_global(&vm, "f", 3.5);
    assert_float_global(&vm, "g", -3.5);
    assert_eq!(vm.get_global("h"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("i"), Some(Value::Bool(true)));
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
fn executes_type_union_operator_for_annotations() {
    let source = "x: type[Warning] | None = None\ny = type[Warning] | None\nz = y | int\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    let y = tuple_values(vm.get_global("y")).expect("y should be a tuple");
    assert_eq!(y.len(), 2);
    assert!(matches!(y[0], Value::Instance(_)));
    assert_eq!(y[1], Value::None);
    let z = tuple_values(vm.get_global("z")).expect("z should be a tuple");
    assert_eq!(z.len(), 3);
    assert!(matches!(z[0], Value::Instance(_)));
    assert!(z.contains(&Value::None));
    assert!(z.contains(&Value::Builtin(BuiltinFunction::Int)));
}

#[test]
fn exposes_sys_version_info() {
    let source = "import sys\nv = sys.version_info\nmajor = v[0]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("major"), Some(Value::Int(3)));
}

#[test]
fn exposes_sys_warnoptions() {
    let source = "import sys\nn = len(sys.warnoptions)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("n"), Some(Value::Int(0)));
}

#[test]
fn exposes_sys_abiflags_and_platlibdir() {
    let source = "import sys\nok = isinstance(sys.abiflags, str) and sys.abiflags == '' and sys.platlibdir == 'lib'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_sys_filesystem_encoding_helpers() {
    let source = "import sys\nok = sys.getfilesystemencoding() == 'utf-8' and sys.getfilesystemencodeerrors() == 'surrogateescape'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_sys_standard_streams() {
    let source = "import sys\nok = hasattr(sys, 'stdout') and hasattr(sys, 'stderr') and hasattr(sys, 'stdin') and hasattr(sys.stderr, 'flush')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_sys_jit_probe_flags() {
    let source = "import sys\nok = hasattr(sys, '_jit') and (not sys._jit.is_enabled()) and (not sys._jit.is_available())\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn imports_sysconfigdata_module_for_platform() {
    let source = "import sys\nname = '_sysconfigdata__' + sys.platform + '_'\nm = __import__(name)\nok = hasattr(m, 'build_time_vars')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_inspect_signature_and_co_flags() {
    let source = "import inspect\nsig = inspect.signature(lambda x, y=1, /, z=2, *, w=3, **kw: x + y)\nparams = sig.parameters\nok = isinstance(params, dict) and params['x'][0] == 'POSITIONAL_ONLY' and params['y'][1] == 1 and params['z'][0] == 'POSITIONAL_OR_KEYWORD' and params['w'][0] == 'KEYWORD_ONLY' and params['kw'][0] == 'VAR_KEYWORD' and sig.return_annotation is None and inspect.CO_VARARGS == 4 and inspect.CO_VARKEYWORDS == 8 and inspect.CO_COROUTINE == 128\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_inspect_private_mro_helpers() {
    let source = "import inspect\nclass A:\n    pass\nclass B(A):\n    pass\nmro = inspect._static_getmro(B)\nd = inspect._get_dunder_dict_of_class(B)\nbuiltins_ns = inspect._get_dunder_dict_of_class(int)\ns = inspect._sentinel\nok = (mro[0] is B) and (mro[1] is A) and ('__name__' in d) and isinstance(builtins_ns, dict) and (inspect._sentinel is s)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_osx_support_customize_config_vars() {
    let source = "import _osx_support\ncfg = {'x': 1}\nout = _osx_support.customize_config_vars(cfg)\nok = out is cfg\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_object_dunder_ne_and_float_getformat() {
    let source = "a = object.__ne__(1, 2)\nb = float.__getformat__('double')\nok = (a is True) and isinstance(b, str)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_dict_attribute_methods() {
    let source = "d = {'a': 1, 'b': 2}\nks = d.keys()\nvs = d.values()\nis_ = d.items()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        list_values(vm.get_global("ks")),
        Some(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string())
        ])
    );
    assert_eq!(
        list_values(vm.get_global("vs")),
        Some(vec![Value::Int(1), Value::Int(2)])
    );
    let items = list_values(vm.get_global("is_")).expect("items should be a list");
    assert_eq!(items.len(), 2);
    assert_eq!(
        items
            .iter()
            .filter(|value| matches!(value, Value::Tuple(_)))
            .count(),
        2
    );
}

#[test]
fn executes_isinstance_and_issubclass_builtins() {
    let source = "a = isinstance(1, int)\nb = isinstance(True, int)\nc = issubclass(int, int)\nd = issubclass(DeprecationWarning, Warning)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("b"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("c"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("d"), Some(Value::Bool(true)));
}

#[test]
fn executes_callable_builtin() {
    let source = "def f():\n    return 1\na = callable(f)\nb = callable(1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("b"), Some(Value::Bool(false)));
}

#[test]
fn executes_reversed_builtin() {
    let source = "vals = reversed([1, 2, 3])\nout = []\nfor x in vals:\n    out += [x]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        list_values(vm.get_global("out")),
        Some(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
    );
}

#[test]
fn executes_zip_builtin() {
    let source = "pairs = zip([1, 2], ['a', 'b'])\nout = []\nfor x in pairs:\n    out += [x]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    let out = list_values(vm.get_global("out")).expect("out should be a list");
    assert_eq!(out.len(), 2);
    assert!(matches!(out[0], Value::Tuple(_)));
    assert!(matches!(out[1], Value::Tuple(_)));
}

#[test]
fn exposes_ellipsis_builtin() {
    let source = "x = Ellipsis\ny = type(...)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert!(matches!(vm.get_global("x"), Some(Value::Instance(_))));
    assert!(matches!(vm.get_global("y"), Some(Value::Class(_))));
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
    let source =
        "def gen():\n    yield 1\n    yield 2\nvals = []\nfor x in gen():\n    vals += [x]\n";
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
    let source =
        "def gen():\n    x = yield 1\n    yield x\ng = gen()\na = g.send(None)\nb = g.send(5)\n";
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
    let source =
        "def gen():\n    yield from [1, 2, 3]\nvals = []\nfor x in gen():\n    vals += [x]\n";
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
    assert_eq!(vm.get_global("v"), Some(Value::Str("bc".to_string())));
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
e = str(3)\n\
f = float(3)\n\
g = float('2.5')\n\
h = int(3.9)\n";
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
    assert_float_global(&vm, "f", 3.0);
    assert_float_global(&vm, "g", 2.5);
    assert_eq!(vm.get_global("h"), Some(Value::Int(3)));
}

#[test]
fn executes_abs_builtin() {
    let source = "a = abs(-3)\n\
b = abs(True)\n\
c = abs(-3.5)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(1)));
    assert_float_global(&vm, "c", 3.5);
}

#[test]
fn executes_sum_builtin() {
    let source = "a = sum([1, 2, 3])\n\
b = sum((1, 2), 5)\n\
c = sum([1.5, 2.5], 1.0)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(6)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(8)));
    assert_float_global(&vm, "c", 5.0);
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
fn executes_min_max_with_default_and_key() {
    let source = "a = min([], default=7)\n\
b = max([], default=-2)\n\
c = min(['bbb', 'a', 'cc'], key=len)\n\
d = max(['bbb', 'a', 'cc'], key=len)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(7)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(-2)));
    assert_eq!(vm.get_global("c"), Some(Value::Str("a".to_string())));
    assert_eq!(vm.get_global("d"), Some(Value::Str("bbb".to_string())));
}

#[test]
fn rejects_min_default_with_multiple_positional_args() {
    let source = "min(1, 2, default=0)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(
        err.message
            .contains("Cannot specify a default for min() with multiple positional arguments")
    );
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
b = pow(2, 3, 5)\n\
c = pow(2, -1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Int(8)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(3)));
    assert_float_global(&vm, "c", 0.5);
}

#[test]
fn executes_pow_builtin_large_modular_exponent_without_overflow() {
    let source = "m = 2305843009213693951\n\
value = pow(10, m - 2, m)\n\
ok = isinstance(value, int)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn executes_set_bytes_memoryview_and_complex_builtins() {
    let source = "s = set([1, 2, 2])\n\
fs = frozenset((1, 2, 2))\n\
b = bytes('ab')\n\
ba = bytearray([65, 66, 67])\n\
mv = memoryview(ba)\n\
x = b[1]\n\
y = ba[0]\n\
ba[1] = 90\n\
z = ba[1]\n\
w = mv[2]\n\
sb = b[0:2]\n\
c = complex(1, 2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");

    let mut set_vals = set_values(vm.get_global("s")).expect("set value");
    set_vals.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    assert_eq!(set_vals, vec![Value::Int(1), Value::Int(2)]);

    let mut frozen_vals = set_values(vm.get_global("fs")).expect("frozenset value");
    frozen_vals.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    assert_eq!(frozen_vals, vec![Value::Int(1), Value::Int(2)]);

    assert_eq!(bytes_values(vm.get_global("b")), Some(vec![97, 98]));
    assert_eq!(bytes_values(vm.get_global("ba")), Some(vec![65, 90, 67]));
    assert_eq!(vm.get_global("x"), Some(Value::Int(98)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(65)));
    assert_eq!(vm.get_global("z"), Some(Value::Int(90)));
    assert_eq!(vm.get_global("w"), Some(Value::Int(67)));
    assert_eq!(bytes_values(vm.get_global("sb")), Some(vec![97, 98]));
    assert_eq!(
        vm.get_global("c"),
        Some(Value::Complex {
            real: 1.0,
            imag: 2.0
        })
    );
}

#[test]
fn executes_iter_and_next_builtins() {
    let source = "it = iter([1, 2])\n\
a = next(it)\n\
b = next(it)\n\
done = False\n\
try:\n    next(it)\n\
except StopIteration:\n    done = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("a"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("done"), Some(Value::Bool(true)));
}

#[test]
fn executes_stdlib_bootstrap_modules() {
    let source = "import math\n\
import json\n\
import codecs\n\
import re\n\
import operator\n\
import itertools\n\
import functools\n\
import collections\n\
import types\n\
import inspect\n\
import datetime\n\
import pathlib\n\
import os\n\
import time\n\
\n\
sqrt = math.sqrt(9)\n\
ceil = math.ceil(2.1)\n\
finite = math.isfinite(1.0)\n\
\n\
encoded = json.dumps({'a': 1, 'b': [2, 3]})\n\
decoded = json.loads(encoded)\n\
encoded_ascii = codecs.encode('AZ', 'ascii')\n\
decoded_ascii = codecs.decode(encoded_ascii, 'ascii')\n\
decoded_ignore = codecs.decode(bytes([65, 255, 66]), 'ascii', 'ignore')\n\
decoded_replace = codecs.decode(bytes([65, 255, 66]), 'ascii', 'replace')\n\
\n\
m1 = re.match('ab', 'abcd')\n\
m2 = re.search('bc', 'abcd')\n\
m3 = re.fullmatch('abcd', 'abcd')\n\
\n\
op = operator.add(2, 3)\n\
contains = operator.contains([1, 2, 3], 2)\n\
item = operator.getitem([9, 8], 1)\n\
\n\
chain_vals = itertools.chain([1, 2], [3])\n\
repeat_vals = itertools.repeat('x', 3)\n\
reduced = functools.reduce(operator.add, [1, 2, 3], 0)\n\
\n\
counter = collections.Counter('abca')\n\
dq = collections.deque((4, 5))\n\
mod = types.ModuleType('tmp')\n\
class Dummy:\n    pass\n\
\n\
is_mod = inspect.ismodule(mod)\n\
is_class = inspect.isclass(Dummy)\n\
is_gen = inspect.isgenerator((x for x in [1]))\n\
\n\
today = datetime.today()\n\
now = datetime.now()\n\
cwd = os.getcwd()\n\
joined = pathlib.joinpath(cwd, 'foo')\n\
pth = pathlib.Path(cwd, 'bar')\n\
t = time.time()\n\
m = time.monotonic()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");

    assert_float_global(&vm, "sqrt", 3.0);
    assert_eq!(vm.get_global("ceil"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("finite"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("op"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("contains"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("item"), Some(Value::Int(8)));
    assert_eq!(
        list_values(vm.get_global("chain_vals")),
        Some(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        list_values(vm.get_global("repeat_vals")),
        Some(vec![
            Value::Str("x".to_string()),
            Value::Str("x".to_string()),
            Value::Str("x".to_string())
        ])
    );
    assert_eq!(vm.get_global("reduced"), Some(Value::Int(6)));
    assert_eq!(vm.get_global("is_mod"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("is_class"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("is_gen"), Some(Value::Bool(true)));

    match vm.get_global("encoded") {
        Some(Value::Str(text)) => assert!(text.contains("\"a\":1")),
        other => panic!("expected encoded JSON string, got {other:?}"),
    }
    assert_eq!(
        bytes_values(vm.get_global("encoded_ascii")),
        Some(vec![65, 90])
    );
    assert_eq!(
        vm.get_global("decoded_ascii"),
        Some(Value::Str("AZ".to_string()))
    );
    assert_eq!(
        vm.get_global("decoded_ignore"),
        Some(Value::Str("AB".to_string()))
    );
    match vm.get_global("decoded_replace") {
        Some(Value::Str(text)) => {
            assert!(text.starts_with('A'));
            assert!(text.ends_with('B'));
            assert_eq!(text.chars().count(), 3);
        }
        other => panic!("expected decoded replacement string, got {other:?}"),
    }
    assert_eq!(
        tuple_values(vm.get_global("m1")),
        Some(vec![Value::Int(0), Value::Int(2)])
    );
    assert_eq!(
        tuple_values(vm.get_global("m2")),
        Some(vec![Value::Int(1), Value::Int(3)])
    );
    assert_eq!(
        tuple_values(vm.get_global("m3")),
        Some(vec![Value::Int(0), Value::Int(4)])
    );
    match vm.get_global("t") {
        Some(Value::Float(value)) => assert!(value > 0.0),
        other => panic!("expected time float, got {other:?}"),
    }
    match vm.get_global("m") {
        Some(Value::Float(value)) => assert!(value >= 0.0),
        other => panic!("expected monotonic float, got {other:?}"),
    }
    match vm.get_global("today") {
        Some(Value::Str(text)) => assert!(text.len() >= 10),
        other => panic!("expected date string, got {other:?}"),
    }
    match vm.get_global("now") {
        Some(Value::Str(text)) => assert!(text.contains('T')),
        other => panic!("expected datetime string, got {other:?}"),
    }
    match vm.get_global("joined") {
        Some(Value::Str(text)) => assert!(text.ends_with("foo")),
        other => panic!("expected joined path string, got {other:?}"),
    }
    match vm.get_global("pth") {
        Some(Value::Str(text)) => assert!(text.ends_with("bar")),
        other => panic!("expected path string, got {other:?}"),
    }

    let counter_entries = dict_entries(vm.get_global("counter")).expect("counter dict");
    assert!(counter_entries.contains(&(Value::Str("a".to_string()), Value::Int(2))));
    assert!(counter_entries.contains(&(Value::Str("b".to_string()), Value::Int(1))));
    assert_eq!(
        list_values(vm.get_global("dq")),
        Some(vec![Value::Int(4), Value::Int(5)])
    );
}

#[test]
fn executes_extended_math_functions() {
    let source = "import math\n\
ld = math.ldexp(0.5, 3)\n\
hyp = math.hypot(3, 4)\n\
fabs = math.fabs(-2.5)\n\
expv = math.exp(1.0)\n\
erfc0 = math.erfc(0.0)\n\
logv = math.log(8.0, 2.0)\n\
fsumv = math.fsum([0.1, 0.2, 0.3])\n\
sumprodv = math.sumprod([1, 2, 3], [4, 5, 6])\n\
cosv = math.cos(0.0)\n\
sinv = math.sin(0.0)\n\
tanv = math.tan(0.0)\n\
coshv = math.cosh(0.0)\n\
asinv = math.asin(1.0)\n\
atanv = math.atan(1.0)\n\
acosv = math.acos(1.0)\n\
close_ok = math.isclose(0.3000000001, 0.3, rel_tol=1e-9, abs_tol=1e-9)\n\
far_ok = not math.isclose(1.0, 1.1)\n\
ok = abs(ld - 4.0) < 1e-12 and abs(hyp - 5.0) < 1e-12 and abs(fabs - 2.5) < 1e-12 and abs(expv - 2.718281828459045) < 1e-12 and abs(erfc0 - 1.0) < 1e-6 and abs(logv - 3.0) < 1e-12 and abs(fsumv - 0.6) < 1e-12 and abs(sumprodv - 32.0) < 1e-12 and abs(cosv - 1.0) < 1e-12 and abs(sinv) < 1e-12 and abs(tanv) < 1e-12 and abs(coshv - 1.0) < 1e-12 and abs(asinv - 1.5707963267948966) < 1e-12 and abs(atanv - 0.7853981633974483) < 1e-12 and abs(acosv) < 1e-12 and close_ok and far_ok\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn raises_valueerror_for_math_domain_and_tolerance_errors() {
    let source = r#"import math
domain_sqrt = False
try:
    math.sqrt(-1)
except ValueError:
    domain_sqrt = True
domain_log = False
try:
    math.log(-1)
except ValueError:
    domain_log = True
domain_acos = False
try:
    math.acos(2)
except ValueError:
    domain_acos = True
bad_tol = False
try:
    math.isclose(1.0, 1.0, rel_tol=-1.0)
except ValueError:
    bad_tol = True
bad_lengths = False
try:
    math.sumprod([1, 2], [3])
except ValueError:
    bad_lengths = True
ok = domain_sqrt and domain_log and domain_acos and bad_tol and bad_lengths
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_io_module_helpers() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_io_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let file = temp_dir.join("sample.txt");
    let path = file.to_string_lossy().replace('\\', "\\\\");

    let source = format!(
        "import io\n\
import os\n\
io.write_text('{path}', 'hello')\n\
txt = io.read_text('{path}')\n\
raw = io.open('{path}', 'rb')\n\
names = os.listdir('{dir}')\n",
        dir = temp_dir.to_string_lossy().replace('\\', "\\\\"),
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");

    assert_eq!(vm.get_global("txt"), Some(Value::Str("hello".to_string())));
    assert_eq!(bytes_values(vm.get_global("raw")), Some(b"hello".to_vec()));
    match vm.get_global("names") {
        Some(Value::List(obj)) => match &*obj.kind() {
            Object::List(values) => {
                assert!(values.contains(&Value::Str("sample.txt".to_string())));
            }
            _ => panic!("expected list"),
        },
        other => panic!("expected names list, got {other:?}"),
    }

    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_dir(temp_dir);
}

#[test]
fn executes_divmod_builtin() {
    let source = "a = divmod(7, 3)\n\
b = divmod(-7, 3)\n\
c = divmod(7.0, 3)\n";
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
    match tuple_values(vm.get_global("c")) {
        Some(values) => {
            assert_eq!(values.len(), 2);
            match (&values[0], &values[1]) {
                (Value::Float(div), Value::Float(rem)) => {
                    assert!((*div - 2.0).abs() < 1e-12);
                    assert!((*rem - 1.0).abs() < 1e-12);
                }
                other => panic!("unexpected divmod floats: {other:?}"),
            }
        }
        other => panic!("expected tuple for c, got {other:?}"),
    }
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
    let source = "try:\n    raise ValueError('bad')\nexcept ValueError as err:\n    x = 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(1)));
    assert_eq!(
        vm.get_global("err"),
        Some(Value::Exception(ExceptionObject::new(
            "ValueError",
            Some("bad".to_string()),
        )))
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
    let source =
        "try:\n    raise ValueError('bad')\nexcept ValueError:\n    x = 1\nfinally:\n    x = 2\n";
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
fn executes_raise_from_and_exception_chaining_metadata() {
    let source = "try:\n    try:\n        raise ValueError('inner')\n    except ValueError as err:\n        raise RuntimeError('outer') from err\nexcept RuntimeError as exc:\n    cause = exc.__cause__\n    context = exc.__context__\n    suppressed = exc.__suppress_context__\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("cause"),
        Some(Value::Exception(ExceptionObject::new(
            "ValueError",
            Some("inner".to_string()),
        )))
    );
    assert_eq!(vm.get_global("context"), Some(Value::None));
    assert_eq!(vm.get_global("suppressed"), Some(Value::Bool(true)));
}

#[test]
fn executes_implicit_exception_context_metadata() {
    let source = "try:\n    try:\n        raise ValueError('inner')\n    except ValueError:\n        raise RuntimeError('outer')\nexcept RuntimeError as exc:\n    cause = exc.__cause__\n    context = exc.__context__\n    suppressed = exc.__suppress_context__\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("cause"), Some(Value::None));
    assert_eq!(
        vm.get_global("context"),
        Some(Value::Exception(ExceptionObject::new(
            "ValueError",
            Some("inner".to_string()),
        )))
    );
    assert_eq!(vm.get_global("suppressed"), Some(Value::Bool(false)));
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
fn executes_callable_instance_via_dunder_call() {
    let source = "class Callable:\n    def __call__(self, x):\n        return x + 1\nvalue = Callable()(41)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("value"), Some(Value::Int(42)));
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
fn executes_descriptor_get_set_protocol() {
    let source = "class Descriptor:\n    def __get__(self, obj, owner):\n        return obj._value + 1\n    def __set__(self, obj, value):\n        obj._value = value * 2\n\nclass Box:\n    x = Descriptor()\n    def __init__(self):\n        self._value = 0\n\nb = Box()\nb.x = 3\nout = b.x\nstate = b._value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("state"), Some(Value::Int(6)));
    assert_eq!(vm.get_global("out"), Some(Value::Int(7)));
}

#[test]
fn non_data_descriptor_respects_instance_attribute_precedence() {
    let source = "class Descriptor:\n    def __get__(self, obj, owner):\n        return 99\n\nclass Box:\n    x = Descriptor()\n\nb = Box()\nb.x = 5\nout = b.x\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("out"), Some(Value::Int(5)));
}

#[test]
fn executes_super_and_mro_lookup() {
    let source = "class A:\n    def who(self):\n        return 'A'\n\nclass B(A):\n    def who(self):\n        return 'B'\n\nclass C(A):\n    def who(self):\n        return 'C'\n\nclass D(B, C):\n    def who(self):\n        return super(D, self).who()\n\nd = D()\nout = d.who()\nm1 = D.__mro__[1].__name__\nm2 = D.__mro__[2].__name__\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("out"), Some(Value::Str("B".to_string())));
    assert_eq!(vm.get_global("m1"), Some(Value::Str("B".to_string())));
    assert_eq!(vm.get_global("m2"), Some(Value::Str("C".to_string())));
}

#[test]
fn executes_slots_restrictions() {
    let source = "class Box:\n    __slots__ = ('x',)\n\nb = Box()\nb.x = 3\nok = b.x\nfailed = False\ntry:\n    b.y = 4\nexcept AttributeError:\n    failed = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("failed"), Some(Value::Bool(true)));
}

#[test]
fn allows_assignments_for_subclasses_of_empty_slots_base() {
    let source = "class Base:\n    __slots__ = ()\n\nclass Child(Base):\n    pass\n\nchild = Child()\nchild.gen = 42\nok = child.gen == 42\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_class_metaclass_keyword_argument() {
    let source = "def meta(name, bases, namespace):\n    namespace['flag'] = 7\n    return type(name, bases, namespace)\n\nclass Sample(metaclass=meta):\n    pass\n\nclass SlotBox(metaclass=type):\n    __slots__ = ('x',)\n\ns = Sample()\nout = Sample.flag\nslot_ok = False\nslot_fail = False\nbox = SlotBox()\nbox.x = 3\nslot_ok = True\ntry:\n    box.y = 4\nexcept AttributeError:\n    slot_fail = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("out"), Some(Value::Int(7)));
    assert_eq!(vm.get_global("slot_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("slot_fail"), Some(Value::Bool(true)));
}

#[test]
fn class_objects_are_instances_of_declared_metaclass() {
    let source = "class Meta(type):\n    pass\nclass Base(metaclass=Meta):\n    pass\nclass Child(Base):\n    pass\nbase_ok = isinstance(Base, Meta)\nchild_ok = isinstance(Child, Meta)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("base_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("child_ok"), Some(Value::Bool(true)));
}

#[test]
fn class_attribute_falls_back_to_metaclass_method() {
    let source = "class Meta(type):\n    def tag(cls):\n        return cls.__name__\nclass Sample(metaclass=Meta):\n    pass\nok = Sample.tag() == 'Sample'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn class_metaclass_conflict_raises_type_error() {
    let source = "class M1(type):\n    pass\nclass M2(type):\n    pass\nclass A(metaclass=M1):\n    pass\nclass B(metaclass=M2):\n    pass\nok = False\ntry:\n    class C(A, B):\n        pass\nexcept TypeError:\n    ok = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn empty_slots_block_dynamic_attributes() {
    let source = "class Empty:\n    __slots__ = ()\nobj = Empty()\nok = False\ntry:\n    obj.value = 1\nexcept AttributeError:\n    ok = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn dict_slot_allows_dynamic_attributes() {
    let source = "class Dynamic:\n    __slots__ = ('__dict__',)\nobj = Dynamic()\nobj.value = 12\nok = obj.value == 12\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_type_three_arg_class_creation() {
    let source = "C = type('C', (), {'x': 7})\nc = C()\nv = c.x\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("v"), Some(Value::Int(7)));
}

#[test]
fn executes_attribute_builtins() {
    let source = "class A:\n    def f(self):\n        return 10\n\nclass B(A):\n    def f(self):\n        return super(B, self).f() + 1\n\nb = B()\ng = getattr(b, 'f')\nout = g()\nsetattr(b, 'x', 7)\nhx = hasattr(b, 'x')\ndelattr(b, 'x')\nhy = hasattr(b, 'x')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("out"), Some(Value::Int(11)));
    assert_eq!(vm.get_global("hx"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("hy"), Some(Value::Bool(false)));
}

#[test]
fn executes_custom_getattribute_with_object_fallback() {
    let source = "class A:\n    def __init__(self):\n        self.y = 5\n    def __getattribute__(self, name):\n        if name == 'x':\n            return 42\n        return object.__getattribute__(self, name)\n\na = A()\nx = a.x\ny = a.y\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("x"), Some(Value::Int(42)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(5)));
}

#[test]
fn object_getattribute_bypasses_getattr_fallback() {
    let source = "class A:\n    def __getattr__(self, name):\n        return 99\n\na = A()\nvia_getattr = a.missing\ncaught = False\ntry:\n    object.__getattribute__(a, 'missing')\nexcept AttributeError:\n    caught = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("via_getattr"), Some(Value::Int(99)));
    assert_eq!(vm.get_global("caught"), Some(Value::Bool(true)));
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
            module_data.globals.insert("x".to_string(), Value::Int(42));
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
    assert_eq!(
        vm.get_global("pkg_name"),
        Some(Value::Str("pkg".to_string()))
    );
    assert_eq!(
        vm.get_global("pkg_package"),
        Some(Value::Str("pkg".to_string()))
    );
    assert_eq!(
        vm.get_global("pkg_spec_name"),
        Some(Value::Str("pkg".to_string()))
    );
    assert_eq!(vm.get_global("pkg_path_len"), Some(Value::Int(1)));
    assert_eq!(
        vm.get_global("sub_package"),
        Some(Value::Str("pkg".to_string()))
    );
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
fn imports_sys_module_and_exposes_modules_dict() {
    let source = "\
import sys\n\
main_name = sys.modules['__main__'].__name__\n\
sys_name = sys.modules['sys'].__name__\n\
path_len = len(sys.path)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("main_name"),
        Some(Value::Str("__main__".to_string()))
    );
    assert_eq!(
        vm.get_global("sys_name"),
        Some(Value::Str("sys".to_string()))
    );
    assert_eq!(vm.get_global("path_len"), Some(Value::Int(1)));
}

#[test]
fn imports_random_module_and_is_seed_deterministic() {
    let source = "\
import random\n\
random.seed(123)\n\
a1 = random.random()\n\
b1 = random.randrange(100)\n\
c1 = random.randint(3, 7)\n\
d1 = random.getrandbits(12)\n\
x = [1, 2, 3, 4]\n\
random.shuffle(x)\n\
e1 = random.choice(x)\n\
random.seed(123)\n\
a2 = random.random()\n\
b2 = random.randrange(100)\n\
c2 = random.randint(3, 7)\n\
d2 = random.getrandbits(12)\n\
y = [1, 2, 3, 4]\n\
random.shuffle(y)\n\
e2 = random.choice(y)\n\
same = (a1 == a2) and (b1 == b2) and (c1 == c2) and (d1 == d2) and (x == y) and (e1 == e2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("same"), Some(Value::Bool(true)));
}

#[test]
fn imports_with_meta_path_finder_object_entry() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_meta_obj_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 97\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import sys\nsys.path = ['{path_literal}']\nsys.meta_path = [{{'kind': 'pyrs.PathFinder'}}]\nimport mod\nx = mod.value\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(97)));

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn imports_with_path_hook_object_entry() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_path_hook_obj_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 101\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import sys\nsys.path = ['{path_literal}']\nsys.path_hooks = [{{'kind': 'pyrs.FileFinder'}}]\nimport mod\nx = mod.value\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(101)));

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn disables_path_imports_when_meta_path_excludes_default_finder() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_meta_path_disabled_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 71\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source =
        format!("import sys\nsys.path = ['{path_literal}']\nsys.meta_path = []\nimport mod\n");
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("module 'mod' not found"));

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn re_enables_path_imports_with_default_meta_path_finder() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_meta_path_enabled_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 73\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import sys\nsys.path = ['{path_literal}']\nsys.meta_path = ['pyrs.PathFinder']\nimport mod\nx = mod.value\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(73)));

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn disables_path_imports_when_path_hooks_are_empty() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_path_hooks_empty_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 79\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source =
        format!("import sys\nsys.path = ['{path_literal}']\nsys.path_hooks = []\nimport mod\n");
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("module 'mod' not found"));

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn populates_path_importer_cache_for_loaded_path_entry() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_path_cache_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 83\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import sys\nsys.path = ['{path_literal}']\nimport mod\ncached = '{path_literal}' in sys.path_importer_cache\nkind = sys.path_importer_cache['{path_literal}']['kind']\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("cached"), Some(Value::Bool(true)));
    assert_eq!(
        vm.get_global("kind"),
        Some(Value::Str("pyrs.FileFinder".to_string()))
    );

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn reuses_cached_importer_when_path_hooks_are_cleared() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_path_cache_reuse_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod_a.py"), "value = 2\n").expect("write module a");
    std::fs::write(temp_dir.join("mod_b.py"), "value = 9\n").expect("write module b");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import sys\nsys.path = ['{path_literal}']\nimport mod_a\nsys.path_hooks = []\nimport mod_b\ntotal = mod_a.value + mod_b.value\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("total"), Some(Value::Int(11)));

    let _ = std::fs::remove_file(temp_dir.join("mod_a.py"));
    let _ = std::fs::remove_file(temp_dir.join("mod_b.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn imports_using_importlib_module_helpers() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_importlib_helpers_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 107\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import sys\nimport importlib\nimport importlib.util\nsys.path = ['{path_literal}']\nspec = importlib.find_spec('mod')\nname = spec['name']\nloader = spec['loader']\nm = importlib.import_module('mod')\nu_spec = importlib.util.find_spec('mod')\nu_name = u_spec['name']\nx = m.value\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("name"), Some(Value::Str("mod".to_string())));
    assert_eq!(
        vm.get_global("loader"),
        Some(Value::Str("pyrs.SourceFileLoader".to_string()))
    );
    assert_eq!(vm.get_global("u_name"), Some(Value::Str("mod".to_string())));
    assert_eq!(vm.get_global("x"), Some(Value::Int(107)));

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn exposes_importlib_cache_path_helpers() {
    let source = "import importlib.util\nsrc = importlib.util.source_from_cache('/tmp/__pycache__/demo.cpython-314.pyc')\ncache = importlib.util.cache_from_source('/tmp/demo.py')\nok = src == '/tmp/demo.py' and ('__pycache__' in cache) and (cache[-4:] == '.pyc')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_importlib_invalidate_caches_and_spec_from_file_location() {
    let source = "import sys\nimport importlib\nimport importlib.util\nsys.path_importer_cache['/tmp/demo'] = 42\nbefore = '/tmp/demo' in sys.path_importer_cache\nimportlib.invalidate_caches()\nafter = '/tmp/demo' in sys.path_importer_cache\nspec = importlib.util.spec_from_file_location('demo', '/tmp/demo.py')\nok = before and (not after) and spec['name'] == 'demo' and spec['origin'] == '/tmp/demo.py' and spec['loader'] == 'pyrs.SourceFileLoader' and spec['has_location'] and spec['cached'][-4:] == '.pyc'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn imports_using_importlib_relative_package_resolution() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_importlib_relative_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");
    std::fs::write(pkg_dir.join("sub.py"), "value = 109\n").expect("write sub module");

    let source = "\
import importlib\n\
m = importlib.import_module('.sub', package='pkg')\n\
x = m.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(109)));

    let _ = std::fs::remove_file(pkg_dir.join("sub.py"));
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&pkg_dir);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn uses_sys_path_mutation_for_module_lookup() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_sys_path_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    std::fs::write(temp_dir.join("mod.py"), "value = 37\n").expect("write module");
    let path_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!("import sys\nsys.path = ['{path_literal}']\nimport mod\nx = mod.value\n");
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(37)));

    let _ = std::fs::remove_file(temp_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_dunder_import_top_level_and_fromlist() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_dunder_import_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");
    std::fs::write(pkg_dir.join("__init__.py"), "flag = 1\n").expect("write package init");
    std::fs::write(pkg_dir.join("sub.py"), "value = 41\n").expect("write sub module");

    let source = "\
root = __import__('pkg.sub')\n\
top_name = root.__name__\n\
leaf = __import__('pkg.sub', fromlist=['sub'])\n\
leaf_name = leaf.__name__\n\
x = leaf.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("top_name"),
        Some(Value::Str("pkg".to_string()))
    );
    assert_eq!(
        vm.get_global("leaf_name"),
        Some(Value::Str("pkg.sub".to_string()))
    );
    assert_eq!(vm.get_global("x"), Some(Value::Int(41)));

    let _ = std::fs::remove_file(pkg_dir.join("sub.py"));
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&pkg_dir);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_dunder_import_relative_level_inside_package() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_dunder_relative_{unique}"));
    let pkg_dir = temp_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp dir");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");
    std::fs::write(pkg_dir.join("sub.py"), "value = 43\n").expect("write sub module");
    std::fs::write(
        pkg_dir.join("mod.py"),
        "m = __import__('sub', globals(), locals(), ['value'], 1)\nresult = m.value\n",
    )
    .expect("write package module");

    let source = "import pkg.mod\nx = pkg.mod.result\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(43)));

    let _ = std::fs::remove_file(pkg_dir.join("mod.py"));
    let _ = std::fs::remove_file(pkg_dir.join("sub.py"));
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&pkg_dir);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_namespace_package_import() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_namespace_pkg_{unique}"));
    let ns_dir = temp_dir.join("ns");
    std::fs::create_dir_all(&ns_dir).expect("create temp dir");
    std::fs::write(ns_dir.join("mod.py"), "value = 53\n").expect("write sub module");

    let source = "\
import ns.mod\n\
x = ns.mod.value\n\
path_len = len(ns.__path__)\n\
is_pkg = ns.__spec__['is_package']\n\
is_namespace = ns.__spec__['is_namespace']\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(53)));
    assert_eq!(vm.get_global("path_len"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("is_pkg"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("is_namespace"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(ns_dir.join("mod.py"));
    let _ = std::fs::remove_dir(&ns_dir);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn aggregates_namespace_package_paths_across_module_roots() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let root_a = std::env::temp_dir().join(format!("pyrs_ns_root_a_{unique}"));
    let root_b = std::env::temp_dir().join(format!("pyrs_ns_root_b_{unique}"));
    let ns_a = root_a.join("ns");
    let ns_b = root_b.join("ns");
    std::fs::create_dir_all(&ns_a).expect("create root a");
    std::fs::create_dir_all(&ns_b).expect("create root b");
    std::fs::write(ns_a.join("a.py"), "value = 2\n").expect("write module a");
    std::fs::write(ns_b.join("b.py"), "value = 5\n").expect("write module b");

    let source = "\
import ns.a\n\
import ns.b\n\
total = ns.a.value + ns.b.value\n\
path_len = len(ns.__path__)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&root_a);
    vm.add_module_path(&root_b);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("total"), Some(Value::Int(7)));
    assert_eq!(vm.get_global("path_len"), Some(Value::Int(2)));

    let _ = std::fs::remove_file(ns_a.join("a.py"));
    let _ = std::fs::remove_file(ns_b.join("b.py"));
    let _ = std::fs::remove_dir(&ns_a);
    let _ = std::fs::remove_dir(&ns_b);
    let _ = std::fs::remove_dir(&root_a);
    let _ = std::fs::remove_dir(&root_b);
}

#[test]
fn resolves_submodule_using_package_dunder_path() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let root_dir = std::env::temp_dir().join(format!("pyrs_pkg_path_root_{unique}"));
    let pkg_dir = root_dir.join("pkg");
    let external_dir = std::env::temp_dir().join(format!("pyrs_pkg_path_external_{unique}"));
    std::fs::create_dir_all(&pkg_dir).expect("create pkg dir");
    std::fs::create_dir_all(&external_dir).expect("create external dir");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");
    std::fs::write(external_dir.join("sub.py"), "value = 59\n").expect("write external submodule");

    let path_literal = external_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import pkg\npkg.__path__ = ['{path_literal}']\nimport pkg.sub\nx = pkg.sub.value\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&root_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(59)));

    let _ = std::fs::remove_file(external_dir.join("sub.py"));
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(&external_dir);
    let _ = std::fs::remove_dir(&pkg_dir);
    let _ = std::fs::remove_dir(&root_dir);
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
fn executes_string_percent_formatting() {
    let source = "a = '(%s+)' % 'x'\nb = '%(name)s' % {'name': 'ok'}\nc = '%d' % 7\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Str("(x+)".to_string())));
    assert_eq!(vm.get_global("b"), Some(Value::Str("ok".to_string())));
    assert_eq!(vm.get_global("c"), Some(Value::Str("7".to_string())));
}

#[test]
fn executes_re_escape_builtin() {
    let source = "import re\nx = re.escape('\\t a+')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(
        vm.get_global("x"),
        Some(Value::Str("\\\t\\ a\\+".to_string()))
    );
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

#[test]
fn executes_decorated_function_definition() {
    let source = "def deco(fn):\n    def wrap(x):\n        return fn(x) + 1\n    return wrap\n@deco\ndef ident(x):\n    return x\ny = ident(4)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("y"), Some(Value::Int(5)));
}

#[test]
fn executes_assignment_expression() {
    let source = "x = 0\nif (x := 3):\n    y = x\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(3)));
}

#[test]
fn executes_list_comprehension_with_scope_isolation() {
    let source = "x = 10\nvals = [x * 2 for x in [1, 2, 3] if x > 1]\nout = x\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(
        list_values(vm.get_global("vals")),
        Some(vec![Value::Int(4), Value::Int(6)])
    );
    assert_eq!(vm.get_global("out"), Some(Value::Int(10)));
}

#[test]
fn executes_dict_comprehension() {
    let source = "d = {x: x + 1 for x in [1, 2, 3]}\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(
        dict_entries(vm.get_global("d")),
        Some(vec![
            (Value::Int(1), Value::Int(2)),
            (Value::Int(2), Value::Int(3)),
            (Value::Int(3), Value::Int(4)),
        ])
    );
}

#[test]
fn executes_generator_expression() {
    let source = "g = (x * 3 for x in [1, 2, 3])\nvals = []\nfor item in g:\n    vals += [item]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(
        list_values(vm.get_global("vals")),
        Some(vec![Value::Int(3), Value::Int(6), Value::Int(9)])
    );
}

#[test]
fn executes_match_case_statement() {
    let source = "value = 2\nmatch value:\n    case 1:\n        out = 'one'\n    case x if x > 1:\n        out = 'many'\n    case _:\n        out = 'other'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("out"), Some(Value::Str("many".to_string())));
    assert_eq!(vm.get_global("x"), Some(Value::Int(2)));
}

#[test]
fn executes_asyncio_run_with_await() {
    let source = "import asyncio\nasync def inner(x):\n    return x + 1\nasync def outer(x):\n    y = await inner(x)\n    return y * 2\nresult = asyncio.run(outer(10))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("result"), Some(Value::Int(22)));
}

#[test]
fn executes_async_for_and_async_with() {
    let source = "import asyncio\nclass AsyncIter:\n    def __init__(self, n):\n        self.n = n\n        self.i = 0\n    def __aiter__(self):\n        return self\n    async def __anext__(self):\n        if self.i >= self.n:\n            raise StopAsyncIteration\n        self.i += 1\n        return self.i\nclass AsyncCtx:\n    async def __aenter__(self):\n        return 5\n    async def __aexit__(self, a, b, c):\n        return False\nasync def run_all():\n    total = 0\n    async for item in AsyncIter(3):\n        total += item\n    async with AsyncCtx() as value:\n        total += value\n    return total\nresult = asyncio.run(run_all())\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("result"), Some(Value::Int(11)));
}

#[test]
fn executes_anext_for_async_generator() {
    let source = "import asyncio\nasync def agen():\n    yield 3\n    yield 4\ng = agen()\na = asyncio.run(anext(g))\nb = asyncio.run(anext(g))\ndone = False\ntry:\n    asyncio.run(anext(g))\nexcept StopAsyncIteration:\n    done = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("a"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(4)));
    assert_eq!(vm.get_global("done"), Some(Value::Bool(true)));
}

#[test]
fn executes_inspect_async_predicates() {
    let source = "import asyncio\nimport inspect\nasync def coro():\n    return 1\nasync def agen():\n    yield 1\nc = coro()\ng = agen()\nis_coro = inspect.iscoroutine(c)\nis_awaitable = inspect.isawaitable(c)\nis_gen = inspect.isgenerator(c)\nis_async_gen = inspect.isasyncgen(g)\nresult = asyncio.run(c)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("is_coro"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("is_awaitable"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("is_gen"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("is_async_gen"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("result"), Some(Value::Int(1)));
}

#[test]
fn executes_threading_and_signal_foundations() {
    let source = "import signal\nimport threading\nseen = 0\ndef handler(signum, frame):\n    global seen\n    seen = signum\nold = signal.signal(signal.SIGINT, handler)\nsignal.raise_signal(signal.SIGINT)\nident = threading.get_ident()\ncount = threading.active_count()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("seen"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("old"), Some(Value::Int(0)));
    assert_eq!(vm.get_global("count"), Some(Value::Int(1)));
    match vm.get_global("ident") {
        Some(Value::Int(value)) => assert!(value > 0),
        other => panic!("expected integer thread id, got {other:?}"),
    }
}

#[test]
fn executes_except_star_as_except() {
    let source =
        "caught = False\ntry:\n    raise ValueError\nexcept* ValueError:\n    caught = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("caught"), Some(Value::Bool(true)));
}

#[test]
fn executes_fstring_lowering() {
    let source = "name = 'Ada'\nout = f\"hello {name} {1 + 2}\"\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(
        vm.get_global("out"),
        Some(Value::Str("hello Ada 3".to_string()))
    );
}

#[test]
fn rejects_misplaced_future_import() {
    let source = "x = 1\nfrom __future__ import annotations\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let err = compiler::compile_module(&module).expect_err("compile should fail");
    assert!(
        err.message
            .contains("from __future__ imports must occur at the beginning"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rejects_unknown_future_feature() {
    let source = "from __future__ import totally_not_real\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let err = compiler::compile_module(&module).expect_err("compile should fail");
    assert!(
        err.message.contains("future feature"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn executes_property_descriptor_getter() {
    let source = "class C:\n    @property\n    def value(self):\n        return 42\nc = C()\nout = c.value\nclass_attr = C.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("out"), Some(Value::Int(42)));
    match vm.get_global("class_attr") {
        Some(Value::Instance(_)) => {}
        other => panic!("expected property descriptor instance, got {other:?}"),
    }
}

#[test]
fn executes_property_setter_decorator() {
    let source = "class C:\n    def __init__(self):\n        self._value = 0\n    @property\n    def value(self):\n        return self._value\n    @value.setter\n    def value(self, new_value):\n        self._value = new_value + 1\nc = C()\nc.value = 41\nout = c.value\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("out"), Some(Value::Int(42)));
}

#[test]
fn exposes_builtin_type_dict() {
    let source =
        "mappingproxy = type(type.__dict__)\nok = isinstance(type.__dict__, mappingproxy)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_py_warnings_filterwarnings_path() {
    let source = "class FilterHolder:\n    @property\n    def filters(self):\n        return [('ignore', None, Warning, None, 0)]\nholder = FilterHolder()\nitem = ('ignore', None, Warning, None, 0)\nok = item in holder.filters\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_list_attribute_methods() {
    let source = "vals = [2]\nvals.append(3)\nvals.insert(0, 1)\nvals.remove(2)\nout = vals\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(
        list_values(vm.get_global("out")),
        Some(vec![Value::Int(1), Value::Int(3)])
    );
}

#[test]
fn exposes_sys_getframe_locals() {
    let source = "import sys\ndef f():\n    local_value = 1\n    frame = sys._getframe()\n    return isinstance(frame.f_locals, dict)\nok = f()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_class_register_fallback() {
    let source = "class C:\n    pass\nresult = C.register(int)\nok = result == int\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_frozenset_contains_method() {
    let source = "checker = frozenset(['if', 'for']).__contains__\nok = checker('if') and not checker('while')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_custom_metaclass_fallback() {
    let source = "class Meta(type):\n    pass\nclass C(metaclass=Meta):\n    pass\nok = isinstance(C, type)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn imports_gc_errno_weakref_and_array_modules() {
    let source = "import gc\nimport errno\nimport weakref\nimport _weakref\nimport array\nvals = array.array('B', b'AB')\nref_value = weakref.ref(1)\nok = gc.isenabled() and errno.ENOENT == 2 and len(vals) == 2 and ref_value == 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn sys_module_exposes_argv_and_executable() {
    let source =
        "import sys\nok = isinstance(sys.argv, list) and isinstance(sys.executable, str)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn time_module_localtime_and_strftime_work() {
    let source = "import time\nt = time.localtime(0)\ns = time.strftime('%Y-%m-%d %H:%M:%S', t)\nok = (t[0], t[1], t[2]) == (1970, 1, 1) and s == '1970-01-01 00:00:00'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn itertools_count_iterates() {
    let source = "import itertools\nit = itertools.count(3, 2)\na = next(it)\nb = next(it)\nok = a == 3 and b == 5\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn functools_singledispatch_exposes_register_attribute() {
    let source = "import functools\n@functools.singledispatch\ndef f(x):\n    return x\nreg = f.register\n@f.register(int)\ndef g(x):\n    return x + 1\nok = callable(reg) and g(2) == 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_nonlocal_bindings_with_block_local_assignments() {
    let source = "def outer_try():\n    def inner():\n        nonlocal x\n        x = 2\n    try:\n        x = 1\n    except Exception:\n        pass\n    inner()\n    return x\ndef outer_for():\n    def inner():\n        nonlocal y\n        y = 3\n    for _ in [0]:\n        y = 1\n    inner()\n    return y\na = outer_try()\nb = outer_for()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("a"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("b"), Some(Value::Int(3)));
}

#[test]
fn exposes_functools_wraps_decorator_callable() {
    let source = "import functools\ndef deco(fn):\n    @functools.wraps(fn)\n    def inner(x):\n        return fn(x)\n    return inner\n@deco\ndef add1(x):\n    return x + 1\nout = add1(4)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("out"), Some(Value::Int(5)));
}

#[test]
fn exposes_functools_total_ordering_decorator() {
    let source =
        "import functools\n@functools.total_ordering\nclass C:\n    pass\nok = C is not None\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_object_new_lookuperror_and_open_builtin() {
    let source = "obj = object.__new__(object)\nstate = object.__getstate__(obj)\nerr = False\ntry:\n    raise LookupError\nexcept LookupError:\n    err = True\nself_ok = int.__new__.__self__ is int\nopen_ok = callable(open)\nok = (state is None) and err and self_ok and open_ok\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_io_textiowrapper_and_sys_platform() {
    let source = "import io\nimport sys\nok = hasattr(io, 'TextIOWrapper') and isinstance(sys.platform, str)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_platform_libc_ver_tuple() {
    let source = "import platform\ninfo = platform.libc_ver()\nok = isinstance(info, tuple) and len(info) == 2 and isinstance(info[0], str) and isinstance(info[1], str)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_platform_win32_is_iot_bool() {
    let source = "import platform\nok = isinstance(platform.win32_is_iot(), bool)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_os_fsencode_fsdecode_and_unicodeerror() {
    let source = "import os\npayload = os.fsencode('abc')\ntext = os.fsdecode(payload)\ncaught = False\ntry:\n    raise UnicodeError\nexcept UnicodeError:\n    caught = True\nok = isinstance(payload, bytes) and text == 'abc' and caught\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_os_fd_stat_and_wait_status_helpers() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_os_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let file = temp_dir.join("sample.txt");
    std::fs::write(&file, b"hello").expect("write sample file");

    let source = format!(
        "import os\nimport posix\n\
path = '{path}'\n\
root = '{root}'\n\
fd = os.open(path, os.O_RDONLY)\n\
is_tty = os.isatty(fd)\n\
os.close(fd)\n\
st = os.stat(path)\n\
lst = os.lstat(path)\n\
pst = posix.stat(path)\n\
items = os.scandir(root)\n\
name_ok = any(item[0] == 'sample.txt' for item in items)\n\
wait_ok = os.WIFEXITED(5 << 8) and os.WEXITSTATUS(5 << 8) == 5 and not os.WIFSIGNALED(5 << 8)\n\
ok = (not is_tty) and st.st_size == 5 and lst.st_size == 5 and pst.st_size == 5 and name_ok and wait_ok\n",
        path = file.to_string_lossy().replace('\\', "\\\\"),
        root = temp_dir.to_string_lossy().replace('\\', "\\\\"),
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_dir(temp_dir);
}

#[test]
fn executes_os_utime_rmdir_and_bad_fd_error_type() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_os_mut_{unique}"));
    let empty_dir = temp_dir.join("empty");
    std::fs::create_dir_all(&empty_dir).expect("create temp dir");
    let file = temp_dir.join("touch.txt");
    std::fs::write(&file, b"x").expect("write sample file");

    let source = format!(
        r#"import os
path = '{path}'
empty = '{empty}'
os.utime(path, (1, 2))
st = os.stat(path)
os.rmdir(empty)
caught = False
try:
    os.close(999999)
except OSError:
    caught = True
ok = st.st_mtime >= 2 and caught
"#,
        path = file.to_string_lossy().replace('\\', "\\\\"),
        empty = empty_dir.to_string_lossy().replace('\\', "\\\\"),
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_dir(temp_dir);
}

#[test]
fn handles_except_tuple_types() {
    let source = "caught = False\ntry:\n    raise AttributeError\nexcept (ImportError, AttributeError):\n    caught = True\nok = caught\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_str_encode_decode_and_unicode_subclass_handling() {
    let source = "text = 'abc'\nencoded = text.encode('utf-8')\ndecoded = text.decode('utf-8')\ncaught = False\ntry:\n    raise UnicodeDecodeError\nexcept UnicodeError:\n    caught = True\nok = isinstance(encoded, bytes) and decoded == text and caught\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_itertools_batched_builtin() {
    let source = "import itertools\nchunks = itertools.batched([1, 2, 3, 4, 5], 2)\nok = chunks == [(1, 2), (3, 4), (5,)]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_collections_defaultdict_factory_on_getitem() {
    let source = "from collections import defaultdict\nd = defaultdict(list)\nd['k'].append(1)\nok = d['k'] == [1]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_collections_count_elements_builtin() {
    let source = "import collections\ncounts = {}\ncollections._count_elements(counts, ['a', 'b', 'a'])\nok = counts['a'] == 2 and counts['b'] == 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_atexit_register_unregister_run_and_clear() {
    let source = "import atexit\nout = []\ndef f(x):\n    out.append(x)\ndef g(x):\n    out.append(x)\natexit.register(f, 1)\natexit.register(g, 2)\natexit.unregister(f)\natexit.register(f, 3)\natexit._run_exitfuncs()\natexit._clear()\nok = out == [3, 2]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_decimal_context_helpers() {
    let source = "import decimal\nctx = decimal.getcontext()\ndecimal.setcontext(ctx)\nctx2 = decimal.localcontext()\nok = (ctx is ctx2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_thread_start_new_thread_baseline() {
    let source = "import _thread\nout = []\ndef fn(x, y=0):\n    out.append(x + y)\ntid = _thread.start_new_thread(fn, (2,), {'y': 3})\nok = isinstance(tid, int) and out == [5]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_threading_class_methods_baseline() {
    let source = "import threading\nout = []\ndef worker(x):\n    out.append(x)\nt = threading.Thread(target=worker, args=(7,))\na = t.is_alive()\nt.start()\nt.join()\nb = t.is_alive()\nok = (not a) and (not b) and out == [7]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_threading_sync_primitives_baseline() {
    let source = "import threading\ne = threading.Event()\na = e.is_set()\ne.set()\nb = e.wait(0.01)\nc = e.is_set()\ne.clear()\nd = (not e.is_set())\ns = threading.Semaphore(1)\nx = s.acquire()\ny = (not s.acquire(False))\ns.release()\nz = s.acquire(False)\nbarrier = threading.Barrier(2)\np = barrier.wait()\nq = barrier.wait()\nok = (not a) and b and c and d and x and y and z and isinstance(p, int) and isinstance(q, int)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_socket_object_methods_baseline() {
    let source = "import _socket\ns = _socket.socket()\nfd0 = s.fileno()\nfd1 = s.detach()\nfd2 = s.fileno()\ns.close()\nok = isinstance(fd0, int) and fd1 == fd0 and fd2 == -1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_uuid_and_reduce_ex_baseline() {
    let source = "import uuid\nu = uuid.uuid4()\nv = uuid.UUID('6ba7b810-9dad-11d1-80b4-00c04fd430c8')\nu3 = uuid.uuid3(uuid.NAMESPACE_DNS, 'example.com')\nnode = uuid.getnode()\nred = object.__reduce_ex__(object(), 4)\nok = isinstance(u, uuid.UUID) and u.version == 4 and isinstance(v.hex, str) and isinstance(u3, uuid.UUID) and isinstance(node, int) and isinstance(red, tuple) and len(red) == 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_warnings_lock_helpers() {
    let source = "import _warnings\n_warnings._acquire_lock()\n_warnings._release_lock()\nok = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_types_markers_and_functools_partial() {
    let source = "import functools\nimport types\ndef add(a, b, c=0):\n    return a + b + c\npart = functools.partial(add, 1, c=3)\nok = part(2) == 6 and hasattr(types, 'BuiltinFunctionType') and hasattr(types, 'EllipsisType')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_types_coroutine_decorator() {
    let source = "import types\ndef f():\n    return 1\ng = types.coroutine(f)\nok = (g is f) and callable(g)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_unicodedata_normalize_stub() {
    let source = "import unicodedata\nname = 'Cafe\\u0301'\nout = unicodedata.normalize('NFD', name)\nok = out == name\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_binascii_crc32() {
    let source = "import binascii\ncrc = binascii.crc32(b'hello')\ncrc2 = binascii.crc32(b'world', crc)\nok = (crc == 907060870) and (crc2 == 4192936109)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_string_startswith_and_replace_methods() {
    let source = "name = 'token'\na = name.startswith('to')\nb = name.replace('to', 'bro')\nok = a and b == 'broken'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_string_remove_prefix_and_suffix_methods() {
    let source = "name = 'test_case.py'\na = name.removeprefix('test_')\nb = name.removesuffix('.py')\nc = name.removeprefix('prod_')\nd = name.removesuffix('.txt')\nok = (a == 'case.py' and b == 'test_case' and c == name and d == name)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_set_and_frozenset_union_operator() {
    let source = "a = {1, 2} | {2, 3}\nb = frozenset({1, 2}) | {2, 3}\nc = {1, 2} | frozenset({2, 4})\nok = len(a) == 3 and (3 in a) and len(b) == 3 and (3 in b) and len(c) == 3 and (4 in c)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_sorted_on_tuple_items() {
    let source =
        "items = {'b': 2, 'a': 1}.items()\nout = sorted(items)\nok = out == [('a', 1), ('b', 2)]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_large_power_without_overflow_error() {
    let source = "value = 2 ** 63\nok = value > 0\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_list_extend_dict_update_and_function_dict_paths() {
    let source = "def f():\n    return None\nf.marker = 1\ndef g():\n    return None\ng.__dict__.update(f.__dict__)\nvals = [1]\nvals.extend((2, 3))\nd = {}\nitem = d.setdefault('k', [])\nitem.append(4)\nd.update({'a': 1}, b=2)\nok = vals == [1, 2, 3] and d['k'] == [4] and d['a'] == 1 and d['b'] == 2 and g.marker == 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_types_codetype_and_arithmeticerror() {
    let source = "import types\nok = hasattr(types, 'CodeType') and issubclass(ArithmeticError, Exception)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn unpacks_non_sequence_iterables() {
    let source = "a, b = iter([1, 2])\nok = (a, b) == (1, 2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_function_doc_attribute() {
    let source = "def f():\n    pass\nok = f.__doc__ is None\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn raises_exception_instances_and_classes() {
    let source = "class MyError(Exception):\n    pass\nfrom_instance = False\ntry:\n    raise MyError('boom')\nexcept Exception:\n    from_instance = True\nfrom_class = False\ntry:\n    raise MyError\nexcept Exception:\n    from_class = True\nok = from_instance and from_class\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn catches_user_defined_exception_classes_in_except_handlers() {
    let source = "class BaseErr(Exception):\n    pass\nclass ChildErr(BaseErr):\n    pass\nexact = False\ntry:\n    raise ChildErr('boom')\nexcept ChildErr:\n    exact = True\nbase = False\ntry:\n    raise ChildErr('boom')\nexcept BaseErr:\n    base = True\ntuple_ok = False\ntry:\n    raise ChildErr('boom')\nexcept (ValueError, BaseErr):\n    tuple_ok = True\nok = exact and base and tuple_ok\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_os_seek_constants() {
    let source = "import os\nok = os.SEEK_SET == 0 and os.SEEK_CUR == 1 and os.SEEK_END == 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_os_platform_separator_constants() {
    let source = "import os\nok = hasattr(os, 'altsep') and os.curdir == '.' and os.pardir == '..' and os.extsep == '.'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn imports_testsinglephase_stub_module() {
    let source = "import _testsinglephase\nok = _testsinglephase is not None\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_sys_builtin_module_names_and_importlib_spec_helper() {
    let source = "import sys\nfrom importlib.util import spec_from_file_location\nok = isinstance(sys.builtin_module_names, tuple) and callable(spec_from_file_location)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_utf16_codec_paths() {
    let source = "import codecs\ntext = 'Hi'\nencoded = text.encode('utf-16-le')\ndecoded = codecs.decode(encoded, 'utf-16-le')\nok = isinstance(encoded, bytes) and decoded == text and len(encoded) == 4\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_utf32_codec_paths() {
    let source = "import codecs\ntext = 'Hi'\nencoded = text.encode('utf-32-le')\ndecoded = codecs.decode(encoded, 'utf-32-le')\nroundtrip = text.encode('utf-32').decode('utf-32')\nok = isinstance(encoded, bytes) and decoded == text and roundtrip == text and len(encoded) == 8\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_and_tuple_consume_iterators_and_generators() {
    let source = "items = tuple(x for x in [1, 2, 3])\nvals = list(iter((4, 5)))\nok = items == (1, 2, 3) and vals == [4, 5]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_dict_pop_with_default_and_missing_key_error() {
    let source = "d = {'a': 1}\na = d.pop('a')\nb = d.pop('b', 7)\ncaught = False\ntry:\n    d.pop('b')\nexcept KeyError:\n    caught = True\nok = a == 1 and b == 7 and caught\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_importlib_dunder_import() {
    let source = "import importlib\nok = callable(importlib.__import__)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_sys_dont_write_bytecode_flag() {
    let source = "import sys\nok = isinstance(sys.dont_write_bytecode, bool)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_types_new_class_constructor() {
    let source = "import types\nC = types.new_class('C', (object,))\nok = isinstance(C, type)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_frozen_importlib_external_helpers() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_frozen_importlib_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let file = temp_dir.join("demo.py");
    std::fs::write(&file, "value = 1\n").expect("write module");
    let file_literal = file.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import _frozen_importlib_external as ext\nparts = ext._path_split('{file_literal}')\njoined = ext._path_join(parts[0], parts[1])\nstat = ext._path_stat('{file_literal}')\nu16 = ext._unpack_uint16(b'\\x01\\x02')\nu32 = ext._unpack_uint32(b'\\x01\\x02\\x03\\x04')\nu64 = ext._unpack_uint64(b'\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00')\nok = hasattr(ext, 'path_sep') and hasattr(ext, '_LoaderBasics') and parts[1] == 'demo.py' and joined[-7:] == 'demo.py' and stat.st_size >= 0 and u16 == 513 and u32 == 67305985 and u64 == 1\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn executes_frozen_importlib_spec_from_loader_helper() {
    let source = "import _frozen_importlib as frozen\nspec = frozen.spec_from_loader('pkg.mod', None, origin='x.py', is_package=False)\nfrozen._verbose_message('x')\nok = spec['name'] == 'pkg.mod' and spec['parent'] == 'pkg' and spec['origin'] == 'x.py' and not spec['is_package']\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_opcode_metadata_helpers() {
    let source = "import _opcode\nse = _opcode.stack_effect(82)\nok = (_opcode.has_arg(82) and _opcode.has_const(82) and _opcode.has_name(92) and _opcode.has_jump(70) and _opcode.has_free(97) and _opcode.has_local(84) and _opcode.has_exc(6) and (not _opcode.has_arg(27)) and isinstance(se, int) and _opcode.get_executor(None, 0) is None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_str_isspace_method() {
    let source = "ok = ' \\t\\n'.isspace() and not ''.isspace() and not 'a'.isspace()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_str_strip_method() {
    let source = "a = '  hi\\n'.strip()\n\
b = '..abc..'.strip('.')\n\
c = 'xyz'.strip('')\n\
ok = a == 'hi' and b == 'abc' and c == 'xyz'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_str_lstrip_and_rstrip_methods() {
    let source = "a = '  hi  '.lstrip()\n\
b = '  hi  '.rstrip()\n\
c = '..abc..'.lstrip('.')\n\
d = '..abc..'.rstrip('.')\n\
ok = a == 'hi  ' and b == '  hi' and c == 'abc..' and d == '..abc'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn class_objects_expose_module_attribute() {
    let source = "class C:\n    pass\nok = C.__module__ == '__main__'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn class_scope_comprehensions_resolve_class_and_module_names() {
    let source = "global_value = 9\nclass C:\n    base = [1, 2, 3]\n    copied = [item for item in base]\n    from_global = [global_value for _ in [0]]\nok = C.copied == [1, 2, 3] and C.from_global == [9] and not hasattr(C, 'item')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_milestone12_shim_symbols() {
    let source = "import sys, functools, inspect, re, io, _thread, math\n\
ok = (sys.byteorder in ('little', 'big') and hasattr(functools, 'cmp_to_key') and hasattr(inspect, 'Signature') and hasattr(re, 'Scanner') and hasattr(io, '__all__') and hasattr(_thread, 'start_new_thread') and hasattr(math, 'ldexp') and callable(exec) and __debug__ and ExceptionGroup is not None and BaseExceptionGroup is not None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn supports_subclassing_enumerate_builtin() {
    let source = "class MyEnum(enumerate):\n    pass\nok = isinstance(MyEnum, type)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn itertools_long_tail_helpers_work() {
    let source = r#"import itertools
import operator
acc = itertools.accumulate([1, 2, 3])
acc_init = itertools.accumulate([1, 2], initial=10)
comb = itertools.combinations('ABC', 2)
comb_rep = itertools.combinations_with_replacement([1, 2], 2)
comp = itertools.compress('ABCDEF', [1, 0, 1, 0, 1, 1])
dw = itertools.dropwhile(lambda x: x < 3, [1, 2, 3, 2, 1])
ff = itertools.filterfalse(lambda x: x % 2, [0, 1, 2, 3, 4])
ff_none = itertools.filterfalse(None, [0, 1, '', 2])
grp = itertools.groupby('AAABBC')
isl1 = itertools.islice(range(10), 3)
isl2 = itertools.islice(range(10), 2, 8, 3)
pw = itertools.pairwise([10, 20, 30])
sm = itertools.starmap(operator.add, [(1, 2), (3, 4)])
tw = itertools.takewhile(lambda x: x < 4, [1, 2, 3, 4, 1])
t1, t2 = itertools.tee([7, 8], 2)
zl = itertools.zip_longest([1, 2], [10], fillvalue=0)
ok = (
    acc == [1, 3, 6]
    and acc_init == [10, 11, 13]
    and comb == [('A', 'B'), ('A', 'C'), ('B', 'C')]
    and comb_rep == [(1, 1), (1, 2), (2, 2)]
    and comp == ['A', 'C', 'E', 'F']
    and dw == [3, 2, 1]
    and ff == [0, 2, 4]
    and ff_none == [0, '']
    and grp == [('A', ['A', 'A', 'A']), ('B', ['B', 'B']), ('C', ['C'])]
    and isl1 == [0, 1, 2]
    and isl2 == [2, 5]
    and pw == [(10, 20), (20, 30)]
    and sm == [3, 7]
    and tw == [1, 2, 3]
    and t1 == [7, 8]
    and t2 == [7, 8]
    and zl == [(1, 10), (2, 0)]
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn operator_getter_and_methodcaller_helpers_work() {
    let source = r#"import operator
class Inner:
    def __init__(self):
        self.b = 20
class X:
    def __init__(self):
        self.a = 10
        self.inner = Inner()
    def mul(self, x, scale=1):
        return (self.a + x) * scale
x = X()
item_single = operator.itemgetter(1)([7, 8, 9])
item_multi = operator.itemgetter(2, 0)('abc')
attr_single = operator.attrgetter('a')(x)
attr_multi = operator.attrgetter('a', 'inner.b')(x)
call_val = operator.methodcaller('mul', 5, scale=2)(x)
ok = item_single == 8 and item_multi == ('c', 'a') and attr_single == 10 and attr_multi == (10, 20) and call_val == 30
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn functools_cmp_to_key_orders_sorted_min_max() {
    let source = r#"import functools
def cmp_len_desc(a, b):
    if len(a) < len(b):
        return 1
    if len(a) > len(b):
        return -1
    if a < b:
        return -1
    if a > b:
        return 1
    return 0
values = ['bbb', 'a', 'cc', 'aa']
ordered = sorted(values, key=functools.cmp_to_key(cmp_len_desc))
smallest = min(values, key=functools.cmp_to_key(cmp_len_desc))
largest = max(values, key=functools.cmp_to_key(cmp_len_desc))
ok = ordered == ['bbb', 'aa', 'cc', 'a'] and smallest == 'bbb' and largest == 'a'
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exec_builtin_executes_source_string() {
    let source = "x = 1\nexec('x = x + 4')\nok = (x == 5)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exec_builtin_respects_explicit_globals_and_locals() {
    let source = "g = {'base': 10}\nl = {}\nexec('value = base + 5', g, l)\nok = (l['value'] == 15 and ('value' not in g))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn socket_byteorder_and_timeout_helpers_work() {
    let source = "import _socket\na = _socket.htons(0x1234)\nb = _socket.ntohs(a)\nc = _socket.htonl(0x12345678)\nd = _socket.ntohl(c)\n_socket.setdefaulttimeout(1.5)\nt = _socket.getdefaulttimeout()\n_socket.setdefaulttimeout(None)\nu = _socket.getdefaulttimeout()\nok = (b == 0x1234 and d == 0x12345678 and t == 1.5 and u is None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn socket_hostname_addrinfo_and_fromfd_smoke() {
    let source = "import _socket\nname = _socket.gethostname()\nhost = _socket.gethostbyname('127.0.0.1')\ninfo = _socket.getaddrinfo('127.0.0.1', 80)\nsock = _socket.fromfd(5, _socket.AF_INET, _socket.SOCK_STREAM)\nok = (isinstance(name, str) and len(name) >= 1 and host == '127.0.0.1' and len(info) >= 1 and sock is not None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pylong_basic_helpers_work() {
    let source = "import _pylong\nv = _pylong.int_from_string('1_234 ')\ns = _pylong.int_to_decimal_string(-42)\nq, r = _pylong.int_divmod(-7, 3)\np = _pylong.compute_powers(5, 2, 3)\nok = (v == 1234 and s == '-42' and q == -3 and r == 2 and p[4] == 16 and p[5] == 32)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pylong_decimal_inner_accepts_guard_keyword() {
    let source = "import _pylong\nv = _pylong._dec_str_to_int_inner('99', GUARD=4)\nok = (v == 99)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}
