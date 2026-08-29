use doriac::build_plan::{
    BuildNativeProfile, BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget, Dependency,
    DependencyKind, GeneratedFor, NamespaceMapping, Package, SelectedTarget, Source, SourceOrigin,
    SourceScope, TargetKind,
};
use doriac::compilation_graph::{
    analyze_compilation_graph_for_ide, load_compilation_graph, load_compilation_graph_detailed,
    load_compilation_graph_with_completeness, GraphCompleteness, GraphLoadOptions,
    ProjectStructureAuthority,
};
use doriac::source_provider::InMemorySourceProvider;
use std::process::Command;

fn source(package: &str, path: &str, origin: SourceOrigin) -> Source {
    Source {
        identity: format!("{package}:{path}"),
        path: path.to_string(),
        scope: SourceScope::Main,
        origin,
        generated_for: None,
    }
}

fn scoped_source(
    package: &str,
    path: &str,
    scope: SourceScope,
    origin: SourceOrigin,
    generated_for: Option<GeneratedFor>,
) -> Source {
    Source {
        identity: format!("{package}:{path}"),
        path: path.to_string(),
        scope,
        origin,
        generated_for,
    }
}

fn package(identity: &str, sources: Vec<Source>, dependencies: Vec<Dependency>) -> Package {
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

fn plan(packages: Vec<Package>, entry: &str) -> BuildPlanDocument {
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
fn strict_schema_rejects_unknown_fields() {
    let text = r#"{
        "schemaVersion": 1,
        "edition": "2026",
        "rootPackage": "acme/application",
        "selectedTarget": {
            "package": "acme/application",
            "name": "application",
            "kind": "binary",
            "entrySource": "acme/application:main.doria",
            "activeScopes": ["main"]
        },
        "packages": [],
        "compiler": { "target": "native", "nativeProfile": "fast", "targetTriple": null },
        "batonManifest": "Baton.toml"
    }"#;

    let diagnostics = doriac::build_plan::parse_build_plan("plan.json", text)
        .expect_err("unknown schema-1 fields must be rejected");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0676"));
}

#[test]
fn unknown_schema_version_is_rejected_before_schema_one_fields() {
    let diagnostics = doriac::build_plan::parse_build_plan(
        "future.json",
        r#"{"schemaVersion":99,"futureOnly":true}"#,
    )
    .expect_err("future schemas must not be interpreted as schema 1");
    assert_eq!(
        diagnostics[0].title,
        "Build Plan Schema Version Is Unsupported"
    );
}

#[test]
fn partial_tooling_graphs_do_not_fabricate_project_structure_authority() {
    let entry = "acme/application:open/app.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "open/app.doria", SourceOrigin::Entry),
                source(
                    "acme/application",
                    "open/model.doria",
                    SourceOrigin::Explicit,
                ),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "open/app.doria",
        r#"namespace Acme\App;
function inspect(Acme\Model\User $user): void {}
function main(): void { inspect(new Acme\Model\User()); }
"#,
    );
    provider.insert(
        "acme/application",
        "open/model.doria",
        "namespace Acme\\Model; class User {}",
    );

    let strict = load_compilation_graph(&document, &provider)
        .expect_err("an authoritative build plan must enforce source layout");
    assert!(strict.iter().any(|diagnostic| diagnostic.code == "E0680"));

    let mut session = doriac::incremental::CompilationSession::new();
    let update = session
        .load_graph_with_options(
            &document,
            &provider,
            GraphLoadOptions {
                completeness: GraphCompleteness::Partial,
                project_structure: ProjectStructureAuthority::Unavailable,
            },
        )
        .expect("open-document tooling graph");
    assert_eq!(update.graph.completeness, GraphCompleteness::Partial);
    assert_eq!(update.facts.parsed_sources.len(), 2);
    let analysis = session.analyze_graph(&update.graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis.semantic_dependency_edges.iter().any(|edge| {
        edge.source.0 == entry
            && edge.target.0 == "acme/application:open/model.doria"
            && edge.symbol.qualified_name == "Acme\\Model\\User"
    }));
}

#[test]
fn schema_one_round_trips_deterministically_and_validates_compiler_profiles() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![source(
                "acme/application",
                "main.doria",
                SourceOrigin::Entry,
            )],
            Vec::new(),
        )],
        entry,
    );
    let encoded = doriac::build_plan::encode_build_plan(&document.plan).expect("encode schema 1");
    let decoded = doriac::build_plan::parse_build_plan("plan.json", &encoded)
        .expect("decode encoded schema 1");
    assert_eq!(decoded, document.plan);
    assert_eq!(
        doriac::build_plan::encode_build_plan(&decoded).expect("re-encode schema 1"),
        encoded
    );

    for (target, profile, valid) in [
        (CompilerTarget::Debug, None, true),
        (CompilerTarget::Php, None, true),
        (CompilerTarget::Native, Some(BuildNativeProfile::Fast), true),
        (
            CompilerTarget::Native,
            Some(BuildNativeProfile::Release),
            true,
        ),
        (CompilerTarget::Native, None, false),
        (CompilerTarget::Debug, Some(BuildNativeProfile::Fast), false),
        (
            CompilerTarget::Php,
            Some(BuildNativeProfile::Release),
            false,
        ),
    ] {
        let mut candidate = document.plan.clone();
        candidate.compiler.target = target;
        candidate.compiler.native_profile = profile;
        assert_eq!(
            doriac::build_plan::validate_build_plan(&candidate).is_ok(),
            valid,
            "target={target:?} profile={profile:?}"
        );
    }
}

#[test]
fn schema_one_rejects_duplicate_missing_and_ambiguous_inventory() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![source(
                "acme/application",
                "main.doria",
                SourceOrigin::Entry,
            )],
            Vec::new(),
        )],
        entry,
    );

    let mut duplicate_package = document.plan.clone();
    duplicate_package
        .packages
        .push(duplicate_package.packages[0].clone());
    assert!(doriac::build_plan::validate_build_plan(&duplicate_package).is_err());

    let mut duplicate_source = document.plan.clone();
    let repeated_source = duplicate_source.packages[0].sources[0].clone();
    duplicate_source.packages[0].sources.push(repeated_source);
    assert!(doriac::build_plan::validate_build_plan(&duplicate_source).is_err());

    let mut missing_root = document.plan.clone();
    missing_root.root_package = "acme/missing".to_string();
    missing_root.selected_target.package = "acme/missing".to_string();
    assert!(doriac::build_plan::validate_build_plan(&missing_root).is_err());

    let mut invalid_generated = document.plan.clone();
    invalid_generated.packages[0].sources.push(scoped_source(
        "acme/application",
        "generated.doria",
        SourceScope::Generated,
        SourceOrigin::Generated,
        None,
    ));
    assert!(doriac::build_plan::validate_build_plan(&invalid_generated).is_err());

    let mut case_collision = document.plan.clone();
    case_collision.packages[0].sources.push(source(
        "acme/application",
        "MAIN.doria",
        SourceOrigin::Explicit,
    ));
    assert!(doriac::build_plan::validate_build_plan(&case_collision).is_err());
}

#[test]
fn schema_one_accepts_only_the_selected_entry_as_generated_entry_origin() {
    let entry = "acme/application:build/generated/tests/dispatcher.doria";
    let mut document = plan(
        vec![package(
            "acme/application",
            vec![scoped_source(
                "acme/application",
                "build/generated/tests/dispatcher.doria",
                SourceScope::Generated,
                SourceOrigin::Entry,
                Some(GeneratedFor::Development),
            )],
            Vec::new(),
        )],
        entry,
    );
    document.plan.selected_target.active_scopes = vec![
        SourceScope::Main,
        SourceScope::Development,
        SourceScope::Generated,
    ];
    assert!(doriac::build_plan::validate_build_plan(&document.plan).is_ok());

    let mut not_selected = document.plan.clone();
    not_selected.packages[0].sources.push(source(
        "acme/application",
        "main.doria",
        SourceOrigin::Entry,
    ));
    not_selected.selected_target.entry_source = Some("acme/application:main.doria".to_string());
    assert!(doriac::build_plan::validate_build_plan(&not_selected).is_err());

    let mut explicit_generated = document.plan.clone();
    explicit_generated.packages[0].sources[0].origin = SourceOrigin::Explicit;
    assert!(doriac::build_plan::validate_build_plan(&explicit_generated).is_err());
}

#[test]
fn top_level_internal_declarations_preserve_modifier_spans() {
    let program = doriac::parse_source(
        "internal.doria",
        "internal class Helper {}\ninternal enum State { case Ready; }\ninternal interface Contract {}\ninternal trait Support {}\ninternal function helper(): void {}\ninternal const int LIMIT = 10;",
    )
    .expect("top-level internal grammar");
    assert_eq!(program.items.len(), 6);
    for item in program.items {
        let span = match item {
            doriac::ast::Item::Class(value) => value.access_span,
            doriac::ast::Item::Enum(value) => value.access_span,
            doriac::ast::Item::Interface(value) => value.access_span,
            doriac::ast::Item::Trait(value) => value.access_span,
            doriac::ast::Item::Function(value) => value.access_span,
            doriac::ast::Item::Constant(value) => value.access_span,
            doriac::ast::Item::Statement(_) => panic!("expected declarations"),
        };
        let span = span.expect("internal span");
        assert_eq!(span.end - span.start, "internal".len());
    }
}

#[test]
fn same_package_sources_share_functions_and_internal_access() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source("acme/application", "helpers.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return helper(); }",
    );
    provider.insert(
        "acme/application",
        "helpers.doria",
        "internal function helper(): int { return 42; }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid package graph");
    let analysis = analyze_compilation_graph_for_ide(&graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let hir = doriac::lower_compilation_graph(&graph).expect("multi-source HIR");
    assert_eq!(hir.sources.len(), 2);
    for item in &hir.items {
        match item {
            doriac::hir::Item::Function(function) => {
                assert!(function.global_id.is_some());
                assert_ne!(function.source_identity.0, "<unknown>");
            }
            doriac::hir::Item::Class(class) => assert!(class.global_id.is_some()),
            doriac::hir::Item::Enum(value) => assert!(value.global_id.is_some()),
            doriac::hir::Item::Constant(value) => assert!(value.global_id.is_some()),
            doriac::hir::Item::Statement(_) => {}
        }
    }
    let mir = doriac::lower_compilation_graph_to_mir(&graph).expect("multi-source MIR");
    assert_eq!(mir.sources.len(), 2);
    assert!(mir.selected_entry.is_some());

    let mut debug_graph = graph.clone();
    debug_graph.build_plan.compiler.target = CompilerTarget::Debug;
    debug_graph.build_plan.compiler.native_profile = None;
    let output = doriac::compile_compilation_graph(&debug_graph).expect("debug graph execution");
    let doriac::backend::BackendOutput::Text { contents, .. } = output else {
        panic!("debug backend must return text");
    };
    assert!(contents.contains("status: 42"), "{contents}");
}

#[test]
fn compiler_known_declarations_have_no_authored_source_or_package_ownership() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![source(
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
        "function main(): void { echo \"hello\\n\"; }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid package graph");
    let hir = doriac::lower_compilation_graph(&graph).expect("graph HIR with compiler-known I/O");

    assert_eq!(hir.sources.len(), 1, "only authored sources belong in HIR");
    let compiler_known_items = hir
        .items
        .iter()
        .filter(|item| match item {
            doriac::hir::Item::Class(value) => {
                value.span.source == doriac::compiler_known_io::SYNTHETIC_SOURCE_ID
            }
            doriac::hir::Item::Enum(value) => {
                value.span.source == doriac::compiler_known_io::SYNTHETIC_SOURCE_ID
            }
            doriac::hir::Item::Function(value) => {
                value.span.source == doriac::compiler_known_io::SYNTHETIC_SOURCE_ID
            }
            doriac::hir::Item::Constant(value) => {
                value.span.source == doriac::compiler_known_io::SYNTHETIC_SOURCE_ID
            }
            doriac::hir::Item::Statement(_) => false,
        })
        .collect::<Vec<_>>();
    assert!(!compiler_known_items.is_empty());
    for item in compiler_known_items {
        let package = match item {
            doriac::hir::Item::Class(value) => &value.package,
            doriac::hir::Item::Enum(value) => &value.package,
            doriac::hir::Item::Function(value) => &value.package,
            doriac::hir::Item::Constant(value) => &value.package,
            doriac::hir::Item::Statement(_) => unreachable!(),
        };
        assert_eq!(package, &doriac::names::PackageIdentity::CompilerKnown);
    }

    let mir = doriac::lower_compilation_graph_to_mir(&graph)
        .expect("compiler-known spans validate without a fake source unit");
    assert_eq!(mir.sources.len(), 1, "only authored sources belong in MIR");
    doriac::mir_validation::validate_program(&mir).expect("valid compiler-known origins");
}

#[test]
fn transitive_dependency_is_not_visible() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![
            package(
                "acme/application",
                vec![source(
                    "acme/application",
                    "main.doria",
                    SourceOrigin::Entry,
                )],
                vec![Dependency {
                    package: "acme/support".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            package(
                "acme/support",
                vec![source(
                    "acme/support",
                    "support.doria",
                    SourceOrigin::Explicit,
                )],
                vec![Dependency {
                    package: "acme/transitive".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            package(
                "acme/transitive",
                vec![source(
                    "acme/transitive",
                    "hidden.doria",
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
        "function main(): int { return hidden(); }",
    );
    provider.insert(
        "acme/support",
        "support.doria",
        "function support(): int { return hidden(); }",
    );
    provider.insert(
        "acme/transitive",
        "hidden.doria",
        "function hidden(): int { return 42; }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid package graph");
    let analysis = analyze_compilation_graph_for_ide(&graph);
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0682" && diagnostic.message.contains("not a direct dependency")
    }));
}

#[test]
fn development_dependency_visibility_follows_the_source_scope() {
    let entry = "acme/application:main.doria";
    let mut application = package(
        "acme/application",
        vec![
            source("acme/application", "main.doria", SourceOrigin::Entry),
            scoped_source(
                "acme/application",
                "development.doria",
                SourceScope::Development,
                SourceOrigin::Explicit,
                None,
            ),
        ],
        vec![Dependency {
            package: "acme/testing".to_string(),
            kind: DependencyKind::Development,
        }],
    );
    application.namespace_mappings.push(NamespaceMapping {
        prefix: String::new(),
        path: String::new(),
        scope: SourceScope::Development,
        generated_for: None,
    });
    let mut document = plan(
        vec![
            application,
            package(
                "acme/testing",
                vec![source(
                    "acme/testing",
                    "testing.doria",
                    SourceOrigin::Explicit,
                )],
                Vec::new(),
            ),
        ],
        entry,
    );
    document
        .plan
        .selected_target
        .active_scopes
        .push(SourceScope::Development);
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return testValue(); }",
    );
    provider.insert(
        "acme/application",
        "development.doria",
        "function developmentValue(): int { return testValue(); }",
    );
    provider.insert(
        "acme/testing",
        "testing.doria",
        "function testValue(): int { return 42; }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("development graph");
    let analysis = analyze_compilation_graph_for_ide(&graph);
    let visibility_diagnostics = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0682")
        .collect::<Vec<_>>();
    assert_eq!(
        visibility_diagnostics.len(),
        1,
        "{:#?}",
        analysis.diagnostics
    );
    assert!(visibility_diagnostics[0]
        .message
        .contains("development dependency"));
}

#[test]
fn inactive_development_sources_are_not_parsed_but_active_dead_sources_are_checked() {
    let entry = "acme/application:main.doria";
    let application = package(
        "acme/application",
        vec![
            source("acme/application", "main.doria", SourceOrigin::Entry),
            scoped_source(
                "acme/application",
                "development.doria",
                SourceScope::Development,
                SourceOrigin::Explicit,
                None,
            ),
            source("acme/application", "dead.doria", SourceOrigin::Explicit),
        ],
        Vec::new(),
    );
    let document = plan(vec![application], entry);
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return 0; }",
    );
    provider.insert(
        "acme/application",
        "development.doria",
        "not valid doria !!!",
    );
    provider.insert(
        "acme/application",
        "dead.doria",
        "function broken(): MissingType {}",
    );

    let graph = load_compilation_graph(&document, &provider).expect("inactive development omitted");
    assert!(!graph
        .sources
        .contains_key("acme/application:development.doria"));
    let analysis = analyze_compilation_graph_for_ide(&graph);
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("MissingType")));
}

#[test]
fn package_cycles_and_non_entry_execution_are_rejected_deterministically() {
    let entry = "acme/application:main.doria";
    let cycle = plan(
        vec![
            package(
                "acme/application",
                vec![source(
                    "acme/application",
                    "main.doria",
                    SourceOrigin::Entry,
                )],
                vec![Dependency {
                    package: "acme/support".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            package(
                "acme/support",
                vec![source(
                    "acme/support",
                    "support.doria",
                    SourceOrigin::Explicit,
                )],
                vec![Dependency {
                    package: "acme/application".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
        ],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return 0; }",
    );
    provider.insert(
        "acme/support",
        "support.doria",
        "function support(): int { return 1; }",
    );
    let diagnostics = load_compilation_graph(&cycle, &provider)
        .expect_err("package dependency cycle must be rejected");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.title == "Package Dependency Cycle Is Not Allowed"));

    let non_entry = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source("acme/application", "extra.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    provider.insert("acme/application", "extra.doria", "echo \"not an entry\";");
    let diagnostics = load_compilation_graph(&non_entry, &provider)
        .expect_err("non-entry executable source must be rejected");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0683"));
}

#[test]
fn complete_and_partial_graphs_classify_missing_symbols_differently() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![source(
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
        "function main(): void { Acme\\Missing\\Tool::run(); }",
    );

    let complete = load_compilation_graph(&document, &provider).expect("complete graph input");
    let complete = analyze_compilation_graph_for_ide(&complete);
    let complete = complete
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0681")
        .expect("complete unknown symbol");
    assert_eq!(complete.kind, doriac::diagnostics::DiagnosticKind::Language);

    let partial =
        load_compilation_graph_with_completeness(&document, &provider, GraphCompleteness::Partial)
            .expect("partial graph input");
    let partial = analyze_compilation_graph_for_ide(&partial);
    let partial = partial
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0681")
        .expect("partial missing source fact");
    assert_eq!(
        partial.kind,
        doriac::diagnostics::DiagnosticKind::CompilerInput
    );
}

#[test]
fn package_internal_types_and_members_are_visible_across_same_package_files() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source("acme/application", "Person.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { let $person = new Person(); return $person->value(); }",
    );
    provider.insert(
        "acme/application",
        "Person.doria",
        "internal class Person { internal function value(): int { return 42; } }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid package graph");
    let analysis = analyze_compilation_graph_for_ide(&graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn package_internal_members_are_rejected_across_packages() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![
            package(
                "acme/application",
                vec![source(
                    "acme/application",
                    "main.doria",
                    SourceOrigin::Entry,
                )],
                vec![Dependency {
                    package: "acme/support".to_string(),
                    kind: DependencyKind::Normal,
                }],
            ),
            package(
                "acme/support",
                vec![source(
                    "acme/support",
                    "Service.doria",
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
            "function main(): int {\n",
            "  let $service = Service::create();\n",
            "  int $property = $service->secret;\n",
            "  int $method = $service->secretValue();\n",
            "  int $static = Service::staticSecret();\n",
            "  let $forbidden = new Service();\n",
            "  return $property + $method + $static;\n",
            "}\n",
        ),
    );
    provider.insert(
        "acme/support",
        "Service.doria",
        concat!(
            "class Service {\n",
            "  internal int $secret = 42;\n",
            "  internal function __construct() {}\n",
            "  static function create(): Service { return new Service(); }\n",
            "  internal function secretValue(): int { return $this->secret; }\n",
            "  internal static function staticSecret(): int { return 42; }\n",
            "}\n",
        ),
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid package graph");
    let analysis = analyze_compilation_graph_for_ide(&graph);
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0306"));
    assert!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0307")
            .count()
            >= 3,
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn duplicate_fqn_diagnostics_render_all_source_files() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source("acme/application", "first.doria", SourceOrigin::Explicit),
                source("acme/application", "second.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return 0; }",
    );
    provider.insert(
        "acme/application",
        "first.doria",
        "function duplicate(): int { return 1; }",
    );
    provider.insert(
        "acme/application",
        "second.doria",
        "function duplicate(): int { return 2; }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid source inventory");
    let analysis = analyze_compilation_graph_for_ide(&graph);
    let duplicate = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0684")
        .expect("duplicate FQN diagnostic");
    assert_eq!(duplicate.labels.len(), 2);

    let human = doriac::diagnostics::render_diagnostics_with_source_map(
        &graph.source_map,
        std::slice::from_ref(duplicate),
        doriac::diagnostics::RenderOptions {
            color: doriac::diagnostics::ColorChoice::Never,
            ..doriac::diagnostics::RenderOptions::default()
        },
    );
    assert!(human.contains("first.doria"), "{human}");
    assert!(human.contains("second.doria"), "{human}");

    let json = doriac::diagnostics::render_diagnostics_with_source_map(
        &graph.source_map,
        std::slice::from_ref(duplicate),
        doriac::diagnostics::RenderOptions {
            format: doriac::diagnostics::DiagnosticFormat::Json,
            ..doriac::diagnostics::RenderOptions::default()
        },
    );
    assert!(json.contains("first.doria"), "{json}");
    assert!(json.contains("second.doria"), "{json}");
}

#[test]
fn include_once_loads_a_recursive_source_once() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![source(
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
        "include \"helpers.doria\"; include \"helpers.doria\"; function main(): int { return helper(); }",
    );
    provider.insert(
        "acme/application",
        "helpers.doria",
        "include \"main.doria\"; function helper(): int { return 42; }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("finite include graph");
    assert_eq!(graph.sources.len(), 2);
    assert_eq!(graph.include_edges.len(), 2);
    let analysis = analyze_compilation_graph_for_ide(&graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn included_source_identity_cannot_alias_a_different_inventoried_file() {
    let entry = "acme/application:main.doria";
    let mut occupied = source("acme/application", "occupied.doria", SourceOrigin::Explicit);
    occupied.identity = "acme/application:included.doria".to_string();
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                occupied,
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "include \"included.doria\"; function main(): int { return answer(); }",
    );
    provider.insert(
        "acme/application",
        "occupied.doria",
        "function occupied(): int { return 1; }",
    );
    provider.insert(
        "acme/application",
        "included.doria",
        "function answer(): int { return 42; }",
    );

    let failure = load_compilation_graph_detailed(&document, &provider)
        .expect_err("one source identity must never name two canonical inputs");
    assert!(failure.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("acme/application:included.doria")
            && diagnostic
                .message
                .contains("already assigned to a different canonical file")
    }));
    assert!(failure
        .source_map
        .by_path("acme/application:main.doria")
        .is_some());
}

#[test]
fn selected_entry_top_level_statements_lower_before_main() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![source(
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
        r#"class Counter
{
    static writable int $value = 0;
}

Counter::value = 42;

function main(): int
{
    return Counter::value;
}
"#,
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid entry prelude");
    let mir = doriac::lower_compilation_graph_to_mir(&graph).expect("entry prelude MIR");
    let output = doriac::mir_interpreter::interpret(&mir).expect("entry prelude execution");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn compilation_session_reuses_unchanged_parses_and_invalidates_changes() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source("acme/application", "helpers.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return helper(); }",
    );
    provider.insert(
        "acme/application",
        "helpers.doria",
        "function helper(): int { return 41; }",
    );
    let mut session = doriac::incremental::CompilationSession::new();

    let first = session
        .load_graph(&document, &provider)
        .expect("first graph update");
    assert_eq!(first.facts.parsed_sources.len(), 2);
    assert_eq!(first.facts.added_sources.len(), 2);
    let first_analysis = session.analyze_graph(&first.graph);
    assert_eq!(first_analysis.semantic_dependency_edges.len(), 1);

    let second = session
        .load_graph(&document, &provider)
        .expect("unchanged graph update");
    assert_eq!(second.facts.reused_sources.len(), 2);
    assert!(second.facts.parsed_sources.is_empty());
    assert_eq!(second.facts.reused_declaration_indexes.len(), 2);
    assert_eq!(first.graph.fingerprint, second.graph.fingerprint);

    provider.insert(
        "acme/application",
        "helpers.doria",
        "function helper(): int { return 42; }",
    );
    let third = session
        .load_graph(&document, &provider)
        .expect("changed graph update");
    assert!(third
        .facts
        .changed_sources
        .contains("acme/application:helpers.doria"));
    assert!(third
        .facts
        .reused_sources
        .contains("acme/application:main.doria"));
    assert_ne!(second.graph.fingerprint, third.graph.fingerprint);
    session.analyze_graph(&third.graph);
    assert!(third
        .facts
        .body_only_changed_sources
        .contains("acme/application:helpers.doria"));
    assert!(!session
        .last_facts()
        .invalidated_sources
        .contains("acme/application:main.doria"));

    provider.insert(
        "acme/application",
        "helpers.doria",
        "function helper(string $value): int { return 42; }",
    );
    let fourth = session
        .load_graph(&document, &provider)
        .expect("signature graph update");
    assert!(fourth
        .facts
        .declaration_changed_sources
        .contains("acme/application:helpers.doria"));
    session.analyze_graph(&fourth.graph);
    assert!(session
        .last_facts()
        .invalidated_sources
        .contains("acme/application:main.doria"));
}

#[test]
fn compilation_session_tracks_context_include_and_plan_input_changes() {
    let entry = "acme/application:main.doria";
    let mut document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source("acme/application", "helpers.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "include \"helpers.doria\"; function main(): int { return helper(); }",
    );
    provider.insert(
        "acme/application",
        "helpers.doria",
        "function helper(): int { return 41; }",
    );
    let mut session = doriac::incremental::CompilationSession::new();
    let first = session
        .load_graph(&document, &provider)
        .expect("first graph");
    session.analyze_graph(&first.graph);

    provider.insert(
        "acme/application",
        "helpers.doria",
        "function helper(): int { return 42; }",
    );
    let include_change = session
        .load_graph(&document, &provider)
        .expect("included source body change");
    assert!(include_change
        .facts
        .body_only_changed_sources
        .contains("acme/application:helpers.doria"));
    session.analyze_graph(&include_change.graph);
    assert!(session
        .last_facts()
        .invalidated_sources
        .contains("acme/application:main.doria"));

    provider.insert(
        "acme/application",
        "main.doria",
        "use Acme\\Helper; include \"helpers.doria\"; function main(): int { return helper(); }",
    );
    let import_change = session
        .load_graph(&document, &provider)
        .expect("import context change");
    assert!(import_change
        .facts
        .context_changed_sources
        .contains("acme/application:main.doria"));

    document.plan.compiler.target = CompilerTarget::Php;
    document.plan.compiler.native_profile = None;
    let backend_change = session
        .load_graph(&document, &provider)
        .expect("backend input change");
    assert!(backend_change.facts.backend_input_changed);
    assert!(!backend_change.facts.compiler_input_changed);

    document.plan.selected_target.active_scopes = vec![SourceScope::Main, SourceScope::Development];
    let target_change = session
        .load_graph(&document, &provider)
        .expect("selected target input change");
    assert!(target_change.facts.selected_target_changed);
    assert!(target_change.facts.compiler_input_changed);
    session.analyze_graph(&target_change.graph);
    assert_eq!(
        session.last_facts().invalidated_sources.len(),
        target_change.graph.sources.len()
    );
}

#[test]
fn compilation_session_additions_reconsider_prior_unresolved_references() {
    let entry = "acme/application:main.doria";
    let mut document = plan(
        vec![package(
            "acme/application",
            vec![source(
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
        "function main(): int { return answer(); }",
    );
    let mut session = doriac::incremental::CompilationSession::new();
    let first = session
        .load_graph(&document, &provider)
        .expect("partial semantic graph input");
    let first_analysis = session.analyze_graph(&first.graph);
    assert!(first_analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("answer")));

    document.plan.packages[0].sources.push(source(
        "acme/application",
        "answers.doria",
        SourceOrigin::Explicit,
    ));
    provider.insert(
        "acme/application",
        "answers.doria",
        "function answer(): int { return 42; }",
    );
    let second = session
        .load_graph(&document, &provider)
        .expect("added declaration source");
    assert!(second
        .facts
        .added_sources
        .contains("acme/application:answers.doria"));
    assert!(second
        .facts
        .reused_sources
        .contains("acme/application:main.doria"));
    let second_analysis = session.analyze_graph(&second.graph);
    assert!(
        second_analysis.diagnostics.is_empty(),
        "{:#?}",
        second_analysis.diagnostics
    );
    assert!(session
        .last_facts()
        .invalidated_sources
        .contains("acme/application:main.doria"));
}

#[test]
fn adding_an_unrelated_source_preserves_existing_source_ids() {
    let entry = "acme/application:main.doria";
    let mut document = plan(
        vec![package(
            "acme/application",
            vec![source(
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
        "function main(): int { return 0; }",
    );
    let first = load_compilation_graph(&document, &provider).expect("first graph");
    let main_id = first.sources[entry].id;

    document.plan.packages[0].sources.push(source(
        "acme/application",
        "helpers.doria",
        SourceOrigin::Explicit,
    ));
    provider.insert(
        "acme/application",
        "helpers.doria",
        "function helper(): int { return 42; }",
    );
    let second = load_compilation_graph(&document, &provider).expect("expanded graph");
    assert_eq!(second.sources[entry].id, main_id);
}

#[test]
fn library_graph_lowers_without_a_process_entry() {
    let mut document = plan(
        vec![package(
            "acme/application",
            vec![source(
                "acme/application",
                "library.doria",
                SourceOrigin::Explicit,
            )],
            Vec::new(),
        )],
        "acme/application:library.doria",
    );
    document.plan.selected_target.kind = TargetKind::Library;
    document.plan.selected_target.entry_source = None;
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "library.doria",
        "function answer(): int { return 42; }",
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid library graph");
    let mir = doriac::lower_compilation_graph_to_mir(&graph).expect("library MIR");
    assert!(mir.selected_entry.is_none());
    let diagnostics = doriac::compile_compilation_graph(&graph)
        .expect_err("libraries do not produce executable artifacts in Stage 31");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0685"));
}

#[test]
fn graph_identity_is_independent_of_source_inventory_order() {
    let entry = "acme/application:main.doria";
    let sources = vec![
        source("acme/application", "main.doria", SourceOrigin::Entry),
        source("acme/application", "helpers.doria", SourceOrigin::Explicit),
    ];
    let first = plan(
        vec![package("acme/application", sources.clone(), Vec::new())],
        entry,
    );
    let second = plan(
        vec![package(
            "acme/application",
            sources.into_iter().rev().collect(),
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return answer(); }",
    );
    provider.insert(
        "acme/application",
        "helpers.doria",
        "function answer(): int { return 42; }",
    );

    let first = load_compilation_graph(&first, &provider).expect("first graph");
    let second = load_compilation_graph(&second, &provider).expect("reordered graph");
    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(
        first
            .sources
            .iter()
            .map(|(identity, source)| (identity, source.id))
            .collect::<Vec<_>>(),
        second
            .sources
            .iter()
            .map(|(identity, source)| (identity, source.id))
            .collect::<Vec<_>>()
    );
}

#[test]
fn root_namespace_mapping_and_exact_external_filename_are_supported() {
    let entry = "acme/application:main.doria";
    let application = package(
        "acme/application",
        vec![
            source("acme/application", "main.doria", SourceOrigin::Entry),
            source(
                "acme/application",
                "Acme/Domain/Person.doria",
                SourceOrigin::Explicit,
            ),
        ],
        Vec::new(),
    );
    let document = plan(vec![application], entry);
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return 0; }",
    );
    provider.insert(
        "acme/application",
        "Acme/Domain/Person.doria",
        "namespace Acme\\Domain; class Person {}",
    );

    load_compilation_graph(&document, &provider).expect("root namespace mapping");
}

#[test]
fn multi_source_graph_emits_through_all_enabled_backends_without_runtime_includes() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source("acme/application", "helpers.doria", SourceOrigin::Explicit),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        "function main(): int { return answer(); }",
    );
    provider.insert(
        "acme/application",
        "helpers.doria",
        "function answer(): int { return 42; }",
    );
    let graph = load_compilation_graph(&document, &provider).expect("valid graph");

    let native = doriac::compile_compilation_graph(&graph).expect("Cranelift graph emission");
    assert!(matches!(
        native,
        doriac::backend::BackendOutput::Executable { .. }
    ));

    #[cfg(feature = "llvm-backend")]
    {
        let mut llvm_graph = graph.clone();
        llvm_graph.build_plan.compiler.native_profile = Some(BuildNativeProfile::Release);
        let llvm = doriac::compile_compilation_graph(&llvm_graph).expect("LLVM graph emission");
        assert!(matches!(
            llvm,
            doriac::backend::BackendOutput::Executable { .. }
        ));
    }

    let mut php_graph = graph;
    php_graph.build_plan.compiler.target = CompilerTarget::Php;
    php_graph.build_plan.compiler.native_profile = None;
    let php = doriac::compile_compilation_graph(&php_graph).expect("PHP graph emission");
    let doriac::backend::BackendOutput::Text { contents, .. } = php else {
        panic!("PHP backend must emit text");
    };
    assert!(!contents.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("include ")
            || line.starts_with("include(")
            || line.starts_with("require ")
            || line.starts_with("require(")
    }));
    assert!(!contents.contains("spl_autoload_register"));
    let php_path = std::env::temp_dir().join(format!("doria-stage31-{}.php", std::process::id()));
    std::fs::write(&php_path, contents).expect("write generated PHP");
    let syntax = Command::new("php").arg("-l").arg(&php_path).output();
    let _ = std::fs::remove_file(&php_path);
    if let Ok(syntax) = syntax {
        assert!(
            syntax.status.success(),
            "{}",
            String::from_utf8_lossy(&syntax.stderr)
        );
    }
}

#[test]
fn ambient_and_finalizer_effects_flow_across_source_graphs() {
    let entry = "acme/application:main.doria";
    let document = plan(
        vec![package(
            "acme/application",
            vec![
                source("acme/application", "main.doria", SourceOrigin::Entry),
                source(
                    "acme/application",
                    "CleanupError.doria",
                    SourceOrigin::Explicit,
                ),
            ],
            Vec::new(),
        )],
        entry,
    );
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/application",
        "main.doria",
        r#"
function main(): void
{
    try { run(); } catch (CleanupError $error) { echo "outer {$error->message}\n"; }
}
"#,
    );
    provider.insert(
        "acme/application",
        "CleanupError.doria",
        r#"
class CleanupError implements Error
{
    function __construct(string $message) {}
}

function writeMarker(): void
{
    echo "helper ";
}

function run(): void throws CleanupError
{
    try { writeMarker(); } finally { throw new CleanupError("cleanup"); }
}
"#,
    );

    let graph = load_compilation_graph(&document, &provider).expect("valid ambient graph");
    let mir = doriac::lower_compilation_graph_to_mir(&graph)
        .expect("cross-file ambient and finalizer effects should lower");
    let profile = |name: &str| {
        mir.functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("{name} should exist in graph MIR"))
    };
    assert!(profile("writeMarker").required_checked_effects.is_empty());
    assert_eq!(profile("writeMarker").ambient_checked_effects.len(), 1);
    assert_eq!(profile("run").required_checked_effects.len(), 1);
    assert_eq!(profile("run").ambient_checked_effects.len(), 1);
    assert!(profile("main").required_checked_effects.is_empty());
    assert_eq!(profile("main").ambient_checked_effects.len(), 1);

    let interpreted = doriac::mir_interpreter::interpret(&mir)
        .expect("cross-file ambient finalizer fixture should execute");
    assert_eq!(interpreted.stdout, b"helper outer cleanup\n");
    assert_eq!(interpreted.exit_status, 0);
}
