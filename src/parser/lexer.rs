use crate::parser::token::{Keyword, Token, TokenKind};
use crate::parser::unicode_names::{UnicodeNameLookup, lookup_unicode_name};

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
    paren_level: usize,
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
            paren_level: 0,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        while self.peek_char().is_some() {
            if self.at_line_start && self.paren_level == 0 {
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
                    if self.paren_level == 0 {
                        tokens.push(Token::new(TokenKind::Newline, "\n", offset, line, column));
                        self.at_line_start = true;
                    } else {
                        self.at_line_start = false;
                    }
                }
                '\\' => {
                    self.advance();
                    if self.peek_char() == Some('\n') {
                        // Explicit line joining: consume the newline and suppress a token.
                        self.advance();
                        self.at_line_start = false;
                    } else {
                        return Err(LexError::new(
                            "unexpected character: \\",
                            offset,
                            line,
                            column,
                        ));
                    }
                }
                ';' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Semicolon, ";", offset, line, column));
                }
                '(' => {
                    self.advance();
                    self.paren_level += 1;
                    tokens.push(Token::new(TokenKind::LParen, "(", offset, line, column));
                }
                ')' => {
                    self.advance();
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    tokens.push(Token::new(TokenKind::RParen, ")", offset, line, column));
                }
                '[' => {
                    self.advance();
                    self.paren_level += 1;
                    tokens.push(Token::new(TokenKind::LBracket, "[", offset, line, column));
                }
                ']' => {
                    self.advance();
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    tokens.push(Token::new(TokenKind::RBracket, "]", offset, line, column));
                }
                '{' => {
                    self.advance();
                    self.paren_level += 1;
                    tokens.push(Token::new(TokenKind::LBrace, "{", offset, line, column));
                }
                '}' => {
                    self.advance();
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    tokens.push(Token::new(TokenKind::RBrace, "}", offset, line, column));
                }
                '.' => {
                    if self.peek_char_at(1) == Some('.') && self.peek_char_at(2) == Some('.') {
                        self.advance();
                        self.advance();
                        self.advance();
                        tokens.push(Token::new(TokenKind::Ellipsis, "...", offset, line, column));
                    } else if matches!(self.peek_char_at(1), Some(ch) if ch.is_ascii_digit()) {
                        let lexeme = self.consume_number_from_dot();
                        tokens.push(Token::new(TokenKind::Number, lexeme, offset, line, column));
                    } else {
                        self.advance();
                        tokens.push(Token::new(TokenKind::Dot, ".", offset, line, column));
                    }
                }
                ':' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::ColonEqual,
                            ":=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Colon, ":", offset, line, column));
                    }
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
                    if self.peek_char() == Some('<') {
                        self.advance();
                        if self.peek_char() == Some('=') {
                            self.advance();
                            tokens.push(Token::new(
                                TokenKind::LeftShiftEqual,
                                "<<=",
                                offset,
                                line,
                                column,
                            ));
                        } else {
                            tokens.push(Token::new(
                                TokenKind::LeftShift,
                                "<<",
                                offset,
                                line,
                                column,
                            ));
                        }
                    } else if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::LessEqual, "<=", offset, line, column));
                    } else {
                        tokens.push(Token::new(TokenKind::Less, "<", offset, line, column));
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek_char() == Some('>') {
                        self.advance();
                        if self.peek_char() == Some('=') {
                            self.advance();
                            tokens.push(Token::new(
                                TokenKind::RightShiftEqual,
                                ">>=",
                                offset,
                                line,
                                column,
                            ));
                        } else {
                            tokens.push(Token::new(
                                TokenKind::RightShift,
                                ">>",
                                offset,
                                line,
                                column,
                            ));
                        }
                    } else if self.peek_char() == Some('=') {
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
                        tokens.push(Token::new(TokenKind::NotEqual, "!=", offset, line, column));
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
                    if self.peek_char() == Some('>') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::Arrow, "->", offset, line, column));
                    } else if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::MinusEqual,
                            "-=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Minus, "-", offset, line, column));
                    }
                }
                '*' => {
                    self.advance();
                    if self.peek_char() == Some('*') {
                        self.advance();
                        if self.peek_char() == Some('=') {
                            self.advance();
                            tokens.push(Token::new(
                                TokenKind::DoubleStarEqual,
                                "**=",
                                offset,
                                line,
                                column,
                            ));
                        } else {
                            tokens.push(Token::new(
                                TokenKind::DoubleStar,
                                "**",
                                offset,
                                line,
                                column,
                            ));
                        }
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
                        if self.peek_char() == Some('=') {
                            self.advance();
                            tokens.push(Token::new(
                                TokenKind::DoubleSlashEqual,
                                "//=",
                                offset,
                                line,
                                column,
                            ));
                        } else {
                            tokens.push(Token::new(
                                TokenKind::DoubleSlash,
                                "//",
                                offset,
                                line,
                                column,
                            ));
                        }
                    } else if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::SlashEqual,
                            "/=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Slash, "/", offset, line, column));
                    }
                }
                '%' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::PercentEqual,
                            "%=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Percent, "%", offset, line, column));
                    }
                }
                '&' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::AmpersandEqual,
                            "&=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Ampersand, "&", offset, line, column));
                    }
                }
                '|' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::PipeEqual, "|=", offset, line, column));
                    } else {
                        tokens.push(Token::new(TokenKind::Pipe, "|", offset, line, column));
                    }
                }
                '^' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(
                            TokenKind::CaretEqual,
                            "^=",
                            offset,
                            line,
                            column,
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::Caret, "^", offset, line, column));
                    }
                }
                '~' => {
                    self.advance();
                    tokens.push(Token::new(TokenKind::Tilde, "~", offset, line, column));
                }
                '@' => {
                    self.advance();
                    if self.peek_char() == Some('=') {
                        self.advance();
                        tokens.push(Token::new(TokenKind::AtEqual, "@=", offset, line, column));
                    } else {
                        tokens.push(Token::new(TokenKind::At, "@", offset, line, column));
                    }
                }
                '\'' | '"' => {
                    let lexeme = self.consume_string(ch, false)?;
                    tokens.push(Token::new(TokenKind::String, lexeme, offset, line, column));
                }
                '0'..='9' => {
                    let lexeme = self.consume_number();
                    tokens.push(Token::new(TokenKind::Number, lexeme, offset, line, column));
                }
                _ if ch == '_' || ch.is_alphabetic() => {
                    if let Some((kind, string_content)) = self.consume_prefixed_string()? {
                        tokens.push(Token::new(kind, string_content, offset, line, column));
                        continue;
                    }
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
                        "nonlocal" => TokenKind::Keyword(Keyword::Nonlocal),
                        "as" => TokenKind::Keyword(Keyword::As),
                        "lambda" => TokenKind::Keyword(Keyword::Lambda),
                        "with" => TokenKind::Keyword(Keyword::With),
                        "yield" => TokenKind::Keyword(Keyword::Yield),
                        "async" => TokenKind::Keyword(Keyword::Async),
                        "await" => TokenKind::Keyword(Keyword::Await),
                        "del" => TokenKind::Keyword(Keyword::Del),
                        // Soft keywords are tokenized as names and disambiguated in parser.
                        "match" | "case" | "type" => TokenKind::Name,
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

    fn peek_char_at(&self, idx: usize) -> Option<char> {
        self.source[self.offset..].chars().nth(idx)
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
            if ch == '_' || ch.is_alphanumeric() {
                self.advance();
            } else {
                break;
            }
        }
        self.source[start..self.offset].to_string()
    }

    fn consume_number(&mut self) -> String {
        let start = self.offset;
        if self.peek_char() == Some('0') {
            self.advance();
            if matches!(self.peek_char(), Some('x' | 'X' | 'o' | 'O' | 'b' | 'B')) {
                self.advance();
                while let Some(ch) = self.peek_char() {
                    if ch == '_' || ch.is_ascii_hexdigit() {
                        self.advance();
                    } else {
                        break;
                    }
                }
                if matches!(self.peek_char(), Some('j' | 'J')) {
                    self.advance();
                }
                return self.source[start..self.offset].to_string();
            }
        }

        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_digit() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }

        if self.peek_char() == Some('.') && self.peek_char_at(1) != Some('.') {
            self.advance();
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_digit() || ch == '_' {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if matches!(self.peek_char(), Some('e' | 'E')) {
            self.advance();
            if matches!(self.peek_char(), Some('+' | '-')) {
                self.advance();
            }
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_digit() || ch == '_' {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if matches!(self.peek_char(), Some('j' | 'J')) {
            self.advance();
        }

        self.source[start..self.offset].to_string()
    }

    fn consume_number_from_dot(&mut self) -> String {
        let start = self.offset;
        self.advance();
        while let Some(ch) = self.peek_char() {
            if ch.is_ascii_digit() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }
        if matches!(self.peek_char(), Some('e' | 'E')) {
            self.advance();
            if matches!(self.peek_char(), Some('+' | '-')) {
                self.advance();
            }
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_digit() || ch == '_' {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek_char(), Some('j' | 'J')) {
            self.advance();
        }
        self.source[start..self.offset].to_string()
    }

    fn consume_prefixed_string(&mut self) -> Result<Option<(TokenKind, String)>, LexError> {
        let mut probe = self.offset;
        let mut prefix = String::new();
        while prefix.len() < 2 {
            let Some(ch) = self.source[probe..].chars().next() else {
                break;
            };
            if ch.is_ascii_alphabetic() {
                prefix.push(ch);
                probe += ch.len_utf8();
            } else {
                break;
            }
        }

        if prefix.is_empty() {
            return Ok(None);
        }

        let Some(quote) = self.source[probe..].chars().next() else {
            return Ok(None);
        };
        if quote != '\'' && quote != '"' {
            return Ok(None);
        }

        let mut seen = std::collections::HashSet::new();
        let mut has_f = false;
        let mut has_r = false;
        let mut has_b = false;
        for ch in prefix.chars() {
            let lowered = ch.to_ascii_lowercase();
            if !matches!(lowered, 'r' | 'u' | 'b' | 'f' | 't') {
                return Ok(None);
            }
            if lowered == 'f' {
                has_f = true;
            }
            if lowered == 'r' {
                has_r = true;
            }
            if lowered == 'b' {
                has_b = true;
            }
            if !seen.insert(lowered) {
                return Ok(None);
            }
        }
        if has_f && has_b {
            return Err(LexError::new(
                "bytes f-strings are not supported",
                self.offset,
                self.line,
                self.column,
            ));
        }

        for _ in 0..prefix.len() {
            self.advance();
        }
        let content = if has_f {
            self.consume_fstring(quote, has_r)?
        } else {
            self.consume_string(quote, has_r)?
        };
        let kind = if has_f {
            TokenKind::FString
        } else if has_b {
            TokenKind::Bytes
        } else {
            TokenKind::String
        };
        Ok(Some((kind, content)))
    }

    fn consume_fstring(&mut self, quote: char, raw: bool) -> Result<String, LexError> {
        let start_offset = self.offset;
        let start_line = self.line;
        let start_column = self.column;
        let is_triple = self.peek_char_at(1) == Some(quote) && self.peek_char_at(2) == Some(quote);
        if is_triple {
            self.advance();
            self.advance();
            self.advance();
        } else {
            self.advance();
        }

        let mut content = String::new();
        let mut expr_depth = 0usize;
        while let Some(ch) = self.peek_char() {
            if expr_depth == 0 {
                if is_triple {
                    if ch == quote
                        && self.peek_char_at(1) == Some(quote)
                        && self.peek_char_at(2) == Some(quote)
                    {
                        self.advance();
                        self.advance();
                        self.advance();
                        return Ok(content);
                    }
                } else if ch == quote {
                    if raw && self.raw_quote_is_escaped() {
                        self.advance();
                        content.push(ch);
                        continue;
                    }
                    self.advance();
                    return Ok(content);
                }
            }

            if ch == '\n' && !is_triple {
                return Err(LexError::new(
                    "unterminated string literal",
                    start_offset,
                    start_line,
                    start_column,
                ));
            }

            if ch == '{' {
                if expr_depth == 0 && self.peek_char_at(1) == Some('{') {
                    self.advance();
                    self.advance();
                    content.push('{');
                    content.push('{');
                    continue;
                }
                expr_depth += 1;
                self.advance();
                content.push(ch);
                continue;
            }

            if ch == '}' {
                if expr_depth == 0 && self.peek_char_at(1) == Some('}') {
                    self.advance();
                    self.advance();
                    content.push('}');
                    content.push('}');
                    continue;
                }
                if expr_depth > 0 {
                    expr_depth -= 1;
                }
                self.advance();
                content.push(ch);
                continue;
            }

            if ch == '\\' && raw {
                self.advance();
                content.push('\\');
                continue;
            }

            if ch == '\\' {
                self.advance();
                if expr_depth > 0 {
                    let next = self.peek_char().ok_or_else(|| {
                        LexError::new(
                            "unterminated escape sequence",
                            start_offset,
                            start_line,
                            start_column,
                        )
                    })?;
                    self.advance();
                    content.push('\\');
                    content.push(next);
                    continue;
                }
                let escaped = self.consume_escaped_char(start_offset, start_line, start_column)?;
                if let Some(escaped) = escaped {
                    content.push(escaped);
                }
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

    fn consume_string(&mut self, quote: char, raw: bool) -> Result<String, LexError> {
        let start_offset = self.offset;
        let start_line = self.line;
        let start_column = self.column;
        let is_triple = self.peek_char_at(1) == Some(quote) && self.peek_char_at(2) == Some(quote);
        if is_triple {
            self.advance();
            self.advance();
            self.advance();
        } else {
            self.advance();
        }
        let mut content = String::new();

        while let Some(ch) = self.peek_char() {
            if is_triple {
                if ch == quote
                    && self.peek_char_at(1) == Some(quote)
                    && self.peek_char_at(2) == Some(quote)
                {
                    self.advance();
                    self.advance();
                    self.advance();
                    return Ok(content);
                }
            } else if ch == quote {
                if raw && self.raw_quote_is_escaped() {
                    self.advance();
                    content.push(ch);
                    continue;
                }
                self.advance();
                return Ok(content);
            }

            if ch == '\n' && !is_triple {
                return Err(LexError::new(
                    "unterminated string literal",
                    start_offset,
                    start_line,
                    start_column,
                ));
            }

            if ch == '\\' && raw {
                self.advance();
                content.push('\\');
                continue;
            }

            if ch == '\\' {
                self.advance();
                let escaped = self.consume_escaped_char(start_offset, start_line, start_column)?;
                if let Some(escaped) = escaped {
                    content.push(escaped);
                }
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

    fn consume_escaped_char(
        &mut self,
        start_offset: usize,
        start_line: usize,
        start_column: usize,
    ) -> Result<Option<char>, LexError> {
        match self.peek_char() {
            Some('n') => {
                self.advance();
                Ok(Some('\n'))
            }
            Some('a') => {
                self.advance();
                Ok(Some('\u{0007}'))
            }
            Some('b') => {
                self.advance();
                Ok(Some('\u{0008}'))
            }
            Some('f') => {
                self.advance();
                Ok(Some('\u{000c}'))
            }
            Some('t') => {
                self.advance();
                Ok(Some('\t'))
            }
            Some('r') => {
                self.advance();
                Ok(Some('\r'))
            }
            Some('v') => {
                self.advance();
                Ok(Some('\u{000b}'))
            }
            Some('\\') => {
                self.advance();
                Ok(Some('\\'))
            }
            Some('\'') => {
                self.advance();
                Ok(Some('\''))
            }
            Some('"') => {
                self.advance();
                Ok(Some('"'))
            }
            Some('x') => {
                self.advance();
                let high = self.peek_char().ok_or_else(|| {
                    LexError::new(
                        "unterminated escape sequence",
                        start_offset,
                        start_line,
                        start_column,
                    )
                })?;
                if !high.is_ascii_hexdigit() {
                    return Err(LexError::new(
                        "invalid hex escape",
                        start_offset,
                        start_line,
                        start_column,
                    ));
                }
                self.advance();
                let low = self.peek_char().ok_or_else(|| {
                    LexError::new(
                        "unterminated escape sequence",
                        start_offset,
                        start_line,
                        start_column,
                    )
                })?;
                if !low.is_ascii_hexdigit() {
                    return Err(LexError::new(
                        "invalid hex escape",
                        start_offset,
                        start_line,
                        start_column,
                    ));
                }
                self.advance();
                let value =
                    ((high.to_digit(16).unwrap_or(0) << 4) | low.to_digit(16).unwrap_or(0)) as u8;
                Ok(Some(value as char))
            }
            Some(first @ '0'..='7') => {
                let mut value = first.to_digit(8).unwrap_or(0);
                self.advance();
                for _ in 0..2 {
                    let Some(next @ '0'..='7') = self.peek_char() else {
                        break;
                    };
                    value = (value << 3) | next.to_digit(8).unwrap_or(0);
                    self.advance();
                }
                Ok(Some((value as u8) as char))
            }
            Some('u') | Some('U') => {
                let width = if self.peek_char() == Some('u') { 4 } else { 8 };
                self.advance();
                let mut value: u32 = 0;
                for _ in 0..width {
                    let ch = self.peek_char().ok_or_else(|| {
                        LexError::new(
                            "unterminated escape sequence",
                            start_offset,
                            start_line,
                            start_column,
                        )
                    })?;
                    if !ch.is_ascii_hexdigit() {
                        return Err(LexError::new(
                            "invalid unicode escape",
                            start_offset,
                            start_line,
                            start_column,
                        ));
                    }
                    self.advance();
                    value = (value << 4) | ch.to_digit(16).unwrap_or(0);
                }
                let ch = if (0xD800..=0xDFFF).contains(&value) {
                    '\u{FFFD}'
                } else {
                    char::from_u32(value).ok_or_else(|| {
                        LexError::new(
                            "invalid unicode escape",
                            start_offset,
                            start_line,
                            start_column,
                        )
                    })?
                };
                Ok(Some(ch))
            }
            Some('N') => {
                self.advance();
                if self.peek_char() != Some('{') {
                    return Ok(Some('N'));
                }
                self.advance();
                let mut name = String::new();
                loop {
                    match self.peek_char() {
                        Some('}') => {
                            self.advance();
                            break;
                        }
                        Some(ch) => {
                            self.advance();
                            name.push(ch);
                        }
                        None => {
                            return Err(LexError::new(
                                "invalid unicode escape",
                                start_offset,
                                start_line,
                                start_column,
                            ));
                        }
                    }
                }
                if name.is_empty() {
                    return Err(LexError::new(
                        "invalid unicode escape",
                        start_offset,
                        start_line,
                        start_column,
                    ));
                }
                let ch = match lookup_unicode_name(&name) {
                    UnicodeNameLookup::Char(ch) => ch,
                    UnicodeNameLookup::NamedSequence | UnicodeNameLookup::Unknown => {
                        return Err(LexError::new(
                            "invalid unicode escape",
                            start_offset,
                            start_line,
                            start_column,
                        ));
                    }
                };
                Ok(Some(ch))
            }
            Some('\n') => {
                // In non-raw strings, backslash-newline is a line continuation.
                self.advance();
                Ok(None)
            }
            Some('\r') => {
                // Handle CRLF continuations as a single escaped newline sequence.
                self.advance();
                if self.peek_char() == Some('\n') {
                    self.advance();
                }
                Ok(None)
            }
            Some(other) => {
                self.advance();
                Ok(Some(other))
            }
            None => Err(LexError::new(
                "unterminated escape sequence",
                start_offset,
                start_line,
                start_column,
            )),
        }
    }

    fn raw_quote_is_escaped(&self) -> bool {
        let bytes = self.source.as_bytes();
        if self.offset == 0 || self.offset > bytes.len() {
            return false;
        }
        let mut index = self.offset;
        let mut backslashes = 0usize;
        while index > 0 && bytes[index - 1] == b'\\' {
            backslashes += 1;
            index -= 1;
        }
        backslashes % 2 == 1
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
            tokens.push(Token::new(TokenKind::Indent, "", offset, line, column));
            return Ok(());
        }

        while let Some(&top) = self.indent_stack.last() {
            if level == top {
                return Ok(());
            }
            self.indent_stack.pop();
            tokens.push(Token::new(TokenKind::Dedent, "", offset, line, column));
        }

        Err(LexError::new(
            "indentation does not match any outer level",
            offset,
            line,
            column,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_named_unicode_escape_in_string_literals() {
        let mut lexer = Lexer::new("\"\\N{EMPTY SET}\"");
        let tokens = lexer.tokenize().expect("tokenization should succeed");
        assert!(
            tokens
                .iter()
                .any(|token| { token.kind == TokenKind::String && token.lexeme == "\u{2205}" })
        );
    }

    #[test]
    fn rejects_unknown_named_unicode_escape() {
        let mut lexer = Lexer::new("\"\\N{THIS NAME DOES NOT EXIST}\"");
        let err = lexer
            .tokenize()
            .expect_err("unknown unicode name should fail");
        assert!(err.message.contains("invalid unicode escape"));
    }

    #[test]
    fn decodes_alias_named_unicode_escape_in_string_literals() {
        let mut lexer = Lexer::new("\"\\N{line feed}\"");
        let tokens = lexer.tokenize().expect("tokenization should succeed");
        assert!(
            tokens
                .iter()
                .any(|token| { token.kind == TokenKind::String && token.lexeme == "\n" })
        );
    }

    #[test]
    fn rejects_named_sequence_in_unicode_escape() {
        let mut lexer = Lexer::new("\"\\N{LATIN SMALL LETTER R WITH TILDE}\"");
        let err = lexer
            .tokenize()
            .expect_err("named sequence should fail in unicode escapes");
        assert!(err.message.contains("invalid unicode escape"));
    }
}
