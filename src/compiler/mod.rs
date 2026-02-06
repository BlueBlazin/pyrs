//! AST to bytecode compiler (minimal subset).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{
    AssignTarget, CallArg, Constant, ExceptHandler, Expr, ExprKind, Module, Parameter, Span, Stmt,
    StmtKind,
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
    nonlocals: HashSet<String>,
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

    fn for_lambda(
        posonly_params: &[Parameter],
        params: &[Parameter],
        kwonly_params: &[Parameter],
        vararg: &Option<Parameter>,
        kwarg: &Option<Parameter>,
        body: &Expr,
        enclosing: &ScopeInfo,
    ) -> Result<Self, CompileError> {
        analyze_scope_expr(
            ScopeType::Lambda,
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

    collect_param_locals(posonly_params, params, kwonly_params, vararg, kwarg, &mut locals);

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

    let mut direct_free: HashSet<String> = uses
        .intersection(&available_nonlocal)
        .cloned()
        .collect();
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
        nonlocals,
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
        StmtKind::Assign { target, .. }
        | StmtKind::AugAssign { target, .. }
        | StmtKind::AnnAssign { target, .. } => {
            collect_locals_target(target, locals);
        }
        StmtKind::For { target, .. } => collect_locals_target(target, locals),
        StmtKind::With { target, .. } => {
            if let Some(target) = target {
                collect_locals_target(target, locals);
            }
        }
        StmtKind::FunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => {
            locals.insert(name.clone());
        }
        StmtKind::Import { names } => {
            for alias in names {
                let binding = alias
                    .asname
                    .clone()
                    .unwrap_or_else(|| alias.name.split('.').next().unwrap_or(&alias.name).to_string());
                locals.insert(binding);
            }
        }
        StmtKind::ImportFrom { names, .. } => {
            for alias in names {
                let binding = alias.asname.clone().unwrap_or_else(|| alias.name.clone());
                locals.insert(binding);
            }
        }
        StmtKind::Try { handlers, .. } => {
            for handler in handlers {
                if let Some(name) = &handler.name {
                    locals.insert(name.clone());
                }
            }
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
}

fn collect_locals_target(target: &AssignTarget, locals: &mut HashSet<String>) {
    match target {
        AssignTarget::Name(name) => {
            locals.insert(name.clone());
        }
        AssignTarget::Tuple(items) | AssignTarget::List(items) => {
            for item in items {
                collect_locals_target(item, locals);
            }
        }
        AssignTarget::Subscript { .. } | AssignTarget::Attribute { .. } => {}
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
        StmtKind::Assign { target, value } => {
            collect_target_uses(target, uses, child_free, enclosing)?;
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
        StmtKind::For { target, iter, body, orelse } => {
            collect_target_uses(target, uses, child_free, enclosing)?;
            collect_uses_expr(iter, uses, child_free, enclosing)?;
            for stmt in body {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
            for stmt in orelse {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
        }
        StmtKind::With { context, target, body } => {
            collect_uses_expr(context, uses, child_free, enclosing)?;
            if let Some(target) = target {
                collect_target_uses(target, uses, child_free, enclosing)?;
            }
            for stmt in body {
                collect_uses_stmt(stmt, uses, child_free, enclosing)?;
            }
        }
        StmtKind::Try { body, handlers, orelse, finalbody } => {
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
        StmtKind::Raise { value } => {
            if let Some(expr) = value {
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
            for param in posonly_params.iter().chain(params.iter()).chain(kwonly_params.iter()) {
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
        StmtKind::ClassDef { bases, body, .. } => {
            for base in bases {
                collect_uses_expr(base, uses, child_free, enclosing)?;
            }
            let scope = analyze_scope(
                ScopeType::Class,
                &[],
                &[],
                &[],
                None,
                None,
                body,
                enclosing,
            )?;
            child_free.extend(scope.freevars.into_iter());
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
                    CallArg::Positional(expr)
                    | CallArg::Star(expr)
                    | CallArg::DoubleStar(expr) => {
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
            for (key, value) in entries {
                collect_uses_expr(key, uses, child_free, enclosing)?;
                collect_uses_expr(value, uses, child_free, enclosing)?;
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
                annotation,
                value,
                ..
            } => {
                if expr_has_yield(annotation)
                    || value
                        .as_ref()
                        .map(expr_has_yield)
                        .unwrap_or(false)
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
                iter,
                body,
                orelse,
                ..
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
                if body_has_yield(body)
                    || body_has_yield(orelse)
                    || body_has_yield(finalbody)
                {
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
            StmtKind::Return { value } | StmtKind::Raise { value } => {
                if value.as_ref().map(expr_has_yield).unwrap_or(false) {
                    return true;
                }
            }
            StmtKind::Assert { test, message } => {
                if expr_has_yield(test)
                    || message
                        .as_ref()
                        .map(expr_has_yield)
                        .unwrap_or(false)
                {
                    return true;
                }
            }
            StmtKind::FunctionDef { .. }
            | StmtKind::ClassDef { .. }
            | StmtKind::Import { .. }
            | StmtKind::ImportFrom { .. }
            | StmtKind::Global { .. }
            | StmtKind::Nonlocal { .. }
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
        ExprKind::Dict(entries) => entries
            .iter()
            .any(|(key, value)| expr_has_yield(key) || expr_has_yield(value)),
        ExprKind::Subscript { value, index } => expr_has_yield(value) || expr_has_yield(index),
        ExprKind::Attribute { value, .. } => expr_has_yield(value),
        ExprKind::IfExpr { test, body, orelse } => {
            expr_has_yield(test) || expr_has_yield(body) || expr_has_yield(orelse)
        }
        ExprKind::Lambda { .. } | ExprKind::Name(_) | ExprKind::Constant(_) => false,
        ExprKind::Slice { lower, upper, step } => {
            lower.as_ref().map(|expr| expr_has_yield(expr)).unwrap_or(false)
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

pub fn compile_module_with_filename(
    module: &Module,
    filename: &str,
) -> Result<CodeObject, CompileError> {
    let scope = ScopeInfo::for_module(module)?;
    let mut compiler = Compiler::new("<module>", filename, scope);
    compiler.compile_module(module)?;
    Ok(compiler.finish())
}

struct Compiler {
    code: CodeObject,
    temp_counter: usize,
    loop_stack: Vec<LoopContext>,
    scope: ScopeInfo,
    current_span: Span,
    cell_index: HashMap<String, u32>,
}

struct LoopContext {
    start: usize,
    continue_target: Option<usize>,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

impl Compiler {
    fn new(name: &str, filename: &str, scope: ScopeInfo) -> Self {
        let mut code = CodeObject::new(name, filename);
        code.cellvars = scope.cellvars.clone();
        code.freevars = scope.freevars.clone();
        let mut cell_index = HashMap::new();
        for (idx, name) in code
            .cellvars
            .iter()
            .chain(code.freevars.iter())
            .enumerate()
        {
            cell_index.insert(name.clone(), idx as u32);
        }
        Self {
            code,
            temp_counter: 0,
            loop_stack: Vec::new(),
            scope,
            current_span: Span::unknown(),
            cell_index,
        }
    }

    fn finish(mut self) -> CodeObject {
        self.emit(Opcode::LoadConst, Some(0));
        self.emit(Opcode::ReturnValue, None);
        self.code
    }

    fn compile_module(&mut self, module: &Module) -> Result<(), CompileError> {
        if body_has_ann_assign(&module.body) {
            self.init_annotations()?;
        }
        for stmt in &module.body {
            self.compile_stmt(stmt)?;
        }
        Ok(())
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
            StmtKind::Assign { target, value } => compiler.compile_assign_target(target, value),
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
                posonly_params,
                params,
                vararg,
                kwarg,
                kwonly_params,
                returns,
                body,
            } => {
                let func_code = compiler.compile_function(
                    name,
                    posonly_params,
                    params,
                    kwonly_params,
                    vararg,
                    kwarg,
                    body,
                )?;
                compiler.emit_function_with_defaults(
                    posonly_params,
                    params,
                    kwonly_params,
                    vararg,
                    kwarg,
                    returns.as_ref(),
                    func_code,
                )?;
                compiler.emit_store_name_scoped(name)?;
                Ok(())
            }
            StmtKind::ClassDef { name, bases, body } => {
                compiler.compile_class_def(name, bases, body)
            }
            StmtKind::Return { value } => {
                if let Some(expr) = value {
                    compiler.compile_expr(expr)?;
                } else {
                    compiler.emit(Opcode::LoadConst, Some(0));
                }
                compiler.emit(Opcode::ReturnValue, None);
                Ok(())
            }
            StmtKind::Raise { value } => compiler.compile_raise(value.as_ref()),
            StmtKind::Assert { test, message } => compiler.compile_assert(test, message.as_ref()),
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => compiler.compile_try(body, handlers, orelse, finalbody),
            StmtKind::For {
                target,
                iter,
                body,
                orelse,
            } => compiler.compile_for(target, iter, body, orelse),
            StmtKind::Import { names } => {
                for alias in names {
                    let const_idx = compiler
                        .code
                        .add_const(Value::Str(alias.name.clone()));
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
            StmtKind::ImportFrom { module, names } => {
                let const_idx = compiler.code.add_const(Value::Str(module.clone()));
                compiler.emit(Opcode::ImportName, Some(const_idx));
                compiler.emit_import_attr_chain(module)?;
                for alias in names {
                    compiler.emit(Opcode::DupTop, None);
                    let attr_idx = compiler.code.add_name(alias.name.clone());
                    compiler.emit(Opcode::LoadAttr, Some(attr_idx << 1));
                    let target = alias.asname.as_deref().unwrap_or(&alias.name);
                    compiler.emit_store_name_scoped(target)?;
                }
                compiler.emit(Opcode::PopTop, None);
                Ok(())
            }
            StmtKind::Global { .. } => Ok(()),
            StmtKind::Nonlocal { .. } => Ok(()),
            StmtKind::With {
                context,
                target,
                body,
            } => compiler.compile_with(context, target.as_ref(), body),
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
                compiler.compile_expr(left)?;
                compiler.compile_expr(right)?;
                let opcode = match op {
                    crate::ast::BinaryOp::Add => Opcode::BinaryAdd,
                    crate::ast::BinaryOp::Sub => Opcode::BinarySub,
                    crate::ast::BinaryOp::Mul => Opcode::BinaryMul,
                    crate::ast::BinaryOp::Pow => Opcode::BinaryPow,
                    crate::ast::BinaryOp::FloorDiv => Opcode::BinaryFloorDiv,
                    crate::ast::BinaryOp::Mod => Opcode::BinaryMod,
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
                };
                compiler.emit(opcode, None);
                Ok(())
            }
            ExprKind::BoolOp { op, left, right } => compiler.compile_bool_op(op, left, right),
            ExprKind::IfExpr { test, body, orelse } => {
                compiler.compile_if_expr(test, body, orelse)
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
            ExprKind::Call { func, args } => {
                compiler.compile_expr(func)?;
                let has_star = args.iter().any(|arg| {
                    matches!(arg, CallArg::Star(_) | CallArg::DoubleStar(_))
                });

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
                                let name_idx =
                                    compiler.code.add_const(Value::Str(name.clone()));
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
                for (key, value) in entries {
                    compiler.compile_expr(key)?;
                    compiler.compile_expr(value)?;
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
        if matches!(self.scope.scope_type, ScopeType::Function | ScopeType::Lambda) {
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
        self.code.instructions.push(Instruction::new(opcode, arg));
        self.code
            .locations
            .push(crate::bytecode::Location::new(
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
        for param in posonly_params.iter().chain(params.iter()).chain(kwonly_params.iter()) {
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
            self.compile_expr(expr)?;
        }
        self.emit(Opcode::BuildDict, Some(count as u32));
        Ok(true)
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
        self.cell_index.get(name).copied().ok_or_else(|| {
            CompileError::new(format!("unknown closure variable '{name}'"))
        })
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
            breaks: Vec::new(),
            continues: Vec::new(),
        });

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        self.emit(Opcode::Jump, Some(loop_start as u32));
        let else_start = self.current_ip();
        self.patch_jump(jump_if_false, else_start)?;

        for stmt in orelse {
            self.compile_stmt(stmt)?;
        }

        let loop_end = self.current_ip();
        self.resolve_loop(loop_end)?;
        Ok(())
    }

    fn compile_function(
        &mut self,
        name: &str,
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
        compiler.code.is_generator = body_has_yield(body);
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

    fn compile_class_def(
        &mut self,
        name: &str,
        bases: &[Expr],
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        let class_code = self.compile_class(name, body)?;
        let code_idx = self.code.add_const(Value::Code(Rc::new(class_code)));
        for base in bases {
            self.compile_expr(base)?;
        }
        self.emit(Opcode::BuildTuple, Some(bases.len() as u32));
        let name_idx = self.code.add_const(Value::Str(name.to_string()));
        self.emit(Opcode::LoadConst, Some(name_idx));
        self.emit(Opcode::BuildClass, Some(code_idx));
        self.emit_store_name_scoped(name)?;
        Ok(())
    }

    fn compile_class(&mut self, name: &str, body: &[Stmt]) -> Result<CodeObject, CompileError> {
        let scope = ScopeInfo::for_class(body, &self.scope)?;
        let mut compiler = Compiler::new(
            &format!("<class {name}>"),
            &self.code.filename,
            scope,
        );
        if body_has_ann_assign(body) {
            compiler.init_annotations()?;
        }
        for stmt in body {
            compiler.compile_stmt(stmt)?;
        }
        Ok(compiler.finish())
    }

    fn compile_assign_target(
        &mut self,
        target: &AssignTarget,
        value: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expr(value)?;
        self.compile_store_target_from_stack(target)
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
                self.compile_expr(annotation)?;
                self.emit(Opcode::DictSet, None);
                self.emit_store_name_scoped("__annotations__")?;
                if let Some(expr) = value {
                    self.compile_assign_target(target, expr)?;
                }
            }
            _ => {
                self.compile_expr(annotation)?;
                self.emit(Opcode::PopTop, None);
                if let Some(expr) = value {
                    self.compile_assign_target(target, expr)?;
                }
            }
        }
        Ok(())
    }

    fn compile_store_target_from_stack(&mut self, target: &AssignTarget) -> Result<(), CompileError> {
        match target {
            AssignTarget::Name(name) => {
                self.emit_store_name_scoped(name)?;
                Ok(())
            }
            AssignTarget::Tuple(items) | AssignTarget::List(items) => {
                self.emit(Opcode::UnpackSequence, Some(items.len() as u32));
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
                Ok(())
            }
            AssignTarget::Subscript { value, index } => {
                if let ExprKind::Name(name) = &value.node {
                    let temp = self.fresh_temp("assign");
                    self.emit_store_name(&temp);
                    self.emit_load_name(name)?;
                    self.compile_expr(index)?;
                    self.emit_load_name(&temp)?;
                    self.emit(Opcode::StoreSubscript, None);
                    self.emit_store_name_scoped(name)?;
                    Ok(())
                } else {
                    Err(CompileError::new(
                        "only name-based subscript assignments supported",
                    ))
                }
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
                    crate::ast::AugOp::Mod => Opcode::BinaryMod,
                    crate::ast::AugOp::FloorDiv => Opcode::BinaryFloorDiv,
                    crate::ast::AugOp::Pow => Opcode::BinaryPow,
                };
                self.emit(opcode, None);
                self.emit_store_name_scoped(name)?;
                Ok(())
            }
            AssignTarget::Subscript { value: container, index } => {
                if let ExprKind::Name(name) = &container.node {
                    let name = name.clone();
                    self.emit_load_name(&name)?;
                    self.compile_expr(index)?;
                    self.emit(Opcode::Subscript, None);
                    self.compile_expr(value)?;
                    let opcode = match op {
                        crate::ast::AugOp::Add => Opcode::BinaryAdd,
                        crate::ast::AugOp::Sub => Opcode::BinarySub,
                        crate::ast::AugOp::Mul => Opcode::BinaryMul,
                        crate::ast::AugOp::Mod => Opcode::BinaryMod,
                        crate::ast::AugOp::FloorDiv => Opcode::BinaryFloorDiv,
                        crate::ast::AugOp::Pow => Opcode::BinaryPow,
                    };
                    self.emit(opcode, None);
                    self.emit_load_name(&name)?;
                    self.compile_expr(index)?;
                    self.emit(Opcode::StoreSubscript, None);
                    self.emit_store_name_scoped(&name)?;
                    Ok(())
                } else {
                    Err(CompileError::new(
                        "only name-based subscript assignments supported",
                    ))
                }
            }
            AssignTarget::Attribute { value: object, name } => {
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
                    crate::ast::AugOp::Mod => Opcode::BinaryMod,
                    crate::ast::AugOp::FloorDiv => Opcode::BinaryFloorDiv,
                    crate::ast::AugOp::Pow => Opcode::BinaryPow,
                };
                self.emit(opcode, None);
                self.emit_store_name(&value_temp);
                self.emit_load_name(&temp)?;
                self.emit_load_name(&value_temp)?;
                let idx = self.code.add_name(name.clone());
                self.emit(Opcode::StoreAttr, Some(idx));
                Ok(())
            }
            _ => Err(CompileError::new(
                "invalid augmented assignment target",
            )),
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
            breaks: Vec::new(),
            continues: Vec::new(),
        });

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        self.emit(Opcode::Jump, Some(loop_start as u32));
        let else_start = self.current_ip();
        self.patch_jump(jump_if_exhausted, else_start)?;

        for stmt in orelse {
            self.compile_stmt(stmt)?;
        }

        let loop_end = self.current_ip();
        self.resolve_loop(loop_end)?;

        Ok(())
    }

    fn compile_with(
        &mut self,
        context: &Expr,
        target: Option<&AssignTarget>,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        let ctx_temp = self.fresh_temp("ctx");
        self.compile_expr(context)?;
        self.emit_store_name(&ctx_temp);

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
        self.emit(Opcode::PopTop, None);
        self.emit_with_exit(&ctx_temp)?;
        self.emit(Opcode::Raise, Some(0));

        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        Ok(())
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

    fn compile_raise(&mut self, value: Option<&Expr>) -> Result<(), CompileError> {
        if let Some(expr) = value {
            self.compile_expr(expr)?;
            self.emit(Opcode::Raise, Some(1));
        } else {
            self.emit(Opcode::Raise, Some(0));
        }
        Ok(())
    }

    fn compile_assert(
        &mut self,
        test: &Expr,
        message: Option<&Expr>,
    ) -> Result<(), CompileError> {
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

    fn compile_try_except_finally(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
        finalbody: &[Stmt],
    ) -> Result<(), CompileError> {
        let setup_finally = self.emit_jump(Opcode::SetupExcept);
        self.compile_try_except(body, handlers, orelse)?;
        self.emit(Opcode::PopBlock, None);
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        let jump_to_end = self.emit_jump(Opcode::Jump);

        let handler_start = self.current_ip();
        self.patch_jump(setup_finally, handler_start)?;
        self.emit(Opcode::PopTop, None);
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit(Opcode::Raise, Some(0));

        let end_target = self.current_ip();
        self.patch_jump(jump_to_end, end_target)?;
        Ok(())
    }

    fn compile_try_finally(
        &mut self,
        body: &[Stmt],
        finalbody: &[Stmt],
    ) -> Result<(), CompileError> {
        let setup_except = self.emit_jump(Opcode::SetupExcept);
        for stmt in body {
            self.compile_stmt(stmt)?;
        }
        self.emit(Opcode::PopBlock, None);
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        let jump_to_end = self.emit_jump(Opcode::Jump);

        let handler_start = self.current_ip();
        self.patch_jump(setup_except, handler_start)?;
        self.emit(Opcode::PopTop, None);
        for stmt in finalbody {
            self.compile_stmt(stmt)?;
        }
        self.emit(Opcode::Raise, Some(0));

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

    fn resolve_loop(&mut self, loop_end: usize) -> Result<(), CompileError> {
        let ctx = self
            .loop_stack
            .pop()
            .ok_or_else(|| CompileError::new("loop stack underflow"))?;
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
        Constant::Str(value) => Value::Str(value.clone()),
    }
}
