use std::collections::HashMap;

use crate::ast::{BinaryOp, Constant, Expr, Module, Stmt};
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
        let token = self.token_at(pos);
        if token.kind == TokenKind::Name
            && matches!(self.token_at(pos + 1).kind, TokenKind::Equal)
        {
            let target = token.lexeme.clone();
            let (value, next) = self.parse_expr_at(pos + 2)?;
            return Ok((Stmt::Assign { target, value }, next));
        }
        match token.kind {
            TokenKind::Keyword(Keyword::If) => self.parse_if_stmt(pos),
            TokenKind::Keyword(Keyword::While) => self.parse_while_stmt(pos),
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
        self.parse_comparison(pos)
    }

    fn parse_comparison(&mut self, pos: usize) -> ParseResult<Expr> {
        let (left, mut pos) = self.parse_add_sub(pos)?;

        let op = match self.token_at(pos).kind {
            TokenKind::DoubleEqual => BinaryOp::Eq,
            TokenKind::Less => BinaryOp::Lt,
            _ => return Ok((left, pos)),
        };

        pos += 1;
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
        let (mut left, mut pos) = self.parse_atom(pos)?;

        loop {
            if !matches!(self.token_at(pos).kind, TokenKind::Star) {
                break;
            }
            pos += 1;
            let (right, next) = self.parse_atom(pos)?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::Mul,
                right: Box::new(right),
            };
            pos = next;
        }

        Ok((left, pos))
    }

    fn parse_atom(&mut self, pos: usize) -> ParseResult<Expr> {
        let token = self.token_at(pos);
        match &token.kind {
            TokenKind::Name => Ok((Expr::Name(token.lexeme.clone()), pos + 1)),
            TokenKind::Number => {
                let value = token
                    .lexeme
                    .parse::<i64>()
                    .map_err(|_| self.error_at(pos, "invalid integer literal"))?;
                Ok((Expr::Constant(Constant::Int(value)), pos + 1))
            }
            TokenKind::String => Ok((Expr::Constant(Constant::Str(token.lexeme.clone())), pos + 1)),
            TokenKind::LParen => {
                let (expr, next) = self.parse_expr_at(pos + 1)?;
                let next = self.expect_kind(next, TokenKind::RParen)?;
                Ok((expr, next))
            }
            _ => Err(self.error_at(pos, "expected expression")),
        }
    }

    fn parse_if_stmt(&mut self, pos: usize) -> ParseResult<Stmt> {
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

    fn parse_suite(&mut self, pos: usize) -> Result<(Vec<Stmt>, usize), ParseError> {
        match self.token_at(pos).kind {
            TokenKind::Newline => self.parse_block_suite(pos),
            _ => self.parse_inline_suite(pos),
        }
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
    matches!(stmt, Stmt::If { .. } | Stmt::While { .. })
}
