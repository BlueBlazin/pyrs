use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pyrs::{compiler, parser, runtime::Value, vm::Vm};

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "pyrs_integration_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn runs_multi_module_package() {
    let root = temp_root();
    let pkg_dir = root.join("app");
    fs::create_dir_all(&pkg_dir).expect("create package dir");
    fs::write(pkg_dir.join("__init__.py"), "").expect("init");

    fs::write(
        pkg_dir.join("math_utils.py"),
        r#"
def add(a, b):
    return a + b
"#,
    )
    .expect("math_utils");

    fs::write(
        pkg_dir.join("models.py"),
        r#"
class Counter:
    def __init__(self, start):
        self.value = start
    def inc(self):
        self.value += 1
        return self.value
"#,
    )
    .expect("models");

    let entry = root.join("main.py");
    fs::write(
        &entry,
        r#"
from app.math_utils import add
from app.models import Counter

total = add(2, 3)
c = Counter(5)
res1 = c.inc()
res2 = c.inc()

numbers = [1, 2, 3]
s = 0
for n in numbers:
    s += n

result = total + res1 + res2 + s
"#,
    )
    .expect("entry");

    let source = fs::read_to_string(&entry).expect("read entry");
    let module = parser::parse_module(&source).expect("parse");
    let code = compiler::compile_module(&module).expect("compile");
    let mut vm = Vm::new();
    vm.add_module_path(&root);
    vm.execute(&code).expect("execute");

    assert_eq!(vm.get_global("result"), Some(Value::Int(24)));
}
