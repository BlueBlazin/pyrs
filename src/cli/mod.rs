//! CLI entry point and argument handling.

mod error_style;
mod repl;

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crate::compiler;
use crate::parser;
use crate::parser::ParseError;
use crate::runtime::Value;
use crate::vm::Vm;
use crate::{CPYTHON_COMPAT_VERSION, CPYTHON_STDLIB_VERSION, VERSION};

const HELP: &str = "pyrs (CPython 3.14 compatible)\n\nUsage:\n  pyrs                    Start interactive REPL (or read from stdin when piped)\n  pyrs <file.py>          Run a Python file\n  pyrs <file.pyc>         Run a CPython .pyc file\n  pyrs -S <file.py>       Run without importing site on startup\n  pyrs --ast <file.py>    Print parsed AST\n  pyrs --bytecode <file.py>  Print bytecode disassembly\n  pyrs --version          Print version\n  pyrs --help             Show help\n";
const CPYTHON_STDLIB_RELEASE_PAGE_URL: &str =
    "https://www.python.org/downloads/release/python-3143/";

pub fn run() -> i32 {
    run_with_args_vec(env::args().skip(1).collect())
}

pub fn run_with_args_vec(arguments: Vec<String>) -> i32 {
    let mut args = arguments.into_iter().peekable();
    let mut import_site = true;
    let mut traceback_caret_enabled = true;
    let mut use_environment = true;
    let mut command_warnoptions = Vec::new();
    let mut startup_tracemalloc_limit = None;

    // Parse a small subset of CPython-style startup flags used by stdlib tests.
    loop {
        let Some(flag) = args.peek().cloned() else {
            break;
        };
        match flag.as_str() {
            "-X" => {
                args.next();
                let Some(option) = args.next() else {
                    eprintln!("error: -X expects an option");
                    return 2;
                };
                if option == "no_debug_ranges" {
                    traceback_caret_enabled = false;
                } else if option == "tracemalloc" {
                    startup_tracemalloc_limit = Some(1);
                } else if let Some(raw_limit) = option.strip_prefix("tracemalloc=") {
                    let Ok(limit) = raw_limit.parse::<usize>() else {
                        eprintln!("error: invalid -X tracemalloc value: {raw_limit}");
                        return 2;
                    };
                    if !(1..=65_535).contains(&limit) {
                        eprintln!("error: invalid -X tracemalloc value: {raw_limit}");
                        return 2;
                    }
                    startup_tracemalloc_limit = Some(limit);
                }
            }
            "-W" => {
                args.next();
                let Some(option) = args.next() else {
                    eprintln!("error: -W expects an option");
                    return 2;
                };
                command_warnoptions.push(option);
            }
            _ if flag.starts_with("-W") && flag.len() > 2 => {
                args.next();
                command_warnoptions.push(flag[2..].to_string());
            }
            // Compatibility no-ops accepted by CPython command-lines used in stdlib tests.
            // We consume them so they are not misparsed as script filenames.
            "-I" | "-u" | "-E" | "-B" => {
                if flag == "-I" || flag == "-E" {
                    use_environment = false;
                }
                args.next();
            }
            "-S" | "--no-site" => {
                import_site = false;
                args.next();
            }
            _ => break,
        }
    }
    let mut warnoptions = if use_environment {
        parse_warning_options(env::var("PYTHONWARNINGS").ok())
    } else {
        Vec::new()
    };
    warnoptions.extend(command_warnoptions);
    warnoptions = sanitize_warning_options(warnoptions);

    match args.next() {
        None => match repl::run_repl(import_site, warnoptions.clone(), startup_tracemalloc_limit) {
            Ok(status) => status,
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
            Some(source) => {
                let command_args = args.collect::<Vec<_>>();
                match run_command(
                    &source,
                    command_args,
                    import_site,
                    traceback_caret_enabled,
                    warnoptions,
                    startup_tracemalloc_limit,
                ) {
                    Ok(status) => status,
                    Err(err) => {
                        eprintln!("{}", error_style::format_error_for_stderr(&err));
                        2
                    }
                }
            }
            None => {
                eprintln!("error: -c expects command string");
                2
            }
        },
        Some(path) => {
            let script_args = args.collect::<Vec<_>>();
            match run_file(
                &path,
                script_args,
                import_site,
                traceback_caret_enabled,
                warnoptions,
                startup_tracemalloc_limit,
            ) {
                Ok(status) => status,
                Err(err) => {
                    eprintln!("{}", error_style::format_error_for_stderr(&err));
                    2
                }
            }
        }
    }
}

fn print_help() {
    println!("{HELP}");
}

fn system_exit_outcome(
    vm: &mut Vm,
    err: &crate::runtime::RuntimeError,
) -> Option<(i32, Option<String>)> {
    let exception = err.exception.as_ref()?;
    if exception.name != "SystemExit" {
        return None;
    }
    let code = exception
        .attrs
        .borrow()
        .get("code")
        .cloned()
        .unwrap_or(Value::None);
    let outcome = match code {
        Value::None => {
            let parsed = err
                .message
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| line.starts_with("SystemExit"))
                .and_then(|line| line.strip_prefix("SystemExit:"))
                .map(str::trim)
                .map(|payload| {
                    if payload == "None" {
                        (0, None)
                    } else if let Ok(code) = payload.parse::<i32>() {
                        (code, None)
                    } else {
                        (1, Some(payload.to_string()))
                    }
                });
            parsed.unwrap_or((0, None))
        }
        Value::Bool(flag) => (if flag { 1 } else { 0 }, None),
        Value::Int(number) => (number as i32, None),
        Value::BigInt(number) => (
            number
                .to_i64()
                .and_then(|value| i32::try_from(value).ok())
                .unwrap_or(-1),
            None,
        ),
        Value::Str(text) => (1, Some(text)),
        other => (
            1,
            Some(
                vm.render_value_repr_for_display(other)
                    .unwrap_or_else(|_| "SystemExit".to_string()),
            ),
        ),
    };
    Some(outcome)
}

fn run_file(
    path: &str,
    script_args: Vec<String>,
    import_site: bool,
    traceback_caret_enabled: bool,
    warnoptions: Vec<String>,
    startup_tracemalloc_limit: Option<usize>,
) -> Result<i32, String> {
    let script_execution_path = {
        let candidate = PathBuf::from(path);
        let absolute = if candidate.is_absolute() {
            candidate
        } else {
            env::current_dir()
                .map_err(|err| format!("failed to resolve current directory: {err}"))?
                .join(candidate)
        };
        std::fs::canonicalize(&absolute).unwrap_or(absolute)
    };
    let script_execution_path = script_execution_path.to_string_lossy().to_string();
    let mut vm = Vm::new();
    configure_vm_for_execution(
        &mut vm,
        &script_execution_path,
        import_site,
        traceback_caret_enabled,
        &warnoptions,
    )?;
    if let Some(limit) = startup_tracemalloc_limit {
        vm.start_tracemalloc(limit);
    }
    let mut argv = Vec::with_capacity(1 + script_args.len());
    argv.push(path.to_string());
    argv.extend(script_args);
    vm.set_sys_argv(argv);
    // CPython script mode exposes the executed path via __main__.__file__.
    vm.set_global("__file__", Value::Str(script_execution_path.clone()));
    if script_execution_path.ends_with(".pyc") {
        let exec_result = vm.execute_pyc_file(&script_execution_path);
        let shutdown_result = vm.run_shutdown_hooks();
        let (status, stderr_message) = match exec_result {
            Ok(_) => (0, None),
            Err(err) => {
                if let Some(outcome) = system_exit_outcome(&mut vm, &err) {
                    outcome
                } else {
                    return Err(err.message);
                }
            }
        };
        shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;
        if let Some(message) = stderr_message {
            eprintln!("{message}");
        }
        return Ok(status);
    }

    let source = vm
        .read_python_source_file(&script_execution_path)
        .map_err(|err| err.message)?;
    vm.cache_source_text(&script_execution_path, &source);

    let module = parser::parse_module(&source)
        .map_err(|err| format_syntax_error(&script_execution_path, &source, &err))?;

    let code = compiler::compile_module_with_filename(&module, &script_execution_path)
        .map_err(|err| format_compile_error(&script_execution_path, &source, &err))?;

    let exec_result = vm.execute(&code);
    let shutdown_result = vm.run_shutdown_hooks();
    let (status, stderr_message) = match exec_result {
        Ok(_) => (0, None),
        Err(err) => {
            if let Some(outcome) = system_exit_outcome(&mut vm, &err) {
                outcome
            } else {
                return Err(err.message);
            }
        }
    };
    shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;
    if let Some(message) = stderr_message {
        eprintln!("{message}");
    }

    Ok(status)
}

fn run_command(
    source: &str,
    command_args: Vec<String>,
    import_site: bool,
    traceback_caret_enabled: bool,
    warnoptions: Vec<String>,
    startup_tracemalloc_limit: Option<usize>,
) -> Result<i32, String> {
    let mut vm = Vm::new();
    configure_vm_for_command(&mut vm, import_site, traceback_caret_enabled, &warnoptions)?;
    if let Some(limit) = startup_tracemalloc_limit {
        vm.start_tracemalloc(limit);
    }
    let mut argv = Vec::with_capacity(1 + command_args.len());
    argv.push("-c".to_string());
    argv.extend(command_args);
    vm.set_sys_argv(argv);
    vm.cache_source_text("<string>", source);

    let module = parser::parse_module(source)
        .map_err(|err| format_syntax_error("<string>", source, &err))?;

    let code = compiler::compile_module_with_filename(&module, "<string>")
        .map_err(|err| format_compile_error("<string>", source, &err))?;
    vm.register_source_in_linecache(&code, source, "<string>");

    let exec_result = vm.execute(&code);
    let shutdown_result = vm.run_shutdown_hooks();
    let (status, stderr_message) = match exec_result {
        Ok(_) => (0, None),
        Err(err) => {
            if let Some(outcome) = system_exit_outcome(&mut vm, &err) {
                outcome
            } else {
                return Err(err.message);
            }
        }
    };
    shutdown_result.map_err(|err| format!("shutdown error: {}", err.message))?;
    if let Some(message) = stderr_message {
        eprintln!("{message}");
    }

    Ok(status)
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

const KEYWORD_SUGGESTION_ORDER: &[&str] = &[
    "and", "for", "else", "while", "if", "elif", "try", "class", "import", "from", "def", "return",
    "lambda", "yield", "global", "async", "await", "raise", "in", "as", "assert", "break", "case",
    "continue", "del", "except", "finally", "is", "match", "nonlocal", "not", "or", "pass", "type",
    "with",
];

fn is_keyword_token(token: &str) -> bool {
    KEYWORD_SUGGESTION_ORDER.contains(&token) || matches!(token, "true" | "false" | "none")
}

fn is_identifier_start_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_identifier_continue_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn bounded_levenshtein(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    if a_bytes.len().abs_diff(b_bytes.len()) > limit {
        return None;
    }
    let mut previous = (0..=b_bytes.len()).collect::<Vec<_>>();
    let mut current = vec![0usize; b_bytes.len() + 1];
    for (i, a_byte) in a_bytes.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, b_byte) in b_bytes.iter().enumerate() {
            let cost = usize::from(*a_byte != *b_byte);
            let deletion = previous[j + 1] + 1;
            let insertion = current[j] + 1;
            let substitution = previous[j] + cost;
            let value = deletion.min(insertion).min(substitution);
            current[j + 1] = value;
            row_min = row_min.min(value);
        }
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b_bytes.len()];
    (distance <= limit).then_some(distance)
}

fn keyword_typo_suggestion(source: &str, line: usize, column: usize) -> Option<&'static str> {
    let source_line = source.lines().nth(line.saturating_sub(1))?;
    let bytes = source_line.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut words: Vec<(usize, usize, String)> = Vec::new();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if !is_identifier_start_byte(bytes[idx]) {
            idx += 1;
            continue;
        }
        let start = idx;
        idx += 1;
        while idx < bytes.len() && is_identifier_continue_byte(bytes[idx]) {
            idx += 1;
        }
        let end = idx;
        words.push((start, end, source_line[start..end].to_ascii_lowercase()));
    }
    if words.is_empty() {
        return None;
    }

    let target = column.saturating_sub(1).min(bytes.len().saturating_sub(1));
    let mut active_index = 0usize;
    let mut best_distance = usize::MAX;
    for (index, (start, end, _word)) in words.iter().enumerate() {
        let distance = if target < *start {
            *start - target
        } else if target >= *end {
            target.saturating_sub(end.saturating_sub(1))
        } else {
            0
        };
        if distance < best_distance {
            best_distance = distance;
            active_index = index;
        }
    }

    let mut ranked = Vec::<(&'static str, usize, usize, usize)>::new();
    for index in 0..words.len() {
        let token = words[index].2.as_str();
        if token.len() < 2
            || token.contains('_')
            || token.bytes().any(|byte| byte.is_ascii_digit())
            || is_keyword_token(token)
        {
            continue;
        }

        let previous_word = index
            .checked_sub(1)
            .and_then(|prev| words.get(prev))
            .map(|entry| entry.2.as_str());
        let previous_previous_word = index
            .checked_sub(2)
            .and_then(|prev| words.get(prev))
            .map(|entry| entry.2.as_str());
        let next_word = words.get(index + 1).map(|entry| entry.2.as_str());
        let has_later_import = words
            .iter()
            .skip(index + 1)
            .any(|entry| entry.2 == "import");

        let forced_keyword = if has_later_import {
            bounded_levenshtein(token, "from", 2).map(|distance| ("from", distance))
        } else if previous_word == Some("for") || previous_previous_word == Some("for") {
            bounded_levenshtein(token, "in", 2).map(|distance| ("in", distance))
        } else if token.starts_with("else") {
            if next_word.is_some() {
                bounded_levenshtein(token, "elif", 2).map(|distance| ("elif", distance))
            } else {
                None
            }
            .or_else(|| bounded_levenshtein(token, "else", 2).map(|distance| ("else", distance)))
        } else if previous_word.is_some_and(|word| !is_keyword_token(word))
            && next_word.is_some_and(|word| !is_keyword_token(word))
        {
            bounded_levenshtein(token, "and", 2).map(|distance| ("and", distance))
        } else {
            None
        };

        let (keyword, distance) = if let Some(forced) = forced_keyword {
            forced
        } else {
            let mut best_keyword = None::<(&'static str, usize)>;
            for keyword in KEYWORD_SUGGESTION_ORDER {
                let Some(distance) = bounded_levenshtein(token, keyword, 2) else {
                    continue;
                };
                if best_keyword
                    .as_ref()
                    .is_none_or(|(_best_keyword, best_distance)| distance < *best_distance)
                {
                    best_keyword = Some((keyword, distance));
                }
            }
            let Some(best) = best_keyword else {
                continue;
            };
            best
        };

        // Parser errors often point at the token after a misspelled keyword.
        // Prefer the word immediately before the caret over the active word on ties.
        let proximity_rank = if index + 1 == active_index {
            0
        } else if index == active_index {
            1
        } else if index + 2 == active_index {
            2
        } else if index == active_index + 1 {
            3
        } else {
            4 + index.abs_diff(active_index)
        };
        ranked.push((keyword, distance, proximity_rank, index));
    }

    ranked
        .into_iter()
        .min_by_key(|(_keyword, distance, proximity_rank, index)| {
            (*distance, *proximity_rank, *index)
        })
        .map(|(keyword, _distance, _proximity_rank, _index)| keyword)
}

fn with_keyword_typo_suggestion(
    source: &str,
    mut diagnostic: SyntaxDiagnostic,
) -> SyntaxDiagnostic {
    if diagnostic.error_type != "SyntaxError" || diagnostic.message.contains("Did you mean '") {
        return diagnostic;
    }
    if let Some(keyword) = keyword_typo_suggestion(source, diagnostic.line, diagnostic.column) {
        if !diagnostic.message.ends_with('.') {
            diagnostic.message.push('.');
        }
        diagnostic
            .message
            .push_str(&format!(" Did you mean '{keyword}'?"));
    }
    diagnostic
}

fn classify_syntax_error(source: &str, err: &ParseError) -> SyntaxDiagnostic {
    if let Some((line, column)) = detect_unexpected_top_level_indent(source) {
        let _ = column;
        return with_keyword_typo_suggestion(
            source,
            SyntaxDiagnostic {
                error_type: "IndentationError",
                message: "unexpected indent".to_string(),
                line,
                // CPython omits the caret for this case.
                column: 0,
            },
        );
    }

    if let Some(issue) = detect_delimiter_issue(source) {
        return match issue {
            DelimiterIssue::UnmatchedClose {
                close,
                line,
                column,
            } => with_keyword_typo_suggestion(
                source,
                SyntaxDiagnostic {
                    error_type: "SyntaxError",
                    message: format!("unmatched '{}'", close),
                    line,
                    column,
                },
            ),
            DelimiterIssue::MismatchedClose {
                close,
                open,
                line,
                column,
            } => with_keyword_typo_suggestion(
                source,
                SyntaxDiagnostic {
                    error_type: "SyntaxError",
                    message: format!(
                        "closing parenthesis '{}' does not match opening parenthesis '{}'",
                        close, open
                    ),
                    line,
                    column,
                },
            ),
            DelimiterIssue::UnclosedOpen { open, line, column } => {
                if matches!(open, '(' | '[')
                    && let Some(colon_column) = line_colon_after_column(source, line, column)
                {
                    with_keyword_typo_suggestion(
                        source,
                        SyntaxDiagnostic {
                            error_type: "SyntaxError",
                            message: "invalid syntax".to_string(),
                            line,
                            column: colon_column,
                        },
                    )
                } else {
                    with_keyword_typo_suggestion(
                        source,
                        SyntaxDiagnostic {
                            error_type: "SyntaxError",
                            message: format!("'{}' was never closed", open),
                            line,
                            column,
                        },
                    )
                }
            }
        };
    }

    let message_lower = err.message.to_ascii_lowercase();
    if message_lower.starts_with("expected indent") {
        return with_keyword_typo_suggestion(
            source,
            SyntaxDiagnostic {
                error_type: "IndentationError",
                message: "expected an indented block".to_string(),
                line: err.line,
                column: err.column,
            },
        );
    }
    if message_lower.contains("indentation does not match any outer level")
        || message_lower.contains("unindent does not match any outer indentation level")
    {
        return with_keyword_typo_suggestion(
            source,
            SyntaxDiagnostic {
                error_type: "IndentationError",
                message: "unindent does not match any outer indentation level".to_string(),
                line: err.line,
                column: line_end_column(source, err.line).unwrap_or(err.column),
            },
        );
    }
    if message_lower.starts_with("expected dedent") {
        return with_keyword_typo_suggestion(
            source,
            SyntaxDiagnostic {
                error_type: "IndentationError",
                message: "unindent does not match any outer indentation level".to_string(),
                line: err.line,
                column: err.column,
            },
        );
    }
    if message_lower.starts_with("unexpected indent") {
        return with_keyword_typo_suggestion(
            source,
            SyntaxDiagnostic {
                error_type: "IndentationError",
                message: "unexpected indent".to_string(),
                line: err.line,
                column: err.column,
            },
        );
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
        return with_keyword_typo_suggestion(
            source,
            SyntaxDiagnostic {
                error_type: "SyntaxError",
                message,
                line: err.line,
                column: err.column,
            },
        );
    }
    if message_lower.starts_with("expected ")
        || message_lower.contains("unexpected token")
        || message_lower.contains("unexpected character")
    {
        return with_keyword_typo_suggestion(
            source,
            SyntaxDiagnostic {
                error_type: "SyntaxError",
                message: "invalid syntax".to_string(),
                line: err.line,
                column: err.column,
            },
        );
    }

    with_keyword_typo_suggestion(
        source,
        SyntaxDiagnostic {
            error_type: "SyntaxError",
            message: err.message.clone(),
            line: err.line,
            column: err.column,
        },
    )
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
    traceback_caret_enabled: bool,
    warnoptions: &[String],
) -> Result<(), String> {
    configure_vm_for_command(vm, import_site, traceback_caret_enabled, warnoptions)?;
    if let Some(parent) = Path::new(script_path).parent()
        && !parent.as_os_str().is_empty()
    {
        vm.add_module_path_front(parent.to_path_buf());
    }
    Ok(())
}

fn configure_vm_for_command(
    vm: &mut Vm,
    import_site: bool,
    traceback_caret_enabled: bool,
    warnoptions: &[String],
) -> Result<(), String> {
    let pythonpath_entries = detect_pythonpath_entries();
    let virtualenv_site_entries = detect_virtualenv_site_packages_entries();
    let (stdlib_paths, strict_site_import) = detect_cpython_stdlib_paths();
    for path in pythonpath_entries {
        vm.add_module_path(path);
    }
    vm.set_sys_no_site_flag(!import_site);
    vm.set_sys_warnoptions(warnoptions.to_vec());
    vm.set_traceback_caret_enabled(traceback_caret_enabled);
    for stdlib_path in &stdlib_paths {
        vm.add_module_path(stdlib_path.clone());
    }
    for path in virtualenv_site_entries {
        vm.add_module_path(path);
    }
    if !warnoptions.is_empty() {
        let _ = vm.import_module("warnings");
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

pub(super) fn missing_cpython_stdlib_warning(import_site: bool) -> Option<String> {
    if !import_site {
        return None;
    }
    let (stdlib_paths, _) = detect_cpython_stdlib_paths();
    if !stdlib_paths.is_empty() {
        return None;
    }
    Some(format!(
        "warning: no CPython {CPYTHON_COMPAT_VERSION} stdlib was detected. `cargo install` only installs the pyrs binary.\n\
set PYRS_CPYTHON_LIB=/path/to/python3.14/Lib or install CPython 3.14.\n\
official CPython 3.14.3 downloads: {CPYTHON_STDLIB_RELEASE_PAGE_URL}"
    ))
}

fn parse_warning_options(raw: Option<String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    raw.split(',')
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.to_string())
        .collect()
}

fn warning_action_is_valid(action: &str) -> bool {
    let action = action.trim().to_ascii_lowercase();
    if action == "all" {
        return true;
    }
    if action.is_empty() {
        return false;
    }
    ["default", "always", "ignore", "module", "once", "error"]
        .iter()
        .any(|candidate| candidate.starts_with(&action))
}

fn sanitize_warning_options(options: Vec<String>) -> Vec<String> {
    let mut valid = Vec::with_capacity(options.len());
    for option in options {
        let action = option.split(':').next().unwrap_or_default();
        if warning_action_is_valid(action) {
            valid.push(option);
        } else {
            eprintln!("Invalid -W option ignored: invalid action: '{action}'");
        }
    }
    valid
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

    fn host_stdlib_roots() -> [PathBuf; 4] {
        [
            PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
            PathBuf::from("/opt/homebrew/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
            PathBuf::from("/usr/local/lib/python3.14"),
            PathBuf::from("/usr/lib/python3.14"),
        ]
    }

    fn register_host_dynload_fallback(out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
        for candidate in host_stdlib_roots() {
            let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
            if register_dynload_for_root(out, seen, &normalized) {
                break;
            }
        }
    }

    fn install_managed_stdlib_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let stdlib_suffix = PathBuf::from(format!("stdlib/{CPYTHON_STDLIB_VERSION}/Lib"));
        if let Ok(executable_path) = env::current_exe()
            && let Some(bin_dir) = executable_path.parent()
        {
            roots.push(bin_dir.join("../share/pyrs").join(&stdlib_suffix));
            roots.push(bin_dir.join("../libexec").join(&stdlib_suffix));
            roots.push(bin_dir.join("../stdlib").join(&stdlib_suffix));
        }
        if let Some(xdg_data_home) = env::var_os("XDG_DATA_HOME") {
            roots.push(
                PathBuf::from(xdg_data_home)
                    .join("pyrs")
                    .join(&stdlib_suffix),
            );
        } else if let Some(home) = env::var_os("HOME") {
            roots.push(
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("pyrs")
                    .join(&stdlib_suffix),
            );
        }
        roots.push(PathBuf::from(format!(
            ".local/Python-{CPYTHON_STDLIB_VERSION}/Lib"
        )));
        roots
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
            register_host_dynload_fallback(&mut out, &mut seen);
        }
        // When PYRS_CPYTHON_LIB is set, keep sys.path isolated to that stdlib root
        // (plus its adjacent lib-dynload if present) instead of mixing in host
        // framework stdlib paths. This avoids cross-root semantic drift in tests.
        return (out, strict_site_import);
    }

    for root_candidate in install_managed_stdlib_roots() {
        if let Some(root) = register_stdlib_root(&mut out, &mut seen, root_candidate) {
            strict_site_import = true;
            if !register_dynload_for_root(&mut out, &mut seen, &root) {
                register_host_dynload_fallback(&mut out, &mut seen);
            }
            return (out, strict_site_import);
        }
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

    for candidate in host_stdlib_roots() {
        let normalized = std::fs::canonicalize(&candidate).unwrap_or(candidate);
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
