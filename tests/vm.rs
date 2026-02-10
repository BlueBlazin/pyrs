use pyrs::{
    compiler, parser,
    runtime::{BuiltinFunction, Object, Value},
    vm::Vm,
};
use std::path::PathBuf;
use std::process::Command;
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
            Object::Set(values) => Some(values.to_vec()),
            _ => None,
        },
        Some(Value::FrozenSet(obj)) => match &*obj.kind() {
            Object::FrozenSet(values) => Some(values.to_vec()),
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

fn assert_exception_global(
    vm: &Vm,
    name: &str,
    expected_type: &str,
    expected_message: Option<&str>,
) {
    match vm.get_global(name) {
        Some(Value::Exception(exc)) => {
            assert_eq!(exc.name, expected_type);
            match (exc.message.as_deref(), expected_message) {
                (Some(actual), Some(expected)) => assert_eq!(actual, expected),
                (None, None) => {}
                (actual, expected) => {
                    panic!("unexpected exception message for {name}: {actual:?} != {expected:?}")
                }
            }
        }
        other => panic!("expected exception global {name}, got {other:?}"),
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

fn python314_path() -> Option<PathBuf> {
    let candidate = PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/bin/python3");
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

fn cpython_lib_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let path = PathBuf::from(path);
        if path.join("ipaddress.py").is_file() {
            return Some(path);
        }
    }
    let candidates = [
        "/Users/$USER/Downloads/Python-3.14.3/Lib",
        "/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
    ];
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.join("ipaddress.py").is_file() {
            return Some(path);
        }
    }
    None
}

fn int_string(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::Int(number)) => Some(number.to_string()),
        Some(Value::BigInt(number)) => Some(number.to_string()),
        Some(Value::Bool(flag)) => Some(if flag {
            "1".to_string()
        } else {
            "0".to_string()
        }),
        _ => None,
    }
}

fn compile_cpython_pyc(source: &str, module_name: &str) -> Option<PathBuf> {
    let python = python314_path()?;
    let base = std::env::temp_dir().join(format!(
        "pyrs_vm_import_pyc_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time works")
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).expect("create temp dir");
    let py_path = base.join(format!("{module_name}.py"));
    std::fs::write(&py_path, source).expect("write source");

    let status = Command::new(python)
        .arg("-m")
        .arg("py_compile")
        .arg(&py_path)
        .status()
        .expect("run py_compile");
    assert!(status.success(), "py_compile failed");

    let cache_dir = base.join("__pycache__");
    let entries = std::fs::read_dir(&cache_dir).expect("read __pycache__");
    for entry in entries {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("pyc")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .starts_with(module_name)
        {
            return Some(path);
        }
    }
    None
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
fn executes_bigint_pow_shift_bitwise_and_comparison_ops() {
    let source = "\
a = 2 ** 128\n\
b = a >> 64\n\
c = 1 << 200\n\
d = c >> 199\n\
e = (-5) & 3\n\
f = (-5) | 3\n\
g = (-5) ^ 3\n\
h = ~(-5)\n\
i = a > (2 ** 127)\n\
j = a < (2 ** 129)\n\
k = a + 1\n\
l = a - 1\n\
m = a * 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");

    assert_eq!(
        int_string(vm.get_global("a")),
        Some("340282366920938463463374607431768211456".to_string())
    );
    assert_eq!(
        int_string(vm.get_global("b")),
        Some("18446744073709551616".to_string())
    );
    assert_eq!(int_string(vm.get_global("d")), Some("2".to_string()));
    assert_eq!(int_string(vm.get_global("e")), Some("3".to_string()));
    assert_eq!(int_string(vm.get_global("f")), Some("-5".to_string()));
    assert_eq!(int_string(vm.get_global("g")), Some("-8".to_string()));
    assert_eq!(int_string(vm.get_global("h")), Some("4".to_string()));
    assert_eq!(vm.get_global("i"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("j"), Some(Value::Bool(true)));
    assert_eq!(
        int_string(vm.get_global("k")),
        Some("340282366920938463463374607431768211457".to_string())
    );
    assert_eq!(
        int_string(vm.get_global("l")),
        Some("340282366920938463463374607431768211455".to_string())
    );
    assert_eq!(
        int_string(vm.get_global("m")),
        Some("1020847100762815390390123822295304634368".to_string())
    );
}

#[test]
fn executes_bigint_floor_div_mod_and_divmod_parity() {
    let source = "\
n = (1 << 130)\n\
a = n // 3\n\
b = n % 3\n\
c = divmod(-n, 3)\n\
d = divmod(n, -3)\n\
e = n // -3\n\
f = n % -3\n\
ok = (a * 3 + b == n and 0 <= b and b < 3 and c[0] * 3 + c[1] == -n and 0 <= c[1] and c[1] < 3 and d[0] * (-3) + d[1] == n and -3 < d[1] and d[1] <= 0 and e == d[0] and f == d[1])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_bigint_int_parsing_and_percent_formatting() {
    let source = "\
a = int('9223372036854775808')\n\
b = int('-9223372036854775809')\n\
c = int(b'0xffffffffffffffff', 0)\n\
d = '%x' % (1 << 80)\n\
e = '%o' % (1 << 70)\n\
f = '%X' % (-(1 << 68))\n\
ok = (a > (2 ** 63 - 1) and b < -(2 ** 63) and c == ((1 << 64) - 1) and int(d, 16) == (1 << 80) and int(e, 8) == (1 << 70) and f.startswith('-') and int(f[1:], 16) == (1 << 68))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_int_float_conversion_and_base0_underscore_rules() {
    let source = "\
a = int(1.9)\n\
b = int(-1.9)\n\
c = int(1e20)\n\
v1 = int('00', 0)\n\
v2 = int('0_0', 0)\n\
v3 = int('0x_ff', 0)\n\
ok = (a == 1 and b == -1 and c == int('100000000000000000000') and v1 == 0 and v2 == 0 and v3 == 255)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_bigint_from_bytes_to_bytes_and_bit_length_paths() {
    let source = "\
big = int.from_bytes(b'\\x01' + (b'\\x00' * 20), 'big')\n\
neg = int.from_bytes((b'\\xff' * 20), 'big', signed=True)\n\
roundtrip = big.to_bytes(21, 'big')\n\
bit_big = (1 << 130).bit_length()\n\
bit_bool = True.bit_length()\n\
ok = (big == (1 << 160) and neg == -1 and len(roundtrip) == 21 and roundtrip[0] == 1 and bit_big == 131 and bit_bool == 1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_int_to_bytes_default_and_keyword_paths() {
    let source = "\
a = (65).to_bytes()\n\
b = (65).to_bytes(byteorder='big')\n\
c = (65).to_bytes(length=2, byteorder='little')\n\
d = (-1).to_bytes(length=2, byteorder='big', signed=True)\n\
ok = (a == b'A' and b == b'A' and c == b'A\\x00' and d == b'\\xff\\xff')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn rejects_int_to_bytes_duplicate_argument_paths() {
    let module = parser::parse_module("(1).to_bytes(1, byteorder='big', length=1)\n")
        .expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(
        err.message.contains("multiple values"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn rejects_invalid_int_literal_underscore_and_base0_forms() {
    for source in [
        "int('010', 0)\n",
        "int('0_1', 0)\n",
        "int('_1')\n",
        "int('1_')\n",
        "int('1__2')\n",
        "int('0x__ff', 0)\n",
    ] {
        let module = parser::parse_module(source).expect("parse should succeed");
        let code = compiler::compile_module(&module).expect("compile should succeed");
        let mut vm = Vm::new();
        let err = vm.execute(&code).expect_err("execution should fail");
        assert!(
            err.message.contains("invalid literal"),
            "unexpected error: {}",
            err.message
        );
    }
}

#[test]
fn rejects_int_from_non_finite_float() {
    for source in ["int(float('nan'))\n", "int(float('inf'))\n"] {
        let module = parser::parse_module(source).expect("parse should succeed");
        let code = compiler::compile_module(&module).expect("compile should succeed");
        let mut vm = Vm::new();
        let err = vm.execute(&code).expect_err("execution should fail");
        assert!(
            err.message.contains("cannot convert float"),
            "unexpected error: {}",
            err.message
        );
    }
}

#[test]
fn rejects_int_to_bytes_overflow_paths() {
    for source in [
        "(1 << 80).to_bytes(10, 'big')\n",
        "(-129).to_bytes(1, 'big', True)\n",
    ] {
        let module = parser::parse_module(source).expect("parse should succeed");
        let code = compiler::compile_module(&module).expect("compile should succeed");
        let mut vm = Vm::new();
        let err = vm.execute(&code).expect_err("execution should fail");
        assert!(
            err.message.contains("int too big to convert"),
            "unexpected error: {}",
            err.message
        );
    }
}

#[test]
fn executes_range_with_large_bigint_stop_lazily() {
    let source = "\
it = iter(range(1 << 1000))\n\
a = next(it)\n\
b = next(it)\n\
ok = (a == 0 and b == 1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn imports_ipaddress_and_executes_ipv6_bigint_paths() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let handle = std::thread::Builder::new()
        .name("ipaddress-parity".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let source = "\
import ipaddress\n\
maxv = int(ipaddress.IPv6Address('ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff')) >> 120\n\
num = ipaddress.IPv6Network('2001:db8::/64').num_addresses\n\
ok = maxv == 255 and num == (1 << 64)\n";
            let module = parser::parse_module(source).expect("parse should succeed");
            let code = compiler::compile_module(&module).expect("compile should succeed");
            let mut vm = Vm::new();
            vm.add_module_path(&lib);
            vm.execute(&code).expect("execution should succeed");
            assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
        })
        .expect("spawn ipaddress thread");
    handle.join().expect("ipaddress thread should complete");
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
fn try_finally_reraises_original_exception_after_finally_calls() {
    let source = "def helper():\n    try:\n        1 / 0\n    except:\n        pass\n\ndef run():\n    try:\n        raise RuntimeError('boom')\n    finally:\n        helper()\n\nok = False\ntry:\n    run()\nexcept RuntimeError as exc:\n    ok = str(exc) == 'boom'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn exposes_sys_exception_for_active_handler_context() {
    let source = "import sys\nnone_before = (sys.exception() is None)\ndef probe():\n    return sys.exception() is not None\ninside = False\ntry:\n    raise ValueError('bad')\nexcept ValueError:\n    inside = probe()\nnone_after = (sys.exception() is None)\nok = none_before and inside and none_after\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn traceback_helpers_can_read_exception_traceback_attr() {
    let source = "tb_visible = False\ntry:\n    raise ValueError('bad')\nexcept ValueError as exc:\n    tb_visible = hasattr(exc, '__traceback__')\nok = tb_visible\n";
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
fn exposes_time_perf_counter_helpers() {
    let source = "import time\na = time.perf_counter()\nb = time.perf_counter()\nns = time.perf_counter_ns()\nok = isinstance(a, float) and isinstance(ns, int) and (b >= a)\n";
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
    let source =
        "d = {'a': 1, 'b': 2}\nks = list(d.keys())\nvs = list(d.values())\nis_ = list(d.items())\n";
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
fn issubclass_accepts_exception_type_values_from_raised_exceptions() {
    let source = "class MyErr(Exception):\n    pass\nok = False\ntry:\n    raise MyErr('boom')\nexcept Exception as err:\n    ok = issubclass(err.__class__, MyErr)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn executes_generator_yield_tuple_values() {
    let source =
        "def gen():\n    yield 1, 2, 3\ng = gen()\nv = g.__next__()\nok = (v == (1, 2, 3))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn executes_builtin_len_via_dunder_len() {
    let source = r#"class Sized:
    def __len__(self):
        return 7

x = len(Sized())
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(7)));
}

#[test]
fn builtin_len_rejects_invalid_dunder_len_results() {
    let source = r#"class Negative:
    def __len__(self):
        return -1

class NonInteger:
    def __len__(self):
        return 'bad'

neg = False
nonint = False
try:
    len(Negative())
except Exception as exc:
    neg = '__len__() should return >= 0' in str(exc)

try:
    len(NonInteger())
except Exception as exc:
    nonint = '__len__() should return an integer' in str(exc)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("neg"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("nonint"), Some(Value::Bool(true)));
}

#[test]
fn bool_and_control_flow_use_truth_protocol_methods() {
    let source = r#"class UsesLen:
    def __len__(self):
        return 0

class UsesBool:
    def __bool__(self):
        return False

class Both:
    def __len__(self):
        return 1
    def __bool__(self):
        return False

a = bool(UsesLen())
b = bool(UsesBool())
c = bool(Both())
if_len = 1 if UsesLen() else 0
not_len = not UsesLen()
default_bool = bool()
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("b"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("c"), Some(Value::Bool(false)));
    assert_eq!(vm.get_global("if_len"), Some(Value::Int(0)));
    assert_eq!(vm.get_global("not_len"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("default_bool"), Some(Value::Bool(false)));
}

#[test]
fn bool_rejects_non_bool_dunder_bool_result() {
    let source = r#"class BadBool:
    def __bool__(self):
        return 1

ok = False
try:
    bool(BadBool())
except TypeError as exc:
    ok = '__bool__ should return bool, returned int' in str(exc)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bool_rejects_non_integer_dunder_len_result() {
    let source = r#"class BadLen:
    def __len__(self):
        return 'bad'

ok = False
try:
    bool(BadLen())
except TypeError as exc:
    ok = "'str' object cannot be interpreted as an integer" in str(exc)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn comparison_result_uses_truth_protocol() {
    let source = r#"class Flag:
    def __init__(self, value):
        self.value = value
    def __bool__(self):
        return self.value

class Thing:
    def __eq__(self, other):
        return Flag(False)

ok = (Thing() == Thing()) is False
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn executes_bin_oct_hex_builtins() {
    let source = "a = bin(5)\n\
b = oct(8)\n\
c = hex(255)\n\
d = hex(-255)\n\
e = hex(True)\n\
f = hex(2**200)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Str("0b101".to_string())));
    assert_eq!(vm.get_global("b"), Some(Value::Str("0o10".to_string())));
    assert_eq!(vm.get_global("c"), Some(Value::Str("0xff".to_string())));
    assert_eq!(vm.get_global("d"), Some(Value::Str("-0xff".to_string())));
    assert_eq!(vm.get_global("e"), Some(Value::Str("0x1".to_string())));
    match vm.get_global("f") {
        Some(Value::Str(text)) => {
            assert!(text.starts_with("0x1"));
            assert!(text.len() > 10);
        }
        other => panic!("expected hex string for large int, got {other:?}"),
    }
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
fn executes_set_relationship_methods() {
    let source = "a = {1, 2, 3}\n\
b = {1, 2}\n\
c = {5, 6}\n\
sup = a.issuperset(b)\n\
sub = b.issubset(a)\n\
dis = a.isdisjoint(c)\n\
f_ok = frozenset({1, 2}).issuperset({2}) and frozenset({1, 2}).issubset({1, 2, 3})\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("sup"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("sub"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("dis"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("f_ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_set_and_frozenset_union_method() {
    let source = "a = {1, 2}.union({2, 3}, [4])\n\
b = frozenset({1, 2}).union({2, 3})\n\
ok = (len(a) == 4 and 4 in a and len(b) == 3 and 3 in b)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_set_and_frozenset_intersection_method() {
    let source = "a = {1, 2, 3}.intersection({2, 3, 4}, [3, 4])\n\
b = frozenset({1, 2, 3}).intersection({2, 3})\n\
ok = (a == {3} and b == frozenset({2, 3}))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_dict_equality_independent_of_insertion_order() {
    let source = "a = {'left': 1, 'right': 2}\nb = {'right': 2, 'left': 1}\nok = (a == b) and not (a != b)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_set_and_frozenset_equality_with_value_semantics() {
    let source = "a = {1, 2, 3}\n\
b = {3, 2, 1}\n\
c = frozenset({1, 2, 3})\n\
d = frozenset({3, 1, 2})\n\
ok = (a == b) and (c == d) and (a == c) and (c == a) and not (a != c)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_hash_equality_numeric_key_canonicalization() {
    let source = "\
d = {}\n\
d[1] = 'int'\n\
d[True] = 'bool'\n\
d[1.0] = 'float'\n\
s = {1, True, 1.0}\n\
ok = (len(d) == 1 and d[1] == 'float' and d[True] == 'float' and len(s) == 1 and (1 in s) and (True in s))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_set_binary_operators() {
    let source = "a = {1, 2, 3}\n\
b = {2, 4}\n\
sub = a - b\n\
inter = a & b\n\
xorv = a ^ b\n\
ok = (sub == {1, 3}) and (inter == {2}) and (xorv == {1, 3, 4})\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn set_relationship_methods_reject_unhashable_items() {
    let source = "{1}.issuperset([[]])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
}

#[test]
fn rejects_unhashable_dict_key_assignment() {
    let source = "d = {}\nd[[1, 2]] = 3\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
}

#[test]
fn rejects_unhashable_dict_literal_key() {
    let source = "d = {[1, 2]: 3}\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
}

#[test]
fn rejects_unhashable_dict_fromkeys_key() {
    let source = "d = dict.fromkeys([[1], [2]], 0)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
}

#[test]
fn rejects_unhashable_dict_key_membership() {
    let source = "d = {1: 2}\nok = [1] in d\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
}

#[test]
fn rejects_unhashable_set_item_add() {
    let source = "s = set()\ns.add([1, 2])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
}

#[test]
fn rejects_unhashable_collections_counter_item() {
    let source = "from collections import Counter\nCounter([[1], [2]])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
}

#[test]
fn rejects_unhashable_set_constructor_items() {
    let source = "s = set([[1], [2]])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("unhashable type: 'list'"));
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
m1_ok = (m1 is not None and m1.start() == 0 and m1.end() == 2)\n\
m2_ok = (m2 is not None and m2.start() == 1 and m2.end() == 3)\n\
m3_ok = (m3 is not None and m3.start() == 0 and m3.end() == 4)\n\
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
        Some(Value::Str(text)) => {
            assert!(text.contains("\"a\": 1"));
            assert!(text.contains("\"b\": [2, 3]"));
        }
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
    assert_eq!(vm.get_global("m1_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("m2_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("m3_ok"), Some(Value::Bool(true)));
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
fn json_dumps_supports_sort_keys_separators_and_default() {
    let source = r#"import json
class Unknown:
    pass
def fallback(obj):
    return {'fallback': 'ok'}
text = json.dumps({'b': 1, 'a': '\u263A'}, sort_keys=True, separators=(',', ':'), ensure_ascii=True)
fallback_text = json.dumps(Unknown(), default=fallback, sort_keys=True)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(
        vm.get_global("text"),
        Some(Value::Str("{\"a\":\"\\u263a\",\"b\":1}".to_string()))
    );
    assert_eq!(
        vm.get_global("fallback_text"),
        Some(Value::Str("{\"fallback\": \"ok\"}".to_string()))
    );
}

#[test]
fn json_import_prefers_cpython_pure_module_when_lib_path_is_added() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pure-json import preference test (CPython Lib path not available)");
        return;
    };
    let source = r#"import json
origin = getattr(json, '__file__', '')
norm = origin.replace("\\", "/")
ok = norm.endswith('/json/__init__.py') and ('/shims/' not in norm) and hasattr(json, 'loads') and hasattr(json, 'dumps')
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.enable_pure_json_preference();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn re_module_exposes_sre_surface_and_basic_match_works() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping re module surface test (CPython Lib path not available)");
        return;
    };
    let source = r#"import re
import _sre
m = re.match(r'a+', 'aaab')
ok = (
    hasattr(_sre, 'compile')
    and hasattr(_sre, 'ascii_tolower')
    and m is not None
    and m.group(0) == 'aaa'
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn re_import_prefers_cpython_pure_module_when_lib_path_is_added() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pure-re import preference test (CPython Lib path not available)");
        return;
    };
    let source = r#"import re
origin = getattr(re, '__file__', '')
norm = origin.replace("\\", "/")
ok = (
    norm.endswith('/re/__init__.py')
    and ('/shims/' not in norm)
    and hasattr(re, 'compile')
    and hasattr(re, 'Pattern')
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn int_subclass_custom_new_skips_double_builtin_initialization() {
    let source = r#"class X(int):
    def __new__(cls, value, name):
        return super(X, cls).__new__(cls, value)

obj = X(5, 'tag')
ok = isinstance(obj, X) and (obj == 5)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn _json_import_does_not_route_through_local_shim() {
    let source = r#"import _json
origin = getattr(_json, '__file__', '')
norm = origin.replace("\\", "/")
ok = ('/shims/' not in norm) and hasattr(_json, 'scanstring')
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn copyreg_imports_from_cpython_lib_without_shim_fallback() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping copyreg import test (CPython Lib path not available)");
        return;
    };
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);

    let source = r#"import sys
import copyreg
after = copyreg.__file__
after_norm = after.replace("\\", "/")
ok = ("/shims/" not in after_norm and after_norm.endswith("/copyreg.py"))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn getattribute_fallback_keeps_try_finally_state_stable() {
    let source = r#"class C:
    def __getattribute__(self, name):
        if name == "value":
            raise AttributeError("missing")
        return object.__getattribute__(self, name)
    def __getattr__(self, name):
        if name == "value":
            return 7
        raise AttributeError(name)

c = C()

def read_value():
    try:
        return c.value
    finally:
        marker = 1

ok = (read_value() == 7)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn struct_pack_unpack_and_offset_helpers_work() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping struct test (CPython Lib path not available)");
        return;
    };
    let source = r#"import struct
b = struct.pack('<B', 5)
q = struct.pack('<Q', 258)
u = struct.unpack('<Q', q)[0]
buf = bytearray(b'\x00\x00\x00\x00')
struct.pack_into('<H', buf, 1, 0x1234)
u2 = struct.unpack_from('<H', buf, 1)[0]
rows = list(struct.iter_unpack('<H', b'\x01\x00\x02\x00'))
ok = (b == b'\x05' and u == 258 and u2 == 0x1234 and rows == [(1,), (2,)] and struct.calcsize('<Q') == 8)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn struct_helpers_accept_bytes_like_format_values() {
    let source = r#"import _struct as struct
fmt = b"<4s4H2LH"
a = struct.calcsize(fmt)
b = struct.calcsize(memoryview(fmt))
packed = struct.pack(fmt, b"ABCD", 1, 2, 3, 4, 5, 6, 7)
ok = (a == 22 and b == 22 and isinstance(packed, bytes))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_protocol_byte_regression_for_struct_pack_is_fixed() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle protocol regression test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
data = pickle.dumps({'x': 1}, protocol=5)
first = data[0]
second = data[1]
value = pickle.loads(data)['x']
ok = (first == 0x80 and second == 5 and value == 1)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn picklebuffer_is_exposed_via_pickle_module() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping PickleBuffer shim test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
if not hasattr(pickle, "PickleBuffer"):
    ok = True
else:
    pb = pickle.PickleBuffer(b"abc")
    with pb.raw() as view:
        raw = bytes(view)
    pb.release()
    caught = False
    try:
        pb.raw()
    except ValueError:
        caught = True
    ok = (raw == b"abc" and caught)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exception_type_metatype_behaves_like_type_and_pickle_handles_builtin_exceptions() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping exception metatype/pickle test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
t = type(Warning)
meta_ok = (t is type and isinstance(t, type) and issubclass(t, type))
accel_ok = True
try:
    import _pickle
except Exception:
    accel_ok = True
else:
    accel_ok = all(hasattr(_pickle, name) for name in (
        "dump", "dumps", "load", "loads", "Pickler", "Unpickler"
    ))
    from _pickle import dump, dumps, load, loads, Pickler, Unpickler
    accel_ok = accel_ok and all((
        callable(dump), callable(dumps), callable(load), callable(loads),
        Pickler is not None, Unpickler is not None
    ))
data = pickle.dumps(Warning, protocol=0)
loaded = pickle.loads(data)
ok = (meta_ok and accel_ok and loaded is Warning)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn builtin_function_names_are_stable_and_pickle_roundtrips_functions() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping builtin-function pickle test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
name_ok = (len.__name__ == "len" and isinstance(len.__name__, str))
data = pickle.dumps(len, protocol=0)
loaded = pickle.loads(data)
ok = (name_ok and loaded is len)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_bytearray_protocol_zero_roundtrips_and_supports_bytes_contains_checks() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle bytearray protocol-0 test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
import pickletools
data = pickletools.optimize(pickle.dumps(bytearray(b"xyz"), 0))
loaded = pickle.loads(data)
ok = (loaded == bytearray(b"xyz"))
ok = ok and (b"bytearray" in data)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_protocol_zero_and_one_roundtrip_instances_with_required_init_args() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle getinitargs protocol test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
class NeedsArgs:
    def __init__(self, a, b):
        self.a = a
        self.b = b

ok = True
for proto in range(pickle.HIGHEST_PROTOCOL + 1):
    dumped = pickle.dumps(NeedsArgs(1, 2), proto)
    loaded = pickle.loads(dumped)
    ok = ok and (loaded.a, loaded.b) == (1, 2)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_list_subclass_roundtrip_preserves_items_and_instance_attrs() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle list-subclass roundtrip test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
class MyList(list):
    pass

ok = True
for proto in range(pickle.HIGHEST_PROTOCOL + 1):
    x = MyList([1, 2, 3])
    x.foo = 42
    y = pickle.loads(pickle.dumps(x, proto))
    ok = ok and (type(y) is MyList and list(y) == [1, 2, 3] and y.foo == 42 and x == y)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_compat_emits_legacy_globals_for_range_and_map() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle compat global test (CPython Lib path not available)");
        return;
    };
    let source = r#"import collections
import pickle
range_pickle = pickle.dumps(range(1, 7), 0)
map_pickle = pickle.dumps(map(int, '123'), 0)
defaultdict_pickle = pickle.dumps(collections.defaultdict(), 0)
defaultdict_roundtrip = pickle.loads(defaultdict_pickle)
ok = (
    b'c__builtin__\nxrange' in range_pickle and
    b'citertools\nimap' in map_pickle and
    b'ccollections\ndefaultdict' in defaultdict_pickle and
    type(defaultdict_roundtrip) is collections.defaultdict
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn nested_class_qualname_tracks_enclosing_class_path() {
    let source = r#"class Nested:
    class A:
        class B:
            pass
ok = (
    Nested.__qualname__ == "Nested" and
    Nested.A.__qualname__ == "Nested.A" and
    Nested.A.B.__qualname__ == "Nested.A.B"
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_roundtrips_nested_classes_from_picklecommon() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle nested-class roundtrip test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import Nested
ok = True
for proto in range(pickle.HIGHEST_PROTOCOL + 1):
    for obj in (Nested.A, Nested.A.B, Nested.A.B.C):
        ok = ok and (pickle.loads(pickle.dumps(obj, proto)) is obj)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn class_base_attribute_returns_first_base_class() {
    let source = r#"class A:
    pass
class B(A):
    pass
ok = (B.__base__ is A and A.__base__ is object)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_dispatch_table_none_item_raises_type_error() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle dispatch-table test (CPython Lib path not available)");
        return;
    };
    let source = r#"import io
import pickle
obj = object()
pickler = pickle.Pickler(io.BytesIO())
pickler.dispatch_table = {type(obj): None}
raised = False
try:
    pickler.dump(obj)
except TypeError:
    raised = True
ok = raised
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_protocol4_preserves_bytes_alias_identity() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle bytes alias test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
b = b""
x, y = pickle.loads(pickle.dumps((b, b), 4))
ok = (x is y)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_protocol4_dict_chunking_emits_multiple_setitems_for_large_dicts() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle dict chunking test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.pickletester import count_opcode
d = dict.fromkeys(range(2500))
s = pickle.dumps(d, 4)
ok = (count_opcode(pickle.SETITEMS, s) >= 2)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn int_subclass_equality_uses_numeric_value_semantics() {
    let source = r#"class myint(int):
    pass
a = myint(4)
b = myint(4)
ok = (a == b and not (a != b) and int(a) == 4 and int(b) == 4)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_roundtrip_preserves_int_subclass_value_equality() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle int-subclass equality test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
class myint(int):
    pass
x = myint(5)
y = pickle.loads(pickle.dumps(x, 4))
ok = (type(y) is myint and y == x and int(y) == 5)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_newobj_generic_matrix_from_pickletester_roundtrips() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle newobj generic matrix test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.pickletester import myclasses, protocols

ok = True
for proto in protocols:
    for C in myclasses:
        B = C.__base__
        x = C(C.sample)
        x.foo = 42
        y = pickle.loads(pickle.dumps(x, proto))
        ok = ok and (x == y) and (B(x) == B(y)) and (x.__dict__ == y.__dict__)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_slot_list_roundtrip_preserves_slots_and_dynamic_dict_attrs() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle SlotList roundtrip test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.pickletester import SlotList
x = SlotList([1, 2, 3])
x.foo = 42
x.bar = "hello"
y = pickle.loads(pickle.dumps(x, 2))
ok = (x == y and x.foo == y.foo and x.bar == y.bar and x.__dict__ == y.__dict__)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn int_instance_exposes_new_and_rejects_non_type_receiver() {
    let source = r#"ok = False
try:
    (42).__new__(42)
except TypeError:
    ok = True
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_object_reduce_base_call_does_not_recurse() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle base reduce recursion test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import REX_five
x = REX_five()
s = pickle.dumps(x, 4)
y = pickle.loads(s)
ok = (x._reduce_called == 1 and y._reduce_called == 1)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn recursive_list_equality_does_not_stack_overflow() {
    let source = r#"a = []
a.append(a)
b = []
b.append(b)
ok = (a == a and a == b and not (a != b))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn recursive_list_repr_uses_ellipsis_marker() {
    let source = r#"l = []
l.append(l)
ok = (repr(l) == "[[...]]")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn object_repr_on_instance_uses_fallback_without_recursing() {
    let source = r#"class C:
    pass
c = C()
c.foo = 1
text = repr(c)
ok = ("instance" in text)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn tuple_structural_equality_works_with_instance_members() {
    let source = r#"class C:
    def __eq__(self, other):
        return self.__dict__ == other.__dict__
a = C()
a.x = 1
b = C()
b.x = 1
t1 = ("abc", "abc", a, a)
t2 = ("abc", "abc", b, b)
ok = (t1 == t2 and not (t1 != t2))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bytearray_subclass_constructor_accepts_payload_and_supports_bytes_conversion() {
    let source = r#"class Z(bytearray):
    pass
z = Z(b"abc")
ok = (type(z) is Z and bytes(z) == b"abc")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bytearray_slice_assignment_and_deletion_follow_python_semantics() {
    let source = r#"buf = bytearray(b"abcd")
buf[1:3] = b"XY"
buf[::2] = b"pq"
del buf[1:3]
ok = (bytes(buf) == b"pd")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn classmethod_bound_method_reduce_ex_returns_getattr_tuple() {
    let source = r#"class C:
    @classmethod
    def f(cls):
        return cls.__name__
r = C.f.__reduce_ex__(4)
rebuilt = r[0](*r[1])
ok = (r[0] is getattr and r[1][0] is C and r[1][1] == "f" and rebuilt() == "C")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn function_qualname_includes_owner_class_path() {
    let source = r#"class Outer:
    class Inner:
        @staticmethod
        def cheese():
            return "cheese"
        def biscuits(self):
            return "biscuits"

ok = (Outer.Inner.cheese.__qualname__ == "Outer.Inner.cheese")
ok = ok and (Outer.Inner.biscuits.__qualname__ == "Outer.Inner.biscuits")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_zero_copy_bytearray_roundtrips_across_protocols() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping ZeroCopyBytearray pickle test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import ZeroCopyBytearray
ok = True
for proto in range(6):
    obj = ZeroCopyBytearray(b"xyz")
    a, b = pickle.loads(pickle.dumps((obj, obj), proto))
    ok = ok and (a == obj) and (a is b)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_class_methods_roundtrip_with_qualified_names() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle class-method qualname test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import PyMethodsTest
payload_a = pickle.dumps(PyMethodsTest.cheese, protocol=4)
payload_b = pickle.dumps(PyMethodsTest().biscuits, protocol=4)
a = pickle.loads(payload_a)
b = pickle.loads(payload_b)
ok = (a() == PyMethodsTest.cheese() and b() == PyMethodsTest().biscuits())
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_static_and_class_method_descriptors_raise_type_error() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle descriptor error test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import PyMethodsTest
ok = True
for descr in (PyMethodsTest.__dict__['cheese'], PyMethodsTest.__dict__['wine']):
    try:
        pickle.dumps(descr, protocol=4)
        ok = False
    except TypeError:
        pass
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_zero_copy_bytes_oob_buffers_preserve_identity() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping ZeroCopyBytes OOB pickle test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import ZeroCopyBytes
obj = ZeroCopyBytes(b"abcdefgh")
buffers = []
payload = pickle.dumps(obj, protocol=5, buffer_callback=lambda pb: buffers.append(pb.raw()))
a = pickle.loads(payload, buffers=buffers)
b = pickle.loads(payload, buffers=iter(buffers))
ok = (a is obj and b is obj and bytes(buffers[0]) == b"abcdefgh")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_proto5_frameless_bytearray_roundtrips() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping protocol-5 frameless pickle test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle, pickletools
frame_size = 64 * 1024
obj = {i: bytearray([i]) * frame_size for i in range(20)}
pickled = pickle.dumps(obj, protocol=5)
frame_positions = [pos for op, _, pos in pickletools.genops(pickled) if op.name == "FRAME"]
out = bytearray()
last = 0
for pos in frame_positions:
    out += pickled[last:pos]
    last = pos + 9
out += pickled[last:]
ok = (pickle.loads(out) == obj)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_respects_custom_getstate_and_errors_on_none_setstate() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping pickle setstate-none test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
class C:
    def __getstate__(self):
        return 1
    __setstate__ = None
payload = pickle.dumps(C())
try:
    pickle.loads(payload)
    ok = False
except TypeError:
    ok = True
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_complex_newobj_preserves_instance_dict_state() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping ComplexNewObj pickle state test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import ComplexNewObj
x = ComplexNewObj.__new__(ComplexNewObj, 0xface)
x.abc = 666
y = pickle.loads(pickle.dumps(x, 4))
ok = (x == y and x.__dict__ == y.__dict__)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_recursive_dict_subclass_roundtrips_identity() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping recursive dict-subclass pickle test (CPython Lib path not available)");
        return;
    };
    let source = r#"import pickle
from test.picklecommon import MyDict
d = MyDict()
d[1] = d
x = pickle.loads(pickle.dumps(d, 2))
ok = isinstance(x, MyDict) and list(x.keys()) == [1] and (x[1] is x)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pickle_dict_subclass_reduce_ex_uses_class_constructor_shape() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping dict-subclass reduce_ex shape test (CPython Lib path not available)");
        return;
    };
    let source = r#"from test.picklecommon import MyDict
d = MyDict()
d[1] = d
r = d.__reduce_ex__(2)
item = next(r[4])
ok = (len(r) == 5 and r[1][0] is MyDict and item[0] == 1 and (item[1] is d))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn class_new_returning_non_instance_skips_init() {
    let source = r#"class Factory:
    def __new__(cls, value):
        return value
    def __init__(self, value):
        raise RuntimeError("init should not run")
result = Factory(42)
ok = (result == 42)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn str_builtin_supports_bytes_decoding_signature() {
    let source = r#"a = str(b'\xff', 'latin-1')
b = str(bytearray(b'abc'), 'utf-8')
try:
    str('already-text', 'utf-8')
    c = False
except TypeError:
    c = True
ok = (a == 'ÿ' and b == 'abc' and c)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_literal_escapes_support_octal_and_control_sequences() {
    let source = r#"nul = "\0"
bell = "\a"
vert = "\v"
octal = "\123"
ok = (len(nul) == 1 and ord(nul) == 0 and ord(bell) == 7 and ord(vert) == 11 and octal == "S")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn str_index_matches_find_and_raises_value_error_when_missing() {
    let source = r#"raised = False
try:
    "abc".index("z")
except ValueError:
    raised = True
ok = ("abc".index("b") == 1 and "abc".find("b") == 1 and str.index("abc", "b") == 1 and raised)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn builtin_unbound_method_descriptors_cover_core_c_method_cases() {
    let source = r#"ok = ("abcd".index("c") == 2)
ok = ok and (str.index("abcd", "c") == 2)
ok = ok and ([1, 2, 3].__len__() == 3)
ok = ok and (list.__len__([1, 2, 3]) == 3)
ok = ok and ({1, 2}.__contains__(2))
ok = ok and set.__contains__({1, 2}, 2)
ok = ok and (bytearray.maketrans(b"ab", b"xy") == bytes.maketrans(b"ab", b"xy"))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn tuple_and_str_subclass_core_methods_work_for_bound_and_unbound_calls() {
    let source = r#"class TupleSub(tuple):
    pass
class StrSub(str):
    pass
t = TupleSub([1, 2, 2])
s = StrSub("sweet")
ok = (t.count(2) == 2 and TupleSub.count(t, 2) == 2 and s.count("e") == 2 and StrSub.count(s, "e") == 2)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn float_builtin_accepts_bytes_like_literals() {
    let source = "a = float(b'1.5')\n\
b = float(bytearray(b'2.25'))\n\
ok = (a == 1.5 and b == 2.25)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn memoryview_supports_context_manager_and_obj_attribute() {
    let source = r#"buf = bytearray(b"abc")
with memoryview(buf) as view:
    got = view.obj
ok = (got is buf)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn json_loads_accepts_utf8_bytes_and_bytearray() {
    let source = r#"import json
a = json.loads(b'{"x": 1, "y": [2, 3]}')
b = json.loads(bytearray(b'{"x": 1, "y": [2, 3]}'))
ok = (a['x'] == 1 and a['y'] == [2, 3] and b == a)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn json_decoder_scanstring_is_exposed() {
    let source = r#"import json.decoder as decoder
value, end = decoder.scanstring('"abc\\n"', 1)
value2, end2 = decoder.c_scanstring('"abc\\n"', 1)
ok = (value == "abc\n" and end == 7 and value2 == value and end2 == end)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_path_relpath_is_available_for_unittest_discovery() {
    let source = r#"import os
p = os.path.relpath('/tmp/a/b', '/tmp')
q = os.path.relpath('/tmp/a', '/tmp/a')
head, tail = os.path.split('/tmp/a/b.txt')
ok = (
    p == 'a/b'
    and q == '.'
    and os.path.isabs('/tmp')
    and not os.path.isabs('tmp')
    and head == '/tmp/a'
    and tail == 'b.txt'
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn executes_datetime_date_constructor() {
    let source = "import datetime\nitem = datetime.date(2024, 1, 2)\nok = item.year == 2024 and item.month == 1 and item.day == 2\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn imports_enum_module_and_handles_function_form() {
    let source = "import enum\nvalue = enum.Enum('E', type=int)\nok = value is not None\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fd = os.open('{path}', os.O_RDONLY)\n\
raw = os.read(fd, 5)\n\
os.close(fd)\n\
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
fn bytesio_supports_tell_and_core_methods() {
    let source = r#"import io
buf = io.BytesIO(b"abc")
pos0 = buf.tell()
part = buf.read(1)
pos1 = buf.tell()
buf.seek(0, 2)
pos2 = buf.tell()
buf.write(b"Z")
data = buf.getvalue()
buf.close()
closed_error = False
try:
    buf.tell()
except Exception:
    closed_error = True
ok = (pos0 == 0 and part == b"a" and pos1 == 1 and pos2 == 3 and data == b"abcZ" and closed_error)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
    assert_exception_global(&vm, "err", "ValueError", Some("bad"));
}

#[test]
fn exception_str_returns_message_only() {
    let source = "text = ''\ntry:\n    raise TypeError('bad')\nexcept TypeError as exc:\n    text = str(exc)\nok = text == 'bad'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn traceback_keeps_exception_type_prefix() {
    let source = "raise TypeError('bad')";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let err = vm.execute(&code).expect_err("execution should fail");
    assert!(err.message.contains("TypeError: bad"));
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
fn executes_try_finally_on_return_statement() {
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
fn executes_try_except_finally_on_return_statement() {
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
fn executes_finally_return_overrides_try_return() {
    let source = "def f():\n    try:\n        return 1\n    finally:\n        return 2\nresult = f()\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("result"), Some(Value::Int(2)));
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
    assert_exception_global(&vm, "cause", "ValueError", Some("inner"));
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
    assert_exception_global(&vm, "context", "ValueError", Some("inner"));
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
fn supports_dunder_class_on_common_values() {
    let source = "n = None.__class__.__name__\n\
l = [1].__class__.__name__\n\
b = True.__class__.__name__\n\
ok = (n == 'NoneType' and l == 'list' and b == 'bool')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn class_without_explicit_base_inherits_object() {
    let source = "class C:\n    pass\nok = (C.__bases__[0].__name__ == 'object')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn class_invocation_uses_metaclass_call_when_present() {
    let source = "class Meta(type):\n    def __call__(cls, *args, **kwargs):\n        return ('meta', cls.__name__, args[0], kwargs.get('x', 0))\nclass Sample(metaclass=Meta):\n    pass\nresult = Sample(7, x=9)\nok = result[0] == 'meta' and result[1] == 'Sample' and result[2] == 7 and result[3] == 9\n";
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
fn instance_dict_attribute_is_available_for_dynamic_instances() {
    let source = "class C:\n    pass\nx = C()\nx.foo = 42\nd = x.__dict__\nvia_object = object.__getattribute__(x, '__dict__')\nok = (d['foo'] == 42 and via_object['foo'] == 42)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn instance_dict_attribute_respects_slots_without_dict() {
    let source = "class S:\n    __slots__ = ('x',)\ns = S()\ns.x = 1\ncaught = False\ntry:\n    _ = s.__dict__\nexcept AttributeError:\n    caught = True\nok = caught\n";
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
fn executes_type_three_arg_class_creation_with_non_object_base() {
    let source = "class A:\n    pass\nN = type('N', (A,), {})\nok = (N.__bases__ == (A,) and isinstance(object.__new__(N), A))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn getattr_default_swallows_attribute_error_from_getattr() {
    let source = "class X:\n    def __getattr__(self, name):\n        raise AttributeError(name)\nx = X()\nok = (getattr(x, 'missing', None) is None and (not hasattr(x, 'missing')))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn getattr_default_handles_generator_missing_call_attr() {
    let source = "def gen():\n    yield 1\ng = gen()\nok = (getattr(g, '__call__', None) is None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
f1 = random.choices([10, 20, 30], k=4)\n\
random.seed(123)\n\
a2 = random.random()\n\
b2 = random.randrange(100)\n\
c2 = random.randint(3, 7)\n\
d2 = random.getrandbits(12)\n\
y = [1, 2, 3, 4]\n\
random.shuffle(y)\n\
e2 = random.choice(y)\n\
f2 = random.choices([10, 20, 30], k=4)\n\
same = (a1 == a2) and (b1 == b2) and (c1 == c2) and (d1 == d2) and (x == y) and (e1 == e2) and (f1 == f2)\n";
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
        "import sys\nimport importlib\nimport importlib.util\nsys.path = ['{path_literal}']\nspec = importlib.find_spec('mod')\nname = spec['name']\nname_attr = spec.name\nloader = spec['loader']\nloader_attr = spec.loader\nm = importlib.import_module('mod')\nu_spec = importlib.util.find_spec('mod')\nu_name = u_spec['name']\nu_name_attr = u_spec.name\nx = m.value\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("name"), Some(Value::Str("mod".to_string())));
    assert_eq!(
        vm.get_global("name_attr"),
        Some(Value::Str("mod".to_string()))
    );
    assert_eq!(
        vm.get_global("loader"),
        Some(Value::Str("pyrs.SourceFileLoader".to_string()))
    );
    assert_eq!(
        vm.get_global("loader_attr"),
        Some(Value::Str("pyrs.SourceFileLoader".to_string()))
    );
    assert_eq!(vm.get_global("u_name"), Some(Value::Str("mod".to_string())));
    assert_eq!(
        vm.get_global("u_name_attr"),
        Some(Value::Str("mod".to_string()))
    );
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
    let source = "import sys\nimport importlib\nimport importlib.util\nsys.path_importer_cache['/tmp/demo'] = 42\nbefore = '/tmp/demo' in sys.path_importer_cache\nimportlib.invalidate_caches()\nafter = '/tmp/demo' in sys.path_importer_cache\nspec = importlib.util.spec_from_file_location('demo', '/tmp/demo.py')\nok = before and (not after) and spec['name'] == 'demo' and spec.name == 'demo' and spec['origin'] == '/tmp/demo.py' and spec.origin == '/tmp/demo.py' and spec['loader'] == 'pyrs.SourceFileLoader' and spec.loader == 'pyrs.SourceFileLoader' and spec['has_location'] and spec.has_location and spec['cached'][-4:] == '.pyc' and spec.cached[-4:] == '.pyc'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exposes_os_error_family_exception_types() {
    let source = "ok = issubclass(TimeoutError, OSError) and issubclass(NotADirectoryError, OSError) and issubclass(PermissionError, OSError)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn main_module_spec_supports_attribute_access() {
    let source = "import sys\nok = (sys.modules['__main__'].__spec__.name == '__main__')\n";
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
fn imports_module_from_cached_pyc_without_source_file() {
    let Some(pyc_path) = compile_cpython_pyc("value = 173\n", "mod") else {
        eprintln!("python3.14 not found; skipping");
        return;
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let root_dir = std::env::temp_dir().join(format!("pyrs_import_pyc_module_{unique}"));
    let pycache = root_dir.join("__pycache__");
    std::fs::create_dir_all(&pycache).expect("create __pycache__");
    let target = pycache.join("mod.cpython-314.pyc");
    std::fs::copy(&pyc_path, &target).expect("copy pyc");

    let source = "import mod\nx = mod.value\nloader = mod.__loader__\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&root_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(173)));
    assert_eq!(
        vm.get_global("loader"),
        Some(Value::Str("pyrs.SourcelessFileLoader".to_string()))
    );

    let _ = std::fs::remove_file(target);
    let _ = std::fs::remove_dir(pycache);
    let _ = std::fs::remove_dir(root_dir);
}

#[test]
fn imports_package_from_cached_pyc_without_source_file() {
    let Some(pyc_path) = compile_cpython_pyc("value = 191\n", "__init__") else {
        eprintln!("python3.14 not found; skipping");
        return;
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let root_dir = std::env::temp_dir().join(format!("pyrs_import_pyc_package_{unique}"));
    let pkg_dir = root_dir.join("pkg");
    let pycache = pkg_dir.join("__pycache__");
    std::fs::create_dir_all(&pycache).expect("create package __pycache__");
    let target = pycache.join("__init__.cpython-314.pyc");
    std::fs::copy(&pyc_path, &target).expect("copy pyc");

    let source = "import pkg\nx = pkg.value\nloader = pkg.__loader__\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&root_dir);
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("x"), Some(Value::Int(191)));
    assert_eq!(
        vm.get_global("loader"),
        Some(Value::Str("pyrs.SourcelessFileLoader".to_string()))
    );

    let _ = std::fs::remove_file(target);
    let _ = std::fs::remove_dir(pycache);
    let _ = std::fs::remove_dir(pkg_dir);
    let _ = std::fs::remove_dir(root_dir);
}

#[test]
fn pkgutil_and_importlib_resources_shims_support_basic_resource_reads() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let root_dir = std::env::temp_dir().join(format!("pyrs_resource_shim_{unique}"));
    let pkg_dir = root_dir.join("pkg");
    std::fs::create_dir_all(&pkg_dir).expect("create temp package");
    std::fs::write(pkg_dir.join("__init__.py"), "").expect("write package init");
    std::fs::write(pkg_dir.join("data.txt"), "hello").expect("write package data");
    let path_literal = root_dir.to_string_lossy().replace('\\', "\\\\");
    let source = format!(
        "import sys\nsys.path = ['{path_literal}']\nimport pkgutil\nimport importlib.resources as resources\nraw = pkgutil.get_data('pkg', 'data.txt')\ntext = resources.files('pkg').joinpath('data.txt').read_text()\nok = raw == b'hello' and text == 'hello'\n"
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(pkg_dir.join("data.txt"));
    let _ = std::fs::remove_file(pkg_dir.join("__init__.py"));
    let _ = std::fs::remove_dir(pkg_dir);
    let _ = std::fs::remove_dir(root_dir);
}

#[test]
fn pkgutil_resolve_name_accepts_module_only_target() {
    let Some(lib_path) = cpython_lib_path() else {
        return;
    };
    let source = "import pkgutil\nimport tempfile\nresolved = pkgutil.resolve_name('tempfile')\nok = (resolved is tempfile)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn round_builtin_handles_numeric_ties_and_ndigits() {
    let source = r#"ok = (
    round(1.5) == 2 and
    round(2.5) == 2 and
    round(-1.5) == -2 and
    round(1250, -2) == 1200 and
    round(1350, -2) == 1400
)
v = round(1.2345, 3)
ok = ok and (v == 1.234)
ok = ok and (round(1.0, 400) == 1.0)
ok = ok and (round(1.0, -400) == 0.0)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn round_builtin_uses_dunder_round_for_user_types() {
    let source = r#"class C:
    def __round__(self, ndigits=None):
        if ndigits is None:
            return 42
        return ndigits + 1

ok = (round(C()) == 42)
ok = ok and (round(C(), 4) == 5)
ok = ok and (round(C(), ndigits=7) == 8)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn with_assert_raises_handles_missing_attr_without_stack_underflow() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping with/assertRaises regression (CPython Lib path not available)");
        return;
    };
    let source = r#"import unittest
class C:
    def __getattr__(self, name):
        raise AttributeError("x")
c = C()
t = unittest.TestCase()
ok = False
with t.assertRaises(AttributeError):
    c.dispatch_table
ok = True
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
    let source = "a = '(%s+)' % 'x'\n\
b = '%(name)s' % {'name': 'ok'}\n\
c = '%d' % 7\n\
d = '%.3f' % 1.23456\n\
e = '%6.2f' % 1.5\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("a"), Some(Value::Str("(x+)".to_string())));
    assert_eq!(vm.get_global("b"), Some(Value::Str("ok".to_string())));
    assert_eq!(vm.get_global("c"), Some(Value::Str("7".to_string())));
    assert_eq!(vm.get_global("d"), Some(Value::Str("1.235".to_string())));
    assert_eq!(vm.get_global("e"), Some(Value::Str("  1.50".to_string())));
}

#[test]
fn iter_on_non_iterable_raises_type_error() {
    let source = r#"ok = False
try:
    iter(1)
except TypeError:
    ok = True
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
    let source = "a = list(range(3))\nb = list(range(1, 4))\nc = list(range(5, 0, -2))\n";
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
        "a = list(range(stop=3))\nb = list(range(start=1, stop=4))\nc = list(range(start=1, stop=6, step=2))\n";
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
fn range_object_supports_len_index_and_slice() {
    let source = "r = range(1, 7)\na = r[-1]\nb = r[2]\nc = r[2:]\nok = (len(r) == 6 and a == 6 and b == 3 and list(c) == [3, 4, 5, 6])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn executes_nested_for_with_break_continue_without_outer_iterator_corruption() {
    let source = "pairs = []\nfor i in range(41, 46):\n    row = []\n    for j in [41, 59, 69]:\n        if j < 59:\n            continue\n        if j >= 64:\n            break\n        row.append(j)\n    pairs.append((i, row))\nok = pairs == [(41, [59]), (42, [59]), (43, [59]), (44, [59]), (45, [59])]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn list_comprehension_evaluates_first_iterable_in_outer_scope() {
    let source = "A = 1\nB = 2\nnames = [x for x in dir() if x.isupper() and not x.startswith('_')]\nok = ('A' in names) and ('B' in names)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn re_match_supports_char_classes_ranges_and_plus() {
    let source = "import re\nm = re.match('[A-Z][A-Z0-9_]+$', 'INT')\nn = re.match('[A-Z][A-Z0-9_]+$', 'dump')\nok = (m is not None) and (n is None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn re_match_supports_basic_capturing_parentheses() {
    let source = "import re\nok = re.match('(-*A-*)', 'A') is not None\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn re_match_exposes_group_groups_and_end() {
    let source = "import re\nm = re.match('([A])([AO]*)', 'AOO')\nok = (m is not None and m.group(1) == 'A' and m.groups() == ('A', 'OO') and m.end() == 3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn argparse_parse_args_accepts_explicit_positional_list() {
    let Some(lib_path) = cpython_lib_path() else {
        return;
    };
    let source = "import argparse\np = argparse.ArgumentParser()\np.add_argument('x')\nns = p.parse_args(['hello'])\nok = (ns.x == 'hello')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn executes_match_sequence_or_and_as_patterns() {
    let source = "subject = [1, 2, 3, 4]\nmatch subject:\n    case [1, *middle, 4]:\n        seq_ok = len(middle) == 2 and middle[0] == 2 and middle[1] == 3\n    case _:\n        seq_ok = False\nvalue = 2\nmatch value:\n    case (1 as captured) | (2 as captured):\n        or_ok = captured == 2\n    case _:\n        or_ok = False\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("seq_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("or_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("captured"), Some(Value::Int(2)));
}

#[test]
fn executes_match_mapping_and_class_patterns() {
    let source = "class Point:\n    __match_args__ = ('x', 'y')\n    def __init__(self, x, y):\n        self.x = x\n        self.y = y\ndata = {'kind': 'pt', 'x': 3, 'y': 4, 'extra': 9}\nmatch data:\n    case {'kind': 'pt', 'x': x, 'y': y, **rest}:\n        map_ok = x == 3 and y == 4 and len(rest) == 1 and ('extra' in rest)\n    case _:\n        map_ok = False\npt = Point(3, 4)\nmatch pt:\n    case Point(3, y=yy):\n        class_ok = yy == 4\n    case _:\n        class_ok = False\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("map_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("class_ok"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("x"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("y"), Some(Value::Int(4)));
}

#[test]
fn rejects_duplicate_capture_names_in_match_pattern() {
    let source = "match value:\n    case [x, x]:\n        out = 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let err = compiler::compile_module(&module).expect_err("compile should fail");
    assert!(
        err.message
            .contains("multiple assignments to name 'x' in pattern"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rejects_or_patterns_with_different_binding_sets() {
    let source = "match value:\n    case [x] | [x, y]:\n        out = 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let err = compiler::compile_module(&module).expect_err("compile should fail");
    assert!(
        err.message
            .contains("alternative patterns bind different names"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rejects_irrefutable_match_case_before_later_case() {
    let source = "match value:\n    case _:\n        out = 0\n    case 1:\n        out = 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let err = compiler::compile_module(&module).expect_err("compile should fail");
    assert!(
        err.message
            .contains("wildcard makes remaining patterns unreachable"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rejects_or_pattern_with_early_irrefutable_alternative() {
    let source = "match value:\n    case x | 1:\n        out = x\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let err = compiler::compile_module(&module).expect_err("compile should fail");
    assert!(
        err.message
            .contains("name capture 'x' makes remaining patterns unreachable"),
        "unexpected message: {}",
        err.message
    );
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
fn executes_except_star_with_exceptiongroup_split_semantics() {
    let source = "value_count = 0\ntype_count = 0\nruntime_count = 0\ntry:\n    raise ExceptionGroup('outer', [ValueError('a'), TypeError('b'), ExceptionGroup('inner', [ValueError('c'), RuntimeError('d')])])\nexcept* ValueError as eg:\n    value_count = len(eg.exceptions)\nexcept* TypeError as eg:\n    type_count = len(eg.exceptions)\nexcept* RuntimeError as eg:\n    runtime_count = len(eg.exceptions)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("value_count"), Some(Value::Int(2)));
    assert_eq!(vm.get_global("type_count"), Some(Value::Int(1)));
    assert_eq!(vm.get_global("runtime_count"), Some(Value::Int(1)));
}

#[test]
fn reraises_except_star_remainder() {
    let source = "caught = False\nremaining = 0\ntry:\n    try:\n        raise ExceptionGroup('outer', [ValueError('a'), RuntimeError('b')])\n    except* ValueError:\n        pass\nexcept ExceptionGroup as eg:\n    caught = True\n    remaining = len(eg.exceptions)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("caught"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("remaining"), Some(Value::Int(1)));
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
fn future_annotations_defer_name_resolution() {
    let source = r#"from __future__ import annotations
def get_pager() -> Pager:
    return None
ann = get_pager.__annotations__
ok = ann['return'] == 'Pager'
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn dataclass_decorator_accepts_keyword_only_form() {
    let source = r#"import dataclasses
@dataclasses.dataclass(frozen=True, slots=True)
class Point:
    x: int
ok = Point.__name__ == 'Point'
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn import_fresh_module_json_with_accelerator_present() {
    let Some(lib_path) = cpython_lib_path() else {
        return;
    };
    let source = "from test.support import import_helper\n\
cjson = import_helper.import_fresh_module('json', fresh=['_json'])\n\
ok = cjson is not None and hasattr(cjson, 'decoder') and hasattr(cjson, 'JSONDecodeError')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code =
        compiler::compile_module_with_filename(&module, "<import_fresh_json>").expect("compile");
    let mut vm = Vm::new();
    vm.add_module_path(&lib_path);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn positional_only_binding_does_not_conflict_with_named_positionals() {
    let source = "def f(a, /, b=2, c=3):\n    return a + b + c\nok = (f(1, b=4, c=5) == 10)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_subclass_inherits_basic_list_behavior() {
    let source = "class L(list):\n    pass\nx = L()\nx.append(1)\nx.append(2)\nvals = []\nfor value in x:\n    vals.append(value)\nok = (len(x) == 2 and vals == [1, 2] and x[1] == 2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_subclass_constructor_accepts_iterable_argument() {
    let source = "class L(list):\n    pass\nx = L([1, 2, 3])\nok = (len(x) == 3 and x[0] == 1 and x[2] == 3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_remove_misses_raise_value_error() {
    let source = "caught = False\ntry:\n    [1].remove(2)\nexcept ValueError:\n    caught = True\nok = caught\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_pop_supports_default_and_index() {
    let source = "values = [1, 2, 3]\na = values.pop()\nb = values.pop(0)\nok = (a == 3 and b == 1 and values == [2])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
    let source = "import gc\nimport errno\nimport weakref\nimport _weakref\nimport array\nvals = array.array('B', b'AB')\nref_value = weakref.ref(1)\nout = []\nfor x in vals:\n    out.append(x)\nok = gc.isenabled() and errno.ENOENT == 2 and len(vals) == 2 and vals.itemsize == 1 and out == [65, 66] and ref_value == 1\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn weakref_finalize_exposes_detach_tuple_contract() {
    let source = r#"import weakref
class Box:
    pass
box = Box()
finalizer = weakref.finalize(box, str, 42, base=10)
detached = finalizer.detach()
ok = (
    detached is not None
    and detached[0] is box
    and detached[1] is str
    and detached[2] == (42,)
    and detached[3].get("base") == 10
    and finalizer.alive is False
    and finalizer.detach() is None
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn from_import_missing_name_raises_importerror() {
    let source = "ok = False\ntry:\n    from _testinternalcapi import hamt\nexcept ImportError:\n    ok = True\n";
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
fn sys_exit_raises_systemexit() {
    let source =
        "import sys\nok = False\ntry:\n    sys.exit(2)\nexcept SystemExit:\n    ok = True\n";
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
fn functools_lru_cache_decorator_with_maxsize_argument_is_callable() {
    let source = r#"import functools

@functools.lru_cache(8)
def add1(x):
    return x + 1

ok = (add1(1) == 2 and add1(2) == 3)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    let value = vm.execute(&code).expect("execution should succeed");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn functools_wraps_preserves_function_dict_metadata() {
    let source = "import functools\n\ndef base(x):\n    return x + 1\nbase.client_skip = lambda f: f\n\n@functools.wraps(base)\ndef wrapper(x):\n    return base(x)\n\nok = (hasattr(wrapper, 'client_skip') and wrapper.client_skip is base.client_skip and wrapper.__wrapped__ is base and wrapper.__name__ == 'base')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn functools_wraps_accepts_bound_method_inputs() {
    let source = "import functools\nclass C:\n    @classmethod\n    def setUpClass(cls):\n        return 1\nwrapped = functools.wraps(C.setUpClass)(lambda cls: None)\nok = (wrapped.__name__ == 'setUpClass' and hasattr(wrapped, '__wrapped__'))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn enumerate_accepts_iterator_inputs() {
    let source = "it = (x for x in [10, 20])\nout = enumerate(it, start=3)\nok = (out == [(3, 10), (4, 20)])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn filter_builtin_supports_callable_and_none_predicate() {
    let source = "a = filter(lambda x: x % 2 == 0, [1, 2, 3, 4])\nb = filter(None, [0, 1, '', 'ok'])\nok = (a == [2, 4] and b == [1, 'ok'])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_slice_assignment_supports_replacement_and_extended_steps() {
    let source = "a = [0, 1, 2, 3, 4]\na[1:4] = [10, 11]\nb = [0, 1, 2, 3, 4, 5]\nb[::2] = [9, 8, 7]\nerr = False\ntry:\n    b[::2] = [1]\nexcept Exception:\n    err = True\nok = (a == [0, 10, 11, 4] and b == [9, 1, 8, 3, 7, 5] and err)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn functools_cached_property_caches_instance_value() {
    let source = r#"import functools
class C:
    def __init__(self):
        self.calls = 0
    @functools.cached_property
    def value(self):
        self.calls += 1
        return 40 + self.calls
c = C()
a = c.value
b = c.value
ok = (a == 41 and b == 41 and c.calls == 1 and hasattr(c, 'value'))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn functools_cached_property_exposes_descriptor_on_class_access() {
    let source = r#"import functools
class C:
    @functools.cached_property
    def value(self):
        return 1
desc = C.value
ok = (hasattr(desc, '__get__') and desc.attrname == 'value')
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
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
fn instance_dict_mutation_reflects_in_attribute_lookup() {
    let source = "class C:\n    pass\nc = C()\nd = c.__dict__\nd['x'] = 7\nc.y = 9\nok = (c.x == 7 and d['y'] == 9 and c.__dict__ is d)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn object_new_via_super_accepts_explicit_target_class() {
    let source = "class A:\n    pass\nclass B(A):\n    pass\nobj = super(A, B).__new__(B)\nok = isinstance(obj, B)\n";
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
fn io_textiowrapper_init_wraps_binary_buffer_for_readline() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_textiowrapper_{unique}.txt"));
    std::fs::write(&temp, b"alpha\nbeta\n").expect("write sample file");

    let source = format!(
        "import io\n\
path = {path:?}\n\
raw = io.open(path, 'rb')\n\
text = io.TextIOWrapper(raw, 'utf-8')\n\
line = text.readline()\n\
text.seek(0)\n\
lines = text.readlines()\n\
text.close()\n\
ok = (line == 'alpha\\n' and lines == ['alpha\\n', 'beta\\n'])\n",
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn io_open_accepts_pathlike_dunder_fspath_string() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_fspath_str_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");
    let path = temp.to_string_lossy().replace('\\', "\\\\");

    let source = format!(
        r#"import io
class P:
    def __fspath__(self):
        return '{path}'
reader = io.open(P(), 'rb')
data = reader.read()
reader.close()
ok = (data == b'payload')
"#,
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_accepts_pathlike_dunder_fspath_bytes() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_fspath_bytes_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");
    let path = temp.to_string_lossy().replace('\\', "\\\\");

    let source = format!(
        r#"import io
import os
path = '{path}'
class P:
    def __fspath__(self):
        return os.fsencode(path)
reader = io.open(P(), 'rb')
data = reader.read()
reader.close()
ok = (data == b'payload')
"#,
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_binary_allows_none_text_kwargs() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_binary_none_kwargs_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        "import io\n\
path = {path:?}\n\
reader = io.open(path, 'rb', encoding=None, errors=None, newline=None)\n\
data = reader.read()\n\
reader.close()\n\
ok = (data == b'payload')\n",
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_rejects_illegal_newline_values() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_newline_validation_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
msgs = []
for nl in ('x', '\n\n', '\r\r', 'abc'):
    try:
        io.open(path, 'r', newline=nl)
    except Exception as exc:
        msgs.append(str(exc))
ok = (len(msgs) == 4 and all('illegal newline value:' in m for m in msgs))
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_translates_universal_newlines_in_default_text_mode() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_universal_newline_{unique}.txt"));
    std::fs::write(&temp, b"a\r\nb\rc\n").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
reader = io.open(path, 'r')
data = reader.read()
reader.close()
reader = io.open(path, 'r')
lines = reader.readlines()
reader.close()
ok = (data == 'a\nb\nc\n' and lines == ['a\n', 'b\n', 'c\n'])
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_preserves_newline_bytes_when_newline_empty() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_newline_empty_{unique}.txt"));
    std::fs::write(&temp, b"a\r\nb\rc\n").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
reader = io.open(path, 'r', newline='')
data = reader.read()
reader.close()
reader = io.open(path, 'r', newline='')
lines = reader.readlines()
reader.close()
ok = (data == 'a\r\nb\rc\n' and lines == ['a\r\n', 'b\r', 'c\n'])
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_writes_with_explicit_newline_translation() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_write_newline_{unique}.txt"));

    let source = format!(
        r#"import io
path = {path:?}
writer = io.open(path, 'w', newline='\r\n')
writer.write('a\nb\n')
writer.close()
reader = io.open(path, 'rb')
payload = reader.read()
reader.close()
ok = (payload == b'a\r\nb\r\n')
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_rejects_unbuffered_text_mode() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_text_buffering_zero_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
caught = False
try:
    io.open(path, 'r', buffering=0)
except ValueError:
    caught = True
ok = caught
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_rejects_modes_without_rwax_component() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_mode_validation_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
count = 0
for mode in ('', 'b', '+'):
    try:
        io.open(path, mode)
    except ValueError:
        count += 1
ok = (count == 3)
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_fd_ignores_opener_and_respects_closefd_false() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_fd_opener_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        r#"import io
import os
path = {path:?}
fd = os.open(path, os.O_RDONLY)
reader = io.open(fd, 'r', opener=os.open, closefd=False)
payload = reader.read()
reader.close()
closed_ok = False
try:
    os.close(fd)
    closed_ok = True
except OSError:
    closed_ok = False
ok = (payload == 'payload' and closed_ok)
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_missing_path_is_catchable_as_os_error_family() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let missing = std::env::temp_dir().join(format!("pyrs_io_missing_{unique}.txt"));
    let _ = std::fs::remove_file(&missing);

    let source = format!(
        r#"import io
path = '{path}'
caught_specific = False
caught_generic = False
try:
    io.open(path, 'r')
except FileNotFoundError:
    caught_specific = True
except OSError:
    caught_generic = True
ok = caught_specific and not caught_generic
"#,
        path = missing.to_string_lossy().replace('\\', "\\\\"),
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn io_open_binary_uses_buffered_classes_with_raw_link() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_buffered_class_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
reader = io.open(path, 'rb')
writer = io.open(path, 'wb')
random = io.open(path, 'r+b')
raw = io.open(path, 'rb', buffering=0)
ok = (
    type(reader).__name__ == 'BufferedReader'
    and type(writer).__name__ == 'BufferedWriter'
    and type(random).__name__ == 'BufferedRandom'
    and type(raw).__name__ == 'FileIO'
    and type(reader.raw).__name__ == 'FileIO'
)
reader.close()
writer.close()
random.close()
raw.close()
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_text_wrapper_close_marks_buffer_chain_closed() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_close_chain_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
text = io.open(path, 'r')
buffer = text.buffer
raw = text.raw
text.close()
ok = text.closed and buffer.closed and raw.closed
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
}

#[test]
fn io_open_rejects_negative_fd_from_opener() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("pyrs_io_bad_opener_{unique}.txt"));
    std::fs::write(&temp, b"payload").expect("write sample file");

    let source = format!(
        r#"import io
path = {path:?}
caught = False
try:
    io.open(path, 'r', opener=lambda p, flags: -1)
except ValueError:
    caught = True
ok = caught
"#,
        path = temp.display().to_string()
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(temp);
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
out = '{out}'\n\
fd = os.open(path, os.O_RDONLY)\n\
is_tty = os.isatty(fd)\n\
os.close(fd)\n\
fdw = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_TRUNC)\n\
written = os.write(fdw, b'xyz')\n\
os.close(fdw)\n\
fdr = os.open(out, os.O_RDONLY)\n\
written_bytes = os.read(fdr, 16)\n\
os.close(fdr)\n\
written_ok = (written == 3 and written_bytes == b'xyz')\n\
st = os.stat(path)\n\
lst = os.lstat(path)\n\
pst = posix.stat(path)\n\
items = os.scandir(root)\n\
name_ok = any(item.name == 'sample.txt' for item in items)\n\
wait_ok = os.WIFEXITED(5 << 8) and os.WEXITSTATUS(5 << 8) == 5 and not os.WIFSIGNALED(5 << 8)\n\
ok = (not is_tty) and written_ok and st.st_size == 5 and lst.st_size == 5 and pst.st_size == 5 and name_ok and wait_ok\n",
        path = file.to_string_lossy().replace('\\', "\\\\"),
        root = temp_dir.to_string_lossy().replace('\\', "\\\\"),
        out = temp_dir
            .join("written.txt")
            .to_string_lossy()
            .replace('\\', "\\\\"),
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_file(temp_dir.join("written.txt"));
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
fn executes_os_open_exclusive_raises_file_exists_error() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_os_excl_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let file = temp_dir.join("exclusive.txt");

    let source = format!(
        r#"import os
path = '{path}'
fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
os.close(fd)
caught = False
try:
    os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
except FileExistsError:
    caught = True
ok = caught
"#,
        path = file.to_string_lossy().replace('\\', "\\\\"),
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
fn executes_os_mkdir_raises_file_exists_error() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_mkdir_exists_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let child = temp_dir.join("child");

    let source = format!(
        r#"import os
path = '{path}'
os.mkdir(path)
caught = False
try:
    os.mkdir(path)
except FileExistsError:
    caught = True
ok = caught
"#,
        path = child.to_string_lossy().replace('\\', "\\\\"),
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_dir(child);
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
fn executes_unicode_escape_codec_variants() {
    let source = "text = '\\\\x\\n\\t\\u263A'\nraw = text.encode('raw-unicode-escape')\nuni = text.encode('unicode-escape')\nraw_roundtrip = raw.decode('raw-unicode-escape')\nuni_roundtrip = uni.decode('unicode-escape')\nok = (raw == b'\\\\x\\n\\t\\\\u263a' and uni == b'\\\\\\\\x\\\\n\\\\t\\\\u263a' and raw_roundtrip == text and uni_roundtrip == text)\n";
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
fn collections_namedtuple_constructor_accepts_positional_and_keyword_fields() {
    let source = "import collections\nT = collections.namedtuple('T', 'a b c')\nx = T(1, 2, 3)\ny = T(a=4, b=5, c=6)\nok = (x.a == 1 and x.b == 2 and x.c == 3 and y.a == 4 and y.b == 5 and y.c == 6)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn collections_namedtuple_supports_percent_tuple_formatting() {
    let source = "import collections\nM = collections.namedtuple('M', 'a b c')\nm = M(1, 2, 'x')\ns = 'First has %d, Second has %d:  %r' % m\nok = (s == \"First has 1, Second has 2:  'x'\")\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn collections_namedtuple_instances_support_iteration_len_and_getitem() {
    let source = "import collections\nM = collections.namedtuple('M', 'a b c')\nm = M(1, 2, 3)\na, b, c = m\nok = (a == 1 and b == 2 and c == 3 and len(m) == 3 and m[1] == 2 and list(m) == [1, 2, 3])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn collections_namedtuple_make_builds_instances_from_iterables() {
    let source = "import collections\nM = collections.namedtuple('M', 'x y')\na = M._make([4, 5])\nb = M._make((6, 7))\nok = isinstance(a, M) and a.x == 4 and a.y == 5 and b.x == 6 and b.y == 7\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn collections_namedtuple_supports_ordering_for_sort() {
    let source = "import collections\nM = collections.namedtuple('M', 'a b c')\nx = M(1, 2, 3)\ny = M(1, 3, 0)\nordered = sorted([y, x])\nok = (x < y) and ordered[0] == x and ordered[1] == y\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exception_hierarchy_matches_oserror_children_expectations() {
    let source = "errs = [BrokenPipeError, ChildProcessError, ConnectionAbortedError, ConnectionError, ConnectionRefusedError, ConnectionResetError, FileExistsError, InterruptedError, IsADirectoryError, ProcessLookupError]\nok = all(issubclass(e, OSError) for e in errs) and issubclass(BrokenPipeError, ConnectionError)\n";
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
fn executes_thread_count_baseline() {
    let source = "import _thread\nok = isinstance(_thread._count(), int) and _thread._count() >= 1\n";
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
    let source =
        "import _warnings\n_warnings._acquire_lock()\n_warnings._release_lock()\nok = True\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn unittest_failed_test_keeps_active_exception_context() {
    let Some(lib_path) = cpython_lib_path() else {
        eprintln!("skipping unittest _FailedTest regression (CPython Lib not found)");
        return;
    };
    let handle = std::thread::Builder::new()
        .name("unittest-failed-test".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let source = "import unittest\nclass T(unittest.TestCase):\n    def test_ok(self):\n        self.assertTrue(True)\nsuite = unittest.defaultTestLoader.loadTestsFromName('missing', T)\nresult = unittest.TextTestRunner(verbosity=0, failfast=True).run(suite)\nok = (not result.wasSuccessful()) and len(result.errors) == 1\n";
            let module = parser::parse_module(source).expect("parse should succeed");
            let code = compiler::compile_module(&module).expect("compile should succeed");
            let mut vm = Vm::new();
            vm.add_module_path(&lib_path);
            vm.execute(&code).expect("execution should succeed");
            assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
        })
        .expect("spawn unittest regression thread");
    handle.join().expect("unittest regression thread should complete");
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
fn executes_string_capitalize_method() {
    let source = "a = 'hELLo world'.capitalize()\nb = ''.capitalize()\nok = (a == 'Hello world') and (b == '')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_backslash_newline_continuation_matches_python_behavior() {
    let source = "s = '''\\\nA\nB'''\nok = (s == 'A\\nB')\n";
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
fn sorted_and_list_sort_use_instance_lt_methods() {
    let source = "\
class K:
    def __init__(self, x):
        self.x = x
    def __lt__(self, other):
        return self.x < other.x

vals = [K(3), K(1), K(2)]
sorted_vals = sorted(vals)
vals.sort()
ok = (sorted_vals[0].x, sorted_vals[1].x, sorted_vals[2].x) == (1, 2, 3) and (vals[0].x, vals[1].x, vals[2].x) == (1, 2, 3)
";
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
    let source = "def f():\n    return None\nf.marker = 1\ndef g():\n    return None\ng.__dict__.update(f.__dict__)\nvals = [1]\nvals.extend((2, 3))\nvals.reverse()\nd = {}\nitem = d.setdefault('k', [])\nitem.append(4)\nd.update({'a': 1}, b=2)\nd.update([('c', 3)])\nok = vals == [3, 2, 1] and d['k'] == [4] and d['a'] == 1 and d['b'] == 2 and d['c'] == 3 and g.marker == 1\n";
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
    let source = "import _pylong\nv = _pylong.int_from_string('1_234 ')\ns = _pylong.int_to_decimal_string(-42)\nq, r = _pylong.int_divmod(-7, 3)\np = _pylong.compute_powers(5, 2, 3)\nbig = _pylong.int_from_string('123456789012345678901234567890')\nbig_s = _pylong.int_to_decimal_string(big)\nbq, br = _pylong.int_divmod(big, 97)\nok = (v == 1234 and s == '-42' and q == -3 and r == 2 and p[4] == 16 and p[5] == 32 and big_s == '123456789012345678901234567890' and bq * 97 + br == big and 0 <= br and br < 97)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pylong_decimal_inner_accepts_guard_keyword() {
    let source =
        "import _pylong\nv = _pylong._dec_str_to_int_inner('99', GUARD=4)\nok = (v == 99)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn float_fromhex_and_hex_helpers_work() {
    let source = "a = float.fromhex('0x1.8p+1')\n\
b = float.hex(3.0)\n\
c = float.fromhex('inf')\n\
ok = (a == 3.0 and b.startswith('0x1.8p+') and c > 1e300)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn str_maketrans_helper_supports_core_forms() {
    let source = "d1 = str.maketrans({'a': 'x', 98: 'y'})\n\
d2 = str.maketrans('ab', 'xy', 'c')\n\
ok = (d1[97] == 'x' and d1[98] == 'y' and d2[97] == 120 and d2[99] is None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bytes_maketrans_and_int_from_bytes_support_stdlib_style_usage() {
    let source = "table = bytes.maketrans(b'ab', b'xy')\n\
value = int.from_bytes(map(int, ['1', '2', '3', '4']), 'big')\n\
ok = (table[97] == 120 and table[98] == 121 and value == 0x01020304)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn map_is_lazy_over_unbounded_iterables() {
    let source = "import itertools\n\
m = map(lambda x: x + 1, itertools.count())\n\
a = next(m)\n\
b = next(m)\n\
ok = (a == 1 and b == 2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn map_callable_exception_is_raised_on_iteration() {
    let init_module =
        parser::parse_module("m = map(lambda x: 1 // 0, [1])\n").expect("parse should succeed");
    let init_code = compiler::compile_module(&init_module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&init_code).expect("execution should succeed");

    let next_module = parser::parse_module("next(m)\n").expect("parse should succeed");
    let next_code = compiler::compile_module(&next_module).expect("compile should succeed");
    let err = vm.execute(&next_code).expect_err("next(m) should raise");
    assert!(
        err.message.contains("ZeroDivisionError"),
        "expected ZeroDivisionError, got: {}",
        err.message
    );
}

#[test]
fn re_bytes_character_class_pattern_matches_tempfile_name_shape() {
    let source = "import re\n\
m = re.match(b'^[a-z0-9_-]{8}$', b'm9bo88vi')\n\
ok = (m is not None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_predicates_isascii_isdigit_and_islower_work() {
    let source = "ok = ('123'.isdigit() and 'abc'.islower() and 'abc'.isascii() and 'abc123'.isalnum() and not ''.isdigit())\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_isidentifier_supports_basic_python_identifier_rules() {
    let source = "ok = ('group_1'.isidentifier() and '_x'.isidentifier() and not ''.isidentifier() and not '9x'.isidentifier() and not 'x-y'.isidentifier())\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_partition_helpers_work() {
    let source = "a = 'ab=cd'.partition('=')\n\
b = 'ab=cd'.rpartition('=')\n\
c = 'abcd'.partition('=')\n\
ok = (a == ('ab', '=', 'cd') and b == ('ab', '=', 'cd') and c == ('abcd', '', ''))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_split_accepts_keyword_arguments() {
    let source = "parts = 'a:b:c'.split(':', maxsplit=1)\nok = (parts == ['a', 'b:c'])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_rsplit_accepts_keyword_arguments_and_whitespace_semantics() {
    let source = "a = 'a:b:c'.rsplit(':', maxsplit=1)\n\
b = '   foo   '.split(maxsplit=0)\n\
c = '   foo   '.rsplit(maxsplit=0)\n\
ok = (a == ['a:b', 'c'] and b == ['foo   '] and c == ['   foo'])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_splitlines_supports_keepends_and_crlf() {
    let source = "a = 'x\\ny\\r\\nz'.splitlines()\n\
b = 'x\\ny\\r\\nz'.splitlines(True)\n\
c = ''.splitlines()\n\
ok = (a == ['x', 'y', 'z'] and b == ['x\\n', 'y\\r\\n', 'z'] and c == [])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn string_find_accepts_optional_bounds_and_keywords() {
    let source = "a = 'abcabc'.find('bc')\n\
b = 'abcabc'.find('bc', 2)\n\
c = 'abcabc'.find('bc', end=3)\n\
ok = (a == 1 and b == 4 and c == 1)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn compile_builtin_exec_mode_returns_code_object() {
    let source = "co = compile('x = 40 + 2', '<inline>', 'exec')\n\
scope = {}\n\
exec(co, scope)\n\
ok = (scope['x'] == 42)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn int_builtin_uses_dunder_int_for_instances() {
    let source = "class C:\n    def __int__(self):\n        return 7\nok = (int(C()) == 7)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn staticmethod_does_not_bind_instance_receiver() {
    let source = "class C:\n    @staticmethod\n    def f(x):\n        return x + 1\nok = (C.f(1) == 2 and C().f(1) == 2)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn classmethod_binds_class_receiver_for_class_and_instance_access() {
    let source = "class C:\n    value = 3\n    @classmethod\n    def f(cls, x):\n        return cls.value + x\nok = (C.f(2) == 5 and C().f(2) == 5)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn builtin_type_descriptor_attrs_exist_for_reduction_paths() {
    let source = "out = []\nlist.append(out, 1)\nok = (out == [1] and int.__add__(1, 2) == 3)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn super_uses_owner_class_for_classmethod_descriptors() {
    let source = "class A:\n    @classmethod\n    def who(cls):\n        return cls.__name__\nclass B(A):\n    @classmethod\n    def who(cls):\n        return super(B, cls).who()\nok = (B.who() == 'B')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn init_subclass_zero_arg_super_runs_for_subclasses() {
    let source = r#"class Base:
    def __init_subclass__(cls, **kwargs):
        super().__init_subclass__(**kwargs)
        cls.flag = True

class Child(Base):
    pass

ok = Child.flag
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn user_classes_expose_qualname_for_unittest_loader_paths() {
    let source = "class KeyOrderingTest:\n    pass\nok = (KeyOrderingTest.__qualname__ == 'KeyOrderingTest')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn class_body_can_resolve_enclosing_function_locals() {
    let source = r#"def make_pickler(base):
    dt = {'x': 1}
    class MyPickler(base):
        dispatch_table = dt
    return MyPickler.dispatch_table is dt

ok = make_pickler(object)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bytes_membership_accepts_bytes_like_needles() {
    let source = "payload = b'c__builtin__\\nbytearray\\n(tR.'\n\
needle = b'bytearray'\n\
ok = (needle in payload)\n\
ok = ok and (bytearray(b'bytearray') in payload)\n\
ok = ok and (memoryview(b'bytearray') in payload)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_sort_supports_key_and_reverse_keywords() {
    let source = "values = ['bbb', 'a', 'cc']\nvalues.sort(key=len, reverse=True)\nok = (values == ['bbb', 'cc', 'a'])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_sort_accepts_cmp_to_key_wrappers() {
    let source = "import functools\n\
names = ['test_c', 'test_a', 'test_b']\n\
cmpf = lambda a, b: (a > b) - (a < b)\n\
names.sort(key=functools.cmp_to_key(cmpf))\n\
ok = (names == ['test_a', 'test_b', 'test_c'])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_sort_keeps_list_identity_and_sorts_in_place() {
    let source = "values = [3, 1, 2]\nident = id(values)\nvalues.sort()\nok = (id(values) == ident and values == [1, 2, 3])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn list_sort_raises_when_list_is_modified_during_sort() {
    let source = r#"values = [3, 2, 1]
def keyf(x):
    values.append(0)
    return x
raised = False
try:
    values.sort(key=keyf)
except ValueError as exc:
    raised = ('list modified during sort' in str(exc))
ok = (raised and values == [1, 2, 3])
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn object_reduce_ex_reconstructs_builtin_payloads() {
    let source = r#"lst = [1, 2, 3]
dct = {'a': 1, 'b': 2}
lst_ctor, lst_args, lst_state = object.__reduce_ex__(lst, 4)
dct_ctor, dct_args, dct_state = object.__reduce_ex__(dct, 4)
lst_roundtrip = lst_ctor(*lst_args)
dct_roundtrip = dct_ctor(*dct_args)
ok = (lst_roundtrip == lst and dct_roundtrip == dct and lst_state is None and dct_state is None)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn object_reduce_ex_honors_custom_reduce_tuple_payloads() {
    let source = r#"class REXSix:
    def __init__(self, items=None):
        self.items = [] if items is None else list(items)
    def append(self, item):
        self.items.append(item)
    def __reduce__(self):
        return type(self), (), None, iter(self.items), None

class REXSeven:
    def __init__(self, table=None):
        self.table = {} if table is None else dict(table)
    def __setitem__(self, key, value):
        self.table[key] = value
    def __reduce__(self):
        return type(self), (), None, None, iter(self.table.items())

six = REXSix([1, 2, 3])
seven = REXSeven({'a': 1, 'b': 2})
r6 = object.__reduce_ex__(six, 0)
r7 = object.__reduce_ex__(seven, 0)
ctor6, args6, state6, list_items, dict_items6 = r6
ctor7, args7, state7, list_items7, dict_items = r7
round6 = ctor6(*args6)
for item in list_items:
    round6.append(item)
round7 = ctor7(*args7)
for key, value in dict_items:
    round7[key] = value
ok = (len(r6) == 5 and len(r7) == 5
      and state6 is None and state7 is None
      and dict_items6 is None and list_items7 is None
      and round6.items == [1, 2, 3]
      and round7.table == {'a': 1, 'b': 2})
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bytes_subclass_constructor_accepts_payload_and_roundtrips_bytes() {
    let source = r#"class AuthenticationString(bytes):
    pass

value = AuthenticationString(b'abc')
ok = isinstance(value, AuthenticationString) and bytes(value) == b'abc'
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bound_method_doc_lookup_is_supported() {
    let source = "class T:\n    def test_one(self):\n        'doc'\n        return 1\nm = T().test_one\nok = (m.__doc__ == None)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_urandom_returns_requested_number_of_bytes() {
    let source = "import os\npayload = os.urandom(16)\nok = (isinstance(payload, bytes) and len(payload) == 16)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_terminal_size_helpers_are_available() {
    let source = "import os\na = os.get_terminal_size()\nb = os.terminal_size((100, 40))\nok = (a.columns == 80 and a.lines == 24 and b.columns == 100 and b.lines == 40)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn colorize_decolor_strips_ansi_sequences() {
    let source =
        "import _colorize\ns = _colorize.decolor('\\x1b[31mhello\\x1b[0m')\nok = (s == 'hello')\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn threading_local_baseline_type_is_available() {
    let source = "import threading\nx = threading.local()\nx.value = 7\nok = (x.value == 7)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn dataclasses_core_helpers_work() {
    let source = r#"import dataclasses
class C:
    pass
C.__dataclass_fields__ = {'x': 1, 'y': 2}
obj = C()
obj.x = 10
obj.y = 20
field_info = dataclasses.field(default=3, repr=False)
values = dataclasses.asdict(obj)
as_tuple = dataclasses.astuple(obj)
repl = dataclasses.replace(obj, y=99)
made = dataclasses.make_dataclass('Point', ['x', ('y', int)])
ok = (dataclasses.is_dataclass(C)
      and dataclasses.is_dataclass(obj)
      and len(dataclasses.fields(C)) == 2
      and field_info['default'] == 3
      and values['x'] == 10 and values['y'] == 20
      and as_tuple == (10, 20)
      and repl.y == 99
      and dataclasses.is_dataclass(made))
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn posixsubprocess_fork_exec_is_explicitly_unsupported() {
    let source = r#"import _posixsubprocess
caught = False
try:
    _posixsubprocess.fork_exec()
except Exception:
    caught = True
ok = caught
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn subprocess_args_from_interpreter_flags_returns_list() {
    let source =
        "import subprocess\nflags = subprocess._args_from_interpreter_flags()\nok = isinstance(flags, list)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn sys_stream_helpers_return_expected_values() {
    let source = "import sys\n\
n = sys.stdout.write('x')\n\
f = sys.stdout.flush()\n\
t = sys.stdout.isatty()\n\
ok = (n == 1 and f is None and t is False)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn module_getattr_fallback_returns_dynamic_attribute() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pyrs_module_getattr_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should work")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp module dir");
    std::fs::write(
        temp_dir.join("modgetattr.py"),
        "def __getattr__(name):\n    if name == 'binary':\n        return 42\n    raise AttributeError(name)\n",
    )
    .expect("write module file");

    let source = "import modgetattr\nok = (modgetattr.binary == 42)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(&temp_dir);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn dict_copy_returns_independent_shallow_copy() {
    let source = "d = {'x': 1}\ncopy = d.copy()\nd['x'] = 2\nok = (copy['x'] == 1 and d['x'] == 2 and copy is not d)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn dict_pop_heavy_mutation_keeps_hash_index_consistent() {
    let source = "d = {}\nfor i in range(300):\n    d[i] = i\nfor i in range(100):\n    d.pop(i)\nfor i in range(100, 200):\n    d[i] = i * 2\nok = (len(d) == 200 and d[150] == 300 and d[250] == 250 and (0 in d) is False and (299 in d) is True)\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_module_baseline_reader_writer_and_registry_work() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
class Sink:
    def __init__(self):
        self.parts = []
    def write(self, text):
        self.parts.append(text)
        return len(text)
sink = Sink()
w = csv.writer(sink)
n = w.writerow(['a', 'b,c'])
rows = list(csv.reader(['a,b', 'c,d']))
ok = (n == 9 and ''.join(sink.parts) == 'a,"b,c"\r\n' and rows == [['a', 'b'], ['c', 'd']] and 'excel' in csv.list_dialects() and csv.get_dialect('excel') is not None)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_module_handles_escapechar_skipinitialspace_and_field_limits() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
class Sink:
    def __init__(self):
        self.parts = []
    def write(self, text):
        self.parts.append(text)
        return len(text)
rows = list(csv.reader(['a, b, c'], skipinitialspace=True))
rows2 = list(csv.reader([r'a\,b,c'], quotechar=None, escapechar='\\'))
sink = Sink()
w = csv.writer(sink, quotechar=None, escapechar='\\', quoting=csv.QUOTE_NONE, lineterminator='\n')
w.writerow(['a,b', 'c'])
csv.field_size_limit(3)
limit_raised = False
try:
    list(csv.reader(['abcd']))
except Exception:
    limit_raised = True
ok = (rows == [['a', 'b', 'c']] and rows2 == [['a,b', 'c']] and ''.join(sink.parts) == 'a\\,b,c\n' and limit_raised)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_writer_accepts_empty_lineterminator_and_reports_value_errors() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
import io
sink = io.StringIO(newline='')
w = csv.writer(sink, lineterminator='')
w.writerow(['a', 'b'])
m1 = None
m2 = None
m3 = None
try:
    csv.writer(io.StringIO(), delimiter='\n')
except Exception as exc:
    m1 = str(exc)
try:
    csv.writer(io.StringIO(), delimiter=';', quotechar=';')
except Exception as exc:
    m2 = str(exc)
try:
    csv.writer(io.StringIO(), delimiter=';', lineterminator=';')
except Exception as exc:
    m3 = str(exc)
ok = (sink.getvalue() == 'a,b' and m1 == 'bad delimiter value' and m2 == 'bad delimiter or quotechar value' and m3 == 'bad delimiter or lineterminator value')
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_reader_treats_empty_line_as_empty_row() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
rows = list(csv.reader(['a,b', '', 'c,d']))
ok = (rows == [['a', 'b'], [], ['c', 'd']])
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_reader_supports_quote_modes_and_dialect_overrides() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
class Base(csv.Dialect):
    delimiter = "\t"
    quotechar = '"'
    doublequote = True
    skipinitialspace = False
    lineterminator = "\r\n"
    quoting = csv.QUOTE_MINIMAL
csv.register_dialect(
    "x",
    Base,
    delimiter=";",
    quotechar="'",
    doublequote=False,
    skipinitialspace=True,
    lineterminator="\n",
    quoting=csv.QUOTE_ALL,
)
d = csv.get_dialect("x")
rows_quote_none = list(csv.reader(['1,",3,",5'], quoting=csv.QUOTE_NONE, escapechar='\\'))
rows_quote_strings = list(csv.reader([',3,"5",7.3, 9'], quoting=csv.QUOTE_STRINGS))
rows_quote_notnull = list(csv.reader([',,"",'], quoting=csv.QUOTE_NOTNULL))
rows_escape_eof = list(csv.reader(['^'], escapechar='^'))
strict_raised = False
try:
    list(csv.reader(['a,"'], strict=True))
except Exception:
    strict_raised = True
ok = (
    d.delimiter == ';'
    and d.quotechar == "'"
    and d.doublequote is False
    and d.skipinitialspace is True
    and d.lineterminator == '\n'
    and d.quoting == csv.QUOTE_ALL
    and rows_quote_none == [['1', '"', '3', '"', '5']]
    and rows_quote_strings == [[None, 3, '5', 7.3, 9]]
    and rows_quote_notnull == [[None, None, '', None]]
    and rows_escape_eof == [['\n']]
    and strict_raised
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_roundtrips_quoted_newlines_for_all_line_terminators() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
import io
rows = [
    ['\na', 'b\nc', 'd\n'],
    ['\r\ni', 'j\r\nk', 'l\r\n'],
    ['\n\nu', 'v\n\nw', 'x\n\n'],
]
ok = True
for lineterminator in ('\r\n', '\n', '\r'):
    sink = io.StringIO(newline='')
    writer = csv.writer(sink, lineterminator=lineterminator)
    writer.writerows(rows)
    sink.seek(0)
    out = list(csv.reader(sink))
    if out != rows:
        ok = False
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_reader_iter_exception_is_catchable_in_try_except() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
class BadIterable:
    def __iter__(self):
        raise OSError("boom")
caught = False
try:
    csv.reader(BadIterable())
except OSError:
    caught = True
ok = caught
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_reader_iter_exception_propagates_through_var_call_forwarding() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
class BadIterable:
    def __iter__(self):
        raise OSError("boom")
def invoke(fn, *args, **kwargs):
    return fn(*args, **kwargs)
caught = False
try:
    invoke(csv.reader, BadIterable())
except OSError:
    caught = True
ok = caught
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_error_is_exception_subclass() {
    let source = r#"import _csv
ok = issubclass(_csv.Error, Exception)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn csv_dictreader_list_exhaustion_stops_cleanly() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import csv
from textwrap import dedent
data = dedent('''\
    FirstName,LastName
    Eric,Idle
    Graham,Chapman,Over1,Over2

    Under1
    John,Cleese
''').splitlines()
total = 0
for _ in range(200):
    rows = list(csv.DictReader(data))
    total += len(rows)
ok = (total == 800 and rows[0]['FirstName'] == 'Eric' and rows[-1]['LastName'] == 'Cleese')
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn contextlib_exit_allows_exception_traceback_assignment() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"from test.support.warnings_helper import save_restore_warnings_filters
ok = False
try:
    with save_restore_warnings_filters():
        import numpy
except ModuleNotFoundError:
    ok = True
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn io_text_encoding_handles_none_and_strings() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let source = r#"import io
ok = (io.text_encoding(None) == 'utf-8' and io.text_encoding('latin-1') == 'latin-1' and io.text_encoding(encoding='utf-8', stacklevel=3) == 'utf-8')
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_getenv_handles_default_and_none() {
    let source = "import os\nmissing = os.getenv('__PYRS_ENV_MISSING__')\nwith_default = os.getenv('__PYRS_ENV_MISSING__', 'fallback')\npath_value = os.getenv('PATH', '')\nok = (missing is None and with_default == 'fallback' and isinstance(path_value, str))\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_getenv_observes_os_environ_updates() {
    let source = r#"import os
key = "__PYRS_ENV_GUARD_TEST__"
prior = os.environ.get(key)
os.environ[key] = "present"
seen = os.getenv(key)
del os.environ[key]
missing = os.getenv(key, "fallback")
if prior is not None:
    os.environ[key] = prior
ok = (seen == "present" and missing == "fallback")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_fspath_supports_str_bytes_and_pathlike() {
    let source = r#"import os
class PathLike:
    def __fspath__(self):
        return b"/tmp/path"
ok = (
    os.fspath("name.txt") == "name.txt"
    and os.fspath(b"name.bin") == b"name.bin"
    and os.fspath(PathLike()) == b"/tmp/path"
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn pathlike_protocol_drives_isinstance_and_issubclass() {
    let source = r#"import os
class PathLike:
    def __fspath__(self):
        return "x"
ok = isinstance(PathLike(), os.PathLike) and issubclass(PathLike, os.PathLike)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_mkdir_creates_directory_with_mode_argument() {
    let source = r#"import os, time
name = "__pyrs_mkdir_test__" + str(int(time.time() * 1000000))
ret = os.mkdir(name, 0o700)
ok = (ret is None and name in os.listdir("."))
os.rmdir(name)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn os_walk_returns_directory_tree_rows() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("pyrs_walk_{unique}"));
    let child = root.join("child");
    std::fs::create_dir_all(&child).expect("create child dir");
    std::fs::write(root.join("root.txt"), b"root").expect("write root file");
    std::fs::write(child.join("child.txt"), b"child").expect("write child file");

    let source = format!(
        r#"import os
root = '{root}'
child = os.path.join(root, 'child')
rows = list(os.walk(root))
top_ok = False
child_ok = False
for path, dirs, files in rows:
    if path == root:
        top_ok = (dirs == ['child'] and files == ['root.txt'])
    if path == child:
        child_ok = (dirs == [] and files == ['child.txt'])
ok = top_ok and child_ok
"#,
        root = root.to_string_lossy().replace('\\', "\\\\"),
    );
    let module = parser::parse_module(&source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));

    let _ = std::fs::remove_file(child.join("child.txt"));
    let _ = std::fs::remove_file(root.join("root.txt"));
    let _ = std::fs::remove_dir(child);
    let _ = std::fs::remove_dir(root);
}

#[test]
fn str_endswith_supports_tuple_and_start_end() {
    let source = r#"text = "alpha-beta-gamma"
ok = (
    text.endswith(("gamma", "delta"))
    and text.endswith("beta", 0, 10)
    and not text.endswith("beta", 0, 9)
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bytes_startswith_and_endswith_support_basic_slices() {
    let source = r#"payload = b"prefix-body-suffix"
ok = (
    payload.startswith((b"prefix", b"zzz"))
    and payload.endswith(b"suffix")
    and payload.endswith(b"body", 7, 11)
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn bytes_join_supports_iterables_and_bytes_like_items() {
    let source = r#"sep = b":"
joined = sep.join([b"a", bytearray(b"b"), memoryview(b"c")])
ok = (joined == b"a:b:c")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn inspect_helpers_cover_getmodule_file_and_predicates() {
    let source = r#"import inspect
def sample():
    return 1
class C:
    pass
method = C().__str__
module = inspect.getmodule(sample)
source_file = inspect.getsourcefile(sample)
code_file = inspect.getfile(sample)
ok = (
    module is not None
    and isinstance(source_file, str)
    and isinstance(code_file, str)
    and inspect.ismethod(method)
    and inspect.isroutine(sample)
    and inspect.iscode(sample.__code__)
    and inspect.unwrap(sample) is sample
)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn exceptions_accept_keyword_attributes() {
    let source = r#"err = ImportError("boom", name="pkg.mod", path="/tmp/pkg/mod.py")
ok = (err is not None)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn user_class_module_builtin_attrs_are_not_descriptor_bound() {
    let source = r#"import gc
import os

class TempHandle:
    _close = os.close
    _unlink = os.unlink

    def __init__(self, path):
        self.name = path
        self.fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_TRUNC, 0o600)

    def __del__(self):
        self._close(self.fd)
        self._unlink(self.name)

def run():
    path = "pyrs_bind_builtin_close_unlink.tmp"
    if os.path.exists(path):
        os.unlink(path)
    TempHandle(path)
    gc.collect()
    exists = os.path.exists(path)
    if exists:
        os.unlink(path)
    return not exists

ok = run()
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn fstring_repr_conversion_uses_repr() {
    let source = r#"class C:
    def __str__(self):
        return "str-value"
    def __repr__(self):
        return "repr-value"

value = f"{C()!r}"
ok = (value == "repr-value")
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn temporary_iterable_with_del_keeps_collection_results() {
    let source = r#"class C:
    def __init__(self):
        self.data = [1, 2, 3]
    def __del__(self):
        pass
    def __iter__(self):
        for item in self.data:
            yield item

x = list(C())
y = enumerate(C())
ok = (x == [1, 2, 3] and y == [(0, 1), (1, 2), (2, 3)])
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn next_accepts_instance_with_next_without_iter() {
    let source = r#"class NextOnly:
    def __init__(self):
        self.i = 0
    def __next__(self):
        self.i += 1
        if self.i > 2:
            raise StopIteration
        return self.i

it = NextOnly()
a = next(it)
b = next(it)
done = False
try:
    next(it)
except StopIteration:
    done = True
ok = (a == 1 and b == 2 and done)
"#;
    let module = parser::parse_module(source).expect("parse should succeed");
    let code = compiler::compile_module(&module).expect("compile should succeed");
    let mut vm = Vm::new();
    vm.execute(&code).expect("execution should succeed");
    assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
}

#[test]
fn unittest_mock_open_callable_path_works() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let handle = std::thread::Builder::new()
        .name("unittest-mock-open".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let source = r#"import unittest.mock as mock
m = mock.mock_open(read_data="hello")
h = m()
ok = (h is not None and h.read() == "hello")
"#;
            let module = parser::parse_module(source).expect("parse should succeed");
            let code = compiler::compile_module(&module).expect("compile should succeed");
            let mut vm = Vm::new();
            vm.add_module_path(lib);
            vm.execute(&code).expect("execution should succeed");
            assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
        })
        .expect("spawn unittest mock_open regression thread");
    handle
        .join()
        .expect("unittest mock_open regression thread should complete");
}

#[test]
fn tempfile_namedtemporaryfile_unexpected_error_path_closes_file() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let handle = std::thread::Builder::new()
        .name("tempfile-unexpected-error".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let source = r#"import os
import tempfile
from unittest import mock
dir = tempfile.mkdtemp()
raised = False
with mock.patch("tempfile._TemporaryFileWrapper") as mock_ntf, mock.patch("io.open", mock.mock_open()) as mock_open:
    mock_ntf.side_effect = KeyboardInterrupt()
    try:
        tempfile.NamedTemporaryFile(dir=dir)
    except KeyboardInterrupt:
        raised = True
ok = raised and os.listdir(dir) == []
"#;
            let module = parser::parse_module(source).expect("parse should succeed");
            let code = compiler::compile_module(&module).expect("compile should succeed");
            let mut vm = Vm::new();
            vm.add_module_path(lib);
            vm.execute(&code).expect("execution should succeed");
            assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
        })
        .expect("spawn tempfile unexpected-error regression thread");
    handle
        .join()
        .expect("tempfile unexpected-error regression thread should complete");
}

#[test]
fn mktemp_style_temp_object_finalizer_runs_before_rmdir() {
    let Some(lib) = cpython_lib_path() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("pyrs_mktemp_style_{unique}"));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let dir_literal = temp_dir.to_string_lossy().replace('\\', "\\\\");
    let handle = std::thread::Builder::new()
        .name("tempfile-mktemp-finalizer".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let source = format!(
                r#"import os
import tempfile

class Mktemped:
    _unlink = os.unlink
    _flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL

    def __init__(self, directory):
        self.name = tempfile.mktemp(dir=directory)
        fd = os.open(self.name, self._flags, 0o600)
        os.close(fd)

    def __del__(self):
        self._unlink(self.name)

def run(directory):
    Mktemped(directory)
    try:
        os.rmdir(directory)
        return True
    except OSError:
        return False

ok = run('{dir}')
"#,
                dir = dir_literal
            );
            let module = parser::parse_module(&source).expect("parse should succeed");
            let code = compiler::compile_module(&module).expect("compile should succeed");
            let mut vm = Vm::new();
            vm.add_module_path(lib);
            vm.execute(&code).expect("execution should succeed");
            assert_eq!(vm.get_global("ok"), Some(Value::Bool(true)));
        })
        .expect("spawn tempfile mktemp-finalizer regression thread");
    handle
        .join()
        .expect("tempfile mktemp-finalizer regression thread should complete");

    let _ = std::fs::remove_dir_all(temp_dir);
}
