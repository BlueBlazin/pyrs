use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nu_ansi_term::{Color as AnsiColor, Style};
use reedline::{
    ColumnarMenu, Completer, DefaultHinter, Emacs, FileBackedHistory, History, KeyCode,
    KeyModifiers, MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineEvent, ReedlineMenu, Signal, Span, StyledText,
    Suggestion, default_emacs_keybindings,
};
use reedline::{EditCommand, Highlighter, Hinter};

use super::{error_style, format_compile_error, format_syntax_error};
use crate::VERSION;
use crate::ast::{AssignTarget, ImportAlias, Module, StmtKind};
use crate::compiler;
use crate::parser::{self, ParseError};
use crate::runtime::{Object, Value};
use crate::vm::Vm;

const HISTORY_CAPACITY: usize = 10_000;
const INDENT_WIDTH: usize = 4;
const COMPLETION_MENU_NAME: &str = "pyrs_repl_completion";
const REPL_ESC_DISMISS_HINT_COMMAND: &str = "__pyrs_repl_dismiss_hint__";
const REPL_THEME_ENV: &str = "PYRS_REPL_THEME";
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum ReplThemeMode {
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum ResolvedReplTheme {
    Dark,
    Light,
}

#[derive(Debug, Copy, Clone)]
struct ReplPalette {
    keyword_style: Style,
    class_name_style: Style,
    function_name_style: Style,
    decorator_style: Style,
    type_name_style: Style,
    number_style: Style,
    string_style: Style,
    comment_style: Style,
    hint_style: Style,
}

fn parse_repl_theme_mode(value: &str) -> Option<ReplThemeMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ReplThemeMode::Auto),
        "dark" => Some(ReplThemeMode::Dark),
        "light" => Some(ReplThemeMode::Light),
        _ => None,
    }
}

fn parse_colorfgbg_background_code(value: &str) -> Option<u8> {
    value.rsplit(';').next()?.trim().parse::<u8>().ok()
}

fn is_light_background_code(code: u8) -> bool {
    matches!(code, 7 | 9 | 10 | 11 | 12 | 13 | 14 | 15)
}

fn resolve_repl_theme(mode: ReplThemeMode, colorfgbg: Option<&str>) -> ResolvedReplTheme {
    match mode {
        ReplThemeMode::Dark => ResolvedReplTheme::Dark,
        ReplThemeMode::Light => ResolvedReplTheme::Light,
        ReplThemeMode::Auto => {
            if let Some(code) = colorfgbg.and_then(parse_colorfgbg_background_code) {
                if is_light_background_code(code) {
                    ResolvedReplTheme::Light
                } else {
                    ResolvedReplTheme::Dark
                }
            } else {
                ResolvedReplTheme::Dark
            }
        }
    }
}

fn resolve_repl_theme_from_env() -> ReplThemeMode {
    let Some(raw_value) = env::var(REPL_THEME_ENV).ok() else {
        return ReplThemeMode::Auto;
    };
    let Some(parsed) = parse_repl_theme_mode(&raw_value) else {
        eprintln!(
            "warning: invalid {REPL_THEME_ENV} value '{}'; expected auto|dark|light",
            raw_value
        );
        return ReplThemeMode::Auto;
    };
    parsed
}

fn repl_palette(theme: ResolvedReplTheme) -> ReplPalette {
    match theme {
        ResolvedReplTheme::Dark => ReplPalette {
            keyword_style: Style::new().fg(AnsiColor::Rgb(46, 149, 211)).bold(),
            class_name_style: Style::new().fg(AnsiColor::Fixed(203)).bold(),
            function_name_style: Style::new().fg(AnsiColor::Fixed(214)).bold(),
            decorator_style: Style::new().fg(AnsiColor::Fixed(111)),
            type_name_style: Style::new().fg(AnsiColor::Fixed(220)),
            number_style: Style::new().fg(AnsiColor::Fixed(204)),
            string_style: Style::new().fg(AnsiColor::Fixed(42)),
            comment_style: Style::new().italic().fg(AnsiColor::Fixed(244)),
            hint_style: Style::new().italic().fg(AnsiColor::LightGray),
        },
        ResolvedReplTheme::Light => ReplPalette {
            keyword_style: Style::new().fg(AnsiColor::Fixed(24)).bold(),
            class_name_style: Style::new().fg(AnsiColor::Fixed(160)).bold(),
            function_name_style: Style::new().fg(AnsiColor::Fixed(166)).bold(),
            decorator_style: Style::new().fg(AnsiColor::Fixed(61)),
            type_name_style: Style::new().fg(AnsiColor::Fixed(130)),
            number_style: Style::new().fg(AnsiColor::Fixed(161)),
            string_style: Style::new().fg(AnsiColor::Fixed(22)),
            comment_style: Style::new().italic().fg(AnsiColor::Fixed(102)),
            hint_style: Style::new().italic().fg(AnsiColor::DarkGray),
        },
    }
}

pub(super) fn run_repl(
    import_site: bool,
    warnoptions: Vec<String>,
    startup_tracemalloc_limit: Option<usize>,
) -> Result<i32, String> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut vm = build_vm_with_warnoptions(
        import_site,
        interactive,
        &warnoptions,
        startup_tracemalloc_limit,
    )?;

    let run_result = if interactive {
        run_interactive_session(&mut vm, import_site, &warnoptions)
    } else {
        run_stdin_script(&mut vm)
    };
    let shutdown_result = vm
        .run_shutdown_hooks()
        .map_err(|err| format!("shutdown error: {}", err.message));
    shutdown_result?;
    run_result
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum CompletionRefreshPlan {
    None,
    Full,
    Symbols(Vec<String>),
}

fn repl_module_completion_plan(module: &Module) -> CompletionRefreshPlan {
    if module.body.len() != 1 {
        return CompletionRefreshPlan::Full;
    }
    let stmt = &module.body[0].node;
    match stmt {
        StmtKind::Expr(_) => CompletionRefreshPlan::None,
        StmtKind::Assign { targets, .. } => {
            let symbols = assignment_target_symbols(targets);
            if symbols.is_empty() {
                CompletionRefreshPlan::None
            } else {
                CompletionRefreshPlan::Symbols(symbols)
            }
        }
        StmtKind::AnnAssign { target, value, .. } => {
            if value.is_none() {
                return CompletionRefreshPlan::None;
            }
            let mut symbols = Vec::new();
            collect_assignment_target_symbols(target, &mut symbols);
            normalize_completion_symbols(symbols).map_or(CompletionRefreshPlan::None, |names| {
                CompletionRefreshPlan::Symbols(names)
            })
        }
        StmtKind::AugAssign { target, .. } => {
            let mut symbols = Vec::new();
            collect_assignment_target_symbols(target, &mut symbols);
            normalize_completion_symbols(symbols).map_or(CompletionRefreshPlan::None, |names| {
                CompletionRefreshPlan::Symbols(names)
            })
        }
        StmtKind::Delete { targets } => {
            let symbols = assignment_target_symbols(targets);
            if symbols.is_empty() {
                CompletionRefreshPlan::None
            } else {
                CompletionRefreshPlan::Symbols(symbols)
            }
        }
        StmtKind::Import { names } => import_stmt_completion_symbols(names, false),
        StmtKind::ImportFrom { names, .. } => import_stmt_completion_symbols(names, true),
        _ => CompletionRefreshPlan::Full,
    }
}

fn import_stmt_completion_symbols(
    names: &[ImportAlias],
    from_import: bool,
) -> CompletionRefreshPlan {
    let mut symbols = Vec::new();
    for alias in names {
        if from_import && alias.name == "*" {
            return CompletionRefreshPlan::Full;
        }
        let symbol = if let Some(asname) = &alias.asname {
            asname.clone()
        } else if from_import {
            alias.name.clone()
        } else {
            alias
                .name
                .split('.')
                .next()
                .unwrap_or(alias.name.as_str())
                .to_string()
        };
        symbols.push(symbol);
    }
    normalize_completion_symbols(symbols).map_or(CompletionRefreshPlan::None, |names| {
        CompletionRefreshPlan::Symbols(names)
    })
}

fn assignment_target_symbols(targets: &[AssignTarget]) -> Vec<String> {
    let mut symbols = Vec::new();
    for target in targets {
        collect_assignment_target_symbols(target, &mut symbols);
    }
    normalize_completion_symbols(symbols).unwrap_or_default()
}

fn collect_assignment_target_symbols(target: &AssignTarget, symbols: &mut Vec<String>) {
    match target {
        AssignTarget::Name(name) => symbols.push(name.clone()),
        AssignTarget::Starred(inner) => collect_assignment_target_symbols(inner, symbols),
        AssignTarget::Tuple(items) | AssignTarget::List(items) => {
            for item in items {
                collect_assignment_target_symbols(item, symbols);
            }
        }
        AssignTarget::Subscript { .. } | AssignTarget::Attribute { .. } => {}
    }
}

fn normalize_completion_symbols(mut symbols: Vec<String>) -> Option<Vec<String>> {
    symbols.retain(|name| is_valid_identifier_name(name));
    if symbols.is_empty() {
        return None;
    }
    symbols.sort();
    symbols.dedup();
    Some(symbols)
}

#[cfg(test)]
fn build_vm(import_site: bool, interactive: bool) -> Result<Vm, String> {
    build_vm_with_warnoptions(import_site, interactive, &[], None)
}

fn build_vm_with_warnoptions(
    import_site: bool,
    interactive: bool,
    warnoptions: &[String],
    startup_tracemalloc_limit: Option<usize>,
) -> Result<Vm, String> {
    let mut vm = Vm::new();
    super::configure_vm_for_command(&mut vm, import_site, true, warnoptions)?;
    if let Some(limit) = startup_tracemalloc_limit {
        vm.start_tracemalloc(limit);
    }
    vm.set_sys_interactive_flag(interactive);
    vm.set_sys_argv(vec![String::new()]);
    Ok(vm)
}

fn parse_system_exit_status(error_text: &str) -> Option<(i32, Option<String>)> {
    let marker = error_text.lines().rev().map(str::trim).find(|line| {
        line.starts_with("SystemExit") || line.starts_with("runtime error: SystemExit")
    })?;
    if marker == "SystemExit" || marker == "runtime error: SystemExit" {
        return Some((0, None));
    }
    let payload = marker
        .strip_prefix("SystemExit:")
        .or_else(|| marker.strip_prefix("runtime error: SystemExit:"))?
        .trim();
    if payload == "None" {
        return Some((0, None));
    }
    if let Ok(code) = payload.parse::<i32>() {
        return Some((code, None));
    }
    Some((1, Some(payload.to_string())))
}

fn run_stdin_script(vm: &mut Vm) -> Result<i32, String> {
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|err| format!("failed to read stdin: {err}"))?;
    if source.trim().is_empty() {
        return Ok(0);
    }
    match execute_module_source(vm, &source, "<stdin>", false) {
        Ok(()) => Ok(0),
        Err(err) => {
            if let Some((status, message)) = parse_system_exit_status(&err) {
                if let Some(message) = message {
                    eprintln!("{message}");
                }
                Ok(status)
            } else {
                Err(err)
            }
        }
    }
}

fn run_interactive_session(
    vm: &mut Vm,
    import_site: bool,
    warnoptions: &[String],
) -> Result<i32, String> {
    println!("PYRS {VERSION} (CPython 3.14 compatible)");
    println!("Type :help for REPL commands, Ctrl-D to exit.");

    let theme_mode = resolve_repl_theme_from_env();
    let colorfgbg = env::var("COLORFGBG").ok();
    let resolved_theme = resolve_repl_theme(theme_mode, colorfgbg.as_deref());
    let palette = repl_palette(resolved_theme);

    let completion_state = Arc::new(Mutex::new(build_completion_state(vm)));
    let hint_control = Arc::new(Mutex::new(HintControl::default()));
    load_repl_startup_script(vm, &completion_state)?;
    let mut line_editor = build_editor(
        Arc::clone(&completion_state),
        Arc::clone(&hint_control),
        palette,
    )?;
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
                if line == REPL_ESC_DISMISS_HINT_COMMAND {
                    dismiss_current_hint(&hint_control);
                    continue;
                }
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
                            refresh_completion_state(vm, &completion_state);
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
                        warnoptions,
                        &completion_state,
                    )? {
                        break;
                    }
                    continue;
                }

                if pending.trim().is_empty()
                    && let Some(command) = parse_magic_command(trimmed)
                {
                    let result = match command {
                        ReplMagicCommand::Time(source) => {
                            execute_with_optional_timing(vm, &source, "<stdin>", true, true)
                        }
                        ReplMagicCommand::TimeIt(request) => {
                            execute_timeit_command(vm, &request, "<stdin>")
                        }
                    };
                    if let Err(err) = result {
                        if let Some((status, message)) = parse_system_exit_status(&err) {
                            if let Some(message) = message {
                                eprintln!("{message}");
                            }
                            return Ok(status);
                        }
                        eprintln!("{}", error_style::format_error_for_stderr(&err));
                    } else {
                        refresh_completion_state(vm, &completion_state);
                    }
                    continue;
                }

                if trimmed.is_empty() && pending.is_empty() {
                    continue;
                }

                pending.push_str(&line);
                pending.push('\n');
                let parse_source = repl_parse_candidate_source(&pending);
                match parser::parse_module(parse_source) {
                    Ok(module) => {
                        if repl_parse_success_requires_more_input(parse_source, &line) {
                            continue;
                        }
                        let completion_plan = repl_module_completion_plan(&module);
                        vm.cache_source_text("<stdin>", parse_source);
                        if let Err(err) = execute_parsed_module_with_timing(
                            vm,
                            &module,
                            parse_source,
                            "<stdin>",
                            true,
                            timing_enabled,
                        ) {
                            if let Some((status, message)) = parse_system_exit_status(&err) {
                                if let Some(message) = message {
                                    eprintln!("{message}");
                                }
                                return Ok(status);
                            }
                            eprintln!("{}", error_style::format_error_for_stderr(&err));
                        } else {
                            apply_completion_refresh_plan(vm, &completion_state, completion_plan);
                        }
                        pending.clear();
                    }
                    Err(parse_err) => {
                        if repl_input_is_incomplete(parse_source, &parse_err) {
                            continue;
                        }
                        eprintln!(
                            "{}",
                            error_style::format_error_for_stderr(&format_parse_error(
                                parse_source, "<stdin>", &parse_err,
                            ))
                        );
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

    Ok(0)
}

#[derive(Default)]
struct HintControl {
    suppress_until_edit: bool,
    last_line: String,
}

fn dismiss_current_hint(control: &Arc<Mutex<HintControl>>) {
    if let Ok(mut guard) = control.lock() {
        guard.suppress_until_edit = true;
    }
}

struct ReplHinter {
    inner: DefaultHinter,
    control: Arc<Mutex<HintControl>>,
}

impl ReplHinter {
    fn new(control: Arc<Mutex<HintControl>>, style: Style) -> Self {
        Self {
            inner: DefaultHinter::default().with_style(style),
            control,
        }
    }

    fn should_suppress_hint(&self, line: &str) -> bool {
        let Ok(mut guard) = self.control.lock() else {
            return false;
        };
        if guard.suppress_until_edit {
            if guard.last_line == line {
                return true;
            }
            guard.suppress_until_edit = false;
        }
        guard.last_line.clear();
        guard.last_line.push_str(line);
        false
    }
}

impl Hinter for ReplHinter {
    fn handle(
        &mut self,
        line: &str,
        pos: usize,
        history: &dyn History,
        use_ansi_coloring: bool,
        cwd: &str,
    ) -> String {
        if self.should_suppress_hint(line) {
            return String::new();
        }
        self.inner
            .handle(line, pos, history, use_ansi_coloring, cwd)
    }

    fn complete_hint(&self) -> String {
        self.inner.complete_hint()
    }

    fn next_hint_token(&self) -> String {
        self.inner.next_hint_token()
    }
}

fn build_editor(
    completion_state: Arc<Mutex<CompletionState>>,
    hint_control: Arc<Mutex<HintControl>>,
    palette: ReplPalette,
) -> Result<Reedline, String> {
    let completion_menu = Box::new(ColumnarMenu::default().with_name(COMPLETION_MENU_NAME));
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::Edit(vec![EditCommand::InsertString(" ".repeat(INDENT_WIDTH))]),
    );
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::BackTab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(COMPLETION_MENU_NAME.to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char(' '),
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(COMPLETION_MENU_NAME.to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Esc,
        ReedlineEvent::Multiple(vec![
            ReedlineEvent::Esc,
            ReedlineEvent::ExecuteHostCommand(REPL_ESC_DISMISS_HINT_COMMAND.to_string()),
        ]),
    );
    let edit_mode = Box::new(Emacs::new(keybindings));
    let mut editor = Reedline::create()
        .with_hinter(Box::new(ReplHinter::new(hint_control, palette.hint_style)))
        .with_highlighter(Box::new(PythonHighlighter::new(palette)))
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode)
        .with_completer(Box::new(ReplCompleter::new(completion_state)));

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

#[derive(Clone, Default)]
struct CompletionState {
    symbols: Vec<String>,
    members: HashMap<String, Vec<String>>,
}

fn refresh_completion_state(vm: &Vm, state: &Arc<Mutex<CompletionState>>) {
    if let Ok(mut guard) = state.lock() {
        *guard = build_completion_state(vm);
    }
}

fn refresh_completion_symbols(vm: &Vm, state: &Arc<Mutex<CompletionState>>, symbols: &[String]) {
    const MAX_COMPLETION_DEPTH: usize = 2;
    if symbols.is_empty() {
        return;
    }
    if let Ok(mut guard) = state.lock() {
        for symbol in symbols {
            guard.symbols.retain(|name| name != symbol);
            guard.members.retain(|path, _| {
                !(path == symbol
                    || path
                        .strip_prefix(symbol)
                        .is_some_and(|suffix| suffix.starts_with('.')))
            });
            if let Some(value) = vm.get_global(symbol) {
                guard.symbols.push(symbol.clone());
                let mut visited = HashSet::new();
                collect_completion_members(
                    &mut guard,
                    symbol,
                    &value,
                    0,
                    MAX_COMPLETION_DEPTH,
                    &mut visited,
                );
            }
        }
        guard.symbols.sort();
        guard.symbols.dedup();
    }
}

fn apply_completion_refresh_plan(
    vm: &Vm,
    state: &Arc<Mutex<CompletionState>>,
    plan: CompletionRefreshPlan,
) {
    match plan {
        CompletionRefreshPlan::None => {}
        CompletionRefreshPlan::Full => refresh_completion_state(vm, state),
        CompletionRefreshPlan::Symbols(symbols) => refresh_completion_symbols(vm, state, &symbols),
    }
}

fn build_completion_state(vm: &Vm) -> CompletionState {
    const MAX_COMPLETION_DEPTH: usize = 2;
    let mut state = CompletionState::default();
    let mut visited = HashSet::new();
    for (name, value) in vm.repl_root_bindings() {
        if !is_valid_identifier_name(&name) {
            continue;
        }
        state.symbols.push(name.clone());
        collect_completion_members(
            &mut state,
            &name,
            &value,
            0,
            MAX_COMPLETION_DEPTH,
            &mut visited,
        );
    }
    state
        .symbols
        .extend(REPL_KEYWORDS.iter().map(|keyword| (*keyword).to_string()));
    state.symbols.sort();
    state.symbols.dedup();
    state
}

fn collect_completion_members(
    state: &mut CompletionState,
    path: &str,
    value: &Value,
    depth: usize,
    max_depth: usize,
    visited: &mut HashSet<(u64, usize)>,
) {
    if depth > max_depth {
        return;
    }
    let object_id = value_object_id(value);
    if let Some(id) = object_id
        && !visited.insert((id, depth))
    {
        return;
    }

    let bindings = value_member_bindings(value);
    if bindings.is_empty() {
        if let Some(id) = object_id {
            visited.remove(&(id, depth));
        }
        return;
    }

    let mut names = Vec::new();
    for (member_name, member_value) in bindings {
        if !is_valid_identifier_name(&member_name) {
            continue;
        }
        names.push(member_name.clone());
        if depth < max_depth && !matches!(member_value, Value::None) {
            let next_path = format!("{path}.{member_name}");
            collect_completion_members(
                state,
                &next_path,
                &member_value,
                depth + 1,
                max_depth,
                visited,
            );
        }
    }
    if !names.is_empty() {
        names.sort();
        names.dedup();
        state.members.insert(path.to_string(), names);
    }
    if let Some(id) = object_id {
        visited.remove(&(id, depth));
    }
}

fn value_member_bindings(value: &Value) -> Vec<(String, Value)> {
    if is_cpython_proxy_completion_value(value) {
        return Vec::new();
    }
    let mut bindings: HashMap<String, Value> = HashMap::new();
    match value {
        Value::Module(obj) => {
            if let Object::Module(module_data) = &*obj.kind() {
                for (name, member_value) in &module_data.globals {
                    bindings.insert(name.clone(), member_value.clone());
                }
            }
        }
        Value::Class(obj) => {
            collect_class_bindings_into(obj, &mut bindings);
        }
        Value::Instance(obj) => {
            if let Object::Instance(instance_data) = &*obj.kind() {
                for (name, member_value) in &instance_data.attrs {
                    bindings.insert(name.clone(), member_value.clone());
                }
                collect_class_bindings_into(&instance_data.class, &mut bindings);
            }
        }
        Value::Builtin(builtin) => {
            for name in builtin_member_names(*builtin) {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Bool(_) => {
            for name in primitive_member_names("bool") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Int(_) => {
            for name in primitive_member_names("int") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Float(_) => {
            for name in primitive_member_names("float") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Complex { .. } => {
            for name in primitive_member_names("complex") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Str(_) => {
            for name in primitive_member_names("str") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Bytes(_) => {
            for name in primitive_member_names("bytes") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::ByteArray(_) => {
            for name in primitive_member_names("bytearray") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::List(_) => {
            for name in primitive_member_names("list") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Tuple(_) => {
            for name in primitive_member_names("tuple") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Dict(_) => {
            for name in primitive_member_names("dict") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::Set(_) => {
            for name in primitive_member_names("set") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        Value::FrozenSet(_) => {
            for name in primitive_member_names("frozenset") {
                bindings.entry(name.to_string()).or_insert(Value::None);
            }
        }
        _ => {}
    }
    let mut out = bindings.into_iter().collect::<Vec<_>>();
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

fn is_cpython_proxy_completion_value(value: &Value) -> bool {
    match value {
        Value::Class(obj) => matches!(
            &*obj.kind(),
            Object::Class(class_data)
                if matches!(
                    class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                    Some(Value::Bool(true))
                )
        ),
        Value::Instance(obj) => match &*obj.kind() {
            Object::Instance(instance_data) => {
                if instance_data
                    .attrs
                    .contains_key("__pyrs_cpython_proxy_ptr__")
                {
                    return true;
                }
                matches!(
                    &*instance_data.class.kind(),
                    Object::Class(class_data)
                        if matches!(
                            class_data.attrs.get("__pyrs_cpython_proxy_marker__"),
                            Some(Value::Bool(true))
                        )
                )
            }
            _ => false,
        },
        _ => false,
    }
}

fn collect_class_bindings_into(
    class_obj: &crate::runtime::ObjRef,
    out: &mut HashMap<String, Value>,
) {
    let Object::Class(class_data) = &*class_obj.kind() else {
        return;
    };
    for (name, value) in &class_data.attrs {
        out.entry(name.clone()).or_insert_with(|| value.clone());
    }
    for mro_entry in &class_data.mro {
        if let Object::Class(parent_data) = &*mro_entry.kind() {
            for (name, value) in &parent_data.attrs {
                out.entry(name.clone()).or_insert_with(|| value.clone());
            }
        }
    }
}

fn builtin_member_names(builtin: crate::runtime::BuiltinFunction) -> &'static [&'static str] {
    if builtin_type_name(builtin).is_some() {
        return primitive_member_names("type");
    }
    &[
        "__annotations__",
        "__call__",
        "__class__",
        "__doc__",
        "__module__",
        "__name__",
        "__qualname__",
    ]
}

fn builtin_type_name(builtin: crate::runtime::BuiltinFunction) -> Option<&'static str> {
    match builtin {
        crate::runtime::BuiltinFunction::Type => Some("type"),
        crate::runtime::BuiltinFunction::Bool => Some("bool"),
        crate::runtime::BuiltinFunction::Int => Some("int"),
        crate::runtime::BuiltinFunction::Float => Some("float"),
        crate::runtime::BuiltinFunction::Complex => Some("complex"),
        crate::runtime::BuiltinFunction::Str => Some("str"),
        crate::runtime::BuiltinFunction::List => Some("list"),
        crate::runtime::BuiltinFunction::Tuple => Some("tuple"),
        crate::runtime::BuiltinFunction::Dict => Some("dict"),
        crate::runtime::BuiltinFunction::Set => Some("set"),
        crate::runtime::BuiltinFunction::FrozenSet => Some("frozenset"),
        crate::runtime::BuiltinFunction::Bytes => Some("bytes"),
        crate::runtime::BuiltinFunction::ByteArray => Some("bytearray"),
        crate::runtime::BuiltinFunction::MemoryView => Some("memoryview"),
        crate::runtime::BuiltinFunction::Slice => Some("slice"),
        crate::runtime::BuiltinFunction::Range => Some("range"),
        crate::runtime::BuiltinFunction::Zip => Some("zip"),
        crate::runtime::BuiltinFunction::Map => Some("map"),
        crate::runtime::BuiltinFunction::Filter => Some("filter"),
        crate::runtime::BuiltinFunction::Enumerate => Some("enumerate"),
        crate::runtime::BuiltinFunction::Super => Some("super"),
        crate::runtime::BuiltinFunction::ClassMethod => Some("classmethod"),
        crate::runtime::BuiltinFunction::StaticMethod => Some("staticmethod"),
        crate::runtime::BuiltinFunction::Property => Some("property"),
        _ => None,
    }
}

fn primitive_member_names(kind: &str) -> &'static [&'static str] {
    match kind {
        "type" => &[
            "__base__",
            "__bases__",
            "__class__",
            "__dict__",
            "__doc__",
            "__module__",
            "__mro__",
            "__name__",
            "__qualname__",
            "__subclasses__",
        ],
        "bool" => &[
            "__class__",
            "__int__",
            "__bool__",
            "bit_count",
            "bit_length",
            "conjugate",
            "to_bytes",
        ],
        "int" => &[
            "__class__",
            "as_integer_ratio",
            "bit_count",
            "bit_length",
            "conjugate",
            "from_bytes",
            "to_bytes",
        ],
        "float" => &[
            "__class__",
            "as_integer_ratio",
            "conjugate",
            "fromhex",
            "hex",
            "is_integer",
        ],
        "complex" => &["__class__", "conjugate", "imag", "real"],
        "str" => &[
            "__class__",
            "capitalize",
            "casefold",
            "center",
            "count",
            "encode",
            "endswith",
            "expandtabs",
            "find",
            "format",
            "format_map",
            "index",
            "isalnum",
            "isalpha",
            "isascii",
            "isdecimal",
            "isdigit",
            "isidentifier",
            "islower",
            "isnumeric",
            "isprintable",
            "isspace",
            "istitle",
            "isupper",
            "join",
            "ljust",
            "lower",
            "lstrip",
            "partition",
            "removeprefix",
            "removesuffix",
            "replace",
            "rfind",
            "rindex",
            "rjust",
            "rpartition",
            "rsplit",
            "rstrip",
            "split",
            "splitlines",
            "startswith",
            "strip",
            "swapcase",
            "title",
            "translate",
            "upper",
            "zfill",
        ],
        "bytes" => &[
            "__class__",
            "capitalize",
            "center",
            "count",
            "decode",
            "endswith",
            "expandtabs",
            "find",
            "fromhex",
            "hex",
            "index",
            "isalnum",
            "isalpha",
            "isascii",
            "isdigit",
            "islower",
            "isspace",
            "istitle",
            "isupper",
            "join",
            "ljust",
            "lower",
            "lstrip",
            "partition",
            "removeprefix",
            "removesuffix",
            "replace",
            "rfind",
            "rindex",
            "rjust",
            "rpartition",
            "rsplit",
            "rstrip",
            "split",
            "splitlines",
            "startswith",
            "strip",
            "swapcase",
            "title",
            "translate",
            "upper",
            "zfill",
        ],
        "bytearray" => &[
            "__class__",
            "append",
            "clear",
            "copy",
            "count",
            "decode",
            "extend",
            "find",
            "fromhex",
            "hex",
            "index",
            "insert",
            "join",
            "pop",
            "remove",
            "replace",
            "reverse",
            "split",
            "strip",
            "translate",
        ],
        "list" => &[
            "__class__",
            "append",
            "clear",
            "copy",
            "count",
            "extend",
            "index",
            "insert",
            "pop",
            "remove",
            "reverse",
            "sort",
        ],
        "tuple" => &["__class__", "count", "index"],
        "dict" => &[
            "__class__",
            "clear",
            "copy",
            "fromkeys",
            "get",
            "items",
            "keys",
            "pop",
            "popitem",
            "setdefault",
            "update",
            "values",
        ],
        "set" => &[
            "__class__",
            "add",
            "clear",
            "copy",
            "difference",
            "difference_update",
            "discard",
            "intersection",
            "intersection_update",
            "isdisjoint",
            "issubset",
            "issuperset",
            "pop",
            "remove",
            "symmetric_difference",
            "symmetric_difference_update",
            "union",
            "update",
        ],
        "frozenset" => &[
            "__class__",
            "copy",
            "difference",
            "intersection",
            "isdisjoint",
            "issubset",
            "issuperset",
            "symmetric_difference",
            "union",
        ],
        _ => &["__class__"],
    }
}

fn is_valid_identifier_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn value_object_id(value: &Value) -> Option<u64> {
    match value {
        Value::List(obj)
        | Value::Tuple(obj)
        | Value::Dict(obj)
        | Value::DictKeys(obj)
        | Value::Set(obj)
        | Value::FrozenSet(obj)
        | Value::Bytes(obj)
        | Value::ByteArray(obj)
        | Value::MemoryView(obj)
        | Value::Iterator(obj)
        | Value::Generator(obj)
        | Value::Module(obj)
        | Value::Class(obj)
        | Value::Instance(obj)
        | Value::Super(obj)
        | Value::BoundMethod(obj)
        | Value::Function(obj)
        | Value::Cell(obj) => Some(obj.id()),
        _ => None,
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
    state: Arc<Mutex<CompletionState>>,
}

impl ReplCompleter {
    fn new(state: Arc<Mutex<CompletionState>>) -> Self {
        Self { state }
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
        let state = self
            .state
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        identifier_suggestions(token_start, pos.min(line.len()), token, &state)
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
    state: &CompletionState,
) -> Vec<Suggestion> {
    let (replace_start, prefix, needle, candidates) = if let Some(index) = token.rfind('.') {
        let object_path = &token[..index];
        let member_prefix = format!("{object_path}.");
        let member_candidates = state.members.get(object_path).cloned().unwrap_or_default();
        (
            start + index + 1,
            member_prefix,
            &token[index + 1..],
            member_candidates,
        )
    } else {
        (start, String::new(), token, state.symbols.clone())
    };

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

#[derive(Clone, Copy)]
struct PythonHighlighter {
    palette: ReplPalette,
}

impl PythonHighlighter {
    fn new(palette: ReplPalette) -> Self {
        Self { palette }
    }
}

impl Highlighter for PythonHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        if line.is_empty() {
            return styled;
        }

        let mut lexer = parser::lexer::Lexer::new(line);
        let tokens = match lexer.tokenize() {
            Ok(tokens) => tokens,
            Err(_) => {
                styled.push((Style::new(), line.to_string()));
                return styled;
            }
        };

        let mut cursor = 0usize;
        let mut pending_name_style = PendingNameStyle::None;
        let mut decorator_path_active = false;
        for token in tokens {
            let style = style_for_token_kind(
                self.palette,
                &token.kind,
                &token.lexeme,
                pending_name_style,
                decorator_path_active,
            );
            let token_start = token.offset.min(line.len());
            let token_end = token_visual_end_offset(line, &token)
                .unwrap_or_else(|| token_start.saturating_add(token.lexeme.len()))
                .min(line.len());

            if token_start > cursor {
                push_plain_or_comment(&mut styled, &line[cursor..token_start], self.palette);
            }
            if token_end > token_start {
                styled.push((style, line[token_start..token_end].to_string()));
            }
            cursor = token_end.max(cursor);
            update_highlighter_state(
                &token.kind,
                &mut pending_name_style,
                &mut decorator_path_active,
            );
        }

        if cursor < line.len() {
            push_plain_or_comment(&mut styled, &line[cursor..], self.palette);
        }
        if styled.buffer.is_empty() {
            styled.push((Style::new(), line.to_string()));
        }
        styled
    }
}

fn token_visual_end_offset(line: &str, token: &parser::token::Token) -> Option<usize> {
    match token.kind {
        parser::token::TokenKind::String
        | parser::token::TokenKind::Bytes
        | parser::token::TokenKind::FString => string_literal_visual_end(line, token.offset),
        _ => Some(token.offset.saturating_add(token.lexeme.len())),
    }
}

fn string_literal_visual_end(line: &str, start_offset: usize) -> Option<usize> {
    let source = line.get(start_offset..)?;
    let mut cursor = 0usize;
    while let Some(ch) = source.get(cursor..)?.chars().next() {
        if ch == '\'' || ch == '"' {
            break;
        }
        if ch.is_ascii_alphabetic() && matches!(ch.to_ascii_lowercase(), 'r' | 'b' | 'f') {
            cursor += ch.len_utf8();
            continue;
        }
        return None;
    }

    let quote = source.get(cursor..)?.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let quote_len = quote.len_utf8();
    let quote_char = quote;
    let mut body_pos = cursor + quote_len;
    let triple = source.get(body_pos..)?.starts_with(quote_char)
        && source.get(body_pos + quote_len..)?.starts_with(quote_char);
    if triple {
        body_pos += quote_len * 2;
    }

    while body_pos < source.len() {
        if triple
            && source.get(body_pos..)?.starts_with(quote_char)
            && source.get(body_pos + quote_len..)?.starts_with(quote_char)
            && source
                .get(body_pos + quote_len * 2..)?
                .starts_with(quote_char)
        {
            return Some(start_offset + body_pos + quote_len * 3);
        }

        let ch = source.get(body_pos..)?.chars().next()?;
        body_pos += ch.len_utf8();
        if ch == '\\' {
            if let Some(next) = source.get(body_pos..)?.chars().next() {
                body_pos += next.len_utf8();
            }
            continue;
        }
        if !triple && ch == quote_char {
            return Some(start_offset + body_pos);
        }
    }
    None
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum PendingNameStyle {
    None,
    Function,
    Class,
}

fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "int"
            | "str"
            | "float"
            | "bool"
            | "bytes"
            | "bytearray"
            | "list"
            | "tuple"
            | "dict"
            | "set"
            | "frozenset"
            | "complex"
            | "object"
    )
}

fn style_for_token_kind(
    palette: ReplPalette,
    token_kind: &parser::token::TokenKind,
    lexeme: &str,
    pending_name_style: PendingNameStyle,
    decorator_path_active: bool,
) -> Style {
    match token_kind {
        parser::token::TokenKind::Keyword(_) => palette.keyword_style,
        parser::token::TokenKind::At => palette.decorator_style,
        parser::token::TokenKind::Dot if decorator_path_active => palette.decorator_style,
        parser::token::TokenKind::Name if pending_name_style == PendingNameStyle::Function => {
            palette.function_name_style
        }
        parser::token::TokenKind::Name if pending_name_style == PendingNameStyle::Class => {
            palette.class_name_style
        }
        parser::token::TokenKind::Name if decorator_path_active => palette.decorator_style,
        parser::token::TokenKind::Name if is_builtin_type_name(lexeme) => palette.type_name_style,
        parser::token::TokenKind::Number => palette.number_style,
        parser::token::TokenKind::String
        | parser::token::TokenKind::Bytes
        | parser::token::TokenKind::FString => palette.string_style,
        parser::token::TokenKind::Name => Style::new(),
        _ => Style::new(),
    }
}

fn update_highlighter_state(
    token_kind: &parser::token::TokenKind,
    pending_name_style: &mut PendingNameStyle,
    decorator_path_active: &mut bool,
) {
    use parser::token::Keyword;
    use parser::token::TokenKind;

    match token_kind {
        TokenKind::Keyword(Keyword::Def) => {
            *pending_name_style = PendingNameStyle::Function;
            *decorator_path_active = false;
        }
        TokenKind::Keyword(Keyword::Class) => {
            *pending_name_style = PendingNameStyle::Class;
            *decorator_path_active = false;
        }
        TokenKind::At => {
            *pending_name_style = PendingNameStyle::None;
            *decorator_path_active = true;
        }
        TokenKind::Name => {
            if *pending_name_style != PendingNameStyle::None {
                *pending_name_style = PendingNameStyle::None;
            } else if *decorator_path_active {
                *decorator_path_active = true;
            }
        }
        TokenKind::Dot if *decorator_path_active => {}
        TokenKind::Newline | TokenKind::Semicolon => {
            *pending_name_style = PendingNameStyle::None;
            *decorator_path_active = false;
        }
        TokenKind::LParen if *decorator_path_active => {
            *decorator_path_active = false;
        }
        _ => {
            *pending_name_style = PendingNameStyle::None;
            if *decorator_path_active {
                *decorator_path_active = false;
            }
        }
    }
}

fn push_plain_or_comment(styled: &mut StyledText, segment: &str, palette: ReplPalette) {
    if segment.is_empty() {
        return;
    }
    if let Some(comment_start) = segment.find('#') {
        let (plain, comment) = segment.split_at(comment_start);
        if !plain.is_empty() {
            styled.push((Style::new(), plain.to_string()));
        }
        styled.push((palette.comment_style, comment.to_string()));
    } else {
        styled.push((Style::new(), segment.to_string()));
    }
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

fn load_repl_startup_script(
    vm: &mut Vm,
    completion_state: &Arc<Mutex<CompletionState>>,
) -> Result<(), String> {
    let Some(path) = resolve_repl_startup_path() else {
        return Ok(());
    };
    if !path.is_file() {
        return Ok(());
    }

    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!(
                "warning: failed reading startup script {}: {err}",
                path.display()
            );
            return Ok(());
        }
    };

    if let Err(err) = execute_module_source(vm, &source, &path.to_string_lossy(), false) {
        eprintln!("warning: startup script {} failed: {err}", path.display());
        return Ok(());
    }
    refresh_completion_state(vm, completion_state);
    Ok(())
}

fn resolve_repl_startup_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("PYRS_REPL_INIT") {
        if path.is_empty() {
            return None;
        }
        return expand_home_path(Path::new(&path));
    }
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".pyrsrc"))
}

fn expand_home_path(path: &Path) -> Option<PathBuf> {
    let path_str = path.to_str()?;
    if path_str == "~" {
        let home = env::var_os("HOME")?;
        return Some(PathBuf::from(home));
    }
    if let Some(rest) = path_str.strip_prefix("~/") {
        let home = env::var_os("HOME")?;
        return Some(PathBuf::from(home).join(rest));
    }
    Some(path.to_path_buf())
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

#[derive(Debug, Clone, Eq, PartialEq)]
enum ReplMagicCommand {
    Time(String),
    TimeIt(TimeItRequest),
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct TimeItRequest {
    source: String,
    loops: Option<u64>,
    repeats: Option<u32>,
}

fn parse_magic_command(line: &str) -> Option<ReplMagicCommand> {
    let trimmed = line.trim_start();
    if let Some(request) = parse_timeit_command(trimmed) {
        return Some(ReplMagicCommand::TimeIt(request));
    }
    parse_time_command(trimmed).map(ReplMagicCommand::Time)
}

fn parse_time_command(line: &str) -> Option<String> {
    let source = strip_magic_prefix(line, "%time")?;
    if source.is_empty() {
        return None;
    }
    Some(format!("{source}\n"))
}

fn parse_timeit_command(line: &str) -> Option<TimeItRequest> {
    let mut cursor = strip_magic_prefix(line, "%timeit")?;
    if cursor.is_empty() {
        return None;
    }

    let mut loops = None;
    let mut repeats = None;
    while !cursor.is_empty() {
        let (token, rest) = split_first_word(cursor);
        if token == "-n" || token == "--number" {
            let (value, tail) = split_first_word(rest.trim_start());
            let parsed = value.parse::<u64>().ok()?;
            if parsed == 0 {
                return None;
            }
            loops = Some(parsed);
            cursor = tail.trim_start();
            continue;
        }
        if token == "-r" || token == "--repeat" {
            let (value, tail) = split_first_word(rest.trim_start());
            let parsed = value.parse::<u32>().ok()?;
            if parsed == 0 {
                return None;
            }
            repeats = Some(parsed);
            cursor = tail.trim_start();
            continue;
        }
        if let Some(value) = token.strip_prefix("-n") {
            let parsed = value.parse::<u64>().ok()?;
            if parsed == 0 {
                return None;
            }
            loops = Some(parsed);
            cursor = rest.trim_start();
            continue;
        }
        if let Some(value) = token.strip_prefix("-r") {
            let parsed = value.parse::<u32>().ok()?;
            if parsed == 0 {
                return None;
            }
            repeats = Some(parsed);
            cursor = rest.trim_start();
            continue;
        }
        if let Some(value) = token.strip_prefix("--number=") {
            let parsed = value.parse::<u64>().ok()?;
            if parsed == 0 {
                return None;
            }
            loops = Some(parsed);
            cursor = rest.trim_start();
            continue;
        }
        if let Some(value) = token.strip_prefix("--repeat=") {
            let parsed = value.parse::<u32>().ok()?;
            if parsed == 0 {
                return None;
            }
            repeats = Some(parsed);
            cursor = rest.trim_start();
            continue;
        }
        break;
    }

    let source = cursor.trim_start();
    if source.is_empty() {
        return None;
    }
    Some(TimeItRequest {
        source: format!("{source}\n"),
        loops,
        repeats,
    })
}

fn strip_magic_prefix<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(prefix)?;
    if rest.is_empty() {
        return Some(rest);
    }
    let first = rest.chars().next()?;
    if first.is_whitespace() {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn split_first_word(input: &str) -> (&str, &str) {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return ("", "");
    }
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_whitespace() {
            return (&trimmed[..idx], &trimmed[idx..]);
        }
    }
    (trimmed, "")
}

fn apply_meta_command(
    command: MetaCommand,
    vm: &mut Vm,
    pending: &mut String,
    paste_mode: &mut bool,
    timing_enabled: &mut bool,
    import_site: bool,
    warnoptions: &[String],
    completion_state: &Arc<Mutex<CompletionState>>,
) -> Result<bool, String> {
    match command {
        MetaCommand::Help => {
            println!(":help / .help     show REPL help");
            println!(":clear / .clear   clear pending input buffer");
            println!(":paste            toggle paste mode (finish with :paste)");
            println!(":timing           toggle execution timing display");
            println!("%time <expr>      run one snippet with timing");
            println!("%timeit <expr>    run repeated timings (options: -n, -r)");
            println!(":reset            reset interpreter state");
            println!(":exit / :quit     exit REPL");
            println!("Tab               insert {} spaces", INDENT_WIDTH);
            println!("Shift-Tab/Ctrl-Space  completion menu");
            println!("Esc               dismiss completion menu/current suggestion");
            println!("env: PYRS_REPL_THEME=auto|dark|light");
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
            *vm = build_vm_with_warnoptions(import_site, true, warnoptions, None)?;
            pending.clear();
            *paste_mode = false;
            refresh_completion_state(vm, completion_state);
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
    vm.cache_source_text(filename, source);
    let module =
        parser::parse_module(source).map_err(|err| format_parse_error(source, filename, &err))?;
    execute_parsed_module(vm, &module, source, filename, echo_expression_result)
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

fn execute_timeit_command(
    vm: &mut Vm,
    request: &TimeItRequest,
    filename: &str,
) -> Result<(), String> {
    const DEFAULT_REPEATS: u32 = 7;
    const TARGET_SECONDS: f64 = 0.2;
    const MAX_CALIBRATION_LOOPS: u64 = 1_000_000_000;

    vm.cache_source_text(filename, &request.source);
    let module = parser::parse_module(&request.source)
        .map_err(|err| format_parse_error(&request.source, filename, &err))?;
    let code = compiler::compile_module_with_filename(&module, filename)
        .map_err(|err| format_compile_error(filename, &request.source, &err))?;
    let loops = if let Some(loops) = request.loops {
        loops
    } else {
        let mut loops = 1u64;
        loop {
            let started = Instant::now();
            for _ in 0..loops {
                vm.execute(&code)
                    .map_err(|err| format!("runtime error: {}", err.message))?;
            }
            let elapsed = started.elapsed().as_secs_f64();
            if elapsed >= TARGET_SECONDS || loops >= MAX_CALIBRATION_LOOPS {
                break loops.max(1);
            }
            let scale = if elapsed > 0.0 {
                ((TARGET_SECONDS / elapsed).ceil() as u64).clamp(2, 10)
            } else {
                10
            };
            loops = loops.saturating_mul(scale).clamp(1, MAX_CALIBRATION_LOOPS);
        }
    };
    let repeats = request.repeats.unwrap_or(DEFAULT_REPEATS).max(1);

    let mut samples = Vec::with_capacity(repeats as usize);
    for _ in 0..repeats {
        let started = Instant::now();
        for _ in 0..loops {
            vm.execute(&code)
                .map_err(|err| format!("runtime error: {}", err.message))?;
        }
        samples.push(started.elapsed().as_secs_f64() / loops as f64);
    }

    let best = samples
        .iter()
        .copied()
        .fold(f64::INFINITY, |current, sample| current.min(sample));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let variance = samples
        .iter()
        .map(|sample| {
            let delta = sample - mean;
            delta * delta
        })
        .sum::<f64>()
        / samples.len() as f64;
    let stddev = variance.sqrt();

    eprintln!(
        "[timeit] {loops} loops, best of {repeats}: {} per loop (mean {} ± {})",
        format_duration(best),
        format_duration(mean),
        format_duration(stddev),
    );
    Ok(())
}

fn format_duration(seconds: f64) -> String {
    if seconds < 1e-6 {
        format!("{:.3} ns", seconds * 1e9)
    } else if seconds < 1e-3 {
        format!("{:.3} us", seconds * 1e6)
    } else if seconds < 1.0 {
        format!("{:.3} ms", seconds * 1e3)
    } else {
        format!("{seconds:.3} s")
    }
}

fn execute_parsed_module_with_timing(
    vm: &mut Vm,
    module: &Module,
    source: &str,
    filename: &str,
    echo_expression_result: bool,
    timing_enabled: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let result = execute_parsed_module(vm, module, source, filename, echo_expression_result);
    if timing_enabled {
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[timing] {elapsed_ms:.3} ms");
    }
    result
}

fn execute_parsed_module(
    vm: &mut Vm,
    module: &Module,
    source: &str,
    filename: &str,
    echo_expression_result: bool,
) -> Result<(), String> {
    if echo_expression_result
        && module.body.len() == 1
        && let StmtKind::Expr(expr) = &module.body[0].node
    {
        let code = compiler::compile_expression_with_filename(expr, filename)
            .map_err(|err| format_compile_error(filename, source, &err))?;
        let value = vm
            .execute(&code)
            .map_err(|err| format!("runtime error: {}", err.message))?;
        if !matches!(value, Value::None) {
            let rendered = vm
                .render_value_repr_for_display(value)
                .map_err(|err| format!("runtime error: {}", err.message))?;
            println!("{rendered}");
        }
        return Ok(());
    }

    let code = compiler::compile_module_with_filename(module, filename)
        .map_err(|err| format_compile_error(filename, source, &err))?;
    vm.execute(&code)
        .map_err(|err| format!("runtime error: {}", err.message))?;
    Ok(())
}

fn format_parse_error(source: &str, filename: &str, err: &ParseError) -> String {
    format_syntax_error(filename, source, err)
}

fn repl_parse_candidate_source(pending: &str) -> &str {
    pending.strip_suffix('\n').unwrap_or(pending)
}

fn repl_parse_success_requires_more_input(source: &str, latest_line: &str) -> bool {
    !latest_line.trim().is_empty() && repl_has_eof_implied_dedent(source)
}

fn repl_has_eof_implied_dedent(source: &str) -> bool {
    let mut lexer = parser::lexer::Lexer::new(source);
    let Ok(tokens) = lexer.tokenize() else {
        return false;
    };
    let eof_offset = source.len();
    let mut index = tokens.len();
    while index > 0 && matches!(tokens[index - 1].kind, parser::token::TokenKind::EndMarker) {
        index -= 1;
    }
    let mut saw_eof_dedent = false;
    while index > 0 {
        let token = &tokens[index - 1];
        if !matches!(token.kind, parser::token::TokenKind::Dedent) {
            break;
        }
        if token.offset == eof_offset {
            saw_eof_dedent = true;
        }
        index -= 1;
    }
    saw_eof_dedent
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

    use reedline::{Completer, Highlighter, StyledText};

    use super::{
        CompletionRefreshPlan, CompletionState, MetaCommand, PythonHighlighter, ReplCompleter,
        ReplMagicCommand, ReplThemeMode, ResolvedReplTheme, TimeItRequest, completion_fragment,
        format_parse_error, is_path_like, parse_colorfgbg_background_code, parse_magic_command,
        parse_meta_command, parse_repl_theme_mode, repl_input_is_incomplete,
        repl_parse_candidate_source, repl_parse_success_requires_more_input,
        repl_module_completion_plan, repl_palette, resolve_repl_theme,
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
    fn repl_candidate_source_omits_latest_synthetic_newline() {
        assert_eq!(
            repl_parse_candidate_source("class A:\n    x = 1\n"),
            "class A:\n    x = 1"
        );
        assert_eq!(
            repl_parse_candidate_source("class A:\n    x = 1\n\n"),
            "class A:\n    x = 1\n"
        );
    }

    #[test]
    fn repl_class_block_stays_incomplete_until_blank_line() {
        let without_blank = repl_parse_candidate_source("class A:\n    x = 1\n");
        assert!(parser::parse_module(without_blank).is_ok());
        assert!(repl_parse_success_requires_more_input(
            without_blank,
            "    x = 1"
        ));

        let with_blank = repl_parse_candidate_source("class A:\n    x = 1\n\n");
        assert!(
            parser::parse_module(with_blank).is_ok(),
            "class block should complete after blank line"
        );
        assert!(!repl_parse_success_requires_more_input(with_blank, ""));
    }

    #[test]
    fn parse_error_is_human_readable() {
        let source = "def f(\n";
        let err = parser::parse_module(source).expect_err("parse should fail");
        let rendered = format_parse_error(source, "<stdin>", &err);
        assert!(rendered.contains("SyntaxError:"));
        assert!(rendered.contains("File \"<stdin>\", line"));
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
    fn parses_time_and_timeit_magic_commands() {
        assert_eq!(
            parse_magic_command("%time 1 + 1"),
            Some(ReplMagicCommand::Time("1 + 1\n".to_string()))
        );
        assert_eq!(
            parse_magic_command("   %time print('ok')"),
            Some(ReplMagicCommand::Time("print('ok')\n".to_string()))
        );
        assert_eq!(
            parse_magic_command("%timeit -n 25 -r 3 x += 1"),
            Some(ReplMagicCommand::TimeIt(TimeItRequest {
                source: "x += 1\n".to_string(),
                loops: Some(25),
                repeats: Some(3),
            }))
        );
        assert_eq!(
            parse_magic_command("%timeit --number=10 --repeat=5 print('ok')"),
            Some(ReplMagicCommand::TimeIt(TimeItRequest {
                source: "print('ok')\n".to_string(),
                loops: Some(10),
                repeats: Some(5),
            }))
        );
        assert_eq!(parse_magic_command("%timeit"), None);
        assert_eq!(parse_magic_command("%time"), None);
        assert_eq!(parse_magic_command("print(1)"), None);
    }

    #[test]
    fn parses_repl_theme_mode_values_case_insensitively() {
        assert_eq!(parse_repl_theme_mode("auto"), Some(ReplThemeMode::Auto));
        assert_eq!(parse_repl_theme_mode("Dark"), Some(ReplThemeMode::Dark));
        assert_eq!(parse_repl_theme_mode("LIGHT"), Some(ReplThemeMode::Light));
        assert_eq!(parse_repl_theme_mode("unknown"), None);
    }

    #[test]
    fn completion_refresh_plan_is_incremental_for_simple_assignment() {
        let expr_module = parser::parse_module("1 + 1\n").expect("parse expression module");
        let assign_module = parser::parse_module("x = 1\n").expect("parse assignment module");
        let import_module = parser::parse_module("import os\n").expect("parse import module");
        assert_eq!(
            repl_module_completion_plan(&expr_module),
            CompletionRefreshPlan::None
        );
        assert_eq!(
            repl_module_completion_plan(&assign_module),
            CompletionRefreshPlan::Symbols(vec!["x".to_string()])
        );
        assert_eq!(
            repl_module_completion_plan(&import_module),
            CompletionRefreshPlan::Symbols(vec!["os".to_string()])
        );
    }

    #[test]
    fn completion_refresh_plan_handles_import_bindings_incrementally() {
        let import_nested =
            parser::parse_module("import os.path\n").expect("parse nested import module");
        let import_as =
            parser::parse_module("import os.path as osp\n").expect("parse import-as module");
        let from_import =
            parser::parse_module("from os import path\n").expect("parse from-import module");
        let from_import_as =
            parser::parse_module("from os import path as p\n").expect("parse from-import-as");
        let from_import_star =
            parser::parse_module("from os import *\n").expect("parse from-import-star module");

        assert_eq!(
            repl_module_completion_plan(&import_nested),
            CompletionRefreshPlan::Symbols(vec!["os".to_string()])
        );
        assert_eq!(
            repl_module_completion_plan(&import_as),
            CompletionRefreshPlan::Symbols(vec!["osp".to_string()])
        );
        assert_eq!(
            repl_module_completion_plan(&from_import),
            CompletionRefreshPlan::Symbols(vec!["path".to_string()])
        );
        assert_eq!(
            repl_module_completion_plan(&from_import_as),
            CompletionRefreshPlan::Symbols(vec!["p".to_string()])
        );
        assert_eq!(
            repl_module_completion_plan(&from_import_star),
            CompletionRefreshPlan::Full
        );
    }

    #[test]
    fn resolves_repl_theme_with_explicit_override() {
        assert_eq!(
            resolve_repl_theme(ReplThemeMode::Dark, Some("15;7")),
            ResolvedReplTheme::Dark
        );
        assert_eq!(
            resolve_repl_theme(ReplThemeMode::Light, Some("15;0")),
            ResolvedReplTheme::Light
        );
    }

    #[test]
    fn resolves_repl_theme_auto_from_colorfgbg() {
        assert_eq!(
            parse_colorfgbg_background_code("15;7"),
            Some(7),
            "parses light background code"
        );
        assert_eq!(
            parse_colorfgbg_background_code("15;0"),
            Some(0),
            "parses dark background code"
        );
        assert_eq!(
            resolve_repl_theme(ReplThemeMode::Auto, Some("15;7")),
            ResolvedReplTheme::Light
        );
        assert_eq!(
            resolve_repl_theme(ReplThemeMode::Auto, Some("15;0")),
            ResolvedReplTheme::Dark
        );
        assert_eq!(
            resolve_repl_theme(ReplThemeMode::Auto, Some("invalid")),
            ResolvedReplTheme::Dark
        );
        assert_eq!(
            resolve_repl_theme(ReplThemeMode::Auto, None),
            ResolvedReplTheme::Dark
        );
    }

    #[test]
    fn meta_reset_rebuilds_vm_state() {
        let mut vm = super::build_vm(false, true).expect("vm");
        vm.set_global("x", crate::runtime::Value::Int(7));
        assert_eq!(vm.get_global("x"), Some(crate::runtime::Value::Int(7)));

        let mut pending = String::new();
        let mut paste_mode = false;
        let mut timing_enabled = true;
        let completion_state = Arc::new(Mutex::new(CompletionState::default()));
        let should_exit = super::apply_meta_command(
            MetaCommand::Reset,
            &mut vm,
            &mut pending,
            &mut paste_mode,
            &mut timing_enabled,
            false,
            &[],
            &completion_state,
        )
        .expect("reset should succeed");

        assert!(!should_exit);
        assert!(vm.get_global("x").is_none());
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
        let state = Arc::new(Mutex::new(CompletionState {
            symbols: vec![
                "statistics".to_string(),
                "str".to_string(),
                "sum".to_string(),
            ],
            members: std::collections::HashMap::new(),
        }));
        let mut completer = ReplCompleter::new(state);

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

    #[test]
    fn completer_resolves_deep_module_member_names_dynamically() {
        let mut vm = super::build_vm(false, true).expect("vm");
        super::execute_module_source(&mut vm, "import os\n", "<stdin>", false)
            .expect("import should succeed");
        let state = Arc::new(Mutex::new(super::build_completion_state(&vm)));
        let mut completer = ReplCompleter::new(state);
        let suggestions = completer.complete("os.path.j", "os.path.j".len());
        let values: Vec<String> = suggestions.into_iter().map(|item| item.value).collect();
        assert!(values.iter().any(|value| value == "os.path.join"));
    }

    #[test]
    fn completer_suggests_common_members_for_primitive_values() {
        let mut vm = super::build_vm(false, true).expect("vm");
        vm.set_global("text", crate::runtime::Value::Str("abc".to_string()));
        let state = Arc::new(Mutex::new(super::build_completion_state(&vm)));
        let mut completer = ReplCompleter::new(state);
        let suggestions = completer.complete("text.spl", "text.spl".len());
        let values: Vec<String> = suggestions.into_iter().map(|item| item.value).collect();
        assert!(values.iter().any(|value| value == "text.split"));
    }

    #[test]
    fn completion_builder_skips_cpython_proxy_values() {
        let mut vm = super::build_vm(false, true).expect("vm");
        super::execute_module_source(
            &mut vm,
            "class Proxy:\n    pass\nProxy.__pyrs_cpython_proxy_marker__ = True\nproxy = Proxy()\nproxy.__pyrs_cpython_proxy_ptr__ = 123\nproxy.field = 1\n",
            "<stdin>",
            false,
        )
        .expect("module execution should succeed");
        let state = super::build_completion_state(&vm);
        assert!(!state.members.contains_key("proxy"));
        assert!(!state.members.contains_key("Proxy"));
    }

    #[test]
    fn refresh_completion_symbols_updates_and_removes_bindings() {
        let mut vm = super::build_vm(false, true).expect("vm");
        let state = Arc::new(Mutex::new(super::build_completion_state(&vm)));

        vm.set_global("x", crate::runtime::Value::Int(1));
        super::refresh_completion_symbols(&vm, &state, &[String::from("x")]);
        {
            let guard = state.lock().expect("completion state");
            assert!(guard.symbols.iter().any(|symbol| symbol == "x"));
            assert!(guard.members.contains_key("x"));
        }

        super::execute_module_source(&mut vm, "del x\n", "<stdin>", false)
            .expect("delete should succeed");
        super::refresh_completion_symbols(&vm, &state, &[String::from("x")]);
        let guard = state.lock().expect("completion state");
        assert!(!guard.symbols.iter().any(|symbol| symbol == "x"));
        assert!(!guard.members.contains_key("x"));
    }

    #[test]
    fn python_highlighter_styles_keywords_strings_and_comments() {
        let highlighter = PythonHighlighter::new(repl_palette(ResolvedReplTheme::Dark));
        let styled = highlighter.highlight("if x == 'a': # comment", 0);
        assert_eq!(styled.raw_string(), "if x == 'a': # comment");
        assert!(styled.buffer.len() > 1);
    }

    #[test]
    fn python_highlighter_uses_terminal_default_for_plain_identifiers() {
        let highlighter = PythonHighlighter::new(repl_palette(ResolvedReplTheme::Dark));
        let styled = highlighter.highlight("identifier", 0);
        assert_eq!(styled.raw_string(), "identifier");
        assert!(!styled.buffer.is_empty());
        assert_eq!(styled.buffer[0].0, nu_ansi_term::Style::new());
    }

    fn style_at_byte(styled: &StyledText, byte_index: usize) -> Option<nu_ansi_term::Style> {
        let mut cursor = 0usize;
        for (style, segment) in &styled.buffer {
            let end = cursor + segment.len();
            if byte_index >= cursor && byte_index < end {
                return Some(*style);
            }
            cursor = end;
        }
        None
    }

    #[test]
    fn python_highlighter_covers_entire_string_literal_span() {
        let palette = repl_palette(ResolvedReplTheme::Dark);
        let highlighter = PythonHighlighter::new(palette);
        let line = "'abc'";
        let styled = highlighter.highlight(line, 0);
        assert_eq!(styled.raw_string(), line);

        let c_style = style_at_byte(&styled, 3).expect("style for last content char");
        let quote_style = style_at_byte(&styled, 4).expect("style for closing quote");
        assert_eq!(c_style, palette.string_style);
        assert_eq!(quote_style, palette.string_style);
    }

    #[test]
    fn python_highlighter_covers_prefixed_string_literal_span() {
        let palette = repl_palette(ResolvedReplTheme::Dark);
        let highlighter = PythonHighlighter::new(palette);
        let line = "b'abc'";
        let styled = highlighter.highlight(line, 0);
        assert_eq!(styled.raw_string(), line);

        let prefix_style = style_at_byte(&styled, 0).expect("style for prefix");
        let quote_style = style_at_byte(&styled, 5).expect("style for closing quote");
        assert_eq!(prefix_style, palette.string_style);
        assert_eq!(quote_style, palette.string_style);
    }

    #[test]
    fn python_highlighter_styles_class_and_builtin_type_names() {
        let palette = repl_palette(ResolvedReplTheme::Dark);
        let highlighter = PythonHighlighter::new(palette);
        let line = "class User: x: int";
        let styled = highlighter.highlight(line, 0);
        assert_eq!(styled.raw_string(), line);

        let class_name_style = style_at_byte(&styled, 6).expect("style for class name");
        let type_name_style = style_at_byte(&styled, 15).expect("style for builtin type");
        assert_eq!(class_name_style, palette.class_name_style);
        assert_eq!(type_name_style, palette.type_name_style);
    }

    #[test]
    fn python_highlighter_styles_decorator_paths() {
        let palette = repl_palette(ResolvedReplTheme::Dark);
        let highlighter = PythonHighlighter::new(palette);
        let line = "@pkg.decorator";
        let styled = highlighter.highlight(line, 0);
        assert_eq!(styled.raw_string(), line);

        let at_style = style_at_byte(&styled, 0).expect("style for at-sign");
        let name_style = style_at_byte(&styled, 1).expect("style for decorator name");
        let dot_style = style_at_byte(&styled, 4).expect("style for decorator dot");
        assert_eq!(at_style, palette.decorator_style);
        assert_eq!(name_style, palette.decorator_style);
        assert_eq!(dot_style, palette.decorator_style);
    }

    #[test]
    fn repl_palette_differs_between_dark_and_light_modes() {
        let dark = repl_palette(ResolvedReplTheme::Dark);
        let light = repl_palette(ResolvedReplTheme::Light);
        assert_ne!(
            format!("{:?}", dark.hint_style),
            format!("{:?}", light.hint_style)
        );
        assert_ne!(
            format!("{:?}", dark.keyword_style),
            format!("{:?}", light.keyword_style)
        );
        assert_ne!(
            format!("{:?}", dark.class_name_style),
            format!("{:?}", light.class_name_style)
        );
        assert_ne!(
            format!("{:?}", dark.decorator_style),
            format!("{:?}", light.decorator_style)
        );
    }

    #[test]
    fn repl_expression_render_uses_python_repr_protocol() {
        let mut vm = super::build_vm(false, true).expect("vm");
        super::execute_module_source(
            &mut vm,
            "class A:\n    def __repr__(self):\n        return 'custom-repr'\na = A()\n",
            "<stdin>",
            false,
        )
        .expect("module execution should succeed");
        let value = vm.get_global("a").expect("global should exist");
        let rendered = vm
            .render_value_repr_for_display(value)
            .expect("repr should render");
        assert_eq!(rendered, "custom-repr");
    }
}
