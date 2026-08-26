use doriac::ast::Item;
use doriac::backend::{BackendOutput, BackendTarget, CompileOptions};
use doriac::diagnostics::DiagnosticKind;
use doriac::names::{
    compiler_known_symbol_facts, edition_prelude, CompilationContext, CompilerKnownProvenance,
    CompilerSymbolIdentity, Edition, GlobalReferenceRole, GlobalSymbolKind, GlobalSymbolOwner,
    PackageIdentity, SourceIdentity, EXTERNAL_SYMBOL_BOUNDARY_CODE, INCLUDE_BOUNDARY_CODE,
};
use doriac::source::Span;
use std::collections::HashSet;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SAME_FILE_SOURCE: &str = r#"
namespace Acme\Demo;

use Acme\Demo\{
    Counter as DemoCounter,
    Status as DemoStatus,
    answer as importedAnswer,
    ANSWER as IMPORTED_ANSWER,
};

const int ANSWER = 42;

enum Status: int
{
    case Ready = 1;
}

class Counter
{
    function __construct(int $value)
    {
    }
}

function answer(): int
{
    return ANSWER;
}

function main(): void
{
    let $counter = new DemoCounter(importedAnswer());
    if (DemoStatus::Ready == DemoStatus::Ready) {
        echo "{$counter->value} " . IMPORTED_ANSWER . "\n";
    }
}
"#;

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run_php_file(source_name: &str, php: &str) -> std::process::Output {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "doria-stage31-{}-{unique}-{source_name}.php",
        std::process::id()
    ));
    fs::write(&path, php).expect("generated PHP should be writable");
    let output = Command::new("php")
        .arg(&path)
        .output()
        .expect("generated PHP should start");
    let _ = fs::remove_file(path);
    output
}

#[test]
fn parser_distinguishes_root_single_and_multi_segment_namespaces() {
    let root = doriac::parse_source("root.doria", "function main(): void {}")
        .expect("root source should parse");
    assert!(root.namespace.is_none());

    let single = doriac::parse_source("single.doria", "namespace Acme; function main(): void {}")
        .expect("single-segment namespace should parse")
        .namespace
        .expect("namespace should be retained");
    assert_eq!(single.name.canonical(), "Acme");
    assert!(!single.name.is_qualified());
    assert!(single.name.separator_spans.is_empty());

    let source = "namespace Acme\\Blog\\Domain; function main(): void {}";
    let multi = doriac::parse_source("multi.doria", source)
        .expect("multi-segment namespace should parse")
        .namespace
        .expect("namespace should be retained");
    assert_eq!(multi.name.canonical(), "Acme\\Blog\\Domain");
    assert_eq!(multi.name.segments.len(), 3);
    assert_eq!(multi.name.separator_spans.len(), 2);
    assert_eq!(
        &source[multi.name.separator_spans[0].start..multi.name.separator_spans[0].end],
        "\\"
    );
    assert_eq!(
        &source[multi.name.separator_spans[1].start..multi.name.separator_spans[1].end],
        "\\"
    );
    assert_eq!(
        &source[multi.semicolon_span.start..multi.semicolon_span.end],
        ";"
    );
}

#[test]
fn use_remains_a_callable_name_when_followed_by_arguments() {
    let source = r#"
namespace Acme;

use Acme\Value as ImportedValue;

class Value {}

function use(int $value): void {}

function main(): void
{
    use(42);
    let $value = new ImportedValue();
}
"#;

    doriac::check_source("contextual-use.doria", source)
        .expect("`use` calls must remain distinct from file-scope imports");
}

#[test]
fn include_preserves_both_literal_quote_forms() {
    let source = "include \"generated/one.doria\"; include 'generated/two.doria';";
    let program = doriac::parse_source("include.doria", source).expect("includes should parse");
    assert_eq!(program.includes.len(), 2);
    assert_eq!(program.includes[0].raw, "generated/one.doria");
    assert_eq!(program.includes[0].value, "generated/one.doria");
    assert_eq!(program.includes[1].raw, "generated/two.doria");
    assert_eq!(program.includes[1].value, "generated/two.doria");
    assert_ne!(program.includes[0].quote, program.includes[1].quote);
    for include in &program.includes {
        assert_eq!(
            &source[include.keyword_span.start..include.keyword_span.end],
            "include"
        );
        assert_eq!(
            &source[include.semicolon_span.start..include.semicolon_span.end],
            ";"
        );
    }
}

#[test]
fn parser_preserves_namespace_import_and_include_source_spans() {
    let source = r#"namespace Acme\Blog;
use Doria\Std\Math\{Vector2, Vector3 as Position3,};
include 'generated/routes.doria';
function main(): void {}
"#;
    let program = doriac::parse_source("test.doria", source).expect("syntax should parse");
    let namespace = program.namespace.expect("namespace should be retained");
    assert_eq!(namespace.name.canonical(), "Acme\\Blog");
    assert_eq!(namespace.name.segments.len(), 2);
    assert_eq!(namespace.name.separator_spans, vec![Span::new(14, 15)]);
    assert_eq!(
        &source[namespace.name.segments[0].span.start..namespace.name.segments[0].span.end],
        "Acme"
    );
    assert_eq!(
        &source[namespace.name.segments[1].span.start..namespace.name.segments[1].span.end],
        "Blog"
    );
    assert_eq!(
        &source[namespace.semicolon_span.start..namespace.semicolon_span.end],
        ";"
    );

    let import = &program.imports[0];
    assert_eq!(
        import.prefix.as_ref().unwrap().canonical(),
        "Doria\\Std\\Math"
    );
    assert_eq!(import.entries.len(), 2);
    assert_eq!(import.entries[1].alias.as_ref().unwrap().text, "Position3");
    assert_eq!(import.comma_spans.len(), 2);
    assert!(import.group_separator_span.is_some());
    assert!(import.group_open_span.is_some());
    assert!(import.group_close_span.is_some());

    let include = &program.includes[0];
    assert_eq!(include.raw, "generated/routes.doria");
    assert_eq!(include.value, "generated/routes.doria");
    assert_eq!(
        &source[include.keyword_span.start..include.keyword_span.end],
        "include"
    );
    assert_eq!(
        &source[include.literal_span.start..include.literal_span.end],
        "'generated/routes.doria'"
    );
}

#[test]
fn parser_rejects_malformed_namespace_import_and_include_forms_deliberately() {
    let cases = [
        ("namespace ;", "Namespace Name Is Missing"),
        (
            "namespace \\Acme;",
            "Leading Namespace Separator Is Not Supported",
        ),
        (
            "namespace Acme\\;",
            "Qualified Name Has A Trailing Separator",
        ),
        (
            "namespace Acme\\\\Blog;",
            "Qualified Name Has An Empty Segment",
        ),
        ("namespace Acme { }", "Braced Namespace Is Not Supported"),
        (
            "namespace Acme; namespace Other;",
            "Namespace Declaration Is Duplicated",
        ),
        (
            "class A {} namespace Other;",
            "Namespace Declaration Is Not First",
        ),
        (
            "namespace Acme function main(): void {}",
            "Directive Semicolon Is Missing",
        ),
        ("use ; function main(): void {}", "Import Target Is Missing"),
        ("use Acme\\Thing as ;", "Import Alias Is Missing"),
        ("use Acme\\Thing", "Directive Semicolon Is Missing"),
        ("use Acme\\*;", "Wildcard Import Is Not Supported"),
        ("use Acme\\{};", "Import Group Is Empty"),
        (
            "use Acme\\{Thing,,Other};",
            "Import Group Has An Empty Entry",
        ),
        ("use Acme\\{Thing;", "Import Group Is Not Closed"),
        ("use Acme\\{Thing,*};", "Wildcard Import Is Not Supported"),
        ("use function Acme\\make;", "Import Kind Is Not Supported"),
        ("use const Acme\\LIMIT;", "Import Kind Is Not Supported"),
        (
            "include ; function main(): void {}",
            "Include Path Is Missing",
        ),
        (
            "include \"x.doria\" function main(): void {}",
            "Directive Semicolon Is Missing",
        ),
        ("include $path;", "Include Path Must Be A Literal"),
        (
            "include \"generated/{$path}.doria\";",
            "Include Path Must Be Constant",
        ),
        (
            "include \"https://example.com/source.doria\";",
            "Remote Include Is Not Supported",
        ),
        (
            "function f(): void { use Acme\\Thing; }",
            "Import Is Not At File Scope",
        ),
        (
            "function f(): void { include \"x.doria\"; }",
            "Include Is Not At File Scope",
        ),
    ];

    for (source, title) in cases {
        let diagnostics = doriac::parse_source("test.doria", source)
            .expect_err("malformed source should be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.title == title),
            "expected `{title}` for `{source}`, got {diagnostics:#?}"
        );
    }
}

#[test]
fn malformed_file_directives_recover_to_later_declarations_without_cascades() {
    for source in [
        "use Acme\\{Thing,,Other}; function main(): void {}",
        "include $path; function main(): void {}",
        "namespace Acme { } function main(): void {}",
    ] {
        let diagnostics = doriac::parse_source("recovery.doria", source)
            .expect_err("malformed directive should be diagnosed");
        assert!(
            diagnostics.len() <= 2,
            "directive recovery cascaded for `{source}`: {diagnostics:#?}"
        );
        assert!(diagnostics.iter().all(|diagnostic| {
            !diagnostic.message.contains("main") && !diagnostic.message.contains("function")
        }));
    }
}

#[test]
fn leading_separator_fix_is_available_in_type_and_value_positions() {
    for source in [
        "function f(\\Acme\\Value $value): void {}",
        "function main(): void { \\Acme\\run(); }",
    ] {
        let diagnostics = doriac::parse_source("test.doria", source)
            .expect_err("leading separators should be rejected");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.title == "Leading Namespace Separator Is Not Supported")
            .expect("leading separator diagnostic should be present");
        assert_eq!(diagnostic.fix.as_ref().unwrap().replacement, "");
    }
}

#[test]
fn php_source_composition_fix_only_applies_to_exact_file_scope_literal_statements() {
    for spelling in ["require", "require_once", "include_once"] {
        let source = format!("{spelling} \"generated/source.doria\";");
        let diagnostics = doriac::parse_source("test.doria", source)
            .expect_err("PHP spelling should be rejected");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.title == "Use Doria Include Syntax")
            .expect("migration diagnostic should be present");
        assert_eq!(diagnostic.fix.as_ref().unwrap().replacement, "include");
    }

    doriac::parse_source(
        "test.doria",
        r#"function require(string $path): void {}
function main(): void { require("user-data"); }
"#,
    )
    .expect("a user function call must not be mistaken for PHP source composition");
}

#[test]
fn same_file_resolution_uses_canonical_package_owned_identities() {
    let context = CompilationContext {
        edition: Edition::Doria2026,
        package: PackageIdentity::named("acme/demo", Span::default()).unwrap(),
        source: SourceIdentity("src/main.doria".to_string()),
    };
    let (_, analysis) = doriac::analyze_source_for_ide_with_context(
        "src/main.doria",
        SAME_FILE_SOURCE,
        context.clone(),
    )
    .expect("same-file namespace imports should analyze");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.compilation_context, context);
    assert_eq!(
        analysis.info.global_symbols.namespace.as_deref(),
        Some("Acme\\Demo")
    );

    let declarations = &analysis.info.global_symbols.declarations;
    for (name, kind) in [
        ("Acme\\Demo\\Counter", GlobalSymbolKind::Class),
        ("Acme\\Demo\\Status", GlobalSymbolKind::Enum),
        ("Acme\\Demo\\answer", GlobalSymbolKind::Function),
        ("Acme\\Demo\\main", GlobalSymbolKind::Function),
        ("Acme\\Demo\\ANSWER", GlobalSymbolKind::Constant),
    ] {
        let declaration = declarations
            .iter()
            .find(|declaration| declaration.qualified_name == name)
            .unwrap_or_else(|| panic!("missing declaration `{name}`"));
        assert_eq!(declaration.kind, kind);
        assert_eq!(
            declaration.source_identity,
            SourceIdentity("src/main.doria".to_string())
        );
        assert_eq!(
            declaration.id.owner,
            GlobalSymbolOwner::Package(PackageIdentity::Named("acme/demo".to_string()))
        );
    }

    let references = &analysis.info.global_symbols.references;
    assert!(references.iter().any(|reference| {
        reference.symbol_id.qualified_name == "Acme\\Demo\\Counter"
            && reference.role == GlobalReferenceRole::Constructor
            && reference.import_alias.as_deref() == Some("DemoCounter")
    }));
    assert!(references.iter().any(|reference| {
        reference.symbol_id.qualified_name == "Acme\\Demo\\answer"
            && reference.role == GlobalReferenceRole::FunctionCall
            && reference.import_alias.as_deref() == Some("importedAnswer")
    }));
}

#[test]
fn exact_same_file_imports_do_not_conflict_with_their_declarations() {
    let source = r#"namespace Acme\Imports;
use Acme\Imports\Individual;
use Acme\Imports\{GroupedOne, GroupedTwo};
class Individual {}
class GroupedOne {}
class GroupedTwo {}
function main(): void
{
    let $individual = new Individual();
    let $one = new GroupedOne();
    let $two = new GroupedTwo();
}
"#;

    let (_, analysis) = doriac::analyze_source_for_ide("same-file-imports.doria", source)
        .expect("exact same-file imports should analyze");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.global_symbols.imports.len(), 3);
    assert!(analysis
        .info
        .global_symbols
        .references
        .iter()
        .any(|reference| {
            reference.role == GlobalReferenceRole::ImportTarget
                && reference.symbol_id.qualified_name == "Acme\\Imports\\Individual"
        }));
}

#[test]
fn package_identity_and_namespace_are_independent() {
    let analyze = |package: &str| {
        let context = CompilationContext {
            edition: Edition::Doria2026,
            package: PackageIdentity::named(package, Span::default()).unwrap(),
            source: SourceIdentity(format!("{package}/main.doria")),
        };
        doriac::analyze_source_for_ide_with_context(
            "main.doria",
            "namespace Shared; class Value {} function main(): void {}",
            context,
        )
        .unwrap()
        .1
        .info
        .global_symbols
        .declarations
        .into_iter()
        .find(|declaration| declaration.qualified_name == "Shared\\Value")
        .unwrap()
        .id
    };

    let first = analyze("one/package");
    let second = analyze("two/package");
    assert_eq!(first.qualified_name, second.qualified_name);
    assert_ne!(first.owner, second.owner);
}

#[test]
fn external_symbols_and_includes_use_slice_two_boundaries_without_cascades() {
    for (source, code) in [
        (
            "use Other\\Package\\Value; function main(): void {}",
            EXTERNAL_SYMBOL_BOUNDARY_CODE,
        ),
        (
            "function main(): void { let $value = new Other\\Package\\Value(); }",
            EXTERNAL_SYMBOL_BOUNDARY_CODE,
        ),
        (
            "include \"generated/routes.doria\"; function main(): void {}",
            INCLUDE_BOUNDARY_CODE,
        ),
    ] {
        let diagnostics = doriac::check_source("test.doria", source)
            .expect_err("unresolved graph input should stop before lowering");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].code, code);
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::UnsupportedDevelopmentSurface
        );
        assert!(diagnostics[0].development_only);
        assert_ne!(diagnostics[0].code, "E0641");
    }
}

#[test]
fn unresolved_import_aliases_preserve_every_tooling_occurrence_without_cascades() {
    let source = r#"namespace Acme\App;
use Other\Model\User as ModelUser;
function inspect(ModelUser $user): void {}
function main(): void { let $user = new ModelUser(); }
"#;
    let context = CompilationContext {
        edition: Edition::Doria2026,
        package: PackageIdentity::SyntheticTooling("test-workspace".to_string()),
        source: SourceIdentity("app.doria".to_string()),
    };
    let (_, analysis) =
        doriac::analyze_source_for_ide_with_context("app.doria", source, context).unwrap();
    assert_eq!(analysis.diagnostics.len(), 1, "{:#?}", analysis.diagnostics);
    assert_eq!(analysis.diagnostics[0].code, EXTERNAL_SYMBOL_BOUNDARY_CODE);

    let occurrences = analysis
        .info
        .global_symbols
        .unresolved
        .iter()
        .filter(|reference| reference.source_spelling == "Other\\Model\\User")
        .collect::<Vec<_>>();
    assert_eq!(occurrences.len(), 3, "{occurrences:#?}");
    assert_eq!(occurrences[0].role, GlobalReferenceRole::ImportTarget);
    assert_eq!(occurrences[0].import_alias, None);
    assert!(occurrences[1..]
        .iter()
        .all(|reference| reference.import_alias.as_deref() == Some("ModelUser")));
}

#[test]
fn unqualified_unknowns_remain_language_diagnostics() {
    let diagnostics = doriac::check_source(
        "test.doria",
        "function main(): void { let $value = new MissingType(); }",
    )
    .expect_err("unknown unqualified type should fail");
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic.kind == DiagnosticKind::Language
            && diagnostic.code != EXTERNAL_SYMBOL_BOUNDARY_CODE
            && diagnostic.code != "E0641"
    }));
}

#[test]
fn namespaced_entrypoint_resolves_through_hir_and_mir() {
    let hir = doriac::lower_source("test.doria", SAME_FILE_SOURCE)
        .expect("same-file imports should lower to HIR");
    assert_eq!(hir.namespace.as_ref().unwrap().name, "Acme\\Demo");
    assert!(hir
        .semantic_info
        .global_symbols
        .imports
        .iter()
        .any(|import| { import.alias == "DemoCounter" && import.target == "Acme\\Demo\\Counter" }));
    assert!(hir.items.iter().any(|item| {
        matches!(item, doriac::hir::Item::Function(function) if function.name == "Acme\\Demo\\main")
    }));

    let mir = doriac::lower_source_to_mir("test.doria", SAME_FILE_SOURCE)
        .expect("same-file imports should lower to MIR");
    assert_eq!(mir.functions[mir.entry.0].name, "Acme\\Demo\\main");
    assert!(mir
        .functions
        .iter()
        .any(|function| function.name == "Acme\\Demo\\answer"));
}

#[test]
fn context_aware_api_preserves_existing_standalone_defaults() {
    let source = "function main(): void { echo \"ok\\n\"; }";
    let standalone = doriac::lower_source("test.doria", source).unwrap();
    assert_eq!(
        standalone.semantic_info.compilation_context,
        CompilationContext::standalone("test.doria")
    );

    let context = CompilationContext {
        edition: Edition::Doria2026,
        package: PackageIdentity::Named("acme/tool".to_string()),
        source: SourceIdentity("src/tool.doria".to_string()),
    };
    let contextual = doriac::lower_source_with_context("test.doria", source, context.clone())
        .expect("context-aware lowering should work");
    assert_eq!(contextual.semantic_info.compilation_context, context);
}

#[test]
fn invalid_compiler_inputs_are_diagnosed_deliberately() {
    assert!(Edition::parse("future", Span::new(1, 2))
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == "E0673"));
    assert!(PackageIdentity::named("Not/A-Package", Span::new(2, 3))
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == "E0674"));
}

#[test]
fn namespace_segments_follow_the_pascal_case_naming_charter() {
    doriac::check_source(
        "valid-namespace.doria",
        "namespace Acme\\Http; function main(): void {}",
    )
    .expect("PascalCase namespace segments with folded acronyms should be accepted");

    let source = "namespace acme\\HTTP; function main(): void {}";
    let diagnostics = doriac::check_source("invalid-namespace.doria", source)
        .expect_err("every invalid namespace segment should be diagnosed");
    let naming = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0675")
        .collect::<Vec<_>>();
    assert_eq!(naming.len(), 2, "{diagnostics:#?}");
    assert_eq!(&source[naming[0].span.start..naming[0].span.end], "acme");
    assert_eq!(&source[naming[1].span.start..naming[1].span.end], "HTTP");
}

#[test]
fn parsed_global_declaration_name_spans_are_exact() {
    let source = "namespace Acme; class Value {} function main(): void {}";
    let program = doriac::parse_source("test.doria", source).unwrap();
    let Item::Class(class) = &program.items[0] else {
        panic!("expected class declaration");
    };
    assert_eq!(&source[class.name_span.start..class.name_span.end], "Value");
    let Item::Function(function) = &program.items[1] else {
        panic!("expected function declaration");
    };
    assert_eq!(
        &source[function.name_span.start..function.name_span.end],
        "main"
    );
}

#[test]
fn namespaced_class_list_algorithms_keep_one_canonical_element_identity() {
    let source = include_str!("../../../examples/native/main_stage31_namespaces.doria");
    let hir = doriac::lower_source("stage31-namespaces.doria", source)
        .expect("namespace parity fixture should lower to HIR");
    let mir = doriac::mir_lowering::lower_program(&hir)
        .expect("namespace parity fixture should lower to MIR");

    doriac::mir_validation::validate_program(&mir).unwrap_or_else(|error| {
        let plans = mir
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.statements)
            .filter_map(|plan| match plan {
                doriac::mir::Statement::ControlFlowPlan(
                    doriac::mir::ControlFlowPlan::ListAlgorithm(plan),
                ) => Some(plan.as_ref()),
                _ => None,
            })
            .collect::<Vec<_>>();
        panic!(
            "{error:?}; List algorithm plans: {plans:#?}; function types: {:#?}; collections: {:#?}",
            mir.function_types, mir.collection_types
        );
    });
}

#[test]
fn resolver_applies_one_exact_precedence_chain_without_qualified_fallback() {
    let source = r#"
namespace Acme;

use Acme\Imported\Comparable as Comparable;

class Local {}
class AcmeComparable {}
function main(): void
{
    let $local = new Local();
    let $absolute = new Acme\Local();
    let $external = new Other\Local();
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("precedence.doria", source)
        .expect("IDE analysis should preserve boundary facts");
    let diagnostics = &analysis.diagnostics;
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == EXTERNAL_SYMBOL_BOUNDARY_CODE
            && diagnostic.message.contains("Acme\\Imported\\Comparable")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == EXTERNAL_SYMBOL_BOUNDARY_CODE
            && diagnostic.message.contains("Other\\Local")
    }));
    assert!(!diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("Acme\\Other\\Local")
            || diagnostic.message.contains("Acme\\Acme\\Local")
    }));
}

#[test]
fn one_resolver_canonicalizes_every_name_bearing_semantic_role() {
    let source = r#"
namespace Acme\Roles;

interface Contract {}
class Base {}
class Failure implements Error
{
    string $message = "failure";
}
enum State { case Ready; }

class Child<T implements Contract> extends Base implements Contract
{
    function apply(
        function(Child): (function(Child): Child throws Failure) $callback,
        Child $value,
    ): Child throws Failure
    {
        try {
            let $created = new Child();
            let $state = State::Ready;
            if ($created is Child) {
                return match ($created) {
                    Child $matched => $matched,
                };
            }
            return Child::identity($value);
        } catch (Failure) {
            return $value;
        }
    }

    static function identity(Child $value): Child { return $value; }
}
"#;
    let program = doriac::parse_source("roles.doria", source)
        .expect("all accepted name-bearing roles should parse");
    let context = CompilationContext::standalone("roles.doria");
    let analysis = doriac::names::resolve_program_for_ide(&program, &context);
    assert!(
        analysis.diagnostics.is_empty(),
        "same-file role resolution should not require a graph: {:#?}",
        analysis.diagnostics
    );

    let roles = analysis
        .resolved
        .facts
        .references
        .iter()
        .map(|reference| (reference.role, reference.symbol_id.qualified_name.as_str()))
        .collect::<Vec<_>>();
    for (role, canonical) in [
        (GlobalReferenceRole::Extends, "Acme\\Roles\\Base"),
        (GlobalReferenceRole::Implements, "Acme\\Roles\\Contract"),
        (GlobalReferenceRole::Throws, "Acme\\Roles\\Failure"),
        (GlobalReferenceRole::Catch, "Acme\\Roles\\Failure"),
        (GlobalReferenceRole::Constructor, "Acme\\Roles\\Child"),
        (GlobalReferenceRole::StaticQualifier, "Acme\\Roles\\State"),
        (GlobalReferenceRole::StaticQualifier, "Acme\\Roles\\Child"),
        (GlobalReferenceRole::TypeTest, "Acme\\Roles\\Child"),
        (GlobalReferenceRole::MatchPattern, "Acme\\Roles\\Child"),
    ] {
        assert!(
            roles.contains(&(role, canonical)),
            "missing {role:?} reference to `{canonical}` in {roles:#?}"
        );
    }

    let child_type_references = roles
        .iter()
        .filter(|(role, canonical)| {
            *role == GlobalReferenceRole::Type && *canonical == "Acme\\Roles\\Child"
        })
        .count();
    assert!(
        child_type_references >= 6,
        "nested function types, parameters, returns, and constraints must share the resolver"
    );
}

#[test]
fn canonical_io_requires_qualification_or_an_explicit_import() {
    let diagnostics = doriac::check_source(
        "implicit-io-error.doria",
        "function write(): void throws IoError {}",
    )
    .expect_err("the standard I/O module must not create an implicit short alias");
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic.code != EXTERNAL_SYMBOL_BOUNDARY_CODE && diagnostic.code != "E0641"
    }));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("unknown type `IoError`")));

    doriac::check_source(
        "aliased-io-error.doria",
        r#"use Doria\Std\Io\IoError as OutputError;
function write(): void throws OutputError { echo "value"; }
"#,
    )
    .expect("an explicit canonical I/O alias should resolve through the general resolver");
}

#[test]
fn import_collisions_are_exact_and_report_both_locations() {
    for source in [
        "namespace Acme; class Value {} use Acme\\Other as Value;",
        "use Acme\\One as Alias; use Acme\\Two as Alias;",
        "use Acme\\One; use Acme\\One;",
        "use Acme\\One as read_line;",
        "use Acme\\One as List;",
    ] {
        let diagnostics = doriac::check_source("collisions.doria", source)
            .expect_err("import collision should be rejected");
        let collision = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "E0670")
            .unwrap_or_else(|| panic!("missing E0670 for `{source}`: {diagnostics:#?}"));
        if collision.title.contains("Duplicate") || collision.title.contains("Conflicts") {
            assert!(
                !collision.related.is_empty(),
                "collision should identify its earlier occurrence: {collision:#?}"
            );
        }
    }

    doriac::check_source(
        "case-sensitive.doria",
        "namespace Acme; class Value {} class value {} function main(): void {}",
    )
    .expect("global names are exact and case-sensitive");
}

#[test]
fn edition_2026_prelude_is_explicit_unique_and_matches_compiler_facts() {
    let entries = edition_prelude(Edition::Doria2026);
    let names = entries
        .iter()
        .map(|entry| entry.name)
        .collect::<HashSet<_>>();
    assert_eq!(names.len(), entries.len(), "prelude aliases must be unique");
    for required in [
        "Displayable",
        "Int",
        "String",
        "List",
        "Dictionary",
        "Set",
        "Bytes",
        "SharedReference",
        "WritableSharedReferenceAccess",
    ] {
        assert!(
            names.contains(required),
            "missing prelude entry `{required}`"
        );
    }
    assert!(!names.contains("Doria\\Std\\Console"));
    assert!(!names.contains("Vector3"));

    let facts = compiler_known_symbol_facts(Edition::Doria2026);
    for entry in entries {
        let matching = facts
            .iter()
            .filter(|fact| {
                fact.source_name == entry.name
                    && fact.provenance
                        == CompilerKnownProvenance::EditionPrelude(Edition::Doria2026)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matching.len(),
            1,
            "prelude entry `{}` must have one identity",
            entry.name
        );
        assert!(matches!(
            matching[0].id.owner,
            GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::Prelude(_))
        ));
    }
}

#[test]
fn ordinary_prelude_names_shadow_but_reserved_names_and_intrinsics_do_not() {
    doriac::check_source(
        "ordinary-shadow.doria",
        "namespace Acme; class Comparable {} function main(): void { let $value = new Comparable(); }",
    )
    .expect("an ordinary prelude convenience may be shadowed in the current namespace");

    for source in [
        "namespace Acme; class List {} function main(): void {}",
        "namespace Acme; function read_line(): void {} function main(): void {}",
    ] {
        doriac::check_source("reserved-shadow.doria", source)
            .expect_err("reserved compiler-known names must remain protected");
    }
}

#[test]
fn compiler_context_reaches_check_hir_mir_and_backend_emission() {
    let source = "namespace Acme\\Tool; function main(): void {}";
    let context = CompilationContext {
        edition: Edition::Doria2026,
        package: PackageIdentity::Named("acme/tool".to_string()),
        source: SourceIdentity("src/main.doria".to_string()),
    };
    doriac::check_source_with_context("src/main.doria", source, context.clone())
        .expect("context-aware checking should succeed");
    let hir = doriac::lower_source_with_context("src/main.doria", source, context.clone())
        .expect("context-aware HIR lowering should succeed");
    assert_eq!(hir.semantic_info.compilation_context, context);
    assert_eq!(hir.namespace.as_ref().unwrap().name, "Acme\\Tool");

    let mir = doriac::lower_source_to_mir_with_context("src/main.doria", source, context.clone())
        .expect("context-aware MIR lowering should succeed");
    assert_eq!(mir.compilation_context, context);
    assert_eq!(mir.namespace.as_deref(), Some("Acme\\Tool"));
    assert_eq!(mir.functions[mir.entry.0].name, "Acme\\Tool\\main");

    let output = doriac::compile_source_with_context(
        "src/main.doria",
        source,
        context,
        CompileOptions::new(BackendTarget::Debug),
    )
    .expect("context-aware backend emission should succeed");
    assert!(matches!(output, BackendOutput::Text { .. }));
}

#[test]
fn generated_php_executes_namespaced_main_and_keeps_imports_compile_time_only() {
    if !php_available() {
        return;
    }
    let source = include_str!("../../../examples/native/main_stage31_namespaces.doria");
    let php = doriac::compile_source_to_php("stage31-namespaces.doria", source)
        .expect("namespace fixture should compile to PHP");
    assert!(!php.contains("spl_autoload_register"));
    assert!(!php.contains("include \"generated"));
    assert!(!php.contains("require \"generated"));
    let lint_path =
        std::env::temp_dir().join(format!("doria-stage31-lint-{}.php", std::process::id()));
    fs::write(&lint_path, &php).expect("generated PHP should be writable");
    let lint = Command::new("php")
        .arg("-l")
        .arg(&lint_path)
        .output()
        .expect("PHP lint should start");
    let _ = fs::remove_file(lint_path);
    assert!(
        lint.status.success(),
        "{}",
        String::from_utf8_lossy(&lint.stderr)
    );

    let run = run_php_file("namespaced-main", &php);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"40 ready 2 40 40 7\n");
    assert!(run.stderr.is_empty());
}

#[test]
fn generated_php_keeps_case_distinct_namespaced_symbols_distinct() {
    if !php_available() {
        return;
    }
    let source = r#"
namespace Acme;
class Value { function label(): string { return "upper"; } }
class value { function label(): string { return "lower"; } }
function main(): void throws Doria\Std\Io\IoError
{
    let $upper = new Value();
    let $lower = new value();
    echo "{$upper->label()} {$lower->label()}\n";
}
"#;
    let php = doriac::compile_source_to_php("case-distinct.doria", source)
        .expect("case-distinct canonical names should compile");
    let run = run_php_file("case-distinct", &php);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"upper lower\n");
}

#[test]
fn namespaced_php_integer_entry_preserves_process_status() {
    if !php_available() {
        return;
    }
    let php = doriac::compile_source_to_php(
        "status.doria",
        "namespace Acme; function main(): int { return 7; }",
    )
    .expect("namespaced integer entry should compile");
    let run = run_php_file("status", &php);
    assert_eq!(run.status.code(), Some(7));
    assert!(run.stdout.is_empty());
    assert!(run.stderr.is_empty());
}

#[test]
fn resolver_scaling_is_structural_and_deterministic() {
    let mut source = String::from("namespace Segment0");
    for index in 1..64 {
        source.push_str(&format!("\\Segment{index}"));
    }
    source.push_str(";\n");
    for index in 0..128 {
        source.push_str(&format!("class Value{index} {{}}\n"));
    }
    source.push_str("function main(): void {\n");
    for index in 0..128 {
        source.push_str(&format!("let $value{index} = new Value{index}();\n"));
    }
    source.push_str("}\n");

    let first = doriac::analyze_source_for_ide("scaling.doria", &source)
        .expect("large same-file namespace source should analyze")
        .1
        .info
        .global_symbols;
    let second = doriac::analyze_source_for_ide("scaling.doria", &source)
        .expect("repeated analysis should succeed")
        .1
        .info
        .global_symbols;
    assert_eq!(first, second);
    assert_eq!(first.declarations.len(), 129);
    assert_eq!(
        first
            .references
            .iter()
            .filter(|reference| reference.role == GlobalReferenceRole::Constructor)
            .count(),
        128
    );
}
