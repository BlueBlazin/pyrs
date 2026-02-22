use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use pyrs::{compiler, parser, runtime::Value, vm::Vm};

fn detect_cpython_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let candidate = PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/bin/python3");
    if candidate.is_file() {
        return Some(candidate);
    }
    let probe = Command::new("python3").arg("--version").output().ok()?;
    if probe.status.success() {
        return Some(PathBuf::from("python3"));
    }
    None
}

fn cpython_bin_or_panic() -> PathBuf {
    if let Some(bin) = detect_cpython_bin() {
        return bin;
    }
    if std::env::var("PYRS_CPYTHON_OPTIONAL").as_deref() == Ok("1") {
        eprintln!(
            "CPython binary not found; skipping differential tests due to PYRS_CPYTHON_OPTIONAL=1"
        );
        return PathBuf::new();
    }
    panic!("CPython 3.x binary not found. Set PYRS_CPYTHON_BIN or install python3.");
}

fn run_cpython_json(source: &str) -> Result<String, String> {
    let bin = cpython_bin_or_panic();
    if bin.as_os_str().is_empty() {
        return Ok(String::new());
    }
    let script = format!("{source}\nimport json\nprint(json.dumps(result))\n");
    let output = Command::new(bin)
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|err| format!("failed to launch CPython: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "CPython execution failed".to_string()
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_pyrs_json(source: &str) -> Result<String, String> {
    let wrapped = format!("{source}\nimport json\n__pyrs_json = json.dumps(result)\n");
    let module =
        parser::parse_module(&wrapped).map_err(|err| format!("parse error {}", err.message))?;
    let code =
        compiler::compile_module(&module).map_err(|err| format!("compile {}", err.message))?;
    let mut vm = Vm::new();
    vm.execute(&code)
        .map_err(|err| format!("runtime {}", err.message))?;
    match vm.get_global("__pyrs_json") {
        Some(Value::Str(text)) => Ok(text),
        other => Err(format!("missing __pyrs_json result: {other:?}")),
    }
}

fn pyrs_bin_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_pyrs") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/pyrs");
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

fn run_cpython_traceback(source: &str) -> Result<String, String> {
    let bin = cpython_bin_or_panic();
    if bin.as_os_str().is_empty() {
        return Ok(String::new());
    }
    let output = Command::new(bin)
        .arg("-c")
        .arg(source)
        .output()
        .map_err(|err| format!("failed to launch CPython: {err}"))?;
    if output.status.success() {
        return Err("expected CPython script to fail with traceback".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stderr).to_string())
}

fn run_pyrs_traceback(source: &str) -> Result<String, String> {
    let Some(bin) = pyrs_bin_path() else {
        return Err("pyrs binary not found".to_string());
    };
    let output = Command::new(bin)
        .arg("-c")
        .arg(source)
        .output()
        .map_err(|err| format!("failed to launch pyrs: {err}"))?;
    if output.status.success() {
        return Err("expected pyrs script to fail with traceback".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stderr).to_string())
}

fn unique_temp_script_path(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut path = std::env::temp_dir();
    let suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
    path.push(format!(
        "pyrs_diff_{prefix}_{}_{}_{}.py",
        std::process::id(),
        std::thread::current().name().unwrap_or("thread"),
        suffix
    ));
    path
}

fn run_traceback_via_file(bin: &PathBuf, source: &str) -> Result<String, String> {
    let path = unique_temp_script_path("traceback");
    std::fs::write(&path, source).map_err(|err| format!("failed to write temp script: {err}"))?;
    let output = Command::new(bin)
        .arg(&path)
        .output()
        .map_err(|err| format!("failed to launch interpreter: {err}"));
    let _ = std::fs::remove_file(&path);
    let output = output?;
    if output.status.success() {
        return Err("expected script to fail with traceback".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stderr).to_string())
}

fn compile_temp_pyc(source: &str, module_name: &str) -> Result<(PathBuf, PathBuf), String> {
    let bin = cpython_bin_or_panic();
    if bin.as_os_str().is_empty() {
        return Err("CPython binary not found".to_string());
    }
    let mut base = std::env::temp_dir();
    base.push(format!(
        "pyrs_diff_pyc_{module_name}_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("thread")
    ));
    std::fs::create_dir_all(&base).map_err(|err| format!("failed to create temp dir: {err}"))?;
    let py_path = base.join(format!("{module_name}.py"));
    std::fs::write(&py_path, source).map_err(|err| format!("failed to write temp source: {err}"))?;
    let output = Command::new(&bin)
        .arg("-m")
        .arg("py_compile")
        .arg(&py_path)
        .output()
        .map_err(|err| format!("failed to launch CPython py_compile: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "CPython py_compile failed".to_string()
        } else {
            stderr
        });
    }
    let cache_dir = base.join("__pycache__");
    let entries = std::fs::read_dir(&cache_dir)
        .map_err(|err| format!("failed to read __pycache__: {err}"))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read __pycache__ entry: {err}"))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("pyc")
            && path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .starts_with(module_name)
        {
            return Ok((base, path));
        }
    }
    Err("compiled .pyc not found".to_string())
}

fn run_traceback_via_pyc_file(bin: &PathBuf, pyc_path: &PathBuf) -> Result<String, String> {
    let output = Command::new(bin)
        .arg(pyc_path)
        .output()
        .map_err(|err| format!("failed to launch interpreter: {err}"))?;
    if output.status.success() {
        return Err("expected .pyc execution to fail with traceback".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stderr).to_string())
}

fn run_cpython_traceback_file(source: &str) -> Result<String, String> {
    let bin = cpython_bin_or_panic();
    if bin.as_os_str().is_empty() {
        return Ok(String::new());
    }
    run_traceback_via_file(&bin, source)
}

fn run_pyrs_traceback_file(source: &str) -> Result<String, String> {
    let Some(bin) = pyrs_bin_path() else {
        return Err("pyrs binary not found".to_string());
    };
    run_traceback_via_file(&bin, source)
}

fn traceback_heading_count(text: &str) -> usize {
    text.matches("Traceback (most recent call last):").count()
}

fn traceback_lines_without_source_carets(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                return Some(String::new());
            }
            let stripped = trimmed.trim_start();
            if !stripped.is_empty() && stripped.chars().all(|ch| ch == '^' || ch == '~') {
                return None;
            }
            if line.starts_with("    ") && !line.trim_start().starts_with("File ") {
                return None;
            }
            Some(trimmed.to_string())
        })
        .collect()
}

fn caret_line_after_source(text: &str, source_line: &str) -> Option<String> {
    let lines = text.lines().collect::<Vec<_>>();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim_end() == source_line
            && let Some(next) = lines.get(idx + 1)
            && next.trim_start().chars().all(|ch| ch == '^' || ch == '~')
        {
            return Some(next.trim().to_string());
        }
    }
    None
}

fn normalize_jsonish(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect()
}

#[test]
fn differential_corpus_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let corpus = [
        "result = 2 + 3 * 4",
        "result = [x * x for x in [1, 2, 3] if x > 1]",
        "result = {str(x): x + 1 for x in [1, 2, 3]}",
        "class A:\n    def __init__(self, x):\n        self.x = x\n    def inc(self):\n        self.x += 1\n        return self.x\nobj = A(4)\nresult = obj.inc()",
        "def f(a, b=3, *, c=4):\n    return a + b + c\nresult = f(1, c=5)",
        "import asyncio\nasync def worker(x):\n    return x + 1\nasync def main():\n    vals = await asyncio.gather(worker(1), worker(2), worker(3))\n    return sum(vals)\nresult = asyncio.run(main())",
    ];

    for source in corpus {
        let py = run_cpython_json(source).expect("CPython should run");
        let ours = run_pyrs_json(source).expect("pyrs should run");
        assert_eq!(
            normalize_jsonish(&py),
            normalize_jsonish(&ours),
            "differential mismatch for source:\n{source}"
        );
    }
}

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    FloorDiv,
    Mod,
}

enum Expr {
    Lit(i64),
    Bin(Box<Expr>, Op, Box<Expr>),
}

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 32) as u32
    }

    fn gen_range(&mut self, max: u32) -> u32 {
        if max == 0 {
            return 0;
        }
        self.next_u32() % max
    }

    fn gen_i64(&mut self, min: i64, max: i64) -> i64 {
        let span = (max - min + 1) as u64;
        min + (self.next_u32() as u64 % span) as i64
    }
}

fn gen_expr(rng: &mut Rng, depth: usize) -> Expr {
    if depth == 0 || rng.gen_range(4) == 0 {
        return Expr::Lit(rng.gen_i64(-30, 30));
    }
    let left = gen_expr(rng, depth - 1);
    let op = match rng.gen_range(5) {
        0 => Op::Add,
        1 => Op::Sub,
        2 => Op::Mul,
        3 => Op::FloorDiv,
        _ => Op::Mod,
    };
    let right = match op {
        Op::FloorDiv | Op::Mod => Expr::Lit(rng.gen_i64(1, 30)),
        _ => gen_expr(rng, depth - 1),
    };
    Expr::Bin(Box::new(left), op, Box::new(right))
}

fn expr_to_source(expr: &Expr) -> String {
    match expr {
        Expr::Lit(value) => {
            if *value < 0 {
                format!("({value})")
            } else {
                value.to_string()
            }
        }
        Expr::Bin(left, op, right) => {
            let symbol = match op {
                Op::Add => "+",
                Op::Sub => "-",
                Op::Mul => "*",
                Op::FloorDiv => "//",
                Op::Mod => "%",
            };
            format!(
                "({} {} {})",
                expr_to_source(left),
                symbol,
                expr_to_source(right)
            )
        }
    }
}

#[test]
fn differential_arithmetic_fuzz_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let mut rng = Rng::new(0xA11CE);
    for _ in 0..200 {
        let expr = gen_expr(&mut rng, 4);
        let source = format!("result = {}", expr_to_source(&expr));
        let py = run_cpython_json(&source).expect("CPython should run fuzz expression");
        let ours = run_pyrs_json(&source).expect("pyrs should run fuzz expression");
        assert_eq!(
            normalize_jsonish(&py),
            normalize_jsonish(&ours),
            "arithmetic differential mismatch for expression {}",
            expr_to_source(&expr)
        );
    }
}

#[test]
fn differential_list_sort_mutation_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"values = [3, 2, 1]
def keyf(x):
    values.append(0)
    return x
caught = None
try:
    values.sort(key=keyf)
except Exception as exc:
    caught = [type(exc).__name__, 'list modified during sort' in str(exc)]
result = {"values": values, "caught": caught}
"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(&py),
        normalize_jsonish(&ours),
        "list.sort mutation differential mismatch"
    );
}

#[test]
fn differential_json_options_and_default_callback_match_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"import json
class Unknown:
    pass
def fallback(value):
    return {"kind": value.__class__.__name__, "emoji": "☺"}
result = {
    "sorted": json.dumps({"b": 1, "a": "☺"}, sort_keys=True, separators=(",", ":"), ensure_ascii=True),
    "fallback": json.dumps(Unknown(), default=fallback, sort_keys=True, separators=(",", ":"), ensure_ascii=True),
    "loaded": json.loads('{"x": 1, "arr": [2, 3]}'),
}
"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(&py),
        normalize_jsonish(&ours),
        "json options/default differential mismatch"
    );
}

#[test]
fn differential_container_semantics_match_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"caught = False
try:
    d = {}
    d[[1]] = 1
except Exception:
    caught = True
d1 = {"a": 1, "b": 2}
d2 = {"b": 2, "a": 1}
s1 = {1, 2, 3}
s2 = {3, 2, 1}
result = {
    "caught": caught,
    "dict_eq": (d1 == d2),
    "set_eq": (s1 == s2),
    "contains": ("a" in d1 and 2 in s1 and 99 not in s1),
}
"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(&py),
        normalize_jsonish(&ours),
        "container semantics differential mismatch"
    );
}

#[test]
fn differential_json_malformed_input_contract_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"import json
cases = [
    '{"a": 1,,}',
    '{"a": [1 2]}',
    '"\\x20"',
    '{"a": "unterminated}',
]
raised = []
for payload in cases:
    failed = False
    try:
        json.loads(payload)
    except Exception:
        failed = True
    raised.append(failed)
result = raised
"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(&py),
        normalize_jsonish(&ours),
        "json malformed-input contract differential mismatch"
    );
}

#[test]
fn differential_csv_malformed_input_contract_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"import _csv
cases = [
    ('a,b', False),
    ('"unterminated', True),
    ('a,"b', True),
]
raised = []
for text, strict in cases:
    failed = False
    try:
        list(_csv.reader([text], strict=strict))
    except Exception:
        failed = True
    raised.append(failed)
result = raised
"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(&py),
        normalize_jsonish(&ours),
        "csv malformed-input contract differential mismatch"
    );
}

#[test]
fn differential_pickle_object_protocol_malformed_contract_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"cases = [
    lambda: object.__reduce_ex__(),
    lambda: object.__reduce_ex__(object(), "bad"),
    lambda: object.__reduce_ex__(object(), 4, 5),
]
raised = []
for fn in cases:
    failed = False
    try:
        fn()
    except Exception:
        failed = True
    raised.append(failed)
result = raised
"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(&py),
        normalize_jsonish(&ours),
        "pickle object-protocol malformed-input contract differential mismatch"
    );
}

#[test]
fn differential_traceback_context_chain_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    raise AttributeError("one")
except Exception:
    raise AttributeError("two")
"#;
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert_eq!(traceback_heading_count(&py), 2, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 2, "{}", ours);
    assert!(
        py.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        py
    );
    assert!(
        ours.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        ours
    );
    assert!(
        py.contains("AttributeError: one") && py.contains("AttributeError: two"),
        "{}",
        py
    );
    assert!(
        ours.contains("AttributeError: one") && ours.contains("AttributeError: two"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "context traceback shape mismatch"
    );
}

#[test]
fn differential_traceback_direct_cause_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    raise ValueError("inner")
except Exception as exc:
    raise RuntimeError("outer") from exc
"#;
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert_eq!(traceback_heading_count(&py), 2, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 2, "{}", ours);
    assert!(
        py.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        py
    );
    assert!(
        ours.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        ours
    );
    assert!(
        py.contains("ValueError: inner") && py.contains("RuntimeError: outer"),
        "{}",
        py
    );
    assert!(
        ours.contains("ValueError: inner") && ours.contains("RuntimeError: outer"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "direct-cause traceback shape mismatch"
    );
}

#[test]
fn differential_traceback_identifier_caret_span_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "x = foo";
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert!(py.contains("NameError"), "{}", py);
    assert!(ours.contains("NameError"), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    x = foo").expect("python caret");
    let ours_caret = caret_line_after_source(&ours, "    x = foo").expect("pyrs caret");
    assert_eq!(py_caret, ours_caret, "identifier caret mismatch");
}

#[test]
fn differential_syntax_error_shape_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "x =";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(!py.contains("Traceback (most recent call last):"), "{}", py);
    assert!(!ours.contains("Traceback (most recent call last):"), "{}", ours);
    assert!(py.contains("File \"<string>\", line 1"), "{}", py);
    assert!(ours.contains("File \"<string>\", line 1"), "{}", ours);
    assert!(py.contains("\n    x =\n"), "{}", py);
    assert!(ours.contains("\n    x =\n"), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    x =").expect("python syntax caret");
    let ours_caret = caret_line_after_source(&ours, "    x =").expect("pyrs syntax caret");
    assert_eq!(py_caret, ours_caret, "syntax caret mismatch");
    assert!(py.contains("SyntaxError:"), "{}", py);
    assert!(ours.contains("SyntaxError:"), "{}", ours);
}

#[test]
fn differential_invalid_syntax_span_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "if True print(1)";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(py.contains("SyntaxError:"), "{}", py);
    assert!(ours.contains("SyntaxError:"), "{}", ours);
    let py_caret =
        caret_line_after_source(&py, "    if True print(1)").expect("python invalid-syntax caret");
    let ours_caret =
        caret_line_after_source(&ours, "    if True print(1)").expect("pyrs invalid-syntax caret");
    assert_eq!(py_caret, ours_caret, "invalid-syntax caret mismatch");
}

#[test]
fn differential_unclosed_delimiter_shape_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f(";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(py.contains("was never closed"), "{}", py);
    assert!(ours.contains("was never closed"), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    def f(").expect("python unclosed caret");
    let ours_caret = caret_line_after_source(&ours, "    def f(").expect("pyrs unclosed caret");
    assert_eq!(py_caret, ours_caret, "unclosed-delimiter caret mismatch");
}

#[test]
fn differential_indentation_error_shape_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "if True:\nprint(1)";
    let py = run_cpython_traceback(source).expect("CPython indentation error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs indentation error should run");
    assert!(py.contains("IndentationError:"), "{}", py);
    assert!(ours.contains("IndentationError:"), "{}", ours);
    let py_caret =
        caret_line_after_source(&py, "    print(1)").expect("python indentation caret");
    let ours_caret =
        caret_line_after_source(&ours, "    print(1)").expect("pyrs indentation caret");
    assert_eq!(py_caret, ours_caret, "indentation caret mismatch");
}

#[test]
fn differential_unmatched_closing_delimiter_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "]";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(py.contains("SyntaxError: unmatched ']'"), "{}", py);
    assert!(ours.contains("SyntaxError: unmatched ']'"), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    ]").expect("python unmatched caret");
    let ours_caret = caret_line_after_source(&ours, "    ]").expect("pyrs unmatched caret");
    assert_eq!(py_caret, ours_caret, "unmatched closing-delimiter caret mismatch");
}

#[test]
fn differential_mismatched_closing_delimiter_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "([)]";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    let expected = "closing parenthesis ')' does not match opening parenthesis '['";
    assert!(py.contains(expected), "{}", py);
    assert!(ours.contains(expected), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    ([)]").expect("python mismatch caret");
    let ours_caret = caret_line_after_source(&ours, "    ([)]").expect("pyrs mismatch caret");
    assert_eq!(py_caret, ours_caret, "mismatched closing-delimiter caret mismatch");
}

#[test]
fn differential_unterminated_triple_quoted_string_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "x = \"\"\"a\nb";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    let expected = "unterminated triple-quoted string literal (detected at line 2)";
    assert!(py.contains(expected), "{}", py);
    assert!(ours.contains(expected), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    x = \"\"\"a").expect("python triple caret");
    let ours_caret = caret_line_after_source(&ours, "    x = \"\"\"a").expect("pyrs triple caret");
    assert_eq!(py_caret, ours_caret, "triple-quote caret mismatch");
}

#[test]
fn differential_unexpected_indent_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "    x = 1\n";
    let py = run_cpython_traceback_file(source).expect("CPython indentation error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs indentation error should run");
    assert!(py.contains("IndentationError: unexpected indent"), "{}", py);
    assert!(ours.contains("IndentationError: unexpected indent"), "{}", ours);
    assert!(!py.contains("^\n"), "{}", py);
    assert!(!ours.contains("^\n"), "{}", ours);
}

#[test]
fn differential_unindent_mismatch_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "if True:\n    if True:\n        pass\n  pass\n";
    let py = run_cpython_traceback_file(source).expect("CPython indentation error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs indentation error should run");
    let expected = "IndentationError: unindent does not match any outer indentation level";
    assert!(py.contains(expected), "{}", py);
    assert!(ours.contains(expected), "{}", ours);
    let py_caret = caret_line_after_source(&py, "      pass").or_else(|| caret_line_after_source(&py, "    pass"));
    let ours_caret =
        caret_line_after_source(&ours, "      pass").or_else(|| caret_line_after_source(&ours, "    pass"));
    assert_eq!(py_caret, ours_caret, "unindent-mismatch caret mismatch");
}

#[test]
fn differential_class_header_colon_inside_unclosed_paren_is_invalid_syntax() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "class A(:\n    pass\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(py.contains("SyntaxError: invalid syntax"), "{}", py);
    assert!(ours.contains("SyntaxError: invalid syntax"), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    class A(:").expect("python class-header caret");
    let ours_caret = caret_line_after_source(&ours, "    class A(:").expect("pyrs class-header caret");
    assert_eq!(py_caret, ours_caret, "class-header caret mismatch");
}

#[test]
fn differential_function_header_colon_inside_unclosed_paren_is_invalid_syntax() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f(:\n    pass\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(py.contains("SyntaxError: invalid syntax"), "{}", py);
    assert!(ours.contains("SyntaxError: invalid syntax"), "{}", ours);
    let py_caret =
        caret_line_after_source(&py, "    def f(:").expect("python function-header caret");
    let ours_caret =
        caret_line_after_source(&ours, "    def f(:").expect("pyrs function-header caret");
    assert_eq!(py_caret, ours_caret, "function-header caret mismatch");
}

#[test]
fn differential_open_bracket_with_colon_is_invalid_syntax() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "[1:";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(py.contains("SyntaxError: invalid syntax"), "{}", py);
    assert!(ours.contains("SyntaxError: invalid syntax"), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    [1:").expect("python list-colon caret");
    let ours_caret = caret_line_after_source(&ours, "    [1:").expect("pyrs list-colon caret");
    assert_eq!(py_caret, ours_caret, "list-colon caret mismatch");
}

#[test]
fn differential_traceback_suppressed_context_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    raise ValueError("inner")
except Exception:
    raise RuntimeError("outer") from None
"#;
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert_eq!(traceback_heading_count(&py), 1, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 1, "{}", ours);
    assert!(
        !py.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        py
    );
    assert!(
        !ours.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        ours
    );
    assert!(
        !py.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        py
    );
    assert!(
        !ours.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        ours
    );
    assert!(py.contains("RuntimeError: outer"), "{}", py);
    assert!(ours.contains("RuntimeError: outer"), "{}", ours);
    assert!(!py.contains("ValueError: inner"), "{}", py);
    assert!(!ours.contains("ValueError: inner"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "suppressed-context traceback shape mismatch"
    );
}

#[test]
fn differential_pyc_traceback_identifier_caret_span_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "x = foo\n";
    let (base, pyc_path) =
        compile_temp_pyc(source, "traceback_nameerror").expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours = run_traceback_via_pyc_file(
        &pyrs_bin_path().expect("pyrs binary not found"),
        &pyc_path,
    )
    .expect("pyrs .pyc traceback should run");
    assert!(py.contains("NameError"), "{}", py);
    assert!(ours.contains("NameError"), "{}", ours);
    let py_caret = caret_line_after_source(&py, "    x = foo").expect("python pyc caret");
    let ours_caret = caret_line_after_source(&ours, "    x = foo").expect("pyrs pyc caret");
    assert_eq!(py_caret, ours_caret, "pyc NameError caret mismatch");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn differential_pyc_traceback_context_chain_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    raise ValueError("inner")
except Exception:
    raise RuntimeError("outer")
"#;
    let (base, pyc_path) =
        compile_temp_pyc(source, "traceback_context_chain").expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours = run_traceback_via_pyc_file(
        &pyrs_bin_path().expect("pyrs binary not found"),
        &pyc_path,
    )
    .expect("pyrs .pyc traceback should run");
    assert_eq!(traceback_heading_count(&py), 2, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 2, "{}", ours);
    assert!(
        py.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        py
    );
    assert!(
        ours.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        ours
    );
    assert!(py.contains("ValueError: inner"), "{}", py);
    assert!(py.contains("RuntimeError: outer"), "{}", py);
    assert!(ours.contains("ValueError: inner"), "{}", ours);
    assert!(ours.contains("RuntimeError: outer"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "pyc context traceback shape mismatch"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn differential_pyc_traceback_suppressed_context_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    raise ValueError("inner")
except Exception:
    raise RuntimeError("outer") from None
"#;
    let (base, pyc_path) = compile_temp_pyc(source, "traceback_suppressed_context_pyc")
        .expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours = run_traceback_via_pyc_file(
        &pyrs_bin_path().expect("pyrs binary not found"),
        &pyc_path,
    )
    .expect("pyrs .pyc traceback should run");
    assert_eq!(traceback_heading_count(&py), 1, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 1, "{}", ours);
    assert!(
        !py.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        py
    );
    assert!(
        !ours.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        ours
    );
    assert!(
        !py.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        py
    );
    assert!(
        !ours.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        ours
    );
    assert!(py.contains("RuntimeError: outer"), "{}", py);
    assert!(ours.contains("RuntimeError: outer"), "{}", ours);
    assert!(!py.contains("ValueError: inner"), "{}", py);
    assert!(!ours.contains("ValueError: inner"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "pyc suppressed-context traceback shape mismatch"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn differential_pyc_traceback_direct_cause_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    raise ValueError("inner")
except Exception as exc:
    raise RuntimeError("outer") from exc
"#;
    let (base, pyc_path) =
        compile_temp_pyc(source, "traceback_direct_cause_pyc").expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours = run_traceback_via_pyc_file(
        &pyrs_bin_path().expect("pyrs binary not found"),
        &pyc_path,
    )
    .expect("pyrs .pyc traceback should run");
    assert_eq!(traceback_heading_count(&py), 2, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 2, "{}", ours);
    assert!(
        py.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        py
    );
    assert!(
        ours.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        ours
    );
    assert!(py.contains("ValueError: inner"), "{}", py);
    assert!(py.contains("RuntimeError: outer"), "{}", py);
    assert!(ours.contains("ValueError: inner"), "{}", ours);
    assert!(ours.contains("RuntimeError: outer"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "pyc direct-cause traceback shape mismatch"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn differential_traceback_mixed_cause_and_context_chain_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    try:
        raise KeyError("k")
    except Exception as inner:
        raise ValueError("v") from inner
except Exception:
    raise RuntimeError("r")
"#;
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert_eq!(traceback_heading_count(&py), 3, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 3, "{}", ours);
    assert!(
        py.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        py
    );
    assert!(
        ours.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        ours
    );
    assert!(
        py.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        py
    );
    assert!(
        ours.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        ours
    );
    assert!(py.contains("KeyError:"), "{}", py);
    assert!(py.contains("ValueError: v"), "{}", py);
    assert!(py.contains("RuntimeError: r"), "{}", py);
    assert!(ours.contains("KeyError:"), "{}", ours);
    assert!(ours.contains("ValueError: v"), "{}", ours);
    assert!(ours.contains("RuntimeError: r"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "mixed cause/context traceback shape mismatch"
    );
}

#[test]
fn differential_pyc_traceback_mixed_cause_and_context_chain_matches_cpython_shape() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    try:
        raise KeyError("k")
    except Exception as inner:
        raise ValueError("v") from inner
except Exception:
    raise RuntimeError("r")
"#;
    let (base, pyc_path) = compile_temp_pyc(source, "traceback_mixed_chain_pyc")
        .expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours = run_traceback_via_pyc_file(
        &pyrs_bin_path().expect("pyrs binary not found"),
        &pyc_path,
    )
    .expect("pyrs .pyc traceback should run");
    assert_eq!(traceback_heading_count(&py), 3, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 3, "{}", ours);
    assert!(
        py.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        py
    );
    assert!(
        ours.contains("The above exception was the direct cause of the following exception:"),
        "{}",
        ours
    );
    assert!(
        py.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        py
    );
    assert!(
        ours.contains("During handling of the above exception, another exception occurred:"),
        "{}",
        ours
    );
    assert!(py.contains("KeyError:"), "{}", py);
    assert!(py.contains("ValueError: v"), "{}", py);
    assert!(py.contains("RuntimeError: r"), "{}", py);
    assert!(ours.contains("KeyError:"), "{}", ours);
    assert!(ours.contains("ValueError: v"), "{}", ours);
    assert!(ours.contains("RuntimeError: r"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "pyc mixed cause/context traceback shape mismatch"
    );
    let _ = std::fs::remove_dir_all(&base);
}
