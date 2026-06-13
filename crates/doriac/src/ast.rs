use crate::source::Span;
use crate::types::TypeRef;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Class(ClassDecl),
    Function(FunctionDecl),
    Statement(Stmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub members: Vec<ClassMember>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassMember {
    Property(PropertyDecl),
    Method(FunctionDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyDecl {
    pub visibility: Visibility,
    pub writable: bool,
    pub ty: TypeRef,
    pub name: String,
    pub initializer: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub visibility: Option<Visibility>,
    pub writable_this: bool,
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub promoted_visibility: Option<Visibility>,
    pub writable: bool,
    pub ty: TypeRef,
    pub name: String,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    VarDecl(VarDecl),
    Assignment(Assignment),
    Echo { expr: Expr, span: Span },
    Return { expr: Option<Expr>, span: Span },
    Foreach(ForeachStmt),
    Expr { expr: Expr, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub writable: bool,
    pub ty: Option<TypeRef>,
    pub name: String,
    pub initializer: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub target: Expr,
    pub op: AssignOp,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeachStmt {
    pub iterable: Expr,
    pub key: Option<ForeachBinding>,
    pub value: ForeachBinding,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeachBinding {
    pub ty: Option<TypeRef>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Variable {
        name: String,
        span: Span,
    },
    This {
        span: Span,
    },
    Identifier {
        name: String,
        span: Span,
    },
    String {
        value: String,
        span: Span,
    },
    Int {
        value: String,
        span: Span,
    },
    Float {
        value: String,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    Null {
        span: Span,
    },
    Array {
        elements: Vec<ArrayElement>,
        span: Span,
    },
    PropertyAccess {
        object: Box<Expr>,
        property: String,
        span: Span,
    },
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    FunctionCall {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    StaticCall {
        class_name: String,
        method: String,
        args: Vec<Expr>,
        span: Span,
    },
    New {
        class_name: String,
        args: Vec<Expr>,
        span: Span,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayElement {
    pub key: Option<Expr>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Concat,
    Equal,
    StrictEqual,
    NotEqual,
    NotStrictEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    Coalesce,
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Variable { span, .. }
            | Expr::This { span }
            | Expr::Identifier { span, .. }
            | Expr::String { span, .. }
            | Expr::Int { span, .. }
            | Expr::Float { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Null { span }
            | Expr::Array { span, .. }
            | Expr::PropertyAccess { span, .. }
            | Expr::MethodCall { span, .. }
            | Expr::FunctionCall { span, .. }
            | Expr::StaticCall { span, .. }
            | Expr::New { span, .. }
            | Expr::Binary { span, .. } => *span,
        }
    }
}
