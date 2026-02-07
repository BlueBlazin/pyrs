use std::collections::BTreeSet;
use std::fs;

use pyrs::vm::Vm;

const INVENTORY_PATH: &str = "docs/NOOP_BUILTIN_INVENTORY.txt";

fn read_inventory_file(path: &str) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("failed to read {path}: {err}");
    });
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

#[test]
fn noop_builtin_inventory_file_is_current() {
    let expected_raw = read_inventory_file(INVENTORY_PATH);
    let expected: BTreeSet<String> = expected_raw.iter().cloned().collect();
    assert_eq!(
        expected.len(),
        expected_raw.len(),
        "{INVENTORY_PATH} contains duplicate entries"
    );

    let actual: BTreeSet<String> = Vm::new().noop_builtin_inventory().into_iter().collect();
    if actual != expected {
        let missing: Vec<String> = actual.difference(&expected).cloned().collect();
        let stale: Vec<String> = expected.difference(&actual).cloned().collect();
        panic!(
            "NoOp builtin inventory mismatch.\n\
             Run: cargo run --quiet --bin print_noop_inventory > {INVENTORY_PATH}\n\
             Missing entries in inventory file:\n{}\n\
             Stale entries in inventory file:\n{}",
            if missing.is_empty() {
                "(none)".to_string()
            } else {
                missing.join("\n")
            },
            if stale.is_empty() {
                "(none)".to_string()
            } else {
                stale.join("\n")
            }
        );
    }
}
