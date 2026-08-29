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

#[test]
fn metadata_schema_2_includes_main_development_and_generated_sources() {
    let package = "acme/application";
    let main_identity = format!("{package}:main.doria");
    let development_identity = format!("{package}:Feature.doria");
    let generated_identity = format!("{package}:GeneratedTest.doria");
    let plan = BuildPlanDocument {
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
    };
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
