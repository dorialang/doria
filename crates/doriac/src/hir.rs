use crate::build_plan::{GeneratedFor, SourceOrigin, SourceScope, TargetKind};
use crate::names::{GlobalSymbolId, PackageIdentity, SourceIdentity};
use crate::source::{SourceFile, SourceId, Span};
use crate::types::{ResolvedType, TypeRef};

pub use crate::ast::{
    ArgumentName, AssignOp, BinaryOp, ClosureCaptureMode, ClosureForm, IncrementOp,
    IncrementPosition, MatchMode, MatchOrigin, MemberAccess, UnaryOp,
};
use crate::symbols::ClosureId;

/// Current Doria IR implementation.
///
/// The module name is historical and may change later. Public architecture
/// should describe this as Doria IR: the resolved, backend-neutral form emitted
/// before backend output. A lower native-oriented IR may come later.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub sources: Vec<SourceUnit>,
    pub packages: Vec<PackageUnit>,
    pub selected_target: SelectedTarget,
    /// Compatibility view for standalone consumers. Graph consumers use
    /// `sources`; this is the selected entry source, or the first source for a
    /// library graph.
    pub source_path: String,
    pub source_text: String,
    pub namespace: Option<NamespaceDecl>,
    pub items: Vec<Item>,
    /// Typed compiler/tooling metadata. Runtime MIR lowering never consumes
    /// this table, so attributes cannot alter Doria runtime identity or layout.
    pub attribute_metadata: crate::attributes::AttributeSemanticInfo,
    pub semantic_info: crate::semantics::SemanticInfo,
    /// Compiler-owned test metadata. Behavioral declaration calls and suite
    /// closures are absent from runtime items; generated tests are ordinary
    /// functions in `items`.
    pub test_suites: Vec<crate::testing::BehavioralTestSuite>,
    pub tests: Vec<crate::testing::TestSemanticInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUnit {
    pub id: SourceId,
    pub identity: SourceIdentity,
    pub package: PackageIdentity,
    pub display_path: String,
    pub scope: SourceScope,
    pub origin: SourceOrigin,
    pub generated_for: Option<GeneratedFor>,
    pub active: bool,
    pub source: SourceFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageUnit {
    pub identity: PackageIdentity,
    pub normal_dependencies: Vec<PackageIdentity>,
    pub development_dependencies: Vec<PackageIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedTarget {
    pub package: PackageIdentity,
    pub kind: TargetKind,
    pub entry_source: Option<SourceIdentity>,
}

impl Program {
    pub fn source(&self, id: SourceId) -> Option<&SourceFile> {
        self.sources
            .iter()
            .find(|source| source.id == id)
            .map(|source| &source.source)
    }

    pub fn selected_entry_source_id(&self) -> Option<SourceId> {
        let identity = self.selected_target.entry_source.as_ref()?;
        self.sources
            .iter()
            .find(|source| &source.identity == identity)
            .map(|source| source.id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamespaceDecl {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Item {
    Class(ClassDecl),
    Enum(EnumDecl),
    Function(FunctionDecl),
    Constant(ConstDecl),
    Statement(Stmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub global_id: Option<GlobalSymbolId>,
    pub source_identity: SourceIdentity,
    pub package: PackageIdentity,
    pub access: MemberAccess,
    pub access_span: Option<Span>,
    pub name: String,
    pub type_params: Vec<TypeParamDecl>,
    pub backing_type: Option<TypeRef>,
    pub cases: Vec<EnumCaseDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumCaseDecl {
    pub name: String,
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
pub struct ClassDecl {
    pub global_id: Option<GlobalSymbolId>,
    pub source_identity: SourceIdentity,
    pub package: PackageIdentity,
    pub access: MemberAccess,
    pub access_span: Option<Span>,
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
    pub global_id: Option<GlobalSymbolId>,
    pub source_identity: SourceIdentity,
    pub package: PackageIdentity,
    pub access: MemberAccess,
    pub access_span: Option<Span>,
    pub ty: Option<TypeRef>,
    pub name: String,
    pub initializer: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub global_id: Option<GlobalSymbolId>,
    pub source_identity: SourceIdentity,
    pub package: PackageIdentity,
    pub access: MemberAccess,
    pub access_span: Option<Span>,
    pub writable_this: bool,
    pub is_static: bool,
    pub name: String,
    pub type_params: Vec<TypeParamDecl>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    /// Source-preserving `throws` syntax. `None` means the author omitted it.
    pub throws: Option<ThrowsClause>,
    /// Effective semantic checked effects used by callable compatibility,
    /// executable IR, ABI selection, and backend lowering.
    pub checked_effects: Vec<ResolvedType>,
    pub required_checked_effects: Vec<ResolvedType>,
    pub ambient_checked_effects: Vec<ResolvedType>,
    pub test_assertion_checked_effects: Vec<ResolvedType>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThrowsClause {
    pub keyword_span: Span,
    pub entries: Vec<ThrowsEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThrowsEntry {
    pub source: TypeRef,
    pub resolved: ResolvedType,
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
    Block(Block),
    VarDecl(VarDecl),
    Assignment(Assignment),
    Echo { expr: Expr, span: Span },
    Return { expr: Option<Expr>, span: Span },
    Throw(ThrowStmt),
    Try(TryStmt),
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
pub struct ThrowStmt {
    pub keyword_span: Span,
    pub expr: Expr,
    pub error_type: ResolvedType,
    pub transfers_ownership: bool,
    pub semicolon_span: Span,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TryStmt {
    pub keyword_span: Span,
    pub body: Block,
    pub catches: Vec<CatchClause>,
    pub finally: Option<TryFinally>,
    pub uncovered_effects: Vec<ResolvedType>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchClause {
    pub keyword_span: Span,
    pub source_type: TypeRef,
    pub error_type: ResolvedType,
    pub binding: Option<CatchBinding>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchBinding {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TryFinally {
    pub keyword_span: Span,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub writable: bool,
    pub ty: Option<TypeRef>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct IncrementStmt {
    pub target: Expr,
    pub op: IncrementOp,
    pub position: IncrementPosition,
    pub span: Span,
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

/// A single call-site argument in Doria IR. Arguments remain in source (written)
/// order; MIR lowering evaluates them in this order and then assembles the callee
/// argument vector in parameter order (decision 0098).
#[derive(Debug, Clone, PartialEq)]
pub struct Argument {
    pub name: Option<ArgumentName>,
    pub value: Expr,
    pub span: Span,
}

/// Backend-independent closure syntax. The checked capture, ownership, effect,
/// and invocation plans are keyed by `closure_id` in `Program::semantic_info`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosureExpression {
    pub closure_id: ClosureId,
    pub form: ClosureForm,
    pub parameters: Vec<ClosureParameter>,
    pub return_type: Option<TypeRef>,
    pub captures: Vec<ClosureCapture>,
    pub body: ClosureBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureParameter {
    pub take: bool,
    pub writable: bool,
    pub ty: TypeRef,
    pub name: String,
    pub name_span: Span,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureCapture {
    pub mode: ClosureCaptureMode,
    pub name: String,
    pub name_span: Span,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClosureBody {
    Expression(Box<Expr>),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallableCall {
    pub callee: Box<Expr>,
    pub args: Vec<Argument>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListAlgorithmKind {
    Map,
    Filter,
    Reduce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListCallbackAccess {
    Readonly,
    Writable,
}

/// A fully specialized compiler-known List algorithm call. The frontend records
/// the contract once; later phases consume these facts instead of interpreting
/// a source method name independently.
#[derive(Debug, Clone, PartialEq)]
pub struct ListAlgorithmCall {
    pub kind: ListAlgorithmKind,
    pub receiver: Box<Expr>,
    pub arguments: Vec<Argument>,
    pub receiver_type: ResolvedType,
    pub element_type: ResolvedType,
    pub result_type: ResolvedType,
    pub accumulator_type: Option<ResolvedType>,
    pub callback_type: ResolvedType,
    pub callback_access: ListCallbackAccess,
    pub checked_effects: Vec<ResolvedType>,
    pub required_checked_effects: Vec<ResolvedType>,
    pub ambient_checked_effects: Vec<ResolvedType>,
    pub test_assertion_checked_effects: Vec<ResolvedType>,
    pub receiver_span: Span,
    pub callback_span: Span,
    pub span: Span,
}

/// One terminal compiler-owned expectation. No intermediate expectation value
/// survives semantic lowering, so this node cannot be stored or dispatched.
#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    pub matcher: crate::assertions::AssertionMatcher,
    pub negated: bool,
    pub actual: Option<Box<Expr>>,
    pub expected: Option<Box<Expr>>,
    pub user_message: Option<Box<Expr>>,
    pub actual_type: Option<ResolvedType>,
    pub expected_type: Option<ResolvedType>,
    pub member_span: Span,
    pub span: Span,
    pub checked_effect: ResolvedType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Assertion(Box<Assertion>),
    Closure(Box<ClosureExpression>),
    CallableCall(Box<CallableCall>),
    ListAlgorithmCall(Box<ListAlgorithmCall>),
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
        null_safe: bool,
        span: Span,
    },
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Argument>,
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
        class_name: String,
        method: String,
        args: Vec<Argument>,
        span: Span,
    },
    StaticMember {
        class_name: String,
        member: String,
        span: Span,
    },
    New {
        class_type: TypeRef,
        args: Vec<Argument>,
        /// See `ast::Expr::New::shared` — preserved through lowering so the
        /// checker never infers shared construction from context.
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

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Assertion(assertion) => assertion.span,
            Expr::Closure(closure) => closure.span,
            Expr::CallableCall(call) => call.span,
            Expr::ListAlgorithmCall(call) => call.span,
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
