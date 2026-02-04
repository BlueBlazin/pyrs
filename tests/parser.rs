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
fn parses_assignment_statement() {
    let module = parser::parse_module("x = 1").expect("parse should succeed");
    assert_eq!(
        module.body,
        vec![Stmt::Assign {
            target: "x".to_string(),
            value: Expr::Constant(Constant::Int(1)),
        }]
    );
}

#[test]
fn parses_binary_expression_with_precedence() {
    let module = parser::parse_module("1 + 2 * 3").expect("parse should succeed");
    let expected = Stmt::Expr(Expr::Binary {
        left: Box::new(Expr::Constant(Constant::Int(1))),
        op: pyrs::ast::BinaryOp::Add,
        right: Box::new(Expr::Binary {
            left: Box::new(Expr::Constant(Constant::Int(2))),
            op: pyrs::ast::BinaryOp::Mul,
            right: Box::new(Expr::Constant(Constant::Int(3))),
        }),
    });
    assert_eq!(module.body, vec![expected]);
}

#[test]
fn parses_comparison_expression() {
    let module = parser::parse_module("1 < 2 + 3").expect("parse should succeed");
    let expected = Stmt::Expr(Expr::Binary {
        left: Box::new(Expr::Constant(Constant::Int(1))),
        op: pyrs::ast::BinaryOp::Lt,
        right: Box::new(Expr::Binary {
            left: Box::new(Expr::Constant(Constant::Int(2))),
            op: pyrs::ast::BinaryOp::Add,
            right: Box::new(Expr::Constant(Constant::Int(3))),
        }),
    });
    assert_eq!(module.body, vec![expected]);
}

#[test]
fn parses_unary_minus() {
    let module = parser::parse_module("-1").expect("parse should succeed");
    let expected = Stmt::Expr(Expr::Unary {
        op: pyrs::ast::UnaryOp::Neg,
        operand: Box::new(Expr::Constant(Constant::Int(1))),
    });
    assert_eq!(module.body, vec![expected]);
}

#[test]
fn parses_boolean_and_none_literals() {
    let module = parser::parse_module("True\nFalse\nNone").expect("parse should succeed");
    assert_eq!(
        module.body,
        vec![
            Stmt::Expr(Expr::Constant(Constant::Bool(true))),
            Stmt::Expr(Expr::Constant(Constant::Bool(false))),
            Stmt::Expr(Expr::Constant(Constant::None)),
        ]
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
