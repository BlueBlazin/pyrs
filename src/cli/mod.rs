//! CLI entry point and argument handling.

mod error_style;
mod repl;

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::VERSION;
use crate::compiler;
use crate::parser;
use crate::parser::ParseError;
use crate::runtime::Value;
use crate::vm::Vm;

const HELP: &str = "pyrs (CPython 3.14 compatible)\n\nUsage:\n  pyrs                    Start interactive REPL (or read from stdin when piped)\n  pyrs <file.py>          Run a Python file\n  pyrs <file.pyc>         Run a CPython .pyc file\n  pyrs -S <file.py>       Run without importing site on startup\n  pyrs --ast <file.py>    Print parsed AST\n  pyrs --bytecode <file.py>  Print bytecode disassembly\n  pyrs --version          Print version\n  pyrs --help             Show help\n";

pub fn run() -> i32 {
    run_with_args_vec(env::args().skip(1).collect())
}

pub fn run_with_args_vec(arguments: Vec<String>) -> i32 {
    let mut args = arguments.into_iter().peekable();
    let mut import_site = true;

    // Parse a small subset of CPython-style startup flags used by stdlib tests.
    loop {
        let Some(flag) = args.peek().cloned() else {
            break;
        };
        match flag.as_str() {
            "-X" => {
                args.next();
                if args.next().is_none() {
                    eprintln!("error: -X expects an option");
                    return 2;
                }
            }
            "-S" | "--no-site" => {
                import_site = false;
                args.next();
            }
            _ => break,
        }
    }

    match args.next() {
        None => match repl::run_repl(import_site) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("error: {err}");
                2
            }
        },
        Some(flag) if flag == "-h" || flag == "--help" => {
            print_help();
            0
        }
        Some(flag) if flag == "--ast" => match args.next() {
            Some(path) => match run_ast(&path) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("error: {err}");
                    2
                }
            },
            None => {
                eprintln!("error: --ast expects a file path");
                2
            }
        },
        Some(flag) if flag == "--bytecode" => match args.next() {
            Some(path) => match run_bytecode(&path) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("{}", error_style::format_error_for_stderr(&err));
                    2
                }
            },
            None => {
                eprintln!("error: --bytecode expects a file path");
                2
            }
        },
        Some(flag) if flag == "-V" || flag == "--version" => {
            println!("pyrs {VERSION}");
            0
        }
        Some(flag) if flag == "-c" => match args.next() {
            Some(source) => match run_command(&source, import_site) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("{}", error_style::format_error_for_stderr(&err));
                    2
                }
            },
            None => {
                eprintln!("error: -c expects command string");
                2
            }
        },
        Some(path) => match run_file(&path, import_site) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{}", error_style::format_error_for_stderr(&err));
                2
            }
        },
    }
}

fn print_help() {
    println!("{HELP}");
}

fn run_file(path: &str, import_site: bool) -> Result<(), String> {
    let mut vm = Vm::new();
    configure_vm_for_execution(&mut vm, path, import_site)?;
    if path.ends_with(".pyc") {
        let exec_result = vm.execute_pyc_file(path);
        let shutdown_result = vm.run_shutdown_hooks();
        exec_result.map_err(|err| err.message)?;
        shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;
        return Ok(());
    }

    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    vm.cache_source_text(path, &source);

    let module =
        parser::parse_module(&source).map_err(|err| format_syntax_error(path, &source, &err))?;

    let code = compiler::compile_module_with_filename(&module, path)
        .map_err(|err| format_compile_error(path, &source, &err))?;

    let exec_result = vm.execute(&code);
    let shutdown_result = vm.run_shutdown_hooks();
    exec_result.map_err(|err| err.message)?;
    shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;

    Ok(())
}

fn run_command(source: &str, import_site: bool) -> Result<(), String> {
    let mut vm = Vm::new();
    configure_vm_for_command(&mut vm, import_site)?;
    vm.cache_source_text("<string>", source);

    let module = parser::parse_module(source)
        .map_err(|err| format_syntax_error("<string>", source, &err))?;

    let code = compiler::compile_module_with_filename(&module, "<string>")
        .map_err(|err| format_compile_error("<string>", source, &err))?;

    let exec_result = vm.execute(&code);
    let shutdown_result = vm.run_shutdown_hooks();
    exec_result.map_err(|err| err.message)?;
    shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;

    Ok(())
}

pub(super) fn format_syntax_error(filename: &str, source: &str, err: &ParseError) -> String {
    let diagnostic = classify_syntax_error(source, err);
    render_syntax_diagnostic(filename, source, &diagnostic, true)
}

pub(super) fn format_compile_error(
    filename: &str,
    source: &str,
    err: &compiler::CompileError,
) -> String {
    let span = err.span.unwrap_or(crate::ast::Span::new(1, 1));
    let diagnostic = SyntaxDiagnostic {
        error_type: "SyntaxError",
        message: err.message.clone(),
        line: span.line.max(1),
        column: span.column.max(1),
    };
    // CPython omits source+caret for semantic compile errors in `-c` mode.
    let include_source = filename != "<string>";
    render_syntax_diagnostic(filename, source, &diagnostic, include_source)
}

fn render_syntax_diagnostic(
    filename: &str,
    source: &str,
    diagnostic: &SyntaxDiagnostic,
    include_source: bool,
) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "  File \"{}\", line {}\n",
        filename, diagnostic.line
    ));
    if include_source
        && let Some((source_line, caret_start)) = source_line_and_caret_start(source, diagnostic)
    {
        output.push_str("    ");
        output.push_str(&source_line);
        output.push('\n');
        if caret_start > 0 {
            let start = caret_start
                .saturating_sub(1)
                .min(source_line.chars().count());
            let width = infer_syntax_caret_width(&source_line, start, diagnostic);
            output.push_str("    ");
            output.push_str(&" ".repeat(start));
            output.push_str(&"^".repeat(width));
            output.push('\n');
        }
    }
    output.push_str(diagnostic.error_type);
    output.push_str(": ");
    output.push_str(&diagnostic.message);
    output
}

#[derive(Debug, Clone)]
struct SyntaxDiagnostic {
    error_type: &'static str,
    message: String,
    line: usize,
    column: usize,
}

fn classify_syntax_error(source: &str, err: &ParseError) -> SyntaxDiagnostic {
    if let Some((line, column)) = detect_unexpected_top_level_indent(source) {
        let _ = column;
        return SyntaxDiagnostic {
            error_type: "IndentationError",
            message: "unexpected indent".to_string(),
            line,
            // CPython omits the caret for this case.
            column: 0,
        };
    }

    if let Some(issue) = detect_delimiter_issue(source) {
        return match issue {
            DelimiterIssue::UnmatchedClose {
                close,
                line,
                column,
            } => SyntaxDiagnostic {
                error_type: "SyntaxError",
                message: format!("unmatched '{}'", close),
                line,
                column,
            },
            DelimiterIssue::MismatchedClose {
                close,
                open,
                line,
                column,
            } => SyntaxDiagnostic {
                error_type: "SyntaxError",
                message: format!(
                    "closing parenthesis '{}' does not match opening parenthesis '{}'",
                    close, open
                ),
                line,
                column,
            },
            DelimiterIssue::UnclosedOpen { open, line, column } => {
                if matches!(open, '(' | '[')
                    && let Some(colon_column) = line_colon_after_column(source, line, column)
                {
                    SyntaxDiagnostic {
                        error_type: "SyntaxError",
                        message: "invalid syntax".to_string(),
                        line,
                        column: colon_column,
                    }
                } else {
                    SyntaxDiagnostic {
                        error_type: "SyntaxError",
                        message: format!("'{}' was never closed", open),
                        line,
                        column,
                    }
                }
            }
        };
    }

    let message_lower = err.message.to_ascii_lowercase();
    if message_lower.starts_with("expected indent") {
        return SyntaxDiagnostic {
            error_type: "IndentationError",
            message: "expected an indented block".to_string(),
            line: err.line,
            column: err.column,
        };
    }
    if message_lower.contains("indentation does not match any outer level")
        || message_lower.contains("unindent does not match any outer indentation level")
    {
        return SyntaxDiagnostic {
            error_type: "IndentationError",
            message: "unindent does not match any outer indentation level".to_string(),
            line: err.line,
            column: line_end_column(source, err.line).unwrap_or(err.column),
        };
    }
    if message_lower.starts_with("expected dedent") {
        return SyntaxDiagnostic {
            error_type: "IndentationError",
            message: "unindent does not match any outer indentation level".to_string(),
            line: err.line,
            column: err.column,
        };
    }
    if message_lower.starts_with("unexpected indent") {
        return SyntaxDiagnostic {
            error_type: "IndentationError",
            message: "unexpected indent".to_string(),
            line: err.line,
            column: err.column,
        };
    }
    if message_lower.contains("unterminated string literal")
        && !message_lower.contains("(detected at line")
    {
        let is_triple_quote = starts_with_triple_quote_at(source, err.line, err.column);
        let detected_line = if is_triple_quote {
            source.lines().count().max(err.line)
        } else {
            err.line
        };
        let message = if is_triple_quote {
            format!(
                "unterminated triple-quoted string literal (detected at line {})",
                detected_line
            )
        } else {
            format!(
                "unterminated string literal (detected at line {})",
                detected_line
            )
        };
        return SyntaxDiagnostic {
            error_type: "SyntaxError",
            message,
            line: err.line,
            column: err.column,
        };
    }
    if message_lower.starts_with("expected ")
        || message_lower.contains("unexpected token")
        || message_lower.contains("unexpected character")
    {
        return SyntaxDiagnostic {
            error_type: "SyntaxError",
            message: "invalid syntax".to_string(),
            line: err.line,
            column: err.column,
        };
    }

    SyntaxDiagnostic {
        error_type: "SyntaxError",
        message: err.message.clone(),
        line: err.line,
        column: err.column,
    }
}

fn source_line_and_caret_start(
    source: &str,
    diagnostic: &SyntaxDiagnostic,
) -> Option<(String, usize)> {
    let source_lines = source
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect::<Vec<_>>();
    if source_lines.is_empty() {
        return None;
    }

    let requested_index = diagnostic.line.saturating_sub(1);
    let (line_index, caret_start_raw) = if requested_index < source_lines.len() {
        (requested_index, diagnostic.column)
    } else {
        // EOF-oriented parser errors typically point to the next line; mirror CPython by
        // showing the last source line and placing caret at end-of-line.
        let last_index = source_lines.len().saturating_sub(1);
        let line = &source_lines[last_index];
        let leading = line.chars().take_while(|ch| ch.is_whitespace()).count();
        let visible_len = line.chars().count().saturating_sub(leading);
        (last_index, visible_len.saturating_add(1))
    };
    let line = source_lines.get(line_index)?.clone();
    let leading = line.chars().take_while(|ch| ch.is_whitespace()).count();
    let display_line: String = line.chars().skip(leading).collect();
    let caret_start = if caret_start_raw == 0 {
        0
    } else {
        caret_start_raw.saturating_sub(leading).max(1)
    };
    Some((display_line, caret_start))
}

enum DelimiterIssue {
    UnmatchedClose {
        close: char,
        line: usize,
        column: usize,
    },
    MismatchedClose {
        close: char,
        open: char,
        line: usize,
        column: usize,
    },
    UnclosedOpen {
        open: char,
        line: usize,
        column: usize,
    },
}

fn detect_delimiter_issue(source: &str) -> Option<DelimiterIssue> {
    let mut lexer = parser::lexer::Lexer::new(source);
    let tokens = lexer.tokenize().ok()?;
    let mut stack: Vec<(char, usize, usize)> = Vec::new();
    for token in tokens {
        match token.kind {
            parser::token::TokenKind::LParen => stack.push(('(', token.line, token.column)),
            parser::token::TokenKind::LBracket => stack.push(('[', token.line, token.column)),
            parser::token::TokenKind::LBrace => stack.push(('{', token.line, token.column)),
            parser::token::TokenKind::RParen => {
                if let Some((open, _, _)) = stack.last().copied() {
                    if open == '(' {
                        stack.pop();
                    } else {
                        return Some(DelimiterIssue::MismatchedClose {
                            close: ')',
                            open,
                            line: token.line,
                            column: token.column,
                        });
                    }
                } else {
                    return Some(DelimiterIssue::UnmatchedClose {
                        close: ')',
                        line: token.line,
                        column: token.column,
                    });
                }
            }
            parser::token::TokenKind::RBracket => {
                if let Some((open, _, _)) = stack.last().copied() {
                    if open == '[' {
                        stack.pop();
                    } else {
                        return Some(DelimiterIssue::MismatchedClose {
                            close: ']',
                            open,
                            line: token.line,
                            column: token.column,
                        });
                    }
                } else {
                    return Some(DelimiterIssue::UnmatchedClose {
                        close: ']',
                        line: token.line,
                        column: token.column,
                    });
                }
            }
            parser::token::TokenKind::RBrace => {
                if let Some((open, _, _)) = stack.last().copied() {
                    if open == '{' {
                        stack.pop();
                    } else {
                        return Some(DelimiterIssue::MismatchedClose {
                            close: '}',
                            open,
                            line: token.line,
                            column: token.column,
                        });
                    }
                } else {
                    return Some(DelimiterIssue::UnmatchedClose {
                        close: '}',
                        line: token.line,
                        column: token.column,
                    });
                }
            }
            _ => {}
        }
    }
    stack
        .pop()
        .map(|(open, line, column)| DelimiterIssue::UnclosedOpen { open, line, column })
}

fn infer_syntax_caret_width(
    source_line: &str,
    start: usize,
    diagnostic: &SyntaxDiagnostic,
) -> usize {
    let chars: Vec<char> = source_line.chars().collect();
    if chars.is_empty() || start >= chars.len() {
        return 1;
    }
    let message = diagnostic.message.as_str();
    if message.contains("global declaration")
        || message.contains("binding for nonlocal")
        || message.contains("nonlocal and global")
        || message.contains("parameter and global")
        || message.contains("parameter and nonlocal")
        || message.contains("nonlocal declaration not allowed at module level")
    {
        return chars.len().saturating_sub(start).max(1);
    }
    let current = chars[start];
    if is_identifier_start(current) {
        let mut idx = start + 1;
        while idx < chars.len() && is_identifier_continue(chars[idx]) {
            idx += 1;
        }
        return idx.saturating_sub(start).max(1);
    }
    1
}

fn line_end_column(source: &str, line: usize) -> Option<usize> {
    source.lines().nth(line.saturating_sub(1)).map(|entry| {
        entry
            .trim_end_matches('\r')
            .chars()
            .count()
            .saturating_add(1)
    })
}

fn line_colon_after_column(source: &str, line: usize, column: usize) -> Option<usize> {
    let line_text = source
        .lines()
        .nth(line.saturating_sub(1))
        .map(|entry| entry.trim_end_matches('\r'))?;
    for (idx, ch) in line_text.chars().enumerate() {
        let one_based = idx.saturating_add(1);
        if one_based <= column {
            continue;
        }
        if ch == ':' {
            return Some(one_based);
        }
    }
    None
}

fn detect_unexpected_top_level_indent(source: &str) -> Option<(usize, usize)> {
    for (idx, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let leading = line
            .chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .count();
        if leading > 0 {
            return Some((idx.saturating_add(1), 0));
        }
        return None;
    }
    None
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn starts_with_triple_quote_at(source: &str, line: usize, column: usize) -> bool {
    let Some(line_text) = source.lines().nth(line.saturating_sub(1)) else {
        return false;
    };
    let mut chars = line_text.chars();
    for _ in 1..column {
        if chars.next().is_none() {
            return false;
        }
    }
    let rest = chars.collect::<String>();
    rest.starts_with("'''") || rest.starts_with("\"\"\"")
}

fn configure_vm_for_execution(
    vm: &mut Vm,
    script_path: &str,
    import_site: bool,
) -> Result<(), String> {
    configure_vm_for_command(vm, import_site)?;
    if let Some(parent) = Path::new(script_path).parent()
        && !parent.as_os_str().is_empty()
    {
        vm.add_module_path_front(parent.to_path_buf());
    }
    Ok(())
}

fn configure_vm_for_command(vm: &mut Vm, import_site: bool) -> Result<(), String> {
    let pythonpath_entries = detect_pythonpath_entries();
    let virtualenv_site_entries = detect_virtualenv_site_packages_entries();
    let (stdlib_paths, strict_site_import) = detect_cpython_stdlib_paths();
    for path in pythonpath_entries {
        vm.add_module_path(path);
    }
    vm.set_sys_no_site_flag(!import_site);
    for stdlib_path in &stdlib_paths {
        vm.add_module_path(stdlib_path.clone());
    }
    for path in virtualenv_site_entries {
        vm.add_module_path(path);
    }
    if import_site
        && !stdlib_paths.is_empty()
        && let Err(err) = vm.import_module("site")
        && strict_site_import
    {
        return Err(format!("startup site import failed: {}", err.message));
    }
    Ok(())
}

fn detect_cpython_stdlib_paths() -> (Vec<PathBuf>, bool) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut strict_site_import = false;

    fn register_unique_path(
        out: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        candidate: PathBuf,
    ) {
        if candidate.as_os_str().is_empty() {
            return;
        }
        let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }

    fn register_stdlib_root(
        out: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        candidate: PathBuf,
    ) -> Option<PathBuf> {
        if candidate.as_os_str().is_empty() {
            return None;
        }
        let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        if !normalized.join("site.py").is_file() {
            return None;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized.clone());
        }
        Some(normalized)
    }

    fn register_dynload_for_root(
        out: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        root: &PathBuf,
    ) -> bool {
        let dynload = root.join("lib-dynload");
        if dynload.is_dir() {
            register_unique_path(out, seen, dynload);
            return true;
        }
        false
    }

    if let Ok(path) = env::var("PYRS_CPYTHON_LIB") {
        strict_site_import = true;
        let mut has_local_dynload = false;
        if let Some(root) = register_stdlib_root(&mut out, &mut seen, PathBuf::from(path)) {
            has_local_dynload = register_dynload_for_root(&mut out, &mut seen, &root);
        }
        // Keep stdlib root isolated from host .py trees, but if the isolated
        // root has no adjacent lib-dynload, fall back to a host 3.14
        // lib-dynload directory for native extension loading.
        if !has_local_dynload {
            for candidate in [
                "/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
                "/opt/homebrew/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
                "/usr/local/lib/python3.14",
                "/usr/lib/python3.14",
            ] {
                let candidate_path = PathBuf::from(candidate);
                let normalized = std::fs::canonicalize(&candidate_path).unwrap_or(candidate_path);
                if register_dynload_for_root(&mut out, &mut seen, &normalized) {
                    break;
                }
            }
        }
        // When PYRS_CPYTHON_LIB is set, keep sys.path isolated to that stdlib root
        // (plus its adjacent lib-dynload if present) instead of mixing in host
        // framework stdlib paths. This avoids cross-root semantic drift in tests.
        return (out, strict_site_import);
    }
    if let Ok(home) = env::var("PYTHONHOME") {
        if let Some(root) = register_stdlib_root(
            &mut out,
            &mut seen,
            PathBuf::from(home).join("lib").join("python3.14"),
        ) {
            register_dynload_for_root(&mut out, &mut seen, &root);
        }
    }

    for candidate in [
        "/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
        "/opt/homebrew/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
        "/usr/local/lib/python3.14",
        "/usr/lib/python3.14",
    ] {
        let candidate_path = PathBuf::from(candidate);
        let normalized = std::fs::canonicalize(&candidate_path).unwrap_or(candidate_path);
        if normalized.join("site.py").is_file() {
            if seen.insert(normalized.clone()) {
                out.push(normalized.clone());
            }
            register_dynload_for_root(&mut out, &mut seen, &normalized);
            break;
        }
    }

    (out, strict_site_import)
}

fn detect_pythonpath_entries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let Some(path) = env::var_os("PYTHONPATH") else {
        return out;
    };
    for entry in env::split_paths(&path) {
        if entry.as_os_str().is_empty() {
            continue;
        }
        let normalized = std::fs::canonicalize(&entry).unwrap_or(entry);
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn detect_virtualenv_site_packages_entries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let Some(venv_raw) = env::var_os("VIRTUAL_ENV") else {
        return out;
    };
    let venv = PathBuf::from(venv_raw);
    if !venv.is_dir() {
        return out;
    }
    for candidate in [
        venv.join("lib").join("python3.14").join("site-packages"),
        venv.join("lib64").join("python3.14").join("site-packages"),
        venv.join("Lib").join("site-packages"),
    ] {
        if !candidate.is_dir() {
            continue;
        }
        let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn run_ast(path: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let module =
        parser::parse_module(&source).map_err(|err| format_syntax_error(path, &source, &err))?;
    println!("{module:#?}");
    Ok(())
}

fn run_bytecode(path: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let module =
        parser::parse_module(&source).map_err(|err| format_syntax_error(path, &source, &err))?;
    let code = compiler::compile_module_with_filename(&module, path)
        .map_err(|err| format!("compile error: {}", err.message))?;
    print_code_recursive(&code, 0);
    Ok(())
}

fn print_code_recursive(code: &crate::bytecode::CodeObject, depth: usize) {
    let indent = "  ".repeat(depth);
    println!("{indent}code {}:", code.name);
    if std::env::var_os("PYRS_BYTECODE_SHOW_TABLES").is_some() {
        println!("{indent}  names:");
        for (idx, name) in code.names.iter().enumerate() {
            println!("{indent}    {idx:04}: {name}");
        }
        println!("{indent}  constants:");
        for (idx, value) in code.constants.iter().enumerate() {
            println!("{indent}    {idx:04}: {value:?}");
        }
    }
    for line in code.disassemble().lines() {
        println!("{indent}  {line}");
    }
    for constant in &code.constants {
        if let Value::Code(code_obj) = constant {
            print_code_recursive(code_obj, depth + 1);
        }
    }
}
