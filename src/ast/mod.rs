//! Minimal AST definitions. These will expand to cover the full CPython 3.14 grammar.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    pub fn unknown() -> Self {
        Self { line: 0, column: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }

    pub fn map<U>(self, node: U) -> Spanned<U> {
        Spanned::new(node, self.span)
    }
}

pub type Expr = Spanned<ExprKind>;
pub type Stmt = Spanned<StmtKind>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub body: Vec<Stmt>,
}

impl Module {
    pub fn empty() -> Self {
        Self { body: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StmtKind {
    Pass,
    Expr(Expr),
    If {
        test: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    Assign {
        target: AssignTarget,
        value: Expr,
    },
    AugAssign {
        target: AssignTarget,
        op: AugOp,
        value: Expr,
    },
    FunctionDef {
        name: String,
        posonly_params: Vec<Parameter>,
        params: Vec<Parameter>,
        vararg: Option<Parameter>,
        kwarg: Option<Parameter>,
        kwonly_params: Vec<Parameter>,
        returns: Option<Expr>,
        body: Vec<Stmt>,
    },
    ClassDef {
        name: String,
        bases: Vec<Expr>,
        body: Vec<Stmt>,
    },
    AnnAssign {
        target: AssignTarget,
        annotation: Expr,
        value: Option<Expr>,
    },
    Return {
        value: Option<Expr>,
    },
    Raise {
        value: Option<Expr>,
    },
    Assert {
        test: Expr,
        message: Option<Expr>,
    },
    Try {
        body: Vec<Stmt>,
        handlers: Vec<ExceptHandler>,
        orelse: Vec<Stmt>,
        finalbody: Vec<Stmt>,
    },
    While {
        test: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    For {
        target: AssignTarget,
        iter: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    Import {
        names: Vec<ImportAlias>,
    },
    ImportFrom {
        module: String,
        names: Vec<ImportAlias>,
    },
    Global {
        names: Vec<String>,
    },
    Nonlocal {
        names: Vec<String>,
    },
    With {
        context: Expr,
        target: Option<AssignTarget>,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptHandler {
    pub type_expr: Option<Expr>,
    pub name: Option<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportAlias {
    pub name: String,
    pub asname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub default: Option<Box<Expr>>,
    pub annotation: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    Name(String),
    Constant(Constant),
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Call {
        func: Box<Expr>,
        args: Vec<CallArg>,
    },
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Subscript {
        value: Box<Expr>,
        index: Box<Expr>,
    },
    Attribute {
        value: Box<Expr>,
        name: String,
    },
    BoolOp {
        op: BoolOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    IfExpr {
        test: Box<Expr>,
        body: Box<Expr>,
        orelse: Box<Expr>,
    },
    Lambda {
        posonly_params: Vec<Parameter>,
        params: Vec<Parameter>,
        vararg: Option<Parameter>,
        kwarg: Option<Parameter>,
        kwonly_params: Vec<Parameter>,
        body: Box<Expr>,
    },
    Yield {
        value: Option<Box<Expr>>,
    },
    YieldFrom {
        value: Box<Expr>,
    },
    Slice {
        lower: Option<Box<Expr>>,
        upper: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignTarget {
    Name(String),
    Tuple(Vec<AssignTarget>),
    List(Vec<AssignTarget>),
    Subscript { value: Box<Expr>, index: Box<Expr> },
    Attribute { value: Box<Expr>, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallArg {
    Positional(Expr),
    Keyword { name: String, value: Expr },
    Star(Expr),
    DoubleStar(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Pow,
    FloorDiv,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    Is,
    IsNot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    Pos,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AugOp {
    Add,
    Sub,
    Mul,
    Mod,
    FloorDiv,
    Pow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constant {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
}
