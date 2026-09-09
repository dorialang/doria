use doriac::build_plan::{
    BuildNativeProfile, BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget, Dependency,
    DependencyKind, NamespaceMapping, Package, SelectedTarget, Source, SourceOrigin, SourceScope,
    TargetKind,
};
use doriac::incremental::CompilationSession;
use doriac::semantics::contracts::ConformanceStatus;
use doriac::source_provider::InMemorySourceProvider;

fn package(identity: &str, files: &[&str], dependencies: &[&str]) -> Package {
    Package {
        identity: identity.into(),
        root: ".".into(),
        namespace_mappings: vec![NamespaceMapping {
            prefix: String::new(),
            path: String::new(),
            scope: SourceScope::Main,
            generated_for: None,
        }],
        sources: files
            .iter()
            .map(|path| Source {
                identity: format!("{identity}:{path}"),
                path: (*path).into(),
                scope: SourceScope::Main,
                origin: if *path == "main.doria" {
                    SourceOrigin::Entry
                } else {
                    SourceOrigin::Explicit
                },
                generated_for: None,
            })
            .collect(),
        dependencies: dependencies
            .iter()
            .map(|dependency| Dependency {
                package: (*dependency).into(),
                kind: DependencyKind::Normal,
            })
            .collect(),
    }
}

fn plan(packages: Vec<Package>) -> BuildPlanDocument {
    BuildPlanDocument {
        path: "plan.json".into(),
        directory: std::env::current_dir().unwrap(),
        text: String::new(),
        plan: BuildPlan {
            schema_version: 1,
            edition: "2026".into(),
            root_package: "acme/app".into(),
            selected_target: SelectedTarget {
                package: "acme/app".into(),
                name: "app".into(),
                kind: TargetKind::Binary,
                entry_source: Some("acme/app:main.doria".into()),
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
fn generic_diamond_edits_invalidate_transitive_cross_package_conformance() {
    let mut document = plan(vec![
        package(
            "acme/app",
            &["main.doria", "Base.doria", "Child.doria", "Unrelated.doria"],
            &["acme/contracts"],
        ),
        package(
            "acme/contracts",
            &["Root.doria", "Left.doria", "Right.doria", "Both.doria"],
            &[],
        ),
    ]);
    document.plan.packages[1].namespace_mappings[0].prefix = "Contracts\\".into();
    let mut provider = InMemorySourceProvider::new();
    for (path, text) in [
        (
            "Root.doria",
            "namespace Contracts; interface Root<T> { function value(T $input): int; }",
        ),
        (
            "Left.doria",
            "namespace Contracts; interface Left<T> extends Root<T> {}",
        ),
        (
            "Right.doria",
            "namespace Contracts; interface Right<T> extends Root<T> {}",
        ),
        (
            "Both.doria",
            "namespace Contracts; interface Both<T> extends Left<T>, Right<T> {}",
        ),
    ] {
        provider.insert("acme/contracts", path, text);
    }
    for (path, text) in [
        ("Base.doria", "use Contracts\\Both as Contract; open class Base implements Contract<int> { function value(int $input): int { return $input; } }"),
        ("Child.doria", "class Child extends Base {}"),
        ("Unrelated.doria", "function unrelated(): int { return 1; }"),
        ("main.doria", "function main(): void { let $child = new Child(); echo $child->value(3); }"),
    ] { provider.insert("acme/app", path, text); }
    let mut session = CompilationSession::new();
    let initial = session.load_graph(&document, &provider).unwrap();
    let analysis = session.analyze_graph(&initial.graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis
        .semantic_info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.status == ConformanceStatus::Checked));

    provider.insert(
        "acme/contracts",
        "Root.doria",
        "namespace Contracts; interface Root<T> { function value(T $renamed): int; }",
    );
    let changed = session.load_graph(&document, &provider).unwrap();
    let analysis = session.analyze_graph(&changed.graph);
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0755"),
        "{:?}",
        analysis.diagnostics
    );
    for source in [
        "acme/contracts:Root.doria",
        "acme/contracts:Left.doria",
        "acme/contracts:Right.doria",
        "acme/contracts:Both.doria",
        "acme/app:Base.doria",
        "acme/app:Child.doria",
        "acme/app:main.doria",
    ] {
        assert!(
            session.last_facts().invalidated_sources.contains(source),
            "{source}: {:?}",
            session.last_facts()
        );
    }
    assert!(!session
        .last_facts()
        .invalidated_sources
        .contains("acme/app:Unrelated.doria"));
    let mismatch = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0755")
        .unwrap();
    assert!(mismatch
        .related
        .iter()
        .any(|related| related.span.source != mismatch.span.source));

    provider.insert(
        "acme/contracts",
        "Root.doria",
        "namespace Contracts; interface Root<T> { function value(T $input): int; }",
    );
    let restored = session.load_graph(&document, &provider).unwrap();
    assert!(session
        .analyze_graph(&restored.graph)
        .diagnostics
        .is_empty());
    provider.insert(
        "acme/app",
        "Unrelated.doria",
        "function unrelated(): int { return 2; }",
    );
    let body_edit = session.load_graph(&document, &provider).unwrap();
    session.analyze_graph(&body_edit.graph);
    assert!(session
        .last_facts()
        .body_only_changed_sources
        .contains("acme/app:Unrelated.doria"));
    assert!(!session
        .last_facts()
        .invalidated_sources
        .contains("acme/app:Base.doria"));
}

#[test]
fn trait_adaptations_and_uses_invalidate_composed_consumers() {
    let document = plan(vec![package(
        "acme/app",
        &["main.doria", "Formatting.doria", "Record.doria"],
        &[],
    )]);
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/app",
        "main.doria",
        "function main(): void { let $record = new Record(); }",
    );
    provider.insert(
        "acme/app",
        "Formatting.doria",
        "trait Formatting { function format(): string { return \"record\"; } }",
    );
    provider.insert(
        "acme/app",
        "Record.doria",
        "class Record { uses Formatting { Formatting::format as text; } }",
    );
    let mut session = CompilationSession::new();
    let initial = session.load_graph(&document, &provider).unwrap();
    let analysis = session.analyze_graph(&initial.graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis
        .semantic_dependency_edges
        .iter()
        .any(|edge| edge.source.0 == "acme/app:Record.doria"
            && edge.target.0 == "acme/app:Formatting.doria"));
    provider.insert(
        "acme/app",
        "Record.doria",
        "class Record { uses Formatting { Formatting::format as internal text; } }",
    );
    let changed = session.load_graph(&document, &provider).unwrap();
    session.analyze_graph(&changed.graph);
    assert!(session
        .last_facts()
        .declaration_changed_sources
        .contains("acme/app:Record.doria"));
    assert!(session
        .last_facts()
        .invalidated_sources
        .contains("acme/app:main.doria"));
    provider.insert(
        "acme/app",
        "Formatting.doria",
        "trait Formatting { function format(): string { return \"changed body\"; } }",
    );
    let changed = session.load_graph(&document, &provider).unwrap();
    let analysis = session.analyze_graph(&changed.graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(session
        .last_facts()
        .declaration_changed_sources
        .contains("acme/app:Formatting.doria"));
    assert!(session
        .last_facts()
        .invalidated_sources
        .contains("acme/app:Record.doria"));
    assert!(session
        .last_facts()
        .invalidated_sources
        .contains("acme/app:main.doria"));
}

#[test]
fn trait_bodies_preserve_definition_site_names_across_packages_and_edits() {
    let mut document = plan(vec![
        package(
            "acme/app",
            &["main.doria", "Record.doria"],
            &["acme/format"],
        ),
        package("acme/format", &["Formatting.doria", "prefix.doria"], &[]),
    ]);
    document.plan.packages[0].namespace_mappings[0].prefix = "App\\".into();
    document.plan.packages[1].namespace_mappings[0].prefix = "Format\\".into();
    let mut provider = InMemorySourceProvider::new();
    provider.insert("acme/app", "main.doria", "namespace App; function main(): void { echo (new Record())->text(); } function prefix(): string { return \"wrong:\"; }");
    provider.insert("acme/app", "Record.doria", "namespace App; use Format\\Formatting as Imported; class Record { uses Imported { Imported::format as text; } }");
    provider.insert(
        "acme/format",
        "prefix.doria",
        "namespace Format; internal function prefix(): string { return \"right:\"; }",
    );
    let mut session = CompilationSession::new();
    for (declaration, expected) in [
        ("trait Formatting { string $suffix = \"one\"; function format(): string { return prefix() . $this->suffix; } }", "right:one"),
        ("trait Formatting { string $suffix = \"two\"; function format(): string { return prefix() . $this->suffix; } }", "right:two"),
        ("trait Formatting { string $suffix = \"two\"; function format(string $tail = \"!\"): string { return prefix() . $this->suffix . $tail; } }", "right:two!"),
    ] {
        provider.insert("acme/format", "Formatting.doria", format!("namespace Format; {declaration}"));
        let loaded = session.load_graph(&document, &provider).unwrap();
        let analysis = session.analyze_graph(&loaded.graph);
        assert!(analysis.diagnostics.is_empty(), "{:?}", analysis.diagnostics);
        let mir = doriac::lower_compilation_graph_to_mir(&loaded.graph).unwrap();
        let output = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(session.last_facts().invalidated_sources.contains("acme/app:Record.doria"));
        assert!(session.last_facts().invalidated_sources.contains("acme/app:main.doria"));
    }
}
