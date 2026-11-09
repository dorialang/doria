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
fn group_defaults_inject_concurrency_once_without_a_user_concurrency_import() {
    let document = plan(vec![package(
        "acme/app",
        &["main.doria", "start.doria"],
        &[],
    )]);
    let mut provider = InMemorySourceProvider::new();
    provider.insert("acme/app", "main.doria", "function main(): void {}");
    provider.insert(
        "acme/app",
        "start.doria",
        r#"
use Doria\Std\Io\{ReadinessSet, TransferPlan, TransferGroup, IoError};
function start(writable ReadinessSet $set, take TransferPlan $plan): TransferGroup throws IoError {
    return $set->begin($plan);
}
"#,
    );
    let mut session = CompilationSession::new();
    let loaded = session.load_graph(&document, &provider).unwrap();
    let analysis = session.analyze_graph(&loaded.graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let program = doriac::lower_compilation_graph_to_mir(&loaded.graph).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    assert_eq!(
        program
            .sources
            .iter()
            .filter(
                |source| source.id == doriac::compiler_known_concurrency::SOURCE_ID
                    && source.source.text == doriac::compiler_known_concurrency::SOURCE_TEXT
            )
            .count(),
        1
    );
    assert_eq!(
        program
            .classes
            .iter()
            .filter(|class| class.declaration_name
                == doriac::compiler_known_concurrency::CANCELLATION_TOKEN)
            .count(),
        1
    );
}

#[test]
fn stream_contracts_resolve_once_across_multifile_graph_and_ide_analysis() {
    let document = plan(vec![package(
        "acme/app",
        &["main.doria", "Reader.doria"],
        &[],
    )]);
    let mut provider = InMemorySourceProvider::new();
    provider.insert(
        "acme/app",
        "Reader.doria",
        r#"
use Doria\Std\Io\{ReadableStream, ReadOutcome, WritableByteRegion, IoError};
class Reader implements ReadableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError {
        if ($destination->length == 0) { return ReadOutcome::Data(0); }
        $destination[0] = 42;
        return ReadOutcome::Data(1);
    }
}
function read(writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome throws IoError {
    return $stream->read($destination);
}
"#,
    );
    provider.insert(
        "acme/app",
        "main.doria",
        r#"
use Doria\Std\Io\ReadOutcome as Progress;
function main(): void {
    let writable $stream = new Reader();
    writable Bytes $bytes = Bytes::allocate(1);
    {
        let writable $destination = $bytes->getWritableRegion(0, 1);
        let $outcome = read($stream, $destination);
        echo match ($outcome) {
            Progress::Data($byteCount) => "count {$byteCount}\n",
            default => "unexpected outcome\n",
        };
    }
    echo "byte {$bytes[0]}\n";
}
"#,
    );
    let mut session = CompilationSession::new();
    let loaded = session.load_graph(&document, &provider).unwrap();
    let analysis = session.analyze_graph(&loaded.graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let program = doriac::lower_compilation_graph_to_mir(&loaded.graph)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(
        program
            .sources
            .iter()
            .filter(|source| {
                source.id == doriac::compiler_known_stream::SOURCE_ID
                    && source.source.text == doriac::compiler_known_stream::SOURCE_TEXT
            })
            .count(),
        1,
        "{:?}",
        program
            .sources
            .iter()
            .map(|source| (
                source.id,
                source.display_path.as_str(),
                source.source.text.len()
            ))
            .collect::<Vec<_>>()
    );
    // Programs include only the std declarations they reach (decision log
    // entry 41): this one reads, so `WriteOutcome` is absent, never doubled.
    for (name, expected) in [("ReadOutcome", 1), ("WriteOutcome", 0)] {
        assert_eq!(
            program
                .enums
                .iter()
                .filter(|definition| definition.name == format!("Doria\\Std\\Io\\{name}"))
                .count(),
            expected
        );
    }
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(result.stdout, b"count 1\nbyte 42\n");
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_status, 0);
    let mut missing_source = program.clone();
    missing_source
        .sources
        .retain(|source| source.id != doriac::compiler_known_stream::SOURCE_ID);
    assert!(doriac::mir_validation::validate_program(&missing_source).is_err());
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

#[test]
fn consuming_receiver_edits_invalidate_class_interface_and_trait_dependents() {
    for (kind, dependent_file, dependent, main) in [
        (
            "class",
            "Dependent.doria",
            "function perform(take Contract $value): int { let writable $receiver = $value; return $receiver->finish(); }",
            "function main(): void { let $answer = perform(new Contract()); }",
        ),
        (
            "interface",
            "ChildContract.doria",
            "interface ChildContract extends Contract {}",
            "function perform(take ChildContract $value): int { let writable $receiver = $value; return $receiver->finish(); } function main(): void {}",
        ),
        (
            "trait",
            "Resource.doria",
            "class Resource { uses Contract; }",
            "function main(): void { let writable $resource = new Resource(); let $answer = $resource->finish(); }",
        ),
    ] {
        let document = plan(vec![package(
            "acme/app",
            &["main.doria", "Contract.doria", dependent_file, "Unrelated.doria"],
            &[],
        )]);
        let mut provider = InMemorySourceProvider::new();
        provider.insert("acme/app", dependent_file, dependent);
        provider.insert("acme/app", "main.doria", main);
        provider.insert("acme/app", "Unrelated.doria", "function unrelated(): int { return 7; }");
        let dependent_identity = format!("acme/app:{dependent_file}");
        let mut session = CompilationSession::new();
        // This walk covers every ordered unequal readonly/writable/consuming pair.
        for (revision, receiver) in ["", "writable ", "consuming ", "", "consuming ", "writable ", ""].into_iter().enumerate() {
            let body = if kind == "interface" { ";" } else { " { return 42; }" };
            provider.insert(
                "acme/app",
                "Contract.doria",
                format!("{kind} Contract {{ {receiver}function finish(): int{body} }}"),
            );
            let loaded = session.load_graph(&document, &provider).unwrap();
            let analysis = session.analyze_graph(&loaded.graph);
            assert!(analysis.diagnostics.is_empty(), "{kind}, revision {revision}: {:?}", analysis.diagnostics);
            if revision == 0 {
                continue;
            }
            let facts = session.last_facts();
            assert!(
                facts.declaration_changed_sources.contains("acme/app:Contract.doria"),
                "{kind}, revision {revision}: {facts:?}"
            );
            assert!(
                !facts.body_only_changed_sources.contains("acme/app:Contract.doria"),
                "receiver ownership is a declaration contract: {kind}, revision {revision}: {facts:?}"
            );
            for source in ["acme/app:Contract.doria", dependent_identity.as_str(), "acme/app:main.doria"] {
                assert!(facts.invalidated_sources.contains(source), "{kind}, revision {revision}, {source}: {facts:?}");
            }
            assert!(
                !facts.invalidated_sources.contains("acme/app:Unrelated.doria"),
                "{kind}, revision {revision}: {facts:?}"
            );
        }
    }
}
