#[cfg(not(target_arch = "wasm32"))]
use pyrs::vm::Vm;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let vm = Vm::new();
    for symbol in vm.noop_builtin_inventory() {
        println!("{symbol}");
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
