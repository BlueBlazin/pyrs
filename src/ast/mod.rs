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
    AugAssign {
        target: Expr,
        op: AugOp,
        value: Expr,
    },
    FunctionDef {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    Return {
        value: Option<Expr>,
    },
    While {
        test: Expr,
        body: Vec<Stmt>,
    },
    For {
        target: String,
        iter: Expr,
        body: Vec<Stmt>,
    },
    Import {
        names: Vec<String>,
    },
    Break,
    Continue,
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
        args: Vec<Expr>,
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
    Slice {
        lower: Option<Box<Expr>>,
        upper: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constant {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
}
