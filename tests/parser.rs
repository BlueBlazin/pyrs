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
fn parses_subscript_assignment_statement() {
    let module = parser::parse_module("x[0] = 1").expect("parse should succeed");
    match &module.body[0] {
        Stmt::AssignSubscript { .. } => {}
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_attribute_assignment_statement() {
    let module = parser::parse_module("mod.x = 1").expect("parse should succeed");
    match &module.body[0] {
        Stmt::AssignAttr { name, .. } => {
            assert_eq!(name, "x");
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_augmented_assignment() {
    let module = parser::parse_module("x += 1").expect("parse should succeed");
    match &module.body[0] {
        Stmt::AugAssign { .. } => {}
        other => panic!("unexpected stmt: {other:?}"),
    }
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
fn parses_mod_expression() {
    let module = parser::parse_module("5 % 2").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::Mod);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_floor_div_expression() {
    let module = parser::parse_module("5 // 2").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::FloorDiv);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
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
fn parses_not_equal_expression() {
    let module = parser::parse_module("1 != 2").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::Ne);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_relational_expressions() {
    let module =
        parser::parse_module("1 <= 2\n3 > 2\n4 >= 4").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::Le);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
    match &module.body[1] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::Gt);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
    match &module.body[2] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::Ge);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_in_expression() {
    let module = parser::parse_module("'a' in 'cat'").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::In);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_is_expression() {
    let module = parser::parse_module("x is y\nx is not y").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::Is);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
    match &module.body[1] {
        Stmt::Expr(Expr::Binary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::BinaryOp::IsNot);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_if_expression() {
    let module = parser::parse_module("1 if x else 2").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::IfExpr { .. }) => {}
        other => panic!("unexpected stmt: {other:?}"),
    }
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
fn parses_unary_plus() {
    let module = parser::parse_module("+1").expect("parse should succeed");
    let expected = Stmt::Expr(Expr::Unary {
        op: pyrs::ast::UnaryOp::Pos,
        operand: Box::new(Expr::Constant(Constant::Int(1))),
    });
    assert_eq!(module.body, vec![expected]);
}

#[test]
fn parses_lambda_expression() {
    let module = parser::parse_module("lambda x: x + 1").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Lambda { params, body }) => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "x");
            assert!(params[0].default.is_none());
            match &**body {
                Expr::Binary { .. } => {}
                other => panic!("unexpected body: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_raise_statement() {
    let module = parser::parse_module("raise ValueError").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Raise { value: Some(Expr::Name(name)) } => {
            assert_eq!(name, "ValueError");
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_try_except_statement() {
    let source = "try:\n  pass\nexcept ValueError as err:\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::Try {
            handlers,
            orelse,
            finalbody,
            ..
        } => {
            assert!(orelse.is_empty());
            assert!(finalbody.is_empty());
            assert_eq!(handlers.len(), 1);
            let handler = &handlers[0];
            match &handler.type_expr {
                Some(Expr::Name(name)) => assert_eq!(name, "ValueError"),
                other => panic!("unexpected handler type: {other:?}"),
            }
            assert_eq!(handler.name.as_deref(), Some("err"));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_try_finally_statement() {
    let source = "try:\n  pass\nfinally:\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::Try {
            handlers,
            finalbody,
            ..
        } => {
            assert!(handlers.is_empty());
            assert_eq!(finalbody, &vec![Stmt::Pass]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_try_except_finally_statement() {
    let source = "try:\n  pass\nexcept Exception:\n  pass\nfinally:\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::Try {
            handlers,
            finalbody,
            ..
        } => {
            assert_eq!(handlers.len(), 1);
            assert_eq!(finalbody, &vec![Stmt::Pass]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_class_definition() {
    let source = "class Foo:\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::ClassDef { name, bases, body } => {
            assert_eq!(name, "Foo");
            assert!(bases.is_empty());
            assert_eq!(body, &vec![Stmt::Pass]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_class_definition_with_base() {
    let source = "class Child(Base):\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::ClassDef { name, bases, body } => {
            assert_eq!(name, "Child");
            assert_eq!(bases.len(), 1);
            match &bases[0] {
                Expr::Name(name) => assert_eq!(name, "Base"),
                other => panic!("unexpected base: {other:?}"),
            }
            assert_eq!(body, &vec![Stmt::Pass]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_assert_statement() {
    let module = parser::parse_module("assert x, 'bad'").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Assert { test, message } => {
            match test {
                Expr::Name(name) => assert_eq!(name, "x"),
                other => panic!("unexpected test: {other:?}"),
            }
            match message {
                Some(Expr::Constant(Constant::Str(value))) => assert_eq!(value, "bad"),
                other => panic!("unexpected message: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
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
fn parses_function_definition_and_return() {
    let source = "def add(a, b):\n    return a + b\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::FunctionDef { name, params, body } => {
            assert_eq!(name, "add");
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].name, "a");
            assert!(params[0].default.is_none());
            assert_eq!(params[1].name, "b");
            assert!(params[1].default.is_none());
            match &body[0] {
                Stmt::Return { value } => {
                    assert!(value.is_some());
                }
                other => panic!("unexpected stmt: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_function_definition_with_defaults() {
    let source = "def add(a, b=1):\n    return a + b\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::FunctionDef { params, .. } => {
            assert_eq!(params.len(), 2);
            assert!(params[0].default.is_none());
            match &params[1].default {
                Some(Expr::Constant(Constant::Int(value))) => assert_eq!(*value, 1),
                other => panic!("unexpected default: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_lambda_with_default() {
    let module = parser::parse_module("lambda x=1: x").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Lambda { params, .. }) => {
            assert_eq!(params.len(), 1);
            match &params[0].default {
                Some(Expr::Constant(Constant::Int(value))) => assert_eq!(*value, 1),
                other => panic!("unexpected default: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_call_expression() {
    let module = parser::parse_module("add(1, 2)").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Call { func, args }) => {
            assert_eq!(**func, Expr::Name("add".to_string()));
            assert_eq!(
                args,
                &vec![
                    pyrs::ast::CallArg::Positional(Expr::Constant(Constant::Int(1))),
                    pyrs::ast::CallArg::Positional(Expr::Constant(Constant::Int(2))),
                ]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_call_with_keywords() {
    let module = parser::parse_module("add(a=1, b=2)").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Call { args, .. }) => {
            assert_eq!(
                args,
                &vec![
                    pyrs::ast::CallArg::Keyword {
                        name: "a".to_string(),
                        value: Expr::Constant(Constant::Int(1)),
                    },
                    pyrs::ast::CallArg::Keyword {
                        name: "b".to_string(),
                        value: Expr::Constant(Constant::Int(2)),
                    },
                ]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_list_literal_and_subscript() {
    let module = parser::parse_module("[1, 2][0]").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Subscript { value, index }) => {
            assert_eq!(
                **value,
                Expr::List(vec![
                    Expr::Constant(Constant::Int(1)),
                    Expr::Constant(Constant::Int(2))
                ])
            );
            assert_eq!(**index, Expr::Constant(Constant::Int(0)));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_attribute_expression() {
    let module = parser::parse_module("mod.value").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Attribute { value, name }) => {
            assert_eq!(**value, Expr::Name("mod".to_string()));
            assert_eq!(name, "value");
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_import_statement() {
    let module = parser::parse_module("import math, sys").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Import { names } => {
            assert_eq!(
                names,
                &vec![
                    pyrs::ast::ImportAlias {
                        name: "math".to_string(),
                        asname: None
                    },
                    pyrs::ast::ImportAlias {
                        name: "sys".to_string(),
                        asname: None
                    }
                ]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_global_statement() {
    let module = parser::parse_module("global x, y").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Global { names } => {
            assert_eq!(names, &vec!["x".to_string(), "y".to_string()]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_from_import_statement() {
    let module = parser::parse_module("from mod import a, b").expect("parse should succeed");
    match &module.body[0] {
        Stmt::ImportFrom { module, names } => {
            assert_eq!(module, "mod");
            assert_eq!(
                names,
                &vec![
                    pyrs::ast::ImportAlias {
                        name: "a".to_string(),
                        asname: None
                    },
                    pyrs::ast::ImportAlias {
                        name: "b".to_string(),
                        asname: None
                    }
                ]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_import_alias() {
    let module = parser::parse_module("import math as m").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Import { names } => {
            assert_eq!(
                names,
                &vec![pyrs::ast::ImportAlias {
                    name: "math".to_string(),
                    asname: Some("m".to_string())
                }]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_from_import_alias() {
    let module = parser::parse_module("from mod import value as v")
        .expect("parse should succeed");
    match &module.body[0] {
        Stmt::ImportFrom { names, .. } => {
            assert_eq!(
                names,
                &vec![pyrs::ast::ImportAlias {
                    name: "value".to_string(),
                    asname: Some("v".to_string())
                }]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_slice_subscript() {
    let module = parser::parse_module("x[1:3]").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Subscript { value, index }) => {
            assert_eq!(**value, Expr::Name("x".to_string()));
            match &**index {
                Expr::Slice { lower, upper, step } => {
                    assert_eq!(
                        lower.as_deref(),
                        Some(&Expr::Constant(Constant::Int(1)))
                    );
                    assert_eq!(
                        upper.as_deref(),
                        Some(&Expr::Constant(Constant::Int(3)))
                    );
                    assert!(step.is_none());
                }
                other => panic!("unexpected index: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_slice_with_step() {
    let module = parser::parse_module("x[::2]").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Subscript { index, .. }) => match &**index {
            Expr::Slice { lower, upper, step } => {
                assert!(lower.is_none());
                assert!(upper.is_none());
                assert_eq!(
                    step.as_deref(),
                    Some(&Expr::Constant(Constant::Int(2)))
                );
            }
            other => panic!("unexpected index: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_tuple_literal() {
    let module = parser::parse_module("(1, 2)").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Tuple(values)) => {
            assert_eq!(
                values,
                &vec![
                    Expr::Constant(Constant::Int(1)),
                    Expr::Constant(Constant::Int(2)),
                ]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_dict_literal() {
    let module = parser::parse_module("{'a': 1, 'b': 2}").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Dict(entries)) => {
            assert_eq!(entries.len(), 2);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_for_loop() {
    let source = "for i in [1, 2]:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::For {
            target,
            iter,
            body,
            orelse,
        } => {
            assert_eq!(target, "i");
            assert_eq!(
                *iter,
                Expr::List(vec![
                    Expr::Constant(Constant::Int(1)),
                    Expr::Constant(Constant::Int(2))
                ])
            );
            assert_eq!(body, &vec![Stmt::Pass]);
            assert!(orelse.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_break_and_continue() {
    let source = "while 1:\n    break\n    continue\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::While { body, orelse, .. } => {
            assert_eq!(body, &vec![Stmt::Break, Stmt::Continue]);
            assert!(orelse.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_while_else_clause() {
    let source = "while 0:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::While { orelse, .. } => {
            assert_eq!(orelse, &vec![Stmt::Pass]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_for_else_clause() {
    let source = "for i in [1]:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::For { orelse, .. } => {
            assert_eq!(orelse, &vec![Stmt::Pass]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_boolean_operators() {
    let module = parser::parse_module("a or b and c").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::BoolOp { op, left, right }) => {
            assert_eq!(*op, pyrs::ast::BoolOp::Or);
            assert_eq!(**left, Expr::Name("a".to_string()));
            match &**right {
                Expr::BoolOp { op, .. } => assert_eq!(*op, pyrs::ast::BoolOp::And),
                _ => panic!("expected nested and"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_not_operator() {
    let module = parser::parse_module("not False").expect("parse should succeed");
    match &module.body[0] {
        Stmt::Expr(Expr::Unary { op, .. }) => {
            assert_eq!(*op, pyrs::ast::UnaryOp::Not);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
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

#[test]
fn parses_if_elif_else_statement() {
    let source = "if x:\n    pass\nelif y:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &module.body[0] {
        Stmt::If { orelse, .. } => {
            assert_eq!(orelse.len(), 1);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}
