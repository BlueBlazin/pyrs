use std::path::PathBuf;
use std::process::Command;

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
    let code = compiler::compile_module(&module).map_err(|err| format!("compile {}", err.message))?;
    let mut vm = Vm::new();
    vm.execute(&code)
        .map_err(|err| format!("runtime {}", err.message))?;
    match vm.get_global("__pyrs_json") {
        Some(Value::Str(text)) => Ok(text),
        other => Err(format!("missing __pyrs_json result: {other:?}")),
    }
}

fn normalize_jsonish(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_ascii_whitespace()).collect()
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
