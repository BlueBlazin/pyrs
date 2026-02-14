use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use pyrs::bytecode::cpython::{dump_pyc, load_pyc};
use pyrs::bytecode::pyc::parse_pyc_header;
use pyrs::runtime::Value;
use pyrs::vm::Vm;

fn python_path() -> Option<PathBuf> {
    let candidate = PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/bin/python3");
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

fn compile_pyc(source: &str, module_name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "pyrs_pyc_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&base).expect("temp dir");
    let py_path = base.join(format!("{module_name}.py"));
    fs::write(&py_path, source).expect("write source");

    let python = python_path().expect("python3.14 not found");
    let status = Command::new(python)
        .arg("-m")
        .arg("py_compile")
        .arg(&py_path)
        .status()
        .expect("py_compile");
    assert!(status.success(), "py_compile failed");

    let cache_dir = base.join("__pycache__");
    let entries = fs::read_dir(&cache_dir).expect("pycache dir");
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("pyc")
            && path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .starts_with(module_name)
        {
            return path;
        }
    }
    panic!("pyc not found");
}

#[test]
fn executes_cpython_pyc() {
    if python_path().is_none() {
        eprintln!("python3.14 not found; skipping");
        return;
    }
    let source = r#"
def f(x, y=1, *, z=2):
    return x + y + z

result = f(1, z=3)

class C:
    def __init__(self, x):
        self.x = x
    def inc(self):
        return self.x + 1

obj = C(2)
result2 = obj.inc()

total = 0
for v in [1, 2, 3]:
    total += v
"#;

    let pyc_path = compile_pyc(source, "module");
    let bytes = fs::read(&pyc_path).expect("read pyc");
    let mut vm = Vm::new();
    let value = vm.execute_pyc_bytes(&bytes).expect("execute pyc");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("result"), Some(Value::Int(5)));
    assert_eq!(vm.get_global("result2"), Some(Value::Int(3)));
    assert_eq!(vm.get_global("total"), Some(Value::Int(6)));
}

#[test]
fn rewrites_and_executes_cpython_pyc() {
    if python_path().is_none() {
        eprintln!("python3.14 not found; skipping");
        return;
    }
    let source = r#"
def mul(a, b):
    return a * b

value = mul(6, 7)
"#;
    let pyc_path = compile_pyc(source, "rewrite_module");
    let bytes = fs::read(&pyc_path).expect("read pyc");
    let (header, _offset) = parse_pyc_header(&bytes).expect("header parse");
    let code = load_pyc(&bytes).expect("load pyc");
    let rewritten = dump_pyc(&code, &header).expect("dump pyc");

    let mut vm = Vm::new();
    let value = vm
        .execute_pyc_bytes(&rewritten)
        .expect("execute rewritten pyc");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("value"), Some(Value::Int(42)));
}

#[test]
fn executes_cpython_pyc_with_frozenset_and_fstring_conversion() {
    if python_path().is_none() {
        eprintln!("python3.14 not found; skipping");
        return;
    }
    let source = r#"
mask = 6 & 3
message = f"mask={mask!r}"
padded = f"{mask:04d}"
in_set = 2 in {1, 2, 3}
"#;

    let pyc_path = compile_pyc(source, "feature_matrix_module");
    let bytes = fs::read(&pyc_path).expect("read pyc");
    let mut vm = Vm::new();
    let value = vm.execute_pyc_bytes(&bytes).expect("execute pyc");
    assert_eq!(value, Value::None);
    assert_eq!(vm.get_global("mask"), Some(Value::Int(2)));
    assert_eq!(
        vm.get_global("message"),
        Some(Value::Str("mask=2".to_string()))
    );
    assert_eq!(
        vm.get_global("padded"),
        Some(Value::Str("0002".to_string()))
    );
    assert_eq!(vm.get_global("in_set"), Some(Value::Bool(true)));
}
