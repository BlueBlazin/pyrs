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
        stacksize: 8,
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

#[test]
fn translates_compare_op_with_masked_arg_bits() {
    let code = test_code(vec![
        op("LOAD_SMALL_INT"),
        1,
        op("LOAD_SMALL_INT"),
        1,
        op("COMPARE_OP"),
        72,
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
            Opcode::CompareEq,
            Opcode::ReturnValue
        ]
    );
}

#[test]
fn translates_load_special_and_call_intrinsic_1() {
    let code = test_code(vec![
        op("LOAD_CONST"),
        0,
        op("LOAD_SPECIAL"),
        1,
        op("POP_TOP"),
        0,
        op("POP_TOP"),
        0,
        op("LOAD_CONST"),
        0,
        op("CALL_INTRINSIC_1"),
        2,
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
            Opcode::LoadSpecial,
            Opcode::PopTop,
            Opcode::PopTop,
            Opcode::LoadConst,
            Opcode::CallIntrinsic1,
            Opcode::ReturnValue
        ]
    );
}

#[test]
fn translates_call_intrinsic_2() {
    let code = test_code(vec![
        op("LOAD_CONST"),
        0,
        op("LOAD_CONST"),
        0,
        op("CALL_INTRINSIC_2"),
        4,
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
            Opcode::CallIntrinsic2,
            Opcode::ReturnValue
        ]
    );
}

#[test]
fn translates_get_len_and_build_template() {
    let code = test_code(vec![
        op("LOAD_CONST"),
        0,
        op("GET_LEN"),
        0,
        op("LOAD_CONST"),
        0,
        op("LOAD_CONST"),
        0,
        op("BUILD_TEMPLATE"),
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
            Opcode::GetLen,
            Opcode::LoadConst,
            Opcode::LoadConst,
            Opcode::BuildTemplate,
            Opcode::ReturnValue,
        ]
    );
}

#[test]
fn translates_build_slice_with_two_operands() {
    let code = test_code(vec![
        op("LOAD_CONST"),
        0,
        op("LOAD_CONST"),
        0,
        op("BUILD_SLICE"),
        2,
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
            Opcode::BuildSlice,
            Opcode::ReturnValue,
        ]
    );
    assert_eq!(translated.instructions[2].arg, Some(2));
}

#[test]
fn translates_cpython_linetable_into_instruction_ranges() {
    let mut code = test_code(vec![op("LOAD_CONST"), 0, op("RETURN_VALUE"), 0]);
    // Entry 1: short form, one code unit, same line.
    // start_col=1 (0-based), end_col=3 (0-based, exclusive-ish).
    // Entry 2: one-line form (+1 line), one code unit.
    // start_col=4, end_col=8.
    code.linetable = vec![
        0x80, // marker | short-form code=0 | length=1 code unit
        0x12, // start=1, width=2 => end=3
        0xD8, // marker | one-line-form code=11 (line +1) | length=1
        0x04, // start col
        0x08, // end col
    ];

    let mut heap = Heap::new();
    let translated = translate_code(&code, &mut heap).expect("translation should succeed");
    assert_eq!(translated.locations.len(), translated.instructions.len());

    let first = translated.locations[0];
    assert_eq!(first.line, 1);
    assert_eq!(first.column, 2);
    assert_eq!(first.end_line, 1);
    assert_eq!(first.end_column, 4);

    let second = translated.locations[1];
    assert_eq!(second.line, 2);
    assert_eq!(second.column, 5);
    assert_eq!(second.end_line, 2);
    assert_eq!(second.end_column, 9);
}

#[test]
fn translates_exception_table_and_with_except_opcodes() {
    let mut code = test_code(vec![
        op("LOAD_CONST"),
        0,
        op("LOAD_CONST"),
        0,
        op("LOAD_CONST"),
        0,
        op("LOAD_CONST"),
        0,
        op("LOAD_CONST"),
        0,
        op("WITH_EXCEPT_START"),
        0,
        op("PUSH_EXC_INFO"),
        0,
        op("POP_EXCEPT"),
        0,
        op("RERAISE"),
        1,
    ]);
    code.exceptiontable = vec![0x80, 8, 2, 1];

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
            Opcode::LoadConst,
            Opcode::LoadConst,
            Opcode::LoadConst,
            Opcode::WithExceptStart,
            Opcode::PushExcInfo,
            Opcode::PopExcept,
            Opcode::Reraise,
        ]
    );
    assert_eq!(translated.exception_handlers.len(), 1);
    let handler = translated.exception_handlers[0];
    assert_eq!(handler.start, 0);
    assert_eq!(handler.end, 8);
    assert_eq!(handler.target, 2);
    assert_eq!(handler.depth, 0);
    assert!(handler.push_lasti);
}
