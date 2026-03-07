#![cfg(not(target_arch = "wasm32"))]

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

fn configure_traceback_subprocess(cmd: &mut Command) {
    // Differential traceback tests compare structure/content, not terminal styling.
    cmd.env("NO_COLOR", "1");
    cmd.env("PYTHON_COLORS", "0");
    cmd.env_remove("FORCE_COLOR");
    cmd.env_remove("CLICOLOR_FORCE");
    // Keep traceback probes deterministic across host virtualenv activation.
    cmd.env_remove("VIRTUAL_ENV");
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env_remove("PYTHONSTARTUP");
    cmd.env_remove("PYTHONNODEBUGRANGES");
}

fn strip_ansi_control_sequences(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&b) {
                        break;
                    }
                }
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            i += 1;
            continue;
        }
        let ch = text[i..].chars().next().expect("valid UTF-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
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

fn detect_cpython_lib_for_cli() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let candidate = PathBuf::from(path);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".local/Python-3.14.3/Lib");
    if local.is_dir() {
        return Some(local);
    }
    None
}

fn run_pyrs_cli_json(source: &str) -> Result<String, String> {
    let Some(bin) = pyrs_bin_path() else {
        return Err("pyrs binary not found".to_string());
    };
    let script = format!("{source}\nimport json\nprint(json.dumps(result))\n");
    let mut cmd = Command::new(bin);
    cmd.arg("-S").arg("-c").arg(script);
    if let Some(lib) = detect_cpython_lib_for_cli() {
        cmd.env("PYRS_CPYTHON_LIB", lib);
    }
    let output = cmd
        .output()
        .map_err(|err| format!("failed to launch pyrs: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "pyrs execution failed".to_string()
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn pyrs_bin_path() -> Option<PathBuf> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_pyrs") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_pyrs") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(debug_dir) = exe.parent().and_then(|p| p.parent())
    {
        let candidate = debug_dir.join("pyrs");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(target_dir).join("debug/pyrs");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let llvm_cov_candidate =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/llvm-cov-target/debug/pyrs");
    if llvm_cov_candidate.is_file() {
        return Some(llvm_cov_candidate);
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
    let mut cmd = Command::new(bin);
    configure_traceback_subprocess(&mut cmd);
    let output = cmd
        .arg("-S")
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
    let mut cmd = Command::new(bin);
    configure_traceback_subprocess(&mut cmd);
    let output = cmd
        .arg("-S")
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
    let mut cmd = Command::new(bin);
    configure_traceback_subprocess(&mut cmd);
    let output = cmd
        .arg("-S")
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
    let mut cmd = Command::new(bin);
    configure_traceback_subprocess(&mut cmd);
    let output = cmd
        .arg("-S")
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
            let plain = strip_ansi_control_sequences(line);
            let trimmed = plain.trim_end();
            if trimmed.is_empty() {
                return Some(String::new());
            }
            let stripped = trimmed.trim_start();
            if !stripped.is_empty() && stripped.chars().all(|ch| ch == '^' || ch == '~') {
                return None;
            }
            if plain.starts_with("    ") && !plain.trim_start().starts_with("File ") {
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
    let lines = text
        .lines()
        .map(strip_ansi_control_sequences)
        .collect::<Vec<_>>();
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

#[test]
fn differential_re_docs_core_examples_match_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"import re
result = {
    "findall_basic": re.compile(r"\bf[a-z]*").findall("which foot or hand fell fastest"),
    "finditer_spans": [(m.group(0), m.span()) for m in re.compile(r"\bf[a-z]*").finditer("which foot or hand fell fastest")],
    "findall_groups": re.compile(r"(\w+)=(\d+)").findall("a=1 b=20"),
    "group_backref": re.search(r"(?P<word>\w+)\s+(?P=word)", "the the").group(0),
    "group_number_backref": re.search(r"(\w+)\s+\1", "bye bye").group(0),
    "lookahead_pos": re.search(r"foo(?=bar)", "foobar").group(0),
    "lookahead_neg": re.search(r"foo(?!bar)", "foobaz").group(0),
    "lookbehind_pos": re.search(r"(?<=abc)def", "abcdef").group(0),
    "lookbehind_neg": re.search(r"(?<!abc)def", "xyzdef").group(0),
    "multiline_anchor": re.compile(r"^\w+", re.MULTILINE).findall("first\nsecond\nthird"),
    "non_greedy": re.search(r"<.*?>", "<a> b <c>").group(0),
    "sub_basic": re.compile(r"\sAND\s", re.IGNORECASE).sub(" & ", "Baked Beans And Spam"),
    "subn_basic": re.compile("x*").subn("-", "abxd"),
    "split_empty_match": re.compile(r"x*").split("abxd"),
    "unicode_w": re.compile(r"\w+").findall("abc ümlaut"),
}
"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_cli_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(&py),
        normalize_jsonish(&ours),
        "regex docs differential mismatch"
    );
}

#[test]
fn differential_re_bound_method_identity_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import re
pat = re.compile("")
m = pat.match("")
result = {
    "pattern_match_self": pat.match.__self__ is pat,
    "pattern_search_self": pat.search.__self__ is pat,
    "pattern_fullmatch_self": pat.fullmatch.__self__ is pat,
    "group_self": m.group.__self__ is m,
    "group_value": m.group(0),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_builtin_type_objects_are_truthy() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"result = [bool(t) for t in [bool, complex, dict, float, int, list, object, set, str, tuple, type]]"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(normalize_jsonish(&py), normalize_jsonish(&ours));
}

#[test]
fn differential_bytes_like_reversed_and_sequence_dunders_match_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"result = {
    "bytes_has_len": hasattr(b"", "__len__"),
    "bytes_has_getitem": hasattr(b"", "__getitem__"),
    "bytes_has_contains": hasattr(b"", "__contains__"),
    "bytearray_has_len": hasattr(bytearray(), "__len__"),
    "bytearray_has_getitem": hasattr(bytearray(), "__getitem__"),
    "bytearray_has_contains": hasattr(bytearray(), "__contains__"),
    "bytes_reversed": list(reversed(b"ab")),
    "bytearray_reversed": list(reversed(bytearray(b"ab"))),
    "bytes_getitem": b"ab".__getitem__(0),
    "bytearray_getitem": bytearray(b"ab").__getitem__(0),
    "bytes_contains": (b"a" in b"ab"),
    "bytearray_contains": (97 in bytearray(b"ab")),
}"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(normalize_jsonish(&py), normalize_jsonish(&ours));
}

#[test]
fn differential_builtin_subclass_reversed_protocol_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"class TokenList(list):
    pass

token_list = TokenList([1, 2, 3])
result = {
    "has_len": hasattr(token_list, "__len__"),
    "has_getitem": hasattr(token_list, "__getitem__"),
    "has_reversed": hasattr(token_list, "__reversed__"),
    "explicit": list(token_list.__reversed__()),
    "implicit": list(reversed(token_list)),
}"#;
    let py = run_cpython_json(source).expect("CPython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(normalize_jsonish(&py), normalize_jsonish(&ours));
}

#[test]
fn differential_template_literal_basic_shape_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"variety = "Stilton"
template = t"Try some {variety} cheese!"
interp = template.interpolations[0]
result = {
    "type": repr(type(template)),
    "strings": list(template.strings),
    "interp": [interp.value, interp.expression, interp.conversion, interp.format_spec],
}
"#;
    let py = run_cpython_json(source).expect("cpython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(py.as_str()),
        normalize_jsonish(ours.as_str())
    );
}

#[test]
fn differential_template_literal_debug_and_concat_matches_cpython() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"x = 7
t1 = t"{x=}"
t2 = t"{x=:>4}"
t3 = t"a{1}" t"b{2}"
result = {
    "t1_strings": list(t1.strings),
    "t1_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t1.interpolations],
    "t2_strings": list(t2.strings),
    "t2_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t2.interpolations],
    "t3_strings": list(t3.strings),
    "t3_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t3.interpolations],
}
"#;
    let py = run_cpython_json(source).expect("cpython should run");
    let ours = run_pyrs_json(source).expect("pyrs should run");
    assert_eq!(
        normalize_jsonish(py.as_str()),
        normalize_jsonish(ours.as_str())
    );
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
    let ours = run_pyrs_cli_json(source).expect("pyrs JSON should run");
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
    let ours = run_pyrs_cli_json(source).expect("pyrs JSON should run");
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
fn differential_compile_only_ast_type_alias_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import _ast
mod = compile("type Pair[T] = tuple[T, T]\n", "<ast>", "exec", _ast.PyCF_ONLY_AST)
node = mod.body[0]
result = {
    "stmt_type": type(node).__name__,
    "stmt_base": isinstance(node, _ast.stmt),
    "name_type": type(node.name).__name__,
    "name_id": node.name.id,
    "name_ctx": type(node.name.ctx).__name__,
    "type_params": [type(tp).__name__ for tp in node.type_params],
    "type_param_names": [tp.name for tp in node.type_params],
    "value_type": type(node.value).__name__,
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
st = compile("{x for x in xs if x > 0}", "<ast>", "eval", _ast.PyCF_ONLY_AST).body
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
    "setcomp_type": type(st).__name__,
    "setcomp_gen_type": type(st.generators[0]).__name__,
    "setcomp_gen_base": isinstance(st.generators[0], _ast.comprehension),
    "setcomp_is_async": st.generators[0].is_async,
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

#[test]
fn differential_runtime_function_type_params_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
def ident[T, *Ts, **P](x):
    return x
params = ident.__type_params__
result = {
    "kind_names": [type(tp).__name__ for tp in params],
    "names": [tp.__name__ for tp in params],
    "call_result": ident(7),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_exception_group_except_star_split_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
try:
    raise ExceptionGroup("eg", [ValueError(1), TypeError(2)])
except* ValueError as eg:
    left = [len(eg.exceptions), type(eg.exceptions[0]).__name__]
except* TypeError as tg:
    right = [len(tg.exceptions), type(tg.exceptions[0]).__name__]
result = {"left": left, "right": right}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_class_type_params_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
class Box[T, *Ts, **P]:
    pass
params = Box.__type_params__
result = {
    "kind_names": [type(tp).__name__ for tp in params],
    "names": [tp.__name__ for tp in params],
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_type_alias_type_params_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
type Pair[T] = tuple[T, T]
result = {
    "type_name": type(Pair).__name__,
    "names": [tp.__name__ for tp in Pair.__type_params__],
    "repr": repr(Pair),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_type_param_bound_constraints_default_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
def f[T: int = str](x):
    return x
def g[T: (int, str)](x):
    return x
def h[*Ts = [int]](x):
    return x
def p[**P = [int, str]](x):
    return x
ft = f.__type_params__[0]
gt = g.__type_params__[0]
ht = h.__type_params__[0]
pt = p.__type_params__[0]
result = {
    "fb": getattr(ft, "__bound__", None) is int,
    "fd": getattr(ft, "__default__", None) is str,
    "gc": [c.__name__ for c in getattr(gt, "__constraints__", ())],
    "hd": repr(getattr(ht, "__default__", None)),
    "pd": repr(getattr(pt, "__default__", None)),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_type_param_cross_reference_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
def f[T, U: T](x):
    return x
def g[T = int, U = list[T]](x):
    return x
ft, fu = f.__type_params__
gt, gu = g.__type_params__
default = gu.__default__
result = {
    "bound_is_prior_param": fu.__bound__ is ft,
    "default_type": type(default).__name__,
    "default_origin_is_list": getattr(default, "__origin__", None) is list,
    "default_arg0_is_prior_param": (
        hasattr(default, "__args__")
        and len(default.__args__) == 1
        and default.__args__[0] is gt
    ),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_builtin_generic_alias_without_types_import_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
before = type(list[int]).__name__
import types
alias = list[int]
result = {
    "before_type": before,
    "isinstance": isinstance(alias, types.GenericAlias),
    "origin_is_list": getattr(alias, "__origin__", None) is list,
    "arg0_is_int": (
        hasattr(alias, "__args__")
        and len(alias.__args__) == 1
        and alias.__args__[0] is int
    ),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_user_generic_class_alias_shape_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import sys
class C[T]:
    pass
alias = C[int]
result = {
    "typing_loaded": "typing" in sys.modules,
    "alias_type_module": type(alias).__module__,
    "alias_type_name": type(alias).__name__,
    "origin_is_c": getattr(alias, "__origin__", None) is C,
    "arg0_is_int": (
        hasattr(alias, "__args__")
        and len(alias.__args__) == 1
        and alias.__args__[0] is int
    ),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_unpacked_builtin_generic_alias_equality_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
class C:
    @classmethod
    def __class_getitem__(cls, item):
        return item
marker = C[*tuple[int, ...]][0]
plain = tuple[int, ...]
result = {
    "marker_repr": repr(marker),
    "marker_eq_plain": marker == plain,
    "dict_len": len({marker: 1, plain: 2}),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_itertools_chain_laziness_and_shape_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import itertools
events = []
class Boom:
    def __iter__(self):
        events.append("boom_iter")
        raise RuntimeError("boom")

c = itertools.chain(Boom(), [10, 11])
constructed_events = list(events)
iter_identity = (iter(c) is c)
try:
    next(c)
except RuntimeError as exc:
    first_error = str(exc)
else:
    first_error = ""
events_after_first_next = list(events)

class Outer:
    def __iter__(self):
        events.append("outer_iter")
        yield [1, 2]
        yield [3]

c2 = itertools.chain.from_iterable(Outer())
events_after_create_c2 = list(events)
first = next(c2)
events_after_first_c2 = list(events)
rest = list(c2)

result = {
    "constructed_events": constructed_events,
    "iter_identity": iter_identity,
    "first_error": first_error,
    "events_after_first_next": events_after_first_next,
    "events_after_create_c2": events_after_create_c2,
    "first": first,
    "events_after_first_c2": events_after_first_c2,
    "rest": rest,
    "chain_repr_prefix": repr(itertools.chain([1])).startswith("<itertools.chain object at 0x"),
    "chain_from_iterable_repr_prefix": repr(itertools.chain.from_iterable([[1]])).startswith("<itertools.chain object at 0x"),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_iterator_type_identity_and_repr_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import itertools

class Counter:
    def __init__(self):
        self.n = 0
    def __call__(self):
        self.n += 1
        return self.n

def normalize_repr(obj):
    text = repr(obj)
    start = text.find("0x")
    if start == -1:
        return text
    end = start + 2
    while end < len(text) and text[end] in "0123456789abcdefABCDEF":
        end += 1
    return text[:start] + "0xADDR" + text[end:]

call_iter = iter(Counter(), 3)
group_item = next(iter(itertools.groupby([1])))[1]
samples = {
    "range_obj": range(3),
    "range_iter": iter(range(3)),
    "list_iter": iter([1]),
    "tuple_iter": iter((1,)),
    "str_ascii_iter": iter("a"),
    "str_unicode_iter": iter("é"),
    "dict_keys": {}.keys(),
    "dict_iter": iter({1: 2}),
    "set_iter": iter({1}),
    "bytes_iter": iter(b"a"),
    "bytearray_iter": iter(bytearray(b"a")),
    "memoryview_iter": iter(memoryview(b"a")),
    "callable_iter": call_iter,
    "enumerate": enumerate([1]),
    "zip": zip([1], [2]),
    "map": map(int, ["1"]),
    "filter": filter(None, [1]),
    "chain": itertools.chain([1]),
    "count": itertools.count(),
    "repeat": itertools.repeat(1),
    "groupby": itertools.groupby([1]),
    "grouper": group_item,
    "tee": itertools.tee([1], 1)[0],
    "zip_longest": itertools.zip_longest([1], [2]),
    "batched": itertools.batched([1], 1),
}
builtin_types = {
    "range": range,
    "enumerate": enumerate,
    "zip": zip,
    "map": map,
    "filter": filter,
    "reversed": reversed,
    "itertools.chain": itertools.chain,
    "itertools.count": itertools.count,
    "itertools.repeat": itertools.repeat,
    "itertools.groupby": itertools.groupby,
    "itertools.batched": itertools.batched,
}
result = {
    "samples": {
        name: {
            "type_repr": repr(type(obj)),
            "class_repr": repr(obj.__class__),
            "type_matches_class": (obj.__class__ is type(obj)),
            "repr": normalize_repr(obj),
        }
        for name, obj in samples.items()
    },
    "builtin_types": {name: repr(obj) for name, obj in builtin_types.items()},
    "checks": {
        "filter_values": list(filter(None, [0, 1, '', 2])),
        "list_iter_isinstance": isinstance(iter([1]), type(iter([1]))),
        "range_isinstance": isinstance(range(3), type(range(3))),
        "dict_keys_isinstance": isinstance({}.keys(), type({}.keys())),
    },
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_reversed_protocol_and_type_identity_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
class Seq:
    def __len__(self):
        return 3
    def __getitem__(self, index):
        if 0 <= index < 3:
            return index + 10
        raise IndexError(index)

def normalize_repr(obj):
    text = repr(obj)
    start = text.find("0x")
    if start == -1:
        return text
    end = start + 2
    while end < len(text) and text[end] in "0123456789abcdefABCDEF":
        end += 1
    return text[:start] + "0xADDR" + text[end:]

samples = {
    "list": reversed([1, 2]),
    "tuple": reversed((1, 2)),
    "range": reversed(range(3)),
    "seq": reversed(Seq()),
    "list_dunder": [].__reversed__(),
    "range_dunder": range(3).__reversed__(),
}
bad = {}
try:
    reversed(iter([1]))
except Exception as exc:
    bad = {"kind": type(exc).__name__, "text": str(exc)}

result = {
    "samples": {
        name: {
            "type_repr": repr(type(obj)),
            "class_repr": repr(obj.__class__),
            "repr": normalize_repr(obj),
            "values": list(obj),
        }
        for name, obj in samples.items()
    },
    "attrs": {
        "list": hasattr([], "__reversed__"),
        "tuple": hasattr((), "__reversed__"),
        "range": hasattr(range(3), "__reversed__"),
    },
    "bad": bad,
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_dict_view_and_reverse_iterator_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
def normalize_repr(obj):
    text = repr(obj)
    start = text.find("0x")
    if start == -1:
        return text
    end = start + 2
    while end < len(text) and text[end] in "0123456789abcdefABCDEF":
        end += 1
    return text[:start] + "0xADDR" + text[end:]

d = {"a": 1, "b": 2}
samples = {
    "keys": d.keys(),
    "items": d.items(),
    "values": d.values(),
    "iter_keys": iter(d.keys()),
    "iter_items": iter(d.items()),
    "iter_values": iter(d.values()),
    "reverse_dict": reversed(d),
    "reverse_keys": reversed(d.keys()),
    "reverse_items": reversed(d.items()),
    "reverse_values": reversed(d.values()),
}
result = {
    "samples": {
        name: {
            "type_repr": repr(type(obj)),
            "class_repr": repr(obj.__class__),
            "type_matches_class": (obj.__class__ is type(obj)),
            "repr": normalize_repr(obj),
            "values": list(obj),
        }
        for name, obj in samples.items()
    }
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_callable_and_descriptor_repr_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import types

def normalize_repr(obj):
    text = repr(obj)
    start = text.find("0x")
    if start == -1:
        return text
    end = start + 2
    while end < len(text) and text[end] in "0123456789abcdefABCDEF":
        end += 1
    return text[:start] + "0xADDR" + text[end:]

class C:
    p = property(lambda self: 1)
    def f(self):
        return 1

samples = {
    "py_unbound": C.f,
    "py_bound": C().f,
    "builtin_bound_list": [].append,
    "builtin_unbound_list": list.append,
    "builtin_bound_dict": {"a": 1}.keys,
    "builtin_unbound_dict": dict.keys,
    "builtin_bound_str": "-".join,
    "builtin_unbound_str": str.join,
    "wrapper_descriptor": object.__str__,
    "method_wrapper": object().__str__,
    "classmethod_descriptor": dict.__dict__["fromkeys"],
    "property_obj": C.__dict__["p"],
    "code_descriptor": types.FunctionType.__code__,
    "globals_descriptor": types.FunctionType.__globals__,
}
result = {
    "samples": {
        name: {
            "type_repr": repr(type(obj)),
            "class_repr": repr(obj.__class__),
            "type_matches_class": (obj.__class__ is type(obj)),
            "repr": normalize_repr(obj),
        }
        for name, obj in samples.items()
    }
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_builtin_slot_wrapper_repr_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
def normalize_repr(obj):
    text = repr(obj)
    start = text.find("0x")
    if start == -1:
        return text
    end = start + 2
    while end < len(text) and text[end] in "0123456789abcdefABCDEF":
        end += 1
    return text[:start] + "0xADDR" + text[end:]

samples = {
    "list_unbound_repr": list.__repr__,
    "list_bound_repr": [].__repr__,
    "list_unbound_str": list.__str__,
    "list_bound_str": [].__str__,
    "dict_unbound_repr": dict.__repr__,
    "dict_bound_repr": {}.__repr__,
    "dict_unbound_str": dict.__str__,
    "dict_bound_str": {}.__str__,
    "tuple_unbound_repr": tuple.__repr__,
    "tuple_bound_repr": ().__repr__,
    "tuple_unbound_str": tuple.__str__,
    "tuple_bound_str": ().__str__,
    "bytes_unbound_repr": bytes.__repr__,
    "bytes_bound_repr": b"".__repr__,
    "bytes_unbound_str": bytes.__str__,
    "bytes_bound_str": b"".__str__,
    "bytearray_unbound_repr": bytearray.__repr__,
    "bytearray_bound_repr": bytearray(b"").__repr__,
    "bytearray_unbound_str": bytearray.__str__,
    "bytearray_bound_str": bytearray(b"").__str__,
    "set_unbound_repr": set.__repr__,
    "set_bound_repr": set().__repr__,
    "set_unbound_str": set.__str__,
    "set_bound_str": set().__str__,
    "str_unbound_repr": str.__repr__,
    "str_bound_repr": "x".__repr__,
    "str_unbound_str": str.__str__,
    "str_bound_str": "x".__str__,
    "int_unbound_repr": int.__repr__,
    "int_bound_repr": (1).__repr__,
    "int_unbound_str": int.__str__,
    "int_bound_str": (1).__str__,
    "bool_unbound_repr": bool.__repr__,
    "bool_bound_repr": True.__repr__,
    "bool_unbound_str": bool.__str__,
    "bool_bound_str": True.__str__,
    "float_unbound_repr": float.__repr__,
    "float_bound_repr": (1.5).__repr__,
    "float_unbound_str": float.__str__,
    "float_bound_str": (1.5).__str__,
    "int_unbound_add": int.__add__,
    "int_bound_add": (1).__add__,
    "bool_unbound_add": bool.__add__,
    "bool_bound_add": True.__add__,
    "bool_unbound_lt": bool.__lt__,
    "bool_bound_lt": True.__lt__,
}
result = {
    name: {
        "type_repr": repr(type(obj)),
        "class_repr": repr(obj.__class__),
        "type_matches_class": obj.__class__ is type(obj),
        "repr": normalize_repr(obj),
    }
    for name, obj in samples.items()
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_unittest_subtest_string_render_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
def normalize_text(text):
    start = text.find("0x")
    if start == -1:
        return text
    end = start + 2
    while end < len(text) and text[end] in "0123456789abcdefABCDEF":
        end += 1
    return text[:start] + "0xADDR" + text[end:]

import unittest

class T(unittest.TestCase):
    def test_x(self):
        pass

sub = unittest.case._SubTest(T("test_x"), None, {"fn": (lambda: 1)})
result = {
    "text": normalize_text(str(sub)),
    "type_repr": repr(type(sub)),
    "class_repr": repr(sub.__class__),
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_cli_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_builtin_backed_subclass_display_parity() {
    let source = r#"
class L(list):
    pass

class S(str):
    pass

class I(int):
    pass

class F(float):
    pass

l = L([1, 2])
s = S("hi")
i = I(3)
f = F(1.5)
result = {
    "reprs": {
        "l": repr(l),
        "s": repr(s),
        "i": repr(i),
        "f": repr(f),
    },
    "strs": {
        "l": str(l),
        "s": str(s),
        "i": str(i),
        "f": str(f),
    },
    "class_attrs": {
        "L_repr": repr(L.__repr__),
        "L_str": repr(L.__str__),
        "S_repr": repr(S.__repr__),
        "S_str": repr(S.__str__),
        "I_repr": repr(I.__repr__),
        "I_str": repr(I.__str__),
        "F_repr": repr(F.__repr__),
        "F_str": repr(F.__str__),
    },
    "calls": {
        "L_repr": L.__repr__(l),
        "L_str": L.__str__(l),
        "S_repr": S.__repr__(s),
        "S_str": S.__str__(s),
        "I_repr": I.__repr__(i),
        "I_str": I.__str__(i),
        "float_repr": float.__repr__(f),
        "F_repr": F.__repr__(f),
        "F_str": F.__str__(f),
    },
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_itertools_iterator_helper_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import itertools, operator

def probe(factory):
    events = []
    class Boom:
        def __iter__(self):
            events.append("iter")
            raise RuntimeError("boom")
    try:
        factory(Boom())
    except RuntimeError as exc:
        return {"events": events, "err": str(exc)}
    return {"events": events, "err": "noerr"}

construct = {
    "accumulate": probe(lambda x: itertools.accumulate(x)),
    "batched": probe(lambda x: itertools.batched(x, 1)),
    "combinations": probe(lambda x: itertools.combinations(x, 1)),
    "combinations_with_replacement": probe(lambda x: itertools.combinations_with_replacement(x, 1)),
    "compress": probe(lambda x: itertools.compress(x, [1])),
    "dropwhile": probe(lambda x: itertools.dropwhile(lambda y: y < 0, x)),
    "filterfalse": probe(lambda x: itertools.filterfalse(None, x)),
    "groupby": probe(lambda x: itertools.groupby(x)),
    "islice": probe(lambda x: itertools.islice(x, 1)),
    "pairwise": probe(lambda x: itertools.pairwise(x)),
    "permutations": probe(lambda x: itertools.permutations(x, 1)),
    "product": probe(lambda x: itertools.product(x)),
    "starmap": probe(lambda x: itertools.starmap(operator.add, x)),
    "takewhile": probe(lambda x: itertools.takewhile(lambda y: y < 0, x)),
    "zip_longest": probe(lambda x: itertools.zip_longest(x, [1])),
    "tee": probe(lambda x: itertools.tee(x)),
}

t1, t2 = itertools.tee([1, 2], 2)
objs = {
    "accumulate": itertools.accumulate([1, 2]),
    "batched": itertools.batched([1, 2, 3], 2),
    "combinations": itertools.combinations([1, 2, 3], 2),
    "combinations_with_replacement": itertools.combinations_with_replacement([1, 2], 2),
    "compress": itertools.compress([1, 2], [1, 0]),
    "dropwhile": itertools.dropwhile(lambda x: x < 2, [1, 2, 3]),
    "filterfalse": itertools.filterfalse(None, [0, 1]),
    "groupby": itertools.groupby([1, 1, 2]),
    "islice": itertools.islice(range(10), 3),
    "pairwise": itertools.pairwise([1, 2, 3]),
    "permutations": itertools.permutations([1, 2, 3], 2),
    "product": itertools.product([1, 2], repeat=2),
    "repeat": itertools.repeat("x", 2),
    "starmap": itertools.starmap(operator.add, [(1, 2)]),
    "takewhile": itertools.takewhile(lambda x: x < 3, [1, 2, 3]),
    "zip_longest": itertools.zip_longest([1, 2], [10], fillvalue=0),
    "tee": t1,
}
is_list = {k: isinstance(v, list) for k, v in objs.items()}
iter_identity = {k: (iter(v) is v) for k, v in objs.items()}
repr_prefix = {
    "accumulate": repr(objs["accumulate"]).startswith("<itertools.accumulate object at 0x"),
    "batched": repr(objs["batched"]).startswith("<itertools.batched object at 0x"),
    "combinations": repr(objs["combinations"]).startswith("<itertools.combinations object at 0x"),
    "combinations_with_replacement": repr(objs["combinations_with_replacement"]).startswith("<itertools.combinations_with_replacement object at 0x"),
    "compress": repr(objs["compress"]).startswith("<itertools.compress object at 0x"),
    "dropwhile": repr(objs["dropwhile"]).startswith("<itertools.dropwhile object at 0x"),
    "filterfalse": repr(objs["filterfalse"]).startswith("<itertools.filterfalse object at 0x"),
    "groupby": repr(objs["groupby"]).startswith("<itertools.groupby object at 0x"),
    "islice": repr(objs["islice"]).startswith("<itertools.islice object at 0x"),
    "pairwise": repr(objs["pairwise"]).startswith("<itertools.pairwise object at 0x"),
    "permutations": repr(objs["permutations"]).startswith("<itertools.permutations object at 0x"),
    "product": repr(objs["product"]).startswith("<itertools.product object at 0x"),
    "repeat": repr(objs["repeat"]) == "repeat('x', 2)",
    "starmap": repr(objs["starmap"]).startswith("<itertools.starmap object at 0x"),
    "takewhile": repr(objs["takewhile"]).startswith("<itertools.takewhile object at 0x"),
    "zip_longest": repr(objs["zip_longest"]).startswith("<itertools.zip_longest object at 0x"),
    "tee": repr(objs["tee"]).startswith("<itertools._tee object at 0x"),
}
values = {
    "accumulate": list(itertools.accumulate([1, 2, 3])),
    "batched": list(itertools.batched([1, 2, 3, 4, 5], 2)),
    "combinations": list(itertools.combinations("ABC", 2)),
    "combinations_with_replacement": list(itertools.combinations_with_replacement([1, 2], 2)),
    "compress": list(itertools.compress("ABCDEF", [1, 0, 1, 0, 1, 1])),
    "dropwhile": list(itertools.dropwhile(lambda x: x < 3, [1, 2, 3, 2, 1])),
    "filterfalse": list(itertools.filterfalse(lambda x: x % 2, [0, 1, 2, 3, 4])),
    "groupby": [(k, list(g)) for k, g in itertools.groupby("AAABBC")],
    "islice": list(itertools.islice(range(10), 2, 8, 3)),
    "pairwise": list(itertools.pairwise([10, 20, 30])),
    "permutations": list(itertools.permutations([1, 2, 3], 2)),
    "product": list(itertools.product([1, 2], repeat=2)),
    "repeat_finite": list(itertools.repeat("x", 2)),
    "repeat_negative": list(itertools.repeat("x", -3)),
    "repeat_prefix": list(itertools.islice(itertools.repeat("x"), 3)),
    "starmap": list(itertools.starmap(operator.add, [(1, 2), (3, 4)])),
    "takewhile": list(itertools.takewhile(lambda x: x < 4, [1, 2, 3, 4, 1])),
    "zip_longest": list(itertools.zip_longest([1, 2], [10], fillvalue=0)),
    "tee_a": [next(t2), *list(t2)],
}

result = {
    "construct": construct,
    "is_list": is_list,
    "iter_identity": iter_identity,
    "repr_prefix": repr_prefix,
    "values": values,
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_runtime_itertools_groupby_partial_consumption_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
import itertools
events = []
def src():
    for x in [1, 1, 2, 2, 3]:
        events.append(x)
        yield x
g = itertools.groupby(src())
created = list(events)
k1, grp1 = next(g)
after_outer_first = list(events)
first_item = next(grp1)
after_first_item = list(events)
k2, grp2 = next(g)
after_outer_second = list(events)
old_group_tail = list(grp1)
group2 = list(grp2)
k3, grp3 = next(g)
group3 = list(grp3)
stopped = False
try:
    next(g)
except StopIteration:
    stopped = True
result = {
    "created": created,
    "k1": k1,
    "after_outer_first": after_outer_first,
    "first_item": first_item,
    "after_first_item": after_first_item,
    "k2": k2,
    "after_outer_second": after_outer_second,
    "old_group_tail": old_group_tail,
    "group2": group2,
    "k3": k3,
    "group3": group3,
    "stopped": stopped,
}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_compile_parse_error_raises_syntaxerror_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
ok = False
try:
    compile("def broken(:\n    pass\n", "<broken>", "exec")
except SyntaxError:
    ok = True
result = {"syntax_error": ok}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}

#[test]
fn differential_posonly_keyword_name_captured_by_varkw_parity() {
    if cpython_bin_or_panic().as_os_str().is_empty() {
        return;
    }
    let source = r#"
def f(a, /, **kw):
    return kw["a"]
result = {"value": f(1, a=2)}
"#;
    let py = run_cpython_json(source).expect("CPython JSON should run");
    let ours = run_pyrs_json(source).expect("pyrs JSON should run");
    assert_eq!(py, ours, "{}", source);
}
