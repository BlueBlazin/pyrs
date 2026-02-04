#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Keyword {
    Pass,
    If,
    Else,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Name,
    Number,
    String,
    Newline,
    Indent,
    Dedent,
    LParen,
    RParen,
    Colon,
    Equal,
    Plus,
    Minus,
    Star,
    Semicolon,
    Keyword(Keyword),
    EndMarker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

impl Token {
    pub fn new(
        kind: TokenKind,
        lexeme: impl Into<String>,
        offset: usize,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            kind,
            lexeme: lexeme.into(),
            offset,
            line,
            column,
        }
    }
}
