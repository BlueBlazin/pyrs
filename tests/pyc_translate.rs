use pyrs::bytecode::Opcode;
use pyrs::bytecode::cpython::{CpythonCode, PyObject, translate_code};
use pyrs::bytecode::metadata::OpcodeMetadata;
use pyrs::runtime::Heap;

fn op(name: &str) -> u8 {
    let metadata = OpcodeMetadata::load_default().expect("load opcode metadata");
    metadata
        .opcodes
        .iter()
        .find(|info| info.name == name)
        .map(|info| info.code as u8)
        .unwrap_or_else(|| panic!("opcode not found: {name}"))
}

fn test_code(code: Vec<u8>) -> CpythonCode {
    CpythonCode {
        argcount: 0,
        posonlyargcount: 0,
        kwonlyargcount: 0,
        stacksize: 1,
        flags: 0,
        code,
        consts: vec![PyObject::None],
        names: Vec::new(),
        localsplusnames: Vec::new(),
        localspluskinds: Vec::new(),
        filename: "<test>".to_string(),
        name: "<module>".to_string(),
        qualname: "<module>".to_string(),
        firstlineno: 1,
        linetable: Vec::new(),
        exceptiontable: Vec::new(),
    }
}

#[test]
fn rejects_out_of_range_jump_target() {
    let code = test_code(vec![op("JUMP_FORWARD"), 250, op("RETURN_VALUE"), 0]);
    let mut heap = Heap::new();
    let err = translate_code(&code, &mut heap).expect_err("translation should fail");
    assert!(
        err.message.contains("jump target"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn rejects_stack_underflow_after_translation() {
    let code = test_code(vec![op("POP_TOP"), 0, op("RETURN_VALUE"), 0]);
    let mut heap = Heap::new();
    let err = translate_code(&code, &mut heap).expect_err("translation should fail");
    assert!(
        err.message.contains("stack underflow"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn translates_binary_op_and_and_formatting_ops() {
    let code = test_code(vec![
        op("LOAD_SMALL_INT"),
        6,
        op("LOAD_SMALL_INT"),
        3,
        op("BINARY_OP"),
        1,
        op("CONVERT_VALUE"),
        2,
        op("FORMAT_SIMPLE"),
        0,
        op("RETURN_VALUE"),
        0,
    ]);
    let mut heap = Heap::new();
    let translated = translate_code(&code, &mut heap).expect("translation should succeed");
    let opcodes: Vec<Opcode> = translated
        .instructions
        .iter()
        .map(|instr| instr.opcode)
        .collect();
    assert_eq!(
        opcodes,
        vec![
            Opcode::LoadConst,
            Opcode::LoadConst,
            Opcode::BinaryAnd,
            Opcode::ConvertValue,
            Opcode::FormatSimple,
            Opcode::ReturnValue
        ]
    );
}
