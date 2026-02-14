//! AST to bytecode compiler (minimal subset).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{
    AssignTarget, BinaryOp, BoolOp, CallArg, ComprehensionClause, Constant, DictEntry,
    ExceptHandler, Expr, ExprKind, MatchCase, Module, Parameter, Pattern, Span, Stmt, StmtKind,
    UnaryOp,
};
use crate::bytecode::{CodeObject, Instruction, Opcode};
use crate::runtime::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
}

impl CompileError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ScopeType {
    Module,
    Function,
    Class,
    Lambda,
}

#[derive(Debug, Clone)]
struct ScopeInfo {
    scope_type: ScopeType,
    locals: HashSet<String>,
    globals: HashSet<String>,
    cellvars: Vec<String>,
    freevars: Vec<String>,
    cellvar_set: HashSet<String>,
    freevar_set: HashSet<String>,
    available_nonlocal: HashSet<String>,
}

impl ScopeInfo {
    fn for_module(module: &Module) -> Result<Self, CompileError> {
        analyze_scope(
            ScopeType::Module,
            &[],
            &[],
            &[],
            None,
            None,
            &module.body,
            &HashSet::new(),
        )
    }

    fn for_class(body: &[Stmt], enclosing: &ScopeInfo) -> Result<Self, CompileError> {
        analyze_scope(
            ScopeType::Class,
            &[],
            &[],
            &[],
            None,
            None,
            body,
            &enclosing.available_nonlocal,
        )
    }

    fn for_function(
        posonly_params: &[Parameter],
        params: &[Parameter],
        kwonly_params: &[Parameter],
        vararg: &Option<Parameter>,
        kwarg: &Option<Parameter>,
        body: &[Stmt],
        enclosing: &ScopeInfo,
    ) -> Result<Self, CompileError> {
        analyze_scope(
            ScopeType::Function,
            posonly_params,
            params,
            kwonly_params,
            vararg.as_ref(),
            kwarg.as_ref(),
            body,
            &enclosing.available_nonlocal,
        )
    }

    fn is_local(&self, name: &str) -> bool {
        self.locals.contains(name)
    }

    fn is_cell(&self, name: &str) -> bool {
        self.cellvar_set.contains(name)
    }

    fn is_free(&self, name: &str) -> bool {
        self.freevar_set.contains(name)
    }

    fn is_global(&self, name: &str) -> bool {
        self.globals.contains(name)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum NameKind {
    Local,
    Cell,
    Free,
    Global,
    Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IrrefutablePatternKind {
    Wildcard,
    Capture(String),
}

impl IrrefutablePatternKind {
    fn unreachable_message(&self) -> String {
        match self {
            IrrefutablePatternKind::Wildcard => {
                "wildcard makes remaining patterns unreachable".to_string()
            }
            IrrefutablePatternKind::Capture(name) => {
                format!("name capture '{name}' makes remaining patterns unreachable")
            }
        }
    }
}

fn analyze_scope(
    scope_type: ScopeType,
    posonly_params: &[Parameter],
    params: &[Parameter],
    kwonly_params: &[Parameter],
    vararg: Option<&Parameter>,
    kwarg: Option<&Parameter>,
    body: &[Stmt],
    enclosing: &HashSet<String>,
) -> Result<ScopeInfo, CompileError> {
    let mut locals = HashSet::new();
    let mut globals = HashSet::new();
    let mut nonlocals = HashSet::new();

    collect_param_locals(
        posonly_params,
        params,
        kwonly_params,
        vararg,
        kwarg,
        &mut locals,
    );

    for stmt in body {
        collect_locals_stmt(stmt, &mut locals, &mut globals, &mut nonlocals);
    }

    if !matches!(scope_type, ScopeType::Function | ScopeType::Lambda) && !nonlocals.is_empty() {
        return Err(CompileError::new(
            "nonlocal declarations only allowed in function scopes",
        ));
    }

    for name in &nonlocals {
        if !enclosing.contains(name) {
            return Err(CompileError::new(format!(
                "no binding for nonlocal '{name}' found"
            )));
        }
    }

    locals.retain(|name| !globals.contains(name) && !nonlocals.contains(name));

    let mut available_nonlocal = enclosing.clone();
    if matches!(scope_type, ScopeType::Function | ScopeType::Lambda) {
        for name in &locals {
            available_nonlocal.insert(name.clone());
        }
    }

    let mut uses = HashSet::new();
    let mut child_free = HashSet::new();
    for stmt in body {
        collect_uses_stmt(stmt, &mut uses, &mut child_free, &available_nonlocal)?;
    }

    let mut direct_free: HashSet<String> =
        uses.intersection(&available_nonlocal).cloned().collect();
    for name in &nonlocals {
        direct_free.insert(name.clone());
    }
    direct_free.retain(|name| !locals.contains(name) && !globals.contains(name));

    let mut cellvar_set = HashSet::new();
    let mut freevar_set = HashSet::new();

    match scope_type {
        ScopeType::Function | ScopeType::Lambda => {
            for name in child_free {
                if locals.contains(&name) {
                    cellvar_set.insert(name);
                } else {
                    freevar_set.insert(name);
                }
            }
            for name in direct_free {
                freevar_set.insert(name);
            }
        }
        ScopeType::Module | ScopeType::Class => {
            for name in child_free {
                freevar_set.insert(name);
            }
        }
    }

    let mut cellvars: Vec<String> = cellvar_set.iter().cloned().collect();
    cellvars.sort();
    let mut freevars: Vec<String> = freevar_set.iter().cloned().collect();
    freevars.sort();

    Ok(ScopeInfo {
        scope_type,
        locals,
        globals,
        cellvars,
        freevars,
        cellvar_set,
        freevar_set,
        available_nonlocal,
    })
}

fn analyze_scope_expr(
    scope_type: ScopeType,
    posonly_params: &[Parameter],
    params: &[Parameter],
    kwonly_params: &[Parameter],
    vararg: Option<&Parameter>,
    kwarg: Option<&Parameter>,
    body: &Expr,
    enclosing: &HashSet<String>,
) -> Result<ScopeInfo, CompileError> {
    let stmt = Stmt {
        node: StmtKind::Expr(body.clone()),
        span: body.span,
    };
    analyze_scope(
        scope_type,
        posonly_params,
        params,
        kwonly_params,
        vararg,
        kwarg,
        std::slice::from_ref(&stmt),
        enclosing,
    )
}

fn collect_param_locals(
    posonly_params: &[Parameter],
    params: &[Parameter],
    kwonly_params: &[Parameter],
    vararg: Option<&Parameter>,
    kwarg: Option<&Parameter>,
    locals: &mut HashSet<String>,
) {
    for param in posonly_params {
        locals.insert(param.name.clone());
    }
    for param in params {
        locals.insert(param.name.clone());
    }
    for param in kwonly_params {
        locals.insert(param.name.clone());
    }
    if let Some(param) = vararg {
        locals.insert(param.name.clone());
    }
    if let Some(param) = kwarg {
        locals.insert(param.name.clone());
    }
}

fn collect_locals_stmt(
    stmt: &Stmt,
    locals: &mut HashSet<String>,
    globals: &mut HashSet<String>,
    nonlocals: &mut HashSet<String>,
) {
    match &stmt.node {
        StmtKind::Assign { targets, .. } => {
            for target in targets {
                collect_locals_target(target, locals);
            }
        }
        StmtKind::AugAssign { target, .. } | StmtKind::AnnAssign { target, .. } => {
            collect_locals_target(target, locals);
        }
        StmtKind::Delete { targets } => {
            for target in targets {
                collect_locals_target(target, locals);
            }
        }
        StmtKind::If { body, orelse, .. } => {
            for stmt in body {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
            for stmt in orelse {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
        }
        StmtKind::While { body, orelse, .. } => {
            for stmt in body {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
            for stmt in orelse {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
        }
        StmtKind::For {
            target,
            body,
            orelse,
            ..
        } => {
            collect_locals_target(target, locals);
            for stmt in body {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
            for stmt in orelse {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
        }
        StmtKind::With { target, body, .. } => {
            if let Some(target) = target {
                collect_locals_target(target, locals);
            }
            for stmt in body {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
        }
        StmtKind::FunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => {
            locals.insert(name.clone());
        }
        StmtKind::Import { names } => {
            for alias in names {
                let binding = alias.asname.clone().unwrap_or_else(|| {
                    alias
                        .name
                        .split('.')
                        .next()
                        .unwrap_or(&alias.name)
                        .to_string()
                });
                locals.insert(binding);
            }
        }
        StmtKind::ImportFrom { names, .. } => {
            for alias in names {
                let binding = alias.asname.clone().unwrap_or_else(|| alias.name.clone());
                locals.insert(binding);
            }
        }
        StmtKind::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for stmt in body {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
            for stmt in orelse {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
            for stmt in finalbody {
                collect_locals_stmt(stmt, locals, globals, nonlocals);
            }
            for handler in handlers {
                if let Some(name) = &handler.name {
                    locals.insert(name.clone());
                }
                for stmt in &handler.body {
                    collect_locals_stmt(stmt, locals, globals, nonlocals);
                }
            }
        }
        StmtKind::Match { cases, .. } => {
            for case in cases {
                collect_pattern_locals(&case.pattern, locals);
                for stmt in &case.body {
                    collect_locals_stmt(stmt, locals, globals, nonlocals);
                }
            }
        }
        StmtKind::Decorated { stmt, .. } => {
            collect_locals_stmt(stmt, locals, globals, nonlocals);
        }
        StmtKind::Global { names } => {
            for name in names {
                globals.insert(name.clone());
            }
        }
        StmtKind::Nonlocal { names } => {
            for name in names {
                nonlocals.insert(name.clone());
            }
        }
        _ => {}
    }
    collect_locals_namedexpr_stmt(stmt, locals);
}

fn collect_pattern_locals(pattern: &Pattern, locals: &mut HashSet<String>) {
    match pattern {
        Pattern::Capture(name) => {
            locals.insert(name.clone());
        }
        Pattern::Sequence(items) | Pattern::Or(items) => {
            for item in items {
                collect_pattern_locals(item, locals);
            }
        }
        Pattern::Mapping { entries, rest } => {
            for (_, value) in entries {
                collect_pattern_locals(value, locals);
            }
            if let Some(name) = rest {
                locals.insert(name.clone());
            }
        }
        Pattern::Class {
            positional,
            keywords,
            ..
        } => {
            for pattern in positional {
                collect_pattern_locals(pattern, locals);
            }
            for (_, pattern) in keywords {
                collect_pattern_locals(pattern, locals);
            }
        }
        Pattern::As { pattern, name } => {
            collect_pattern_locals(pattern, locals);
            locals.insert(name.clone());
        }
        Pattern::Star(Some(name)) => {
            locals.insert(name.clone());
        }
        Pattern::Wildcard | Pattern::Constant(_) | Pattern::Value(_) | Pattern::Star(None) => {}
    }
}

fn collect_locals_target(target: &AssignTarget, locals: &mut HashSet<String>) {
    match target {
        AssignTarget::Name(name) => {
            locals.insert(name.clone());
        }
        AssignTarget::Starred(item) => {
            collect_locals_target(item, locals);
        }
        AssignTarget::Tuple(items) | AssignTarget::List(items) => {
            for item in items {
                collect_locals_target(item, locals);
            }
        }
        AssignTarget::Subscript { .. } | AssignTarget::Attribute { .. } => {}
    }
}

fn collect_locals_namedexpr_stmt(stmt: &Stmt, locals: &mut HashSet<String>) {
    match &stmt.node {
        StmtKind::Expr(expr) => collect_locals_namedexpr_expr(expr, locals),
        StmtKind::Assign { targets, value } => {
            for target in targets {
                collect_locals_namedexpr_target(target, locals);
            }
            collect_locals_namedexpr_expr(value, locals);
        }
        StmtKind::AugAssign { target, value, .. } => {
            collect_locals_namedexpr_target(target, locals);
            collect_locals_namedexpr_expr(value, locals);
        }
        StmtKind::AnnAssign {
            target,
            annotation,
            value,
        } => {
            collect_locals_namedexpr_target(target, locals);
            collect_locals_namedexpr_expr(annotation, locals);
            if let Some(value) = value {
                collect_locals_namedexpr_expr(value, locals);
            }
        }
        StmtKind::Delete { targets } => {
            for target in targets {
                collect_locals_namedexpr_target(target, locals);
            }
        }
        StmtKind::If { test, .. }
        | StmtKind::While { test, .. }
        | StmtKind::Assert { test, .. } => collect_locals_namedexpr_expr(test, locals),
        StmtKind::For { target, iter, .. } => {
            collect_locals_namedexpr_target(target, locals);
            collect_locals_namedexpr_expr(iter, locals);
        }
        StmtKind::With {
            context, target, ..
        } => {
            collect_locals_namedexpr_expr(context, locals);
            if let Some(target) = target {
                collect_locals_namedexpr_target(target, locals);
            }
        }
        StmtKind::Return { value } => {
            if let Some(value) = value {
                collect_locals_namedexpr_expr(value, locals);
            }
        }
        StmtKind::Raise { value, cause } => {
            if let Some(value) = value {
                collect_locals_namedexpr_expr(value, locals);
            }
            if let Some(cause) = cause {
                collect_locals_namedexpr_expr(cause, locals);
            }
        }
        StmtKind::Try { handlers, .. } => {
            for handler in handlers {
                if let Some(type_expr) = &handler.type_expr {
                    collect_locals_namedexpr_expr(type_expr, locals);
                }
            }
        }
        StmtKind::FunctionDef {
            posonly_params,
            params,
            vararg,
            kwarg,
            kwonly_params,
            returns,
            ..
        } => {
            collect_locals_namedexpr_params(
                posonly_params,
                params,
                kwonly_params,
                vararg.as_ref(),
                kwarg.as_ref(),
                locals,
            );
            if let Some(returns) = returns {
                collect_locals_namedexpr_expr(returns, locals);
            }
        }
        StmtKind::ClassDef {
            bases,
            metaclass,
            keywords,
            ..
        } => {
            for base in bases {
                collect_locals_namedexpr_expr(base, locals);
            }
            if let Some(metaclass) = metaclass {
                collect_locals_namedexpr_expr(metaclass, locals);
            }
            for (_, value) in keywords {
                collect_locals_namedexpr_expr(value, locals);
            }
        }
        StmtKind::Decorated { decorators, .. } => {
            for decorator in decorators {
                collect_locals_namedexpr_expr(decorator, locals);
            }
        }
        StmtKind::Match { subject, cases } => {
            collect_locals_namedexpr_expr(subject, locals);
            for case in cases {
                if let Some(guard) = &case.guard {
                    collect_locals_namedexpr_expr(guard, locals);
                }
            }
        }
        StmtKind::Pass
        | StmtKind::Import { .. }
        | StmtKind::ImportFrom { .. }
        | StmtKind::Global { .. }
        | StmtKind::Nonlocal { .. }
        | StmtKind::Break
        | StmtKind::Continue => {}
    }
}

fn collect_locals_namedexpr_params(
    posonly_params: &[Parameter],
    params: &[Parameter],
    kwonly_params: &[Parameter],
    vararg: Option<&Parameter>,
    kwarg: Option<&Parameter>,
    locals: &mut HashSet<String>,
) {
    for param in posonly_params
        .iter()
        .chain(params.iter())
        .chain(kwonly_params.iter())
    {
        if let Some(default) = &param.default {
            collect_locals_namedexpr_expr(default, locals);
        }
        if let Some(annotation) = &param.annotation {
            collect_locals_namedexpr_expr(annotation, locals);
        }
    }
    if let Some(param) = vararg {
        if let Some(annotation) = &param.annotation {
            collect_locals_namedexpr_expr(annotation, locals);
        }
    }
    if let Some(param) = kwarg {
        if let Some(annotation) = &param.annotation {
            collect_locals_namedexpr_expr(annotation, locals);
        }
    }
}

fn collect_locals_namedexpr_target(target: &AssignTarget, locals: &mut HashSet<String>) {
    match target {
        AssignTarget::Name(_) => {}
        AssignTarget::Starred(item) => collect_locals_namedexpr_target(item, locals),
        AssignTarget::Tuple(items) | AssignTarget::List(items) => {
            for item in items {
                collect_locals_namedexpr_target(item, locals);
            }
        }
        AssignTarget::Subscript { value, index } => {
            collect_locals_namedexpr_expr(value, locals);
            collect_locals_namedexpr_expr(index, locals);
        }
        AssignTarget::Attribute { value, .. } => {
            collect_locals_namedexpr_expr(value, locals);
        }
    }
}

fn collect_locals_namedexpr_expr(expr: &Expr, locals: &mut HashSet<String>) {
    match &expr.node {
        ExprKind::Name(_) | ExprKind::Constant(_) => {}
        ExprKind::NamedExpr { target, value } => {
            locals.insert(target.clone());
            collect_locals_namedexpr_expr(value, locals);
        }
        ExprKind::Binary { left, right, .. } | ExprKind::BoolOp { left, right, .. } => {
            collect_locals_namedexpr_expr(left, locals);
            collect_locals_namedexpr_expr(right, locals);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Await { value: operand } => {
            collect_locals_namedexpr_expr(operand, locals);
        }
        ExprKind::Call { func, args } => {
            collect_locals_namedexpr_expr(func, locals);
            for arg in args {
                match arg {
                    CallArg::Positional(expr)
                    | CallArg::Keyword { value: expr, .. }
                    | CallArg::Star(expr)
                    | CallArg::DoubleStar(expr) => collect_locals_namedexpr_expr(expr, locals),
                }
            }
        }
        ExprKind::List(values) | ExprKind::Tuple(values) => {
            for value in values {
                collect_locals_namedexpr_expr(value, locals);
            }
        }
        ExprKind::Dict(entries) => {
            for entry in entries {
                match entry {
                    DictEntry::Pair(key, value) => {
                        collect_locals_namedexpr_expr(key, locals);
                        collect_locals_namedexpr_expr(value, locals);
                    }
                    DictEntry::Unpack(value) => collect_locals_namedexpr_expr(value, locals),
                }
            }
        }
        ExprKind::Subscript { value, index } => {
            collect_locals_namedexpr_expr(value, locals);
            collect_locals_namedexpr_expr(index, locals);
        }
        ExprKind::Attribute { value, .. } => collect_locals_namedexpr_expr(value, locals),
        ExprKind::IfExpr { test, body, orelse } => {
            collect_locals_namedexpr_expr(test, locals);
            collect_locals_namedexpr_expr(body, locals);
            collect_locals_namedexpr_expr(orelse, locals);
        }
        ExprKind::Lambda {
            posonly_params,
            params,
            kwonly_params,
            vararg,
            kwarg,
            ..
        } => collect_locals_namedexpr_params(
            posonly_params,
            params,
            kwonly_params,
            vararg.as_ref(),
            kwarg.as_ref(),
            locals,
        ),
        ExprKind::ListComp { elt, clauses } | ExprKind::GeneratorExp { elt, clauses } => {
            collect_locals_namedexpr_expr(elt, locals);
            for clause in clauses {
                collect_locals_namedexpr_target(&clause.target, locals);
                collect_locals_namedexpr_expr(&clause.iter, locals);
                for cond in &clause.ifs {
                    collect_locals_namedexpr_expr(cond, locals);
                }
            }
        }
        ExprKind::DictComp {
            key,
            value,
            clauses,
        } => {
            collect_locals_namedexpr_expr(key, locals);
            collect_locals_namedexpr_expr(value, locals);
            for clause in clauses {
                collect_locals_namedexpr_target(&clause.target, locals);
                collect_locals_namedexpr_expr(&clause.iter, locals);
                for cond in &clause.ifs {
                    collect_locals_namedexpr_expr(cond, locals);
                }
            }
        }
        ExprKind::Yield { value } => {
            if let Some(value) = value {
                collect_locals_namedexpr_expr(value, locals);
            }
        }
        ExprKind::YieldFrom { value } => collect_locals_namedexpr_expr(value, locals),
        ExprKind::Slice { lower, upper, step } => {
            if let Some(lower) = lower {
                collect_locals_namedexpr_expr(lower, locals);
            }
            if let Some(upper) = upper {
                collect_locals_namedexpr_expr(upper, locals);
            }
            if let Some(step) = step {
                collect_locals_namedexpr_expr(step, locals);
            }
        }
    }
}

fn collect_uses_stmt(
    stmt: &Stmt,
    uses: &mut HashSet<String>,
    child_free: &mut HashSet<String>,
    enclosing: &HashSet<String>,
) -> Result<(), CompileError> {
    match &stmt.node {
        StmtKind::Expr(expr) => collect_uses_expr(expr, uses, child_free, enclosing)?,
        StmtKind::Assign { targets, value } => {
            for target in targets {
                collect_target_uses(target, uses, child_free, enclosing)?;
            }
            collect_uses_expr(value, uses, child_free, enclosing)?;
        }
        StmtKind::AnnAssign {
            target,
            annotation,
            value,
        } => {
            collect_target_uses(target, uses, child_free, enclosing)?;
            collect_uses_expr(annotation, uses, child_free, enclosing)?;
            if let Some(expr) = value {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
        }
        StmtKind::AugAssign { target, value, .. } => {
            collect_target_uses(target, uses, child_free, enclosing)?;
            collect_uses_expr(value, uses, child_free, enclosing)?;
        }
        StmtKind::Delete { targets } => {
            for target in targets {
                collect_target_uses(target, uses, child_free, enclosing)?;
            }
        }
        StmtKind::If { test, body, orelse } => {
            collect_uses_expr(test, uses, child_free, enclosing)?;
            for stmt in body {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
            for stmt in orelse {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
        }
        StmtKind::While { test, body, orelse } => {
            collect_uses_expr(test, uses, child_free, enclosing)?;
            for stmt in body {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
            for stmt in orelse {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
        }
        StmtKind::For {
            target,
            iter,
            body,
            orelse,
            ..
        } => {
            collect_target_uses(target, uses, child_free, enclosing)?;
            collect_uses_expr(iter, uses, child_free, enclosing)?;
            for stmt in body {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
            for stmt in orelse {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
        }
        StmtKind::With {
            context,
            target,
            body,
            ..
        } => {
            collect_uses_expr(context, uses, child_free, enclosing)?;
            if let Some(target) = target {
                collect_target_uses(target, uses, child_free, enclosing)?;
            }
            for stmt in body {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
        }
        StmtKind::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for stmt in body {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
            for handler in handlers {
                if let Some(expr) = &handler.type_expr {
                    collect_uses_expr(expr, uses, child_free, enclosing)?;
                }
                for stmt in &handler.body {
                    collect_uses_stmt(stmt, uses, child_free, enclosing)?;
                }
            }
            for stmt in orelse {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
            for stmt in finalbody {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
        }
        StmtKind::Return { value } => {
            if let Some(expr) = value {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
        }
        StmtKind::Raise { value, cause } => {
            if let Some(expr) = value {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
            if let Some(expr) = cause {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
        }
        StmtKind::Assert { test, message } => {
            collect_uses_expr(test, uses, child_free, enclosing)?;
            if let Some(expr) = message {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
        }
        StmtKind::FunctionDef {
            posonly_params,
            params,
            kwonly_params,
            vararg,
            kwarg,
            returns,
            body,
            ..
        } => {
            for param in posonly_params
                .iter()
                .chain(params.iter())
                .chain(kwonly_params.iter())
            {
                if let Some(default) = &param.default {
                    collect_uses_expr(default, uses, child_free, enclosing)?;
                }
                if let Some(annotation) = &param.annotation {
                    collect_uses_expr(annotation, uses, child_free, enclosing)?;
                }
            }
            for param in vararg.iter().chain(kwarg.iter()) {
                if let Some(annotation) = &param.annotation {
                    collect_uses_expr(annotation, uses, child_free, enclosing)?;
                }
            }
            if let Some(annotation) = returns {
                collect_uses_expr(annotation, uses, child_free, enclosing)?;
            }
            let scope = analyze_scope(
                ScopeType::Function,
                posonly_params,
                params,
                kwonly_params,
                vararg.as_ref(),
                kwarg.as_ref(),
                body,
                enclosing,
            )?;
            child_free.extend(scope.freevars.into_iter());
        }
        StmtKind::ClassDef {
            bases,
            metaclass,
            keywords,
            body,
            ..
        } => {
            for base in bases {
                collect_uses_expr(base, uses, child_free, enclosing)?;
            }
            if let Some(meta) = metaclass {
                collect_uses_expr(meta, uses, child_free, enclosing)?;
            }
            for (_name, value) in keywords {
                collect_uses_expr(value, uses, child_free, enclosing)?;
            }
            let scope =
                analyze_scope(ScopeType::Class, &[], &[], &[], None, None, body, enclosing)?;
            child_free.extend(scope.freevars.into_iter());
        }
        StmtKind::Decorated { decorators, stmt } => {
            for decorator in decorators {
                collect_uses_expr(decorator, uses, child_free, enclosing)?;
            }
            collect_uses_stmt(stmt, uses, child_free, enclosing)?;
        }
        StmtKind::Match { subject, cases } => {
            collect_uses_expr(subject, uses, child_free, enclosing)?;
            for case in cases {
                if let Some(guard) = &case.guard {
                    collect_uses_expr(guard, uses, child_free, enclosing)?;
                }
                for stmt in &case.body {
                    collect_uses_stmt(stmt, uses, child_free, enclosing)?;
                }
            }
        }
        StmtKind::Import { .. }
        | StmtKind::ImportFrom { .. }
        | StmtKind::Global { .. }
        | StmtKind::Nonlocal { .. }
        | StmtKind::Pass
        | StmtKind::Break
        | StmtKind::Continue => {}
    }
    Ok(())
}

fn collect_target_uses(
    target: &AssignTarget,
    uses: &mut HashSet<String>,
    child_free: &mut HashSet<String>,
    enclosing: &HashSet<String>,
) -> Result<(), CompileError> {
    match target {
        AssignTarget::Starred(item) => {
            collect_target_uses(item, uses, child_free, enclosing)?;
        }
        AssignTarget::Subscript { value, index } => {
            collect_uses_expr(value, uses, child_free, enclosing)?;
            collect_uses_expr(index, uses, child_free, enclosing)?;
        }
        AssignTarget::Attribute { value, .. } => {
            collect_uses_expr(value, uses, child_free, enclosing)?;
        }
        AssignTarget::Tuple(items) | AssignTarget::List(items) => {
            for item in items {
                collect_target_uses(item, uses, child_free, enclosing)?;
            }
        }
        AssignTarget::Name(_) => {}
    }
    Ok(())
}

fn collect_uses_expr(
    expr: &Expr,
    uses: &mut HashSet<String>,
    child_free: &mut HashSet<String>,
    enclosing: &HashSet<String>,
) -> Result<(), CompileError> {
    match &expr.node {
        ExprKind::Name(name) => {
            uses.insert(name.clone());
        }
        ExprKind::Constant(_) => {}
        ExprKind::Binary { left, right, .. } => {
            collect_uses_expr(left, uses, child_free, enclosing)?;
            collect_uses_expr(right, uses, child_free, enclosing)?;
        }
        ExprKind::Unary { operand, .. } => {
            collect_uses_expr(operand, uses, child_free, enclosing)?;
        }
        ExprKind::Call { func, args } => {
            collect_uses_expr(func, uses, child_free, enclosing)?;
            for arg in args {
                match arg {
                    CallArg::Positional(expr) | CallArg::Star(expr) | CallArg::DoubleStar(expr) => {
                        collect_uses_expr(expr, uses, child_free, enclosing)?;
                    }
                    CallArg::Keyword { value, .. } => {
                        collect_uses_expr(value, uses, child_free, enclosing)?;
                    }
                }
            }
        }
        ExprKind::List(values) | ExprKind::Tuple(values) => {
            for value in values {
                collect_uses_expr(value, uses, child_free, enclosing)?;
            }
        }
        ExprKind::Dict(entries) => {
            for entry in entries {
                match entry {
                    DictEntry::Pair(key, value) => {
                        collect_uses_expr(key, uses, child_free, enclosing)?;
                        collect_uses_expr(value, uses, child_free, enclosing)?;
                    }
                    DictEntry::Unpack(value) => {
                        collect_uses_expr(value, uses, child_free, enclosing)?;
                    }
                }
            }
        }
        ExprKind::Subscript { value, index } => {
            collect_uses_expr(value, uses, child_free, enclosing)?;
            collect_uses_expr(index, uses, child_free, enclosing)?;
        }
        ExprKind::Attribute { value, .. } => {
            collect_uses_expr(value, uses, child_free, enclosing)?;
        }
        ExprKind::BoolOp { left, right, .. } => {
            collect_uses_expr(left, uses, child_free, enclosing)?;
            collect_uses_expr(right, uses, child_free, enclosing)?;
        }
        ExprKind::IfExpr { test, body, orelse } => {
            collect_uses_expr(test, uses, child_free, enclosing)?;
            collect_uses_expr(body, uses, child_free, enclosing)?;
            collect_uses_expr(orelse, uses, child_free, enclosing)?;
        }
        ExprKind::NamedExpr { value, .. } => {
            collect_uses_expr(value, uses, child_free, enclosing)?;
        }
        ExprKind::Lambda {
            posonly_params,
            params,
            kwonly_params,
            vararg,
            kwarg,
            body,
        } => {
            for param in posonly_params
                .iter()
                .chain(params.iter())
                .chain(kwonly_params.iter())
            {
                if let Some(default) = &param.default {
                    collect_uses_expr(default, uses, child_free, enclosing)?;
                }
            }
            let scope = analyze_scope_expr(
                ScopeType::Lambda,
                posonly_params,
                params,
                kwonly_params,
                vararg.as_ref(),
                kwarg.as_ref(),
                body,
                enclosing,
            )?;
            child_free.extend(scope.freevars.into_iter());
        }
        ExprKind::Yield { value } => {
            if let Some(expr) = value.as_ref() {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
        }
        ExprKind::YieldFrom { value } => {
            collect_uses_expr(value, uses, child_free, enclosing)?;
        }
        ExprKind::Await { value } => {
            collect_uses_expr(value, uses, child_free, enclosing)?;
        }
        ExprKind::ListComp { elt, clauses } => {
            let body = build_list_comp_body(elt, clauses);
            let scope = analyze_scope(
                ScopeType::Function,
                &[],
                &[],
                &[],
                None,
                None,
                &body,
                enclosing,
            )?;
            child_free.extend(scope.freevars.into_iter());
        }
        ExprKind::GeneratorExp { elt, clauses } => {
            let body = build_genexpr_body(elt, clauses);
            let scope = analyze_scope(
                ScopeType::Function,
                &[],
                &[],
                &[],
                None,
                None,
                &body,
                enclosing,
            )?;
            child_free.extend(scope.freevars.into_iter());
        }
        ExprKind::DictComp {
            key,
            value,
            clauses,
        } => {
            let body = build_dict_comp_body(key, value, clauses);
            let scope = analyze_scope(
                ScopeType::Function,
                &[],
                &[],
                &[],
                None,
                None,
                &body,
                enclosing,
            )?;
            child_free.extend(scope.freevars.into_iter());
        }
        ExprKind::Slice { lower, upper, step } => {
            if let Some(expr) = lower.as_ref() {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
            if let Some(expr) = upper.as_ref() {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
            if let Some(expr) = step.as_ref() {
                collect_uses_expr(expr, uses, child_free, enclosing)?;
            }
        }
    }
    Ok(())
}

fn body_has_ann_assign(body: &[Stmt]) -> bool {
    for stmt in body {
        match &stmt.node {
            StmtKind::AnnAssign { .. } => return true,
            StmtKind::If { body, orelse, .. } => {
                if body_has_ann_assign(body) || body_has_ann_assign(orelse) {
                    return true;
                }
            }
            StmtKind::While { body, orelse, .. } => {
                if body_has_ann_assign(body) || body_has_ann_assign(orelse) {
                    return true;
                }
            }
            StmtKind::For { body, orelse, .. } => {
                if body_has_ann_assign(body) || body_has_ann_assign(orelse) {
                    return true;
                }
            }
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                if body_has_ann_assign(body)
                    || body_has_ann_assign(orelse)
                    || body_has_ann_assign(finalbody)
                {
                    return true;
                }
                for handler in handlers {
                    if body_has_ann_assign(&handler.body) {
                        return true;
                    }
                }
            }
            StmtKind::With { body, .. } => {
                if body_has_ann_assign(body) {
                    return true;
                }
            }
            StmtKind::Decorated { stmt, .. } => {
                if body_has_ann_assign(std::slice::from_ref(stmt)) {
                    return true;
                }
            }
            StmtKind::Match { cases, .. } => {
                for case in cases {
                    if body_has_ann_assign(&case.body) {
                        return true;
                    }
                }
            }
            StmtKind::FunctionDef { .. } | StmtKind::ClassDef { .. } => {}
            _ => {}
        }
    }
    false
}

fn body_has_yield(body: &[Stmt]) -> bool {
    for stmt in body {
        match &stmt.node {
            StmtKind::Expr(expr) => {
                if expr_has_yield(expr) {
                    return true;
                }
            }
            StmtKind::Assign { value, .. } => {
                if expr_has_yield(value) {
                    return true;
                }
            }
            StmtKind::AnnAssign {
                annotation, value, ..
            } => {
                if expr_has_yield(annotation) || value.as_ref().map(expr_has_yield).unwrap_or(false)
                {
                    return true;
                }
            }
            StmtKind::AugAssign { value, .. } => {
                if expr_has_yield(value) {
                    return true;
                }
            }
            StmtKind::If { test, body, orelse } => {
                if expr_has_yield(test) || body_has_yield(body) || body_has_yield(orelse) {
                    return true;
                }
            }
            StmtKind::While { test, body, orelse } => {
                if expr_has_yield(test) || body_has_yield(body) || body_has_yield(orelse) {
                    return true;
                }
            }
            StmtKind::For {
                iter, body, orelse, ..
            } => {
                if expr_has_yield(iter) || body_has_yield(body) || body_has_yield(orelse) {
                    return true;
                }
            }
            StmtKind::With { context, body, .. } => {
                if expr_has_yield(context) || body_has_yield(body) {
                    return true;
                }
            }
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                if body_has_yield(body) || body_has_yield(orelse) || body_has_yield(finalbody) {
                    return true;
                }
                for handler in handlers {
                    if handler
                        .type_expr
                        .as_ref()
                        .map(expr_has_yield)
                        .unwrap_or(false)
                        || body_has_yield(&handler.body)
                    {
                        return true;
                    }
                }
            }
            StmtKind::Return { value } => {
                if value.as_ref().map(expr_has_yield).unwrap_or(false) {
                    return true;
                }
            }
            StmtKind::Raise { value, cause } => {
                if value.as_ref().map(expr_has_yield).unwrap_or(false)
                    || cause.as_ref().map(expr_has_yield).unwrap_or(false)
                {
                    return true;
                }
            }
            StmtKind::Assert { test, message } => {
                if expr_has_yield(test) || message.as_ref().map(expr_has_yield).unwrap_or(false) {
                    return true;
                }
            }
            StmtKind::Decorated { stmt, .. } => {
                if body_has_yield(std::slice::from_ref(stmt)) {
                    return true;
                }
            }
            StmtKind::Match { subject, cases } => {
                if expr_has_yield(subject) {
                    return true;
                }
                for case in cases {
                    if case.guard.as_ref().map(expr_has_yield).unwrap_or(false)
                        || body_has_yield(&case.body)
                    {
                        return true;
                    }
                }
            }
            StmtKind::FunctionDef { .. }
            | StmtKind::ClassDef { .. }
            | StmtKind::Import { .. }
            | StmtKind::ImportFrom { .. }
            | StmtKind::Global { .. }
            | StmtKind::Nonlocal { .. }
            | StmtKind::Delete { .. }
            | StmtKind::Pass
            | StmtKind::Break
            | StmtKind::Continue => {}
        }
    }
    false
}

fn expr_has_yield(expr: &Expr) -> bool {
    match &expr.node {
        ExprKind::Yield { .. } | ExprKind::YieldFrom { .. } => true,
        ExprKind::Binary { left, right, .. } | ExprKind::BoolOp { left, right, .. } => {
            expr_has_yield(left) || expr_has_yield(right)
        }
        ExprKind::Unary { operand, .. } => expr_has_yield(operand),
        ExprKind::Call { func, args } => {
            if expr_has_yield(func) {
                return true;
            }
            for arg in args {
                let has = match arg {
                    CallArg::Positional(expr)
                    | CallArg::Keyword { value: expr, .. }
                    | CallArg::Star(expr)
                    | CallArg::DoubleStar(expr) => expr_has_yield(expr),
                };
                if has {
                    return true;
                }
            }
            false
        }
        ExprKind::List(values) | ExprKind::Tuple(values) => values.iter().any(expr_has_yield),
        ExprKind::Dict(entries) => entries.iter().any(|entry| match entry {
            DictEntry::Pair(key, value) => expr_has_yield(key) || expr_has_yield(value),
            DictEntry::Unpack(value) => expr_has_yield(value),
        }),
        ExprKind::Subscript { value, index } => expr_has_yield(value) || expr_has_yield(index),
        ExprKind::Attribute { value, .. } => expr_has_yield(value),
        ExprKind::IfExpr { test, body, orelse } => {
            expr_has_yield(test) || expr_has_yield(body) || expr_has_yield(orelse)
        }
        ExprKind::NamedExpr { value, .. } | ExprKind::Await { value } => expr_has_yield(value),
        ExprKind::ListComp { elt, clauses } | ExprKind::GeneratorExp { elt, clauses } => {
            if expr_has_yield(elt) {
                return true;
            }
            clauses
                .iter()
                .any(|clause| expr_has_yield(&clause.iter) || clause.ifs.iter().any(expr_has_yield))
        }
        ExprKind::DictComp {
            key,
            value,
            clauses,
        } => {
            if expr_has_yield(key) || expr_has_yield(value) {
                return true;
            }
            clauses
                .iter()
                .any(|clause| expr_has_yield(&clause.iter) || clause.ifs.iter().any(expr_has_yield))
        }
        ExprKind::Lambda { .. } | ExprKind::Name(_) | ExprKind::Constant(_) => false,
        ExprKind::Slice { lower, upper, step } => {
            lower
                .as_ref()
                .map(|expr| expr_has_yield(expr))
                .unwrap_or(false)
                || upper
                    .as_ref()
                    .map(|expr| expr_has_yield(expr))
                    .unwrap_or(false)
                || step
                    .as_ref()
                    .map(|expr| expr_has_yield(expr))
                    .unwrap_or(false)
        }
    }
}

pub fn compile_module(module: &Module) -> Result<CodeObject, CompileError> {
    compile_module_with_filename(module, "<module>")
}

pub fn compile_expression(expr: &Expr) -> Result<CodeObject, CompileError> {
    compile_expression_with_filename(expr, "<string>")
}

pub fn compile_module_with_filename(
    module: &Module,
    filename: &str,
) -> Result<CodeObject, CompileError> {
    let scope = ScopeInfo::for_module(module)?;
    let mut compiler = Compiler::new("<module>", filename, scope);
    compiler.compile_module(module)?;
    Ok(compiler.finish())
}

pub fn compile_expression_with_filename(
    expr: &Expr,
    filename: &str,
) -> Result<CodeObject, CompileError> {
    let scope_module = Module {
        body: vec![Stmt {
            node: StmtKind::Expr(expr.clone()),
            span: expr.span,
        }],
    };
    let scope = ScopeInfo::for_module(&scope_module)?;
    let mut compiler = Compiler::new("<module>", filename, scope);
    compiler.compile_expr(expr)?;
    Ok(compiler.finish_expression())
}

struct Compiler {
    code: CodeObject,
    temp_counter: usize,
    loop_stack: Vec<LoopContext>,
    finally_return_stack: Vec<FinallyReturnContext>,
    scope: ScopeInfo,
    current_span: Span,
    cell_index: HashMap<String, u32>,
    future_annotations: bool,
}

struct LoopContext {
    start: usize,
    continue_target: Option<usize>,
    break_cleanup_pops: usize,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

#[derive(Debug, Clone)]
struct FinallyReturnContext {
    return_value_name: String,
    return_flag_name: String,
    pending_return_jumps: Vec<usize>,
}

impl Compiler {
    fn new(name: &str, filename: &str, scope: ScopeInfo) -> Self {
        let mut code = CodeObject::new(name, filename);
        code.cellvars = scope.cellvars.clone();
        code.freevars = scope.freevars.clone();
        let mut cell_index = HashMap::new();
        for (idx, name) in code.cellvars.iter().chain(code.freevars.iter()).enumerate() {
            cell_index.insert(name.clone(), idx as u32);
        }
        Self {
            code,
            temp_counter: 0,
            loop_stack: Vec::new(),
            finally_return_stack: Vec::new(),
            scope,
            current_span: Span::unknown(),
            cell_index,
            future_annotations: false,
        }
    }

    fn finish(mut self) -> CodeObject {
        self.emit(Opcode::LoadConst, Some(0));
        self.emit(Opcode::ReturnValue, None);
        self.code.rebuild_layout_indexes();
        self.code
    }

    fn finish_expression(mut self) -> CodeObject {
        self.emit(Opcode::ReturnValue, None);
        self.code.rebuild_layout_indexes();
        self.code
    }

    fn compile_module(&mut self, module: &Module) -> Result<(), CompileError> {
        self.future_annotations = self.validate_future_imports(&module.body)?;
        if body_has_ann_assign(&module.body) {
            self.init_annotations()?;
        }
        for stmt in &module.body {
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn validate_future_imports(&self, body: &[Stmt]) -> Result<bool, CompileError> {
        let mut idx = 0usize;
        if let Some(first) = body.first() {
            if matches!(
                first.node,
                StmtKind::Expr(Expr {
                    node: ExprKind::Constant(Constant::Str(_)),
                    ..
                })
            ) {
                idx = 1;
            }
        }

        let mut seen_non_future = false;
        let mut future_annotations = false;
        for stmt in body.iter().skip(idx) {
            match &stmt.node {
                StmtKind::ImportFrom {
                    module,
                    names,
                    level,
                } if *level == 0 => {
                    if module.as_deref() == Some("__future__") {
                        if seen_non_future {
                            return Err(CompileError::new(
                                "from __future__ imports must occur at the beginning of the file",
                            ));
                        }
                        for alias in names {
                            let name = alias.name.as_str();
                            if name == "annotations" {
                                future_annotations = true;
                            }
                            let known = matches!(
                                name,
                                "annotations"
                                    | "nested_scopes"
                                    | "generators"
                                    | "division"
                                    | "absolute_import"
                                    | "with_statement"
                                    | "print_function"
                                    | "unicode_literals"
                                    | "generator_stop"
                                    | "barry_as_FLUFL"
                            );
                            if !known {
                                return Err(CompileError::new(format!(
                                    "future feature '{}' is not defined",
                                    alias.name
                                )));
                            }
                        }
                        continue;
                    }
                    seen_non_future = true;
                }
                _ => {
                    seen_non_future = true;
                }
            }
        }
        Ok(future_annotations)
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        let span = stmt.span;
        self.with_span(span, |compiler| match &stmt.node {
            StmtKind::Pass => {
                compiler.emit(Opcode::Nop, None);
                Ok(())
            }
            StmtKind::Expr(expr) => {
                compiler.compile_expr(expr)?;
                compiler.emit(Opcode::PopTop, None);
                Ok(())
            }
            StmtKind::Assign { targets, value } => compiler.compile_assign_targets(targets, value),
            StmtKind::AnnAssign {
                target,
                annotation,
                value,
            } => compiler.compile_ann_assign(target, annotation, value.as_ref()),
            StmtKind::AugAssign { target, op, value } => {
                compiler.compile_aug_assign(target, op, value)
            }
            StmtKind::If { test, body, orelse } => compiler.compile_if(test, body, orelse),
            StmtKind::While { test, body, orelse } => compiler.compile_while(test, body, orelse),
            StmtKind::FunctionDef {
                name,
                type_params,
                is_async,
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                returns,
                body,
            } => compiler.compile_function_def_stmt(
                name,
                type_params,
                *is_async,
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                returns.as_ref(),
                body,
                true,
            ),
            StmtKind::ClassDef {
                name,
                type_params,
                bases,
                metaclass,
                keywords,
                body,
            } => {
                let _ = type_params;
                compiler.compile_class_def(name, bases, metaclass.as_ref(), keywords, body, true)
            }
            StmtKind::Delete { targets } => compiler.compile_delete(targets),
            StmtKind::Decorated { decorators, stmt } => {
                compiler.compile_decorated_stmt(decorators, stmt)
            }
            StmtKind::Return { value } => {
                if let Some(expr) = value {
                    compiler.compile_expr(expr)?;
                } else {
                    compiler.emit(Opcode::LoadConst, Some(0));
                }
                compiler.emit_return_or_defer()?;
                Ok(())
            }
            StmtKind::Raise { value, cause } => {
                compiler.compile_raise(value.as_ref(), cause.as_ref())
            }
            StmtKind::Assert { test, message } => compiler.compile_assert(test, message.as_ref()),
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => compiler.compile_try(body, handlers, orelse, finalbody),
            StmtKind::For {
                is_async,
                target,
                iter,
                body,
                orelse,
            } => {
                if *is_async {
                    compiler.compile_async_for(target, iter, body, orelse)
                } else {
                    compiler.compile_for(target, iter, body, orelse)
                }
            }
            StmtKind::Import { names } => {
                for alias in names {
                    let const_idx = compiler.code.add_const(Value::Str(alias.name.clone()));
                    compiler.emit(Opcode::ImportName, Some(const_idx));
                    let parts: Vec<&str> = alias.name.split('.').collect();
                    let has_dots = parts.len() > 1;
                    if alias.asname.is_some() && has_dots {
                        compiler.emit_import_attr_chain(&alias.name)?;
                    }
                    let target = if let Some(asname) = alias.asname.as_deref() {
                        asname
                    } else {
                        parts.first().copied().unwrap_or(&alias.name)
                    };
                    compiler.emit_store_name_scoped(target)?;
                }
                Ok(())
            }
            StmtKind::ImportFrom {
                module,
                names,
                level,
            } => {
                if *level == 0 && module.as_deref() == Some("__future__") {
                    // __future__ imports are compile-time directives and should
                    // not execute runtime import side effects.
                    compiler.emit(Opcode::Nop, None);
                    return Ok(());
                }
                let module_name = module.clone().unwrap_or_default();
                let import_name_idx = compiler.code.add_name(module_name);
                compiler.emit_const(Value::Int(*level as i64));
                for alias in names.iter() {
                    compiler.emit_const(Value::Str(alias.name.clone()));
                }
                compiler.emit(Opcode::BuildTuple, Some(names.len() as u32));
                compiler.emit(Opcode::ImportNameCpython, Some(import_name_idx));
                for alias in names {
                    let attr_idx = compiler.code.add_name(alias.name.clone());
                    compiler.emit(Opcode::ImportFromCpython, Some(attr_idx));
                    if alias.name == "*" {
                        compiler.emit(Opcode::PopTop, None);
                    } else {
                        let target = alias.asname.as_deref().unwrap_or(&alias.name);
                        compiler.emit_store_name_scoped(target)?;
                    }
                }
                compiler.emit(Opcode::PopTop, None);
                Ok(())
            }
            StmtKind::Global { .. } => Ok(()),
            StmtKind::Nonlocal { .. } => Ok(()),
            StmtKind::With {
                is_async,
                context,
                target,
                body,
            } => {
                if *is_async {
                    compiler.compile_async_with(context, target.as_ref(), body)
                } else {
                    compiler.compile_with(context, target.as_ref(), body)
                }
            }
            StmtKind::Match { subject, cases } => compiler.compile_match(subject, cases),
            StmtKind::Break => compiler.compile_break(),
            StmtKind::Continue => compiler.compile_continue(),
        })
    }

    fn emit_import_attr_chain(&mut self, module: &str) -> Result<(), CompileError> {
        let mut parts = module.split('.');
        let _root = parts.next();
        for part in parts {
            let attr_idx = self.code.add_name(part.to_string());
            self.emit(Opcode::LoadAttr, Some(attr_idx << 1));
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        let span = expr.span;
        self.with_span(span, |compiler| match &expr.node {
            ExprKind::Name(name) => compiler.emit_load_name(name),
            ExprKind::Constant(constant) => {
                let idx = compiler.code.add_const(constant_to_value(constant));
                compiler.emit(Opcode::LoadConst, Some(idx));
                Ok(())
            }
            ExprKind::Binary { left, op, right } => {
                if matches!(op, crate::ast::BinaryOp::Sub | crate::ast::BinaryOp::Lt) {
                    if let ExprKind::Constant(constant) = &right.node {
                        compiler.compile_expr(left)?;
                        let idx = compiler.code.add_const(constant_to_value(constant));
                        let opcode = match op {
                            crate::ast::BinaryOp::Sub => Opcode::BinarySubConst,
                            crate::ast::BinaryOp::Lt => Opcode::CompareLtConst,
                            _ => unreachable!(),
                        };
                        compiler.emit(opcode, Some(idx));
                        return Ok(());
                    }
                }
                compiler.compile_expr(left)?;
                compiler.compile_expr(right)?;
                let opcode = match op {
                    crate::ast::BinaryOp::Add => Opcode::BinaryAdd,
                    crate::ast::BinaryOp::Sub => Opcode::BinarySub,
                    crate::ast::BinaryOp::Mul => Opcode::BinaryMul,
                    crate::ast::BinaryOp::MatMul => Opcode::BinaryMatMul,
                    crate::ast::BinaryOp::Div => Opcode::BinaryDiv,
                    crate::ast::BinaryOp::Pow => Opcode::BinaryPow,
                    crate::ast::BinaryOp::FloorDiv => Opcode::BinaryFloorDiv,
                    crate::ast::BinaryOp::Mod => Opcode::BinaryMod,
                    crate::ast::BinaryOp::LShift => Opcode::BinaryLShift,
                    crate::ast::BinaryOp::RShift => Opcode::BinaryRShift,
                    crate::ast::BinaryOp::BitAnd => Opcode::BinaryAnd,
                    crate::ast::BinaryOp::BitXor => Opcode::BinaryXor,
                    crate::ast::BinaryOp::BitOr => Opcode::BinaryOr,
                    crate::ast::BinaryOp::Eq => Opcode::CompareEq,
                    crate::ast::BinaryOp::Ne => Opcode::CompareNe,
                    crate::ast::BinaryOp::Lt => Opcode::CompareLt,
                    crate::ast::BinaryOp::Le => Opcode::CompareLe,
                    crate::ast::BinaryOp::Gt => Opcode::CompareGt,
                    crate::ast::BinaryOp::Ge => Opcode::CompareGe,
                    crate::ast::BinaryOp::In => Opcode::CompareIn,
                    crate::ast::BinaryOp::NotIn => Opcode::CompareNotIn,
                    crate::ast::BinaryOp::Is => Opcode::CompareIs,
                    crate::ast::BinaryOp::IsNot => Opcode::CompareIsNot,
                };
                compiler.emit(opcode, None);
                Ok(())
            }
            ExprKind::Unary { op, operand } => {
                compiler.compile_expr(operand)?;
                let opcode = match op {
                    crate::ast::UnaryOp::Neg => Opcode::UnaryNeg,
                    crate::ast::UnaryOp::Not => Opcode::UnaryNot,
                    crate::ast::UnaryOp::Pos => Opcode::UnaryPos,
                    crate::ast::UnaryOp::Invert => Opcode::UnaryInvert,
                };
                compiler.emit(opcode, None);
                Ok(())
            }
            ExprKind::BoolOp { op, left, right } => compiler.compile_bool_op(op, left, right),
            ExprKind::IfExpr { test, body, orelse } => compiler.compile_if_expr(test, body, orelse),
            ExprKind::NamedExpr { target, value } => {
                compiler.compile_expr(value)?;
                compiler.emit(Opcode::DupTop, None);
                compiler.emit_store_name_scoped(target)?;
                Ok(())
            }
            ExprKind::Lambda {
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                body,
            } => {
                let return_stmt = Stmt {
                    node: StmtKind::Return {
                        value: Some((**body).clone()),
                    },
                    span,
                };
                let func_code = compiler.compile_function(
                    "<lambda>",
                    false,
                    posonly_params,
                    params,
                    kwonly_params,
                    vararg,
                    kwarg,
                    &[return_stmt],
                )?;
                compiler.emit_function_with_defaults(
                    posonly_params,
                    params,
                    kwonly_params,
                    vararg,
                    kwarg,
                    None,
                    func_code,
                )?;
                Ok(())
            }
            ExprKind::Yield { value } => {
                if let Some(value) = value {
                    compiler.compile_expr(value)?;
                } else {
                    compiler.emit(Opcode::LoadConst, Some(0));
                }
                compiler.emit(Opcode::YieldValue, None);
                Ok(())
            }
            ExprKind::YieldFrom { value } => {
                compiler.compile_expr(value)?;
                compiler.emit(Opcode::YieldFrom, None);
                Ok(())
            }
            ExprKind::Await { value } => {
                compiler.compile_expr(value)?;
                compiler.emit(Opcode::GetAwaitable, None);
                compiler.emit(Opcode::YieldFrom, None);
                Ok(())
            }
            ExprKind::ListComp { elt, clauses } => compiler.compile_list_comp(elt, clauses),
            ExprKind::DictComp {
                key,
                value,
                clauses,
            } => compiler.compile_dict_comp(key, value, clauses),
            ExprKind::GeneratorExp { elt, clauses } => {
                compiler.compile_generator_expr(elt, clauses)
            }
            ExprKind::Call { func, args } => {
                compiler.compile_expr(func)?;
                let has_star = args
                    .iter()
                    .any(|arg| matches!(arg, CallArg::Star(_) | CallArg::DoubleStar(_)));

                if has_star {
                    enum TempArg {
                        Positional(String),
                        Keyword(String, String),
                        Star(String),
                        DoubleStar(String),
                    }

                    let mut temps = Vec::new();
                    for arg in args {
                        match arg {
                            CallArg::Positional(expr) => {
                                let temp = compiler.fresh_temp("arg");
                                compiler.compile_expr(expr)?;
                                compiler.emit_store_name(&temp);
                                temps.push(TempArg::Positional(temp));
                            }
                            CallArg::Keyword { name, value } => {
                                let temp = compiler.fresh_temp("arg");
                                compiler.compile_expr(value)?;
                                compiler.emit_store_name(&temp);
                                temps.push(TempArg::Keyword(name.clone(), temp));
                            }
                            CallArg::Star(expr) => {
                                let temp = compiler.fresh_temp("arg");
                                compiler.compile_expr(expr)?;
                                compiler.emit_store_name(&temp);
                                temps.push(TempArg::Star(temp));
                            }
                            CallArg::DoubleStar(expr) => {
                                let temp = compiler.fresh_temp("arg");
                                compiler.compile_expr(expr)?;
                                compiler.emit_store_name(&temp);
                                temps.push(TempArg::DoubleStar(temp));
                            }
                        }
                    }

                    compiler.emit(Opcode::BuildList, Some(0));
                    for temp in &temps {
                        match temp {
                            TempArg::Positional(name) => {
                                compiler.emit_load_name(name)?;
                                compiler.emit(Opcode::ListAppend, None);
                            }
                            TempArg::Star(name) => {
                                compiler.emit_load_name(name)?;
                                compiler.emit(Opcode::ListExtend, None);
                            }
                            _ => {}
                        }
                    }

                    compiler.emit(Opcode::BuildDict, Some(0));
                    for temp in &temps {
                        match temp {
                            TempArg::Keyword(name, value) => {
                                let name_idx = compiler.code.add_const(Value::Str(name.clone()));
                                compiler.emit(Opcode::LoadConst, Some(name_idx));
                                compiler.emit_load_name(value)?;
                                compiler.emit(Opcode::DictSet, None);
                            }
                            TempArg::DoubleStar(name) => {
                                compiler.emit_load_name(name)?;
                                compiler.emit(Opcode::DictUpdate, None);
                            }
                            _ => {}
                        }
                    }

                    compiler.emit(Opcode::CallFunctionVar, None);
                    return Ok(());
                }

                let mut pos_count = 0u32;
                let mut kw_count = 0u32;
                for arg in args {
                    match arg {
                        CallArg::Positional(expr) => {
                            compiler.compile_expr(expr)?;
                            pos_count += 1;
                        }
                        CallArg::Keyword { name, value } => {
                            let name_idx = compiler.code.add_const(Value::Str(name.clone()));
                            compiler.emit(Opcode::LoadConst, Some(name_idx));
                            compiler.compile_expr(value)?;
                            kw_count += 1;
                        }
                        CallArg::Star(_) | CallArg::DoubleStar(_) => {}
                    }
                }
                if kw_count > 0 {
                    let packed = pack_call_counts(pos_count, kw_count)?;
                    compiler.emit(Opcode::CallFunctionKw, Some(packed));
                } else {
                    compiler.emit(Opcode::CallFunction, Some(pos_count));
                }
                Ok(())
            }
            ExprKind::List(elements) => {
                for elem in elements {
                    compiler.compile_expr(elem)?;
                }
                compiler.emit(Opcode::BuildList, Some(elements.len() as u32));
                Ok(())
            }
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    compiler.compile_expr(elem)?;
                }
                compiler.emit(Opcode::BuildTuple, Some(elements.len() as u32));
                Ok(())
            }
            ExprKind::Dict(entries) => {
                if entries
                    .iter()
                    .any(|entry| matches!(entry, DictEntry::Unpack(_)))
                {
                    compiler.emit(Opcode::BuildDict, Some(0));
                    for entry in entries {
                        match entry {
                            DictEntry::Pair(key, value) => {
                                compiler.compile_expr(key)?;
                                compiler.compile_expr(value)?;
                                compiler.emit(Opcode::DictSet, None);
                            }
                            DictEntry::Unpack(mapping) => {
                                compiler.compile_expr(mapping)?;
                                compiler.emit(Opcode::DictUpdate, None);
                            }
                        }
                    }
                    return Ok(());
                }
                for entry in entries {
                    if let DictEntry::Pair(key, value) = entry {
                        compiler.compile_expr(key)?;
                        compiler.compile_expr(value)?;
                    }
                }
                compiler.emit(Opcode::BuildDict, Some(entries.len() as u32));
                Ok(())
            }
            ExprKind::Subscript { value, index } => {
                compiler.compile_expr(value)?;
                compiler.compile_expr(index)?;
                compiler.emit(Opcode::Subscript, None);
                Ok(())
            }
            ExprKind::Attribute { value, name } => {
                compiler.compile_expr(value)?;
                let idx = compiler.code.add_name(name.clone());
                compiler.emit(Opcode::LoadAttr, Some(idx << 1));
                Ok(())
            }
            ExprKind::Slice { lower, upper, step } => {
                compiler.compile_slice_part(lower)?;
                compiler.compile_slice_part(upper)?;
                compiler.compile_slice_part(step)?;
                compiler.emit(Opcode::BuildSlice, None);
                Ok(())
            }
        })
    }

    fn with_span<T>(
        &mut self,
        span: Span,
        f: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let prev = self.current_span;
        self.current_span = span;
        let result = f(self);
        self.current_span = prev;
        result
    }

    fn ensure_local_name(&mut self, name: &str) {
        if matches!(
            self.scope.scope_type,
            ScopeType::Function | ScopeType::Lambda
        ) {
            self.scope.locals.insert(name.to_string());
        }
    }

    fn init_annotations(&mut self) -> Result<(), CompileError> {
        self.ensure_local_name("__annotations__");
        self.emit(Opcode::BuildDict, Some(0));
        self.emit_store_name_scoped("__annotations__")?;
        Ok(())
    }

    fn emit(&mut self, opcode: Opcode, arg: Option<u32>) {
        let (opcode, arg) = if matches!(opcode, Opcode::CallFunction) && arg == Some(1) {
            (Opcode::CallFunction1, None)
        } else {
            (opcode, arg)
        };
        self.code.instructions.push(Instruction::new(opcode, arg));
        self.code.locations.push(crate::bytecode::Location::new(
            self.current_span.line,
            self.current_span.column,
        ));
    }

    fn emit_const(&mut self, value: Value) {
        let idx = self.code.add_const(value);
        self.emit(Opcode::LoadConst, Some(idx));
    }

    fn compile_slice_part(&mut self, part: &Option<Box<Expr>>) -> Result<(), CompileError> {
        if let Some(expr) = part {
            self.compile_expr(expr)?;
        } else {
            self.emit(Opcode::LoadConst, Some(0));
        }
        Ok(())
    }

    fn emit_load_name(&mut self, name: &str) -> Result<(), CompileError> {
        let idx = self.code.add_name(name.to_string());
        match self.name_kind(name) {
            NameKind::Local => self.emit(Opcode::LoadFast, Some(idx)),
            NameKind::Cell | NameKind::Free => {
                let deref = self.deref_index(name)?;
                self.emit(Opcode::LoadDeref, Some(deref));
            }
            NameKind::Global => {
                let encoded = idx << 1;
                self.emit(Opcode::LoadGlobal, Some(encoded));
            }
            NameKind::Name => self.emit(Opcode::LoadName, Some(idx)),
        }
        Ok(())
    }

    fn emit_store_name(&mut self, name: &str) {
        let idx = self.code.add_name(name.to_string());
        self.emit(Opcode::StoreFast, Some(idx));
    }

    fn emit_delete_name(&mut self, name: &str) {
        let idx = self.code.add_name(name.to_string());
        self.emit(Opcode::DeleteName, Some(idx));
    }

    fn emit_store_name_scoped(&mut self, name: &str) -> Result<(), CompileError> {
        let idx = self.code.add_name(name.to_string());
        match self.name_kind(name) {
            NameKind::Global => self.emit(Opcode::StoreGlobal, Some(idx)),
            NameKind::Cell | NameKind::Free => {
                let deref = self.deref_index(name)?;
                self.emit(Opcode::StoreDeref, Some(deref));
            }
            NameKind::Local => self.emit(Opcode::StoreFast, Some(idx)),
            NameKind::Name => self.emit(Opcode::StoreName, Some(idx)),
        }
        Ok(())
    }

    fn emit_closure_tuple(&mut self, freevars: &[String]) -> Result<(), CompileError> {
        for name in freevars {
            let deref = self.deref_index(name)?;
            self.emit(Opcode::LoadClosure, Some(deref));
        }
        self.emit(Opcode::BuildTuple, Some(freevars.len() as u32));
        Ok(())
    }

    fn emit_function_annotations(
        &mut self,
        posonly_params: &[Parameter],
        params: &[Parameter],
        kwonly_params: &[Parameter],
        vararg: &Option<Parameter>,
        kwarg: &Option<Parameter>,
        returns: Option<&Expr>,
    ) -> Result<bool, CompileError> {
        let mut items: Vec<(String, &Expr)> = Vec::new();
        for param in posonly_params
            .iter()
            .chain(params.iter())
            .chain(kwonly_params.iter())
        {
            if let Some(annotation) = &param.annotation {
                items.push((param.name.clone(), annotation.as_ref()));
            }
        }
        for param in vararg.iter().chain(kwarg.iter()) {
            if let Some(annotation) = &param.annotation {
                items.push((param.name.clone(), annotation.as_ref()));
            }
        }
        if let Some(annotation) = returns {
            items.push(("return".to_string(), annotation));
        }
        if items.is_empty() {
            return Ok(false);
        }
        let count = items.len();
        for (name, expr) in items {
            self.emit_const(Value::Str(name));
            self.emit_annotation_expr(expr)?;
        }
        self.emit(Opcode::BuildDict, Some(count as u32));
        Ok(true)
    }

    fn emit_annotation_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        if self.future_annotations {
            self.emit_const(Value::Str(self.annotation_expr_to_string(expr)));
            Ok(())
        } else {
            self.compile_expr(expr)
        }
    }

    fn annotation_expr_to_string(&self, expr: &Expr) -> String {
        match &expr.node {
            ExprKind::Name(name) => name.clone(),
            ExprKind::Constant(Constant::None) => "None".to_string(),
            ExprKind::Constant(Constant::Bool(value)) => value.to_string(),
            ExprKind::Constant(Constant::Int(value)) => value.to_string(),
            ExprKind::Constant(Constant::Float(value)) => value.value().to_string(),
            ExprKind::Constant(Constant::Str(value)) => format!("{value:?}"),
            ExprKind::Attribute { value, name } => {
                format!("{}.{}", self.annotation_expr_to_string(value), name)
            }
            ExprKind::Subscript { value, index } => format!(
                "{}[{}]",
                self.annotation_expr_to_string(value),
                self.annotation_expr_to_string(index)
            ),
            ExprKind::Tuple(items) => items
                .iter()
                .map(|item| self.annotation_expr_to_string(item))
                .collect::<Vec<_>>()
                .join(", "),
            ExprKind::List(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| self.annotation_expr_to_string(item))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ExprKind::Call { func, args } => {
                let rendered = args
                    .iter()
                    .map(|arg| match arg {
                        CallArg::Positional(value) => self.annotation_expr_to_string(value),
                        CallArg::Keyword { name, value } => {
                            format!("{name}={}", self.annotation_expr_to_string(value))
                        }
                        CallArg::Star(value) => {
                            format!("*{}", self.annotation_expr_to_string(value))
                        }
                        CallArg::DoubleStar(value) => {
                            format!("**{}", self.annotation_expr_to_string(value))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({rendered})", self.annotation_expr_to_string(func))
            }
            ExprKind::Binary { left, op, right } => format!(
                "{} {} {}",
                self.annotation_expr_to_string(left),
                annotation_binary_op_symbol(op),
                self.annotation_expr_to_string(right)
            ),
            ExprKind::Unary { op, operand } => {
                format!(
                    "{}{}",
                    annotation_unary_op_symbol(op),
                    self.annotation_expr_to_string(operand)
                )
            }
            ExprKind::BoolOp { op, left, right } => format!(
                "{} {} {}",
                self.annotation_expr_to_string(left),
                annotation_bool_op_symbol(op),
                self.annotation_expr_to_string(right)
            ),
            ExprKind::Slice { lower, upper, step } => {
                let mut text = String::new();
                if let Some(lower) = lower {
                    text.push_str(&self.annotation_expr_to_string(lower));
                }
                text.push(':');
                if let Some(upper) = upper {
                    text.push_str(&self.annotation_expr_to_string(upper));
                }
                if let Some(step) = step {
                    text.push(':');
                    text.push_str(&self.annotation_expr_to_string(step));
                }
                text
            }
            _ => format!("<expr {:?}>", expr.node),
        }
    }

    fn name_kind(&self, name: &str) -> NameKind {
        match self.scope.scope_type {
            ScopeType::Module | ScopeType::Class => NameKind::Name,
            ScopeType::Function | ScopeType::Lambda => {
                if self.scope.is_global(name) {
                    NameKind::Global
                } else if self.scope.is_cell(name) {
                    NameKind::Cell
                } else if self.scope.is_local(name) {
                    NameKind::Local
                } else if self.scope.is_free(name) {
                    NameKind::Free
                } else {
                    NameKind::Global
                }
            }
        }
    }

    fn deref_index(&self, name: &str) -> Result<u32, CompileError> {
        self.cell_index
            .get(name)
            .copied()
            .ok_or_else(|| CompileError::new(format!("unknown closure variable '{name}'")))
    }

    fn emit_jump(&mut self, opcode: Opcode) -> usize {
        let index = self.code.instructions.len();
        self.emit(opcode, Some(0));
        index
    }

    fn patch_jump(&mut self, index: usize, target: usize) -> Result<(), CompileError> {
        let instr = self
            .code
            .instructions
            .get_mut(index)
            .ok_or_else(|| CompileError::new("invalid jump patch"))?;
        instr.arg = Some(target as u32);
        Ok(())
    }

    fn current_ip(&self) -> usize {
        self.code.instructions.len()
    }

    fn compile_if(
        &mut self,
        test: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
    ) -> Result<(), CompileError> {
        self.compile_expr(test)?;
        let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        let jump_to_end = if !orelse.is_empty() {
            Some(self.emit_jump(Opcode::Jump))
        } else {
            None
        };

        let else_target = self.current_ip();
        self.patch_jump(jump_if_false, else_target)?;

        if !orelse.is_empty() {
            for stmt in orelse {
                self.compile_stmt(stmt)?;
            }
            let end_target = self.current_ip();
            if let Some(jump_to_end) = jump_to_end {
                self.patch_jump(jump_to_end, end_target)?;
            }
        }

        Ok(())
    }

    fn compile_while(
        &mut self,
        test: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
    ) -> Result<(), CompileError> {
        let loop_start = self.current_ip();
        self.compile_expr(test)?;
        let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);

        self.loop_stack.push(LoopContext {
            start: loop_start,
            continue_target: Some(loop_start),
            break_cleanup_pops: 0,
            breaks: Vec::new(),
            continues: Vec::new(),
        });

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        self.emit(Opcode::Jump, Some(loop_start as u32));
        let else_start = self.current_ip();
        self.patch_jump(jump_if_false, else_start)?;
        let loop_ctx = self
            .loop_stack
            .pop()
            .ok_or_else(|| CompileError::new("loop stack underflow"))?;

        for stmt in orelse {
            self.compile_stmt(stmt)?;
        }

        let loop_end = self.current_ip();
        self.resolve_loop_context(loop_ctx, loop_end)?;
        Ok(())
    }

    fn compile_function(
        &mut self,
        name: &str,
        is_async: bool,
        posonly_params: &[Parameter],
        params: &[Parameter],
        kwonly_params: &[Parameter],
        vararg: &Option<Parameter>,
        kwarg: &Option<Parameter>,
        body: &[Stmt],
    ) -> Result<CodeObject, CompileError> {
        let scope = ScopeInfo::for_function(
            posonly_params,
            params,
            kwonly_params,
            vararg,
            kwarg,
            body,
            &self.scope,
        )?;
        let mut compiler = Compiler::new(name, &self.code.filename, scope);
        compiler.code.posonly_params = posonly_params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        compiler.code.params = params.iter().map(|param| param.name.clone()).collect();
        compiler.code.kwonly_params = kwonly_params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        compiler.code.vararg = vararg.as_ref().map(|param| param.name.clone());
        compiler.code.kwarg = kwarg.as_ref().map(|param| param.name.clone());
        let has_yield = body_has_yield(body);
        compiler.code.is_generator = has_yield || is_async;
        compiler.code.is_coroutine = is_async && !has_yield;
        compiler.code.is_async_generator = is_async && has_yield;
        if body_has_ann_assign(body) {
            compiler.init_annotations()?;
        }
        for stmt in body {
            compiler.compile_stmt(stmt)?;
        }
        Ok(compiler.finish())
    }

    fn emit_function_with_defaults(
        &mut self,
        posonly_params: &[Parameter],
        params: &[Parameter],
        kwonly_params: &[Parameter],
        vararg: &Option<Parameter>,
        kwarg: &Option<Parameter>,
        returns: Option<&Expr>,
        func_code: CodeObject,
    ) -> Result<(), CompileError> {
        let needs_closure = !func_code.freevars.is_empty();
        if needs_closure {
            self.emit_closure_tuple(&func_code.freevars)?;
        }
        let needs_annotations = self.emit_function_annotations(
            posonly_params,
            params,
            kwonly_params,
            vararg,
            kwarg,
            returns,
        )?;
        let defaults: Vec<&Expr> = posonly_params
            .iter()
            .chain(params.iter())
            .filter_map(|param| param.default.as_deref())
            .collect();
        for expr in &defaults {
            self.compile_expr(expr)?;
        }
        self.emit(Opcode::BuildTuple, Some(defaults.len() as u32));
        let mut kwonly_count = 0;
        for param in kwonly_params {
            if let Some(default) = &param.default {
                self.emit_const(Value::Str(param.name.clone()));
                self.compile_expr(default)?;
                kwonly_count += 1;
            }
        }
        self.emit(Opcode::BuildDict, Some(kwonly_count));
        let const_idx = self.code.add_const(Value::Code(Rc::new(func_code)));
        self.emit(Opcode::MakeFunction, Some(const_idx));
        if needs_annotations {
            self.emit(Opcode::SetFunctionAttribute, Some(0x04));
        }
        if needs_closure {
            self.emit(Opcode::SetFunctionAttribute, Some(0x08));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_function_def_stmt(
        &mut self,
        name: &str,
        type_params: &[String],
        is_async: bool,
        posonly_params: &[Parameter],
        params: &[Parameter],
        vararg: &Option<Parameter>,
        kwarg: &Option<Parameter>,
        kwonly_params: &[Parameter],
        returns: Option<&Expr>,
        body: &[Stmt],
        store_target: bool,
    ) -> Result<(), CompileError> {
        let drop_annotations = !type_params.is_empty();
        let func_code = self.compile_function(
            name,
            is_async,
            posonly_params,
            params,
            kwonly_params,
            vararg,
            kwarg,
            body,
        )?;
        let mut ann_posonly = posonly_params.to_vec();
        let mut ann_params = params.to_vec();
        let mut ann_kwonly = kwonly_params.to_vec();
        let mut ann_vararg = vararg.clone();
        let mut ann_kwarg = kwarg.clone();
        if drop_annotations {
            for param in ann_posonly
                .iter_mut()
                .chain(ann_params.iter_mut())
                .chain(ann_kwonly.iter_mut())
            {
                param.annotation = None;
            }
            if let Some(param) = &mut ann_vararg {
                param.annotation = None;
            }
            if let Some(param) = &mut ann_kwarg {
                param.annotation = None;
            }
        }
        self.emit_function_with_defaults(
            &ann_posonly,
            &ann_params,
            &ann_kwonly,
            &ann_vararg,
            &ann_kwarg,
            if drop_annotations { None } else { returns },
            func_code,
        )?;
        if store_target {
            self.emit_store_name_scoped(name)?;
        }
        Ok(())
    }

    fn compile_class_def(
        &mut self,
        name: &str,
        bases: &[Expr],
        metaclass: Option<&Expr>,
        keywords: &[(String, Expr)],
        body: &[Stmt],
        store_target: bool,
    ) -> Result<(), CompileError> {
        let class_code = self.compile_class(name, body)?;
        let code_idx = self.code.add_const(Value::Code(Rc::new(class_code)));
        for base in bases {
            self.compile_expr(base)?;
        }
        self.emit(Opcode::BuildTuple, Some(bases.len() as u32));
        let name_idx = self.code.add_const(Value::Str(name.to_string()));
        self.emit(Opcode::LoadConst, Some(name_idx));
        if let Some(meta) = metaclass {
            self.compile_expr(meta)?;
        } else {
            self.emit(Opcode::LoadConst, Some(0));
        }
        for (name, value) in keywords {
            self.emit_const(Value::Str(name.clone()));
            self.compile_expr(value)?;
        }
        self.emit(Opcode::BuildDict, Some(keywords.len() as u32));
        self.emit(Opcode::BuildClass, Some(code_idx));
        if store_target {
            self.emit_store_name_scoped(name)?;
        }
        Ok(())
    }

    fn compile_class(&mut self, name: &str, body: &[Stmt]) -> Result<CodeObject, CompileError> {
        let scope = ScopeInfo::for_class(body, &self.scope)?;
        let mut compiler = Compiler::new(&format!("<class {name}>"), &self.code.filename, scope);
        if body_has_ann_assign(body) {
            compiler.init_annotations()?;
        }
        for stmt in body {
            compiler.compile_stmt(stmt)?;
        }
        Ok(compiler.finish())
    }

    fn compile_decorated_stmt(
        &mut self,
        decorators: &[Expr],
        stmt: &Stmt,
    ) -> Result<(), CompileError> {
        let target_name = match &stmt.node {
            StmtKind::FunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => name.clone(),
            _ => {
                return Err(CompileError::new(
                    "decorators can only target function or class definitions",
                ));
            }
        };

        // Decorator expressions are evaluated before the function/class object is
        // created and bound to the target name.
        let mut temp_names = Vec::new();
        for decorator in decorators {
            let temp = self.fresh_temp("decorator");
            self.compile_expr(decorator)?;
            self.emit_store_name(&temp);
            temp_names.push(temp);
        }

        match &stmt.node {
            StmtKind::FunctionDef {
                name,
                type_params,
                is_async,
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                returns,
                body,
            } => self.compile_function_def_stmt(
                name,
                type_params,
                *is_async,
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                returns.as_ref(),
                body,
                false,
            )?,
            StmtKind::ClassDef {
                name,
                type_params,
                bases,
                metaclass,
                keywords,
                body,
            } => {
                let _ = type_params;
                self.compile_class_def(name, bases, metaclass.as_ref(), keywords, body, false)?
            }
            _ => unreachable!("decorated stmt target is validated above"),
        }

        let decorated_temp = self.fresh_temp("decorated");
        self.emit_store_name(&decorated_temp);

        for temp in temp_names.iter().rev() {
            self.emit_load_name(temp)?;
            self.emit_load_name(&decorated_temp)?;
            self.emit(Opcode::CallFunction, Some(1));
            self.emit_store_name(&decorated_temp);
        }
        self.emit_load_name(&decorated_temp)?;
        self.emit_store_name_scoped(&target_name)?;
        self.emit_delete_name(&decorated_temp);
        for temp in temp_names {
            self.emit_delete_name(&temp);
        }

        Ok(())
    }

    fn compile_match(&mut self, subject: &Expr, cases: &[MatchCase]) -> Result<(), CompileError> {
        self.validate_match_cases(cases)?;

        let subject_temp = self.fresh_temp("match_subject");
        self.compile_expr(subject)?;
        self.emit_store_name(&subject_temp);

        let mut end_jumps = Vec::new();
        for case in cases {
            self.compile_pattern_test(&case.pattern, &subject_temp)?;
            let next_case = self.emit_jump(Opcode::JumpIfFalse);

            self.compile_pattern_bindings(&case.pattern, &subject_temp)?;
            let guard_jump = if let Some(guard) = &case.guard {
                self.compile_expr(guard)?;
                Some(self.emit_jump(Opcode::JumpIfFalse))
            } else {
                None
            };

            for stmt in &case.body {
                self.compile_stmt(stmt)?;
            }
            end_jumps.push(self.emit_jump(Opcode::Jump));

            let next_ip = self.current_ip();
            self.patch_jump(next_case, next_ip)?;
            if let Some(jump) = guard_jump {
                self.patch_jump(jump, next_ip)?;
            }
        }

        let end_ip = self.current_ip();
        for jump in end_jumps {
            self.patch_jump(jump, end_ip)?;
        }
        Ok(())
    }

    fn validate_match_cases(&self, cases: &[MatchCase]) -> Result<(), CompileError> {
        for (idx, case) in cases.iter().enumerate() {
            Self::validate_pattern_bindings(&case.pattern)?;
            if idx + 1 < cases.len() && case.guard.is_none() {
                if let Some(kind) = Self::irrefutable_pattern_kind(&case.pattern) {
                    return Err(CompileError::new(kind.unreachable_message()));
                }
            }
        }
        Ok(())
    }

    fn validate_pattern_bindings(pattern: &Pattern) -> Result<HashSet<String>, CompileError> {
        let mut bound = HashSet::new();
        match pattern {
            Pattern::Wildcard | Pattern::Constant(_) | Pattern::Value(_) | Pattern::Star(None) => {}
            Pattern::Capture(name) | Pattern::Star(Some(name)) => {
                Self::insert_pattern_binding(&mut bound, name)?;
            }
            Pattern::As { pattern, name } => {
                Self::merge_pattern_bindings(
                    &mut bound,
                    Self::validate_pattern_bindings(pattern)?,
                )?;
                Self::insert_pattern_binding(&mut bound, name)?;
            }
            Pattern::Sequence(items) => {
                for item in items {
                    Self::merge_pattern_bindings(
                        &mut bound,
                        Self::validate_pattern_bindings(item)?,
                    )?;
                }
            }
            Pattern::Mapping { entries, rest } => {
                for (_, value_pattern) in entries {
                    Self::merge_pattern_bindings(
                        &mut bound,
                        Self::validate_pattern_bindings(value_pattern)?,
                    )?;
                }
                if let Some(name) = rest {
                    Self::insert_pattern_binding(&mut bound, name)?;
                }
            }
            Pattern::Class {
                positional,
                keywords,
                ..
            } => {
                for subpattern in positional {
                    Self::merge_pattern_bindings(
                        &mut bound,
                        Self::validate_pattern_bindings(subpattern)?,
                    )?;
                }
                for (_, subpattern) in keywords {
                    Self::merge_pattern_bindings(
                        &mut bound,
                        Self::validate_pattern_bindings(subpattern)?,
                    )?;
                }
            }
            Pattern::Or(options) => {
                let mut expected_bindings: Option<HashSet<String>> = None;
                for (idx, option) in options.iter().enumerate() {
                    if idx + 1 < options.len() {
                        if let Some(kind) = Self::irrefutable_pattern_kind(option) {
                            return Err(CompileError::new(kind.unreachable_message()));
                        }
                    }

                    let option_bindings = Self::validate_pattern_bindings(option)?;
                    if let Some(expected) = &expected_bindings {
                        if option_bindings != *expected {
                            let mut expected_names = expected.iter().cloned().collect::<Vec<_>>();
                            expected_names.sort();
                            let mut option_names =
                                option_bindings.iter().cloned().collect::<Vec<_>>();
                            option_names.sort();
                            return Err(CompileError::new(format!(
                                "alternative patterns bind different names: expected {:?}, got {:?}",
                                expected_names, option_names
                            )));
                        }
                    } else {
                        expected_bindings = Some(option_bindings);
                    }
                }
                bound = expected_bindings.unwrap_or_default();
            }
        }
        Ok(bound)
    }

    fn merge_pattern_bindings(
        target: &mut HashSet<String>,
        incoming: HashSet<String>,
    ) -> Result<(), CompileError> {
        for name in incoming {
            Self::insert_pattern_binding(target, &name)?;
        }
        Ok(())
    }

    fn insert_pattern_binding(
        target: &mut HashSet<String>,
        name: &str,
    ) -> Result<(), CompileError> {
        if !target.insert(name.to_string()) {
            return Err(CompileError::new(format!(
                "multiple assignments to name '{name}' in pattern"
            )));
        }
        Ok(())
    }

    fn irrefutable_pattern_kind(pattern: &Pattern) -> Option<IrrefutablePatternKind> {
        match pattern {
            Pattern::Wildcard => Some(IrrefutablePatternKind::Wildcard),
            Pattern::Capture(name) => Some(IrrefutablePatternKind::Capture(name.clone())),
            Pattern::As { pattern, .. } => Self::irrefutable_pattern_kind(pattern),
            Pattern::Or(options) => options.iter().find_map(Self::irrefutable_pattern_kind),
            Pattern::Constant(_)
            | Pattern::Value(_)
            | Pattern::Sequence(_)
            | Pattern::Mapping { .. }
            | Pattern::Class { .. }
            | Pattern::Star(_) => None,
        }
    }

    fn compile_pattern_test(
        &mut self,
        pattern: &Pattern,
        subject_temp: &str,
    ) -> Result<(), CompileError> {
        match pattern {
            Pattern::Wildcard | Pattern::Capture(_) | Pattern::Star(_) => {
                self.emit_const(Value::Bool(true));
                Ok(())
            }
            Pattern::Constant(value) => {
                self.emit_load_name(subject_temp)?;
                self.emit_const(constant_to_value(value));
                self.emit(Opcode::CompareEq, None);
                Ok(())
            }
            Pattern::Value(expr) => {
                self.emit_load_name(subject_temp)?;
                self.compile_expr(expr)?;
                self.emit(Opcode::CompareEq, None);
                Ok(())
            }
            Pattern::Sequence(items) => self.compile_sequence_pattern_test(items, subject_temp),
            Pattern::Mapping { entries, .. } => {
                self.compile_mapping_pattern_test(entries, subject_temp)
            }
            Pattern::Class {
                class,
                positional,
                keywords,
            } => self.compile_class_pattern_test(class, positional, keywords, subject_temp),
            Pattern::Or(options) => self.compile_or_pattern_test(options, subject_temp),
            Pattern::As { pattern, .. } => self.compile_pattern_test(pattern, subject_temp),
        }
    }

    fn compile_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        subject_temp: &str,
    ) -> Result<(), CompileError> {
        match pattern {
            Pattern::Capture(name) => {
                self.emit_load_name(subject_temp)?;
                self.emit_store_name_scoped(name)?;
            }
            Pattern::As { pattern, name } => {
                self.compile_pattern_bindings(pattern, subject_temp)?;
                self.emit_load_name(subject_temp)?;
                self.emit_store_name_scoped(name)?;
            }
            Pattern::Sequence(items) => {
                let star_index = items
                    .iter()
                    .position(|item| matches!(item, Pattern::Star(_)));
                for (idx, item) in items.iter().enumerate() {
                    match item {
                        Pattern::Star(Some(name)) => {
                            let trailing = items.len().saturating_sub(idx + 1);
                            self.emit_load_name("list")?;
                            self.emit_load_name(subject_temp)?;
                            self.emit_const(Value::Int(idx as i64));
                            if trailing == 0 {
                                self.emit_const(Value::None);
                            } else {
                                self.emit_const(Value::Int(-(trailing as i64)));
                            }
                            self.emit_const(Value::None);
                            self.emit(Opcode::BuildSlice, Some(3));
                            self.emit(Opcode::Subscript, None);
                            self.emit(Opcode::CallFunction, Some(1));
                            self.emit_store_name_scoped(name)?;
                        }
                        Pattern::Star(None) => {}
                        _ => {
                            let from_end = if let Some(star) = star_index {
                                idx > star
                            } else {
                                false
                            };
                            let index = if from_end {
                                -((items.len() - idx) as i64)
                            } else {
                                idx as i64
                            };
                            let temp = self.fresh_temp("match_bind_item");
                            self.emit_extract_pattern_item(subject_temp, index, &temp)?;
                            self.compile_pattern_bindings(item, &temp)?;
                        }
                    }
                }
            }
            Pattern::Mapping { entries, rest } => {
                for (key, value_pattern) in entries {
                    let temp = self.fresh_temp("match_map_bind");
                    self.emit_load_name(subject_temp)?;
                    self.compile_expr(key)?;
                    self.emit(Opcode::Subscript, None);
                    self.emit_store_name(&temp);
                    self.compile_pattern_bindings(value_pattern, &temp)?;
                }
                if let Some(name) = rest {
                    let rest_temp = self.fresh_temp("match_map_rest");
                    self.emit_load_name("dict")?;
                    self.emit_load_name(subject_temp)?;
                    self.emit(Opcode::CallFunction, Some(1));
                    self.emit_store_name(&rest_temp);
                    for (key, _) in entries {
                        self.emit_load_name(&rest_temp)?;
                        self.compile_expr(key)?;
                        self.emit(Opcode::DeleteSubscript, None);
                    }
                    self.emit_load_name(&rest_temp)?;
                    self.emit_store_name_scoped(name)?;
                }
            }
            Pattern::Class {
                class,
                positional,
                keywords,
            } => {
                let class_temp = self.fresh_temp("match_class_bind");
                self.compile_expr(class)?;
                self.emit_store_name(&class_temp);

                let match_args_temp = self.fresh_temp("match_args_bind");
                self.emit_load_name("getattr")?;
                self.emit_load_name(&class_temp)?;
                self.emit_const(Value::Str("__match_args__".to_string()));
                self.emit(Opcode::BuildTuple, Some(0));
                self.emit(Opcode::CallFunction, Some(3));
                self.emit_store_name(&match_args_temp);

                for (idx, pattern) in positional.iter().enumerate() {
                    let attr_name_temp = self.fresh_temp("match_attr_name_bind");
                    self.emit_extract_pattern_item(&match_args_temp, idx as i64, &attr_name_temp)?;

                    let attr_value_temp = self.fresh_temp("match_attr_bind");
                    self.emit_load_name("getattr")?;
                    self.emit_load_name(subject_temp)?;
                    self.emit_load_name(&attr_name_temp)?;
                    self.emit(Opcode::CallFunction, Some(2));
                    self.emit_store_name(&attr_value_temp);
                    self.compile_pattern_bindings(pattern, &attr_value_temp)?;
                }

                for (name, pattern) in keywords {
                    let attr_value_temp = self.fresh_temp("match_kw_attr_bind");
                    self.emit_load_name("getattr")?;
                    self.emit_load_name(subject_temp)?;
                    self.emit_const(Value::Str(name.clone()));
                    self.emit(Opcode::CallFunction, Some(2));
                    self.emit_store_name(&attr_value_temp);
                    self.compile_pattern_bindings(pattern, &attr_value_temp)?;
                }
            }
            Pattern::Or(options) => {
                if options.is_empty() {
                    return Ok(());
                }
                let mut end_jumps = Vec::new();
                for option in options.iter().take(options.len().saturating_sub(1)) {
                    self.compile_pattern_test(option, subject_temp)?;
                    let next_option = self.emit_jump(Opcode::JumpIfFalse);
                    self.compile_pattern_bindings(option, subject_temp)?;
                    end_jumps.push(self.emit_jump(Opcode::Jump));
                    let next_ip = self.current_ip();
                    self.patch_jump(next_option, next_ip)?;
                }
                let last = options.last().expect("checked not empty");
                self.compile_pattern_bindings(last, subject_temp)?;
                let end_ip = self.current_ip();
                for jump in end_jumps {
                    self.patch_jump(jump, end_ip)?;
                }
            }
            Pattern::Wildcard | Pattern::Constant(_) | Pattern::Value(_) | Pattern::Star(None) => {}
            Pattern::Star(Some(name)) => {
                self.emit_load_name(subject_temp)?;
                self.emit_store_name_scoped(name)?;
            }
        }
        Ok(())
    }

    fn compile_or_pattern_test(
        &mut self,
        options: &[Pattern],
        subject_temp: &str,
    ) -> Result<(), CompileError> {
        if options.is_empty() {
            self.emit_const(Value::Bool(false));
            return Ok(());
        }
        self.compile_pattern_test(&options[0], subject_temp)?;
        let mut end_jumps = Vec::new();
        for option in options.iter().skip(1) {
            self.emit(Opcode::DupTop, None);
            let done_jump = self.emit_jump(Opcode::JumpIfTrue);
            self.emit(Opcode::PopTop, None);
            self.compile_pattern_test(option, subject_temp)?;
            end_jumps.push(done_jump);
        }
        let end = self.current_ip();
        for jump in end_jumps {
            self.patch_jump(jump, end)?;
        }
        Ok(())
    }

    fn compile_sequence_pattern_test(
        &mut self,
        items: &[Pattern],
        subject_temp: &str,
    ) -> Result<(), CompileError> {
        let star_positions = items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| matches!(item, Pattern::Star(_)).then_some(idx))
            .collect::<Vec<_>>();
        if star_positions.len() > 1 {
            return Err(CompileError::new(
                "multiple starred patterns in sequence pattern",
            ));
        }
        let star_index = star_positions.first().copied();
        let min_required = items.len().saturating_sub(star_positions.len());

        let mut fail_jumps = Vec::new();

        self.emit_load_name("isinstance")?;
        self.emit_load_name(subject_temp)?;
        self.emit_load_name("list")?;
        self.emit_load_name("tuple")?;
        self.emit(Opcode::BuildTuple, Some(2));
        self.emit(Opcode::CallFunction, Some(2));
        fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

        self.emit_load_name("len")?;
        self.emit_load_name(subject_temp)?;
        self.emit(Opcode::CallFunction, Some(1));
        if star_index.is_some() {
            self.emit_const(Value::Int(min_required as i64));
            self.emit(Opcode::CompareGe, None);
        } else {
            self.emit_const(Value::Int(items.len() as i64));
            self.emit(Opcode::CompareEq, None);
        }
        fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

        for (idx, item) in items.iter().enumerate() {
            if matches!(item, Pattern::Star(_)) {
                continue;
            }
            let from_end = if let Some(star) = star_index {
                idx > star
            } else {
                false
            };
            let index = if from_end {
                -((items.len() - idx) as i64)
            } else {
                idx as i64
            };
            let item_temp = self.fresh_temp("match_seq_item");
            self.emit_extract_pattern_item(subject_temp, index, &item_temp)?;
            self.compile_pattern_test(item, &item_temp)?;
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));
        }

        self.emit_const(Value::Bool(true));
        let end_jump = self.emit_jump(Opcode::Jump);
        let fail_target = self.current_ip();
        for jump in fail_jumps {
            self.patch_jump(jump, fail_target)?;
        }
        self.emit_const(Value::Bool(false));
        let end_target = self.current_ip();
        self.patch_jump(end_jump, end_target)?;
        Ok(())
    }

    fn compile_mapping_pattern_test(
        &mut self,
        entries: &[(Expr, Pattern)],
        subject_temp: &str,
    ) -> Result<(), CompileError> {
        let mut fail_jumps = Vec::new();

        self.emit_load_name("isinstance")?;
        self.emit_load_name(subject_temp)?;
        self.emit_load_name("dict")?;
        self.emit(Opcode::CallFunction, Some(2));
        fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

        for (key, value_pattern) in entries {
            self.compile_expr(key)?;
            self.emit_load_name(subject_temp)?;
            self.emit(Opcode::CompareIn, None);
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

            let value_temp = self.fresh_temp("match_map_value");
            self.emit_load_name(subject_temp)?;
            self.compile_expr(key)?;
            self.emit(Opcode::Subscript, None);
            self.emit_store_name(&value_temp);
            self.compile_pattern_test(value_pattern, &value_temp)?;
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));
        }

        self.emit_const(Value::Bool(true));
        let end_jump = self.emit_jump(Opcode::Jump);
        let fail_target = self.current_ip();
        for jump in fail_jumps {
            self.patch_jump(jump, fail_target)?;
        }
        self.emit_const(Value::Bool(false));
        let end_target = self.current_ip();
        self.patch_jump(end_jump, end_target)?;
        Ok(())
    }

    fn compile_class_pattern_test(
        &mut self,
        class: &Expr,
        positional: &[Pattern],
        keywords: &[(String, Pattern)],
        subject_temp: &str,
    ) -> Result<(), CompileError> {
        let mut fail_jumps = Vec::new();

        let class_temp = self.fresh_temp("match_class");
        self.compile_expr(class)?;
        self.emit_store_name(&class_temp);

        self.emit_load_name("isinstance")?;
        self.emit_load_name(subject_temp)?;
        self.emit_load_name(&class_temp)?;
        self.emit(Opcode::CallFunction, Some(2));
        fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

        let match_args_temp = self.fresh_temp("match_args");
        self.emit_load_name("getattr")?;
        self.emit_load_name(&class_temp)?;
        self.emit_const(Value::Str("__match_args__".to_string()));
        self.emit(Opcode::BuildTuple, Some(0));
        self.emit(Opcode::CallFunction, Some(3));
        self.emit_store_name(&match_args_temp);

        for (idx, pattern) in positional.iter().enumerate() {
            self.emit_load_name("len")?;
            self.emit_load_name(&match_args_temp)?;
            self.emit(Opcode::CallFunction, Some(1));
            self.emit_const(Value::Int((idx + 1) as i64));
            self.emit(Opcode::CompareGe, None);
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

            let attr_name_temp = self.fresh_temp("match_attr_name");
            self.emit_extract_pattern_item(&match_args_temp, idx as i64, &attr_name_temp)?;

            self.emit_load_name("hasattr")?;
            self.emit_load_name(subject_temp)?;
            self.emit_load_name(&attr_name_temp)?;
            self.emit(Opcode::CallFunction, Some(2));
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

            let attr_value_temp = self.fresh_temp("match_attr_value");
            self.emit_load_name("getattr")?;
            self.emit_load_name(subject_temp)?;
            self.emit_load_name(&attr_name_temp)?;
            self.emit(Opcode::CallFunction, Some(2));
            self.emit_store_name(&attr_value_temp);
            self.compile_pattern_test(pattern, &attr_value_temp)?;
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));
        }

        for (name, pattern) in keywords {
            self.emit_load_name("hasattr")?;
            self.emit_load_name(subject_temp)?;
            self.emit_const(Value::Str(name.clone()));
            self.emit(Opcode::CallFunction, Some(2));
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));

            let attr_value_temp = self.fresh_temp("match_kw_attr");
            self.emit_load_name("getattr")?;
            self.emit_load_name(subject_temp)?;
            self.emit_const(Value::Str(name.clone()));
            self.emit(Opcode::CallFunction, Some(2));
            self.emit_store_name(&attr_value_temp);
            self.compile_pattern_test(pattern, &attr_value_temp)?;
            fail_jumps.push(self.emit_jump(Opcode::JumpIfFalse));
        }

        self.emit_const(Value::Bool(true));
        let end_jump = self.emit_jump(Opcode::Jump);
        let fail_target = self.current_ip();
        for jump in fail_jumps {
            self.patch_jump(jump, fail_target)?;
        }
        self.emit_const(Value::Bool(false));
        let end_target = self.current_ip();
        self.patch_jump(end_jump, end_target)?;
        Ok(())
    }

    fn emit_extract_pattern_item(
        &mut self,
        subject_temp: &str,
        index: i64,
        target_temp: &str,
    ) -> Result<(), CompileError> {
        self.emit_load_name(subject_temp)?;
        self.emit_const(Value::Int(index));
        self.emit(Opcode::Subscript, None);
        self.emit_store_name(target_temp);
        Ok(())
    }

    fn compile_list_comp(
        &mut self,
        elt: &Expr,
        clauses: &[ComprehensionClause],
    ) -> Result<(), CompileError> {
        let body = build_list_comp_body(elt, clauses);
        self.emit_comp_function("<listcomp>", clauses, body)
    }

    fn compile_dict_comp(
        &mut self,
        key: &Expr,
        value: &Expr,
        clauses: &[ComprehensionClause],
    ) -> Result<(), CompileError> {
        let body = build_dict_comp_body(key, value, clauses);
        self.emit_comp_function("<dictcomp>", clauses, body)
    }

    fn compile_generator_expr(
        &mut self,
        elt: &Expr,
        clauses: &[ComprehensionClause],
    ) -> Result<(), CompileError> {
        let body = build_genexpr_body(elt, clauses);
        self.emit_comp_function("<genexpr>", clauses, body)
    }

    fn emit_comp_function(
        &mut self,
        name: &str,
        clauses: &[ComprehensionClause],
        mut body: Vec<Stmt>,
    ) -> Result<(), CompileError> {
        // CPython evaluates the first comprehension iterable in the enclosing scope.
        // We model this by passing it as an explicit synthetic positional argument.
        let mut params: Vec<Parameter> = Vec::new();
        let mut outer_iter: Option<Expr> = None;
        if let Some(first_clause) = clauses.first() {
            params.push(Parameter {
                name: "__pyrs_comp_iter0".to_string(),
                default: None,
                annotation: None,
            });
            rewrite_first_comp_iter_to_param(&mut body);
            outer_iter = Some(first_clause.iter.clone());
        }
        let empty_params: Vec<Parameter> = Vec::new();
        let vararg: Option<Parameter> = None;
        let kwarg: Option<Parameter> = None;
        let func_code = self.compile_function(
            name,
            false,
            &empty_params,
            &params,
            &empty_params,
            &vararg,
            &kwarg,
            &body,
        )?;
        self.emit_function_with_defaults(
            &empty_params,
            &params,
            &empty_params,
            &vararg,
            &kwarg,
            None,
            func_code,
        )?;
        if let Some(iter) = outer_iter {
            self.compile_expr(&iter)?;
        }
        self.emit(Opcode::CallFunction, Some(params.len() as u32));
        Ok(())
    }

    fn compile_assign_targets(
        &mut self,
        targets: &[AssignTarget],
        value: &Expr,
    ) -> Result<(), CompileError> {
        if targets.is_empty() {
            return Ok(());
        }
        self.compile_expr(value)?;
        for (idx, target) in targets.iter().enumerate() {
            if idx + 1 < targets.len() {
                self.emit(Opcode::DupTop, None);
            }
            self.compile_store_target_from_stack(target)?;
        }
        Ok(())
    }

    fn compile_assign_target(
        &mut self,
        target: &AssignTarget,
        value: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_assign_targets(std::slice::from_ref(target), value)
    }

    fn compile_delete(&mut self, targets: &[AssignTarget]) -> Result<(), CompileError> {
        for target in targets {
            self.compile_delete_target(target)?;
        }
        Ok(())
    }

    fn compile_delete_target(&mut self, target: &AssignTarget) -> Result<(), CompileError> {
        match target {
            AssignTarget::Name(name) => {
                let idx = self.code.add_name(name.clone());
                self.emit(Opcode::DeleteName, Some(idx));
                Ok(())
            }
            AssignTarget::Starred(_) => Err(CompileError::new("cannot delete starred target")),
            AssignTarget::Attribute { value, name } => {
                self.compile_expr(value)?;
                let idx = self.code.add_name(name.clone());
                self.emit(Opcode::DeleteAttr, Some(idx));
                Ok(())
            }
            AssignTarget::Subscript { value, index } => {
                self.compile_expr(value)?;
                self.compile_expr(index)?;
                self.emit(Opcode::DeleteSubscript, None);
                Ok(())
            }
            AssignTarget::Tuple(items) | AssignTarget::List(items) => {
                for item in items {
                    self.compile_delete_target(item)?;
                }
                Ok(())
            }
        }
    }

    fn compile_ann_assign(
        &mut self,
        target: &AssignTarget,
        annotation: &Expr,
        value: Option<&Expr>,
    ) -> Result<(), CompileError> {
        match target {
            AssignTarget::Name(name) => {
                self.ensure_local_name("__annotations__");
                self.emit_load_name("__annotations__")?;
                self.emit_const(Value::Str(name.clone()));
                self.emit_annotation_expr(annotation)?;
                self.emit(Opcode::DictSet, None);
                self.emit_store_name_scoped("__annotations__")?;
                if let Some(expr) = value {
                    self.compile_assign_target(target, expr)?;
                }
            }
            _ => {
                self.emit_annotation_expr(annotation)?;
                self.emit(Opcode::PopTop, None);
                if let Some(expr) = value {
                    self.compile_assign_target(target, expr)?;
                }
            }
        }
        Ok(())
    }

    fn compile_store_target_from_stack(
        &mut self,
        target: &AssignTarget,
    ) -> Result<(), CompileError> {
        match target {
            AssignTarget::Name(name) => {
                self.emit_store_name_scoped(name)?;
                Ok(())
            }
            AssignTarget::Starred(item) => self.compile_store_target_from_stack(item),
            AssignTarget::Tuple(items) | AssignTarget::List(items) => {
                let starred = items
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, item)| {
                        if matches!(item, AssignTarget::Starred(_)) {
                            Some(idx)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                if starred.len() > 1 {
                    return Err(CompileError::new("multiple starred targets in assignment"));
                }
                if let Some(star_idx) = starred.first().copied() {
                    let before = star_idx as u32;
                    let after = (items.len() - star_idx - 1) as u32;
                    let packed = (after << 16) | before;
                    self.emit(Opcode::UnpackEx, Some(packed));
                } else {
                    self.emit(Opcode::UnpackSequence, Some(items.len() as u32));
                }
                for item in items.iter().rev() {
                    self.compile_store_target_from_stack(item)?;
                }
                Ok(())
            }
            AssignTarget::Attribute { value, name } => {
                let temp = self.fresh_temp("assign");
                self.emit_store_name(&temp);
                self.compile_expr(value)?;
                self.emit_load_name(&temp)?;
                let idx = self.code.add_name(name.clone());
                self.emit(Opcode::StoreAttr, Some(idx));
                self.emit_delete_name(&temp);
                Ok(())
            }
            AssignTarget::Subscript { value, index } => {
                let temp = self.fresh_temp("assign");
                self.emit_store_name(&temp);
                self.compile_expr(value)?;
                self.compile_expr(index)?;
                self.emit_load_name(&temp)?;
                self.emit(Opcode::StoreSubscript, None);
                self.emit(Opcode::PopTop, None);
                self.emit_delete_name(&temp);
                Ok(())
            }
        }
    }

    fn compile_aug_assign(
        &mut self,
        target: &AssignTarget,
        op: &crate::ast::AugOp,
        value: &Expr,
    ) -> Result<(), CompileError> {
        match target {
            AssignTarget::Name(name) => {
                self.emit_load_name(name)?;
                self.compile_expr(value)?;
                let opcode = match op {
                    crate::ast::AugOp::Add => Opcode::BinaryAdd,
                    crate::ast::AugOp::Sub => Opcode::BinarySub,
                    crate::ast::AugOp::Mul => Opcode::BinaryMul,
                    crate::ast::AugOp::MatMul => Opcode::BinaryMatMul,
                    crate::ast::AugOp::Div => Opcode::BinaryDiv,
                    crate::ast::AugOp::Mod => Opcode::BinaryMod,
                    crate::ast::AugOp::FloorDiv => Opcode::BinaryFloorDiv,
                    crate::ast::AugOp::Pow => Opcode::BinaryPow,
                    crate::ast::AugOp::LShift => Opcode::BinaryLShift,
                    crate::ast::AugOp::RShift => Opcode::BinaryRShift,
                    crate::ast::AugOp::BitAnd => Opcode::BinaryAnd,
                    crate::ast::AugOp::BitXor => Opcode::BinaryXor,
                    crate::ast::AugOp::BitOr => Opcode::BinaryOr,
                };
                self.emit(opcode, None);
                self.emit_store_name_scoped(name)?;
                Ok(())
            }
            AssignTarget::Subscript {
                value: container,
                index,
            } => {
                let container_temp = self.fresh_temp("assign_obj");
                let index_temp = self.fresh_temp("assign_idx");
                let value_temp = self.fresh_temp("assign_val");
                self.compile_expr(container)?;
                self.emit_store_name(&container_temp);
                self.compile_expr(index)?;
                self.emit_store_name(&index_temp);

                self.emit_load_name(&container_temp)?;
                self.emit_load_name(&index_temp)?;
                self.emit(Opcode::Subscript, None);
                self.compile_expr(value)?;
                let opcode = match op {
                    crate::ast::AugOp::Add => Opcode::BinaryAdd,
                    crate::ast::AugOp::Sub => Opcode::BinarySub,
                    crate::ast::AugOp::Mul => Opcode::BinaryMul,
                    crate::ast::AugOp::MatMul => Opcode::BinaryMatMul,
                    crate::ast::AugOp::Div => Opcode::BinaryDiv,
                    crate::ast::AugOp::Mod => Opcode::BinaryMod,
                    crate::ast::AugOp::FloorDiv => Opcode::BinaryFloorDiv,
                    crate::ast::AugOp::Pow => Opcode::BinaryPow,
                    crate::ast::AugOp::LShift => Opcode::BinaryLShift,
                    crate::ast::AugOp::RShift => Opcode::BinaryRShift,
                    crate::ast::AugOp::BitAnd => Opcode::BinaryAnd,
                    crate::ast::AugOp::BitXor => Opcode::BinaryXor,
                    crate::ast::AugOp::BitOr => Opcode::BinaryOr,
                };
                self.emit(opcode, None);
                self.emit_store_name(&value_temp);
                self.emit_load_name(&container_temp)?;
                self.emit_load_name(&index_temp)?;
                self.emit_load_name(&value_temp)?;
                self.emit(Opcode::StoreSubscript, None);
                self.emit(Opcode::PopTop, None);
                Ok(())
            }
            AssignTarget::Attribute {
                value: object,
                name,
            } => {
                let temp = self.fresh_temp("assign_obj");
                let value_temp = self.fresh_temp("assign_val");
                self.compile_expr(object)?;
                self.emit_store_name(&temp);
                self.emit_load_name(&temp)?;
                let idx = self.code.add_name(name.clone());
                self.emit(Opcode::LoadAttr, Some(idx << 1));
                self.compile_expr(value)?;
                let opcode = match op {
                    crate::ast::AugOp::Add => Opcode::BinaryAdd,
                    crate::ast::AugOp::Sub => Opcode::BinarySub,
                    crate::ast::AugOp::Mul => Opcode::BinaryMul,
                    crate::ast::AugOp::MatMul => Opcode::BinaryMatMul,
                    crate::ast::AugOp::Div => Opcode::BinaryDiv,
                    crate::ast::AugOp::Mod => Opcode::BinaryMod,
                    crate::ast::AugOp::FloorDiv => Opcode::BinaryFloorDiv,
                    crate::ast::AugOp::Pow => Opcode::BinaryPow,
                    crate::ast::AugOp::LShift => Opcode::BinaryLShift,
                    crate::ast::AugOp::RShift => Opcode::BinaryRShift,
                    crate::ast::AugOp::BitAnd => Opcode::BinaryAnd,
                    crate::ast::AugOp::BitXor => Opcode::BinaryXor,
                    crate::ast::AugOp::BitOr => Opcode::BinaryOr,
                };
                self.emit(opcode, None);
                self.emit_store_name(&value_temp);
                self.emit_load_name(&temp)?;
                self.emit_load_name(&value_temp)?;
                let idx = self.code.add_name(name.clone());
                self.emit(Opcode::StoreAttr, Some(idx));
                Ok(())
            }
            _ => Err(CompileError::new("invalid augmented assignment target")),
        }
    }

    fn compile_for(
        &mut self,
        target: &AssignTarget,
        iter: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
    ) -> Result<(), CompileError> {
        self.compile_expr(iter)?;
        self.emit(Opcode::GetIter, None);

        let loop_start = self.current_ip();
        let jump_if_exhausted = self.emit_jump(Opcode::ForIter);
        self.compile_store_target_from_stack(target)?;

        self.loop_stack.push(LoopContext {
            start: loop_start,
            continue_target: Some(loop_start),
            break_cleanup_pops: 1,
            breaks: Vec::new(),
            continues: Vec::new(),
        });

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        self.emit(Opcode::Jump, Some(loop_start as u32));
        let else_start = self.current_ip();
        self.patch_jump(jump_if_exhausted, else_start)?;
        let loop_ctx = self
            .loop_stack
            .pop()
            .ok_or_else(|| CompileError::new("loop stack underflow"))?;

        for stmt in orelse {
            self.compile_stmt(stmt)?;
        }

        let loop_end = self.current_ip();
        self.resolve_loop_context(loop_ctx, loop_end)?;

        Ok(())
    }

    fn compile_async_for(
        &mut self,
        target: &AssignTarget,
        iter: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
    ) -> Result<(), CompileError> {
        let span = iter.span;
        let iter_temp = self.fresh_temp("aiter");
        let exhausted_temp = self.fresh_temp("async_for_exhausted");
        let iter_assign = Stmt::new(
            StmtKind::Assign {
                targets: vec![AssignTarget::Name(iter_temp.clone())],
                value: Expr::new(
                    ExprKind::Call {
                        func: Box::new(Expr::new(ExprKind::Name("aiter".to_string()), span)),
                        args: vec![CallArg::Positional(iter.clone())],
                    },
                    span,
                ),
            },
            span,
        );
        self.compile_stmt(&iter_assign)?;
        let exhausted_assign = Stmt::new(
            StmtKind::Assign {
                targets: vec![AssignTarget::Name(exhausted_temp.clone())],
                value: Expr::new(ExprKind::Constant(Constant::Bool(false)), span),
            },
            span,
        );
        self.compile_stmt(&exhausted_assign)?;

        let fetch_stmt = Stmt::new(
            StmtKind::Assign {
                targets: vec![target.clone()],
                value: Expr::new(
                    ExprKind::Await {
                        value: Box::new(Expr::new(
                            ExprKind::Call {
                                func: Box::new(Expr::new(
                                    ExprKind::Name("anext".to_string()),
                                    span,
                                )),
                                args: vec![CallArg::Positional(Expr::new(
                                    ExprKind::Name(iter_temp.clone()),
                                    span,
                                ))],
                            },
                            span,
                        )),
                    },
                    span,
                ),
            },
            span,
        );

        let fetch_try = Stmt::new(
            StmtKind::Try {
                body: vec![fetch_stmt],
                handlers: vec![ExceptHandler {
                    type_expr: Some(Expr::new(
                        ExprKind::Name("StopAsyncIteration".to_string()),
                        span,
                    )),
                    name: None,
                    is_star: false,
                    body: vec![Stmt::new(
                        StmtKind::Assign {
                            targets: vec![AssignTarget::Name(exhausted_temp.clone())],
                            value: Expr::new(ExprKind::Constant(Constant::Bool(true)), span),
                        },
                        span,
                    )],
                }],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            },
            span,
        );
        let break_if_exhausted = Stmt::new(
            StmtKind::If {
                test: Expr::new(ExprKind::Name(exhausted_temp.clone()), span),
                body: vec![Stmt::new(StmtKind::Break, span)],
                orelse: Vec::new(),
            },
            span,
        );

        let mut while_body = vec![fetch_try, break_if_exhausted];
        while_body.extend(body.iter().cloned());
        let while_stmt = Stmt::new(
            StmtKind::While {
                test: Expr::new(ExprKind::Constant(Constant::Bool(true)), span),
                body: while_body,
                orelse: Vec::new(),
            },
            span,
        );
        self.compile_stmt(&while_stmt)?;
        if !orelse.is_empty() {
            let orelse_stmt = Stmt::new(
                StmtKind::If {
                    test: Expr::new(ExprKind::Name(exhausted_temp), span),
                    body: orelse.to_vec(),
                    orelse: Vec::new(),
                },
                span,
            );
            self.compile_stmt(&orelse_stmt)?;
        }
        Ok(())
    }

    fn compile_with(
        &mut self,
        context: &Expr,
        target: Option<&AssignTarget>,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        let ctx_temp = self.fresh_temp("ctx");
        let exc_temp = self.fresh_temp("ctx_exc");
        self.compile_expr(context)?;
        self.emit_store_name(&ctx_temp);
        self.emit(Opcode::LoadConst, Some(0));
        self.emit_store_name(&exc_temp);

        self.emit_load_name(&ctx_temp)?;
        let enter_idx = self.code.add_name("__enter__".to_string());
        self.emit(Opcode::LoadAttr, Some(enter_idx << 1));
        self.emit(Opcode::CallFunction, Some(0));
        if let Some(target) = target {
            self.compile_store_target_from_stack(target)?;
        } else {
            self.emit(Opcode::PopTop, None);
        }

        let setup_except = self.emit_jump(Opcode::SetupExcept);
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.emit(Opcode::PopBlock, None);
        self.emit_with_exit(&ctx_temp)?;
        let jump_to_end = self.emit_jump(Opcode::Jump);

        let handler_start = self.current_ip();
        self.patch_jump(setup_except, handler_start)?;
        self.emit_store_name(&exc_temp);
        self.emit_load_name(&ctx_temp)?;
        let exit_idx = self.code.add_name("__exit__".to_string());
        self.emit(Opcode::LoadAttr, Some(exit_idx << 1));
        self.emit_load_name(&exc_temp)?;
        let class_idx = self.code.add_name("__class__".to_string());
        self.emit(Opcode::LoadAttr, Some(class_idx << 1));
        self.emit_load_name(&exc_temp)?;
        self.emit(Opcode::LoadConst, Some(0));
        self.emit(Opcode::CallFunction, Some(3));
        let jump_if_not_suppressed = self.emit_jump(Opcode::JumpIfFalse);
        self.emit(Opcode::ClearException, None);
        let suppressed_jump = self.emit_jump(Opcode::Jump);
        let reraise_target = self.current_ip();
        self.patch_jump(jump_if_not_suppressed, reraise_target)?;
        self.emit(Opcode::ClearException, None);
        self.emit_load_name(&exc_temp)?;
        self.emit(Opcode::Raise, Some(1));

        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        self.patch_jump(suppressed_jump, end_target)?;
        self.emit_delete_name(&exc_temp);
        self.emit_delete_name(&ctx_temp);
        Ok(())
    }

    fn compile_async_with(
        &mut self,
        context: &Expr,
        target: Option<&AssignTarget>,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        let span = context.span;
        let ctx_temp = self.fresh_temp("actx");

        let assign_ctx = Stmt::new(
            StmtKind::Assign {
                targets: vec![AssignTarget::Name(ctx_temp.clone())],
                value: context.clone(),
            },
            span,
        );
        self.compile_stmt(&assign_ctx)?;

        let enter_call = Expr::new(
            ExprKind::Await {
                value: Box::new(Expr::new(
                    ExprKind::Call {
                        func: Box::new(Expr::new(
                            ExprKind::Attribute {
                                value: Box::new(Expr::new(ExprKind::Name(ctx_temp.clone()), span)),
                                name: "__aenter__".to_string(),
                            },
                            span,
                        )),
                        args: Vec::new(),
                    },
                    span,
                )),
            },
            span,
        );
        let enter_stmt = if let Some(target) = target {
            Stmt::new(
                StmtKind::Assign {
                    targets: vec![target.clone()],
                    value: enter_call,
                },
                span,
            )
        } else {
            Stmt::new(StmtKind::Expr(enter_call), span)
        };
        self.compile_stmt(&enter_stmt)?;

        let exit_call = Stmt::new(
            StmtKind::Expr(Expr::new(
                ExprKind::Await {
                    value: Box::new(Expr::new(
                        ExprKind::Call {
                            func: Box::new(Expr::new(
                                ExprKind::Attribute {
                                    value: Box::new(Expr::new(
                                        ExprKind::Name(ctx_temp.clone()),
                                        span,
                                    )),
                                    name: "__aexit__".to_string(),
                                },
                                span,
                            )),
                            args: vec![
                                CallArg::Positional(Expr::new(
                                    ExprKind::Constant(Constant::None),
                                    span,
                                )),
                                CallArg::Positional(Expr::new(
                                    ExprKind::Constant(Constant::None),
                                    span,
                                )),
                                CallArg::Positional(Expr::new(
                                    ExprKind::Constant(Constant::None),
                                    span,
                                )),
                            ],
                        },
                        span,
                    )),
                },
                span,
            )),
            span,
        );

        let try_stmt = Stmt::new(
            StmtKind::Try {
                body: body.to_vec(),
                handlers: Vec::new(),
                orelse: Vec::new(),
                finalbody: vec![exit_call],
            },
            span,
        );
        self.compile_stmt(&try_stmt)
    }

    fn emit_with_exit(&mut self, ctx_temp: &str) -> Result<(), CompileError> {
        self.emit_load_name(ctx_temp)?;
        let exit_idx = self.code.add_name("__exit__".to_string());
        self.emit(Opcode::LoadAttr, Some(exit_idx << 1));
        self.emit(Opcode::LoadConst, Some(0));
        self.emit(Opcode::LoadConst, Some(0));
        self.emit(Opcode::LoadConst, Some(0));
        self.emit(Opcode::CallFunction, Some(3));
        self.emit(Opcode::PopTop, None);
        Ok(())
    }

    fn compile_raise(
        &mut self,
        value: Option<&Expr>,
        cause: Option<&Expr>,
    ) -> Result<(), CompileError> {
        if let Some(expr) = value {
            self.compile_expr(expr)?;
            if let Some(cause) = cause {
                self.compile_expr(cause)?;
                self.emit(Opcode::Raise, Some(2));
            } else {
                self.emit(Opcode::Raise, Some(1));
            }
        } else if cause.is_some() {
            return Err(CompileError::new("raise from requires an exception value"));
        } else {
            self.emit(Opcode::Raise, Some(0));
        }
        Ok(())
    }

    fn compile_assert(&mut self, test: &Expr, message: Option<&Expr>) -> Result<(), CompileError> {
        self.compile_expr(test)?;
        let jump_if_true = self.emit_jump(Opcode::JumpIfTrue);

        self.emit_load_name("AssertionError")?;
        if let Some(expr) = message {
            self.compile_expr(expr)?;
            self.emit(Opcode::CallFunction, Some(1));
        } else {
            self.emit(Opcode::CallFunction, Some(0));
        }
        self.emit(Opcode::Raise, Some(1));

        let end_target = self.current_ip();
        self.patch_jump(jump_if_true, end_target)?;
        Ok(())
    }

    fn push_finally_return_context(&mut self) -> Result<(), CompileError> {
        let return_value_name = self.fresh_temp("finally_return_value");
        let return_flag_name = self.fresh_temp("finally_return_flag");
        self.emit_const(Value::Bool(false));
        self.emit_store_name_scoped(&return_flag_name)?;
        self.finally_return_stack.push(FinallyReturnContext {
            return_value_name,
            return_flag_name,
            pending_return_jumps: Vec::new(),
        });
        Ok(())
    }

    fn pop_finally_return_context(&mut self) -> Result<FinallyReturnContext, CompileError> {
        self.finally_return_stack
            .pop()
            .ok_or_else(|| CompileError::new("missing finally return context"))
    }

    fn emit_return_or_defer(&mut self) -> Result<(), CompileError> {
        if self.finally_return_stack.is_empty() {
            self.emit(Opcode::ReturnValue, None);
            return Ok(());
        }
        let context_index = self.finally_return_stack.len() - 1;
        let return_value_name = self.finally_return_stack[context_index]
            .return_value_name
            .clone();
        let return_flag_name = self.finally_return_stack[context_index]
            .return_flag_name
            .clone();
        self.emit_store_name_scoped(&return_value_name)?;
        self.emit_const(Value::Bool(true));
        self.emit_store_name_scoped(&return_flag_name)?;
        let jump = self.emit_jump(Opcode::Jump);
        self.finally_return_stack[context_index]
            .pending_return_jumps
            .push(jump);
        Ok(())
    }

    fn patch_deferred_returns_to(
        &mut self,
        context: &mut FinallyReturnContext,
        target: usize,
    ) -> Result<(), CompileError> {
        for jump in context.pending_return_jumps.drain(..) {
            self.patch_jump(jump, target)?;
        }
        Ok(())
    }

    fn emit_finally_return_epilogue(
        &mut self,
        context: &FinallyReturnContext,
    ) -> Result<(), CompileError> {
        self.emit_load_name(&context.return_flag_name)?;
        let no_return = self.emit_jump(Opcode::JumpIfFalse);
        self.emit_load_name(&context.return_value_name)?;
        self.emit_return_or_defer()?;
        let end = self.current_ip();
        self.patch_jump(no_return, end)?;
        Ok(())
    }

    fn compile_try(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
        finalbody: &[Stmt],
    ) -> Result<(), CompileError> {
        if handlers.is_empty() {
            if finalbody.is_empty() {
                return Err(CompileError::new("try requires except or finally"));
            }
            return self.compile_try_finally(body, finalbody);
        }

        let has_star = handlers.iter().any(|handler| handler.is_star);
        let has_plain = handlers.iter().any(|handler| !handler.is_star);
        if has_star && has_plain {
            return Err(CompileError::new(
                "cannot mix 'except' and 'except*' in the same try",
            ));
        }

        if has_star {
            if finalbody.is_empty() {
                return self.compile_try_except_star(body, handlers, orelse);
            }
            return self.compile_try_except_star_finally(body, handlers, orelse, finalbody);
        }

        if finalbody.is_empty() {
            return self.compile_try_except(body, handlers, orelse);
        }

        self.compile_try_except_finally(body, handlers, orelse, finalbody)
    }

    fn compile_try_except(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
    ) -> Result<(), CompileError> {
        let setup_except = self.emit_jump(Opcode::SetupExcept);
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.emit(Opcode::PopBlock, None);

        for stmt in orelse {
            self.compile_stmt(stmt)?;
        }

        let jump_to_end = self.emit_jump(Opcode::Jump);
        let handler_start = self.current_ip();
        self.patch_jump(setup_except, handler_start)?;

        let mut end_jumps = Vec::new();
        for handler in handlers {
            let mut next_handler_jump = None;
            if let Some(type_expr) = &handler.type_expr {
                self.emit(Opcode::DupTop, None);
                self.compile_expr(type_expr)?;
                self.emit(Opcode::MatchException, None);
                next_handler_jump = Some(self.emit_jump(Opcode::JumpIfFalse));
            }

            if let Some(name) = &handler.name {
                self.emit_store_name_scoped(name)?;
            } else {
                self.emit(Opcode::PopTop, None);
            }

            for stmt in &handler.body {
                self.compile_stmt(stmt)?;
            }
            self.emit(Opcode::ClearException, None);
            end_jumps.push(self.emit_jump(Opcode::Jump));

            if let Some(next_handler_jump) = next_handler_jump {
                let next_handler_start = self.current_ip();
                self.patch_jump(next_handler_jump, next_handler_start)?;
            }
        }

        self.emit(Opcode::Raise, Some(1));
        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        for jump in end_jumps {
            self.patch_jump(jump, end_target)?;
        }

        Ok(())
    }

    fn compile_try_except_star(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
    ) -> Result<(), CompileError> {
        let setup_except = self.emit_jump(Opcode::SetupExcept);
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.emit(Opcode::PopBlock, None);

        for stmt in orelse {
            self.compile_stmt(stmt)?;
        }

        let jump_to_end = self.emit_jump(Opcode::Jump);
        let handler_start = self.current_ip();
        self.patch_jump(setup_except, handler_start)?;

        // The raised exception is on the stack at handler entry.
        let remaining_name = self.fresh_temp("except_star_remaining");
        self.emit_store_name(&remaining_name);

        for handler in handlers {
            if !handler.is_star {
                return Err(CompileError::new(
                    "cannot mix 'except' and 'except*' in the same try",
                ));
            }
            let type_expr = handler
                .type_expr
                .as_ref()
                .ok_or_else(|| CompileError::new("except* requires an exception type"))?;

            // Split the current remainder against this handler type.
            self.emit_load_name(&remaining_name)?;
            self.compile_expr(type_expr)?;
            self.emit(Opcode::MatchExceptionStar, None);
            // Stack layout now: [matched_or_none, remainder_or_none]
            self.emit_store_name(&remaining_name);
            // Keep a copy for null-check while preserving the matched value for binding.
            self.emit(Opcode::DupTop, None);
            let next_handler_jump = self.emit_jump(Opcode::JumpIfNone);

            if let Some(name) = &handler.name {
                self.emit_store_name_scoped(name)?;
            } else {
                self.emit(Opcode::PopTop, None);
            }

            for stmt in &handler.body {
                self.compile_stmt(stmt)?;
            }
            self.emit(Opcode::ClearException, None);
            let matched_continue_jump = self.emit_jump(Opcode::Jump);

            let next_handler_start = self.current_ip();
            self.patch_jump(next_handler_jump, next_handler_start)?;
            // Drop the unmatched marker (`None`) before evaluating the next handler.
            self.emit(Opcode::PopTop, None);
            let continue_target = self.current_ip();
            self.patch_jump(matched_continue_jump, continue_target)?;
        }

        // Re-raise any remainder that no except* handler matched.
        self.emit_load_name(&remaining_name)?;
        let no_reraise_jump = self.emit_jump(Opcode::JumpIfNone);
        self.emit_load_name(&remaining_name)?;
        self.emit(Opcode::Raise, Some(1));
        let after_reraise = self.current_ip();
        self.patch_jump(no_reraise_jump, after_reraise)?;

        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;

        Ok(())
    }

    fn compile_try_except_finally(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
        finalbody: &[Stmt],
    ) -> Result<(), CompileError> {
        self.push_finally_return_context()?;
        let setup_finally = self.emit_jump(Opcode::SetupExcept);
        let compile_result = self.compile_try_except(body, handlers, orelse);
        let mut return_context = self.pop_finally_return_context()?;
        compile_result?;
        self.emit(Opcode::PopBlock, None);

        let finally_start = self.current_ip();
        self.patch_deferred_returns_to(&mut return_context, finally_start)?;
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit_finally_return_epilogue(&return_context)?;
        let jump_to_end = self.emit_jump(Opcode::Jump);

        let handler_start = self.current_ip();
        self.patch_jump(setup_finally, handler_start)?;
        let finally_exc_name = self.fresh_temp("finally_exc");
        self.emit_store_name(&finally_exc_name);
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit_load_name(&finally_exc_name)?;
        self.emit(Opcode::Raise, Some(1));

        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        Ok(())
    }

    fn compile_try_except_star_finally(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
        finalbody: &[Stmt],
    ) -> Result<(), CompileError> {
        self.push_finally_return_context()?;
        let setup_finally = self.emit_jump(Opcode::SetupExcept);
        let compile_result = self.compile_try_except_star(body, handlers, orelse);
        let mut return_context = self.pop_finally_return_context()?;
        compile_result?;
        self.emit(Opcode::PopBlock, None);

        let finally_start = self.current_ip();
        self.patch_deferred_returns_to(&mut return_context, finally_start)?;
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit_finally_return_epilogue(&return_context)?;
        let jump_to_end = self.emit_jump(Opcode::Jump);

        let handler_start = self.current_ip();
        self.patch_jump(setup_finally, handler_start)?;
        let finally_exc_name = self.fresh_temp("finally_exc");
        self.emit_store_name(&finally_exc_name);
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit_load_name(&finally_exc_name)?;
        self.emit(Opcode::Raise, Some(1));

        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        Ok(())
    }

    fn compile_try_finally(
        &mut self,
        body: &[Stmt],
        finalbody: &[Stmt],
    ) -> Result<(), CompileError> {
        self.push_finally_return_context()?;
        let setup_except = self.emit_jump(Opcode::SetupExcept);
        let compile_result = (|| -> Result<(), CompileError> {
            for stmt in body {
                self.compile_stmt(stmt)?;
            }
            Ok(())
        })();
        let mut return_context = self.pop_finally_return_context()?;
        compile_result?;
        self.emit(Opcode::PopBlock, None);

        let finally_start = self.current_ip();
        self.patch_deferred_returns_to(&mut return_context, finally_start)?;
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit_finally_return_epilogue(&return_context)?;
        let jump_to_end = self.emit_jump(Opcode::Jump);

        let handler_start = self.current_ip();
        self.patch_jump(setup_except, handler_start)?;
        let finally_exc_name = self.fresh_temp("finally_exc");
        self.emit_store_name(&finally_exc_name);
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit_load_name(&finally_exc_name)?;
        self.emit(Opcode::Raise, Some(1));

        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        Ok(())
    }

    fn compile_bool_op(
        &mut self,
        op: &crate::ast::BoolOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expr(left)?;
        self.emit(Opcode::DupTop, None);

        match op {
            crate::ast::BoolOp::And => {
                let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);
                self.emit(Opcode::PopTop, None);
                self.compile_expr(right)?;
                let end = self.current_ip();
                self.patch_jump(jump_if_false, end)?;
            }
            crate::ast::BoolOp::Or => {
                let jump_if_true = self.emit_jump(Opcode::JumpIfTrue);
                self.emit(Opcode::PopTop, None);
                self.compile_expr(right)?;
                let end = self.current_ip();
                self.patch_jump(jump_if_true, end)?;
            }
        }

        Ok(())
    }

    fn compile_if_expr(
        &mut self,
        test: &Expr,
        body: &Expr,
        orelse: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expr(test)?;
        let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);
        self.compile_expr(body)?;
        let jump_to_end = self.emit_jump(Opcode::Jump);
        let else_target = self.current_ip();
        self.patch_jump(jump_if_false, else_target)?;
        self.compile_expr(orelse)?;
        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        Ok(())
    }

    fn fresh_temp(&mut self, prefix: &str) -> String {
        let name = format!("__pyrs_{prefix}_{}", self.temp_counter);
        self.temp_counter += 1;
        self.scope.locals.insert(name.clone());
        name
    }

    fn compile_break(&mut self) -> Result<(), CompileError> {
        let break_cleanup_pops = self
            .loop_stack
            .last()
            .ok_or_else(|| CompileError::new("break outside loop"))?
            .break_cleanup_pops;
        for _ in 0..break_cleanup_pops {
            self.emit(Opcode::PopTop, None);
        }
        let jump = self.emit_jump(Opcode::Jump);
        let ctx = self
            .loop_stack
            .last_mut()
            .ok_or_else(|| CompileError::new("break outside loop"))?;
        ctx.breaks.push(jump);
        Ok(())
    }

    fn compile_continue(&mut self) -> Result<(), CompileError> {
        let jump = self.emit_jump(Opcode::Jump);
        let ctx = self
            .loop_stack
            .last_mut()
            .ok_or_else(|| CompileError::new("continue outside loop"))?;
        ctx.continues.push(jump);
        Ok(())
    }

    fn resolve_loop_context(&mut self, ctx: LoopContext, loop_end: usize) -> Result<(), CompileError> {
        for jump in ctx.breaks {
            self.patch_jump(jump, loop_end)?;
        }
        let continue_target = ctx.continue_target.unwrap_or(ctx.start);
        for jump in ctx.continues {
            self.patch_jump(jump, continue_target)?;
        }
        Ok(())
    }
}

fn annotation_binary_op_symbol(op: &BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::MatMul => "@",
        BinaryOp::Div => "/",
        BinaryOp::Pow => "**",
        BinaryOp::FloorDiv => "//",
        BinaryOp::Mod => "%",
        BinaryOp::LShift => "<<",
        BinaryOp::RShift => ">>",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitXor => "^",
        BinaryOp::BitOr => "|",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::In => "in",
        BinaryOp::NotIn => "not in",
        BinaryOp::Is => "is",
        BinaryOp::IsNot => "is not",
    }
}

fn annotation_unary_op_symbol(op: &UnaryOp) -> &'static str {
    match op {
        UnaryOp::Not => "not ",
        UnaryOp::Pos => "+",
        UnaryOp::Neg => "-",
        UnaryOp::Invert => "~",
    }
}

fn annotation_bool_op_symbol(op: &BoolOp) -> &'static str {
    match op {
        BoolOp::And => "and",
        BoolOp::Or => "or",
    }
}

fn build_list_comp_body(elt: &Expr, clauses: &[ComprehensionClause]) -> Vec<Stmt> {
    let result_name = "__pyrs_comp_result".to_string();
    let append_stmt = Stmt {
        span: elt.span,
        node: StmtKind::AugAssign {
            target: AssignTarget::Name(result_name.clone()),
            op: crate::ast::AugOp::Add,
            value: Expr {
                span: elt.span,
                node: ExprKind::List(vec![elt.clone()]),
            },
        },
    };
    let mut body = vec![Stmt {
        span: elt.span,
        node: StmtKind::Assign {
            targets: vec![AssignTarget::Name(result_name.clone())],
            value: Expr {
                span: elt.span,
                node: ExprKind::List(Vec::new()),
            },
        },
    }];
    body.extend(build_comp_stmt_chain(clauses, vec![append_stmt], elt.span));
    body.push(Stmt {
        span: elt.span,
        node: StmtKind::Return {
            value: Some(Expr {
                span: elt.span,
                node: ExprKind::Name(result_name),
            }),
        },
    });
    body
}

fn build_dict_comp_body(key: &Expr, value: &Expr, clauses: &[ComprehensionClause]) -> Vec<Stmt> {
    let result_name = "__pyrs_comp_result".to_string();
    let assign_stmt = Stmt {
        span: key.span,
        node: StmtKind::Assign {
            targets: vec![AssignTarget::Subscript {
                value: Box::new(Expr {
                    span: key.span,
                    node: ExprKind::Name(result_name.clone()),
                }),
                index: Box::new(key.clone()),
            }],
            value: value.clone(),
        },
    };
    let mut body = vec![Stmt {
        span: key.span,
        node: StmtKind::Assign {
            targets: vec![AssignTarget::Name(result_name.clone())],
            value: Expr {
                span: key.span,
                node: ExprKind::Dict(Vec::new()),
            },
        },
    }];
    body.extend(build_comp_stmt_chain(clauses, vec![assign_stmt], key.span));
    body.push(Stmt {
        span: key.span,
        node: StmtKind::Return {
            value: Some(Expr {
                span: key.span,
                node: ExprKind::Name(result_name),
            }),
        },
    });
    body
}

fn build_genexpr_body(elt: &Expr, clauses: &[ComprehensionClause]) -> Vec<Stmt> {
    let yield_stmt = Stmt {
        span: elt.span,
        node: StmtKind::Expr(Expr {
            span: elt.span,
            node: ExprKind::Yield {
                value: Some(Box::new(elt.clone())),
            },
        }),
    };
    build_comp_stmt_chain(clauses, vec![yield_stmt], elt.span)
}

fn build_comp_stmt_chain(
    clauses: &[ComprehensionClause],
    leaf: Vec<Stmt>,
    span: Span,
) -> Vec<Stmt> {
    if clauses.is_empty() {
        return leaf;
    }
    let first = &clauses[0];
    let mut nested = build_comp_stmt_chain(&clauses[1..], leaf, span);
    for cond in first.ifs.iter().rev() {
        nested = vec![Stmt {
            span,
            node: StmtKind::If {
                test: cond.clone(),
                body: nested,
                orelse: Vec::new(),
            },
        }];
    }
    vec![Stmt {
        span,
        node: StmtKind::For {
            is_async: first.is_async,
            target: first.target.clone(),
            iter: first.iter.clone(),
            body: nested,
            orelse: Vec::new(),
        },
    }]
}

fn rewrite_first_comp_iter_to_param(body: &mut [Stmt]) {
    let _ = rewrite_first_comp_iter_stmt(body);
}

fn rewrite_first_comp_iter_stmt(stmts: &mut [Stmt]) -> bool {
    for stmt in stmts {
        match &mut stmt.node {
            StmtKind::For { iter, .. } => {
                *iter = Expr {
                    span: iter.span,
                    node: ExprKind::Name("__pyrs_comp_iter0".to_string()),
                };
                return true;
            }
            StmtKind::If { body, orelse, .. } => {
                if rewrite_first_comp_iter_stmt(body) {
                    return true;
                }
                if rewrite_first_comp_iter_stmt(orelse) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn pack_call_counts(positional: u32, keywords: u32) -> Result<u32, CompileError> {
    if positional > u16::MAX as u32 || keywords > u16::MAX as u32 {
        return Err(CompileError::new("too many call arguments"));
    }
    Ok((keywords << 16) | positional)
}

fn constant_to_value(constant: &Constant) -> Value {
    match constant {
        Constant::None => Value::None,
        Constant::Bool(value) => Value::Bool(*value),
        Constant::Int(value) => Value::Int(*value),
        Constant::Float(value) => Value::Float(value.value()),
        Constant::Str(value) => Value::Str(value.clone()),
    }
}
