use std::env;
use std::io::IsTerminal;

const ANSI_RESET: &str = "\x1b[0m";

#[derive(Clone, Copy)]
struct TracebackPalette {
    type_color: &'static str,
    message_color: &'static str,
    filename_color: &'static str,
    line_no_color: &'static str,
    frame_color: &'static str,
    error_highlight: &'static str,
    error_range: &'static str,
}

const DARK_PALETTE: TracebackPalette = TracebackPalette {
    type_color: "\x1b[1;35m",
    message_color: "\x1b[35m",
    filename_color: "\x1b[35m",
    line_no_color: "\x1b[35m",
    frame_color: "\x1b[35m",
    error_highlight: "\x1b[1;31m",
    error_range: "\x1b[31m",
};

const LIGHT_PALETTE: TracebackPalette = TracebackPalette {
    type_color: "\x1b[1;34m",
    message_color: "\x1b[34m",
    filename_color: "\x1b[34m",
    line_no_color: "\x1b[34m",
    frame_color: "\x1b[34m",
    error_highlight: "\x1b[1;31m",
    error_range: "\x1b[31m",
};

// Safe default when terminal background is unknown: keep high-contrast blues/reds.
const FALLBACK_PALETTE: TracebackPalette = TracebackPalette {
    type_color: "\x1b[1;34m",
    message_color: "\x1b[34m",
    filename_color: "\x1b[36m",
    line_no_color: "\x1b[36m",
    frame_color: "\x1b[36m",
    error_highlight: "\x1b[1;31m",
    error_range: "\x1b[31m",
};

pub(super) fn format_error_for_stderr(message: &str) -> String {
    if !should_colorize_stderr() {
        return message.to_string();
    }
    colorize_error_message(message, select_traceback_palette())
}

fn should_colorize_stderr() -> bool {
    let python_colors = env::var("PYTHON_COLORS").ok();
    if matches!(python_colors.as_deref(), Some("0")) {
        return false;
    }
    if matches!(python_colors.as_deref(), Some("1")) {
        return true;
    }
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if env::var_os("FORCE_COLOR").is_some() {
        return true;
    }
    if matches!(env::var("TERM").ok().as_deref(), Some("dumb")) {
        return false;
    }
    std::io::stderr().is_terminal()
}

fn parse_colorfgbg_background_code(value: &str) -> Option<u8> {
    value.rsplit(';').next()?.trim().parse::<u8>().ok()
}

fn is_light_background_code(code: u8) -> bool {
    matches!(code, 7 | 9 | 10 | 11 | 12 | 13 | 14 | 15)
}

fn select_traceback_palette() -> TracebackPalette {
    if let Some(code) = env::var("COLORFGBG")
        .ok()
        .as_deref()
        .and_then(parse_colorfgbg_background_code)
    {
        if is_light_background_code(code) {
            return LIGHT_PALETTE;
        }
        return DARK_PALETTE;
    }
    FALLBACK_PALETTE
}

fn colorize_error_message(message: &str, palette: TracebackPalette) -> String {
    let mut out = String::with_capacity(message.len().saturating_add(32));
    for chunk in message.split_inclusive('\n') {
        let (line, newline) = if let Some(stripped) = chunk.strip_suffix('\n') {
            (stripped, "\n")
        } else {
            (chunk, "")
        };
        out.push_str(&colorize_line(line, palette));
        out.push_str(newline);
    }
    out
}

fn colorize_line(line: &str, palette: TracebackPalette) -> String {
    if let Some(styled) = colorize_file_location_line(line, palette) {
        return styled;
    }
    if line.starts_with("    ")
        && line
            .trim_start()
            .chars()
            .next()
            .is_some_and(|ch| ch == '^' || ch == '~')
    {
        return colorize_caret_line(line, palette);
    }
    if let Some(styled) = colorize_exception_line(line, palette) {
        return styled;
    }
    line.to_string()
}

fn colorize_file_location_line(line: &str, palette: TracebackPalette) -> Option<String> {
    if !line.starts_with("  File \"") {
        return None;
    }
    let open_quote = line.find('"')?;
    let after_open = open_quote + 1;
    let close_quote = line[after_open..].find('"')? + after_open;
    let filename = &line[after_open..close_quote];
    let mut out = String::with_capacity(line.len().saturating_add(24));
    out.push_str("  File \"");
    out.push_str(palette.filename_color);
    out.push_str(filename);
    out.push_str(ANSI_RESET);
    out.push('"');

    let rest = &line[close_quote + 1..];
    if let Some(after_prefix) = rest.strip_prefix(", line ") {
        let digits_end = after_prefix
            .char_indices()
            .take_while(|(_, ch)| ch.is_ascii_digit())
            .last()
            .map(|(idx, ch)| idx + ch.len_utf8())
            .unwrap_or(0);
        out.push_str(", line ");
        if digits_end > 0 {
            out.push_str(palette.line_no_color);
            out.push_str(&after_prefix[..digits_end]);
            out.push_str(ANSI_RESET);
            out.push_str(&after_prefix[digits_end..]);
        } else {
            out.push_str(after_prefix);
        }
    } else {
        out.push_str(rest);
    }

    if let Some(in_index) = out.find(", in ") {
        let frame_start = in_index + ", in ".len();
        let frame_name = &out[frame_start..];
        let mut rewritten = String::with_capacity(out.len().saturating_add(16));
        rewritten.push_str(&out[..frame_start]);
        rewritten.push_str(palette.frame_color);
        rewritten.push_str(frame_name);
        rewritten.push_str(ANSI_RESET);
        return Some(rewritten);
    }
    Some(out)
}

fn colorize_caret_line(line: &str, palette: TracebackPalette) -> String {
    let mut out = String::with_capacity(line.len().saturating_add(16));
    for ch in line.chars() {
        match ch {
            '^' => {
                out.push_str(palette.error_highlight);
                out.push('^');
                out.push_str(ANSI_RESET);
            }
            '~' => {
                out.push_str(palette.error_range);
                out.push('~');
                out.push_str(ANSI_RESET);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn colorize_exception_line(line: &str, palette: TracebackPalette) -> Option<String> {
    if line.starts_with(' ') || line.is_empty() {
        return None;
    }
    if line == "Traceback (most recent call last):" {
        return None;
    }
    if let Some(colon) = line.find(':') {
        let ty = &line[..colon];
        if !looks_like_exception_type(ty) {
            return None;
        }
        let mut out = String::with_capacity(line.len().saturating_add(24));
        out.push_str(palette.type_color);
        out.push_str(ty);
        out.push_str(ANSI_RESET);
        out.push(':');
        let remainder = &line[colon + 1..];
        if !remainder.is_empty() {
            out.push_str(palette.message_color);
            out.push_str(remainder);
            out.push_str(ANSI_RESET);
        }
        return Some(out);
    }
    if looks_like_exception_type(line) {
        return Some(format!("{}{}{}", palette.type_color, line, ANSI_RESET));
    }
    None
}

fn looks_like_exception_type(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

#[cfg(test)]
mod tests {
    use super::{
        DARK_PALETTE, FALLBACK_PALETTE, LIGHT_PALETTE, colorize_error_message,
        parse_colorfgbg_background_code, select_traceback_palette,
    };

    #[test]
    fn parses_colorfgbg_background_component() {
        assert_eq!(parse_colorfgbg_background_code("15;7"), Some(7));
        assert_eq!(parse_colorfgbg_background_code("15;0"), Some(0));
        assert_eq!(parse_colorfgbg_background_code("bad"), None);
    }

    #[test]
    fn colors_traceback_file_and_exception_lines() {
        let text = "Traceback (most recent call last):\n  File \"<stdin>\", line 1, in <module>\n    boom()\n    ^^^^^\nNameError: name 'boom' is not defined";
        let styled = colorize_error_message(text, DARK_PALETTE);
        assert!(styled.contains("\x1b[35m<stdin>\x1b[0m"));
        assert!(styled.contains("\x1b[35m1\x1b[0m"));
        assert!(styled.contains("\x1b[1;31m^\x1b[0m"));
        assert!(styled.contains("\x1b[1;35mNameError\x1b[0m"));
    }

    #[test]
    fn colors_syntax_error_line() {
        let text = "  File \"<string>\", line 1\n    x =\n       ^\nSyntaxError: invalid syntax";
        let styled = colorize_error_message(text, DARK_PALETTE);
        assert!(styled.contains("\x1b[35m<string>\x1b[0m"));
        assert!(styled.contains("\x1b[1;35mSyntaxError\x1b[0m"));
    }

    #[test]
    fn selects_light_palette_from_colorfgbg() {
        let previous = std::env::var_os("COLORFGBG");
        // SAFETY: tests run in-process; we restore the variable at the end.
        unsafe {
            std::env::set_var("COLORFGBG", "15;7");
        }
        assert_eq!(
            select_traceback_palette().type_color,
            LIGHT_PALETTE.type_color
        );
        // SAFETY: restoring process environment for test isolation.
        unsafe {
            if let Some(value) = previous {
                std::env::set_var("COLORFGBG", value);
            } else {
                std::env::remove_var("COLORFGBG");
            }
        }
    }

    #[test]
    fn falls_back_to_safe_palette_without_colorfgbg() {
        let previous = std::env::var_os("COLORFGBG");
        unsafe {
            std::env::remove_var("COLORFGBG");
        }
        assert_eq!(
            select_traceback_palette().type_color,
            FALLBACK_PALETTE.type_color
        );
        unsafe {
            if let Some(value) = previous {
                std::env::set_var("COLORFGBG", value);
            }
        }
    }
}
