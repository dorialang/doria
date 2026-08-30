use doriac::build_plan::{
    BuildNativeProfile, BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget,
    GeneratedFor, NamespaceMapping, Package, SelectedTarget, Source, SourceOrigin, SourceScope,
    TargetKind,
};
use doriac::compilation_graph::{analyze_compilation_graph_for_ide, load_compilation_graph};
use doriac::source_provider::InMemorySourceProvider;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PACKAGE: &str = "acme/tests";
const TEST_SOURCE: &str = "acme/tests:tests.doria";

fn document(scope: SourceScope, generated_for: Option<GeneratedFor>) -> BuildPlanDocument {
    BuildPlanDocument {
        path: "build-plan.json".to_string(),
        directory: std::env::current_dir().expect("current directory"),
        text: String::new(),
        plan: BuildPlan {
            schema_version: 1,
            edition: "2026".to_string(),
            root_package: PACKAGE.to_string(),
            selected_target: SelectedTarget {
                package: PACKAGE.to_string(),
                name: "tests".to_string(),
                kind: TargetKind::Library,
                entry_source: None,
                active_scopes: vec![
                    SourceScope::Main,
                    SourceScope::Development,
                    SourceScope::Generated,
                ],
            },
            packages: vec![Package {
                identity: PACKAGE.to_string(),
                root: ".".to_string(),
                namespace_mappings: vec![NamespaceMapping {
                    prefix: String::new(),
                    path: String::new(),
                    scope,
                    generated_for,
                }],
                sources: vec![Source {
                    identity: TEST_SOURCE.to_string(),
                    path: "tests.doria".to_string(),
                    scope,
                    origin: if scope == SourceScope::Generated {
                        SourceOrigin::Generated
                    } else {
                        SourceOrigin::Explicit
                    },
                    generated_for,
                }],
                dependencies: Vec::new(),
            }],
            compiler: CompilerOptions {
                target: CompilerTarget::Debug,
                native_profile: None,
                target_triple: None,
            },
        },
    }
}

fn graph(
    source: &str,
    scope: SourceScope,
    generated_for: Option<GeneratedFor>,
) -> doriac::compilation_graph::CompilationGraph {
    let mut provider = InMemorySourceProvider::new();
    provider.insert(PACKAGE, "tests.doria", source);
    load_compilation_graph(&document(scope, generated_for), &provider).expect("test graph")
}

fn dispatcher_graph(
    test_source: &str,
    dispatcher: &str,
    target: CompilerTarget,
    native_profile: Option<BuildNativeProfile>,
) -> doriac::compilation_graph::CompilationGraph {
    let test_identity = format!("{PACKAGE}:tests.doria");
    let dispatcher_identity = format!("{PACKAGE}:dispatcher.doria");
    let document = BuildPlanDocument {
        path: "build-plan.json".to_string(),
        directory: std::env::current_dir().expect("current directory"),
        text: String::new(),
        plan: BuildPlan {
            schema_version: 1,
            edition: "2026".to_string(),
            root_package: PACKAGE.to_string(),
            selected_target: SelectedTarget {
                package: PACKAGE.to_string(),
                name: "tests".to_string(),
                kind: TargetKind::Binary,
                entry_source: Some(dispatcher_identity.clone()),
                active_scopes: vec![
                    SourceScope::Main,
                    SourceScope::Development,
                    SourceScope::Generated,
                ],
            },
            packages: vec![Package {
                identity: PACKAGE.to_string(),
                root: ".".to_string(),
                namespace_mappings: vec![
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
                        identity: test_identity,
                        path: "tests.doria".to_string(),
                        scope: SourceScope::Development,
                        origin: SourceOrigin::Explicit,
                        generated_for: None,
                    },
                    Source {
                        identity: dispatcher_identity,
                        path: "dispatcher.doria".to_string(),
                        scope: SourceScope::Generated,
                        origin: SourceOrigin::Entry,
                        generated_for: Some(GeneratedFor::Development),
                    },
                ],
                dependencies: Vec::new(),
            }],
            compiler: CompilerOptions {
                target,
                native_profile,
                target_triple: None,
            },
        },
    };
    let mut provider = InMemorySourceProvider::new();
    provider.insert(PACKAGE, "tests.doria", test_source);
    provider.insert(PACKAGE, "dispatcher.doria", dispatcher);
    load_compilation_graph(&document, &provider).expect("dispatcher graph")
}

fn assertion_mir(source: &str) -> doriac::mir::Program {
    let metadata =
        doriac::metadata_compilation_graph_v3(&graph(source, SourceScope::Development, None))
            .expect("assertion metadata");
    let callable = &metadata.tests[0]
        .callable
        .as_ref()
        .expect("generated assertion callable")
        .canonical_name;
    doriac::lower_compilation_graph_to_mir(&dispatcher_graph(
        source,
        &format!("function main(): void {{ {callable}(); }}"),
        CompilerTarget::Debug,
        None,
    ))
    .expect("valid assertion source should lower")
}

fn assertion_plan_mut(program: &mut doriac::mir::Program) -> &mut doriac::mir::AssertionPlan {
    program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::Assertion(
                plan,
            )) => Some(plan.as_mut()),
            _ => None,
        })
        .expect("assertion plan")
}

fn assert_malformed_assertion_mir(program: &doriac::mir::Program, expected: &str) {
    let error = doriac::mir_validation::validate_program(program)
        .expect_err("malformed assertion MIR must stop before backend execution");
    assert!(
        error.message.contains(expected),
        "expected {expected:?}, got {:?}",
        error.message
    );
}

fn temporary_path(extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    let mut path = std::env::temp_dir().join(format!(
        "doriac-native-testing-{}-{nanos}",
        std::process::id()
    ));
    if !extension.is_empty() {
        path.set_extension(extension);
    }
    path
}

fn run_emitted(output: doriac::backend::BackendOutput) -> std::process::Output {
    let doriac::backend::BackendOutput::Executable { bytes, extension } = output else {
        panic!("native backend must emit an executable");
    };
    let path = temporary_path(&extension);
    fs::write(&path, bytes).expect("write executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path)
            .expect("executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("executable permissions");
    }
    let output =
        retry_transient_executable_busy(|| Command::new(&path).output()).expect("run executable");
    let _ = fs::remove_file(path);
    output
}

fn run_emitted_with_assertion_outcome(
    output: doriac::backend::BackendOutput,
) -> (std::process::Output, Vec<u8>) {
    let doriac::backend::BackendOutput::Executable { bytes, extension } = output else {
        panic!("native backend must emit an executable");
    };
    let path = temporary_path(&extension);
    let outcome = temporary_path("outcome");
    fs::write(&path, bytes).expect("write executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path)
            .expect("executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("executable permissions");
    }
    let output = retry_transient_executable_busy(|| {
        Command::new(&path)
            .env("DORIA_RUNTIME_OUTCOME_V3", &outcome)
            .env("DORIA_RUNTIME_OUTCOME_V4", &outcome)
            .output()
    })
    .expect("run executable");
    let payload = fs::read(&outcome).expect("assertion outcome");
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(outcome);
    (output, payload)
}

fn retry_transient_executable_busy<T>(
    mut operation: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    const MAX_ATTEMPTS: usize = 20;
    for attempt in 0..MAX_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_transient_executable_busy(&error) && attempt + 1 < MAX_ATTEMPTS => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("retry loop returns on its final attempt")
}

fn is_transient_executable_busy(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(26)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn first_behavioral_dispatcher(source: &str) -> String {
    let metadata =
        doriac::metadata_compilation_graph_v3(&graph(source, SourceScope::Development, None))
            .expect("behavioral metadata");
    let callable = &metadata.tests[0]
        .callable
        .as_ref()
        .expect("generated callable")
        .canonical_name;
    format!("function main(): void {{ {callable}(); }}")
}

fn assert_behavioral_output_on_every_enabled_backend(source: &str, expected_stdout: &[u8]) {
    let dispatcher = first_behavioral_dispatcher(source);
    let debug = doriac::compile_compilation_graph(&dispatcher_graph(
        source,
        &dispatcher,
        CompilerTarget::Debug,
        None,
    ))
    .expect("debug dispatcher");
    let doriac::backend::BackendOutput::Text { contents, .. } = debug else {
        panic!("debug backend must emit text");
    };
    assert!(
        contents.contains(&format!(
            "stdout: {}",
            String::from_utf8_lossy(expected_stdout)
        )),
        "{contents}"
    );

    if Command::new(if cfg!(windows) { "cl.exe" } else { "cc" })
        .arg("--version")
        .output()
        .is_ok()
    {
        for profile in [BuildNativeProfile::Fast, BuildNativeProfile::Release] {
            if profile == BuildNativeProfile::Release && !cfg!(feature = "llvm-backend") {
                continue;
            }
            let native = doriac::compile_compilation_graph(&dispatcher_graph(
                source,
                &dispatcher,
                CompilerTarget::Native,
                Some(profile),
            ))
            .unwrap_or_else(|error| panic!("{profile:?} dispatcher: {error:?}"));
            let output = run_emitted(native);
            assert_eq!(output.status.code(), Some(0), "{profile:?}");
            assert_eq!(output.stdout, expected_stdout, "{profile:?}");
            assert!(
                output.stderr.is_empty(),
                "{profile:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    if Command::new("php").arg("--version").output().is_ok() {
        let php = doriac::compile_compilation_graph(&dispatcher_graph(
            source,
            &dispatcher,
            CompilerTarget::Php,
            None,
        ))
        .expect("PHP dispatcher");
        let doriac::backend::BackendOutput::Text { contents, .. } = php else {
            panic!("PHP backend must emit text");
        };
        for forbidden in ["assert(", "PHPUnit", "Pest", "Reflection", "testRegistry"] {
            assert!(
                !contents.contains(forbidden),
                "generated PHP contains {forbidden}"
            );
        }
        let path = temporary_path("php");
        fs::write(&path, contents).expect("write PHP");
        let lint = Command::new("php")
            .arg("-l")
            .arg(&path)
            .output()
            .expect("lint PHP");
        assert!(
            lint.status.success(),
            "{}",
            String::from_utf8_lossy(&lint.stderr)
        );
        let output = Command::new("php").arg(&path).output().expect("run PHP");
        let _ = fs::remove_file(path);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, expected_stdout);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn behavioral_declarations_elaborate_into_unified_metadata_and_hir() {
    let source = r#"
use Doria\Std\Test\{describe, it, test as scenario};

const string SUBJECT = "Shopping " . "Cart";

#[Test]
function lowLevel(): void {}

describe(SUBJECT, function (): void {
    describe("when empty", function (): void {
        it("has zero total", function (): void {
            echo "behavioral\n";
        });
    });

    scenario("accepts alias", fn() => writeGreeting());
});

function writeGreeting(): void {
    echo "alias\n";
}
"#;
    let graph = graph(source, SourceScope::Development, None);
    let analysis = analyze_compilation_graph_for_ide(&graph);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );

    let metadata = doriac::metadata_compilation_graph_v3(&graph).expect("schema 3 metadata");
    assert_eq!(metadata.schema_version, 3);
    assert_eq!(metadata.test_suites.len(), 2);
    assert_eq!(metadata.tests.len(), 3);
    let behavioral = metadata
        .tests
        .iter()
        .filter(|test| {
            matches!(
                test.origin,
                doriac::attributes::MetadataTestOriginV3::Behavioral
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(behavioral.len(), 2);
    assert_eq!(
        behavioral[0].display_name,
        "Shopping Cart > when empty > has zero total"
    );
    assert!(behavioral.iter().all(|test| test.executable));
    assert!(behavioral.iter().all(|test| test.callable.is_some()));
    assert!(behavioral.iter().all(|test| {
        test.callable
            .as_ref()
            .is_some_and(|callable| callable.canonical_name.contains("__doria_test_"))
    }));
    let low_level = metadata
        .tests
        .iter()
        .find(|test| {
            matches!(
                test.origin,
                doriac::attributes::MetadataTestOriginV3::Attribute
            )
        })
        .expect("low-level test projection");
    assert_eq!(low_level.display_name, "lowLevel");
    assert!(low_level.executable);

    let hir = doriac::lower_compilation_graph(&graph).expect("test HIR");
    assert_eq!(hir.test_suites.len(), 2);
    assert_eq!(hir.tests.len(), 3);
    assert!(!hir
        .items
        .iter()
        .any(|item| matches!(item, doriac::hir::Item::Statement(_))));
    assert_eq!(
        hir.items
            .iter()
            .filter(|item| matches!(item, doriac::hir::Item::Function(function) if function.name.contains("__doria_test_")))
            .count(),
        2
    );
    let mir = doriac::lower_compilation_graph_to_mir(&graph).expect("ordinary generated MIR");
    assert!(mir
        .functions
        .iter()
        .any(|function| function.name.contains("__doria_test_")));
}

#[test]
fn behavioral_elaboration_preserves_authored_import_alias_facts() {
    let source = r#"
use Doria\Std\Io\IoError as Failure;

function inspect(Failure $failure): Failure {
    return $failure;
}
"#;
    let analysis =
        analyze_compilation_graph_for_ide(&graph(source, SourceScope::Development, None));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let alias_references = analysis
        .semantic_info
        .global_symbols
        .references
        .iter()
        .filter(|reference| reference.source_spelling == "Failure")
        .collect::<Vec<_>>();
    assert_eq!(alias_references.len(), 2);
    assert!(alias_references.iter().all(|reference| {
        reference.import_alias.as_deref() == Some("Failure")
            && reference.symbol_id.qualified_name == "Doria\\Std\\Io\\IoError"
    }));
}

#[test]
fn behavioral_identity_and_metadata_are_deterministic() {
    let source = r#"
use Doria\Std\Test\it;
it("stable", function (): void {});
"#;
    let graph = graph(source, SourceScope::Development, None);
    let first = doriac::metadata_compilation_graph_v3(&graph).expect("first metadata");
    let second = doriac::metadata_compilation_graph_v3(&graph).expect("second metadata");
    assert_eq!(first, second);
    assert_eq!(first.tests[0].identity, second.tests[0].identity);
}

#[test]
fn generated_behavioral_callables_preserve_required_and_ambient_effects() {
    let source = r#"
use Doria\Std\Test\it;

internal class Failure implements Error
{
    function __construct(string $message) {}
}

function fail(): void throws Failure
{
    throw new Failure("stop");
}

it("effects", function (): void {
    echo "ambient";
    fail();
});
"#;
    let metadata =
        doriac::metadata_compilation_graph_v3(&graph(source, SourceScope::Development, None))
            .expect("effectful behavioral metadata");
    let generated_identity = &metadata.tests[0]
        .callable
        .as_ref()
        .expect("generated callable")
        .identity;
    let generated = metadata
        .callables
        .iter()
        .find(|callable| &callable.identity == generated_identity)
        .expect("generated callable metadata");
    assert_eq!(generated.required_effects, ["Failure"]);
    assert_eq!(generated.ambient_effects, ["Doria\\Std\\Io\\IoError"]);
}

#[test]
fn generated_development_sources_are_accepted_but_main_sources_are_not() {
    let source = r#"
use Doria\Std\Test\it;
it("scoped", function (): void {});
"#;
    let generated = graph(
        source,
        SourceScope::Generated,
        Some(GeneratedFor::Development),
    );
    assert!(analyze_compilation_graph_for_ide(&generated)
        .diagnostics
        .is_empty());

    let main = graph(source, SourceScope::Main, None);
    let diagnostics = analyze_compilation_graph_for_ide(&main).diagnostics;
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0701")
            .count(),
        1,
        "{diagnostics:#?}"
    );
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0683"));
}

#[test]
fn malformed_behavioral_shapes_and_unterminated_expectations_have_single_boundaries() {
    let source = r#"
use Doria\Std\Test\{describe, it, expect, fail};
describe("bad", function (): void { echo "setup"; });
it("parameters", function (int $value): void {});
function malformed(): void { expect(1); }
"#;
    let graph = graph(source, SourceScope::Development, None);
    let diagnostics = analyze_compilation_graph_for_ide(&graph).diagnostics;
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0708"));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0706"));
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0714")
            .count(),
        1,
        "{diagnostics:#?}"
    );
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0710"));
}

#[test]
fn user_functions_with_test_like_short_names_remain_ordinary() {
    let source = r#"
function it(string $name): void { echo $name; }
function main(): void { it("ordinary"); }
"#;
    doriac::check_source("ordinary.doria", source).expect("ordinary short-name function");
}

#[test]
fn nested_test_references_do_not_reclassify_the_outer_call() {
    let source = r#"
use Doria\Std\Test\it;

function wrapper(string $name, function(): void $body): void {}

wrapper("outer", function (): void {
    it("nested", function (): void {});
});
"#;
    let analysis =
        analyze_compilation_graph_for_ide(&graph(source, SourceScope::Development, None));

    assert!(
        analysis.semantic_info.test_semantics.tests.is_empty(),
        "a nested test reference must not turn its containing call into a test: {:#?}",
        analysis.semantic_info.test_semantics.tests
    );
    assert!(
        analysis
            .semantic_info
            .test_semantics
            .compiler_elided_statement_spans
            .is_empty(),
        "the ordinary outer call must remain in the authored program"
    );
}

#[test]
fn invalid_constants_do_not_erase_valid_test_descriptions() {
    let source = r#"
use Doria\Std\Test\it;

const string NAME = "works";
const int BROKEN = 1 / 0;

it(NAME, function (): void {});
"#;
    let analysis =
        analyze_compilation_graph_for_ide(&graph(source, SourceScope::Development, None));

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.starts_with("E04")),
        "the invalid constant must retain its own diagnostic: {:#?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "E0704"),
        "an unrelated constant failure must not invalidate NAME: {:#?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.semantic_info.test_semantics.tests.len(), 1);
    assert_eq!(
        analysis.semantic_info.test_semantics.tests[0].display_name,
        "works"
    );
}

#[test]
fn metadata_schema_three_is_strict_and_older_schemas_remain_disjoint() {
    let source = "function main(): void {}";
    let v1 = serde_json::to_string(&doriac::metadata_source("main.doria", source).unwrap())
        .expect("schema 1 JSON");
    let v2 = serde_json::to_string(&doriac::metadata_source_v2("main.doria", source).unwrap())
        .expect("schema 2 JSON");
    let v3 = doriac::metadata_source_v3("main.doria", source).unwrap();
    let v3_json = serde_json::to_string(&v3).expect("schema 3 JSON");
    assert!(!v1.contains("callables"));
    assert!(!v1.contains("testSuites"));
    assert!(v2.contains("callables"));
    assert!(!v2.contains("testSuites"));
    assert!(v3_json.find("\"callables\"").unwrap() < v3_json.find("\"testSuites\"").unwrap());
    assert!(v3_json.find("\"testSuites\"").unwrap() < v3_json.find("\"tests\"").unwrap());
    assert!(v3.test_suites.is_empty());
    assert!(v3.tests.is_empty());

    let mut value = serde_json::to_value(&v3).expect("schema 3 value");
    value
        .as_object_mut()
        .expect("schema 3 object")
        .insert("unexpected".to_string(), serde_json::Value::Bool(true));
    assert!(
        serde_json::from_value::<doriac::attributes::AttributeMetadataDocumentV3>(value).is_err()
    );
}

#[test]
fn test_identity_and_graph_invalidation_are_source_aware() {
    let first_source = r#"
use Doria\Std\Test\it;
it("stable", function (): void { echo "a"; });
"#;
    let second_source = first_source.replace("\"a\"", "\"b\"");
    let renamed_source = first_source.replace("stable", "renamed");
    let first_graph = graph(first_source, SourceScope::Development, None);
    let second_graph = graph(&second_source, SourceScope::Development, None);
    let renamed_graph = graph(&renamed_source, SourceScope::Development, None);
    let first = doriac::metadata_compilation_graph_v3(&first_graph).unwrap();
    let second = doriac::metadata_compilation_graph_v3(&second_graph).unwrap();
    let renamed = doriac::metadata_compilation_graph_v3(&renamed_graph).unwrap();
    assert_ne!(first.graph_fingerprint, second.graph_fingerprint);
    assert_eq!(first.tests[0].identity, second.tests[0].identity);
    assert_ne!(first.tests[0].identity, renamed.tests[0].identity);
}

#[test]
fn duplicate_full_names_and_generated_name_collisions_are_rejected() {
    let duplicates = r#"
use Doria\Std\Test\{describe, it};
describe("suite", function (): void { it("case", function (): void {}); });
describe("suite", function (): void { it("case", function (): void {}); });
"#;
    let diagnostics =
        analyze_compilation_graph_for_ide(&graph(duplicates, SourceScope::Development, None))
            .diagnostics;
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0709")
            .count(),
        1,
        "{diagnostics:#?}"
    );

    let source = "use Doria\\Std\\Test\\it; it(\"case\", function (): void {});";
    let metadata =
        doriac::metadata_compilation_graph_v3(&graph(source, SourceScope::Development, None))
            .unwrap();
    let generated = &metadata.tests[0]
        .callable
        .as_ref()
        .expect("generated callable")
        .canonical_name;
    let collision = format!("{source}\nfunction {generated}(): void {{}}");
    let diagnostics =
        analyze_compilation_graph_for_ide(&graph(&collision, SourceScope::Development, None))
            .diagnostics;
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0711"));
}

#[test]
fn assertion_error_is_an_exact_development_only_compiler_known_type() {
    let source = r#"
use Doria\Std\Test\AssertionError;
function consume(AssertionError $error): void {}
"#;
    let development =
        analyze_compilation_graph_for_ide(&graph(source, SourceScope::Development, None));
    assert!(
        development.diagnostics.is_empty(),
        "{:#?}",
        development.diagnostics
    );

    let main = analyze_compilation_graph_for_ide(&graph(source, SourceScope::Main, None));
    assert_eq!(
        main.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0712")
            .count(),
        1,
        "{:#?}",
        main.diagnostics
    );
}

#[test]
fn generated_dispatcher_executes_behavioral_callable_on_every_enabled_backend() {
    let source = r#"
use Doria\Std\Test\{expect, fail, it};

internal class Token {}

it("dispatches", function (): void {
    expect(42)->toEqual(42);
    let $token = new Token();
    if ($token != $token) { fail("ordinary class identity drifted"); }
    expect($token)->toEqual($token);
    echo "behavioral\n";
});
"#;
    let metadata =
        doriac::metadata_compilation_graph_v3(&graph(source, SourceScope::Development, None))
            .expect("behavioral metadata");
    let callable = &metadata.tests[0]
        .callable
        .as_ref()
        .expect("generated callable")
        .canonical_name;
    let dispatcher = format!("function main(): void {{ {callable}(); }}");

    let debug = doriac::compile_compilation_graph(&dispatcher_graph(
        source,
        &dispatcher,
        CompilerTarget::Debug,
        None,
    ))
    .expect("debug dispatcher");
    let doriac::backend::BackendOutput::Text { contents, .. } = debug else {
        panic!("debug backend must emit text");
    };
    assert!(contents.contains("stdout: behavioral\n"), "{contents}");

    if Command::new(if cfg!(windows) { "cl.exe" } else { "cc" })
        .arg("--version")
        .output()
        .is_ok()
    {
        let native = doriac::compile_compilation_graph(&dispatcher_graph(
            source,
            &dispatcher,
            CompilerTarget::Native,
            Some(BuildNativeProfile::Fast),
        ))
        .expect("Cranelift dispatcher");
        let output = run_emitted(native);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, b"behavioral\n");
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        #[cfg(feature = "llvm-backend")]
        {
            let llvm = doriac::compile_compilation_graph(&dispatcher_graph(
                source,
                &dispatcher,
                CompilerTarget::Native,
                Some(BuildNativeProfile::Release),
            ))
            .expect("LLVM dispatcher");
            let output = run_emitted(llvm);
            assert_eq!(output.status.code(), Some(0));
            assert_eq!(output.stdout, b"behavioral\n");
            assert!(output.stderr.is_empty());
        }
    }

    if Command::new("php").arg("--version").output().is_ok() {
        let php = doriac::compile_compilation_graph(&dispatcher_graph(
            source,
            &dispatcher,
            CompilerTarget::Php,
            None,
        ))
        .expect("PHP dispatcher");
        let doriac::backend::BackendOutput::Text { contents, .. } = php else {
            panic!("PHP backend must emit text");
        };
        for forbidden in [
            "PHPUnit",
            "Pest",
            "Reflection",
            "testRegistry",
            "suiteRegistry",
        ] {
            assert!(
                !contents.contains(forbidden),
                "generated PHP contains {forbidden}"
            );
        }
        let path = temporary_path("php");
        fs::write(&path, contents).expect("write PHP");
        let output = Command::new("php").arg(&path).output().expect("run PHP");
        let _ = fs::remove_file(path);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, b"behavioral\n");
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn slice_two_matchers_evaluate_once_in_order_and_execute_on_every_backend() {
    let source = r#"
use Doria\Std\Test\{expect, it};

internal class Token {}
internal enum State { case Ready; case Waiting; }
internal enum Payload { case Number(int $value); case Empty; }

function actual(): int { echo "a"; return 7; }
function expected(): int { echo "e"; return 7; }
function assertPresent(?int $value): void { expect($value)->not->toBeNull(); }

it("core matcher matrix", function (): void {
    expect(actual())->toEqual(expected());
    expect(7)->not->toEqual(8);
    expect(true)->toBeTrue();
    expect(false)->toBeFalse();
    expect(true)->not->toBeFalse();
    expect(8)->toBeGreaterThan(7);
    expect(8)->toBeGreaterThanOrEqual(8);
    expect(7)->toBeLessThan(8);
    expect(8)->toBeLessThanOrEqual(8);
    expect(3.5)->toBeGreaterThan(3.0);
    expect("alpha")->toContain("ph");
    expect("alpha")->toStartWith("al");
    expect("alpha")->toEndWith("ha");
    expect("")->toBeEmpty();
    expect("alpha")->not->toBeEmpty();

    ?int $missing = null;
    mixed $mixedNull = null;
    mixed $mixedValue = "value";
    expect(null)->toBeNull();
    expect($missing)->toBeNull();
    assertPresent(1);
    expect($mixedNull)->toBeNull();
    expect($mixedValue)->not->toBeNull();

    expect(State::Ready)->toEqual(State::Ready);
    expect(Payload::Number(42))->toEqual(Payload::Number(42));
    let $token = new Token();
    let $other = new Token();
    expect($token)->toEqual($token);
    expect($token)->not->toEqual($other);
    if ($token != $token) { echo "identity drift"; }
    echo " ok\n";
});
"#;
    assert_behavioral_output_on_every_enabled_backend(source, b"ae ok\n");
}

#[test]
fn assertion_errors_are_catchable_and_helpers_need_no_throws_clause() {
    let source = r#"
use Doria\Std\Test\{AssertionError, expect, fail, it};

function helper(): void { fail("caught"); }
function forwarded(): void { helper(); }

it("catch assertion", function (): void {
    try { forwarded(); } catch (AssertionError $error) {
        expect($error->message)->toEqual("caught");
        echo "exact ";
    }
    try { expect(false)->toBeTrue(); } catch (Error) { echo "base\n"; }
});
"#;
    assert_behavioral_output_on_every_enabled_backend(source, b"exact base\n");

    let analysis =
        analyze_compilation_graph_for_ide(&graph(source, SourceScope::Development, None));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let helper = analysis
        .semantic_info
        .callable_effective_checked_effects
        .iter()
        .find(|(span, _)| source[span.start..span.end].starts_with("function helper"))
        .map(|(_, effects)| effects)
        .expect("helper effect profile");
    assert_eq!(helper.len(), 1);
    assert_eq!(
        doriac::attributes::metadata_type_name(&helper[0]),
        "Doria\\Std\\Test\\AssertionError"
    );
}

#[test]
fn plain_development_helpers_can_catch_assertion_errors() {
    let source = r#"
use Doria\Std\Test\{AssertionError, fail};

function helper(): void
{
    try { fail("caught"); } catch (AssertionError $error) { echo $error->message; }
}
"#;
    let analysis =
        analyze_compilation_graph_for_ide(&graph(source, SourceScope::Development, None));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn malformed_expectation_shapes_report_one_root_diagnostic() {
    let cases = [
        ("function f(): void { expect(); }", "E0713"),
        ("function f(): void { expect(1, 2); }", "E0713"),
        ("function f(): void { expect(value: 1); }", "E0713"),
        ("function f(): void { expect(1); }", "E0714"),
        ("function f(): void { expect(1)->not; }", "E0714"),
        ("function f(): void { expect(1)->not(); }", "E0716"),
        (
            "function f(): void { expect(1)->not->not->toEqual(2); }",
            "E0716",
        ),
        (
            "function f(): void { expect(1)->unknownMatcher(); }",
            "E0717",
        ),
        (
            "function f(): void { expect(1)->toEqual(value: 1); }",
            "E0717",
        ),
        ("function f(): void { expect(1)->toEqual(); }", "E0717"),
        (
            "function f(): void { let $value = expect(1)->toEqual(1); }",
            "E0715",
        ),
        ("function f(): void { expect([1])->toContain(1); }", "E0718"),
        ("function f(): void { expect(1)->toBeNull(); }", "E0720"),
        ("function f(): void { expect(1)->toBeTrue(); }", "E0720"),
    ];
    for (body, expected) in cases {
        let source = format!("use Doria\\Std\\Test\\expect;\n{body}");
        let diagnostics =
            analyze_compilation_graph_for_ide(&graph(&source, SourceScope::Development, None))
                .diagnostics;
        let roots = diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.is_consequence)
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 1, "{body}: {diagnostics:#?}");
        assert_eq!(roots[0].code, expected, "{body}: {diagnostics:#?}");
    }
}

#[test]
fn shared_validator_rejects_malformed_assertion_plans_and_descriptors() {
    let source = r#"
use Doria\Std\Test\{expect, it};
it("valid", function (): void { expect(42)->toEqual(42); });
"#;
    let valid = assertion_mir(source);
    doriac::mir_validation::validate_program(&valid).expect("valid assertion MIR");

    let mut shared_edge = valid.clone();
    let plan = assertion_plan_mut(&mut shared_edge);
    plan.failure = plan.success;
    assert_malformed_assertion_mir(&shared_edge, "success and failure blocks must be distinct");

    let mut missing_actual = valid.clone();
    assertion_plan_mut(&mut missing_actual).actual = None;
    assert_malformed_assertion_mir(
        &missing_actual,
        "assertion operand and source type metadata disagree",
    );

    let mut missing_source = valid.clone();
    assertion_plan_mut(&mut missing_source).source_span.source = doriac::source::SourceId(999);
    assert_malformed_assertion_mir(&missing_source, "references a missing source");

    let mut invalid_span = valid.clone();
    assertion_plan_mut(&mut invalid_span).source_span.end = usize::MAX;
    assert_malformed_assertion_mir(&invalid_span, "invalid source span");

    let mut main_source = valid.clone();
    let source_id = assertion_plan_mut(&mut main_source).source_span.source;
    main_source
        .sources
        .iter_mut()
        .find(|source| source.id == source_id)
        .expect("assertion source")
        .scope = SourceScope::Main;
    assert_malformed_assertion_mir(&main_source, "non-development source");

    let mut missing_assertion_descriptor = valid.clone();
    let descriptor = assertion_plan_mut(&mut missing_assertion_descriptor).descriptor;
    missing_assertion_descriptor.error_descriptors[descriptor.0].assertion = None;
    assert_malformed_assertion_mir(
        &missing_assertion_descriptor,
        "does not use the compiler-known AssertionError descriptor",
    );

    let mut reordered_facts = valid.clone();
    let descriptor = assertion_plan_mut(&mut reordered_facts).descriptor;
    reordered_facts.error_descriptors[descriptor.0]
        .assertion
        .as_mut()
        .expect("assertion descriptor")
        .fact_properties
        .swap(0, 1);
    assert_malformed_assertion_mir(&reordered_facts, "invalid fact projection at slot 0");

    let fail_source = r#"
use Doria\Std\Test\{fail, it};
it("valid", function (): void { fail("stop"); });
"#;
    let mut negated_fail = assertion_mir(fail_source);
    assertion_plan_mut(&mut negated_fail).negated = true;
    assert_malformed_assertion_mir(
        &negated_fail,
        "fail assertion plan has incompatible operands",
    );
}

#[test]
fn escaping_assertion_uses_the_strict_v4_outcome_on_every_enabled_backend() {
    let source = r#"
use Doria\Std\Test\it;
use Doria\Std\Test\expect;
it("reports", function (): void {
    expect("alpha")->not->toContain("ph");
});
"#;
    let metadata =
        doriac::metadata_compilation_graph_v3(&graph(source, SourceScope::Development, None))
            .expect("behavioral metadata");
    let callable = &metadata.tests[0]
        .callable
        .as_ref()
        .expect("generated callable")
        .canonical_name;
    let dispatcher = format!("function main(): void {{ {callable}(); }}");

    let debug = doriac::compile_compilation_graph(&dispatcher_graph(
        source,
        &dispatcher,
        CompilerTarget::Debug,
        None,
    ))
    .expect("debug dispatcher");
    let doriac::backend::BackendOutput::Text { contents, .. } = debug else {
        panic!("debug backend must emit text");
    };
    assert!(
        contents.contains("Error[R1001]: Assertion Failed"),
        "{contents}"
    );
    assert!(contents.contains("Expected\n  \"ph\""), "{contents}");
    assert!(contents.contains("Actual\n  \"alpha\""), "{contents}");

    let assert_v4 = |payload: &[u8]| {
        assert!(payload.starts_with(b"DORIAO4\0\x04\0"), "{payload:?}");
        for value in [
            b"Doria\\Std\\Test\\AssertionError".as_slice(),
            b"StringContains".as_slice(),
            b"string".as_slice(),
            b"\"alpha\"".as_slice(),
            b"\"ph\"".as_slice(),
        ] {
            assert!(
                payload.windows(value.len()).any(|window| window == value),
                "missing {:?} in {payload:?}",
                String::from_utf8_lossy(value)
            );
        }
    };

    if Command::new(if cfg!(windows) { "cl.exe" } else { "cc" })
        .arg("--version")
        .output()
        .is_ok()
    {
        let native = doriac::compile_compilation_graph(&dispatcher_graph(
            source,
            &dispatcher,
            CompilerTarget::Native,
            Some(BuildNativeProfile::Fast),
        ))
        .expect("Cranelift dispatcher");
        let (output, payload) = run_emitted_with_assertion_outcome(native);
        assert_eq!(output.status.code(), Some(70));
        assert!(output.stderr.is_empty());
        assert_v4(&payload);

        #[cfg(feature = "llvm-backend")]
        {
            let llvm = doriac::compile_compilation_graph(&dispatcher_graph(
                source,
                &dispatcher,
                CompilerTarget::Native,
                Some(BuildNativeProfile::Release),
            ))
            .expect("LLVM dispatcher");
            let (output, payload) = run_emitted_with_assertion_outcome(llvm);
            assert_eq!(output.status.code(), Some(70));
            assert!(output.stderr.is_empty());
            assert_v4(&payload);
        }
    }

    if Command::new("php").arg("--version").output().is_ok() {
        let php = doriac::compile_compilation_graph(&dispatcher_graph(
            source,
            &dispatcher,
            CompilerTarget::Php,
            None,
        ))
        .expect("PHP dispatcher");
        let doriac::backend::BackendOutput::Text { contents, .. } = php else {
            panic!("PHP backend must emit text");
        };
        let path = temporary_path("php");
        let outcome = temporary_path("outcome");
        fs::write(&path, contents).expect("write PHP");
        let output = Command::new("php")
            .env("DORIA_RUNTIME_OUTCOME_V3", &outcome)
            .env("DORIA_RUNTIME_OUTCOME_V4", &outcome)
            .arg(&path)
            .output()
            .expect("run PHP");
        let payload = fs::read(&outcome).unwrap_or_else(|error| {
            panic!(
                "PHP assertion outcome: {error}; status={:?}; stderr={}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(outcome);
        assert_eq!(output.status.code(), Some(70));
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_v4(&payload);
    }
}
