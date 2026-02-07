use pyrs::ast::{
    AssignTarget, ComprehensionClause, Constant, DictEntry, Expr, ExprKind, FloatLiteral,
    MatchCase, Module, Parameter, Pattern, Span, Stmt, StmtKind,
};
use pyrs::parser;

fn spanned_expr(node: ExprKind) -> Expr {
    Expr {
        node,
        span: Span::unknown(),
    }
}

fn spanned_stmt(node: StmtKind) -> Stmt {
    Stmt {
        node,
        span: Span::unknown(),
    }
}

fn strip_module(module: &Module) -> Vec<Stmt> {
    module.body.iter().map(strip_stmt).collect()
}

fn strip_stmt(stmt: &Stmt) -> Stmt {
    let node = match &stmt.node {
        StmtKind::Pass => StmtKind::Pass,
        StmtKind::Expr(expr) => StmtKind::Expr(strip_expr(expr)),
        StmtKind::If { test, body, orelse } => StmtKind::If {
            test: strip_expr(test),
            body: body.iter().map(strip_stmt).collect(),
            orelse: orelse.iter().map(strip_stmt).collect(),
        },
        StmtKind::Assign { targets, value } => StmtKind::Assign {
            targets: targets.iter().map(strip_target).collect(),
            value: strip_expr(value),
        },
        StmtKind::AugAssign { target, op, value } => StmtKind::AugAssign {
            target: strip_target(target),
            op: op.clone(),
            value: strip_expr(value),
        },
        StmtKind::FunctionDef {
            name,
            type_params,
            is_async,
            posonly_params,
            params,
            vararg,
            kwarg,
            kwonly_params,
            returns,
            body,
        } => StmtKind::FunctionDef {
            name: name.clone(),
            type_params: type_params.clone(),
            is_async: *is_async,
            posonly_params: posonly_params.iter().map(strip_param).collect(),
            params: params.iter().map(strip_param).collect(),
            vararg: vararg.clone(),
            kwarg: kwarg.clone(),
            kwonly_params: kwonly_params.iter().map(strip_param).collect(),
            returns: returns.as_ref().map(strip_expr),
            body: body.iter().map(strip_stmt).collect(),
        },
        StmtKind::ClassDef {
            name,
            type_params,
            bases,
            metaclass,
            keywords,
            body,
        } => StmtKind::ClassDef {
            name: name.clone(),
            type_params: type_params.clone(),
            bases: bases.iter().map(strip_expr).collect(),
            metaclass: metaclass.as_ref().map(strip_expr),
            keywords: keywords
                .iter()
                .map(|(name, value)| (name.clone(), strip_expr(value)))
                .collect(),
            body: body.iter().map(strip_stmt).collect(),
        },
        StmtKind::Decorated { decorators, stmt } => StmtKind::Decorated {
            decorators: decorators.iter().map(strip_expr).collect(),
            stmt: Box::new(strip_stmt(stmt)),
        },
        StmtKind::Return { value } => StmtKind::Return {
            value: value.as_ref().map(strip_expr),
        },
        StmtKind::Raise { value, cause } => StmtKind::Raise {
            value: value.as_ref().map(strip_expr),
            cause: cause.as_ref().map(strip_expr),
        },
        StmtKind::Assert { test, message } => StmtKind::Assert {
            test: strip_expr(test),
            message: message.as_ref().map(strip_expr),
        },
        StmtKind::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => StmtKind::Try {
            body: body.iter().map(strip_stmt).collect(),
            handlers: handlers.iter().map(strip_handler).collect(),
            orelse: orelse.iter().map(strip_stmt).collect(),
            finalbody: finalbody.iter().map(strip_stmt).collect(),
        },
        StmtKind::While { test, body, orelse } => StmtKind::While {
            test: strip_expr(test),
            body: body.iter().map(strip_stmt).collect(),
            orelse: orelse.iter().map(strip_stmt).collect(),
        },
        StmtKind::For {
            is_async,
            target,
            iter,
            body,
            orelse,
        } => StmtKind::For {
            is_async: *is_async,
            target: strip_target(target),
            iter: strip_expr(iter),
            body: body.iter().map(strip_stmt).collect(),
            orelse: orelse.iter().map(strip_stmt).collect(),
        },
        StmtKind::Import { names } => StmtKind::Import {
            names: names.clone(),
        },
        StmtKind::ImportFrom {
            module,
            names,
            level,
        } => StmtKind::ImportFrom {
            module: module.clone(),
            names: names.clone(),
            level: *level,
        },
        StmtKind::Global { names } => StmtKind::Global {
            names: names.clone(),
        },
        StmtKind::Nonlocal { names } => StmtKind::Nonlocal {
            names: names.clone(),
        },
        StmtKind::AnnAssign {
            target,
            annotation,
            value,
        } => StmtKind::AnnAssign {
            target: strip_target(target),
            annotation: strip_expr(annotation),
            value: value.as_ref().map(strip_expr),
        },
        StmtKind::With {
            is_async,
            context,
            target,
            body,
        } => StmtKind::With {
            is_async: *is_async,
            context: strip_expr(context),
            target: target.as_ref().map(strip_target),
            body: body.iter().map(strip_stmt).collect(),
        },
        StmtKind::Match { subject, cases } => StmtKind::Match {
            subject: strip_expr(subject),
            cases: cases.iter().map(strip_case).collect(),
        },
        StmtKind::Delete { targets } => StmtKind::Delete {
            targets: targets.iter().map(strip_target).collect(),
        },
        StmtKind::Break => StmtKind::Break,
        StmtKind::Continue => StmtKind::Continue,
    };
    spanned_stmt(node)
}

fn strip_expr(expr: &Expr) -> Expr {
    let node = match &expr.node {
        ExprKind::Name(name) => ExprKind::Name(name.clone()),
        ExprKind::Constant(constant) => ExprKind::Constant(constant.clone()),
        ExprKind::Binary { left, op, right } => ExprKind::Binary {
            left: Box::new(strip_expr(left)),
            op: op.clone(),
            right: Box::new(strip_expr(right)),
        },
        ExprKind::Unary { op, operand } => ExprKind::Unary {
            op: op.clone(),
            operand: Box::new(strip_expr(operand)),
        },
        ExprKind::Call { func, args } => ExprKind::Call {
            func: Box::new(strip_expr(func)),
            args: args
                .iter()
                .map(|arg| match arg {
                    pyrs::ast::CallArg::Positional(expr) => {
                        pyrs::ast::CallArg::Positional(strip_expr(expr))
                    }
                    pyrs::ast::CallArg::Keyword { name, value } => pyrs::ast::CallArg::Keyword {
                        name: name.clone(),
                        value: strip_expr(value),
                    },
                    pyrs::ast::CallArg::Star(expr) => pyrs::ast::CallArg::Star(strip_expr(expr)),
                    pyrs::ast::CallArg::DoubleStar(expr) => {
                        pyrs::ast::CallArg::DoubleStar(strip_expr(expr))
                    }
                })
                .collect(),
        },
        ExprKind::List(values) => ExprKind::List(values.iter().map(strip_expr).collect()),
        ExprKind::Tuple(values) => ExprKind::Tuple(values.iter().map(strip_expr).collect()),
        ExprKind::Dict(entries) => ExprKind::Dict(
            entries
                .iter()
                .map(|entry| match entry {
                    DictEntry::Pair(key, value) => {
                        DictEntry::Pair(strip_expr(key), strip_expr(value))
                    }
                    DictEntry::Unpack(value) => DictEntry::Unpack(strip_expr(value)),
                })
                .collect(),
        ),
        ExprKind::Subscript { value, index } => ExprKind::Subscript {
            value: Box::new(strip_expr(value)),
            index: Box::new(strip_expr(index)),
        },
        ExprKind::Attribute { value, name } => ExprKind::Attribute {
            value: Box::new(strip_expr(value)),
            name: name.clone(),
        },
        ExprKind::BoolOp { op, left, right } => ExprKind::BoolOp {
            op: op.clone(),
            left: Box::new(strip_expr(left)),
            right: Box::new(strip_expr(right)),
        },
        ExprKind::IfExpr { test, body, orelse } => ExprKind::IfExpr {
            test: Box::new(strip_expr(test)),
            body: Box::new(strip_expr(body)),
            orelse: Box::new(strip_expr(orelse)),
        },
        ExprKind::NamedExpr { target, value } => ExprKind::NamedExpr {
            target: target.clone(),
            value: Box::new(strip_expr(value)),
        },
        ExprKind::Lambda {
            posonly_params,
            params,
            vararg,
            kwarg,
            kwonly_params,
            body,
        } => ExprKind::Lambda {
            posonly_params: posonly_params.iter().map(strip_param).collect(),
            params: params.iter().map(strip_param).collect(),
            vararg: vararg.clone(),
            kwarg: kwarg.clone(),
            kwonly_params: kwonly_params.iter().map(strip_param).collect(),
            body: Box::new(strip_expr(body)),
        },
        ExprKind::Await { value } => ExprKind::Await {
            value: Box::new(strip_expr(value)),
        },
        ExprKind::ListComp { elt, clauses } => ExprKind::ListComp {
            elt: Box::new(strip_expr(elt)),
            clauses: clauses.iter().map(strip_comp_clause).collect(),
        },
        ExprKind::DictComp {
            key,
            value,
            clauses,
        } => ExprKind::DictComp {
            key: Box::new(strip_expr(key)),
            value: Box::new(strip_expr(value)),
            clauses: clauses.iter().map(strip_comp_clause).collect(),
        },
        ExprKind::GeneratorExp { elt, clauses } => ExprKind::GeneratorExp {
            elt: Box::new(strip_expr(elt)),
            clauses: clauses.iter().map(strip_comp_clause).collect(),
        },
        ExprKind::Yield { value } => ExprKind::Yield {
            value: value.as_ref().map(|expr| Box::new(strip_expr(expr))),
        },
        ExprKind::YieldFrom { value } => ExprKind::YieldFrom {
            value: Box::new(strip_expr(value)),
        },
        ExprKind::Slice { lower, upper, step } => ExprKind::Slice {
            lower: lower.as_ref().map(|expr| Box::new(strip_expr(expr))),
            upper: upper.as_ref().map(|expr| Box::new(strip_expr(expr))),
            step: step.as_ref().map(|expr| Box::new(strip_expr(expr))),
        },
    };
    spanned_expr(node)
}

fn strip_param(param: &Parameter) -> Parameter {
    Parameter {
        name: param.name.clone(),
        default: param
            .default
            .as_ref()
            .map(|expr| Box::new(strip_expr(expr))),
        annotation: param
            .annotation
            .as_ref()
            .map(|expr| Box::new(strip_expr(expr))),
    }
}

fn strip_handler(handler: &pyrs::ast::ExceptHandler) -> pyrs::ast::ExceptHandler {
    pyrs::ast::ExceptHandler {
        type_expr: handler.type_expr.as_ref().map(strip_expr),
        name: handler.name.clone(),
        is_star: handler.is_star,
        body: handler.body.iter().map(strip_stmt).collect(),
    }
}

fn strip_case(case: &MatchCase) -> MatchCase {
    MatchCase {
        pattern: strip_pattern(&case.pattern),
        guard: case.guard.as_ref().map(strip_expr),
        body: case.body.iter().map(strip_stmt).collect(),
    }
}

fn strip_pattern(pattern: &Pattern) -> Pattern {
    match pattern {
        Pattern::Wildcard => Pattern::Wildcard,
        Pattern::Capture(name) => Pattern::Capture(name.clone()),
        Pattern::Constant(value) => Pattern::Constant(value.clone()),
        Pattern::Value(expr) => Pattern::Value(strip_expr(expr)),
        Pattern::Sequence(items) => Pattern::Sequence(items.iter().map(strip_pattern).collect()),
        Pattern::Mapping { entries, rest } => Pattern::Mapping {
            entries: entries
                .iter()
                .map(|(key, value)| (strip_expr(key), strip_pattern(value)))
                .collect(),
            rest: rest.clone(),
        },
        Pattern::Class {
            class,
            positional,
            keywords,
        } => Pattern::Class {
            class: strip_expr(class),
            positional: positional.iter().map(strip_pattern).collect(),
            keywords: keywords
                .iter()
                .map(|(name, value)| (name.clone(), strip_pattern(value)))
                .collect(),
        },
        Pattern::Or(options) => Pattern::Or(options.iter().map(strip_pattern).collect()),
        Pattern::As { pattern, name } => Pattern::As {
            pattern: Box::new(strip_pattern(pattern)),
            name: name.clone(),
        },
        Pattern::Star(name) => Pattern::Star(name.clone()),
    }
}

fn strip_comp_clause(clause: &ComprehensionClause) -> ComprehensionClause {
    ComprehensionClause {
        is_async: clause.is_async,
        target: strip_target(&clause.target),
        iter: strip_expr(&clause.iter),
        ifs: clause.ifs.iter().map(strip_expr).collect(),
    }
}

fn strip_target(target: &AssignTarget) -> AssignTarget {
    match target {
        AssignTarget::Name(name) => AssignTarget::Name(name.clone()),
        AssignTarget::Starred(item) => AssignTarget::Starred(Box::new(strip_target(item))),
        AssignTarget::Tuple(items) => AssignTarget::Tuple(items.iter().map(strip_target).collect()),
        AssignTarget::List(items) => AssignTarget::List(items.iter().map(strip_target).collect()),
        AssignTarget::Subscript { value, index } => AssignTarget::Subscript {
            value: Box::new(strip_expr(value)),
            index: Box::new(strip_expr(index)),
        },
        AssignTarget::Attribute { value, name } => AssignTarget::Attribute {
            value: Box::new(strip_expr(value)),
            name: name.clone(),
        },
    }
}

#[test]
fn parses_pass_statement() {
    let module = parser::parse_module("pass\n").expect("parse should succeed");
    assert_eq!(strip_module(&module), vec![spanned_stmt(StmtKind::Pass)]);
}

#[test]
fn parses_name_expression_statement() {
    let module = parser::parse_module("spam").expect("parse should succeed");
    assert_eq!(
        strip_module(&module),
        vec![spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Name(
            "spam".to_string()
        ))))]
    );
}

#[test]
fn parses_assignment_statement() {
    let module = parser::parse_module("x = 1").expect("parse should succeed");
    assert_eq!(
        strip_module(&module),
        vec![spanned_stmt(StmtKind::Assign {
            targets: vec![AssignTarget::Name("x".to_string())],
            value: spanned_expr(ExprKind::Constant(Constant::Int(1))),
        })]
    );
}

#[test]
fn parses_chained_assignment_statement() {
    let module = parser::parse_module("a = b = 1").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { targets, value } => {
            assert_eq!(targets.len(), 2);
            assert_eq!(targets[0], AssignTarget::Name("a".to_string()));
            assert_eq!(targets[1], AssignTarget::Name("b".to_string()));
            assert_eq!(*value, spanned_expr(ExprKind::Constant(Constant::Int(1))));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_destructuring_assignment_statement() {
    let module = parser::parse_module("a, b = (1, 2)").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { targets, .. } => {
            assert_eq!(targets.len(), 1);
            let AssignTarget::Tuple(items) = &targets[0] else {
                panic!("unexpected target: {:?}", targets[0]);
            };
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], AssignTarget::Name("a".to_string()));
            assert_eq!(items[1], AssignTarget::Name("b".to_string()));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_subscript_assignment_statement() {
    let module = parser::parse_module("x[0] = 1").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { targets, .. }
            if matches!(targets.as_slice(), [AssignTarget::Subscript { .. }]) => {}
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_attribute_assignment_statement() {
    let module = parser::parse_module("mod.x = 1").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { targets, .. } => match targets.as_slice() {
            [AssignTarget::Attribute { name, .. }] => assert_eq!(name, "x"),
            _ => panic!("unexpected targets: {targets:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_delete_statement() {
    let module = parser::parse_module("del x, y").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Delete { targets } => {
            assert_eq!(
                targets,
                &vec![
                    AssignTarget::Name("x".to_string()),
                    AssignTarget::Name("y".to_string())
                ]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_augmented_assignment() {
    let module = parser::parse_module("x += 1").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::AugAssign { .. } => {}
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_augmented_assignment_variants() {
    let module =
        parser::parse_module("x /= 2\nx %= 2\nx //= 3\nx **= 2\n").expect("parse should succeed");
    assert_eq!(strip_module(&module).len(), 4);
    for stmt in &strip_module(&module) {
        match &stmt.node {
            StmtKind::AugAssign { .. } => {}
            other => panic!("unexpected stmt: {other:?}"),
        }
    }
}

#[test]
fn parses_parenthesized_attribute_assignment_target() {
    let module = parser::parse_module("(1.0).__class__ = x\n").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { targets, .. } => match targets.first() {
            Some(AssignTarget::Attribute { name, .. }) => assert_eq!(name, "__class__"),
            other => panic!("unexpected target: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_with_statement() {
    let source = "with mgr as value:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::With {
            context, target, ..
        } => {
            assert_eq!(&context.node, &ExprKind::Name("mgr".to_string()));
            let target = target.as_ref().expect("with target");
            assert_eq!(*target, AssignTarget::Name("value".to_string()));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_with_multiple_items() {
    let source = "with a() as x, b() as y:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let stripped = strip_module(&module);
    match &stripped[0].node {
        StmtKind::With {
            target: Some(AssignTarget::Name(name)),
            body,
            ..
        } => {
            assert_eq!(name, "x");
            match &body[0].node {
                StmtKind::With {
                    target: Some(AssignTarget::Name(name)),
                    ..
                } => assert_eq!(name, "y"),
                other => panic!("unexpected nested with: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_with_starred_target() {
    let source = "with ctx as (a, *b, c):\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::With {
            target: Some(AssignTarget::Tuple(items)),
            ..
        } => {
            assert!(matches!(items[1], AssignTarget::Starred(_)));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_for_starred_target() {
    let source = "for a, *b in [(1, 2, 3)]:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::For {
            target: AssignTarget::Tuple(items),
            ..
        } => {
            assert!(matches!(items[1], AssignTarget::Starred(_)));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_binary_expression_with_precedence() {
    let module = parser::parse_module("1 + 2 * 3").expect("parse should succeed");
    let expected = spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Binary {
        left: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(1)))),
        op: pyrs::ast::BinaryOp::Add,
        right: Box::new(spanned_expr(ExprKind::Binary {
            left: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(2)))),
            op: pyrs::ast::BinaryOp::Mul,
            right: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(3)))),
        })),
    })));
    assert_eq!(strip_module(&module), vec![expected]);
}

#[test]
fn parses_mod_expression() {
    let module = parser::parse_module("5 % 2").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Mod);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_floor_div_expression() {
    let module = parser::parse_module("5 // 2").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::FloorDiv);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_true_div_expression() {
    let module = parser::parse_module("5 / 2").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Div);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_power_expression() {
    let module = parser::parse_module("2 ** 3 ** 2").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, left, right } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Pow);
                assert_eq!(&left.node, &ExprKind::Constant(Constant::Int(2)));
                match &right.node {
                    ExprKind::Binary {
                        op: rhs_op,
                        left: rhs_left,
                        right: rhs_right,
                    } => {
                        assert_eq!(*rhs_op, pyrs::ast::BinaryOp::Pow);
                        assert_eq!(&rhs_left.node, &ExprKind::Constant(Constant::Int(3)));
                        assert_eq!(&rhs_right.node, &ExprKind::Constant(Constant::Int(2)));
                    }
                    other => panic!("unexpected rhs: {other:?}"),
                }
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_comparison_expression() {
    let module = parser::parse_module("1 < 2 + 3").expect("parse should succeed");
    let expected = spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Binary {
        left: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(1)))),
        op: pyrs::ast::BinaryOp::Lt,
        right: Box::new(spanned_expr(ExprKind::Binary {
            left: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(2)))),
            op: pyrs::ast::BinaryOp::Add,
            right: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(3)))),
        })),
    })));
    assert_eq!(strip_module(&module), vec![expected]);
}

#[test]
fn parses_not_equal_expression() {
    let module = parser::parse_module("1 != 2").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Ne);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_relational_expressions() {
    let module = parser::parse_module("1 <= 2\n3 > 2\n4 >= 4").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Le);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
    match &strip_module(&module)[1].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Gt);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
    match &strip_module(&module)[2].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Ge);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_in_expression() {
    let module = parser::parse_module("'a' in 'cat'").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::In);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_is_expression() {
    let module = parser::parse_module("x is y\nx is not y").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::Is);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
    match &strip_module(&module)[1].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Binary { op, .. } => {
                assert_eq!(*op, pyrs::ast::BinaryOp::IsNot);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_if_expression() {
    let module = parser::parse_module("1 if x else 2").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::IfExpr { .. } => {}
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_unary_minus() {
    let module = parser::parse_module("-1").expect("parse should succeed");
    let expected = spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Unary {
        op: pyrs::ast::UnaryOp::Neg,
        operand: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(1)))),
    })));
    assert_eq!(strip_module(&module), vec![expected]);
}

#[test]
fn parses_unary_plus() {
    let module = parser::parse_module("+1").expect("parse should succeed");
    let expected = spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Unary {
        op: pyrs::ast::UnaryOp::Pos,
        operand: Box::new(spanned_expr(ExprKind::Constant(Constant::Int(1)))),
    })));
    assert_eq!(strip_module(&module), vec![expected]);
}

#[test]
fn parses_lambda_expression() {
    let module = parser::parse_module("lambda x: x + 1").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Lambda {
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                body,
            } => {
                assert!(posonly_params.is_empty());
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "x");
                assert!(params[0].default.is_none());
                assert!(vararg.is_none());
                assert!(kwarg.is_none());
                assert!(kwonly_params.is_empty());
                match &body.node {
                    ExprKind::Binary { .. } => {}
                    other => panic!("unexpected body: {other:?}"),
                }
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_raise_statement() {
    let module = parser::parse_module("raise ValueError").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Raise {
            value: Some(expr),
            cause: None,
        } => match &expr.node {
            ExprKind::Name(name) => {
                assert_eq!(name, "ValueError");
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_raise_from_statement() {
    let module = parser::parse_module("raise ValueError from err").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Raise {
            value: Some(value),
            cause: Some(cause),
        } => {
            match &value.node {
                ExprKind::Name(name) => assert_eq!(name, "ValueError"),
                other => panic!("unexpected raise value: {other:?}"),
            }
            match &cause.node {
                ExprKind::Name(name) => assert_eq!(name, "err"),
                other => panic!("unexpected raise cause: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_try_except_statement() {
    let source = "try:\n  pass\nexcept ValueError as err:\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Try {
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
                Some(expr) => match &expr.node {
                    ExprKind::Name(name) => assert_eq!(name, "ValueError"),
                    other => panic!("unexpected handler type: {other:?}"),
                },
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
    match &strip_module(&module)[0].node {
        StmtKind::Try {
            handlers,
            finalbody,
            ..
        } => {
            assert!(handlers.is_empty());
            assert_eq!(finalbody, &vec![spanned_stmt(StmtKind::Pass)]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_try_except_finally_statement() {
    let source = "try:\n  pass\nexcept Exception:\n  pass\nfinally:\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Try {
            handlers,
            finalbody,
            ..
        } => {
            assert_eq!(handlers.len(), 1);
            assert_eq!(finalbody, &vec![spanned_stmt(StmtKind::Pass)]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_class_definition() {
    let source = "class Foo:\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ClassDef {
            name, bases, body, ..
        } => {
            assert_eq!(name, "Foo");
            assert!(bases.is_empty());
            assert_eq!(body, &vec![spanned_stmt(StmtKind::Pass)]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_class_definition_with_base() {
    let source = "class Child(Base):\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ClassDef {
            name, bases, body, ..
        } => {
            assert_eq!(name, "Child");
            assert_eq!(bases.len(), 1);
            match &bases[0].node {
                ExprKind::Name(name) => assert_eq!(name, "Base"),
                other => panic!("unexpected base: {other:?}"),
            }
            assert_eq!(body, &vec![spanned_stmt(StmtKind::Pass)]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_class_definition_with_keywords() {
    let source = "class Flag(Enum, boundary=STRICT, metaclass=Meta):\n  pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ClassDef {
            bases,
            metaclass,
            keywords,
            ..
        } => {
            assert_eq!(bases.len(), 1);
            match &bases[0].node {
                ExprKind::Name(name) => assert_eq!(name, "Enum"),
                other => panic!("unexpected base: {other:?}"),
            }
            match metaclass {
                Some(expr) => match &expr.node {
                    ExprKind::Name(name) => assert_eq!(name, "Meta"),
                    other => panic!("unexpected metaclass: {other:?}"),
                },
                None => panic!("missing metaclass"),
            }
            assert_eq!(keywords.len(), 1);
            assert_eq!(keywords[0].0, "boundary");
            match &keywords[0].1.node {
                ExprKind::Name(name) => assert_eq!(name, "STRICT"),
                other => panic!("unexpected class keyword value: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_assert_statement() {
    let module = parser::parse_module("assert x, 'bad'").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assert { test, message } => {
            match &test.node {
                ExprKind::Name(name) => assert_eq!(name, "x"),
                other => panic!("unexpected test: {other:?}"),
            }
            match message {
                Some(expr) => match &expr.node {
                    ExprKind::Constant(Constant::Str(value)) => assert_eq!(value, "bad"),
                    other => panic!("unexpected message: {other:?}"),
                },
                None => panic!("unexpected message: None"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_boolean_and_none_literals() {
    let module = parser::parse_module("True\nFalse\nNone").expect("parse should succeed");
    assert_eq!(
        strip_module(&module),
        vec![
            spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Constant(
                Constant::Bool(true)
            )))),
            spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Constant(
                Constant::Bool(false)
            )))),
            spanned_stmt(StmtKind::Expr(spanned_expr(ExprKind::Constant(
                Constant::None
            )))),
        ]
    );
}

#[test]
fn parses_function_definition_and_return() {
    let source = "def add(a, b):\n    return a + b\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef {
            name,
            posonly_params,
            params,
            vararg,
            kwarg,
            kwonly_params,
            body,
            ..
        } => {
            assert!(posonly_params.is_empty());
            assert_eq!(name, "add");
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].name, "a");
            assert!(params[0].default.is_none());
            assert_eq!(params[1].name, "b");
            assert!(params[1].default.is_none());
            assert!(vararg.is_none());
            assert!(kwarg.is_none());
            assert!(kwonly_params.is_empty());
            match &body[0].node {
                StmtKind::Return { value } => {
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
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef { params, .. } => {
            assert_eq!(params.len(), 2);
            assert!(params[0].default.is_none());
            match &params[1].default {
                Some(expr) => match &expr.node {
                    ExprKind::Constant(Constant::Int(value)) => assert_eq!(*value, 1),
                    other => panic!("unexpected default: {other:?}"),
                },
                other => panic!("unexpected default: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_lambda_with_default() {
    let module = parser::parse_module("lambda x=1: x").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Lambda { params, .. } => {
                assert_eq!(params.len(), 1);
                match &params[0].default {
                    Some(expr) => match &expr.node {
                        ExprKind::Constant(Constant::Int(value)) => assert_eq!(*value, 1),
                        other => panic!("unexpected default: {other:?}"),
                    },
                    other => panic!("unexpected default: {other:?}"),
                }
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_function_definition_with_varargs() {
    let source = "def collect(a, *rest, **kw):\n    return a\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef {
            posonly_params,
            params,
            vararg,
            kwarg,
            kwonly_params,
            ..
        } => {
            assert!(posonly_params.is_empty());
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "a");
            assert_eq!(
                vararg.as_ref().map(|param| param.name.as_str()),
                Some("rest")
            );
            assert_eq!(kwarg.as_ref().map(|param| param.name.as_str()), Some("kw"));
            assert!(kwonly_params.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_lambda_with_varargs() {
    let module = parser::parse_module("lambda *args, **kw: args").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Lambda {
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                ..
            } => {
                assert!(posonly_params.is_empty());
                assert!(params.is_empty());
                assert_eq!(
                    vararg.as_ref().map(|param| param.name.as_str()),
                    Some("args")
                );
                assert_eq!(kwarg.as_ref().map(|param| param.name.as_str()), Some("kw"));
                assert!(kwonly_params.is_empty());
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_lambda_with_annotations() {
    let module = parser::parse_module("lambda x: int: x").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Lambda { params, .. } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "x");
                assert!(params[0].annotation.is_some());
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_keyword_only_parameters() {
    let module =
        parser::parse_module("def f(a, *, b, c=2):\n    return a\n").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef {
            params,
            kwonly_params,
            vararg,
            kwarg,
            ..
        } => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "a");
            assert!(vararg.is_none());
            assert!(kwarg.is_none());
            assert_eq!(kwonly_params.len(), 2);
            assert_eq!(kwonly_params[0].name, "b");
            assert!(kwonly_params[0].default.is_none());
            assert_eq!(kwonly_params[1].name, "c");
            assert!(kwonly_params[1].default.is_some());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_required_kwonly_after_default() {
    let source = "def f(*, a=1, b):\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef { kwonly_params, .. } => {
            assert_eq!(kwonly_params.len(), 2);
            assert!(kwonly_params[0].default.is_some());
            assert!(kwonly_params[1].default.is_none());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_positional_only_parameters() {
    let module = parser::parse_module("def f(a, b=1, /, c=2):\n    return a\n")
        .expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef {
            posonly_params,
            params,
            kwonly_params,
            ..
        } => {
            assert_eq!(posonly_params.len(), 2);
            assert_eq!(posonly_params[0].name, "a");
            assert_eq!(posonly_params[1].name, "b");
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "c");
            assert!(kwonly_params.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_lambda_with_keyword_only() {
    let module = parser::parse_module("lambda *, b=3: b").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Lambda {
                params,
                kwonly_params,
                vararg,
                kwarg,
                ..
            } => {
                assert!(params.is_empty());
                assert!(vararg.is_none());
                assert!(kwarg.is_none());
                assert_eq!(kwonly_params.len(), 1);
                assert_eq!(kwonly_params[0].name, "b");
                assert!(kwonly_params[0].default.is_some());
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_lambda_with_positional_only() {
    let module = parser::parse_module("lambda a, /, b: b").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Lambda {
                posonly_params,
                params,
                kwonly_params,
                ..
            } => {
                assert_eq!(posonly_params.len(), 1);
                assert_eq!(posonly_params[0].name, "a");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "b");
                assert!(kwonly_params.is_empty());
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_call_expression() {
    let module = parser::parse_module("add(1, 2)").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Call { func, args } => {
                assert_eq!(&func.node, &ExprKind::Name("add".to_string()));
                assert_eq!(
                    args,
                    &vec![
                        pyrs::ast::CallArg::Positional(spanned_expr(ExprKind::Constant(
                            Constant::Int(1)
                        ))),
                        pyrs::ast::CallArg::Positional(spanned_expr(ExprKind::Constant(
                            Constant::Int(2)
                        ))),
                    ]
                );
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_call_with_keywords() {
    let module = parser::parse_module("add(a=1, b=2)").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Call { args, .. } => {
                assert_eq!(
                    args,
                    &vec![
                        pyrs::ast::CallArg::Keyword {
                            name: "a".to_string(),
                            value: spanned_expr(ExprKind::Constant(Constant::Int(1))),
                        },
                        pyrs::ast::CallArg::Keyword {
                            name: "b".to_string(),
                            value: spanned_expr(ExprKind::Constant(Constant::Int(2))),
                        },
                    ]
                );
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_call_with_star_args() {
    let module = parser::parse_module("f(*args, **kwargs)").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Call { args, .. } => {
                assert_eq!(
                    args,
                    &vec![
                        pyrs::ast::CallArg::Star(spanned_expr(ExprKind::Name("args".to_string()))),
                        pyrs::ast::CallArg::DoubleStar(spanned_expr(ExprKind::Name(
                            "kwargs".to_string()
                        ))),
                    ]
                );
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_list_literal_and_subscript() {
    let module = parser::parse_module("[1, 2][0]").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Subscript { value, index } => {
                assert_eq!(
                    &value.node,
                    &ExprKind::List(vec![
                        spanned_expr(ExprKind::Constant(Constant::Int(1))),
                        spanned_expr(ExprKind::Constant(Constant::Int(2)))
                    ])
                );
                assert_eq!(&index.node, &ExprKind::Constant(Constant::Int(0)));
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_attribute_expression() {
    let module = parser::parse_module("mod.value").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Attribute { value, name } => {
                assert_eq!(&value.node, &ExprKind::Name("mod".to_string()));
                assert_eq!(name, "value");
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_import_statement() {
    let module = parser::parse_module("import math, sys").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Import { names } => {
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
fn parses_dotted_import_statement() {
    let module = parser::parse_module("import pkg.sub").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Import { names } => {
            assert_eq!(names.len(), 1);
            assert_eq!(names[0].name, "pkg.sub");
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_global_statement() {
    let module = parser::parse_module("global x, y").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Global { names } => {
            assert_eq!(names, &vec!["x".to_string(), "y".to_string()]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_nonlocal_statement() {
    let module = parser::parse_module("nonlocal a, b").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Nonlocal { names } => {
            assert_eq!(names, &vec!["a".to_string(), "b".to_string()]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_annotated_assignment() {
    let module = parser::parse_module("x: int = 1").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::AnnAssign {
            target,
            annotation,
            value,
        } => {
            assert_eq!(target, &AssignTarget::Name("x".to_string()));
            assert_eq!(&annotation.node, &ExprKind::Name("int".to_string()));
            assert!(value.is_some());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_function_annotations() {
    let source = "def f(x: int) -> str:\n    return 'ok'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef {
            params, returns, ..
        } => {
            assert_eq!(params.len(), 1);
            assert!(params[0].annotation.is_some());
            match returns {
                Some(expr) => match &expr.node {
                    ExprKind::Name(name) => assert_eq!(name, "str"),
                    other => panic!("unexpected return annotation: {other:?}"),
                },
                None => panic!("missing return annotation"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_yield_expression_statement() {
    let module = parser::parse_module("yield 1").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Yield { value } => {
                assert!(value.is_some());
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_yield_from_expression_statement() {
    let module = parser::parse_module("yield from items").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::YieldFrom { value } => match &value.node {
                ExprKind::Name(name) => assert_eq!(name, "items"),
                other => panic!("unexpected yield from source: {other:?}"),
            },
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_from_import_statement() {
    let module = parser::parse_module("from mod import a, b").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ImportFrom {
            module,
            names,
            level,
        } => {
            assert_eq!(module.as_deref(), Some("mod"));
            assert_eq!(*level, 0);
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
fn parses_from_dotted_import_statement() {
    let module = parser::parse_module("from pkg.sub import item").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ImportFrom {
            module,
            names,
            level,
        } => {
            assert_eq!(module.as_deref(), Some("pkg.sub"));
            assert_eq!(*level, 0);
            assert_eq!(
                names,
                &vec![pyrs::ast::ImportAlias {
                    name: "item".to_string(),
                    asname: None
                }]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_import_alias() {
    let module = parser::parse_module("import math as m").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Import { names } => {
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
    let module = parser::parse_module("from mod import value as v").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ImportFrom { names, .. } => {
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
fn parses_relative_from_import_statement() {
    let module = parser::parse_module("from .sub import value").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ImportFrom {
            module,
            names,
            level,
        } => {
            assert_eq!(*level, 1);
            assert_eq!(module.as_deref(), Some("sub"));
            assert_eq!(
                names,
                &vec![pyrs::ast::ImportAlias {
                    name: "value".to_string(),
                    asname: None
                }]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_relative_parent_from_import_statement() {
    let module = parser::parse_module("from .. import item").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::ImportFrom {
            module,
            names,
            level,
        } => {
            assert_eq!(*level, 2);
            assert!(module.is_none());
            assert_eq!(
                names,
                &vec![pyrs::ast::ImportAlias {
                    name: "item".to_string(),
                    asname: None
                }]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_slice_subscript() {
    let module = parser::parse_module("x[1:3]").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Subscript { value, index } => {
                assert_eq!(&value.node, &ExprKind::Name("x".to_string()));
                match &index.node {
                    ExprKind::Slice { lower, upper, step } => {
                        assert_eq!(
                            lower.as_ref().map(|expr| &expr.node),
                            Some(&ExprKind::Constant(Constant::Int(1)))
                        );
                        assert_eq!(
                            upper.as_ref().map(|expr| &expr.node),
                            Some(&ExprKind::Constant(Constant::Int(3)))
                        );
                        assert!(step.is_none());
                    }
                    other => panic!("unexpected index: {other:?}"),
                }
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_slice_with_step() {
    let module = parser::parse_module("x[::2]").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Subscript { index, .. } => match &index.node {
                ExprKind::Slice { lower, upper, step } => {
                    assert!(lower.is_none());
                    assert!(upper.is_none());
                    assert_eq!(
                        step.as_ref().map(|expr| &expr.node),
                        Some(&ExprKind::Constant(Constant::Int(2)))
                    );
                }
                other => panic!("unexpected index: {other:?}"),
            },
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_multi_item_subscript_with_slices() {
    let module = parser::parse_module("x[:42, ..., :24:, 24, 100]").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Subscript { index, .. } => match &index.node {
                ExprKind::Tuple(items) => {
                    assert_eq!(items.len(), 5);
                    match &items[0].node {
                        ExprKind::Slice { lower, upper, step } => {
                            assert!(lower.is_none());
                            assert_eq!(
                                upper.as_ref().map(|expr| &expr.node),
                                Some(&ExprKind::Constant(Constant::Int(42)))
                            );
                            assert!(step.is_none());
                        }
                        other => panic!("unexpected index item: {other:?}"),
                    }
                    match &items[2].node {
                        ExprKind::Slice { lower, upper, step } => {
                            assert!(lower.is_none());
                            assert_eq!(
                                upper.as_ref().map(|expr| &expr.node),
                                Some(&ExprKind::Constant(Constant::Int(24)))
                            );
                            assert!(step.is_none());
                        }
                        other => panic!("unexpected index item: {other:?}"),
                    }
                }
                other => panic!("unexpected index: {other:?}"),
            },
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_dict_unpack_literal() {
    let module = parser::parse_module("{'a': 1, **mapping, 'b': 2}").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Dict(entries) => {
                assert_eq!(entries.len(), 3);
                assert!(matches!(&entries[0], DictEntry::Pair(_, _)));
                assert!(matches!(
                    &entries[1],
                    DictEntry::Unpack(Expr {
                        node: ExprKind::Name(_),
                        ..
                    })
                ));
                assert!(matches!(&entries[2], DictEntry::Pair(_, _)));
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_set_unpack_literal() {
    parser::parse_module("{*range(10), 42, *items}\n").expect("parse should succeed");
}

#[test]
fn parses_empty_list_assignment_target() {
    let module = parser::parse_module("[] = value\n").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { targets, .. } => {
            assert_eq!(targets, &vec![AssignTarget::List(Vec::new())]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_raw_string_with_backslashes() {
    let module = parser::parse_module("x = r'\\\\'\n").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { value, .. } => {
            assert_eq!(
                value,
                &spanned_expr(ExprKind::Constant(Constant::Str("\\\\".to_string())))
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_raw_string_with_escaped_quote() {
    let module = parser::parse_module("x = r'[\\w!\"\\'&.,?]'\n").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { value, .. } => {
            assert_eq!(
                value,
                &spanned_expr(ExprKind::Constant(Constant::Str(
                    "[\\w!\"\\'&.,?]".to_string()
                )))
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_template_string_prefix() {
    let module = parser::parse_module("x = t\"hello\"\n").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { value, .. } => {
            assert_eq!(
                value,
                &spanned_expr(ExprKind::Constant(Constant::Str("hello".to_string())))
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_fstring_with_nested_format_fields() {
    let source = "x = f\"{abs(n):{thousands_sep}}/{d:{thousands_sep}}\"\n";
    parser::parse_module(source).expect("parse should succeed");
}

#[test]
fn parses_fstring_expression_with_escaped_string_literal() {
    let source = "x = f\"{json.dumps('\\\\n'.join(commands))}\"\n";
    parser::parse_module(source).expect("parse should succeed");
}

#[test]
fn parses_tuple_literal() {
    let module = parser::parse_module("(1, 2)").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Tuple(values) => {
                assert_eq!(values.len(), 2);
                assert_eq!(&values[0].node, &ExprKind::Constant(Constant::Int(1)));
                assert_eq!(&values[1].node, &ExprKind::Constant(Constant::Int(2)));
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_dict_literal() {
    let module = parser::parse_module("{'a': 1, 'b': 2}").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Dict(entries) => {
                assert_eq!(entries.len(), 2);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_for_loop() {
    let source = "for i in [1, 2]:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::For {
            target,
            iter,
            body,
            orelse,
            ..
        } => {
            assert_eq!(target, &AssignTarget::Name("i".to_string()));
            assert_eq!(
                &iter.node,
                &ExprKind::List(vec![
                    spanned_expr(ExprKind::Constant(Constant::Int(1))),
                    spanned_expr(ExprKind::Constant(Constant::Int(2)))
                ])
            );
            assert_eq!(body, &vec![spanned_stmt(StmtKind::Pass)]);
            assert!(orelse.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_break_and_continue() {
    let source = "while 1:\n    break\n    continue\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::While { body, orelse, .. } => {
            assert_eq!(
                body,
                &vec![
                    spanned_stmt(StmtKind::Break),
                    spanned_stmt(StmtKind::Continue)
                ]
            );
            assert!(orelse.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_while_else_clause() {
    let source = "while 0:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::While { orelse, .. } => {
            assert_eq!(orelse, &vec![spanned_stmt(StmtKind::Pass)]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_for_else_clause() {
    let source = "for i in [1]:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::For { orelse, .. } => {
            assert_eq!(orelse, &vec![spanned_stmt(StmtKind::Pass)]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_boolean_operators() {
    let module = parser::parse_module("a or b and c").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::BoolOp { op, left, right } => {
                assert_eq!(*op, pyrs::ast::BoolOp::Or);
                assert_eq!(&left.node, &ExprKind::Name("a".to_string()));
                match &right.node {
                    ExprKind::BoolOp { op, .. } => assert_eq!(*op, pyrs::ast::BoolOp::And),
                    _ => panic!("expected nested and"),
                }
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_not_operator() {
    let module = parser::parse_module("not False").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::Unary { op, .. } => {
                assert_eq!(*op, pyrs::ast::UnaryOp::Not);
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_integer_literal() {
    let module = parser::parse_module("42").expect("parse should succeed");
    assert_eq!(
        strip_module(&module),
        vec![spanned_stmt(StmtKind::Expr(spanned_expr(
            ExprKind::Constant(Constant::Int(42))
        )))]
    );
}

#[test]
fn parses_float_literal() {
    let module = parser::parse_module("3.5").expect("parse should succeed");
    assert_eq!(
        strip_module(&module),
        vec![spanned_stmt(StmtKind::Expr(spanned_expr(
            ExprKind::Constant(Constant::Float(FloatLiteral(3.5)))
        )))]
    );
}

#[test]
fn parses_leading_dot_float_literal() {
    let module = parser::parse_module(".5").expect("parse should succeed");
    assert_eq!(
        strip_module(&module),
        vec![spanned_stmt(StmtKind::Expr(spanned_expr(
            ExprKind::Constant(Constant::Float(FloatLiteral(0.5)))
        )))]
    );
}

#[test]
fn parses_string_literal() {
    let module = parser::parse_module("'hi'").expect("parse should succeed");
    assert_eq!(
        strip_module(&module),
        vec![spanned_stmt(StmtKind::Expr(spanned_expr(
            ExprKind::Constant(Constant::Str("hi".to_string()))
        )))]
    );
}

#[test]
fn rejects_unknown_token() {
    let err = parser::parse_module("@").expect_err("parse should fail");
    assert_eq!(err.offset, 1);
}

#[test]
fn parses_inline_if_statement() {
    let module = parser::parse_module("if x: pass\n").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::If { test, body, orelse } => {
            assert_eq!(&test.node, &ExprKind::Name("x".to_string()));
            assert_eq!(body, &vec![spanned_stmt(StmtKind::Pass)]);
            assert!(orelse.is_empty());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_block_if_else_statement() {
    let source = "if x:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::If { test, body, orelse } => {
            assert_eq!(&test.node, &ExprKind::Name("x".to_string()));
            assert_eq!(body, &vec![spanned_stmt(StmtKind::Pass)]);
            assert_eq!(orelse, &vec![spanned_stmt(StmtKind::Pass)]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_if_elif_else_statement() {
    let source = "if x:\n    pass\nelif y:\n    pass\nelse:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::If { orelse, .. } => {
            assert_eq!(orelse.len(), 1);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_decorated_function_definition() {
    let source = "@d1\n@d2\ndef f(x):\n    return x\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Decorated { decorators, stmt } => {
            assert_eq!(decorators.len(), 2);
            assert_eq!(decorators[0].node, ExprKind::Name("d1".to_string()));
            assert_eq!(decorators[1].node, ExprKind::Name("d2".to_string()));
            match &stmt.node {
                StmtKind::FunctionDef { name, .. } => assert_eq!(name, "f"),
                other => panic!("unexpected decorated stmt: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_named_expression() {
    let module = parser::parse_module("(x := 3)").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => match &expr.node {
            ExprKind::NamedExpr { target, value } => {
                assert_eq!(target, "x");
                assert_eq!(value.node, ExprKind::Constant(Constant::Int(3)));
            }
            other => panic!("unexpected expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_comprehensions_and_generator_expression() {
    let source = "a = [x * 2 for x in [1, 2, 3] if x > 1]\n\
b = {x: x + 1 for x in [1, 2]}\n\
c = (x for x in [1, 2])\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let stripped = strip_module(&module);

    match &stripped[0].node {
        StmtKind::Assign { value, .. } => match &value.node {
            ExprKind::ListComp { clauses, .. } => {
                assert_eq!(clauses.len(), 1);
                assert_eq!(clauses[0].ifs.len(), 1);
            }
            other => panic!("unexpected list comp expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }

    match &stripped[1].node {
        StmtKind::Assign { value, .. } => match &value.node {
            ExprKind::DictComp { clauses, .. } => assert_eq!(clauses.len(), 1),
            other => panic!("unexpected dict comp expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }

    match &stripped[2].node {
        StmtKind::Assign { value, .. } => match &value.node {
            ExprKind::GeneratorExp { clauses, .. } => assert_eq!(clauses.len(), 1),
            other => panic!("unexpected generator expr: {other:?}"),
        },
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_match_case_statement() {
    let source = "match value:\n    case 1:\n        out = 'one'\n    case x if x > 1:\n        out = 'many'\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Match { subject, cases } => {
            assert_eq!(subject.node, ExprKind::Name("value".to_string()));
            assert_eq!(cases.len(), 2);
            assert!(matches!(
                cases[0].pattern,
                Pattern::Constant(Constant::Int(1))
            ));
            assert!(matches!(cases[1].pattern, Pattern::Capture(_)));
            assert!(cases[1].guard.is_some());
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_match_pattern_families() {
    let source = "match value:\n    case [1, *rest, 3]:\n        a = rest\n    case {'kind': kind, **tail}:\n        b = kind\n    case Point(x=1, y=y):\n        c = y\n    case 1 | 2 as z:\n        d = z\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Match { cases, .. } => {
            assert_eq!(cases.len(), 4);
            assert!(matches!(cases[0].pattern, Pattern::Sequence(_)));
            assert!(matches!(cases[1].pattern, Pattern::Mapping { .. }));
            assert!(matches!(cases[2].pattern, Pattern::Class { .. }));
            assert!(matches!(cases[3].pattern, Pattern::Or(_)));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn rejects_positional_after_keyword_in_class_pattern() {
    let source = "match value:\n    case Point(x=1, 2):\n        out = 1\n";
    let err = parser::parse_module(source).expect_err("parse should fail");
    assert!(
        err.message.contains("positional patterns follow keyword patterns"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn parses_async_statements_and_await() {
    let source = "async def f(x):\n    return await x\nasync for i in [1]:\n    pass\nasync with ctx:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let stripped = strip_module(&module);
    match &stripped[0].node {
        StmtKind::FunctionDef { is_async, body, .. } => {
            assert!(*is_async);
            match &body[0].node {
                StmtKind::Return { value } => {
                    let value = value.as_ref().expect("await return value");
                    assert!(matches!(value.node, ExprKind::Await { .. }));
                }
                other => panic!("unexpected async function body stmt: {other:?}"),
            }
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
    assert!(matches!(
        stripped[1].node,
        StmtKind::For { is_async: true, .. }
    ));
    assert!(matches!(
        stripped[2].node,
        StmtKind::With { is_async: true, .. }
    ));
}

#[test]
fn parses_except_star_handler() {
    let source = "try:\n    pass\nexcept* ValueError:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Try { handlers, .. } => {
            assert_eq!(handlers.len(), 1);
            assert!(handlers[0].is_star);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn rejects_except_star_without_type() {
    let source = "try:\n    pass\nexcept*:\n    pass\n";
    let err = parser::parse_module(source).expect_err("parse should fail");
    assert!(
        err.message.contains("except* requires an exception type"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rejects_mixing_except_and_except_star() {
    let source = "try:\n    pass\nexcept ValueError:\n    pass\nexcept* TypeError:\n    pass\n";
    let err = parser::parse_module(source).expect_err("parse should fail");
    assert!(
        err.message.contains("cannot mix 'except' and 'except*'"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn parses_fstring_literal_expression() {
    let module = parser::parse_module("f\"hello {name} {1 + 2}\"").expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Expr(expr) => {
            // f-strings are lowered to concatenation + str(...) calls.
            assert!(matches!(expr.node, ExprKind::Binary { .. }));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_type_parameters_on_defs_and_classes() {
    let source = "def f[T](x):\n    return x\nclass Box[T]:\n    pass\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    let stripped = strip_module(&module);
    match &stripped[0].node {
        StmtKind::FunctionDef { type_params, .. } => {
            assert_eq!(type_params, &vec!["T".to_string()]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
    match &stripped[1].node {
        StmtKind::ClassDef { type_params, .. } => {
            assert_eq!(type_params, &vec!["T".to_string()]);
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_type_parameter_variants() {
    let source = "def f[*Ts, **P, T: int = int](x):\n    return x\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::FunctionDef { type_params, .. } => {
            assert_eq!(
                type_params,
                &vec!["Ts".to_string(), "P".to_string(), "T".to_string()]
            );
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}

#[test]
fn parses_type_alias_statement() {
    let source = "type Alias[T] = tuple[T]\n";
    let module = parser::parse_module(source).expect("parse should succeed");
    match &strip_module(&module)[0].node {
        StmtKind::Assign { targets, .. } => {
            assert_eq!(targets.len(), 1);
            assert!(matches!(targets[0], AssignTarget::Name(ref name) if name == "Alias"));
        }
        other => panic!("unexpected stmt: {other:?}"),
    }
}
