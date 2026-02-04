use pyrs::ast::{Constant, Expr, Stmt};
use pyrs::parser;

#[test]
fn parses_pass_statement() {
    let module = parser::parse_module("pass\n").expect("parse should succeed");
    assert_eq!(module.body, vec![Stmt::Pass]);
}

#[test]
fn parses_name_expression_statement() {
    let module = parser::parse_module("spam").expect("parse should succeed");
    assert_eq!(
        module.body,
        vec![Stmt::Expr(Expr::Name("spam".to_string()))]
    );
}

#[test]
fn parses_integer_literal() {
    let module = parser::parse_module("42").expect("parse should succeed");
    assert_eq!(
        module.body,
        vec![Stmt::Expr(Expr::Constant(Constant::Int(42)))]
    );
}

#[test]
fn parses_string_literal() {
    let module = parser::parse_module("'hi'").expect("parse should succeed");
    assert_eq!(
        module.body,
        vec![Stmt::Expr(Expr::Constant(Constant::Str("hi".to_string())))]
    );
}

#[test]
fn rejects_unknown_token() {
    let err = parser::parse_module("@").expect_err("parse should fail");
    assert_eq!(err.offset, 0);
}

#[test]
fn parses_inline_if_statement() {
    let module = parser::parse_module("if x: pass\n").expect("parse should succeed");
    match &module.body[0] {
        Stmt::If { test, body, orelse } => {
            assert_eq!(test, &Expr::Name("x".to_string()));
            assert_eq!(body, &vec![Stmt::Pass]);
            assert!(orelse.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_block_if_else_statement() {
    let source = "if x:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::If { test, body, orelse } => {
            assert_eq!(test, &Expr::Name("x".to_string()));
            assert_eq!(body, &vec![Stmt::Pass]);
            assert_eq!(orelse, &vec![Stmt::Pass]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}
