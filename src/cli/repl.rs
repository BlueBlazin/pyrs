use std::borrow::Cow;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use reedline::{
    ColumnarMenu, Completer, DefaultHinter, Emacs, FileBackedHistory, KeyCode, KeyModifiers,
    MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, ReedlineMenu, Signal, Span, Suggestion, default_emacs_keybindings,
};

use crate::VERSION;
use crate::ast::{Module, StmtKind};
use crate::compiler;
use crate::parser::{self, ParseError};
use crate::runtime::{Value, format_repr};
use crate::stdlib;
use crate::vm::Vm;

const HISTORY_CAPACITY: usize = 10_000;
const COMPLETION_MENU_NAME: &str = "pyrs_repl_completion";
const REPL_COMMANDS: &[&str] = &[
    ":help", ".help", ":clear", ".clear", ":paste", ":timing", ":reset", ":exit", ":quit", ".exit",
    ".quit",
];
const REPL_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "case", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return",
    "try", "while", "with", "yield",
];

pub(super) fn run_repl(import_site: bool) -> Result<(), String> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut vm = build_vm(import_site, interactive)?;

    let run_result = if interactive {
        run_interactive_session(&mut vm, import_site)
    } else {
        run_stdin_script(&mut vm)
    };
    let shutdown_result = vm
        .run_shutdown_hooks()
        .map_err(|err| format!("shutdown error: {}", err.message));
    run_result.and(shutdown_result)
}

fn build_vm(import_site: bool, interactive: bool) -> Result<Vm, String> {
    let mut vm = Vm::new();
    super::configure_vm_for_command(&mut vm, import_site)?;
    stdlib::initialize();
    vm.set_sys_interactive_flag(interactive);
    Ok(vm)
}

fn run_stdin_script(vm: &mut Vm) -> Result<(), String> {
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|err| format!("failed to read stdin: {err}"))?;
    if source.trim().is_empty() {
        return Ok(());
    }
    execute_module_source(vm, &source, "<stdin>", false)
}

fn run_interactive_session(vm: &mut Vm, import_site: bool) -> Result<(), String> {
    println!("RSPYTHON {VERSION} (CPython 3.14 compatible)");
    println!("Type :help for REPL commands, Ctrl-D to exit.");

    let symbols = Arc::new(Mutex::new(vm.repl_symbol_names()));
    let mut line_editor = build_editor(Arc::clone(&symbols))?;
    let primary_prompt = ReplPrompt::primary();
    let continuation_prompt = ReplPrompt::continuation();

    let mut pending = String::new();
    let mut paste_mode = false;
    let mut timing_enabled = false;
    loop {
        let prompt = if pending.is_empty() {
            &primary_prompt
        } else {
            &continuation_prompt
        };

        match line_editor.read_line(prompt) {
            Ok(Signal::Success(line)) => {
                let trimmed = line.trim();
                if paste_mode {
                    if parse_meta_command(trimmed) == Some(MetaCommand::TogglePaste) {
                        paste_mode = false;
                        if !pending.trim().is_empty() {
                            if let Err(err) = execute_with_optional_timing(
                                vm,
                                &pending,
                                "<stdin>",
                                false,
                                timing_enabled,
                            ) {
                                eprintln!("{err}");
                            }
                            refresh_completion_symbols(vm, &symbols);
                        }
                        pending.clear();
                        eprintln!("paste mode disabled");
                        continue;
                    }
                    pending.push_str(&line);
                    pending.push('\n');
                    continue;
                }

                if let Some(command) = parse_meta_command(trimmed) {
                    if apply_meta_command(
                        command,
                        vm,
                        &mut pending,
                        &mut paste_mode,
                        &mut timing_enabled,
                        import_site,
                        &symbols,
                    )? {
                        break;
                    }
                    continue;
                }

                if trimmed.is_empty() && pending.is_empty() {
                    continue;
                }

                pending.push_str(&line);
                pending.push('\n');
                match parser::parse_module(&pending) {
                    Ok(module) => {
                        if let Err(err) = execute_parsed_module_with_timing(
                            vm,
                            &module,
                            "<stdin>",
                            true,
                            timing_enabled,
                        ) {
                            eprintln!("{err}");
                        } else {
                            refresh_completion_symbols(vm, &symbols);
                        }
                        pending.clear();
                    }
                    Err(parse_err) => {
                        if repl_input_is_incomplete(&pending, &parse_err) {
                            continue;
                        }
                        eprintln!("{}", format_parse_error(&parse_err));
                        pending.clear();
                    }
                }
            }
            Ok(Signal::CtrlC) => {
                eprintln!("KeyboardInterrupt");
                pending.clear();
                paste_mode = false;
            }
            Ok(Signal::CtrlD) => {
                if paste_mode || !pending.trim().is_empty() {
                    eprintln!("KeyboardInterrupt");
                    pending.clear();
                    paste_mode = false;
                } else {
                    break;
                }
            }
            Err(err) => {
                return Err(format!("repl input error: {err}"));
            }
        }
    }

    Ok(())
}

fn build_editor(symbols: Arc<Mutex<Vec<String>>>) -> Result<Reedline, String> {
    let completion_menu = Box::new(ColumnarMenu::default().with_name(COMPLETION_MENU_NAME));
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(COMPLETION_MENU_NAME.to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let edit_mode = Box::new(Emacs::new(keybindings));
    let mut editor = Reedline::create()
        .with_hinter(Box::new(DefaultHinter::default()))
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode)
        .with_completer(Box::new(ReplCompleter::new(symbols)));

    if let Some(history_path) = resolve_history_path() {
        let history_setup = if let Some(parent) = history_path.parent() {
            fs::create_dir_all(parent).and_then(|_| {
                FileBackedHistory::with_file(HISTORY_CAPACITY, history_path)
                    .map_err(io::Error::other)
            })
        } else {
            FileBackedHistory::with_file(HISTORY_CAPACITY, history_path).map_err(io::Error::other)
        };
        match history_setup {
            Ok(history) => editor = editor.with_history(Box::new(history)),
            Err(err) => eprintln!("warning: REPL history disabled: {err}"),
        }
    }

    Ok(editor)
}

fn refresh_completion_symbols(vm: &Vm, symbols: &Arc<Mutex<Vec<String>>>) {
    if let Ok(mut guard) = symbols.lock() {
        *guard = vm.repl_symbol_names();
    }
}

#[derive(Clone)]
struct ReplPrompt {
    indicator: &'static str,
}

impl ReplPrompt {
    const fn primary() -> Self {
        Self { indicator: ">>> " }
    }

    const fn continuation() -> Self {
        Self { indicator: "... " }
    }
}

impl Prompt for ReplPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(self.indicator)
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.indicator)
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        match history_search.status {
            PromptHistorySearchStatus::Passing => {
                Cow::Owned(format!("(reverse-search: {}) ", history_search.term))
            }
            PromptHistorySearchStatus::Failing => Cow::Owned(format!(
                "(failing reverse-search: {}) ",
                history_search.term
            )),
        }
    }
}

struct ReplCompleter {
    symbols: Arc<Mutex<Vec<String>>>,
}

impl ReplCompleter {
    fn new(symbols: Arc<Mutex<Vec<String>>>) -> Self {
        Self { symbols }
    }
}

impl Completer for ReplCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let (token_start, token) = completion_fragment(line, pos);
        if token.is_empty() {
            return Vec::new();
        }
        if token.starts_with(['.', ':']) {
            return command_suggestions(token_start, pos.min(line.len()), token);
        }
        if is_path_like(token) {
            return path_suggestions(token_start, pos.min(line.len()), token);
        }
        let symbols = self
            .symbols
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        identifier_suggestions(token_start, pos.min(line.len()), token, &symbols)
    }
}

fn completion_fragment(line: &str, pos: usize) -> (usize, &str) {
    let safe_pos = pos.min(line.len());
    let bytes = line.as_bytes();
    let mut start = safe_pos;
    while start > 0 && !is_completion_delimiter(bytes[start - 1]) {
        start -= 1;
    }
    (start, &line[start..safe_pos])
}

fn is_completion_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b','
            | b';'
            | b'+'
            | b'-'
            | b'*'
            | b'%'
            | b'='
            | b'!'
            | b'|'
            | b'&'
            | b'^'
            | b'<'
            | b'>'
            | b'"'
            | b'\''
    )
}

fn command_suggestions(start: usize, end: usize, token: &str) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();
    for command in REPL_COMMANDS {
        if command.starts_with(token) {
            suggestions.push(Suggestion {
                value: (*command).to_string(),
                description: Some("repl command".to_string()),
                style: None,
                extra: None,
                span: Span::new(start, end),
                append_whitespace: false,
                match_indices: None,
            });
        }
    }
    suggestions.sort_by(|left, right| left.value.cmp(&right.value));
    suggestions.dedup_by(|left, right| left.value == right.value);
    suggestions
}

fn identifier_suggestions(
    start: usize,
    end: usize,
    token: &str,
    symbols: &[String],
) -> Vec<Suggestion> {
    let (replace_start, prefix, needle) = if let Some(index) = token.rfind('.') {
        (
            start + index + 1,
            token[..=index].to_string(),
            &token[index + 1..],
        )
    } else {
        (start, String::new(), token)
    };

    let mut candidates = symbols.to_vec();
    candidates.extend(REPL_KEYWORDS.iter().map(|value| (*value).to_string()));
    candidates.sort();
    candidates.dedup();

    let mut suggestions = Vec::new();
    for candidate in candidates {
        if candidate.starts_with(needle) {
            suggestions.push(Suggestion {
                value: format!("{prefix}{candidate}"),
                description: Some("identifier".to_string()),
                style: None,
                extra: None,
                span: Span::new(replace_start, end),
                append_whitespace: false,
                match_indices: None,
            });
        }
    }
    suggestions
}

fn is_path_like(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/")
        || token.contains('/')
}

fn path_suggestions(start: usize, end: usize, token: &str) -> Vec<Suggestion> {
    let (head, leaf_prefix) = split_path_token(token);
    let Some(search_root) = expand_path_head(head) else {
        return Vec::new();
    };

    let entries = match fs::read_dir(search_root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut suggestions = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(leaf_prefix) {
            continue;
        }
        let is_directory = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let suffix = if is_directory { "/" } else { "" };
        suggestions.push(Suggestion {
            value: format!("{head}{file_name}{suffix}"),
            description: Some(if is_directory {
                "directory".to_string()
            } else {
                "path".to_string()
            }),
            style: None,
            extra: None,
            span: Span::new(start, end),
            append_whitespace: !is_directory,
            match_indices: None,
        });
    }
    suggestions.sort_by(|left, right| left.value.cmp(&right.value));
    suggestions
}

fn split_path_token(token: &str) -> (&str, &str) {
    if let Some(index) = token.rfind('/') {
        (&token[..=index], &token[index + 1..])
    } else {
        ("", token)
    }
}

fn expand_path_head(head: &str) -> Option<PathBuf> {
    if head.is_empty() {
        return Some(Path::new(".").to_path_buf());
    }
    if head == "~/" {
        let home = env::var_os("HOME")?;
        return Some(PathBuf::from(home));
    }
    if let Some(rest) = head.strip_prefix("~/") {
        let home = env::var_os("HOME")?;
        return Some(PathBuf::from(home).join(rest));
    }
    if head.starts_with('~') {
        return None;
    }
    Some(PathBuf::from(head))
}

fn resolve_history_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("PYRS_REPL_HISTORY") {
        if path.is_empty() {
            return None;
        }
        return Some(PathBuf::from(path));
    }
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".pyrs_history"))
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum MetaCommand {
    Help,
    Clear,
    Exit,
    Reset,
    ToggleTiming,
    TogglePaste,
}

fn parse_meta_command(line: &str) -> Option<MetaCommand> {
    match line {
        ":help" | ".help" => Some(MetaCommand::Help),
        ":clear" | ".clear" => Some(MetaCommand::Clear),
        ":exit" | ":quit" | ".exit" | ".quit" => Some(MetaCommand::Exit),
        ":reset" => Some(MetaCommand::Reset),
        ":timing" => Some(MetaCommand::ToggleTiming),
        ":paste" => Some(MetaCommand::TogglePaste),
        _ => None,
    }
}

fn apply_meta_command(
    command: MetaCommand,
    vm: &mut Vm,
    pending: &mut String,
    paste_mode: &mut bool,
    timing_enabled: &mut bool,
    import_site: bool,
    symbols: &Arc<Mutex<Vec<String>>>,
) -> Result<bool, String> {
    match command {
        MetaCommand::Help => {
            println!(":help / .help     show REPL help");
            println!(":clear / .clear   clear pending input buffer");
            println!(":paste            toggle paste mode (finish with :paste)");
            println!(":timing           toggle execution timing display");
            println!(":reset            reset interpreter state");
            println!(":exit / :quit     exit REPL");
            Ok(false)
        }
        MetaCommand::Clear => {
            pending.clear();
            *paste_mode = false;
            Ok(false)
        }
        MetaCommand::Exit => Ok(true),
        MetaCommand::ToggleTiming => {
            *timing_enabled = !*timing_enabled;
            eprintln!(
                "timing {}",
                if *timing_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            Ok(false)
        }
        MetaCommand::TogglePaste => {
            *paste_mode = true;
            pending.clear();
            eprintln!("paste mode enabled (finish with :paste)");
            Ok(false)
        }
        MetaCommand::Reset => {
            if let Err(err) = vm.run_shutdown_hooks() {
                eprintln!("shutdown warning before reset: {}", err.message);
            }
            *vm = build_vm(import_site, true)?;
            pending.clear();
            *paste_mode = false;
            refresh_completion_symbols(vm, symbols);
            eprintln!("interpreter state reset");
            Ok(false)
        }
    }
}

fn execute_module_source(
    vm: &mut Vm,
    source: &str,
    filename: &str,
    echo_expression_result: bool,
) -> Result<(), String> {
    let module = parser::parse_module(source).map_err(|err| format_parse_error(&err))?;
    execute_parsed_module(vm, &module, filename, echo_expression_result)
}

fn execute_with_optional_timing(
    vm: &mut Vm,
    source: &str,
    filename: &str,
    echo_expression_result: bool,
    timing_enabled: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let result = execute_module_source(vm, source, filename, echo_expression_result);
    if timing_enabled {
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[timing] {elapsed_ms:.3} ms");
    }
    result
}

fn execute_parsed_module_with_timing(
    vm: &mut Vm,
    module: &Module,
    filename: &str,
    echo_expression_result: bool,
    timing_enabled: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let result = execute_parsed_module(vm, module, filename, echo_expression_result);
    if timing_enabled {
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[timing] {elapsed_ms:.3} ms");
    }
    result
}

fn execute_parsed_module(
    vm: &mut Vm,
    module: &Module,
    filename: &str,
    echo_expression_result: bool,
) -> Result<(), String> {
    if echo_expression_result
        && module.body.len() == 1
        && let StmtKind::Expr(expr) = &module.body[0].node
    {
        let code = compiler::compile_expression_with_filename(expr, filename)
            .map_err(|err| format!("compile error: {}", err.message))?;
        let value = vm
            .execute(&code)
            .map_err(|err| format!("runtime error: {}", err.message))?;
        if !matches!(value, Value::None) {
            println!("{}", format_repr(&value));
        }
        return Ok(());
    }

    let code = compiler::compile_module_with_filename(module, filename)
        .map_err(|err| format!("compile error: {}", err.message))?;
    vm.execute(&code)
        .map_err(|err| format!("runtime error: {}", err.message))?;
    Ok(())
}

fn format_parse_error(err: &ParseError) -> String {
    format!(
        "parse error at {} (line {}, column {}): {}",
        err.offset, err.line, err.column, err.message
    )
}

fn repl_input_is_incomplete(source: &str, err: &ParseError) -> bool {
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
    let mut lexer = parser::lexer::Lexer::new(source);
    let Ok(tokens) = lexer.tokenize() else {
        return false;
    };

    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    for token in tokens {
        match token.kind {
            parser::token::TokenKind::LParen => paren_depth += 1,
            parser::token::TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
            parser::token::TokenKind::LBracket => bracket_depth += 1,
            parser::token::TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
            parser::token::TokenKind::LBrace => brace_depth += 1,
            parser::token::TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
    }
    paren_depth != 0 || bracket_depth != 0 || brace_depth != 0
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use reedline::Completer;

    use super::{
        MetaCommand, ReplCompleter, completion_fragment, format_parse_error, is_path_like,
        parse_meta_command, repl_input_is_incomplete,
    };
    use crate::parser;

    #[test]
    fn repl_marks_colon_blocks_as_incomplete() {
        let source = "if True:\n";
        let err = parser::parse_module(source).expect_err("parse should fail while incomplete");
        assert!(repl_input_is_incomplete(source, &err));
    }

    #[test]
    fn repl_marks_unclosed_delimiter_as_incomplete() {
        let source = "print((1 + 2\n";
        let err = parser::parse_module(source).expect_err("parse should fail while incomplete");
        assert!(repl_input_is_incomplete(source, &err));
    }

    #[test]
    fn repl_treats_real_syntax_error_as_complete() {
        let source = "if True print(1)\n";
        let err = parser::parse_module(source).expect_err("parse should fail");
        assert!(!repl_input_is_incomplete(source, &err));
    }

    #[test]
    fn parse_error_is_human_readable() {
        let source = "def f(\n";
        let err = parser::parse_module(source).expect_err("parse should fail");
        let rendered = format_parse_error(&err);
        assert!(rendered.contains("parse error at"));
        assert!(rendered.contains("line"));
        assert!(rendered.contains("column"));
    }

    #[test]
    fn parses_meta_commands() {
        assert_eq!(
            parse_meta_command(":timing"),
            Some(MetaCommand::ToggleTiming)
        );
        assert_eq!(parse_meta_command(".help"), Some(MetaCommand::Help));
        assert_eq!(parse_meta_command(":reset"), Some(MetaCommand::Reset));
        assert_eq!(parse_meta_command("print(1)"), None);
    }

    #[test]
    fn completion_fragment_extracts_last_token() {
        let (start, fragment) = completion_fragment("print(math.sq", "print(math.sq".len());
        assert_eq!(fragment, "math.sq");
        assert_eq!(start, 6);
    }

    #[test]
    fn path_like_detection_matches_core_patterns() {
        assert!(is_path_like("./foo"));
        assert!(is_path_like("../foo"));
        assert!(is_path_like("/tmp/foo"));
        assert!(is_path_like("pkg/module.py"));
        assert!(!is_path_like("identifier"));
    }

    #[test]
    fn completer_suggests_identifiers_and_commands() {
        let symbols = Arc::new(Mutex::new(vec![
            "statistics".to_string(),
            "str".to_string(),
            "sum".to_string(),
        ]));
        let mut completer = ReplCompleter::new(symbols);

        let suggestions = completer.complete("st", 2);
        let values: Vec<String> = suggestions.into_iter().map(|item| item.value).collect();
        assert!(values.iter().any(|value| value == "str"));
        assert!(values.iter().any(|value| value == "statistics"));

        let command_suggestions = completer.complete(":ti", 3);
        let command_values: Vec<String> = command_suggestions
            .into_iter()
            .map(|item| item.value)
            .collect();
        assert!(command_values.iter().any(|value| value == ":timing"));
    }
}
