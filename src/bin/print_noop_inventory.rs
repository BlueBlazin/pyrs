use pyrs::vm::Vm;

fn main() {
    let vm = Vm::new();
    for symbol in vm.noop_builtin_inventory() {
        println!("{symbol}");
    }
}
