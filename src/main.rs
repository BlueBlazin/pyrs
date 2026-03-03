#[cfg(not(target_arch = "wasm32"))]
fn main() {
    std::process::exit(pyrs::cli::run());
}

#[cfg(target_arch = "wasm32")]
fn main() {}
