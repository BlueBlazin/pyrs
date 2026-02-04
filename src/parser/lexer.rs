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

pub struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        while let Some(ch) = self.peek_char() {
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
                ':' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Colon, ":", offset, line, column));
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
}
