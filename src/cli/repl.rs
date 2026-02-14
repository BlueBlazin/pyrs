use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

use reedline::{DefaultHinter, FileBackedHistory, Prompt, PromptEditMode, PromptHistorySearch};
use reedline::{PromptHistorySearchStatus, Reedline, Signal};

use crate::VERSION;
use crate::ast::{Module, StmtKind};
use crate::compiler;
use crate::parser::{self, ParseError};
use crate::runtime::{Value, format_repr};
use crate::stdlib;
use crate::vm::Vm;

const HISTORY_CAPACITY: usize = 10_000;

pub(super) fn run_repl(import_site: bool) -> Result<(), String> {
    let mut vm = Vm::new();
    super::configure_vm_for_command(&mut vm, import_site)?;
    stdlib::initialize();

    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    vm.set_sys_interactive_flag(interactive);

    let run_result = if interactive {
        run_interactive_session(&mut vm)
    } else {
        run_stdin_script(&mut vm)
    };
    let shutdown_result = vm
        .run_shutdown_hooks()
        .map_err(|err| format!("shutdown error: {}", err.message));
    run_result.and(shutdown_result)
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

fn run_interactive_session(vm: &mut Vm) -> Result<(), String> {
    println!("pyrs {VERSION} (CPython 3.14 compatible)");
    println!("Type .help for REPL commands, Ctrl-D to exit.");

    let mut line_editor = build_editor()?;
    let primary_prompt = ReplPrompt::primary();
    let continuation_prompt = ReplPrompt::continuation();

    let mut pending = String::new();
    loop {
        let prompt = if pending.is_empty() {
            &primary_prompt
        } else {
            &continuation_prompt
        };
        match line_editor.read_line(prompt) {
            Ok(Signal::Success(line)) => {
                if matches!(
                    handle_meta_command(&line, &mut pending),
                    ReplControl::ExitSession
                ) {
                    break;
                }
                if line.trim().is_empty() && pending.is_empty() {
                    continue;
                }
                pending.push_str(&line);
                pending.push('\n');

                match parser::parse_module(&pending) {
                    Ok(module) => {
                        if let Err(err) = execute_parsed_module(vm, &module, "<stdin>", true) {
                            eprintln!("{err}");
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
            }
            Ok(Signal::CtrlD) => {
                if !pending.trim().is_empty() {
                    eprintln!("KeyboardInterrupt");
                    pending.clear();
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

fn build_editor() -> Result<Reedline, String> {
    let mut editor = Reedline::create().with_hinter(Box::new(DefaultHinter::default()));

    if let Some(history_path) = resolve_history_path() {
        if let Some(parent) = history_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create history directory {}: {err}",
                    parent.display()
                )
            })?;
        }
        let history = FileBackedHistory::with_file(HISTORY_CAPACITY, history_path)
            .map_err(|err| format!("failed to initialize REPL history: {err}"))?;
        editor = editor.with_history(Box::new(history));
    }

    Ok(editor)
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
    fn render_prompt_left(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("")
    }

    fn render_prompt_right(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(self.indicator)
    }

    fn render_prompt_multiline_indicator(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(self.indicator)
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> std::borrow::Cow<'_, str> {
        match history_search.status {
            PromptHistorySearchStatus::Passing => {
                std::borrow::Cow::Owned(format!("(reverse-search: {}) ", history_search.term))
            }
            PromptHistorySearchStatus::Failing => std::borrow::Cow::Owned(format!(
                "(failing reverse-search: {}) ",
                history_search.term
            )),
        }
    }
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

#[derive(Copy, Clone, Eq, PartialEq)]
enum ReplControl {
    ContinueSession,
    ExitSession,
}

fn handle_meta_command(line: &str, pending: &mut String) -> ReplControl {
    if !pending.is_empty() {
        return ReplControl::ContinueSession;
    }

    match line.trim() {
        ".exit" | ".quit" => ReplControl::ExitSession,
        ".help" => {
            println!(".help            show REPL help");
            println!(".clear           clear pending multi-line input");
            println!(".exit / .quit    exit REPL");
            ReplControl::ContinueSession
        }
        ".clear" => {
            pending.clear();
            ReplControl::ContinueSession
        }
        _ => ReplControl::ContinueSession,
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
    use super::{format_parse_error, repl_input_is_incomplete};
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
}
