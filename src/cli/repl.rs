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
    ColumnarMenu, Completer, DefaultHinter, Emacs, FileBackedHistory, KeyCode, KeyModifiers,
    MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, ReedlineMenu, Signal, Span, StyledText, Suggestion, default_emacs_keybindings,
};
use reedline::{EditCommand, Highlighter};

use crate::VERSION;
use crate::ast::{Module, StmtKind};
use crate::compiler;
use crate::parser::{self, ParseError};
use crate::runtime::{Object, Value, format_repr};
use crate::stdlib;
use crate::vm::Vm;

const HISTORY_CAPACITY: usize = 10_000;
const INDENT_WIDTH: usize = 4;
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

    let completion_state = Arc::new(Mutex::new(build_completion_state(vm)));
    load_repl_startup_script(vm, &completion_state)?;
    let mut line_editor = build_editor(Arc::clone(&completion_state))?;
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
                        eprintln!("{err}");
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
                            refresh_completion_state(vm, &completion_state);
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

fn build_editor(completion_state: Arc<Mutex<CompletionState>>) -> Result<Reedline, String> {
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
    keybindings.add_binding(KeyModifiers::NONE, KeyCode::Esc, ReedlineEvent::Esc);
    let edit_mode = Box::new(Emacs::new(keybindings));
    let history_hint_style = Style::new().italic().fg(AnsiColor::DarkGray);
    let mut editor = Reedline::create()
        .with_hinter(Box::new(
            DefaultHinter::default().with_style(history_hint_style),
        ))
        .with_highlighter(Box::new(PythonHighlighter::default()))
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

fn build_completion_state(vm: &Vm) -> CompletionState {
    const MAX_COMPLETION_DEPTH: usize = 6;
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

#[derive(Default)]
struct PythonHighlighter;

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
        for token in tokens {
            let token_start = token.offset.min(line.len());
            let token_end = token_start
                .saturating_add(token.lexeme.len())
                .min(line.len());

            if token_start > cursor {
                push_plain_or_comment(&mut styled, &line[cursor..token_start]);
            }
            if token_end > token_start {
                styled.push((
                    style_for_token_kind(&token.kind),
                    line[token_start..token_end].to_string(),
                ));
            }
            cursor = token_end.max(cursor);
        }

        if cursor < line.len() {
            push_plain_or_comment(&mut styled, &line[cursor..]);
        }
        if styled.buffer.is_empty() {
            styled.push((Style::new(), line.to_string()));
        }
        styled
    }
}

fn style_for_token_kind(token_kind: &parser::token::TokenKind) -> Style {
    match token_kind {
        parser::token::TokenKind::Keyword(_) => Style::new().fg(AnsiColor::Cyan).bold(),
        parser::token::TokenKind::Number => Style::new().fg(AnsiColor::Purple),
        parser::token::TokenKind::String
        | parser::token::TokenKind::Bytes
        | parser::token::TokenKind::FString => Style::new().fg(AnsiColor::Green),
        parser::token::TokenKind::Name => Style::new().fg(AnsiColor::White),
        _ => Style::new().fg(AnsiColor::White),
    }
}

fn push_plain_or_comment(styled: &mut StyledText, segment: &str) {
    if segment.is_empty() {
        return;
    }
    if let Some(comment_start) = segment.find('#') {
        let (plain, comment) = segment.split_at(comment_start);
        if !plain.is_empty() {
            styled.push((Style::new(), plain.to_string()));
        }
        styled.push((Style::new().fg(AnsiColor::LightGreen), comment.to_string()));
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

fn execute_timeit_command(
    vm: &mut Vm,
    request: &TimeItRequest,
    filename: &str,
) -> Result<(), String> {
    const DEFAULT_REPEATS: u32 = 7;
    const TARGET_SECONDS: f64 = 0.2;
    const MAX_CALIBRATION_LOOPS: u64 = 1_000_000_000;

    let module = parser::parse_module(&request.source).map_err(|err| format_parse_error(&err))?;
    let code = compiler::compile_module_with_filename(&module, filename)
        .map_err(|err| format!("compile error: {}", err.message))?;
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
            loops = loops
                .saturating_mul(scale)
                .min(MAX_CALIBRATION_LOOPS)
                .max(1);
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

    use reedline::{Completer, Highlighter};

    use super::{
        CompletionState, MetaCommand, PythonHighlighter, ReplCompleter, ReplMagicCommand,
        TimeItRequest, completion_fragment, format_parse_error, is_path_like, parse_magic_command,
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
    fn python_highlighter_styles_keywords_strings_and_comments() {
        let highlighter = PythonHighlighter;
        let styled = highlighter.highlight("if x == 'a': # comment", 0);
        assert_eq!(styled.raw_string(), "if x == 'a': # comment");
        assert!(styled.buffer.len() > 1);
    }
}
