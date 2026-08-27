use doriac::ast::{AttributeTargetKind, AttributeTargetRole};
use doriac::attributes::{AttributeClassIdentity, AttributeValueKind};
use doriac::build_plan::{
    BuildNativeProfile, BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget, Dependency,
    DependencyKind, NamespaceMapping, Package, SelectedTarget, Source, SourceOrigin, SourceScope,
    TargetKind,
};
use doriac::compilation_graph::load_compilation_graph;
use doriac::lexer::TokenKind;
use doriac::source_provider::InMemorySourceProvider;

fn graph_source(package: &str, path: &str, origin: SourceOrigin) -> Source {
    Source {
        identity: format!("{package}:{path}"),
        path: path.to_string(),
        scope: SourceScope::Main,
        origin,
        generated_for: None,
    }
}

fn graph_package(identity: &str, sources: Vec<Source>, dependencies: Vec<Dependency>) -> Package {
    Package {
        identity: identity.to_string(),
        root: ".".to_string(),
        namespace_mappings: vec![NamespaceMapping {
            prefix: String::new(),
            path: String::new(),
            scope: SourceScope::Main,
            generated_for: None,
        }],
        sources,
        dependencies,
    }
}

fn graph_plan(packages: Vec<Package>, entry: &str) -> BuildPlanDocument {
    BuildPlanDocument {
        path: "plan.json".to_string(),
        directory: std::env::current_dir().expect("current directory"),
        text: String::new(),
        plan: BuildPlan {
            schema_version: 1,
            edition: "2026".to_string(),
            root_package: "acme/application".to_string(),
            selected_target: SelectedTarget {
                package: "acme/application".to_string(),
                name: "application".to_string(),
                kind: TargetKind::Binary,
                entry_source: Some(entry.to_string()),
                active_scopes: vec![SourceScope::Main],
            },
            packages,
            compiler: CompilerOptions {
                target: CompilerTarget::Native,
                native_profile: Some(BuildNativeProfile::Fast),
                target_triple: None,
            },
        },
    }
}

#[test]
fn adjacent_attribute_opening_does_not_change_hash_comments_or_strings() {
    let tokens = doriac::lex_source(
        "attributes.doria",
        "#[Test]\n# comment\n# [Test]\n\"#[Test]\"\n'#[Test]'\n// #[Test]\n/* #[Test] */\n",
    )
    .expect("attribute and comment forms should lex");
    assert_eq!(
        tokens
            .iter()
            .filter(|token| token.kind == TokenKind::AttributeOpen)
            .count(),
        1
    );
    assert_eq!(
        tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::StringLiteral { .. }))
            .count(),
        2
    );
}

#[test]
fn parser_preserves_attribute_groups_arguments_and_promoted_roles() {
    let source = r#"
#[Attribute]
class Field {}

#[Field]
class User
{
    function __construct(
        #[Field] string $name,
    ) {}
}

#[Test, Field(),]
function main(): void {}
"#;
    let program = doriac::parse_source("attributes.doria", source)
        .expect("attribute-bearing declarations should parse");
    assert_eq!(program.attributes.len(), 4);
    let promoted = program
        .attributes
        .iter()
        .find(|attachment| attachment.target.kind == AttributeTargetKind::Parameter)
        .expect("constructor parameter carries an attribute");
    assert_eq!(
        promoted.target.roles,
        vec![
            AttributeTargetRole::Parameter,
            AttributeTargetRole::PromotedProperty
        ]
    );
    let main = program.attributes.last().expect("main attributes exist");
    assert_eq!(main.groups[0].attributes.len(), 2);
    assert_eq!(main.groups[0].comma_spans.len(), 2);
    assert!(main.groups[0].attributes[1].argument_list.is_some());
}

#[test]
fn parser_attaches_attributes_to_the_complete_stage32_target_surface() {
    let source = r#"
#[Test]
class Example
{
    #[Test] const VALUE = 1;
    #[Test] string $name;
    #[Test] function method(#[Test] string $value): void {}
    #[Test] function __construct(#[Test] string $promoted) {}
    #[Test] function __destruct() {}
}

#[Test]
enum Choice
{
    #[Test] case Text(#[Test] string $value);
}

#[Test] interface Contract {}
#[Test] trait Behavior { #[Test] function helper(): void {} }
#[Test] function helper(#[Test] string $value): void {}
#[Test] const GLOBAL_VALUE = 1;
"#;
    let program = doriac::parse_source("targets.doria", source)
        .expect("every Stage 32 declaration target should parse");
    let kinds = program
        .attributes
        .iter()
        .map(|attachment| attachment.target.kind)
        .collect::<Vec<_>>();
    for expected in [
        AttributeTargetKind::Class,
        AttributeTargetKind::Enum,
        AttributeTargetKind::Interface,
        AttributeTargetKind::Trait,
        AttributeTargetKind::Function,
        AttributeTargetKind::Constant,
        AttributeTargetKind::Property,
        AttributeTargetKind::Method,
        AttributeTargetKind::Constructor,
        AttributeTargetKind::Destructor,
        AttributeTargetKind::ClassConstant,
        AttributeTargetKind::Parameter,
        AttributeTargetKind::EnumCase,
        AttributeTargetKind::EnumPayloadField,
    ] {
        assert!(
            kinds.contains(&expected),
            "missing target kind {expected:?}"
        );
    }
}

#[test]
fn malformed_and_misplaced_attributes_recover_with_deliberate_diagnostics() {
    for (source, title) in [
        ("#[] function valid(): void {}", "Empty Attribute Group"),
        (
            "internal #[Test] function invalid(): void {} function valid(): void {}",
            "Attribute Must Precede Declaration Modifiers",
        ),
        (
            "#[Test] echo \"invalid\"; function valid(): void {}",
            "Attribute Is Not Valid On This Target",
        ),
        (
            "function owner(): void { #[Test] let $value = 1; } function valid(): void {}",
            "Attribute Is Not Valid On This Target",
        ),
    ] {
        let diagnostics = doriac::parse_source("recovery.doria", source)
            .expect_err("misplaced or malformed attributes must fail deliberately");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.title == title),
            "{source}: {diagnostics:#?}"
        );
    }
}

#[test]
fn semantic_attributes_bind_named_arguments_and_defaults_without_execution() {
    let source = r#"
#[Attribute]
class Route
{
    function __construct(string $path, int32 $status = 200)
    {
        panic("attribute constructors are runtime-only");
    }
}

#[Route(status: 201, path: "/posts")]
#[Route(path: "/health")]
#[Test]
function main(): void
{
    echo "ok\n";
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("attributes.doria", source)
        .expect("source should be accepted by compiler input processing");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let route = analysis
        .info
        .attributes
        .schemas
        .iter()
        .find(|schema| schema.canonical_name == "Route")
        .expect("user attribute schema is recorded");
    assert!(matches!(route.identity, AttributeClassIdentity::User(_)));
    assert_eq!(route.parameters.len(), 2);
    assert_eq!(analysis.info.attributes.applications.len(), 3);

    let defaulted = &analysis.info.attributes.applications[1];
    assert_eq!(defaulted.bound_arguments[0].parameter_name, "path");
    assert_eq!(defaulted.bound_arguments[1].parameter_name, "status");
    assert!(defaulted.bound_arguments[1].defaulted);
    assert!(matches!(
        defaulted.bound_arguments[1].value.value,
        AttributeValueKind::Integer { ref value } if value == "200"
    ));
}

#[test]
fn invalid_attribute_classes_and_runtime_values_produce_no_partial_metadata() {
    let source = r#"
class Ordinary {}

#[Attribute]
class Route
{
    function __construct(string $path) {}
}

function runtimePath(): string { return "/"; }

#[Ordinary]
#[Route(runtimePath())]
function main(): void {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("invalid-attributes.doria", source)
        .expect("IDE analysis should recover semantic diagnostics");
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0687"));
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0693"));
    assert!(analysis.info.attributes.applications.is_empty());
}

#[test]
fn attribute_schema_and_binding_failures_are_causal_and_leave_no_partial_records() {
    let source = r#"
#[Attribute]
function notAClass(): void {}

#[Attribute(repeatable: true)]
class MarkerArguments {}

#[Attribute]
class GenericAttribute<T> {}

#[Attribute]
class WritableAttribute { function __construct(writable string $value) {} }

#[Attribute]
class OwnedAttribute { function __construct(take string $value) {} }

#[Attribute]
class CollectionAttribute { function __construct(List<int> $values) {} }

#[Attribute]
class Route { function __construct(string $path, int $status) {} }

class Ordinary {}

#[Ordinary] function unmarked(): void {}
#[Route(path: "/")] function missing(): void {}
#[Route(path: "/", unknown: 200)] function unknown(): void {}
#[Route(path: "/", status: 200, status: 201)] function duplicate(): void {}
#[Route(path: 1, status: 200)] function wrongType(): void {}
#[Test(1)] function markerOverflow(): void {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("invalid-schema.doria", source)
        .expect("IDE analysis should retain semantic failures");
    for code in [
        "E0688", "E0689", "E0690", "E0691", "E0692", "E0687", "E0516", "E0517", "E0518", "E0403",
        "E0695",
    ] {
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "missing {code}: {:#?}",
            analysis.diagnostics
        );
    }
    assert!(analysis.info.attributes.applications.is_empty());
}

#[test]
fn metadata_document_is_strict_deterministic_typed_and_runtime_free() {
    let source = r#"
#[Attribute]
class Route
{
    function __construct(string $path, int32 $status = 200) {}
}

#[Route(status: 201, path: "/posts")]
#[Test]
function main(): void
{
    echo "ok\n";
}
"#;
    let first = doriac::metadata_source("/private/work/main.doria", source)
        .expect("metadata should be produced");
    let second = doriac::metadata_source("/private/work/main.doria", source)
        .expect("metadata should be deterministic");
    let first_json = serde_json::to_string_pretty(&first).unwrap();
    let second_json = serde_json::to_string_pretty(&second).unwrap();
    assert_eq!(first_json, second_json);
    assert_eq!(first.schema_version, 1);
    assert_eq!(first.applications.len(), 2);
    assert!(!first_json.contains("/private/work"));
    assert!(!first_json.contains(":Function"));
    assert!(!first_json.contains("PromotedProperty"));
    assert!(first_json.contains(r#""kind": "integer""#));
    assert!(first_json.contains(r#""type": "int32""#));

    let decoded: doriac::attributes::AttributeMetadataDocumentV1 =
        serde_json::from_str(&first_json).expect("public metadata must round trip strictly");
    assert_eq!(decoded, first);

    let hir = doriac::lower_source("attributes.doria", source).expect("HIR should lower");
    assert_eq!(hir.attribute_metadata.applications.len(), 2);
    let mir = doriac::lower_source_to_mir("attributes.doria", source)
        .expect("metadata-only attributes must not add a MIR operation")
        .to_string();
    assert!(!mir.contains("Attribute"));
    assert!(!mir.contains("PHPExport"));
    let php_source = "#[Test] function main(): void { echo \"ok\\n\"; }";
    let php = doriac::compile_source_to_php("attributes.doria", php_source)
        .expect("PHP compatibility output should compile");
    assert!(!php.contains("#["));
    assert!(!php.contains("PHPExport"));
}

#[test]
fn unknown_attribute_is_language_or_compiler_input_according_to_graph_completeness() {
    let (_, analysis) =
        doriac::analyze_source_for_ide("partial.doria", "#[Missing] function main(): void {}")
            .expect("partial IDE analysis should recover");
    let diagnostic = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0686")
        .expect("missing attribute receives a compiler-owned diagnostic");
    assert_eq!(
        diagnostic.kind,
        doriac::diagnostics::DiagnosticKind::CompilerInput
    );
}

#[test]
fn namespaced_dependency_attributes_use_the_shared_graph_resolver() {
    let entry = "acme/application:main.doria";
    let document = graph_plan(
        vec![
            graph_package(
                "acme/application",
                vec![graph_source(
                    "acme/application",
                    "main.doria",
                    SourceOrigin::Entry,
                )],
                vec![Dependency {
                    package: "acme/metadata".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            graph_package(
                "acme/metadata",
                vec![graph_source(
                    "acme/metadata",
                    "Acme/Metadata/Route.doria",
                    SourceOrigin::Explicit,
                )],
                Vec::new(),
            ),
        ],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        concat!(
            "use Acme\\Metadata\\Route as WebRoute;\n",
            "#[WebRoute(path: \"/posts\")]\n",
            "function main(): void {}\n",
        ),
    );
    provider.insert(
        "acme/metadata",
        "Acme/Metadata/Route.doria",
        concat!(
            "namespace Acme\\Metadata;\n",
            "#[Attribute]\n",
            "class Route { function __construct(string $path) {} }\n",
        ),
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid attribute graph");
    let metadata = doriac::metadata_compilation_graph(&graph)
        .expect("dependency attribute should resolve through the graph");
    let route = metadata
        .attribute_classes
        .iter()
        .find(|schema| schema.canonical_name == "Acme\\Metadata\\Route")
        .expect("canonical dependency schema");
    assert_eq!(route.identity, "Acme\\Metadata\\Route");
    assert_eq!(metadata.applications.len(), 1);
    assert_eq!(
        metadata.applications[0].attribute_class,
        "Acme\\Metadata\\Route"
    );
}

#[test]
fn internal_attribute_visibility_uses_package_identity() {
    let entry = "acme/application:main.doria";
    let same_package = graph_plan(
        vec![graph_package(
            "acme/application",
            vec![
                graph_source("acme/application", "main.doria", SourceOrigin::Entry),
                graph_source("acme/application", "Route.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "#[Route] function main(): void {}",
    );
    provider.insert(
        "acme/application",
        "Route.doria",
        "#[Attribute] internal class Route {}",
    );
    let graph = load_compilation_graph(&same_package, &provider).expect("valid package graph");
    let metadata = doriac::metadata_compilation_graph(&graph)
        .expect("package-internal attributes are visible throughout their package");
    assert_eq!(metadata.applications.len(), 1);

    let cross_package = graph_plan(
        vec![
            graph_package(
                "acme/application",
                vec![graph_source(
                    "acme/application",
                    "main.doria",
                    SourceOrigin::Entry,
                )],
                vec![Dependency {
                    package: "acme/metadata".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            graph_package(
                "acme/metadata",
                vec![graph_source(
                    "acme/metadata",
                    "Acme/Metadata/Route.doria",
                    SourceOrigin::Explicit,
                )],
                Vec::new(),
            ),
        ],
        entry,
    );
    provider.insert(
        "acme/application",
        "main.doria",
        "#[Acme\\Metadata\\Route] function main(): void {}",
    );
    provider.insert(
        "acme/metadata",
        "Acme/Metadata/Route.doria",
        concat!(
            "namespace Acme\\Metadata;\n",
            "#[Attribute] internal class Route {}\n",
        ),
    );
    let graph = load_compilation_graph(&cross_package, &provider).expect("valid package graph");
    let diagnostics = doriac::metadata_compilation_graph(&graph)
        .expect_err("a dependency's package-internal attribute must remain inaccessible");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0682"
            && diagnostic.title == "Package-Internal Declaration Is Inaccessible"
    }));
}

#[test]
fn transitive_attribute_dependencies_remain_hidden() {
    let entry = "acme/application:main.doria";
    let document = graph_plan(
        vec![
            graph_package(
                "acme/application",
                vec![graph_source(
                    "acme/application",
                    "main.doria",
                    SourceOrigin::Entry,
                )],
                vec![Dependency {
                    package: "acme/support".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            graph_package(
                "acme/support",
                vec![graph_source(
                    "acme/support",
                    "support.doria",
                    SourceOrigin::Explicit,
                )],
                vec![Dependency {
                    package: "acme/metadata".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            graph_package(
                "acme/metadata",
                vec![graph_source(
                    "acme/metadata",
                    "Acme/Metadata/Route.doria",
                    SourceOrigin::Explicit,
                )],
                Vec::new(),
            ),
        ],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "#[Acme\\Metadata\\Route] function main(): void {}",
    );
    provider.insert(
        "acme/support",
        "support.doria",
        "function support(): void {}",
    );
    provider.insert(
        "acme/metadata",
        "Acme/Metadata/Route.doria",
        concat!(
            "namespace Acme\\Metadata;\n",
            "#[Attribute] class Route {}\n",
        ),
    );
    let graph = load_compilation_graph(&document, &provider).expect("valid package graph");
    let diagnostics = doriac::metadata_compilation_graph(&graph)
        .expect_err("transitive attribute dependencies are not implicitly visible");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0682" && diagnostic.message.contains("not a direct dependency")
    }));
}

#[test]
fn complete_graph_unknown_attribute_is_a_language_diagnostic() {
    let entry = "acme/application:main.doria";
    let document = graph_plan(
        vec![graph_package(
            "acme/application",
            vec![graph_source(
                "acme/application",
                "main.doria",
                SourceOrigin::Entry,
            )],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "#[Missing] function main(): void {}",
    );
    let graph = load_compilation_graph(&document, &provider).expect("complete graph input");
    let analysis = doriac::analyze_compilation_graph_for_ide(&graph);
    let diagnostic = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0686")
        .expect("unknown attribute should be diagnosed");
    assert_eq!(
        diagnostic.kind,
        doriac::diagnostics::DiagnosticKind::Language
    );
}

#[test]
fn incremental_fingerprints_track_attribute_surfaces_and_schema_dependencies() {
    let entry = "acme/application:main.doria";
    let document = graph_plan(
        vec![graph_package(
            "acme/application",
            vec![
                graph_source("acme/application", "main.doria", SourceOrigin::Entry),
                graph_source("acme/application", "Route.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "Route.doria",
        concat!(
            "#[Attribute]\n",
            "class Route { function __construct(string $path, int $status = 200) {} }\n",
        ),
    );
    provider.insert(
        "acme/application",
        "main.doria",
        "#[Route(path: \"/one\")] function main(): void { echo \"one\\n\"; }",
    );
    let mut session = doriac::incremental::CompilationSession::new();

    let first = session
        .load_graph(&document, &provider)
        .expect("initial attributed graph");
    let analysis = session.analyze_graph(&first.graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis
        .semantic_dependency_edges
        .iter()
        .any(|edge| edge.source.0 == entry
            && edge.target.0 == "acme/application:Route.doria"
            && edge.role == doriac::names::GlobalReferenceRole::AttributeClass));

    let unchanged = session
        .load_graph(&document, &provider)
        .expect("unchanged attributed graph");
    assert_eq!(unchanged.facts.reused_declaration_indexes.len(), 2);

    provider.insert(
        "acme/application",
        "main.doria",
        "#[Route(path: \"/one\")] function main(): void { echo \"body only\\n\"; }",
    );
    let body_only = session
        .load_graph(&document, &provider)
        .expect("body-only attributed graph update");
    assert!(body_only.facts.body_only_changed_sources.contains(entry));
    assert!(!body_only.facts.declaration_changed_sources.contains(entry));
    session.analyze_graph(&body_only.graph);

    provider.insert(
        "acme/application",
        "main.doria",
        "#[Route(path: \"/two\")] function main(): void { echo \"body only\\n\"; }",
    );
    let attribute_edit = session
        .load_graph(&document, &provider)
        .expect("attribute argument graph update");
    assert!(attribute_edit
        .facts
        .declaration_changed_sources
        .contains(entry));
    session.analyze_graph(&attribute_edit.graph);

    provider.insert(
        "acme/application",
        "Route.doria",
        concat!(
            "#[Attribute]\n",
            "class Route { function __construct(string $path, int $status = 201) {} }\n",
        ),
    );
    let schema_edit = session
        .load_graph(&document, &provider)
        .expect("attribute schema graph update");
    assert!(schema_edit
        .facts
        .declaration_changed_sources
        .contains("acme/application:Route.doria"));
    session.analyze_graph(&schema_edit.graph);
    assert!(session.last_facts().invalidated_sources.contains(entry));

    let metadata = doriac::metadata_compilation_graph(&schema_edit.graph)
        .expect("updated default should rebuild metadata");
    let route = metadata
        .applications
        .iter()
        .find(|application| application.attribute_class == "Route")
        .expect("Route application");
    assert!(route.bound_arguments[1].defaulted);
    assert!(matches!(
        route.bound_arguments[1].value,
        doriac::attributes::MetadataValueV1::Integer { ref value, .. } if value == "201"
    ));
}

#[test]
fn metadata_values_preserve_exact_scalar_nullable_and_enum_types() {
    let source = r#"
enum State { case Ready; }
enum Priority: int { case High = 7; }
enum Point { case At(int16 $x, string $label); }

const int8 TINY = 1;
class Labels { const string PREFIX = "Dor"; }

#[Attribute]
class Values
{
    function __construct(
        int8 $i8,
        int16 $i16,
        int32 $i32,
        int64 $i64,
        uint8 $u8,
        uint16 $u16,
        uint32 $u32,
        uint64 $u64,
        float32 $f32,
        float64 $f64,
        bool $flag,
        string $label,
        ?string $optional,
        State $state,
        Priority $priority,
        Point $point,
        int $converted,
        float $convertedFloat,
    ) {}
}

#[Values(
    i8: TINY,
    i16: 2 + 3,
    i32: Int32::from(6),
    i64: 7,
    u8: 8,
    u16: 9,
    u32: 10,
    u64: 11,
    f32: 1.25,
    f64: 2.5,
    flag: true && !false,
    label: Labels::PREFIX . "ia",
    optional: null,
    state: State::Ready,
    priority: Priority::High,
    point: Point::At(x: 12, label: "origin"),
    converted: Float::toInt(13.0),
    convertedFloat: Int::toFloat(14),
)]
function main(): void {}
"#;
    let metadata = doriac::metadata_source("typed-values.doria", source)
        .expect("the complete metadata-compatible constant tier should evaluate");
    let application = metadata
        .applications
        .iter()
        .find(|application| application.attribute_class == "Values")
        .expect("Values application should be serialized");
    let types = application
        .bound_arguments
        .iter()
        .map(|argument| argument.r#type.clone())
        .collect::<Vec<_>>();
    for expected in [
        "int8", "int16", "int32", "int", "uint8", "uint16", "uint32", "uint64", "float32", "float",
        "bool", "string", "?string", "State", "Priority", "Point",
    ] {
        assert!(
            types.iter().any(|ty| ty == expected),
            "missing {expected}: {types:?}"
        );
    }
    assert_eq!(types.iter().filter(|ty| ty.as_str() == "int").count(), 2);
    assert_eq!(types.iter().filter(|ty| ty.as_str() == "float").count(), 2);
    assert!(application.bound_arguments.iter().any(|argument| {
        matches!(
            argument.value,
            doriac::attributes::MetadataValueV1::Null { ref r#type } if r#type == "?string"
        )
    }));
    assert!(application.bound_arguments.iter().any(|argument| {
        matches!(
            argument.value,
            doriac::attributes::MetadataValueV1::PayloadEnum { ref r#type, ref case, .. }
                if r#type == "Point" && case == "At"
        )
    }));
}

#[test]
fn runtime_expressions_and_nonconstant_defaults_never_enter_metadata() {
    let source = r#"
function runtimePath(): string { return "/runtime"; }

class Paths
{
    static function fromEnvironment(): string { return "/environment"; }
}

class Configuration {}

#[Attribute]
class Route { function __construct(string $path) {} }

#[Attribute]
class InvalidDefault
{
    function __construct(string $path = runtimePath()) {}
}

#[Route(path: runtimePath())] function callValue(): void {}
#[Route(path: read_file("route.txt"))] function ioValue(): void {}
#[Route(path: Paths::fromEnvironment())] function factoryValue(): void {}
#[Route(path: new Configuration())] function objectValue(): void {}
#[Route(path: ["runtime"])] function collectionValue(): void {}
#[InvalidDefault] function defaultValue(): void {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("runtime-values.doria", source)
        .expect("IDE analysis should retain deterministic metadata diagnostics");
    assert!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0693")
            .count()
            >= 3,
        "function, I/O, and static factory calls must be rejected as runtime expressions: {:#?}",
        analysis.diagnostics
    );
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0696"));
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0403"));
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0693"
            && diagnostic
                .message
                .contains("not available in constant evaluation")
    }));
    assert!(analysis.info.attributes.applications.is_empty());
}
