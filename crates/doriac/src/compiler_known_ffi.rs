//! Compiler-owned `Doria\Std\Ffi` declarations (Decision 0148 section 1).
//!
//! `Pointer`, `WritablePointer`, `Opaque` and the C boundary types are carried by the type
//! model itself ([`crate::foreign`]). The enums and attribute classes below
//! are ordinary Doria, so attribute arguments are checked by the typed
//! attribute machinery of Decision 0125.

use crate::ast::{Item, Program};
use crate::names::{CompilerSymbolIdentity, GlobalSymbolFacts, GlobalSymbolOwner};
use crate::source::{SourceFile, SourceId};

pub const OPERATING_SYSTEM: &str = "Doria\\Std\\Ffi\\OperatingSystem";
pub const ARCHITECTURE: &str = "Doria\\Std\\Ffi\\Architecture";
pub const SYSTEM_CODE_SOURCE: &str = "Doria\\Std\\Ffi\\SystemCodeSource";
pub const LIBRARY: &str = "Doria\\Std\\Ffi\\Library";
pub const LINK_NAME: &str = "Doria\\Std\\Ffi\\LinkName";
pub const VARIADIC: &str = "Doria\\Std\\Ffi\\Variadic";
pub const CAPTURE_SYSTEM_CODE: &str = "Doria\\Std\\Ffi\\CaptureSystemCode";
pub const NON_BLOCKING: &str = "Doria\\Std\\Ffi\\NonBlocking";

/// Members of `Doria\Std\Ffi` whose surface lands with later Stage 40 work.
/// They resolve, so their use reads as not available yet rather than unknown.
pub const PLATFORM: &str = "Doria\\Std\\Ffi\\Platform";
pub const FOREIGN_CALL: &str = "Doria\\Std\\Ffi\\ForeignCall";
pub const HANDLE: &str = "Doria\\Std\\Ffi\\Handle";
/// The owned foreign strings, which the type model carries
/// ([`crate::foreign::ForeignStringEncoding`]).
pub const C_STRING: &str = "Doria\\Std\\Ffi\\CString";
pub const UTF16_STRING: &str = "Doria\\Std\\Ffi\\Utf16String";
pub const PENDING_TYPES: [&str; 2] = [PLATFORM, HANDLE];

/// Members of `Doria\Std\Ffi` that checking models directly: the
/// `ForeignCall::lastSystemCode` static (section 16).
pub const MODELED_TYPES: [&str; 1] = [FOREIGN_CALL];
pub const LAST_SYSTEM_CODE: &str = "lastSystemCode";

/// The declarations in [`SOURCE_TEXT`].
pub const DECLARED_TYPES: [&str; 8] = [
    OPERATING_SYSTEM,
    ARCHITECTURE,
    SYSTEM_CODE_SOURCE,
    LIBRARY,
    LINK_NAME,
    VARIADIC,
    CAPTURE_SYSTEM_CODE,
    NON_BLOCKING,
];

/// The attributes that give a foreign declaration its link facts and
/// contract (sections 14 to 16, 18 and 26).
pub const FOREIGN_FUNCTION_ATTRIBUTES: [&str; 4] =
    [LINK_NAME, VARIADIC, CAPTURE_SYSTEM_CODE, NON_BLOCKING];

pub const SOURCE_ID: SourceId = SourceId(0x7fff_fff7);
pub const SOURCE_NAME: &str = "<compiler-known:ffi>";

pub const SOURCE_TEXT: &str = r#"
enum OperatingSystem {
    case Linux;
    case MacOs;
    case Windows;
}
enum Architecture {
    case X64;
    case Arm64;
}
enum SystemCodeSource {
    case Errno;
    case WindowsLastError;
}
#[Attribute]
class Library {
    function __construct(string $name) {}
}
#[Attribute]
class LinkName {
    function __construct(string $symbol) {}
}
#[Attribute]
class Variadic {
    function __construct(int $fixedParameters) {}
}
#[Attribute]
class CaptureSystemCode {
    function __construct(Doria\Std\Ffi\SystemCodeSource $source) {}
}
#[Attribute]
class NonBlocking {
}
"#;

/// Every name of `Doria\Std\Ffi` that authored source can reach.
pub fn canonical_types() -> impl Iterator<Item = &'static str> {
    crate::foreign::type_names()
        .chain(DECLARED_TYPES)
        .chain(MODELED_TYPES)
        .chain(PENDING_TYPES)
}

pub fn is_canonical_type(name: &str) -> bool {
    canonical_types().any(|candidate| candidate == name)
}

pub fn resolved_facts_use_ffi(facts: &GlobalSymbolFacts) -> bool {
    facts.references.iter().any(|reference| {
        matches!(
            &reference.symbol_id.owner,
            GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardFfi(name))
                if DECLARED_TYPES.contains(&name.as_str())
        )
    })
}

fn declarations() -> &'static Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        let mut program =
            crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
                .expect("compiler-known Ffi declarations parse");
        for item in &mut program.items {
            match item {
                Item::Class(class) => {
                    class.name = format!("Doria\\Std\\Ffi\\{}", class.name);
                }
                Item::Enum(declaration) => {
                    declaration.name = format!("Doria\\Std\\Ffi\\{}", declaration.name);
                }
                _ => unreachable!("compiler-known Ffi declarations are classes and enums"),
            }
        }
        // The module is added after name resolution, so its attribute
        // markers carry the identity resolution gives `#[Attribute]`.
        for attachment in &mut program.attributes {
            for group in &mut attachment.groups {
                for attribute in &mut group.attributes {
                    attribute.canonical_name = Some(attribute.name.canonical());
                }
            }
        }
        program
    });
    &DECLARATIONS
}

pub fn augment_program(program: &Program) -> Program {
    let mut augmented = program.clone();
    augmented.items.extend(declarations().items.iter().cloned());
    augmented
        .attributes
        .extend(declarations().attributes.iter().cloned());
    augmented
}

pub(crate) fn append_hir_source(program: &mut crate::hir::Program) {
    if !program.items.iter().any(|item| match item {
        crate::hir::Item::Class(class) => class.span.source == SOURCE_ID,
        crate::hir::Item::Enum(declaration) => declaration.span.source == SOURCE_ID,
        _ => false,
    }) {
        return;
    }
    program.sources.push(crate::hir::SourceUnit {
        id: SOURCE_ID,
        identity: crate::names::SourceIdentity(SOURCE_NAME.to_string()),
        package: crate::names::PackageIdentity::CompilerKnown,
        display_path: SOURCE_NAME.to_string(),
        scope: crate::build_plan::SourceScope::Main,
        origin: crate::build_plan::SourceOrigin::Explicit,
        generated_for: None,
        active: true,
        source: SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT),
    });
    if !program
        .packages
        .iter()
        .any(|package| package.identity == crate::names::PackageIdentity::CompilerKnown)
    {
        program.packages.push(crate::hir::PackageUnit {
            identity: crate::names::PackageIdentity::CompilerKnown,
            normal_dependencies: Vec::new(),
            development_dependencies: Vec::new(),
        });
    }
}
