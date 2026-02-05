use pyrs::{compiler, parser, runtime::Value, vm::Vm};

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    FloorDiv,
    Mod,
    Pow,
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
        // xorshift64*
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        ((x.wrapping_mul(0x2545F4914F6CDD1D)) >> 32) as u32
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
    if depth == 0 || rng.gen_range(5) == 0 {
        return Expr::Lit(rng.gen_i64(-20, 20));
    }
    let left = gen_expr(rng, depth - 1);
    let right = gen_expr(rng, depth - 1);
    let op = match rng.gen_range(6) {
        0 => Op::Add,
        1 => Op::Sub,
        2 => Op::Mul,
        3 => Op::FloorDiv,
        4 => Op::Mod,
        _ => Op::Pow,
    };
    Expr::Bin(Box::new(left), op, Box::new(right))
}

fn expr_to_source(expr: &Expr) -> String {
    match expr {
        Expr::Lit(value) => {
            if *value < 0 {
                format!("({})", value)
            } else {
                value.to_string()
            }
        }
        Expr::Bin(left, op, right) => {
            let op_str = match op {
                Op::Add => "+",
                Op::Sub => "-",
                Op::Mul => "*",
                Op::FloorDiv => "//",
                Op::Mod => "%",
                Op::Pow => "**",
            };
            format!("({} {} {})", expr_to_source(left), op_str, expr_to_source(right))
        }
    }
}

fn eval_expr(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Lit(value) => Some(*value),
        Expr::Bin(left, op, right) => {
            let left = eval_expr(left)?;
            let right = eval_expr(right)?;
            match op {
                Op::Add => left.checked_add(right),
                Op::Sub => left.checked_sub(right),
                Op::Mul => left.checked_mul(right),
                Op::FloorDiv => python_floordiv(left, right),
                Op::Mod => python_mod(left, right),
                Op::Pow => python_pow(left, right),
            }
        }
    }
}

fn python_floordiv(a: i64, b: i64) -> Option<i64> {
    if b == 0 {
        return None;
    }
    let div = a / b;
    let rem = a % b;
    if rem != 0 && ((a < 0) ^ (b < 0)) {
        div.checked_sub(1)
    } else {
        Some(div)
    }
}

fn python_mod(a: i64, b: i64) -> Option<i64> {
    if b == 0 {
        return None;
    }
    let div = python_floordiv(a, b)?;
    a.checked_sub(div.checked_mul(b)?)
}

fn python_pow(base: i64, exp: i64) -> Option<i64> {
    if exp < 0 {
        return None;
    }
    let mut result: i64 = 1;
    let mut acc = base;
    let mut exp = exp as u64;
    while exp > 0 {
        if exp & 1 == 1 {
            result = result.checked_mul(acc)?;
        }
        exp >>= 1;
        if exp > 0 {
            acc = acc.checked_mul(acc)?;
        }
    }
    Some(result)
}

#[test]
fn fuzz_arithmetic_expressions() {
    let mut rng = Rng::new(0xC0FFEE);
    for _ in 0..300 {
        let expr = gen_expr(&mut rng, 4);
        let expected = match eval_expr(&expr) {
            Some(value) => value,
            None => continue,
        };
        let source = format!("result = {}", expr_to_source(&expr));
        let module = parser::parse_module(&source).expect("parse");
        let code = compiler::compile_module(&module).expect("compile");
        let mut vm = Vm::new();
        vm.execute(&code).expect("execute");
        let actual = vm.get_global("result");
        let expected_value = Some(Value::Int(expected));
        if actual != expected_value {
            panic!(
                "mismatch for expr {}: expected {:?}, got {:?}",
                expr_to_source(&expr),
                expected_value,
                actual
            );
        }
    }
}
