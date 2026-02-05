use crate::parser::token::{Keyword, Token, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

impl LexError {
    pub fn new(message: impl Into<String>, offset: usize, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            offset,
            line,
            column,
        }
    }
}

enum IndentResult {
    Blank,
    Indent {
        level: usize,
        offset: usize,
        line: usize,
        column: usize,
    },
}

pub struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
    column: usize,
    indent_stack: Vec<usize>,
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
            column: 1,
            indent_stack: vec![0],
            at_line_start: true,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        while self.peek_char().is_some() {
            if self.at_line_start {
                match self.consume_indentation() {
                    IndentResult::Blank => {
                        self.at_line_start = false;
                    }
                    IndentResult::Indent {
                        level,
                        offset,
                        line,
                        column,
                    } => {
                        self.emit_indent_tokens(level, offset, line, column, &mut tokens)?;
                        self.at_line_start = false;
                    }
                }
            }

            let ch = match self.peek_char() {
                Some(value) => value,
                None => break,
            };

            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
                continue;
            }

            if ch == '#' {
                self.consume_comment();
                continue;
            }

            let offset = self.offset;
            let line = self.line;
            let column = self.column;

            match ch {
                '\n' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Newline, "\n", offset, line, column));
                    self.at_line_start = true;
                }
                ';' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Semicolon, ";", offset, line, column));
                }
                '(' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::LParen, "(", offset, line, column));
                }
                ')' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::RParen, ")", offset, line, column));
                }
                '[' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::LBracket, "[", offset, line, column));
                }
                ']' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::RBracket, "]", offset, line, column));
                }
                '{' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::LBrace, "{", offset, line, column));
                }
                '}' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::RBrace, "}", offset, line, column));
                }
                '.' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Dot, ".", offset, line, column));
                }
                ':' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Colon, ":", offset, line, column));
                }
                '=' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::DoubleEqual,
                            "==",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Equal, "=", offset, line, column));
                    }
                }
                ',' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Comma, ",", offset, line, column));
                }
                '<' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::LessEqual,
                            "<=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Less, "<", offset, line, column));
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::GreaterEqual,
                            ">=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Greater, ">", offset, line, column));
                    }
                }
                '!' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::NotEqual,
                            "!=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        return Err(LexError::new(
                            "unexpected character: !",
                            offset,
                            line,
                            column,
                        ));
                    }
                }
                '+' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::PlusEqual, "+=", offset, line, column));
                    } else {
                        tokens.push(Token::new(TokenKind::Plus, "+", offset, line, column));
                    }
                }
                '-' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::MinusEqual, "-=", offset, line, column));
                    } else {
                        tokens.push(Token::new(TokenKind::Minus, "-", offset, line, column));
                    }
                }
                '*' => {
                    self.advance();
                    if self.peek_char() == Some('*') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::DoubleStar, "**", offset, line, column));
                    } else if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::StarEqual, "*=", offset, line, column));
                    } else {
                        tokens.push(Token::new(TokenKind::Star, "*", offset, line, column));
                    }
                }
                '/' => {
                    self.advance();
                    if self.peek_char() == Some('/') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::DoubleSlash, "//", offset, line, column));
                    } else {
                        return Err(LexError::new(
                            "unexpected character: /",
                            offset,
                            line,
                            column,
                        ));
                    }
                }
                '%' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Percent, "%", offset, line, column));
                }
                '\'' | '"' => {
                    let lexeme = self.consume_string(ch)?;
                    tokens.push(Token::new(TokenKind::String, lexeme, offset, line, column));
                }
                '0'..='9' => {
                    let lexeme = self.consume_number();
                    tokens.push(Token::new(TokenKind::Number, lexeme, offset, line, column));
                }
                '_' | 'a'..='z' | 'A'..='Z' => {
                    let lexeme = self.consume_identifier();
                    let kind = match lexeme.as_str() {
                        "pass" => TokenKind::Keyword(Keyword::Pass),
                        "if" => TokenKind::Keyword(Keyword::If),
                        "else" => TokenKind::Keyword(Keyword::Else),
                        "while" => TokenKind::Keyword(Keyword::While),
                        "try" => TokenKind::Keyword(Keyword::Try),
                        "except" => TokenKind::Keyword(Keyword::Except),
                        "finally" => TokenKind::Keyword(Keyword::Finally),
                        "raise" => TokenKind::Keyword(Keyword::Raise),
                        "assert" => TokenKind::Keyword(Keyword::Assert),
                        "True" => TokenKind::Keyword(Keyword::TrueLiteral),
                        "False" => TokenKind::Keyword(Keyword::FalseLiteral),
                        "None" => TokenKind::Keyword(Keyword::NoneLiteral),
                        "def" => TokenKind::Keyword(Keyword::Def),
                        "class" => TokenKind::Keyword(Keyword::Class),
                        "return" => TokenKind::Keyword(Keyword::Return),
                        "for" => TokenKind::Keyword(Keyword::For),
                        "in" => TokenKind::Keyword(Keyword::In),
                        "is" => TokenKind::Keyword(Keyword::Is),
                        "break" => TokenKind::Keyword(Keyword::Break),
                        "continue" => TokenKind::Keyword(Keyword::Continue),
                        "and" => TokenKind::Keyword(Keyword::And),
                        "or" => TokenKind::Keyword(Keyword::Or),
                        "not" => TokenKind::Keyword(Keyword::Not),
                        "elif" => TokenKind::Keyword(Keyword::Elif),
                        "import" => TokenKind::Keyword(Keyword::Import),
                        "from" => TokenKind::Keyword(Keyword::From),
                        "global" => TokenKind::Keyword(Keyword::Global),
                        "as" => TokenKind::Keyword(Keyword::As),
                        "lambda" => TokenKind::Keyword(Keyword::Lambda),
                        _ => TokenKind::Name,
                    };
                    tokens.push(Token::new(kind, lexeme, offset, line, column));
                }
                _ => {
                    return Err(LexError::new(
                        format!("unexpected character: {ch}"),
                        offset,
                        line,
                        column,
                    ));
                }
            }
        }

        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            tokens.push(Token::new(
                TokenKind::Dedent,
                "",
                self.offset,
                self.line,
                self.column,
            ));
        }

        tokens.push(Token::new(
            TokenKind::EndMarker,
            "",
            self.offset,
            self.line,
            self.column,
        ));

        Ok(tokens)
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.offset += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn consume_comment(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.advance();
        }
    }

    fn consume_identifier(&mut self) -> String {
        let start = self.offset;
        while let Some(ch) = self.peek_char() {
            if ch == '_' || ch.is_ascii_alphanumeric() {
                self.advance();
            } else {
                break;
            }
        }
        self.source[start..self.offset].to_string()
    }

    fn consume_number(&mut self) -> String {
        let start = self.offset;
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        self.source[start..self.offset].to_string()
    }

    fn consume_string(&mut self, quote: char) -> Result<String, LexError> {
        let start_offset = self.offset;
        let start_line = self.line;
        let start_column = self.column;
        self.advance();
        let mut content = String::new();

        while let Some(ch) = self.peek_char() {
            if ch == quote {
                self.advance();
                return Ok(content);
            }

            if ch == '\n' {
                return Err(LexError::new(
                    "unterminated string literal",
                    start_offset,
                    start_line,
                    start_column,
                ));
            }

            if ch == '\\' {
                self.advance();
                let escaped = match self.peek_char() {
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some('r') => '\r',
                    Some('\\') => '\\',
                    Some('\'') => '\'',
                    Some('"') => '"',
                    Some(other) => other,
                    None => {
                        return Err(LexError::new(
                            "unterminated escape sequence",
                            start_offset,
                            start_line,
                            start_column,
                        ));
                    }
                };
                self.advance();
                content.push(escaped);
                continue;
            }

            self.advance();
            content.push(ch);
        }

        Err(LexError::new(
            "unterminated string literal",
            start_offset,
            start_line,
            start_column,
        ))
    }

    fn consume_indentation(&mut self) -> IndentResult {
        let mut level = 0usize;

        while let Some(ch) = self.peek_char() {
            match ch {
                ' ' => {
                    level += 1;
                    self.advance();
                }
                '\t' => {
                    let next = 8 - (level % 8);
                    level += next;
                    self.advance();
                }
                _ => break,
            }
        }

        let offset = self.offset;
        let line = self.line;
        let column = self.column;

        match self.peek_char() {
            Some('\n') | Some('#') | None => IndentResult::Blank,
            _ => IndentResult::Indent {
                level,
                offset,
                line,
                column,
            },
        }
    }

    fn emit_indent_tokens(
        &mut self,
        level: usize,
        offset: usize,
        line: usize,
        column: usize,
        tokens: &mut Vec<Token>,
    ) -> Result<(), LexError> {
        let current = *self.indent_stack.last().unwrap_or(&0);
        if level == current {
            return Ok(());
        }

        if level > current {
            self.indent_stack.push(level);
            tokens.push(Token::new(
                TokenKind::Indent,
                "",
                offset,
                line,
                column,
            ));
            return Ok(());
        }

        while let Some(&top) = self.indent_stack.last() {
            if level == top {
                return Ok(());
            }
            self.indent_stack.pop();
            tokens.push(Token::new(
                TokenKind::Dedent,
                "",
                offset,
                line,
                column,
            ));
        }

        Err(LexError::new(
            "indentation does not match any outer level",
            offset,
            line,
            column,
        ))
    }
}
