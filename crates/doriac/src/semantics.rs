use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::attributes::{
    AttributeApplication, AttributeAuthoredArgument, AttributeBoundArgument,
    AttributeClassIdentity, AttributeClassSchema, AttributeSchemaParameter, AttributeSemanticInfo,
    AttributeTarget,
};
use crate::builtins::{is_reserved_intrinsic_name, php_function_suggestion, Builtin};
use crate::class_layout::{ClassId, PropertyId};
use crate::collection_diagnostics::{
    self, ArgumentShape, CollectionMemberKind, CollectionReceiver, ImplementationStatus,
};
pub use crate::control_flow::GivenSemanticInfo;
use crate::diagnostics::{
    Diagnostic, DiagnosticResult, DiagnosticSource, FixApplicability, FixEdit,
};
use crate::enums::{
    EnumBackingType, EnumBackingValue, EnumCapabilities, EnumCaseId, EnumId, EnumLayout, EnumType,
    EnumValue, LayoutShape,
};
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::numeric::{parse_decimal_magnitude, FloatType, FloatValue, IntegerType, IntegerValue};
use crate::source::Span;
use crate::symbols::{
    Binding, BindingDeclaration, BindingId, BindingKind, BindingOwnership, BindingResolution,
    BorrowSource, BuiltinInterface, ClassInfo, ClosureId, ConstantInfo, FunctionInfo, LexicalOwner,
    MemberDeclaration, MemberKind, MethodInfo, ParamInfo, PropertyInfo, PropertyInitState,
    ReceiverMode, ReturnBorrow, ScopeStack, StaticPropertyInfo, TypeParamInfo,
};
use crate::types::{
    resolved_type_complexity, ClassType, FunctionBorrowSource, FunctionInvocationMode,
    FunctionReturnBorrow, FunctionTypeParameterMode, FunctionTypeRef, ResolvedType,
    SemanticFunctionParameter, SemanticFunctionType, SharedHandleKind, TypeId, TypeKind, TypeRef,
    TypeRegistry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DictionaryProjection {
    Keys,
    Values,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedBoxPlan {
    pub source_type: ResolvedType,
    pub nullable_target: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticInfo {
    /// Compiler inputs that own this semantic analysis. Namespace and package
    /// identity deliberately remain independent.
    pub compilation_context: crate::names::CompilationContext,
    /// Source-local contexts for graph compilation. The standalone context
    /// above remains the selected-entry compatibility view.
    pub compilation_contexts: HashMap<crate::source::SourceId, crate::names::CompilationContext>,
    /// Canonical global declaration/reference facts produced before checking.
    pub global_symbols: crate::names::GlobalSymbolFacts,
    /// Fully resolved, type-checked, const-evaluated compiler metadata.
    /// Runtime lowering deliberately ignores this table.
    pub attributes: AttributeSemanticInfo,
    /// Canonical integer type for every integer-valued source expression.
    ///
    /// Spans are stable across AST-to-HIR structural lowering, so the MIR
    /// lowering pass can consume semantic decisions without re-parsing type
    /// names or guessing contextual literal types.
    pub integer_expression_types: HashMap<Span, IntegerType>,
    /// Canonical width for every floating-point-valued source expression.
    pub float_expression_types: HashMap<Span, FloatType>,
    /// Resolved semantic type for checked expressions, independent of backend layout.
    pub expression_types: HashMap<Span, ResolvedType>,
    /// Concrete transport plan at each value-to-`mixed` boundary.
    ///
    /// Compatibility backends use this semantic fact to preserve Doria's exact
    /// runtime identity instead of reconstructing it from a host value after
    /// width, signedness, or nominal identity has been erased.
    pub mixed_box_plans: HashMap<Span, MixedBoxPlan>,
    /// Resolved enum case for each unit/backed case expression.
    pub enum_case_values: HashMap<Span, EnumValue>,
    /// Resolved payload-enum construction for each checked case call.
    pub enum_case_constructions: HashMap<Span, EnumCaseId>,
    /// Resolved concrete target for each checked `is` expression.
    pub type_test_types: HashMap<Span, ResolvedType>,
    /// Fully checked match plans consumed by backend-independent MIR lowering.
    pub matches: HashMap<Span, MatchSemanticInfo>,
    /// Fully checked value type for each `when` expression.
    pub whens: HashMap<Span, WhenSemanticInfo>,
    /// Source-order statement classification for each `given` prelude.
    pub given_preludes: HashMap<Span, GivenSemanticInfo>,
    /// Compiler-resolved callable target for each user-defined call expression.
    pub call_targets: HashMap<Span, CallableTarget>,
    /// Concrete generic arguments selected for each checked user-defined call.
    ///
    /// The argument enum is intentionally kinded: Stage 24 supplies only type
    /// arguments, while future compile-time value parameters can add another
    /// variant without changing specialization identity.
    pub generic_call_specializations: HashMap<Span, GenericSpecialization>,
    /// Calls to compiler-known `Displayable::toString` through a constrained
    /// type parameter. MIR specializes these directly for each concrete type.
    pub(crate) constrained_display_calls: HashSet<Span>,
    /// Stable nominal class identities and the total Stage 19 property order.
    pub classes: Vec<ClassSemanticInfo>,
    /// Stable nominal enum identities and declaration-order case metadata.
    pub enums: Vec<EnumSemanticInfo>,
    /// Values produced by the bounded Stage 20 constant evaluator.
    pub const_evaluation: crate::const_eval::Evaluation,
    /// Const-folded Copy-scalar defaults keyed by callable and parameter identity.
    pub parameter_defaults:
        HashMap<crate::const_eval::ParameterDefaultKey, crate::const_eval::ConstValue>,
    /// Elided class-result borrows keyed by callable source span.
    pub return_borrows: HashMap<Span, ReturnBorrow>,
    /// Flow facts at checked source uses, consumed by MIR lowering so
    /// statically selected nullable paths stay selected after lowering.
    pub(crate) flow_facts: crate::narrowing::FactsByUse,
    /// Effective, source-ordered checked effects for each callable declaration.
    ///
    /// Ordinary callables receive this set from their written `throws` clause.
    /// The selected clause-free program entrypoint receives it from the checked
    /// effects that escape its body. Source syntax remains in the AST/HIR
    /// `throws` field and is never synthesized for inferred effects.
    pub callable_effective_checked_effects: HashMap<Span, Vec<ResolvedType>>,
    /// Resolved callable signatures keyed by declaration span for tooling
    /// metadata. Runtime lowering uses the existing HIR callable fields.
    pub callable_signatures: HashMap<Span, CallableSignatureSemanticInfo>,
    /// Exact checked effects produced at each source operation.
    pub(crate) checked_effect_sites: crate::checked_effects::EffectSiteMap,
    /// Resolved owned Error type transferred by each `throw` statement.
    pub throw_error_types: HashMap<Span, ResolvedType>,
    /// Protected effects left after catch coverage for each `try` statement.
    pub try_uncovered_effects: HashMap<Span, Vec<ResolvedType>>,
    /// Resolved catch type for each catch clause.
    pub catch_error_types: HashMap<Span, ResolvedType>,
    /// Canonical structural function types resolved from authored type syntax.
    pub function_types_by_span: HashMap<Span, FunctionTypeSemanticInfo>,
    /// Stable lexical binding declarations and source-use resolutions.
    pub binding_resolution: BindingResolution,
    /// Fully checked closure plans keyed by stable source-derived identity.
    pub closures: HashMap<ClosureId, ClosureSemanticInfo>,
    /// Semantically checked indirect-call plans, still blocked from execution.
    pub callable_value_calls: HashMap<Span, CallableValueCallInfo>,
    /// Fully specialized compiler-known `List<T>` algorithm calls.
    ///
    /// HIR and MIR consume this plan directly. Backends must not recover the
    /// contract from a method-name string.
    pub list_algorithm_calls: HashMap<Span, ListAlgorithmCallInfo>,
    /// Backend-independent capture acquisition, provenance, escape, and
    /// invocation-consumption plans proven by Stage 30c.
    pub closure_ownership: HashMap<ClosureId, crate::ownership::ClosureOwnershipInfo>,
    /// Backend-independent classification of each checked instance-property write.
    pub property_writes: HashMap<Span, PropertyWriteSemanticInfo>,
    /// Object-path expressions proven to carry ordinary writable access.
    pub(crate) writable_object_paths: HashSet<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallableSignatureSemanticInfo {
    pub generic_parameter_count: usize,
    pub parameters: Vec<CallableParameterSemanticInfo>,
    pub return_type: ResolvedType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallableParameterSemanticInfo {
    pub name: String,
    pub r#type: ResolvedType,
    pub take: bool,
    pub writable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyWriteKind {
    Initialize,
    Replace,
    InitializeOrReplace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyWriteSemanticInfo {
    pub kind: PropertyWriteKind,
    pub class_name: String,
    pub property_name: String,
    pub constructor_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTypeSemanticInfo {
    pub ty: ResolvedType,
    pub authored_checked_effects: Vec<ResolvedType>,
    pub ambient_checked_effects: Vec<ResolvedType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CaptureRequirement {
    Readonly,
    Writable,
    Take,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSemanticInfo {
    pub source_binding_id: BindingId,
    pub environment_binding_id: BindingId,
    pub mode: ClosureCaptureMode,
    pub declaration_span: Span,
    pub first_use_span: Option<Span>,
    pub use_spans: Vec<Span>,
    pub source_type: ResolvedType,
    pub required_capability: CaptureRequirement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureSemanticInfo {
    pub closure_id: ClosureId,
    /// The closure's inferred source type before capability/effect substitution.
    pub function_type: ResolvedType,
    /// The compatible contextual function type selected at this creation site.
    ///
    /// Native lowering uses this contract so accepted invocation-capability and
    /// checked-effect substitutions are reflected in the closure ABI.
    pub execution_function_type: ResolvedType,
    pub captures: Vec<CaptureSemanticInfo>,
    pub inferred_invocation_mode: FunctionInvocationMode,
    pub inferred_checked_effects: Vec<ResolvedType>,
    pub required_checked_effects: Vec<ResolvedType>,
    pub ambient_checked_effects: Vec<ResolvedType>,
    pub inferred_return_type: ResolvedType,
    pub execution_boundary_span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallableValueTargetKind {
    Value,
    Property,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallableValueCallInfo {
    pub function_type: ResolvedType,
    pub invocation_mode: FunctionInvocationMode,
    pub return_type: ResolvedType,
    pub checked_effects: Vec<ResolvedType>,
    pub required_checked_effects: Vec<ResolvedType>,
    pub ambient_checked_effects: Vec<ResolvedType>,
    pub target_kind: CallableValueTargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListAlgorithmKind {
    Map,
    Filter,
    Reduce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListCallbackAccess {
    Readonly,
    Writable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAlgorithmCallInfo {
    pub kind: ListAlgorithmKind,
    pub receiver_type: ResolvedType,
    pub element_type: ResolvedType,
    pub result_type: ResolvedType,
    pub accumulator_type: Option<ResolvedType>,
    pub callback_type: ResolvedType,
    pub callback_access: ListCallbackAccess,
    pub checked_effects: Vec<ResolvedType>,
    pub required_checked_effects: Vec<ResolvedType>,
    pub ambient_checked_effects: Vec<ResolvedType>,
    pub source_span: Span,
    pub receiver_span: Span,
    pub callback_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericSpecialization {
    pub arguments: Vec<GenericArgument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GenericArgument {
    Type(ResolvedType),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CallableTarget {
    Function {
        name: String,
    },
    Method {
        class_type: ClassType<ResolvedType>,
        method_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticAnalysis {
    pub info: SemanticInfo,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSemanticInfo {
    pub id: ClassId,
    pub declaration_name: String,
    pub name: String,
    pub arguments: Vec<ResolvedType>,
    pub builtin_interfaces: Vec<BuiltinInterface>,
    pub properties: Vec<PropertySemanticInfo>,
}

impl ClassSemanticInfo {
    pub fn implements(&self, interface: BuiltinInterface) -> bool {
        self.builtin_interfaces.contains(&interface)
    }
}

fn contains_comment(text: &str) -> bool {
    text.contains("//") || text.contains("/*") || text.contains('#')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySemanticInfo {
    pub id: PropertyId,
    pub name: String,
    pub ty: ResolvedType,
    pub writable: bool,
    pub promoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumSemanticInfo {
    pub id: EnumId,
    pub name: String,
    pub backing_type: Option<EnumBackingType>,
    pub cases: Vec<EnumCaseSemanticInfo>,
    pub capabilities: EnumCapabilities,
    pub layout: EnumLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumCaseSemanticInfo {
    pub id: EnumCaseId,
    pub name: String,
    pub tag: u32,
    pub backing_value: Option<EnumBackingValue>,
    pub payload: Vec<EnumPayloadSemanticInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumPayloadSemanticInfo {
    pub name: String,
    pub ty: ResolvedType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSemanticInfo {
    pub scrutinee_type: ResolvedType,
    pub result_type: ResolvedType,
    pub origin: MatchOrigin,
    pub mode: MatchMode,
    pub condition_mode: bool,
    pub arms: Vec<MatchArmSemanticInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenSemanticInfo {
    pub result_type: ResolvedType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArmSemanticInfo {
    pub pattern: ResolvedMatchPattern,
    pub guard: MatchGuardSemanticInfo,
    pub bindings: Vec<MatchBindingSemanticInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchGuardSemanticInfo {
    None,
    Runtime,
    AlwaysTrue,
    AlwaysFalse,
}

impl MatchGuardSemanticInfo {
    const fn covers_pattern(self) -> bool {
        matches!(self, Self::None | Self::AlwaysTrue)
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Block(block) => block.span,
        Stmt::VarDecl(declaration) => declaration.span,
        Stmt::Assignment(assignment) => assignment.span,
        Stmt::Echo { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Throw(ThrowStmt { span, .. })
        | Stmt::Try(TryStmt { span, .. })
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Expr { span, .. } => *span,
        Stmt::If(statement) => statement.span,
        Stmt::While(statement) => statement.span,
        Stmt::DoWhile(statement) => statement.span,
        Stmt::For(statement) => statement.span,
        Stmt::Foreach(statement) => statement.span,
        Stmt::Increment(statement) => statement.span,
    }
}

fn closure_local_declarations(body: &ClosureBody) -> HashMap<String, Span> {
    let mut declarations = HashMap::new();
    if let ClosureBody::Block(block) = body {
        for statement in &block.statements {
            if let Stmt::VarDecl(declaration) = statement {
                for binding in &declaration.bindings {
                    declarations
                        .entry(binding.name.clone())
                        .or_insert(binding.span);
                }
            }
        }
    }
    declarations
}

fn collect_return_expression_spans(block: &Block, returns: &mut Vec<Option<Span>>) {
    for statement in &block.statements {
        match statement {
            Stmt::Return { expr, .. } => returns.push(expr.as_ref().map(Expr::span)),
            Stmt::Block(block) => collect_return_expression_spans(block, returns),
            Stmt::If(statement) => {
                collect_return_expression_spans(&statement.then_block, returns);
                if let Some(branch) = &statement.else_branch {
                    collect_else_return_spans(branch, returns);
                }
                if let Some(finally) = &statement.finally {
                    collect_return_expression_spans(&finally.block, returns);
                }
            }
            Stmt::While(statement) => {
                collect_return_expression_spans(&statement.body, returns);
                if let Some(finally) = &statement.finally {
                    collect_return_expression_spans(&finally.block, returns);
                }
            }
            Stmt::DoWhile(statement) => {
                collect_return_expression_spans(&statement.body, returns);
                if let Some(finally) = &statement.finally {
                    collect_return_expression_spans(&finally.block, returns);
                }
            }
            Stmt::For(statement) => collect_return_expression_spans(&statement.body, returns),
            Stmt::Foreach(statement) => collect_return_expression_spans(&statement.body, returns),
            Stmt::Try(statement) => {
                collect_return_expression_spans(&statement.body, returns);
                for catch in &statement.catches {
                    collect_return_expression_spans(&catch.body, returns);
                }
                if let Some(finally) = &statement.finally {
                    collect_return_expression_spans(&finally.body, returns);
                }
            }
            Stmt::VarDecl(_)
            | Stmt::Assignment(_)
            | Stmt::Echo { .. }
            | Stmt::Throw(_)
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Increment(_)
            | Stmt::Expr { .. } => {}
        }
    }
}

fn collect_else_return_spans(branch: &ElseBranch, returns: &mut Vec<Option<Span>>) {
    match branch {
        ElseBranch::Block(block) => collect_return_expression_spans(block, returns),
        ElseBranch::If(statement) => {
            collect_return_expression_spans(&statement.then_block, returns);
            if let Some(branch) = &statement.else_branch {
                collect_else_return_spans(branch, returns);
            }
            if let Some(finally) = &statement.finally {
                collect_return_expression_spans(&finally.block, returns);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchBindingSemanticInfo {
    pub name: String,
    pub ty: ResolvedType,
    pub borrowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedMatchPattern {
    Default,
    Constant(crate::const_eval::ConstValue),
    Null,
    EnumCase {
        enum_id: EnumId,
        case_id: EnumCaseId,
    },
    ExactType(ResolvedType),
    Condition,
}

impl SemanticInfo {
    pub fn integer_type(&self, span: Span) -> Option<IntegerType> {
        self.integer_expression_types.get(&span).copied()
    }

    pub fn float_type(&self, span: Span) -> Option<FloatType> {
        self.float_expression_types.get(&span).copied()
    }

    pub fn expression_type(&self, span: Span) -> Option<&ResolvedType> {
        self.expression_types.get(&span)
    }

    pub fn type_test_type(&self, span: Span) -> Option<&ResolvedType> {
        self.type_test_types.get(&span)
    }

    pub fn call_target(&self, span: Span) -> Option<&CallableTarget> {
        self.call_targets.get(&span)
    }
}

pub fn analyze_program(program: &Program) -> DiagnosticResult<SemanticInfo> {
    let analysis = analyze_program_for_ide(program);
    if analysis.diagnostics.is_empty() {
        Ok(analysis.info)
    } else {
        Err(analysis.diagnostics)
    }
}

pub fn analyze_program_for_ide(program: &Program) -> SemanticAnalysis {
    analyze_program_for_ide_with_sources(program, &HashMap::new())
}

pub fn analyze_program_with_source<'source>(
    program: &'source Program,
    source_text: &'source str,
) -> DiagnosticResult<SemanticInfo> {
    let analysis = analyze_program_for_ide_with_source(program, Some(source_text));
    if analysis.diagnostics.is_empty() {
        Ok(analysis.info)
    } else {
        Err(analysis.diagnostics)
    }
}

pub fn analyze_program_for_ide_with_source<'source>(
    program: &'source Program,
    source_text: Option<&'source str>,
) -> SemanticAnalysis {
    let mut sources = HashMap::new();
    if let Some(source_text) = source_text {
        sources.insert(crate::source::SourceId::default(), source_text);
    }
    analyze_program_for_ide_with_sources(program, &sources)
}

pub fn analyze_program_with_sources<'source>(
    program: &'source Program,
    source_texts: &HashMap<crate::source::SourceId, &'source str>,
) -> DiagnosticResult<SemanticInfo> {
    let analysis = analyze_program_for_ide_with_sources(program, source_texts);
    if analysis.diagnostics.is_empty() {
        Ok(analysis.info)
    } else {
        Err(analysis.diagnostics)
    }
}

pub fn analyze_program_for_ide_with_sources<'source>(
    program: &'source Program,
    source_texts: &HashMap<crate::source::SourceId, &'source str>,
) -> SemanticAnalysis {
    analyze_program_for_ide_with_graph_context(
        program,
        source_texts,
        crate::names::CompilationContext::default(),
        HashMap::new(),
        crate::names::GlobalSymbolFacts::default(),
    )
}

pub fn analyze_program_for_ide_with_graph_context<'source>(
    program: &'source Program,
    source_texts: &HashMap<crate::source::SourceId, &'source str>,
    compilation_context: crate::names::CompilationContext,
    compilation_contexts: HashMap<crate::source::SourceId, crate::names::CompilationContext>,
    global_symbols: crate::names::GlobalSymbolFacts,
) -> SemanticAnalysis {
    let (const_evaluation, const_diagnostics) = match crate::const_eval::evaluate_program(program) {
        Ok(evaluation) => (evaluation, Vec::new()),
        Err(diagnostics) => (crate::const_eval::Evaluation::default(), diagnostics),
    };
    // Discover ambient I/O through the complete direct-call graph before the
    // diagnostic pass. This preserves forward-call precision without making
    // source order part of a callable's runtime Error transport.
    let mut discovery = Checker::new(
        program,
        const_evaluation.clone(),
        source_texts,
        compilation_context.clone(),
        compilation_contexts.clone(),
        global_symbols.clone(),
    );
    discovery.check();
    let mut ambient_effect_seed = discovery.inferred_ambient_effects();

    // The first pass deliberately over-approximates transitive ambient effects
    // so forward and recursive calls have checked transport available. Narrow
    // that seed to the effects which actually escape each callable after local
    // catch coverage. Repeating to a fixpoint removes stale effects through
    // arbitrarily deep call chains without making source order observable.
    loop {
        let mut refinement = Checker::new(
            program,
            const_evaluation.clone(),
            source_texts,
            compilation_context.clone(),
            compilation_contexts.clone(),
            global_symbols.clone(),
        );
        refinement.ambient_effect_seed = ambient_effect_seed.clone();
        refinement.check();
        let refined = refinement.escaping_ambient_effects();
        if ambient_effect_maps_equal(&ambient_effect_seed, &refined) {
            ambient_effect_seed = refined;
            break;
        }
        ambient_effect_seed = refined;
    }

    let mut checker = Checker::new(
        program,
        const_evaluation,
        source_texts,
        compilation_context,
        compilation_contexts,
        global_symbols,
    );
    checker.ambient_effect_seed = ambient_effect_seed;
    checker.diagnostics.extend(const_diagnostics);
    checker.check();
    checker.check_attributes();
    let constructor_analysis = crate::constructor_init::check_program(
        program,
        &checker.given_preludes,
        &checker.checked_effect_sites,
        &checker.catch_error_types,
    );
    for (span, kind) in constructor_analysis.property_writes {
        if let Some(write) = checker.property_writes.get_mut(&span) {
            write.kind = kind;
        }
    }
    checker.diagnostics.extend(constructor_analysis.diagnostics);
    let inferred_move_returns = checker
        .function_signatures
        .iter()
        .filter_map(|(span_start, signature)| {
            checker
                .type_is_move_type(signature.return_ty)
                .then_some(*span_start)
        })
        .collect();
    let return_borrows = checker
        .function_signatures
        .iter()
        .filter_map(|(span, signature)| signature.return_borrow.map(|borrow| (*span, borrow)))
        .collect();
    let move_enum_names = checker
        .enums
        .values()
        .filter(|definition| !definition.capabilities.copy)
        .map(|definition| definition.name.clone())
        .collect();
    let ownership_analysis = crate::ownership::check_program_with_inferred_move_returns(
        program,
        &crate::ownership::OwnershipAnalysisContext {
            inferred_move_returns: &inferred_move_returns,
            return_borrows: &return_borrows,
            resolved_types: &checker.expression_types,
            flow_facts: &checker.flow_facts,
            move_enum_names: &move_enum_names,
            given_preludes: &checker.given_preludes,
            checked_effect_sites: &checker.checked_effect_sites,
            catch_error_types: &checker.catch_error_types,
            binding_resolution: &checker.binding_resolution,
            closures: &checker.closures,
            callable_value_calls: &checker.callable_value_calls,
            list_algorithm_calls: &checker.list_algorithm_calls,
        },
    );
    let closure_ownership = ownership_analysis.closures;
    let return_borrows = ownership_analysis.return_borrows;
    checker.diagnostics.extend(ownership_analysis.diagnostics);
    let classes = collect_ordered_class_semantics(program, &mut checker);
    let enums = collect_ordered_enum_semantics(&checker);
    let callable_signatures = checker
        .function_signatures
        .iter()
        .map(|(span, signature)| {
            (
                *span,
                CallableSignatureSemanticInfo {
                    generic_parameter_count: signature.type_params.len(),
                    parameters: signature
                        .params
                        .iter()
                        .map(|parameter| CallableParameterSemanticInfo {
                            name: parameter.name.clone(),
                            r#type: checker.types.resolved(parameter.ty),
                            take: parameter.take,
                            writable: parameter.writable,
                        })
                        .collect(),
                    return_type: checker.types.resolved(signature.return_ty),
                },
            )
        })
        .collect();
    SemanticAnalysis {
        info: SemanticInfo {
            compilation_context: checker.compilation_context,
            compilation_contexts: checker.compilation_contexts,
            global_symbols: checker.global_symbols,
            attributes: checker.attributes,
            integer_expression_types: checker.integer_expression_types,
            float_expression_types: checker.float_expression_types,
            expression_types: checker.expression_types,
            mixed_box_plans: checker.mixed_box_plans,
            enum_case_values: checker.enum_case_values,
            enum_case_constructions: checker.enum_case_constructions,
            type_test_types: checker.type_test_types,
            matches: checker.matches,
            whens: checker.whens,
            given_preludes: checker.given_preludes,
            call_targets: checker.call_targets,
            generic_call_specializations: checker.generic_call_specializations,
            constrained_display_calls: checker.constrained_display_calls,
            classes,
            enums,
            const_evaluation: checker.const_evaluation,
            parameter_defaults: checker.parameter_defaults,
            return_borrows,
            flow_facts: checker.flow_facts,
            callable_effective_checked_effects: checker.callable_effective_checked_effects,
            callable_signatures,
            checked_effect_sites: checker.checked_effect_sites,
            throw_error_types: checker.throw_error_types,
            try_uncovered_effects: checker.try_uncovered_effects,
            catch_error_types: checker.catch_error_types,
            function_types_by_span: checker.function_types_by_span,
            binding_resolution: checker.binding_resolution,
            closures: checker.closures,
            callable_value_calls: checker.callable_value_calls,
            list_algorithm_calls: checker.list_algorithm_calls,
            closure_ownership,
            property_writes: checker.property_writes,
            writable_object_paths: checker.writable_object_paths,
        },
        diagnostics: checker.diagnostics,
    }
}

fn ambient_effect_maps_equal(
    left: &HashMap<Span, Vec<ResolvedType>>,
    right: &HashMap<Span, Vec<ResolvedType>>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(callable, effects)| {
            right.get(callable).is_some_and(|other| {
                effects.len() == other.len() && effects.iter().all(|effect| other.contains(effect))
            })
        })
}

fn collect_ordered_enum_semantics(checker: &Checker<'_>) -> Vec<EnumSemanticInfo> {
    let mut enums = checker.enums.values().cloned().collect::<Vec<_>>();
    enums.sort_by_key(|definition| definition.id);
    enums
        .into_iter()
        .map(|definition| {
            let case_count = definition.cases.len();
            EnumSemanticInfo {
                id: definition.id,
                name: definition.name,
                backing_type: definition.backing_type,
                cases: definition
                    .cases
                    .into_iter()
                    .map(|case| EnumCaseSemanticInfo {
                        id: case.id,
                        name: case.name,
                        tag: case.tag,
                        backing_value: case.backing_value,
                        payload: case
                            .payload
                            .into_iter()
                            .map(|field| EnumPayloadSemanticInfo {
                                name: field.name,
                                ty: checker.types.resolved(field.ty),
                            })
                            .collect(),
                    })
                    .collect(),
                capabilities: definition.capabilities,
                layout: definition.layout.unwrap_or_else(|| {
                    let empty_cases = vec![Vec::new(); case_count.max(1)];
                    crate::enums::compute_enum_layout(definition.id, &empty_cases)
                        .expect("an empty recovery enum layout is finite")
                }),
            }
        })
        .collect()
}

fn collect_ordered_class_semantics(
    program: &Program,
    checker: &mut Checker<'_>,
) -> Vec<ClassSemanticInfo> {
    let mut expanded_classes = HashSet::new();
    loop {
        let previous_count = checker.class_instantiations.len();
        collect_callable_class_instantiations(program, checker);
        expand_class_instantiations(program, checker, &mut expanded_classes);
        if checker.class_instantiations.len() == previous_count {
            break;
        }
    }

    let mut instances = checker
        .class_instantiations
        .iter()
        .filter(|class| {
            !class
                .arguments
                .iter()
                .any(|argument| checker.type_is_symbolic(*argument))
        })
        .cloned()
        .collect::<Vec<_>>();
    instances.sort_by_key(|class| {
        (
            class.name.clone(),
            class
                .arguments
                .iter()
                .map(|argument| checker.types.display(*argument))
                .collect::<Vec<_>>(),
        )
    });
    let concrete_instances = instances.into_iter().collect::<HashSet<_>>();

    let declarations = program.items.iter().filter_map(|item| match item {
        Item::Class(class) => Some(class),
        _ => None,
    });
    let mut classes = Vec::new();
    for declaration in declarations {
        let Some(class_info) = checker.classes.get(&declaration.name).cloned() else {
            continue;
        };
        let mut instances = if declaration.type_params.is_empty() {
            vec![ClassType::new(declaration.name.clone(), Vec::new())]
        } else {
            concrete_instances
                .iter()
                .filter(|class| class.name == declaration.name)
                .cloned()
                .collect::<Vec<_>>()
        };
        instances.sort_by_key(|class| {
            class
                .arguments
                .iter()
                .map(|argument| checker.types.display(*argument))
                .collect::<Vec<_>>()
                .join("\u{0}")
        });

        for instance in instances {
            let id = ClassId(classes.len());
            let substitutions = checker.class_type_substitutions(&instance);
            let explicit = declaration
                .members
                .iter()
                .filter_map(|member| match member {
                    ClassMember::Property(property) if !property.is_static => {
                        Some((property.name.clone(), property.writable, false))
                    }
                    ClassMember::Property(_)
                    | ClassMember::Method(_)
                    | ClassMember::Constant(_) => None,
                });
            let promoted = declaration.members.iter().find_map(|member| match member {
                ClassMember::Method(method) if method.name == "__construct" => {
                    Some(method.params.iter().filter_map(|param| {
                        param
                            .promoted_access
                            .as_ref()
                            .map(|_| (param.name.clone(), param.writable, true))
                    }))
                }
                _ => None,
            });
            let mut properties = explicit.collect::<Vec<_>>();
            if let Some(promoted) = promoted {
                properties.extend(promoted);
            }
            let class_type_id = checker.types.intern(TypeKind::Class(instance.clone()));
            let mut builtin_interfaces = class_info
                .builtin_interfaces
                .iter()
                .copied()
                .collect::<Vec<_>>();
            builtin_interfaces.sort();
            classes.push(ClassSemanticInfo {
                id,
                declaration_name: declaration.name.clone(),
                name: checker.types.display(class_type_id),
                arguments: instance
                    .arguments
                    .iter()
                    .map(|argument| checker.types.resolved(*argument))
                    .collect(),
                builtin_interfaces,
                properties: properties
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, (name, writable, promoted))| {
                        let property = class_info.properties.get(&name)?;
                        let ty = checker.substitute_type_id(property.ty, &substitutions);
                        Some(PropertySemanticInfo {
                            id: PropertyId { class: id, index },
                            name,
                            ty: checker.types.resolved(ty),
                            writable,
                            promoted,
                        })
                    })
                    .collect(),
            });
        }
    }
    classes
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SemanticCallableInstance {
    declaration: Span,
    bindings: Vec<(String, TypeId)>,
}

impl SemanticCallableInstance {
    fn new(declaration: Span, bindings: HashMap<String, TypeId>) -> Self {
        let mut bindings = bindings.into_iter().collect::<Vec<_>>();
        bindings.sort_by(|left, right| left.0.cmp(&right.0));
        Self {
            declaration,
            bindings,
        }
    }

    fn substitutions(&self) -> HashMap<String, TypeId> {
        self.bindings.iter().cloned().collect()
    }
}

fn enqueue_callable_instance(
    instance: SemanticCallableInstance,
    parent: Option<usize>,
    instances: &mut Vec<SemanticCallableInstance>,
    parents: &mut Vec<Option<usize>>,
    ids: &mut HashMap<SemanticCallableInstance, usize>,
) {
    if ids.contains_key(&instance) {
        return;
    }
    let index = instances.len();
    ids.insert(instance.clone(), index);
    instances.push(instance);
    parents.push(parent);
}

fn collect_callable_class_instantiations(program: &Program, checker: &mut Checker<'_>) {
    let mut declarations = HashMap::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                declarations.insert(function.span, (function, None));
            }
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(method) = member {
                        declarations.insert(method.span, (method, Some(class)));
                    }
                }
            }
            Item::Enum(_)
            | Item::Interface(_)
            | Item::Trait(_)
            | Item::Constant(_)
            | Item::Statement(_) => {}
        }
    }

    let mut instances = Vec::new();
    let mut parents = Vec::new();
    let mut ids = HashMap::new();

    for (declaration, (function, class)) in &declarations {
        if !function.type_params.is_empty() {
            continue;
        }
        match class {
            None => {
                enqueue_callable_instance(
                    SemanticCallableInstance::new(*declaration, HashMap::new()),
                    None,
                    &mut instances,
                    &mut parents,
                    &mut ids,
                );
            }
            Some(class) if class.type_params.is_empty() => {
                enqueue_callable_instance(
                    SemanticCallableInstance::new(*declaration, HashMap::new()),
                    None,
                    &mut instances,
                    &mut parents,
                    &mut ids,
                );
            }
            Some(class) => {
                let concrete_classes = checker
                    .class_instantiations
                    .iter()
                    .filter(|instance| {
                        instance.name == class.name
                            && !instance
                                .arguments
                                .iter()
                                .any(|argument| checker.type_is_symbolic(*argument))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                for instance in concrete_classes {
                    enqueue_callable_instance(
                        SemanticCallableInstance::new(
                            *declaration,
                            checker.class_type_substitutions(&instance),
                        ),
                        None,
                        &mut instances,
                        &mut parents,
                        &mut ids,
                    );
                }
            }
        }
    }

    let mut calls = checker
        .pending_generic_calls
        .iter()
        .filter(|(span, _)| checker.generic_call_specializations.contains_key(span))
        .map(|(span, pending)| (*span, pending.clone()))
        .collect::<Vec<_>>();
    calls.sort_by_key(|(span, _)| *span);
    for (_, pending) in &calls {
        if pending
            .bindings
            .values()
            .any(|argument| checker.type_is_symbolic(*argument))
        {
            continue;
        }
        enqueue_callable_instance(
            SemanticCallableInstance::new(pending.declaration, pending.bindings.clone()),
            None,
            &mut instances,
            &mut parents,
            &mut ids,
        );
    }

    let mut cursor = 0;
    while cursor < instances.len() {
        let instance_index = cursor;
        let instance = instances[cursor].clone();
        cursor += 1;
        let Some((function, _)) = declarations.get(&instance.declaration) else {
            continue;
        };
        let substitutions = instance.substitutions();

        if let Some(templates) = checker
            .callable_class_instantiation_templates
            .get(&instance.declaration)
            .cloned()
        {
            for template in templates {
                let arguments = template
                    .arguments
                    .iter()
                    .map(|argument| checker.substitute_type_id(*argument, &substitutions))
                    .collect::<Vec<_>>();
                if arguments
                    .iter()
                    .any(|argument| checker.type_is_symbolic(*argument))
                {
                    continue;
                }
                let class = ClassType::new(template.name, arguments);
                checker.check_concrete_class_constraints(&class, function.span);
                checker.class_instantiations.insert(class);
            }
        }

        for (span, pending) in &calls {
            if span.source != function.span.source
                || span.start < function.span.start
                || span.end > function.span.end
            {
                continue;
            }
            let bindings = pending
                .bindings
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        checker.substitute_type_id(*ty, &substitutions),
                    )
                })
                .collect::<HashMap<_, _>>();
            if bindings
                .values()
                .any(|argument| checker.type_is_symbolic(*argument))
            {
                continue;
            }
            let target = SemanticCallableInstance::new(pending.declaration, bindings);
            if ids.contains_key(&target) {
                continue;
            }
            if callable_specialization_expands_recursively(
                &instances,
                &parents,
                instance_index,
                &target,
                &checker.types,
            ) {
                let name = declarations
                    .get(&target.declaration)
                    .map(|(function, _)| function.name.as_str())
                    .unwrap_or(pending.callee.as_str());
                let message = format!(
                    "generic specialization of `{name}` recursively expands its type arguments and has no finite monomorphization"
                );
                if !checker
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0539" && diagnostic.message == message)
                {
                    checker.diagnostics.push(
                        Diagnostic::new("E0539", message, *span).with_help(
                            "keep recursive generic calls at the same concrete type, or move the type-changing step outside the recursion",
                        ),
                    );
                }
                continue;
            }
            enqueue_callable_instance(
                target,
                Some(instance_index),
                &mut instances,
                &mut parents,
                &mut ids,
            );
        }
    }
}

fn callable_specialization_expands_recursively(
    instances: &[SemanticCallableInstance],
    parents: &[Option<usize>],
    current: usize,
    target: &SemanticCallableInstance,
    types: &TypeRegistry,
) -> bool {
    let mut matching_ancestors = Vec::new();
    let mut cursor = Some(current);
    while let Some(index) = cursor {
        if instances[index].declaration == target.declaration {
            matching_ancestors.push(&instances[index]);
            if matching_ancestors.len() == 2 {
                break;
            }
        }
        cursor = parents[index];
    }
    let [nearest, previous] = matching_ancestors.as_slice() else {
        return false;
    };
    callable_specialization_complexity(target, types)
        > callable_specialization_complexity(nearest, types)
        && callable_specialization_complexity(nearest, types)
            > callable_specialization_complexity(previous, types)
}

fn callable_specialization_complexity(
    instance: &SemanticCallableInstance,
    types: &TypeRegistry,
) -> usize {
    instance
        .bindings
        .iter()
        .map(|(_, ty)| resolved_type_complexity(&types.resolved(*ty)))
        .sum()
}

fn expand_class_instantiations(
    program: &Program,
    checker: &mut Checker<'_>,
    expanded: &mut HashSet<ClassType<TypeId>>,
) {
    let mut instances = checker
        .class_instantiations
        .iter()
        .filter(|class| {
            !expanded.contains(*class)
                && !class
                    .arguments
                    .iter()
                    .any(|argument| checker.type_is_symbolic(*argument))
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut parents = vec![None; instances.len()];
    let mut ids = checker
        .class_instantiations
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut cursor = 0;
    while cursor < instances.len() {
        let instance_index = cursor;
        let instance = instances[cursor].clone();
        cursor += 1;
        expanded.insert(instance.clone());
        let instance_span = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Class(class) if class.name == instance.name => Some(class.span),
                _ => None,
            })
            .unwrap_or_default();
        checker.check_specialized_class_shared_payloads(&instance, instance_span);
        let Some(templates) = checker
            .class_instantiation_templates
            .get(&instance.name)
            .cloned()
        else {
            continue;
        };
        let substitutions = checker.class_type_substitutions(&instance);
        for template in templates {
            let arguments = template
                .arguments
                .iter()
                .map(|argument| checker.substitute_type_id(*argument, &substitutions))
                .collect::<Vec<_>>();
            if arguments
                .iter()
                .any(|argument| checker.type_is_symbolic(*argument))
            {
                continue;
            }
            let specialized = ClassType::new(template.name, arguments);
            if ids.contains(&specialized) {
                continue;
            }
            let span = program
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Class(class) if class.name == specialized.name => Some(class.span),
                    _ => None,
                })
                .unwrap_or_default();
            if class_specialization_expands_recursively(
                &instances,
                &parents,
                instance_index,
                &specialized,
                &checker.types,
            ) {
                let message = format!(
                    "generic specialization of class `{}` recursively expands its type arguments and has no finite monomorphization",
                    specialized.name
                );
                if !checker
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0539" && diagnostic.message == message)
                {
                    checker.diagnostics.push(
                        Diagnostic::new("E0539", message, span).with_help(
                            "keep recursive class references at the same concrete type, or move the type-changing step outside the recursive class",
                        ),
                    );
                }
                continue;
            }
            checker.check_concrete_class_constraints(&specialized, span);
            ids.insert(specialized.clone());
            checker.class_instantiations.insert(specialized.clone());
            instances.push(specialized);
            parents.push(Some(instance_index));
        }
    }
}

fn class_specialization_expands_recursively(
    instances: &[ClassType<TypeId>],
    parents: &[Option<usize>],
    current: usize,
    target: &ClassType<TypeId>,
    types: &TypeRegistry,
) -> bool {
    let mut matching_ancestors = Vec::new();
    let mut cursor = Some(current);
    while let Some(index) = cursor {
        if instances[index].name == target.name {
            matching_ancestors.push(&instances[index]);
            if matching_ancestors.len() == 2 {
                break;
            }
        }
        cursor = parents[index];
    }
    let [nearest, previous] = matching_ancestors.as_slice() else {
        return false;
    };
    class_specialization_complexity(target, types) > class_specialization_complexity(nearest, types)
        && class_specialization_complexity(nearest, types)
            > class_specialization_complexity(previous, types)
}

fn class_specialization_complexity(class: &ClassType<TypeId>, types: &TypeRegistry) -> usize {
    class
        .arguments
        .iter()
        .map(|argument| resolved_type_complexity(&types.resolved(*argument)))
        .sum()
}

pub fn check_program(program: &Program) -> DiagnosticResult<()> {
    analyze_program(program).map(|_| ())
}

pub(crate) fn interface_declaration_diagnostic(interface_decl: &InterfaceDecl) -> Diagnostic {
    let (code, message) = if matches!(interface_decl.name.as_str(), "Displayable" | "Error") {
        (
            "E0309",
            format!(
                "`{}` is a compiler-known interface and cannot be redeclared",
                interface_decl.name
            ),
        )
    } else {
        (
            "E0464",
            format!(
                "interface declaration `{}` is accepted syntax but is not available in this compiler version",
                interface_decl.name
            ),
        )
    };
    Diagnostic::new(code, message, interface_decl.span)
}

pub(crate) fn trait_declaration_diagnostic(trait_decl: &TraitDecl) -> Diagnostic {
    Diagnostic::unsupported_stage(
        "E0493",
        format!(
            "trait declaration `{}` is accepted syntax; trait composition semantics land in Stage 35",
            trait_decl.name
        ),
        trait_decl.span,
    )
}

struct Checker<'program> {
    program: &'program Program,
    source_texts: HashMap<crate::source::SourceId, &'program str>,
    compilation_context: crate::names::CompilationContext,
    compilation_contexts: HashMap<crate::source::SourceId, crate::names::CompilationContext>,
    global_symbols: crate::names::GlobalSymbolFacts,
    classes: HashMap<String, ClassInfo>,
    enums: HashMap<String, EnumDefinition>,
    functions: HashMap<String, FunctionInfo>,
    function_signatures: HashMap<Span, FunctionInfo>,
    types: TypeRegistry,
    diagnostics: Vec<Diagnostic>,
    integer_expression_types: HashMap<Span, IntegerType>,
    float_expression_types: HashMap<Span, FloatType>,
    expression_types: HashMap<Span, ResolvedType>,
    mixed_box_plans: HashMap<Span, MixedBoxPlan>,
    enum_case_values: HashMap<Span, EnumValue>,
    enum_case_constructions: HashMap<Span, EnumCaseId>,
    type_test_types: HashMap<Span, ResolvedType>,
    matches: HashMap<Span, MatchSemanticInfo>,
    whens: HashMap<Span, WhenSemanticInfo>,
    given_preludes: HashMap<Span, GivenSemanticInfo>,
    call_targets: HashMap<Span, CallableTarget>,
    generic_call_specializations: HashMap<Span, GenericSpecialization>,
    constrained_display_calls: HashSet<Span>,
    pending_generic_calls: HashMap<Span, PendingGenericCall>,
    type_parameter_scopes: Vec<HashMap<String, Vec<TypeRef>>>,
    class_instantiations: HashSet<ClassType<TypeId>>,
    class_instantiation_templates: HashMap<String, HashSet<ClassType<TypeId>>>,
    callable_class_instantiation_templates: HashMap<Span, HashSet<ClassType<TypeId>>>,
    current_callable: Option<Span>,
    integer_literals: HashMap<Span, u128>,
    negative_integer_literals: HashMap<Span, u128>,
    negated_integer_literal_operands: HashSet<Span>,
    const_evaluation: crate::const_eval::Evaluation,
    parameter_defaults:
        HashMap<crate::const_eval::ParameterDefaultKey, crate::const_eval::ConstValue>,
    flow_facts: crate::narrowing::FactsByUse,
    contextual_expression_types: HashMap<Span, TypeId>,
    when_contexts: Vec<WhenCheckContext>,
    active_loop_depth: usize,
    finalizer_boundaries: Vec<FinalizerBoundary>,
    effect_scopes: Vec<CheckedEffectSet>,
    class_initializer_effects: HashMap<String, CheckedEffectSet>,
    callable_observed_checked_effects: HashMap<Span, Vec<ResolvedType>>,
    callable_declared_checked_effects: HashMap<Span, Vec<ResolvedType>>,
    callable_dependencies: HashMap<Span, Vec<Span>>,
    ambient_effect_seed: HashMap<Span, Vec<ResolvedType>>,
    callable_effective_checked_effects: HashMap<Span, Vec<ResolvedType>>,
    checked_effect_sites: crate::checked_effects::EffectSiteMap,
    throw_error_types: HashMap<Span, ResolvedType>,
    try_uncovered_effects: HashMap<Span, Vec<ResolvedType>>,
    catch_error_types: HashMap<Span, ResolvedType>,
    function_types_by_span: HashMap<Span, FunctionTypeSemanticInfo>,
    binding_resolution: BindingResolution,
    binding_ids: HashMap<(usize, usize, BindingKind, LexicalOwner, String), BindingId>,
    next_binding_id: usize,
    current_lexical_owner: LexicalOwner,
    closures: HashMap<ClosureId, ClosureSemanticInfo>,
    closure_types: HashMap<Span, TypeId>,
    callable_value_calls: HashMap<Span, CallableValueCallInfo>,
    list_algorithm_calls: HashMap<Span, ListAlgorithmCallInfo>,
    property_writes: HashMap<Span, PropertyWriteSemanticInfo>,
    writable_object_paths: HashSet<Span>,
    attributes: AttributeSemanticInfo,
    active_closures: Vec<ActiveClosure>,
    initializing_bindings: Vec<HashMap<String, Span>>,
}

#[derive(Clone)]
struct AttributeSchemaDraft {
    schema: AttributeClassSchema,
    parameters: Vec<AttributeSchemaParameterDraft>,
    declaring_class: Option<String>,
}

#[derive(Clone)]
struct AttributeSchemaParameterDraft {
    declaration: Param,
    ty: TypeId,
    default_value: Option<crate::attributes::AttributeValue>,
}

#[derive(Debug, Clone)]
struct ActiveClosure {
    id: ClosureId,
    captures: Vec<CaptureDraft>,
    capture_by_environment: HashMap<BindingId, usize>,
    missing: HashMap<BindingId, MissingCaptureDraft>,
}

#[derive(Debug, Clone)]
struct CaptureDraft {
    source_binding_id: BindingId,
    environment_binding_id: BindingId,
    mode: ClosureCaptureMode,
    declaration_span: Span,
    first_use_span: Option<Span>,
    use_spans: Vec<Span>,
    source_type: TypeId,
    required_capability: CaptureRequirement,
}

#[derive(Debug, Clone)]
struct MissingCaptureDraft {
    source_binding_id: BindingId,
    first_use_span: Span,
    use_spans: Vec<Span>,
    required_capability: CaptureRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionTypeMismatch {
    Nullability,
    Arity,
    ParameterOwnership(usize),
    ParameterType(usize),
    ReturnType,
    InvocationMode,
    CheckedEffects,
    ReturnBorrow,
}

#[derive(Debug, Clone, Default)]
struct CheckedEffectSet {
    ordered: Vec<TypeId>,
}

impl CheckedEffectSet {
    fn insert(&mut self, ty: TypeId) {
        if !self.ordered.contains(&ty) {
            self.ordered.push(ty);
        }
    }

    fn extend(&mut self, effects: impl IntoIterator<Item = TypeId>) {
        for effect in effects {
            self.insert(effect);
        }
    }
}

fn extend_type_ids(target: &mut Vec<TypeId>, effects: impl IntoIterator<Item = TypeId>) {
    for effect in effects {
        if !target.contains(&effect) {
            target.push(effect);
        }
    }
}

#[derive(Debug, Clone)]
struct WhenCheckContext {
    expected: Option<TypeId>,
    inferred: Option<TypeId>,
    saw_value: bool,
}

#[derive(Debug, Clone, Copy)]
struct FinalizerBoundary {
    loop_depth: usize,
    when_depth: usize,
}

#[derive(Debug, Clone)]
struct EnumDefinition {
    id: EnumId,
    name: String,
    backing_type: Option<EnumBackingType>,
    cases: Vec<EnumCaseDefinition>,
    case_by_name: HashMap<String, usize>,
    capabilities: EnumCapabilities,
    layout: Option<EnumLayout>,
    span: Span,
}

#[derive(Debug, Clone)]
struct EnumCaseDefinition {
    id: EnumCaseId,
    name: String,
    tag: u32,
    backing_value: Option<EnumBackingValue>,
    payload: Vec<EnumPayloadDefinition>,
}

#[derive(Debug, Clone)]
struct EnumPayloadDefinition {
    name: String,
    ty: TypeId,
    span: Span,
}

#[allow(clippy::too_many_arguments)]
fn detect_inline_enum_cycles(
    id: EnumId,
    definitions: &HashMap<EnumId, EnumDefinition>,
    types: &TypeRegistry,
    states: &mut [u8],
    stack: &mut Vec<EnumId>,
    recursive: &mut HashSet<EnumId>,
    reported: &mut HashSet<Vec<usize>>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if states[id.0] == 2 {
        return;
    }
    if states[id.0] == 1 {
        return;
    }
    states[id.0] = 1;
    stack.push(id);
    let Some(definition) = definitions.get(&id) else {
        stack.pop();
        states[id.0] = 2;
        return;
    };
    for (case, field) in definition
        .cases
        .iter()
        .flat_map(|case| case.payload.iter().map(move |field| (case, field)))
    {
        let Some(dependency) = inline_enum_dependency(types, field.ty) else {
            continue;
        };
        if states[dependency.0] == 1 {
            let start = stack
                .iter()
                .position(|candidate| *candidate == dependency)
                .unwrap_or(0);
            let cycle = stack[start..].to_vec();
            recursive.extend(cycle.iter().copied());
            let mut identity = cycle.iter().map(|value| value.0).collect::<Vec<_>>();
            identity.sort_unstable();
            identity.dedup();
            if reported.insert(identity) {
                let mut names = cycle
                    .iter()
                    .filter_map(|value| definitions.get(value).map(|value| value.name.as_str()))
                    .collect::<Vec<_>>();
                names.push(
                    definitions
                        .get(&dependency)
                        .map_or("<unknown>", |value| value.name.as_str()),
                );
                diagnostics.push(
                    Diagnostic::new(
                        "E0581",
                        format!(
                            "recursive inline enum layout through `{}::{}` payload `${}`: {}",
                            definition.name,
                            case.name,
                            field.name,
                            names.join(" -> ")
                        ),
                        field.span,
                    )
                    .with_title("Recursive Inline Enum Layout")
                    .with_help(
                        "break the by-value cycle with a pointer-shaped owner such as a class or collection",
                    ),
                );
            }
            continue;
        }
        detect_inline_enum_cycles(
            dependency,
            definitions,
            types,
            states,
            stack,
            recursive,
            reported,
            diagnostics,
        );
    }
    stack.pop();
    states[id.0] = 2;
}

fn inline_enum_dependency(types: &TypeRegistry, ty: TypeId) -> Option<EnumId> {
    match types.kind(ty) {
        TypeKind::Enum(enum_type) => Some(enum_type.id),
        TypeKind::Nullable(inner) => inline_enum_dependency(types, *inner),
        _ => None,
    }
}

fn semantic_type_capabilities(
    types: &TypeRegistry,
    ty: TypeId,
    enums: &HashMap<EnumId, EnumCapabilities>,
) -> EnumCapabilities {
    let copy_trivial = EnumCapabilities {
        copy: true,
        trivial_copy: true,
        needs_drop: false,
        equality: true,
    };
    match types.kind(ty) {
        TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool | TypeKind::Null => copy_trivial,
        TypeKind::String => EnumCapabilities {
            copy: true,
            trivial_copy: false,
            needs_drop: true,
            equality: true,
        },
        TypeKind::Enum(enum_type) => {
            enums
                .get(&enum_type.id)
                .copied()
                .unwrap_or(EnumCapabilities {
                    copy: false,
                    trivial_copy: false,
                    needs_drop: true,
                    equality: false,
                })
        }
        TypeKind::Nullable(inner) => semantic_type_capabilities(types, *inner, enums),
        TypeKind::Class(_) => EnumCapabilities {
            copy: false,
            trivial_copy: false,
            needs_drop: true,
            equality: true,
        },
        TypeKind::Error => EnumCapabilities {
            copy: false,
            trivial_copy: false,
            needs_drop: true,
            equality: true,
        },
        TypeKind::Bytes => EnumCapabilities {
            copy: false,
            trivial_copy: false,
            needs_drop: true,
            equality: true,
        },
        TypeKind::Void
        | TypeKind::Function(_)
        | TypeKind::Mixed
        | TypeKind::TypedArray(_)
        | TypeKind::Unknown
        | TypeKind::Heterogeneous
        | TypeKind::EmptyCollection
        | TypeKind::TypeParameter(_)
        | TypeKind::List(_)
        | TypeKind::Dictionary(_, _)
        | TypeKind::SortedDictionary(_, _)
        | TypeKind::Set(_)
        | TypeKind::SortedSet(_)
        | TypeKind::PriorityQueue(_)
        | TypeKind::Deque(_)
        | TypeKind::SharedHandle(_, _) => EnumCapabilities {
            copy: false,
            trivial_copy: false,
            needs_drop: true,
            equality: false,
        },
    }
}

fn semantic_layout_shape(
    types: &TypeRegistry,
    ty: TypeId,
    enum_layouts: &HashMap<EnumId, EnumLayout>,
) -> Option<LayoutShape> {
    const POINTER: u32 = 8;
    let scalar = |bytes| LayoutShape {
        size: bytes,
        align: bytes,
    };
    match types.kind(ty) {
        TypeKind::Integer(value) => Some(scalar(value.storage_bytes())),
        TypeKind::Float(value) => Some(scalar(value.storage_bytes())),
        TypeKind::Bool => Some(scalar(1)),
        TypeKind::String
        | TypeKind::Bytes
        | TypeKind::Mixed
        | TypeKind::Class(_)
        | TypeKind::TypedArray(_)
        | TypeKind::List(_)
        | TypeKind::Dictionary(_, _)
        | TypeKind::SortedDictionary(_, _)
        | TypeKind::Set(_)
        | TypeKind::SortedSet(_)
        | TypeKind::PriorityQueue(_)
        | TypeKind::Deque(_)
        | TypeKind::SharedHandle(_, _) => Some(scalar(POINTER)),
        TypeKind::Function(_) => Some(LayoutShape {
            size: POINTER * crate::native_closure_abi::CARRIER_WORDS,
            align: POINTER,
        }),
        TypeKind::Enum(enum_type) => enum_layouts.get(&enum_type.id).map(|layout| LayoutShape {
            size: layout.size,
            align: layout.align,
        }),
        TypeKind::Nullable(inner) => {
            let inner_kind = types.kind(*inner);
            if matches!(inner_kind, TypeKind::Function(_)) {
                return Some(LayoutShape {
                    size: POINTER * crate::native_closure_abi::CARRIER_WORDS,
                    align: POINTER,
                });
            }
            if matches!(
                inner_kind,
                TypeKind::Class(_)
                    | TypeKind::Mixed
                    | TypeKind::Bytes
                    | TypeKind::TypedArray(_)
                    | TypeKind::List(_)
                    | TypeKind::Dictionary(_, _)
                    | TypeKind::SortedDictionary(_, _)
                    | TypeKind::Set(_)
                    | TypeKind::SortedSet(_)
                    | TypeKind::PriorityQueue(_)
                    | TypeKind::Deque(_)
                    | TypeKind::SharedHandle(_, _)
            ) {
                return Some(scalar(POINTER));
            }
            let payload = semantic_layout_shape(types, *inner, enum_layouts)?;
            let align = POINTER.max(payload.align);
            let payload_offset = checked_layout_align(POINTER, payload.align)?;
            let size = checked_layout_align(payload_offset.checked_add(payload.size)?, align)?;
            Some(LayoutShape { size, align })
        }
        TypeKind::Null => Some(LayoutShape { size: 0, align: 1 }),
        TypeKind::Void
        | TypeKind::Error
        | TypeKind::Unknown
        | TypeKind::Heterogeneous
        | TypeKind::EmptyCollection
        | TypeKind::TypeParameter(_) => None,
    }
}

fn checked_layout_align(value: u32, alignment: u32) -> Option<u32> {
    value
        .checked_add(alignment - 1)
        .map(|sum| sum & !(alignment - 1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiverAccess {
    Unavailable,
    Readonly,
    Writable,
    ConstructionRoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectPathAccess {
    Readonly,
    Writable,
    ConstructionRoot,
}

impl ReceiverAccess {
    fn is_available(self) -> bool {
        self != Self::Unavailable
    }

    fn is_writable(self) -> bool {
        self == Self::Writable
    }
}

#[derive(Debug, Clone)]
struct MethodContext {
    class_name: String,
    receiver_access: ReceiverAccess,
}

fn type_parameter_scope(params: &[TypeParamDecl]) -> HashMap<String, Vec<TypeRef>> {
    params
        .iter()
        .map(|param| (param.name.clone(), param.constraints.clone()))
        .collect()
}

/// The callee facts a call site needs to resolve a returned borrow back to the
/// argument that feeds it. Carrying the parameter list alongside the arguments
/// is what lets named-argument binding (decision 0098) find the right argument
/// when the written order differs from the parameter order.
#[derive(Clone, Copy)]
struct CallSite<'a> {
    return_ty: TypeId,
    return_borrow: Option<ReturnBorrow>,
    params: &'a [ParamInfo],
    args: &'a [Argument],
}

#[derive(Clone, Copy)]
struct CollectionMethodCall<'a> {
    object: &'a Expr,
    method: &'a str,
    args: &'a [Argument],
    member_span: Span,
    argument_list_span: Span,
    span: Span,
    scopes: &'a ScopeStack,
    method_context: Option<&'a MethodContext>,
}

#[derive(Debug, Clone)]
struct PendingGenericCall {
    callee: String,
    declaration: Span,
    type_params: Vec<TypeParamInfo>,
    bindings: HashMap<String, TypeId>,
    return_ty: TypeId,
}

#[derive(Debug, Clone)]
struct ConstructorInitContext {
    class_name: String,
    repeatable_body: bool,
}

impl ConstructorInitContext {
    fn nested(&self) -> Self {
        Self {
            class_name: self.class_name.clone(),
            repeatable_body: self.repeatable_body,
        }
    }

    fn repeatable(&self) -> Self {
        Self {
            class_name: self.class_name.clone(),
            repeatable_body: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstructorInitDecision {
    Allowed,
    Rejected,
    NotApplicable,
}

#[derive(Debug, Clone)]
struct ReturnContext {
    name: String,
    expected: Option<TypeId>,
    lifecycle: Option<LifecycleMethod>,
    is_method: bool,
}

impl ReturnContext {
    fn kind_name(&self) -> &'static str {
        if self.is_method {
            "method"
        } else {
            "function"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleMethod {
    Constructor,
    Destructor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypePosition {
    Return,
    Value,
}

impl LifecycleMethod {
    fn from_method_name(name: &str) -> Option<Self> {
        match name {
            "__construct" => Some(Self::Constructor),
            "__destruct" => Some(Self::Destructor),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Constructor => "constructor",
            Self::Destructor => "destructor",
        }
    }

    fn doria_name(self) -> &'static str {
        match self {
            Self::Constructor => "__construct",
            Self::Destructor => "__destruct",
        }
    }

    fn return_value_message(self) -> &'static str {
        match self {
            Self::Constructor => "constructors cannot return a value",
            Self::Destructor => "destructors cannot return a value",
        }
    }
}

#[derive(Debug, Clone)]
struct AssignmentTarget {
    ty: TypeId,
    destination: AssignmentDestination,
}

#[derive(Debug, Clone, Copy)]
struct StaticAccess<'a> {
    qualifier: &'a StaticQualifier,
    qualifier_span: Span,
    member_sigil_span: Option<Span>,
    member: &'a str,
    member_span: Span,
    span: Span,
}

#[derive(Debug, Clone)]
enum AssignmentDestination {
    Type,
    Parameter { name: String },
    Property { class_name: String, name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntConstantEval {
    Known(IntegerValue),
    Unknown,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayConversionKind {
    Primitive,
    DisplayableClass,
    NonDisplayableClass,
    Excluded,
    Recovery,
}

impl<'program> Checker<'program> {
    fn new(
        program: &'program Program,
        const_evaluation: crate::const_eval::Evaluation,
        source_texts: &HashMap<crate::source::SourceId, &'program str>,
        compilation_context: crate::names::CompilationContext,
        compilation_contexts: HashMap<crate::source::SourceId, crate::names::CompilationContext>,
        global_symbols: crate::names::GlobalSymbolFacts,
    ) -> Self {
        Self {
            program,
            source_texts: source_texts.clone(),
            compilation_context,
            compilation_contexts,
            global_symbols,
            classes: HashMap::new(),
            enums: HashMap::new(),
            functions: HashMap::new(),
            function_signatures: HashMap::new(),
            types: TypeRegistry::new(),
            diagnostics: Vec::new(),
            integer_expression_types: HashMap::new(),
            float_expression_types: HashMap::new(),
            expression_types: HashMap::new(),
            mixed_box_plans: HashMap::new(),
            enum_case_values: HashMap::new(),
            enum_case_constructions: HashMap::new(),
            type_test_types: HashMap::new(),
            matches: HashMap::new(),
            whens: HashMap::new(),
            given_preludes: HashMap::new(),
            call_targets: HashMap::new(),
            generic_call_specializations: HashMap::new(),
            constrained_display_calls: HashSet::new(),
            pending_generic_calls: HashMap::new(),
            type_parameter_scopes: Vec::new(),
            class_instantiations: HashSet::new(),
            class_instantiation_templates: HashMap::new(),
            callable_class_instantiation_templates: HashMap::new(),
            current_callable: None,
            integer_literals: HashMap::new(),
            negative_integer_literals: HashMap::new(),
            negated_integer_literal_operands: HashSet::new(),
            const_evaluation,
            parameter_defaults: HashMap::new(),
            flow_facts: crate::narrowing::analyze_program(program),
            contextual_expression_types: HashMap::new(),
            when_contexts: Vec::new(),
            active_loop_depth: 0,
            finalizer_boundaries: Vec::new(),
            effect_scopes: Vec::new(),
            class_initializer_effects: HashMap::new(),
            callable_observed_checked_effects: HashMap::new(),
            callable_declared_checked_effects: HashMap::new(),
            callable_dependencies: HashMap::new(),
            ambient_effect_seed: HashMap::new(),
            callable_effective_checked_effects: HashMap::new(),
            checked_effect_sites: HashMap::new(),
            throw_error_types: HashMap::new(),
            try_uncovered_effects: HashMap::new(),
            catch_error_types: HashMap::new(),
            function_types_by_span: HashMap::new(),
            binding_resolution: BindingResolution::default(),
            binding_ids: HashMap::new(),
            next_binding_id: 0,
            current_lexical_owner: LexicalOwner::TopLevel,
            closures: HashMap::new(),
            closure_types: HashMap::new(),
            callable_value_calls: HashMap::new(),
            list_algorithm_calls: HashMap::new(),
            property_writes: HashMap::new(),
            writable_object_paths: HashSet::new(),
            attributes: AttributeSemanticInfo::default(),
            active_closures: Vec::new(),
            initializing_bindings: Vec::new(),
        }
    }

    fn check(&mut self) {
        self.predeclare_classes();
        self.collect_enums();
        self.collect_classes();
        self.collect_functions();
        self.apply_ambient_effect_seed();
        self.check_instance_property_initializers();
        self.infer_return_borrow_signatures();
        self.infer_unannotated_move_return_signatures();

        // A clause-free selected entrypoint infers its public checked-effect
        // contract from the body. Check it provisionally to establish that
        // contract before the source-order pass reaches recursive calls and
        // earlier callers. Only the source-order pass publishes diagnostics.
        let inferred_entrypoint = self.program.items.iter().find_map(|item| match item {
            Item::Function(function)
                if function.throws.is_none() && self.is_accepted_program_entrypoint(function) =>
            {
                Some(function.clone())
            }
            _ => None,
        });
        if let Some(entrypoint) = inferred_entrypoint.as_ref() {
            let diagnostics_before_inference = self.diagnostics.len();
            self.check_function(entrypoint, None);
            self.diagnostics.truncate(diagnostics_before_inference);
        }

        let mut scopes = ScopeStack::new();
        for item in &self.program.items {
            match item {
                Item::Statement(statement) => {
                    self.check_statement(statement, &mut scopes, None, None, None, 0);
                }
                Item::Function(function) => self.check_function(function, None),
                Item::Constant(constant) => {
                    self.check_constant_initializer(&constant.initializer, None)
                }
                Item::Class(class_decl) => self.check_class(class_decl),
                Item::Enum(enum_decl) => self.check_enum(enum_decl),
                Item::Interface(interface_decl) => {
                    self.diagnostics
                        .push(interface_declaration_diagnostic(interface_decl));
                }
                Item::Trait(trait_decl) => {
                    self.diagnostics
                        .push(trait_declaration_diagnostic(trait_decl));
                }
            }
        }
        self.report_unresolved_generic_calls();
        self.check_pending_integer_literal_ranges();
    }

    fn check_attributes(&mut self) {
        let attachments = self.program.attributes.clone();
        let mut schemas = HashMap::<String, AttributeSchemaDraft>::new();
        for name in ["Attribute", "Test", "PHPExport"] {
            let schema = AttributeClassSchema {
                identity: AttributeClassIdentity::CompilerKnown(name.to_string()),
                canonical_name: name.to_string(),
                source: None,
                package: crate::names::PackageIdentity::CompilerKnown,
                declaration_span: None,
                parameters: Vec::new(),
            };
            schemas.insert(
                name.to_string(),
                AttributeSchemaDraft {
                    schema,
                    parameters: Vec::new(),
                    declaring_class: None,
                },
            );
        }

        // Schema discovery is deliberately complete before application binding,
        // so forward and cross-file attribute classes behave like every other
        // Stage 31 global declaration.
        for attachment in &attachments {
            for group in &attachment.groups {
                for attribute in &group.attributes {
                    if attribute.canonical_name.as_deref() != Some("Attribute") {
                        continue;
                    }
                    self.collect_attribute_schema(attachment, attribute, &mut schemas);
                }
            }
        }

        let mut applications = Vec::new();
        for attachment in &attachments {
            let Some(target) = self.resolve_attribute_target(&attachment.target) else {
                continue;
            };
            let (source, package) = self.attribute_context(attachment.target.target_span);
            let mut application_ordinal = 0usize;
            for (group_ordinal, group) in attachment.groups.iter().enumerate() {
                for attribute in &group.attributes {
                    let current_ordinal = application_ordinal;
                    application_ordinal += 1;
                    let Some(canonical_name) = attribute.canonical_name.as_deref() else {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0686",
                                format!("unknown attribute class `{}`", attribute.name.canonical()),
                                attribute.name.span,
                            )
                            .with_title("Unknown Attribute Class"),
                        );
                        continue;
                    };
                    if canonical_name == "Attribute" {
                        continue;
                    }
                    let Some(schema) = schemas.get(canonical_name).cloned() else {
                        let is_class = self.global_symbols.declarations.iter().any(|declaration| {
                            declaration.qualified_name == canonical_name
                                && declaration.kind == crate::names::GlobalSymbolKind::Class
                        });
                        let (code, title, message) = if is_class {
                            (
                                "E0687",
                                "Class Is Not Marked As An Attribute",
                                format!(
                                    "class `{canonical_name}` must be marked with `#[Attribute]` before it can be applied"
                                ),
                            )
                        } else {
                            (
                                "E0686",
                                "Unknown Attribute Class",
                                format!("unknown attribute class `{canonical_name}`"),
                            )
                        };
                        self.diagnostics.push(
                            Diagnostic::new(code, message, attribute.name.span).with_title(title),
                        );
                        continue;
                    };
                    if let Some(application) = self.bind_attribute_application(
                        attribute,
                        &schema,
                        target.clone(),
                        source.clone(),
                        package.clone(),
                        group_ordinal,
                        current_ordinal,
                    ) {
                        applications.push(application);
                    }
                }
            }
        }

        let mut schema_values = schemas
            .into_values()
            .map(|draft| draft.schema)
            .collect::<Vec<_>>();
        schema_values.sort_by(|left, right| left.canonical_name.cmp(&right.canonical_name));
        applications.sort_by(|left, right| {
            (
                left.source.0.as_str(),
                left.span.start,
                left.group_ordinal,
                left.application_ordinal,
            )
                .cmp(&(
                    right.source.0.as_str(),
                    right.span.start,
                    right.group_ordinal,
                    right.application_ordinal,
                ))
        });
        self.attributes = AttributeSemanticInfo {
            schemas: schema_values,
            applications,
        };
    }

    fn collect_attribute_schema(
        &mut self,
        attachment: &AttributeAttachment,
        marker: &AttributeRef,
        schemas: &mut HashMap<String, AttributeSchemaDraft>,
    ) {
        if attachment.target.kind != AttributeTargetKind::Class {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0688",
                    "`#[Attribute]` may mark only a class declaration",
                    marker.span,
                )
                .with_title("Attribute Marker Is Only Valid On A Class"),
            );
            return;
        }
        if marker
            .argument_list
            .as_ref()
            .is_some_and(|arguments| !arguments.arguments.is_empty())
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0689",
                    "`#[Attribute]` does not accept arguments",
                    marker.span,
                )
                .with_title("Attribute Marker Does Not Accept Arguments"),
            );
            return;
        }
        let Some(class_decl) = self.program.items.iter().find_map(|item| match item {
            Item::Class(class_decl) if class_decl.span == attachment.target.target_span => {
                Some(class_decl.clone())
            }
            _ => None,
        }) else {
            return;
        };
        let Some(global_id) = self.global_id_for_declaration_span(class_decl.span) else {
            return;
        };
        let diagnostics_before = self.diagnostics.len();
        if !class_decl.type_params.is_empty() {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0690",
                    format!("attribute class `{}` cannot be generic", class_decl.name),
                    class_decl.span,
                )
                .with_title("Attribute Class Cannot Be Generic"),
            );
        }
        let constructor = class_decl.members.iter().find_map(|member| match member {
            ClassMember::Method(method) if method.name == "__construct" => Some(method.clone()),
            _ => None,
        });
        let parameters = constructor
            .as_ref()
            .map_or_else(Vec::new, |constructor| constructor.params.clone());
        let mut parameter_drafts = Vec::with_capacity(parameters.len());
        let mut schema_parameters = Vec::with_capacity(parameters.len());
        for (index, param) in parameters.into_iter().enumerate() {
            if param.writable || param.take {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0691",
                        format!(
                            "attribute schema parameter `${}` must be readonly",
                            param.name
                        ),
                        param.span,
                    )
                    .with_title("Attribute Schema Parameter Must Be Readonly"),
                );
            }
            let ty = self.resolve_type_ref_with_class(
                &param.ty,
                param.span,
                Some(class_decl.name.as_str()),
            );
            if !self.attribute_metadata_type_is_compatible(ty, &mut HashSet::new()) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0692",
                        format!(
                            "attribute schema parameter `${}` has unsupported metadata type `{}`",
                            param.name,
                            self.types.display(ty)
                        ),
                        param.span,
                    )
                    .with_title("Attribute Schema Type Is Not Metadata Compatible"),
                );
            }
            let default_value = param.default.as_ref().and_then(|default| {
                let scopes = ScopeStack::new();
                let diagnostics_before_default = self.diagnostics.len();
                self.check_expr_assignable(
                    ty,
                    default,
                    &scopes,
                    None,
                    AssignmentDestination::Parameter {
                        name: param.name.clone(),
                    },
                );
                if self.diagnostics.len() != diagnostics_before_default {
                    return None;
                }
                match crate::const_eval::evaluate_attribute_value(
                    &self.const_evaluation,
                    default,
                    &param.ty,
                    Some(class_decl.name.as_str()),
                ) {
                    Ok(value) => {
                        let converted = crate::attributes::attribute_value_from_const(
                            self.types.resolved(ty),
                            value,
                            &self.const_evaluation,
                        );
                        if converted.is_none() {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0696",
                                    "attribute default could not be represented as typed metadata",
                                    default.span(),
                                )
                                .with_title("Attribute Default Is Not Const Evaluable"),
                            );
                        }
                        converted
                    }
                    Err(mut diagnostics) => {
                        for diagnostic in &mut diagnostics {
                            if diagnostic.code == "E0693" {
                                diagnostic.code = "E0696";
                                diagnostic.title =
                                    "Attribute Default Is Not Const Evaluable".to_string();
                            }
                        }
                        self.diagnostics.extend(diagnostics);
                        None
                    }
                }
            });
            schema_parameters.push(AttributeSchemaParameter {
                index,
                name: param.name.clone(),
                ty: self.types.resolved(ty),
                has_default: param.default.is_some(),
                span: param.span,
            });
            parameter_drafts.push(AttributeSchemaParameterDraft {
                declaration: param,
                ty,
                default_value,
            });
        }
        if self.diagnostics.len() != diagnostics_before {
            return;
        }
        let (source, package) = self.attribute_context(class_decl.span);
        let canonical_name = global_id.qualified_name.clone();
        schemas.insert(
            canonical_name.clone(),
            AttributeSchemaDraft {
                schema: AttributeClassSchema {
                    identity: AttributeClassIdentity::User(global_id),
                    canonical_name,
                    source: Some(source),
                    package,
                    declaration_span: Some(class_decl.span),
                    parameters: schema_parameters,
                },
                parameters: parameter_drafts,
                declaring_class: Some(class_decl.name),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_attribute_application(
        &mut self,
        attribute: &AttributeRef,
        schema: &AttributeSchemaDraft,
        target: AttributeTarget,
        source: crate::names::SourceIdentity,
        package: crate::names::PackageIdentity,
        group_ordinal: usize,
        application_ordinal: usize,
    ) -> Option<AttributeApplication> {
        let args = attribute
            .argument_list
            .as_ref()
            .map_or(&[][..], |arguments| arguments.arguments.as_slice());
        let param_names = schema
            .parameters
            .iter()
            .map(|parameter| parameter.declaration.name.as_str())
            .collect::<Vec<_>>();
        let param_has_default = schema
            .parameters
            .iter()
            .map(|parameter| parameter.declaration.default.is_some())
            .collect::<Vec<_>>();
        let arg_names = args
            .iter()
            .map(|argument| argument.name.as_ref().map(|name| name.text.as_str()))
            .collect::<Vec<_>>();
        let bound =
            crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names);
        let diagnostics_before = self.diagnostics.len();
        if bound.overflow > 0 {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0695",
                    format!(
                        "attribute `{}` accepts at most {} arguments, but {} were supplied",
                        schema.schema.canonical_name,
                        schema.parameters.len(),
                        args.len()
                    ),
                    attribute.span,
                )
                .with_title("Too Many Attribute Arguments"),
            );
        }
        for &argument_index in &bound.unknown {
            let name = args[argument_index]
                .name
                .as_ref()
                .expect("unknown named attribute argument has a name");
            let mut diagnostic = Diagnostic::new(
                "E0516",
                format!(
                    "attribute `{}` has no parameter named `{}`",
                    schema.schema.canonical_name, name.text
                ),
                name.span,
            )
            .with_title("Unknown Named Argument");
            if let Some(suggestion) =
                crate::arg_binding::unambiguous_name_suggestion(&name.text, &param_names)
            {
                diagnostic = diagnostic.with_help(format!("Did you mean `{suggestion}`?"));
            }
            self.diagnostics.push(diagnostic);
        }
        for &argument_index in &bound.duplicate {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0517",
                    "attribute argument was already supplied",
                    args[argument_index].span,
                )
                .with_title("Duplicate Named Argument"),
            );
        }
        if !bound.missing.is_empty() {
            let missing = bound
                .missing
                .iter()
                .map(|index| format!("`${}`", schema.parameters[*index].declaration.name))
                .collect::<Vec<_>>()
                .join(", ");
            self.diagnostics.push(
                Diagnostic::new(
                    "E0518",
                    format!("attribute application is missing required argument {missing}"),
                    attribute.span,
                )
                .with_title("Missing Required Argument"),
            );
        }
        if self.diagnostics.len() != diagnostics_before {
            return None;
        }

        let mut evaluated_args = vec![None; args.len()];
        let scopes = ScopeStack::new();
        for (argument_index, argument) in args.iter().enumerate() {
            let parameter_index = bound.arg_to_param[argument_index]
                .expect("valid attribute binding maps every authored argument");
            let parameter = &schema.parameters[parameter_index];
            let diagnostics_before_argument = self.diagnostics.len();
            self.check_expr_assignable(
                parameter.ty,
                &argument.value,
                &scopes,
                None,
                AssignmentDestination::Parameter {
                    name: parameter.declaration.name.clone(),
                },
            );
            if self.diagnostics.len() != diagnostics_before_argument {
                continue;
            }
            match crate::const_eval::evaluate_attribute_value(
                &self.const_evaluation,
                &argument.value,
                &parameter.declaration.ty,
                schema.declaring_class.as_deref(),
            ) {
                Ok(value) => {
                    evaluated_args[argument_index] = crate::attributes::attribute_value_from_const(
                        self.types.resolved(parameter.ty),
                        value,
                        &self.const_evaluation,
                    );
                    if evaluated_args[argument_index].is_none() {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0693",
                                "attribute argument could not be represented as typed metadata",
                                argument.span,
                            )
                            .with_title("Attribute Argument Must Be Const Evaluable"),
                        );
                    }
                }
                Err(diagnostics) => self.diagnostics.extend(diagnostics),
            }
        }
        if self.diagnostics.len() != diagnostics_before {
            return None;
        }

        let authored_arguments = args
            .iter()
            .enumerate()
            .map(|(index, argument)| AttributeAuthoredArgument {
                index,
                name: argument.name.as_ref().map(|name| name.text.clone()),
                span: argument.span,
                bound_parameter_index: bound.arg_to_param[index]
                    .expect("valid attribute binding maps every argument"),
            })
            .collect::<Vec<_>>();
        let mut bound_arguments = Vec::with_capacity(schema.parameters.len());
        for (parameter_index, parameter) in schema.parameters.iter().enumerate() {
            let (value, authored_argument_index, defaulted) =
                if let Some(argument_index) = bound.param_to_arg[parameter_index] {
                    (
                        evaluated_args[argument_index]
                            .clone()
                            .expect("valid attribute argument was evaluated"),
                        Some(argument_index),
                        false,
                    )
                } else {
                    (
                        parameter
                            .default_value
                            .clone()
                            .expect("binder omits only a validated defaulted parameter"),
                        None,
                        true,
                    )
                };
            bound_arguments.push(AttributeBoundArgument {
                parameter_index,
                parameter_name: parameter.declaration.name.clone(),
                ty: self.types.resolved(parameter.ty),
                value,
                defaulted,
                authored_argument_index,
            });
        }
        let target_key = target.canonical_key();
        let identity = format!(
            "{}#{target_key}:{group_ordinal}:{application_ordinal}",
            source.0
        );
        Some(AttributeApplication {
            identity,
            class_identity: schema.schema.identity.clone(),
            canonical_class_name: schema.schema.canonical_name.clone(),
            target,
            source,
            package,
            group_ordinal,
            application_ordinal,
            authored_arguments,
            bound_arguments,
            span: attribute.span,
        })
    }

    fn attribute_metadata_type_is_compatible(
        &self,
        ty: TypeId,
        visiting: &mut HashSet<EnumId>,
    ) -> bool {
        match self.types.kind(ty) {
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::String | TypeKind::Bool => true,
            TypeKind::Nullable(inner) => {
                self.attribute_metadata_type_is_compatible(*inner, visiting)
            }
            TypeKind::Enum(enum_type) => {
                if !visiting.insert(enum_type.id) {
                    return false;
                }
                let compatible = self.enums.get(&enum_type.name).is_some_and(|definition| {
                    definition.cases.iter().all(|case| {
                        case.payload.iter().all(|field| {
                            self.attribute_metadata_type_is_compatible(field.ty, visiting)
                        })
                    })
                });
                visiting.remove(&enum_type.id);
                compatible
            }
            TypeKind::Void
            | TypeKind::Bytes
            | TypeKind::Null
            | TypeKind::Mixed
            | TypeKind::Error
            | TypeKind::TypedArray(_)
            | TypeKind::Unknown
            | TypeKind::Heterogeneous
            | TypeKind::EmptyCollection
            | TypeKind::TypeParameter(_)
            | TypeKind::Function(_)
            | TypeKind::Class(_)
            | TypeKind::List(_)
            | TypeKind::Dictionary(_, _)
            | TypeKind::SortedDictionary(_, _)
            | TypeKind::Set(_)
            | TypeKind::SortedSet(_)
            | TypeKind::PriorityQueue(_)
            | TypeKind::Deque(_)
            | TypeKind::SharedHandle(_, _) => false,
        }
    }

    fn attribute_context(
        &self,
        span: Span,
    ) -> (crate::names::SourceIdentity, crate::names::PackageIdentity) {
        let context = self
            .compilation_contexts
            .get(&span.source)
            .unwrap_or(&self.compilation_context);
        (context.source.clone(), context.package.clone())
    }

    fn global_id_for_declaration_span(&self, span: Span) -> Option<crate::names::GlobalSymbolId> {
        self.global_symbols
            .declarations
            .iter()
            .find(|declaration| declaration.declaration_span == span)
            .map(|declaration| declaration.id.clone())
    }

    fn resolve_attribute_target(&self, syntax: &AttributeTargetSyntax) -> Option<AttributeTarget> {
        if matches!(
            syntax.kind,
            AttributeTargetKind::Class
                | AttributeTargetKind::Enum
                | AttributeTargetKind::Interface
                | AttributeTargetKind::Trait
                | AttributeTargetKind::Function
                | AttributeTargetKind::Constant
        ) {
            return self
                .global_id_for_declaration_span(syntax.target_span)
                .map(|declaration| AttributeTarget::GlobalDeclaration {
                    declaration,
                    kind: syntax.kind,
                });
        }

        for item in &self.program.items {
            match item {
                Item::Class(class_decl) => {
                    let Some(class_id) = self.global_id_for_declaration_span(class_decl.span)
                    else {
                        continue;
                    };
                    for member in &class_decl.members {
                        match member {
                            ClassMember::Property(property)
                                if property.span == syntax.target_span =>
                            {
                                return Some(AttributeTarget::ClassMember {
                                    class: class_id,
                                    kind: syntax.kind,
                                    name: property.name.clone(),
                                    span: property.span,
                                });
                            }
                            ClassMember::Constant(constant)
                                if constant.span == syntax.target_span =>
                            {
                                return Some(AttributeTarget::ClassMember {
                                    class: class_id,
                                    kind: syntax.kind,
                                    name: constant.name.clone(),
                                    span: constant.span,
                                });
                            }
                            ClassMember::Method(method) => {
                                if method.span == syntax.target_span {
                                    return Some(AttributeTarget::ClassMember {
                                        class: class_id.clone(),
                                        kind: syntax.kind,
                                        name: method.name.clone(),
                                        span: method.span,
                                    });
                                }
                                if let Some((index, parameter)) = method
                                    .params
                                    .iter()
                                    .enumerate()
                                    .find(|(_, parameter)| parameter.span == syntax.target_span)
                                {
                                    return Some(AttributeTarget::CallableParameter {
                                        callable: format!("{}::{}", class_decl.name, method.name),
                                        parameter_index: index,
                                        parameter_name: parameter.name.clone(),
                                        roles: syntax.roles.clone(),
                                        span: parameter.span,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Item::Trait(trait_decl) => {
                    let Some(trait_id) = self.global_id_for_declaration_span(trait_decl.span)
                    else {
                        continue;
                    };
                    for member in &trait_decl.members {
                        let (span, name) = match member {
                            ClassMember::Property(property) => {
                                (property.span, property.name.as_str())
                            }
                            ClassMember::Constant(constant) => {
                                (constant.span, constant.name.as_str())
                            }
                            ClassMember::Method(method) => {
                                if let Some((index, parameter)) = method
                                    .params
                                    .iter()
                                    .enumerate()
                                    .find(|(_, parameter)| parameter.span == syntax.target_span)
                                {
                                    return Some(AttributeTarget::CallableParameter {
                                        callable: format!("{}::{}", trait_decl.name, method.name),
                                        parameter_index: index,
                                        parameter_name: parameter.name.clone(),
                                        roles: syntax.roles.clone(),
                                        span: parameter.span,
                                    });
                                }
                                (method.span, method.name.as_str())
                            }
                        };
                        if span == syntax.target_span {
                            return Some(AttributeTarget::ClassMember {
                                class: trait_id.clone(),
                                kind: syntax.kind,
                                name: name.to_string(),
                                span,
                            });
                        }
                    }
                }
                Item::Function(function) => {
                    if let Some((index, parameter)) = function
                        .params
                        .iter()
                        .enumerate()
                        .find(|(_, parameter)| parameter.span == syntax.target_span)
                    {
                        return Some(AttributeTarget::CallableParameter {
                            callable: function.name.clone(),
                            parameter_index: index,
                            parameter_name: parameter.name.clone(),
                            roles: syntax.roles.clone(),
                            span: parameter.span,
                        });
                    }
                }
                Item::Enum(enum_decl) => {
                    let Some(enum_id) = self.global_id_for_declaration_span(enum_decl.span) else {
                        continue;
                    };
                    for (case_index, case) in enum_decl.cases.iter().enumerate() {
                        if case.span == syntax.target_span {
                            return Some(AttributeTarget::EnumCase {
                                enumeration: enum_id.clone(),
                                case_index,
                                case_name: case.name.clone(),
                                span: case.span,
                            });
                        }
                        if let Some((field_index, field)) = case
                            .payload
                            .iter()
                            .enumerate()
                            .find(|(_, field)| field.span == syntax.target_span)
                        {
                            return Some(AttributeTarget::EnumPayloadField {
                                enumeration: enum_id.clone(),
                                case_index,
                                field_index,
                                field_name: field.name.clone(),
                                span: field.span,
                            });
                        }
                    }
                }
                Item::Interface(_) | Item::Constant(_) | Item::Statement(_) => {}
            }
        }
        None
    }

    fn inferred_ambient_effects(&self) -> HashMap<Span, Vec<ResolvedType>> {
        let mut ambient = HashMap::<Span, Vec<ResolvedType>>::new();
        for (callable, effects) in self
            .callable_declared_checked_effects
            .iter()
            .chain(self.callable_observed_checked_effects.iter())
        {
            let target = ambient.entry(*callable).or_default();
            for effect in effects {
                if crate::checked_effects::is_ambient_io_effect(effect) && !target.contains(effect)
                {
                    target.push(effect.clone());
                }
            }
        }

        loop {
            let mut changed = false;
            for (caller, callees) in &self.callable_dependencies {
                let inherited = callees
                    .iter()
                    .flat_map(|callee| ambient.get(callee).into_iter().flatten())
                    .cloned()
                    .collect::<Vec<_>>();
                let target = ambient.entry(*caller).or_default();
                for effect in inherited {
                    if !target.contains(&effect) {
                        target.push(effect);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        ambient.retain(|_, effects| !effects.is_empty());
        ambient
    }

    fn escaping_ambient_effects(&self) -> HashMap<Span, Vec<ResolvedType>> {
        let mut ambient = HashMap::<Span, Vec<ResolvedType>>::new();
        for (callable, effects) in self
            .callable_declared_checked_effects
            .iter()
            .chain(self.callable_observed_checked_effects.iter())
        {
            let target = ambient.entry(*callable).or_default();
            for effect in effects {
                if crate::checked_effects::is_ambient_io_effect(effect) && !target.contains(effect)
                {
                    target.push(effect.clone());
                }
            }
        }
        ambient.retain(|_, effects| !effects.is_empty());
        ambient
    }

    fn apply_ambient_effect_seed(&mut self) {
        if self.ambient_effect_seed.is_empty() {
            return;
        }
        let seeds = self.ambient_effect_seed.clone();
        for (declaration, effects) in seeds {
            let ids = effects
                .iter()
                .map(|effect| self.types.intern_resolved(effect))
                .collect::<Vec<_>>();
            if let Some(signature) = self.function_signatures.get_mut(&declaration) {
                extend_type_ids(&mut signature.checked_effects, ids.iter().copied());
            }
            for signature in self.functions.values_mut() {
                if signature.declaration == declaration {
                    extend_type_ids(&mut signature.checked_effects, ids.iter().copied());
                }
            }
            for class in self.classes.values_mut() {
                for method in class.methods.values_mut() {
                    if method.declaration == declaration {
                        extend_type_ids(&mut method.checked_effects, ids.iter().copied());
                    }
                }
            }
            let effective = self
                .callable_effective_checked_effects
                .entry(declaration)
                .or_default();
            for effect in effects {
                if !effective.contains(&effect) {
                    effective.push(effect);
                }
            }
        }
    }

    fn predeclare_classes(&mut self) {
        for item in &self.program.items {
            let Item::Class(class_decl) = item else {
                continue;
            };

            let source_name = class_decl
                .name
                .rsplit('\\')
                .next()
                .unwrap_or(class_decl.name.as_str());
            if let Some(message) = Self::reserved_type_name_message(source_name) {
                self.diagnostics
                    .push(Diagnostic::new("E0309", message, class_decl.span));
                continue;
            }

            if self.classes.contains_key(&class_decl.name) {
                self.diagnostics.push(Diagnostic::new(
                    "E0300",
                    format!("class `{}` is already declared", class_decl.name),
                    class_decl.span,
                ));
                continue;
            }

            self.classes.insert(
                class_decl.name.clone(),
                ClassInfo {
                    type_params: class_decl
                        .type_params
                        .iter()
                        .map(|param| TypeParamInfo {
                            name: param.name.clone(),
                            constraints: param.constraints.clone(),
                        })
                        .collect(),
                    builtin_interfaces: HashSet::new(),
                    properties: HashMap::new(),
                    static_properties: HashMap::new(),
                    constants: HashMap::new(),
                    methods: HashMap::new(),
                    members: HashMap::new(),
                },
            );
        }
    }

    fn collect_classes(&mut self) {
        let mut processed = HashSet::new();
        let declarations = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(declaration) => Some(declaration),
                _ => None,
            })
            .collect::<Vec<_>>();

        for class_decl in declarations {
            if !processed.insert(class_decl.name.clone())
                || !self.classes.contains_key(&class_decl.name)
            {
                continue;
            }
            self.check_class_type_parameter_declarations(class_decl);
            self.type_parameter_scopes
                .push(type_parameter_scope(&class_decl.type_params));
            let mut info = ClassInfo {
                type_params: class_decl
                    .type_params
                    .iter()
                    .map(|param| TypeParamInfo {
                        name: param.name.clone(),
                        constraints: param.constraints.clone(),
                    })
                    .collect(),
                builtin_interfaces: class_decl
                    .implements
                    .iter()
                    .filter_map(|name| match name.as_str() {
                        "Displayable" => Some(BuiltinInterface::Displayable),
                        "Error" => Some(BuiltinInterface::Error),
                        _ => None,
                    })
                    .collect(),
                properties: HashMap::new(),
                static_properties: HashMap::new(),
                constants: HashMap::new(),
                methods: HashMap::new(),
                members: HashMap::new(),
            };

            for member in &class_decl.members {
                match member {
                    ClassMember::Property(property) => {
                        if property.is_static {
                            self.declare_static_property(&mut info, &class_decl.name, property);
                        } else {
                            self.declare_property(&mut info, &class_decl.name, property);
                        }
                    }
                    ClassMember::Constant(constant) => {
                        self.declare_class_constant(&mut info, &class_decl.name, constant);
                    }
                    ClassMember::Method(method) => {
                        if let Some(message) = Self::reserved_callable_name_message(&method.name) {
                            self.diagnostics
                                .push(Diagnostic::new("E0310", message, method.span));
                            continue;
                        }

                        let signature =
                            self.resolve_function_signature(method, Some(&class_decl.name));
                        self.function_signatures
                            .insert(method.span, signature.clone());

                        self.check_lifecycle_declaration_shape(method);

                        if method.is_static && method.writable_this {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0497",
                                    "static methods cannot be declared `writable` because they have no `$this` receiver",
                                    method.span,
                                )
                                .with_help("remove `writable`; mutate writable static properties through `ClassName::member` or `self::member`"),
                            );
                        }

                        if method.name == "__destruct" && !method.params.is_empty() {
                            self.diagnostics.push(Diagnostic::new(
                                "E0411",
                                "destructor `__destruct` cannot declare parameters",
                                method.span,
                            ));
                        }

                        let kind = if method.is_static {
                            MemberKind::StaticMethod
                        } else {
                            MemberKind::InstanceMethod
                        };
                        if self.declare_member_name(
                            &mut info,
                            &class_decl.name,
                            &method.name,
                            kind,
                            method.span,
                        ) {
                            info.methods.insert(
                                method.name.clone(),
                                MethodInfo {
                                    declaration: signature.declaration,
                                    access: method.access,
                                    receiver_mode: (!method.is_static).then_some(
                                        if method.writable_this
                                            && LifecycleMethod::from_method_name(&method.name)
                                                .is_none()
                                        {
                                            ReceiverMode::Writable
                                        } else {
                                            ReceiverMode::Readonly
                                        },
                                    ),
                                    return_borrow: signature.return_borrow,
                                    is_static: method.is_static,
                                    enclosing_type_bindings: HashMap::new(),
                                    type_params: signature.type_params,
                                    params: signature.params,
                                    return_ty: signature.return_ty,
                                    checked_effects: signature.checked_effects,
                                },
                            );
                        }

                        if method.name == "__construct" {
                            for param in &method.params {
                                if param.promoted_access.is_some() {
                                    self.declare_promoted_property(
                                        &mut info,
                                        &class_decl.name,
                                        param,
                                    );
                                }
                            }
                        }
                    }
                }
            }

            self.check_class_interfaces(class_decl, &info);

            self.type_parameter_scopes.pop();
            self.classes.insert(class_decl.name.clone(), info);
        }
    }

    fn collect_enums(&mut self) {
        let non_enum_types = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(declaration) => Some(declaration.name.as_str()),
                Item::Interface(declaration) => Some(declaration.name.as_str()),
                Item::Trait(declaration) => Some(declaration.name.as_str()),
                Item::Enum(_) | Item::Function(_) | Item::Constant(_) | Item::Statement(_) => None,
            })
            .collect::<HashSet<_>>();

        let declarations = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Enum(declaration) => Some(declaration),
                _ => None,
            })
            .collect::<Vec<_>>();

        for declaration in &declarations {
            let source_name = declaration
                .name
                .rsplit('\\')
                .next()
                .unwrap_or(declaration.name.as_str());
            if let Some(message) = Self::reserved_type_name_message(source_name) {
                self.diagnostics.push(
                    Diagnostic::new("E0561", message, declaration.span)
                        .with_title("Type Name Collision"),
                );
                continue;
            }
            if non_enum_types.contains(declaration.name.as_str()) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0561",
                        format!(
                            "type name `{}` is already used by another declaration",
                            declaration.name
                        ),
                        declaration.span,
                    )
                    .with_title("Type Name Collision"),
                );
                continue;
            }
            if self.enums.contains_key(&declaration.name) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0560",
                        format!("enum `{}` is already declared", declaration.name),
                        declaration.span,
                    )
                    .with_title("Duplicate Enum"),
                );
                continue;
            }
            let id = EnumId(self.enums.len());
            self.enums.insert(
                declaration.name.clone(),
                EnumDefinition {
                    id,
                    name: declaration.name.clone(),
                    backing_type: None,
                    cases: Vec::new(),
                    case_by_name: HashMap::new(),
                    capabilities: EnumCapabilities {
                        copy: true,
                        trivial_copy: true,
                        needs_drop: false,
                        equality: true,
                    },
                    layout: None,
                    span: declaration.span,
                },
            );
        }

        let mut processed = HashSet::new();
        for declaration in declarations {
            if !processed.insert(declaration.name.clone()) {
                continue;
            }
            let Some(mut definition) = self.enums.get(&declaration.name).cloned() else {
                continue;
            };
            let source_name = declaration
                .name
                .rsplit('\\')
                .next()
                .unwrap_or(declaration.name.as_str());
            if !Self::uses_pascal_case(source_name) {
                let mut diagnostic = Diagnostic::new(
                    "E0563",
                    format!("enum name `{source_name}` must use PascalCase"),
                    declaration.name_span,
                )
                .with_title("Enum Name Must Use PascalCase");
                if let Some(replacement) = Self::safe_pascal_case_fix(source_name) {
                    let canonical_replacement = declaration.name.rsplit_once('\\').map_or_else(
                        || replacement.clone(),
                        |(namespace, _)| format!("{namespace}\\{replacement}"),
                    );
                    let collides = non_enum_types.contains(canonical_replacement.as_str())
                        || self.enums.contains_key(&canonical_replacement);
                    if !collides {
                        diagnostic = diagnostic.with_fix(declaration.name_span, replacement);
                    }
                }
                self.diagnostics.push(diagnostic);
            }
            if !declaration.type_params.is_empty() {
                self.diagnostics.push(
                    Diagnostic::unsupported_stage(
                        "E0572",
                        "generic enum syntax is accepted, but generic enums are not implemented",
                        declaration.span,
                    )
                    .with_title("Generic Enums Are Not Implemented"),
                );
            }
            if declaration.cases.is_empty() {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0562",
                        format!("enum `{}` must declare at least one case", declaration.name),
                        declaration.span,
                    )
                    .with_title("Enum Must Have A Case"),
                );
            }

            definition.backing_type = declaration.backing_type.as_ref().and_then(|ty| {
                if ty.nullable || !ty.arguments.is_empty() {
                    self.report_invalid_enum_backing(ty, declaration.span);
                    return None;
                }
                match ty.name.as_str() {
                    "int" => Some(EnumBackingType::Int),
                    "string" => Some(EnumBackingType::String),
                    _ => {
                        self.report_invalid_enum_backing(ty, declaration.span);
                        None
                    }
                }
            });

            let mut backing_values = HashSet::new();
            let declared_case_names = declaration
                .cases
                .iter()
                .map(|case| case.name.as_str())
                .collect::<HashSet<_>>();
            for case in &declaration.cases {
                if definition.case_by_name.contains_key(&case.name) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0565",
                            format!(
                                "enum `{}` already declares case `{}`",
                                declaration.name, case.name
                            ),
                            case.span,
                        )
                        .with_title("Duplicate Enum Case"),
                    );
                    continue;
                }
                if !Self::uses_pascal_case(&case.name) {
                    let mut diagnostic = Diagnostic::new(
                        "E0564",
                        format!("enum case `{}` must use PascalCase", case.name),
                        case.name_span,
                    )
                    .with_title("Case Name Must Use PascalCase");
                    if let Some(replacement) = Self::safe_pascal_case_fix(&case.name) {
                        if !declared_case_names.contains(replacement.as_str()) {
                            diagnostic = diagnostic.with_fix(case.name_span, replacement);
                        }
                    }
                    self.diagnostics.push(diagnostic);
                }
                let mut field_names = HashSet::new();
                let payload = case
                    .payload
                    .iter()
                    .filter_map(|field| {
                        if !field_names.insert(field.name.clone()) {
                            self.diagnostics.push(Diagnostic::new(
                                "E0565",
                                format!("payload field `${}` is already declared", field.name),
                                field.span,
                            ));
                            return None;
                        }
                        Some(EnumPayloadDefinition {
                            name: field.name.clone(),
                            ty: self.resolve_type_ref(&field.ty, field.span),
                            span: field.span,
                        })
                    })
                    .collect::<Vec<_>>();
                if definition.backing_type.is_some() && !payload.is_empty() {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0571",
                            "a backed enum cannot declare payload cases",
                            case.span,
                        )
                        .with_title("Backed Enum Cannot Have Payload Cases"),
                    );
                }

                let backing_value = match (definition.backing_type, &case.backing_value) {
                    (Some(backing_type), Some(value)) if payload.is_empty() => {
                        self.evaluate_enum_backing(backing_type, value, case.span)
                    }
                    (Some(_), None) if payload.is_empty() => {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0567",
                                "every backed enum case must declare a backing value",
                                case.span,
                            )
                            .with_title("Backed Case Requires A Value"),
                        );
                        None
                    }
                    (None, Some(_)) => {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0568",
                                "a plain enum case cannot declare a backing value",
                                case.span,
                            )
                            .with_title("Plain Case Cannot Have A Backing Value"),
                        );
                        None
                    }
                    _ => None,
                };
                if let Some(value) = &backing_value {
                    if !backing_values.insert(value.clone()) {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0569",
                                "backed enum case values must be unique",
                                case.span,
                            )
                            .with_title("Duplicate Backing Value"),
                        );
                    }
                }

                let id = EnumCaseId {
                    enum_id: definition.id,
                    index: definition.cases.len(),
                };
                let tag = definition.cases.len() as u32;
                definition
                    .case_by_name
                    .insert(case.name.clone(), definition.cases.len());
                definition.cases.push(EnumCaseDefinition {
                    id,
                    name: case.name.clone(),
                    tag,
                    backing_value,
                    payload,
                });
            }
            self.enums.insert(declaration.name.clone(), definition);
        }
        self.finalize_enum_metadata();
    }

    fn check_enum(&mut self, _declaration: &EnumDecl) {}

    fn finalize_enum_metadata(&mut self) {
        let definitions = self
            .enums
            .values()
            .cloned()
            .map(|definition| (definition.id, definition))
            .collect::<HashMap<_, _>>();
        let mut states = vec![0_u8; definitions.len()];
        let mut stack = Vec::new();
        let mut recursive = HashSet::new();
        let mut reported_cycles = HashSet::new();
        let mut diagnostics = Vec::new();
        for id in definitions.keys().copied() {
            detect_inline_enum_cycles(
                id,
                &definitions,
                &self.types,
                &mut states,
                &mut stack,
                &mut recursive,
                &mut reported_cycles,
                &mut diagnostics,
            );
        }
        self.diagnostics.extend(diagnostics);

        let mut capabilities = definitions
            .keys()
            .copied()
            .map(|id| {
                (
                    id,
                    if recursive.contains(&id) {
                        EnumCapabilities {
                            copy: false,
                            trivial_copy: false,
                            needs_drop: true,
                            equality: false,
                        }
                    } else {
                        EnumCapabilities {
                            copy: true,
                            trivial_copy: true,
                            needs_drop: false,
                            equality: true,
                        }
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        loop {
            let mut changed = false;
            for definition in definitions.values() {
                if recursive.contains(&definition.id) {
                    continue;
                }
                let mut next = EnumCapabilities {
                    copy: true,
                    trivial_copy: true,
                    needs_drop: false,
                    equality: true,
                };
                for field in definition.cases.iter().flat_map(|case| &case.payload) {
                    let field = semantic_type_capabilities(&self.types, field.ty, &capabilities);
                    next.copy &= field.copy;
                    next.trivial_copy &= field.trivial_copy;
                    next.needs_drop |= field.needs_drop;
                    next.equality &= field.equality;
                }
                if capabilities.get(&definition.id) != Some(&next) {
                    capabilities.insert(definition.id, next);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let mut layouts = HashMap::<EnumId, EnumLayout>::new();
        loop {
            let mut changed = false;
            for definition in definitions.values() {
                if recursive.contains(&definition.id) || layouts.contains_key(&definition.id) {
                    continue;
                }
                let case_fields = definition
                    .cases
                    .iter()
                    .map(|case| {
                        case.payload
                            .iter()
                            .map(|field| semantic_layout_shape(&self.types, field.ty, &layouts))
                            .collect::<Option<Vec<_>>>()
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(case_fields) = case_fields else {
                    continue;
                };
                match crate::enums::compute_enum_layout(definition.id, &case_fields) {
                    Ok(layout) => {
                        layouts.insert(definition.id, layout);
                        changed = true;
                    }
                    Err(error) => self.diagnostics.push(
                        Diagnostic::new(
                            "E0582",
                            format!(
                                "enum `{}` has no representable finite inline layout: {error:?}",
                                definition.name
                            ),
                            definition.span,
                        )
                        .with_title("Enum Layout Is Too Large"),
                    ),
                }
            }
            if !changed {
                break;
            }
        }

        for definition in self.enums.values_mut() {
            definition.capabilities = capabilities[&definition.id];
            definition.layout = layouts.get(&definition.id).cloned();
        }
    }

    fn uses_pascal_case(name: &str) -> bool {
        name.chars()
            .next()
            .is_some_and(|first| first.is_ascii_uppercase())
    }

    fn safe_pascal_case_fix(name: &str) -> Option<String> {
        let mut characters = name.chars();
        let first = characters.next()?;
        if !first.is_ascii_lowercase()
            || !characters
                .clone()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return None;
        }
        let mut replacement = first.to_ascii_uppercase().to_string();
        replacement.extend(characters);
        Some(replacement)
    }

    fn report_invalid_enum_backing(&mut self, ty: &TypeRef, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0566",
                format!("enum backing type must be `int` or `string`, found `{ty}`"),
                span,
            )
            .with_title("Invalid Backing Type"),
        );
    }

    fn evaluate_enum_backing(
        &mut self,
        backing_type: EnumBackingType,
        value: &Expr,
        span: Span,
    ) -> Option<EnumBackingValue> {
        let expected = TypeRef::named(match backing_type {
            EnumBackingType::Int => "int",
            EnumBackingType::String => "string",
        });
        let evaluated = crate::const_eval::evaluate_parameter_default(
            &self.const_evaluation,
            value,
            &expected,
            None,
        );
        let result = match (backing_type, evaluated) {
            (EnumBackingType::Int, Some(crate::const_eval::ConstValue::Integer(value))) => {
                Some(EnumBackingValue::Int(value))
            }
            (EnumBackingType::String, Some(crate::const_eval::ConstValue::String(value))) => {
                Some(EnumBackingValue::String(value))
            }
            _ => None,
        };
        if result.is_none() {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0570",
                    "enum backing value must be a constant expression of the declared backing type",
                    span,
                )
                .with_title("Wrong Backing Value Type"),
            );
        }
        result
    }

    fn check_class_type_parameter_declarations(&mut self, class: &ClassDecl) {
        self.check_type_parameter_declarations(
            &class.type_params,
            &format!("class `{}`", class.name),
        );
    }

    fn check_class_interfaces(&mut self, class_decl: &ClassDecl, info: &ClassInfo) {
        let mut seen = HashSet::new();
        for interface in &class_decl.implements {
            if !seen.insert(interface) {
                self.diagnostics.push(Diagnostic::new(
                    "E0464",
                    format!(
                        "class `{}` implements `{interface}` more than once",
                        class_decl.name
                    ),
                    class_decl.span,
                ));
                continue;
            }
            if !matches!(interface.as_str(), "Displayable" | "Error") {
                self.diagnostics.push(Diagnostic::new(
                    "E0464",
                    format!(
                        "general interface conformance for `{interface}` is accepted syntax but is not available in this compiler version"
                    ),
                    class_decl.span,
                ));
            }
        }

        if info.implements(BuiltinInterface::Error) {
            self.check_error_interface(class_decl, info);
        }

        if !info.implements(BuiltinInterface::Displayable) {
            return;
        }

        let Some(method) = info.methods.get("toString") else {
            let help = if info.methods.contains_key("__toString") {
                "Doria does not use `__toString`; declare `function toString(): string`"
            } else if info.methods.contains_key("to_string") {
                "Doria member names use camelCase; declare `function toString(): string`"
            } else {
                "declare `function toString(): string`"
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0463",
                    format!(
                        "class `{}` implements `Displayable` but does not provide `toString(): string`",
                        class_decl.name
                    ),
                    class_decl.span,
                )
                .with_help(help),
            );
            return;
        };

        let valid = method.access == MemberAccess::External
            && method.receiver_mode == Some(ReceiverMode::Readonly)
            && !method.is_static
            && method.type_params.is_empty()
            && method.params.is_empty()
            && method.checked_effects.is_empty()
            && matches!(self.types.kind(method.return_ty), TypeKind::String);
        if !valid {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0463",
                    format!(
                        "class `{}` has an incompatible `Displayable::toString` method",
                        class_decl.name
                    ),
                    class_decl.span,
                )
                .with_help(
                    "declare exactly `function toString(): string` as an externally accessible readonly instance method",
                ),
            );
        }
    }

    fn check_error_interface(&mut self, class_decl: &ClassDecl, info: &ClassInfo) {
        let Some(message) = info.properties.get("message") else {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0613",
                    format!(
                        "class `{}` implements `Error` but has no `message` property",
                        class_decl.name
                    ),
                    class_decl.span,
                )
                .with_title("Error Message Property Is Missing")
                .with_help("declare an externally accessible readonly `string $message` property"),
            );
            return;
        };

        if !matches!(self.types.kind(message.ty), TypeKind::String) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0614",
                    "Error `message` must have type `string`",
                    message.declaration_span,
                )
                .with_title("Error Message Property Must Be String"),
            );
        }
        if message.writable {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0615",
                    "Error `message` must be readonly",
                    message.declaration_span,
                )
                .with_title("Error Message Property Must Be Readonly")
                .with_help("remove `writable` from the message property"),
            );
        }
        if message.access != MemberAccess::External {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0616",
                    "Error `message` must be externally accessible",
                    message.declaration_span,
                )
                .with_title("Error Message Property Must Be Externally Accessible")
                .with_help("remove `internal` from the message property"),
            );
        }
    }

    fn check_lifecycle_declaration_shape(&mut self, method: &FunctionDecl) {
        let Some(lifecycle) = LifecycleMethod::from_method_name(&method.name) else {
            return;
        };

        if let Some(span) = method.static_span {
            let message = match lifecycle {
                LifecycleMethod::Constructor => {
                    "`__construct` is invoked by `new` and cannot be `static`"
                }
                LifecycleMethod::Destructor => {
                    "`__destruct` is invoked automatically when an instance is destroyed and cannot be `static`"
                }
            };
            self.diagnostics
                .push(Diagnostic::new("E0465", message, span));
        }

        if let Some(span) = method.writable_span {
            let help = match lifecycle {
                LifecycleMethod::Constructor => {
                    "remove `writable`; construction grants `__construct` its access to the new instance"
                }
                LifecycleMethod::Destructor => {
                    "remove `writable`; destruction invokes `__destruct` through the lifecycle protocol"
                }
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0466",
                    format!("`{}` cannot be declared `writable`", lifecycle.doria_name()),
                    span,
                )
                .with_help(help)
                .with_fix(span, ""),
            );
        }
    }

    fn reserved_type_name_message(name: &str) -> Option<String> {
        if SharedHandleKind::from_source_name(name).is_some() {
            return Some(format!(
                "`{name}` is a compiler-known shared-ownership type and cannot be redeclared"
            ));
        }
        if matches!(name, "Float" | "Float32" | "Float64" | "Bool") {
            return Some(format!(
                "`{name}` is a compiler-known scalar companion and cannot be redeclared"
            ));
        }
        if IntegerType::from_companion_name(name).is_some() {
            return Some(format!(
                "`{name}` is a compiler-known integer companion and cannot be redeclared"
            ));
        }
        if name.to_ascii_lowercase().starts_with("__doria") {
            return Some(format!(
                "the `__Doria` type prefix is reserved for compiler-generated PHP compatibility output; `{name}` cannot be redeclared"
            ));
        }
        match name {
            "self" => Some(
                "`self` is reserved for the declaring or composing class context and cannot be redeclared as a type"
                    .to_string(),
            ),
            "Displayable" => Some(
                "`Displayable` is a compiler-known interface and cannot be redeclared"
                    .to_string(),
            ),
            "Error" => Some(
                "`Error` is a compiler-known interface and cannot be redeclared".to_string(),
            ),
            "Bytes" => Some(
                "`Bytes` is the compiler-known byte-buffer type and cannot be redeclared"
                    .to_string(),
            ),
            "String" => Some(
                "`String` is the compiler-known string companion and cannot be redeclared"
                    .to_string(),
            ),
            "List"
            | "Dictionary"
            | "SortedDictionary"
            | "Set"
            | "SortedSet"
            | "PriorityQueue"
            | "Deque" => Some(format!(
                "`{name}` is a compiler-known collection alias and cannot be redeclared"
            )),
            "array" => Some(
                "`array` is not a Doria type name; use typed arrays like `T[]` or collection aliases"
                    .to_string(),
            ),
            "mixed" => Some(
                "`mixed` is a Doria dynamic-boundary type and cannot be redeclared"
                    .to_string(),
            ),
            "object" => Some(
                "`object` is not a Doria type and cannot be redeclared".to_string(),
            ),
            "resource" => Some(
                "`resource` is reserved for future PHP interop and cannot be redeclared"
                    .to_string(),
            ),
            _ => None,
        }
    }

    fn reserved_callable_name_message(name: &str) -> Option<String> {
        if matches!(name, "Float" | "Float32" | "Float64" | "Bool") {
            return Some(format!(
                "`{name}` is a compiler-known scalar companion and cannot be redeclared"
            ));
        }
        if IntegerType::from_companion_name(name).is_some() {
            return Some(format!(
                "`{name}` is a compiler-known integer companion and cannot be redeclared"
            ));
        }
        match name {
            name if php_function_suggestion(name) == Some("read_line") => Some(format!(
                "Doria uses `read_line`; the PHP spelling `{name}` cannot be declared"
            )),
            name if is_reserved_intrinsic_name(name) => Some(format!(
                "`{name}` is a compiler-known Doria intrinsic name and cannot be redeclared"
            )),
            "array" => Some(
                "`array` is not a Doria callable name; use typed arrays like `T[]` or collection aliases"
                    .to_string(),
            ),
            "mixed" => Some(
                "`mixed` is a Doria dynamic-boundary type and cannot be used as a callable name"
                    .to_string(),
            ),
            "object" => Some(
                "`object` is not a Doria type and cannot be used as a callable name".to_string(),
            ),
            "resource" => Some(
                "`resource` is reserved for future PHP interop and cannot be used as a callable name"
                    .to_string(),
            ),
            _ => None,
        }
    }

    fn collect_functions(&mut self) {
        for item in &self.program.items {
            let Item::Function(function) = item else {
                continue;
            };
            let source_name = function
                .name
                .rsplit('\\')
                .next()
                .unwrap_or(function.name.as_str());

            if source_name
                .get(.."__doria_".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("__doria_"))
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0310",
                        "top-level function names beginning with `__doria_` are reserved for compiler-generated helpers",
                        function.span,
                    )
                    .with_help("choose a function name that does not begin with `__doria_`"),
                );
                continue;
            }

            if source_name == "print" {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0310",
                        "Doria does not support a top-level `print` function; use `echo`",
                        function.span,
                    )
                    .with_help("remove the `print` declaration and use `echo` for output"),
                );
                continue;
            }

            if let Some(message) = Self::reserved_callable_name_message(source_name) {
                self.diagnostics
                    .push(Diagnostic::new("E0310", message, function.span));
                continue;
            }

            let signature = self.resolve_function_signature(function, None);
            self.function_signatures
                .insert(function.span, signature.clone());
            if self.functions.contains_key(&function.name) {
                self.diagnostics.push(Diagnostic::new(
                    "E0308",
                    format!("function `{}` is already declared", function.name),
                    function.span,
                ));
                continue;
            }

            self.functions.insert(function.name.clone(), signature);
        }
    }

    fn resolve_function_signature(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) -> FunctionInfo {
        self.check_function_type_parameter_declarations(function, declaring_class);
        let previous_callable = self.current_callable.replace(function.span);
        self.type_parameter_scopes
            .push(type_parameter_scope(&function.type_params));
        let params = self.resolve_param_infos(function, declaring_class);
        let return_ty = self.resolve_function_return_type(function, declaring_class);
        let checked_effects = self.resolve_throws_clause(function, declaring_class);
        let resolved_checked_effects = checked_effects
            .iter()
            .map(|effect| self.types.resolved(*effect))
            .collect::<Vec<_>>();
        self.callable_declared_checked_effects
            .insert(function.span, resolved_checked_effects.clone());
        self.callable_effective_checked_effects
            .insert(function.span, resolved_checked_effects);
        self.type_parameter_scopes.pop();
        self.current_callable = previous_callable;
        let return_borrow = self
            .type_can_return_borrow(return_ty)
            .then(|| {
                let enclosing_type_params = declaring_class
                    .and_then(|class_name| {
                        self.program.items.iter().find_map(|item| match item {
                            Item::Class(class) if class.name == class_name => {
                                Some(&class.type_params)
                            }
                            _ => None,
                        })
                    })
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                crate::ownership::function_return_borrow_in_context(
                    function,
                    enclosing_type_params,
                    &mut |_| None,
                )
            })
            .flatten();

        FunctionInfo {
            declaration: function.span,
            type_params: function
                .type_params
                .iter()
                .map(|param| TypeParamInfo {
                    name: param.name.clone(),
                    constraints: param.constraints.clone(),
                })
                .collect(),
            params,
            return_ty,
            return_borrow,
            checked_effects,
        }
    }

    fn resolve_throws_clause(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) -> Vec<TypeId> {
        let Some(clause) = &function.throws else {
            return Vec::new();
        };
        if function.name == "__destruct" {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0622",
                    "destructors cannot declare checked errors",
                    clause.span,
                )
                .with_title("Destructors Cannot Throw Checked Errors")
                .with_help("catch every checked error inside `__destruct`"),
            );
        }

        let mut effects = Vec::new();
        let mut saw_error = false;
        for (entry_index, entry) in clause.entries.iter().enumerate() {
            if entry.ty.nullable {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0619",
                        format!("throws type `{}` cannot be nullable", entry.ty),
                        entry.span,
                    )
                    .with_title("Throws Type Cannot Be Nullable"),
                );
                continue;
            }
            let ty = self.resolve_type_ref_with_class(&entry.ty, entry.span, declaring_class);
            if self.is_unknown_type(ty) {
                continue;
            }
            if !self.type_implements_error(ty) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0618",
                        format!(
                            "throws type `{}` does not implement `Error`",
                            self.types.display(ty)
                        ),
                        entry.span,
                    )
                    .with_title("Throws Type Must Implement Error"),
                );
                continue;
            }
            if effects.contains(&ty) {
                let (fix_span, applicability) =
                    self.trailing_throws_entry_removal(clause, entry_index);
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0620",
                        format!("duplicate throws entry `{}`", self.types.display(ty)),
                        entry.span,
                    )
                    .with_title("Duplicate Throws Entry")
                    .with_help("remove the duplicate entry")
                    .with_structured_fix(
                        "Remove Duplicate Throws Entry",
                        applicability,
                        vec![FixEdit {
                            source: DiagnosticSource::Current,
                            span: fix_span,
                            replacement: String::new(),
                        }],
                    ),
                );
                continue;
            }
            if saw_error || (matches!(self.types.kind(ty), TypeKind::Error) && !effects.is_empty())
            {
                let (fix_span, applicability) =
                    self.trailing_throws_entry_removal(clause, entry_index);
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0621",
                        "`Error` already covers every concrete checked error in this throws list",
                        entry.span,
                    )
                    .with_title("Error Already Covers This Throws Entry")
                    .with_help("remove the redundant entry")
                    .with_structured_fix(
                        "Remove Redundant Throws Entry",
                        applicability,
                        vec![FixEdit {
                            source: DiagnosticSource::Current,
                            span: fix_span,
                            replacement: String::new(),
                        }],
                    ),
                );
                continue;
            }
            saw_error = matches!(self.types.kind(ty), TypeKind::Error);
            effects.push(ty);
        }
        effects
    }

    fn trailing_throws_entry_removal(
        &self,
        clause: &ThrowsClause,
        entry_index: usize,
    ) -> (Span, FixApplicability) {
        let entry = &clause.entries[entry_index];
        let Some(previous) = entry_index
            .checked_sub(1)
            .and_then(|index| clause.entries.get(index))
        else {
            return (entry.span, FixApplicability::RequiresReview);
        };
        let separator = Span::new(previous.span.end, entry.span.start);
        let applicability = if self.source_slice(separator).is_some_and(|source| {
            source
                .chars()
                .all(|character| character == ',' || character.is_ascii_whitespace())
        }) {
            FixApplicability::MachineApplicable
        } else {
            FixApplicability::RequiresReview
        };
        (Span::new(previous.span.end, entry.span.end), applicability)
    }

    fn type_implements_error(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::Error => true,
            TypeKind::Class(class) => {
                self.classes
                    .get(&class.name)
                    .is_some_and(|info| info.implements(BuiltinInterface::Error))
                    || self.program.items.iter().any(|item| {
                        matches!(item, Item::Class(declaration)
                        if declaration.name == class.name
                            && declaration.implements.iter().any(|name| name == "Error"))
                    })
            }
            _ => false,
        }
    }

    fn check_function_type_parameter_declarations(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) {
        if !function.type_params.is_empty()
            && matches!(
                function.name.as_str(),
                "main" | "__construct" | "__destruct"
            )
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0530",
                    format!(
                        "`{}` cannot declare type parameters because it is invoked by the runtime",
                        function.name
                    ),
                    function.span,
                )
                .with_help("move the generic behavior into a regular function or method"),
            );
        }
        if let Some(class_name) = declaring_class {
            let enclosing_parameters = self.program.items.iter().find_map(|item| match item {
                Item::Class(class) if class.name == class_name => Some(&class.type_params),
                _ => None,
            });
            if let Some(enclosing_parameters) = enclosing_parameters {
                for parameter in &function.type_params {
                    if enclosing_parameters
                        .iter()
                        .any(|enclosing| enclosing.name == parameter.name)
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0530",
                                format!(
                                    "type parameter `{}` on method `{class_name}::{}` shadows the enclosing class type parameter",
                                    parameter.name, function.name
                                ),
                                parameter.span,
                            )
                            .with_help(
                                "rename the method type parameter so class and method substitutions remain distinct",
                            ),
                        );
                    }
                }
            }
        }
        self.check_type_parameter_declarations(
            &function.type_params,
            &format!("function `{}`", function.name),
        );
    }

    fn check_type_parameter_declarations(&mut self, params: &[TypeParamDecl], owner: &str) {
        let mut names = HashSet::new();
        self.type_parameter_scopes
            .push(type_parameter_scope(params));
        for param in params {
            let valid_name =
                param.name.len() == 1 && param.name.bytes().all(|byte| byte.is_ascii_uppercase());
            if !valid_name {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0530",
                        format!(
                            "type parameter `{}` must be a single Pascal capital",
                            param.name
                        ),
                        param.span,
                    )
                    .with_help("use a name such as `T`, `K`, or `V`"),
                );
            }
            if !names.insert(param.name.clone()) {
                self.diagnostics.push(Diagnostic::new(
                    "E0530",
                    format!("type parameter `{}` is declared more than once", param.name),
                    param.span,
                ));
            }
            if param.default_type.is_some() {
                self.diagnostics.push(Diagnostic::unsupported_stage(
                    "E0534",
                    format!(
                        "{owner} cannot give type parameter `{}` a default type; decision 0105 reserves default type arguments beyond v1.0",
                        param.name
                    ),
                    param.span,
                ));
            }
            for constraint in &param.constraints {
                if Self::is_compiler_known_constraint(&constraint.name) {
                    self.check_compiler_known_constraint_declaration(param, constraint);
                } else {
                    self.diagnostics.push(Diagnostic::unsupported_stage(
                        "E0533",
                        format!(
                            "constraint `{constraint}` on type parameter `{}` requires Stage 35 user-defined interfaces",
                            param.name
                        ),
                        param.span,
                    ));
                }
            }
        }
        self.type_parameter_scopes.pop();
    }

    fn check_compiler_known_constraint_declaration(
        &mut self,
        param: &TypeParamDecl,
        constraint: &TypeRef,
    ) {
        let type_arguments = constraint.type_argument_count();
        let valid_arity = match constraint.name.as_str() {
            "Hashable" | "Displayable" => type_arguments == 0,
            "Comparable" | "Equatable" => type_arguments <= 1,
            _ => unreachable!("caller filters compiler-known constraints"),
        };
        if !valid_arity || constraint.has_value_arguments() {
            let expected = match constraint.name.as_str() {
                "Hashable" | "Displayable" => "no type arguments",
                "Comparable" | "Equatable" => "zero or one type argument",
                _ => unreachable!("caller filters compiler-known constraints"),
            };
            self.diagnostics.push(Diagnostic::new(
                "E0530",
                format!(
                    "constraint `{}` on type parameter `{}` expects {expected}",
                    constraint.name, param.name
                ),
                param.span,
            ));
        }
        for argument in constraint.type_arguments() {
            self.resolve_type_ref_in_position(argument, param.span, TypePosition::Value, None);
        }
    }

    fn is_compiler_known_constraint(name: &str) -> bool {
        matches!(
            name,
            "Comparable" | "Hashable" | "Equatable" | "Displayable"
        )
    }

    fn infer_return_borrow_signatures(&mut self) {
        let callables = self
            .program
            .items
            .iter()
            .flat_map(|item| match item {
                Item::Function(function) => vec![(function.clone(), None)],
                Item::Class(class) => class
                    .members
                    .iter()
                    .filter_map(|member| match member {
                        ClassMember::Method(method) => {
                            Some((method.clone(), Some(class.name.clone())))
                        }
                        ClassMember::Property(_) | ClassMember::Constant(_) => None,
                    })
                    .collect(),
                Item::Enum(_)
                | Item::Interface(_)
                | Item::Trait(_)
                | Item::Constant(_)
                | Item::Statement(_) => Vec::new(),
            })
            .collect::<Vec<_>>();

        for _ in 0..callables.len().max(1) {
            let mut changed = false;
            for (function, declaring_class) in &callables {
                let Some(signature) = self.function_signatures.get(&function.span).cloned() else {
                    continue;
                };
                if !self.type_can_return_borrow(signature.return_ty) {
                    continue;
                }
                let mut scopes = ScopeStack::new();
                for param in &signature.params {
                    let _ = scopes.declare(
                        param.name.clone(),
                        Binding::unresolved(param.writable, param.ty, param.ty, None, None),
                    );
                }
                let method_context = declaring_class.as_ref().map(|class_name| MethodContext {
                    class_name: class_name.clone(),
                    receiver_access: if function.is_static {
                        ReceiverAccess::Unavailable
                    } else if function.name == "__construct" {
                        ReceiverAccess::ConstructionRoot
                    } else if function.writable_this {
                        ReceiverAccess::Writable
                    } else {
                        ReceiverAccess::Readonly
                    },
                });
                let enclosing_type_params = declaring_class
                    .as_ref()
                    .and_then(|class_name| {
                        self.program.items.iter().find_map(|item| match item {
                            Item::Class(class) if &class.name == class_name => {
                                Some(class.type_params.clone())
                            }
                            _ => None,
                        })
                    })
                    .unwrap_or_default();
                let inferred = crate::ownership::function_return_borrow_in_context(
                    function,
                    &enclosing_type_params,
                    &mut |call| self.call_return_borrow(call, &scopes, method_context.as_ref()),
                );
                if inferred.is_some() && inferred != signature.return_borrow {
                    self.set_return_borrow(function, declaring_class.as_deref(), inferred);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn call_return_borrow(
        &mut self,
        call: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<ReturnBorrow> {
        match call {
            Expr::FunctionCall { name, .. } => {
                self.functions.get(name).and_then(|info| info.return_borrow)
            }
            Expr::MethodCall { object, method, .. } => {
                let object_ty = self.infer_expr_type(object, scopes, method_context);
                let object_ty = self.forwarded_access_payload_type(object_ty);
                if matches!(
                    (self.types.kind(object_ty), method.as_str()),
                    (TypeKind::Dictionary(_, _), "get")
                ) {
                    return Some(ReturnBorrow {
                        source: BorrowSource::Receiver,
                        writable: false,
                    });
                }
                self.expr_class_name(object, scopes, method_context)
                    .and_then(|class_name| self.classes.get(&class_name))
                    .and_then(|class| class.methods.get(method))
                    .and_then(|info| info.return_borrow)
            }
            Expr::StaticCall {
                qualifier, method, ..
            } => Self::static_qualifier_class_name(qualifier, method_context)
                .and_then(|class_name| self.classes.get(&class_name))
                .and_then(|class| class.methods.get(method))
                .and_then(|info| info.return_borrow),
            _ => None,
        }
    }

    fn set_return_borrow(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
        borrow: Option<ReturnBorrow>,
    ) {
        if let Some(signature) = self.function_signatures.get_mut(&function.span) {
            signature.return_borrow = borrow;
        }
        if let Some(class_name) = declaring_class {
            if let Some(signature) = self
                .classes
                .get_mut(class_name)
                .and_then(|class| class.methods.get_mut(&function.name))
            {
                signature.return_borrow = borrow;
            }
        } else if let Some(signature) = self.functions.get_mut(&function.name) {
            signature.return_borrow = borrow;
        }
    }

    fn resolve_param_infos(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) -> Vec<ParamInfo> {
        let mut params = Vec::new();
        let mut saw_optional = false;

        for param in &function.params {
            let ty = self.resolve_type_ref_with_class(&param.ty, param.span, declaring_class);
            let has_default = param.default.is_some();

            if param.take && param.writable {
                let span = param
                    .take_span
                    .zip(param.writable_span)
                    .map(|(take, writable)| take.merge(writable))
                    .unwrap_or(param.span);
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0467",
                        "a parameter cannot be both `take` and `writable`",
                        span,
                    )
                    .with_help(
                        "use `take` to give ownership to the callee, or `writable` for exclusive mutation without giving ownership",
                    ),
                );
            }

            if param.promoted_access.is_some() && self.type_is_move_type(ty) && !param.take {
                let diagnostic = Diagnostic::new(
                    "E0468",
                    format!(
                        "promoted move-type parameter `${}` must use `take`",
                        param.name
                    ),
                    param.span,
                );
                self.diagnostics
                    .push(if let Some(writable_span) = param.writable_span {
                        diagnostic
                            .with_help(
                                "promotion transfers ownership; replace `writable` with `take`",
                            )
                            .with_fix(writable_span, "take")
                    } else {
                        diagnostic
                        .with_help(
                            "promotion gives ownership directly to the new property; insert `take`",
                        )
                        .with_fix(param.ownership_modifier_insert, "take ")
                    });
            }

            if !has_default && saw_optional {
                self.diagnostics.push(Diagnostic::new(
                    "E0410",
                    format!(
                        "required parameter `${}` cannot follow an optional parameter",
                        param.name
                    ),
                    param.span,
                ));
            }

            if has_default {
                saw_optional = true;
            }

            params.push(ParamInfo {
                name: param.name.clone(),
                ty,
                take: param.take,
                writable: param.writable,
                has_default,
            });
        }

        params
    }

    fn resolve_function_return_type(
        &mut self,
        function: &FunctionDecl,
        declaring_class: Option<&str>,
    ) -> TypeId {
        function
            .return_type
            .as_ref()
            .map(|return_type| {
                self.resolve_type_ref_in_position(
                    return_type,
                    function.span,
                    TypePosition::Return,
                    declaring_class,
                )
            })
            .unwrap_or_else(|| self.types.unknown())
    }

    fn infer_unannotated_move_return_signatures(&mut self) {
        let max_iterations = self.move_return_inference_signature_count();

        for _ in 0..max_iterations {
            let mut changed = false;

            for item in &self.program.items {
                match item {
                    Item::Function(function) => {
                        changed |= self.update_function_move_return_signature(function);
                    }
                    Item::Class(class_decl) => {
                        for member in &class_decl.members {
                            let ClassMember::Method(method) = member else {
                                continue;
                            };
                            changed |=
                                self.update_method_move_return_signature(&class_decl.name, method);
                        }
                    }
                    Item::Enum(_)
                    | Item::Interface(_)
                    | Item::Trait(_)
                    | Item::Constant(_)
                    | Item::Statement(_) => {}
                }
            }

            if !changed {
                break;
            }
        }
    }

    fn move_return_inference_signature_count(&self) -> usize {
        self.program
            .items
            .iter()
            .map(|item| match item {
                Item::Function(function) if function.return_type.is_none() => 1,
                Item::Class(class_decl) => class_decl
                    .members
                    .iter()
                    .filter(|member| match member {
                        ClassMember::Method(method) => {
                            method.return_type.is_none()
                                && LifecycleMethod::from_method_name(&method.name).is_none()
                        }
                        ClassMember::Property(_) | ClassMember::Constant(_) => false,
                    })
                    .count(),
                _ => 0,
            })
            .sum::<usize>()
            .max(1)
    }

    fn update_function_move_return_signature(&mut self, function: &FunctionDecl) -> bool {
        if function.return_type.is_some() {
            return false;
        }

        let Some(signature) = self.function_signatures.get(&function.span).cloned() else {
            return false;
        };
        let inferred = self.infer_unannotated_move_return_type(function, &signature.params, None);

        if !self.type_is_move_type(inferred) || signature.return_ty == inferred {
            return false;
        }

        if let Some(signature) = self.function_signatures.get_mut(&function.span) {
            signature.return_ty = inferred;
        }
        if let Some(function_info) = self.functions.get_mut(&function.name) {
            function_info.return_ty = inferred;
        }
        true
    }

    fn update_method_move_return_signature(
        &mut self,
        class_name: &str,
        method: &FunctionDecl,
    ) -> bool {
        if method.return_type.is_some() || LifecycleMethod::from_method_name(&method.name).is_some()
        {
            return false;
        }

        let Some(signature) = self.function_signatures.get(&method.span).cloned() else {
            return false;
        };
        let method_context = MethodContext {
            class_name: class_name.to_string(),
            receiver_access: if method.name == "__construct" {
                ReceiverAccess::ConstructionRoot
            } else if method.writable_this {
                ReceiverAccess::Writable
            } else {
                ReceiverAccess::Readonly
            },
        };
        let inferred = self.infer_unannotated_move_return_type(
            method,
            &signature.params,
            Some(&method_context),
        );

        if !self.type_is_move_type(inferred) || signature.return_ty == inferred {
            return false;
        }

        if let Some(signature) = self.function_signatures.get_mut(&method.span) {
            signature.return_ty = inferred;
        }
        if let Some(method_info) = self
            .classes
            .get_mut(class_name)
            .and_then(|class_info| class_info.methods.get_mut(&method.name))
        {
            method_info.return_ty = inferred;
        }
        true
    }

    fn infer_unannotated_move_return_type(
        &mut self,
        function: &FunctionDecl,
        params: &[ParamInfo],
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let mut scopes = ScopeStack::new();
        for param in params {
            let _ = scopes.declare(
                param.name.clone(),
                Binding::unresolved(false, param.ty, param.ty, None, None),
            );
        }

        self.infer_move_return_from_block(&function.body, &mut scopes, method_context)
            .unwrap_or_else(|| self.types.unknown())
    }

    fn infer_move_return_from_statements(
        &mut self,
        statements: &[Stmt],
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let mut inferred = None;

        for statement in statements {
            let statement_ty =
                self.infer_move_return_from_statement(statement, scopes, method_context);
            inferred = self.merge_optional_inferred_return_types(inferred, statement_ty);

            if !crate::return_analysis::statement_falls_through(statement) {
                break;
            }
        }

        inferred
    }

    fn infer_move_return_from_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        match statement {
            Stmt::VarDecl(decl) => {
                let ty = self.infer_local_declaration_type(
                    decl.ty.as_ref(),
                    &decl.initializer,
                    scopes,
                    method_context,
                );
                for binding in &decl.bindings {
                    let _ = scopes.declare(
                        binding.name.clone(),
                        Binding::unresolved(decl.writable, ty, ty, None, None),
                    );
                }
                None
            }
            Stmt::Assignment(assignment) => {
                self.infer_move_return_from_assignment(assignment, scopes, method_context);
                None
            }
            Stmt::Return {
                expr: Some(expr), ..
            } => Some(self.infer_expr_type(expr, scopes, method_context)),
            Stmt::If(if_stmt) => {
                self.infer_move_return_from_if_statement(if_stmt, scopes, method_context)
            }
            Stmt::While(while_stmt) => {
                self.infer_move_return_from_block(&while_stmt.body, scopes, method_context)
            }
            Stmt::For(for_stmt) => {
                scopes.push();
                if let Some(initializer) = &for_stmt.initializer {
                    self.infer_move_return_from_for_initializer(
                        initializer,
                        scopes,
                        method_context,
                    );
                }
                let result =
                    self.infer_move_return_from_block(&for_stmt.body, scopes, method_context);
                if let Some(increment) = &for_stmt.increment {
                    self.infer_move_return_from_for_increment(increment, scopes, method_context);
                }
                scopes.pop();
                result
            }
            Stmt::Foreach(foreach) => {
                self.infer_move_return_from_foreach(foreach, scopes, method_context)
            }
            _ => None,
        }
    }

    fn infer_move_return_from_if_statement(
        &mut self,
        if_stmt: &IfStmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let incoming_scopes = scopes.clone();
        let mut falling_through_scopes = Vec::new();

        let mut then_scopes = incoming_scopes.clone();
        let mut inferred = self.infer_move_return_from_block(
            &if_stmt.then_block,
            &mut then_scopes,
            method_context,
        );
        if crate::return_analysis::block_falls_through(&if_stmt.then_block) {
            falling_through_scopes.push(then_scopes);
        }

        if let Some(branch) = &if_stmt.else_branch {
            let mut else_scopes = incoming_scopes;
            let branch_ty =
                self.infer_move_return_from_else_branch(branch, &mut else_scopes, method_context);
            inferred = self.merge_optional_inferred_return_types(inferred, branch_ty);
            if crate::return_analysis::else_branch_falls_through(branch) {
                falling_through_scopes.push(else_scopes);
            }
        } else {
            falling_through_scopes.push(incoming_scopes);
        }

        scopes.replace_types_from_branches(&falling_through_scopes, |left, right| {
            self.merge_inferred_binding_type(left, right)
        });

        inferred
    }
    fn infer_move_return_from_block(
        &mut self,
        block: &Block,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        scopes.push();
        let inferred =
            self.infer_move_return_from_statements(&block.statements, scopes, method_context);
        scopes.pop();
        inferred
    }

    fn infer_move_return_from_else_branch(
        &mut self,
        branch: &ElseBranch,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        match branch {
            ElseBranch::If(if_stmt) => self.infer_move_return_from_statement(
                &Stmt::If((**if_stmt).clone()),
                scopes,
                method_context,
            ),
            ElseBranch::Block(block) => {
                self.infer_move_return_from_block(block, scopes, method_context)
            }
        }
    }

    fn infer_move_return_from_for_initializer(
        &mut self,
        initializer: &ForInitializer,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match initializer {
            ForInitializer::VarDecl(decl) => {
                let ty = self.infer_local_declaration_type(
                    decl.ty.as_ref(),
                    &decl.initializer,
                    scopes,
                    method_context,
                );
                for binding in &decl.bindings {
                    let _ = scopes.declare(
                        binding.name.clone(),
                        Binding::unresolved(decl.writable, ty, ty, None, None),
                    );
                }
            }
            ForInitializer::Assignment(assignment) => {
                self.infer_move_return_from_assignment(assignment, scopes, method_context);
            }
        }
    }

    fn infer_move_return_from_for_increment(
        &mut self,
        increment: &ForIncrement,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if let ForIncrement::Assignment(assignment) = increment {
            self.infer_move_return_from_assignment(assignment, scopes, method_context);
        }
    }

    fn infer_move_return_from_assignment(
        &mut self,
        assignment: &Assignment,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if !matches!(assignment.op, AssignOp::Assign) {
            return;
        }

        if let Some(name) = Self::assignment_target_variable_name(&assignment.target) {
            let ty = self.infer_expr_type(&assignment.value, scopes, method_context);
            if let Some(binding) = scopes.lookup_mut(name) {
                binding.ty = self.merge_inferred_binding_type(binding.ty, ty);
            }
        }
    }

    fn assignment_target_variable_name(target: &Expr) -> Option<&str> {
        match target {
            Expr::Grouped { expr, .. } => Self::assignment_target_variable_name(expr),
            Expr::Variable { name, .. } => Some(name),
            _ => None,
        }
    }

    fn infer_move_return_from_foreach(
        &mut self,
        foreach: &ForeachStmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        scopes.push();
        let (inferred_key, inferred_value) =
            self.infer_foreach_binding_types(foreach, scopes, method_context);
        if let Some(key) = &foreach.key {
            let ty = key
                .ty
                .as_ref()
                .map(|ty| self.resolve_type_ref_for_return_inference(ty))
                .unwrap_or(inferred_key);
            let _ = scopes.declare(
                key.name.clone(),
                Binding::unresolved(false, ty, ty, None, None),
            );
        }

        let value_ty = foreach
            .value
            .ty
            .as_ref()
            .map(|ty| self.resolve_type_ref_for_return_inference(ty))
            .unwrap_or(inferred_value);
        let _ = scopes.declare(
            foreach.value.name.clone(),
            Binding::unresolved(false, value_ty, value_ty, None, None),
        );

        let inferred = self.infer_move_return_from_statements(
            &foreach.body.statements,
            scopes,
            method_context,
        );
        scopes.pop();
        inferred
    }

    fn merge_optional_inferred_return_types(
        &mut self,
        current: Option<TypeId>,
        next: Option<TypeId>,
    ) -> Option<TypeId> {
        let next = next?;
        if matches!(self.types.kind(next), TypeKind::Unknown) {
            return current;
        }
        Some(match current {
            Some(current) if matches!(self.types.kind(current), TypeKind::Unknown) => next,
            Some(current) => self.merge_inferred_return_types(current, next),
            None => next,
        })
    }

    fn merge_inferred_return_types(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if left == right {
            return left;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::List(left), TypeKind::List(right)) => {
                let element = self.merge_inferred_return_types(left, right);
                self.types.intern(TypeKind::List(element))
            }
            (TypeKind::TypedArray(left), TypeKind::TypedArray(right)) => {
                let element = self.merge_inferred_return_types(left, right);
                self.types.intern(TypeKind::TypedArray(element))
            }
            (
                TypeKind::Dictionary(left_key, left_value),
                TypeKind::Dictionary(right_key, right_value),
            ) => {
                let key = self.merge_inferred_return_types(left_key, right_key);
                let value = self.merge_inferred_return_types(left_value, right_value);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            (TypeKind::Set(left), TypeKind::Set(right)) => {
                let element = self.merge_inferred_return_types(left, right);
                self.types.intern(TypeKind::Set(element))
            }
            _ => self.types.intern(TypeKind::Mixed),
        }
    }

    fn infer_local_declaration_type(
        &mut self,
        annotation: Option<&TypeRef>,
        initializer: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let initializer_ty = self.infer_expr_type(initializer, scopes, method_context);
        annotation.map_or(initializer_ty, |ty| {
            self.resolve_type_ref_for_return_inference(ty)
        })
    }

    fn merge_inferred_binding_type(&mut self, current: TypeId, next: TypeId) -> TypeId {
        if self.type_contains_mixed(current) {
            self.merge_inferred_return_types(current, next)
        } else {
            next
        }
    }

    fn infer_foreach_binding_types(
        &mut self,
        foreach: &ForeachStmt,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> (TypeId, TypeId) {
        let unknown = self.types.unknown();
        if Self::is_grouped_range_expr(&foreach.iterable) {
            let integer = self
                .range_integer_type(&foreach.iterable, scopes, method_context)
                .unwrap_or(IntegerType::Int64);
            return (unknown, self.types.intern(TypeKind::Integer(integer)));
        }

        if let Some((dictionary, projection)) =
            Self::dictionary_foreach_projection(&foreach.iterable)
        {
            let dictionary_ty = self.infer_expr_type(dictionary, scopes, method_context);
            if let TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) =
                self.types.kind(dictionary_ty).clone()
            {
                return (
                    unknown,
                    match projection {
                        DictionaryProjection::Keys => key,
                        DictionaryProjection::Values => value,
                    },
                );
            }
        }

        let iterable_ty = self.infer_expr_type(&foreach.iterable, scopes, method_context);
        match self.types.kind(iterable_ty).clone() {
            TypeKind::List(value)
            | TypeKind::Set(value)
            | TypeKind::SortedSet(value)
            | TypeKind::Deque(value) => (
                self.types.intern(TypeKind::Integer(IntegerType::Int64)),
                value,
            ),
            TypeKind::TypedArray(value) => (
                self.types.intern(TypeKind::Integer(IntegerType::Int64)),
                value,
            ),
            TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) => {
                (key, value)
            }
            TypeKind::Mixed => (unknown, self.types.intern(TypeKind::Mixed)),
            _ => (unknown, unknown),
        }
    }

    fn dictionary_foreach_projection(expr: &Expr) -> Option<(&Expr, DictionaryProjection)> {
        match expr {
            Expr::Grouped { expr, .. } => Self::dictionary_foreach_projection(expr),
            Expr::PropertyAccess {
                object,
                property,
                null_safe: false,
                ..
            } => match property.as_str() {
                "keys" => Some((object, DictionaryProjection::Keys)),
                "values" => Some((object, DictionaryProjection::Values)),
                _ => None,
            },
            _ => None,
        }
    }

    fn resolve_type_ref_for_return_inference(&mut self, ty: &TypeRef) -> TypeId {
        if let Some(grouped) = &ty.grouped {
            let mut inner = grouped.inner.clone();
            inner.nullable |= ty.nullable;
            return self.resolve_type_ref_for_return_inference(&inner);
        }
        if ty.nullable {
            let mut inner = ty.clone();
            inner.nullable = false;
            let inner = self.resolve_type_ref_for_return_inference(&inner);
            return if matches!(self.types.kind(inner), TypeKind::Unknown | TypeKind::Void) {
                self.types.unknown()
            } else {
                self.types.intern(TypeKind::Nullable(inner))
            };
        }
        if ty.arguments.is_empty() {
            if let Some(integer) = IntegerType::from_source_name(&ty.name) {
                return self.types.intern(TypeKind::Integer(integer));
            }
            if let Some(float) = FloatType::from_source_name(&ty.name) {
                return self.types.intern(TypeKind::Float(float));
            }
        }
        match ty.name.as_str() {
            "void" if ty.arguments.is_empty() => self.types.intern(TypeKind::Void),
            "string" if ty.arguments.is_empty() => self.types.intern(TypeKind::String),
            "bool" if ty.arguments.is_empty() => self.types.intern(TypeKind::Bool),
            "mixed" if ty.arguments.is_empty() => self.types.intern(TypeKind::Mixed),
            "Error" if ty.arguments.is_empty() => self.types.intern(TypeKind::Error),
            "[]" if ty.type_argument_count() == 1 && !ty.has_value_arguments() => {
                let element =
                    self.resolve_type_ref_for_return_inference(ty.type_argument(0).unwrap());
                self.types.intern(TypeKind::TypedArray(element))
            }
            "List" if ty.type_argument_count() == 1 && !ty.has_value_arguments() => {
                let element =
                    self.resolve_type_ref_for_return_inference(ty.type_argument(0).unwrap());
                self.types.intern(TypeKind::List(element))
            }
            "Dictionary" if ty.type_argument_count() == 2 && !ty.has_value_arguments() => {
                let key = self.resolve_type_ref_for_return_inference(ty.type_argument(0).unwrap());
                let value =
                    self.resolve_type_ref_for_return_inference(ty.type_argument(1).unwrap());
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            "Set" if ty.type_argument_count() == 1 && !ty.has_value_arguments() => {
                let element =
                    self.resolve_type_ref_for_return_inference(ty.type_argument(0).unwrap());
                self.types.intern(TypeKind::Set(element))
            }
            name if self.classes.contains_key(name) && !ty.has_value_arguments() => {
                let arguments = ty
                    .type_arguments()
                    .map(|argument| self.resolve_type_ref_for_return_inference(argument))
                    .collect();
                self.types
                    .intern(TypeKind::Class(ClassType::new(name, arguments)))
            }
            _ => self.types.unknown(),
        }
    }

    fn check_class(&mut self, class_decl: &ClassDecl) {
        self.type_parameter_scopes
            .push(type_parameter_scope(&class_decl.type_params));
        if class_decl.parent.is_some() {
            self.diagnostics.push(Diagnostic::new(
                "E0476",
                "class inheritance is accepted syntax but `extends` semantics are not available in this compiler version",
                class_decl.parent_span.unwrap_or(class_decl.span),
            ));
        }
        for member in &class_decl.members {
            match member {
                ClassMember::Property(property) => {
                    if property.is_static {
                        if let Some(initializer) = &property.initializer {
                            self.check_nonthrowing_initializer(
                                initializer,
                                Some(&class_decl.name),
                                "E0634",
                                "Static Initializer Cannot Throw",
                            );
                        }
                    }
                }
                ClassMember::Constant(constant) => {
                    self.check_nonthrowing_initializer(
                        &constant.initializer,
                        Some(&class_decl.name),
                        "E0633",
                        "Constant Initializer Cannot Throw",
                    );
                }
                ClassMember::Method(_) => {}
            }
        }
        let has_constructor = class_decl.members.iter().any(
            |member| matches!(member, ClassMember::Method(method) if method.name == "__construct"),
        );
        if !has_constructor {
            if let Some(effects) = self.class_initializer_effects.get(&class_decl.name) {
                let required = effects.ordered.iter().any(|effect| {
                    !crate::checked_effects::is_ambient_io_effect(&self.types.resolved(*effect))
                });
                if required {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0635",
                            format!(
                                "implicit constructor for `{}` cannot hide a throwing property initializer",
                                class_decl.name
                            ),
                            class_decl.span,
                        )
                        .with_title("Implicit Constructor Cannot Hide A Throwing Initializer")
                        .with_help("declare `__construct` explicitly and list the initializer errors in its `throws` clause"),
                    );
                }
            }
        }
        for member in &class_decl.members {
            let ClassMember::Method(method) = member else {
                continue;
            };
            let lifecycle = LifecycleMethod::from_method_name(&method.name);
            self.check_function(
                method,
                Some(MethodContext {
                    class_name: class_decl.name.clone(),
                    receiver_access: if method.is_static {
                        ReceiverAccess::Unavailable
                    } else if method.name == "__construct" {
                        ReceiverAccess::ConstructionRoot
                    } else if method.writable_this && lifecycle.is_none() {
                        ReceiverAccess::Writable
                    } else {
                        ReceiverAccess::Readonly
                    },
                }),
            );
        }
        self.type_parameter_scopes.pop();
    }

    fn check_constant_initializer(&mut self, initializer: &Expr, class_name: Option<&str>) {
        self.check_nonthrowing_initializer(
            initializer,
            class_name,
            "E0633",
            "Constant Initializer Cannot Throw",
        );
    }

    fn check_nonthrowing_initializer(
        &mut self,
        initializer: &Expr,
        class_name: Option<&str>,
        code: &'static str,
        title: &'static str,
    ) {
        let scopes = ScopeStack::new();
        let context = class_name.map(|class_name| MethodContext {
            class_name: class_name.to_string(),
            receiver_access: ReceiverAccess::Unavailable,
        });
        self.effect_scopes.push(CheckedEffectSet::default());
        self.check_expr(initializer, &scopes, context.as_ref());
        let effects = self.effect_scopes.pop().expect("initializer effect scope");
        if !effects.ordered.is_empty() {
            self.diagnostics.push(
                Diagnostic::new(
                    code,
                    "compile-time and static initialization cannot propagate checked errors",
                    initializer.span(),
                )
                .with_title(title)
                .with_help(
                    "move the throwing operation into an explicitly declared runtime callable",
                ),
            );
        }
    }

    fn check_property_initializer(&mut self, class_name: &str, property: &PropertyDecl) {
        let Some(initializer) = &property.initializer else {
            return;
        };

        let scopes = ScopeStack::new();
        let initializer_context = MethodContext {
            class_name: class_name.to_string(),
            receiver_access: ReceiverAccess::Unavailable,
        };
        let target_ty = self
            .classes
            .get(class_name)
            .and_then(|class_info| class_info.properties.get(&property.name))
            .map(|property| property.ty)
            .unwrap_or_else(|| {
                self.resolve_type_ref_with_class(&property.ty, property.span, Some(class_name))
            });
        self.record_expected_expression_type(initializer, target_ty);
        self.effect_scopes.push(CheckedEffectSet::default());
        self.check_expr(initializer, &scopes, Some(&initializer_context));
        let effects = self
            .effect_scopes
            .pop()
            .expect("property initializer effect scope");
        self.class_initializer_effects
            .entry(class_name.to_string())
            .or_default()
            .extend(effects.ordered);
        self.check_expr_assignable(
            target_ty,
            initializer,
            &scopes,
            Some(&initializer_context),
            AssignmentDestination::Property {
                class_name: class_name.to_string(),
                name: property.name.clone(),
            },
        );
    }

    fn check_instance_property_initializers(&mut self) {
        let classes = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(class.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for class in classes {
            self.type_parameter_scopes
                .push(type_parameter_scope(&class.type_params));
            for member in &class.members {
                if let ClassMember::Property(property) = member {
                    if !property.is_static {
                        self.check_property_initializer(&class.name, property);
                    }
                }
            }
            self.type_parameter_scopes.pop();
        }
    }

    fn declare_property(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        property: &PropertyDecl,
    ) {
        if !self.declare_member_name(
            info,
            class_name,
            &property.name,
            MemberKind::InstanceProperty,
            property.span,
        ) {
            return;
        }

        let ty = self.resolve_type_ref_with_class(&property.ty, property.span, Some(class_name));
        info.properties.insert(
            property.name.clone(),
            PropertyInfo {
                access: property.access,
                writable: property.writable,
                ty,
                init_state: if property.initializer.is_some() {
                    PropertyInitState::HasInitializer
                } else {
                    PropertyInitState::Uninitialized
                },
                declaration_span: property.span,
            },
        );
    }

    fn declare_static_property(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        property: &PropertyDecl,
    ) {
        if !self.declare_member_name(
            info,
            class_name,
            &property.name,
            MemberKind::StaticProperty,
            property.span,
        ) {
            return;
        }
        let ty = self.resolve_type_ref_with_class(&property.ty, property.span, Some(class_name));
        if self.type_is_move_type(ty) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0486",
                    format!(
                        "static property `{class_name}::{}` cannot use owned type `{}`",
                        property.name,
                        self.types.display(ty)
                    ),
                    property.span,
                )
                .with_help(
                    "owned-static lifetime and concurrency rules are deferred pending Sendable/Shareable design",
                ),
            );
        }
        info.static_properties.insert(
            property.name.clone(),
            StaticPropertyInfo {
                access: property.access,
                writable: property.writable,
                ty,
            },
        );
    }

    fn declare_class_constant(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        constant: &ConstDecl,
    ) {
        if !self.declare_member_name(
            info,
            class_name,
            &constant.name,
            MemberKind::Constant,
            constant.span,
        ) {
            return;
        }
        let key = crate::const_eval::ConstKey::Class {
            class_name: class_name.to_string(),
            name: constant.name.clone(),
        };
        let ty = if let Some(value) = self.const_evaluation.values.get(&key) {
            self.const_type_id(value.ty)
        } else {
            let ty_ref = constant.ty.clone().unwrap_or_else(TypeRef::unknown);
            self.resolve_type_ref_with_class(&ty_ref, constant.span, Some(class_name))
        };
        info.constants.insert(
            constant.name.clone(),
            ConstantInfo {
                access: constant.access,
                ty,
            },
        );
    }

    fn declare_promoted_property(&mut self, info: &mut ClassInfo, class_name: &str, param: &Param) {
        if !self.declare_member_name(
            info,
            class_name,
            &param.name,
            MemberKind::PromotedProperty,
            param.span,
        ) {
            return;
        }

        let ty = self.resolve_type_ref_with_class(&param.ty, param.span, Some(class_name));
        info.properties.insert(
            param.name.clone(),
            PropertyInfo {
                access: param.promoted_access.unwrap_or(MemberAccess::External),
                writable: param.writable,
                ty,
                init_state: PropertyInitState::PromotedParameter,
                declaration_span: param.span,
            },
        );
    }

    fn declare_member_name(
        &mut self,
        info: &mut ClassInfo,
        class_name: &str,
        name: &str,
        kind: MemberKind,
        span: Span,
    ) -> bool {
        if let Some(original) = info.members.get(name) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0481",
                    format!(
                        "class `{class_name}` cannot declare {} `{name}` because that name is already used by a previous {}",
                        kind.description(),
                        original.kind.description()
                    ),
                    span,
                )
                .with_related(
                    original.span,
                    format!("original {} `{name}` is declared here", original.kind.description()),
                ),
            );
            return false;
        }

        info.members
            .insert(name.to_string(), MemberDeclaration { kind, span });
        true
    }

    fn declare_binding(
        &mut self,
        scopes: &mut ScopeStack,
        name: String,
        mut binding: Binding,
        span: Span,
        kind: BindingKind,
        ownership: BindingOwnership,
    ) {
        let key = (
            span.start,
            span.end,
            kind,
            self.current_lexical_owner,
            name.clone(),
        );
        let id = if let Some(id) = self.binding_ids.get(&key) {
            *id
        } else {
            let id = BindingId(self.next_binding_id);
            self.next_binding_id += 1;
            self.binding_ids.insert(key, id);
            id
        };
        binding.id = id;
        binding.kind = kind;
        binding.ownership = ownership;
        binding.owner = self.current_lexical_owner;
        let source_span = (kind != BindingKind::MethodReceiver).then_some(span);
        self.binding_resolution.declarations_by_id.insert(
            id,
            BindingDeclaration {
                id,
                name: name.clone(),
                span: source_span,
                kind,
                writable: binding.writable,
                ownership,
                owner: self.current_lexical_owner,
                source_type: Some(self.types.resolved(binding.declared_ty)),
            },
        );
        if let Some(source_span) = source_span {
            self.binding_resolution
                .declaration_by_span
                .insert(source_span, id);
        }
        if !scopes.declare(name.clone(), binding) {
            self.diagnostics.push(Diagnostic::new(
                "E0103",
                format!("variable `${name}` is already declared in this scope"),
                span,
            ));
        }
    }

    /// Decision 0099: `main` takes either no parameters or exactly one
    /// `List<string>` holding the program's arguments. There is no `argc` — the
    /// list carries its own length — and the parameter is an ordinary readonly
    /// borrow, because the entry glue owns the list and releases it after `main`
    /// returns.
    fn check_entry_parameters(&mut self, function: &FunctionDecl) {
        if function.params.is_empty() {
            return;
        }

        if function.params.len() > 1 {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0526",
                    format!(
                        "process entrypoint `main` takes at most one parameter, got {}",
                        function.params.len()
                    ),
                    function.span,
                )
                .with_help(
                    "declare `main(List<string> $args)`; the list carries its own length, so there is no separate argument count",
                ),
            );
            return;
        }

        let param = &function.params[0];
        if param.take || param.writable {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0526",
                    "the `main` argument list cannot be `writable` or `take`",
                    param.span,
                )
                .with_help("declare it as `main(List<string> $args)`; the program arguments are borrowed for the duration of `main`"),
            );
            return;
        }

        let expected = {
            let string = self.types.intern(TypeKind::String);
            self.types.intern(TypeKind::List(string))
        };
        let actual = self.resolve_type_ref(&param.ty, param.span);
        if actual != expected && !self.is_unknown_type(actual) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0526",
                    format!(
                        "process entrypoint `main` expects `List<string>` for its argument list, got `{}`",
                        self.types.display(actual)
                    ),
                    param.span,
                )
                .with_help("declare `main(List<string> $args)`"),
            );
        }
    }

    fn check_function(&mut self, function: &FunctionDecl, method_context: Option<MethodContext>) {
        let previous_callable = self.current_callable.replace(function.span);
        let previous_owner = std::mem::replace(
            &mut self.current_lexical_owner,
            LexicalOwner::Callable(function.span.start),
        );
        self.binding_resolution
            .lexical_parents
            .insert(self.current_lexical_owner, previous_owner);
        self.type_parameter_scopes
            .push(type_parameter_scope(&function.type_params));
        let mut scopes = ScopeStack::new();
        let signature = self.current_function_signature(function);
        if let Some(context) = method_context.as_ref().filter(|_| !function.is_static) {
            let receiver_ty = self.symbolic_class_type(&context.class_name);
            self.declare_binding(
                &mut scopes,
                "this".to_string(),
                Binding::unresolved(
                    context.receiver_access.is_writable(),
                    receiver_ty,
                    receiver_ty,
                    None,
                    None,
                ),
                Span::new(function.span.start, function.span.start),
                BindingKind::MethodReceiver,
                if context.receiver_access.is_writable() {
                    BindingOwnership::WritableBorrow
                } else {
                    BindingOwnership::ReadonlyBorrow
                },
            );
        }
        if function.return_type.is_none()
            && LifecycleMethod::from_method_name(&function.name).is_none()
            && function.throws.is_some()
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0617",
                    format!("function `{}` must declare its return type", function.name),
                    function.span,
                )
                .with_title("Function Return Type Is Required")
                .with_help(
                    "write an explicit return type, including `: void` when no value is returned",
                ),
            );
        }
        let mut body_effects = CheckedEffectSet::default();
        if function.name == "__construct" {
            if let Some(class_name) = method_context.as_ref().map(|context| &context.class_name) {
                if let Some(initializer_effects) = self.class_initializer_effects.get(class_name) {
                    body_effects.extend(initializer_effects.ordered.iter().copied());
                }
            }
        }
        self.effect_scopes.push(body_effects);
        if method_context.is_none()
            && crate::names::source_name_is(&function.name, "main")
            && !matches!(
                self.types.kind(signature.return_ty),
                TypeKind::Integer(IntegerType::Int64) | TypeKind::Void | TypeKind::Unknown
            )
        {
            let actual = self.types.display(signature.return_ty);
            self.diagnostics.push(
                Diagnostic::new(
                    "E0442",
                    format!(
                        "process entrypoint `main` cannot return `{actual}`; expected `void`, `int`, or `int64`"
                    ),
                    function.span,
                )
                .with_help("helper functions may return fixed-width integers, floats, or bool"),
            );
        }

        if method_context.is_none() && crate::names::source_name_is(&function.name, "main") {
            self.check_entry_parameters(function);
        }
        let return_context = self.return_context_for_function(function, method_context.as_ref());
        for (parameter_index, (param, param_info)) in function
            .params
            .iter()
            .zip(signature.params.iter())
            .enumerate()
        {
            let ty = param_info.ty;
            if let Some(default) = &param.default {
                let default_context = method_context.as_ref().map(|context| MethodContext {
                    class_name: context.class_name.clone(),
                    receiver_access: ReceiverAccess::Unavailable,
                });
                let default_context = default_context.as_ref();

                self.record_expected_expression_type(default, ty);
                self.check_expr(default, &scopes, default_context);
                self.check_expr_assignable(
                    ty,
                    default,
                    &scopes,
                    default_context,
                    AssignmentDestination::Parameter {
                        name: param.name.clone(),
                    },
                );
                self.check_parameter_default_support(
                    function,
                    parameter_index,
                    param,
                    ty,
                    method_context.as_ref(),
                );
            }
            self.declare_binding(
                &mut scopes,
                param.name.clone(),
                Binding::unresolved(param.writable, ty, ty, None, None),
                param.span,
                if method_context.is_some() {
                    BindingKind::MethodParameter
                } else {
                    BindingKind::FunctionParameter
                },
                if param.take {
                    BindingOwnership::Owned
                } else if param.writable {
                    BindingOwnership::WritableBorrow
                } else {
                    BindingOwnership::ReadonlyBorrow
                },
            );
        }
        let mut constructor_init_context = method_context.as_ref().and_then(|context| {
            (function.name == "__construct").then(|| ConstructorInitContext {
                class_name: context.class_name.clone(),
                repeatable_body: false,
            })
        });

        self.check_block(
            &function.body,
            &mut scopes,
            method_context.as_ref(),
            constructor_init_context.as_mut(),
            Some(&return_context),
            0,
        );
        self.check_missing_final_return(function, &return_context);
        let body_effects = self.effect_scopes.pop().expect("callable effect scope");
        self.callable_observed_checked_effects.insert(
            function.span,
            body_effects
                .ordered
                .iter()
                .map(|effect| self.types.resolved(*effect))
                .collect(),
        );
        if function.throws.is_none() && self.is_accepted_program_entrypoint(function) {
            self.set_inferred_entrypoint_effects(function, &body_effects);
        } else {
            self.check_callable_effect_contract(
                function,
                &signature.checked_effects,
                &body_effects,
            );
        }
        self.type_parameter_scopes.pop();
        self.current_callable = previous_callable;
        self.current_lexical_owner = previous_owner;
    }

    fn is_accepted_program_entrypoint(&self, function: &FunctionDecl) -> bool {
        let Some(signature) = self.function_signatures.get(&function.span) else {
            return false;
        };
        if !crate::names::source_name_is(&function.name, "main")
            || !function.type_params.is_empty()
            || self
                .functions
                .get(&function.name)
                .is_none_or(|entry| entry.declaration != function.span)
            || !matches!(
                self.types.kind(signature.return_ty),
                TypeKind::Integer(IntegerType::Int64) | TypeKind::Void
            )
        {
            return false;
        }

        match (function.params.as_slice(), signature.params.as_slice()) {
            ([], []) => true,
            ([source], [resolved]) if !source.take && !source.writable => {
                matches!(
                    self.types.kind(resolved.ty),
                    TypeKind::List(element)
                        if matches!(self.types.kind(*element), TypeKind::String)
                )
            }
            _ => false,
        }
    }

    fn set_inferred_entrypoint_effects(
        &mut self,
        function: &FunctionDecl,
        observed: &CheckedEffectSet,
    ) {
        let seeded = self
            .function_signatures
            .get(&function.span)
            .map(|signature| signature.checked_effects.clone())
            .unwrap_or_default();
        let mut effects = observed.ordered.clone();
        extend_type_ids(&mut effects, seeded);
        let resolved = effects
            .iter()
            .map(|effect| self.types.resolved(*effect))
            .collect::<Vec<_>>();
        self.callable_effective_checked_effects
            .insert(function.span, resolved.clone());
        if let Some(signature) = self.function_signatures.get_mut(&function.span) {
            signature.checked_effects = effects.clone();
        }
        if let Some(signature) = self.functions.get_mut(&function.name) {
            if signature.declaration == function.span {
                signature.checked_effects = effects;
            }
        }

        // Recursive entrypoint calls were resolved before the first inference
        // pass completed. Refresh those exact sites so ownership, MIR lowering,
        // and catch routing see the same effective contract as later callers.
        for (span, target) in &self.call_targets {
            if matches!(target, CallableTarget::Function { name } if name == &function.name) {
                let site = self.checked_effect_sites.entry(*span).or_default();
                for effect in &resolved {
                    if !site.contains(effect) {
                        site.push(effect.clone());
                    }
                }
            }
        }
    }

    fn check_callable_effect_contract(
        &mut self,
        function: &FunctionDecl,
        declared: &[TypeId],
        observed: &CheckedEffectSet,
    ) {
        let uncovered = observed
            .ordered
            .iter()
            .copied()
            .filter(|effect| {
                function.name == "__destruct"
                    || !crate::checked_effects::is_ambient_io_effect(&self.types.resolved(*effect))
            })
            // The effective signature includes inferred ambient transport for
            // ABI lowering. Destructors still cannot let any checked Error
            // escape, so that runtime profile must never count as an authored
            // declaration satisfying the lifecycle boundary.
            .filter(|effect| {
                function.name == "__destruct"
                    || !Self::checked_error_type_covers(&self.types, declared, *effect)
            })
            .collect::<Vec<_>>();
        if uncovered.is_empty() {
            return;
        }

        let displays = uncovered
            .iter()
            .map(|effect| self.types.display(*effect))
            .collect::<Vec<_>>();
        let listed = displays
            .iter()
            .map(|display| format!("`{display}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let (message, title, help) = if function.name == "__destruct" {
            (
                format!("destructor `__destruct` allows checked errors {listed} to escape"),
                "Destructors Cannot Throw Checked Errors",
                "catch every checked error completely inside `__destruct`".to_string(),
            )
        } else {
            (
                format!(
                    "checked errors {listed} are not declared by `{}`",
                    function.name
                ),
                "Checked Errors Are Not Declared",
                format!(
                    "catch each error or add these exact types to the callable's `throws` clause: {}",
                    displays.join(", ")
                ),
            )
        };
        self.diagnostics.push(
            Diagnostic::new("E0631", message, function.span)
                .with_title(title)
                .with_help(help),
        );
    }

    fn checked_error_type_covers(
        types: &TypeRegistry,
        covering: &[TypeId],
        effect: TypeId,
    ) -> bool {
        covering.iter().any(|candidate| {
            *candidate == effect || matches!(types.kind(*candidate), TypeKind::Error)
        })
    }

    fn record_checked_effects(&mut self, effects: impl IntoIterator<Item = TypeId>, span: Span) {
        let effects = effects.into_iter().collect::<Vec<_>>();
        crate::checked_effects::record_effect_site(
            &mut self.checked_effect_sites,
            span,
            effects.iter().map(|effect| self.types.resolved(*effect)),
        );
        if let Some(scope) = self.effect_scopes.last_mut() {
            scope.extend(effects);
            return;
        }
        let required = effects
            .into_iter()
            .filter(|effect| {
                !crate::checked_effects::is_ambient_io_effect(&self.types.resolved(*effect))
            })
            .collect::<Vec<_>>();
        if required.is_empty() {
            return;
        }
        let displays = required
            .iter()
            .map(|effect| self.types.display(*effect))
            .collect::<Vec<_>>();
        let listed = displays
            .iter()
            .map(|display| format!("`{display}`"))
            .collect::<Vec<_>>()
            .join(", ");
        self.diagnostics.push(
            Diagnostic::new(
                "E0630",
                format!("checked errors {listed} are not handled"),
                span,
            )
            .with_title("Checked Errors Are Not Handled")
            .with_help(format!(
                "catch each error or move this operation into a callable declaring these exact types: {}",
                displays.join(", ")
            )),
        );
    }

    fn record_callable_dependency(&mut self, callee: Span) {
        let Some(caller) = self.current_callable else {
            return;
        };
        let dependencies = self.callable_dependencies.entry(caller).or_default();
        if !dependencies.contains(&callee) {
            dependencies.push(callee);
        }
    }

    fn complete_function_value_effects(&mut self, required: &[TypeId], span: Span) -> Vec<TypeId> {
        let mut complete = required.to_vec();
        for name in [
            crate::compiler_known_io::IO_ERROR,
            crate::compiler_known_io::INVALID_UTF8_ERROR,
        ] {
            let effect = self.resolve_type_ref(&TypeRef::named(name), span);
            if !complete.contains(&effect) {
                complete.push(effect);
            }
        }
        complete
    }

    fn check_parameter_default_support(
        &mut self,
        function: &FunctionDecl,
        parameter_index: usize,
        param: &Param,
        ty: TypeId,
        method_context: Option<&MethodContext>,
    ) {
        let default = param
            .default
            .as_ref()
            .expect("parameter-default validation requires a default");
        let kind = self.types.kind(ty).clone();

        if matches!(kind, TypeKind::String) && param.take {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for `take string` parameters are not yet supported",
                default.span(),
            ));
            return;
        }

        if matches!(kind, TypeKind::String) && param.writable {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for `writable string` parameters are not yet supported",
                default.span(),
            ));
            return;
        }

        let nullable_copy_enum = match &kind {
            TypeKind::Nullable(inner) => {
                matches!(self.types.kind(*inner), TypeKind::Enum(_))
                    && !self.type_is_move_type(*inner)
            }
            _ => false,
        };
        let compiler_known_nullable_scalar = method_context.is_some_and(|context| {
            matches!(
                context.class_name.as_str(),
                crate::compiler_known_io::IO_ERROR | crate::compiler_known_io::INVALID_UTF8_ERROR
            ) && matches!(default, Expr::Null { .. })
                && matches!(
                    &kind,
                    TypeKind::Nullable(inner)
                        if matches!(self.types.kind(*inner), TypeKind::Integer(_))
                )
        });

        if let TypeKind::Nullable(inner) = &kind {
            if !nullable_copy_enum && !compiler_known_nullable_scalar {
                let message = if matches!(self.types.kind(*inner), TypeKind::String) {
                    "default values for nullable string parameters are not yet supported"
                } else {
                    "default values for this nullable parameter type are not yet supported"
                };
                self.diagnostics
                    .push(Diagnostic::new("E0498", message, default.span()));
                return;
            }
        }

        if param.take || self.type_is_move_type(ty) {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for move-type or `take` parameters are not yet supported",
                default.span(),
            ));
            return;
        }

        if !nullable_copy_enum
            && !compiler_known_nullable_scalar
            && !matches!(
                &kind,
                TypeKind::Integer(_)
                    | TypeKind::Float(_)
                    | TypeKind::Bool
                    | TypeKind::String
                    | TypeKind::Enum(_)
            )
        {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "default values for this parameter type are not yet supported",
                default.span(),
            ));
            return;
        }

        let declaring_class = method_context.map(|context| context.class_name.as_str());
        let Some(value) = crate::const_eval::evaluate_parameter_default(
            &self.const_evaluation,
            default,
            &param.ty,
            declaring_class,
        ) else {
            self.diagnostics.push(Diagnostic::new(
                "E0498",
                "a default value must be a constant expression",
                default.span(),
            ));
            return;
        };

        self.parameter_defaults.insert(
            crate::const_eval::ParameterDefaultKey {
                function_start: function.span.start,
                parameter_index,
            },
            value,
        );
    }

    fn return_context_for_function(
        &mut self,
        function: &FunctionDecl,
        method_context: Option<&MethodContext>,
    ) -> ReturnContext {
        let is_method = method_context.is_some();
        let lifecycle = is_method
            .then(|| LifecycleMethod::from_method_name(&function.name))
            .flatten();
        let name = method_context
            .map(|context| format!("{}::{}", context.class_name, function.name))
            .unwrap_or_else(|| function.name.clone());

        if let Some(lifecycle) = lifecycle {
            self.check_lifecycle_return_type(function, lifecycle);
        }

        let expected = if lifecycle.is_some() {
            None
        } else if function.return_type.is_some() {
            Some(self.current_function_return_type(function))
        } else {
            None
        };

        ReturnContext {
            name,
            expected,
            lifecycle,
            is_method,
        }
    }

    fn check_lifecycle_return_type(&mut self, function: &FunctionDecl, lifecycle: LifecycleMethod) {
        if function.return_type.is_none() {
            return;
        }

        let return_ty = self.current_function_return_type(function);

        if self.is_void_type(return_ty) {
            return;
        }

        self.diagnostics.push(
            Diagnostic::new(
                "E0407",
                format!(
                    "{} `{}` cannot declare non-void return type",
                    lifecycle.label(),
                    lifecycle.doria_name()
                ),
                function.span,
            )
            .with_help(format!(
                "remove the return type annotation or use `{}(): void`",
                lifecycle.doria_name()
            )),
        );
    }

    fn current_function_return_type(&mut self, function: &FunctionDecl) -> TypeId {
        self.current_function_signature(function).return_ty
    }

    fn current_function_signature(&mut self, function: &FunctionDecl) -> FunctionInfo {
        self.function_signatures
            .get(&function.span)
            .cloned()
            .unwrap_or_else(|| self.resolve_function_signature(function, None))
    }

    fn check_block(
        &mut self,
        block: &Block,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        mut constructor_init_context: Option<&mut ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        scopes.push();
        for statement in &block.statements {
            self.check_statement(
                statement,
                scopes,
                method_context,
                constructor_init_context.as_deref_mut(),
                return_context,
                loop_depth,
            );
        }
        scopes.pop();
    }

    fn check_statement(
        &mut self,
        statement: &Stmt,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        mut constructor_init_context: Option<&mut ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        self.active_loop_depth = loop_depth;
        match statement {
            Stmt::Block(block) => {
                let mut nested_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::nested);
                self.check_block(
                    block,
                    scopes,
                    method_context,
                    nested_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth,
                );
            }
            Stmt::VarDecl(decl) => {
                self.check_local_declaration(decl, scopes, method_context);
            }
            Stmt::Assignment(assignment) => {
                if let Some(target) = self.check_writable_place(
                    &assignment.target,
                    &assignment.op,
                    scopes,
                    method_context,
                    constructor_init_context,
                ) {
                    self.record_expected_expression_type(&assignment.value, target.ty);
                    self.check_expr(&assignment.value, scopes, method_context);
                    self.check_assignment_value(assignment, target, scopes, method_context);
                } else {
                    self.check_expr(&assignment.value, scopes, method_context);
                }
            }
            Stmt::Echo { expr, span } => {
                let diagnostics_before = self.diagnostics.len();
                self.check_expr(expr, scopes, method_context);
                self.check_mixed_value_operation(expr, "echo", scopes, method_context);
                let ty = self.infer_expr_type(expr, scopes, method_context);
                if matches!(
                    self.display_conversion_kind(ty),
                    DisplayConversionKind::NonDisplayableClass
                ) {
                    self.report_non_displayable_class(ty, expr.span());
                } else if !self.is_display_convertible_type(ty) {
                    self.diagnostics.push(Diagnostic::new(
                        "E0445",
                        format!(
                            "value of type `{}` cannot be displayed by echo",
                            self.types.display(ty)
                        ),
                        expr.span(),
                    ));
                }
                if self.diagnostics.len() == diagnostics_before {
                    self.record_compiler_known_effects(
                        crate::builtins::ECHO_CHECKED_ERROR_TYPES,
                        *span,
                    );
                }
            }
            Stmt::Expr { expr, .. } => match expr {
                Expr::FunctionCall { name, args, span } if name == "panic" => {
                    for arg in args {
                        self.check_expr(&arg.value, scopes, method_context);
                    }
                    self.check_panic_call(args, *span, scopes, method_context);
                }
                _ => self.check_expr(expr, scopes, method_context),
            },
            Stmt::Return { expr, span } => {
                if self.return_leaves_active_finalizer() {
                    if let Some(expr) = expr {
                        self.check_expr(expr, scopes, method_context);
                    }
                    self.report_finalizer_transfer("return", *span);
                    return;
                }
                if self.when_contexts.is_empty() {
                    self.check_return_statement(
                        expr.as_ref(),
                        *span,
                        scopes,
                        method_context,
                        return_context,
                    );
                } else {
                    self.check_when_yield(expr.as_ref(), *span, scopes, method_context);
                }
            }
            Stmt::Throw(statement) => {
                self.check_throw_statement(statement, scopes, method_context);
            }
            Stmt::Try(statement) => {
                self.check_try_statement(
                    statement,
                    scopes,
                    method_context,
                    constructor_init_context.as_deref(),
                    return_context,
                    loop_depth,
                );
            }
            Stmt::If(if_stmt) => {
                let mut construct_scopes = scopes.clone();
                construct_scopes.push();
                if let Some(given) = &if_stmt.given {
                    self.check_given_prelude(given, &mut construct_scopes, method_context);
                }
                self.check_condition(&if_stmt.condition, &construct_scopes, method_context);
                let mut then_scopes = construct_scopes.clone();
                let mut then_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::nested);
                self.check_block(
                    &if_stmt.then_block,
                    &mut then_scopes,
                    method_context,
                    then_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth,
                );
                if let Some(else_branch) = &if_stmt.else_branch {
                    self.check_else_branch(
                        else_branch,
                        &construct_scopes,
                        method_context,
                        constructor_init_context.as_deref(),
                        return_context,
                        loop_depth,
                    );
                }
                self.check_finally(
                    if_stmt.finally.as_ref(),
                    &construct_scopes,
                    method_context,
                    constructor_init_context.as_deref(),
                    return_context,
                    loop_depth,
                );
            }
            Stmt::While(while_stmt) => {
                let mut construct_scopes = scopes.clone();
                construct_scopes.push();
                if let Some(given) = &while_stmt.given {
                    self.check_given_prelude(given, &mut construct_scopes, method_context);
                }
                self.check_condition(&while_stmt.condition, &construct_scopes, method_context);
                let mut body_scopes = construct_scopes.clone();
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::repeatable);
                self.check_block(
                    &while_stmt.body,
                    &mut body_scopes,
                    method_context,
                    loop_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth + 1,
                );
                self.check_finally(
                    while_stmt.finally.as_ref(),
                    &construct_scopes,
                    method_context,
                    constructor_init_context.as_deref(),
                    return_context,
                    loop_depth,
                );
            }
            Stmt::DoWhile(do_while) => {
                let mut construct_scopes = scopes.clone();
                construct_scopes.push();
                let mut body_scopes = construct_scopes.clone();
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::repeatable);
                self.check_block(
                    &do_while.body,
                    &mut body_scopes,
                    method_context,
                    loop_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth + 1,
                );
                self.check_condition(&do_while.condition, &construct_scopes, method_context);
                self.check_finally(
                    do_while.finally.as_ref(),
                    &construct_scopes,
                    method_context,
                    constructor_init_context.as_deref(),
                    return_context,
                    loop_depth,
                );
            }
            Stmt::For(for_stmt) => {
                let mut loop_scopes = scopes.clone();
                loop_scopes.push();
                if let Some(initializer) = &for_stmt.initializer {
                    self.check_for_initializer(
                        initializer,
                        &mut loop_scopes,
                        method_context,
                        constructor_init_context.as_deref_mut(),
                    );
                }
                if let Some(condition) = &for_stmt.condition {
                    self.check_condition(condition, &loop_scopes, method_context);
                }
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::repeatable);
                self.check_block(
                    &for_stmt.body,
                    &mut loop_scopes,
                    method_context,
                    loop_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth + 1,
                );
                if let Some(increment) = &for_stmt.increment {
                    self.check_for_increment(
                        increment,
                        &mut loop_scopes,
                        method_context,
                        loop_constructor_init_context.as_mut(),
                    );
                }
            }
            Stmt::Break { span } => {
                if self.loop_transfer_leaves_active_finalizer(loop_depth) {
                    self.report_finalizer_transfer("break", *span);
                } else if loop_depth == 0 {
                    self.diagnostics.push(Diagnostic::new(
                        "E0421",
                        "`break` may only be used inside a loop",
                        *span,
                    ));
                }
            }
            Stmt::Continue { span } => {
                if self.loop_transfer_leaves_active_finalizer(loop_depth) {
                    self.report_finalizer_transfer("continue", *span);
                } else if loop_depth == 0 {
                    self.diagnostics.push(Diagnostic::new(
                        "E0422",
                        "`continue` may only be used inside a loop",
                        *span,
                    ));
                }
            }
            Stmt::Foreach(foreach) => {
                let range_iterable = Self::is_grouped_range_expr(&foreach.iterable);
                let dictionary_projection = Self::dictionary_foreach_projection(&foreach.iterable)
                    .and_then(|(dictionary, projection)| {
                        let ty = self.infer_expr_type(dictionary, scopes, method_context);
                        matches!(
                            self.types.kind(ty),
                            TypeKind::Dictionary(_, _) | TypeKind::SortedDictionary(_, _)
                        )
                        .then_some((dictionary, projection))
                    });
                let iterable_ty = self.infer_expr_type(&foreach.iterable, scopes, method_context);
                if matches!(self.types.kind(iterable_ty), TypeKind::PriorityQueue(_)) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0529",
                            "PriorityQueue does not support foreach because heap layout is not a public iteration order",
                            foreach.span,
                        )
                        .with_title("PriorityQueue Has No Foreach Order")
                        .with_help(
                            "use repeated `pop()` only when destructive min-first traversal is appropriate",
                        ),
                    );
                }
                if range_iterable {
                    if let Some(integer) = foreach
                        .value
                        .ty
                        .as_ref()
                        .and_then(|ty| ty.arguments.is_empty().then_some(ty))
                        .and_then(|ty| IntegerType::from_source_name(&ty.name))
                    {
                        self.contextualize_range_literals(&foreach.iterable, integer);
                    }
                }
                let unknown_ty = self.types.unknown();
                let range_integer = self
                    .range_integer_type(&foreach.iterable, scopes, method_context)
                    .unwrap_or(IntegerType::Int64);
                let int_ty = self.types.intern(TypeKind::Integer(range_integer));
                let (iterable_key_ty, iterable_value_ty) = if range_iterable {
                    (unknown_ty, int_ty)
                } else {
                    self.infer_foreach_binding_types(foreach, scopes, method_context)
                };

                if range_iterable {
                    self.check_expr_with_range_context(
                        &foreach.iterable,
                        scopes,
                        method_context,
                        true,
                    );
                } else if let Some((dictionary, _)) = dictionary_projection {
                    self.check_expr(dictionary, scopes, method_context);
                    self.check_mixed_operation(
                        dictionary,
                        "foreach iterable",
                        scopes,
                        method_context,
                    );
                    self.check_nullable_member_access(
                        dictionary,
                        false,
                        "foreach iterable",
                        scopes,
                        method_context,
                    );
                } else {
                    self.check_expr(&foreach.iterable, scopes, method_context);
                    self.check_mixed_operation(
                        &foreach.iterable,
                        "foreach iterable",
                        scopes,
                        method_context,
                    );
                }
                let mut loop_scopes = scopes.clone();
                loop_scopes.push();
                if let Some(key) = &foreach.key {
                    if key.writable {
                        self.diagnostics.push(Diagnostic::new(
                            "E0520",
                            "foreach key bindings are readonly",
                            foreach.span,
                        ));
                    }
                    let ty = if dictionary_projection.is_some() {
                        self.diagnostics.push(Diagnostic::new(
                            "E0522",
                            "dictionary `keys` and `values` projections yield one readonly element and do not support a key binding",
                            foreach.span,
                        ));
                        self.types.unknown()
                    } else if range_iterable {
                        self.diagnostics.push(Diagnostic::new(
                            "E0425",
                            "foreach over integer ranges does not support key bindings",
                            foreach.span,
                        ));
                        self.types.unknown()
                    } else {
                        key.ty.as_ref().map_or(iterable_key_ty, |ty| {
                            let annotated_ty = self.resolve_type_ref(ty, foreach.span);
                            self.check_foreach_binding_type(
                                annotated_ty,
                                iterable_key_ty,
                                foreach.span,
                            );
                            annotated_ty
                        })
                    };
                    self.declare_binding(
                        &mut loop_scopes,
                        key.name.clone(),
                        Binding::unresolved(false, ty, ty, None, None),
                        key.span,
                        BindingKind::ForeachKey,
                        BindingOwnership::ReadonlyBorrow,
                    );
                }
                let value_ty = if range_iterable {
                    if let Some(annotation) = &foreach.value.ty {
                        let annotated_ty = self.resolve_type_ref(annotation, foreach.span);
                        self.check_foreach_binding_type(annotated_ty, int_ty, foreach.span);
                    }
                    int_ty
                } else {
                    foreach.value.ty.as_ref().map_or(iterable_value_ty, |ty| {
                        let annotated_ty = self.resolve_type_ref(ty, foreach.span);
                        self.check_foreach_binding_type(
                            annotated_ty,
                            iterable_value_ty,
                            foreach.span,
                        );
                        annotated_ty
                    })
                };
                if dictionary_projection.is_some() && foreach.value.writable {
                    self.diagnostics.push(Diagnostic::new(
                        "E0522",
                        "dictionary `keys` and `values` projections are readonly; use the main dictionary `foreach` form for writable values",
                        foreach.span,
                    ));
                }
                if range_iterable && foreach.value.writable {
                    self.diagnostics.push(Diagnostic::new(
                        "E0425",
                        "foreach over integer ranges produces readonly value bindings",
                        foreach.span,
                    ));
                }
                if matches!(
                    self.types.kind(iterable_ty),
                    TypeKind::Set(_) | TypeKind::SortedSet(_)
                ) && foreach.value.writable
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0530",
                            "set elements cannot be written in place",
                            foreach.span,
                        )
                        .with_title("Set Elements Cannot Be Written In Place")
                        .with_explanation(
                            "Changing a set element in place could invalidate uniqueness, hashing, or sorted order.",
                        )
                        .with_help(
                            "remove the old element and add the replacement through the collection",
                        ),
                    );
                }
                self.declare_binding(
                    &mut loop_scopes,
                    foreach.value.name.clone(),
                    Binding::unresolved(
                        foreach.value.writable
                            && dictionary_projection.is_none()
                            && !range_iterable
                            && !matches!(
                                self.types.kind(iterable_ty),
                                TypeKind::Set(_) | TypeKind::SortedSet(_)
                            ),
                        value_ty,
                        value_ty,
                        None,
                        None,
                    ),
                    foreach.value.span,
                    BindingKind::ForeachValue,
                    if foreach.value.writable {
                        BindingOwnership::WritableBorrow
                    } else {
                        BindingOwnership::ReadonlyBorrow
                    },
                );
                let mut loop_constructor_init_context = constructor_init_context
                    .as_deref()
                    .map(ConstructorInitContext::repeatable);
                for statement in &foreach.body.statements {
                    self.check_statement(
                        statement,
                        &mut loop_scopes,
                        method_context,
                        loop_constructor_init_context.as_mut(),
                        return_context,
                        loop_depth + 1,
                    );
                }
            }
            Stmt::Increment(increment) => {
                self.check_increment_statement(
                    increment,
                    scopes,
                    method_context,
                    constructor_init_context,
                );
            }
        }
    }

    fn check_throw_statement(
        &mut self,
        statement: &ThrowStmt,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.check_expr(&statement.expr, scopes, method_context);
        let error_type = self.infer_expr_type(&statement.expr, scopes, method_context);
        if self.is_unknown_type(error_type) {
            return;
        }
        if !self.type_implements_error(error_type) {
            let (title, help) = match self.types.kind(error_type) {
                TypeKind::Class(class) => (
                    "Class Must Explicitly Implement Error",
                    format!("declare `class {} implements Error` and provide its required readonly `string $message` property", class.name),
                ),
                _ => (
                    "Throw Requires An Error Value",
                    "throw an owned instance of a class that explicitly implements `Error`".to_string(),
                ),
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0623",
                    format!(
                        "cannot throw value of type `{}` because it does not implement `Error`",
                        self.types.display(error_type)
                    ),
                    statement.expr.span(),
                )
                .with_title(title)
                .with_help(help),
            );
            return;
        }
        self.throw_error_types
            .insert(statement.span, self.types.resolved(error_type));
        self.record_checked_effects([error_type], statement.span);
    }

    #[allow(clippy::too_many_arguments)]
    fn check_try_statement(
        &mut self,
        statement: &TryStmt,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        self.effect_scopes.push(CheckedEffectSet::default());
        let mut protected_scopes = scopes.clone();
        let mut protected_constructor =
            constructor_init_context.map(ConstructorInitContext::nested);
        self.check_block(
            &statement.body,
            &mut protected_scopes,
            method_context,
            protected_constructor.as_mut(),
            return_context,
            loop_depth,
        );
        let protected = self.effect_scopes.pop().expect("try effect scope");
        let mut uncovered = protected.clone();
        let mut seen_catches = Vec::new();
        let mut saw_error_catch = false;

        for catch in &statement.catches {
            let catch_type = self.resolve_type_ref_with_class(
                &catch.ty,
                catch.ty_span,
                method_context.map(|context| context.class_name.as_str()),
            );
            if self.is_unknown_type(catch_type) {
                continue;
            }
            if !self.type_implements_error(catch_type) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0626",
                        format!(
                            "catch type `{}` does not implement `Error`",
                            self.types.display(catch_type)
                        ),
                        catch.ty_span,
                    )
                    .with_title("Catch Must Name An Error Type"),
                );
                continue;
            }
            self.catch_error_types
                .insert(catch.span, self.types.resolved(catch_type));
            if seen_catches.contains(&catch_type) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0627",
                        format!("duplicate catch for `{}`", self.types.display(catch_type)),
                        catch.ty_span,
                    )
                    .with_title("Duplicate Catch"),
                );
                continue;
            }
            if saw_error_catch {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0628",
                        "no catch can follow the catch-all `Error` clause",
                        catch.ty_span,
                    )
                    .with_title("Catch After Error Is Unreachable"),
                );
                continue;
            }
            let catches_all = matches!(self.types.kind(catch_type), TypeKind::Error);
            let catch_set = [catch_type];
            let reachable = protected.ordered.iter().any(|effect| {
                matches!(self.types.kind(*effect), TypeKind::Error)
                    || Self::checked_error_type_covers(&self.types, &catch_set, *effect)
            });
            if !reachable {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0629",
                        format!(
                            "this try cannot produce `{}`",
                            self.types.display(catch_type)
                        ),
                        catch.ty_span,
                    )
                    .with_title("Catch Cannot Match An Error From This Try"),
                );
            }
            if catches_all {
                uncovered.ordered.clear();
                saw_error_catch = true;
            } else {
                uncovered.ordered.retain(|effect| {
                    !Self::checked_error_type_covers(&self.types, &catch_set, *effect)
                });
            }
            seen_catches.push(catch_type);

            let mut catch_scopes = scopes.clone();
            catch_scopes.push();
            if let Some(binding) = &catch.binding {
                self.declare_binding(
                    &mut catch_scopes,
                    binding.name.clone(),
                    Binding::unresolved(false, catch_type, catch_type, None, None),
                    binding.span,
                    BindingKind::CatchBinding,
                    BindingOwnership::Owned,
                );
            }
            let mut catch_constructor =
                constructor_init_context.map(ConstructorInitContext::nested);
            self.check_block(
                &catch.body,
                &mut catch_scopes,
                method_context,
                catch_constructor.as_mut(),
                return_context,
                loop_depth,
            );
            catch_scopes.pop();
        }

        if let Some(finally) = &statement.finally {
            self.effect_scopes.push(CheckedEffectSet::default());
            self.finalizer_boundaries.push(FinalizerBoundary {
                loop_depth,
                when_depth: self.when_contexts.len(),
            });
            let mut finally_scopes = scopes.clone();
            let mut finally_constructor =
                constructor_init_context.map(ConstructorInitContext::nested);
            self.check_block(
                &finally.body,
                &mut finally_scopes,
                method_context,
                finally_constructor.as_mut(),
                return_context,
                loop_depth,
            );
            self.finalizer_boundaries
                .pop()
                .expect("checked try finalizer boundary");
            let finalizer_effects = self.effect_scopes.pop().expect("finally effect scope");
            self.record_checked_effects(finalizer_effects.ordered, finally.span);
        }

        self.try_uncovered_effects.insert(
            statement.span,
            uncovered
                .ordered
                .iter()
                .map(|effect| self.types.resolved(*effect))
                .collect(),
        );
        self.record_checked_effects(uncovered.ordered, statement.span);
    }

    fn check_local_declaration(
        &mut self,
        decl: &VarDecl,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.check_local_declaration_with_kind(decl, scopes, method_context, None);
    }

    fn check_local_declaration_with_kind(
        &mut self,
        decl: &VarDecl,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        kind: Option<BindingKind>,
    ) {
        let diagnostics_before = self.diagnostics.len();
        let explicit_ty = decl
            .ty
            .as_ref()
            .map(|ty| self.resolve_type_ref(ty, decl.span));
        if let Some(target_ty) = explicit_ty {
            self.record_expected_expression_type(&decl.initializer, target_ty);
        }
        self.initializing_bindings.push(
            decl.bindings
                .iter()
                .map(|binding| (binding.name.clone(), binding.span))
                .collect(),
        );
        self.check_expr(&decl.initializer, scopes, method_context);
        let value_ty = self.infer_expr_type(&decl.initializer, scopes, method_context);
        self.initializing_bindings.pop();
        if self.is_void_type(value_ty) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0403",
                    "a `void` expression cannot initialize a local binding",
                    decl.initializer.span(),
                )
                .with_title("Void Expression Has No Value")
                .with_primary_label("This Call Returns `void`")
                .with_explanation(
                    "A `void` call performs an action but does not produce a value that can be stored.",
                )
                .with_help("call the operation as an expression statement instead"),
            );
            return;
        }
        let ty = match explicit_ty {
            Some(target_ty) => {
                self.check_expr_assignable(
                    target_ty,
                    &decl.initializer,
                    scopes,
                    method_context,
                    AssignmentDestination::Type,
                );
                target_ty
            }
            None => {
                if let TypeKind::Integer(integer) = *self.types.kind(value_ty) {
                    self.contextualize_integer_literals(&decl.initializer, integer);
                }
                if self.stage26_collection_has_unknown_arguments(value_ty) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0539",
                            "an empty collection source does not reveal the destination element type",
                            decl.initializer.span(),
                        )
                        .with_title("Collection Type Cannot Be Inferred")
                        .with_explanation(
                            "An empty source contains no element or key/value pair from which to infer the generic arguments.",
                        )
                        .with_help(
                            "add an explicit destination type, for example `Deque<int> $values = Deque::from([])`",
                        ),
                    );
                }
                value_ty
            }
        };

        let grouped = decl.bindings.len() > 1;
        if grouped && decl.ty.is_none() && Self::is_null_literal(&decl.initializer) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0552",
                    "`null` does not reveal the nullable payload type shared by this local group",
                    decl.initializer.span(),
                )
                .with_title("Grouped Null Declaration Needs An Explicit Type")
                .with_explanation(
                    "Each empty binding needs the same explicit nullable type; Doria does not widen an untyped null group to `mixed`.",
                )
                .with_help("write an explicit nullable type before the grouped bindings"),
            );
        } else if grouped && self.type_is_move_type(ty) {
            let explicitly_empty_nullable = decl.ty.is_some()
                && Self::is_null_literal(&decl.initializer)
                && matches!(self.types.kind(ty), TypeKind::Nullable(_));
            if !explicitly_empty_nullable {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0551",
                        "one owned initializer cannot initialize several independent local bindings",
                        decl.initializer.span(),
                    )
                    .with_title("Initializer Cannot Create Multiple Owned Bindings")
                    .with_explanation(format!(
                        "The initializer has move type `{}`, so copying it would create more than one owner of the same value.",
                        self.types.display(ty)
                    ))
                    .with_help(
                        "create each owned value explicitly, or use explicit shared ownership when the bindings should refer to one allocation",
                    ),
                );
            }
        }

        if grouped {
            let mut names = HashMap::<&str, Span>::new();
            for binding in &decl.bindings {
                if let Some(original) = names.insert(&binding.name, binding.span) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0103",
                            format!(
                                "variable `${}` is already declared in this local group",
                                binding.name
                            ),
                            binding.span,
                        )
                        .with_related(original, "the first binding with this name is here"),
                    );
                } else if scopes.contains_in_current_scope(&binding.name) {
                    self.diagnostics.push(Diagnostic::new(
                        "E0103",
                        format!(
                            "variable `${}` is already declared in this scope",
                            binding.name
                        ),
                        binding.span,
                    ));
                }
            }

            // A grouped declaration is atomic. Its initializer is checked in
            // the previous scope, and no name from the group is introduced if
            // any part of the declaration is invalid.
            if self.diagnostics.len() != diagnostics_before {
                return;
            }
        }

        let int_constant = self.readonly_int_constant(decl.writable, ty, &decl.initializer, scopes);
        let string_constant =
            self.readonly_string_constant(decl.writable, ty, &decl.initializer, scopes);
        for binding in &decl.bindings {
            self.declare_binding(
                scopes,
                binding.name.clone(),
                Binding::unresolved(decl.writable, ty, ty, int_constant, string_constant.clone()),
                binding.span,
                kind.unwrap_or({
                    if decl.bindings.len() > 1 {
                        BindingKind::GroupedLocal
                    } else {
                        BindingKind::Local
                    }
                }),
                BindingOwnership::Owned,
            );
        }
    }

    fn check_for_initializer(
        &mut self,
        initializer: &ForInitializer,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        match initializer {
            ForInitializer::VarDecl(decl) => {
                self.check_local_declaration_with_kind(
                    decl,
                    scopes,
                    method_context,
                    Some(BindingKind::LoopBinding),
                );
            }
            ForInitializer::Assignment(assignment) => {
                if let Some(target) = self.check_writable_place(
                    &assignment.target,
                    &assignment.op,
                    scopes,
                    method_context,
                    constructor_init_context,
                ) {
                    self.record_expected_expression_type(&assignment.value, target.ty);
                    self.check_expr(&assignment.value, scopes, method_context);
                    self.check_assignment_value(assignment, target, scopes, method_context);
                } else {
                    self.check_expr(&assignment.value, scopes, method_context);
                }
            }
        }
    }

    fn check_for_increment(
        &mut self,
        increment: &ForIncrement,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        match increment {
            ForIncrement::Increment(increment) => {
                self.check_increment_statement(
                    increment,
                    scopes,
                    method_context,
                    constructor_init_context,
                );
            }
            ForIncrement::Assignment(assignment) => {
                if let Some(target) = self.check_writable_place(
                    &assignment.target,
                    &assignment.op,
                    scopes,
                    method_context,
                    constructor_init_context,
                ) {
                    self.record_expected_expression_type(&assignment.value, target.ty);
                    self.check_expr(&assignment.value, scopes, method_context);
                    self.check_assignment_value(assignment, target, scopes, method_context);
                } else {
                    self.check_expr(&assignment.value, scopes, method_context);
                }
            }
        }
    }
    fn check_assignment_value(
        &mut self,
        assignment: &Assignment,
        target: AssignmentTarget,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match assignment.op {
            AssignOp::Assign => {
                let target_ty = target.ty;
                let destination = target.destination.clone();
                let assignment_ok = self.check_expr_assignable(
                    target.ty,
                    &assignment.value,
                    scopes,
                    method_context,
                    destination,
                );
                if assignment_ok {
                    let value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                    self.narrow_empty_collection_assignment(
                        &assignment.target,
                        target_ty,
                        value_ty,
                        scopes,
                    );
                    self.update_nullable_assignment_flow_type(&assignment.target, value_ty, scopes);
                }
            }
            AssignOp::AddAssign
            | AssignOp::SubAssign
            | AssignOp::MulAssign
            | AssignOp::DivAssign
            | AssignOp::ModAssign
            | AssignOp::ShiftLeftAssign
            | AssignOp::ShiftRightAssign
            | AssignOp::BitwiseAndAssign
            | AssignOp::BitwiseOrAssign
            | AssignOp::BitwiseXorAssign => {
                let mut value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                if let TypeKind::Integer(integer) = *self.types.kind(target.ty) {
                    self.contextualize_integer_literals(&assignment.value, integer);
                    value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                } else if let TypeKind::Float(float) = *self.types.kind(target.ty) {
                    self.contextualize_float_literals(&assignment.value, float);
                    value_ty = self.infer_expr_type(&assignment.value, scopes, method_context);
                }
                let target_contains_mixed = self.type_contains_mixed(target.ty);
                let value_contains_mixed = self.type_contains_mixed(value_ty);

                if target_contains_mixed {
                    self.report_mixed_operation(assignment.target.span(), "compound assignment");
                }

                if value_contains_mixed {
                    self.report_mixed_operation(assignment.value.span(), "compound assignment");
                }

                if target_contains_mixed || value_contains_mixed {
                    return;
                }

                let integers_only = matches!(
                    assignment.op,
                    AssignOp::ModAssign
                        | AssignOp::ShiftLeftAssign
                        | AssignOp::ShiftRightAssign
                        | AssignOp::BitwiseAndAssign
                        | AssignOp::BitwiseOrAssign
                        | AssignOp::BitwiseXorAssign
                );
                let result_ty = self.infer_numeric_binary_type(target.ty, value_ty);
                if integers_only
                    && !matches!(
                        self.types.kind(result_ty),
                        TypeKind::Integer(_) | TypeKind::Unknown
                    )
                {
                    self.report_integer_operand_mismatch(
                        target.ty,
                        value_ty,
                        assignment.span,
                        "compound assignment",
                    );
                    return;
                }
                if !self.is_assignable(target.ty, result_ty) {
                    if matches!(self.types.kind(target.ty), TypeKind::Integer(_))
                        && matches!(self.types.kind(value_ty), TypeKind::Integer(_))
                    {
                        self.report_integer_operand_mismatch(
                            target.ty,
                            value_ty,
                            assignment.span,
                            "compound assignment",
                        );
                    } else {
                        self.check_assignable(
                            target.ty,
                            result_ty,
                            assignment.value.span(),
                            target.destination,
                        );
                    }
                }
            }
        }
    }

    fn check_increment_statement(
        &mut self,
        increment: &IncrementStmt,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        self.check_increment_target(
            &increment.target,
            &increment.op,
            scopes,
            method_context,
            constructor_init_context,
        );
    }

    fn check_increment_target(
        &mut self,
        target: &Expr,
        op: &IncrementOp,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) {
        let (op_name, assignment_op) = match op {
            IncrementOp::Increment => ("++", AssignOp::AddAssign),
            IncrementOp::Decrement => ("--", AssignOp::SubAssign),
        };
        let target_span = target.span();
        let Some(place) = self.check_writable_place(
            target,
            &assignment_op,
            scopes,
            method_context,
            constructor_init_context,
        ) else {
            return;
        };

        if !matches!(
            self.types.kind(place.ty),
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Unknown
        ) {
            self.diagnostics.push(Diagnostic::new(
                "E0423",
                format!("{op_name} requires a writable integer or float target"),
                target_span,
            ));
        }
    }

    fn check_else_branch(
        &mut self,
        branch: &ElseBranch,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        match branch {
            ElseBranch::If(if_stmt) => {
                let mut construct_scopes = scopes.clone();
                construct_scopes.push();
                if let Some(given) = &if_stmt.given {
                    self.check_given_prelude(given, &mut construct_scopes, method_context);
                }
                self.check_condition(&if_stmt.condition, &construct_scopes, method_context);
                let mut then_scopes = construct_scopes.clone();
                let mut then_constructor_init_context =
                    constructor_init_context.map(ConstructorInitContext::nested);
                self.check_block(
                    &if_stmt.then_block,
                    &mut then_scopes,
                    method_context,
                    then_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth,
                );
                if let Some(else_branch) = &if_stmt.else_branch {
                    self.check_else_branch(
                        else_branch,
                        &construct_scopes,
                        method_context,
                        constructor_init_context,
                        return_context,
                        loop_depth,
                    );
                }
                self.check_finally(
                    if_stmt.finally.as_ref(),
                    &construct_scopes,
                    method_context,
                    constructor_init_context,
                    return_context,
                    loop_depth,
                );
            }
            ElseBranch::Block(block) => {
                let mut block_scopes = scopes.clone();
                let mut block_constructor_init_context =
                    constructor_init_context.map(ConstructorInitContext::nested);
                self.check_block(
                    block,
                    &mut block_scopes,
                    method_context,
                    block_constructor_init_context.as_mut(),
                    return_context,
                    loop_depth,
                );
            }
        }
    }

    fn check_condition(
        &mut self,
        condition: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.check_expr(condition, scopes, method_context);
        let ty = self.infer_expr_type(condition, scopes, method_context);
        if matches!(self.types.kind(ty), TypeKind::Bool | TypeKind::Unknown) {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0416",
            format!("condition must be `bool`, got `{}`", self.types.display(ty)),
            condition.span(),
        ));
    }

    fn check_given_prelude(
        &mut self,
        given: &GivenPrelude,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let mut predicates_started = false;
        let mut predicate_statement_indices = Vec::new();
        for (index, statement) in given.block.statements.iter().enumerate() {
            match statement {
                Stmt::VarDecl(declaration) => {
                    if predicates_started {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0605",
                                "`given` setup must appear before its first predicate",
                                declaration.span,
                            )
                            .with_title("Given Setup Appears After A Predicate")
                            .with_help("move this declaration before the first bool predicate"),
                        );
                    }
                    self.check_local_declaration_with_kind(
                        declaration,
                        scopes,
                        method_context,
                        Some(BindingKind::GivenBinding),
                    );
                }
                Stmt::Expr { expr, span } => {
                    self.check_expr(expr, scopes, method_context);
                    let ty = self.infer_expr_type(expr, scopes, method_context);
                    if self.is_void_type(ty) {
                        if predicates_started {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0605",
                                    "`given` setup must appear before its first predicate",
                                    *span,
                                )
                                .with_title("Given Setup Appears After A Predicate")
                                .with_help(
                                    "move this void setup call before the first bool predicate",
                                ),
                            );
                        }
                    } else if matches!(self.types.kind(ty), TypeKind::Bool | TypeKind::Unknown) {
                        predicates_started = true;
                        predicate_statement_indices.push(index);
                    } else {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0606",
                                format!(
                                    "`given` predicate must be `bool`, got `{}`",
                                    self.types.display(ty)
                                ),
                                expr.span(),
                            )
                            .with_title("Given Predicate Must Be Bool")
                            .with_help("use a bool expression, or make this a void setup call"),
                        );
                    }
                }
                _ => {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0607",
                            "`given` accepts only setup declarations, void setup calls, and bool predicates",
                            statement_span(statement),
                        )
                        .with_title("Nested Control Flow Is Not Allowed In Given")
                        .with_help("move control flow into the attached construct"),
                    );
                }
            }
        }
        self.given_preludes.insert(
            given.span,
            GivenSemanticInfo {
                predicate_statement_indices,
            },
        );
    }

    fn check_finally(
        &mut self,
        finally: Option<&ControlFlowFinally>,
        construct_scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&ConstructorInitContext>,
        return_context: Option<&ReturnContext>,
        loop_depth: usize,
    ) {
        let Some(finally) = finally else {
            return;
        };
        self.effect_scopes.push(CheckedEffectSet::default());
        self.finalizer_boundaries.push(FinalizerBoundary {
            loop_depth,
            when_depth: self.when_contexts.len(),
        });
        let mut finally_scopes = construct_scopes.clone();
        let mut finally_constructor_init_context =
            constructor_init_context.map(ConstructorInitContext::nested);
        self.check_block(
            &finally.block,
            &mut finally_scopes,
            method_context,
            finally_constructor_init_context.as_mut(),
            return_context,
            loop_depth,
        );
        self.finalizer_boundaries
            .pop()
            .expect("checked finalizer boundary");
        let finalizer_effects = self.effect_scopes.pop().expect("finally effect scope");
        self.record_checked_effects(finalizer_effects.ordered, finally.span);
        self.active_loop_depth = loop_depth;
    }

    fn return_leaves_active_finalizer(&self) -> bool {
        self.finalizer_boundaries
            .last()
            .is_some_and(|boundary| self.when_contexts.len() <= boundary.when_depth)
    }

    fn loop_transfer_leaves_active_finalizer(&self, loop_depth: usize) -> bool {
        self.finalizer_boundaries
            .last()
            .is_some_and(|boundary| loop_depth <= boundary.loop_depth)
    }

    fn report_finalizer_transfer(&mut self, keyword: &str, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0612",
                format!("`{keyword}` cannot leave a `finally` block"),
                span,
            )
            .with_title("Control Transfer Cannot Leave Finally")
            .with_help(format!(
                "move `{keyword}` outside `finally`, or target control flow declared inside the finalizer"
            )),
        );
    }

    fn check_when_yield(
        &mut self,
        expr: Option<&Expr>,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(expr) = expr else {
            self.diagnostics.push(
                Diagnostic::new("E0608", "a `when` branch must return a value", span)
                    .with_title("When Cannot Yield Void")
                    .with_help("return a value from this branch"),
            );
            return;
        };
        let index = self.when_contexts.len() - 1;
        let expected = self.when_contexts[index].expected;
        if let Some(expected) = expected {
            self.record_expected_expression_type(expr, expected);
        }
        self.check_expr(expr, scopes, method_context);
        let value = self.infer_expr_type(expr, scopes, method_context);
        if self.is_void_type(value) {
            self.diagnostics.push(
                Diagnostic::new("E0608", "a `when` branch cannot yield `void`", expr.span())
                    .with_title("When Cannot Yield Void"),
            );
            return;
        }
        self.when_contexts[index].saw_value = true;
        if let Some(expected) = expected {
            if !self.is_expr_assignable(expected, expr, scopes, method_context)
                && !self.is_assignable(expected, value)
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0609",
                        format!(
                            "when branch yields `{}` but the result type is `{}`",
                            self.types.display(value),
                            self.types.display(expected)
                        ),
                        expr.span(),
                    )
                    .with_title("When Branch Result Type Mismatch"),
                );
            }
            return;
        }

        let inferred = self.when_contexts[index].inferred;
        match inferred {
            None => self.when_contexts[index].inferred = Some(value),
            Some(target) if self.is_assignable(target, value) => {}
            Some(target) => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0609",
                        format!(
                            "when branch yields `{}` but the head branch yields `{}`",
                            self.types.display(value),
                            self.types.display(target)
                        ),
                        expr.span(),
                    )
                    .with_title("When Branch Result Type Mismatch")
                    .with_help("add an explicit `when (...): Type` result type when a broader context is intended"),
                );
            }
        }
    }

    fn check_return_statement(
        &mut self,
        expr: Option<&Expr>,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        return_context: Option<&ReturnContext>,
    ) {
        let Some(context) = return_context else {
            if let Some(expr) = expr {
                self.check_expr(expr, scopes, method_context);
            }
            return;
        };

        if let Some(expr) = expr {
            if context.lifecycle.is_none() {
                if let Some(expected) = context.expected {
                    if !self.is_void_type(expected) {
                        self.record_expected_expression_type(expr, expected);
                    }
                }
            }
            self.check_expr(expr, scopes, method_context);
        }

        if let Some(lifecycle) = context.lifecycle {
            if expr.is_some() {
                self.diagnostics.push(Diagnostic::new(
                    "E0405",
                    lifecycle.return_value_message(),
                    span,
                ));
            }
            return;
        }

        let Some(expected) = context.expected else {
            return;
        };

        if self.is_void_type(expected) {
            if expr.is_some() {
                self.diagnostics.push(Diagnostic::new(
                    "E0405",
                    format!(
                        "cannot return a value from void {} `{}`",
                        context.kind_name(),
                        context.name
                    ),
                    span,
                ));
            }
            return;
        }

        let Some(expr) = expr else {
            self.report_missing_return_value(context, expected, span);
            return;
        };

        let value = self.infer_expr_type(expr, scopes, method_context);
        if self.is_expr_assignable(expected, expr, scopes, method_context)
            || self.is_assignable(expected, value)
        {
            self.check_closure_return_capture(expr, expected, scopes);
            return;
        }

        self.report_return_type_mismatch(context, expected, value, expr.span());
    }

    fn check_missing_final_return(&mut self, function: &FunctionDecl, context: &ReturnContext) {
        if context.lifecycle.is_some() {
            return;
        }

        let Some(expected) = context.expected else {
            return;
        };

        if !self.requires_return_value(expected) {
            return;
        }

        if crate::return_analysis::analyze_with_given(function, &self.given_preludes)
            .fallthrough_reachable
        {
            self.report_missing_return_value(context, expected, function.span);
        }
    }

    fn report_return_type_mismatch(
        &mut self,
        context: &ReturnContext,
        expected: TypeId,
        value: TypeId,
        span: Span,
    ) {
        self.diagnostics.push(Diagnostic::new(
            "E0404",
            format!(
                "cannot return value of type `{}` from {} `{}` with return type `{}`",
                self.types.display(value),
                context.kind_name(),
                context.name,
                self.types.display(expected)
            ),
            span,
        ));
    }

    fn report_missing_return_value(
        &mut self,
        context: &ReturnContext,
        expected: TypeId,
        span: Span,
    ) {
        self.diagnostics.push(Diagnostic::new(
            "E0406",
            format!(
                "{} `{}` must return a value of type `{}`",
                context.kind_name(),
                context.name,
                self.types.display(expected)
            ),
            span,
        ));
    }

    fn check_closure_return_capture(&mut self, expr: &Expr, expected: TypeId, scopes: &ScopeStack) {
        if self.active_closures.is_empty() || !self.type_is_move_type(expected) {
            return;
        }
        let Some(binding) = self.expression_root_binding(expr, scopes) else {
            return;
        };
        let Some(active) = self.active_closures.last() else {
            return;
        };
        let Some(index) = active.capture_by_environment.get(&binding.id).copied() else {
            return;
        };
        let mode = active.captures[index].mode;
        if mode == ClosureCaptureMode::Take {
            self.record_binding_use(&binding, expr.span(), CaptureRequirement::Take);
            return;
        }
        self.diagnostics.push(
            Diagnostic::new(
                "E0653",
                "a closure cannot return a borrow rooted in its captured environment",
                expr.span(),
            )
            .with_title("Captured Environment Borrow Cannot Be Returned")
            .with_help(
                "capture the owned value with `take`, or return a value created inside the closure",
            ),
        );
    }

    fn expression_root_binding(&self, expr: &Expr, scopes: &ScopeStack) -> Option<Binding> {
        match expr {
            Expr::Grouped { expr, .. } => self.expression_root_binding(expr, scopes),
            Expr::Variable { name, .. } => scopes.lookup(name).cloned(),
            Expr::This { .. } => scopes.lookup("this").cloned(),
            Expr::PropertyAccess { object, .. }
            | Expr::Index {
                collection: object, ..
            } => self.expression_root_binding(object, scopes),
            _ => None,
        }
    }

    fn is_void_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Void)
    }

    fn requires_return_value(&self, ty: TypeId) -> bool {
        !matches!(self.types.kind(ty), TypeKind::Void | TypeKind::Unknown)
    }

    fn record_expected_expression_type(&mut self, expr: &Expr, expected: TypeId) {
        let span = expr.span();
        self.contextual_expression_types.insert(span, expected);
        if let Expr::Grouped { expr, .. } = expr {
            self.record_expected_expression_type(expr, expected);
        }
    }

    fn record_expected_argument_types(&mut self, params: &[ParamInfo], args: &[Argument]) {
        let param_names = params
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>();
        let param_has_default = params
            .iter()
            .map(|param| param.has_default)
            .collect::<Vec<_>>();
        let arg_names = args
            .iter()
            .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
            .collect::<Vec<_>>();
        let bound =
            crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names);
        for (param_index, arg_index) in bound.param_to_arg.into_iter().enumerate() {
            if let Some(arg_index) = arg_index {
                self.record_expected_expression_type(
                    &args[arg_index].value,
                    params[param_index].ty,
                );
            }
        }
    }

    fn record_function_argument_types(&mut self, name: &str, args: &[Argument]) {
        let params = self
            .functions
            .get(name)
            .map(|function| function.params.clone());
        if let Some(params) = params {
            self.record_expected_argument_types(&params, args);
        }
    }

    fn record_method_argument_types(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        if let TypeKind::List(element) = self.types.kind(object_ty).clone() {
            if matches!(method, "map" | "filter" | "reduce") {
                self.record_list_algorithm_argument_types(
                    method,
                    args,
                    span,
                    element,
                    scopes,
                    method_context,
                );
                return;
            }
        }
        if matches!(self.types.kind(object_ty), TypeKind::String) {
            if let Some((params, _)) = self.string_companion_signature(method) {
                self.record_expected_argument_types(&params[1..], args);
            }
            return;
        }
        let Some(class_type) = self.expr_class_type(object, scopes, method_context) else {
            return;
        };
        let method_info = self
            .classes
            .get(&class_type.name)
            .and_then(|class| class.methods.get(method))
            .cloned();
        if let Some(method_info) = method_info {
            let method_info = self.specialize_method_for_class(&method_info, &class_type);
            self.record_expected_argument_types(&method_info.params, args);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_list_algorithm_argument_types(
        &mut self,
        method: &str,
        args: &[Argument],
        span: Span,
        element: TypeId,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let bool_ty = self.types.intern(TypeKind::Bool);
        let void = self.types.intern(TypeKind::Void);
        let expected_result = self.contextual_expression_types.get(&span).copied();
        let make_callback = |checker: &mut Self, invocation_mode, parameters, return_type| {
            checker
                .types
                .intern(TypeKind::Function(SemanticFunctionType {
                    invocation_mode,
                    parameters,
                    return_type,
                    checked_effects: Vec::new(),
                    return_borrow: None,
                }))
        };
        match (method, args) {
            ("map", [callback]) => {
                let result = expected_result.and_then(|result| match self.types.kind(result) {
                    TypeKind::List(value) => Some(*value),
                    _ => None,
                });
                if let Some(result) = result {
                    let callback_ty = make_callback(
                        self,
                        FunctionInvocationMode::Readonly,
                        vec![SemanticFunctionParameter {
                            ownership_mode: FunctionTypeParameterMode::Readonly,
                            ty: element,
                        }],
                        result,
                    );
                    self.record_expected_expression_type(&callback.value, callback_ty);
                }
            }
            ("filter", [callback]) => {
                let callback_ty = make_callback(
                    self,
                    FunctionInvocationMode::Readonly,
                    vec![SemanticFunctionParameter {
                        ownership_mode: FunctionTypeParameterMode::Readonly,
                        ty: element,
                    }],
                    bool_ty,
                );
                self.record_expected_expression_type(&callback.value, callback_ty);
            }
            ("reduce", [initial, callback]) => {
                let inferred = self.infer_expr_type(&initial.value, scopes, method_context);
                let accumulator = expected_result.or_else(|| {
                    (!matches!(
                        self.types.kind(inferred),
                        TypeKind::Unknown | TypeKind::EmptyCollection
                    ))
                    .then_some(inferred)
                });
                if let Some(accumulator) = accumulator {
                    self.record_expected_expression_type(&initial.value, accumulator);
                    let callback_ty = make_callback(
                        self,
                        FunctionInvocationMode::Readonly,
                        vec![
                            SemanticFunctionParameter {
                                ownership_mode: FunctionTypeParameterMode::Writable,
                                ty: accumulator,
                            },
                            SemanticFunctionParameter {
                                ownership_mode: FunctionTypeParameterMode::Readonly,
                                ty: element,
                            },
                        ],
                        void,
                    );
                    self.record_expected_expression_type(&callback.value, callback_ty);
                }
            }
            _ => {}
        }
    }

    fn record_static_argument_types(
        &mut self,
        qualifier: &StaticQualifier,
        method: &str,
        args: &[Argument],
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_name) = Self::static_qualifier_class_name(qualifier, method_context) else {
            return;
        };
        let params = self
            .classes
            .get(&class_name)
            .and_then(|class| class.methods.get(method))
            .map(|method| method.params.clone());
        if let Some(params) = params {
            self.record_expected_argument_types(&params, args);
        }
    }

    fn record_constructor_argument_types(&mut self, class_type: &TypeRef, args: &[Argument]) {
        let params = self
            .classes
            .get(&class_type.name)
            .and_then(|class| class.methods.get("__construct"))
            .map(|constructor| constructor.params.clone());
        if let Some(params) = params {
            self.record_expected_argument_types(&params, args);
        }
    }

    fn check_expr(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.check_expr_with_range_context(expr, scopes, method_context, false);
    }

    fn check_expr_with_range_context(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        allow_range_expr: bool,
    ) {
        match expr {
            Expr::Closure(closure) => {
                self.check_closure_expression(closure, scopes, method_context);
            }
            Expr::CallableCall {
                callee, args, span, ..
            } => {
                self.check_callable_value_call(callee, args, *span, scopes, method_context);
            }
            Expr::Variable { name, span } => {
                if let Some(binding) = scopes.lookup(name) {
                    self.record_binding_use(binding, *span, CaptureRequirement::Readonly);
                } else {
                    self.undeclared_variable(name, *span);
                }
            }
            Expr::This { span } => {
                if !self.active_closures.is_empty() {
                    if let Some(binding) = scopes.lookup("this") {
                        self.record_binding_use(binding, *span, CaptureRequirement::Readonly);
                    } else {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0644",
                                "this closure has no enclosing method receiver to capture",
                                *span,
                            )
                            .with_title("Closure Has No `$this` Receiver")
                            .with_help("Use `$this` only in a closure nested inside an instance method, and list it in `with (...)`."),
                        );
                    }
                } else if !method_context
                    .is_some_and(|context| context.receiver_access.is_available())
                {
                    self.diagnostics.push(Diagnostic::new(
                        "E0102",
                        "`$this` is only available inside methods",
                        *span,
                    ));
                } else if let Some(binding) = scopes.lookup("this") {
                    self.record_binding_use(binding, *span, CaptureRequirement::Readonly);
                }
            }
            Expr::InterpolatedString { parts, .. } => {
                self.check_interpolated_string(parts, scopes, method_context);
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    if let Some(key) = &element.key {
                        self.check_expr(key, scopes, method_context);
                    }
                    self.check_expr(&element.value, scopes, method_context);
                }
            }
            Expr::ArrayRepeat { value, count, .. } => {
                self.check_expr(value, scopes, method_context);
                self.check_expr(count, scopes, method_context);
                self.check_repeat_count(count, scopes, method_context);
                let element = self.infer_expr_type(value, scopes, method_context);
                self.check_repeat_element_eligibility(element, value.span());
            }
            Expr::Index {
                collection,
                index,
                span,
            } => {
                self.check_expr(collection, scopes, method_context);
                self.check_expr(index, scopes, method_context);
                self.check_collection_index(collection, index, *span, scopes, method_context);
            }
            Expr::PropertyAccess {
                object,
                property,
                member_span,
                null_safe,
                span,
            } => {
                self.check_expr(object, scopes, method_context);
                self.check_mixed_operation(object, "property access", scopes, method_context);
                self.check_nullable_member_access(
                    object,
                    *null_safe,
                    "property access",
                    scopes,
                    method_context,
                );
                let object_ty = self.infer_expr_type(object, scopes, method_context);
                if let Some((kind, _)) = self.shared_handle_type(object_ty, *null_safe) {
                    if kind == SharedHandleKind::SharedReference && property == "referencedValue" {
                        // The compiler-known readonly projection; nothing to look up.
                    } else if self.reject_nonforwarding_shared_handle_member_access(
                        object,
                        property,
                        *null_safe,
                        *span,
                        scopes,
                        method_context,
                    ) {
                    } else {
                        self.lookup_property(object, property, *span, scopes, method_context);
                    }
                } else {
                    let receiver_ty = self.forwarded_access_payload_type(object_ty);
                    let enum_receiver = match self.types.kind(receiver_ty) {
                        TypeKind::Enum(_) => true,
                        TypeKind::Nullable(inner) => {
                            matches!(self.types.kind(*inner), TypeKind::Enum(_))
                        }
                        _ => false,
                    };
                    if enum_receiver
                        || self
                            .compiler_known_property_type(
                                object,
                                property,
                                *null_safe,
                                scopes,
                                method_context,
                            )
                            .is_some()
                    {
                        self.check_compiler_known_property(
                            object,
                            property,
                            *member_span,
                            *span,
                            scopes,
                            method_context,
                        );
                    } else {
                        self.lookup_property(object, property, *span, scopes, method_context);
                    }
                }
            }
            Expr::MethodCall {
                object,
                method,
                member_span,
                span,
                args,
                argument_list_span,
                null_safe,
            } => {
                self.check_expr(object, scopes, method_context);
                self.record_method_argument_types(
                    object,
                    method,
                    args,
                    *span,
                    scopes,
                    method_context,
                );
                for arg in args {
                    self.check_expr(&arg.value, scopes, method_context);
                }
                self.check_mixed_operation(object, "method call", scopes, method_context);
                self.check_nullable_member_access(
                    object,
                    *null_safe,
                    "method call",
                    scopes,
                    method_context,
                );
                if self.check_enum_property_method_call(
                    object,
                    method,
                    *member_span,
                    *argument_list_span,
                    args,
                    scopes,
                    method_context,
                ) {
                    return;
                }
                if self.check_callable_property_call(
                    object,
                    method,
                    args,
                    *null_safe,
                    *member_span,
                    *span,
                    scopes,
                    method_context,
                ) {
                    return;
                }
                if !self.check_string_instance_method_call(
                    object,
                    method,
                    *span,
                    scopes,
                    method_context,
                ) && !self.check_collection_method_call(CollectionMethodCall {
                    object,
                    method,
                    args,
                    member_span: *member_span,
                    argument_list_span: *argument_list_span,
                    span: *span,
                    scopes,
                    method_context,
                }) {
                    self.check_method_call(
                        object,
                        method,
                        args,
                        *null_safe,
                        *span,
                        scopes,
                        method_context,
                    );
                }
            }
            Expr::IsType { expr, ty, span } => {
                self.check_expr(expr, scopes, method_context);
                self.check_is_type(expr, ty, *span, scopes, method_context);
            }
            Expr::FunctionCall { name, args, span } => {
                self.record_function_argument_types(name, args);
                for arg in args {
                    self.check_expr(&arg.value, scopes, method_context);
                }
                self.check_function_call(name, args, *span, scopes, method_context);
            }
            Expr::StaticCall {
                qualifier,
                qualifier_span,
                method,
                member_span,
                member_sigil_span,
                args,
                span,
                ..
            } => {
                self.record_static_argument_types(qualifier, method, args, method_context);
                for arg in args {
                    self.check_expr(&arg.value, scopes, method_context);
                }
                self.check_static_call(
                    StaticAccess {
                        qualifier,
                        qualifier_span: *qualifier_span,
                        member_sigil_span: *member_sigil_span,
                        member: method,
                        member_span: *member_span,
                        span: *span,
                    },
                    args,
                    scopes,
                    method_context,
                );
            }
            Expr::StaticMember {
                qualifier,
                qualifier_span,
                member,
                member_span,
                member_sigil_span,
                span,
            } => {
                self.check_static_member(
                    StaticAccess {
                        qualifier,
                        qualifier_span: *qualifier_span,
                        member_sigil_span: *member_sigil_span,
                        member,
                        member_span: *member_span,
                        span: *span,
                    },
                    method_context,
                );
            }
            Expr::New {
                class_type,
                args,
                shared,
                span,
            } => {
                let class_name = &class_type.name;
                self.record_constructor_argument_types(class_type, args);
                if let Some(kind) = SharedHandleKind::from_source_name(class_name) {
                    for arg in args {
                        self.check_expr(&arg.value, scopes, method_context);
                    }
                    self.check_shared_handle_construction(
                        kind,
                        class_type,
                        args,
                        *shared,
                        *span,
                        scopes,
                        method_context,
                    );
                    return;
                }
                if *shared && !self.check_shared_new_payload(class_type, *span) {
                    for arg in args {
                        self.check_expr(&arg.value, scopes, method_context);
                    }
                    return;
                }
                let is_current_class = class_name == "self"
                    && class_type.arguments.is_empty()
                    && method_context.is_some();
                let class_exists = is_current_class || self.classes.contains_key(class_name);
                if !class_exists {
                    self.diagnostics.push(Diagnostic::new(
                        "E0305",
                        format!("unknown class `{class_name}`"),
                        *span,
                    ));
                }
                for arg in args {
                    self.check_expr(&arg.value, scopes, method_context);
                }
                if class_exists {
                    let resolved = self.resolve_type_ref_with_class(
                        class_type,
                        *span,
                        method_context.map(|context| context.class_name.as_str()),
                    );
                    if let Some(class_type) = self.class_type(resolved) {
                        self.check_constructor_call(
                            &class_type,
                            args,
                            *span,
                            scopes,
                            method_context,
                        );
                    }
                }
            }
            Expr::Grouped { expr, .. } => {
                self.check_expr_with_range_context(expr, scopes, method_context, allow_range_expr)
            }
            Expr::Unary { op, expr, span } => {
                if *op == UnaryOp::Negate {
                    if let (Some(magnitude), Some(operand_span)) = (
                        Self::unsigned_integer_literal_magnitude(expr),
                        Self::unsigned_integer_literal_span(expr),
                    ) {
                        self.negative_integer_literals.insert(*span, magnitude);
                        self.negated_integer_literal_operands.insert(operand_span);
                    }
                }
                self.check_expr(expr, scopes, method_context);
                self.check_unary_operand(op, expr, scopes, method_context);
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                self.check_expr(left, scopes, method_context);
                self.check_expr(right, scopes, method_context);
                self.check_mixed_binary_operands(left, op, right, *span, scopes, method_context);
                self.check_binary_operands(left, op, right, *span, scopes, method_context);
            }
            Expr::Range {
                start, end, span, ..
            } => {
                self.check_expr(start, scopes, method_context);
                self.check_expr(end, scopes, method_context);
                self.check_range_operands(start, end, *span, scopes, method_context);
                if !allow_range_expr {
                    self.diagnostics.push(Diagnostic::new(
                        "E0426",
                        "range expressions are only supported as foreach iterables",
                        *span,
                    ));
                }
            }
            Expr::Match { .. } => self.check_match_expression(expr, scopes, method_context),
            Expr::When(_) => self.check_when_expression(expr, scopes, method_context),
            Expr::Float { .. } => self.check_float_literal_range(expr, FloatType::Float64),
            Expr::Identifier { name, span } => {
                if !self
                    .const_evaluation
                    .values
                    .contains_key(&crate::const_eval::ConstKey::TopLevel(name.clone()))
                {
                    self.diagnostics.push(Diagnostic::new(
                        "E0491",
                        format!("unknown constant `{name}`"),
                        *span,
                    ));
                }
            }
            Expr::String { .. } | Expr::Bool { .. } | Expr::Null { .. } => {}
            Expr::Int { value, span } => {
                if let Some(magnitude) = parse_decimal_magnitude(value) {
                    self.integer_literals.insert(*span, magnitude);
                } else {
                    self.report_integer_literal_range(*span, IntegerType::Int64);
                }
            }
        }
    }

    fn record_binding_use(
        &mut self,
        binding: &Binding,
        span: Span,
        requirement: CaptureRequirement,
    ) {
        self.binding_resolution
            .uses_by_span
            .insert(span, binding.id);
        let Some(active) = self.active_closures.last_mut() else {
            return;
        };
        if binding.owner == LexicalOwner::Closure(active.id) {
            if let Some(index) = active.capture_by_environment.get(&binding.id).copied() {
                let capture = &mut active.captures[index];
                capture.first_use_span.get_or_insert(span);
                capture.use_spans.push(span);
                capture.required_capability = capture.required_capability.max(requirement);
            }
            return;
        }
        let missing = active
            .missing
            .entry(binding.id)
            .or_insert_with(|| MissingCaptureDraft {
                source_binding_id: binding.id,
                first_use_span: span,
                use_spans: Vec::new(),
                required_capability: requirement,
            });
        missing.use_spans.push(span);
        missing.required_capability = missing.required_capability.max(requirement);
    }

    fn check_callable_value_call(
        &mut self,
        callee: &Expr,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        self.check_expr(callee, scopes, method_context);
        for argument in args {
            self.check_expr(&argument.value, scopes, method_context);
        }
        let callee_ty = self.infer_expr_type(callee, scopes, method_context);
        self.check_callable_signature(
            callee,
            callee_ty,
            args,
            span,
            CallableValueTargetKind::Value,
            scopes,
            method_context,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn check_callable_property_call(
        &mut self,
        object: &Expr,
        member: &str,
        args: &[Argument],
        null_safe: bool,
        member_span: Span,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let Some(class_type) = self.expr_class_type(object, scopes, method_context) else {
            return false;
        };
        let Some(class) = self.classes.get(&class_type.name) else {
            return false;
        };
        if class.methods.contains_key(member) {
            return false;
        }
        let Some(property) = class.properties.get(member).cloned() else {
            return false;
        };
        let property = self.specialize_property_for_class(&property, &class_type);
        if self.non_null_function_type(property.ty).is_none() {
            return false;
        }
        let property_expr = Expr::PropertyAccess {
            object: Box::new(object.clone()),
            property: member.to_string(),
            member_span,
            null_safe: false,
            span: object.span().merge(member_span),
        };
        if null_safe {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0652",
                    "null-safe member-call syntax cannot invoke a callable property",
                    span,
                )
                .with_title("Callable Property Has No Null-Safe Call Form")
                .with_help("narrow the receiver and callable property, then invoke it with ordinary `->` syntax"),
            );
            return true;
        }
        self.lookup_property(object, member, span, scopes, method_context);
        self.check_callable_signature(
            &property_expr,
            property.ty,
            args,
            span,
            CallableValueTargetKind::Property,
            scopes,
            method_context,
        );
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn check_callable_signature(
        &mut self,
        callee: &Expr,
        callee_ty: TypeId,
        args: &[Argument],
        span: Span,
        target_kind: CallableValueTargetKind,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let function = match self.types.kind(callee_ty).clone() {
            TypeKind::Function(function) => function,
            TypeKind::Nullable(inner)
                if matches!(self.types.kind(inner), TypeKind::Function(_)) =>
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0650",
                        "nullable function value cannot be invoked before narrowing",
                        callee.span(),
                    )
                    .with_title("Nullable Callable Must Be Narrowed")
                    .with_help("prove the value is non-null before invoking it"),
                );
                return self.types.unknown();
            }
            TypeKind::Unknown => return self.types.unknown(),
            _ => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0649",
                        format!(
                            "value of type `{}` is not callable",
                            self.types.display(callee_ty)
                        ),
                        callee.span(),
                    )
                    .with_title("Value Is Not Callable")
                    .with_help("invoke a value whose semantic type is `function(...): ReturnType`"),
                );
                return self.types.unknown();
            }
        };

        for argument in args {
            if let Some(name) = &argument.name {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0652",
                        "function values accept positional arguments only",
                        name.span,
                    )
                    .with_title("Callable Argument Cannot Be Named")
                    .with_help("remove the argument name and preserve argument order"),
                );
            }
        }

        if args.len() != function.parameters.len() {
            self.report_argument_count_mismatch(
                "function value",
                function.parameters.len(),
                function.parameters.len(),
                args.len(),
                span,
            );
        } else {
            for (index, (argument, parameter)) in args.iter().zip(&function.parameters).enumerate()
            {
                let actual = self.infer_expr_type(&argument.value, scopes, method_context);
                if !self.is_expr_assignable(parameter.ty, &argument.value, scopes, method_context)
                    && !self.is_assignable(parameter.ty, actual)
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0408",
                            format!(
                                "argument {} of function value expects `{}`, got `{}`",
                                index + 1,
                                self.types.display(parameter.ty),
                                self.types.display(actual)
                            ),
                            argument.value.span(),
                        )
                        .with_title("Callable Argument Type Mismatch"),
                    );
                    continue;
                }
                match parameter.ownership_mode {
                    FunctionTypeParameterMode::Readonly => {}
                    FunctionTypeParameterMode::Writable => {
                        self.record_capture_requirement_for_expr(
                            &argument.value,
                            scopes,
                            CaptureRequirement::Writable,
                        );
                        if !self.is_writable_object_path(&argument.value, scopes, method_context) {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0651",
                                    format!("argument {} requires writable access", index + 1),
                                    argument.value.span(),
                                )
                                .with_title("Writable Callable Argument Requires Writable Access"),
                            );
                        }
                    }
                    FunctionTypeParameterMode::Take => {
                        if self.type_is_move_type(actual) {
                            self.record_capture_requirement_for_expr(
                                &argument.value,
                                scopes,
                                CaptureRequirement::Take,
                            );
                            if !self.expression_provides_owned_value(&argument.value, scopes) {
                                self.diagnostics.push(
                                    Diagnostic::new(
                                        "E0645",
                                        format!("argument {} requires an owned value", index + 1),
                                        argument.value.span(),
                                    )
                                    .with_title("Taking Callable Argument Requires Ownership"),
                                );
                            }
                        }
                    }
                }
            }
        }

        let requirement = match function.invocation_mode {
            FunctionInvocationMode::Readonly => CaptureRequirement::Readonly,
            FunctionInvocationMode::Writable => CaptureRequirement::Writable,
            FunctionInvocationMode::Once => CaptureRequirement::Take,
        };
        self.record_capture_requirement_for_expr(callee, scopes, requirement);
        if function.invocation_mode != FunctionInvocationMode::Readonly
            && !self.callable_access_is_sufficient(
                callee,
                function.invocation_mode,
                scopes,
                method_context,
            )
        {
            if function.invocation_mode == FunctionInvocationMode::Once
                && matches!(
                    Self::ungroup_expr(callee),
                    Expr::PropertyAccess { .. } | Expr::Index { .. }
                )
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0660",
                        "once function cannot be consumed from a property or aggregate slot",
                        callee.span(),
                    )
                    .with_title("Once Function Cannot Be Consumed From Stored Place")
                    .with_help(
                        "first obtain an owned local through an ownership-transferring operation",
                    ),
                );
            } else {
                let access = match function.invocation_mode {
                    FunctionInvocationMode::Readonly => "readonly",
                    FunctionInvocationMode::Writable => "writable",
                    FunctionInvocationMode::Once => "owned",
                };
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0651",
                        format!("this function value requires {access} invocation access"),
                        callee.span(),
                    )
                    .with_title("Callable Invocation Requires Stronger Access")
                    .with_help(format!("invoke it through a {access} callable place")),
                );
            }
        }

        let complete_effects =
            self.complete_function_value_effects(&function.checked_effects, span);
        self.record_checked_effects(complete_effects.iter().copied(), span);
        let resolved_function = self.types.resolved(callee_ty);
        let resolved_effects = complete_effects
            .iter()
            .map(|effect| self.types.resolved(*effect))
            .collect::<Vec<_>>();
        let effect_profile =
            crate::checked_effects::CheckedEffectProfile::classify(resolved_effects.clone());
        self.callable_value_calls.insert(
            span,
            CallableValueCallInfo {
                function_type: resolved_function,
                invocation_mode: function.invocation_mode,
                return_type: self.types.resolved(function.return_type),
                checked_effects: resolved_effects,
                required_checked_effects: effect_profile.required,
                ambient_checked_effects: effect_profile.ambient,
                target_kind,
            },
        );
        function.return_type
    }

    fn expression_provides_owned_value(&self, expr: &Expr, scopes: &ScopeStack) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => self.expression_provides_owned_value(expr, scopes),
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .is_some_and(|binding| binding.ownership == BindingOwnership::Owned),
            Expr::This { .. } | Expr::PropertyAccess { .. } | Expr::Index { .. } => false,
            _ => true,
        }
    }

    fn callable_access_is_sufficient(
        &mut self,
        callee: &Expr,
        mode: FunctionInvocationMode,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        match mode {
            FunctionInvocationMode::Readonly => true,
            FunctionInvocationMode::Writable => {
                self.is_writable_object_path(callee, scopes, method_context)
                    || matches!(callee, Expr::Closure(_) | Expr::CallableCall { .. })
            }
            FunctionInvocationMode::Once => self.expression_provides_owned_value(callee, scopes),
        }
    }

    fn check_closure_expression(
        &mut self,
        closure: &ClosureExpression,
        outer_scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let key = closure.span;
        let closure_id = ClosureId::from_span(closure.span);
        let previous_owner = std::mem::replace(
            &mut self.current_lexical_owner,
            LexicalOwner::Closure(closure_id),
        );
        self.binding_resolution
            .closure_owners
            .insert(closure_id, previous_owner);
        self.binding_resolution
            .lexical_parents
            .insert(LexicalOwner::Closure(closure_id), previous_owner);

        let declaring_class = method_context.map(|context| context.class_name.as_str());
        let parameter_names = closure
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<HashSet<_>>();
        let own_locals = closure_local_declarations(&closure.body);
        let mut closure_scopes = outer_scopes.clone();
        closure_scopes.push();
        let mut captures = Vec::new();
        let mut capture_by_environment = HashMap::new();
        let mut seen_sources = HashMap::<BindingId, Span>::new();

        if let Some(clause) = &closure.captures {
            for capture in &clause.captures {
                if parameter_names.contains(capture.name.as_str())
                    || own_locals.contains_key(capture.name.as_str())
                {
                    let related = closure
                        .parameters
                        .iter()
                        .find(|parameter| parameter.name == capture.name)
                        .map(|parameter| parameter.name_span)
                        .or_else(|| own_locals.get(capture.name.as_str()).copied());
                    let mut diagnostic = Diagnostic::new(
                        "E0644",
                        format!(
                            "`${}` belongs to this closure and cannot be captured from an enclosing scope",
                            capture.name
                        ),
                        capture.span,
                    )
                    .with_title("Closure Cannot Capture Its Own Binding");
                    if let Some(related) = related {
                        diagnostic =
                            diagnostic.with_related(related, "the inner binding is declared here");
                    }
                    self.diagnostics.push(diagnostic);
                    continue;
                }

                let source = outer_scopes.lookup(&capture.name).cloned();
                let Some(source) = source else {
                    if let Some(initializer) = self
                        .initializing_bindings
                        .last()
                        .and_then(|bindings| bindings.get(&capture.name))
                        .copied()
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0647",
                                format!(
                                    "closure initializer cannot capture `${}` before that binding exists",
                                    capture.name
                                ),
                                capture.span,
                            )
                            .with_title("Closure Cannot Capture Its Initializing Binding")
                            .with_related(initializer, "this binding is still being initialized")
                            .with_help("Use a named function for recursion; recursive closure syntax is not available."),
                        );
                    } else {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0644",
                                format!("`${}` is not an enclosing lexical binding", capture.name),
                                capture.span,
                            )
                            .with_title("Capture Does Not Name An Enclosing Binding")
                            .with_help("Capture a parameter or local declared in an enclosing lexical scope."),
                        );
                    }
                    continue;
                };
                if let Some(first) = seen_sources.insert(source.id, capture.span) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0643",
                            format!("`${}` is captured more than once", capture.name),
                            capture.span,
                        )
                        .with_title("Duplicate Closure Capture")
                        .with_related(first, "the first capture entry is here")
                        .with_help("Keep exactly one capture entry for this binding."),
                    );
                    continue;
                }
                if source.kind == BindingKind::MethodReceiver
                    && capture.mode == ClosureCaptureMode::Take
                {
                    self.diagnostics.push(
                        Diagnostic::new("E0644", "`$this` cannot be captured with `take`", capture.span)
                            .with_title("Method Receiver Cannot Be Taken")
                            .with_help("Capture `$this` readonly, or use `writable $this` in a writable method."),
                    );
                    continue;
                }
                if capture.mode == ClosureCaptureMode::Writable && !source.writable {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0645",
                            format!(
                                "readonly binding `${}` cannot provide a writable capture",
                                capture.name
                            ),
                            capture.span,
                        )
                        .with_title("Writable Capture Requires Writable Source")
                        .with_related(
                            self.binding_resolution
                                .declarations_by_id
                                .get(&source.id)
                                .and_then(|declaration| declaration.span)
                                .unwrap_or(capture.span),
                            "the source binding is readonly here",
                        ),
                    );
                    continue;
                }
                if capture.mode == ClosureCaptureMode::Take
                    && self.type_is_move_type(source.ty)
                    && source.ownership != BindingOwnership::Owned
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0645",
                            format!(
                                "borrowed move value `${}` cannot be taken into a closure",
                                capture.name
                            ),
                            capture.span,
                        )
                        .with_title("Taking Capture Requires Ownership")
                        .with_help("Take an owned local, or capture this borrowed value readonly."),
                    );
                    continue;
                }

                // A nested closure captures through its immediate parent
                // environment. Recording that use here preserves the lineage
                // instead of letting an inner closure point into a grandparent
                // frame by name.
                self.record_binding_use(
                    &source,
                    capture.span,
                    match capture.mode {
                        ClosureCaptureMode::Readonly => CaptureRequirement::Readonly,
                        ClosureCaptureMode::Writable => CaptureRequirement::Writable,
                        ClosureCaptureMode::Take => CaptureRequirement::Take,
                    },
                );

                let ownership = match capture.mode {
                    ClosureCaptureMode::Readonly => BindingOwnership::ReadonlyBorrow,
                    ClosureCaptureMode::Writable => BindingOwnership::WritableBorrow,
                    ClosureCaptureMode::Take => BindingOwnership::Owned,
                };
                self.declare_binding(
                    &mut closure_scopes,
                    capture.name.clone(),
                    Binding::unresolved(
                        capture.mode != ClosureCaptureMode::Readonly,
                        source.ty,
                        source.declared_ty,
                        source.int_constant,
                        source.string_constant.clone(),
                    ),
                    capture.span,
                    BindingKind::ClosureCapture,
                    ownership,
                );
                let environment = closure_scopes
                    .lookup(&capture.name)
                    .expect("declared closure capture")
                    .id;
                let index = captures.len();
                capture_by_environment.insert(environment, index);
                captures.push(CaptureDraft {
                    source_binding_id: source.id,
                    environment_binding_id: environment,
                    mode: capture.mode,
                    declaration_span: capture.span,
                    first_use_span: None,
                    use_spans: Vec::new(),
                    source_type: source.ty,
                    required_capability: CaptureRequirement::Readonly,
                });
            }
        }

        let parameters = closure
            .parameters
            .iter()
            .map(|parameter| {
                let ty = self.resolve_type_ref_in_position(
                    &parameter.ty,
                    parameter.type_span,
                    TypePosition::Value,
                    declaring_class,
                );
                let ownership_mode = if parameter.take {
                    FunctionTypeParameterMode::Take
                } else if parameter.writable {
                    FunctionTypeParameterMode::Writable
                } else {
                    FunctionTypeParameterMode::Readonly
                };
                self.declare_binding(
                    &mut closure_scopes,
                    parameter.name.clone(),
                    Binding::unresolved(parameter.writable, ty, ty, None, None),
                    parameter.name_span,
                    BindingKind::ClosureParameter,
                    match ownership_mode {
                        FunctionTypeParameterMode::Readonly => BindingOwnership::ReadonlyBorrow,
                        FunctionTypeParameterMode::Writable => BindingOwnership::WritableBorrow,
                        FunctionTypeParameterMode::Take => BindingOwnership::Owned,
                    },
                );
                SemanticFunctionParameter { ownership_mode, ty }
            })
            .collect::<Vec<_>>();

        self.active_closures.push(ActiveClosure {
            id: closure_id,
            captures,
            capture_by_environment,
            missing: HashMap::new(),
        });
        let receiver_capture = closure_scopes.lookup("this").cloned().filter(|binding| {
            binding.owner == LexicalOwner::Closure(closure_id)
                && binding.kind == BindingKind::ClosureCapture
        });
        let closure_method_context = method_context.map(|context| MethodContext {
            class_name: context.class_name.clone(),
            receiver_access: receiver_capture.as_ref().map_or(
                ReceiverAccess::Unavailable,
                |binding| {
                    if binding.writable {
                        ReceiverAccess::Writable
                    } else {
                        ReceiverAccess::Readonly
                    }
                },
            ),
        });
        let closure_method_context = closure_method_context.as_ref();

        let expected_function = self
            .contextual_expression_types
            .get(&key)
            .copied()
            .and_then(|expected| self.non_null_function_type(expected).cloned());
        let written_return = closure.return_type.as_ref().map(|return_type| {
            self.resolve_type_ref_in_position(
                &return_type.ty,
                return_type.type_span,
                TypePosition::Return,
                declaring_class,
            )
        });
        let expected_return = written_return.or_else(|| {
            expected_function
                .as_ref()
                .map(|function| function.return_type)
        });

        self.effect_scopes.push(CheckedEffectSet::default());
        let inferred_return = match &closure.body {
            ClosureBody::Expression { expression, .. } => {
                if let Some(expected) = expected_return {
                    self.record_expected_expression_type(expression, expected);
                }
                self.check_expr(expression, &closure_scopes, closure_method_context);
                let inferred =
                    self.infer_expr_type(expression, &closure_scopes, closure_method_context);
                if let Some(expected) = expected_return {
                    if !self.is_expr_assignable(
                        expected,
                        expression,
                        &closure_scopes,
                        closure_method_context,
                    ) && !self.is_assignable(expected, inferred)
                    {
                        self.report_closure_return_mismatch(expected, inferred, expression.span());
                    }
                    expected
                } else {
                    inferred
                }
            }
            ClosureBody::Block(block) => {
                let return_context = ReturnContext {
                    name: format!("closure at {}", closure.span.start),
                    expected: expected_return,
                    lifecycle: None,
                    is_method: false,
                };
                self.check_block(
                    block,
                    &mut closure_scopes,
                    closure_method_context,
                    None,
                    Some(&return_context),
                    0,
                );
                let inferred = expected_return.unwrap_or_else(|| {
                    self.infer_closure_block_return_type(block)
                        .unwrap_or_else(|| self.types.intern(TypeKind::Void))
                });
                if !self.is_void_type(inferred)
                    && crate::return_analysis::analyze_block_with_given(
                        block,
                        closure.span,
                        &self.given_preludes,
                    )
                    .fallthrough_reachable
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0406",
                            format!(
                                "closure must return a value of type `{}` on every path",
                                self.types.display(inferred)
                            ),
                            closure.span,
                        )
                        .with_title("Closure Must Return On Every Path"),
                    );
                }
                inferred
            }
        };
        let inferred_effects = self.effect_scopes.pop().expect("closure effect scope");
        let mut active = self.active_closures.pop().expect("active closure");

        let invocation_mode = active
            .captures
            .iter()
            .map(|capture| capture.required_capability)
            .max()
            .map_or(
                FunctionInvocationMode::Readonly,
                |requirement| match requirement {
                    CaptureRequirement::Readonly => FunctionInvocationMode::Readonly,
                    CaptureRequirement::Writable => FunctionInvocationMode::Writable,
                    CaptureRequirement::Take => FunctionInvocationMode::Once,
                },
            );
        self.validate_capture_requirements(closure, &mut active);
        self.report_missing_captures(closure, &active);
        self.report_unused_captures(closure, &active);

        let inferred_resolved_effects = inferred_effects
            .ordered
            .iter()
            .map(|effect| self.types.resolved(*effect))
            .collect::<Vec<_>>();
        let _ = self.complete_function_value_effects(&[], closure.span);
        let effect_profile = crate::checked_effects::CheckedEffectProfile::classify(
            inferred_resolved_effects.clone(),
        );
        let mut normalized_effects = effect_profile
            .required
            .iter()
            .map(|effect| self.types.intern_resolved(effect))
            .collect::<Vec<_>>();
        normalized_effects.sort_by_key(|effect| self.types.display(*effect));
        normalized_effects.dedup();
        let return_borrow = self
            .type_can_return_borrow(inferred_return)
            .then(|| {
                self.infer_closure_return_borrow(closure, &closure_scopes, closure_method_context)
            })
            .flatten();
        let semantic = SemanticFunctionType {
            invocation_mode,
            parameters,
            return_type: inferred_return,
            checked_effects: normalized_effects,
            return_borrow,
        };
        let ty = self.types.intern(TypeKind::Function(semantic.clone()));
        let execution_semantic = expected_function
            .filter(|expected| {
                self.function_type_compatibility(expected, &semantic)
                    .is_ok()
            })
            .unwrap_or_else(|| semantic.clone());
        let execution_ty = self.types.intern(TypeKind::Function(execution_semantic));
        self.closure_types.insert(key, ty);

        let capture_info = active
            .captures
            .into_iter()
            .map(|capture| CaptureSemanticInfo {
                source_binding_id: capture.source_binding_id,
                environment_binding_id: capture.environment_binding_id,
                mode: capture.mode,
                declaration_span: capture.declaration_span,
                first_use_span: capture.first_use_span,
                use_spans: capture.use_spans,
                source_type: self.types.resolved(capture.source_type),
                required_capability: capture.required_capability,
            })
            .collect();
        self.closures.insert(
            closure_id,
            ClosureSemanticInfo {
                closure_id,
                function_type: self.types.resolved(ty),
                execution_function_type: self.types.resolved(execution_ty),
                captures: capture_info,
                inferred_invocation_mode: invocation_mode,
                inferred_checked_effects: inferred_resolved_effects,
                required_checked_effects: effect_profile.required,
                ambient_checked_effects: effect_profile.ambient,
                inferred_return_type: self.types.resolved(inferred_return),
                execution_boundary_span: closure.span,
            },
        );

        self.current_lexical_owner = previous_owner;
        ty
    }

    fn infer_closure_return_borrow(
        &mut self,
        closure: &ClosureExpression,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<FunctionReturnBorrow> {
        let body = match &closure.body {
            ClosureBody::Expression { expression, .. } => Block {
                statements: vec![Stmt::Return {
                    expr: Some((**expression).clone()),
                    span: expression.span(),
                }],
                span: closure.span,
            },
            ClosureBody::Block(block) => block.clone(),
        };
        let parameters = closure
            .parameters
            .iter()
            .map(|parameter| Param {
                promoted_access: None,
                take: parameter.take,
                take_span: parameter.take_span,
                writable: parameter.writable,
                writable_span: parameter.writable_span,
                ownership_modifier_insert: parameter.type_span,
                ty: parameter.ty.clone(),
                name: parameter.name.clone(),
                default: None,
                span: parameter.span,
            })
            .collect();
        let mut seen_type_parameters = HashSet::new();
        let type_params = self
            .type_parameter_scopes
            .iter()
            .flat_map(|scope| scope.iter())
            .filter(|(name, _)| seen_type_parameters.insert((*name).clone()))
            .map(|(name, constraints)| TypeParamDecl {
                name: name.clone(),
                constraints: constraints.clone(),
                default_type: None,
                span: closure.span,
            })
            .collect();
        let function = FunctionDecl {
            access: MemberAccess::External,
            access_span: None,
            writable_this: false,
            writable_span: None,
            is_static: true,
            static_span: None,
            name: "<closure>".to_string(),
            name_span: closure.keyword_span,
            type_params,
            params: parameters,
            return_type: closure
                .return_type
                .as_ref()
                .map(|return_type| return_type.ty.clone()),
            throws: None,
            body,
            span: closure.span,
        };
        let borrow =
            crate::ownership::function_return_borrow_in_context(&function, &[], &mut |call| {
                self.call_return_borrow(call, scopes, method_context)
            })?;
        match borrow.source {
            BorrowSource::Parameter(index) => Some(FunctionReturnBorrow {
                source: FunctionBorrowSource::Parameter(index),
                writable: borrow.writable,
            }),
            BorrowSource::Receiver => None,
        }
    }

    fn validate_capture_requirements(
        &mut self,
        _closure: &ClosureExpression,
        active: &mut ActiveClosure,
    ) {
        for capture in &active.captures {
            let sufficient = match capture.required_capability {
                CaptureRequirement::Readonly => true,
                CaptureRequirement::Writable => matches!(
                    capture.mode,
                    ClosureCaptureMode::Writable | ClosureCaptureMode::Take
                ),
                CaptureRequirement::Take => capture.mode == ClosureCaptureMode::Take,
            };
            if sufficient || capture.first_use_span.is_none() {
                continue;
            }
            let declaration = self
                .binding_resolution
                .declarations_by_id
                .get(&capture.source_binding_id);
            let name = declaration.map_or("value", |declaration| declaration.name.as_str());
            let needed = match capture.required_capability {
                CaptureRequirement::Readonly => "readonly",
                CaptureRequirement::Writable => "writable",
                CaptureRequirement::Take => "take",
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0645",
                    format!("closure use of `${name}` requires a {needed} capture"),
                    capture.first_use_span.unwrap_or(capture.declaration_span),
                )
                .with_title("Closure Capture Mode Is Too Weak")
                .with_related(
                    capture.declaration_span,
                    "this capture mode is declared here",
                )
                .with_help(match capture.required_capability {
                    CaptureRequirement::Readonly => "capture the binding as written".to_string(),
                    CaptureRequirement::Writable => {
                        format!("write `with (writable ${name})` when the source is writable")
                    }
                    CaptureRequirement::Take => {
                        format!("write `with (take ${name})` only when ownership should transfer")
                    }
                }),
            );
        }
    }

    fn report_missing_captures(&mut self, closure: &ClosureExpression, active: &ActiveClosure) {
        let mut missing = active.missing.values().collect::<Vec<_>>();
        missing.sort_by_key(|entry| entry.first_use_span.start);
        let safe_insertions = missing
            .iter()
            .filter_map(|entry| {
                let declaration = self
                    .binding_resolution
                    .declarations_by_id
                    .get(&entry.source_binding_id)?;
                let mode = match entry.required_capability {
                    CaptureRequirement::Readonly => "",
                    CaptureRequirement::Writable if declaration.writable => "writable ",
                    CaptureRequirement::Writable | CaptureRequirement::Take => return None,
                };
                Some(format!("{mode}${}", declaration.name))
            })
            .collect::<Vec<_>>();

        for (index, entry) in missing.into_iter().enumerate() {
            let Some(declaration) = self
                .binding_resolution
                .declarations_by_id
                .get(&entry.source_binding_id)
                .cloned()
            else {
                continue;
            };
            let mut diagnostic = Diagnostic::new(
                "E0642",
                format!("closure must capture `${}`", declaration.name),
                entry.first_use_span,
            )
            .with_title(format!("Closure Must Capture `${}`", declaration.name))
            .with_cause(format!(
                "closure:{}:{}:binding:{}",
                closure.span.start, closure.span.end, declaration.id.0
            ))
            .with_help(
                "list every enclosing binding used by the closure in its `with (...)` clause",
            );
            if let Some(declaration_span) = declaration.span {
                diagnostic = diagnostic
                    .with_related(declaration_span, "the enclosing binding is declared here");
            }
            if index == 0 && safe_insertions.len() == active.missing.len() {
                let joined = safe_insertions.join(", ");
                diagnostic = if let Some(clause) = &closure.captures {
                    if let Some(edit) = self.safe_capture_extension_edit(clause, &joined) {
                        diagnostic.with_structured_fix(
                            "Add Missing Closure Captures",
                            FixApplicability::MachineApplicable,
                            vec![edit],
                        )
                    } else {
                        diagnostic
                    }
                } else {
                    let insertion = closure
                        .return_type
                        .as_ref()
                        .map_or(closure.parameter_list_span.end, |return_type| {
                            return_type.span.end
                        });
                    diagnostic.with_structured_fix(
                        "Add Missing Closure Captures",
                        FixApplicability::MachineApplicable,
                        vec![FixEdit {
                            source: DiagnosticSource::Current,
                            span: Span::new(insertion, insertion),
                            replacement: format!(" with ({joined})"),
                        }],
                    )
                };
            }
            self.diagnostics.push(diagnostic);
        }
    }

    fn report_unused_captures(&mut self, closure: &ClosureExpression, active: &ActiveClosure) {
        let Some(clause) = &closure.captures else {
            return;
        };
        for capture in active
            .captures
            .iter()
            .filter(|capture| capture.first_use_span.is_none())
        {
            let declaration = self
                .binding_resolution
                .declarations_by_id
                .get(&capture.source_binding_id);
            let name = declaration.map_or("value", |declaration| declaration.name.as_str());
            let mut diagnostic = Diagnostic::new(
                "E0646",
                format!("closure captures `${name}` but never uses it"),
                capture.declaration_span,
            )
            .with_severity(crate::diagnostics::DiagnosticSeverity::Warning)
            .with_title("Unused Closure Capture")
            .with_help("remove the unused capture entry");
            if let Some(removal) = self.safe_capture_removal_span(clause, capture.declaration_span)
            {
                diagnostic = diagnostic.with_fix(removal, "");
            }
            self.diagnostics.push(diagnostic);
        }
    }

    fn safe_capture_removal_span(
        &self,
        clause: &ClosureCaptureClause,
        capture_span: Span,
    ) -> Option<Span> {
        let clause_text = self.source_slice(clause.span)?;
        if contains_comment(clause_text) {
            return None;
        }
        if clause.captures.len() == 1 {
            return Some(clause.span);
        }
        let index = clause
            .captures
            .iter()
            .position(|capture| capture.span == capture_span)?;
        let removal = if let Some(next) = clause.captures.get(index + 1) {
            Span::in_source(capture_span.source, capture_span.start, next.span.start)
        } else {
            let previous = clause.captures.get(index.checked_sub(1)?)?;
            Span::in_source(capture_span.source, previous.span.end, capture_span.end)
        };
        let text = self.source_slice(removal)?;
        (!contains_comment(text)).then_some(removal)
    }

    fn safe_capture_extension_edit(
        &self,
        clause: &ClosureCaptureClause,
        insertion: &str,
    ) -> Option<FixEdit> {
        let last = clause.captures.last()?;
        let tail = self
            .source_texts
            .get(&last.span.source)?
            .get(last.span.end..clause.close_span.start)?;
        if contains_comment(tail) {
            return None;
        }
        let offset = if tail.contains(',') {
            last.span.end
        } else {
            clause.close_span.start
        };
        Some(FixEdit {
            source: DiagnosticSource::Current,
            span: Span::in_source(last.span.source, offset, offset),
            replacement: format!(", {insertion}"),
        })
    }

    fn resolve_function_type_ref(
        &mut self,
        function: &FunctionTypeRef,
        declaring_class: Option<&str>,
    ) -> TypeId {
        let _ = self.complete_function_value_effects(&[], function.span);
        let parameters = function
            .parameters
            .iter()
            .map(|parameter| SemanticFunctionParameter {
                ownership_mode: parameter.ownership_mode,
                ty: self.resolve_type_ref_in_position(
                    &parameter.ty,
                    parameter.type_span,
                    TypePosition::Value,
                    declaring_class,
                ),
            })
            .collect::<Vec<_>>();
        let return_type = self.resolve_type_ref_in_position(
            &function.return_type,
            function.return_type_span,
            TypePosition::Return,
            declaring_class,
        );

        let mut authored_checked_effects = Vec::new();
        let mut checked_effects = Vec::new();
        let mut ambient_checked_effects = Vec::new();
        let mut saw_error = false;
        if let Some(clause) = &function.throws_clause {
            for entry in &clause.entries {
                if entry.ty.nullable {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0619",
                            format!("throws type `{}` cannot be nullable", entry.ty),
                            entry.span,
                        )
                        .with_title("Throws Type Cannot Be Nullable"),
                    );
                    continue;
                }
                let effect = self.resolve_type_ref_in_position(
                    &entry.ty,
                    entry.type_span,
                    TypePosition::Value,
                    declaring_class,
                );
                if self.is_unknown_type(effect) {
                    continue;
                }
                if !self.type_implements_error(effect) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0618",
                            format!(
                                "throws type `{}` does not implement `Error`",
                                self.types.display(effect)
                            ),
                            entry.span,
                        )
                        .with_title("Throws Type Must Implement Error"),
                    );
                    continue;
                }
                if authored_checked_effects.contains(&effect) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0620",
                            format!("duplicate throws entry `{}`", self.types.display(effect)),
                            entry.span,
                        )
                        .with_title("Duplicate Throws Entry"),
                    );
                    continue;
                }
                if saw_error
                    || (matches!(self.types.kind(effect), TypeKind::Error)
                        && !authored_checked_effects.is_empty())
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0621",
                            "`Error` already covers every concrete checked error in this throws list",
                            entry.span,
                        )
                        .with_title("Error Already Covers This Throws Entry"),
                    );
                    continue;
                }
                saw_error = matches!(self.types.kind(effect), TypeKind::Error);
                authored_checked_effects.push(effect);
                let resolved = self.types.resolved(effect);
                if crate::checked_effects::is_ambient_io_effect(&resolved) {
                    ambient_checked_effects.push(resolved);
                } else {
                    checked_effects.push(effect);
                }
            }
        }
        checked_effects.sort_by_key(|effect| self.types.display(*effect));
        checked_effects.dedup();

        let borrowed_parameters = parameters
            .iter()
            .enumerate()
            .filter(|(_, parameter)| {
                parameter.ownership_mode != FunctionTypeParameterMode::Take
                    && parameter.ty == return_type
                    && self.type_is_move_type(parameter.ty)
            })
            .map(|(index, parameter)| (index, parameter.ownership_mode))
            .collect::<Vec<_>>();
        let return_borrow = match borrowed_parameters.as_slice() {
            [(index, mode)] => Some(FunctionReturnBorrow {
                source: FunctionBorrowSource::Parameter(*index),
                writable: *mode == FunctionTypeParameterMode::Writable,
            }),
            _ => None,
        };
        let semantic = SemanticFunctionType {
            invocation_mode: function.invocation_mode,
            parameters,
            return_type,
            checked_effects,
            return_borrow,
        };
        let ty = self.types.intern(TypeKind::Function(semantic));
        self.function_types_by_span.insert(
            function.span,
            FunctionTypeSemanticInfo {
                ty: self.types.resolved(ty),
                authored_checked_effects: authored_checked_effects
                    .into_iter()
                    .map(|effect| self.types.resolved(effect))
                    .collect(),
                ambient_checked_effects,
            },
        );
        ty
    }

    fn check_match_expression(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Expr::Match {
            scrutinee,
            mode,
            arms,
            origin,
            span,
        } = expr
        else {
            unreachable!("match checking requires a match expression")
        };
        self.check_expr(scrutinee, scopes, method_context);
        let scrutinee_ty = self.infer_expr_type(scrutinee, scopes, method_context);
        if let MatchMode::Consumed { take_span } = *mode {
            if !self.type_is_move_type(scrutinee_ty) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0600",
                        "Copy match scrutinee does not need `take`",
                        take_span,
                    )
                    .with_title("Copy Match Does Not Need Take")
                    .with_help("remove `take`; Copy values remain available after matching")
                    .with_fix(Span::new(take_span.start, scrutinee.span().start), ""),
                );
            }
        }
        let condition_mode = *origin == MatchOrigin::Match
            && matches!(
                Self::ungroup_expr(scrutinee),
                Expr::Bool { value: true, .. }
            );

        if *origin == MatchOrigin::Ternary
            && !matches!(
                self.types.kind(scrutinee_ty),
                TypeKind::Bool | TypeKind::Unknown
            )
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0595",
                    "ternary condition must have type `bool`",
                    scrutinee.span(),
                )
                .with_title("Ternary Condition Must Be Bool")
                .with_help("use an explicit comparison that produces `bool`"),
            );
        }

        let expected = self.contextual_expression_types.get(span).copied();
        let mut resolved_arms = Vec::with_capacity(arms.len());
        let mut result_ty = expected;
        let mut seen_default = false;
        let mut default_span = None;
        let mut seen_enum_cases = HashSet::new();
        let mut seen_constants = Vec::new();
        let mut seen_types = HashSet::new();
        let mut seen_null = false;
        let mut shape_valid = true;

        for (index, arm) in arms.iter().enumerate() {
            let pattern_span = Self::match_pattern_span(&arm.pattern);
            let guard_info = self.classify_match_guard(arm.guard.as_ref());
            let covers_pattern = guard_info.covers_pattern();
            if seen_default {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0589",
                        "match arm is unreachable after `default`",
                        pattern_span,
                    )
                    .with_title("Unreachable Match Arm")
                    .with_help("remove the arm or move `default` to the end"),
                );
                shape_valid = false;
            }

            let mut arm_scopes = scopes.clone();
            arm_scopes.push();
            let mut bindings = Vec::new();
            let resolved_pattern = if condition_mode {
                match &arm.pattern {
                    MatchPattern::Default { .. } => {
                        seen_default = true;
                        default_span = Some(pattern_span);
                        ResolvedMatchPattern::Default
                    }
                    MatchPattern::Expression(condition) => {
                        self.check_expr(condition, &arm_scopes, method_context);
                        let condition_ty =
                            self.infer_expr_type(condition, &arm_scopes, method_context);
                        if !matches!(
                            self.types.kind(condition_ty),
                            TypeKind::Bool | TypeKind::Unknown
                        ) {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0594",
                                    "`match (true)` arm condition must have type `bool`",
                                    condition.span(),
                                )
                                .with_title("Match Condition Must Be Bool")
                                .with_help("use an explicit comparison that produces `bool`"),
                            );
                            shape_valid = false;
                        }
                        ResolvedMatchPattern::Condition
                    }
                    _ => {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0588",
                                "`match (true)` accepts bool conditions and `default`",
                                pattern_span,
                            )
                            .with_title("Invalid Match Pattern"),
                        );
                        shape_valid = false;
                        ResolvedMatchPattern::Condition
                    }
                }
            } else {
                self.resolve_match_pattern(
                    &arm.pattern,
                    scrutinee_ty,
                    &mut arm_scopes,
                    method_context,
                    &mut bindings,
                    &mut seen_default,
                    &mut seen_enum_cases,
                    &mut seen_constants,
                    &mut seen_types,
                    &mut seen_null,
                    covers_pattern,
                    matches!(mode, MatchMode::Consumed { .. }),
                    &mut shape_valid,
                )
            };

            if let Some(guard) = &arm.guard {
                if matches!(resolved_pattern, ResolvedMatchPattern::Default) {
                    self.diagnostics.push(
                        Diagnostic::new("E0598", "`default` cannot have a match guard", guard.span)
                            .with_title("Default Cannot Have A Guard")
                            .with_help(
                                "move the condition to an earlier pattern or use a nested match",
                            ),
                    );
                    shape_valid = false;
                } else if condition_mode {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0599",
                            "`match (true)` arms do not use pattern guards",
                            guard.span,
                        )
                        .with_title("Match True Arm Cannot Have A Guard")
                        .with_help("combine the arm condition and guard with `&&`"),
                    );
                    shape_valid = false;
                }
                self.check_expr(&guard.condition, &arm_scopes, method_context);
                let guard_ty = self.infer_expr_type(&guard.condition, &arm_scopes, method_context);
                if !matches!(
                    self.types.kind(guard_ty),
                    TypeKind::Bool | TypeKind::Unknown
                ) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0597",
                            "match guard must have type `bool`",
                            guard.condition.span(),
                        )
                        .with_title("Match Guard Must Be Bool")
                        .with_help("use an explicit comparison that produces `bool`"),
                    );
                    shape_valid = false;
                }
                match guard_info {
                    MatchGuardSemanticInfo::AlwaysTrue => {
                        self.diagnostics.push(
                            Diagnostic::new("E0601", "match guard is always `true`", guard.span)
                                .with_title("Match Guard Is Redundant")
                                .with_help("remove the redundant guard")
                                .with_fix(guard.span, ""),
                        );
                        shape_valid = false;
                    }
                    MatchGuardSemanticInfo::AlwaysFalse => {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0589",
                                "match arm is unreachable because its guard is always `false`",
                                arm.span,
                            )
                            .with_title("Unreachable Match Arm")
                            .with_help("remove the arm or change its guard"),
                        );
                        shape_valid = false;
                    }
                    MatchGuardSemanticInfo::None | MatchGuardSemanticInfo::Runtime => {}
                }
            }

            if matches!(resolved_pattern, ResolvedMatchPattern::Default) {
                default_span = Some(pattern_span);
            }

            if matches!(resolved_pattern, ResolvedMatchPattern::Default) && index + 1 != arms.len()
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0589",
                        "`default` must be the final match arm",
                        pattern_span,
                    )
                    .with_title("Unreachable Match Arm")
                    .with_help("move `default` to the end of the match"),
                );
                shape_valid = false;
            }

            if let Some(target) = expected {
                self.record_expected_expression_type(&arm.value, target);
            }
            self.check_expr(&arm.value, &arm_scopes, method_context);
            let arm_ty = self.infer_expr_type(&arm.value, &arm_scopes, method_context);
            if self.is_void_type(arm_ty) {
                self.diagnostics.push(
                    Diagnostic::new("E0593", "match arms must produce a value", arm.value.span())
                        .with_title("Match Arm Has No Value")
                        .with_help("use `if` or future `when` for side-effect-only branching"),
                );
                shape_valid = false;
            } else if let Some(target) = expected {
                self.check_expr_assignable(
                    target,
                    &arm.value,
                    &arm_scopes,
                    method_context,
                    AssignmentDestination::Type,
                );
            } else {
                result_ty = self.unify_match_result_type(result_ty, arm_ty, arm.value.span());
            }

            resolved_arms.push(MatchArmSemanticInfo {
                pattern: resolved_pattern,
                guard: guard_info,
                bindings,
            });
        }

        if arms.is_empty() {
            self.diagnostics.push(
                Diagnostic::new("E0585", "match expression has no arms", *span)
                    .with_title("Non-Exhaustive Match"),
            );
            shape_valid = false;
        }

        if condition_mode && !seen_default {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0585",
                    "`match (true)` requires a final `default` arm",
                    *span,
                )
                .with_title("Non-Exhaustive Match")
                .with_help("add `default => value` as the final arm"),
            );
            shape_valid = false;
        } else if !condition_mode {
            let missing = self.missing_match_coverage(
                scrutinee_ty,
                seen_null,
                &seen_enum_cases,
                &seen_constants,
                &seen_types,
            );
            if seen_default {
                if missing.is_empty() {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0589",
                            "`default` is unreachable because earlier arms cover the match domain",
                            default_span.unwrap_or(*span),
                        )
                        .with_title("Unreachable Match Arm")
                        .with_help("remove the redundant `default` arm"),
                    );
                    shape_valid = false;
                }
            } else if !missing.is_empty() {
                self.report_missing_match_coverage(&missing, *span);
                shape_valid = false;
            }
        }

        let result_ty = result_ty.unwrap_or_else(|| self.types.unknown());
        if expected.is_none() && matches!(self.types.kind(result_ty), TypeKind::Null) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0592",
                    "a match whose arms are all `null` needs an expected nullable type",
                    *span,
                )
                .with_title("Match Result Type Cannot Be Inferred")
                .with_help("add an explicit nullable destination type"),
            );
            shape_valid = false;
        }
        self.expression_types
            .insert(*span, self.types.resolved(result_ty));
        if shape_valid || !resolved_arms.is_empty() {
            self.matches.insert(
                *span,
                MatchSemanticInfo {
                    scrutinee_type: self.types.resolved(scrutinee_ty),
                    result_type: self.types.resolved(result_ty),
                    origin: *origin,
                    mode: *mode,
                    condition_mode,
                    arms: resolved_arms,
                },
            );
        }
    }

    fn non_null_function_type(&self, ty: TypeId) -> Option<&SemanticFunctionType<TypeId>> {
        match self.types.kind(ty) {
            TypeKind::Function(function) => Some(function),
            TypeKind::Nullable(inner) => match self.types.kind(*inner) {
                TypeKind::Function(function) => Some(function),
                _ => None,
            },
            _ => None,
        }
    }

    fn function_type_compatibility(
        &self,
        expected: &SemanticFunctionType<TypeId>,
        actual: &SemanticFunctionType<TypeId>,
    ) -> Result<(), FunctionTypeMismatch> {
        if expected.parameters.len() != actual.parameters.len() {
            return Err(FunctionTypeMismatch::Arity);
        }
        for (index, (expected, actual)) in expected
            .parameters
            .iter()
            .zip(&actual.parameters)
            .enumerate()
        {
            if expected.ownership_mode != actual.ownership_mode {
                return Err(FunctionTypeMismatch::ParameterOwnership(index));
            }
            if expected.ty != actual.ty {
                return Err(FunctionTypeMismatch::ParameterType(index));
            }
        }
        if expected.return_type != actual.return_type {
            return Err(FunctionTypeMismatch::ReturnType);
        }
        let capability = |mode| match mode {
            FunctionInvocationMode::Readonly => 0,
            FunctionInvocationMode::Writable => 1,
            FunctionInvocationMode::Once => 2,
        };
        if capability(actual.invocation_mode) > capability(expected.invocation_mode) {
            return Err(FunctionTypeMismatch::InvocationMode);
        }
        if actual
            .checked_effects
            .iter()
            .any(|effect| !expected.checked_effects.contains(effect))
        {
            return Err(FunctionTypeMismatch::CheckedEffects);
        }
        if expected.return_borrow != actual.return_borrow {
            return Err(FunctionTypeMismatch::ReturnBorrow);
        }
        Ok(())
    }

    fn report_function_type_mismatch(
        &mut self,
        expected: &SemanticFunctionType<TypeId>,
        actual: &SemanticFunctionType<TypeId>,
        mismatch: FunctionTypeMismatch,
        span: Span,
    ) {
        let expected_ty = self.types.intern(TypeKind::Function(expected.clone()));
        let actual_ty = self.types.intern(TypeKind::Function(actual.clone()));
        let detail = match mismatch {
            FunctionTypeMismatch::Nullability => {
                "a nullable callable cannot satisfy a non-null function type".to_string()
            }
            FunctionTypeMismatch::Arity => "parameter count differs".to_string(),
            FunctionTypeMismatch::ParameterOwnership(index) => {
                format!("parameter {} has a different ownership mode", index + 1)
            }
            FunctionTypeMismatch::ParameterType(index) => {
                format!("parameter {} has a different type", index + 1)
            }
            FunctionTypeMismatch::ReturnType => "return types differ".to_string(),
            FunctionTypeMismatch::InvocationMode => {
                "the callable requires stronger invocation access".to_string()
            }
            FunctionTypeMismatch::CheckedEffects => {
                "the callable may raise checked errors outside the expected set".to_string()
            }
            FunctionTypeMismatch::ReturnBorrow => "return-borrow provenance differs".to_string(),
        };
        self.diagnostics.push(
            Diagnostic::new(
                "E0648",
                format!(
                    "function value `{}` is not compatible with expected type `{}`: {detail}",
                    self.types.display(actual_ty),
                    self.types.display(expected_ty)
                ),
                span,
            )
            .with_title("Function Type Mismatch")
            .with_help("use a function value with the exact parameter ownership and value types, a compatible invocation mode, and no additional checked effects"),
        );
    }

    fn report_closure_return_mismatch(&mut self, expected: TypeId, actual: TypeId, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0648",
                format!(
                    "closure returns `{}` but its expected return type is `{}`",
                    self.types.display(actual),
                    self.types.display(expected)
                ),
                span,
            )
            .with_title("Closure Return Type Mismatch"),
        );
    }

    fn infer_closure_block_return_type(&mut self, block: &Block) -> Option<TypeId> {
        let mut return_spans = Vec::new();
        collect_return_expression_spans(block, &mut return_spans);
        let mut inferred = None;
        for span in return_spans.into_iter().flatten() {
            let Some(ty) = self.expression_types.get(&span).cloned() else {
                continue;
            };
            let ty = self.types.intern_resolved(&ty);
            match inferred {
                None => inferred = Some(ty),
                Some(previous) if previous == ty => {}
                Some(previous) => {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0648",
                            format!(
                                "closure return paths produce both `{}` and `{}`",
                                self.types.display(previous),
                                self.types.display(ty)
                            ),
                            span,
                        )
                        .with_title("Closure Return Types Do Not Agree")
                        .with_help(
                            "return one exact semantic type from every value-returning path",
                        ),
                    );
                    return Some(self.types.unknown());
                }
            }
        }
        inferred
    }

    fn check_when_expression(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Expr::When(when) = expr else {
            unreachable!("when checking requires a when expression")
        };
        let given = &when.given;
        let result_type = &when.result_type;
        let branches = &when.branches;
        let finally = &when.finally;
        let span = when.span;
        let enclosing_loop_depth = self.active_loop_depth;

        let mut when_scopes = scopes.clone();
        when_scopes.push();
        if let Some(given) = given {
            self.check_given_prelude(given, &mut when_scopes, method_context);
        }

        let explicit = result_type
            .as_ref()
            .map(|ty| self.resolve_type_ref(ty, span));
        if explicit.is_some_and(|ty| self.is_void_type(ty)) {
            self.diagnostics.push(
                Diagnostic::new("E0608", "a `when` expression cannot have type `void`", span)
                    .with_title("When Cannot Produce Void"),
            );
        }
        let contextual = self.contextual_expression_types.get(&span).copied();
        let expected = explicit.or(contextual);
        self.when_contexts.push(WhenCheckContext {
            expected,
            inferred: None,
            saw_value: false,
        });

        for branch in branches {
            if let Some(condition) = &branch.condition {
                self.check_condition(condition, &when_scopes, method_context);
            }
            let mut branch_scopes = when_scopes.clone();
            self.check_block(
                &branch.block,
                &mut branch_scopes,
                method_context,
                None,
                None,
                enclosing_loop_depth,
            );
            if crate::return_analysis::analyze_block_with_given(
                &branch.block,
                branch.block.span,
                &self.given_preludes,
            )
            .fallthrough_reachable
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0610",
                        "every normal path through a `when` branch must return a value",
                        branch.block.span,
                    )
                    .with_title("When Branch Does Not Yield On Every Path")
                    .with_help("add `return value;` on the branch's remaining path"),
                );
            }
        }

        self.active_loop_depth = enclosing_loop_depth;

        let context = self.when_contexts.pop().expect("when result context");
        let result = context
            .expected
            .or(context.inferred)
            .unwrap_or_else(|| self.types.unknown());
        if context.expected.is_none()
            && context.saw_value
            && matches!(self.types.kind(result), TypeKind::Null)
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0610",
                    "a `when` whose branches all yield `null` needs an expected nullable type",
                    span,
                )
                .with_title("When Result Type Cannot Be Inferred")
                .with_help("add an explicit nullable result type or use a nullable destination"),
            );
        }
        self.expression_types
            .insert(span, self.types.resolved(result));
        self.whens.insert(
            span,
            WhenSemanticInfo {
                result_type: self.types.resolved(result),
            },
        );
        self.check_finally(
            finally.as_ref(),
            &when_scopes,
            method_context,
            None,
            None,
            enclosing_loop_depth,
        );
    }

    fn classify_match_guard(&self, guard: Option<&MatchGuard>) -> MatchGuardSemanticInfo {
        let Some(guard) = guard else {
            return MatchGuardSemanticInfo::None;
        };
        let value = match Self::ungroup_expr(&guard.condition) {
            Expr::Bool { value, .. } => Some(*value),
            Expr::Identifier { name, .. } => self
                .const_evaluation
                .values
                .get(&crate::const_eval::ConstKey::TopLevel(name.clone()))
                .and_then(|value| match &value.value {
                    crate::const_eval::ConstValue::Bool(value) => Some(*value),
                    _ => None,
                }),
            Expr::StaticMember {
                qualifier: StaticQualifier::Class(class_name),
                member,
                ..
            } => self
                .const_evaluation
                .values
                .get(&crate::const_eval::ConstKey::Class {
                    class_name: class_name.clone(),
                    name: member.clone(),
                })
                .and_then(|value| match &value.value {
                    crate::const_eval::ConstValue::Bool(value) => Some(*value),
                    _ => None,
                }),
            _ => None,
        };
        match value {
            Some(true) => MatchGuardSemanticInfo::AlwaysTrue,
            Some(false) => MatchGuardSemanticInfo::AlwaysFalse,
            None => MatchGuardSemanticInfo::Runtime,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_match_pattern(
        &mut self,
        pattern: &MatchPattern,
        scrutinee_ty: TypeId,
        scopes: &mut ScopeStack,
        method_context: Option<&MethodContext>,
        bindings: &mut Vec<MatchBindingSemanticInfo>,
        seen_default: &mut bool,
        seen_enum_cases: &mut HashSet<EnumCaseId>,
        seen_constants: &mut Vec<crate::const_eval::ConstValue>,
        seen_types: &mut HashSet<ResolvedType>,
        seen_null: &mut bool,
        covers_pattern: bool,
        consuming: bool,
        shape_valid: &mut bool,
    ) -> ResolvedMatchPattern {
        let pattern_span = Self::match_pattern_span(pattern);
        match pattern {
            MatchPattern::Default { .. } => {
                if *seen_default {
                    self.duplicate_match_pattern("duplicate `default` arm", pattern_span);
                    *shape_valid = false;
                }
                *seen_default = true;
                ResolvedMatchPattern::Default
            }
            MatchPattern::EnumCase {
                qualifier,
                case,
                bindings: authored_bindings,
                ..
            } => {
                if authored_bindings.is_none() {
                    if let Some(value) = self
                        .const_evaluation
                        .values
                        .get(&crate::const_eval::ConstKey::Class {
                            class_name: qualifier.clone(),
                            name: case.clone(),
                        })
                        .map(|value| value.value.clone())
                    {
                        let value_ty = self.const_value_type(&value);
                        let (base, _) = self.match_base_type(scrutinee_ty);
                        if value_ty != base {
                            self.incompatible_match_pattern(
                                &format!("constant of type `{}`", self.types.display(value_ty)),
                                scrutinee_ty,
                                pattern_span,
                            );
                            *shape_valid = false;
                        }
                        if seen_constants.contains(&value) {
                            self.duplicate_match_pattern(
                                "literal or constant pattern is unreachable after an earlier unguarded arm",
                                pattern_span,
                            );
                            *shape_valid = false;
                        } else if covers_pattern {
                            seen_constants.push(value.clone());
                        }
                        return ResolvedMatchPattern::Constant(value);
                    }
                }
                let (base_ty, _) = self.match_base_type(scrutinee_ty);
                let TypeKind::Enum(scrutinee_enum) = self.types.kind(base_ty).clone() else {
                    self.incompatible_match_pattern("enum case", scrutinee_ty, pattern_span);
                    *shape_valid = false;
                    return ResolvedMatchPattern::EnumCase {
                        enum_id: EnumId(usize::MAX),
                        case_id: EnumCaseId {
                            enum_id: EnumId(usize::MAX),
                            index: usize::MAX,
                        },
                    };
                };
                if qualifier != &scrutinee_enum.name {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0588",
                            format!(
                                "case `{qualifier}::{case}` does not belong to matched enum `{}`",
                                scrutinee_enum.name
                            ),
                            pattern_span,
                        )
                        .with_title("Match Pattern Has Wrong Type"),
                    );
                    *shape_valid = false;
                }
                let Some(definition) = self.enums.get(&scrutinee_enum.name).cloned() else {
                    *shape_valid = false;
                    return ResolvedMatchPattern::EnumCase {
                        enum_id: scrutinee_enum.id,
                        case_id: EnumCaseId {
                            enum_id: scrutinee_enum.id,
                            index: usize::MAX,
                        },
                    };
                };
                let Some(case_index) = definition.case_by_name.get(case).copied() else {
                    self.diagnostics.push(Diagnostic::new(
                        "E0575",
                        format!("enum `{}` has no case `{case}`", definition.name),
                        pattern_span,
                    ));
                    *shape_valid = false;
                    return ResolvedMatchPattern::EnumCase {
                        enum_id: definition.id,
                        case_id: EnumCaseId {
                            enum_id: definition.id,
                            index: usize::MAX,
                        },
                    };
                };
                let case_definition = &definition.cases[case_index];
                if seen_enum_cases.contains(&case_definition.id) {
                    self.duplicate_match_pattern(
                        &format!(
                            "match arm for `{}::{case}` is unreachable after an earlier unguarded arm",
                            definition.name
                        ),
                        pattern_span,
                    );
                    *shape_valid = false;
                } else if covers_pattern {
                    seen_enum_cases.insert(case_definition.id);
                }
                if let Some(authored_bindings) = authored_bindings {
                    if authored_bindings.len() != case_definition.payload.len() {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0590",
                                format!(
                                    "case `{}::{case}` has {} payload field(s), but this pattern binds {}",
                                    definition.name,
                                    case_definition.payload.len(),
                                    authored_bindings.len()
                                ),
                                pattern_span,
                            )
                            .with_title("Match Payload Arity Mismatch"),
                        );
                        *shape_valid = false;
                    }
                    let mut names = HashSet::new();
                    for (binding, field) in
                        authored_bindings.iter().zip(case_definition.payload.iter())
                    {
                        if !names.insert(binding.name.clone()) {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0103",
                                    format!(
                                        "variable `${}` is already declared in this match pattern",
                                        binding.name
                                    ),
                                    binding.span,
                                )
                                .with_title("Duplicate Pattern Binding"),
                            );
                            *shape_valid = false;
                            continue;
                        }
                        let borrowed = !consuming && self.type_is_move_type(field.ty);
                        self.declare_binding(
                            scopes,
                            binding.name.clone(),
                            Binding::unresolved(false, field.ty, field.ty, None, None),
                            binding.span,
                            BindingKind::MatchBinding,
                            if borrowed {
                                BindingOwnership::ReadonlyBorrow
                            } else {
                                BindingOwnership::Owned
                            },
                        );
                        self.expression_types
                            .insert(binding.span, self.types.resolved(field.ty));
                        bindings.push(MatchBindingSemanticInfo {
                            name: binding.name.clone(),
                            ty: self.types.resolved(field.ty),
                            borrowed,
                        });
                    }
                }
                ResolvedMatchPattern::EnumCase {
                    enum_id: definition.id,
                    case_id: case_definition.id,
                }
            }
            MatchPattern::TypeBinding { ty, binding, .. } => {
                let pattern_ty = self.resolve_type_ref(ty, pattern_span);
                if ty.nullable || !self.is_exact_match_pattern_type(pattern_ty) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0588",
                            "type-binding patterns require a concrete non-null exact type",
                            pattern_span,
                        )
                        .with_title("Invalid Match Type Pattern")
                        .with_help("use `null` for absence and `default` for the open remainder"),
                    );
                    *shape_valid = false;
                }
                let (base, nullable) = self.match_base_type(scrutinee_ty);
                let compatible = matches!(self.types.kind(scrutinee_ty), TypeKind::Mixed)
                    || (nullable && self.is_assignable(base, pattern_ty));
                if !compatible {
                    if base == pattern_ty && !nullable {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0589",
                                format!(
                                    "type pattern `{}` is always true for this scrutinee",
                                    self.types.display(pattern_ty)
                                ),
                                pattern_span,
                            )
                            .with_title("Unreachable Match Arm")
                            .with_help(
                                "use the value directly instead of matching its existing type",
                            ),
                        );
                    } else {
                        self.incompatible_match_pattern(
                            &format!("type `{}`", self.types.display(pattern_ty)),
                            scrutinee_ty,
                            pattern_span,
                        );
                    }
                    *shape_valid = false;
                }
                let resolved = self.types.resolved(pattern_ty);
                if seen_types.contains(&resolved) {
                    self.duplicate_match_pattern(
                        "exact type pattern is unreachable after an earlier unguarded arm",
                        pattern_span,
                    );
                    *shape_valid = false;
                } else if covers_pattern {
                    seen_types.insert(resolved.clone());
                }
                self.declare_binding(
                    scopes,
                    binding.name.clone(),
                    Binding::unresolved(false, pattern_ty, pattern_ty, None, None),
                    binding.span,
                    BindingKind::MatchBinding,
                    if !consuming && self.type_is_move_type(pattern_ty) {
                        BindingOwnership::ReadonlyBorrow
                    } else {
                        BindingOwnership::Owned
                    },
                );
                self.expression_types.insert(binding.span, resolved.clone());
                bindings.push(MatchBindingSemanticInfo {
                    name: binding.name.clone(),
                    ty: resolved.clone(),
                    borrowed: !consuming && self.type_is_move_type(pattern_ty),
                });
                ResolvedMatchPattern::ExactType(resolved)
            }
            MatchPattern::Expression(expr)
                if matches!(Self::ungroup_expr(expr), Expr::Null { .. }) =>
            {
                let (_, nullable) = self.match_base_type(scrutinee_ty);
                if !nullable && !matches!(self.types.kind(scrutinee_ty), TypeKind::Mixed) {
                    self.incompatible_match_pattern("`null`", scrutinee_ty, pattern_span);
                    *shape_valid = false;
                }
                if *seen_null {
                    self.duplicate_match_pattern(
                        "`null` pattern is unreachable after an earlier unguarded arm",
                        pattern_span,
                    );
                    *shape_valid = false;
                } else if covers_pattern {
                    *seen_null = true;
                }
                ResolvedMatchPattern::Null
            }
            MatchPattern::Expression(expr) => {
                self.check_expr(expr, scopes, method_context);
                let (base, _) = self.match_base_type(scrutinee_ty);
                if let TypeKind::Integer(integer) = *self.types.kind(base) {
                    if self.check_contextual_integer_literal(expr, integer) == Some(false) {
                        *shape_valid = false;
                        return ResolvedMatchPattern::Constant(crate::const_eval::ConstValue::Null);
                    }
                }
                let Some(value) = self.match_constant_value(expr, scrutinee_ty, scopes) else {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0587",
                            "ordinary match patterns must be known during compilation",
                            pattern_span,
                        )
                        .with_title("Match Pattern Is Not Constant")
                        .with_help(
                            "use a literal, an accessible constant, an enum case, or `default`",
                        ),
                    );
                    *shape_valid = false;
                    return ResolvedMatchPattern::Constant(crate::const_eval::ConstValue::Null);
                };
                let value_ty = self.infer_expr_type(expr, scopes, method_context);
                if value_ty != base {
                    self.incompatible_match_pattern(
                        &format!("value of type `{}`", self.types.display(value_ty)),
                        scrutinee_ty,
                        pattern_span,
                    );
                    *shape_valid = false;
                }
                if seen_constants.contains(&value) {
                    self.duplicate_match_pattern(
                        "literal or constant pattern is unreachable after an earlier unguarded arm",
                        pattern_span,
                    );
                    *shape_valid = false;
                } else if covers_pattern {
                    seen_constants.push(value.clone());
                }
                ResolvedMatchPattern::Constant(value)
            }
        }
    }

    fn missing_match_coverage(
        &self,
        scrutinee_ty: TypeId,
        has_null: bool,
        enum_cases: &HashSet<EnumCaseId>,
        constants: &[crate::const_eval::ConstValue],
        exact_types: &HashSet<ResolvedType>,
    ) -> Vec<String> {
        let (base, nullable) = self.match_base_type(scrutinee_ty);
        let null_covered = !nullable || has_null;
        let missing = match self.types.kind(base).clone() {
            TypeKind::Enum(enum_type) => self
                .enums
                .get(&enum_type.name)
                .map(|definition| {
                    definition
                        .cases
                        .iter()
                        .filter(|case| !enum_cases.contains(&case.id))
                        .map(|case| format!("{}::{}", definition.name, case.name))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            TypeKind::Bool => [true, false]
                .into_iter()
                .filter(|value| !constants.contains(&crate::const_eval::ConstValue::Bool(*value)))
                .map(|value| value.to_string())
                .collect(),
            _ if nullable && exact_types.contains(&self.types.resolved(base)) => Vec::new(),
            _ => vec!["default".to_string()],
        };
        let mut missing = missing;
        if !null_covered {
            missing.insert(0, "null".to_string());
        }
        missing
    }

    fn report_missing_match_coverage(&mut self, missing: &[String], span: Span) {
        let detail = missing
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n");
        self.diagnostics.push(
            Diagnostic::new("E0585", "match does not cover every possible value", span)
                .with_title("Non-Exhaustive Match")
                .with_explanation(format!("Missing match cases:\n{detail}"))
                .with_help("add the missing cases or a final `default` arm"),
        );
    }

    fn is_exact_match_pattern_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::Integer(_)
                | TypeKind::Float(_)
                | TypeKind::String
                | TypeKind::Bool
                | TypeKind::Enum(_)
                | TypeKind::Class(_)
                | TypeKind::Function(_)
        )
    }

    fn unify_match_result_type(
        &mut self,
        current: Option<TypeId>,
        next: TypeId,
        span: Span,
    ) -> Option<TypeId> {
        let Some(current) = current else {
            return Some(next);
        };
        if current == next || self.is_assignable(current, next) {
            return Some(current);
        }
        if self.is_assignable(next, current) {
            return Some(next);
        }
        let nullable = match (
            self.types.kind(current).clone(),
            self.types.kind(next).clone(),
        ) {
            (TypeKind::Null, _)
                if !matches!(self.types.kind(next), TypeKind::Null | TypeKind::Void) =>
            {
                Some(next)
            }
            (_, TypeKind::Null)
                if !matches!(self.types.kind(current), TypeKind::Null | TypeKind::Void) =>
            {
                Some(current)
            }
            _ => None,
        };
        if let Some(inner) = nullable {
            return Some(self.types.intern(TypeKind::Nullable(inner)));
        }
        self.diagnostics.push(
            Diagnostic::new(
                "E0592",
                format!(
                    "match arm type `{}` does not match result type `{}`",
                    self.types.display(next),
                    self.types.display(current)
                ),
                span,
            )
            .with_title("Match Arm Type Mismatch")
            .with_help("make every arm produce one compatible type"),
        );
        Some(current)
    }

    fn match_constant_value(
        &mut self,
        expr: &Expr,
        scrutinee_ty: TypeId,
        scopes: &ScopeStack,
    ) -> Option<crate::const_eval::ConstValue> {
        let (base, _) = self.match_base_type(scrutinee_ty);
        match Self::ungroup_expr(expr) {
            Expr::Bool { value, .. } => Some(crate::const_eval::ConstValue::Bool(*value)),
            Expr::String { .. }
            | Expr::Binary {
                op: BinaryOp::Concat,
                ..
            } => {
                Self::eval_string_constant(expr, scopes).map(crate::const_eval::ConstValue::String)
            }
            Expr::Int { .. }
            | Expr::Unary {
                op: UnaryOp::Negate,
                ..
            } if matches!(self.types.kind(base), TypeKind::Integer(_)) => {
                let TypeKind::Integer(integer) = *self.types.kind(base) else {
                    unreachable!()
                };
                match Self::eval_int_constant(expr, scopes, integer) {
                    IntConstantEval::Known(value) => {
                        Some(crate::const_eval::ConstValue::Integer(value))
                    }
                    IntConstantEval::Unknown | IntConstantEval::Invalid => None,
                }
            }
            Expr::Float { value, .. } if matches!(self.types.kind(base), TypeKind::Float(_)) => {
                let TypeKind::Float(float) = *self.types.kind(base) else {
                    unreachable!()
                };
                FloatValue::parse_decimal(float, value).map(crate::const_eval::ConstValue::Float)
            }
            Expr::Identifier { name, .. } => self
                .const_evaluation
                .values
                .get(&crate::const_eval::ConstKey::TopLevel(name.clone()))
                .map(|value| value.value.clone()),
            Expr::StaticMember {
                qualifier: StaticQualifier::Class(class_name),
                member,
                ..
            } => self
                .const_evaluation
                .enum_cases
                .get(&(class_name.clone(), member.clone()))
                .copied()
                .map(crate::const_eval::ConstValue::Enum)
                .or_else(|| {
                    self.const_evaluation
                        .values
                        .get(&crate::const_eval::ConstKey::Class {
                            class_name: class_name.clone(),
                            name: member.clone(),
                        })
                        .map(|value| value.value.clone())
                }),
            _ => None,
        }
    }

    fn const_value_type(&mut self, value: &crate::const_eval::ConstValue) -> TypeId {
        match value {
            crate::const_eval::ConstValue::Integer(value) => {
                self.types.intern(TypeKind::Integer(value.ty))
            }
            crate::const_eval::ConstValue::Float(value) => {
                self.types.intern(TypeKind::Float(value.ty))
            }
            crate::const_eval::ConstValue::String(_) => self.types.intern(TypeKind::String),
            crate::const_eval::ConstValue::Bool(_) => self.types.intern(TypeKind::Bool),
            crate::const_eval::ConstValue::Null => self.types.intern(TypeKind::Null),
            crate::const_eval::ConstValue::Enum(value) => {
                let enum_id = value.enum_id;
                let name = self
                    .enums
                    .values()
                    .find(|definition| definition.id == enum_id)
                    .map(|definition| definition.name.clone())
                    .unwrap_or_default();
                self.types
                    .intern(TypeKind::Enum(crate::enums::EnumType { id: enum_id, name }))
            }
            crate::const_eval::ConstValue::PayloadEnum(value) => {
                let enum_id = value.enum_id;
                let name = self
                    .enums
                    .values()
                    .find(|definition| definition.id == enum_id)
                    .map(|definition| definition.name.clone())
                    .unwrap_or_default();
                self.types
                    .intern(TypeKind::Enum(crate::enums::EnumType { id: enum_id, name }))
            }
        }
    }

    fn match_base_type(&self, ty: TypeId) -> (TypeId, bool) {
        match self.types.kind(ty) {
            TypeKind::Nullable(inner) => (*inner, true),
            _ => (ty, false),
        }
    }

    fn duplicate_match_pattern(&mut self, message: &str, span: Span) {
        self.diagnostics.push(
            Diagnostic::new("E0586", message, span)
                .with_title("Duplicate Match Pattern")
                .with_help("remove the repeated arm"),
        );
    }

    fn incompatible_match_pattern(&mut self, pattern: &str, scrutinee_ty: TypeId, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0588",
                format!(
                    "{pattern} cannot match scrutinee type `{}`",
                    self.types.display(scrutinee_ty)
                ),
                span,
            )
            .with_title("Match Pattern Has Wrong Type"),
        );
    }

    fn match_pattern_span(pattern: &MatchPattern) -> Span {
        match pattern {
            MatchPattern::Default { span }
            | MatchPattern::EnumCase { span, .. }
            | MatchPattern::TypeBinding { span, .. } => *span,
            MatchPattern::Expression(expr) => expr.span(),
        }
    }

    fn ungroup_expr(mut expr: &Expr) -> &Expr {
        while let Expr::Grouped { expr: inner, .. } = expr {
            expr = inner;
        }
        expr
    }

    fn is_grouped_range_expr(expr: &Expr) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => Self::is_grouped_range_expr(expr),
            Expr::Range { .. } => true,
            _ => false,
        }
    }

    fn range_integer_type(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<IntegerType> {
        match expr {
            Expr::Grouped { expr, .. } => self.range_integer_type(expr, scopes, method_context),
            Expr::Range { start, end, .. } => {
                let ty = self.infer_binary_type(start, &BinaryOp::Add, end, scopes, method_context);
                match self.types.kind(ty) {
                    TypeKind::Integer(integer) => Some(*integer),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn check_range_operands(
        &mut self,
        start: &Expr,
        end: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let (start_ty, end_ty) =
            self.infer_contextual_binary_operand_types(start, end, scopes, method_context);
        match (self.types.kind(start_ty), self.types.kind(end_ty)) {
            (TypeKind::Integer(start), TypeKind::Integer(end)) if start == end => {}
            (TypeKind::Integer(_), TypeKind::Integer(_)) => {
                self.report_integer_operand_mismatch(start_ty, end_ty, span, "range")
            }
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown) => {}
            _ => self.diagnostics.push(Diagnostic::new(
                "E0424",
                format!(
                    "range endpoints must be integers of the same type, got `{}` and `{}`",
                    self.types.display(start_ty),
                    self.types.display(end_ty)
                ),
                span,
            )),
        }
    }

    fn contextualize_range_literals(&mut self, expr: &Expr, target: IntegerType) {
        match expr {
            Expr::Grouped { expr, .. } => self.contextualize_range_literals(expr, target),
            Expr::Range { start, end, .. } => {
                self.contextualize_integer_literals(start, target);
                self.contextualize_integer_literals(end, target);
            }
            _ => {}
        }
    }

    fn report_integer_literal_range(&mut self, span: Span, target: IntegerType) {
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0417" && diagnostic.span == span)
        {
            return;
        }

        let mut diagnostic = Diagnostic::new(
            "E0417",
            format!(
                "integer literal is outside the Doria `{}` range",
                target.source_name()
            ),
            span,
        );
        if target == IntegerType::Int64 {
            diagnostic = diagnostic.with_help(
                "unconstrained integer literals default to `int`; add a `uint64` context when that is intended",
            );
        }
        self.diagnostics.push(diagnostic);
    }

    fn check_pending_integer_literal_ranges(&mut self) {
        let mut literals = Vec::new();
        for (span, magnitude) in &self.integer_literals {
            if self.negated_integer_literal_operands.contains(span) {
                continue;
            }
            literals.push((*span, *magnitude, false));
        }
        literals.extend(
            self.negative_integer_literals
                .iter()
                .map(|(span, magnitude)| (*span, *magnitude, true)),
        );
        literals.sort_unstable_by_key(|(span, _, _)| *span);

        for (span, magnitude, negative) in literals {
            let target = self
                .integer_expression_types
                .get(&span)
                .copied()
                .unwrap_or(IntegerType::Int64);
            if IntegerValue::from_literal(target, magnitude, negative).is_none() {
                self.report_integer_literal_range(span, target);
            }
        }
    }

    fn integer_literal_parts(expr: &Expr) -> Option<(u128, bool)> {
        match expr {
            Expr::Int { value, .. } => parse_decimal_magnitude(value).map(|value| (value, false)),
            Expr::Grouped { expr, .. } => Self::integer_literal_parts(expr),
            Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => Self::unsigned_integer_literal_magnitude(expr).map(|value| (value, true)),
            _ => None,
        }
    }

    fn unsigned_integer_literal_magnitude(expr: &Expr) -> Option<u128> {
        match expr {
            Expr::Int { value, .. } => parse_decimal_magnitude(value),
            Expr::Grouped { expr, .. } => Self::unsigned_integer_literal_magnitude(expr),
            _ => None,
        }
    }

    fn unsigned_integer_literal_span(expr: &Expr) -> Option<Span> {
        match expr {
            Expr::Int { span, .. } => Some(*span),
            Expr::Grouped { expr, .. } => Self::unsigned_integer_literal_span(expr),
            _ => None,
        }
    }

    fn record_integer_expression_type(&mut self, expr: &Expr, integer: IntegerType) {
        self.integer_expression_types.insert(expr.span(), integer);
        match expr {
            Expr::Grouped { expr, .. }
            | Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => self.record_integer_expression_type(expr, integer),
            _ => {}
        }
    }

    /// Returns `Some(true/false)` for a literal form and `None` for a
    /// non-literal expression. A contextual literal is typing, not conversion.
    fn check_contextual_integer_literal(
        &mut self,
        expr: &Expr,
        target: IntegerType,
    ) -> Option<bool> {
        let (magnitude, negative) = Self::integer_literal_parts(expr)?;
        if IntegerValue::from_literal(target, magnitude, negative).is_some() {
            self.record_integer_expression_type(expr, target);
            Some(true)
        } else {
            self.report_integer_literal_range(expr.span(), target);
            Some(false)
        }
    }

    fn contextualize_integer_literals(&mut self, expr: &Expr, target: IntegerType) {
        if self
            .check_contextual_integer_literal(expr, target)
            .is_some()
        {
            return;
        }

        match expr {
            Expr::Grouped { expr, .. }
            | Expr::Unary {
                op: UnaryOp::Negate | UnaryOp::BitwiseNot,
                expr,
                ..
            } => self.contextualize_integer_literals(expr, target),
            Expr::Binary {
                left,
                op:
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::ShiftLeft
                    | BinaryOp::ShiftRight
                    | BinaryOp::BitwiseAnd
                    | BinaryOp::BitwiseXor
                    | BinaryOp::BitwiseOr,
                right,
                ..
            } => {
                self.contextualize_integer_literals(left, target);
                self.contextualize_integer_literals(right, target);
            }
            _ => {}
        }
    }

    fn is_float_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Float { .. } => true,
            Expr::Grouped { expr, .. } => Self::is_float_literal(expr),
            _ => false,
        }
    }

    fn record_float_expression_type(&mut self, expr: &Expr, float: FloatType) {
        self.float_expression_types.insert(expr.span(), float);
        self.check_float_literal_range(expr, float);
        if let Expr::Grouped { expr, .. } = expr {
            self.record_float_expression_type(expr, float);
        }
    }

    fn check_float_literal_range(&mut self, expr: &Expr, target: FloatType) {
        let Expr::Float { value, span } = expr else {
            return;
        };
        let out_of_range = FloatValue::parse_decimal(target, value)
            .map(FloatValue::is_infinite)
            .unwrap_or(true);
        if out_of_range
            && !self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0444" && diagnostic.span == *span)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0444",
                format!(
                    "floating literal is outside the Doria `{}` finite range",
                    target.source_name()
                ),
                *span,
            ));
        }
    }

    fn contextualize_float_literals(&mut self, expr: &Expr, target: FloatType) {
        if Self::is_float_literal(expr) {
            self.record_float_expression_type(expr, target);
            return;
        }

        match expr {
            Expr::Grouped { expr, .. }
            | Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => self.contextualize_float_literals(expr, target),
            Expr::Binary {
                left,
                op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div,
                right,
                ..
            } => {
                self.contextualize_float_literals(left, target);
                self.contextualize_float_literals(right, target);
                self.float_expression_types.insert(expr.span(), target);
            }
            _ => {}
        }
    }

    fn check_unary_operand(
        &mut self,
        op: &UnaryOp,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        match op {
            UnaryOp::Not => {
                let ty = self.infer_expr_type(expr, scopes, method_context);
                if self.is_mixed_type(ty) {
                    self.report_mixed_operation(expr.span(), "boolean operator");
                    return;
                }

                if self.is_bool_or_recovery_type(ty) {
                    return;
                }

                self.diagnostics.push(Diagnostic::new(
                    "E0419",
                    format!(
                        "boolean operator `not`/`!` requires a `bool` operand, got `{}`",
                        self.types.display(ty)
                    ),
                    expr.span(),
                ));
            }
            UnaryOp::Negate => {
                let ty = self.infer_expr_type(expr, scopes, method_context);
                match self.types.kind(ty) {
                    TypeKind::Integer(integer) if integer.is_signed() => {}
                    TypeKind::Float(_) => {}
                    TypeKind::Unknown => {}
                    TypeKind::Integer(integer) => self.diagnostics.push(
                        Diagnostic::new(
                            "E0440",
                            format!("unary `-` requires a signed integer operand, got `{integer}`"),
                            expr.span(),
                        )
                        .with_help("explicitly convert to a signed integer type first"),
                    ),
                    _ => self.diagnostics.push(Diagnostic::new(
                        "E0440",
                        format!(
                            "unary `-` requires a signed integer or float operand, got `{}`",
                            self.types.display(ty)
                        ),
                        expr.span(),
                    )),
                }
            }
            UnaryOp::BitwiseNot => {
                let ty = self.infer_expr_type(expr, scopes, method_context);
                if !matches!(
                    self.types.kind(ty),
                    TypeKind::Integer(_) | TypeKind::Unknown
                ) {
                    self.diagnostics.push(Diagnostic::new(
                        "E0440",
                        format!(
                            "bitwise operator `~` requires an integer operand, got `{}`",
                            self.types.display(ty)
                        ),
                        expr.span(),
                    ));
                }
            }
        }
    }

    fn check_binary_operands(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if self.has_mixed_operand(left, right, scopes, method_context) {
            return;
        }

        match op {
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                self.check_logical_binary_operands(left, op, right, span, scopes, method_context);
            }
            BinaryOp::Equal | BinaryOp::NotEqual => {
                self.check_equality_operands(left, right, span, scopes, method_context);
            }
            BinaryOp::Concat => {
                self.check_concat_operands(left, right, span, scopes, method_context);
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.check_numeric_binary_operands(left, right, span, scopes, method_context, false)
            }
            BinaryOp::Mod
            | BinaryOp::ShiftLeft
            | BinaryOp::ShiftRight
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseOr => {
                self.check_numeric_binary_operands(left, right, span, scopes, method_context, true)
            }
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                let (left_ty, right_ty) =
                    self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
                if matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right
                ) || matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::Float(left), TypeKind::Float(right)) if left == right
                ) || matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::String, TypeKind::String)
                ) || matches!(
                    (self.types.kind(left_ty), self.types.kind(right_ty)),
                    (TypeKind::Bool, TypeKind::Bool)
                ) || self.constrained_relational_operands(left_ty, right_ty)
                    || matches!(
                        (self.types.kind(left_ty), self.types.kind(right_ty)),
                        (TypeKind::Unknown, _) | (_, TypeKind::Unknown)
                    )
                {
                    return;
                }
                self.report_integer_operand_mismatch(left_ty, right_ty, span, "comparison");
            }
            BinaryOp::Coalesce => {
                let result = self.infer_binary_type(left, op, right, scopes, method_context);
                if matches!(self.types.kind(result), TypeKind::Heterogeneous) {
                    let left_ty = self.infer_expr_type(left, scopes, method_context);
                    let right_ty = self.infer_expr_type(right, scopes, method_context);
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0512",
                            format!(
                                "null-coalescing operands are incompatible: `{}` cannot fall back to `{}`",
                                self.types.display(left_ty),
                                self.types.display(right_ty)
                            ),
                            span,
                        )
                        .with_help("use a fallback assignable to the nullable payload type"),
                    );
                }
            }
        }
    }

    fn check_numeric_binary_operands(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        integers_only: bool,
    ) {
        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
        let compatible_integer = matches!(
            (self.types.kind(left_ty), self.types.kind(right_ty)),
            (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right
        );
        let compatible_float = !integers_only
            && matches!(
                (self.types.kind(left_ty), self.types.kind(right_ty)),
                (TypeKind::Float(left), TypeKind::Float(right)) if left == right
            );
        let recovering = matches!(
            (self.types.kind(left_ty), self.types.kind(right_ty)),
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown)
        );
        if compatible_integer || compatible_float || recovering {
            return;
        }

        self.report_integer_operand_mismatch(left_ty, right_ty, span, "integer operator");
    }

    fn report_integer_operand_mismatch(
        &mut self,
        left: TypeId,
        right: TypeId,
        span: Span,
        operation: &str,
    ) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0441",
                format!(
                    "{operation} operands must have the same integer type, got `{}` and `{}`",
                    self.types.display(left),
                    self.types.display(right)
                ),
                span,
            )
            .with_help("explicitly convert one operand with a companion `::from(...)` call"),
        );
    }

    fn check_logical_binary_operands(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
        if self.is_bool_or_recovery_type(left_ty) && self.is_bool_or_recovery_type(right_ty) {
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0419",
            format!(
                "boolean operator {} requires `bool` operands, got `{}` and `{}`",
                Self::logical_operator_name(op),
                self.types.display(left_ty),
                self.types.display(right_ty)
            ),
            span,
        ));
    }

    fn check_equality_operands(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);
        let left_kind = self.types.kind(left_ty).clone();
        let right_kind = self.types.kind(right_ty).clone();
        if self.is_supported_nullable_equality(
            left,
            left_ty,
            right,
            right_ty,
            scopes,
            method_context,
        ) {
            return;
        }
        if matches!(left_kind, TypeKind::Function(_)) || matches!(right_kind, TypeKind::Function(_))
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0420",
                    "function values cannot be compared for equality",
                    span,
                )
                .with_title("Function Values Have No Equality")
                .with_help("compare explicit application state instead of callable identity"),
            );
            return;
        }
        if self.constrained_equality_operands(left_ty, right_ty) {
            return;
        }
        if let Some(parameter) = match (&left_kind, &right_kind) {
            (TypeKind::TypeParameter(parameter), _) | (_, TypeKind::TypeParameter(parameter)) => {
                Some(parameter)
            }
            _ => None,
        } {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0537",
                    format!(
                        "equality is not guaranteed by the constraints on type parameter `{parameter}`"
                    ),
                    span,
                )
                .with_help(format!(
                    "declare `{parameter} implements Equatable` before comparing values of this type"
                )),
            );
            return;
        }
        let left_collection = self.is_runtime_collection_type(left_ty);
        let right_collection = self.is_runtime_collection_type(right_ty);
        if left_collection || right_collection {
            if matches!(self.types.kind(left_ty), TypeKind::Bytes)
                && matches!(self.types.kind(right_ty), TypeKind::Bytes)
            {
                return;
            }
            self.diagnostics.push(Diagnostic::unsupported_stage(
                "E0525",
                "collection equality is not supported in Stage 23; only `Bytes` values have value equality",
                span,
            ));
            return;
        }
        match (&left_kind, &right_kind) {
            (TypeKind::Enum(left), TypeKind::Enum(right)) if left.id != right.id => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0579",
                        format!(
                            "different enum types `{}` and `{}` cannot be compared",
                            left.name, right.name
                        ),
                        span,
                    )
                    .with_title("Different Enum Types Cannot Be Compared"),
                );
                return;
            }
            (TypeKind::Enum(left), TypeKind::Enum(right)) if left.id == right.id => {
                if let Some(definition) = self
                    .enums
                    .values()
                    .find(|definition| definition.id == left.id)
                    .filter(|definition| !definition.capabilities.equality)
                {
                    let unavailable = definition.cases.iter().find_map(|case| {
                        case.payload.iter().find_map(|field| {
                            (!semantic_type_capabilities(
                                &self.types,
                                field.ty,
                                &self
                                    .enums
                                    .values()
                                    .map(|definition| (definition.id, definition.capabilities))
                                    .collect(),
                            )
                            .equality)
                                .then_some((case, field))
                        })
                    });
                    let detail = unavailable.map_or_else(
                        || "one of its payload types has no Doria equality".to_string(),
                        |(case, field)| {
                            format!(
                                "case `{}` field `${}` has type `{}` without Doria equality",
                                case.name,
                                field.name,
                                self.types.display(field.ty)
                            )
                        },
                    );
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0584",
                            format!(
                                "payload enum `{}` cannot be compared because {detail}",
                                definition.name
                            ),
                            span,
                        )
                        .with_title("Payload Enum Equality Is Unavailable"),
                    );
                    return;
                }
            }
            (TypeKind::Enum(enum_type), _) | (_, TypeKind::Enum(enum_type))
                if !matches!(
                    (&left_kind, &right_kind),
                    (TypeKind::Enum(_), TypeKind::Enum(_))
                ) =>
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0580",
                        format!(
                            "enum `{}` cannot be compared with a non-enum value",
                            enum_type.name
                        ),
                        span,
                    )
                    .with_title("Enum Cannot Be Compared With Its Backing Type")
                    .with_help("compare the backed enum's `value` property explicitly when the backing value is intended"),
                );
                return;
            }
            _ => {}
        }
        if self.is_equality_compatible(left_ty, right_ty) {
            return;
        }

        let mut diagnostic = Diagnostic::new(
            "E0420",
            format!(
                "equality operands must have compatible types, got `{}` and `{}`",
                self.types.display(left_ty),
                self.types.display(right_ty)
            ),
            span,
        );
        if matches!(self.types.kind(left_ty), TypeKind::Integer(_))
            && matches!(self.types.kind(right_ty), TypeKind::Integer(_))
        {
            diagnostic = diagnostic.with_help(
                "integer comparisons do not widen implicitly; explicitly convert one operand",
            );
        }
        self.diagnostics.push(diagnostic);
    }

    fn check_concat_operands(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let left_ty = self.infer_expr_type(left, scopes, method_context);
        let right_ty = self.infer_expr_type(right, scopes, method_context);
        let mut rejected_class = false;
        for (ty, expr) in [(left_ty, left), (right_ty, right)] {
            if matches!(
                self.display_conversion_kind(ty),
                DisplayConversionKind::NonDisplayableClass
            ) {
                self.report_non_displayable_class(ty, expr.span());
                rejected_class = true;
            }
        }
        if rejected_class {
            return;
        }
        let has_string = matches!(
            self.types.kind(left_ty),
            TypeKind::String | TypeKind::Unknown
        ) || matches!(
            self.types.kind(right_ty),
            TypeKind::String | TypeKind::Unknown
        );
        if has_string
            && self.is_display_convertible_type(left_ty)
            && self.is_display_convertible_type(right_ty)
        {
            return;
        }

        self.diagnostics.push(
            Diagnostic::new(
                "E0425",
                "concatenation requires at least one string operand",
                span,
            )
            .with_help("use + for numeric addition or add a string/interpolation context"),
        );
    }

    fn is_bool_or_recovery_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Bool | TypeKind::Unknown)
    }

    fn is_display_convertible_type(&self, ty: TypeId) -> bool {
        matches!(
            self.display_conversion_kind(ty),
            DisplayConversionKind::Primitive
                | DisplayConversionKind::DisplayableClass
                | DisplayConversionKind::Recovery
        )
    }

    fn display_conversion_kind(&self, ty: TypeId) -> DisplayConversionKind {
        match self.types.kind(ty) {
            TypeKind::String | TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool => {
                DisplayConversionKind::Primitive
            }
            TypeKind::Class(class) => {
                if self
                    .classes
                    .get(&class.name)
                    .is_some_and(|class| class.implements(BuiltinInterface::Displayable))
                {
                    DisplayConversionKind::DisplayableClass
                } else {
                    DisplayConversionKind::NonDisplayableClass
                }
            }
            TypeKind::Unknown => DisplayConversionKind::Recovery,
            _ => DisplayConversionKind::Excluded,
        }
    }

    fn report_non_displayable_class(&mut self, ty: TypeId, span: Span) {
        let class_name = self.types.display(ty);
        self.diagnostics.push(
            Diagnostic::new(
                "E0462",
                format!(
                    "`{class_name}` cannot be displayed; implement `Displayable` with `function toString(): string`"
                ),
                span,
            )
            .with_help(
                "add `implements Displayable` and an externally accessible readonly `function toString(): string` method",
            ),
        );
    }

    fn is_equality_compatible(&self, left: TypeId, right: TypeId) -> bool {
        if self.type_contains_mixed(left) || self.type_contains_mixed(right) {
            return false;
        }

        if matches!(
            self.types.kind(left),
            TypeKind::Nullable(_) | TypeKind::Null
        ) || matches!(
            self.types.kind(right),
            TypeKind::Nullable(_) | TypeKind::Null
        ) {
            return false;
        }

        self.is_assignable(left, right) || self.is_assignable(right, left)
    }

    fn is_runtime_collection_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::Bytes
                | TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_)
                | TypeKind::EmptyCollection
        )
    }

    fn is_supported_nullable_equality(
        &mut self,
        left: &Expr,
        left_ty: TypeId,
        right: &Expr,
        right_ty: TypeId,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let left_is_null = Self::is_null_literal(left);
        let right_is_null = Self::is_null_literal(right);
        if left_is_null || right_is_null {
            if left_is_null && right_is_null {
                return false;
            }
            let (other, other_ty) = if left_is_null {
                (right, right_ty)
            } else {
                (left, left_ty)
            };
            return self.expr_declares_nullable(other, scopes, method_context)
                || matches!(self.types.kind(other_ty), TypeKind::Nullable(_));
        }

        matches!(
            (self.types.kind(left_ty), self.types.kind(right_ty)),
            (TypeKind::String, TypeKind::Nullable(inner))
                | (TypeKind::Nullable(inner), TypeKind::String)
                if matches!(self.types.kind(*inner), TypeKind::String)
        ) || matches!(
            (self.types.kind(left_ty), self.types.kind(right_ty)),
            (TypeKind::Nullable(left), TypeKind::Nullable(right))
                if matches!(self.types.kind(*left), TypeKind::String)
                    && matches!(self.types.kind(*right), TypeKind::String)
        )
    }

    fn is_null_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => Self::is_null_literal(expr),
            Expr::Null { .. } => true,
            _ => false,
        }
    }

    fn logical_operator_name(op: &BinaryOp) -> &'static str {
        match op {
            BinaryOp::And => "`and`/`&&`",
            BinaryOp::Or => "`or`/`||`",
            BinaryOp::Xor => "`xor`",
            _ => "logical operator",
        }
    }

    fn check_mixed_operation(
        &mut self,
        expr: &Expr,
        operation: &'static str,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        if self.is_mixed_type(ty) {
            self.report_mixed_operation(expr.span(), operation);
        }
    }

    fn check_mixed_value_operation(
        &mut self,
        expr: &Expr,
        operation: &'static str,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        if self.type_contains_mixed(ty) {
            self.report_mixed_operation(expr.span(), operation);
        }
    }

    fn check_mixed_binary_operands(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        // Null-coalescing and `== null` / `!= null` are the only operations permitted on
        // an un-narrowed value, but only when that value is actually nullable (`?mixed`).
        // A bare, non-null `mixed` has nothing to coalesce and cannot be null, so those
        // forms must still be reported here rather than being admitted by `check`/IDE
        // analysis and then rejected during MIR lowering.
        let bypass = match op {
            BinaryOp::Coalesce => self.expr_declares_nullable(left, scopes, method_context),
            BinaryOp::Equal | BinaryOp::NotEqual
                if Self::is_null_literal(left) || Self::is_null_literal(right) =>
            {
                let operand = if Self::is_null_literal(left) {
                    right
                } else {
                    left
                };
                self.expr_declares_nullable(operand, scopes, method_context)
            }
            _ => false,
        };
        if bypass {
            return;
        }
        if self.has_mixed_operand(left, right, scopes, method_context) {
            self.report_mixed_operation(span, "operator");
        }
    }

    fn expr_declares_nullable(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        // Use the operand's DECLARED nullability, not its flow-narrowed type: a `?mixed`
        // binding still permits `== null` / `??` after a `!= null` guard or a non-null
        // assignment has narrowed it (a redundant but valid check), while a bare `mixed`
        // (never nullable-declared) is reported.
        let mut operand = expr;
        while let Expr::Grouped { expr, .. } = operand {
            operand = expr;
        }
        if let Expr::Variable { name, .. } = operand {
            if let Some(binding) = scopes.lookup(name) {
                return matches!(self.types.kind(binding.declared_ty), TypeKind::Nullable(_));
            }
        }
        let ty = self.infer_expr_type(operand, scopes, method_context);
        matches!(self.types.kind(ty), TypeKind::Nullable(_))
    }

    fn has_mixed_operand(
        &mut self,
        left: &Expr,
        right: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let left_ty = self.infer_expr_type(left, scopes, method_context);
        let right_ty = self.infer_expr_type(right, scopes, method_context);

        self.type_contains_mixed(left_ty) || self.type_contains_mixed(right_ty)
    }

    fn is_mixed_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Mixed)
    }

    fn type_contains_mixed(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::Mixed => true,
            TypeKind::Nullable(inner) => self.type_contains_mixed(*inner),
            TypeKind::TypedArray(element)
            | TypeKind::List(element)
            | TypeKind::Set(element)
            | TypeKind::SortedSet(element)
            | TypeKind::PriorityQueue(element)
            | TypeKind::Deque(element) => self.type_contains_mixed(*element),
            TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) => {
                self.type_contains_mixed(*key) || self.type_contains_mixed(*value)
            }
            TypeKind::Function(function) => {
                function
                    .parameters
                    .iter()
                    .any(|parameter| self.type_contains_mixed(parameter.ty))
                    || self.type_contains_mixed(function.return_type)
                    || function
                        .checked_effects
                        .iter()
                        .any(|effect| self.type_contains_mixed(*effect))
            }
            _ => false,
        }
    }

    fn type_is_move_type(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::Nullable(inner) => self.type_is_move_type(*inner),
            TypeKind::Enum(enum_type) => self
                .enums
                .values()
                .find(|definition| definition.id == enum_type.id)
                .is_some_and(|definition| !definition.capabilities.copy),
            TypeKind::Class(_)
            | TypeKind::Function(_)
            | TypeKind::Error
            | TypeKind::SharedHandle(_, _)
            | TypeKind::TypeParameter(_)
            | TypeKind::Bytes
            | TypeKind::Mixed
            | TypeKind::TypedArray(_)
            | TypeKind::List(_)
            | TypeKind::Dictionary(_, _)
            | TypeKind::SortedDictionary(_, _)
            | TypeKind::Set(_)
            | TypeKind::SortedSet(_)
            | TypeKind::PriorityQueue(_)
            | TypeKind::Deque(_)
            | TypeKind::EmptyCollection
            | TypeKind::Heterogeneous => true,
            _ => false,
        }
    }

    fn type_can_return_borrow(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::Nullable(inner) => self.type_can_return_borrow(*inner),
            TypeKind::Class(_) | TypeKind::TypeParameter(_) => true,
            _ => false,
        }
    }

    fn type_is_symbolic(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::TypeParameter(_) => true,
            TypeKind::Nullable(inner)
            | TypeKind::TypedArray(inner)
            | TypeKind::List(inner)
            | TypeKind::Set(inner)
            | TypeKind::SortedSet(inner)
            | TypeKind::PriorityQueue(inner)
            | TypeKind::Deque(inner)
            | TypeKind::SharedHandle(_, inner) => self.type_is_symbolic(*inner),
            TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) => {
                self.type_is_symbolic(*key) || self.type_is_symbolic(*value)
            }
            TypeKind::Class(class) => class
                .arguments
                .iter()
                .any(|argument| self.type_is_symbolic(*argument)),
            TypeKind::Function(function) => {
                function
                    .parameters
                    .iter()
                    .any(|parameter| self.type_is_symbolic(parameter.ty))
                    || self.type_is_symbolic(function.return_type)
                    || function
                        .checked_effects
                        .iter()
                        .any(|effect| self.type_is_symbolic(*effect))
            }
            _ => false,
        }
    }

    fn class_type(&self, ty: TypeId) -> Option<ClassType<TypeId>> {
        match self.types.kind(ty) {
            TypeKind::Class(class) => Some(class.clone()),
            TypeKind::Nullable(inner) => self.class_type(*inner),
            _ => None,
        }
    }

    fn class_type_substitutions(&self, class: &ClassType<TypeId>) -> HashMap<String, TypeId> {
        self.classes
            .get(&class.name)
            .map(|info| {
                info.type_params
                    .iter()
                    .zip(&class.arguments)
                    .map(|(param, argument)| (param.name.clone(), *argument))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn substitute_type_id(
        &mut self,
        ty: TypeId,
        substitutions: &HashMap<String, TypeId>,
    ) -> TypeId {
        match self.types.kind(ty).clone() {
            TypeKind::TypeParameter(name) => substitutions.get(&name).copied().unwrap_or(ty),
            TypeKind::Nullable(inner) => {
                let inner = self.substitute_type_id(inner, substitutions);
                // `?(?X)` collapses to `?X`: a `?T` member substituted with a
                // nullable argument is already nullable, not doubly-nullable.
                if matches!(self.types.kind(inner), TypeKind::Nullable(_)) {
                    inner
                } else {
                    self.types.intern(TypeKind::Nullable(inner))
                }
            }
            TypeKind::TypedArray(element) => {
                let element = self.substitute_type_id(element, substitutions);
                self.types.intern(TypeKind::TypedArray(element))
            }
            TypeKind::List(element) => {
                let element = self.substitute_type_id(element, substitutions);
                self.types.intern(TypeKind::List(element))
            }
            TypeKind::Dictionary(key, value) => {
                let key = self.substitute_type_id(key, substitutions);
                let value = self.substitute_type_id(value, substitutions);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            TypeKind::SortedDictionary(key, value) => {
                let key = self.substitute_type_id(key, substitutions);
                let value = self.substitute_type_id(value, substitutions);
                self.types.intern(TypeKind::SortedDictionary(key, value))
            }
            TypeKind::Set(element) => {
                let element = self.substitute_type_id(element, substitutions);
                self.types.intern(TypeKind::Set(element))
            }
            TypeKind::SortedSet(element) => {
                let element = self.substitute_type_id(element, substitutions);
                self.types.intern(TypeKind::SortedSet(element))
            }
            TypeKind::PriorityQueue(element) => {
                let element = self.substitute_type_id(element, substitutions);
                self.types.intern(TypeKind::PriorityQueue(element))
            }
            TypeKind::Deque(element) => {
                let element = self.substitute_type_id(element, substitutions);
                self.types.intern(TypeKind::Deque(element))
            }
            TypeKind::SharedHandle(kind, payload) => {
                let payload = self.substitute_type_id(payload, substitutions);
                self.types.intern(TypeKind::SharedHandle(kind, payload))
            }
            TypeKind::Function(function) => {
                let function = SemanticFunctionType {
                    invocation_mode: function.invocation_mode,
                    parameters: function
                        .parameters
                        .into_iter()
                        .map(|parameter| SemanticFunctionParameter {
                            ownership_mode: parameter.ownership_mode,
                            ty: self.substitute_type_id(parameter.ty, substitutions),
                        })
                        .collect(),
                    return_type: self.substitute_type_id(function.return_type, substitutions),
                    checked_effects: function
                        .checked_effects
                        .into_iter()
                        .map(|effect| self.substitute_type_id(effect, substitutions))
                        .collect(),
                    return_borrow: function.return_borrow,
                };
                self.types.intern(TypeKind::Function(function))
            }
            TypeKind::Class(class) => {
                let arguments = class
                    .arguments
                    .into_iter()
                    .map(|argument| self.substitute_type_id(argument, substitutions))
                    .collect();
                self.types
                    .intern(TypeKind::Class(ClassType::new(class.name, arguments)))
            }
            _ => ty,
        }
    }

    fn specialize_method_for_class(
        &mut self,
        method: &MethodInfo,
        class: &ClassType<TypeId>,
    ) -> MethodInfo {
        let substitutions = self.class_type_substitutions(class);
        let mut specialized = method.clone();
        specialized
            .enclosing_type_bindings
            .extend(substitutions.clone());
        for param in &mut specialized.params {
            param.ty = self.substitute_type_id(param.ty, &substitutions);
        }
        specialized.return_ty = self.substitute_type_id(method.return_ty, &substitutions);
        specialized.checked_effects = method
            .checked_effects
            .iter()
            .map(|effect| self.substitute_type_id(*effect, &substitutions))
            .collect();
        specialized
    }

    fn specialize_property_for_class(
        &mut self,
        property: &PropertyInfo,
        class: &ClassType<TypeId>,
    ) -> PropertyInfo {
        let mut specialized = property.clone();
        specialized.ty =
            self.substitute_type_id(property.ty, &self.class_type_substitutions(class));
        specialized
    }

    fn report_mixed_operation(&mut self, span: Span, operation: &'static str) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0433",
                format!("cannot use `mixed` value in {operation} before narrowing"),
                span,
            )
            .with_help("narrow the value with `is` before using it"),
        );
    }

    fn check_foreach_binding_type(&mut self, target: TypeId, value: TypeId, span: Span) {
        if self.is_unknown_type(target) || self.is_unknown_type(value) {
            return;
        }

        if self.type_contains_mixed(value) && !self.type_contains_mixed(target) {
            self.report_mixed_operation(span, "foreach binding");
            return;
        }

        if !self.is_assignable(target, value) {
            self.check_assignable(target, value, span, AssignmentDestination::Type);
        }
    }

    fn readonly_int_constant(
        &self,
        writable: bool,
        ty: TypeId,
        initializer: &Expr,
        scopes: &ScopeStack,
    ) -> Option<IntegerValue> {
        let TypeKind::Integer(integer) = *self.types.kind(ty) else {
            return None;
        };
        if writable {
            return None;
        }

        match Self::eval_int_constant(initializer, scopes, integer) {
            IntConstantEval::Known(value) => Some(value),
            IntConstantEval::Unknown | IntConstantEval::Invalid => None,
        }
    }

    fn eval_int_constant(expr: &Expr, scopes: &ScopeStack, target: IntegerType) -> IntConstantEval {
        match expr {
            Expr::Int { value, .. } => parse_decimal_magnitude(value)
                .and_then(|magnitude| IntegerValue::from_literal(target, magnitude, false))
                .map(IntConstantEval::Known)
                .unwrap_or(IntConstantEval::Invalid),
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .and_then(|binding| binding.int_constant)
                .filter(|value| value.ty == target)
                .map(IntConstantEval::Known)
                .unwrap_or(IntConstantEval::Unknown),
            Expr::Grouped { expr, .. } => Self::eval_int_constant(expr, scopes, target),
            Expr::Unary {
                op: UnaryOp::Negate,
                expr,
                ..
            } => match Self::unsigned_integer_literal_magnitude(expr)
                .and_then(|magnitude| IntegerValue::from_literal(target, magnitude, true))
            {
                Some(value) => IntConstantEval::Known(value),
                None => IntConstantEval::Invalid,
            },
            Expr::Binary {
                left, op, right, ..
            } if Self::is_checked_int_arithmetic_op(op) => {
                let left = Self::eval_int_constant(left, scopes, target);
                let right = Self::eval_int_constant(right, scopes, target);
                match (left, right) {
                    (IntConstantEval::Known(left), IntConstantEval::Known(right)) => {
                        Self::checked_int_arithmetic(left, op, right)
                            .map(IntConstantEval::Known)
                            .unwrap_or(IntConstantEval::Invalid)
                    }
                    (IntConstantEval::Invalid, _) | (_, IntConstantEval::Invalid) => {
                        IntConstantEval::Invalid
                    }
                    _ => IntConstantEval::Unknown,
                }
            }
            _ => IntConstantEval::Unknown,
        }
    }

    fn readonly_string_constant(
        &self,
        writable: bool,
        ty: TypeId,
        initializer: &Expr,
        scopes: &ScopeStack,
    ) -> Option<String> {
        if writable || !matches!(self.types.kind(ty), TypeKind::String) {
            return None;
        }

        Self::eval_string_constant(initializer, scopes)
    }

    fn eval_string_constant(expr: &Expr, scopes: &ScopeStack) -> Option<String> {
        match expr {
            Expr::String { value, .. } => Some(value.clone()),
            Expr::Variable { name, .. } => scopes
                .lookup(name)
                .and_then(|binding| binding.string_constant.clone()),
            Expr::Grouped { expr, .. } => Self::eval_string_constant(expr, scopes),
            Expr::Binary {
                left,
                op: BinaryOp::Concat,
                right,
                ..
            } => {
                let mut value = Self::eval_string_constant(left, scopes)?;
                value.push_str(&Self::eval_string_constant(right, scopes)?);
                Some(value)
            }
            _ => None,
        }
    }

    fn is_checked_int_arithmetic_op(op: &BinaryOp) -> bool {
        matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
    }

    fn checked_int_arithmetic(
        left: IntegerValue,
        op: &BinaryOp,
        right: IntegerValue,
    ) -> Option<IntegerValue> {
        match op {
            BinaryOp::Add => left.checked_add(right).ok(),
            BinaryOp::Sub => left.checked_sub(right).ok(),
            BinaryOp::Mul => left.checked_mul(right).ok(),
            _ => None,
        }
    }

    fn check_interpolated_string(
        &mut self,
        parts: &[InterpolatedStringPart],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        for part in parts {
            let InterpolatedStringPart::Expr(expr) = part else {
                continue;
            };

            self.check_expr(expr, scopes, method_context);
            let ty = self.infer_expr_type(expr, scopes, method_context);
            if matches!(
                self.display_conversion_kind(ty),
                DisplayConversionKind::NonDisplayableClass
            ) {
                self.report_non_displayable_class(ty, expr.span());
            } else if !self.is_display_convertible_type(ty) {
                let ty_name = self.types.display(ty);
                self.diagnostics.push(Diagnostic::new(
                    "E0415",
                    format!("value of type {ty_name} cannot be interpolated into a string"),
                    expr.span(),
                ));
            }
        }
    }

    fn check_writable_place(
        &mut self,
        target: &Expr,
        op: &AssignOp,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) -> Option<AssignmentTarget> {
        match target {
            Expr::Grouped { expr, .. } => self.check_writable_place(
                expr,
                op,
                scopes,
                method_context,
                constructor_init_context,
            ),
            Expr::Variable { name, span } => match scopes.lookup(name).cloned() {
                Some(binding) => {
                    self.record_binding_use(&binding, *span, CaptureRequirement::Writable);
                    if !binding.writable {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0201",
                                format!("cannot assign to readonly variable `${name}`"),
                                *span,
                            )
                            .with_title("Cannot Write to Readonly Binding")
                            .with_primary_label("This Assignment Needs Writable Access")
                            .with_explanation(
                                "Readonly bindings may be initialized once but cannot be assigned another value.",
                            )
                            .with_help(format!(
                                "Declare it as `let writable ${name} = ...` if mutation is intended."
                            )),
                        );
                    }
                    Some(AssignmentTarget {
                        ty: if matches!(op, AssignOp::Assign)
                            && matches!(self.types.kind(binding.declared_ty), TypeKind::Nullable(_))
                        {
                            binding.declared_ty
                        } else {
                            self.infer_expr_type(target, scopes, method_context)
                        },
                        destination: AssignmentDestination::Type,
                    })
                }
                None => {
                    self.undeclared_variable(name, *span);
                    None
                }
            },
            Expr::PropertyAccess {
                object,
                property,
                null_safe,
                span,
                ..
            } => {
                if *null_safe {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0511",
                            "null-safe property access cannot be used as a write target",
                            *span,
                        )
                        .with_help(
                            "narrow the receiver to a non-null value, then write through `->`",
                        ),
                    );
                    return None;
                }
                self.check_expr(object, scopes, method_context);
                self.check_nullable_member_access(
                    object,
                    false,
                    "property write",
                    scopes,
                    method_context,
                );
                self.record_capture_requirement_for_expr(
                    object,
                    scopes,
                    CaptureRequirement::Writable,
                );
                self.check_mixed_operation(object, "property write", scopes, method_context);
                if self.reject_nonforwarding_shared_handle_member_access(
                    object,
                    property,
                    false,
                    *span,
                    scopes,
                    method_context,
                ) {
                    return None;
                }
                let object_ty = self.infer_expr_type(object, scopes, method_context);
                if matches!(self.types.kind(object_ty), TypeKind::Enum(_)) {
                    self.diagnostics.push(
                        Diagnostic::new("E0578", "enum properties are readonly", *span)
                            .with_title("Enum Value Is Readonly"),
                    );
                    return None;
                }
                if !Self::is_property_write_object_path(object) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0204",
                            "property assignment requires a stable object path",
                            object.span(),
                        )
                        .with_help(
                            "bind the object to a writable local before assigning its property",
                        ),
                    );
                    return None;
                }
                if let Some((class_name, property_info)) =
                    self.lookup_property(object, property, *span, scopes, method_context)
                {
                    let constructor_context = Self::is_direct_this(object)
                        && constructor_init_context
                            .as_deref()
                            .is_some_and(|context| context.class_name == class_name);
                    let constructor_init_decision = if Self::is_direct_this(object) {
                        self.check_constructor_init_assignment(
                            &class_name,
                            property,
                            &property_info,
                            op,
                            *span,
                            constructor_init_context,
                        )
                    } else {
                        ConstructorInitDecision::NotApplicable
                    };

                    if matches!(
                        constructor_init_decision,
                        ConstructorInitDecision::NotApplicable
                    ) {
                        let writable_path =
                            self.is_writable_object_path(object, scopes, method_context);
                        if !writable_path {
                            let message = if Self::is_direct_this(object) {
                                "cannot mutate `$this` in a readonly method".to_string()
                            } else {
                                match object.as_ref() {
                                    Expr::Variable { name, .. } => {
                                        format!("cannot write through readonly variable `${name}`")
                                    }
                                    _ => "cannot write through readonly object path".to_string(),
                                }
                            };
                            self.diagnostics
                                .push(Diagnostic::new("E0201", message, object.span()));
                        }

                        if !property_info.writable {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0202",
                                    format!(
                                        "cannot assign to readonly property `{class_name}::{property}`"
                                    ),
                                    *span,
                                )
                                .with_title(format!(
                                    "Cannot Write to Readonly Property `{property}`"
                                ))
                                .with_primary_label("This Operation Needs Writable Access")
                                .with_explanation(
                                    "This assignment changes a property whose declaration does not grant writable access.",
                                )
                                .with_related(
                                    property_info.declaration_span,
                                    format!("`{property}` Is Readonly Here"),
                                )
                                .with_help(format!(
                                    "Mark the property writable: `writable {} ${property};`",
                                    self.types.display(property_info.ty)
                                )),
                            );
                        }
                    }
                    if !matches!(constructor_init_decision, ConstructorInitDecision::Rejected) {
                        let kind = if constructor_context
                            && matches!(op, AssignOp::Assign)
                            && property_info.init_state == PropertyInitState::Uninitialized
                        {
                            PropertyWriteKind::Initialize
                        } else {
                            PropertyWriteKind::Replace
                        };
                        self.property_writes.insert(
                            target.span(),
                            PropertyWriteSemanticInfo {
                                kind,
                                class_name: class_name.clone(),
                                property_name: property.clone(),
                                constructor_context,
                            },
                        );
                    }
                    Some(AssignmentTarget {
                        ty: property_info.ty,
                        destination: AssignmentDestination::Property {
                            class_name,
                            name: property.clone(),
                        },
                    })
                } else {
                    None
                }
            }
            Expr::Index {
                collection,
                index,
                span,
            } => {
                self.check_expr(collection, scopes, method_context);
                self.check_expr(index, scopes, method_context);
                self.check_collection_index(collection, index, *span, scopes, method_context);
                let (_, value) = self.collection_index_types(collection, scopes, method_context)?;
                self.record_capture_requirement_for_expr(
                    collection,
                    scopes,
                    CaptureRequirement::Writable,
                );
                if !self.is_writable_object_path(collection, scopes, method_context) {
                    self.diagnostics.push(Diagnostic::new(
                        "E0201",
                        "cannot write through a readonly collection value",
                        collection.span(),
                    ));
                }
                Some(AssignmentTarget {
                    ty: value,
                    destination: AssignmentDestination::Type,
                })
            }
            Expr::StaticMember {
                qualifier,
                qualifier_span,
                member,
                member_span,
                member_sigil_span,
                span,
            } => self.check_static_assignment_target(
                StaticAccess {
                    qualifier,
                    qualifier_span: *qualifier_span,
                    member_sigil_span: *member_sigil_span,
                    member,
                    member_span: *member_span,
                    span: *span,
                },
                method_context,
            ),
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    "E0204",
                    "unsupported mutation target",
                    target.span(),
                ));
                None
            }
        }
    }

    fn record_capture_requirement_for_expr(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        requirement: CaptureRequirement,
    ) {
        match expr {
            Expr::Grouped { expr, .. } => {
                self.record_capture_requirement_for_expr(expr, scopes, requirement)
            }
            Expr::Variable { name, span } => {
                if let Some(binding) = scopes.lookup(name).cloned() {
                    self.record_binding_use(&binding, *span, requirement);
                }
            }
            Expr::This { span } => {
                if let Some(binding) = scopes.lookup("this").cloned() {
                    self.record_binding_use(&binding, *span, requirement);
                }
            }
            Expr::PropertyAccess { object, .. }
            | Expr::Index {
                collection: object, ..
            } => self.record_capture_requirement_for_expr(object, scopes, requirement),
            _ => {}
        }
    }

    fn check_constructor_init_assignment(
        &mut self,
        class_name: &str,
        property: &str,
        property_info: &PropertyInfo,
        op: &AssignOp,
        span: Span,
        constructor_init_context: Option<&mut ConstructorInitContext>,
    ) -> ConstructorInitDecision {
        let Some(context) = constructor_init_context else {
            return ConstructorInitDecision::NotApplicable;
        };

        if context.class_name != class_name {
            return ConstructorInitDecision::NotApplicable;
        }

        if property_info.writable {
            return ConstructorInitDecision::Allowed;
        }

        if !matches!(op, AssignOp::Assign) {
            self.diagnostics.push(Diagnostic::new(
                "E0413",
                "constructor init access only applies to simple `$this->property = value` assignments",
                span,
            ));
            return ConstructorInitDecision::Rejected;
        }

        if context.repeatable_body {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0504",
                    format!(
                        "readonly property `{class_name}::{property}` cannot be initialized inside a repeatable constructor body"
                    ),
                    span,
                )
                .with_help("initialize the property on each non-repeating constructor path"),
            );
            return ConstructorInitDecision::Rejected;
        }

        ConstructorInitDecision::Allowed
    }

    fn check_function_call(
        &mut self,
        name: &str,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if name == "print" {
            self.diagnostics.push(
                Diagnostic::new("E0462", "Doria does not support `print`; use `echo`", span)
                    .with_help("echo writes output and does not return a value"),
            );
            for arg in args {
                self.check_expr(&arg.value, scopes, method_context);
            }
            return;
        }

        if name == "panic" {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0436",
                    "`panic` may only be called as a standalone statement",
                    span,
                )
                .with_help("use `panic(\"message\");` as its own statement"),
            );
            self.check_panic_call(args, span, scopes, method_context);
            return;
        }

        if let Some(builtin) = Builtin::from_name(name) {
            self.check_builtin_call(builtin, args, span, scopes, method_context);
            return;
        }

        let Some(function_info) = self.functions.get(name).cloned() else {
            if let Some((replacement, executable, direct_replacement)) = match name {
                "str_starts_with" => Some((
                    "String::startsWith($text, $prefix)",
                    true,
                    Some("String::startsWith"),
                )),
                "str_ends_with" => Some((
                    "String::endsWith($text, $suffix)",
                    true,
                    Some("String::endsWith"),
                )),
                "str_contains" => Some((
                    "String::contains($text, $needle)",
                    true,
                    Some("String::contains"),
                )),
                "str_case_compare" => {
                    Some(("String::compareIgnoreCase($left, $right)", false, None))
                }
                "explode" => Some(("String::split($text, $separator)", true, None)),
                "implode" => Some((
                    "String::join($separator, $values)",
                    true,
                    Some("String::join"),
                )),
                "substr" => Some((
                    "String::slice($text, $start, $length)",
                    true,
                    Some("String::slice"),
                )),
                _ => None,
            } {
                let mut diagnostic = Diagnostic::new(
                    "E0461",
                    format!("`{name}` is not a Doria string operation"),
                    span,
                )
                .with_title("Use The String Companion")
                .with_primary_label("Removed Or Foreign String Spelling")
                .with_explanation(
                    "Doria keeps string-specific operations on the `String` companion and has no public `str_*` family.",
                )
                .with_help(format!("write `{replacement}`"));
                if !executable {
                    diagnostic = diagnostic.with_help(
                        "`String::compareIgnoreCase` is accepted but remains pending until `Ordering` is executable; use `String::equalsIgnoreCase` for equality",
                    );
                } else if let Some(direct_replacement) = direct_replacement {
                    diagnostic = diagnostic.with_fix(
                        Span::new(span.start, span.start + name.len()),
                        direct_replacement,
                    );
                }
                self.diagnostics.push(diagnostic);
                return;
            }
            if let Some(suggestion) = php_function_suggestion(name) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0461",
                        format!("unknown function `{name}`; did you mean `{suggestion}`?"),
                        span,
                    )
                    .with_help(format!("replace `{name}()` with `{suggestion}()`")),
                );
                for arg in args {
                    self.check_expr(&arg.value, scopes, method_context);
                }
            } else {
                self.diagnostics.push(Diagnostic::new(
                    "E0309",
                    format!("unknown function `{name}`"),
                    span,
                ));
            }
            return;
        };

        self.call_targets.insert(
            span,
            CallableTarget::Function {
                name: name.to_string(),
            },
        );
        self.record_callable_dependency(function_info.declaration);
        let function_info = self.instantiate_generic_call(
            &format!("function `{name}`"),
            &function_info,
            args,
            span,
            scopes,
            method_context,
        );
        let diagnostics_before = self.diagnostics.len();
        self.check_call_arguments(
            &format!("function `{name}`"),
            &function_info.params,
            args,
            span,
            scopes,
            method_context,
        );
        if self.diagnostics.len() == diagnostics_before {
            self.record_checked_effects(function_info.checked_effects, span);
        }
    }

    fn check_builtin_call(
        &mut self,
        builtin: Builtin,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if self.reject_named_arguments(builtin.name(), args) {
            return;
        }
        if matches!(builtin, Builtin::Panic) {
            return self.check_panic_call(args, span, scopes, method_context);
        }
        // One compiler-owned arity definition drives every builtin, including the
        // optional prompt on `read_line`.
        let expected = builtin.arity();
        if let Some((minimum, maximum)) = expected {
            if args.len() < minimum || args.len() > maximum {
                let requirement = if minimum == maximum {
                    format!("exactly {minimum}")
                } else if minimum == 0 {
                    format!("at most {maximum}")
                } else {
                    format!("between {minimum} and {maximum}")
                };
                let plural = if maximum == 1 {
                    "argument"
                } else {
                    "arguments"
                };
                self.diagnostics.push(Diagnostic::new(
                    "E0450",
                    format!(
                        "{} expects {requirement} {plural}, got {}",
                        builtin.name(),
                        args.len()
                    ),
                    span,
                ));
                return;
            }
        } else if args.is_empty() {
            self.diagnostics.push(Diagnostic::new(
                "E0451",
                format!("{} expects a literal format argument", builtin.name()),
                span,
            ));
            return;
        }

        let diagnostics_before = self.diagnostics.len();
        match builtin {
            Builtin::ReadFile | Builtin::WriteStderr => {
                self.require_builtin_string_arg(builtin, &args[0].value, scopes, method_context)
            }
            Builtin::WriteFile | Builtin::AppendFile => {
                self.require_builtin_string_arg(builtin, &args[0].value, scopes, method_context);
                self.require_builtin_string_arg(builtin, &args[1].value, scopes, method_context);
            }
            Builtin::ReadFileBytes => {
                self.require_builtin_string_arg(builtin, &args[0].value, scopes, method_context);
            }
            Builtin::WriteFileBytes | Builtin::AppendFileBytes => {
                self.require_builtin_string_arg(builtin, &args[0].value, scopes, method_context);
                self.require_builtin_bytes_arg(builtin, &args[1].value, scopes, method_context);
            }
            Builtin::WriteStdoutBytes | Builtin::WriteStderrBytes => {
                self.require_builtin_bytes_arg(builtin, &args[0].value, scopes, method_context);
            }
            Builtin::Sprintf | Builtin::Printf => {
                let Some(Argument {
                    value: Expr::String { value, span },
                    ..
                }) = args.first()
                else {
                    self.diagnostics.push(Diagnostic::new(
                        "E0452",
                        format!("{} format must be a direct string literal", builtin.name()),
                        args[0].value.span(),
                    ));
                    return;
                };
                let pieces = match format_string::parse(value, *span) {
                    Ok(pieces) => pieces,
                    Err(diagnostic) => {
                        self.diagnostics.push(diagnostic);
                        return;
                    }
                };
                let specs = pieces
                    .iter()
                    .filter_map(|piece| match piece {
                        FormatPiece::Argument { spec, .. } => Some(*spec),
                        FormatPiece::Literal(_) => None,
                    })
                    .collect::<Vec<_>>();
                if args.len() - 1 != specs.len() {
                    self.diagnostics.push(Diagnostic::new(
                        "E0456",
                        format!(
                            "{} format expects {} arguments, got {}",
                            builtin.name(),
                            specs.len(),
                            args.len() - 1
                        ),
                        *span,
                    ));
                    return;
                }
                for (argument, spec) in args[1..].iter().zip(specs) {
                    let ty = self.infer_expr_type(&argument.value, scopes, method_context);
                    if spec.conversion == FormatConversion::Display
                        && matches!(
                            self.display_conversion_kind(ty),
                            DisplayConversionKind::NonDisplayableClass
                        )
                    {
                        self.report_non_displayable_class(ty, argument.value.span());
                        continue;
                    }
                    let valid = match spec.conversion {
                        FormatConversion::Display => self.is_display_convertible_type(ty),
                        FormatConversion::Decimal
                        | FormatConversion::HexLower
                        | FormatConversion::HexUpper
                        | FormatConversion::Octal
                        | FormatConversion::Binary => {
                            matches!(
                                self.types.kind(ty),
                                TypeKind::Integer(_) | TypeKind::Unknown
                            )
                        }
                        FormatConversion::Float => {
                            matches!(self.types.kind(ty), TypeKind::Float(_) | TypeKind::Unknown)
                        }
                    };
                    if !valid {
                        self.diagnostics.push(Diagnostic::new(
                            "E0457",
                            format!(
                                "format conversion `{}` does not accept `{}`",
                                spec.conversion.specifier(),
                                self.types.display(ty)
                            ),
                            argument.value.span(),
                        ));
                    }
                }
            }
            Builtin::ReadLine => {
                // The prompt is exactly `string`; callers convert before calling.
                if let Some(prompt) = args.first() {
                    self.require_builtin_string_arg(builtin, &prompt.value, scopes, method_context);
                }
            }
            Builtin::ReadStdinBytes | Builtin::Panic => {}
        }
        if self.diagnostics.len() == diagnostics_before {
            self.record_compiler_known_effects(builtin.required_error_types(), span);
            self.record_compiler_known_effects(builtin.ambient_error_types(), span);
        }
    }

    fn record_compiler_known_effects(&mut self, names: &[&str], span: Span) {
        let effects = names
            .iter()
            .map(|name| self.resolve_type_ref(&TypeRef::named(*name), span))
            .collect::<Vec<_>>();
        self.record_checked_effects(effects, span);
    }

    fn require_builtin_bytes_arg(
        &mut self,
        builtin: Builtin,
        argument: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(argument, scopes, method_context);
        if !matches!(self.types.kind(ty), TypeKind::Bytes | TypeKind::Unknown) {
            self.diagnostics.push(Diagnostic::new(
                "E0453",
                format!(
                    "{} expects `Bytes`, got `{}`",
                    builtin.name(),
                    self.types.display(ty)
                ),
                argument.span(),
            ));
        }
    }

    fn require_builtin_string_arg(
        &mut self,
        builtin: Builtin,
        argument: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(argument, scopes, method_context);
        if !matches!(self.types.kind(ty), TypeKind::String | TypeKind::Unknown) {
            self.diagnostics.push(Diagnostic::new(
                "E0453",
                format!(
                    "{} expects `string`, got `{}`",
                    builtin.name(),
                    self.types.display(ty)
                ),
                argument.span(),
            ));
        }
    }

    fn check_panic_call(
        &mut self,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if self.reject_named_arguments("panic", args) {
            return;
        }
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                "E0434",
                format!("panic expects exactly 1 argument, got {}", args.len()),
                span,
            ));
            return;
        }

        let message = &args[0].value;
        let message_ty = self.infer_expr_type(message, scopes, method_context);
        if !matches!(
            self.types.kind(message_ty),
            TypeKind::String | TypeKind::Unknown
        ) {
            self.diagnostics.push(Diagnostic::new(
                "E0435",
                format!(
                    "panic message must be `string`, got `{}`",
                    self.types.display(message_ty)
                ),
                message.span(),
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Argument],
        null_safe: bool,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        if let Some((kind, payload)) = self.shared_handle_type(object_ty, null_safe) {
            for arg in args {
                self.check_expr(&arg.value, scopes, method_context);
            }
            if self
                .shared_handle_member_return_type(kind, payload, method)
                .is_some()
            {
                if !args.is_empty() {
                    self.diagnostics.push(Diagnostic::new(
                        "E0550",
                        format!(
                            "`{}::{method}` Takes No Arguments, But {} Were Given",
                            kind.source_name(),
                            args.len()
                        ),
                        span,
                    ));
                }
                return;
            }
            if self.reject_nonforwarding_shared_handle_member_access(
                object,
                method,
                null_safe,
                span,
                scopes,
                method_context,
            ) {
                return;
            }
            // Falls through: the payload class resolves the member transparently.
        }
        if let TypeKind::TypeParameter(parameter) = self.types.kind(object_ty).clone() {
            if method == "toString"
                && args.is_empty()
                && self.type_parameter_has_constraint(&parameter, "Displayable")
            {
                self.constrained_display_calls.insert(span);
                return;
            }
            self.diagnostics.push(
                Diagnostic::new(
                    "E0537",
                    format!(
                        "method `{method}` is not guaranteed by the constraints on type parameter `{parameter}`"
                    ),
                    span,
                )
                .with_help("add a compiler-known constraint that declares this method"),
            );
            return;
        }
        let Some(class_type) = self.expr_class_type(object, scopes, method_context) else {
            return;
        };
        let class_name = class_type.name.clone();
        let Some(class_info) = self.classes.get(&class_name).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                span,
            ));
            return;
        };
        let Some(method_info) = class_info.methods.get(method).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0304",
                format!("unknown method `{class_name}::{method}`"),
                span,
            ));
            return;
        };

        let class_type_id = self.types.intern(TypeKind::Class(class_type.clone()));
        let ResolvedType::Class(resolved_class_type) = self.types.resolved(class_type_id) else {
            unreachable!("interned class type must resolve as a class");
        };
        self.call_targets.insert(
            span,
            CallableTarget::Method {
                class_type: resolved_class_type,
                method_name: method.to_string(),
            },
        );
        self.record_callable_dependency(method_info.declaration);
        if method_info.is_static {
            self.diagnostics.push(Diagnostic::new(
                "E0487",
                format!("static method `{class_name}::{method}` must be called with `::`"),
                span,
            ));
            return;
        }

        if self.check_direct_lifecycle_method_call(&class_name, method, span) {
            return;
        }

        if matches!(method_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(&class_name, span, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::{method}` is internal"),
                span,
            ));
        }

        if method_info
            .receiver_mode
            .is_some_and(ReceiverMode::is_writable)
        {
            self.record_capture_requirement_for_expr(object, scopes, CaptureRequirement::Writable);
            if !self.is_writable_object_path(object, scopes, method_context) {
                self.diagnostics.push(Diagnostic::new(
                    "E0203",
                    format!(
                        "cannot call writable method `{class_name}::{method}` through readonly value"
                    ),
                    span,
                ));
            }
        }

        let method_info = self.specialize_method_for_class(&method_info, &class_type);
        let method_info = self.instantiate_generic_method_call(
            &format!("method `{class_name}::{method}`"),
            &method_info,
            args,
            span,
            scopes,
            method_context,
        );
        let diagnostics_before = self.diagnostics.len();
        self.check_call_arguments(
            &format!("method `{class_name}::{method}`"),
            &method_info.params,
            args,
            span,
            scopes,
            method_context,
        );
        if self.diagnostics.len() == diagnostics_before {
            self.record_checked_effects(method_info.checked_effects, span);
        }
    }

    fn string_companion_signature(&mut self, method: &str) -> Option<(Vec<ParamInfo>, TypeId)> {
        let string = self.types.intern(TypeKind::String);
        let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
        let bool_ty = self.types.intern(TypeKind::Bool);
        let bytes = self.types.intern(TypeKind::Bytes);
        let nullable_int = self.types.intern(TypeKind::Nullable(int));
        let nullable_string = self.types.intern(TypeKind::Nullable(string));
        let string_list = self.types.intern(TypeKind::List(string));
        let required = |name: &str, ty| ParamInfo {
            name: name.to_string(),
            ty,
            take: false,
            writable: false,
            has_default: false,
        };
        let optional = |name: &str, ty| ParamInfo {
            name: name.to_string(),
            ty,
            take: false,
            writable: false,
            has_default: true,
        };
        let signature = match method {
            "trim" | "trimStart" | "trimEnd" | "lower" | "upper" | "lowerFirst" | "upperFirst" => {
                (vec![required("text", string)], string)
            }
            "contains" | "containsIgnoreCase" => (
                vec![required("text", string), required("needle", string)],
                bool_ty,
            ),
            "startsWith" | "startsWithIgnoreCase" => (
                vec![required("text", string), required("prefix", string)],
                bool_ty,
            ),
            "endsWith" | "endsWithIgnoreCase" => (
                vec![required("text", string), required("suffix", string)],
                bool_ty,
            ),
            "equalsIgnoreCase" => (
                vec![required("left", string), required("right", string)],
                bool_ty,
            ),
            "indexOf" | "lastIndexOf" | "indexOfIgnoreCase" | "lastIndexOfIgnoreCase" => (
                vec![required("text", string), required("needle", string)],
                nullable_int,
            ),
            "countOccurrences" => (
                vec![required("text", string), required("needle", string)],
                int,
            ),
            "replace" => (
                vec![
                    required("text", string),
                    required("search", string),
                    required("replacement", string),
                ],
                string,
            ),
            "split" => (
                vec![required("text", string), required("separator", string)],
                string_list,
            ),
            "join" => (
                vec![
                    required("separator", string),
                    required("values", string_list),
                ],
                string,
            ),
            "slice" => (
                vec![
                    required("text", string),
                    required("start", int),
                    optional("length", nullable_int),
                ],
                string,
            ),
            "repeat" => (
                vec![required("text", string), required("count", int)],
                string,
            ),
            "padStart" | "padEnd" => (
                vec![
                    required("text", string),
                    required("length", int),
                    required("padding", string),
                ],
                string,
            ),
            "fromBytes" => (vec![required("bytes", bytes)], nullable_string),
            _ => return None,
        };
        Some(signature)
    }

    fn check_string_companion_call(
        &mut self,
        method: &str,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if matches!(method, "compare" | "compareIgnoreCase") {
            self.diagnostics.push(
                Diagnostic::unsupported_stage(
                    "E0304",
                    format!("String::{method} requires the executable `Ordering` type"),
                    span,
                )
                .with_help(
                    "use typed equality or `String::equalsIgnoreCase` when only equality is needed",
                ),
            );
            return;
        }
        let Some((params, _)) = self.string_companion_signature(method) else {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0304",
                    format!("unknown String companion operation `String::{method}`"),
                    span,
                )
                .with_help(
                    "use the canonical `String::` surface documented in the standard-library reference",
                ),
            );
            return;
        };
        self.check_call_arguments(
            &format!("String::{method}"),
            &params,
            args,
            span,
            scopes,
            method_context,
        );
    }

    fn check_string_instance_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let ty = self.infer_expr_type(object, scopes, method_context);
        if !matches!(self.types.kind(ty), TypeKind::String) {
            return false;
        }
        let canonical = match method {
            "trim"
            | "trimStart"
            | "trimEnd"
            | "lower"
            | "upper"
            | "lowerFirst"
            | "upperFirst"
            | "contains"
            | "startsWith"
            | "endsWith"
            | "containsIgnoreCase"
            | "startsWithIgnoreCase"
            | "endsWithIgnoreCase"
            | "indexOf"
            | "lastIndexOf"
            | "indexOfIgnoreCase"
            | "lastIndexOfIgnoreCase"
            | "countOccurrences"
            | "replace"
            | "split"
            | "slice"
            | "repeat"
            | "padStart"
            | "padEnd" => Some(method),
            _ => None,
        };
        let mut diagnostic = Diagnostic::new(
            "E0304",
            format!("String operation `{method}` belongs on the `String` companion"),
            span,
        )
        .with_title("Use The String Companion")
        .with_primary_label("String Action Method Alias")
        .with_explanation(
            "String values expose intrinsic measurements and views as properties; string operations use `String::`.",
        );
        if let Some(canonical) = canonical {
            diagnostic = diagnostic.with_help(format!(
                "write `String::{canonical}($text, ...)` and pass the string as the first argument"
            ));
        }
        self.diagnostics.push(diagnostic);
        true
    }

    fn check_static_call(
        &mut self,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if let StaticQualifier::Class(collection) = access.qualifier {
            if matches!(collection.as_str(), "List" | "Dictionary") && access.member == "from" {
                self.report_withdrawn_collection_from(
                    collection,
                    access,
                    args,
                    scopes,
                    method_context,
                );
                return;
            }
        }
        let Some(class_name) = self.resolve_static_qualifier(access, method_context) else {
            return;
        };
        let class_name = class_name.as_str();
        if let Some(definition) = self.enums.get(class_name).cloned() {
            self.check_enum_case_call(&definition, access, args, scopes, method_context);
            return;
        }
        if class_name == "String" {
            self.check_string_companion_call(
                access.member,
                args,
                access.span,
                scopes,
                method_context,
            );
            return;
        }
        if class_name == "Int" && access.member == "toFloat" {
            self.check_cross_kind_intrinsic_argument(
                "Int::toFloat",
                args,
                TypeKind::Integer(IntegerType::Int64),
                access.span,
                scopes,
                method_context,
            );
            return;
        }
        if class_name == "Float" && access.member == "toInt" {
            self.check_cross_kind_intrinsic_argument(
                "Float::toInt",
                args,
                TypeKind::Float(FloatType::Float64),
                access.span,
                scopes,
                method_context,
            );
            return;
        }
        if matches!(class_name, "Float" | "Float64") && access.member == "parse" {
            self.check_parse_intrinsic_argument(
                "Float::parse",
                args,
                access.span,
                scopes,
                method_context,
            );
            return;
        }
        if class_name == "Float32" && access.member == "parse" {
            self.diagnostics.push(Diagnostic::new(
                "E0304",
                "fixed-width `Float32::parse` is not available yet; parse with `Float::parse` and convert"
                    .to_string(),
                access.span,
            ));
            return;
        }
        if class_name == "Set" {
            if access.member != "from" {
                self.diagnostics.push(Diagnostic::unsupported_stage(
                    "E0521",
                    format!(
                        "collection method `Set::{}` is not part of the collection surface settled by Decision 0113",
                        access.member
                    ),
                    access.span,
                ));
                return;
            }
            if self.reject_named_arguments("Set::from", args) {
                return;
            }
            if args.len() != 1 {
                self.report_argument_count_mismatch("Set::from", 1, 1, args.len(), access.span);
                return;
            }
            let source = self.infer_expr_type(&args[0].value, scopes, method_context);
            match self.types.kind(source).clone() {
                TypeKind::TypedArray(element) | TypeKind::List(element) => {
                    self.check_stage23_hashable_type(element, args[0].value.span(), "Set element");
                    self.check_non_consuming_collection_duplication(
                        "Set",
                        "Set::from",
                        element,
                        None,
                        args[0].value.span(),
                    );
                }
                TypeKind::EmptyCollection => {
                    // The destination Set<T> supplies the element type. Lowering
                    // materializes the empty source directly as that Set<T>.
                }
                _ => {
                    self.diagnostics.push(Diagnostic::new(
                        "E0403",
                        format!(
                            "`Set::from` requires a sequence collection, got `{}`",
                            self.types.display(source)
                        ),
                        args[0].value.span(),
                    ));
                }
            }
            return;
        }
        if matches!(
            class_name,
            "SortedDictionary" | "SortedSet" | "PriorityQueue" | "Deque"
        ) {
            self.check_stage26_collection_from_call(
                class_name,
                access.member,
                args,
                access.span,
                scopes,
                method_context,
            );
            return;
        }
        if class_name == "Bytes" {
            if access.member != "fromArray" {
                self.diagnostics.push(Diagnostic::unsupported_stage(
                    "E0524",
                    format!(
                        "Bytes method `Bytes::{}` is deferred to the future Bytes method-surface record",
                        access.member
                    ),
                    access.span,
                ));
                return;
            }
            if self.reject_named_arguments("Bytes::fromArray", args) {
                return;
            }
            if args.len() != 1 {
                self.report_argument_count_mismatch(
                    "Bytes::fromArray",
                    1,
                    1,
                    args.len(),
                    access.span,
                );
                return;
            }
            let uint8 = self.types.intern(TypeKind::Integer(IntegerType::UInt8));
            let expected = self.types.intern(TypeKind::TypedArray(uint8));
            self.check_expr_assignable(
                expected,
                &args[0].value,
                scopes,
                method_context,
                AssignmentDestination::Type,
            );
            return;
        }

        if let Some(target) = IntegerType::from_companion_name(class_name) {
            if self.reject_named_arguments(&format!("{class_name}::{}", access.member), args) {
                return;
            }
            if access.member == "parse" {
                if target != IntegerType::Int64 {
                    self.diagnostics.push(Diagnostic::new(
                        "E0304",
                        format!(
                            "fixed-width `{class_name}::parse` is not available yet; parse with `Int::parse` and convert with `{class_name}::from(...)`"
                        ),
                        access.span,
                    ));
                    return;
                }
                self.check_parse_intrinsic_argument(
                    "Int::parse",
                    args,
                    access.span,
                    scopes,
                    method_context,
                );
                return;
            }
            if access.member != "from" {
                self.diagnostics.push(Diagnostic::new(
                    "E0304",
                    format!(
                        "unknown integer companion intrinsic `{class_name}::{}`; `{class_name}::from(...)` and `Int::parse(...)` are available",
                        access.member
                    ),
                    access.span,
                ));
                return;
            }
            if args.len() != 1 {
                self.diagnostics.push(Diagnostic::new(
                    "E0443",
                    format!(
                        "{}::from expects exactly 1 argument, got {}",
                        target.companion_name(),
                        args.len()
                    ),
                    access.span,
                ));
                return;
            }

            let argument = &args[0].value;
            self.contextualize_integer_literals(argument, IntegerType::Int64);
            let argument_ty = self.infer_expr_type(argument, scopes, method_context);
            if !matches!(
                self.types.kind(argument_ty),
                TypeKind::Integer(_) | TypeKind::Unknown
            ) {
                self.diagnostics.push(Diagnostic::new(
                    "E0443",
                    format!(
                        "{}::from requires an integer argument, got `{}`",
                        target.companion_name(),
                        self.types.display(argument_ty)
                    ),
                    argument.span(),
                ));
            }
            return;
        }

        let Some(class_info) = self.classes.get(class_name).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                access.span,
            ));
            return;
        };
        let Some(method_info) = class_info.methods.get(access.member).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0304",
                format!("unknown method `{class_name}::{}`", access.member),
                access.span,
            ));
            return;
        };

        let target_class = if matches!(access.qualifier, StaticQualifier::SelfType) {
            let class_type = self.symbolic_class_type(class_name);
            match self.types.resolved(class_type) {
                ResolvedType::Class(class_type) => class_type,
                _ => unreachable!("symbolic declaring class type must resolve as a class"),
            }
        } else {
            ClassType::new(class_name, Vec::new())
        };
        self.call_targets.insert(
            access.span,
            CallableTarget::Method {
                class_type: target_class,
                method_name: access.member.to_string(),
            },
        );
        self.record_callable_dependency(method_info.declaration);
        if self.check_direct_lifecycle_method_call(class_name, access.member, access.span) {
            return;
        }

        if !method_info.is_static {
            self.diagnostics.push(Diagnostic::new(
                "E0487",
                format!(
                    "instance method `{class_name}::{}` requires an object receiver",
                    access.member
                ),
                access.span,
            ));
            return;
        }

        if matches!(method_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(class_name, access.span, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::{}` is internal", access.member),
                access.span,
            ));
        }

        let method_info = self.instantiate_generic_method_call(
            &format!("method `{class_name}::{}`", access.member),
            &method_info,
            args,
            access.span,
            scopes,
            method_context,
        );
        let diagnostics_before = self.diagnostics.len();
        self.check_call_arguments(
            &format!("method `{class_name}::{}`", access.member),
            &method_info.params,
            args,
            access.span,
            scopes,
            method_context,
        );
        if self.diagnostics.len() == diagnostics_before {
            self.record_checked_effects(method_info.checked_effects, access.span);
        }
    }

    fn report_withdrawn_collection_from(
        &mut self,
        collection: &str,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let mut diagnostic = Diagnostic::new(
            "E0558",
            format!("`{collection}::from` is not a Doria collection constructor"),
            access.member_span,
        )
        .with_title("Use A Collection Literal")
        .with_primary_label("This Construction Form Was Withdrawn")
        .with_explanation(
            "List and Dictionary use bracket literals. `::from` is reserved for collection types that have no literal form.",
        );

        let expected = self.contextual_expression_types.get(&access.span).copied();
        let direct_literal = match args {
            [Argument {
                name: None,
                value: Expr::Array { elements, span },
                ..
            }] => Some((elements, *span)),
            _ => None,
        };
        let expected_matches_collection = expected.is_none_or(|target| {
            matches!(
                (collection, self.types.kind(target)),
                ("List", TypeKind::List(_)) | ("Dictionary", TypeKind::Dictionary(_, _))
            )
        });
        let literal_matches_context = match (expected, direct_literal) {
            (Some(target), Some((elements, _))) => {
                self.is_array_literal_assignable(target, elements, scopes, method_context)
            }
            (None, Some((elements, _))) if !elements.is_empty() => {
                let inferred = self.infer_array_type(elements, scopes, method_context);
                !matches!(
                    self.types.kind(inferred),
                    TypeKind::Heterogeneous | TypeKind::Unknown
                )
            }
            _ => false,
        };
        let safe_literal = direct_literal.is_some_and(|(elements, _)| {
            let shape_matches = if elements.is_empty() {
                expected.is_some()
            } else if collection == "List" {
                elements.iter().all(|element| element.key.is_none())
            } else {
                elements.iter().all(|element| element.key.is_some())
            };
            expected_matches_collection && literal_matches_context && shape_matches
        });

        if safe_literal {
            let (_, literal_span) = direct_literal.expect("safe direct literal exists");
            if let Some(literal) = self.source_slice(literal_span).map(str::to_owned) {
                diagnostic = diagnostic
                    .with_help("use the bracket literal directly")
                    .with_structured_fix(
                        format!("Replace `{collection}::from` With The Literal"),
                        FixApplicability::MachineApplicable,
                        vec![FixEdit {
                            source: DiagnosticSource::Current,
                            span: access.span,
                            replacement: literal,
                        }],
                    );
            } else {
                diagnostic = diagnostic.with_help("use the bracket literal directly");
            }
        } else if expected.is_some() && !expected_matches_collection {
            diagnostic = diagnostic.with_help(format!(
                "`{collection}::from` does not match the contextual type `{}`; use that type's bracket-literal shape",
                self.types.display(expected.expect("mismatched contextual type exists"))
            ));
        } else if direct_literal.is_some_and(|(elements, _)| elements.is_empty()) {
            diagnostic = diagnostic.with_help(format!(
                "declare the element types explicitly, for example `{collection}<...> $values = []`"
            ));
        } else if direct_literal.is_some() {
            diagnostic = diagnostic.with_help(if collection == "List" {
                "use an unkeyed bracket literal for List values"
            } else {
                "use a keyed bracket literal for Dictionary values"
            });
        } else {
            diagnostic = diagnostic.with_help(
                "iterate the source into an explicitly typed literal-constructible collection; general cross-collection materialization remains deferred",
            );
        }
        self.diagnostics.push(diagnostic);
    }

    fn check_static_member(
        &mut self,
        access: StaticAccess<'_>,
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_name) = self.resolve_static_qualifier(access, method_context) else {
            return;
        };
        let class_name = class_name.as_str();
        if let Some(definition) = self.enums.get(class_name).cloned() {
            self.check_enum_case_member(&definition, access);
            return;
        }
        let Some(class_info) = self.classes.get(class_name) else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                access.span,
            ));
            return;
        };
        let member_access = class_info
            .constants
            .get(access.member)
            .map(|constant| constant.access)
            .or_else(|| {
                class_info
                    .static_properties
                    .get(access.member)
                    .map(|property| property.access)
            });
        let Some(member_access) = member_access else {
            self.diagnostics.push(Diagnostic::new(
                "E0488",
                format!("unknown static member `{class_name}::{}`", access.member),
                access.span,
            ));
            return;
        };
        if member_access == MemberAccess::Internal
            && !self.can_access_internal_member(class_name, access.span, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!(
                    "static member `{class_name}::{}` is internal",
                    access.member
                ),
                access.span,
            ));
        }
    }

    fn check_enum_case_member(&mut self, definition: &EnumDefinition, access: StaticAccess<'_>) {
        let Some(index) = definition.case_by_name.get(access.member).copied() else {
            let candidates = definition
                .cases
                .iter()
                .map(|case| case.name.as_str())
                .collect::<Vec<_>>();
            let mut diagnostic = Diagnostic::new(
                "E0574",
                format!("unknown case `{}::{}`", definition.name, access.member),
                access.member_span,
            )
            .with_title("Unknown Enum Case");
            if let Some(suggestion) =
                crate::arg_binding::unambiguous_name_suggestion(access.member, &candidates)
            {
                diagnostic = diagnostic
                    .with_help(format!("did you mean `{}::{suggestion}`?", definition.name))
                    .with_fix(access.member_span, suggestion);
            }
            self.diagnostics.push(diagnostic);
            return;
        };
        let case = &definition.cases[index];
        if !case.payload.is_empty() {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0583",
                    format!(
                        "payload case `{}::{}` requires an argument list",
                        definition.name, case.name
                    ),
                    access.span,
                )
                .with_title("Payload Case Requires Arguments")
                .with_help(format!(
                    "construct it as `{}::{}(...)`",
                    definition.name, case.name
                )),
            );
            return;
        }
        self.enum_case_values.insert(
            access.span,
            EnumValue {
                enum_id: definition.id,
                case_id: case.id,
            },
        );
    }

    fn check_enum_case_call(
        &mut self,
        definition: &EnumDefinition,
        access: StaticAccess<'_>,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(index) = definition.case_by_name.get(access.member).copied() else {
            let candidates = definition
                .cases
                .iter()
                .map(|case| case.name.as_str())
                .collect::<Vec<_>>();
            let mut diagnostic = Diagnostic::new(
                "E0574",
                format!("unknown case `{}::{}`", definition.name, access.member),
                access.member_span,
            )
            .with_title("Unknown Enum Case");
            if let Some(suggestion) =
                crate::arg_binding::unambiguous_name_suggestion(access.member, &candidates)
            {
                diagnostic = diagnostic
                    .with_help(format!("did you mean `{}::{suggestion}`?", definition.name))
                    .with_fix(access.member_span, suggestion);
            }
            self.diagnostics.push(diagnostic);
            return;
        };
        let case = &definition.cases[index];
        if !case.payload.is_empty() {
            let params = case
                .payload
                .iter()
                .map(|field| ParamInfo {
                    name: field.name.clone(),
                    ty: field.ty,
                    take: self.type_is_move_type(field.ty),
                    writable: false,
                    has_default: false,
                })
                .collect::<Vec<_>>();
            self.check_call_arguments(
                &format!("payload case `{}::{}`", definition.name, case.name),
                &params,
                args,
                access.span,
                scopes,
                method_context,
            );
            self.enum_case_constructions.insert(access.span, case.id);
            return;
        }
        let parentheses = Span::new(access.member_span.end, access.span.end);
        self.diagnostics.push(
            Diagnostic::new(
                "E0575",
                format!(
                    "unit enum case `{}::{}` has no payload and is not called",
                    definition.name, case.name
                ),
                access.span,
            )
            .with_title("Unit Case Has No Payload")
            .with_help("remove the empty parentheses")
            .with_fix(parentheses, ""),
        );
    }

    fn check_static_assignment_target(
        &mut self,
        access: StaticAccess<'_>,
        method_context: Option<&MethodContext>,
    ) -> Option<AssignmentTarget> {
        let class_name = self.resolve_static_qualifier(access, method_context)?;
        self.check_resolved_static_member(&class_name, access.member, access.span, method_context);
        let class_info = self.classes.get(&class_name)?;
        if class_info.constants.contains_key(access.member) {
            self.diagnostics.push(Diagnostic::new(
                "E0489",
                format!(
                    "cannot assign to constant `{class_name}::{}`",
                    access.member
                ),
                access.span,
            ));
            return None;
        }
        let property = class_info.static_properties.get(access.member)?.clone();
        if !property.writable {
            self.diagnostics.push(Diagnostic::new(
                "E0202",
                format!(
                    "cannot assign to readonly static property `{class_name}::{}`",
                    access.member
                ),
                access.span,
            ));
        }
        Some(AssignmentTarget {
            ty: property.ty,
            destination: AssignmentDestination::Type,
        })
    }

    fn resolve_static_qualifier(
        &mut self,
        access: StaticAccess<'_>,
        method_context: Option<&MethodContext>,
    ) -> Option<String> {
        if let Some(sigil_span) = access.member_sigil_span {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0494",
                    "Doria static member access is sigil-free; remove `$`",
                    sigil_span,
                )
                .with_help("declarations carry `$`; member accesses do not")
                .with_fix(sigil_span, ""),
            );
            return None;
        }

        match access.qualifier {
            StaticQualifier::Class(name)
                if self
                    .classes
                    .get(name)
                    .is_some_and(|class| !class.type_params.is_empty()) =>
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0540",
                        format!(
                            "static access through generic class `{name}` does not identify a concrete specialization"
                        ),
                        access.qualifier_span,
                    )
                    .with_help(format!(
                        "use `self::{}` inside `{name}<...>`, or move the operation to a free generic function",
                        access.member
                    )),
                );
                None
            }
            StaticQualifier::Class(name) => Some(name.clone()),
            StaticQualifier::SelfType => method_context
                .map(|context| context.class_name.clone())
                .or_else(|| {
                    self.diagnostics.push(Diagnostic::new(
                        "E0492",
                        "`self` is only available in a declaring or composing class context",
                        access.qualifier_span,
                    ));
                    None
                }),
            StaticQualifier::Parent => {
                self.diagnostics.push(Diagnostic::unsupported_stage(
                    "E0496",
                    "generalized `parent::member()` syntax is accepted; parent implementation semantics land in Stage 34",
                    access.span,
                ));
                None
            }
            StaticQualifier::InvalidStatic => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0495",
                        "Doria does not support late static binding; use `self::`",
                        access.qualifier_span,
                    )
                    .with_help(
                        "replace the qualifier with `self` and keep the member access unchanged",
                    )
                    .with_fix(access.qualifier_span, "self"),
                );
                None
            }
        }
    }

    fn check_resolved_static_member(
        &mut self,
        class_name: &str,
        member: &str,
        span: Span,
        method_context: Option<&MethodContext>,
    ) {
        let Some(class_info) = self.classes.get(class_name) else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                span,
            ));
            return;
        };
        let access = class_info
            .constants
            .get(member)
            .map(|constant| constant.access)
            .or_else(|| {
                class_info
                    .static_properties
                    .get(member)
                    .map(|property| property.access)
            });
        let Some(access) = access else {
            self.diagnostics.push(Diagnostic::new(
                "E0488",
                format!("unknown static member `{class_name}::{member}`"),
                span,
            ));
            return;
        };
        if access == MemberAccess::Internal
            && !self.can_access_internal_member(class_name, span, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("static member `{class_name}::{member}` is internal"),
                span,
            ));
        }
    }

    fn check_cross_kind_intrinsic_argument(
        &mut self,
        name: &str,
        args: &[Argument],
        expected_kind: TypeKind,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if self.reject_named_arguments(name, args) {
            return;
        }
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                "E0443",
                format!("{name} expects exactly 1 argument, got {}", args.len()),
                span,
            ));
            return;
        }
        let expected = self.types.intern(expected_kind);
        let actual = self.infer_expr_type(&args[0].value, scopes, method_context);
        if actual != expected && !self.is_unknown_type(actual) {
            self.diagnostics.push(Diagnostic::new(
                "E0443",
                format!(
                    "{name} requires a `{}` argument, got `{}`",
                    self.types.display(expected),
                    self.types.display(actual)
                ),
                args[0].value.span(),
            ));
        }
    }

    fn check_parse_intrinsic_argument(
        &mut self,
        name: &str,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if self.reject_named_arguments(name, args) {
            return;
        }
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                "E0443",
                format!("{name} expects exactly 1 argument, got {}", args.len()),
                span,
            ));
            return;
        }
        let actual = self.infer_expr_type(&args[0].value, scopes, method_context);
        if !matches!(
            self.types.kind(actual),
            TypeKind::String | TypeKind::Unknown
        ) {
            self.diagnostics.push(Diagnostic::new(
                "E0443",
                format!(
                    "{name} requires a `string` argument, got `{}`",
                    self.types.display(actual)
                ),
                args[0].value.span(),
            ));
        }
    }

    fn check_direct_lifecycle_method_call(
        &mut self,
        class_name: &str,
        method: &str,
        span: Span,
    ) -> bool {
        let Some(lifecycle) = LifecycleMethod::from_method_name(method) else {
            return false;
        };

        let help = match lifecycle {
            LifecycleMethod::Constructor => {
                format!("construct `{class_name}` with `new {class_name}(...)`")
            }
            LifecycleMethod::Destructor => {
                "destructors are invoked by the runtime, not user code".to_string()
            }
        };

        self.diagnostics.push(
            Diagnostic::new(
                "E0414",
                format!(
                    "{} `{class_name}::{method}` cannot be called directly",
                    lifecycle.label()
                ),
                span,
            )
            .with_help(help),
        );
        true
    }

    fn check_constructor_call(
        &mut self,
        class_type: &ClassType<TypeId>,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let class_name = class_type.name.as_str();
        let Some(class_info) = self.classes.get(class_name).cloned() else {
            return;
        };

        let Some(constructor) = class_info.methods.get("__construct").cloned() else {
            if !args.is_empty() {
                self.report_argument_count_mismatch(
                    &format!("constructor `{class_name}::__construct`"),
                    0,
                    0,
                    args.len(),
                    span,
                );
            }
            if args.is_empty() {
                let effects = self
                    .class_initializer_effects
                    .get(class_name)
                    .map(|effects| effects.ordered.clone())
                    .unwrap_or_default();
                self.record_checked_effects(effects, span);
            }
            return;
        };

        if matches!(constructor.access, MemberAccess::Internal)
            && !self.can_access_internal_member(class_name, span, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0307",
                format!("method `{class_name}::__construct` is internal"),
                span,
            ));
        }

        let class_type_id = self.types.intern(TypeKind::Class(class_type.clone()));
        let ResolvedType::Class(resolved_class_type) = self.types.resolved(class_type_id) else {
            unreachable!("interned constructor class type must resolve as a class");
        };
        self.call_targets.insert(
            span,
            CallableTarget::Method {
                class_type: resolved_class_type,
                method_name: "__construct".to_string(),
            },
        );
        self.record_callable_dependency(constructor.declaration);
        let constructor = self.specialize_method_for_class(&constructor, class_type);
        let diagnostics_before = self.diagnostics.len();
        self.check_call_arguments(
            &format!("constructor `{class_name}::__construct`"),
            &constructor.params,
            args,
            span,
            scopes,
            method_context,
        );
        if self.diagnostics.len() == diagnostics_before {
            self.record_checked_effects(constructor.checked_effects, span);
        }
    }

    fn check_call_arguments(
        &mut self,
        callee: &str,
        params: &[ParamInfo],
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let required = params.iter().filter(|param| !param.has_default).count();
        let total = params.len();

        let param_names: Vec<&str> = params.iter().map(|param| param.name.as_str()).collect();
        let param_has_default: Vec<bool> = params.iter().map(|param| param.has_default).collect();
        let arg_names: Vec<Option<&str>> = args
            .iter()
            .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
            .collect();

        // Positional-only calls keep the exact existing arity and positional
        // type-checking behavior; named binding (decision 0098) only engages once
        // a name appears in the call.
        if !crate::arg_binding::BoundArguments::has_named(&arg_names) {
            if args.len() < required || args.len() > total {
                self.report_argument_count_mismatch(callee, required, total, args.len(), span);
                return;
            }
            for (index, (arg, param)) in args.iter().zip(params.iter()).enumerate() {
                self.check_bound_argument_type(
                    callee,
                    param,
                    &arg.value,
                    index,
                    scopes,
                    method_context,
                );
            }
            return;
        }

        let bound =
            crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names);

        let mut fatal = false;
        if bound.overflow > 0 {
            self.report_argument_count_mismatch(callee, required, total, args.len(), span);
            fatal = true;
        }
        for &arg_index in &bound.unknown {
            let name = args[arg_index]
                .name
                .as_ref()
                .expect("an unknown-named argument always carries a name");
            let mut diagnostic = Diagnostic::new(
                "E0516",
                format!("{callee} has no parameter named `{}`", name.text),
                name.span,
            )
            .with_title("Unknown Named Argument")
            .with_primary_label("No Parameter Has This Name")
            .with_explanation(
                "Named arguments must match a parameter name in the called declaration.",
            )
            .with_help(format!(
                "Available parameter names: {}.",
                param_names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            if let Some(suggestion) =
                crate::arg_binding::unambiguous_name_suggestion(&name.text, &param_names)
            {
                diagnostic = diagnostic.with_help(format!("Did you mean `{suggestion}`?"));
                let suggestion_is_unbound = param_names
                    .iter()
                    .position(|candidate| *candidate == suggestion)
                    .is_some_and(|index| bound.param_to_arg[index].is_none());
                if suggestion_is_unbound {
                    diagnostic = diagnostic.with_fix(name.span, suggestion);
                }
            }
            self.diagnostics.push(diagnostic);
            fatal = true;
        }
        for &arg_index in &bound.duplicate {
            let arg = &args[arg_index];
            let (report_span, message) = match &arg.name {
                Some(name) => (
                    name.span,
                    format!("argument `{}` of {callee} was already supplied", name.text),
                ),
                None => (
                    arg.span,
                    format!("this argument of {callee} was already supplied"),
                ),
            };
            self.diagnostics
                .push(Diagnostic::new("E0517", message, report_span).with_help(
                    "each parameter may be supplied once, positionally or by name, not both",
                ));
            fatal = true;
        }
        if !bound.missing.is_empty() {
            let names = bound
                .missing
                .iter()
                .map(|&param_index| format!("`{}`", param_names[param_index]))
                .collect::<Vec<_>>()
                .join(", ");
            let word = if bound.missing.len() == 1 {
                "argument"
            } else {
                "arguments"
            };
            self.diagnostics.push(Diagnostic::new(
                "E0518",
                format!("{callee} is missing required {word} {names}"),
                span,
            ));
            fatal = true;
        }
        if fatal {
            return;
        }

        // Binding resolved cleanly: type-check each parameter against the
        // argument bound to it, in parameter order. Generic inference (Stage 24)
        // will consume this same parameter->argument assignment unchanged.
        for (param_index, param) in params.iter().enumerate() {
            let Some(arg_index) = bound.param_to_arg[param_index] else {
                continue;
            };
            self.check_bound_argument_type(
                callee,
                param,
                &args[arg_index].value,
                param_index,
                scopes,
                method_context,
            );
        }
    }

    fn instantiate_generic_call(
        &mut self,
        callee: &str,
        function: &FunctionInfo,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> FunctionInfo {
        if function.type_params.is_empty() {
            return function.clone();
        }
        let bindings = self.infer_generic_call_bindings(
            callee,
            &function.type_params,
            &function.params,
            args,
            span,
            scopes,
            method_context,
            function.return_ty,
            function.declaration,
            &HashMap::new(),
        );
        let specialized = FunctionInfo {
            declaration: function.declaration,
            type_params: function.type_params.clone(),
            params: function
                .params
                .iter()
                .map(|param| self.substitute_param_info(param, &bindings))
                .collect(),
            return_ty: self.substitute_type(function.return_ty, &bindings),
            return_borrow: function.return_borrow,
            checked_effects: function
                .checked_effects
                .iter()
                .map(|effect| self.substitute_type(*effect, &bindings))
                .collect(),
        };
        for param in &specialized.params {
            self.check_specialized_shared_payloads(param.ty, span);
        }
        self.check_specialized_shared_payloads(specialized.return_ty, span);
        specialized
    }

    fn instantiate_generic_method_call(
        &mut self,
        callee: &str,
        method: &MethodInfo,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> MethodInfo {
        if method.type_params.is_empty() {
            return method.clone();
        }
        let bindings = self.infer_generic_call_bindings(
            callee,
            &method.type_params,
            &method.params,
            args,
            span,
            scopes,
            method_context,
            method.return_ty,
            method.declaration,
            &method.enclosing_type_bindings,
        );
        let specialized = MethodInfo {
            declaration: method.declaration,
            access: method.access,
            receiver_mode: method.receiver_mode,
            return_borrow: method.return_borrow,
            is_static: method.is_static,
            enclosing_type_bindings: method.enclosing_type_bindings.clone(),
            type_params: method.type_params.clone(),
            params: method
                .params
                .iter()
                .map(|param| self.substitute_param_info(param, &bindings))
                .collect(),
            return_ty: self.substitute_type(method.return_ty, &bindings),
            checked_effects: method
                .checked_effects
                .iter()
                .map(|effect| self.substitute_type(*effect, &bindings))
                .collect(),
        };
        for param in &specialized.params {
            self.check_specialized_shared_payloads(param.ty, span);
        }
        self.check_specialized_shared_payloads(specialized.return_ty, span);
        specialized
    }

    #[allow(clippy::too_many_arguments)]
    fn infer_generic_call_bindings(
        &mut self,
        callee: &str,
        type_params: &[TypeParamInfo],
        params: &[ParamInfo],
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        return_ty: TypeId,
        declaration: Span,
        initial_bindings: &HashMap<String, TypeId>,
    ) -> HashMap<String, TypeId> {
        let param_names = params
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>();
        let param_has_default = params
            .iter()
            .map(|param| param.has_default)
            .collect::<Vec<_>>();
        let arg_names = args
            .iter()
            .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
            .collect::<Vec<_>>();
        let bound =
            crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names);
        let mut bindings = initial_bindings.clone();
        for (param_index, param) in params.iter().enumerate() {
            let Some(arg_index) = bound.param_to_arg[param_index] else {
                continue;
            };
            let actual = self.infer_expr_type(&args[arg_index].value, scopes, method_context);
            self.infer_type_parameter_bindings(
                callee,
                param.ty,
                actual,
                args[arg_index].value.span(),
                &mut bindings,
            );
        }

        let key = span;
        self.pending_generic_calls.insert(
            key,
            PendingGenericCall {
                callee: callee.to_string(),
                declaration,
                type_params: type_params.to_vec(),
                bindings: bindings.clone(),
                return_ty,
            },
        );
        self.publish_generic_specialization(key);
        bindings
    }

    fn infer_type_parameter_bindings(
        &mut self,
        callee: &str,
        pattern: TypeId,
        actual: TypeId,
        span: Span,
        bindings: &mut HashMap<String, TypeId>,
    ) {
        match (
            self.types.kind(pattern).clone(),
            self.types.kind(actual).clone(),
        ) {
            (TypeKind::TypeParameter(name), TypeKind::Null) => {
                let message = format!(
                    "cannot infer type parameter `{name}` of {callee} from `null`; `null` requires a nullable expected type"
                );
                if !self.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "E0538"
                        && diagnostic.span == span
                        && diagnostic.message == message
                }) {
                    self.diagnostics
                        .push(Diagnostic::new("E0538", message, span).with_help(
                            "pass a value with a concrete type or use a nullable parameter shape",
                        ));
                }
                let unknown = self.types.unknown();
                bindings.entry(name).or_insert(unknown);
            }
            (TypeKind::TypeParameter(name), TypeKind::Void) => {
                let message = format!(
                    "cannot infer type parameter `{name}` of {callee} from a `void` expression"
                );
                if !self.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "E0538"
                        && diagnostic.span == span
                        && diagnostic.message == message
                }) {
                    self.diagnostics.push(
                        Diagnostic::new("E0538", message, span)
                            .with_help("pass an expression that produces a value"),
                    );
                }
                let unknown = self.types.unknown();
                bindings.entry(name).or_insert(unknown);
            }
            (TypeKind::TypeParameter(name), TypeKind::Unknown | TypeKind::EmptyCollection) => {
                let _ = name;
            }
            (TypeKind::TypeParameter(name), _) => {
                if let Some(previous) = bindings.get(&name).copied() {
                    if previous != actual {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0532",
                                format!(
                                    "type parameter `{name}` of {callee} was inferred as both `{}` and `{}`",
                                    self.types.display(previous),
                                    self.types.display(actual)
                                ),
                                span,
                            )
                            .with_help(
                                "pass arguments whose corresponding generic types are identical",
                            ),
                        );
                    }
                } else {
                    bindings.insert(name, actual);
                }
            }
            (TypeKind::Nullable(pattern), TypeKind::Nullable(actual)) => {
                self.infer_type_parameter_bindings(callee, pattern, actual, span, bindings);
            }
            (TypeKind::Nullable(pattern), TypeKind::Null) => {
                let _ = pattern;
            }
            (TypeKind::Nullable(pattern), _) => {
                self.infer_type_parameter_bindings(callee, pattern, actual, span, bindings);
            }
            (TypeKind::TypedArray(pattern), TypeKind::TypedArray(actual))
            | (TypeKind::List(pattern), TypeKind::List(actual))
            | (TypeKind::Set(pattern), TypeKind::Set(actual))
            | (TypeKind::SortedSet(pattern), TypeKind::SortedSet(actual))
            | (TypeKind::PriorityQueue(pattern), TypeKind::PriorityQueue(actual))
            | (TypeKind::Deque(pattern), TypeKind::Deque(actual)) => {
                self.infer_type_parameter_bindings(callee, pattern, actual, span, bindings);
            }
            // A shared handle unifies with the same handle kind, through its payload:
            // `SharedReference<T>` against `SharedReference<Node>` binds `T = Node`.
            // The kinds must match, so unification never crosses the family boundary.
            (
                TypeKind::SharedHandle(pattern_kind, pattern),
                TypeKind::SharedHandle(actual_kind, actual),
            ) if pattern_kind == actual_kind => {
                self.infer_type_parameter_bindings(callee, pattern, actual, span, bindings);
            }
            (
                TypeKind::Dictionary(pattern_key, pattern_value),
                TypeKind::Dictionary(actual_key, actual_value),
            )
            | (
                TypeKind::SortedDictionary(pattern_key, pattern_value),
                TypeKind::SortedDictionary(actual_key, actual_value),
            ) => {
                self.infer_type_parameter_bindings(callee, pattern_key, actual_key, span, bindings);
                self.infer_type_parameter_bindings(
                    callee,
                    pattern_value,
                    actual_value,
                    span,
                    bindings,
                );
            }
            (TypeKind::Class(pattern), TypeKind::Class(actual))
                if pattern.name == actual.name
                    && pattern.arguments.len() == actual.arguments.len() =>
            {
                for (pattern, actual) in pattern.arguments.into_iter().zip(actual.arguments) {
                    self.infer_type_parameter_bindings(callee, pattern, actual, span, bindings);
                }
            }
            (TypeKind::Function(pattern), TypeKind::Function(actual))
                if pattern.invocation_mode == actual.invocation_mode
                    && pattern.parameters.len() == actual.parameters.len()
                    && pattern.parameters.iter().zip(&actual.parameters).all(
                        |(pattern, actual)| pattern.ownership_mode == actual.ownership_mode,
                    )
                    && pattern.checked_effects.len() == actual.checked_effects.len() =>
            {
                for (pattern, actual) in pattern.parameters.into_iter().zip(actual.parameters) {
                    self.infer_type_parameter_bindings(
                        callee, pattern.ty, actual.ty, span, bindings,
                    );
                }
                self.infer_type_parameter_bindings(
                    callee,
                    pattern.return_type,
                    actual.return_type,
                    span,
                    bindings,
                );
                for (pattern, actual) in pattern
                    .checked_effects
                    .into_iter()
                    .zip(actual.checked_effects)
                {
                    self.infer_type_parameter_bindings(callee, pattern, actual, span, bindings);
                }
            }
            _ => {}
        }
    }

    fn substitute_param_info(
        &mut self,
        param: &ParamInfo,
        bindings: &HashMap<String, TypeId>,
    ) -> ParamInfo {
        ParamInfo {
            name: param.name.clone(),
            ty: self.substitute_type(param.ty, bindings),
            take: param.take,
            writable: param.writable,
            has_default: param.has_default,
        }
    }

    fn substitute_type(&mut self, ty: TypeId, bindings: &HashMap<String, TypeId>) -> TypeId {
        match self.types.kind(ty).clone() {
            TypeKind::TypeParameter(name) => bindings.get(&name).copied().unwrap_or(ty),
            TypeKind::Nullable(inner) => {
                let inner = self.substitute_type(inner, bindings);
                self.types.intern(TypeKind::Nullable(inner))
            }
            TypeKind::TypedArray(element) => {
                let element = self.substitute_type(element, bindings);
                self.types.intern(TypeKind::TypedArray(element))
            }
            TypeKind::List(element) => {
                let element = self.substitute_type(element, bindings);
                self.types.intern(TypeKind::List(element))
            }
            TypeKind::Dictionary(key, value) => {
                let key = self.substitute_type(key, bindings);
                let value = self.substitute_type(value, bindings);
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            TypeKind::SortedDictionary(key, value) => {
                let key = self.substitute_type(key, bindings);
                let value = self.substitute_type(value, bindings);
                self.types.intern(TypeKind::SortedDictionary(key, value))
            }
            TypeKind::Set(element) => {
                let element = self.substitute_type(element, bindings);
                self.types.intern(TypeKind::Set(element))
            }
            TypeKind::SortedSet(element) => {
                let element = self.substitute_type(element, bindings);
                self.types.intern(TypeKind::SortedSet(element))
            }
            TypeKind::PriorityQueue(element) => {
                let element = self.substitute_type(element, bindings);
                self.types.intern(TypeKind::PriorityQueue(element))
            }
            TypeKind::Deque(element) => {
                let element = self.substitute_type(element, bindings);
                self.types.intern(TypeKind::Deque(element))
            }
            TypeKind::SharedHandle(kind, payload) => {
                let payload = self.substitute_type(payload, bindings);
                self.types.intern(TypeKind::SharedHandle(kind, payload))
            }
            TypeKind::Function(function) => {
                let function = SemanticFunctionType {
                    invocation_mode: function.invocation_mode,
                    parameters: function
                        .parameters
                        .into_iter()
                        .map(|parameter| SemanticFunctionParameter {
                            ownership_mode: parameter.ownership_mode,
                            ty: self.substitute_type(parameter.ty, bindings),
                        })
                        .collect(),
                    return_type: self.substitute_type(function.return_type, bindings),
                    checked_effects: function
                        .checked_effects
                        .into_iter()
                        .map(|effect| self.substitute_type(effect, bindings))
                        .collect(),
                    return_borrow: function.return_borrow,
                };
                self.types.intern(TypeKind::Function(function))
            }
            TypeKind::Class(class) => {
                let arguments = class
                    .arguments
                    .into_iter()
                    .map(|argument| self.substitute_type(argument, bindings))
                    .collect::<Vec<_>>();
                let class = ClassType::new(class.name, arguments);
                if !class
                    .arguments
                    .iter()
                    .any(|argument| self.type_is_symbolic(*argument))
                {
                    self.class_instantiations.insert(class.clone());
                }
                self.types.intern(TypeKind::Class(class))
            }
            _ => ty,
        }
    }

    fn publish_generic_specialization(&mut self, key: Span) {
        let Some(pending) = self.pending_generic_calls.get(&key) else {
            return;
        };
        let Some(arguments) = pending
            .type_params
            .iter()
            .map(|param| {
                pending
                    .bindings
                    .get(&param.name)
                    .map(|ty| GenericArgument::Type(self.types.resolved(*ty)))
            })
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        let callee = pending.callee.clone();
        let type_params = pending.type_params.clone();
        let bindings = pending.bindings.clone();
        self.check_generic_type_constraints(&callee, &type_params, &bindings, key);
        self.generic_call_specializations
            .insert(key, GenericSpecialization { arguments });
    }

    fn complete_generic_call_from_expected(&mut self, expr: &Expr, expected: TypeId) {
        if let Expr::Grouped { expr, .. } = expr {
            self.complete_generic_call_from_expected(expr, expected);
            return;
        }
        let key = match expr {
            Expr::FunctionCall { span, .. }
            | Expr::MethodCall { span, .. }
            | Expr::StaticCall { span, .. } => span,
            _ => return,
        };
        let Some(pending) = self.pending_generic_calls.get(key).cloned() else {
            return;
        };
        let mut bindings = pending.bindings;
        self.infer_type_parameter_bindings(
            &pending.callee,
            pending.return_ty,
            expected,
            expr.span(),
            &mut bindings,
        );
        if let Some(call) = self.pending_generic_calls.get_mut(key) {
            call.bindings = bindings;
        }
        self.publish_generic_specialization(*key);
    }

    fn generic_call_result_type(&mut self, span: Span, fallback: TypeId) -> TypeId {
        let key = span;
        let Some(pending) = self.pending_generic_calls.get(&key).cloned() else {
            return fallback;
        };
        if pending
            .type_params
            .iter()
            .all(|param| pending.bindings.contains_key(&param.name))
        {
            return self.substitute_type(pending.return_ty, &pending.bindings);
        }
        fallback
    }

    fn report_unresolved_generic_calls(&mut self) {
        let mut calls = self.pending_generic_calls.iter().collect::<Vec<_>>();
        calls.sort_by_key(|(span, _)| **span);
        for (span, pending) in calls {
            let missing_names = pending
                .type_params
                .iter()
                .filter(|param| !pending.bindings.contains_key(&param.name))
                .collect::<Vec<_>>();
            if missing_names.is_empty() {
                continue;
            }
            let missing = missing_names
                .iter()
                .map(|param| format!("`{}`", param.name))
                .collect::<Vec<_>>()
                .join(", ");
            let plural = if missing_names.len() == 1 { "" } else { "s" };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0531",
                    format!(
                        "cannot infer type parameter{plural} {missing} for {}",
                        pending.callee
                    ),
                    *span,
                )
                .with_help(
                    "bind the result through a typed declaration so the expected type determines the missing generic argument",
                ),
            );
        }
    }

    /// Intrinsics and built-in calls bind positionally only; their parameter
    /// names are not public API (decision 0098 makes parameter names public for
    /// user free functions, methods, static methods, and constructors — not for
    /// language intrinsics). Reject any named argument, returning whether one was
    /// found so the caller can stop before positional processing.
    fn reject_named_arguments(&mut self, callee: &str, args: &[Argument]) -> bool {
        let mut rejected = false;
        for arg in args {
            if let Some(name) = &arg.name {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0519",
                        format!("{callee} does not accept named arguments"),
                        name.span,
                    )
                    .with_help("call this intrinsic with positional arguments"),
                );
                rejected = true;
            }
        }
        rejected
    }

    fn check_bound_argument_type(
        &mut self,
        callee: &str,
        param: &ParamInfo,
        arg: &Expr,
        index: usize,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let got = self.infer_expr_type(arg, scopes, method_context);
        let parameter_is_class_like = self.type_is_class_or_nullable_class(param.ty);

        if self.is_expr_assignable(param.ty, arg, scopes, method_context)
            || self.is_assignable(param.ty, got)
        {
            if param.take {
                self.record_capture_requirement_for_expr(arg, scopes, CaptureRequirement::Take);
            } else if param.writable {
                self.record_capture_requirement_for_expr(arg, scopes, CaptureRequirement::Writable);
            }
            if param.writable
                && (parameter_is_class_like
                    || matches!(self.types.kind(param.ty), TypeKind::Mixed)
                        && self.writable_mixed_requires_semantic_check(got, arg, method_context))
                && !self.is_writable_object_path(arg, scopes, method_context)
            {
                let message = if parameter_is_class_like {
                    format!(
                        "argument {} of {callee} must be a writable class value",
                        index + 1
                    )
                } else {
                    format!(
                        "argument {} of {callee} must reference writable storage",
                        index + 1
                    )
                };
                self.diagnostics.push(
                    Diagnostic::new("E0204", message, arg.span()).with_help(
                        "pass a `writable` binding or property that the callee can mutate",
                    ),
                );
            }
            return;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0408",
            format!(
                "argument {} of {callee} expects `{}`, got `{}`",
                index + 1,
                self.types.display(param.ty),
                self.types.display(got)
            ),
            arg.span(),
        ));
    }

    fn record_mixed_boundary(&mut self, target: TypeId, value_expr: &Expr, value: TypeId) {
        let nullable_target = matches!(
            self.types.kind(target),
            TypeKind::Nullable(inner) if matches!(self.types.kind(*inner), TypeKind::Mixed)
        );
        if (matches!(self.types.kind(target), TypeKind::Mixed) || nullable_target)
            && !matches!(self.types.kind(value), TypeKind::Mixed | TypeKind::Unknown)
            && !(nullable_target && matches!(self.types.kind(value), TypeKind::Null))
        {
            self.mixed_box_plans.insert(
                value_expr.span(),
                MixedBoxPlan {
                    source_type: self.types.resolved(value),
                    nullable_target,
                },
            );
        }
    }

    fn type_is_class_or_nullable_class(&self, ty: TypeId) -> bool {
        match *self.types.kind(ty) {
            TypeKind::Class(_) => true,
            TypeKind::Nullable(inner) => matches!(self.types.kind(inner), TypeKind::Class(_)),
            _ => false,
        }
    }

    fn report_argument_count_mismatch(
        &mut self,
        callee: &str,
        required: usize,
        total: usize,
        got: usize,
        span: Span,
    ) {
        let expectation = if required == total {
            format!("{} {}", required, Self::argument_word(required))
        } else {
            format!("between {} and {} arguments", required, total)
        };

        self.diagnostics.push(Diagnostic::new(
            "E0409",
            format!("{callee} expects {expectation}, got {got}"),
            span,
        ));
    }

    fn argument_word(count: usize) -> &'static str {
        if count == 1 {
            "argument"
        } else {
            "arguments"
        }
    }

    fn is_writable_object_path(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let writable =
            self.object_path_access(expr, scopes, method_context) == ObjectPathAccess::Writable;
        if writable {
            self.writable_object_paths.insert(expr.span());
        }
        writable
    }

    fn object_path_access(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> ObjectPathAccess {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        if let TypeKind::SharedHandle(kind, _) = self.types.kind(ty) {
            return if *kind == SharedHandleKind::WritableSharedReferenceAccess {
                ObjectPathAccess::Writable
            } else {
                ObjectPathAccess::Readonly
            };
        }
        let access = match expr {
            Expr::Grouped { expr, .. } => self.object_path_access(expr, scopes, method_context),
            Expr::New { .. } => ObjectPathAccess::Writable,
            Expr::Variable { name, .. } => {
                scopes
                    .lookup(name)
                    .map_or(ObjectPathAccess::Readonly, |binding| {
                        if binding.writable {
                            ObjectPathAccess::Writable
                        } else {
                            ObjectPathAccess::Readonly
                        }
                    })
            }
            Expr::This { .. } => match method_context.map(|context| context.receiver_access) {
                Some(ReceiverAccess::Writable) => ObjectPathAccess::Writable,
                Some(ReceiverAccess::ConstructionRoot) => ObjectPathAccess::ConstructionRoot,
                _ => ObjectPathAccess::Readonly,
            },
            Expr::PropertyAccess {
                null_safe: true, ..
            } => ObjectPathAccess::Readonly,
            Expr::PropertyAccess {
                object, property, ..
            } => {
                if !Self::is_property_write_object_path(object) {
                    return ObjectPathAccess::Readonly;
                }
                let parent_access = self.object_path_access(object, scopes, method_context);
                if parent_access == ObjectPathAccess::Readonly {
                    return ObjectPathAccess::Readonly;
                }
                let Some(class_name) = self.expr_class_name(object, scopes, method_context) else {
                    return ObjectPathAccess::Readonly;
                };
                if self
                    .classes
                    .get(&class_name)
                    .and_then(|class_info| class_info.properties.get(property))
                    .map(|property| property.writable)
                    .unwrap_or(false)
                {
                    ObjectPathAccess::Writable
                } else {
                    ObjectPathAccess::Readonly
                }
            }
            Expr::Index { collection, .. } => {
                self.object_path_access(collection, scopes, method_context)
            }
            Expr::FunctionCall {
                name, args, span, ..
            } => {
                if self.functions.get(name).cloned().is_some_and(|function| {
                    let return_ty = self.generic_call_result_type(*span, function.return_ty);
                    self.call_result_is_writable(
                        CallSite {
                            return_ty,
                            return_borrow: function.return_borrow,
                            params: &function.params,
                            args,
                        },
                        None,
                        scopes,
                        method_context,
                    )
                }) {
                    ObjectPathAccess::Writable
                } else {
                    ObjectPathAccess::Readonly
                }
            }
            Expr::MethodCall {
                object,
                method,
                args,
                span,
                ..
            } => {
                let Some(class_type) = self.expr_class_type(object, scopes, method_context) else {
                    return ObjectPathAccess::Readonly;
                };
                let Some(method_info) = self
                    .classes
                    .get(&class_type.name)
                    .and_then(|class_info| class_info.methods.get(method))
                    .cloned()
                else {
                    return ObjectPathAccess::Readonly;
                };
                let method_info = self.specialize_method_for_class(&method_info, &class_type);
                let return_ty = self.generic_call_result_type(*span, method_info.return_ty);
                if self.call_result_is_writable(
                    CallSite {
                        return_ty,
                        return_borrow: method_info.return_borrow,
                        params: &method_info.params,
                        args,
                    },
                    Some(object),
                    scopes,
                    method_context,
                ) {
                    ObjectPathAccess::Writable
                } else {
                    ObjectPathAccess::Readonly
                }
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                span,
                ..
            } => {
                let class_name = match qualifier {
                    StaticQualifier::Class(name) => Some(name.as_str()),
                    StaticQualifier::SelfType => {
                        method_context.map(|context| context.class_name.as_str())
                    }
                    StaticQualifier::Parent | StaticQualifier::InvalidStatic => None,
                };
                let Some(method_info) = class_name
                    .and_then(|class| self.classes.get(class))
                    .and_then(|class| class.methods.get(method))
                    .cloned()
                else {
                    return ObjectPathAccess::Readonly;
                };
                let return_ty = self.generic_call_result_type(*span, method_info.return_ty);
                if self.call_result_is_writable(
                    CallSite {
                        return_ty,
                        return_borrow: method_info.return_borrow,
                        params: &method_info.params,
                        args,
                    },
                    None,
                    scopes,
                    method_context,
                ) {
                    ObjectPathAccess::Writable
                } else {
                    ObjectPathAccess::Readonly
                }
            }
            _ => ObjectPathAccess::Readonly,
        };
        if access == ObjectPathAccess::Writable {
            self.writable_object_paths.insert(expr.span());
        }
        access
    }

    fn writable_mixed_requires_semantic_check(
        &self,
        argument_ty: TypeId,
        argument: &Expr,
        method_context: Option<&MethodContext>,
    ) -> bool {
        if self.type_is_move_type(argument_ty) {
            return false;
        }
        match argument {
            Expr::Grouped { expr, .. } => {
                self.writable_mixed_requires_semantic_check(argument_ty, expr, method_context)
            }
            Expr::This { .. } | Expr::PropertyAccess { .. } => false,
            Expr::StaticMember {
                qualifier, member, ..
            } => Self::static_qualifier_class_name(qualifier, method_context)
                .and_then(|class_name| self.classes.get(&class_name))
                .is_none_or(|class| !class.static_properties.contains_key(member)),
            _ => true,
        }
    }

    fn is_property_write_object_path(expr: &Expr) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => Self::is_property_write_object_path(expr),
            Expr::Variable { .. } | Expr::This { .. } => true,
            Expr::PropertyAccess { object, .. } => Self::is_property_write_object_path(object),
            _ => false,
        }
    }

    fn call_result_is_writable(
        &mut self,
        callee: CallSite<'_>,
        receiver: Option<&Expr>,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let CallSite {
            return_ty,
            return_borrow,
            params,
            args,
        } = callee;
        if !self.type_is_class_or_nullable_class(return_ty) {
            return false;
        }
        let Some(return_borrow) = return_borrow else {
            return true;
        };
        if !return_borrow.writable {
            return false;
        }
        match return_borrow.source {
            BorrowSource::Receiver => receiver.is_some_and(|receiver| {
                self.is_writable_object_path(receiver, scopes, method_context)
            }),
            BorrowSource::Parameter(index) => {
                // The borrow annotation refers to a parameter position; resolve
                // the argument that binds to it (named binding may reorder or
                // skip), then check that source expression's writability.
                Self::argument_bound_to_parameter(params, args, index).is_some_and(|argument| {
                    self.is_writable_object_path(argument, scopes, method_context)
                })
            }
        }
    }

    /// Resolve the source-order argument expression bound to parameter
    /// `param_index` under named-argument binding (decision 0098). Returns `None`
    /// when the parameter was omitted (its default applies) or the binding could
    /// not resolve it.
    fn argument_bound_to_parameter<'a>(
        params: &[ParamInfo],
        args: &'a [Argument],
        param_index: usize,
    ) -> Option<&'a Expr> {
        let param_names: Vec<&str> = params.iter().map(|param| param.name.as_str()).collect();
        let param_has_default: Vec<bool> = params.iter().map(|param| param.has_default).collect();
        let arg_names: Vec<Option<&str>> = args
            .iter()
            .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
            .collect();
        let bound =
            crate::arg_binding::bind_arguments(&param_names, &param_has_default, &arg_names);
        bound
            .param_to_arg
            .get(param_index)
            .copied()
            .flatten()
            .map(|arg_index| &args[arg_index].value)
    }

    fn is_direct_this(expr: &Expr) -> bool {
        match expr {
            Expr::Grouped { expr, .. } => Self::is_direct_this(expr),
            Expr::This { .. } => true,
            _ => false,
        }
    }

    fn lookup_property(
        &mut self,
        object: &Expr,
        property: &str,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<(String, PropertyInfo)> {
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        if let TypeKind::TypeParameter(parameter) = self.types.kind(object_ty) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0537",
                    format!(
                        "property `{property}` is not guaranteed by the constraints on type parameter `{parameter}`"
                    ),
                    span,
                )
                .with_help("type-parameter bodies may use only members guaranteed by their constraints"),
            );
            return None;
        }
        let class_type = self.expr_class_type(object, scopes, method_context)?;
        let class_name = class_type.name.clone();
        let Some(class_info) = self.classes.get(&class_name) else {
            self.diagnostics.push(Diagnostic::new(
                "E0305",
                format!("unknown class `{class_name}`"),
                span,
            ));
            return None;
        };
        let Some(property_info) = class_info.properties.get(property).cloned() else {
            self.diagnostics.push(Diagnostic::new(
                "E0303",
                format!("unknown property `{class_name}::{property}`"),
                span,
            ));
            return None;
        };

        let property_info = self.specialize_property_for_class(&property_info, &class_type);
        if matches!(property_info.access, MemberAccess::Internal)
            && !self.can_access_internal_member(&class_name, span, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0306",
                format!("property `{class_name}::{property}` is internal"),
                span,
            ));
        }

        Some((class_name, property_info))
    }

    fn can_access_internal_member(
        &self,
        declaring_class: &str,
        use_span: Span,
        method_context: Option<&MethodContext>,
    ) -> bool {
        if method_context.is_some_and(|context| context.class_name == declaring_class) {
            return true;
        }
        let using_package = self
            .compilation_contexts
            .get(&use_span.source)
            .map(|context| &context.package)
            .unwrap_or(&self.compilation_context.package);
        self.global_symbols
            .declarations
            .iter()
            .find(|declaration| declaration.qualified_name == declaring_class)
            .and_then(|declaration| match &declaration.id.owner {
                crate::names::GlobalSymbolOwner::Package(package) => Some(package),
                crate::names::GlobalSymbolOwner::CompilerKnown(_) => None,
            })
            .is_some_and(|declaring_package| declaring_package == using_package)
    }

    fn type_parameter_has_constraint(&self, parameter: &str, required: &str) -> bool {
        self.type_parameter_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(parameter))
            .is_some_and(|constraints| {
                constraints
                    .iter()
                    .any(|constraint| constraint.name == required)
            })
    }

    fn constrained_relational_operands(&mut self, left: TypeId, right: TypeId) -> bool {
        self.constrained_binary_operands(left, right, "Comparable")
    }

    fn constrained_equality_operands(&mut self, left: TypeId, right: TypeId) -> bool {
        self.constrained_binary_operands(left, right, "Equatable")
    }

    fn constrained_binary_operands(
        &mut self,
        left: TypeId,
        right: TypeId,
        constraint_name: &str,
    ) -> bool {
        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        matches!(left_kind, TypeKind::TypeParameter(ref parameter)
        if self.type_parameter_accepts_constraint_operand(
            parameter,
            constraint_name,
            right
        )) || matches!(right_kind, TypeKind::TypeParameter(ref parameter)
        if self.type_parameter_accepts_constraint_operand(
            parameter,
            constraint_name,
            left
        ))
    }

    fn type_parameter_accepts_constraint_operand(
        &mut self,
        parameter: &str,
        constraint_name: &str,
        operand: TypeId,
    ) -> bool {
        let constraints = self
            .type_parameter_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(parameter))
            .into_iter()
            .flatten()
            .filter(|constraint| constraint.name == constraint_name)
            .cloned()
            .collect::<Vec<_>>();

        constraints.into_iter().any(|constraint| {
            if matches!(
                self.types.kind(operand),
                TypeKind::TypeParameter(other) if other == parameter
            ) {
                return true;
            }
            if constraint.arguments.is_empty() {
                return false;
            }
            if constraint.has_value_arguments() || constraint.type_argument_count() != 1 {
                return false;
            }
            constraint
                .type_argument(0)
                .and_then(|argument| self.resolve_constraint_operand_type(argument))
                == Some(operand)
        })
    }

    fn resolve_constraint_operand_type(&mut self, ty: &TypeRef) -> Option<TypeId> {
        if ty.nullable || !ty.arguments.is_empty() {
            return None;
        }
        if self
            .type_parameter_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(&ty.name))
        {
            return Some(self.types.intern(TypeKind::TypeParameter(ty.name.clone())));
        }
        if let Some(integer) = IntegerType::from_source_name(&ty.name) {
            return Some(self.types.intern(TypeKind::Integer(integer)));
        }
        if let Some(float) = FloatType::from_source_name(&ty.name) {
            return Some(self.types.intern(TypeKind::Float(float)));
        }
        match ty.name.as_str() {
            "string" => Some(self.types.intern(TypeKind::String)),
            "bool" => Some(self.types.intern(TypeKind::Bool)),
            _ => None,
        }
    }

    fn resolve_type_ref(&mut self, ty: &TypeRef, span: Span) -> TypeId {
        self.resolve_type_ref_in_position(ty, span, TypePosition::Value, None)
    }

    fn const_type_id(&mut self, ty: crate::const_eval::ConstType) -> TypeId {
        let kind = match ty {
            crate::const_eval::ConstType::Integer(ty) => TypeKind::Integer(ty),
            crate::const_eval::ConstType::NullableInteger(ty) => {
                let inner = self.types.intern(TypeKind::Integer(ty));
                TypeKind::Nullable(inner)
            }
            crate::const_eval::ConstType::Float(ty) => TypeKind::Float(ty),
            crate::const_eval::ConstType::NullableFloat(ty) => {
                let inner = self.types.intern(TypeKind::Float(ty));
                TypeKind::Nullable(inner)
            }
            crate::const_eval::ConstType::String => TypeKind::String,
            crate::const_eval::ConstType::Bool => TypeKind::Bool,
            crate::const_eval::ConstType::NullableBool => {
                let inner = self.types.intern(TypeKind::Bool);
                TypeKind::Nullable(inner)
            }
            crate::const_eval::ConstType::Enum(enum_id) => TypeKind::Enum(
                self.enums
                    .values()
                    .find(|definition| definition.id == enum_id)
                    .map(|definition| crate::enums::EnumType {
                        id: definition.id,
                        name: definition.name.clone(),
                    })
                    .unwrap_or_else(|| crate::enums::EnumType {
                        id: enum_id,
                        name: format!("enum#{}", enum_id.0),
                    }),
            ),
            crate::const_eval::ConstType::NullableEnum(enum_id) => {
                let enum_type = self.const_type_id(crate::const_eval::ConstType::Enum(enum_id));
                TypeKind::Nullable(enum_type)
            }
            crate::const_eval::ConstType::Null => TypeKind::Null,
            crate::const_eval::ConstType::NullableString => {
                let string = self.types.intern(TypeKind::String);
                TypeKind::Nullable(string)
            }
        };
        self.types.intern(kind)
    }

    fn resolve_type_ref_with_class(
        &mut self,
        ty: &TypeRef,
        span: Span,
        declaring_class: Option<&str>,
    ) -> TypeId {
        self.resolve_type_ref_in_position(ty, span, TypePosition::Value, declaring_class)
    }

    fn resolve_type_ref_in_position(
        &mut self,
        ty: &TypeRef,
        span: Span,
        position: TypePosition,
        declaring_class: Option<&str>,
    ) -> TypeId {
        if let Some(grouped) = &ty.grouped {
            let mut inner = grouped.inner.clone();
            inner.nullable |= ty.nullable;
            return self.resolve_type_ref_in_position(&inner, span, position, declaring_class);
        }
        if let Some(function) = &ty.function {
            let function = self.resolve_function_type_ref(function, declaring_class);
            return if ty.nullable {
                self.types.intern(TypeKind::Nullable(function))
            } else {
                function
            };
        }
        if ty.has_value_arguments() {
            for argument in ty.type_arguments() {
                self.resolve_type_ref_in_position(
                    argument,
                    span,
                    TypePosition::Value,
                    declaring_class,
                );
            }
            self.diagnostics.push(Diagnostic::unsupported_stage(
                "E0536",
                format!(
                    "compile-time value arguments in `{ty}` are reserved by decision 0105 but are not available in v1.0"
                ),
                span,
            ));
            return self.types.unknown();
        }
        if ty.nullable {
            let mut inner = ty.clone();
            inner.nullable = false;
            let inner = self.resolve_type_ref_in_position(
                &inner,
                span,
                TypePosition::Value,
                declaring_class,
            );
            return match self.types.kind(inner) {
                TypeKind::Integer(_)
                | TypeKind::Float(_)
                | TypeKind::String
                | TypeKind::Bool
                | TypeKind::Mixed
                | TypeKind::Error
                | TypeKind::Enum(_)
                | TypeKind::TypeParameter(_)
                | TypeKind::Function(_)
                | TypeKind::Class(_)
                | TypeKind::SharedHandle(_, _)
                | TypeKind::Bytes
                | TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_) => self.types.intern(TypeKind::Nullable(inner)),
                TypeKind::Void
                | TypeKind::Null
                | TypeKind::Nullable(_)
                | TypeKind::Unknown
                | TypeKind::Heterogeneous
                | TypeKind::EmptyCollection => self.reject_type_ref_with_help(
                    ty,
                    span,
                    "E0454",
                    format!("`{ty}` is not a valid nullable type"),
                    "write `?T` where `T` is a supported concrete value type",
                ),
            };
        }
        if let Some(integer) = IntegerType::from_source_name(&ty.name) {
            return self.resolve_zero_arg_type(ty, span, TypeKind::Integer(integer));
        }

        if let Some(float) = FloatType::from_source_name(&ty.name) {
            return self.resolve_zero_arg_type(ty, span, TypeKind::Float(float));
        }
        if ty.arguments.is_empty()
            && self
                .type_parameter_scopes
                .iter()
                .rev()
                .any(|params| params.contains_key(&ty.name))
        {
            return self.types.intern(TypeKind::TypeParameter(ty.name.clone()));
        }

        match ty.name.as_str() {
            "self" if ty.arguments.is_empty() => match declaring_class {
                Some(class_name) => self.symbolic_class_type(class_name),
                None => self.reject_type_ref(
                    ty,
                    span,
                    "E0492",
                    "`self` is reserved for the declaring or composing class context",
                ),
            },
            "void" if position == TypePosition::Return => {
                self.resolve_zero_arg_type(ty, span, TypeKind::Void)
            }
            "void" => {
                self.reject_type_ref(ty, span, "E0430", "`void` is only valid as a return type")
            }
            "string" => self.resolve_zero_arg_type(ty, span, TypeKind::String),
            "Bytes" => self.resolve_zero_arg_type(ty, span, TypeKind::Bytes),
            "bool" => self.resolve_zero_arg_type(ty, span, TypeKind::Bool),
            "null" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0431",
                "`null` is a literal, not a type name",
                "write `?T` with a concrete type, such as `?string` or `?Person`",
            ),
            "mixed" => self.resolve_zero_arg_type(ty, span, TypeKind::Mixed),
            "Error" => self.resolve_zero_arg_type(ty, span, TypeKind::Error),
            "object" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0401",
                "`object` does not exist as a Doria type",
                "use a concrete class type, or `mixed` at a dynamic boundary and narrow it with `is`",
            ),
            "array" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0401",
                "unknown type `array`",
                "use typed array suffixes like `T[]` or named collection aliases",
            ),
            // The superseded shared-ownership spellings. Doria's canonical
            // vocabulary is complete words (record 0106); these never shipped as
            // accepted surface and are not retained as aliases.
            "Shared" | "Weak" | "SharedMut" => {
                let replacement = match ty.name.as_str() {
                    "Shared" => "SharedReference",
                    "Weak" => "WeakReference",
                    _ => "WritableSharedReference",
                };
                self.reject_type_ref_with_help(
                    ty,
                    span,
                    "E0547",
                    format!("Unknown Type `{}`", ty.name),
                    format!("Doria spells this `{replacement}<T>`"),
                )
            }
            "resource" => self.reject_type_ref(
                ty,
                span,
                "E0432",
                "`resource` is reserved for PHP interop through the future Phase I bridge and is not a Doria value type",
            ),
            name if self.enums.contains_key(name) => {
                if !self.expect_type_arg_count(ty, 0, span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(
                            arg,
                            span,
                            TypePosition::Value,
                            declaring_class,
                        );
                    }
                    return self.types.unknown();
                }
                let definition = self.enums.get(name).expect("enum existence checked");
                self.types.intern(TypeKind::Enum(EnumType::new(
                    definition.id,
                    definition.name.clone(),
                )))
            }
            "uint" => self.reject_type_ref_with_help(
                ty,
                span,
                "E0401",
                "Doria has no bare `uint`; use an explicit width such as `uint64`",
                "choose `uint8`, `uint16`, `uint32`, or `uint64`",
            ),
            "i8" | "i16" | "i32" | "i64" => {
                let width = &ty.name[1..];
                self.reject_type_ref_with_help(
                    ty,
                    span,
                    "E0401",
                    format!("Doria uses `int{width}`, not `{}`", ty.name),
                    "use the Doria fixed-width integer spelling",
                )
            }
            "u8" | "u16" | "u32" | "u64" => {
                let width = &ty.name[1..];
                self.reject_type_ref_with_help(
                    ty,
                    span,
                    "E0401",
                    format!("Doria uses `uint{width}`, not `{}`", ty.name),
                    "use the Doria fixed-width integer spelling",
                )
            }
            "[]" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(ty.type_argument(0).unwrap(), span, TypePosition::Value, declaring_class);
                self.types.intern(TypeKind::TypedArray(element))
            }
            "List" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(ty.type_argument(0).unwrap(), span, TypePosition::Value, declaring_class);
                self.types.intern(TypeKind::List(element))
            }
            name if SharedHandleKind::from_source_name(name).is_some() => {
                let kind = SharedHandleKind::from_source_name(name).unwrap();
                if ty.type_argument_count() != 1 {
                    self.report_shared_handle_arity(kind, ty.type_argument_count(), span);
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let payload = self.resolve_type_ref_in_position(
                    ty.type_argument(0).unwrap(),
                    span,
                    TypePosition::Value,
                    declaring_class,
                );
                if !self.shared_handle_payload_is_supported(kind, payload) {
                    self.report_shared_handle_payload(kind, payload, span);
                    return self.types.unknown();
                }
                self.types.intern(TypeKind::SharedHandle(kind, payload))
            }
            "Dictionary" => {
                if !self.expect_type_arg_count(ty, 2, span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let key = self.resolve_type_ref_in_position(ty.type_argument(0).unwrap(), span, TypePosition::Value, declaring_class);
                let value =
                    self.resolve_type_ref_in_position(ty.type_argument(1).unwrap(), span, TypePosition::Value, declaring_class);
                self.check_stage23_hashable_type(key, span, "Dictionary key");
                self.types.intern(TypeKind::Dictionary(key, value))
            }
            "SortedDictionary" => {
                if !self.expect_type_arg_count(ty, 2, span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let key = self.resolve_type_ref_in_position(ty.type_argument(0).unwrap(), span, TypePosition::Value, declaring_class);
                let value = self.resolve_type_ref_in_position(ty.type_argument(1).unwrap(), span, TypePosition::Value, declaring_class);
                self.check_stage26_comparable_type(key, span, "SortedDictionary key");
                self.types.intern(TypeKind::SortedDictionary(key, value))
            }
            "Set" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let element =
                    self.resolve_type_ref_in_position(ty.type_argument(0).unwrap(), span, TypePosition::Value, declaring_class);
                self.check_stage23_hashable_type(element, span, "Set element");
                self.types.intern(TypeKind::Set(element))
            }
            "SortedSet" | "PriorityQueue" | "Deque" => {
                if !self.expect_type_arg_count(ty, 1, span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let element = self.resolve_type_ref_in_position(
                    ty.type_argument(0).unwrap(),
                    span,
                    TypePosition::Value,
                    declaring_class,
                );
                match ty.name.as_str() {
                    "SortedSet" => {
                        self.check_stage26_comparable_type(element, span, "SortedSet element");
                        self.types.intern(TypeKind::SortedSet(element))
                    }
                    "PriorityQueue" => {
                        self.check_stage26_comparable_type(element, span, "PriorityQueue element");
                        self.types.intern(TypeKind::PriorityQueue(element))
                    }
                    "Deque" => self.types.intern(TypeKind::Deque(element)),
                    _ => unreachable!(),
                }
            }
            "Queue" | "Stack" | "HashMap" | "HashSet" | "SortedMap" => {
                let replacement = match ty.name.as_str() {
                    "Queue" | "Stack" => "Deque<T>",
                    "HashMap" => "Dictionary<K, V>",
                    "HashSet" => "Set<T>",
                    "SortedMap" => "SortedDictionary<K, V>",
                    _ => unreachable!(),
                };
                self.reject_type_ref_with_help(
                    ty,
                    span,
                    "E0401",
                    format!("Unknown Type `{}`", ty.name),
                    format!("use `{replacement}`"),
                )
            }
            name if self.classes.contains_key(name) => {
                let type_params = self
                    .classes
                    .get(name)
                    .map(|class| class.type_params.clone())
                    .unwrap_or_default();
                if !self.expect_type_arg_count(ty, type_params.len(), span) {
                    for arg in ty.type_arguments() {
                        self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                    }
                    return self.types.unknown();
                }
                let arguments = ty
                    .type_arguments()
                    .map(|argument| {
                        self.resolve_type_ref_in_position(
                            argument,
                            span,
                            TypePosition::Value,
                            declaring_class,
                        )
                    })
                    .collect::<Vec<_>>();
                self.check_class_type_constraints(name, &type_params, &arguments, span);
                let class = ClassType::new(name, arguments);
                if class
                    .arguments
                    .iter()
                    .any(|argument| self.type_is_symbolic(*argument))
                {
                    if let Some(owner) = declaring_class {
                        self.class_instantiation_templates
                            .entry(owner.to_string())
                            .or_default()
                            .insert(class.clone());
                    }
                    if let Some(callable) = self.current_callable {
                        self.callable_class_instantiation_templates
                            .entry(callable)
                            .or_default()
                            .insert(class.clone());
                    }
                } else {
                    self.class_instantiations.insert(class.clone());
                }
                self.types.intern(TypeKind::Class(class))
            }
            name => {
                for arg in ty.type_arguments() {
                    self.resolve_type_ref_in_position(arg, span, TypePosition::Value, declaring_class);
                }
                self.diagnostics.push(Diagnostic::new(
                    "E0401",
                    format!("unknown type `{name}`"),
                    span,
                ));
                self.types.unknown()
            }
        }
    }

    fn symbolic_class_type(&mut self, class_name: &str) -> TypeId {
        let parameter_names = self
            .classes
            .get(class_name)
            .map(|class| {
                class
                    .type_params
                    .iter()
                    .map(|param| param.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let arguments = parameter_names
            .into_iter()
            .map(|name| self.types.intern(TypeKind::TypeParameter(name)))
            .collect();
        self.types
            .intern(TypeKind::Class(ClassType::new(class_name, arguments)))
    }

    fn check_stage23_hashable_type(&mut self, ty: TypeId, span: Span, role: &str) {
        let diagnostic = match self.types.kind(ty) {
            TypeKind::Integer(_)
            | TypeKind::String
            | TypeKind::Bool
            | TypeKind::Unknown => return,
            TypeKind::TypeParameter(parameter)
                if self.type_parameter_has_constraint(parameter, "Hashable") =>
            {
                return;
            }
            TypeKind::TypeParameter(parameter) => Diagnostic::new(
                "E0537",
                format!(
                    "{role} type parameter `{parameter}` is not guaranteed to implement `Hashable`"
                ),
                span,
            )
            .with_help(format!(
                "declare `{parameter} implements Hashable` before using it as a hash key or set element"
            )),
            TypeKind::Class(_) => Diagnostic::unsupported_stage(
                "E0523",
                format!(
                    "{role} type `{}` requires Stage 35 user-defined `Hashable` conformance",
                    self.types.display(ty)
                ),
                span,
            ),
            _ => Diagnostic::new(
                "E0523",
                format!(
                    "{role} type `{}` does not conform to `Hashable`",
                    self.types.display(ty)
                ),
                span,
            )
            .with_help("use an integer, string, or bool key/element; float is not Hashable"),
        };
        if !self.diagnostics.iter().any(|existing| {
            existing.code == diagnostic.code && existing.message == diagnostic.message
        }) {
            self.diagnostics.push(diagnostic);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_stage26_collection_from_call(
        &mut self,
        collection: &str,
        method: &str,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if method != "from" {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0521",
                    format!("Unknown {collection} Companion Operation `{collection}::{method}`"),
                    span,
                )
                .with_title("Unknown Collection Operation")
                .with_help(format!(
                    "construct this collection with `{collection}::from(...)`"
                )),
            );
            return;
        }
        if self.reject_named_arguments(&format!("{collection}::from"), args) {
            return;
        }
        if args.len() != 1 {
            self.report_argument_count_mismatch(
                &format!("{collection}::from"),
                1,
                1,
                args.len(),
                span,
            );
            return;
        }

        let source_expr = &args[0].value;
        let source = self.infer_expr_type(source_expr, scopes, method_context);
        let elements = match (collection, self.types.kind(source).clone()) {
            ("SortedDictionary", TypeKind::Dictionary(key, value)) => Some((key, Some(value))),
            (
                "SortedSet" | "PriorityQueue" | "Deque",
                TypeKind::TypedArray(element) | TypeKind::List(element),
            ) => Some((element, None)),
            (_, TypeKind::EmptyCollection) => None,
            ("SortedDictionary", _) => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0403",
                        format!(
                            "`SortedDictionary::from` requires a `Dictionary<K, V>` source, got `{}`",
                            self.types.display(source)
                        ),
                        source_expr.span(),
                    )
                    .with_title("Dictionary Source Required"),
                );
                return;
            }
            (_, _) => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0403",
                        format!(
                            "`{collection}::from` requires a `T[]` or `List<T>` source, got `{}`",
                            self.types.display(source)
                        ),
                        source_expr.span(),
                    )
                    .with_title("Sequence Source Required"),
                );
                return;
            }
        };

        let Some((first, second)) = elements else {
            return;
        };
        if collection == "SortedDictionary" {
            self.check_stage26_comparable_type(first, source_expr.span(), "SortedDictionary key");
        } else if matches!(collection, "SortedSet" | "PriorityQueue") {
            self.check_stage26_comparable_type(
                first,
                source_expr.span(),
                &format!("{collection} element"),
            );
        }

        self.check_non_consuming_collection_duplication(
            collection,
            &format!("{collection}::from"),
            first,
            second,
            source_expr.span(),
        );
    }

    fn check_non_consuming_collection_duplication(
        &mut self,
        collection: &str,
        operation: &str,
        first: TypeId,
        second: Option<TypeId>,
        span: Span,
    ) {
        if self.type_is_move_type(first)
            || second.is_some_and(|value| self.type_is_move_type(value))
        {
            let element = second
                .map(|value| {
                    format!(
                        "{} and {}",
                        self.types.display(first),
                        self.types.display(value)
                    )
                })
                .unwrap_or_else(|| self.types.display(first));
            self.diagnostics.push(
                Diagnostic::unsupported_stage(
                    "E0528",
                    format!(
                        "`{operation}` preserves its source, so owned move value `{element}` cannot be duplicated before Stage 35 `Cloneable`"
                    ),
                    span,
                )
                .with_title("Collection Elements Cannot Be Duplicated")
                .with_explanation(format!(
                    "`{operation}` leaves every input collection unchanged, so every stored value must be copied."
                ))
                .with_help(match collection {
                    "Deque" => format!(
                        "create an empty `Deque<{}>` and move each value into it with `pushBack`",
                        self.types.display(first)
                    ),
                    "PriorityQueue" => format!(
                        "create an empty `PriorityQueue<{}>` and move each value into it with `push`",
                        self.types.display(first)
                    ),
                    "Set" | "SortedSet" => {
                        format!("move values individually into an empty {collection} with `add`")
                    }
                    "SortedDictionary" => {
                        "insert entries individually into an empty SortedDictionary with `set`"
                            .to_string()
                    }
                    _ => format!(
                        "build the destination incrementally; Stage 35 widens `{collection}` duplication through `Cloneable`"
                    ),
                }),
            );
        }
    }

    fn check_stage26_comparable_type(&mut self, ty: TypeId, span: Span, role: &str) {
        let diagnostic = match self.types.kind(ty) {
            TypeKind::Integer(_)
            | TypeKind::String
            | TypeKind::Bool
            | TypeKind::Unknown => return,
            TypeKind::TypeParameter(parameter)
                if self.type_parameter_has_constraint(parameter, "Comparable") =>
            {
                return;
            }
            TypeKind::TypeParameter(parameter) => Diagnostic::new(
                "E0537",
                format!(
                    "{role} type parameter `{parameter}` is not guaranteed to implement `Comparable`"
                ),
                span,
            )
            .with_title("Comparable Constraint Required")
            .with_help(format!(
                "declare `{parameter} implements Comparable` before using it in an ordered collection"
            )),
            TypeKind::Float(_) => Diagnostic::new(
                "E0523",
                format!(
                    "{role} type `{}` does not provide the total order required by ordered collections",
                    self.types.display(ty)
                ),
                span,
            )
            .with_title("Float Has No Collection Order")
            .with_explanation(
                "NaN and signed zero prevent Doria floats from defining the total order required by sorted collections and PriorityQueue.",
            )
            .with_help("use an integer, bool, or string element, or wrap the value in a later user-defined Comparable type"),
            TypeKind::Class(_) => Diagnostic::unsupported_stage(
                "E0523",
                format!(
                    "{role} type `{}` requires Stage 35 user-defined `Comparable` conformance",
                    self.types.display(ty)
                ),
                span,
            ),
            _ => Diagnostic::new(
                "E0523",
                format!(
                    "{role} type `{}` does not conform to `Comparable`",
                    self.types.display(ty)
                ),
                span,
            )
            .with_title("Comparable Type Required")
            .with_help("use an integer, bool, or string element; float has no total collection order"),
        };
        if !self.diagnostics.iter().any(|existing| {
            existing.code == diagnostic.code && existing.message == diagnostic.message
        }) {
            self.diagnostics.push(diagnostic);
        }
    }

    fn reject_type_ref(
        &mut self,
        ty: &TypeRef,
        span: Span,
        code: &'static str,
        message: impl Into<String>,
    ) -> TypeId {
        for arg in ty.type_arguments() {
            self.resolve_type_ref_in_position(arg, span, TypePosition::Value, None);
        }
        self.diagnostics.push(Diagnostic::new(code, message, span));
        self.types.unknown()
    }

    fn reject_type_ref_with_help(
        &mut self,
        ty: &TypeRef,
        span: Span,
        code: &'static str,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> TypeId {
        for arg in ty.type_arguments() {
            self.resolve_type_ref_in_position(arg, span, TypePosition::Value, None);
        }
        self.diagnostics
            .push(Diagnostic::new(code, message, span).with_help(help));
        self.types.unknown()
    }

    fn resolve_zero_arg_type(&mut self, ty: &TypeRef, span: Span, kind: TypeKind) -> TypeId {
        self.expect_type_arg_count(ty, 0, span);
        for arg in ty.type_arguments() {
            self.resolve_type_ref(arg, span);
        }
        self.types.intern(kind)
    }

    fn expect_type_arg_count(&mut self, ty: &TypeRef, expected: usize, span: Span) -> bool {
        let found = ty.type_argument_count();
        if found == expected && !ty.has_value_arguments() {
            return true;
        }

        self.diagnostics.push(Diagnostic::new(
            "E0402",
            format!(
                "type `{}` expects {} type argument{}, found {}",
                ty.name,
                expected,
                if expected == 1 { "" } else { "s" },
                found
            ),
            span,
        ));
        false
    }

    fn check_class_type_constraints(
        &mut self,
        class_name: &str,
        params: &[TypeParamInfo],
        arguments: &[TypeId],
        span: Span,
    ) {
        let bindings = params
            .iter()
            .zip(arguments)
            .map(|(param, argument)| (param.name.clone(), *argument))
            .collect::<HashMap<_, _>>();
        for (param, argument) in params.iter().zip(arguments) {
            for constraint in &param.constraints {
                if !Self::is_compiler_known_constraint(&constraint.name)
                    || self.type_is_symbolic(*argument)
                {
                    continue;
                }
                if self.constraint_accepts_type_argument(constraint, *argument, &bindings, span) {
                    continue;
                }
                let message = format!(
                    "type argument `{}` for `{class_name}<...>` does not satisfy required constraint `{}`",
                    self.types.display(*argument),
                    constraint
                );
                if !self.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "E0535"
                        && diagnostic.span == span
                        && diagnostic.message == message
                }) {
                    self.diagnostics
                        .push(Diagnostic::new("E0535", message, span));
                }
            }
        }
    }

    fn check_concrete_class_constraints(&mut self, class: &ClassType<TypeId>, span: Span) {
        let params = self
            .classes
            .get(&class.name)
            .map(|info| info.type_params.clone())
            .unwrap_or_default();
        self.check_class_type_constraints(&class.name, &params, &class.arguments, span);
    }

    fn check_generic_type_constraints(
        &mut self,
        callee: &str,
        params: &[TypeParamInfo],
        bindings: &HashMap<String, TypeId>,
        span: Span,
    ) {
        for param in params {
            let Some(argument) = bindings.get(&param.name).copied() else {
                continue;
            };
            for constraint in &param.constraints {
                if !Self::is_compiler_known_constraint(&constraint.name)
                    || self.type_is_symbolic(argument)
                    || self.constraint_accepts_type_argument(constraint, argument, bindings, span)
                {
                    continue;
                }
                let message = format!(
                    "type argument `{}` for {callee} does not satisfy required constraint `{constraint}`",
                    self.types.display(argument)
                );
                if !self.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "E0535"
                        && diagnostic.span == span
                        && diagnostic.message == message
                }) {
                    self.diagnostics
                        .push(Diagnostic::new("E0535", message, span));
                }
            }
        }
    }

    fn constraint_accepts_type_argument(
        &mut self,
        constraint: &TypeRef,
        argument: TypeId,
        bindings: &HashMap<String, TypeId>,
        span: Span,
    ) -> bool {
        if !self.type_satisfies_compiler_constraint(argument, &constraint.name) {
            return false;
        }
        if constraint.arguments.is_empty() {
            return true;
        }
        if !matches!(constraint.name.as_str(), "Equatable" | "Comparable")
            || constraint.has_value_arguments()
            || constraint.type_argument_count() != 1
        {
            return false;
        }

        let scope = bindings
            .keys()
            .map(|name| (name.clone(), Vec::new()))
            .collect::<HashMap<_, _>>();
        self.type_parameter_scopes.push(scope);
        let required = self.resolve_type_ref_in_position(
            constraint.type_argument(0).unwrap(),
            span,
            TypePosition::Value,
            None,
        );
        self.type_parameter_scopes.pop();
        self.substitute_type_id(required, bindings) == argument
    }

    fn type_satisfies_compiler_constraint(&self, ty: TypeId, constraint: &str) -> bool {
        match constraint {
            "Equatable" => matches!(
                self.types.kind(ty),
                TypeKind::Integer(_)
                    | TypeKind::Float(_)
                    | TypeKind::Bool
                    | TypeKind::String
                    | TypeKind::Unknown
            ),
            "Comparable" | "Hashable" => matches!(
                self.types.kind(ty),
                TypeKind::Integer(_) | TypeKind::Bool | TypeKind::String | TypeKind::Unknown
            ),
            "Displayable" => match self.types.kind(ty) {
                TypeKind::Integer(_)
                | TypeKind::Float(_)
                | TypeKind::Bool
                | TypeKind::String
                | TypeKind::Unknown => true,
                TypeKind::Class(class) => self
                    .classes
                    .get(&class.name)
                    .is_some_and(|info| info.implements(BuiltinInterface::Displayable)),
                _ => false,
            },
            _ => false,
        }
    }

    fn check_assignable(
        &mut self,
        target: TypeId,
        value: TypeId,
        span: Span,
        destination: AssignmentDestination,
    ) {
        let expected_function = self.non_null_function_type(target).cloned();
        let actual_function = self.non_null_function_type(value).cloned();
        if let (Some(expected), Some(actual)) = (expected_function, actual_function) {
            let mismatch = if matches!(self.types.kind(target), TypeKind::Function(_))
                && matches!(self.types.kind(value), TypeKind::Nullable(_))
            {
                Some(FunctionTypeMismatch::Nullability)
            } else {
                self.function_type_compatibility(&expected, &actual).err()
            };
            if let Some(mismatch) = mismatch {
                self.report_function_type_mismatch(&expected, &actual, mismatch, span);
                return;
            }
        }

        let target_name = self.types.display(target);
        let value_name = self.types.display(value);
        let message = match destination {
            AssignmentDestination::Type => {
                format!("cannot assign value of type `{value_name}` to `{target_name}`")
            }
            AssignmentDestination::Parameter { name } => format!(
                "cannot assign default value of type `{value_name}` to parameter `${name}` of type `{target_name}`"
            ),
            AssignmentDestination::Property { class_name, name } => format!(
                "cannot assign value of type `{value_name}` to property `{class_name}::{name}` of type `{target_name}`"
            ),
        };

        self.diagnostics.push(
            Diagnostic::new("E0403", message, span)
                .with_title("Type Mismatch")
                .with_primary_label("This Value Has the Wrong Type")
                .with_explanation(format!(
                    "This position requires `{target_name}`, but the expression produces `{value_name}`."
                ))
                .with_help("Change the expression or the declared destination type."),
        );
    }

    fn check_expr_assignable(
        &mut self,
        target: TypeId,
        value_expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        destination: AssignmentDestination,
    ) -> bool {
        self.complete_generic_call_from_expected(value_expr, target);

        if matches!(value_expr, Expr::Array { .. }) {
            let constructor = match self.types.kind(target) {
                TypeKind::SortedDictionary(_, _) => Some("SortedDictionary::from"),
                TypeKind::SortedSet(_) => Some("SortedSet::from"),
                TypeKind::PriorityQueue(_) => Some("PriorityQueue::from"),
                TypeKind::Deque(_) => Some("Deque::from"),
                _ => None,
            };
            if let Some(constructor) = constructor {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0538",
                        "this collection type has no direct bracket-literal construction form",
                        value_expr.span(),
                    )
                    .with_title("Explicit Collection Construction Required")
                    .with_explanation(
                        "Bracket literals directly construct List, Dictionary, or typed-array values; this collection family is constructed explicitly.",
                    )
                    .with_help(format!("wrap the literal with `{constructor}(...)`")),
                );
                return false;
            }
        }
        if let Some(fits) = self.contextualize_scalar_literals(target, value_expr) {
            return fits;
        }

        if let Expr::ArrayRepeat { value, .. } = value_expr {
            return self.check_repeat_literal_assignable(
                target,
                value,
                scopes,
                method_context,
                destination,
            );
        }

        let value = self.infer_expr_type(value_expr, scopes, method_context);
        if self.is_expr_assignable(target, value_expr, scopes, method_context)
            || self.is_assignable(target, value)
        {
            return true;
        }

        self.check_assignable(target, value, value_expr.span(), destination);
        false
    }

    fn stage26_collection_has_unknown_arguments(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TypeKind::SortedDictionary(key, value) => matches!(
                (self.types.kind(*key), self.types.kind(*value)),
                (TypeKind::Unknown, _) | (_, TypeKind::Unknown)
            ),
            TypeKind::SortedSet(value)
            | TypeKind::PriorityQueue(value)
            | TypeKind::Deque(value) => matches!(self.types.kind(*value), TypeKind::Unknown),
            _ => false,
        }
    }

    fn narrow_empty_collection_assignment(
        &self,
        target: &Expr,
        target_ty: TypeId,
        value_ty: TypeId,
        scopes: &mut ScopeStack,
    ) {
        if !matches!(self.types.kind(target_ty), TypeKind::EmptyCollection)
            || !self.is_non_empty_collection_like_type(value_ty)
        {
            return;
        }

        let Expr::Variable { name, .. } = target else {
            return;
        };

        let Some(binding) = scopes.lookup_mut(name) else {
            return;
        };

        if matches!(self.types.kind(binding.ty), TypeKind::EmptyCollection) {
            binding.ty = value_ty;
        }
    }

    fn update_nullable_assignment_flow_type(
        &self,
        target: &Expr,
        value_ty: TypeId,
        scopes: &mut ScopeStack,
    ) {
        let Some(name) = Self::assignment_target_variable_name(target) else {
            return;
        };
        let Some(binding) = scopes.lookup_mut(name) else {
            return;
        };
        let TypeKind::Nullable(inner) = *self.types.kind(binding.declared_ty) else {
            return;
        };
        binding.ty = if value_ty == inner {
            value_ty
        } else {
            binding.declared_ty
        };
    }

    fn is_expr_assignable(
        &mut self,
        target: TypeId,
        value_expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        self.is_expr_assignable_impl(target, value_expr, scopes, method_context, true)
    }

    fn is_expr_assignable_impl(
        &mut self,
        target: TypeId,
        value_expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        record_boundary: bool,
    ) -> bool {
        self.complete_generic_call_from_expected(value_expr, target);
        let target_is_mixed_boundary = matches!(self.types.kind(target), TypeKind::Mixed)
            || matches!(
                self.types.kind(target),
                TypeKind::Nullable(inner) if matches!(self.types.kind(*inner), TypeKind::Mixed)
            );
        if record_boundary && target_is_mixed_boundary {
            let value = self.infer_expr_type(value_expr, scopes, method_context);
            self.record_mixed_boundary(target, value_expr, value);
        }
        if let Some(fits) = self.contextualize_scalar_literals(target, value_expr) {
            return fits;
        }

        match value_expr {
            Expr::Grouped { expr, .. } => {
                self.is_expr_assignable_impl(target, expr, scopes, method_context, false)
            }
            Expr::Array { elements, .. } => {
                self.is_array_literal_assignable(target, elements, scopes, method_context)
            }
            Expr::ArrayRepeat { value, .. } => {
                self.is_repeat_literal_assignable(target, value, scopes, method_context)
            }
            _ => {
                let value = self.infer_expr_type(value_expr, scopes, method_context);
                self.is_assignable(target, value)
            }
        }
    }

    fn contextualize_scalar_literals(&mut self, target: TypeId, value_expr: &Expr) -> Option<bool> {
        let target = match *self.types.kind(target) {
            TypeKind::Nullable(inner) => inner,
            _ => target,
        };
        match *self.types.kind(target) {
            TypeKind::Integer(integer) => {
                if let Some(fits) = self.check_contextual_integer_literal(value_expr, integer) {
                    return Some(fits);
                }
                self.contextualize_integer_literals(value_expr, integer);
            }
            TypeKind::Float(float) => self.contextualize_float_literals(value_expr, float),
            _ => {}
        }
        None
    }

    fn is_array_literal_assignable(
        &mut self,
        target: TypeId,
        elements: &[ArrayElement],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let target_kind = self.types.kind(target).clone();

        match target_kind {
            TypeKind::Mixed | TypeKind::Unknown => true,
            TypeKind::TypedArray(element) => {
                if elements.iter().any(|element| element.key.is_some()) {
                    return false;
                }

                elements.iter().all(|array_element| {
                    self.is_expr_assignable(element, &array_element.value, scopes, method_context)
                })
            }
            TypeKind::List(element) => {
                if self.is_unknown_type(element)
                    || elements.iter().any(|element| element.key.is_some())
                {
                    return false;
                }

                elements.iter().all(|array_element| {
                    self.is_expr_assignable(element, &array_element.value, scopes, method_context)
                })
            }
            TypeKind::Dictionary(key, value) => {
                if self.is_unknown_type(key) || self.is_unknown_type(value) {
                    return false;
                }

                if elements.is_empty() {
                    return true;
                }

                if elements.iter().any(|element| element.key.is_none()) {
                    return false;
                }

                elements.iter().all(|array_element| {
                    let key_ok = array_element.key.as_ref().is_some_and(|key_expr| {
                        self.is_expr_assignable(key, key_expr, scopes, method_context)
                    });
                    let value_ok = self.is_expr_assignable(
                        value,
                        &array_element.value,
                        scopes,
                        method_context,
                    );
                    key_ok && value_ok
                })
            }
            _ => {
                let value = self.infer_array_type(elements, scopes, method_context);
                self.is_assignable(target, value)
            }
        }
    }

    fn check_repeat_count(
        &mut self,
        count: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        self.contextualize_integer_literals(count, IntegerType::Int64);
        let count_ty = self.infer_expr_type(count, scopes, method_context);
        if !matches!(
            self.types.kind(count_ty),
            TypeKind::Integer(IntegerType::Int64) | TypeKind::Unknown
        ) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0527",
                    format!(
                        "sequence fill count must have type `int`, got `{}`",
                        self.types.display(count_ty)
                    ),
                    count.span(),
                )
                .with_help("use a runtime `int` expression for the fill count"),
            );
            return;
        }

        let local_constant = Self::eval_int_constant(count, scopes, IntegerType::Int64);
        let evaluated_constant = matches!(
            local_constant,
            IntConstantEval::Unknown | IntConstantEval::Invalid
        )
        .then(|| {
            crate::const_eval::evaluate_parameter_default(
                &self.const_evaluation,
                count,
                &TypeRef::named("int"),
                method_context.map(|context| context.class_name.as_str()),
            )
        })
        .flatten();
        let is_negative = match (local_constant, evaluated_constant) {
            (IntConstantEval::Known(value), _) => value.signed_value() < 0,
            (
                IntConstantEval::Unknown | IntConstantEval::Invalid,
                Some(crate::const_eval::ConstValue::Integer(value)),
            ) => value.signed_value() < 0,
            (IntConstantEval::Unknown | IntConstantEval::Invalid, _) => false,
        };
        if is_negative {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0527",
                    "sequence fill count cannot be negative at compile time",
                    count.span(),
                )
                .with_help(
                    "use a non-negative count; a runtime-negative count panics with `fill count is negative`",
                ),
            );
        }
    }

    fn check_repeat_element_eligibility(&mut self, element: TypeId, span: Span) -> bool {
        match self.types.kind(element) {
            TypeKind::Bool
            | TypeKind::Integer(_)
            | TypeKind::Float(_)
            | TypeKind::String
            | TypeKind::Unknown => true,
            _ => {
                if !self
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0528" && diagnostic.span == span)
                {
                    self.diagnostics.push(
                        Diagnostic::unsupported_stage(
                            "E0528",
                            format!(
                                "sequence fill cannot replicate move-type element `{}` in Stage 23c (decision 0102); this requires `Cloneable` in Stage 35",
                                self.types.display(element)
                            ),
                            span,
                        )
                        .with_help(
                            "use a Copy scalar or string element until the `Cloneable` contract is available",
                        ),
                    );
                }
                false
            }
        }
    }

    fn is_repeat_literal_assignable(
        &mut self,
        target: TypeId,
        value: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        match self.types.kind(target).clone() {
            TypeKind::Mixed | TypeKind::Unknown => true,
            TypeKind::TypedArray(element) | TypeKind::List(element) => {
                self.check_repeat_element_eligibility(element, value.span())
                    && self.is_expr_assignable(element, value, scopes, method_context)
            }
            TypeKind::Set(_) | TypeKind::Dictionary(_, _) => false,
            _ => {
                let inferred = self.infer_expr_type(value, scopes, method_context);
                let repeated = self.types.intern(TypeKind::List(inferred));
                self.is_assignable(target, repeated)
            }
        }
    }

    fn check_repeat_literal_assignable(
        &mut self,
        target: TypeId,
        value: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
        destination: AssignmentDestination,
    ) -> bool {
        match self.types.kind(target).clone() {
            TypeKind::Set(_) | TypeKind::Dictionary(_, _) => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0529",
                        format!(
                            "sequence fill literal cannot construct `{}`",
                            self.types.display(target)
                        ),
                        value.span(),
                    )
                    .with_help(
                        "`[value; count]` constructs only `T[]` or `List<T>`; Set and Dictionary fills are intentionally unsupported",
                    ),
                );
                false
            }
            TypeKind::TypedArray(element) | TypeKind::List(element) => {
                if !self.check_repeat_element_eligibility(element, value.span()) {
                    return false;
                }
                if self.is_expr_assignable(element, value, scopes, method_context) {
                    true
                } else {
                    let value_ty = self.infer_expr_type(value, scopes, method_context);
                    self.check_assignable(
                        element,
                        value_ty,
                        value.span(),
                        AssignmentDestination::Type,
                    );
                    false
                }
            }
            _ => {
                let element = self.infer_expr_type(value, scopes, method_context);
                let repeated = self.types.intern(TypeKind::List(element));
                if self.is_assignable(target, repeated) {
                    true
                } else {
                    self.check_assignable(target, repeated, value.span(), destination);
                    false
                }
            }
        }
    }

    fn is_unknown_type(&self, ty: TypeId) -> bool {
        matches!(self.types.kind(ty), TypeKind::Unknown)
    }

    fn is_assignable(&self, target: TypeId, value: TypeId) -> bool {
        if target == value {
            return true;
        }

        let target_kind = self.types.kind(target).clone();
        let value_kind = self.types.kind(value).clone();
        match (target_kind, value_kind) {
            (TypeKind::Heterogeneous, _) | (_, TypeKind::Heterogeneous) => false,
            (TypeKind::Mixed, _) => true,
            (TypeKind::Nullable(target), TypeKind::Mixed)
                if matches!(self.types.kind(target), TypeKind::Mixed) =>
            {
                true
            }
            (_, TypeKind::Mixed) => false,
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown) => true,
            (TypeKind::Nullable(_), TypeKind::Null) => true,
            (TypeKind::Nullable(target), TypeKind::Nullable(value)) => {
                self.is_assignable(target, value)
            }
            (TypeKind::Nullable(target), _) => self.is_assignable(target, value),
            (
                TypeKind::TypedArray(_) | TypeKind::List(_) | TypeKind::Dictionary(_, _),
                TypeKind::EmptyCollection,
            ) => true,
            (
                TypeKind::EmptyCollection,
                TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_),
            ) => true,
            (TypeKind::Class(target), TypeKind::Class(value)) => target == value,
            (TypeKind::Function(target), TypeKind::Function(value)) => {
                self.function_type_compatibility(&target, &value).is_ok()
            }
            (TypeKind::Error, TypeKind::Class(value)) => self
                .classes
                .get(&value.name)
                .is_some_and(|class| class.implements(BuiltinInterface::Error)),
            (TypeKind::TypedArray(target), TypeKind::TypedArray(value)) => {
                self.is_assignable(target, value)
            }
            (TypeKind::List(target), TypeKind::List(value)) => self.is_assignable(target, value),
            (
                TypeKind::Dictionary(target_key, target_value),
                TypeKind::Dictionary(value_key, value_value),
            ) => {
                self.is_assignable(target_key, value_key)
                    && self.is_assignable(target_value, value_value)
            }
            (
                TypeKind::SortedDictionary(target_key, target_value),
                TypeKind::SortedDictionary(value_key, value_value),
            ) => {
                self.is_assignable(target_key, value_key)
                    && self.is_assignable(target_value, value_value)
            }
            (TypeKind::Set(target), TypeKind::Set(value)) => self.is_assignable(target, value),
            (TypeKind::SortedSet(target), TypeKind::SortedSet(value))
            | (TypeKind::PriorityQueue(target), TypeKind::PriorityQueue(value))
            | (TypeKind::Deque(target), TypeKind::Deque(value)) => {
                self.is_assignable(target, value)
            }
            // Shared handles are assignable only within the same handle kind, so the
            // families stay disjoint (record 0106) while a symbolic payload still
            // matches its concrete specialization.
            (
                TypeKind::SharedHandle(target_kind, target),
                TypeKind::SharedHandle(value_kind, value),
            ) if target_kind == value_kind => {
                // An unresolved payload parameter matches its concrete
                // specialization; the binding itself is checked by inference.
                matches!(self.types.kind(target), TypeKind::TypeParameter(_))
                    || self.is_assignable(target, value)
            }
            (TypeKind::TypeParameter(target), TypeKind::TypeParameter(value)) => target == value,
            _ => false,
        }
    }

    fn infer_expr_type(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let ty = self.infer_expr_type_unrecorded(expr, scopes, method_context);
        match self.types.kind(ty) {
            TypeKind::Integer(integer) => {
                self.integer_expression_types.insert(expr.span(), *integer);
            }
            TypeKind::Float(float) => {
                self.float_expression_types.insert(expr.span(), *float);
            }
            _ => {}
        }
        self.expression_types
            .insert(expr.span(), self.types.resolved(ty));
        ty
    }

    fn infer_expr_type_unrecorded(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        match expr {
            Expr::Closure(closure) => self
                .closure_types
                .get(&closure.span)
                .copied()
                .unwrap_or_else(|| self.types.unknown()),
            Expr::CallableCall { span, .. } => self
                .callable_value_calls
                .get(span)
                .map(|call| call.return_type.clone())
                .map(|ty| self.types.intern_resolved(&ty))
                .unwrap_or_else(|| self.types.unknown()),
            Expr::String { .. } | Expr::InterpolatedString { .. } => {
                self.types.intern(TypeKind::String)
            }
            Expr::Int { span, .. } => {
                let integer = self
                    .integer_expression_types
                    .get(span)
                    .copied()
                    .unwrap_or(IntegerType::Int64);
                self.types.intern(TypeKind::Integer(integer))
            }
            Expr::Float { span, .. } => {
                let float = self
                    .float_expression_types
                    .get(span)
                    .copied()
                    .unwrap_or(FloatType::Float64);
                self.types.intern(TypeKind::Float(float))
            }
            Expr::Bool { .. } => self.types.intern(TypeKind::Bool),
            Expr::Null { .. } => self.types.intern(TypeKind::Null),
            Expr::New {
                class_type,
                args,
                shared,
                span,
            } => {
                // `new WritableSharedReference(new T(...))` infers its one type
                // argument from the single constructor argument (record 0106). This
                // is compiler-known inference for this type, not a general
                // user-defined generic-constructor rule.
                if let Some(kind) = SharedHandleKind::from_source_name(&class_type.name) {
                    let payload = match class_type.type_argument(0) {
                        Some(argument) => self.resolve_type_ref_with_class(
                            argument,
                            *span,
                            method_context.map(|context| context.class_name.as_str()),
                        ),
                        None => match args.first() {
                            Some(argument) => {
                                self.infer_expr_type(&argument.value, scopes, method_context)
                            }
                            None => self.types.unknown(),
                        },
                    };
                    return self.types.intern(TypeKind::SharedHandle(kind, payload));
                }
                let payload = self.resolve_type_ref_with_class(
                    class_type,
                    *span,
                    method_context.map(|context| context.class_name.as_str()),
                );
                if *shared {
                    // `shared new T(...)` has static type `SharedReference<T>`
                    // (record 0106); plain `new T(...)` stays an owned `T`.
                    self.types.intern(TypeKind::SharedHandle(
                        SharedHandleKind::SharedReference,
                        payload,
                    ))
                } else {
                    payload
                }
            }
            Expr::Array { elements, .. } => self.infer_array_type(elements, scopes, method_context),
            Expr::ArrayRepeat { value, .. } => {
                let element = self.infer_expr_type(value, scopes, method_context);
                self.types.intern(TypeKind::List(element))
            }
            Expr::Index { collection, .. } => self
                .collection_index_types(collection, scopes, method_context)
                .map(|(_, value)| value)
                .unwrap_or_else(|| self.types.unknown()),
            Expr::Variable { name, span } => {
                let Some(binding) = scopes.lookup(name) else {
                    return self.types.unknown();
                };
                self.flow_narrowed_type(binding.declared_ty, binding.ty, *span, method_context)
            }
            Expr::Identifier { name, .. } => {
                let key = crate::const_eval::ConstKey::TopLevel(name.clone());
                let ty = self.const_evaluation.values.get(&key).map(|value| value.ty);
                ty.map(|ty| self.const_type_id(ty))
                    .unwrap_or_else(|| self.types.unknown())
            }
            Expr::This { .. } => method_context
                .filter(|context| context.receiver_access.is_available())
                .map(|context| context.class_name.clone())
                .map(|class_name| self.symbolic_class_type(&class_name))
                .unwrap_or_else(|| self.types.unknown()),
            Expr::PropertyAccess {
                object,
                property,
                null_safe,
                ..
            } => {
                if let Some(result) = self.shared_handle_property_type(
                    object,
                    property,
                    *null_safe,
                    scopes,
                    method_context,
                ) {
                    return result;
                }
                let object_ty = self.infer_expr_type(object, scopes, method_context);
                if let Some((kind, _)) = self.shared_handle_type(object_ty, *null_safe) {
                    if !Self::shared_handle_forwards(kind) {
                        return self.types.unknown();
                    }
                }
                if let Some(result) = self.compiler_known_property_type(
                    object,
                    property,
                    *null_safe,
                    scopes,
                    method_context,
                ) {
                    return result;
                }
                let Some(class_type) = self.expr_class_type(object, scopes, method_context) else {
                    return self.types.unknown();
                };
                let property = self
                    .classes
                    .get(&class_type.name)
                    .and_then(|class_info| class_info.properties.get(property))
                    .cloned();
                let result = property
                    .map(|property| {
                        self.specialize_property_for_class(&property, &class_type)
                            .ty
                    })
                    .unwrap_or_else(|| self.types.unknown());
                let result = if *null_safe
                    && !matches!(self.types.kind(result), TypeKind::Void | TypeKind::Unknown)
                {
                    if matches!(self.types.kind(result), TypeKind::Nullable(_)) {
                        result
                    } else {
                        self.types.intern(TypeKind::Nullable(result))
                    }
                } else {
                    result
                };
                result
            }
            Expr::MethodCall {
                object,
                method,
                null_safe,
                span,
                ..
            } => {
                if let Some(call) = self.callable_value_calls.get(span) {
                    return self.types.intern_resolved(&call.return_type.clone());
                }
                if let Some(call) = self.list_algorithm_calls.get(span) {
                    return self.types.intern_resolved(&call.result_type.clone());
                }
                let object_ty = self.infer_expr_type(object, scopes, method_context);
                if let Some((kind, payload)) = self.shared_handle_type(object_ty, *null_safe) {
                    if let Some(result) =
                        self.shared_handle_member_return_type(kind, payload, method)
                    {
                        return self.null_safe_result_type(result, *null_safe);
                    }
                    if !Self::shared_handle_forwards(kind) {
                        return self.types.unknown();
                    }
                }
                if let Some(result) = self.collection_method_return_type(
                    object,
                    method,
                    *null_safe,
                    scopes,
                    method_context,
                ) {
                    return result;
                }
                if let TypeKind::TypeParameter(parameter) = self.types.kind(object_ty) {
                    if method == "toString"
                        && self.type_parameter_has_constraint(parameter, "Displayable")
                    {
                        return self.types.intern(TypeKind::String);
                    }
                    return self.types.unknown();
                }
                let Some(class_type) = self.expr_class_type(object, scopes, method_context) else {
                    return self.types.unknown();
                };
                let method_info = self
                    .classes
                    .get(&class_type.name)
                    .and_then(|class_info| class_info.methods.get(method))
                    .cloned();
                let result = method_info
                    .map(|method| {
                        self.specialize_method_for_class(&method, &class_type)
                            .return_ty
                    })
                    .unwrap_or_else(|| self.types.unknown());
                let result = self.generic_call_result_type(*span, result);
                if *null_safe
                    && !matches!(self.types.kind(result), TypeKind::Void | TypeKind::Unknown)
                {
                    if matches!(self.types.kind(result), TypeKind::Nullable(_)) {
                        result
                    } else {
                        self.types.intern(TypeKind::Nullable(result))
                    }
                } else {
                    result
                }
            }
            Expr::IsType { .. } => self.types.intern(TypeKind::Bool),
            Expr::FunctionCall { name, span, .. } => {
                if let Some(builtin) = Builtin::from_name(name) {
                    match builtin {
                        Builtin::ReadLine => {
                            let string = self.types.intern(TypeKind::String);
                            self.types.intern(TypeKind::Nullable(string))
                        }
                        Builtin::Sprintf | Builtin::ReadFile => self.types.intern(TypeKind::String),
                        Builtin::ReadFileBytes | Builtin::ReadStdinBytes => {
                            self.types.intern(TypeKind::Bytes)
                        }
                        Builtin::Printf
                        | Builtin::WriteFile
                        | Builtin::AppendFile
                        | Builtin::WriteStderr
                        | Builtin::WriteFileBytes
                        | Builtin::AppendFileBytes
                        | Builtin::WriteStdoutBytes
                        | Builtin::WriteStderrBytes
                        | Builtin::Panic => self.types.intern(TypeKind::Void),
                    }
                } else {
                    let result = self
                        .functions
                        .get(name)
                        .map(|function| function.return_ty)
                        .unwrap_or_else(|| self.types.unknown());
                    self.generic_call_result_type(*span, result)
                }
            }
            Expr::StaticCall {
                qualifier,
                method,
                args,
                span,
                ..
            } => {
                let Some(class_name) = Self::static_qualifier_class_name(qualifier, method_context)
                else {
                    return self.types.unknown();
                };
                if let Some(definition) = self.enums.get(&class_name) {
                    return self.types.intern(TypeKind::Enum(EnumType::new(
                        definition.id,
                        definition.name.clone(),
                    )));
                }
                if class_name == "Int" && method == "toFloat" {
                    return self.types.intern(TypeKind::Float(FloatType::Float64));
                }
                if class_name == "Float" && method == "toInt" {
                    return self.types.intern(TypeKind::Integer(IntegerType::Int64));
                }
                if method == "parse" {
                    if IntegerType::from_companion_name(&class_name) == Some(IntegerType::Int64) {
                        let inner = self.types.intern(TypeKind::Integer(IntegerType::Int64));
                        return self.types.intern(TypeKind::Nullable(inner));
                    }
                    if matches!(class_name.as_str(), "Float" | "Float64") {
                        let inner = self.types.intern(TypeKind::Float(FloatType::Float64));
                        return self.types.intern(TypeKind::Nullable(inner));
                    }
                }
                if method == "from" {
                    if class_name == "Set" {
                        let element = match args
                            .first()
                            .map(|arg| self.infer_expr_type(&arg.value, scopes, method_context))
                        {
                            Some(source) => match self.types.kind(source) {
                                TypeKind::TypedArray(element) | TypeKind::List(element) => *element,
                                _ => self.types.unknown(),
                            },
                            None => self.types.unknown(),
                        };
                        return self.types.intern(TypeKind::Set(element));
                    }
                    if matches!(
                        class_name.as_str(),
                        "SortedDictionary" | "SortedSet" | "PriorityQueue" | "Deque"
                    ) {
                        let source = args
                            .first()
                            .map(|arg| self.infer_expr_type(&arg.value, scopes, method_context));
                        return match (
                            class_name.as_str(),
                            source.map(|source| self.types.kind(source).clone()),
                        ) {
                            ("SortedDictionary", Some(TypeKind::Dictionary(key, value))) => {
                                self.types.intern(TypeKind::SortedDictionary(key, value))
                            }
                            ("SortedDictionary", _) => {
                                let unknown = self.types.unknown();
                                self.types
                                    .intern(TypeKind::SortedDictionary(unknown, unknown))
                            }
                            (
                                "SortedSet",
                                Some(TypeKind::TypedArray(element) | TypeKind::List(element)),
                            ) => self.types.intern(TypeKind::SortedSet(element)),
                            ("SortedSet", _) => {
                                let unknown = self.types.unknown();
                                self.types.intern(TypeKind::SortedSet(unknown))
                            }
                            (
                                "PriorityQueue",
                                Some(TypeKind::TypedArray(element) | TypeKind::List(element)),
                            ) => self.types.intern(TypeKind::PriorityQueue(element)),
                            ("PriorityQueue", _) => {
                                let unknown = self.types.unknown();
                                self.types.intern(TypeKind::PriorityQueue(unknown))
                            }
                            (
                                "Deque",
                                Some(TypeKind::TypedArray(element) | TypeKind::List(element)),
                            ) => self.types.intern(TypeKind::Deque(element)),
                            ("Deque", _) => {
                                let unknown = self.types.unknown();
                                self.types.intern(TypeKind::Deque(unknown))
                            }
                            _ => unreachable!(),
                        };
                    }
                    if let Some(integer) = IntegerType::from_companion_name(&class_name) {
                        return self.types.intern(TypeKind::Integer(integer));
                    }
                }
                if class_name == "String" {
                    return self
                        .string_companion_signature(method)
                        .map(|(_, return_ty)| return_ty)
                        .unwrap_or_else(|| self.types.unknown());
                }
                if class_name == "Bytes" && method == "fromArray" {
                    return self.types.intern(TypeKind::Bytes);
                }
                let result = self
                    .classes
                    .get(&class_name)
                    .and_then(|class_info| class_info.methods.get(method))
                    .map(|method| method.return_ty)
                    .unwrap_or_else(|| self.types.unknown());
                self.generic_call_result_type(*span, result)
            }
            Expr::StaticMember {
                qualifier, member, ..
            } => {
                let Some(class_name) = Self::static_qualifier_class_name(qualifier, method_context)
                else {
                    return self.types.unknown();
                };
                if let Some(definition) = self.enums.get(&class_name) {
                    if definition.case_by_name.contains_key(member) {
                        return self.types.intern(TypeKind::Enum(EnumType::new(
                            definition.id,
                            definition.name.clone(),
                        )));
                    }
                    return self.types.unknown();
                }
                let ty = self.classes.get(&class_name).and_then(|class_info| {
                    class_info
                        .constants
                        .get(member)
                        .map(|constant| constant.ty)
                        .or_else(|| {
                            class_info
                                .static_properties
                                .get(member)
                                .map(|property| property.ty)
                        })
                });
                ty.unwrap_or_else(|| {
                    let key = crate::const_eval::ConstKey::Class {
                        class_name,
                        name: member.clone(),
                    };
                    let ty = self.const_evaluation.values.get(&key).map(|value| value.ty);
                    ty.map(|ty| self.const_type_id(ty))
                        .unwrap_or_else(|| self.types.unknown())
                })
            }
            Expr::Grouped { expr, .. } => self.infer_expr_type(expr, scopes, method_context),
            Expr::Unary { op, expr, span } => {
                self.infer_unary_type(op, expr, *span, scopes, method_context)
            }
            Expr::Binary {
                left, op, right, ..
            } => self.infer_binary_type(left, op, right, scopes, method_context),
            Expr::Range { .. } => self.types.unknown(),
            Expr::Match { span, .. } => self
                .expression_types
                .get(span)
                .cloned()
                .map(|ty| self.types.intern_resolved(&ty))
                .unwrap_or_else(|| self.types.unknown()),
            Expr::When(when) => self
                .whens
                .get(&when.span)
                .map(|info| info.result_type.clone())
                .or_else(|| self.expression_types.get(&when.span).cloned())
                .map(|ty| self.types.intern_resolved(&ty))
                .unwrap_or_else(|| self.types.unknown()),
        }
    }

    fn collection_index_types(
        &mut self,
        collection: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<(TypeId, TypeId)> {
        let mut collection_ty = self.infer_expr_type(collection, scopes, method_context);
        // Access objects forward indexed operations to their payload (record 0106),
        // which is why the writable family may hold collection payloads.
        if let TypeKind::SharedHandle(kind, payload) = *self.types.kind(collection_ty) {
            if kind.is_access() {
                collection_ty = payload;
            }
        }
        let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
        match self.types.kind(collection_ty).clone() {
            TypeKind::TypedArray(value) | TypeKind::List(value) => Some((int, value)),
            TypeKind::Bytes => {
                let byte = self.types.intern(TypeKind::Integer(IntegerType::UInt8));
                Some((int, byte))
            }
            TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) => {
                Some((key, value))
            }
            TypeKind::Set(_)
            | TypeKind::SortedSet(_)
            | TypeKind::PriorityQueue(_)
            | TypeKind::Deque(_) => None,
            _ => None,
        }
    }

    fn check_collection_index(
        &mut self,
        collection: &Expr,
        index: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let collection_ty = self.infer_expr_type(collection, scopes, method_context);
        if matches!(
            self.types.kind(collection_ty),
            TypeKind::Set(_) | TypeKind::SortedSet(_)
        ) {
            self.diagnostics.push(Diagnostic::new(
                "E0520",
                format!(
                    "`{}` does not support indexed access; use `contains` or `foreach`",
                    self.types.display(collection_ty)
                ),
                span,
            ));
            return;
        }
        if matches!(
            self.types.kind(collection_ty),
            TypeKind::PriorityQueue(_) | TypeKind::Deque(_)
        ) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0520",
                    format!(
                        "`{}` does not support indexed access",
                        self.types.display(collection_ty)
                    ),
                    span,
                )
                .with_title("Collection Is Not Indexable")
                .with_help(match self.types.kind(collection_ty) {
                    TypeKind::PriorityQueue(_) => "use `peek` or `pop()` for min-first access",
                    TypeKind::Deque(_) => {
                        "use `peekFront`, `peekBack`, `popFront()`, or `popBack()`"
                    }
                    _ => unreachable!(),
                }),
            );
            return;
        }
        let Some((key, _)) = self.collection_index_types(collection, scopes, method_context) else {
            if !matches!(self.types.kind(collection_ty), TypeKind::Unknown) {
                self.diagnostics.push(Diagnostic::new(
                    "E0520",
                    format!(
                        "value of type `{}` is not indexable",
                        self.types.display(collection_ty)
                    ),
                    span,
                ));
            }
            return;
        };
        self.check_expr_assignable(
            key,
            index,
            scopes,
            method_context,
            AssignmentDestination::Type,
        );
    }

    fn compiler_known_property_type(
        &mut self,
        object: &Expr,
        property: &str,
        null_safe: bool,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let ty = self.infer_expr_type(object, scopes, method_context);
        let forwarded_nullable_access = self
            .forwarded_access_payload(ty)
            .is_some_and(|(_, nullable)| nullable);
        let mut ty = self.forwarded_access_payload_type(ty);
        let nullable_access = if null_safe {
            match self.types.kind(ty) {
                TypeKind::Nullable(inner) => {
                    ty = *inner;
                    true
                }
                _ => forwarded_nullable_access,
            }
        } else {
            forwarded_nullable_access
        };
        let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
        let bool_ty = self.types.intern(TypeKind::Bool);
        let result = match (self.types.kind(ty), property) {
            (TypeKind::Enum(enum_type), "value") => {
                let definition = self
                    .enums
                    .values()
                    .find(|definition| definition.id == enum_type.id)?;
                definition.backing_type.map(|backing| match backing {
                    EnumBackingType::Int => int,
                    EnumBackingType::String => self.types.intern(TypeKind::String),
                })
            }
            (TypeKind::Error, "message") => Some(self.types.intern(TypeKind::String)),
            (TypeKind::String, "length" | "byteLength") => Some(int),
            (TypeKind::String, "isEmpty") => Some(bool_ty),
            (TypeKind::String, "bytes") => Some(self.types.intern(TypeKind::Bytes)),
            (TypeKind::String, "graphemes" | "codePoints") => Some(self.types.unknown()),
            (TypeKind::TypedArray(_), "length") => Some(int),
            (TypeKind::Bytes, "length") => Some(int),
            (
                TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_),
                "count",
            ) => Some(int),
            (
                TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_),
                "isEmpty",
            ) => Some(bool_ty),
            (
                TypeKind::List(value) | TypeKind::Set(value) | TypeKind::SortedSet(value),
                "first" | "last",
            ) => Some(self.types.intern(TypeKind::Nullable(*value))),
            (TypeKind::Dictionary(_, _) | TypeKind::SortedDictionary(_, _), "keys" | "values") => {
                Some(self.types.unknown())
            }
            (TypeKind::PriorityQueue(value), "peek")
            | (TypeKind::Deque(value), "peekFront" | "peekBack") => {
                Some(self.types.intern(TypeKind::Nullable(*value)))
            }
            (
                TypeKind::String
                | TypeKind::Error
                | TypeKind::Bytes
                | TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_),
                _,
            ) => Some(self.types.unknown()),
            (TypeKind::Enum(_), _) => Some(self.types.unknown()),
            _ => None,
        }?;
        Some(self.null_safe_result_type(result, null_safe && nullable_access))
    }

    fn shared_handle_type(
        &self,
        ty: TypeId,
        unwrap_nullable: bool,
    ) -> Option<(SharedHandleKind, TypeId)> {
        match self.types.kind(ty) {
            TypeKind::SharedHandle(kind, payload) => Some((*kind, *payload)),
            TypeKind::Nullable(inner) if unwrap_nullable => match self.types.kind(*inner) {
                TypeKind::SharedHandle(kind, payload) => Some((*kind, *payload)),
                _ => None,
            },
            _ => None,
        }
    }

    fn forwarded_access_payload_type(&self, ty: TypeId) -> TypeId {
        self.forwarded_access_payload(ty)
            .map(|(payload, _)| payload)
            .unwrap_or(ty)
    }

    fn forwarded_access_payload(&self, ty: TypeId) -> Option<(TypeId, bool)> {
        match self.types.kind(ty) {
            TypeKind::SharedHandle(kind, payload) if kind.is_access() => Some((*payload, false)),
            TypeKind::Nullable(inner) => match self.types.kind(*inner) {
                TypeKind::SharedHandle(kind, payload) if kind.is_access() => Some((*payload, true)),
                _ => None,
            },
            _ => None,
        }
    }

    fn null_safe_result_type(&mut self, result: TypeId, null_safe: bool) -> TypeId {
        if !null_safe
            || matches!(
                self.types.kind(result),
                TypeKind::Void | TypeKind::Unknown | TypeKind::Nullable(_)
            )
        {
            result
        } else {
            self.types.intern(TypeKind::Nullable(result))
        }
    }

    /// Return type of a compiler-known member on a shared-ownership handle
    /// (record 0106). Returns `None` when the name is not owned by the wrapper, so
    /// callers can fall through to transparent payload forwarding.
    fn shared_handle_member_return_type(
        &mut self,
        kind: SharedHandleKind,
        payload: TypeId,
        method: &str,
    ) -> Option<TypeId> {
        use SharedHandleKind::*;
        let result = match (kind, method) {
            (SharedReference, "share") => self
                .types
                .intern(TypeKind::SharedHandle(SharedReference, payload)),
            (SharedReference, "createWeakReference") => self
                .types
                .intern(TypeKind::SharedHandle(WeakReference, payload)),
            (WritableSharedReference, "share") => self
                .types
                .intern(TypeKind::SharedHandle(WritableSharedReference, payload)),
            (WritableSharedReference, "createWeakReference") => self
                .types
                .intern(TypeKind::SharedHandle(WritableWeakReference, payload)),
            (WritableSharedReference, "acquireReadonlyAccess") => self.types.intern(
                TypeKind::SharedHandle(ReadonlySharedReferenceAccess, payload),
            ),
            (WritableSharedReference, "acquireWritableAccess") => self.types.intern(
                TypeKind::SharedHandle(WritableSharedReferenceAccess, payload),
            ),
            // A weak reference acquires only within its own family, and the result
            // is nullable because the payload may already be gone.
            (WeakReference, "acquire") => {
                let strong = self
                    .types
                    .intern(TypeKind::SharedHandle(SharedReference, payload));
                self.types.intern(TypeKind::Nullable(strong))
            }
            (WritableWeakReference, "acquire") => {
                let strong = self
                    .types
                    .intern(TypeKind::SharedHandle(WritableSharedReference, payload));
                self.types.intern(TypeKind::Nullable(strong))
            }
            _ => return None,
        };
        Some(result)
    }

    /// `referencedValue` is the compiler-known readonly projection to the payload,
    /// and exists only on `SharedReference<T>` (record 0106).
    fn shared_handle_property_type(
        &mut self,
        object: &Expr,
        property: &str,
        null_safe: bool,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        let (kind, payload) = self.shared_handle_type(object_ty, null_safe)?;
        if kind != SharedHandleKind::SharedReference || property != "referencedValue" {
            return None;
        }
        Some(self.null_safe_result_type(payload, null_safe))
    }

    /// Whether a handle forwards member access to its payload at all. The writable
    /// family deliberately does not: access must be acquired first.
    fn shared_handle_forwards(kind: SharedHandleKind) -> bool {
        kind.forwards_payload()
    }

    /// Reject payload member access through handles that require an explicit
    /// access object. Every read, call, and mutation target uses this gate so a
    /// failed lookup cannot silently bypass record 0106's access protocol.
    fn reject_nonforwarding_shared_handle_member_access(
        &mut self,
        object: &Expr,
        member: &str,
        null_safe: bool,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let object_ty = self.infer_expr_type(object, scopes, method_context);
        let Some((kind, _)) = self.shared_handle_type(object_ty, null_safe) else {
            return false;
        };
        if Self::shared_handle_forwards(kind) {
            return false;
        }
        self.reject_shared_handle_member_access(kind, member, span);
        true
    }

    /// Diagnostic for member access on a handle that does not forward.
    fn reject_shared_handle_member_access(
        &mut self,
        kind: SharedHandleKind,
        member: &str,
        span: Span,
    ) {
        let name = kind.source_name();
        let (code, message, help) = match kind {
            SharedHandleKind::WritableSharedReference => (
                "E0548",
                format!("`{name}` Does Not Provide Direct Access To Its Value"),
                "acquire controlled access first: `$reference->acquireReadonlyAccess()` or \
                 `$reference->acquireWritableAccess()`"
                    .to_string(),
            ),
            SharedHandleKind::WeakReference | SharedHandleKind::WritableWeakReference => (
                "E0549",
                format!("`{name}` Has No Live Value To Access"),
                "a weak reference does not keep its value alive; call `acquire()` and check the \
                 result for `null` first"
                    .to_string(),
            ),
            _ => unreachable!("forwarding handles do not reach this diagnostic"),
        };
        self.diagnostics.push(
            Diagnostic::new(code, message, span).with_help(format!("{help} (member `{member}`)")),
        );
    }

    /// Construction rules for the six compiler-known shared-ownership types
    /// (record 0106). Only `WritableSharedReference<T>` is built with `new`, and
    /// `shared new` never names one of these types.
    #[allow(clippy::too_many_arguments)]
    fn check_shared_handle_construction(
        &mut self,
        kind: SharedHandleKind,
        class_type: &crate::types::TypeRef,
        args: &[Argument],
        shared: bool,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let name = kind.source_name();
        if shared {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0542",
                    format!("`shared new` Cannot Construct `{name}`"),
                    span,
                )
                .with_help(
                    "`shared new T(...)` constructs `SharedReference<T>` from an ordinary class; \
                     the writable family is built with `new WritableSharedReference(new T(...))`",
                ),
            );
            return;
        }

        if !kind.is_directly_constructible() {
            let help = match kind {
                SharedHandleKind::SharedReference => {
                    "construct a shared reference with `shared new T(...)`".to_string()
                }
                SharedHandleKind::WeakReference => {
                    "derive one with `$reference->createWeakReference()`".to_string()
                }
                SharedHandleKind::WritableWeakReference => {
                    "derive one with `$reference->createWeakReference()` on a \
                     `WritableSharedReference<T>`"
                        .to_string()
                }
                SharedHandleKind::ReadonlySharedReferenceAccess => {
                    "acquire one with `$reference->acquireReadonlyAccess()`".to_string()
                }
                SharedHandleKind::WritableSharedReferenceAccess => {
                    "acquire one with `$reference->acquireWritableAccess()`".to_string()
                }
                SharedHandleKind::WritableSharedReference => unreachable!(),
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0543",
                    format!("`{name}` Cannot Be Constructed Directly"),
                    span,
                )
                .with_help(help),
            );
            return;
        }

        // `WritableSharedReference<T>`: exactly one owned payload argument, and at
        // most one explicit type argument.
        if class_type.type_argument_count() > 1 {
            self.report_shared_handle_arity(kind, class_type.type_argument_count(), span);
        }
        if args.len() != 1 {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0544",
                    format!(
                        "`{name}` Takes Ownership Of Exactly One Value, But {} Were Given",
                        args.len()
                    ),
                    span,
                )
                .with_help(format!(
                    "write `new {name}(new T(...))`; the constructor parameter is named `value`"
                )),
            );
            return;
        }

        let payload = match class_type.arguments.as_slice() {
            [crate::types::TypeArgumentRef::Type(payload)] => self.resolve_type_ref_with_class(
                payload,
                span,
                method_context.map(|context| context.class_name.as_str()),
            ),
            [] => self.infer_expr_type(&args[0].value, scopes, method_context),
            _ => return,
        };
        self.check_call_arguments(
            &format!("constructor `{name}::__construct`"),
            &[ParamInfo {
                name: "value".to_string(),
                ty: payload,
                take: true,
                writable: false,
                has_default: false,
            }],
            args,
            span,
            scopes,
            method_context,
        );
    }

    /// `shared new T(...)` requires an ordinary class payload.
    fn check_shared_new_payload(&mut self, class_type: &crate::types::TypeRef, span: Span) -> bool {
        if class_type.name == "self" || class_type.as_class_name().is_some() {
            return true;
        }
        self.diagnostics.push(
            Diagnostic::new(
                "E0545",
                format!("`shared new` Requires A Class Payload, But Found `{class_type}`"),
                span,
            )
            .with_help(
                "the readonly shared-ownership family accepts class payloads in v1.0; wrap the \
                 value in a class, keep it as an ordinary owned value, or use \
                 `new WritableSharedReference(...)`, whose access objects forward member and \
                 indexed operations",
            ),
        );
        false
    }

    /// Whether a resolved payload is acceptable for a shared-handle family.
    ///
    /// The readonly family (`SharedReference<T>` / `WeakReference<T>`) accepts class
    /// payloads only in v1.0 (record 0106): it forwards readonly member access
    /// directly and has no access object to route indexed or collection operations
    /// through. The writable family carries no such restriction — its access objects
    /// explicitly support member and indexed forwarding, so supported owned
    /// collection move types may be shared through `new WritableSharedReference(...)`.
    ///
    /// A symbolic payload (an unresolved type parameter) is accepted here; every
    /// concrete specialization is checked where it is written.
    fn shared_handle_payload_is_supported(&self, kind: SharedHandleKind, payload: TypeId) -> bool {
        if kind.family() != crate::types::SharedFamily::Readonly {
            return true;
        }
        matches!(
            self.types.kind(payload),
            TypeKind::Class(_) | TypeKind::TypeParameter(_) | TypeKind::Unknown
        )
    }

    fn check_specialized_shared_payloads(&mut self, ty: TypeId, span: Span) {
        match self.types.kind(ty).clone() {
            TypeKind::Nullable(inner)
            | TypeKind::TypedArray(inner)
            | TypeKind::List(inner)
            | TypeKind::Set(inner)
            | TypeKind::SortedSet(inner)
            | TypeKind::PriorityQueue(inner)
            | TypeKind::Deque(inner) => self.check_specialized_shared_payloads(inner, span),
            TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) => {
                self.check_specialized_shared_payloads(key, span);
                self.check_specialized_shared_payloads(value, span);
            }
            TypeKind::Class(class) => {
                for argument in class.arguments {
                    self.check_specialized_shared_payloads(argument, span);
                }
            }
            TypeKind::Function(function) => {
                for parameter in function.parameters {
                    self.check_specialized_shared_payloads(parameter.ty, span);
                }
                self.check_specialized_shared_payloads(function.return_type, span);
                for effect in function.checked_effects {
                    self.check_specialized_shared_payloads(effect, span);
                }
            }
            TypeKind::SharedHandle(kind, payload) => {
                if !self.shared_handle_payload_is_supported(kind, payload) {
                    self.report_shared_handle_payload(kind, payload, span);
                }
                self.check_specialized_shared_payloads(payload, span);
            }
            _ => {}
        }
    }

    fn check_specialized_class_shared_payloads(&mut self, class: &ClassType<TypeId>, span: Span) {
        let Some(info) = self.classes.get(&class.name).cloned() else {
            return;
        };
        let substitutions = self.class_type_substitutions(class);
        let mut types = Vec::new();
        types.extend(
            info.properties
                .values()
                .map(|property| property.ty)
                .chain(info.static_properties.values().map(|property| property.ty))
                .chain(info.constants.values().map(|constant| constant.ty)),
        );
        for method in info.methods.values() {
            types.extend(method.params.iter().map(|param| param.ty));
            types.push(method.return_ty);
        }
        for ty in types {
            let specialized = self.substitute_type_id(ty, &substitutions);
            self.check_specialized_shared_payloads(specialized, span);
        }
    }

    fn report_shared_handle_payload(
        &mut self,
        kind: SharedHandleKind,
        payload: TypeId,
        span: Span,
    ) {
        let name = kind.source_name();
        let payload = self.types.display(payload);
        let message = format!("`{name}<{payload}>` Requires A Class Payload");
        if !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0545" && diagnostic.message == message)
        {
            self.diagnostics
                .push(Diagnostic::new("E0545", message, span).with_help(format!(
                    "the readonly shared-ownership family accepts class payloads in v1.0; wrap \
                     `{payload}` in a class, or use `WritableSharedReference<{payload}>`, whose \
                     access objects forward member and indexed operations"
                )));
        }
    }

    fn report_shared_handle_arity(&mut self, kind: SharedHandleKind, found: usize, span: Span) {
        let name = kind.source_name();
        self.diagnostics.push(
            Diagnostic::new(
                "E0546",
                format!("`{name}` Takes Exactly One Type Argument, But {found} Were Given"),
                span,
            )
            .with_help(format!("write `{name}<T>`")),
        );
    }

    fn collection_method_return_type(
        &mut self,
        object: &Expr,
        method: &str,
        null_safe: bool,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<TypeId> {
        let ty = self.infer_expr_type(object, scopes, method_context);
        let nullable_access = self
            .forwarded_access_payload(ty)
            .is_some_and(|(_, nullable)| nullable);
        let ty = self.forwarded_access_payload_type(ty);
        let void = self.types.intern(TypeKind::Void);
        let bool_ty = self.types.intern(TypeKind::Bool);
        let result = match (self.types.kind(ty).clone(), method) {
            (TypeKind::List(_), "add" | "insertAt")
            | (TypeKind::Dictionary(_, _) | TypeKind::SortedDictionary(_, _), "set")
            | (TypeKind::PriorityQueue(_), "push")
            | (TypeKind::Deque(_), "pushFront" | "pushBack")
            | (
                TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_),
                "clear",
            ) => Some(void),
            (TypeKind::List(value), "removeAt") => Some(value),
            (TypeKind::List(value), "pop") => Some(self.types.intern(TypeKind::Nullable(value))),
            (TypeKind::List(_), "indexOf") => {
                let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
                Some(self.types.intern(TypeKind::Nullable(int)))
            }
            (TypeKind::List(_), "remove") => Some(bool_ty),
            (TypeKind::List(_), "contains")
            | (TypeKind::TypedArray(_), "contains")
            | (TypeKind::PriorityQueue(_), "contains")
            | (TypeKind::Deque(_), "contains") => Some(bool_ty),
            (TypeKind::Dictionary(_, value) | TypeKind::SortedDictionary(_, value), "get") => {
                Some(self.types.intern(TypeKind::Nullable(value)))
            }
            (TypeKind::Dictionary(_, value) | TypeKind::SortedDictionary(_, value), "remove") => {
                Some(self.types.intern(TypeKind::Nullable(value)))
            }
            (TypeKind::Dictionary(_, _) | TypeKind::SortedDictionary(_, _), "containsKey")
            | (TypeKind::Dictionary(_, _) | TypeKind::SortedDictionary(_, _), "containsValue")
            | (TypeKind::Set(_) | TypeKind::SortedSet(_), "add" | "remove" | "contains") => {
                Some(bool_ty)
            }
            (TypeKind::Set(value), "union" | "intersect" | "difference") => {
                Some(self.types.intern(TypeKind::Set(value)))
            }
            (TypeKind::SortedSet(value), "union" | "intersect" | "difference") => {
                Some(self.types.intern(TypeKind::SortedSet(value)))
            }
            (TypeKind::PriorityQueue(value), "pop")
            | (TypeKind::Deque(value), "popFront" | "popBack") => {
                Some(self.types.intern(TypeKind::Nullable(value)))
            }
            (TypeKind::Bytes, "toArray") => {
                let byte = self.types.intern(TypeKind::Integer(IntegerType::UInt8));
                Some(self.types.intern(TypeKind::TypedArray(byte)))
            }
            (
                TypeKind::Bytes
                | TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_),
                _,
            ) => Some(self.types.unknown()),
            _ => None,
        }?;
        Some(self.null_safe_result_type(result, null_safe && nullable_access))
    }

    #[allow(clippy::too_many_arguments)]
    fn check_enum_property_method_call(
        &mut self,
        object: &Expr,
        method: &str,
        member_span: Span,
        argument_list_span: Span,
        args: &[Argument],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> bool {
        let ty = self.infer_expr_type(object, scopes, method_context);
        let ty = match self.types.kind(ty) {
            TypeKind::Nullable(inner) => *inner,
            _ => ty,
        };
        let TypeKind::Enum(enum_type) = self.types.kind(ty).clone() else {
            return false;
        };
        let definition = self
            .enums
            .values()
            .find(|definition| definition.id == enum_type.id);
        let backed = definition.is_some_and(|definition| definition.backing_type.is_some());
        if method != "value" {
            let payload_field = definition.is_some_and(|definition| {
                definition
                    .cases
                    .iter()
                    .any(|case| case.payload.iter().any(|field| field.name == method))
            });
            if payload_field {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0577",
                        format!(
                            "payload field `${method}` on enum `{}` is observed through pattern matching",
                            enum_type.name
                        ),
                        member_span,
                    )
                    .with_title("Enum Payload Requires Pattern Matching")
                    .with_help(
                        "payload fields are not properties; observe them through an enum case pattern",
                    ),
                );
                return true;
            }
            self.diagnostics.push(
                Diagnostic::new(
                    "E0577",
                    format!("enum `{}` has no method `{method}`", enum_type.name),
                    member_span,
                )
                .with_title("Unknown Enum Member"),
            );
            return true;
        }
        if !backed {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0577",
                    format!("unit enum `{}` has no `value` property", enum_type.name),
                    member_span,
                )
                .with_title("Unit Enum Has No Value Property"),
            );
            return true;
        }
        let mut diagnostic = Diagnostic::new(
            "E0575",
            "`value` is a readonly enum property, not a method",
            member_span,
        )
        .with_title("Property Invoked As Method")
        .with_help("read `->value` without parentheses");
        if args.is_empty() {
            diagnostic = diagnostic.with_fix(argument_list_span, "");
        }
        self.diagnostics.push(diagnostic);
        true
    }

    fn collection_receiver(kind: &TypeKind) -> Option<CollectionReceiver> {
        match kind {
            TypeKind::TypedArray(_) => Some(CollectionReceiver::TypedArray),
            TypeKind::List(_) => Some(CollectionReceiver::List),
            TypeKind::Dictionary(_, _) => Some(CollectionReceiver::Dictionary),
            TypeKind::Set(_) => Some(CollectionReceiver::Set),
            TypeKind::SortedDictionary(_, _) => Some(CollectionReceiver::SortedDictionary),
            TypeKind::SortedSet(_) => Some(CollectionReceiver::SortedSet),
            TypeKind::PriorityQueue(_) => Some(CollectionReceiver::PriorityQueue),
            TypeKind::Deque(_) => Some(CollectionReceiver::Deque),
            _ => None,
        }
    }

    fn report_unknown_collection_member(
        &mut self,
        receiver_ty: TypeId,
        receiver: CollectionReceiver,
        written: &str,
        member_span: Span,
        written_argument_count: Option<usize>,
        suggestion: &collection_diagnostics::CollectionMemberSuggestion,
    ) {
        let mut diagnostic = Diagnostic::new(
            "E0521",
            format!(
                "unknown collection member `{written}` on `{}`; did you mean `{}`?",
                self.types.display(receiver_ty),
                suggestion.canonical
            ),
            member_span,
        )
        .with_title("Unknown Collection Member")
        .with_primary_label("This Member Name Is Not Available On The Receiver")
        .with_explanation(format!(
            "`{}` uses `{}` for this operation under {}'s collection naming rules.",
            receiver.source_name(),
            suggestion.canonical,
            suggestion.decision_owner
        ))
        .with_help(format!(
            "replace `{written}` with `{}`",
            suggestion.canonical
        ));

        let argument_shape_matches = match (suggestion.arguments, written_argument_count) {
            (ArgumentShape::Property, None) => true,
            (ArgumentShape::Exact(expected), Some(found)) => expected == found,
            _ => false,
        };
        let applicability = if argument_shape_matches {
            suggestion.applicability
        } else {
            FixApplicability::RequiresReview
        };
        diagnostic = diagnostic.with_structured_fix(
            format!("Rename `{written}` To `{}`", suggestion.canonical),
            applicability,
            vec![FixEdit {
                source: DiagnosticSource::Current,
                span: member_span,
                replacement: suggestion.canonical.to_string(),
            }],
        );
        self.diagnostics.push(diagnostic);
    }

    #[allow(clippy::too_many_arguments)]
    fn report_property_invoked_as_method(
        &mut self,
        receiver_ty: TypeId,
        receiver: CollectionReceiver,
        written: &str,
        member_span: Span,
        argument_list_span: Span,
        args: &[Argument],
        suggestion: Option<&collection_diagnostics::CollectionMemberSuggestion>,
        status: ImplementationStatus,
    ) {
        let canonical = suggestion.map_or(written, |entry| entry.canonical);
        let mut diagnostic = Diagnostic::new(
            "E0557",
            format!(
                "`{canonical}` is a property on `{}`, not a method",
                self.types.display(receiver_ty)
            ),
            member_span,
        )
        .with_title("Property Is Not A Method")
        .with_primary_label("This Collection State Is Read As A Property")
        .with_explanation(format!(
            "`{canonical}` represents collection state on `{}` and is read without parentheses.",
            receiver.source_name()
        ));

        if args.is_empty() {
            let mut edits = Vec::with_capacity(2);
            if canonical != written {
                edits.push(FixEdit {
                    source: DiagnosticSource::Current,
                    span: member_span,
                    replacement: canonical.to_string(),
                });
            }
            edits.push(FixEdit {
                source: DiagnosticSource::Current,
                span: argument_list_span,
                replacement: String::new(),
            });

            let projection_requires_context = matches!(canonical, "keys" | "values");
            let applicability = if status == ImplementationStatus::Executable
                && !projection_requires_context
                && suggestion
                    .is_none_or(|entry| entry.applicability == FixApplicability::MachineApplicable)
            {
                FixApplicability::MachineApplicable
            } else {
                FixApplicability::RequiresReview
            };
            diagnostic = diagnostic
                .with_help(format!("read the property as `->{canonical}`"))
                .with_structured_fix(
                    if canonical == written {
                        format!("Remove Parentheses From `{canonical}`")
                    } else {
                        format!("Use The `{canonical}` Property")
                    },
                    applicability,
                    edits,
                );
        } else {
            diagnostic = diagnostic.with_help(format!(
                "read `->{canonical}` without arguments; the supplied arguments cannot be removed automatically"
            ));
        }
        self.diagnostics.push(diagnostic);
    }

    fn source_slice(&self, span: Span) -> Option<&str> {
        self.source_texts
            .get(&span.source)?
            .get(span.start..span.end)
    }

    fn check_compiler_known_property(
        &mut self,
        object: &Expr,
        property: &str,
        member_span: Span,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(object, scopes, method_context);
        let ty = self.forwarded_access_payload_type(ty);
        let ty = match self.types.kind(ty) {
            TypeKind::Nullable(inner) => *inner,
            _ => ty,
        };
        if let TypeKind::Enum(enum_type) = self.types.kind(ty).clone() {
            let definition = self
                .enums
                .values()
                .find(|definition| definition.id == enum_type.id);
            if property == "value" && definition.is_some_and(|value| value.backing_type.is_some()) {
                return;
            }
            let payload_field = definition.is_some_and(|definition| {
                definition
                    .cases
                    .iter()
                    .any(|case| case.payload.iter().any(|field| field.name == property))
            });
            if payload_field {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0577",
                        format!(
                            "payload field `${property}` on enum `{}` is observed through pattern matching",
                            enum_type.name
                        ),
                        member_span,
                    )
                    .with_title("Enum Payload Requires Pattern Matching")
                    .with_help(
                        "payload fields are not properties; observe them through an enum case pattern",
                    ),
                );
                return;
            }
            let message = if property == "value" {
                format!("unit enum `{}` has no `value` property", enum_type.name)
            } else {
                format!("enum `{}` has no property `{property}`", enum_type.name)
            };
            self.diagnostics.push(
                Diagnostic::new("E0577", message, member_span)
                    .with_title("Unit Enum Has No Value Property"),
            );
            return;
        }
        if matches!(self.types.kind(ty), TypeKind::String) {
            match property {
                "length" | "byteLength" | "isEmpty" | "bytes" => {}
                "graphemes" | "codePoints" => self.diagnostics.push(
                    Diagnostic::unsupported_stage(
                        "E0304",
                        format!(
                            "String property `{property}` requires the future public iteration protocol"
                        ),
                        span,
                    )
                    .with_help(
                        "use the executable `length`, `byteLength`, `isEmpty`, or `bytes` property",
                    ),
                ),
                _ => self.diagnostics.push(
                    Diagnostic::new(
                        "E0306",
                        format!("unknown String property `{property}`"),
                        span,
                    )
                    .with_help(
                        "String intrinsic properties are `length`, `byteLength`, `isEmpty`, and `bytes`",
                    ),
                ),
            }
            return;
        }
        if matches!(self.types.kind(ty), TypeKind::Error) {
            if property != "message" {
                self.diagnostics.push(Diagnostic::new(
                    "E0306",
                    format!("unknown Error property `{property}`"),
                    member_span,
                ));
            }
            return;
        }
        let receiver = Self::collection_receiver(self.types.kind(ty));
        if let Some(receiver) = receiver {
            if let Some(suggestion) = collection_diagnostics::suggestion_for(receiver, property) {
                self.report_unknown_collection_member(
                    ty,
                    receiver,
                    property,
                    member_span,
                    None,
                    suggestion,
                );
                return;
            }
        }
        let supported = matches!(
            (self.types.kind(ty), property),
            (TypeKind::TypedArray(_), "length")
                | (TypeKind::Bytes, "length")
                | (
                    TypeKind::List(_)
                        | TypeKind::Dictionary(_, _)
                        | TypeKind::SortedDictionary(_, _)
                        | TypeKind::Set(_)
                        | TypeKind::SortedSet(_)
                        | TypeKind::PriorityQueue(_)
                        | TypeKind::Deque(_),
                    "count"
                )
                | (
                    TypeKind::List(_)
                        | TypeKind::Dictionary(_, _)
                        | TypeKind::SortedDictionary(_, _)
                        | TypeKind::Set(_)
                        | TypeKind::SortedSet(_)
                        | TypeKind::PriorityQueue(_)
                        | TypeKind::Deque(_),
                    "isEmpty"
                )
                | (
                    TypeKind::List(_) | TypeKind::Set(_) | TypeKind::SortedSet(_),
                    "first" | "last"
                )
                | (TypeKind::PriorityQueue(_), "peek")
                | (TypeKind::Deque(_), "peekFront" | "peekBack")
        );
        if matches!(
            (self.types.kind(ty), property),
            (
                TypeKind::Dictionary(_, _) | TypeKind::SortedDictionary(_, _),
                "keys" | "values",
            )
        ) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0522",
                    format!(
                        "Dictionary::{property} is a foreach-only projection and cannot be stored or used as a value"
                    ),
                    span,
                )
                .with_help(format!(
                    "iterate it with `foreach ($dictionary->{property} as $value)` or build an owned copy explicitly"
                )),
            );
            return;
        }
        if !supported {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0521",
                    format!(
                        "unknown collection property `{property}` on `{}`; it is not part of the surface settled by Decision 0113",
                        self.types.display(ty),
                    ),
                    member_span,
                )
                .with_title("Unknown Collection Member")
                .with_primary_label("This Property Is Not Available On The Receiver")
                .with_explanation(
                    "Collection members are resolved from the receiver type, so names from another collection family do not apply here.",
                )
                .with_help("use a member documented for this collection type"),
            );
        }
    }

    fn check_collection_method_call(&mut self, call: CollectionMethodCall<'_>) -> bool {
        let CollectionMethodCall {
            object,
            method,
            args,
            member_span,
            argument_list_span,
            span,
            scopes,
            method_context,
        } = call;
        let ty = self.infer_expr_type(object, scopes, method_context);
        let ty = self.forwarded_access_payload_type(ty);
        let kind = self.types.kind(ty).clone();
        let is_collection = matches!(
            kind,
            TypeKind::Bytes
                | TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_)
        );
        if !is_collection {
            return false;
        }
        let receiver = Self::collection_receiver(&kind);
        if let Some(receiver) = receiver {
            if let Some(status) =
                collection_diagnostics::canonical_property_status(receiver, method)
            {
                self.report_property_invoked_as_method(
                    ty,
                    receiver,
                    method,
                    member_span,
                    argument_list_span,
                    args,
                    None,
                    status,
                );
                return true;
            }
            if let Some(suggestion) = collection_diagnostics::suggestion_for(receiver, method) {
                if suggestion.member_kind == CollectionMemberKind::Property {
                    self.report_property_invoked_as_method(
                        ty,
                        receiver,
                        method,
                        member_span,
                        argument_list_span,
                        args,
                        Some(suggestion),
                        suggestion.implementation,
                    );
                } else {
                    self.report_unknown_collection_member(
                        ty,
                        receiver,
                        method,
                        member_span,
                        Some(args.len()),
                        suggestion,
                    );
                }
                return true;
            }
        }
        if self.reject_named_arguments(&format!("collection method `{method}`"), args) {
            return true;
        }

        if matches!(kind, TypeKind::List(_)) && matches!(method, "map" | "filter" | "reduce") {
            self.check_list_algorithm_call(
                ty,
                &kind,
                method,
                args,
                object,
                span,
                scopes,
                method_context,
            );
            return true;
        }

        let int = self.types.intern(TypeKind::Integer(IntegerType::Int64));
        let compared_type = match (&kind, method) {
            (
                TypeKind::List(value)
                | TypeKind::TypedArray(value)
                | TypeKind::Set(value)
                | TypeKind::SortedSet(value)
                | TypeKind::PriorityQueue(value)
                | TypeKind::Deque(value),
                "contains",
            ) => Some(*value),
            (TypeKind::Dictionary(key, _) | TypeKind::SortedDictionary(key, _), "containsKey") => {
                Some(*key)
            }
            (TypeKind::List(value), "indexOf" | "remove")
            | (
                TypeKind::Dictionary(_, value) | TypeKind::SortedDictionary(_, value),
                "containsValue",
            ) => Some(*value),
            _ => None,
        };
        if let Some(compared_type) = compared_type {
            let receiver = receiver.expect("collection equality receiver");
            self.check_stage23_equatable_type(
                compared_type,
                span,
                &format!("{}::{method}", receiver.source_name()),
            );
        }
        let (expected, mutating): (Vec<TypeId>, bool) = match (kind, method) {
            (TypeKind::List(value), "add") => (vec![value], true),
            (TypeKind::List(value), "insertAt") => (vec![int, value], true),
            (TypeKind::List(_), "removeAt") => (vec![int], true),
            (TypeKind::List(_), "pop") => (vec![], true),
            (TypeKind::List(value), "contains") => (vec![value], false),
            (TypeKind::List(value), "indexOf") => (vec![value], false),
            (TypeKind::List(value), "remove") => (vec![value], true),
            (TypeKind::TypedArray(value), "contains") => (vec![value], false),
            (TypeKind::PriorityQueue(value), "contains") => (vec![value], false),
            (TypeKind::Deque(value), "contains") => (vec![value], false),
            (TypeKind::Dictionary(key, value), "set") => (vec![key, value], true),
            (TypeKind::Dictionary(key, _), "get" | "containsKey") => (vec![key], false),
            (TypeKind::Dictionary(_, value), "containsValue") => (vec![value], false),
            (TypeKind::Dictionary(key, _), "remove") => (vec![key], true),
            (TypeKind::SortedDictionary(key, value), "set") => (vec![key, value], true),
            (TypeKind::SortedDictionary(key, _), "get" | "containsKey") => (vec![key], false),
            (TypeKind::SortedDictionary(_, value), "containsValue") => (vec![value], false),
            (TypeKind::SortedDictionary(key, _), "remove") => (vec![key], true),
            (TypeKind::Set(value), "add" | "remove") => (vec![value], true),
            (TypeKind::Set(value), "contains") => (vec![value], false),
            (TypeKind::Set(value), "union" | "intersect" | "difference") => {
                self.check_non_consuming_collection_duplication(
                    "Set",
                    &format!("Set::{method}"),
                    value,
                    None,
                    span,
                );
                let set = self.types.intern(TypeKind::Set(value));
                (vec![set], false)
            }
            (TypeKind::SortedSet(value), "add" | "remove") => (vec![value], true),
            (TypeKind::SortedSet(value), "contains") => (vec![value], false),
            (TypeKind::SortedSet(value), "union" | "intersect" | "difference") => {
                self.check_non_consuming_collection_duplication(
                    "SortedSet",
                    &format!("SortedSet::{method}"),
                    value,
                    None,
                    span,
                );
                let set = self.types.intern(TypeKind::SortedSet(value));
                (vec![set], false)
            }
            (TypeKind::PriorityQueue(value), "push") => (vec![value], true),
            (TypeKind::PriorityQueue(_), "pop") => (vec![], true),
            (TypeKind::Deque(value), "pushFront" | "pushBack") => (vec![value], true),
            (TypeKind::Deque(_), "popFront" | "popBack") => (vec![], true),
            (
                TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_),
                "clear",
            ) => (vec![], true),
            (TypeKind::Bytes, "toArray") => (vec![], false),
            (TypeKind::Bytes, _) => {
                self.diagnostics.push(Diagnostic::unsupported_stage(
                    "E0524",
                    format!(
                        "Bytes method `{method}` is deferred to the future Bytes method-surface record"
                    ),
                    span,
                ));
                return true;
            }
            _ => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0521",
                        format!(
                            "unknown collection method `{method}` on `{}`; it is not part of the surface settled by Decision 0113",
                            self.types.display(ty),
                        ),
                        member_span,
                    )
                    .with_title("Unknown Collection Member")
                    .with_primary_label("This Method Is Not Available On The Receiver")
                    .with_explanation(
                        "Collection members are resolved from the receiver type, so names from another collection family do not apply here.",
                    )
                    .with_help("use a member documented for this collection type"),
                );
                return true;
            }
        };
        if args.len() != expected.len() {
            self.report_argument_count_mismatch(
                &format!("collection method `{method}`"),
                expected.len(),
                expected.len(),
                args.len(),
                span,
            );
            return true;
        }
        for (argument, expected) in args.iter().zip(expected) {
            self.check_expr_assignable(
                expected,
                &argument.value,
                scopes,
                method_context,
                AssignmentDestination::Type,
            );
        }
        if mutating {
            self.record_capture_requirement_for_expr(object, scopes, CaptureRequirement::Writable);
            if !self.is_writable_object_path(object, scopes, method_context) {
                self.diagnostics.push(Diagnostic::new(
                    "E0201",
                    format!(
                        "cannot call mutating collection method `{method}` through a readonly value"
                    ),
                    object.span(),
                ));
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn check_list_algorithm_call(
        &mut self,
        receiver_ty: TypeId,
        receiver_kind: &TypeKind,
        method: &str,
        args: &[Argument],
        receiver: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let TypeKind::List(element) = receiver_kind else {
            unreachable!("List algorithm recognition requires a List receiver");
        };
        let expected_count = if method == "reduce" { 2 } else { 1 };
        if args.len() != expected_count {
            self.report_argument_count_mismatch(
                &format!("List::{method}"),
                expected_count,
                expected_count,
                args.len(),
                span,
            );
            return;
        }

        let callback_index = usize::from(method == "reduce");
        let callback = &args[callback_index].value;
        let callback_ty = self.infer_expr_type(callback, scopes, method_context);
        let TypeKind::Function(function) = self.types.kind(callback_ty).clone() else {
            if !matches!(self.types.kind(callback_ty), TypeKind::Unknown) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0665",
                        format!(
                            "List::{method} expects a function value, got `{}`",
                            self.types.display(callback_ty)
                        ),
                        callback.span(),
                    )
                    .with_title("List Algorithm Callback Type Mismatch"),
                );
            }
            return;
        };

        let callback_access = match function.invocation_mode {
            FunctionInvocationMode::Readonly => ListCallbackAccess::Readonly,
            FunctionInvocationMode::Writable => {
                self.record_capture_requirement_for_expr(
                    callback,
                    scopes,
                    CaptureRequirement::Writable,
                );
                if !self.callable_access_is_sufficient(
                    callback,
                    FunctionInvocationMode::Writable,
                    scopes,
                    method_context,
                ) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0668",
                            format!(
                                "writable callback passed to List::{method} requires writable access"
                            ),
                            callback.span(),
                        )
                        .with_title("List Algorithm Callback Requires Writable Access")
                        .with_help("store the callback in a writable binding before this call"),
                    );
                    return;
                }
                ListCallbackAccess::Writable
            }
            FunctionInvocationMode::Once => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0664",
                        format!(
                            "List::{method} requires a repeatable callback, but this callback is `once`"
                        ),
                        callback.span(),
                    )
                    .with_title("List Algorithm Requires A Repeatable Callback")
                    .with_explanation(
                        "The algorithm may invoke its callback once for every element in the List.",
                    ),
                );
                return;
            }
        };
        if callback_access == ListCallbackAccess::Readonly {
            self.record_capture_requirement_for_expr(
                callback,
                scopes,
                CaptureRequirement::Readonly,
            );
        }

        let void = self.types.intern(TypeKind::Void);
        let bool_ty = self.types.intern(TypeKind::Bool);
        let (kind, result, accumulator) = match method {
            "map" => {
                let [parameter] = function.parameters.as_slice() else {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Arity Mismatch",
                        "exactly one element parameter",
                    );
                    return;
                };
                if parameter.ownership_mode != FunctionTypeParameterMode::Readonly {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Parameter Ownership Mismatch",
                        "a readonly element parameter",
                    );
                    return;
                }
                if parameter.ty != *element {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Parameter Type Mismatch",
                        &format!(
                            "an element parameter of type `{}`",
                            self.types.display(*element)
                        ),
                    );
                    return;
                }
                if function.return_type == void {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0665",
                            "List::map callback must return a value",
                            callback.span(),
                        )
                        .with_title("List Map Callback Cannot Return Void"),
                    );
                    return;
                }
                if self.type_is_move_type(function.return_type) && function.return_borrow.is_some()
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0667",
                            "List::map cannot store a result borrowed from its callback input",
                            callback.span(),
                        )
                        .with_title("List Map Result Must Be Owned")
                        .with_explanation("List::map stores owned result elements in a new List."),
                    );
                    return;
                }
                (
                    ListAlgorithmKind::Map,
                    self.types.intern(TypeKind::List(function.return_type)),
                    None,
                )
            }
            "filter" => {
                let [parameter] = function.parameters.as_slice() else {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Arity Mismatch",
                        "exactly one element parameter",
                    );
                    return;
                };
                if parameter.ownership_mode != FunctionTypeParameterMode::Readonly {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Parameter Ownership Mismatch",
                        "a readonly element parameter",
                    );
                    return;
                }
                if parameter.ty != *element {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Parameter Type Mismatch",
                        &format!(
                            "an element parameter of type `{}`",
                            self.types.display(*element)
                        ),
                    );
                    return;
                }
                if function.return_type != bool_ty {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Return Type Mismatch",
                        "a `bool` return",
                    );
                    return;
                }
                if self.type_is_move_type(*element) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0666",
                            format!(
                                "List::filter cannot preserve source elements of Move type `{}`",
                                self.types.display(*element)
                            ),
                            span,
                        )
                        .with_title("List Filter Requires Copy Elements")
                        .with_explanation(
                            "The preserving filter leaves the source unchanged and copies selected elements into a new List.",
                        )
                        .with_help("Move-element preserving filter waits for `Cloneable`"),
                    );
                    return;
                }
                (ListAlgorithmKind::Filter, receiver_ty, None)
            }
            "reduce" => {
                let [accumulator_parameter, element_parameter] = function.parameters.as_slice()
                else {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Arity Mismatch",
                        "exactly two parameters",
                    );
                    return;
                };
                if accumulator_parameter.ownership_mode != FunctionTypeParameterMode::Writable {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Reduce Accumulator Must Be Writable",
                        "a writable accumulator parameter first",
                    );
                    return;
                }
                if element_parameter.ownership_mode != FunctionTypeParameterMode::Readonly {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Parameter Ownership Mismatch",
                        "a readonly element parameter second",
                    );
                    return;
                }
                if element_parameter.ty != *element {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Callback Parameter Type Mismatch",
                        &format!(
                            "a second parameter of type `{}`",
                            self.types.display(*element)
                        ),
                    );
                    return;
                }
                if function.return_type != void {
                    self.report_list_callback_mismatch(
                        method,
                        callback.span(),
                        "Reduce Callback Must Return Void",
                        "a `void` return",
                    );
                    return;
                }
                let accumulator = accumulator_parameter.ty;
                let initial = &args[0].value;
                if !self.check_expr_assignable(
                    accumulator,
                    initial,
                    scopes,
                    method_context,
                    AssignmentDestination::Type,
                ) {
                    return;
                }
                (ListAlgorithmKind::Reduce, accumulator, Some(accumulator))
            }
            _ => unreachable!("recognized List algorithm"),
        };

        let complete_effects =
            self.complete_function_value_effects(&function.checked_effects, span);
        self.record_checked_effects(complete_effects.iter().copied(), span);
        let checked_effects = complete_effects
            .iter()
            .map(|effect| self.types.resolved(*effect))
            .collect::<Vec<_>>();
        let effect_profile =
            crate::checked_effects::CheckedEffectProfile::classify(checked_effects.clone());
        self.list_algorithm_calls.insert(
            span,
            ListAlgorithmCallInfo {
                kind,
                receiver_type: self.types.resolved(receiver_ty),
                element_type: self.types.resolved(*element),
                result_type: self.types.resolved(result),
                accumulator_type: accumulator.map(|ty| self.types.resolved(ty)),
                callback_type: self.types.resolved(callback_ty),
                callback_access,
                checked_effects,
                required_checked_effects: effect_profile.required,
                ambient_checked_effects: effect_profile.ambient,
                source_span: span,
                receiver_span: receiver.span(),
                callback_span: callback.span(),
            },
        );
    }

    fn report_list_callback_mismatch(
        &mut self,
        method: &str,
        span: Span,
        axis: &str,
        expected: &str,
    ) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0665",
                format!("List::{method} callback must have {expected}"),
                span,
            )
            .with_title(format!("List Algorithm {axis}")),
        );
    }

    fn check_stage23_equatable_type(&mut self, ty: TypeId, span: Span, operation: &str) {
        match self.types.kind(ty) {
            TypeKind::Nullable(inner) => self.check_stage23_equatable_type(*inner, span, operation),
            TypeKind::Integer(_)
            | TypeKind::Float(_)
            | TypeKind::String
            | TypeKind::Bool
            | TypeKind::Enum(_)
            | TypeKind::Unknown => {}
            TypeKind::Class(_) => self.diagnostics.push(
                Diagnostic::unsupported_stage(
                    "E0524",
                    format!(
                        "{operation} cannot yet compare user-defined values of type `{}`",
                        self.types.display(ty)
                    ),
                    span,
                )
                .with_title("Collection Value Cannot Be Compared")
                .with_explanation(
                    "The selected collection operation compares values for equality, but user-defined equality is not executable in the current language stage.",
                ),
            ),
            _ => self.diagnostics.push(
                Diagnostic::new(
                    "E0524",
                    format!(
                        "{operation} cannot compare values of type `{}`",
                        self.types.display(ty)
                    ),
                    span,
                )
                .with_title("Collection Value Cannot Be Compared")
                .with_explanation(
                    "The selected collection operation compares values for equality, and this value type does not support that comparison in the current language.",
                )
                .with_help("use a value type with defined equality for this collection operation"),
            ),
        }
    }

    fn static_qualifier_class_name(
        qualifier: &StaticQualifier,
        method_context: Option<&MethodContext>,
    ) -> Option<String> {
        match qualifier {
            StaticQualifier::Class(name) => Some(name.clone()),
            StaticQualifier::SelfType => method_context.map(|context| context.class_name.clone()),
            StaticQualifier::Parent | StaticQualifier::InvalidStatic => None,
        }
    }

    fn infer_unary_type(
        &mut self,
        op: &UnaryOp,
        expr: &Expr,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        match op {
            UnaryOp::Not => match self.types.kind(ty) {
                TypeKind::Bool => self.types.intern(TypeKind::Bool),
                TypeKind::Unknown => self.types.unknown(),
                _ => self.types.intern(TypeKind::Heterogeneous),
            },
            UnaryOp::Negate => {
                if Self::integer_literal_parts(expr).is_some() {
                    self.check_contextual_integer_literal(
                        &Expr::Unary {
                            op: UnaryOp::Negate,
                            expr: Box::new(expr.clone()),
                            span,
                        },
                        IntegerType::Int64,
                    );
                }
                match self.types.kind(ty) {
                    TypeKind::Integer(integer) if integer.is_signed() => {
                        self.types.intern(TypeKind::Integer(*integer))
                    }
                    TypeKind::Float(float) => self.types.intern(TypeKind::Float(*float)),
                    TypeKind::Unknown => self.types.unknown(),
                    _ => self.types.intern(TypeKind::Heterogeneous),
                }
            }
            UnaryOp::BitwiseNot => match self.types.kind(ty) {
                TypeKind::Integer(integer) => self.types.intern(TypeKind::Integer(*integer)),
                TypeKind::Unknown => self.types.unknown(),
                _ => self.types.intern(TypeKind::Heterogeneous),
            },
        }
    }

    fn infer_binary_type(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        if *op == BinaryOp::Coalesce {
            let left_ty = self.infer_expr_type(left, scopes, method_context);
            if let TypeKind::Nullable(inner) = *self.types.kind(left_ty) {
                self.contextualize_scalar_literals(inner, right);
            }
            let right_ty = self.infer_expr_type(right, scopes, method_context);
            return self.infer_coalesce_binary_type(left_ty, right_ty);
        }

        let (left_ty, right_ty) =
            self.infer_contextual_binary_operand_types(left, right, scopes, method_context);

        match op {
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::ShiftLeft
            | BinaryOp::ShiftRight
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseOr => self.infer_numeric_binary_type(left_ty, right_ty),
            BinaryOp::Concat => self.infer_concat_binary_type(left_ty, right_ty),
            BinaryOp::Equal | BinaryOp::NotEqual => self.types.intern(TypeKind::Bool),
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                self.infer_relational_binary_type(left_ty, right_ty)
            }
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                self.infer_logical_binary_type(left_ty, right_ty)
            }
            BinaryOp::Coalesce => unreachable!("coalesce is handled before numeric context"),
        }
    }

    fn infer_contextual_binary_operand_types(
        &mut self,
        left: &Expr,
        right: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> (TypeId, TypeId) {
        let mut left_ty = self.infer_expr_type(left, scopes, method_context);
        let mut right_ty = self.infer_expr_type(right, scopes, method_context);

        let left_literal = Self::integer_literal_parts(left).is_some();
        let right_literal = Self::integer_literal_parts(right).is_some();
        let left_integer = match self.types.kind(left_ty) {
            TypeKind::Integer(integer) => Some(*integer),
            _ => None,
        };
        let right_integer = match self.types.kind(right_ty) {
            TypeKind::Integer(integer) => Some(*integer),
            _ => None,
        };

        if left_literal && !right_literal {
            if let Some(integer) = right_integer {
                self.check_contextual_integer_literal(left, integer);
                left_ty = self.types.intern(TypeKind::Integer(integer));
            }
        } else if right_literal && !left_literal {
            if let Some(integer) = left_integer {
                self.check_contextual_integer_literal(right, integer);
                right_ty = self.types.intern(TypeKind::Integer(integer));
            }
        } else if left_literal && right_literal {
            let integer = match (left_integer, right_integer) {
                (Some(left), Some(right)) if left == right => left,
                _ => IntegerType::Int64,
            };
            self.check_contextual_integer_literal(left, integer);
            self.check_contextual_integer_literal(right, integer);
            left_ty = self.types.intern(TypeKind::Integer(integer));
            right_ty = self.types.intern(TypeKind::Integer(integer));
        }

        let left_float_literal = Self::is_float_literal(left);
        let right_float_literal = Self::is_float_literal(right);
        let left_float = match self.types.kind(left_ty) {
            TypeKind::Float(float) => Some(*float),
            _ => None,
        };
        let right_float = match self.types.kind(right_ty) {
            TypeKind::Float(float) => Some(*float),
            _ => None,
        };

        if left_float_literal && !right_float_literal {
            if let Some(float) = right_float {
                self.record_float_expression_type(left, float);
                left_ty = self.types.intern(TypeKind::Float(float));
            }
        } else if right_float_literal && !left_float_literal {
            if let Some(float) = left_float {
                self.record_float_expression_type(right, float);
                right_ty = self.types.intern(TypeKind::Float(float));
            }
        }

        (left_ty, right_ty)
    }

    fn infer_numeric_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right => {
                self.types.intern(TypeKind::Integer(left))
            }
            (TypeKind::Float(left), TypeKind::Float(right)) if left == right => {
                self.types.intern(TypeKind::Float(left))
            }
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_concat_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_is_string = matches!(self.types.kind(left), TypeKind::String);
        let right_is_string = matches!(self.types.kind(right), TypeKind::String);
        if (left_is_string && self.is_display_convertible_type(right))
            || (right_is_string && self.is_display_convertible_type(left))
        {
            self.types.intern(TypeKind::String)
        } else {
            self.types.intern(TypeKind::Heterogeneous)
        }
    }

    fn infer_logical_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Bool, TypeKind::Bool) => self.types.intern(TypeKind::Bool),
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_relational_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }
        if self.constrained_relational_operands(left, right) {
            return self.types.intern(TypeKind::Bool);
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Integer(left), TypeKind::Integer(right)) if left == right => {
                self.types.intern(TypeKind::Bool)
            }
            (TypeKind::Float(left), TypeKind::Float(right)) if left == right => {
                self.types.intern(TypeKind::Bool)
            }
            (TypeKind::String, TypeKind::String) => self.types.intern(TypeKind::Bool),
            (TypeKind::Bool, TypeKind::Bool) => self.types.intern(TypeKind::Bool),
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn infer_coalesce_binary_type(&mut self, left: TypeId, right: TypeId) -> TypeId {
        if let Some(recovery) = self.recovery_binary_type(left, right) {
            return recovery;
        }

        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();
        match (left_kind, right_kind) {
            (TypeKind::Nullable(inner), _) if self.is_assignable(inner, right) => inner,
            (TypeKind::Null, _) => right,
            (_, TypeKind::Null) => left,
            _ if left == right => left,
            _ => self.types.intern(TypeKind::Heterogeneous),
        }
    }

    fn recovery_binary_type(&mut self, left: TypeId, right: TypeId) -> Option<TypeId> {
        let left_kind = self.types.kind(left).clone();
        let right_kind = self.types.kind(right).clone();

        match (left_kind, right_kind) {
            (TypeKind::Unknown, _) | (_, TypeKind::Unknown) => Some(self.types.unknown()),
            _ => None,
        }
    }

    fn infer_array_type(
        &mut self,
        elements: &[ArrayElement],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        if elements.is_empty() {
            return self.types.intern(TypeKind::EmptyCollection);
        }

        if elements.iter().any(|element| element.key.is_some()) {
            if elements.iter().any(|element| element.key.is_none()) {
                return self.types.intern(TypeKind::Heterogeneous);
            }
            let explicit_keys = elements
                .iter()
                .filter_map(|element| element.key.as_ref())
                .collect::<Vec<_>>();
            let key_types =
                self.infer_collection_member_types(&explicit_keys, scopes, method_context);
            let values = elements
                .iter()
                .map(|element| &element.value)
                .collect::<Vec<_>>();
            let value_types = self.infer_collection_member_types(&values, scopes, method_context);
            let key = self.common_clear_type(key_types);
            let value = self.common_clear_type(value_types);
            self.check_stage23_hashable_type(
                key,
                explicit_keys
                    .first()
                    .map_or_else(|| elements[0].value.span(), |key| key.span()),
                "Dictionary key",
            );
            self.types.intern(TypeKind::Dictionary(key, value))
        } else {
            let values = elements
                .iter()
                .map(|element| &element.value)
                .collect::<Vec<_>>();
            let element_types = self.infer_collection_member_types(&values, scopes, method_context);
            let element = self.common_clear_type(element_types);
            self.types.intern(TypeKind::List(element))
        }
    }

    fn infer_collection_member_types(
        &mut self,
        expressions: &[&Expr],
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Vec<TypeId> {
        let mut contextual_integer = None;
        for expr in expressions {
            if Self::integer_literal_parts(expr).is_some() {
                continue;
            }
            let ty = self.infer_expr_type(expr, scopes, method_context);
            if let TypeKind::Integer(integer) = self.types.kind(ty) {
                match contextual_integer {
                    None => contextual_integer = Some(*integer),
                    Some(current) if current == *integer => {}
                    Some(_) => {
                        contextual_integer = None;
                        break;
                    }
                }
            }
        }

        if let Some(integer) = contextual_integer {
            for expr in expressions {
                self.contextualize_integer_literals(expr, integer);
            }
        }

        expressions
            .iter()
            .map(|expr| self.infer_expr_type(expr, scopes, method_context))
            .collect()
    }

    fn exact_type_test_can_match(&self, value: TypeId, tested: TypeId) -> bool {
        match self.types.kind(value).clone() {
            TypeKind::Mixed | TypeKind::Unknown => true,
            TypeKind::Nullable(inner) => {
                inner == tested || matches!(self.types.kind(inner), TypeKind::Mixed)
            }
            _ => value == tested,
        }
    }

    fn common_clear_type(&mut self, types: Vec<TypeId>) -> TypeId {
        let mut common = None;
        let mut saw_empty_collection = false;
        let mut saw_mixed = false;
        let mut saw_heterogeneous = false;

        for ty in types {
            let kind = self.types.kind(ty).clone();
            if self.type_contains_mixed(ty) {
                saw_mixed = true;
            }

            match kind {
                TypeKind::Unknown => {
                    continue;
                }
                TypeKind::EmptyCollection => {
                    saw_empty_collection = true;
                    continue;
                }
                _ => {
                    if let Some(common_ty) = common {
                        if common_ty != ty {
                            if self.type_contains_mixed(common_ty) || self.type_contains_mixed(ty) {
                                common = Some(self.merge_inferred_return_types(common_ty, ty));
                            } else {
                                saw_heterogeneous = true;
                            }
                        }
                    } else {
                        common = Some(ty);
                    }
                }
            }
        }

        if saw_heterogeneous {
            if saw_mixed {
                if let Some(common) = common {
                    if self.type_contains_mixed(common) {
                        return common;
                    }
                }
                return self.types.intern(TypeKind::Mixed);
            }
            return self.types.intern(TypeKind::Heterogeneous);
        }

        if let Some(common) = common {
            if saw_empty_collection && !self.is_collection_like_type(common) {
                if saw_mixed {
                    return self.types.intern(TypeKind::Mixed);
                }
                return self.types.intern(TypeKind::Heterogeneous);
            }
            common
        } else if saw_empty_collection {
            self.types.intern(TypeKind::EmptyCollection)
        } else {
            self.types.unknown()
        }
    }

    fn is_collection_like_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_)
                | TypeKind::EmptyCollection
        )
    }

    fn is_non_empty_collection_like_type(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::TypedArray(_)
                | TypeKind::List(_)
                | TypeKind::Dictionary(_, _)
                | TypeKind::SortedDictionary(_, _)
                | TypeKind::Set(_)
                | TypeKind::SortedSet(_)
                | TypeKind::PriorityQueue(_)
                | TypeKind::Deque(_)
        )
    }

    fn expr_class_type(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<ClassType<TypeId>> {
        let ty = self.infer_expr_type(expr, scopes, method_context);
        // Compiler-known place behavior (record 0106): a forwarding handle resolves
        // member access against its payload class. Deliberately closed to these
        // types — this is not a general proxy or dynamic-lookup mechanism.
        let handle = match *self.types.kind(ty) {
            TypeKind::SharedHandle(kind, payload) => Some((kind, payload)),
            TypeKind::Nullable(inner) => match *self.types.kind(inner) {
                TypeKind::SharedHandle(kind, payload) => Some((kind, payload)),
                _ => None,
            },
            _ => None,
        };
        if let Some((kind, payload)) = handle {
            return Self::shared_handle_forwards(kind)
                .then(|| self.class_type(payload))
                .flatten();
        }
        self.class_type(ty)
    }

    fn expr_class_name(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) -> Option<String> {
        self.expr_class_type(expr, scopes, method_context)
            .map(|class| class.name)
    }

    fn flow_narrowed_type(
        &mut self,
        declared_ty: TypeId,
        fallback_ty: TypeId,
        span: Span,
        method_context: Option<&MethodContext>,
    ) -> TypeId {
        match self.flow_facts.get(&span).cloned() {
            Some(crate::narrowing::Fact::NonNull) => match self.types.kind(declared_ty) {
                TypeKind::Nullable(inner) => *inner,
                _ => fallback_ty,
            },
            Some(crate::narrowing::Fact::Null) => {
                if matches!(self.types.kind(declared_ty), TypeKind::Nullable(_)) {
                    declared_ty
                } else {
                    self.types.intern(TypeKind::Null)
                }
            }
            Some(crate::narrowing::Fact::Exact(ty)) => {
                let tested = self.resolve_type_ref_with_class(
                    &ty,
                    span,
                    method_context.map(|context| context.class_name.as_str()),
                );
                if self.exact_type_test_can_match(declared_ty, tested) {
                    tested
                } else {
                    fallback_ty
                }
            }
            None => fallback_ty,
        }
    }

    fn check_nullable_member_access(
        &mut self,
        object: &Expr,
        null_safe: bool,
        operation: &'static str,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let ty = self.infer_expr_type(object, scopes, method_context);
        match self.types.kind(ty) {
            TypeKind::Nullable(inner) => {
                let member_receiver = matches!(
                    self.types.kind(*inner),
                    TypeKind::Class(_) | TypeKind::SharedHandle(_, _)
                ) || (operation == "property access"
                    && matches!(self.types.kind(*inner), TypeKind::Enum(_)));
                if !member_receiver {
                    self.diagnostics.push(Diagnostic::new(
                        "E0507",
                        format!("{operation} requires a class value"),
                        object.span(),
                    ));
                } else if !null_safe {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0506",
                            format!("cannot use {operation} on a possibly-null value"),
                            object.span(),
                        )
                        .with_help("narrow it with a null check or use `?->`"),
                    );
                }
            }
            TypeKind::Unknown | TypeKind::Mixed => {}
            _ if null_safe => self.diagnostics.push(
                Diagnostic::new(
                    "E0507",
                    "null-safe access requires a nullable class or shared-reference receiver",
                    object.span(),
                )
                .with_help("use `->` after the value has been narrowed to a non-null class"),
            ),
            _ => {}
        }
    }

    fn check_is_type(
        &mut self,
        value: &Expr,
        ty: &TypeRef,
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if ty.name == "Displayable"
            || self
                .program
                .items
                .iter()
                .any(|item| matches!(item, Item::Interface(interface) if interface.name == ty.name))
        {
            self.diagnostics.push(Diagnostic::unsupported_stage(
                "E0510",
                "interface `is` tests are accepted syntax; interface conformance tests land in Stage 35",
                span,
            ));
            return;
        }

        if self.program.items.iter().any(|item| {
            matches!(
                item,
                Item::Class(class)
                    if (class.name == ty.name && class.parent.is_some())
                        || class.parent.as_deref() == Some(ty.name.as_str())
            )
        }) {
            self.diagnostics.push(Diagnostic::unsupported_stage(
                "E0509",
                "class-hierarchy `is` tests are accepted syntax; subtype tests land in Stage 34",
                span,
            ));
            return;
        }

        let tested = self.resolve_type_ref_with_class(
            ty,
            span,
            method_context.map(|context| context.class_name.as_str()),
        );
        self.type_test_types
            .insert(span, self.types.resolved(tested));
        if ty.nullable
            || !matches!(
                self.types.kind(tested),
                TypeKind::Integer(_)
                    | TypeKind::Float(_)
                    | TypeKind::String
                    | TypeKind::Bool
                    | TypeKind::Enum(_)
                    | TypeKind::Class(_)
                    | TypeKind::Function(_)
                    | TypeKind::Unknown
            )
        {
            self.diagnostics.push(Diagnostic::new(
                "E0508",
                format!("`{ty}` is not a concrete type that can be tested with `is`"),
                span,
            ));
        }

        let value_ty = self.infer_expr_type(value, scopes, method_context);
        if !matches!(
            self.types.kind(value_ty),
            TypeKind::Mixed | TypeKind::Nullable(_)
        ) && !self.is_unknown_type(value_ty)
        {
            // Exact tests over already-concrete values are valid. An always-true lint is a
            // separate diagnostics-quality follow-up and does not affect semantics.
        }
    }

    fn undeclared_variable(&mut self, name: &str, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "E0101",
                format!("cannot assign to undeclared variable `${name}`"),
                span,
            )
            .with_help(format!(
                "use `let ${name} = ...` or an explicit type declaration"
            )),
        );
    }
}
