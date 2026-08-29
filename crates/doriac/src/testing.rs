use std::collections::{HashMap, HashSet};

use crate::ast::{self, ClosureBody, Expr, Item, MemberAccess, Stmt};
use crate::build_plan::{GeneratedFor, SourceOrigin, SourceScope};
use crate::const_eval::Evaluation;
use crate::diagnostics::Diagnostic;
use crate::names::{
    CompilationContext, GlobalReferenceRole, GlobalSymbolFacts, PackageIdentity, SourceIdentity,
};
use crate::source::Span;
use crate::types::TypeRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSemanticContext {
    pub compilation: CompilationContext,
    pub scope: SourceScope,
    pub origin: SourceOrigin,
    pub generated_for: Option<GeneratedFor>,
}

impl SourceSemanticContext {
    pub fn standalone(compilation: CompilationContext) -> Self {
        Self {
            compilation,
            scope: SourceScope::Main,
            origin: SourceOrigin::Explicit,
            generated_for: None,
        }
    }

    pub fn is_development(&self) -> bool {
        self.scope == SourceScope::Development
            || (self.scope == SourceScope::Generated
                && self.generated_for == Some(GeneratedFor::Development))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehavioralTestSpelling {
    It,
    Test,
}

impl BehavioralTestSpelling {
    pub fn source_name(self) -> &'static str {
        match self {
            Self::It => "it",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestOrigin {
    Attribute,
    Behavioral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestShapeIssue {
    TargetIsNotCallable,
    CallableIsNotAFunction,
    FunctionIsGeneric,
    FunctionHasParameters,
    FunctionDoesNotReturnVoid,
    UnsupportedAccess,
}

impl TestShapeIssue {
    pub fn protocol_name(self) -> &'static str {
        match self {
            Self::TargetIsNotCallable => "targetIsNotCallable",
            Self::CallableIsNotAFunction => "callableIsNotAFunction",
            Self::FunctionIsGeneric => "functionIsGeneric",
            Self::FunctionHasParameters => "functionHasParameters",
            Self::FunctionDoesNotReturnVoid => "functionDoesNotReturnVoid",
            Self::UnsupportedAccess => "unsupportedAccess",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehavioralTestSuite {
    pub identity: String,
    pub package: PackageIdentity,
    pub source: SourceIdentity,
    pub parent_suite: Option<String>,
    pub path_segments: Vec<String>,
    pub display_name: String,
    pub call_name_span: Span,
    pub description_span: Span,
    pub body_span: Span,
    pub declaration_span: Span,
    pub authored_ordinal: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSemanticInfo {
    pub identity: String,
    pub package: PackageIdentity,
    pub source: SourceIdentity,
    pub suite: Option<String>,
    pub path_segments: Vec<String>,
    pub display_name: String,
    pub origin: TestOrigin,
    pub authored_spelling: Option<BehavioralTestSpelling>,
    pub target: String,
    pub callable_identity: Option<String>,
    pub callable_canonical_name: Option<String>,
    pub executable: bool,
    pub shape_issue: Option<TestShapeIssue>,
    pub call_name_span: Span,
    pub description_span: Span,
    pub body_span: Span,
    pub arrow_body_span: Option<Span>,
    pub declaration_span: Span,
    pub authored_ordinal: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestSemanticFacts {
    pub suites: Vec<BehavioralTestSuite>,
    pub tests: Vec<TestSemanticInfo>,
    pub declaration_spans: HashSet<Span>,
    pub compiler_elided_statement_spans: HashSet<Span>,
    pub generated_function_spans: HashSet<Span>,
}

impl TestSemanticFacts {
    pub fn is_declaration(&self, span: Span) -> bool {
        self.declaration_spans.contains(&span)
    }

    pub fn is_compiler_elided_statement(&self, span: Span) -> bool {
        self.compiler_elided_statement_spans.contains(&span)
    }

    pub fn is_generated_function(&self, span: Span) -> bool {
        self.generated_function_spans.contains(&span)
    }
}

pub struct Elaboration {
    pub program: ast::Program,
    pub facts: TestSemanticFacts,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn elaborate_source(
    program: &ast::Program,
    symbols: &GlobalSymbolFacts,
    context: &SourceSemanticContext,
    evaluation: &Evaluation,
) -> Elaboration {
    let mut elaborator = Elaborator {
        symbols,
        context,
        evaluation,
        facts: TestSemanticFacts::default(),
        diagnostics: Vec::new(),
        generated: Vec::new(),
        ordinal: 0,
        namespace: program
            .namespace
            .as_ref()
            .map(|namespace| namespace.name.canonical()),
    };
    for item in &program.items {
        if let Item::Statement(statement) = item {
            elaborator.declaration_statement(statement, &[], None);
            elaborator.elide_future_statement(statement);
        }
    }
    elaborator.future_surface_diagnostics();
    let mut program = program.clone();
    program.items.retain(|item| match item {
        Item::Statement(statement) => !elaborator
            .facts
            .is_compiler_elided_statement(statement_span(statement)),
        _ => true,
    });
    program.items.extend(elaborator.generated);
    Elaboration {
        program,
        facts: elaborator.facts,
        diagnostics: elaborator.diagnostics,
    }
}

struct Elaborator<'a> {
    symbols: &'a GlobalSymbolFacts,
    context: &'a SourceSemanticContext,
    evaluation: &'a Evaluation,
    facts: TestSemanticFacts,
    diagnostics: Vec<Diagnostic>,
    generated: Vec<Item>,
    ordinal: usize,
    namespace: Option<String>,
}

impl Elaborator<'_> {
    fn declaration_statement(
        &mut self,
        statement: &Stmt,
        parent_path: &[String],
        parent_suite: Option<&str>,
    ) -> bool {
        let Stmt::Expr { expr, span } = statement else {
            return false;
        };
        let Expr::FunctionCall { name, args, .. } = expr else {
            return false;
        };
        if !crate::compiler_known_test::is_declaration(name) {
            return false;
        }
        self.facts.declaration_spans.insert(*span);
        self.facts.compiler_elided_statement_spans.insert(*span);
        let ordinal = self.next_ordinal();
        let call_name_span = self.call_name_span(name, expr.span());
        if !self.context.is_development() {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0701",
                    "behavioral test declarations require development source",
                    call_name_span,
                )
                .with_title("Behavioral Test Declaration Requires Development Source"),
            );
            return true;
        }
        if args.len() != 2 {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0702",
                    format!(
                        "behavioral test declaration requires two arguments, got {}",
                        args.len()
                    ),
                    *span,
                )
                .with_title("Behavioral Test Declaration Requires Two Arguments"),
            );
            return true;
        }
        if args.iter().any(|argument| argument.name.is_some()) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0703",
                    "behavioral test declaration arguments must be positional",
                    *span,
                )
                .with_title("Behavioral Test Arguments Must Be Positional"),
            );
            return true;
        }
        let description_span = args[0].value.span();
        let Some(description) =
            crate::const_eval::evaluate_string_expression(self.evaluation, &args[0].value)
        else {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0704",
                    "test description must be a const-evaluable string",
                    description_span,
                )
                .with_title("Test Description Must Be A Const-Evaluable String"),
            );
            return true;
        };
        let Expr::Closure(closure) = &args[1].value else {
            let title = if name == crate::compiler_known_test::DESCRIBE {
                "Describe Body Must Be A Zero-Parameter Void Closure"
            } else {
                "Behavioral Test Body Must Be A Closure"
            };
            self.diagnostics.push(
                Diagnostic::new(
                    "E0705",
                    "behavioral test body must be a closure",
                    args[1].value.span(),
                )
                .with_title(title),
            );
            return true;
        };
        if !closure.parameters.is_empty() {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0706",
                    "behavioral test body must have no parameters",
                    closure.parameter_list_span,
                )
                .with_title(if name == crate::compiler_known_test::DESCRIBE {
                    "Describe Body Must Be A Zero-Parameter Void Closure"
                } else {
                    "Behavioral Test Body Must Have No Parameters"
                }),
            );
            return true;
        }
        if closure
            .return_type
            .as_ref()
            .is_some_and(|return_type| return_type.ty.name != "void")
        {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0707",
                    "behavioral test body must return void",
                    closure.return_type.as_ref().unwrap().span,
                )
                .with_title(if name == crate::compiler_known_test::DESCRIBE {
                    "Describe Body Must Be A Zero-Parameter Void Closure"
                } else {
                    "Behavioral Test Body Must Return Void"
                }),
            );
            return true;
        }
        if name == crate::compiler_known_test::DESCRIBE {
            self.describe(
                description,
                description_span,
                closure,
                *span,
                call_name_span,
                ordinal,
                parent_path,
                parent_suite,
            );
        } else {
            self.test(
                name,
                description,
                description_span,
                closure,
                *span,
                call_name_span,
                ordinal,
                parent_path,
                parent_suite,
            );
        }
        true
    }

    fn elide_future_statement(&mut self, statement: &Stmt) {
        let Stmt::Expr { expr, span } = statement else {
            return;
        };
        let Expr::FunctionCall { name, .. } = expr else {
            return;
        };
        if crate::compiler_known_test::is_future_member(name) {
            self.facts.compiler_elided_statement_spans.insert(*span);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn describe(
        &mut self,
        description: String,
        description_span: Span,
        closure: &ast::ClosureExpression,
        declaration_span: Span,
        call_name_span: Span,
        authored_ordinal: usize,
        parent_path: &[String],
        parent_suite: Option<&str>,
    ) {
        let mut path = parent_path.to_vec();
        path.push(description);
        let display_name = path.join(" > ");
        let identity = stable_identity(
            "suite",
            &self.context.compilation.package,
            &self.context.compilation.source,
            declaration_span,
            &display_name,
        );
        let body_span = closure_body_span(&closure.body);
        self.facts.suites.push(BehavioralTestSuite {
            identity: identity.clone(),
            package: self.context.compilation.package.clone(),
            source: self.context.compilation.source.clone(),
            parent_suite: parent_suite.map(str::to_string),
            path_segments: path.clone(),
            display_name,
            call_name_span,
            description_span,
            body_span,
            declaration_span,
            authored_ordinal,
        });
        match &closure.body {
            ClosureBody::Block(block) => {
                for statement in &block.statements {
                    if !self.declaration_statement(statement, &path, Some(&identity)) {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0708",
                                "describe body may contain only test declarations",
                                statement_span(statement),
                            )
                            .with_title("Describe Body May Contain Only Test Declarations"),
                        );
                    }
                }
            }
            ClosureBody::Expression { expression, .. } => {
                let statement = Stmt::Expr {
                    expr: (**expression).clone(),
                    span: expression.span(),
                };
                if !self.declaration_statement(&statement, &path, Some(&identity)) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0708",
                            "describe body may contain only test declarations",
                            expression.span(),
                        )
                        .with_title("Describe Body May Contain Only Test Declarations"),
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn test(
        &mut self,
        canonical_name: &str,
        description: String,
        description_span: Span,
        closure: &ast::ClosureExpression,
        declaration_span: Span,
        call_name_span: Span,
        authored_ordinal: usize,
        parent_path: &[String],
        parent_suite: Option<&str>,
    ) {
        let mut path = parent_path.to_vec();
        path.push(description);
        let display_name = path.join(" > ");
        let identity = stable_identity(
            "test",
            &self.context.compilation.package,
            &self.context.compilation.source,
            declaration_span,
            &display_name,
        );
        let digest = identity.rsplit(':').next().unwrap_or(&identity);
        let generated_name = format!("__doria_test_{}", &digest[..32]);
        let generated_canonical_name =
            program_canonical_name(self.namespace.as_deref(), &generated_name);
        let function_span = closure.span;
        let body = match &closure.body {
            ClosureBody::Block(block) => block.clone(),
            ClosureBody::Expression { expression, .. } => ast::Block {
                statements: vec![Stmt::Expr {
                    expr: (**expression).clone(),
                    span: expression.span(),
                }],
                span: closure.span,
            },
        };
        self.generated.push(Item::Function(ast::FunctionDecl {
            access: MemberAccess::Internal,
            access_span: None,
            writable_this: false,
            writable_span: None,
            is_static: false,
            static_span: None,
            name: generated_name,
            name_span: call_name_span,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(TypeRef::named("void")),
            throws: None,
            body,
            span: function_span,
        }));
        self.facts.generated_function_spans.insert(function_span);
        self.facts.tests.push(TestSemanticInfo {
            identity,
            package: self.context.compilation.package.clone(),
            source: self.context.compilation.source.clone(),
            suite: parent_suite.map(str::to_string),
            path_segments: path,
            display_name,
            origin: TestOrigin::Behavioral,
            authored_spelling: Some(if canonical_name == crate::compiler_known_test::IT {
                BehavioralTestSpelling::It
            } else {
                BehavioralTestSpelling::Test
            }),
            target: generated_canonical_name.clone(),
            callable_identity: None,
            callable_canonical_name: Some(generated_canonical_name),
            executable: true,
            shape_issue: None,
            call_name_span,
            description_span,
            body_span: closure_body_span(&closure.body),
            arrow_body_span: match &closure.body {
                ClosureBody::Expression { expression, .. } => Some(expression.span()),
                ClosureBody::Block(_) => None,
            },
            declaration_span,
            authored_ordinal,
        });
    }

    fn call_name_span(&self, canonical: &str, within: Span) -> Span {
        self.symbols
            .references
            .iter()
            .filter(|reference| {
                reference.role == GlobalReferenceRole::TestDeclaration
                    && reference.symbol_id.qualified_name == canonical
                    && reference.source_span.source == within.source
                    && reference.source_span.start >= within.start
                    && reference.source_span.end <= within.end
            })
            .min_by_key(|reference| reference.source_span.start)
            .map_or(within, |reference| reference.source_span)
    }

    fn future_surface_diagnostics(&mut self) {
        let mut seen = HashSet::new();
        for reference in &self.symbols.references {
            if reference.source_identity != self.context.compilation.source
                || matches!(
                    reference.role,
                    GlobalReferenceRole::ImportTarget | GlobalReferenceRole::ImportAliasUse
                )
                || !crate::compiler_known_test::is_future_member(
                    &reference.symbol_id.qualified_name,
                )
                || !seen.insert(reference.source_span)
            {
                continue;
            }
            self.diagnostics.push(
                Diagnostic::unsupported_stage(
                    "E0710",
                    format!(
                        "`{}` awaits Native Testing Foundation Slice 2",
                        reference.symbol_id.qualified_name
                    ),
                    reference.source_span,
                )
                .with_title("Expectation Kernel Awaits Native Testing Foundation Slice 2"),
            );
        }
    }

    fn next_ordinal(&mut self) -> usize {
        let ordinal = self.ordinal;
        self.ordinal += 1;
        ordinal
    }
}

pub fn reject_duplicate_behavioral_names(facts: &TestSemanticFacts) -> Vec<Diagnostic> {
    let mut by_name: HashMap<(String, String), Vec<&TestSemanticInfo>> = HashMap::new();
    for test in facts
        .tests
        .iter()
        .filter(|test| test.origin == TestOrigin::Behavioral)
    {
        by_name
            .entry((
                test.package.display_name().to_string(),
                test.display_name.clone(),
            ))
            .or_default()
            .push(test);
    }
    let mut groups = by_name
        .into_values()
        .filter(|group| group.len() > 1)
        .collect::<Vec<_>>();
    groups.sort_by_key(|group| {
        (
            group[0].package.display_name().to_string(),
            group[0].display_name.clone(),
        )
    });
    let mut diagnostics = Vec::new();
    for mut group in groups {
        group.sort_by_key(|test| (test.source.0.as_str(), test.declaration_span));
        let first = group[0];
        let mut diagnostic = Diagnostic::new(
            "E0709",
            format!(
                "behavioral test name `{}` is declared more than once in package `{}`",
                first.display_name,
                first.package.display_name()
            ),
            first.description_span,
        )
        .with_title("Behavioral Test Name Is Duplicated");
        for duplicate in group.iter().skip(1) {
            diagnostic = diagnostic.with_related(
                duplicate.description_span,
                "another test with the same full display name is here",
            );
        }
        diagnostics.push(diagnostic);
    }
    diagnostics
}

pub fn project_attribute_tests(
    program: &ast::Program,
    attributes: &crate::attributes::AttributeSemanticInfo,
    symbols: &GlobalSymbolFacts,
) -> Vec<TestSemanticInfo> {
    let mut projected = Vec::new();
    for application in &attributes.applications {
        if !matches!(
            &application.class_identity,
            crate::attributes::AttributeClassIdentity::CompilerKnown(name) if name == "Test"
        ) {
            continue;
        }
        let target = application.target.canonical_key();
        let (canonical_name, callable_identity, function, shape_issue, declaration_span) =
            match &application.target {
                crate::attributes::AttributeTarget::GlobalDeclaration {
                    declaration,
                    kind: ast::AttributeTargetKind::Function,
                } => {
                    let function = symbols
                        .declarations
                        .iter()
                        .find(|candidate| candidate.id == *declaration)
                        .and_then(|declaration_fact| {
                            program.items.iter().find_map(|item| match item {
                                Item::Function(function)
                                    if function.span == declaration_fact.declaration_span =>
                                {
                                    Some(function)
                                }
                                _ => None,
                            })
                        });
                    let issue = function.and_then(low_level_shape_issue);
                    (
                        declaration.qualified_name.clone(),
                        Some(format!("global:{}:function", declaration.qualified_name)),
                        function,
                        issue,
                        function.map_or(application.span, |function| function.span),
                    )
                }
                crate::attributes::AttributeTarget::ClassMember { span, .. } => (
                    target.clone(),
                    None,
                    None,
                    Some(TestShapeIssue::CallableIsNotAFunction),
                    *span,
                ),
                _ => (
                    target.clone(),
                    None,
                    None,
                    Some(TestShapeIssue::TargetIsNotCallable),
                    application.span,
                ),
            };
        let executable = function.is_some() && shape_issue.is_none();
        projected.push(TestSemanticInfo {
            identity: stable_identity(
                "attribute-test",
                &application.package,
                &application.source,
                declaration_span,
                &canonical_name,
            ),
            package: application.package.clone(),
            source: application.source.clone(),
            suite: None,
            path_segments: vec![canonical_name.clone()],
            display_name: canonical_name.clone(),
            origin: TestOrigin::Attribute,
            authored_spelling: None,
            target,
            callable_identity,
            callable_canonical_name: function.map(|_| canonical_name),
            executable,
            shape_issue,
            call_name_span: declaration_span,
            description_span: declaration_span,
            body_span: declaration_span,
            arrow_body_span: None,
            declaration_span,
            authored_ordinal: application.application_ordinal,
        });
    }
    projected.sort_by(|left, right| {
        (
            left.package.display_name(),
            left.source.0.as_str(),
            left.declaration_span,
            left.authored_ordinal,
        )
            .cmp(&(
                right.package.display_name(),
                right.source.0.as_str(),
                right.declaration_span,
                right.authored_ordinal,
            ))
    });
    projected
}

fn low_level_shape_issue(function: &ast::FunctionDecl) -> Option<TestShapeIssue> {
    if !function.type_params.is_empty() {
        Some(TestShapeIssue::FunctionIsGeneric)
    } else if !function.params.is_empty() {
        Some(TestShapeIssue::FunctionHasParameters)
    } else if function
        .return_type
        .as_ref()
        .is_none_or(|return_type| return_type.name != "void")
    {
        Some(TestShapeIssue::FunctionDoesNotReturnVoid)
    } else if !matches!(
        function.access,
        MemberAccess::External | MemberAccess::Internal
    ) {
        Some(TestShapeIssue::UnsupportedAccess)
    } else {
        None
    }
}

fn stable_identity(
    kind: &str,
    package: &PackageIdentity,
    source: &SourceIdentity,
    span: Span,
    display_name: &str,
) -> String {
    let key = format!(
        "{kind}\0{}\0{}\0{}\0{}\0{display_name}",
        package.display_name(),
        source.0,
        span.start,
        span.end,
    );
    format!(
        "{kind}:{}",
        crate::runtime_digest::sha256_hex(key.as_bytes())
    )
}

fn closure_body_span(body: &ClosureBody) -> Span {
    match body {
        ClosureBody::Expression { expression, .. } => expression.span(),
        ClosureBody::Block(block) => block.span,
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Block(block) => block.span,
        Stmt::VarDecl(value) => value.span,
        Stmt::Assignment(value) => value.span,
        Stmt::Echo { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Expr { span, .. } => *span,
        Stmt::Throw(value) => value.span,
        Stmt::Try(value) => value.span,
        Stmt::If(value) => value.span,
        Stmt::While(value) => value.span,
        Stmt::DoWhile(value) => value.span,
        Stmt::For(value) => value.span,
        Stmt::Foreach(value) => value.span,
        Stmt::Increment(value) => value.span,
    }
}

fn program_canonical_name(namespace: Option<&str>, name: &str) -> String {
    namespace.map_or_else(
        || name.to_string(),
        |namespace| format!("{namespace}\\{name}"),
    )
}
