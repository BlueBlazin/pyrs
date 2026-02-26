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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeParamKind {
    TypeVar,
    TypeVarTuple,
    ParamSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParam {
    pub kind: TypeParamKind,
    pub name: String,
    pub bound: Option<Box<Expr>>,
    pub default: Option<Box<Expr>>,
}

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
        targets: Vec<AssignTarget>,
        value: Expr,
    },
    AugAssign {
        target: AssignTarget,
        op: AugOp,
        value: Expr,
    },
    FunctionDef {
        name: String,
        type_params: Vec<TypeParam>,
        is_async: bool,
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
        type_params: Vec<TypeParam>,
        bases: Vec<Expr>,
        metaclass: Option<Expr>,
        keywords: Vec<(String, Expr)>,
        body: Vec<Stmt>,
    },
    TypeAlias {
        name: String,
        type_params: Vec<TypeParam>,
        value: Expr,
    },
    Decorated {
        decorators: Vec<Expr>,
        stmt: Box<Stmt>,
    },
    AnnAssign {
        target: AssignTarget,
        annotation: Expr,
        value: Option<Expr>,
        simple: bool,
    },
    Return {
        value: Option<Expr>,
    },
    Raise {
        value: Option<Expr>,
        cause: Option<Expr>,
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
        is_async: bool,
        target: AssignTarget,
        iter: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    Import {
        names: Vec<ImportAlias>,
    },
    ImportFrom {
        module: Option<String>,
        names: Vec<ImportAlias>,
        level: usize,
    },
    Global {
        names: Vec<String>,
    },
    Nonlocal {
        names: Vec<String>,
    },
    With {
        is_async: bool,
        context: Expr,
        target: Option<AssignTarget>,
        body: Vec<Stmt>,
    },
    Match {
        subject: Expr,
        cases: Vec<MatchCase>,
    },
    Delete {
        targets: Vec<AssignTarget>,
    },
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptHandler {
    pub type_expr: Option<Expr>,
    pub name: Option<String>,
    pub is_star: bool,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
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
pub struct TemplateInterpolation {
    pub value: Box<Expr>,
    pub expression: String,
    pub conversion: Option<char>,
    pub format_spec: Option<String>,
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
    Dict(Vec<DictEntry>),
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
    NamedExpr {
        target: String,
        value: Box<Expr>,
    },
    Lambda {
        posonly_params: Vec<Parameter>,
        params: Vec<Parameter>,
        vararg: Option<Parameter>,
        kwarg: Option<Parameter>,
        kwonly_params: Vec<Parameter>,
        body: Box<Expr>,
    },
    Await {
        value: Box<Expr>,
    },
    ListComp {
        elt: Box<Expr>,
        clauses: Vec<ComprehensionClause>,
    },
    SetComp {
        elt: Box<Expr>,
        clauses: Vec<ComprehensionClause>,
    },
    DictComp {
        key: Box<Expr>,
        value: Box<Expr>,
        clauses: Vec<ComprehensionClause>,
    },
    GeneratorExp {
        elt: Box<Expr>,
        clauses: Vec<ComprehensionClause>,
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
    TemplateLiteral {
        strings: Vec<String>,
        interpolations: Vec<TemplateInterpolation>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DictEntry {
    Pair(Expr, Expr),
    Unpack(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComprehensionClause {
    pub is_async: bool,
    pub target: AssignTarget,
    pub iter: Expr,
    pub ifs: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Wildcard,
    Capture(String),
    Constant(Constant),
    Value(Expr),
    Sequence(Vec<Pattern>),
    Mapping {
        entries: Vec<(Expr, Pattern)>,
        rest: Option<String>,
    },
    Class {
        class: Expr,
        positional: Vec<Pattern>,
        keywords: Vec<(String, Pattern)>,
    },
    Or(Vec<Pattern>),
    As {
        pattern: Box<Pattern>,
        name: String,
    },
    Star(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignTarget {
    Name(String),
    Starred(Box<AssignTarget>),
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
    MatMul,
    Div,
    Pow,
    FloorDiv,
    Mod,
    LShift,
    RShift,
    BitAnd,
    BitXor,
    BitOr,
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
    Invert,
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
    MatMul,
    Div,
    Mod,
    FloorDiv,
    Pow,
    LShift,
    RShift,
    BitAnd,
    BitXor,
    BitOr,
}

#[derive(Debug, Clone, Copy)]
pub struct FloatLiteral(pub f64);

impl FloatLiteral {
    pub fn value(self) -> f64 {
        self.0
    }
}

impl PartialEq for FloatLiteral {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for FloatLiteral {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constant {
    None,
    Bool(bool),
    Int(i64),
    Float(FloatLiteral),
    Str(String),
}
