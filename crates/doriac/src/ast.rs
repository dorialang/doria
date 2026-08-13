use crate::source::Span;
use crate::types::TypeRef;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub namespace: Option<NamespaceDecl>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamespaceDecl {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Class(ClassDecl),
    Enum(EnumDecl),
    Interface(InterfaceDecl),
    Trait(TraitDecl),
    Function(FunctionDecl),
    Constant(ConstDecl),
    Statement(Stmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub name: String,
    pub name_span: Span,
    pub type_params: Vec<TypeParamDecl>,
    pub backing_type: Option<TypeRef>,
    pub cases: Vec<EnumCaseDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumCaseDecl {
    pub name: String,
    pub name_span: Span,
    pub payload: Vec<EnumPayloadField>,
    pub backing_value: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumPayloadField {
    pub ty: TypeRef,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDecl {
    pub name: String,
    pub members: Vec<ClassMember>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub type_params: Vec<TypeParamDecl>,
    pub parent: Option<String>,
    pub parent_span: Option<Span>,
    pub implements: Vec<String>,
    pub members: Vec<ClassMember>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassMember {
    Property(PropertyDecl),
    Method(FunctionDecl),
    Constant(ConstDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberAccess {
    External,
    Internal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyDecl {
    pub access: MemberAccess,
    pub is_static: bool,
    pub writable: bool,
    pub ty: TypeRef,
    pub name: String,
    pub initializer: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub access: MemberAccess,
    pub ty: Option<TypeRef>,
    pub name: String,
    pub initializer: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub access: MemberAccess,
    pub writable_this: bool,
    pub writable_span: Option<Span>,
    pub is_static: bool,
    pub static_span: Option<Span>,
    pub name: String,
    pub type_params: Vec<TypeParamDecl>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeParamDecl {
    pub name: String,
    pub constraints: Vec<TypeRef>,
    pub default_type: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub promoted_access: Option<MemberAccess>,
    pub take: bool,
    pub take_span: Option<Span>,
    pub writable: bool,
    pub writable_span: Option<Span>,
    pub ownership_modifier_insert: Span,
    pub ty: TypeRef,
    pub name: String,
    pub default: Option<Expr>,
    pub span: Span,
}

/// The name written before a named call argument (`name: value`), with the
/// span of the identifier for diagnostics. `None` on an `Argument` means the
/// argument was supplied positionally.
#[derive(Debug, Clone, PartialEq)]
pub struct ArgumentName {
    pub text: String,
    pub span: Span,
}

/// A single call-site argument. Arguments are stored in source (written) order
/// regardless of the parameter each named argument binds to; name-resolution
/// binding (decision 0098) maps them onto parameters in a later step.
#[derive(Debug, Clone, PartialEq)]
pub struct Argument {
    pub name: Option<ArgumentName>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Block(Block),
    VarDecl(VarDecl),
    Assignment(Assignment),
    Echo { expr: Expr, span: Span },
    Return { expr: Option<Expr>, span: Span },
    If(IfStmt),
    While(WhileStmt),
    DoWhile(DoWhileStmt),
    For(Box<ForStmt>),
    Break { span: Span },
    Continue { span: Span },
    Foreach(ForeachStmt),
    Increment(IncrementStmt),
    Expr { expr: Expr, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub writable: bool,
    pub ty: Option<TypeRef>,
    /// Local bindings in source order. A declaration always has at least one
    /// binding; grouped declarations keep one shared initializer here rather
    /// than cloning the expression for each name.
    pub bindings: Vec<VarBinding>,
    pub initializer: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarBinding {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub target: Expr,
    pub op: AssignOp,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub given: Option<GivenPrelude>,
    pub condition: Expr,
    pub then_block: Block,
    pub else_branch: Option<ElseBranch>,
    pub finally: Option<ControlFlowFinally>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElseBranch {
    If(Box<IfStmt>),
    Block(Block),
}

impl ElseBranch {
    pub fn span(&self) -> Span {
        match self {
            ElseBranch::If(if_stmt) => if_stmt.span,
            ElseBranch::Block(block) => block.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStmt {
    pub given: Option<GivenPrelude>,
    pub condition: Expr,
    pub body: Block,
    pub finally: Option<ControlFlowFinally>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoWhileStmt {
    pub body: Block,
    pub condition: Expr,
    pub semicolon_span: Option<Span>,
    pub finally: Option<ControlFlowFinally>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GivenPrelude {
    pub block: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlFlowFinally {
    pub keyword_span: Span,
    pub block: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub initializer: Option<ForInitializer>,
    pub condition: Option<Expr>,
    pub increment: Option<ForIncrement>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForInitializer {
    VarDecl(VarDecl),
    Assignment(Assignment),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForIncrement {
    Increment(IncrementStmt),
    Assignment(Assignment),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    ShiftLeftAssign,
    ShiftRightAssign,
    BitwiseAndAssign,
    BitwiseOrAssign,
    BitwiseXorAssign,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncrementStmt {
    pub target: Expr,
    pub op: IncrementOp,
    pub position: IncrementPosition,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncrementOp {
    Increment,
    Decrement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncrementPosition {
    Pre,
    Post,
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
    pub writable: bool,
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
    InterpolatedString {
        parts: Vec<InterpolatedStringPart>,
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
    ArrayRepeat {
        value: Box<Expr>,
        count: Box<Expr>,
        span: Span,
    },
    Index {
        collection: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    PropertyAccess {
        object: Box<Expr>,
        property: String,
        member_span: Span,
        null_safe: bool,
        span: Span,
    },
    MethodCall {
        object: Box<Expr>,
        method: String,
        member_span: Span,
        args: Vec<Argument>,
        argument_list_span: Span,
        null_safe: bool,
        span: Span,
    },
    IsType {
        expr: Box<Expr>,
        ty: TypeRef,
        span: Span,
    },
    FunctionCall {
        name: String,
        args: Vec<Argument>,
        span: Span,
    },
    StaticCall {
        qualifier: StaticQualifier,
        qualifier_span: Span,
        method: String,
        member_span: Span,
        member_sigil_span: Option<Span>,
        args: Vec<Argument>,
        argument_list_span: Span,
        span: Span,
    },
    StaticMember {
        qualifier: StaticQualifier,
        qualifier_span: Span,
        member: String,
        member_span: Span,
        member_sigil_span: Option<Span>,
        span: Span,
    },
    New {
        class_type: TypeRef,
        args: Vec<Argument>,
        /// `true` for `shared new T(...)` (record 0106): the value is created
        /// directly under shared ownership and the expression's type is
        /// `SharedReference<T>`. Recorded explicitly here rather than
        /// reconstructed from an expected type.
        shared: bool,
        span: Span,
    },
    Grouped {
        expr: Box<Expr>,
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
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
        span: Span,
    },
    Match {
        scrutinee: Box<Expr>,
        mode: MatchMode,
        arms: Vec<MatchArm>,
        origin: MatchOrigin,
        span: Span,
    },
    When(Box<WhenExpression>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenExpression {
    pub given: Option<GivenPrelude>,
    pub result_type: Option<TypeRef>,
    pub branches: Vec<WhenBranch>,
    pub finally: Option<ControlFlowFinally>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenBranch {
    pub condition: Option<Expr>,
    pub block: Block,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOrigin {
    Match,
    Ternary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    Borrowed,
    Consumed { take_span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub guard: Option<MatchGuard>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchGuard {
    pub condition: Expr,
    pub keyword_span: Span,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchPattern {
    Default {
        span: Span,
    },
    EnumCase {
        qualifier: String,
        qualifier_span: Span,
        case: String,
        case_span: Span,
        /// `None` ignores a payload. `Some` preserves an authored binding list,
        /// including an empty one, so semantic analysis can enforce exact arity.
        bindings: Option<Vec<MatchBinding>>,
        span: Span,
    },
    TypeBinding {
        ty: TypeRef,
        binding: MatchBinding,
        span: Span,
    },
    Expression(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchBinding {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticQualifier {
    Class(String),
    SelfType,
    Parent,
    InvalidStatic,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InterpolatedStringPart {
    Text { value: String, span: Span },
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayElement {
    pub key: Option<Expr>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Negate,
    BitwiseNot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    ShiftLeft,
    ShiftRight,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    Concat,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    Xor,
    Coalesce,
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Variable { span, .. }
            | Expr::This { span }
            | Expr::Identifier { span, .. }
            | Expr::String { span, .. }
            | Expr::InterpolatedString { span, .. }
            | Expr::Int { span, .. }
            | Expr::Float { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Null { span }
            | Expr::Array { span, .. }
            | Expr::ArrayRepeat { span, .. }
            | Expr::Index { span, .. }
            | Expr::PropertyAccess { span, .. }
            | Expr::MethodCall { span, .. }
            | Expr::IsType { span, .. }
            | Expr::FunctionCall { span, .. }
            | Expr::StaticCall { span, .. }
            | Expr::StaticMember { span, .. }
            | Expr::New { span, .. }
            | Expr::Grouped { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Range { span, .. }
            | Expr::Match { span, .. } => *span,
            Expr::When(when) => when.span,
        }
    }
}
