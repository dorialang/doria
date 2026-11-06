//! Compiler-owned Time declarations, independent of the I/O error family.

use crate::ast::{Item, Program};
use crate::names::{CompilerSymbolIdentity, GlobalSymbolFacts, GlobalSymbolOwner};
use crate::source::{SourceFile, SourceId};

pub const CLOCK_ERROR: &str = "Doria\\Std\\Time\\ClockError";
pub const DURATION: &str = "Doria\\Std\\Time\\Duration";
pub const DEADLINE: &str = "Doria\\Std\\Time\\Deadline";
pub const CANONICAL_TYPES: [&str; 3] = [CLOCK_ERROR, DURATION, DEADLINE];
pub const SOURCE_ID: SourceId = SourceId(0x7fff_fffb);
pub const SOURCE_NAME: &str = "<compiler-known:time>";
pub const CLOCK_FAILURE_MESSAGE: &str = doria_time::CLOCK_FAILURE_MESSAGE;
pub const SOURCE_TEXT: &str = r#"
class ClockError implements Error {
    function __construct(string $message, ?int $systemCode = null) {}
}
class Duration {
    internal function __construct() {}
    static function fromNanoseconds(int64 $nanoseconds): self {}
    int64 $nanoseconds { get {} }
}
class Deadline {
    internal function __construct() {}
    static function fromNow(Doria\Std\Time\Duration $duration): self throws Doria\Std\Time\ClockError {}
}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeFunction {
    DurationFromNanoseconds,
    DurationNanoseconds,
    DeadlineFromNow,
}

pub fn resolved_facts_use_time(facts: &GlobalSymbolFacts) -> bool {
    crate::compiler_known_stream::resolved_facts_use_streams(facts)
        || facts.references.iter().any(|reference| {
            matches!(
                reference.symbol_id.owner,
                GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTime(_))
            )
        })
}

pub fn augment_program(program: &Program) -> Program {
    static DECLARATIONS: std::sync::LazyLock<Program> = std::sync::LazyLock::new(|| {
        let mut program =
            crate::parse_source_file(&SourceFile::with_id(SOURCE_ID, SOURCE_NAME, SOURCE_TEXT))
                .expect("compiler-known Time declarations parse");
        for item in &mut program.items {
            let Item::Class(class) = item else {
                unreachable!("compiler-known Time declarations are nominal types")
            };
            if class.name == "Duration" {
                for member in &mut class.members {
                    match member {
                        crate::ast::ClassMember::Method(method)
                            if method.name == "fromNanoseconds" =>
                        {
                            method.body = crate::ast::FunctionBody::Intrinsic {
                                implementation: crate::ast::CompilerIntrinsic::Time(
                                    TimeFunction::DurationFromNanoseconds,
                                ),
                                span: method.body.span(),
                            };
                        }
                        crate::ast::ClassMember::Property(property) => {
                            for hook in &mut property.hooks {
                                hook.body = crate::ast::FunctionBody::Intrinsic {
                                    implementation: crate::ast::CompilerIntrinsic::Time(
                                        TimeFunction::DurationNanoseconds,
                                    ),
                                    span: hook.body.span(),
                                };
                            }
                        }
                        _ => {}
                    }
                }
            }
            if class.name == "Deadline" {
                for member in &mut class.members {
                    if let crate::ast::ClassMember::Method(method) = member {
                        if method.name == "fromNow" {
                            method.body = crate::ast::FunctionBody::Intrinsic {
                                implementation: crate::ast::CompilerIntrinsic::Time(
                                    TimeFunction::DeadlineFromNow,
                                ),
                                span: method.body.span(),
                            };
                        }
                    }
                }
            }
            class.name = format!("Doria\\Std\\Time\\{}", class.name);
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
