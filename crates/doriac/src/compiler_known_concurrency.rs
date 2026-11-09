//! Compiler-owned concurrency declarations share ordinary checked-error transport.

use crate::ast::{Item, Program};
use crate::names::{CompilerSymbolIdentity, GlobalSymbolFacts, GlobalSymbolOwner};
use crate::source::{SourceFile, SourceId};

pub const CANCELLATION_ERROR: &str = "Doria\\Std\\Concurrency\\CancellationError";
pub const CANCELLATION_SOURCE: &str = "Doria\\Std\\Concurrency\\CancellationSource";
pub const CANCELLATION_TOKEN: &str = "Doria\\Std\\Concurrency\\CancellationToken";
pub const CANCELLATION_NOTIFICATION_FAILURE_MESSAGE: &str =
    doria_diagnostic_catalogue::CANCELLATION_NOTIFICATION_FAILURE_MESSAGE;
pub const CANONICAL_TYPES: [&str; 3] =
    [CANCELLATION_ERROR, CANCELLATION_SOURCE, CANCELLATION_TOKEN];
pub const SOURCE_ID: SourceId = SourceId(0x7fff_fffa);
pub const SOURCE_NAME: &str = "<compiler-known:concurrency>";
pub const SOURCE_TEXT: &str = r#"
class CancellationError implements Error {
    function __construct(string $message, ?int $systemCode = null) {}
}
class CancellationSource {
    function __construct() {}
    Doria\Std\Concurrency\CancellationToken $token { get {} }
    bool $cancellationRequested { get {} }
    writable function cancel(): void throws Doria\Std\Concurrency\CancellationError {}
}
class CancellationToken {
    internal function __construct() {}
}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyFunction {
    SourceConstructor,
    SourceToken,
    SourceRequested,
    SourceCancel,
}

pub fn resolved_facts_use_concurrency(facts: &GlobalSymbolFacts) -> bool {
    crate::compiler_known_stream::resolved_facts_use_streams(facts)
        || facts.references.iter().any(|reference| {
            matches!(
                reference.symbol_id.owner,
                GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardConcurrency(_))
            )
        })
}

pub fn augment_program(program: &Program) -> Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        let mut program =
            crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
                .expect("compiler-known concurrency declarations parse");
        for item in &mut program.items {
            let Item::Class(class) = item else {
                unreachable!("compiler-known concurrency declarations are nominal types")
            };
            if class.name == "CancellationSource" {
                for member in &mut class.members {
                    match member {
                        crate::ast::ClassMember::Method(method) => {
                            let implementation = match method.name.as_str() {
                                "__construct" => ConcurrencyFunction::SourceConstructor,
                                "cancel" => ConcurrencyFunction::SourceCancel,
                                _ => unreachable!("cancellation source method"),
                            };
                            method.body = crate::ast::FunctionBody::Intrinsic {
                                implementation: crate::ast::CompilerIntrinsic::Concurrency(
                                    implementation,
                                ),
                                span: method.body.span(),
                            };
                        }
                        crate::ast::ClassMember::Property(property) => {
                            let implementation = match property.name.as_str() {
                                "token" => ConcurrencyFunction::SourceToken,
                                "cancellationRequested" => ConcurrencyFunction::SourceRequested,
                                _ => unreachable!("cancellation source property"),
                            };
                            for hook in &mut property.hooks {
                                hook.body = crate::ast::FunctionBody::Intrinsic {
                                    implementation: crate::ast::CompilerIntrinsic::Concurrency(
                                        implementation,
                                    ),
                                    span: hook.body.span(),
                                };
                            }
                        }
                        _ => unreachable!("cancellation source member"),
                    }
                }
            }
            class.name = format!("Doria\\Std\\Concurrency\\{}", class.name);
        }
        program
    });
    let mut augmented = program.clone();
    augmented.items.extend(DECLARATIONS.items.iter().cloned());
    augmented
}

pub(crate) fn append_hir_source(program: &mut crate::hir::Program) {
    if !program.items.iter().any(
        |item| matches!(item, crate::hir::Item::Class(class) if class.span.source == SOURCE_ID),
    ) {
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
