use crate::ast::{Block, ClassDecl, ClassMember, FunctionDecl, Item, MemberAccess, Param, Program};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::names::{CompilerSymbolIdentity, GlobalSymbolFacts, GlobalSymbolId, GlobalSymbolOwner};
use crate::source::Span;
use crate::types::TypeRef;

pub const NAMESPACE: &str = "Doria\\Std\\Test";
pub const DESCRIBE: &str = "Doria\\Std\\Test\\describe";
pub const IT: &str = "Doria\\Std\\Test\\it";
pub const TEST: &str = "Doria\\Std\\Test\\test";
pub const EXPECT: &str = "Doria\\Std\\Test\\expect";
pub const FAIL: &str = "Doria\\Std\\Test\\fail";
pub const ASSERTION_ERROR: &str = "Doria\\Std\\Test\\AssertionError";

pub const ASSERTION_MATCHER_PROPERTY: &str = "__assertionMatcher";
pub const ASSERTION_NEGATED_PROPERTY: &str = "__assertionNegated";
pub const ASSERTION_ACTUAL_PRESENT_PROPERTY: &str = "__assertionActualPresent";
pub const ASSERTION_ACTUAL_TYPE_PROPERTY: &str = "__assertionActualType";
pub const ASSERTION_ACTUAL_PRESENTATION_PROPERTY: &str = "__assertionActualPresentation";
pub const ASSERTION_EXPECTED_PRESENT_PROPERTY: &str = "__assertionExpectedPresent";
pub const ASSERTION_EXPECTED_TYPE_PROPERTY: &str = "__assertionExpectedType";
pub const ASSERTION_EXPECTED_PRESENTATION_PROPERTY: &str = "__assertionExpectedPresentation";
pub const ASSERTION_DIFFERENCE_PRESENT_PROPERTY: &str = "__assertionDifferencePresent";
pub const ASSERTION_DIFFERENCE_PROPERTY: &str = "__assertionDifference";
pub const ASSERTION_USER_MESSAGE_PRESENT_PROPERTY: &str = "__assertionUserMessagePresent";
pub const ASSERTION_USER_MESSAGE_PROPERTY: &str = "__assertionUserMessage";

pub const ASSERTION_FACT_PROPERTIES: [&str; 12] = [
    ASSERTION_MATCHER_PROPERTY,
    ASSERTION_NEGATED_PROPERTY,
    ASSERTION_ACTUAL_PRESENT_PROPERTY,
    ASSERTION_ACTUAL_TYPE_PROPERTY,
    ASSERTION_ACTUAL_PRESENTATION_PROPERTY,
    ASSERTION_EXPECTED_PRESENT_PROPERTY,
    ASSERTION_EXPECTED_TYPE_PROPERTY,
    ASSERTION_EXPECTED_PRESENTATION_PROPERTY,
    ASSERTION_DIFFERENCE_PRESENT_PROPERTY,
    ASSERTION_DIFFERENCE_PROPERTY,
    ASSERTION_USER_MESSAGE_PRESENT_PROPERTY,
    ASSERTION_USER_MESSAGE_PROPERTY,
];

pub const DECLARATIONS: [&str; 3] = [DESCRIBE, IT, TEST];
pub const ASSERTION_FUNCTIONS: [&str; 2] = [EXPECT, FAIL];
pub const IMPLEMENTED_MEMBERS: [&str; 6] = [DESCRIBE, IT, TEST, EXPECT, FAIL, ASSERTION_ERROR];
pub const FUTURE_MEMBERS: [&str; 0] = [];

pub fn is_declaration(name: &str) -> bool {
    DECLARATIONS.contains(&name)
}

pub fn is_future_member(name: &str) -> bool {
    FUTURE_MEMBERS.contains(&name)
}

pub fn is_canonical_member(name: &str) -> bool {
    IMPLEMENTED_MEMBERS.contains(&name) || is_future_member(name)
}

pub fn is_assertion_function(name: &str) -> bool {
    ASSERTION_FUNCTIONS.contains(&name)
}

pub fn resolved_facts_use_assertion_surface(facts: &GlobalSymbolFacts) -> bool {
    facts.references.iter().any(|reference| {
        matches!(
            &reference.symbol_id.owner,
            GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTest(name))
                if name == EXPECT || name == FAIL || name == ASSERTION_ERROR
        )
    })
}

pub fn symbol_id(name: &str) -> Option<GlobalSymbolId> {
    is_canonical_member(name).then(|| GlobalSymbolId {
        owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTest(
            name.to_string(),
        )),
        qualified_name: name.to_string(),
    })
}

/// Adds the compiler-owned assertion Error to the ordinary class model. Name
/// resolution has already established the exact canonical identity; all later
/// phases therefore use normal nominal typing and checked-Error transport.
pub fn augment_program(program: &Program) -> Program {
    let mut augmented = program.clone();
    if augmented
        .items
        .iter()
        .any(|item| matches!(item, Item::Class(class) if class.name == ASSERTION_ERROR))
    {
        return augmented;
    }
    let span = Span::in_source(crate::compiler_known_io::SYNTHETIC_SOURCE_ID, 6, 7);
    let property = |ty: &str, name: &str, access| Param {
        promoted_access: Some(access),
        take: false,
        take_span: None,
        writable: false,
        writable_span: None,
        ownership_modifier_insert: span,
        ty: TypeRef::named(ty),
        name: name.to_string(),
        default: None,
        span,
    };
    let mut params = vec![property("string", "message", MemberAccess::External)];
    for (ty, name) in [
        ("string", ASSERTION_MATCHER_PROPERTY),
        ("bool", ASSERTION_NEGATED_PROPERTY),
        ("bool", ASSERTION_ACTUAL_PRESENT_PROPERTY),
        ("string", ASSERTION_ACTUAL_TYPE_PROPERTY),
        ("string", ASSERTION_ACTUAL_PRESENTATION_PROPERTY),
        ("bool", ASSERTION_EXPECTED_PRESENT_PROPERTY),
        ("string", ASSERTION_EXPECTED_TYPE_PROPERTY),
        ("string", ASSERTION_EXPECTED_PRESENTATION_PROPERTY),
        ("bool", ASSERTION_DIFFERENCE_PRESENT_PROPERTY),
        ("string", ASSERTION_DIFFERENCE_PROPERTY),
        ("bool", ASSERTION_USER_MESSAGE_PRESENT_PROPERTY),
        ("string", ASSERTION_USER_MESSAGE_PROPERTY),
    ] {
        params.push(property(ty, name, MemberAccess::Internal));
    }
    augmented.items.push(Item::Class(ClassDecl {
        access: MemberAccess::External,
        access_span: None,
        name: ASSERTION_ERROR.to_string(),
        name_span: span,
        type_params: Vec::new(),
        parent: None,
        parent_span: None,
        implements: vec!["Error".to_string()],
        members: vec![ClassMember::Method(FunctionDecl {
            access: MemberAccess::Internal,
            access_span: None,
            writable_this: false,
            writable_span: None,
            is_static: false,
            static_span: None,
            name: "__construct".to_string(),
            name_span: span,
            type_params: Vec::new(),
            params,
            return_type: None,
            throws: None,
            body: Block {
                statements: Vec::new(),
                span,
            },
            span,
        })],
        span,
    }));
    augmented
}

pub fn validate_reserved_identities(program: &Program) -> DiagnosticResult<()> {
    let namespace = program
        .namespace
        .as_ref()
        .map(|namespace| namespace.name.canonical());
    let mut diagnostics = Vec::new();
    for item in &program.items {
        let (name, span) = match item {
            Item::Class(value) => (&value.name, value.name_span),
            Item::Enum(value) => (&value.name, value.name_span),
            Item::Interface(value) => (&value.name, value.name_span),
            Item::Trait(value) => (&value.name, value.name_span),
            Item::Function(value) => (&value.name, value.name_span),
            Item::Constant(value) => (&value.name, value.name_span),
            Item::Statement(_) => continue,
        };
        let canonical = namespace
            .as_ref()
            .map_or_else(|| name.clone(), |namespace| format!("{namespace}\\{name}"));
        if is_canonical_member(&canonical) {
            diagnostics.push(
                Diagnostic::new(
                    "E0700",
                    format!("`{canonical}` is reserved for the compiler-owned Doria test module"),
                    span,
                )
                .with_title("Compiler-Known Test Identity Is Reserved")
                .with_help("choose a different declaration name"),
            );
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}
