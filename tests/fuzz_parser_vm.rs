use pyrs::{compiler, parser, vm::Vm};

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

fn gen_ident(rng: &mut Rng) -> &'static str {
    match rng.gen_range(6) {
        0 => "a",
        1 => "b",
        2 => "c",
        3 => "x",
        4 => "y",
        _ => "z",
    }
}

fn gen_expr(rng: &mut Rng, depth: usize) -> String {
    if depth == 0 || rng.gen_range(4) == 0 {
        return rng.gen_i64(-20, 20).to_string();
    }
    let left = gen_expr(rng, depth - 1);
    let right = gen_expr(rng, depth - 1);
    let op = match rng.gen_range(5) {
        0 => "+",
        1 => "-",
        2 => "*",
        3 => "//",
        _ => "%",
    };
    format!("({left} {op} {right})")
}

fn gen_stmt(rng: &mut Rng) -> String {
    match rng.gen_range(5) {
        0 => format!("{} = {}", gen_ident(rng), gen_expr(rng, 3)),
        1 => format!("{} += {}", gen_ident(rng), rng.gen_i64(0, 5)),
        2 => format!(
            "if {}:\n    {} = {}\n",
            gen_expr(rng, 2),
            gen_ident(rng),
            gen_expr(rng, 2)
        ),
        3 => format!(
            "while {}:\n    {} = {}\n    break\n",
            gen_expr(rng, 1),
            gen_ident(rng),
            gen_expr(rng, 1)
        ),
        _ => format!("assert {}", gen_expr(rng, 2)),
    }
}

#[test]
fn fuzz_parser_compiler_vm_no_panics() {
    let mut rng = Rng::new(0xFACE_B00C);
    for _ in 0..400 {
        let mut source = String::from("a = 1\nb = 2\n");
        let lines = 1 + rng.gen_range(8);
        for _ in 0..lines {
            source.push_str(&gen_stmt(&mut rng));
            source.push('\n');
        }
        let result = std::panic::catch_unwind(|| {
            if let Ok(module) = parser::parse_module(&source)
                && let Ok(code) = compiler::compile_module(&module) {
                    let mut vm = Vm::new();
                    let _ = vm.execute(&code);
                }
        });
        assert!(
            result.is_ok(),
            "fuzzed parser/compiler/vm panicked on source:\n{source}"
        );
    }
}
