use doriac::attributes::{
    AttributeMetadataDocumentV2, AttributeProcessorRequestV1, ATTRIBUTE_PROCESSOR_SCHEMA_VERSION,
};
use doriac::build_plan::{
    BuildNativeProfile, BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget,
    GeneratedFor, NamespaceMapping, Package, SelectedTarget, Source, SourceOrigin, SourceScope,
    TargetKind,
};
use doriac::compilation_graph::load_compilation_graph;
use doriac::source_provider::InMemorySourceProvider;

#[test]
fn metadata_schema_2_exposes_exact_callable_signatures_without_runtime_data() {
    let source = r#"
#[Attribute]
class Marker {}

class Failure implements Error
{
    function __construct(string $message) {}
}

class Box<T>
{
    internal function __construct(take T $value) {}
    function __destruct() {}

    internal function transform<U>(
        writable int $count,
        take T $value,
        function writable(take T): U throws Failure $callback,
    ): Box<U> throws Failure {
        throw new Failure("stop");
    }
}

#[Marker]
function task<T>(string $name, writable int $count, take T $value): T
{
    return $value;
}

function ambient(): void
{
    echo "ambient";
}
"#;

    let first = doriac::metadata_source_v2("callables.doria", source)
        .expect("schema-2 callable metadata should lower");
    let second = doriac::metadata_source_v2("callables.doria", source)
        .expect("schema-2 callable metadata should be deterministic");
    assert_eq!(first, second);
    assert_eq!(first.schema_version, 2);

    let task = callable(&first, "task");
    assert_eq!(task.identity, "global:task:function");
    assert_eq!(task.kind, "function");
    assert_eq!(task.package, "standalone");
    assert_eq!(task.source, "callables.doria");
    assert_eq!(task.access, "external");
    assert_eq!(task.generic_parameter_count, 1);
    assert_eq!(task.return_type, "T");
    assert_eq!(task.parameters.len(), 3);
    assert_eq!(task.parameters[0].ownership, "readonly");
    assert_eq!(task.parameters[1].ownership, "writable");
    assert_eq!(task.parameters[2].ownership, "take");
    assert_eq!(first.applications[0].target, task.identity);

    let transform = callable(&first, "Box::transform");
    assert_eq!(transform.source, "callables.doria");
    assert_eq!(transform.package, "standalone");
    assert_eq!(transform.identity, "member:Box:method:transform");
    assert_eq!(transform.kind, "method");
    assert_eq!(transform.access, "internal");
    assert_eq!(transform.generic_parameter_count, 1);
    assert_eq!(transform.return_type, "Box<U>");
    assert_eq!(transform.parameters.len(), 3);
    assert_eq!(transform.parameters[0].r#type, "int");
    assert_eq!(transform.parameters[0].ownership, "writable");
    assert_eq!(transform.parameters[1].r#type, "T");
    assert_eq!(transform.parameters[1].ownership, "take");
    assert_eq!(
        transform.parameters[2].r#type,
        "function writable(take T): U throws Failure"
    );
    assert_eq!(transform.required_effects, ["Failure"]);
    assert!(transform.ambient_effects.is_empty());

    let constructor = callable(&first, "Box::__construct");
    assert_eq!(constructor.identity, "member:Box:constructor:__construct");
    assert_eq!(constructor.kind, "constructor");
    assert_eq!(constructor.return_type, "void");
    assert_eq!(constructor.parameters.len(), 1);

    let destructor = callable(&first, "Box::__destruct");
    assert_eq!(destructor.identity, "member:Box:destructor:__destruct");
    assert_eq!(destructor.kind, "destructor");
    assert!(destructor.parameters.is_empty());

    let ambient = callable(&first, "ambient");
    assert!(ambient.required_effects.is_empty());
    assert_eq!(ambient.ambient_effects, ["Doria\\Std\\Io\\IoError"]);

    let json = serde_json::to_string_pretty(&first).expect("metadata should encode");
    let decoded: AttributeMetadataDocumentV2 =
        serde_json::from_str(&json).expect("schema 2 must round trip strictly");
    assert_eq!(decoded, first);
    for forbidden in ["TypeId", "FunctionId", "mir", "cranelift", "llvm", "php"] {
        assert!(!json.contains(forbidden), "metadata leaked `{forbidden}`");
    }
}

fn development_plan() -> BuildPlanDocument {
    let package = "acme/application";
    let main_identity = format!("{package}:main.doria");
    let development_identity = format!("{package}:Feature.doria");
    let generated_identity = format!("{package}:GeneratedTest.doria");
    BuildPlanDocument {
        path: "plan.json".to_string(),
        directory: std::env::current_dir().expect("current directory"),
        text: String::new(),
        plan: BuildPlan {
            schema_version: 1,
            edition: "2026".to_string(),
            root_package: package.to_string(),
            selected_target: SelectedTarget {
                package: package.to_string(),
                name: "application".to_string(),
                kind: TargetKind::Binary,
                entry_source: Some(main_identity.clone()),
                active_scopes: vec![
                    SourceScope::Main,
                    SourceScope::Development,
                    SourceScope::Generated,
                ],
            },
            packages: vec![Package {
                identity: package.to_string(),
                root: ".".to_string(),
                namespace_mappings: vec![
                    NamespaceMapping {
                        prefix: String::new(),
                        path: String::new(),
                        scope: SourceScope::Main,
                        generated_for: None,
                    },
                    NamespaceMapping {
                        prefix: String::new(),
                        path: String::new(),
                        scope: SourceScope::Development,
                        generated_for: None,
                    },
                    NamespaceMapping {
                        prefix: String::new(),
                        path: String::new(),
                        scope: SourceScope::Generated,
                        generated_for: Some(GeneratedFor::Development),
                    },
                ],
                sources: vec![
                    Source {
                        identity: main_identity.clone(),
                        path: "main.doria".to_string(),
                        scope: SourceScope::Main,
                        origin: SourceOrigin::Entry,
                        generated_for: None,
                    },
                    Source {
                        identity: development_identity.clone(),
                        path: "Feature.doria".to_string(),
                        scope: SourceScope::Development,
                        origin: SourceOrigin::Explicit,
                        generated_for: None,
                    },
                    Source {
                        identity: generated_identity.clone(),
                        path: "GeneratedTest.doria".to_string(),
                        scope: SourceScope::Generated,
                        origin: SourceOrigin::Generated,
                        generated_for: Some(GeneratedFor::Development),
                    },
                ],
                dependencies: Vec::new(),
            }],
            compiler: CompilerOptions {
                target: CompilerTarget::Native,
                native_profile: Some(BuildNativeProfile::Fast),
                target_triple: None,
            },
        },
    }
}

#[test]
fn metadata_schema_2_includes_main_development_and_generated_sources() {
    let plan = development_plan();
    let package = plan.plan.root_package.as_str();
    let main_identity = format!("{package}:main.doria");
    let development_identity = format!("{package}:Feature.doria");
    let generated_identity = format!("{package}:GeneratedTest.doria");
    let mut provider = InMemorySourceProvider::new();
    provider.insert(package, "main.doria", "function main(): void {}");
    provider.insert(
        package,
        "Feature.doria",
        "#[Test] function featureTest(): void {}",
    );
    provider.insert(
        package,
        "GeneratedTest.doria",
        "#[Test] function generatedTest(): void {}",
    );

    let graph = load_compilation_graph(&plan, &provider).expect("complete development graph");
    let metadata = doriac::metadata_compilation_graph_v2(&graph)
        .expect("cross-file schema-2 metadata should lower");
    assert_eq!(metadata.selected_target.entry_source, Some(main_identity));
    assert_eq!(
        callable(&metadata, "featureTest").source,
        development_identity
    );
    assert_eq!(
        callable(&metadata, "generatedTest").source,
        generated_identity
    );
    assert_eq!(
        metadata
            .callables
            .iter()
            .map(|callable| callable.canonical_name.as_str())
            .collect::<Vec<_>>(),
        ["featureTest", "generatedTest", "main"]
    );
}

#[test]
fn processor_protocol_remains_schema_1() {
    assert_eq!(ATTRIBUTE_PROCESSOR_SCHEMA_VERSION, 1);
    let fields = serde_json::to_value(AttributeProcessorRequestV1 {
        schema_version: ATTRIBUTE_PROCESSOR_SCHEMA_VERSION,
        edition: "2026".to_string(),
        compiler_revision: "revision".to_string(),
        graph_fingerprint: "graph".to_string(),
        processor_package: "acme/processor".to_string(),
        selected_target: doriac::attributes::MetadataTargetV1 {
            package: "acme/application".to_string(),
            kind: "binary".to_string(),
            entry_source: Some("acme/application:main.doria".to_string()),
        },
        sources: Vec::new(),
        attribute_classes: Vec::new(),
        applications: Vec::new(),
    })
    .expect("processor request should encode");
    assert_eq!(fields["schemaVersion"], 1);
    assert!(fields.get("callables").is_none());
}

#[test]
fn composed_generic_callables_keep_authored_type_names_in_frozen_metadata_schemas() {
    let source = format!(
        "{}\n{}",
        include_str!("fixtures/nongeneric_trait_method_parameters.doria"),
        r#"
trait Callable<T> {
    function relay<U>(function(U): U $callback): ?U { return null; }
}
class Other<V> { uses Callable<V>; }
"#,
    );
    let one = doriac::metadata_source("generic-metadata.doria", &source).unwrap();
    let two = doriac::metadata_source_v2("generic-metadata.doria", &source).unwrap();
    let three = doriac::metadata_source_v3("generic-metadata.doria", &source).unwrap();
    assert_eq!(one.schema_version, 1);
    assert_eq!(two.schema_version, 2);
    assert_eq!(three.schema_version, 3);
    assert_eq!(two.callables, three.callables);
    for name in ["Box::identity", "Plain::identity"] {
        let method = callable(&two, name);
        assert_eq!(method.generic_parameter_count, 1);
        assert_eq!(method.parameters[0].r#type, "U");
        assert_eq!(method.return_type, "U");
    }
    let relay = callable(&two, "Other::relay");
    assert_eq!(relay.parameters[0].r#type, "function(U): U");
    assert_eq!(relay.return_type, "?U");
    let json_one = serde_json::to_value(&one).unwrap();
    let json_two = serde_json::to_value(&two).unwrap();
    let json_three = serde_json::to_value(&three).unwrap();
    assert!(json_one.get("callables").is_none());
    assert_eq!(
        serde_json::from_value::<AttributeMetadataDocumentV2>(json_two.clone()).unwrap(),
        two
    );
    assert_eq!(
        serde_json::from_value::<doriac::attributes::AttributeMetadataDocumentV3>(
            json_three.clone()
        )
        .unwrap(),
        three
    );
    for json in [json_one, json_two, json_three] {
        let text = serde_json::to_string(&json).unwrap();
        for private in ["#trait", "TypeId(", "ExpansionId("] {
            assert!(
                !text.contains(private),
                "metadata exposed {private}: {text}"
            );
        }
    }
}

#[test]
fn guarded_composed_callables_preserve_authored_signatures_in_frozen_metadata() {
    let source = r#"
interface Ready {}
trait Relay<T> {
    function relay<U>(function(U): U $callback): ?U where (T implements Ready) {
        return null;
    }
}
class Adapter<V> { uses Relay<V>; }
"#;
    let one = doriac::metadata_source("guarded-metadata.doria", source).unwrap();
    let two = doriac::metadata_source_v2("guarded-metadata.doria", source).unwrap();
    let three = doriac::metadata_source_v3("guarded-metadata.doria", source).unwrap();
    assert_eq!(one.schema_version, 1);
    assert_eq!(two.schema_version, 2);
    assert_eq!(three.schema_version, 3);
    assert_eq!(two.callables, three.callables);
    assert_eq!(
        two,
        doriac::metadata_source_v2("guarded-metadata.doria", source).unwrap()
    );
    let relay = callable(&two, "Adapter::relay");
    assert_eq!(relay.identity, "member:Adapter:method:relay");
    assert_eq!(relay.source, "guarded-metadata.doria");
    assert_eq!(relay.generic_parameter_count, 1);
    assert_eq!(relay.parameters[0].r#type, "function(U): U");
    assert_eq!(relay.return_type, "?U");

    let one_json = serde_json::to_value(&one).unwrap();
    let two_json = serde_json::to_value(&two).unwrap();
    let three_json = serde_json::to_value(&three).unwrap();
    assert!(one_json.get("callables").is_none());
    assert_eq!(
        serde_json::from_value::<doriac::attributes::AttributeMetadataDocumentV1>(one_json.clone())
            .unwrap(),
        one
    );
    assert_eq!(
        serde_json::from_value::<AttributeMetadataDocumentV2>(two_json.clone()).unwrap(),
        two
    );
    assert_eq!(
        serde_json::from_value::<doriac::attributes::AttributeMetadataDocumentV3>(
            three_json.clone()
        )
        .unwrap(),
        three
    );
    for json in [one_json, two_json, three_json] {
        let text = serde_json::to_string(&json).unwrap();
        for private in ["#trait", "TypeId(", "ExpansionId(", "callable_conditions"] {
            assert!(
                !text.contains(private),
                "metadata exposed {private}: {text}"
            );
        }
    }
}

#[test]
fn conditional_conformance_without_guarded_members_reaches_public_lowering() {
    let source = r#"
interface Ready {}
class Yes implements Ready {}
interface Answer { function answer(): int; }
class Adapter<T> implements Answer where (T implements Ready) {
    function answer(): int { return 42; }
}
function inspect(Answer $value): int { return $value->answer(); }
function main(): int {
    let $inactive = new Adapter<int>();
    return $inactive->answer() + inspect(new Adapter<Yes>());
}
"#;
    let one = doriac::metadata_source("conformance-only.doria", source).unwrap();
    let two = doriac::metadata_source_v2("conformance-only.doria", source).unwrap();
    let three = doriac::metadata_source_v3("conformance-only.doria", source).unwrap();
    assert_eq!(one.schema_version, 1);
    assert_eq!(two.callables, three.callables);
    assert_eq!(callable(&two, "Adapter::answer").return_type, "int");

    let parsed = doriac::parse_source("conformance-only.doria", source).unwrap();
    let diagnostics =
        doriac::lowering::lower_program_with_semantics(&parsed, Default::default()).unwrap_err();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(
        diagnostics[0].kind,
        doriac::diagnostics::DiagnosticKind::InternalCompiler
    );

    let inactive = source.replace("inspect(new Adapter<Yes>())", "inspect(new Adapter<int>())");
    let diagnostics = doriac::metadata_source_v2("conformance-only.doria", &inactive).unwrap_err();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "E0408");
    assert_eq!(
        &inactive[diagnostics[0].span.start..diagnostics[0].span.end],
        "new Adapter<int>()"
    );
}

#[test]
fn conditional_project_metadata_checks_cross_source_capability_at_the_caller() {
    let mut plan = development_plan();
    let declarations = [
        ("Ready.doria", "interface Ready { function read(): int; }"),
        ("Yes.doria", "class Yes implements Ready { function read(): int { return 7; } }"),
        ("No.doria", "class No {}"),
        ("Reading.doria", "trait Reading<T> { function read(T $source): int where (T implements Ready) { return $source->read(); } }"),
        ("Adapter.doria", "class Adapter<T> { uses Reading<T>; }"),
    ];
    for (path, _) in declarations {
        plan.plan.packages[0].sources.push(Source {
            identity: format!("{}:{path}", plan.plan.root_package),
            path: path.to_string(),
            scope: SourceScope::Development,
            origin: SourceOrigin::Explicit,
            generated_for: None,
        });
    }
    let package = plan.plan.root_package.as_str();
    for (source_type, accepted) in [("Yes", true), ("No", false)] {
        let caller = format!(
            "function inspect(Adapter<{source_type}> $adapter, {source_type} $source): int {{ return $adapter->read($source); }}"
        );
        let mut provider = InMemorySourceProvider::new();
        provider.insert(package, "main.doria", "function main(): void {}");
        provider.insert(package, "Feature.doria", "function feature(): void {}");
        for (path, source) in declarations {
            provider.insert(package, path, source);
        }
        provider.insert(package, "GeneratedTest.doria", &caller);
        let graph = load_compilation_graph(&plan, &provider).unwrap();
        let result = doriac::metadata_compilation_graph_v2(&graph);
        if accepted {
            let metadata = result.expect("active cross-source capability should lower");
            assert_eq!(
                callable(&metadata, "inspect").source,
                format!("{package}:GeneratedTest.doria")
            );
            assert_eq!(
                callable(&metadata, "inspect").parameters[0].r#type,
                "Adapter<Yes>"
            );
            assert_eq!(
                callable(&metadata, "Adapter::read").source,
                format!("{package}:Reading.doria")
            );
        } else {
            let diagnostics = result.expect_err("inactive cross-source member must be rejected");
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert_eq!(diagnostics[0].code, "E0779");
            let span = diagnostics[0].span;
            assert_eq!(&caller[span.start..span.end], "$adapter->read($source)");
            let source = graph
                .sources
                .values()
                .find(|source| source.id == span.source)
                .unwrap();
            assert_eq!(source.identity.0, format!("{package}:GeneratedTest.doria"));
        }
    }
}

fn callable<'a>(
    metadata: &'a AttributeMetadataDocumentV2,
    name: &str,
) -> &'a doriac::attributes::MetadataCallableV2 {
    metadata
        .callables
        .iter()
        .find(|callable| callable.canonical_name == name)
        .unwrap_or_else(|| panic!("missing callable `{name}`: {:#?}", metadata.callables))
}
