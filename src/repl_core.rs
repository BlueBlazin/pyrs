//! Shared REPL parse/incomplete-input semantics used by native and wasm adapters.

use crate::parser::ParseError;

/// Strips the synthetic trailing newline added by line-based REPL submit loops.
pub(crate) fn parse_candidate_source(pending: &str) -> &str {
    pending.strip_suffix('\n').unwrap_or(pending)
}

/// When parse succeeds but an EOF-implied dedent is present, keep collecting lines.
pub(crate) fn parse_success_requires_more_input(source: &str, latest_line: &str) -> bool {
    !latest_line.trim().is_empty() && has_eof_implied_dedent(source)
}

fn has_eof_implied_dedent(source: &str) -> bool {
    let mut lexer = crate::parser::lexer::Lexer::new(source);
    let Ok(tokens) = lexer.tokenize() else {
        return false;
    };
    let eof_offset = source.len();
    let mut index = tokens.len();
    while index > 0
        && matches!(
            tokens[index - 1].kind,
            crate::parser::token::TokenKind::EndMarker
        )
    {
        index -= 1;
    }
    let mut saw_eof_dedent = false;
    while index > 0 {
        let token = &tokens[index - 1];
        if !matches!(token.kind, crate::parser::token::TokenKind::Dedent) {
            break;
        }
        if token.offset == eof_offset {
            saw_eof_dedent = true;
        }
        index -= 1;
    }
    saw_eof_dedent
}

/// Determines whether parser failure should keep REPL in continuation mode.
pub(crate) fn input_is_incomplete(source: &str, err: &ParseError) -> bool {
    let source_trimmed = source.trim_end();
    if source_trimmed.is_empty() {
        return false;
    }

    let lower_msg = err.message.to_ascii_lowercase();
    if lower_msg.contains("unterminated string literal")
        || lower_msg.contains("unterminated escape sequence")
    {
        return true;
    }

    if source_trimmed.ends_with('\\') || source_trimmed.ends_with(':') {
        return true;
    }

    if has_unclosed_delimiters(source) {
        return true;
    }

    if err.offset >= source.len() {
        return true;
    }

    lower_msg.contains("expected indent")
        || lower_msg.contains("expected dedent")
        || lower_msg.contains("expected rparen")
        || lower_msg.contains("expected rbracket")
        || lower_msg.contains("expected rbrace")
}

fn has_unclosed_delimiters(source: &str) -> bool {
    let mut lexer = crate::parser::lexer::Lexer::new(source);
    let Ok(tokens) = lexer.tokenize() else {
        return false;
    };

    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    for token in tokens {
        match token.kind {
            crate::parser::token::TokenKind::LParen => paren_depth += 1,
            crate::parser::token::TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
            crate::parser::token::TokenKind::LBracket => bracket_depth += 1,
            crate::parser::token::TokenKind::RBracket => {
                bracket_depth = bracket_depth.saturating_sub(1)
            }
            crate::parser::token::TokenKind::LBrace => brace_depth += 1,
            crate::parser::token::TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
    }
    paren_depth != 0 || bracket_depth != 0 || brace_depth != 0
}

#[cfg(test)]
mod tests {
    use super::{input_is_incomplete, parse_candidate_source, parse_success_requires_more_input};

    #[test]
    fn marks_colon_blocks_as_incomplete() {
        let source = "if True:\n";
        let err =
            crate::parser::parse_module(source).expect_err("parse should fail while incomplete");
        assert!(input_is_incomplete(source, &err));
    }

    #[test]
    fn marks_unclosed_delimiter_as_incomplete() {
        let source = "print((1 + 2\n";
        let err =
            crate::parser::parse_module(source).expect_err("parse should fail while incomplete");
        assert!(input_is_incomplete(source, &err));
    }

    #[test]
    fn treats_real_syntax_error_as_complete() {
        let source = "if True print(1)\n";
        let err = crate::parser::parse_module(source).expect_err("parse should fail");
        assert!(!input_is_incomplete(source, &err));
    }

    #[test]
    fn candidate_source_omits_latest_synthetic_newline() {
        assert_eq!(
            parse_candidate_source("class A:\n    x = 1\n"),
            "class A:\n    x = 1"
        );
        assert_eq!(
            parse_candidate_source("class A:\n    x = 1\n\n"),
            "class A:\n    x = 1\n"
        );
    }

    #[test]
    fn class_block_stays_incomplete_until_blank_line() {
        let without_blank = parse_candidate_source("class A:\n    x = 1\n");
        assert!(crate::parser::parse_module(without_blank).is_ok());
        assert!(parse_success_requires_more_input(
            without_blank,
            "    x = 1"
        ));

        let with_blank = parse_candidate_source("class A:\n    x = 1\n\n");
        assert!(
            crate::parser::parse_module(with_blank).is_ok(),
            "class block should complete after blank line"
        );
        assert!(!parse_success_requires_more_input(with_blank, ""));
    }
}
