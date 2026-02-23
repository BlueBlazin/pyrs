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
    std::fs::write(&py_path, source)
        .map_err(|err| format!("failed to write temp source: {err}"))?;
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
            if trimmed.starts_with("  File \"")
                && let Some(rest) = trimmed.split_once("\", line ").map(|(_, rest)| rest)
            {
                return Some(format!("  File \"<FILE>\", line {rest}"));
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
    assert!(
        !ours.contains("Traceback (most recent call last):"),
        "{}",
        ours
    );
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
    let py_caret = caret_line_after_source(&py, "    print(1)").expect("python indentation caret");
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
    assert_eq!(
        py_caret, ours_caret,
        "unmatched closing-delimiter caret mismatch"
    );
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
    assert_eq!(
        py_caret, ours_caret,
        "mismatched closing-delimiter caret mismatch"
    );
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
    assert!(
        ours.contains("IndentationError: unexpected indent"),
        "{}",
        ours
    );
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
    let py_caret = caret_line_after_source(&py, "      pass")
        .or_else(|| caret_line_after_source(&py, "    pass"));
    let ours_caret = caret_line_after_source(&ours, "      pass")
        .or_else(|| caret_line_after_source(&ours, "    pass"));
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
    let py_caret =
        caret_line_after_source(&py, "    class A(:").expect("python class-header caret");
    let ours_caret =
        caret_line_after_source(&ours, "    class A(:").expect("pyrs class-header caret");
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
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
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
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
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
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
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
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
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
    let (base, pyc_path) =
        compile_temp_pyc(source, "traceback_mixed_chain_pyc").expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
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

#[test]
fn differential_traceback_reraise_preserves_original_fault_line() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"def boom():
    try:
        1 / 0
    except Exception:
        raise
    finally:
        sentinel = 1

boom()
"#;
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert_eq!(traceback_heading_count(&py), 1, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 1, "{}", ours);
    assert!(py.contains("ZeroDivisionError"), "{}", py);
    assert!(ours.contains("ZeroDivisionError"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "reraise traceback shape mismatch"
    );
}

#[test]
fn differential_pyc_traceback_reraise_preserves_original_fault_line() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"def boom():
    try:
        1 / 0
    except Exception:
        raise
    finally:
        sentinel = 1

boom()
"#;
    let (base, pyc_path) =
        compile_temp_pyc(source, "traceback_reraise_pyc").expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
            .expect("pyrs .pyc traceback should run");
    assert_eq!(traceback_heading_count(&py), 1, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 1, "{}", ours);
    assert!(py.contains("ZeroDivisionError"), "{}", py);
    assert!(ours.contains("ZeroDivisionError"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "pyc reraise traceback shape mismatch"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn differential_traceback_raise_exc_keeps_original_traceback_chain() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"def boom():
    try:
        1 / 0
    except Exception as exc:
        raise exc

boom()
"#;
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert_eq!(traceback_heading_count(&py), 1, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 1, "{}", ours);
    assert!(py.contains("ZeroDivisionError"), "{}", py);
    assert!(ours.contains("ZeroDivisionError"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "raise-exc traceback shape mismatch"
    );
}

#[test]
fn differential_pyc_traceback_raise_exc_keeps_original_traceback_chain() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"def boom():
    try:
        1 / 0
    except Exception as exc:
        raise exc

boom()
"#;
    let (base, pyc_path) =
        compile_temp_pyc(source, "traceback_raise_exc_pyc").expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
            .expect("pyrs .pyc traceback should run");
    assert_eq!(traceback_heading_count(&py), 1, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 1, "{}", ours);
    assert!(py.contains("ZeroDivisionError"), "{}", py);
    assert!(ours.contains("ZeroDivisionError"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "pyc raise-exc traceback shape mismatch"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn differential_traceback_with_traceback_restores_supplied_chain() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    1 / 0
except Exception as exc:
    tb = exc.__traceback__
    raise RuntimeError("x").with_traceback(tb)
"#;
    let py = run_cpython_traceback(source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(source).expect("pyrs traceback should run");
    assert_eq!(traceback_heading_count(&py), 2, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 2, "{}", ours);
    assert!(py.contains("ZeroDivisionError"), "{}", py);
    assert!(ours.contains("ZeroDivisionError"), "{}", ours);
    assert!(py.contains("RuntimeError: x"), "{}", py);
    assert!(ours.contains("RuntimeError: x"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "with_traceback traceback shape mismatch"
    );
}

#[test]
fn differential_pyc_traceback_with_traceback_restores_supplied_chain() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"try:
    1 / 0
except Exception as exc:
    tb = exc.__traceback__
    raise RuntimeError("x").with_traceback(tb)
"#;
    let (base, pyc_path) = compile_temp_pyc(source, "traceback_with_traceback_pyc")
        .expect("compile pyc should succeed");
    let py = run_traceback_via_pyc_file(&cpython_bin_or_panic(), &pyc_path)
        .expect("CPython .pyc traceback should run");
    let ours =
        run_traceback_via_pyc_file(&pyrs_bin_path().expect("pyrs binary not found"), &pyc_path)
            .expect("pyrs .pyc traceback should run");
    assert_eq!(traceback_heading_count(&py), 2, "{}", py);
    assert_eq!(traceback_heading_count(&ours), 2, "{}", ours);
    assert!(py.contains("ZeroDivisionError"), "{}", py);
    assert!(ours.contains("ZeroDivisionError"), "{}", ours);
    assert!(py.contains("RuntimeError: x"), "{}", py);
    assert!(ours.contains("RuntimeError: x"), "{}", ours);
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "pyc with_traceback traceback shape mismatch"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn differential_semantic_syntax_return_outside_function_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "return";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(!py.contains("Traceback (most recent call last):"), "{}", py);
    assert!(
        !ours.contains("Traceback (most recent call last):"),
        "{}",
        ours
    );
    assert!(
        py.contains("SyntaxError: 'return' outside function"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: 'return' outside function"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic return-outside-function syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_break_outside_loop_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "break";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(!py.contains("Traceback (most recent call last):"), "{}", py);
    assert!(
        !ours.contains("Traceback (most recent call last):"),
        "{}",
        ours
    );
    assert!(py.contains("SyntaxError: 'break' outside loop"), "{}", py);
    assert!(
        ours.contains("SyntaxError: 'break' outside loop"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic break-outside-loop syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_continue_outside_loop_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "continue";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(!py.contains("Traceback (most recent call last):"), "{}", py);
    assert!(
        !ours.contains("Traceback (most recent call last):"),
        "{}",
        ours
    );
    assert!(
        py.contains("SyntaxError: 'continue' not properly in loop"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: 'continue' not properly in loop"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic continue-outside-loop syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_await_outside_function_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "await 1";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(!py.contains("Traceback (most recent call last):"), "{}", py);
    assert!(
        !ours.contains("Traceback (most recent call last):"),
        "{}",
        ours
    );
    assert!(
        py.contains("SyntaxError: 'await' outside function"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: 'await' outside function"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic await-outside-function syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_yield_outside_function_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "yield 1";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(!py.contains("Traceback (most recent call last):"), "{}", py);
    assert!(
        !ours.contains("Traceback (most recent call last):"),
        "{}",
        ours
    );
    assert!(
        py.contains("SyntaxError: 'yield' outside function"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: 'yield' outside function"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic yield-outside-function syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_yield_from_outside_function_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "yield from []";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(!py.contains("Traceback (most recent call last):"), "{}", py);
    assert!(
        !ours.contains("Traceback (most recent call last):"),
        "{}",
        ours
    );
    assert!(
        py.contains("SyntaxError: 'yield from' outside function"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: 'yield from' outside function"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic yield-from-outside-function syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_return_with_value_in_async_generator_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "async def f():\n    yield 1\n    return 2\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: 'return' with value in async generator"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: 'return' with value in async generator"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic async-generator return syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_global_used_prior_declaration_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f():\n    print(x)\n    global x\n";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: name 'x' is used prior to global declaration"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: name 'x' is used prior to global declaration"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic global-used-prior syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_global_assigned_prior_declaration_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f():\n    x += 1\n    global x\n";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: name 'x' is assigned to before global declaration"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: name 'x' is assigned to before global declaration"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic global-assigned-prior syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_nonlocal_at_module_level_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "nonlocal x";
    let py = run_cpython_traceback(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: nonlocal declaration not allowed at module level"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: nonlocal declaration not allowed at module level"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic nonlocal-module syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_nonlocal_without_binding_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f():\n    nonlocal x\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: no binding for nonlocal 'x' found"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: no binding for nonlocal 'x' found"),
        "{}",
        ours
    );
    let py_caret =
        caret_line_after_source(&py, "    nonlocal x").expect("python nonlocal-binding caret");
    let ours_caret =
        caret_line_after_source(&ours, "    nonlocal x").expect("pyrs nonlocal-binding caret");
    assert_eq!(py_caret, ours_caret, "nonlocal-binding caret mismatch");
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic nonlocal-binding syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_global_used_prior_declaration_file_caret_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f():\n    print(x)\n    global x\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: name 'x' is used prior to global declaration"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: name 'x' is used prior to global declaration"),
        "{}",
        ours
    );
    let py_caret = caret_line_after_source(&py, "    global x").expect("python global caret");
    let ours_caret = caret_line_after_source(&ours, "    global x").expect("pyrs global caret");
    assert_eq!(py_caret, ours_caret, "global-used-prior caret mismatch");
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic global-used-prior file syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_parameter_and_global_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f(x):\n    global x\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: name 'x' is parameter and global"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: name 'x' is parameter and global"),
        "{}",
        ours
    );
    let py_caret = caret_line_after_source(&py, "    global x").expect("python param/global caret");
    let ours_caret =
        caret_line_after_source(&ours, "    global x").expect("pyrs param/global caret");
    assert_eq!(py_caret, ours_caret, "parameter/global caret mismatch");
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic parameter/global syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_parameter_and_nonlocal_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f(x):\n    nonlocal x\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: name 'x' is parameter and nonlocal"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: name 'x' is parameter and nonlocal"),
        "{}",
        ours
    );
    let py_caret =
        caret_line_after_source(&py, "    nonlocal x").expect("python param/nonlocal caret");
    let ours_caret =
        caret_line_after_source(&ours, "    nonlocal x").expect("pyrs param/nonlocal caret");
    assert_eq!(py_caret, ours_caret, "parameter/nonlocal caret mismatch");
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic parameter/nonlocal syntax mismatch"
    );
}

#[test]
fn differential_semantic_syntax_nonlocal_and_global_conflict_global_first_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f():\n    global x\n    nonlocal x\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: name 'x' is nonlocal and global"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: name 'x' is nonlocal and global"),
        "{}",
        ours
    );
    let py_caret =
        caret_line_after_source(&py, "    global x").expect("python nonlocal/global caret");
    let ours_caret =
        caret_line_after_source(&ours, "    global x").expect("pyrs nonlocal/global caret");
    assert_eq!(
        py_caret, ours_caret,
        "nonlocal/global (global first) caret mismatch"
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic nonlocal/global (global first) mismatch"
    );
}

#[test]
fn differential_semantic_syntax_nonlocal_and_global_conflict_nonlocal_first_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = "def f():\n    nonlocal x\n    global x\n";
    let py = run_cpython_traceback_file(source).expect("CPython syntax error should run");
    let ours = run_pyrs_traceback_file(source).expect("pyrs syntax error should run");
    assert!(
        py.contains("SyntaxError: name 'x' is nonlocal and global"),
        "{}",
        py
    );
    assert!(
        ours.contains("SyntaxError: name 'x' is nonlocal and global"),
        "{}",
        ours
    );
    let py_caret =
        caret_line_after_source(&py, "    nonlocal x").expect("python nonlocal/global caret");
    let ours_caret =
        caret_line_after_source(&ours, "    nonlocal x").expect("pyrs nonlocal/global caret");
    assert_eq!(
        py_caret, ours_caret,
        "nonlocal/global (nonlocal first) caret mismatch"
    );
    assert_eq!(
        traceback_lines_without_source_carets(&py),
        traceback_lines_without_source_carets(&ours),
        "semantic nonlocal/global (nonlocal first) mismatch"
    );
}

#[test]
fn differential_from_import_reads_attribute_from_replaced_sys_modules_entry() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let module_name = format!("_pyrs_diff_swapmod_{unique}");
    let source = format!(
        r#"
import os
import sys

name = "{module_name}"
path = name + ".py"
with open(path, "w", encoding="utf-8") as handle:
    handle.write("import sys as _sys\n_sys.modules[__name__] = _sys\n")
try:
    sys.path.insert(0, ".")
    from {module_name} import version as v
    import {module_name} as swapmod
    result = (swapmod is sys) and (v == sys.version)
finally:
    try:
        os.remove(path)
    except FileNotFoundError:
        pass
"#
    );
    let py = run_cpython_json(&source).expect("CPython run should succeed");
    let ours = run_pyrs_json(&source).expect("pyrs run should succeed");
    assert_eq!(py, ours, "from-import sys.modules replacement mismatch");
}

#[test]
fn differential_from_import_star_missing_all_entry_raises_attribute_error() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time works")
        .as_nanos();
    let module_name = format!("_pyrs_diff_mod_{unique}");
    let source = format!(
        r#"
import os
import sys

name = "{module_name}"
path = name + ".py"
with open(path, "w", encoding="utf-8") as handle:
    handle.write("__all__ = ['missing']\n")
try:
    sys.path.insert(0, ".")
    from {module_name} import *
finally:
    try:
        os.remove(path)
    except FileNotFoundError:
        pass
"#
    );
    let py = run_cpython_traceback(&source).expect("CPython traceback should run");
    let ours = run_pyrs_traceback(&source).expect("pyrs traceback should run");
    assert!(
        py.contains("AttributeError: module") && py.contains("has no attribute 'missing'"),
        "{}",
        py
    );
    assert!(
        ours.contains("AttributeError: module") && ours.contains("has no attribute 'missing'"),
        "{}",
        ours
    );
    assert_eq!(
        traceback_heading_count(&py),
        traceback_heading_count(&ours),
        "from-import-* traceback block count mismatch"
    );
    assert_eq!(
        py.lines().last().map(str::trim),
        ours.lines().last().map(str::trim),
        "from-import-* exception footer mismatch"
    );
}

#[test]
fn differential_compile_only_ast_assign_fields_and_match_args() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
node = compile("x = f()", "<ast>", "exec", _ast.PyCF_ONLY_AST)
stmt = node.body[0]
result = {
    "fields": list(stmt._fields),
    "match_args": list(stmt.__match_args__),
    "stmt_type": type(stmt).__name__,
    "is_stmt": isinstance(stmt, _ast.stmt),
    "value_type": type(stmt.value).__name__,
    "value_is_expr": isinstance(stmt.value, _ast.expr),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_only_ast_operator_hierarchy_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
expr = compile("not a or b + c < d", "<ast>", "eval", _ast.PyCF_ONLY_AST).body
result = {
    "expr_type": type(expr).__name__,
    "boolop_is_base": isinstance(expr.op, _ast.boolop),
    "cmpop_is_base": isinstance(expr.values[1].ops[0], _ast.cmpop),
    "binop_is_base": isinstance(expr.values[1].left.op, _ast.operator),
    "unary_is_base": isinstance(expr.values[0].op, _ast.unaryop),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_only_ast_function_class_and_type_param_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
mod = compile("@dec\ndef f[T](x, /, y=2, *args, z, **kw):\n    return x\n\n@dec\nclass C[T](B, metaclass=M, y=1):\n    pass\n", "<ast>", "exec", _ast.PyCF_ONLY_AST)
fn = mod.body[0]
cls = mod.body[1]
result = {
    "fn_type": type(fn).__name__,
    "fn_fields": list(fn._fields),
    "fn_stmt_base": isinstance(fn, _ast.stmt),
    "fn_decorators": len(fn.decorator_list),
    "fn_args_type": type(fn.args).__name__,
    "fn_posonly": [a.arg for a in fn.args.posonlyargs],
    "fn_args": [a.arg for a in fn.args.args],
    "fn_vararg": None if fn.args.vararg is None else fn.args.vararg.arg,
    "fn_kwonly": [a.arg for a in fn.args.kwonlyargs],
    "fn_kwarg": None if fn.args.kwarg is None else fn.args.kwarg.arg,
    "fn_defaults_len": len(fn.args.defaults),
    "fn_type_params": [type(tp).__name__ for tp in fn.type_params],
    "fn_type_param_base": all(isinstance(tp, _ast.type_param) for tp in fn.type_params),
    "cls_type": type(cls).__name__,
    "cls_stmt_base": isinstance(cls, _ast.stmt),
    "cls_decorators": len(cls.decorator_list),
    "cls_keyword_args": [(k.arg, type(k.value).__name__) for k in cls.keywords],
    "cls_type_params": [type(tp).__name__ for tp in cls.type_params],
    "withitem_attrs": list(_ast.withitem._attributes),
    "arg_attrs": list(_ast.arg._attributes),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_only_ast_augassign_and_annassign_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
aug = compile("x += 1", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0]
ann = compile("x: int = 1", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0]
sub_ann = compile("obj.x: int", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0]
result = {
    "aug_type": type(aug).__name__,
    "aug_op_type": type(aug.op).__name__,
    "ann_type": type(ann).__name__,
    "ann_fields": list(ann._fields),
    "ann_simple": ann.simple,
    "ann_target_ctx": type(ann.target.ctx).__name__,
    "sub_ann_simple": sub_ann.simple,
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_only_ast_match_and_pattern_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
node = compile(
    "match subject:\n    case [1, *rest]:\n        pass\n    case {'k': v, **more}:\n        pass\n    case Point(x=px, y=py):\n        pass\n    case True:\n        pass\n    case capture if capture > 0:\n        pass\n    case _:\n        pass\n",
    "<ast>",
    "exec",
    _ast.PyCF_ONLY_AST,
)
match_node = node.body[0]
case_seq = match_node.cases[0]
case_map = match_node.cases[1]
case_class = match_node.cases[2]
case_singleton = match_node.cases[3]
case_guard = match_node.cases[4]
case_wild = match_node.cases[5]
result = {
    "match_type": type(match_node).__name__,
    "match_is_stmt": isinstance(match_node, _ast.stmt),
    "case_type": type(case_seq).__name__,
    "case_is_match_case": isinstance(case_seq, _ast.match_case),
    "pattern_base": isinstance(case_seq.pattern, _ast.pattern),
    "seq_pattern_type": type(case_seq.pattern).__name__,
    "seq_subpatterns": [type(p).__name__ for p in case_seq.pattern.patterns],
    "mapping_type": type(case_map.pattern).__name__,
    "mapping_rest": case_map.pattern.rest,
    "mapping_pattern_types": [type(p).__name__ for p in case_map.pattern.patterns],
    "class_type": type(case_class.pattern).__name__,
    "class_kwd_attrs": list(case_class.pattern.kwd_attrs),
    "class_kwd_pattern_types": [type(p).__name__ for p in case_class.pattern.kwd_patterns],
    "singleton_type": type(case_singleton.pattern).__name__,
    "guard_type": type(case_guard.guard).__name__,
    "guard_pattern_name": case_guard.pattern.name,
    "wild_type": type(case_wild.pattern).__name__,
    "wild_name": case_wild.pattern.name,
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_only_ast_alias_keyword_and_handler_location_attrs_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
imp = compile("import os as o", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0]
call = compile("f(x=1, **d)", "<ast>", "eval", _ast.PyCF_ONLY_AST).body
tr = compile("try:\n    pass\nexcept E as e:\n    pass", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0]
alias = imp.names[0]
kw0 = call.keywords[0]
kw1 = call.keywords[1]
handler = tr.handlers[0]
attrs = ["lineno", "col_offset", "end_lineno", "end_col_offset"]
result = {
    "alias_has_attrs": all(hasattr(alias, a) for a in attrs),
    "kw0_has_attrs": all(hasattr(kw0, a) for a in attrs),
    "kw1_has_attrs": all(hasattr(kw1, a) for a in attrs),
    "handler_has_attrs": all(hasattr(handler, a) for a in attrs),
    "alias_lineno": alias.lineno,
    "kw0_lineno": kw0.lineno,
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_only_ast_lambda_await_comprehension_and_yield_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
lam = compile("lambda x, /, y=2, *args, z, **kw: x + y", "<ast>", "eval", _ast.PyCF_ONLY_AST).body
lst = compile("[x async for x in xs if x > 0]", "<ast>", "eval", _ast.PyCF_ONLY_AST).body
dct = compile("{x: x + 1 for x in xs if x > 0}", "<ast>", "eval", _ast.PyCF_ONLY_AST).body
gen = compile("(x for x in xs if x > 0)", "<ast>", "eval", _ast.PyCF_ONLY_AST).body
await_node = compile("async def f():\n    return await g()\n", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0].body[0].value
yield_node = compile("def f():\n    yield 1\n", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0].body[0].value
yield_from_node = compile("def f():\n    yield from it\n", "<ast>", "exec", _ast.PyCF_ONLY_AST).body[0].body[0].value
result = {
    "lam_type": type(lam).__name__,
    "lam_args_type": type(lam.args).__name__,
    "lam_body_type": type(lam.body).__name__,
    "listcomp_type": type(lst).__name__,
    "listcomp_gen_type": type(lst.generators[0]).__name__,
    "listcomp_gen_base": isinstance(lst.generators[0], _ast.comprehension),
    "listcomp_is_async": lst.generators[0].is_async,
    "dictcomp_type": type(dct).__name__,
    "dictcomp_is_async": dct.generators[0].is_async,
    "genexp_type": type(gen).__name__,
    "await_type": type(await_node).__name__,
    "yield_type": type(yield_node).__name__,
    "yield_from_type": type(yield_from_node).__name__,
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_only_ast_type_param_kind_parity_for_star_and_doublestar() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
mod = compile("def f[T, *Ts, **P](x):\n    return x\nclass C[T, *Ts, **P]:\n    pass\n", "<ast>", "exec", _ast.PyCF_ONLY_AST)
fn = mod.body[0]
cls = mod.body[1]
result = {
    "fn_kind_names": [type(tp).__name__ for tp in fn.type_params],
    "cls_kind_names": [type(tp).__name__ for tp in cls.type_params],
    "fn_param_names": [tp.name for tp in fn.type_params],
    "cls_param_names": [tp.name for tp in cls.type_params],
    "fn_type_param_base": all(isinstance(tp, _ast.type_param) for tp in fn.type_params),
    "cls_type_param_base": all(isinstance(tp, _ast.type_param) for tp in cls.type_params),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}
