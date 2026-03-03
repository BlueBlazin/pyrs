#![cfg(not(target_arch = "wasm32"))]

use std::fs;

use pyrs::bytecode::metadata::OpcodeMetadata;

#[test]
fn loads_opcode_csv() {
    let mut path = std::env::temp_dir();
    path.push(format!("pyrs_opcode_{}.csv", std::process::id()));

    fs::write(&path, "1,LOAD_CONST,1,stack").expect("write should succeed");

    let metadata = OpcodeMetadata::load_from_csv(&path).expect("load should succeed");
    assert_eq!(metadata.opcodes.len(), 1);
    assert_eq!(metadata.opcodes[0].code, 1);
    assert_eq!(metadata.opcodes[0].name, "LOAD_CONST");
    assert_eq!(metadata.opcodes[0].stack_effect, 1);
    assert_eq!(metadata.opcodes[0].flags, "stack");

    let _ = fs::remove_file(&path);
}
