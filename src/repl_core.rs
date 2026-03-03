//! Shared REPL parse/incomplete-input semantics used by native and wasm adapters.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use crate::ast::Module;
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

/// Structured parse outcome for a single REPL line submission.
pub(crate) enum ReplLineParseResult {
    NeedMoreInput,
    Ready { source: String, module: Module },
    ParseError { source: String, error: ParseError },
}

pub(crate) enum ReplLinePrepareResult {
    NeedMoreInput,
    ParseError {
        source: String,
        error: ParseError,
    },
    CompileError {
        source: String,
        error: crate::compiler::CompileError,
    },
    Ready {
        source: String,
        module: Module,
        code: crate::bytecode::CodeObject,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplPromptKind {
    Primary,
    Continuation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplProfile {
    NativeFull,
    #[cfg(any(target_arch = "wasm32", feature = "wasm-vm-probe", test))]
    WasmLean,
}

/// Shared pending-input state for interactive REPL adapters.
#[derive(Default)]
pub(crate) struct ReplInputSession {
    pending: String,
}

impl ReplInputSession {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(crate) fn has_pending_nonempty(&self) -> bool {
        !self.pending.trim().is_empty()
    }

    pub(crate) fn pending_source(&self) -> &str {
        &self.pending
    }

    pub(crate) fn prompt_kind(&self) -> ReplPromptKind {
        if self.is_empty() {
            ReplPromptKind::Primary
        } else {
            ReplPromptKind::Continuation
        }
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }

    pub(crate) fn reset(&mut self) {
        self.clear();
    }

    pub(crate) fn interrupt(&mut self) {
        self.clear();
    }

    pub(crate) fn append_paste_line(&mut self, line: &str) {
        self.pending.push_str(line);
        self.pending.push('\n');
    }

    pub(crate) fn submit_line(&mut self, line: &str) -> ReplLineParseResult {
        submit_line_for_module(&mut self.pending, line)
    }
}

/// Shared core state holder for REPL adapters with explicit behavior profile.
pub(crate) struct ReplCoreState {
    profile: ReplProfile,
    input: ReplInputSession,
}

impl ReplCoreState {
    pub(crate) fn new(profile: ReplProfile) -> Self {
        Self {
            profile,
            input: ReplInputSession::new(),
        }
    }

    pub(crate) fn profile(&self) -> ReplProfile {
        self.profile
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub(crate) fn has_pending_nonempty(&self) -> bool {
        self.input.has_pending_nonempty()
    }

    pub(crate) fn pending_source(&self) -> &str {
        self.input.pending_source()
    }

    pub(crate) fn prompt_kind(&self) -> ReplPromptKind {
        self.input.prompt_kind()
    }

    pub(crate) fn clear(&mut self) {
        self.input.clear();
    }

    pub(crate) fn reset(&mut self) {
        self.input.reset();
    }

    pub(crate) fn interrupt(&mut self) {
        self.input.interrupt();
    }

    pub(crate) fn append_paste_line(&mut self, line: &str) {
        self.input.append_paste_line(line);
    }

    pub(crate) fn submit_line(&mut self, line: &str) -> ReplLineParseResult {
        self.input.submit_line(line)
    }

    pub(crate) fn submit_line_prepare_module(
        &mut self,
        line: &str,
        filename: &str,
    ) -> ReplLinePrepareResult {
        match self.submit_line(line) {
            ReplLineParseResult::NeedMoreInput => ReplLinePrepareResult::NeedMoreInput,
            ReplLineParseResult::ParseError { source, error } => {
                ReplLinePrepareResult::ParseError { source, error }
            }
            ReplLineParseResult::Ready { source, module } => {
                match crate::compiler::compile_module_with_filename(&module, filename) {
                    Ok(code) => ReplLinePrepareResult::Ready {
                        source,
                        module,
                        code,
                    },
                    Err(error) => ReplLinePrepareResult::CompileError { source, error },
                }
            }
        }
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    pub(crate) fn submit_line_and_execute(
        &mut self,
        vm: &mut crate::vm::Vm,
        line: &str,
        filename: &str,
    ) -> ReplLineExecuteResult {
        match self.submit_line_prepare_module(line, filename) {
            ReplLinePrepareResult::NeedMoreInput => ReplLineExecuteResult::NeedMoreInput,
            ReplLinePrepareResult::ParseError { source, error } => {
                ReplLineExecuteResult::ParseError { source, error }
            }
            ReplLinePrepareResult::CompileError { source, error } => {
                ReplLineExecuteResult::ExecutionError {
                    source,
                    error: ReplExecutionError::Compile(error),
                }
            }
            ReplLinePrepareResult::Ready {
                source,
                module,
                code,
            } => {
                match run_ready_module(vm, &source, &module, filename, Some(&code)) {
                    Ok(display) => ReplLineExecuteResult::Executed {
                        module,
                        display,
                    },
                    Err(error) => ReplLineExecuteResult::ExecutionError {
                        source,
                        error,
                    },
                }
            }
        }
    }
}

/// Appends one user-entered line to the pending REPL buffer and tries to parse.
///
/// Behavior mirrors CPython-style interactive continuation semantics:
/// - keep collecting input when parse succeeds with EOF-implied dedent,
/// - keep collecting input for parse errors classified as incomplete,
/// - clear pending buffer and return parse/ready results otherwise.
pub(crate) fn submit_line_for_module(pending: &mut String, line: &str) -> ReplLineParseResult {
    pending.push_str(line);
    pending.push('\n');

    let parse_source = parse_candidate_source(pending);
    match crate::parser::parse_module(parse_source) {
        Ok(module) => {
            if parse_success_requires_more_input(parse_source, line) {
                ReplLineParseResult::NeedMoreInput
            } else {
                let source = parse_source.to_string();
                pending.clear();
                ReplLineParseResult::Ready { source, module }
            }
        }
        Err(error) => {
            if input_is_incomplete(parse_source, &error) {
                ReplLineParseResult::NeedMoreInput
            } else {
                let source = parse_source.to_string();
                pending.clear();
                ReplLineParseResult::ParseError { source, error }
            }
        }
    }
}

/// Unified module/expression execution path used by REPL adapters.
///
/// Returns `Ok(Some(repr))` only when executing a single expression with a non-`None` value.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
pub(crate) fn execute_module_or_expression(
    vm: &mut crate::vm::Vm,
    module: &Module,
    filename: &str,
    precompiled_module_code: Option<&crate::bytecode::CodeObject>,
) -> Result<Option<String>, ReplExecutionError> {
    if module.body.len() == 1
        && let crate::ast::StmtKind::Expr(expr) = &module.body[0].node
    {
        let code = crate::compiler::compile_expression_with_filename(expr, filename)
            .map_err(ReplExecutionError::Compile)?;
        let value = vm.execute(&code).map_err(ReplExecutionError::Runtime)?;
        if matches!(value, crate::runtime::Value::None) {
            return Ok(None);
        }
        let rendered = vm
            .render_value_repr_for_display(value)
            .map_err(ReplExecutionError::Runtime)?;
        return Ok(Some(rendered));
    }

    let maybe_compiled;
    let code = if let Some(code) = precompiled_module_code {
        code
    } else {
        maybe_compiled = crate::compiler::compile_module_with_filename(module, filename)
            .map_err(ReplExecutionError::Compile)?;
        &maybe_compiled
    };
    vm.execute(&code).map_err(ReplExecutionError::Runtime)?;
    Ok(None)
}

/// Executes a parse-ready module through shared REPL semantics after caching source text.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
pub(crate) fn run_ready_module(
    vm: &mut crate::vm::Vm,
    source: &str,
    module: &Module,
    filename: &str,
    precompiled_module_code: Option<&crate::bytecode::CodeObject>,
) -> Result<Option<String>, ReplExecutionError> {
    vm.cache_source_text(filename, source);
    execute_module_or_expression(vm, module, filename, precompiled_module_code)
}

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
pub(crate) enum ReplLineExecuteResult {
    NeedMoreInput,
    ParseError {
        source: String,
        error: ParseError,
    },
    Executed {
        module: Module,
        display: Option<String>,
    },
    ExecutionError {
        source: String,
        error: ReplExecutionError,
    },
}

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
#[derive(Debug)]
pub(crate) enum ReplExecutionError {
    Compile(crate::compiler::CompileError),
    Runtime(crate::runtime::RuntimeError),
}

#[cfg(test)]
mod tests {
    use super::{
        ReplCoreState, ReplInputSession, ReplLineParseResult, ReplProfile, ReplPromptKind,
        input_is_incomplete,
        parse_candidate_source, parse_success_requires_more_input, submit_line_for_module,
    };
    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    use super::{
        ReplLineExecuteResult, ReplLinePrepareResult, execute_module_or_expression,
        run_ready_module,
    };

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

    #[test]
    fn submit_line_transitions_from_incomplete_to_ready() {
        let mut pending = String::new();
        assert!(matches!(
            submit_line_for_module(&mut pending, "class A:"),
            ReplLineParseResult::NeedMoreInput
        ));
        assert!(matches!(
            submit_line_for_module(&mut pending, "    x = 1"),
            ReplLineParseResult::NeedMoreInput
        ));
        assert!(matches!(
            submit_line_for_module(&mut pending, ""),
            ReplLineParseResult::Ready { .. }
        ));
        assert!(pending.is_empty());
    }

    #[test]
    fn submit_line_clears_pending_on_non_incomplete_parse_error() {
        let mut pending = String::new();
        let result = submit_line_for_module(&mut pending, "if True print(1)");
        assert!(matches!(result, ReplLineParseResult::ParseError { .. }));
        assert!(pending.is_empty());
    }

    #[test]
    fn input_session_tracks_pending_and_interrupt_reset_paths() {
        let mut session = ReplInputSession::new();
        assert!(session.is_empty());
        assert!(!session.has_pending_nonempty());
        assert_eq!(session.prompt_kind(), ReplPromptKind::Primary);

        session.append_paste_line("class User:");
        assert!(!session.is_empty());
        assert!(session.has_pending_nonempty());
        assert_eq!(session.prompt_kind(), ReplPromptKind::Continuation);

        session.interrupt();
        assert!(session.is_empty());
        assert!(!session.has_pending_nonempty());
        assert_eq!(session.prompt_kind(), ReplPromptKind::Primary);

        session.append_paste_line("x = 1");
        assert!(session.has_pending_nonempty());
        session.reset();
        assert!(session.is_empty());
    }

    #[test]
    fn core_state_exposes_profile_and_input_session_ops() {
        let mut state = ReplCoreState::new(ReplProfile::NativeFull);
        assert_eq!(state.profile(), ReplProfile::NativeFull);
        assert!(state.is_empty());
        assert_eq!(state.prompt_kind(), ReplPromptKind::Primary);

        state.append_paste_line("x = 1");
        assert!(state.has_pending_nonempty());
        assert_eq!(state.prompt_kind(), ReplPromptKind::Continuation);
        state.clear();
        assert!(state.is_empty());
        assert_eq!(state.prompt_kind(), ReplPromptKind::Primary);

        let lean = ReplCoreState::new(ReplProfile::WasmLean);
        assert_eq!(lean.profile(), ReplProfile::WasmLean);
    }

    #[test]
    fn line_submit_semantics_match_across_profiles() {
        for profile in [ReplProfile::NativeFull, ReplProfile::WasmLean] {
            let mut state = ReplCoreState::new(profile);
            assert!(matches!(
                state.submit_line("class A:"),
                ReplLineParseResult::NeedMoreInput
            ));
            assert!(matches!(
                state.submit_line("    x = 1"),
                ReplLineParseResult::NeedMoreInput
            ));
            assert!(matches!(
                state.submit_line(""),
                ReplLineParseResult::Ready { .. }
            ));
            assert!(state.is_empty());
        }
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn execute_module_or_expression_echoes_expression_repr() {
        let mut vm = crate::vm::Vm::new();
        let module = crate::parser::parse_module("1 + 1").expect("module should parse");
        let rendered = execute_module_or_expression(&mut vm, &module, "<stdin>", None)
            .expect("expression execution should succeed");
        assert_eq!(rendered.as_deref(), Some("2"));
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn execute_module_or_expression_returns_none_for_statements() {
        let mut vm = crate::vm::Vm::new();
        let module = crate::parser::parse_module("x = 1").expect("module should parse");
        let rendered = execute_module_or_expression(&mut vm, &module, "<stdin>", None)
            .expect("statement execution should succeed");
        assert!(rendered.is_none());
        assert!(matches!(vm.get_global("x"), Some(crate::runtime::Value::Int(1))));
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn execute_module_or_expression_supports_precompiled_module_code() {
        let mut vm = crate::vm::Vm::new();
        let module = crate::parser::parse_module("x = 5").expect("module should parse");
        let code =
            crate::compiler::compile_module_with_filename(&module, "<stdin>").expect("compile");
        let rendered = execute_module_or_expression(&mut vm, &module, "<stdin>", Some(&code))
            .expect("statement execution should succeed");
        assert!(rendered.is_none());
        assert!(matches!(vm.get_global("x"), Some(crate::runtime::Value::Int(5))));
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn execute_module_or_expression_expression_path_ignores_precompiled_module_code_for_echo() {
        let mut vm = crate::vm::Vm::new();
        let module = crate::parser::parse_module("1 + 2").expect("module should parse");
        let module_code =
            crate::compiler::compile_module_with_filename(&module, "<stdin>").expect("compile");
        let rendered = execute_module_or_expression(&mut vm, &module, "<stdin>", Some(&module_code))
            .expect("expression execution should succeed");
        assert_eq!(rendered.as_deref(), Some("3"));
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn run_ready_module_executes_statement_payload() {
        let mut vm = crate::vm::Vm::new();
        let source = "x = 9\n";
        let module = crate::parser::parse_module(source).expect("module should parse");
        let rendered = run_ready_module(&mut vm, source, &module, "<stdin>", None)
            .expect("execution should succeed");
        assert!(rendered.is_none());
        assert!(matches!(vm.get_global("x"), Some(crate::runtime::Value::Int(9))));
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn submit_line_and_execute_runs_ready_input_and_returns_display() {
        let mut vm = crate::vm::Vm::new();
        let mut state = ReplCoreState::new(ReplProfile::NativeFull);
        let result = state.submit_line_and_execute(&mut vm, "1 + 4", "<stdin>");
        if let ReplLineExecuteResult::Executed { display, .. } = result {
            assert_eq!(display.as_deref(), Some("5"));
        } else {
            panic!("expected executed result");
        }
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn submit_line_and_execute_preserves_need_more_input_state() {
        let mut vm = crate::vm::Vm::new();
        let mut state = ReplCoreState::new(ReplProfile::WasmLean);
        assert!(matches!(
            state.submit_line_and_execute(&mut vm, "class A:", "<stdin>"),
            ReplLineExecuteResult::NeedMoreInput
        ));
        assert!(!state.is_empty());
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-vm-probe"))]
    #[test]
    fn submit_line_prepare_module_reports_compile_errors_with_source() {
        let mut state = ReplCoreState::new(ReplProfile::NativeFull);
        let result = state.submit_line_prepare_module("return 1", "<stdin>");
        match result {
            ReplLinePrepareResult::CompileError { source, .. } => {
                assert_eq!(source, "return 1".to_string());
            }
            _ => panic!("expected compile error result"),
        }
        assert!(state.is_empty());
    }
}
