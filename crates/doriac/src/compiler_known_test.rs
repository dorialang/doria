use crate::ast::{Item, Program};
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::names::{CompilerSymbolIdentity, GlobalSymbolId, GlobalSymbolOwner};

pub const NAMESPACE: &str = "Doria\\Std\\Test";
pub const DESCRIBE: &str = "Doria\\Std\\Test\\describe";
pub const IT: &str = "Doria\\Std\\Test\\it";
pub const TEST: &str = "Doria\\Std\\Test\\test";
pub const EXPECT: &str = "Doria\\Std\\Test\\expect";
pub const FAIL: &str = "Doria\\Std\\Test\\fail";
pub const ASSERTION_ERROR: &str = "Doria\\Std\\Test\\AssertionError";

pub const DECLARATIONS: [&str; 3] = [DESCRIBE, IT, TEST];
pub const FUTURE_MEMBERS: [&str; 3] = [EXPECT, FAIL, ASSERTION_ERROR];

pub fn is_declaration(name: &str) -> bool {
    DECLARATIONS.contains(&name)
}

pub fn is_future_member(name: &str) -> bool {
    FUTURE_MEMBERS.contains(&name)
}

pub fn is_canonical_member(name: &str) -> bool {
    is_declaration(name) || is_future_member(name)
}

pub fn symbol_id(name: &str) -> Option<GlobalSymbolId> {
    is_canonical_member(name).then(|| GlobalSymbolId {
        owner: GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTest(
            name.to_string(),
        )),
        qualified_name: name.to_string(),
    })
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
