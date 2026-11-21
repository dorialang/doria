//! `declare` directives and the unsafe policy (Decision 0148 section 25).
//!
//! A file's `declare(<key>: "<value>");` directives come first in the file,
//! which the parser enforces. This module checks which keys and values exist,
//! that each key appears once, and the unsafe policy: a file or package that
//! denies unsafe code may contain no unsafe form, and a file cannot relax its
//! package's `"deny"`.
//!
//! The unsafe forms are syntax, so the authored tree of one source lists
//! them ([`inventory`]); the policy check and the `baton audit unsafe`
//! inventory read the same list.

use crate::ast::transform::{Transform, Transformable};
use crate::ast::*;
use crate::build_plan::UnsafePolicy;
use crate::diagnostics::{Diagnostic, FixApplicability, FixEdit};
use crate::names::{CompilationContext, PackageIdentity};
use crate::source::Span;

pub const UNKNOWN_DIRECTIVE_CODE: &str = "E0813";
pub const REPEATED_DIRECTIVE_CODE: &str = "E0814";
pub const DENIED_FORM_CODE: &str = "E0815";
pub const RELAXED_POLICY_CODE: &str = "E0816";

/// The keys a `declare` directive may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveKey {
    /// `declare(unsafe: "allow" | "deny")`.
    Unsafe,
}

impl DirectiveKey {
    pub fn from_source(key: &str) -> Option<Self> {
        match key {
            "unsafe" => Some(Self::Unsafe),
            _ => None,
        }
    }
}

impl UnsafePolicy {
    pub fn from_source(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// What makes a source form unsafe code for the policy and the audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnsafeFormKind {
    /// `unsafe { ... }` or `unsafe(<expression>)`.
    UnsafeContext,
    /// `unsafe function`.
    UnsafeFunction,
    /// `extern "C" { ... }`.
    ExternBlock,
    /// One declaration of an `extern "C"` block.
    ExternDeclaration,
    /// `extern "C" struct` or `extern "C" union`.
    ForeignAggregate,
    /// `extern "C" function` with a body.
    CCallableFunction,
}

impl UnsafeFormKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsafeContext => "unsafeContext",
            Self::UnsafeFunction => "unsafeFunction",
            Self::ExternBlock => "externBlock",
            Self::ExternDeclaration => "externDeclaration",
            Self::ForeignAggregate => "foreignAggregate",
            Self::CCallableFunction => "cCallableFunction",
        }
    }

    /// The form in prose, with its article.
    pub const fn noun(self) -> &'static str {
        match self {
            Self::UnsafeContext => "an unsafe context",
            Self::UnsafeFunction => "an `unsafe function`",
            Self::ExternBlock => "an `extern \"C\"` block",
            Self::ExternDeclaration => "an extern declaration",
            Self::ForeignAggregate => "a foreign struct or union",
            Self::CCallableFunction => "a C-callable function",
        }
    }

    /// Whether the policy reports the form. An extern declaration is
    /// reported through its block.
    const fn denied_alone(self) -> bool {
        !matches!(self, Self::ExternDeclaration)
    }
}

/// One unsafe form of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsafeForm {
    pub kind: UnsafeFormKind,
    /// The keyword that makes the form unsafe: `unsafe` or `extern`, or the
    /// name of an extern declaration.
    pub keyword_span: Span,
    /// The whole form.
    pub span: Span,
    /// The declared name, for a declaration.
    pub name: Option<String>,
}

/// Every unsafe form of `program`, in source order.
pub fn inventory(program: &Program) -> Vec<UnsafeForm> {
    let mut finder = FormFinder::default();
    for item in &program.items {
        finder.item(item);
    }
    finder
        .forms
        .sort_by_key(|form| (form.span.source.0, form.span.start, form.kind));
    finder.forms
}

#[derive(Default)]
struct FormFinder {
    forms: Vec<UnsafeForm>,
}

impl FormFinder {
    fn push(&mut self, kind: UnsafeFormKind, keyword_span: Span, span: Span, name: Option<&str>) {
        self.forms.push(UnsafeForm {
            kind,
            keyword_span,
            span,
            name: name.map(str::to_string),
        });
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Class(class) => self.members(&class.members),
            Item::Trait(declaration) => self.members(&declaration.members),
            Item::Interface(declaration) => declaration.clone().transform(self),
            Item::Enum(declaration) => {
                for case in &declaration.cases {
                    if let Some(value) = &case.backing_value {
                        value.clone().transform(self);
                    }
                }
            }
            Item::Function(function) => function.clone().transform(self),
            Item::Constant(constant) => constant.clone().transform(self),
            Item::ExternBlock(block) => self.push(
                UnsafeFormKind::ExternBlock,
                block.extern_span,
                block.span,
                None,
            ),
            Item::ForeignAggregate(aggregate) => self.push(
                UnsafeFormKind::ForeignAggregate,
                aggregate.extern_span,
                aggregate.span,
                Some(&aggregate.name),
            ),
            Item::Statement(statement) => statement.clone().transform(self),
        }
    }

    fn members(&mut self, members: &[ClassMember]) {
        for member in members {
            member.clone().transform(self);
        }
    }
}

impl Transform for FormFinder {
    fn function(&mut self, function: &mut FunctionDecl) {
        if let Some(unsafe_span) = function.unsafe_span {
            self.push(
                UnsafeFormKind::UnsafeFunction,
                unsafe_span,
                function.span,
                Some(&function.name),
            );
        }
        match &function.linkage {
            FunctionLinkage::Doria => {}
            FunctionLinkage::CAbi { extern_span, .. } => self.push(
                UnsafeFormKind::CCallableFunction,
                *extern_span,
                function.span,
                Some(&function.name),
            ),
            FunctionLinkage::Foreign { .. } => self.push(
                UnsafeFormKind::ExternDeclaration,
                function.name_span,
                function.span,
                Some(&function.name),
            ),
        }
        function.walk(self);
    }

    fn statement(&mut self, statement: &mut Stmt) {
        if let Stmt::Unsafe(block) = statement {
            self.push(
                UnsafeFormKind::UnsafeContext,
                block.keyword_span,
                block.span,
                None,
            );
        }
        statement.walk(self);
    }

    fn expression(&mut self, expression: &mut Expr) {
        if let Expr::Unsafe {
            keyword_span, span, ..
        } = expression
        {
            self.push(UnsafeFormKind::UnsafeContext, *keyword_span, *span, None);
        }
        expression.walk(self);
    }
}

/// The diagnostics of `program`'s directives and of its unsafe policy, for
/// the source and package `context` names.
pub fn check(program: &Program, context: &CompilationContext) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut file_policy: Option<(UnsafePolicy, &DeclareDirective)> = None;
    for directive in &program.directives {
        let Some(key) = DirectiveKey::from_source(&directive.key.text) else {
            diagnostics.push(unknown_key(directive));
            continue;
        };
        match key {
            DirectiveKey::Unsafe => {
                let Some(policy) = UnsafePolicy::from_source(&directive.value) else {
                    diagnostics.push(unknown_value(directive));
                    continue;
                };
                if let Some((first_policy, first)) = file_policy {
                    diagnostics.push(repeated(directive, first, first_policy == policy));
                    continue;
                }
                file_policy = Some((policy, directive));
            }
        }
    }
    let package_denies = context.unsafe_policy == UnsafePolicy::Deny;
    if package_denies {
        if let Some((UnsafePolicy::Allow, directive)) = file_policy {
            diagnostics.push(relaxed(directive, &context.package));
        }
    }
    let denial = if package_denies {
        Some(Denial::Package(&context.package))
    } else {
        match file_policy {
            Some((UnsafePolicy::Deny, directive)) => Some(Denial::File(directive)),
            _ => None,
        }
    };
    if let Some(denial) = denial {
        diagnostics.extend(
            inventory(program)
                .iter()
                .filter(|form| form.kind.denied_alone())
                .map(|form| denied(form, &denial)),
        );
    }
    diagnostics
}

enum Denial<'a> {
    File(&'a DeclareDirective),
    Package(&'a PackageIdentity),
}

fn unknown_key(directive: &DeclareDirective) -> Diagnostic {
    Diagnostic::new(
        UNKNOWN_DIRECTIVE_CODE,
        format!(
            "`declare({}: ...)` names no directive; Doria has `declare(unsafe: \"allow\")` and `declare(unsafe: \"deny\")`",
            directive.key.text
        ),
        directive.key.span,
    )
    .with_title("Unknown Declare Directive")
    .with_primary_label("No Directive Has This Key")
    .with_explanation(
        "A `declare` directive names one compiler directive and its value. The `unsafe` directive is the one Doria has; it sets whether the file may contain unsafe code.",
    )
    .with_help("remove the directive, or write `declare(unsafe: \"deny\");`")
}

fn unknown_value(directive: &DeclareDirective) -> Diagnostic {
    Diagnostic::new(
        UNKNOWN_DIRECTIVE_CODE,
        format!(
            "`\"{}\"` is not an unsafe policy; the unsafe policy is `\"allow\"` or `\"deny\"`",
            directive.value
        ),
        directive.value_span,
    )
    .with_title("Unknown Unsafe Policy")
    .with_primary_label("Not A Policy")
    .with_explanation(
        "`declare(unsafe: \"deny\");` forbids unsafe code in the file, and `declare(unsafe: \"allow\");` permits it, as a file without the directive does.",
    )
    .with_help("write `\"deny\"` or `\"allow\"`")
}

fn repeated(directive: &DeclareDirective, first: &DeclareDirective, same: bool) -> Diagnostic {
    let diagnostic = Diagnostic::new(
        REPEATED_DIRECTIVE_CODE,
        format!(
            "`declare({}: ...)` appears more than once in this file; a file states each directive once",
            directive.key.text
        ),
        directive.key.span,
    )
    .with_title("Repeated Declare Directive")
    .with_primary_label("Repeated Here")
    .with_related(first.span, "First Stated Here")
    .with_explanation("Each `declare` key appears at most once in a file, so the file has one value for it.");
    let fix = vec![FixEdit {
        source: crate::diagnostics::DiagnosticSource::Current,
        span: directive.span,
        replacement: String::new(),
    }];
    if same {
        diagnostic
            .with_help("remove the repeated directive, which states the same value")
            .with_structured_fix(
                "Remove The Repeated Directive",
                FixApplicability::MachineApplicable,
                fix,
            )
    } else {
        diagnostic
            .with_help("keep the directive with the policy the file needs and remove the other")
            .with_structured_fix(
                "Remove This Directive",
                FixApplicability::RequiresReview,
                fix,
            )
    }
}

fn relaxed(directive: &DeclareDirective, package: &PackageIdentity) -> Diagnostic {
    Diagnostic::new(
        RELAXED_POLICY_CODE,
        format!(
            "package `{}` sets `unsafe = \"deny\"`, which a file cannot relax with `declare(unsafe: \"allow\")`",
            package.display_name()
        ),
        directive.span,
    )
    .with_title("File Cannot Allow Unsafe Code In A Denying Package")
    .with_primary_label("The Package Denies Unsafe Code")
    .with_explanation(
        "A package's `unsafe = \"deny\"` in `Baton.toml` applies to every source of the package. Unsafe code belongs in a package that allows it, which this package can depend on.",
    )
    .with_help("remove the directive, and move the unsafe code to a package that allows it")
    .with_structured_fix(
        "Remove The Directive",
        FixApplicability::MachineApplicable,
        vec![FixEdit {
            source: crate::diagnostics::DiagnosticSource::Current,
            span: directive.span,
            replacement: String::new(),
        }],
    )
}

fn denied(form: &UnsafeForm, denial: &Denial<'_>) -> Diagnostic {
    let noun = form.kind.noun();
    let diagnostic = match denial {
        Denial::File(directive) => Diagnostic::new(
            DENIED_FORM_CODE,
            format!("this file declares `declare(unsafe: \"deny\")`, so it may contain no {noun}"),
            form.keyword_span,
        )
        .with_title("Unsafe Code Denied By This File")
        .with_related(directive.span, "The File Denies Unsafe Code Here"),
        Denial::Package(package) => Diagnostic::new(
            DENIED_FORM_CODE,
            format!(
                "package `{}` sets `unsafe = \"deny\"` in `Baton.toml`, so its sources may contain no {noun}",
                package.display_name()
            ),
            form.keyword_span,
        )
        .with_title("Unsafe Code Denied By This Package"),
    };
    diagnostic
        .with_primary_label("Unsafe Code")
        .with_explanation(
            "An unsafe policy of \"deny\" forbids unsafe contexts, `unsafe function`s, `extern \"C\"` blocks, foreign structs and unions, and C-callable functions. Calling a safe function that another package or file defines stays allowed.",
        )
        .with_help("move this code to a file or package that allows unsafe code, and call its safe wrapper from here")
}
