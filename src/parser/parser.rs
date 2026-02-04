use std::collections::HashMap;

use crate::ast::{BinaryOp, BoolOp, Constant, Expr, Module, Stmt, UnaryOp};
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

    fn parse_module_at(&mut self, pos: usize) -> ParseResult<Module> {
        if let Some(entry) = self.module_memo.get(&pos) {
            return entry.result.clone();
        }

        let result = self.parse_module_uncached(pos);
        self.module_memo
            .insert(pos, Memo { result: result.clone() });
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
            if matches!(next_kind, TokenKind::Newline | TokenKind::Semicolon | TokenKind::EndMarker)
            {
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
        self.stmt_memo
            .insert(pos, Memo { result: result.clone() });
        result
    }

    fn parse_stmt_uncached(&mut self, pos: usize) -> ParseResult<Stmt> {
        if let Some((target_expr, next_pos)) = self.parse_assignment_target(pos) {
            let kind = self.token_at(next_pos).kind.clone();
            if kind == TokenKind::Equal {
                let (value, next) = self.parse_expr_at(next_pos + 1)?;
                return match target_expr {
                    Expr::Name(name) => Ok((Stmt::Assign { target: name, value }, next)),
                    _ => Ok((Stmt::AssignSubscript { target: target_expr, value }, next)),
                };
            }

            let aug_op = match kind {
                TokenKind::PlusEqual => Some(crate::ast::AugOp::Add),
                TokenKind::MinusEqual => Some(crate::ast::AugOp::Sub),
                TokenKind::StarEqual => Some(crate::ast::AugOp::Mul),
                _ => None,
            };

            if let Some(op) = aug_op {
                let (value, next) = self.parse_expr_at(next_pos + 1)?;
                return Ok((Stmt::AugAssign { target: target_expr, op, value }, next));
            }
        }
        let token = self.token_at(pos);
        match token.kind {
            TokenKind::Keyword(Keyword::Def) => self.parse_function_def(pos),
            TokenKind::Keyword(Keyword::Return) => self.parse_return_stmt(pos),
            TokenKind::Keyword(Keyword::If) => self.parse_if_stmt(pos),
            TokenKind::Keyword(Keyword::While) => self.parse_while_stmt(pos),
            TokenKind::Keyword(Keyword::For) => self.parse_for_stmt(pos),
            TokenKind::Keyword(Keyword::Break) => Ok((Stmt::Break, pos + 1)),
            TokenKind::Keyword(Keyword::Continue) => Ok((Stmt::Continue, pos + 1)),
            TokenKind::Keyword(Keyword::Pass) => Ok((Stmt::Pass, pos + 1)),
            _ => {
                let (expr, next) = self.parse_expr_at(pos)?;
                Ok((Stmt::Expr(expr), next))
            }
        }
    }

    fn parse_expr_at(&mut self, pos: usize) -> ParseResult<Expr> {
        if let Some(entry) = self.expr_memo.get(&pos) {
            return entry.result.clone();
        }

        let result = self.parse_expr_uncached(pos);
        self.expr_memo
            .insert(pos, Memo { result: result.clone() });
        result
    }

    fn parse_expr_uncached(&mut self, pos: usize) -> ParseResult<Expr> {
        self.parse_if_expr(pos)
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
            return Ok((
                Expr::IfExpr {
                    test: Box::new(test),
                    body: Box::new(body),
                    orelse: Box::new(orelse),
                },
                next,
            ));
        }
        Ok((body, pos))
    }

    fn parse_or(&mut self, pos: usize) -> ParseResult<Expr> {
        let (mut left, mut pos) = self.parse_and(pos)?;
        while self.match_keyword(pos, Keyword::Or) {
            pos += 1;
            let (right, next) = self.parse_and(pos)?;
            left = Expr::BoolOp {
                op: BoolOp::Or,
                left: Box::new(left),
                right: Box::new(right),
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
            left = Expr::BoolOp {
                op: BoolOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
            pos = next;
        }
        Ok((left, pos))
    }

    fn parse_not(&mut self, pos: usize) -> ParseResult<Expr> {
        if self.match_keyword(pos, Keyword::Not) {
            let (expr, next) = self.parse_not(pos + 1)?;
            return Ok((
                Expr::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(expr),
                },
                next,
            ));
        }
        self.parse_comparison(pos)
    }

    fn parse_comparison(&mut self, pos: usize) -> ParseResult<Expr> {
        let (left, mut pos) = self.parse_add_sub(pos)?;

        let (op, consumed) = match self.token_at(pos).kind {
            TokenKind::DoubleEqual => (BinaryOp::Eq, 1),
            TokenKind::Less => (BinaryOp::Lt, 1),
            TokenKind::Keyword(Keyword::In) => (BinaryOp::In, 1),
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
        Ok((
            Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
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
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
            pos = next;
        }

        Ok((left, pos))
    }

    fn parse_mul(&mut self, pos: usize) -> ParseResult<Expr> {
        let (mut left, mut pos) = self.parse_unary(pos)?;

        loop {
            if !matches!(self.token_at(pos).kind, TokenKind::Star) {
                break;
            }
            pos += 1;
            let (right, next) = self.parse_unary(pos)?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::Mul,
                right: Box::new(right),
            };
            pos = next;
        }

        Ok((left, pos))
    }

    fn parse_unary(&mut self, pos: usize) -> ParseResult<Expr> {
        if matches!(self.token_at(pos).kind, TokenKind::Minus) {
            let (expr, next) = self.parse_unary(pos + 1)?;
            return Ok((
                Expr::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(expr),
                },
                next,
            ));
        }
        self.parse_atom(pos)
    }

    fn parse_atom(&mut self, pos: usize) -> ParseResult<Expr> {
        let token = self.token_at(pos);
        let (mut expr, mut pos) = match &token.kind {
            TokenKind::Name => (Expr::Name(token.lexeme.clone()), pos + 1),
            TokenKind::Number => {
                let value = token
                    .lexeme
                    .parse::<i64>()
                    .map_err(|_| self.error_at(pos, "invalid integer literal"))?;
                (Expr::Constant(Constant::Int(value)), pos + 1)
            }
            TokenKind::String => (
                Expr::Constant(Constant::Str(token.lexeme.clone())),
                pos + 1,
            ),
            TokenKind::Keyword(Keyword::TrueLiteral) => {
                (Expr::Constant(Constant::Bool(true)), pos + 1)
            }
            TokenKind::Keyword(Keyword::FalseLiteral) => {
                (Expr::Constant(Constant::Bool(false)), pos + 1)
            }
            TokenKind::Keyword(Keyword::NoneLiteral) => (Expr::Constant(Constant::None), pos + 1),
            TokenKind::LParen => {
                let (expr, next) = self.parse_paren_expr(pos + 1)?;
                (expr, next)
            }
            TokenKind::LBracket => {
                let (elements, next) = self.parse_list_elements(pos + 1)?;
                (Expr::List(elements), next)
            }
            TokenKind::LBrace => {
                let (entries, next) = self.parse_dict_entries(pos + 1)?;
                (Expr::Dict(entries), next)
            }
            _ => return Err(self.error_at(pos, "expected expression")),
        };

        loop {
            match self.token_at(pos).kind {
                TokenKind::LParen => {
                    let (args, next) = self.parse_call_args(pos + 1)?;
                    expr = Expr::Call {
                        func: Box::new(expr),
                        args,
                    };
                    pos = next;
                }
                TokenKind::LBracket => {
                    let (index, next) = self.parse_subscript(pos + 1)?;
                    expr = Expr::Subscript {
                        value: Box::new(expr),
                        index: Box::new(index),
                    };
                    pos = next;
                }
                _ => break,
            }
        }

        Ok((expr, pos))
    }

    fn parse_if_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        self.parse_if_after_keyword(pos + 1)
    }

    fn parse_if_after_keyword(&mut self, pos: usize) -> ParseResult<Stmt> {
        let mut pos = pos;
        let (test, next) = self.parse_expr_at(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;

        let mut orelse = Vec::new();
        let else_pos = self.skip_newlines(pos);
        if self.match_keyword(else_pos, Keyword::Elif) {
            let (elif_stmt, next) = self.parse_if_after_keyword(else_pos + 1)?;
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
            Stmt::If {
                test,
                body,
                orelse,
            },
            pos,
        ))
    }

    fn parse_while_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let mut pos = pos + 1;
        let (test, next) = self.parse_expr_at(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;
        Ok((Stmt::While { test, body }, pos))
    }

    fn parse_for_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let mut pos = pos + 1;
        let target_token = self.token_at(pos);
        if target_token.kind != TokenKind::Name {
            return Err(self.error_at(pos, "expected loop target"));
        }
        let target = target_token.lexeme.clone();
        pos += 1;
        if !self.match_keyword(pos, Keyword::In) {
            return Err(self.error_at(pos, "expected 'in'"));
        }
        pos += 1;
        let (iter, next) = self.parse_expr_at(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;
        Ok((Stmt::For { target, iter, body }, pos))
    }

    fn parse_assignment_target(&mut self, pos: usize) -> Option<(Expr, usize)> {
        let token = self.token_at(pos);
        if token.kind != TokenKind::Name {
            return None;
        }

        let mut expr = Expr::Name(token.lexeme.clone());
        let mut pos = pos + 1;

        loop {
            if !matches!(self.token_at(pos).kind, TokenKind::LBracket) {
                break;
            }
            let (index, next) = self.parse_subscript(pos + 1).ok()?;
            expr = Expr::Subscript {
                value: Box::new(expr),
                index: Box::new(index),
            };
            pos = next;
        }

        Some((expr, pos))
    }

    fn parse_function_def(&mut self, pos: usize) -> ParseResult<Stmt> {
        let mut pos = pos + 1;
        let name_token = self.token_at(pos);
        if name_token.kind != TokenKind::Name {
            return Err(self.error_at(pos, "expected function name"));
        }
        let name = name_token.lexeme.clone();
        pos += 1;
        pos = self.expect_kind(pos, TokenKind::LParen)?;
        let (params, next) = self.parse_parameters(pos)?;
        pos = next;
        pos = self.expect_kind(pos, TokenKind::Colon)?;
        let (body, next) = self.parse_suite(pos)?;
        pos = next;
        Ok((
            Stmt::FunctionDef {
                name,
                params,
                body,
            },
            pos,
        ))
    }

    fn parse_return_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
        let mut pos = pos + 1;
        if matches!(
            self.token_at(pos).kind,
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Dedent | TokenKind::EndMarker
        ) {
            return Ok((Stmt::Return { value: None }, pos));
        }
        let (expr, next) = self.parse_expr_at(pos)?;
        pos = next;
        Ok((Stmt::Return { value: Some(expr) }, pos))
    }

    fn parse_suite(&mut self, pos: usize) -> Result<(Vec<Stmt>, usize), ParseError> {
        match self.token_at(pos).kind {
            TokenKind::Newline => self.parse_block_suite(pos),
            _ => self.parse_inline_suite(pos),
        }
    }

    fn parse_call_args(&mut self, pos: usize) -> Result<(Vec<Expr>, usize), ParseError> {
        let mut pos = pos;
        let mut args = Vec::new();

        if matches!(self.token_at(pos).kind, TokenKind::RParen) {
            return Ok((args, pos + 1));
        }

        loop {
            let (expr, next) = self.parse_expr_at(pos)?;
            args.push(expr);
            pos = next;

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
        if matches!(self.token_at(pos).kind, TokenKind::RParen) {
            return Ok((Expr::Tuple(Vec::new()), pos + 1));
        }

        let (first, mut pos) = self.parse_expr_at(pos)?;
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
        Ok((Expr::Tuple(elements), pos))
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
        let (expr, next) = self.parse_expr_at(pos)?;
        let next = self.expect_kind(next, TokenKind::RBracket)?;
        Ok((expr, next))
    }

    fn parse_parameters(&mut self, pos: usize) -> Result<(Vec<String>, usize), ParseError> {
        let mut pos = pos;
        let mut params = Vec::new();

        if matches!(self.token_at(pos).kind, TokenKind::RParen) {
            return Ok((params, pos + 1));
        }

        loop {
            let token = self.token_at(pos);
            if token.kind != TokenKind::Name {
                return Err(self.error_at(pos, "expected parameter name"));
            }
            params.push(token.lexeme.clone());
            pos += 1;

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
        Ok((params, pos))
    }

    fn parse_block_suite(&mut self, pos: usize) -> Result<(Vec<Stmt>, usize), ParseError> {
        let mut pos = pos;
        pos = self.expect_kind(pos, TokenKind::Newline)?;
        pos = self.expect_kind(pos, TokenKind::Indent)?;

        let mut body = Vec::new();
        pos = self.consume_separators(pos);

        while !matches!(self.token_at(pos).kind, TokenKind::Dedent | TokenKind::EndMarker) {
            let (stmt, next) = self.parse_stmt_at(pos)?;
            body.push(stmt);
            pos = self.consume_terminators(next)?;
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
        stmt,
        Stmt::If { .. }
            | Stmt::While { .. }
            | Stmt::For { .. }
            | Stmt::FunctionDef { .. }
    )
}
