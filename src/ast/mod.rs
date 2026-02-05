//! Minimal AST definitions. These will expand to cover the full CPython 3.14 grammar.

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
pub enum Stmt {
    Pass,
    Expr(Expr),
    If {
        test: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    Assign {
        target: String,
        value: Expr,
    },
    AssignSubscript {
        target: Expr,
        value: Expr,
    },
    AssignAttr {
        object: Expr,
        name: String,
        value: Expr,
    },
    AugAssign {
        target: Expr,
        op: AugOp,
        value: Expr,
    },
    FunctionDef {
        name: String,
        posonly_params: Vec<Parameter>,
        params: Vec<Parameter>,
        vararg: Option<String>,
        kwarg: Option<String>,
        kwonly_params: Vec<Parameter>,
        body: Vec<Stmt>,
    },
    ClassDef {
        name: String,
        bases: Vec<Expr>,
        body: Vec<Stmt>,
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
        target: String,
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
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
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
        vararg: Option<String>,
        kwarg: Option<String>,
        kwonly_params: Vec<Parameter>,
        body: Box<Expr>,
    },
    Slice {
        lower: Option<Box<Expr>>,
        upper: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
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
