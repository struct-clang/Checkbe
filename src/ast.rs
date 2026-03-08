use crate::span::Span;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ValueType {
    Int,
    Float,
    String,
    Bool,
    Array(Box<ValueType>),
    Void,
}

impl ValueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Float => "float",
            Self::String => "string",
            Self::Bool => "bool",
            Self::Array(_) => "array",
            Self::Void => "void",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "int" => Some(Self::Int),
            "float" => Some(Self::Float),
            "string" => Some(Self::String),
            "bool" => Some(Self::Bool),
            "void" => Some(Self::Void),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Program {
    pub imports: Vec<ImportDecl>,
    pub body: BodyDecl,
}

#[derive(Clone, Debug)]
pub struct ImportDecl {
    pub module: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct BodyDecl {
    pub name: String,
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Item {
    Capability(CapabilityDecl),
    Right(RightDecl),
    Var(VarDecl),
    Func(FuncDecl),
}

#[derive(Clone, Debug)]
pub struct CapabilityDecl {
    pub name: String,
    pub initializer: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RightDecl {
    pub name: String,
    pub initializer: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct VarDecl {
    pub name: String,
    pub explicit_type: Option<ValueType>,
    pub initializer: Expr,
    pub entitlement: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FuncDecl {
    pub name: String,
    pub params: Vec<FuncParam>,
    pub return_type: ValueType,
    pub right: Option<String>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FuncParam {
    pub name: String,
    pub ty: ValueType,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    VarDecl(VarDecl),
    Assign {
        target: AssignTarget,
        value: Expr,
        span: Span,
    },
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Vec<Stmt>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    DoWhile {
        body: Vec<Stmt>,
        condition: Expr,
        span: Span,
    },
    Expr {
        expr: Expr,
        span: Span,
    },
    Block {
        statements: Vec<Stmt>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
}

#[derive(Clone, Debug)]
pub enum AssignTarget {
    Identifier(String),
    Index { array: Expr, index: Expr },
}

#[derive(Clone, Debug)]
pub enum Expr {
    IntLiteral(i64, Span),
    FloatLiteral(f64, Span),
    StringLiteral(String, Span),
    BoolLiteral(bool, Span),
    Identifier(String, Span),
    ArrayLiteral(Vec<Expr>, Span),
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    NewObject {
        kind: String,
        args: Vec<Expr>,
        span: Span,
    },
    Call {
        callee: CallTarget,
        args: Vec<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    Ternary {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::IntLiteral(_, span)
            | Self::FloatLiteral(_, span)
            | Self::StringLiteral(_, span)
            | Self::BoolLiteral(_, span)
            | Self::Identifier(_, span)
            | Self::ArrayLiteral(_, span) => *span,
            Self::NewObject { span, .. }
            | Self::Index { span, .. }
            | Self::Call { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Ternary { span, .. } => *span,
        }
    }
}

#[derive(Clone, Debug)]
pub enum CallTarget {
    Name(String),
    Qualified { module: String, name: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Greater,
    GreaterEq,
    Less,
    LessEq,
    And,
    Or,
}
