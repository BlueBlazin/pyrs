use std::collections::HashMap;

use crate::ast::{
    AssignTarget, BinaryOp, BoolOp, CallArg, ComprehensionClause, Constant, ExceptHandler, Expr,
    ExprKind, ImportAlias, MatchCase, Module, Parameter, Pattern, Span, Stmt, StmtKind, UnaryOp,
};
use crate::parser::lexer::{LexError, Lexer};
use crate::parser::token::{Keyword, Token, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

impl ParseError {
    pub fn new(message: impl Into<String>, offset: usize, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            offset,
            line,
            column,
        }
    }
}

impl From<LexError> for ParseError {
    fn from(err: LexError) -> Self {
        Self::new(err.message, err.offset, err.line, err.column)
    }
}

type ParseResult<T> = Result<(T, usize), ParseError>;

#[derive(Debug, Clone)]
struct Memo<T> {
    result: ParseResult<T>,
}

pub fn parse_module(source: &str) -> Result<Module, ParseError> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().map_err(ParseError::from)?;
    let mut parser = Parser::new(tokens);
    let (module, pos) = parser.parse_module_at(0)?;
    parser.expect_end(pos)?;
    Ok(module)
}

struct Parser {
    tokens: Vec<Token>,
    module_memo: HashMap<usize, Memo<Module>>,
    stmt_memo: HashMap<usize, Memo<Stmt>>,
    expr_memo: HashMap<usize, Memo<Expr>>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            module_memo: HashMap::new(),
            stmt_memo: HashMap::new(),
            expr_memo: HashMap::new(),
        }
    }

    fn span_at(&self, pos: usize) -> Span {
        let token = self.token_at(pos);
        Span::new(token.line, token.column)
    }

    fn make_stmt(&self, pos: usize, node: StmtKind) -> Stmt {
        Stmt {
            node,
            span: self.span_at(pos),
        }
    }

    fn make_expr(&self, pos: usize, node: ExprKind) -> Expr {
        Expr {
            node,
            span: self.span_at(pos),
        }
    }

    fn parse_module_at(&mut self, pos: usize) -> ParseResult<Module> {
        if let Some(entry) = self.module_memo.get(&pos) {
            return entry.result.clone();
        }

        let result = self.parse_module_uncached(pos);
        self.module_memo.insert(
            pos,
            Memo {
                result: result.clone(),
            },
        );
        result
    }

    fn parse_module_uncached(&mut self, pos: usize) -> ParseResult<Module> {
        let mut pos = self.consume_separators(pos);
        let mut body = Vec::new();

        while !self.is_end(pos) {
            let (stmt, next) = self.parse_stmt_at(pos)?;
            let allows_missing = stmt_allows_missing_terminator(&stmt);
            body.push(stmt);

            let next_kind = &self.token_at(next).kind;
            if matches!(
                next_kind,
                TokenKind::Newline | TokenKind::Semicolon | TokenKind::EndMarker
            ) {
                pos = self.consume_terminators(next)?;
            } else if allows_missing {
                pos = next;
            } else {
                return Err(self.error_at(next, "expected statement terminator"));
            }

            pos = self.consume_separators(pos);
        }

        Ok((Module { body }, pos))
    }

    fn parse_stmt_at(&mut self, pos: usize) -> ParseResult<Stmt> {
        if let Some(entry) = self.stmt_memo.get(&pos) {
            return entry.result.clone();
        }

        let result = self.parse_stmt_uncached(pos);
        self.stmt_memo.insert(
            pos,
            Memo {
                result: result.clone(),
            },
        );
        result
    }

    fn parse_stmt_uncached(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        if matches!(self.token_at(pos).kind, TokenKind::At) {
            return self.parse_decorated_stmt(pos);
        }
        if let Some((target_expr, next_pos)) = self.parse_assignment_target_list(pos) {
            let kind = self.token_at(next_pos).kind.clone();
            if kind == TokenKind::Colon {
                if matches!(target_expr, AssignTarget::Tuple(_) | AssignTarget::List(_)) {
                    return Err(self.error_at(next_pos, "invalid annotation target"));
                }
                let (annotation, mut next) = self.parse_expr_at(next_pos + 1)?;
                let mut value = None;
                if matches!(self.token_at(next).kind, TokenKind::Equal) {
                    let (expr, after) = self.parse_expr_at(next + 1)?;
                    value = Some(expr);
                    next = after;
                }
                return Ok((
                    self.make_stmt(
                        start,
                        StmtKind::AnnAssign {
                            target: target_expr,
                            annotation,
                            value,
                        },
                    ),
                    next,
                ));
            }
            if kind == TokenKind::Equal {
                let (value, next) = self.parse_expr_at(next_pos + 1)?;
                return Ok((
                    self.make_stmt(
                        start,
                        StmtKind::Assign {
                            target: target_expr,
                            value,
                        },
                    ),
                    next,
                ));
            }

            let aug_op = match kind {
                TokenKind::PlusEqual => Some(crate::ast::AugOp::Add),
                TokenKind::MinusEqual => Some(crate::ast::AugOp::Sub),
                TokenKind::StarEqual => Some(crate::ast::AugOp::Mul),
                TokenKind::PercentEqual => Some(crate::ast::AugOp::Mod),
                TokenKind::DoubleSlashEqual => Some(crate::ast::AugOp::FloorDiv),
                TokenKind::DoubleStarEqual => Some(crate::ast::AugOp::Pow),
                _ => None,
            };

            if let Some(op) = aug_op {
                let (value, next) = self.parse_expr_at(next_pos + 1)?;
                return Ok((
                    self.make_stmt(
                        start,
                        StmtKind::AugAssign {
                            target: target_expr,
                            op,
                            value,
                        },
                    ),
                    next,
                ));
            }
        }
        if self.match_soft_keyword(pos, Keyword::Match, "match")
            && !matches!(
                self.token_at(pos + 1).kind,
                TokenKind::Equal
                    | TokenKind::PlusEqual
                    | TokenKind::MinusEqual
                    | TokenKind::StarEqual
                    | TokenKind::DoubleStarEqual
                    | TokenKind::DoubleSlashEqual
                    | TokenKind::PercentEqual
                    | TokenKind::ColonEqual
            )
        {
            if let Ok(parsed) = self.parse_match_stmt(pos) {
                return Ok(parsed);
            }
        }
        let token = self.token_at(pos);
        match token.kind {
            TokenKind::Keyword(Keyword::Def) => self.parse_function_def(pos),
            TokenKind::Keyword(Keyword::Async) => self.parse_async_stmt(pos),
            TokenKind::Keyword(Keyword::Return) => self.parse_return_stmt(pos),
            TokenKind::Keyword(Keyword::If) => self.parse_if_stmt(pos),
            TokenKind::Keyword(Keyword::While) => self.parse_while_stmt(pos),
            TokenKind::Keyword(Keyword::Try) => self.parse_try_stmt(pos),
            TokenKind::Keyword(Keyword::For) => self.parse_for_stmt(pos),
            TokenKind::Keyword(Keyword::Class) => self.parse_class_def(pos),
            TokenKind::Keyword(Keyword::Break) => {
                Ok((self.make_stmt(start, StmtKind::Break), pos + 1))
            }
            TokenKind::Keyword(Keyword::Continue) => {
                Ok((self.make_stmt(start, StmtKind::Continue), pos + 1))
            }
            TokenKind::Keyword(Keyword::Import) => self.parse_import_stmt(pos),
            TokenKind::Keyword(Keyword::From) => self.parse_from_import_stmt(pos),
            TokenKind::Keyword(Keyword::Global) => self.parse_global_stmt(pos),
            TokenKind::Keyword(Keyword::Nonlocal) => self.parse_nonlocal_stmt(pos),
            TokenKind::Keyword(Keyword::With) => self.parse_with_stmt(pos),
            TokenKind::Keyword(Keyword::Raise) => self.parse_raise_stmt(pos),
            TokenKind::Keyword(Keyword::Assert) => self.parse_assert_stmt(pos),
            TokenKind::Keyword(Keyword::Pass) => {
                Ok((self.make_stmt(start, StmtKind::Pass), pos + 1))
            }
            _ => {
                let (expr, next) = self.parse_expr_at(pos)?;
                Ok((self.make_stmt(start, StmtKind::Expr(expr)), next))
            }
        }
    }

    fn parse_decorated_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos;
        let mut decorators = Vec::new();
        while matches!(self.token_at(pos).kind, TokenKind::At) {
            pos += 1;
            let (expr, next) = self.parse_expr_at(pos)?;
            decorators.push(expr);
            pos = self.consume_terminators(next)?;
            pos = self.consume_separators(pos);
        }
        let (stmt, pos) = self.parse_stmt_at(pos)?;
        let valid_target = matches!(
            stmt.node,
            StmtKind::FunctionDef { .. } | StmtKind::ClassDef { .. }
        );
        if !valid_target {
            return Err(self.error_at(start, "decorator must target function or class definition"));
        }
        Ok((
            self.make_stmt(
                start,
                StmtKind::Decorated {
                    decorators,
                    stmt: Box::new(stmt),
                },
            ),
            pos,
        ))
    }

    fn parse_async_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let next = pos + 1;
        match self.token_at(next).kind {
            TokenKind::Keyword(Keyword::Def) => self.parse_function_def_internal(pos, next, true),
            TokenKind::Keyword(Keyword::For) => self.parse_for_stmt_internal(pos, next, true),
            TokenKind::Keyword(Keyword::With) => self.parse_with_stmt_internal(pos, next, true),
            _ => Err(self.error_at(pos, "expected 'def', 'for', or 'with' after 'async'")),
        }
    }

    fn parse_match_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let (subject, next) = self.parse_expr_at(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        pos = self.expect_kind(pos, TokenKind::Newline)?;
        pos = self.expect_kind(pos, TokenKind::Indent)?;

        let mut cases = Vec::new();
        pos = self.consume_separators(pos);
        while !matches!(
            self.token_at(pos).kind,
            TokenKind::Dedent | TokenKind::EndMarker
        ) {
            if !self.match_soft_keyword(pos, Keyword::Case, "case") {
                return Err(self.error_at(pos, "expected case clause"));
            }
            pos += 1;
            let (pattern, next) = self.parse_match_pattern(pos)?;
            pos = next;
            let mut guard = None;
            if self.match_keyword(pos, Keyword::If) {
                let (expr, next) = self.parse_expr_at(pos + 1)?;
                guard = Some(expr);
                pos = next;
            }
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (body, next) = self.parse_suite(pos)?;
            pos = next;
            cases.push(MatchCase {
                pattern,
                guard,
                body,
            });
            pos = self.consume_separators(pos);
        }
        pos = self.expect_kind(pos, TokenKind::Dedent)?;
        Ok((self.make_stmt(start, StmtKind::Match { subject, cases }), pos))
    }

    fn parse_match_pattern(&mut self, pos: usize) -> Result<(Pattern, usize), ParseError> {
        let token = self.token_at(pos);
        match &token.kind {
            TokenKind::Name => {
                if token.lexeme == "_" {
                    Ok((Pattern::Wildcard, pos + 1))
                } else {
                    Ok((Pattern::Capture(token.lexeme.clone()), pos + 1))
                }
            }
            TokenKind::Number => {
                let value = self.parse_int_literal(&token.lexeme, pos)?;
                Ok((Pattern::Constant(Constant::Int(value)), pos + 1))
            }
            TokenKind::String => Ok((
                Pattern::Constant(Constant::Str(token.lexeme.clone())),
                pos + 1,
            )),
            TokenKind::Keyword(Keyword::TrueLiteral) => {
                Ok((Pattern::Constant(Constant::Bool(true)), pos + 1))
            }
            TokenKind::Keyword(Keyword::FalseLiteral) => {
                Ok((Pattern::Constant(Constant::Bool(false)), pos + 1))
            }
            TokenKind::Keyword(Keyword::NoneLiteral) => {
                Ok((Pattern::Constant(Constant::None), pos + 1))
            }
            TokenKind::Minus => {
                let next = self.token_at(pos + 1);
                if next.kind != TokenKind::Number {
                    return Err(self.error_at(pos, "expected numeric pattern"));
                }
                let value = self.parse_int_literal(&next.lexeme, pos + 1)?;
                Ok((Pattern::Constant(Constant::Int(-value)), pos + 2))
            }
            _ => Err(self.error_at(pos, "unsupported pattern")),
        }
    }

    fn parse_expr_at(&mut self, pos: usize) -> ParseResult<Expr> {
        if let Some(entry) = self.expr_memo.get(&pos) {
            return entry.result.clone();
        }

        let result = self.parse_expr_uncached(pos);
        self.expr_memo.insert(
            pos,
            Memo {
                result: result.clone(),
            },
        );
        result
    }

    fn parse_expr_uncached(&mut self, pos: usize) -> ParseResult<Expr> {
        if self.match_keyword(pos, Keyword::Yield) {
            self.parse_yield_expr(pos)
        } else if self.match_keyword(pos, Keyword::Lambda) {
            self.parse_lambda(pos)
        } else {
            self.parse_named_expr(pos)
        }
    }

    fn parse_named_expr(&mut self, pos: usize) -> ParseResult<Expr> {
        let (left, pos) = self.parse_if_expr(pos)?;
        if self.token_at(pos).kind != TokenKind::ColonEqual {
            return Ok((left, pos));
        }
        let target = match &left.node {
            ExprKind::Name(name) => name.clone(),
            _ => return Err(self.error_at(pos, "assignment expression target must be a name")),
        };
        let (value, next) = self.parse_expr_at(pos + 1)?;
        Ok((
            Expr {
                node: ExprKind::NamedExpr {
                    target,
                    value: Box::new(value),
                },
                span: left.span,
            },
            next,
        ))
    }

    fn parse_yield_expr(&mut self, pos: usize) -> ParseResult<Expr> {
        let start = pos;
        let pos = pos + 1;
        if self.match_keyword(pos, Keyword::From) {
            let (value, next) = self.parse_expr_at(pos + 1)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::YieldFrom {
                        value: Box::new(value),
                    },
                ),
                next,
            ));
        }
        if matches!(
            self.token_at(pos).kind,
            TokenKind::Newline
                | TokenKind::Semicolon
                | TokenKind::Dedent
                | TokenKind::EndMarker
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::Comma
                | TokenKind::Colon
        ) {
            return Ok((self.make_expr(start, ExprKind::Yield { value: None }), pos));
        }

        let (value, next) = self.parse_if_expr(pos)?;
        Ok((
            self.make_expr(
                start,
                ExprKind::Yield {
                    value: Some(Box::new(value)),
                },
            ),
            next,
        ))
    }

    fn parse_lambda(&mut self, pos: usize) -> ParseResult<Expr> {
        let start = pos;
        let mut pos = pos + 1;
        let (posonly_params, params, kwonly_params, vararg, kwarg, next) =
            if matches!(self.token_at(pos).kind, TokenKind::Colon) {
                (Vec::new(), Vec::new(), Vec::new(), None, None, pos)
            } else {
                self.parse_lambda_params(pos)?
            };
        pos = next;

        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_expr_at(pos)?;
        Ok((
            self.make_expr(
                start,
                ExprKind::Lambda {
                    posonly_params,
                    params,
                    vararg,
                    kwarg,
                    kwonly_params,
                    body: Box::new(body),
                },
            ),
            next,
        ))
    }

    fn parse_if_expr(&mut self, pos: usize) -> ParseResult<Expr> {
        let (body, mut pos) = self.parse_or(pos)?;
        if self.match_keyword(pos, Keyword::If) {
            pos += 1;
            let (test, next) = self.parse_or(pos)?;
            pos = next;
            if !self.match_keyword(pos, Keyword::Else) {
                return Err(self.error_at(pos, "expected else"));
            }
            pos += 1;
            let (orelse, next) = self.parse_if_expr(pos)?;
            let span = body.span;
            let expr = Expr {
                node: ExprKind::IfExpr {
                    test: Box::new(test),
                    body: Box::new(body),
                    orelse: Box::new(orelse),
                },
                span,
            };
            return Ok((expr, next));
        }
        Ok((body, pos))
    }

    fn parse_or(&mut self, pos: usize) -> ParseResult<Expr> {
        let (mut left, mut pos) = self.parse_and(pos)?;
        while self.match_keyword(pos, Keyword::Or) {
            pos += 1;
            let (right, next) = self.parse_and(pos)?;
            let span = left.span;
            left = Expr {
                node: ExprKind::BoolOp {
                    op: BoolOp::Or,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
            pos = next;
        }
        Ok((left, pos))
    }

    fn parse_and(&mut self, pos: usize) -> ParseResult<Expr> {
        let (mut left, mut pos) = self.parse_not(pos)?;
        while self.match_keyword(pos, Keyword::And) {
            pos += 1;
            let (right, next) = self.parse_not(pos)?;
            let span = left.span;
            left = Expr {
                node: ExprKind::BoolOp {
                    op: BoolOp::And,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
            pos = next;
        }
        Ok((left, pos))
    }

    fn parse_not(&mut self, pos: usize) -> ParseResult<Expr> {
        if self.match_keyword(pos, Keyword::Not) {
            let start = pos;
            let (expr, next) = self.parse_not(pos + 1)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(expr),
                    },
                ),
                next,
            ));
        }
        self.parse_comparison(pos)
    }

    fn parse_comparison(&mut self, pos: usize) -> ParseResult<Expr> {
        let (left, mut pos) = self.parse_add_sub(pos)?;

        let (op, consumed) = match self.token_at(pos).kind {
            TokenKind::DoubleEqual => (BinaryOp::Eq, 1),
            TokenKind::NotEqual => (BinaryOp::Ne, 1),
            TokenKind::Less => (BinaryOp::Lt, 1),
            TokenKind::LessEqual => (BinaryOp::Le, 1),
            TokenKind::Greater => (BinaryOp::Gt, 1),
            TokenKind::GreaterEqual => (BinaryOp::Ge, 1),
            TokenKind::Keyword(Keyword::In) => (BinaryOp::In, 1),
            TokenKind::Keyword(Keyword::Is) => {
                if self.match_keyword(pos + 1, Keyword::Not) {
                    (BinaryOp::IsNot, 2)
                } else {
                    (BinaryOp::Is, 1)
                }
            }
            TokenKind::Keyword(Keyword::Not) => {
                if self.match_keyword(pos + 1, Keyword::In) {
                    (BinaryOp::NotIn, 2)
                } else {
                    return Ok((left, pos));
                }
            }
            _ => return Ok((left, pos)),
        };

        pos += consumed;
        let (right, next) = self.parse_add_sub(pos)?;
        let span = left.span;
        Ok((
            Expr {
                node: ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            },
            next,
        ))
    }

    fn parse_add_sub(&mut self, pos: usize) -> ParseResult<Expr> {
        let (mut left, mut pos) = self.parse_mul(pos)?;

        loop {
            let op = match self.token_at(pos).kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            pos += 1;
            let (right, next) = self.parse_mul(pos)?;
            let span = left.span;
            left = Expr {
                node: ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
            pos = next;
        }

        Ok((left, pos))
    }

    fn parse_mul(&mut self, pos: usize) -> ParseResult<Expr> {
        let (mut left, mut pos) = self.parse_unary(pos)?;

        loop {
            let op = match self.token_at(pos).kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::DoubleSlash => BinaryOp::FloorDiv,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            pos += 1;
            let (right, next) = self.parse_unary(pos)?;
            let span = left.span;
            left = Expr {
                node: ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            };
            pos = next;
        }

        Ok((left, pos))
    }

    fn parse_power(&mut self, pos: usize) -> ParseResult<Expr> {
        let (left, mut pos) = self.parse_atom(pos)?;
        if matches!(self.token_at(pos).kind, TokenKind::DoubleStar) {
            pos += 1;
            let (right, next) = self.parse_unary(pos)?;
            let span = left.span;
            return Ok((
                Expr {
                    node: ExprKind::Binary {
                        left: Box::new(left),
                        op: BinaryOp::Pow,
                        right: Box::new(right),
                    },
                    span,
                },
                next,
            ));
        }
        Ok((left, pos))
    }

    fn parse_unary(&mut self, pos: usize) -> ParseResult<Expr> {
        if self.match_keyword(pos, Keyword::Await) {
            let start = pos;
            let (expr, next) = self.parse_unary(pos + 1)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::Await {
                        value: Box::new(expr),
                    },
                ),
                next,
            ));
        }
        if matches!(self.token_at(pos).kind, TokenKind::Minus) {
            let start = pos;
            let (expr, next) = self.parse_unary(pos + 1)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(expr),
                    },
                ),
                next,
            ));
        }
        if matches!(self.token_at(pos).kind, TokenKind::Plus) {
            let start = pos;
            let (expr, next) = self.parse_unary(pos + 1)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::Unary {
                        op: UnaryOp::Pos,
                        operand: Box::new(expr),
                    },
                ),
                next,
            ));
        }
        self.parse_power(pos)
    }

    fn parse_atom(&mut self, pos: usize) -> ParseResult<Expr> {
        let token = self.token_at(pos).clone();
        let (mut expr, mut pos) = match &token.kind {
            TokenKind::Name => (
                self.make_expr(pos, ExprKind::Name(token.lexeme.clone())),
                pos + 1,
            ),
            TokenKind::Number => {
                let value = self.parse_int_literal(&token.lexeme, pos)?;
                (
                    self.make_expr(pos, ExprKind::Constant(Constant::Int(value))),
                    pos + 1,
                )
            }
            TokenKind::String => (
                self.make_expr(pos, ExprKind::Constant(Constant::Str(token.lexeme.clone()))),
                pos + 1,
            ),
            TokenKind::FString => (self.parse_fstring_literal(pos, &token.lexeme)?, pos + 1),
            TokenKind::Keyword(Keyword::TrueLiteral) => (
                self.make_expr(pos, ExprKind::Constant(Constant::Bool(true))),
                pos + 1,
            ),
            TokenKind::Keyword(Keyword::FalseLiteral) => (
                self.make_expr(pos, ExprKind::Constant(Constant::Bool(false))),
                pos + 1,
            ),
            TokenKind::Keyword(Keyword::NoneLiteral) => (
                self.make_expr(pos, ExprKind::Constant(Constant::None)),
                pos + 1,
            ),
            TokenKind::LParen => {
                let (expr, next) = self.parse_paren_expr(pos + 1)?;
                (expr, next)
            }
            TokenKind::LBracket => {
                let (expr, next) = self.parse_list_or_comp(pos + 1)?;
                (expr, next)
            }
            TokenKind::LBrace => {
                let (expr, next) = self.parse_dict_or_comp(pos + 1)?;
                (expr, next)
            }
            _ => return Err(self.error_at(pos, "expected expression")),
        };

        loop {
            match self.token_at(pos).kind {
                TokenKind::LParen => {
                    let (args, next) = self.parse_call_args(pos + 1)?;
                    let span = expr.span;
                    expr = Expr {
                        node: ExprKind::Call {
                            func: Box::new(expr),
                            args,
                        },
                        span,
                    };
                    pos = next;
                }
                TokenKind::LBracket => {
                    let (index, next) = self.parse_subscript(pos + 1)?;
                    let span = expr.span;
                    expr = Expr {
                        node: ExprKind::Subscript {
                            value: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    };
                    pos = next;
                }
                TokenKind::Dot => {
                    pos += 1;
                    let token = self.token_at(pos);
                    if token.kind != TokenKind::Name {
                        return Err(self.error_at(pos, "expected attribute name"));
                    }
                    let span = expr.span;
                    expr = Expr {
                        node: ExprKind::Attribute {
                            value: Box::new(expr),
                            name: token.lexeme.clone(),
                        },
                        span,
                    };
                    pos += 1;
                }
                _ => break,
            }
        }

        Ok((expr, pos))
    }

    fn parse_if_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        self.parse_if_after_keyword(pos, pos + 1)
    }

    fn parse_if_after_keyword(&mut self, start: usize, pos: usize) -> ParseResult<Stmt> {
        let mut pos = pos;
        let (test, next) = self.parse_expr_at(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;

        let mut orelse = Vec::new();
        let else_pos = self.skip_newlines(pos);
        if self.match_keyword(else_pos, Keyword::Elif) {
            let (elif_stmt, next) = self.parse_if_after_keyword(else_pos, else_pos + 1)?;
            orelse.push(elif_stmt);
            pos = next;
        } else if self.match_keyword(else_pos, Keyword::Else) {
            pos = else_pos + 1;
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (suite, next) = self.parse_suite(pos)?;
            orelse = suite;
            pos = next;
        }

        Ok((
            self.make_stmt(start, StmtKind::If { test, body, orelse }),
            pos,
        ))
    }

    fn parse_while_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let (test, next) = self.parse_expr_at(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;

        let mut orelse = Vec::new();
        let else_pos = self.skip_newlines(pos);
        if self.match_keyword(else_pos, Keyword::Else) {
            pos = else_pos + 1;
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (suite, next) = self.parse_suite(pos)?;
            orelse = suite;
            pos = next;
        }

        Ok((
            self.make_stmt(start, StmtKind::While { test, body, orelse }),
            pos,
        ))
    }

    fn parse_for_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        self.parse_for_stmt_internal(pos, pos, false)
    }

    fn parse_with_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        self.parse_with_stmt_internal(pos, pos, false)
    }

    fn parse_for_stmt_internal(
        &mut self,
        start: usize,
        for_pos: usize,
        is_async: bool,
    ) -> ParseResult<Stmt> {
        let mut pos = for_pos + 1;
        let (target, next) = self
            .parse_assignment_target_list(pos)
            .ok_or_else(|| self.error_at(pos, "expected loop target"))?;
        pos = next;
        if !self.match_keyword(pos, Keyword::In) {
            return Err(self.error_at(pos, "expected 'in'"));
        }
        pos += 1;
        let (iter, next) = self.parse_expr_at(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;

        let mut orelse = Vec::new();
        let else_pos = self.skip_newlines(pos);
        if self.match_keyword(else_pos, Keyword::Else) {
            pos = else_pos + 1;
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (suite, next) = self.parse_suite(pos)?;
            orelse = suite;
            pos = next;
        }

        Ok((
            self.make_stmt(
                start,
                StmtKind::For {
                    is_async,
                    target,
                    iter,
                    body,
                    orelse,
                },
            ),
            pos,
        ))
    }

    fn parse_with_stmt_internal(
        &mut self,
        start: usize,
        with_pos: usize,
        is_async: bool,
    ) -> ParseResult<Stmt> {
        let mut pos = with_pos + 1;
        let (context, next) = self.parse_expr_at(pos)?;
        pos = next;

        let mut target = None;
        if self.match_keyword(pos, Keyword::As) {
            pos += 1;
            let (target_expr, next) = self
                .parse_assignment_target_list(pos)
                .ok_or_else(|| self.error_at(pos, "expected with target"))?;
            target = Some(target_expr);
            pos = next;
        }

        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;
        Ok((
            self.make_stmt(
                start,
                StmtKind::With {
                    is_async,
                    context,
                    target,
                    body,
                },
            ),
            pos,
        ))
    }

    fn parse_try_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;

        let mut handlers = Vec::new();
        let mut orelse = Vec::new();
        let mut finalbody = Vec::new();

        loop {
            let except_pos = self.skip_newlines(pos);
            if !self.match_keyword(except_pos, Keyword::Except) {
                break;
            }
            pos = except_pos + 1;
            let mut is_star = false;
            if matches!(self.token_at(pos).kind, TokenKind::Star) {
                is_star = true;
                pos += 1;
            }

            let mut type_expr = None;
            let mut name = None;
            if !matches!(self.token_at(pos).kind, TokenKind::Colon) {
                let (expr, next) = self.parse_expr_at(pos)?;
                type_expr = Some(expr);
                pos = next;
                if self.match_keyword(pos, Keyword::As) {
                    pos += 1;
                    let token = self.token_at(pos);
                    if token.kind != TokenKind::Name {
                        return Err(self.error_at(pos, "expected exception name"));
                    }
                    name = Some(token.lexeme.clone());
                    pos += 1;
                }
            }

            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (suite, next) = self.parse_suite(pos)?;
            pos = next;
            handlers.push(ExceptHandler {
                type_expr,
                name,
                is_star,
                body: suite,
            });
        }

        let else_pos = self.skip_newlines(pos);
        if self.match_keyword(else_pos, Keyword::Else) {
            if handlers.is_empty() {
                return Err(self.error_at(else_pos, "else requires except"));
            }
            pos = else_pos + 1;
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (suite, next) = self.parse_suite(pos)?;
            orelse = suite;
            pos = next;
        }

        let finally_pos = self.skip_newlines(pos);
        if self.match_keyword(finally_pos, Keyword::Finally) {
            pos = finally_pos + 1;
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (suite, next) = self.parse_suite(pos)?;
            finalbody = suite;
            pos = next;
        }

        if handlers.is_empty() && finalbody.is_empty() {
            return Err(self.error_at(pos, "try requires except or finally"));
        }

        Ok((
            self.make_stmt(
                start,
                StmtKind::Try {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                },
            ),
            pos,
        ))
    }

    fn parse_import_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let mut names = Vec::new();

        loop {
            let (alias, next) = self.parse_import_alias(pos)?;
            names.push(alias);
            pos = next;

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                continue;
            }
            break;
        }

        Ok((self.make_stmt(start, StmtKind::Import { names }), pos))
    }

    fn parse_from_import_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let mut level = 0usize;
        while matches!(self.token_at(pos).kind, TokenKind::Dot) {
            level += 1;
            pos += 1;
        }

        let module = if self.token_at(pos).kind == TokenKind::Name {
            let (module, next) = self.parse_import_name(pos)?;
            pos = next;
            Some(module)
        } else {
            None
        };

        if level == 0 && module.is_none() {
            return Err(self.error_at(pos, "expected module name"));
        }

        if !self.match_keyword(pos, Keyword::Import) {
            return Err(self.error_at(pos, "expected 'import'"));
        }
        pos += 1;

        let mut names = Vec::new();
        loop {
            let (alias, next) = self.parse_import_alias_name(pos)?;
            names.push(alias);
            pos = next;

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                continue;
            }
            break;
        }

        Ok((
            self.make_stmt(
                start,
                StmtKind::ImportFrom {
                    module,
                    names,
                    level,
                },
            ),
            pos,
        ))
    }

    fn parse_global_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let mut names = Vec::new();

        loop {
            let token = self.token_at(pos);
            if token.kind != TokenKind::Name {
                return Err(self.error_at(pos, "expected global name"));
            }
            names.push(token.lexeme.clone());
            pos += 1;

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                continue;
            }
            break;
        }

        Ok((self.make_stmt(start, StmtKind::Global { names }), pos))
    }

    fn parse_nonlocal_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let mut names = Vec::new();

        loop {
            let token = self.token_at(pos);
            if token.kind != TokenKind::Name {
                return Err(self.error_at(pos, "expected name"));
            }
            names.push(token.lexeme.clone());
            pos += 1;
            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                continue;
            }
            break;
        }

        Ok((self.make_stmt(start, StmtKind::Nonlocal { names }), pos))
    }

    fn parse_raise_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        if matches!(
            self.token_at(pos).kind,
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Dedent | TokenKind::EndMarker
        ) {
            return Ok((self.make_stmt(start, StmtKind::Raise { value: None }), pos));
        }
        let (expr, next) = self.parse_expr_at(pos)?;
        pos = next;
        Ok((
            self.make_stmt(start, StmtKind::Raise { value: Some(expr) }),
            pos,
        ))
    }

    fn parse_assert_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let (test, next) = self.parse_expr_at(pos)?;
        pos = next;

        let mut message = None;
        if matches!(self.token_at(pos).kind, TokenKind::Comma) {
            pos += 1;
            let (expr, next) = self.parse_expr_at(pos)?;
            message = Some(expr);
            pos = next;
        }

        Ok((
            self.make_stmt(start, StmtKind::Assert { test, message }),
            pos,
        ))
    }

    fn parse_import_alias(&mut self, pos: usize) -> Result<(ImportAlias, usize), ParseError> {
        let (name, mut pos) = self.parse_import_name(pos)?;
        let mut asname = None;
        if self.match_keyword(pos, Keyword::As) {
            pos += 1;
            let token = self.token_at(pos);
            if token.kind != TokenKind::Name {
                return Err(self.error_at(pos, "expected alias name"));
            }
            asname = Some(token.lexeme.clone());
            pos += 1;
        }

        Ok((ImportAlias { name, asname }, pos))
    }

    fn parse_import_alias_name(&mut self, pos: usize) -> Result<(ImportAlias, usize), ParseError> {
        let mut pos = pos;
        let token = self.token_at(pos);
        if token.kind != TokenKind::Name {
            return Err(self.error_at(pos, "expected imported name"));
        }
        let name = token.lexeme.clone();
        pos += 1;

        let mut asname = None;
        if self.match_keyword(pos, Keyword::As) {
            pos += 1;
            let token = self.token_at(pos);
            if token.kind != TokenKind::Name {
                return Err(self.error_at(pos, "expected alias name"));
            }
            asname = Some(token.lexeme.clone());
            pos += 1;
        }

        Ok((ImportAlias { name, asname }, pos))
    }

    fn parse_import_name(&mut self, pos: usize) -> Result<(String, usize), ParseError> {
        let mut pos = pos;
        let token = self.token_at(pos);
        if token.kind != TokenKind::Name {
            return Err(self.error_at(pos, "expected module name"));
        }
        let mut name = token.lexeme.clone();
        pos += 1;

        while matches!(self.token_at(pos).kind, TokenKind::Dot) {
            pos += 1;
            let token = self.token_at(pos);
            if token.kind != TokenKind::Name {
                return Err(self.error_at(pos, "expected module name after '.'"));
            }
            name.push('.');
            name.push_str(&token.lexeme);
            pos += 1;
        }

        Ok((name, pos))
    }

    fn parse_assignment_target_list(&mut self, pos: usize) -> Option<(AssignTarget, usize)> {
        let (first, mut pos) = self.parse_assignment_target(pos)?;
        let mut targets = vec![first];

        if matches!(self.token_at(pos).kind, TokenKind::Comma) {
            while matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                if matches!(self.token_at(pos).kind, TokenKind::Equal) {
                    break;
                }
                let (target, next) = self.parse_assignment_target(pos)?;
                targets.push(target);
                pos = next;
            }
            return Some((AssignTarget::Tuple(targets), pos));
        }

        Some((targets.remove(0), pos))
    }

    fn parse_assignment_target(&mut self, pos: usize) -> Option<(AssignTarget, usize)> {
        let token = self.token_at(pos);
        match &token.kind {
            TokenKind::Name => {
                let mut expr = self.make_expr(pos, ExprKind::Name(token.lexeme.clone()));
                let mut pos = pos + 1;

                loop {
                    match self.token_at(pos).kind {
                        TokenKind::LBracket => {
                            let (index, next) = self.parse_subscript(pos + 1).ok()?;
                            let span = expr.span;
                            expr = Expr {
                                node: ExprKind::Subscript {
                                    value: Box::new(expr),
                                    index: Box::new(index),
                                },
                                span,
                            };
                            pos = next;
                        }
                        TokenKind::Dot => {
                            pos += 1;
                            let token = self.token_at(pos);
                            if token.kind != TokenKind::Name {
                                return None;
                            }
                            let span = expr.span;
                            expr = Expr {
                                node: ExprKind::Attribute {
                                    value: Box::new(expr),
                                    name: token.lexeme.clone(),
                                },
                                span,
                            };
                            pos += 1;
                        }
                        _ => break,
                    }
                }

                let target = match expr.node {
                    ExprKind::Name(name) => AssignTarget::Name(name),
                    ExprKind::Subscript { value, index } => {
                        AssignTarget::Subscript { value, index }
                    }
                    ExprKind::Attribute { value, name } => AssignTarget::Attribute { value, name },
                    _ => return None,
                };

                Some((target, pos))
            }
            TokenKind::LParen => {
                let (targets, next) = self.parse_target_sequence(pos + 1, TokenKind::RParen)?;
                Some((AssignTarget::Tuple(targets), next))
            }
            TokenKind::LBracket => {
                let (targets, next) = self.parse_target_sequence(pos + 1, TokenKind::RBracket)?;
                Some((AssignTarget::List(targets), next))
            }
            _ => None,
        }
    }

    fn parse_target_sequence(
        &mut self,
        pos: usize,
        end: TokenKind,
    ) -> Option<(Vec<AssignTarget>, usize)> {
        let mut pos = pos;
        let mut targets = Vec::new();

        if matches!(self.token_at(pos).kind, ref kind if *kind == end) {
            return None;
        }

        loop {
            let (target, next) = self.parse_assignment_target(pos)?;
            targets.push(target);
            pos = next;
            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                if matches!(self.token_at(pos).kind, ref kind if *kind == end) {
                    break;
                }
                continue;
            }
            break;
        }

        if !matches!(self.token_at(pos).kind, ref kind if *kind == end) {
            return None;
        }
        Some((targets, pos + 1))
    }

    fn parse_function_def(&mut self, pos: usize) -> ParseResult<Stmt> {
        self.parse_function_def_internal(pos, pos, false)
    }

    fn parse_function_def_internal(
        &mut self,
        start: usize,
        def_pos: usize,
        is_async: bool,
    ) -> ParseResult<Stmt> {
        let mut pos = def_pos + 1;
        let name_token = self.token_at(pos);
        if name_token.kind != TokenKind::Name {
            return Err(self.error_at(pos, "expected function name"));
        }
        let name = name_token.lexeme.clone();
        pos += 1;
        let (type_params, next) = self.parse_type_params(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::LParen)?;
        let (posonly_params, params, kwonly_params, vararg, kwarg, next) =
            self.parse_parameters(pos)?;
        pos = next;
        let mut returns = None;
        if matches!(self.token_at(pos).kind, TokenKind::Arrow) {
            let (expr, next) = self.parse_expr_at(pos + 1)?;
            returns = Some(expr);
            pos = next;
        }
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;
        Ok((
            self.make_stmt(
                start,
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
                },
            ),
            pos,
        ))
    }

    fn parse_class_def(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        let name_token = self.token_at(pos);
        if name_token.kind != TokenKind::Name {
            return Err(self.error_at(pos, "expected class name"));
        }
        let name = name_token.lexeme.clone();
        pos += 1;
        let (type_params, next) = self.parse_type_params(pos)?;
        pos = next;

        let mut bases = Vec::new();
        if matches!(self.token_at(pos).kind, TokenKind::LParen) {
            let (args, next) = self.parse_call_args(pos + 1)?;
            for arg in args {
                match arg {
                    CallArg::Positional(expr) => bases.push(expr),
                    CallArg::Keyword { .. } | CallArg::Star(_) | CallArg::DoubleStar(_) => {
                        return Err(self.error_at(pos, "class bases cannot be keyword arguments"));
                    }
                }
            }
            pos = next;
        }

        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;
        Ok((
            self.make_stmt(
                start,
                StmtKind::ClassDef {
                    name,
                    type_params,
                    bases,
                    body,
                },
            ),
            pos,
        ))
    }

    fn parse_type_params(&mut self, pos: usize) -> Result<(Vec<String>, usize), ParseError> {
        if !matches!(self.token_at(pos).kind, TokenKind::LBracket) {
            return Ok((Vec::new(), pos));
        }
        let mut pos = pos + 1;
        let mut params = Vec::new();
        if matches!(self.token_at(pos).kind, TokenKind::RBracket) {
            return Err(self.error_at(pos, "type parameter list cannot be empty"));
        }
        loop {
            let token = self.token_at(pos);
            if token.kind != TokenKind::Name {
                return Err(self.error_at(pos, "expected type parameter name"));
            }
            params.push(token.lexeme.clone());
            pos += 1;
            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                continue;
            }
            break;
        }
        pos = self.expect_kind(pos, TokenKind::RBracket)?;
        Ok((params, pos))
    }

    fn parse_return_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let start = pos;
        let mut pos = pos + 1;
        if matches!(
            self.token_at(pos).kind,
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Dedent | TokenKind::EndMarker
        ) {
            return Ok((self.make_stmt(start, StmtKind::Return { value: None }), pos));
        }
        let (expr, next) = self.parse_expr_at(pos)?;
        pos = next;
        Ok((
            self.make_stmt(start, StmtKind::Return { value: Some(expr) }),
            pos,
        ))
    }

    fn parse_suite(&mut self, pos: usize) -> Result<(Vec<Stmt>, usize), ParseError> {
        match self.token_at(pos).kind {
            TokenKind::Newline => self.parse_block_suite(pos),
            _ => self.parse_inline_suite(pos),
        }
    }

    fn parse_call_args(&mut self, pos: usize) -> Result<(Vec<CallArg>, usize), ParseError> {
        let mut pos = pos;
        let mut args = Vec::new();

        if matches!(self.token_at(pos).kind, TokenKind::RParen) {
            return Ok((args, pos + 1));
        }

        loop {
            let token = self.token_at(pos);
            if token.kind == TokenKind::Star {
                pos += 1;
                let (expr, next) = self.parse_expr_at(pos)?;
                args.push(CallArg::Star(expr));
                pos = next;
            } else if token.kind == TokenKind::DoubleStar {
                pos += 1;
                let (expr, next) = self.parse_expr_at(pos)?;
                args.push(CallArg::DoubleStar(expr));
                pos = next;
            } else if token.kind == TokenKind::Name
                && matches!(self.token_at(pos + 1).kind, TokenKind::Equal)
            {
                let name = token.lexeme.clone();
                pos += 2;
                let (value, next) = self.parse_expr_at(pos)?;
                args.push(CallArg::Keyword { name, value });
                pos = next;
            } else {
                let (expr, next) = self.parse_expr_at(pos)?;
                args.push(CallArg::Positional(expr));
                pos = next;
            }

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                if matches!(self.token_at(pos).kind, TokenKind::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        pos = self.expect_kind(pos, TokenKind::RParen)?;
        Ok((args, pos))
    }

    fn parse_int_literal(&self, lexeme: &str, pos: usize) -> Result<i64, ParseError> {
        let normalized = lexeme.replace('_', "");
        if let Some(rest) = normalized
            .strip_prefix("0x")
            .or_else(|| normalized.strip_prefix("0X"))
        {
            return i64::from_str_radix(rest, 16)
                .map_err(|_| self.error_at(pos, "invalid integer literal"));
        }
        if let Some(rest) = normalized
            .strip_prefix("0o")
            .or_else(|| normalized.strip_prefix("0O"))
        {
            return i64::from_str_radix(rest, 8)
                .map_err(|_| self.error_at(pos, "invalid integer literal"));
        }
        if let Some(rest) = normalized
            .strip_prefix("0b")
            .or_else(|| normalized.strip_prefix("0B"))
        {
            return i64::from_str_radix(rest, 2)
                .map_err(|_| self.error_at(pos, "invalid integer literal"));
        }
        normalized
            .parse::<i64>()
            .map_err(|_| self.error_at(pos, "invalid integer literal"))
    }

    fn parse_fstring_literal(&mut self, pos: usize, content: &str) -> Result<Expr, ParseError> {
        let span = self.span_at(pos);
        let mut pieces: Vec<Expr> = Vec::new();
        let mut literal = String::new();
        let mut chars = content.char_indices().peekable();

        while let Some((idx, ch)) = chars.next() {
            if ch == '{' {
                if matches!(chars.peek(), Some((_, '{'))) {
                    chars.next();
                    literal.push('{');
                    continue;
                }
                if !literal.is_empty() {
                    pieces.push(Expr {
                        node: ExprKind::Constant(Constant::Str(std::mem::take(&mut literal))),
                        span,
                    });
                }

                let expr_start = idx + ch.len_utf8();
                let mut depth = 1usize;
                let mut expr_end = None;
                for (inner_idx, inner_ch) in chars.by_ref() {
                    if inner_ch == '{' {
                        depth += 1;
                    } else if inner_ch == '}' {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            expr_end = Some(inner_idx);
                            break;
                        }
                    }
                }

                let end = expr_end.ok_or_else(|| self.error_at(pos, "unterminated f-string"))?;
                let expr_text = content[expr_start..end].trim();
                if expr_text.is_empty() {
                    return Err(self.error_at(pos, "empty f-string expression"));
                }
                let embedded = self.parse_embedded_expr(expr_text).map_err(|err| {
                    self.error_at(
                        pos,
                        format!("invalid f-string expression: {}", err.message),
                    )
                })?;
                let str_name = Expr {
                    node: ExprKind::Name("str".to_string()),
                    span,
                };
                pieces.push(Expr {
                    node: ExprKind::Call {
                        func: Box::new(str_name),
                        args: vec![CallArg::Positional(embedded)],
                    },
                    span,
                });
                continue;
            }

            if ch == '}' {
                if matches!(chars.peek(), Some((_, '}'))) {
                    chars.next();
                    literal.push('}');
                    continue;
                }
                return Err(self.error_at(pos, "single '}' is not allowed in f-string"));
            }

            literal.push(ch);
        }

        if !literal.is_empty() {
            pieces.push(Expr {
                node: ExprKind::Constant(Constant::Str(literal)),
                span,
            });
        }

        if pieces.is_empty() {
            return Ok(Expr {
                node: ExprKind::Constant(Constant::Str(String::new())),
                span,
            });
        }

        let mut iter = pieces.into_iter();
        let mut expr = iter.next().expect("non-empty");
        for piece in iter {
            expr = Expr {
                span,
                node: ExprKind::Binary {
                    left: Box::new(expr),
                    op: BinaryOp::Add,
                    right: Box::new(piece),
                },
            };
        }
        Ok(expr)
    }

    fn parse_embedded_expr(&self, source: &str) -> Result<Expr, ParseError> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().map_err(ParseError::from)?;
        let mut parser = Parser::new(tokens);
        let (expr, pos) = parser.parse_expr_at(0)?;
        parser.expect_end(pos)?;
        Ok(expr)
    }

    fn is_comprehension_start(&self, pos: usize) -> bool {
        self.match_keyword(pos, Keyword::For)
            || (self.match_keyword(pos, Keyword::Async) && self.match_keyword(pos + 1, Keyword::For))
    }

    fn parse_comp_clauses(
        &mut self,
        mut pos: usize,
    ) -> Result<(Vec<ComprehensionClause>, usize), ParseError> {
        let mut clauses = Vec::new();
        loop {
            let mut is_async = false;
            if self.match_keyword(pos, Keyword::Async) {
                is_async = true;
                pos += 1;
            }
            if !self.match_keyword(pos, Keyword::For) {
                if clauses.is_empty() {
                    return Err(self.error_at(pos, "expected comprehension for-clause"));
                }
                break;
            }
            pos += 1;
            let (target, next) = self
                .parse_assignment_target_list(pos)
                .ok_or_else(|| self.error_at(pos, "expected comprehension target"))?;
            pos = next;
            if !self.match_keyword(pos, Keyword::In) {
                return Err(self.error_at(pos, "expected 'in' in comprehension"));
            }
            pos += 1;
            let (iter, next) = self.parse_or(pos)?;
            pos = next;

            let mut ifs = Vec::new();
            while self.match_keyword(pos, Keyword::If) {
                let (cond, next) = self.parse_or(pos + 1)?;
                ifs.push(cond);
                pos = next;
            }
            clauses.push(ComprehensionClause {
                is_async,
                target,
                iter,
                ifs,
            });

            if !self.is_comprehension_start(pos) {
                break;
            }
        }
        Ok((clauses, pos))
    }

    fn parse_list_or_comp(&mut self, pos: usize) -> Result<(Expr, usize), ParseError> {
        let start = pos.saturating_sub(1);
        if matches!(self.token_at(pos).kind, TokenKind::RBracket) {
            return Ok((self.make_expr(start, ExprKind::List(Vec::new())), pos + 1));
        }
        let (first, mut pos) = self.parse_expr_at(pos)?;
        if self.is_comprehension_start(pos) {
            let (clauses, next) = self.parse_comp_clauses(pos)?;
            let pos = self.expect_kind(next, TokenKind::RBracket)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::ListComp {
                        elt: Box::new(first),
                        clauses,
                    },
                ),
                pos,
            ));
        }

        let mut elements = vec![first];
        while matches!(self.token_at(pos).kind, TokenKind::Comma) {
            pos += 1;
            if matches!(self.token_at(pos).kind, TokenKind::RBracket) {
                break;
            }
            let (expr, next) = self.parse_expr_at(pos)?;
            elements.push(expr);
            pos = next;
        }
        pos = self.expect_kind(pos, TokenKind::RBracket)?;
        Ok((self.make_expr(start, ExprKind::List(elements)), pos))
    }

    fn parse_dict_or_comp(&mut self, pos: usize) -> Result<(Expr, usize), ParseError> {
        let start = pos.saturating_sub(1);
        if matches!(self.token_at(pos).kind, TokenKind::RBrace) {
            return Ok((self.make_expr(start, ExprKind::Dict(Vec::new())), pos + 1));
        }

        let (first_key, mut pos) = self.parse_expr_at(pos)?;
        if !matches!(self.token_at(pos).kind, TokenKind::Colon) {
            return Err(self.error_at(pos, "set literals are not supported"));
        }
        pos += 1;
        let (first_value, mut pos) = self.parse_expr_at(pos)?;

        if self.is_comprehension_start(pos) {
            let (clauses, next) = self.parse_comp_clauses(pos)?;
            let pos = self.expect_kind(next, TokenKind::RBrace)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::DictComp {
                        key: Box::new(first_key),
                        value: Box::new(first_value),
                        clauses,
                    },
                ),
                pos,
            ));
        }

        let mut entries = vec![(first_key, first_value)];
        while matches!(self.token_at(pos).kind, TokenKind::Comma) {
            pos += 1;
            if matches!(self.token_at(pos).kind, TokenKind::RBrace) {
                break;
            }
            let (key, next) = self.parse_expr_at(pos)?;
            pos = next;
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (value, next) = self.parse_expr_at(pos)?;
            entries.push((key, value));
            pos = next;
        }

        pos = self.expect_kind(pos, TokenKind::RBrace)?;
        Ok((self.make_expr(start, ExprKind::Dict(entries)), pos))
    }

    fn parse_list_elements(&mut self, pos: usize) -> Result<(Vec<Expr>, usize), ParseError> {
        let mut pos = pos;
        let mut elements = Vec::new();

        if matches!(self.token_at(pos).kind, TokenKind::RBracket) {
            return Ok((elements, pos + 1));
        }

        loop {
            let (expr, next) = self.parse_expr_at(pos)?;
            elements.push(expr);
            pos = next;

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                if matches!(self.token_at(pos).kind, TokenKind::RBracket) {
                    break;
                }
                continue;
            }
            break;
        }

        pos = self.expect_kind(pos, TokenKind::RBracket)?;
        Ok((elements, pos))
    }

    fn parse_paren_expr(&mut self, pos: usize) -> Result<(Expr, usize), ParseError> {
        let start = pos.saturating_sub(1);
        if matches!(self.token_at(pos).kind, TokenKind::RParen) {
            return Ok((
                self.make_expr(start, ExprKind::Tuple(Vec::new())),
                pos + 1,
            ));
        }

        let (first, mut pos) = self.parse_expr_at(pos)?;
        if self.is_comprehension_start(pos) {
            let (clauses, next) = self.parse_comp_clauses(pos)?;
            let pos = self.expect_kind(next, TokenKind::RParen)?;
            return Ok((
                self.make_expr(
                    start,
                    ExprKind::GeneratorExp {
                        elt: Box::new(first),
                        clauses,
                    },
                ),
                pos,
            ));
        }
        if !matches!(self.token_at(pos).kind, TokenKind::Comma) {
            let pos = self.expect_kind(pos, TokenKind::RParen)?;
            return Ok((first, pos));
        }

        let mut elements = vec![first];
        while matches!(self.token_at(pos).kind, TokenKind::Comma) {
            pos += 1;
            if matches!(self.token_at(pos).kind, TokenKind::RParen) {
                break;
            }
            let (expr, next) = self.parse_expr_at(pos)?;
            elements.push(expr);
            pos = next;
        }

        pos = self.expect_kind(pos, TokenKind::RParen)?;
        let span = elements
            .first()
            .map(|expr| expr.span)
            .unwrap_or_else(Span::unknown);
        Ok((
            Expr {
                node: ExprKind::Tuple(elements),
                span,
            },
            pos,
        ))
    }

    fn parse_dict_entries(&mut self, pos: usize) -> Result<(Vec<(Expr, Expr)>, usize), ParseError> {
        let mut pos = pos;
        let mut entries = Vec::new();

        if matches!(self.token_at(pos).kind, TokenKind::RBrace) {
            return Ok((entries, pos + 1));
        }

        loop {
            let (key, next) = self.parse_expr_at(pos)?;
            pos = next;
            pos = self.expect_kind(pos, TokenKind::Colon)?;
            let (value, next) = self.parse_expr_at(pos)?;
            entries.push((key, value));
            pos = next;

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                if matches!(self.token_at(pos).kind, TokenKind::RBrace) {
                    break;
                }
                continue;
            }
            break;
        }

        pos = self.expect_kind(pos, TokenKind::RBrace)?;
        Ok((entries, pos))
    }

    fn parse_subscript(&mut self, pos: usize) -> Result<(Expr, usize), ParseError> {
        let mut pos = pos;
        if matches!(self.token_at(pos).kind, TokenKind::Colon) {
            return self.parse_slice(None, pos);
        }

        let (expr, next) = self.parse_expr_at(pos)?;
        pos = next;
        if matches!(self.token_at(pos).kind, TokenKind::Colon) {
            return self.parse_slice(Some(expr), pos);
        }

        pos = self.expect_kind(pos, TokenKind::RBracket)?;
        Ok((expr, pos))
    }

    fn parse_slice(
        &mut self,
        lower: Option<Expr>,
        pos: usize,
    ) -> Result<(Expr, usize), ParseError> {
        let mut pos = pos;
        pos = self.expect_kind(pos, TokenKind::Colon)?;

        let mut upper = None;
        if !matches!(
            self.token_at(pos).kind,
            TokenKind::Colon | TokenKind::RBracket
        ) {
            let (expr, next) = self.parse_expr_at(pos)?;
            upper = Some(expr);
            pos = next;
        }

        let mut step = None;
        if matches!(self.token_at(pos).kind, TokenKind::Colon) {
            pos += 1;
            if !matches!(self.token_at(pos).kind, TokenKind::RBracket) {
                let (expr, next) = self.parse_expr_at(pos)?;
                step = Some(expr);
                pos = next;
            }
        }

        pos = self.expect_kind(pos, TokenKind::RBracket)?;
        let span = lower
            .as_ref()
            .map(|expr| expr.span)
            .unwrap_or_else(|| self.span_at(pos.saturating_sub(1)));
        Ok((
            Expr {
                node: ExprKind::Slice {
                    lower: lower.map(Box::new),
                    upper: upper.map(Box::new),
                    step: step.map(Box::new),
                },
                span,
            },
            pos,
        ))
    }

    fn parse_parameters(
        &mut self,
        pos: usize,
    ) -> Result<
        (
            Vec<Parameter>,
            Vec<Parameter>,
            Vec<Parameter>,
            Option<Parameter>,
            Option<Parameter>,
            usize,
        ),
        ParseError,
    > {
        let mut pos = pos;
        let mut posonly_params = Vec::new();
        let mut params = Vec::new();
        let mut kwonly_params = Vec::new();
        let mut vararg: Option<Parameter> = None;
        let mut kwarg: Option<Parameter> = None;
        let mut saw_default = false;
        let mut saw_kwonly_default = false;
        let mut keyword_only = false;
        let mut saw_slash = false;

        if matches!(self.token_at(pos).kind, TokenKind::RParen) {
            return Ok((posonly_params, params, kwonly_params, None, None, pos + 1));
        }

        loop {
            let token = self.token_at(pos);
            match token.kind {
                TokenKind::Slash => {
                    if saw_slash {
                        return Err(self.error_at(pos, "multiple '/' in parameters"));
                    }
                    if keyword_only || vararg.is_some() || kwarg.is_some() {
                        return Err(self.error_at(pos, "invalid '/' position"));
                    }
                    saw_slash = true;
                    posonly_params = params;
                    params = Vec::new();
                    pos += 1;
                }
                TokenKind::Star => {
                    if vararg.is_some() {
                        return Err(self.error_at(pos, "multiple *args parameters"));
                    }
                    pos += 1;
                    let name_token = self.token_at(pos);
                    if name_token.kind == TokenKind::Comma || name_token.kind == TokenKind::RParen {
                        keyword_only = true;
                        if name_token.kind == TokenKind::Comma {
                            pos += 1;
                            continue;
                        }
                        break;
                    }
                    if name_token.kind != TokenKind::Name {
                        return Err(self.error_at(pos, "expected parameter name after *"));
                    }
                    let name = name_token.lexeme.clone();
                    pos += 1;
                    let mut annotation = None;
                    if matches!(self.token_at(pos).kind, TokenKind::Colon) {
                        let (expr, next) = self.parse_expr_at(pos + 1)?;
                        annotation = Some(Box::new(expr));
                        pos = next;
                    }
                    vararg = Some(Parameter {
                        name,
                        default: None,
                        annotation,
                    });
                    keyword_only = true;
                }
                TokenKind::DoubleStar => {
                    if kwarg.is_some() {
                        return Err(self.error_at(pos, "multiple **kwargs parameters"));
                    }
                    pos += 1;
                    let name_token = self.token_at(pos);
                    if name_token.kind != TokenKind::Name {
                        return Err(self.error_at(pos, "expected parameter name after **"));
                    }
                    let name = name_token.lexeme.clone();
                    pos += 1;
                    let mut annotation = None;
                    if matches!(self.token_at(pos).kind, TokenKind::Colon) {
                        let (expr, next) = self.parse_expr_at(pos + 1)?;
                        annotation = Some(Box::new(expr));
                        pos = next;
                    }
                    kwarg = Some(Parameter {
                        name,
                        default: None,
                        annotation,
                    });
                }
                TokenKind::Name => {
                    let name = token.lexeme.clone();
                    pos += 1;

                    let mut annotation = None;
                    if matches!(self.token_at(pos).kind, TokenKind::Colon) {
                        let (expr, next) = self.parse_expr_at(pos + 1)?;
                        annotation = Some(Box::new(expr));
                        pos = next;
                    }

                    let mut default = None;
                    if matches!(self.token_at(pos).kind, TokenKind::Equal) {
                        pos += 1;
                        let (expr, next) = self.parse_expr_at(pos)?;
                        default = Some(Box::new(expr));
                        pos = next;
                        if keyword_only {
                            saw_kwonly_default = true;
                        } else {
                            saw_default = true;
                        }
                    } else if keyword_only && saw_kwonly_default {
                        return Err(self.error_at(pos, "non-default parameter follows default"));
                    } else if !keyword_only && saw_default {
                        return Err(self.error_at(pos, "non-default parameter follows default"));
                    }

                    let param = Parameter {
                        name,
                        default,
                        annotation,
                    };
                    if keyword_only {
                        kwonly_params.push(param);
                    } else {
                        params.push(param);
                    }
                }
                _ => return Err(self.error_at(pos, "expected parameter name")),
            }

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                if matches!(self.token_at(pos).kind, TokenKind::RParen) {
                    break;
                }
                if kwarg.is_some() {
                    return Err(self.error_at(pos, "**kwargs must be last parameter"));
                }
                continue;
            }
            break;
        }

        pos = self.expect_kind(pos, TokenKind::RParen)?;
        Ok((posonly_params, params, kwonly_params, vararg, kwarg, pos))
    }

    fn parse_lambda_params(
        &mut self,
        pos: usize,
    ) -> Result<
        (
            Vec<Parameter>,
            Vec<Parameter>,
            Vec<Parameter>,
            Option<Parameter>,
            Option<Parameter>,
            usize,
        ),
        ParseError,
    > {
        let mut pos = pos;
        let mut posonly_params = Vec::new();
        let mut params = Vec::new();
        let mut kwonly_params = Vec::new();
        let mut vararg: Option<Parameter> = None;
        let mut kwarg: Option<Parameter> = None;
        let mut saw_default = false;
        let mut saw_kwonly_default = false;
        let mut keyword_only = false;
        let mut saw_slash = false;

        loop {
            let token = self.token_at(pos);
            match token.kind {
                TokenKind::Slash => {
                    if saw_slash {
                        return Err(self.error_at(pos, "multiple '/' in parameters"));
                    }
                    if keyword_only || vararg.is_some() || kwarg.is_some() {
                        return Err(self.error_at(pos, "invalid '/' position"));
                    }
                    saw_slash = true;
                    posonly_params = params;
                    params = Vec::new();
                    pos += 1;
                }
                TokenKind::Star => {
                    if vararg.is_some() {
                        return Err(self.error_at(pos, "multiple *args parameters"));
                    }
                    pos += 1;
                    let name_token = self.token_at(pos);
                    if name_token.kind == TokenKind::Comma || name_token.kind == TokenKind::Colon {
                        keyword_only = true;
                        if name_token.kind == TokenKind::Comma {
                            pos += 1;
                            continue;
                        }
                        break;
                    }
                    if name_token.kind != TokenKind::Name {
                        return Err(self.error_at(pos, "expected parameter name after *"));
                    }
                    let name = name_token.lexeme.clone();
                    pos += 1;
                    let (annotation, next) = self.parse_lambda_annotation(pos)?;
                    pos = next;
                    vararg = Some(Parameter {
                        name,
                        default: None,
                        annotation,
                    });
                    keyword_only = true;
                }
                TokenKind::DoubleStar => {
                    if kwarg.is_some() {
                        return Err(self.error_at(pos, "multiple **kwargs parameters"));
                    }
                    pos += 1;
                    let name_token = self.token_at(pos);
                    if name_token.kind != TokenKind::Name {
                        return Err(self.error_at(pos, "expected parameter name after **"));
                    }
                    let name = name_token.lexeme.clone();
                    pos += 1;
                    let (annotation, next) = self.parse_lambda_annotation(pos)?;
                    pos = next;
                    kwarg = Some(Parameter {
                        name,
                        default: None,
                        annotation,
                    });
                }
                TokenKind::Name => {
                    let name = token.lexeme.clone();
                    pos += 1;
                    let (annotation, next) = self.parse_lambda_annotation(pos)?;
                    pos = next;

                    let mut default = None;
                    if matches!(self.token_at(pos).kind, TokenKind::Equal) {
                        pos += 1;
                        let (expr, next) = self.parse_expr_at(pos)?;
                        default = Some(Box::new(expr));
                        pos = next;
                        if keyword_only {
                            saw_kwonly_default = true;
                        } else {
                            saw_default = true;
                        }
                    } else if keyword_only && saw_kwonly_default {
                        return Err(self.error_at(pos, "non-default parameter follows default"));
                    } else if !keyword_only && saw_default {
                        return Err(self.error_at(pos, "non-default parameter follows default"));
                    }

                    let param = Parameter {
                        name,
                        default,
                        annotation,
                    };
                    if keyword_only {
                        kwonly_params.push(param);
                    } else {
                        params.push(param);
                    }
                }
                _ => return Err(self.error_at(pos, "expected parameter name")),
            }

            if matches!(self.token_at(pos).kind, TokenKind::Comma) {
                pos += 1;
                if matches!(self.token_at(pos).kind, TokenKind::Colon) {
                    return Err(self.error_at(pos, "expected parameter name"));
                }
                if kwarg.is_some() {
                    return Err(self.error_at(pos, "**kwargs must be last parameter"));
                }
                continue;
            }
            break;
        }

        Ok((posonly_params, params, kwonly_params, vararg, kwarg, pos))
    }

    fn parse_lambda_annotation(
        &mut self,
        pos: usize,
    ) -> Result<(Option<Box<Expr>>, usize), ParseError> {
        if !matches!(self.token_at(pos).kind, TokenKind::Colon) {
            return Ok((None, pos));
        }
        let (expr, next) = self.parse_expr_at(pos + 1)?;
        let next_kind = &self.token_at(next).kind;
        let allowed = matches!(
            next_kind,
            TokenKind::Comma
                | TokenKind::Equal
                | TokenKind::Slash
                | TokenKind::Star
                | TokenKind::DoubleStar
                | TokenKind::Colon
        );
        if !allowed {
            return Ok((None, pos));
        }
        Ok((Some(Box::new(expr)), next))
    }

    fn parse_block_suite(&mut self, pos: usize) -> Result<(Vec<Stmt>, usize), ParseError> {
        let mut pos = pos;
        pos = self.expect_kind(pos, TokenKind::Newline)?;
        pos = self.expect_kind(pos, TokenKind::Indent)?;

        let mut body = Vec::new();
        pos = self.consume_separators(pos);

        while !matches!(
            self.token_at(pos).kind,
            TokenKind::Dedent | TokenKind::EndMarker
        ) {
            let (stmt, next) = self.parse_stmt_at(pos)?;
            let allows_missing = stmt_allows_missing_terminator(&stmt);
            body.push(stmt);
            let next_kind = &self.token_at(next).kind;
            if matches!(
                next_kind,
                TokenKind::Newline | TokenKind::Semicolon | TokenKind::EndMarker
            ) {
                pos = self.consume_terminators(next)?;
            } else if matches!(next_kind, TokenKind::Dedent) || allows_missing {
                pos = next;
            } else {
                return Err(self.error_at(next, "expected statement terminator"));
            }
            pos = self.consume_separators(pos);
        }

        pos = self.expect_kind(pos, TokenKind::Dedent)?;
        Ok((body, pos))
    }

    fn parse_inline_suite(&mut self, pos: usize) -> Result<(Vec<Stmt>, usize), ParseError> {
        let mut pos = pos;
        let mut body = Vec::new();

        let (stmt, next) = self.parse_stmt_at(pos)?;
        body.push(stmt);
        pos = next;

        while matches!(self.token_at(pos).kind, TokenKind::Semicolon) {
            pos += 1;
            if matches!(
                self.token_at(pos).kind,
                TokenKind::Newline | TokenKind::EndMarker
            ) {
                break;
            }
            let (stmt, next) = self.parse_stmt_at(pos)?;
            body.push(stmt);
            pos = next;
        }

        if !matches!(
            self.token_at(pos).kind,
            TokenKind::Newline | TokenKind::EndMarker
        ) {
            return Err(self.error_at(pos, "expected newline after inline suite"));
        }

        Ok((body, pos))
    }

    fn consume_separators(&self, mut pos: usize) -> usize {
        while matches!(
            self.token_at(pos).kind,
            TokenKind::Newline | TokenKind::Semicolon
        ) {
            pos += 1;
        }
        pos
    }

    fn skip_newlines(&self, mut pos: usize) -> usize {
        while matches!(self.token_at(pos).kind, TokenKind::Newline) {
            pos += 1;
        }
        pos
    }

    fn consume_terminators(&self, pos: usize) -> Result<usize, ParseError> {
        if self.is_end(pos) {
            return Ok(pos);
        }

        let mut pos = pos;
        let mut consumed = false;
        while matches!(
            self.token_at(pos).kind,
            TokenKind::Newline | TokenKind::Semicolon
        ) {
            consumed = true;
            pos += 1;
        }

        if consumed {
            Ok(pos)
        } else {
            Err(self.error_at(pos, "expected statement terminator"))
        }
    }

    fn expect_kind(&self, pos: usize, kind: TokenKind) -> Result<usize, ParseError> {
        let token = self.token_at(pos);
        if token.kind == kind {
            Ok(pos + 1)
        } else {
            Err(self.error_at(pos, format!("expected {:?}", kind)))
        }
    }

    fn match_keyword(&self, pos: usize, keyword: Keyword) -> bool {
        matches!(self.token_at(pos).kind, TokenKind::Keyword(k) if k == keyword)
    }

    fn match_soft_keyword(&self, pos: usize, keyword: Keyword, lexeme: &str) -> bool {
        self.match_keyword(pos, keyword)
            || matches!(self.token_at(pos).kind, TokenKind::Name)
                && self.token_at(pos).lexeme == lexeme
    }

    fn expect_end(&self, pos: usize) -> Result<(), ParseError> {
        if self.is_end(pos) {
            Ok(())
        } else {
            Err(self.error_at(pos, "unexpected token"))
        }
    }

    fn is_end(&self, pos: usize) -> bool {
        matches!(self.token_at(pos).kind, TokenKind::EndMarker)
    }

    fn token_at(&self, pos: usize) -> &Token {
        self.tokens
            .get(pos)
            .unwrap_or_else(|| self.tokens.last().expect("token stream is empty"))
    }

    fn error_at(&self, pos: usize, message: impl Into<String>) -> ParseError {
        let token = self.token_at(pos);
        ParseError::new(message, token.offset, token.line, token.column)
    }
}

fn stmt_allows_missing_terminator(stmt: &Stmt) -> bool {
    matches!(
        stmt.node,
        StmtKind::If { .. }
            | StmtKind::While { .. }
            | StmtKind::For { .. }
            | StmtKind::FunctionDef { .. }
            | StmtKind::ClassDef { .. }
            | StmtKind::Try { .. }
            | StmtKind::With { .. }
            | StmtKind::Match { .. }
            | StmtKind::Decorated { .. }
    )
}
