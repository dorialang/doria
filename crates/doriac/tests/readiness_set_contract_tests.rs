#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::mir;
use doriac::numeric::{IntegerType, IntegerValue};
use doriac::semantics::SemanticAnalysis;
use doriac::types::ResolvedType;

const IMPORTS: &str = r#"
use Doria\Std\Io\{ReadinessSet, ReadinessStream, ReadinessSource,
    RegistrationKey, ReadableFile, WritableFile, File, IoError};
"#;

fn analyze(source: &str) -> SemanticAnalysis {
    doriac::analyze_source_for_ide("readiness-set.doria", format!("{IMPORTS}\n{source}"))
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
        .1
}

fn assert_valid(analysis: &SemanticAnalysis) {
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn approved_constructor_is_known_and_declares_checked_setup_failure() {
    let analysis =
        analyze("function main(): void throws IoError { let $set = new ReadinessSet(); }");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis.info.expression_types.values().any(|ty| {
        matches!(ty, ResolvedType::Class(class) if class.name == doriac::compiler_known_stream::READINESS_SET)
    }));
    let implicit = analyze("function main(): void { let $set = new ReadinessSet(); }");
    assert!(
        implicit.diagnostics.is_empty(),
        "{:#?}",
        implicit.diagnostics
    );
    assert!(
        implicit.info.callable_effective_checked_effects.iter().any(|(span, effects)| {
            span.source != doriac::compiler_known_stream::SOURCE_ID
                && effects.iter().any(|effect| {
                    matches!(effect, ResolvedType::Class(class) if class.name == doriac::compiler_known_io::IO_ERROR)
                })
        }),
        "{:#?}",
        implicit.info.callable_effective_checked_effects
    );
}

#[test]
fn unused_set_declarations_do_not_prevent_other_stream_execution() {
    let source = format!(
        "{IMPORTS}\nfunction inspect(ReadinessSet $set): void {{}} function main(): void {{}}"
    );
    doriac::check_source("unused-set.doria", &source).unwrap();
    let program = doriac::lower_source_to_mir("unused-set.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    doriac::compile_source("unused-set.doria", &source, BackendTarget::Php).unwrap();
}

#[test]
fn constructor_does_not_accept_arguments_or_direct_lifecycle_calls() {
    for body in [
        "let $set = new ReadinessSet(1);",
        "let $set = new ReadinessSet(capacity: 1);",
        "let $set = new ReadinessSet(); $set->__construct();",
    ] {
        let analysis = analyze(&format!("function main(): void {{ {body} }}"));
        assert!(!analysis.diagnostics.is_empty(), "{body}");
    }
}

#[test]
fn registration_and_removal_preserve_concrete_and_erased_types() {
    let analysis = analyze(
        r#"
function concrete(writable ReadinessSet $set, take ReadableFile $stream): ReadableFile throws IoError {
    let $key = $set->register($stream);
    let $copy = $key;
    let $returned = $set->unregister($copy);
    return $returned;
}
function erased(writable ReadinessSet $set, take ReadinessStream $stream): ReadinessStream throws IoError {
    let $key = $set->register($stream);
    return $set->unregister($key);
}
function main(): void {}
"#,
    );
    assert_valid(&analysis);
    for name in [
        doriac::compiler_known_stream::READABLE_FILE,
        doriac::compiler_known_stream::READINESS_STREAM,
    ] {
        assert!(analysis.info.expression_types.values().any(|ty| {
            matches!(ty, ResolvedType::Class(class)
                if class.name == doriac::compiler_known_stream::REGISTRATION_KEY
                    && class.arguments.len() == 1
                    && match &class.arguments[0] {
                        ResolvedType::Class(value) | ResolvedType::Interface(value) => value.name == name,
                        _ => false,
                    })
        }), "missing exact RegistrationKey<{name}>");
    }
}

#[test]
fn generic_forwarding_preserves_the_same_readiness_constraint_and_result() {
    let analysis = analyze(
        r#"
function relay<T implements ReadinessStream>(writable ReadinessSet $set, take T $stream): T throws IoError {
    let $key = $set->register($stream);
    return $set->unregister($key);
}
function main(): void {}
"#,
    );
    assert_valid(&analysis);
}

const ADAPTER: &str = r#"
class ReadyOnly implements ReadinessStream {
    internal ReadableFile $source;
    function __construct(parameter take ReadableFile $source) { $this->source = $source; }
    ReadinessSource $readinessSource { borrowed get => $this->source->readinessSource; }
}
"#;

#[test]
fn registered_stream_access_preserves_exact_types_and_receiver_loans() {
    let source = format!(
        "{IMPORTS}\n{ADAPTER}\n{}",
        r#"
function inspect(ReadinessSet $set, RegistrationKey<ReadyOnly> $key): ReadinessSource throws IoError {
    return relay($set, $key)->readinessSource;
}
function inspectWritable(writable ReadinessSet $set, RegistrationKey<ReadyOnly> $key): ReadinessSource throws IoError {
    return writableRelay($set, $key)->readinessSource;
}
function relay<T>(ReadinessSet $set, RegistrationKey<T> $key): T throws IoError {
    return $set->getStream($key);
}
function writableRelay<T>(writable ReadinessSet $set, RegistrationKey<T> $key): T throws IoError {
    return $set->getWritableStream($key);
}
function main(): void {}
"#
    );
    doriac::check_source("readiness-access.doria", &source).unwrap();
    let program = doriac::lower_source_to_mir("readiness-access.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for (name, writable) in [
        ("inspect", false),
        ("relay", false),
        ("writableRelay", true),
    ] {
        for function in program
            .functions
            .iter()
            .filter(|function| function.name.starts_with(name))
        {
            let borrow = function
                .return_borrow
                .expect("lookup returns a receiver loan");
            assert_eq!(borrow.source, mir::BorrowSource::Parameter(0));
            assert_eq!(borrow.writable, writable);
        }
    }
}

#[test]
fn readonly_access_does_not_allow_consumption_or_writable_access() {
    for body in [
        "$set->getWritableStream($key);",
        "$set->getStream($key)->close();",
    ] {
        let analysis = analyze(&format!(
            "function inspect(ReadinessSet $set, RegistrationKey<ReadableFile> $key): void throws IoError {{ {body} }} function main(): void {{}}"
        ));
        assert!(!analysis.diagnostics.is_empty(), "{body}");
    }
}

#[test]
fn a_live_registered_stream_loan_prevents_removal_and_set_escape() {
    for body in [
        "let $stream = $set->getStream($key); let $removed = $set->unregister($key); let $source = $stream->readinessSource;",
        "let $stream = $set->getWritableStream($key); let $removed = $set->unregister($key); let $source = $stream->readinessSource;",
    ] {
        let analysis = analyze(&format!(
            "function inspect(writable ReadinessSet $set, RegistrationKey<ReadableFile> $key): void throws IoError {{ {body} }} function main(): void {{}}"
        ));
        assert!(!analysis.diagnostics.is_empty(), "{body}");
    }
    let analysis = analyze(
        r#"
function escape(): ReadableFile throws IoError {
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(File::openRead("input"));
    return $set->getStream($key);
}
function main(): void {}
"#,
    );
    assert!(
        !analysis.diagnostics.is_empty(),
        "loan must not outlive its set"
    );
}

#[test]
fn mir_independently_checks_registered_stream_access_and_loan_ownership() {
    let source = format!(
        "{IMPORTS}\n{}",
        r#"
function view(ReadinessSet $set, RegistrationKey<ReadableFile> $key): ReadableFile throws IoError {
    return $set->getStream($key);
}

function writableView(writable ReadinessSet $set, RegistrationKey<ReadableFile> $key): ReadableFile throws IoError {
    return $set->getWritableStream($key);
}
function main(): void {}
"#
    );
    let valid = doriac::lower_source_to_mir("readiness-loan-mir.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&valid).unwrap();
    for fault in 0..6 {
        let mut invalid = valid.clone();
        let function = invalid
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedIo {
                            operation: mir::CheckedIoOperation::BorrowReadinessStream {
                                writable: true,
                                ..
                            },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let block = function
            .blocks
            .iter_mut()
            .find(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::CheckedIo {
                        operation: mir::CheckedIoOperation::BorrowReadinessStream { .. },
                        ..
                    }
                )
            })
            .unwrap();
        let mir::Terminator::CheckedIo {
            operation:
                mir::CheckedIoOperation::BorrowReadinessStream {
                    set,
                    key,
                    stream_type,
                    ..
                },
            result: Some(result),
            ..
        } = &mut block.terminator
        else {
            unreachable!()
        };
        match fault {
            0 => function.locals[result.0].owned = true,
            1 => function.locals[result.0].writable = false,
            2 => function.locals[set.0].writable = false,
            3 => *stream_type = function.locals[set.0].ty,
            4 => *key = *set,
            5 => function.return_borrow = None,
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "fault {fault}"
        );
    }
}

#[test]
fn unissued_key_types_do_not_add_a_readiness_bound_to_access() {
    for ty in ["int", "string", "NotReady"] {
        let source = format!(
            r#"{IMPORTS}
class NotReady {{}}
function inspect(ReadinessSet $set, RegistrationKey<{ty}> $key): {ty} throws IoError {{
    return $set->getStream($key);
}}
function inspectWritable(writable ReadinessSet $set, RegistrationKey<{ty}> $key): {ty} throws IoError {{
    return $set->getWritableStream($key);
}}
function main(): void {{}}
"#
        );
        let program = doriac::lower_source_to_mir("unissued-readiness-key.doria", &source).unwrap();
        doriac::mir_validation::validate_program(&program).unwrap();
        native_execution::assert_native_execution(&program, NativeProfile::Fast, "");
        #[cfg(feature = "llvm-backend")]
        native_execution::assert_native_execution(&program, NativeProfile::Release, "");
        doriac::compile_source("unissued-readiness-key.doria", &source, BackendTarget::Php)
            .unwrap();
    }
}

const BORROWING_ADAPTER: &str = r#"
class BorrowingReady implements ReadinessStream, Iterator<int> {
    internal ReadableFile $file;
    function __construct(parameter take ReadableFile $file, borrow List<int> $source) {
        $this->file = $file;
    }
    ReadinessSource $readinessSource { borrowed get => $this->file->readinessSource; }
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
"#;

#[test]
fn registered_complete_owners_retain_their_external_source_loans() {
    let body = r#"
function main(): void {
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $source));
    $source->add(2);
    let $returned = $set->unregister($key);
    echo $returned->getCurrent();
}
"#;
    assert_retained_source_error(body, "E0763");
}

#[test]
fn registered_complete_owners_cannot_escape_their_external_source() {
    let body = r#"
function escape(): ReadinessSet throws IoError {
    let $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    $set->register(new BorrowingReady(File::openRead("input"), $source));
    return $set;
}
function main(): void {}
"#;
    assert_retained_source_error(body, "E0762");
}

#[test]
fn discarded_registration_keys_do_not_release_registered_source_loans() {
    assert_retained_source_error(
        r#"
function main(): void {
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    $set->register(new BorrowingReady(File::openRead("input"), $source));
    $source->add(2);
}
"#,
        "E0763",
    );
}

#[test]
fn removed_complete_owners_keep_their_source_loans() {
    assert_retained_source_error(
        r#"
function main(): void {
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $source));
    let $copy = $key;
    let $returned = $set->unregister($copy);
    $source->add(2);
    echo $returned->getCurrent();
}
"#,
        "E0763",
    );
}

#[test]
fn forwarding_registration_keeps_the_same_owner_and_source_lifetime() {
    assert_retained_source_error(
        r#"
function keep<T implements ReadinessStream>(writable ReadinessSet $set, take T $stream): RegistrationKey<T> throws IoError {
    return $set->register($stream);
}
function main(): void {
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    let $key = keep($set, new BorrowingReady(File::openRead("input"), $source));
    $source->add(2);
    let $returned = $set->unregister($key);
    echo $returned->getCurrent();
}
"#,
        "E0763",
    );
}

#[test]
fn helper_returned_keys_release_only_the_selected_member() {
    let analysis = analyze(&format!(
        r#"{BORROWING_ADAPTER}
function keep<T implements ReadinessStream>(writable ReadinessSet $set, take T $stream): RegistrationKey<T> throws IoError {{
    return $set->register($stream);
}}
function release(take BorrowingReady $stream): void {{}}
function main(): void {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = keep($set, new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = keep($set, new BorrowingReady(File::openRead("input"), $second));
    let $copiedKey = $firstKey;
    release($set->unregister($copiedKey));
    $first->add(3);
    let $remaining = $set->unregister($secondKey);
    echo $remaining->getCurrent();
}}
"#
    ));
    assert_valid(&analysis);
}

#[test]
fn moved_sets_preserve_member_identity_without_keys_owning_sources() {
    let analysis = analyze(&format!(
        r#"{BORROWING_ADAPTER}
function release(take BorrowingReady $stream): void {{}}
function main(): void {{
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $source));
    let $copy = $key;
    let writable $renamed = $set;
    release($renamed->unregister($copy));
    $source->add(2);
}}
"#
    ));
    assert_valid(&analysis);
}

#[test]
fn registration_into_an_owned_field_retains_the_complete_adapters_sources() {
    assert_retained_source_error(
        r#"
class Owner {
    writable ReadinessSet $set;
    function __construct() { $this->set = new ReadinessSet(); }
}

function main(): void {
    writable List<int> $source = [1];
    let writable $owner = new Owner();
    let $key = $owner->set->register(new BorrowingReady(File::openRead("input"), $source));
    $source->add(2);
    let $returned = $owner->set->unregister($key);
    echo $returned->getCurrent();
}
"#,
        "E0763",
    );
}

#[test]
fn registration_cannot_store_local_source_loans_in_a_callers_set() {
    assert_retained_source_error(
        r#"
function store(writable ReadinessSet $set): void throws IoError {
    let $source = [1];
    $set->register(new BorrowingReady(File::openRead("input"), $source));
}
function main(): void {}
"#,
        "E0762",
    );
}

#[test]
fn returned_sets_preserve_borrowed_parameter_sources() {
    assert_retained_source_error(
        r#"
function make(List<int> $source): ReadinessSet throws IoError {
    writable ReadinessSet $set = new ReadinessSet();
    $set->register(new BorrowingReady(File::openRead("input"), $source));
    return $set;
}
function main(): void {
    writable List<int> $source = [1];
    let $set = make($source);
    $source->add(2);
}
"#,
        "E0763",
    );
}

#[test]
fn removing_and_destroying_one_member_releases_only_its_sources() {
    let analysis = analyze(&format!(
        r#"{BORROWING_ADAPTER}
function release(take BorrowingReady $stream): void {{}}
function main(): void {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    let $copy = $firstKey;
    release($set->unregister($copy));
    $first->add(3);
    echo $set->unregister($secondKey)->getCurrent();
}}
"#
    ));
    assert_valid(&analysis);
}

#[test]
fn removing_one_member_does_not_release_another_members_sources() {
    assert_retained_source_error(
        r#"
function release(take BorrowingReady $stream): void {}
function main(): void {
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    release($set->unregister($firstKey));
    $second->add(3);
    echo $set->unregister($secondKey)->getCurrent();
}
"#,
        "E0763",
    );
}

#[test]
fn an_error_after_successful_registration_keeps_the_registered_source_loans() {
    assert_retained_source_error(
        r#"
use Doria\Std\Io\{IoOperation, IoTarget, IoErrorReason};
function attach<T implements ReadinessStream>(writable ReadinessSet $set, take T $stream): void throws IoError {
    $set->register($stream);
    throw new IoError("after registration", IoOperation::Wait, IoTarget::ReadinessSet, IoErrorReason::Other);
}
function main(): void {
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    try {
        attach($set, new BorrowingReady(File::openRead("input"), $source));
    } catch (IoError) {
        $source->add(2);
    }
}
"#,
        "E0763",
    );
}

#[test]
fn failed_registration_does_not_retain_the_consumed_adapters_sources() {
    let analysis = analyze(&format!(
        r#"{BORROWING_ADAPTER}
function main(): void {{
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    try {{
        $set->register(new BorrowingReady(File::openRead("input"), $source));
    }} catch (IoError) {{
        $source->add(2);
    }}
}}
"#
    ));
    assert_valid(&analysis);
}

fn assert_retained_source_error(body: &str, expected: &str) {
    let analysis = analyze(&format!("{BORROWING_ADAPTER}\n{body}"));
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == expected),
        "{body}\n{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn registration_does_not_require_transfer_or_pending_capabilities() {
    let analysis = analyze(&format!(
        r#"{ADAPTER}
function retain(writable ReadinessSet $set, take ReadyOnly $stream): ReadyOnly throws IoError {{
    let $key = $set->register($stream);
    return $set->unregister($key);
}}
function main(): void {{}}
"#
    ));
    assert_valid(&analysis);
}

#[test]
fn registration_requires_nominal_readiness_not_a_similarly_named_getter() {
    let source = format!(
        r#"{}
function retain(writable ReadinessSet $set, take ReadyOnly $stream): void throws IoError {{
    let $key = $set->register($stream);
}}
function main(): void {{}}
"#,
        ADAPTER.replace(" implements ReadinessStream", "")
    );
    let analysis = analyze(&source);
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("ReadinessStream") }),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn readonly_set_rejects_registration_and_removal() {
    let analysis = analyze(
        r#"
function retain(ReadinessSet $set, take ReadableFile $stream): ReadableFile throws IoError {
    let $key = $set->register($stream);
    return $set->unregister($key);
}
function main(): void {}
"#,
    );
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0203")
            .count(),
        2,
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn removal_cannot_claim_a_different_static_stream_type() {
    let analysis = analyze(
        r#"
function recover(writable ReadinessSet $set, RegistrationKey<ReadableFile> $key): WritableFile throws IoError {
    return $set->unregister($key);
}
function main(): void {}
"#,
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("WritableFile") }),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn source_authored_names_do_not_inherit_the_compiler_execution_boundary() {
    let source = r#"
class ReadinessSet {
    function __construct() {}
    writable function register(int $stream): int { return $stream; }
    writable function unregister(int $key): int { return $key; }
}
function main(): void {
    let writable $set = new ReadinessSet();
    let $key = $set->register(7);
    echo $set->unregister($key);
}
"#;
    doriac::check_source("authored-set.doria", source).unwrap();
    let program = doriac::lower_source_to_mir("authored-set.doria", source).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(result.stdout, b"7");
    assert!(matches!(
        doriac::compile_source("authored-set.doria", source, BackendTarget::Php).unwrap(),
        doriac::backend::BackendOutput::Text { .. }
    ));
}

#[test]
fn php_membership_uses_checked_native_transport_for_concrete_and_generic_calls() {
    let source = format!(
        "{IMPORTS}\nfunction main(): void {{ let writable $set = new ReadinessSet(); let $key = $set->register(File::openRead(\"input\")); let $returned = $set->unregister($key); }}"
    );
    doriac::check_source("membership-set.doria", &source).unwrap();
    let program = doriac::lower_source_to_mir("membership-set.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for body in [
        "function main(): void { let writable $set = new ReadinessSet(); $set->register(File::openRead(\"input\")); }",
        "function remove(writable ReadinessSet $set, RegistrationKey<ReadableFile> $key): ReadableFile { return $set->unregister($key); } function main(): void {}",
        "function remove<T>(writable ReadinessSet $set, RegistrationKey<T> $key): T { return $set->unregister($key); } function main(): void { let writable $set = new ReadinessSet(); remove($set, makeKey()); } function makeKey(): RegistrationKey<ReadableFile> { panic(\"key\"); }",
    ] {
        let source = format!("{IMPORTS}\n{body}");
        doriac::check_source("membership-set.doria", &source).unwrap();
        let artifact = doriac::compile_source("membership-set.doria", &source, BackendTarget::Php)
            .unwrap();
        assert!(matches!(artifact, doriac::backend::BackendOutput::TextWithRuntime { .. }));
    }
}

#[test]
fn php_registration_getters_use_the_same_borrow_transport_as_property_access() {
    for stream in ["ReadableFile", "ReadyOnly", "ReadinessStream"] {
        let source = format!(
            "{IMPORTS}\n{ADAPTER}\nfunction register(writable ReadinessSet $set, take {stream} $stream): void throws IoError {{ $set->register($stream); }} function main(): void {{}}"
        );
        let artifact =
            doriac::compile_source("registration-getter.doria", &source, BackendTarget::Php)
                .unwrap();
        let doriac::backend::BackendOutput::TextWithRuntime { contents, .. } = artifact else {
            panic!("registration must include its native runtime");
        };
        let getter = contents
            .lines()
            .find(|line| line.contains("$source = $taken->"))
            .expect("registration emits its checked getter call");
        assert!(
            getter
                .contains("(__DoriaCell::borrowed($taken))->value->__doriaNativeReadinessSource()"),
            "{stream}: {getter}"
        );
    }
}

const CONSTRUCTION_SOURCE: &str =
    include_str!("../../../examples/native/main_readiness_set_construction.doria");

#[test]
fn construction_and_owned_cleanup_preserve_backend_parity() {
    let expected =
        include_str!("fixtures/native_io/main_readiness_set_construction/expected_stdout");
    let program =
        doriac::lower_source_to_mir("readiness-set-construction.doria", CONSTRUCTION_SOURCE)
            .unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);

    let artifact = doriac::compile_source(
        "readiness-set-construction.doria",
        CONSTRUCTION_SOURCE,
        BackendTarget::Php,
    )
    .unwrap();
    assert!(matches!(
        artifact,
        doriac::backend::BackendOutput::TextWithRuntime { .. }
    ));
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(&path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for readiness-set parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");

    let disabled = std::process::Command::new("php")
        .args(["-d", "ffi.enable=0"])
        .arg(path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(disabled.status.code(), Some(1), "{disabled:?}");
    let error = String::from_utf8_lossy(&disabled.stderr);
    assert!(error.contains("B3602"), "{error}");
    assert!(!error.contains("Fatal error"), "{error}");
}

#[test]
fn constructor_uses_exact_checked_runtime_storage_and_rejects_forged_mir() {
    let valid = doriac::lower_source_to_mir("set-mir.doria", CONSTRUCTION_SOURCE).unwrap();
    let class = valid
        .classes
        .iter()
        .find(|class| class.runtime_payload == Some(mir::RuntimeClassPayload::ReadinessSet))
        .unwrap();
    assert_eq!(class.name, doriac::compiler_known_stream::READINESS_SET);
    assert!(class.properties.is_empty());
    let class_id = class.id;

    for fault in 0..9 {
        let mut invalid = valid.clone();
        if (4..7).contains(&fault) || fault == 8 {
            let definition = &mut invalid.classes[class_id.0];
            match fault {
                4 => definition.runtime_payload = None,
                5 => definition.declaration_name = "AuthoredSet".into(),
                6 => definition.source_span.source = doriac::source::SourceId(0),
                8 => definition.arguments.push(mir::Type::String),
                _ => unreachable!(),
            }
        } else {
            let function = invalid
                .functions
                .iter_mut()
                .find(|function| {
                    function.blocks.iter().any(|block| {
                        matches!(
                            block.terminator,
                            mir::Terminator::CheckedIo {
                                operation: mir::CheckedIoOperation::NewReadinessSet { .. },
                                ..
                            }
                        )
                    })
                })
                .unwrap();
            let block = function
                .blocks
                .iter_mut()
                .find(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedIo {
                            operation: mir::CheckedIoOperation::NewReadinessSet { .. },
                            ..
                        }
                    )
                })
                .unwrap();
            let mir::Terminator::CheckedIo {
                result,
                error,
                success,
                failure,
                ..
            } = &mut block.terminator
            else {
                unreachable!()
            };
            match fault {
                0 => {
                    function.locals[result.unwrap().0].ty = mir::Type::Scalar(
                        mir::ScalarType::Integer(doriac::numeric::IntegerType::Int64),
                    )
                }
                1 => *result = None,
                2 => *error = result.unwrap(),
                3 => *failure = *success,
                7 => {
                    let target = result.unwrap();
                    let next = *success;
                    block.terminator = mir::Terminator::Jump(next);
                    block.statements.push(mir::Statement::AssignLocal {
                        target,
                        value: mir::Rvalue::Class(mir::ClassExpression::New {
                            class: class_id,
                            concrete_class: class_id,
                            properties: Vec::new(),
                            constructor: None,
                            args: Vec::new(),
                            span: function.source_span,
                        }),
                    });
                }
                _ => unreachable!(),
            }
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "fault {fault}"
        );
    }
}

fn stored_owner_mir() -> mir::Program {
    let source = format!(
        r#"{IMPORTS}
{BORROWING_ADAPTER}
function retain(writable ReadinessSet $set, take BorrowingReady $stream,
    RegistrationKey<BorrowingReady> $seed): RegistrationKey<BorrowingReady> throws IoError {{
    return $seed;
}}
function fresh(RegistrationKey<BorrowingReady> $seed): RegistrationKey<BorrowingReady> throws IoError {{
    return $seed;
}}
function main(): void {{}}
"#
    );
    let mut program = doriac::lower_source_to_mir("stored-owner-mir.doria", source).unwrap();
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "retain")
        .unwrap();
    let result = mir::LocalId(function.locals.len());
    let key_type = function.locals[function.params[2].0].ty;
    let mir::Type::Scalar(mir::ScalarType::Inline(key)) = key_type else {
        panic!("exact key")
    };
    function.locals.push(mir::Local {
        id: result,
        name: "result".into(),
        ty: key_type,
        writable: false,
        owned: false,
        synthetic: true,
    });
    let error = mir::LocalId(function.locals.len());
    function.locals.push(mir::Local {
        id: error,
        name: "error".into(),
        ty: mir::Type::ERROR,
        writable: false,
        owned: true,
        synthetic: true,
    });
    let identity = mir::RetainedStorageIdentity {
        origin: function.source_span,
        callers: Vec::new(),
    };
    let plan = mir::RetainedSourcesPlan {
        stores: vec![mir::RetainedStore {
            destination: function.params[0],
            path: Vec::new(),
            identity: identity.clone(),
            sources: vec![mir::RetainedSource {
                local: function.params[1],
                inherited: true,
                writable: false,
                lifetime: mir::RetainedLifetime::Cleanup,
                owned_path: Vec::new(),
            }],
        }],
        returned_selectors: vec![mir::RetainedStorageSelector::Member(identity)],
        ..Default::default()
    };
    function.blocks = vec![
        mir::BasicBlock {
            id: mir::BlockId(0),
            statements: vec![mir::Statement::ControlFlowPlan(
                mir::ControlFlowPlan::RetainedSources(plan),
            )],
            terminator: mir::Terminator::CheckedIo {
                operation: mir::CheckedIoOperation::RegisterReadinessStream {
                    set: function.params[0],
                    stream: function.params[1],
                    key,
                },
                result: Some(result),
                error,
                success: mir::BlockId(1),
                failure: mir::BlockId(2),
                span: function.source_span,
            },
        },
        mir::BasicBlock {
            id: mir::BlockId(1),
            statements: Vec::new(),
            terminator: mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Inline(
                mir::InlineValueExpression::Use {
                    ty: key,
                    mode: mir::InlineValueUseMode::Borrow,
                    operand: mir::Operand::Local(result),
                },
            ))),
        },
        mir::BasicBlock {
            id: mir::BlockId(2),
            statements: Vec::new(),
            terminator: mir::Terminator::PropagateError { error },
        },
    ];
    program
}

fn retained_store_plan(program: &mut mir::Program) -> &mut mir::RetainedSourcesPlan {
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "retain")
        .unwrap();
    let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
        &mut function.blocks[0].statements[0]
    else {
        panic!("retained plan")
    };
    plan
}

#[test]
fn mir_derives_stored_loans_and_nonowning_keys_from_executable_membership() {
    doriac::mir_validation::validate_program(&stored_owner_mir()).unwrap();
}
#[test]
fn mir_helpers_preserve_storage_effects_across_return_types_and_expression_positions() {
    let results = [
        mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::constant(IntegerValue::from_bits(IntegerType::Int64, 1)),
        )),
        mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
        })),
        mir::Rvalue::String(mir::StringExpression::Literal("stored".into())),
    ];
    for result in results {
        for position in 0..4 {
            let mut program = stored_owner_mir();
            let retain = program
                .functions
                .iter_mut()
                .find(|f| f.name == "retain")
                .unwrap();
            let mir::Terminator::CheckedIo { error, .. } = retain.blocks[0].terminator else {
                panic!("registration")
            };
            let result_type = result.ty();
            retain.return_type = mir::ReturnType::Value(result_type);
            retain.checked_effects.clear();
            retain.required_checked_effects.clear();
            retain.ambient_checked_effects.clear();
            retain.test_assertion_checked_effects.clear();
            retain.blocks[1].terminator = mir::Terminator::Return(result.clone());
            retain.blocks[2]
                .statements
                .push(mir::Statement::DropError { local: error });
            retain.blocks[2].terminator = mir::Terminator::Return(result.clone());
            let mut relay = retain.clone();
            let retain_id = retain.id;
            relay.id = mir::FunctionId(program.functions.len());
            relay.name = "relay".into();
            let site = relay.source_span;
            let mut plan = retained_store_plan(&mut program).clone();
            plan.returned_selectors.clear();
            plan.stores[0].identity.callers.push(site);
            retained_store_plan(&mut program).returned_selectors.clear();
            let args = relay
                .params
                .iter()
                .map(|local| match relay.locals[local.0].ty {
                    mir::Type::Class(class) => mir::Rvalue::Class(mir::ClassExpression::Local {
                        class,
                        local: *local,
                        transfer: *local == relay.params[1],
                    }),
                    mir::Type::Scalar(mir::ScalarType::Inline(ty)) => mir::Rvalue::Value(
                        mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
                            ty,
                            mode: mir::InlineValueUseMode::Borrow,
                            operand: mir::Operand::Local(*local),
                        }),
                    ),
                    _ => panic!("parameter"),
                })
                .collect();
            let call = match result_type {
                mir::Type::Scalar(mir::ScalarType::Integer(ty)) => mir::Rvalue::Value(
                    mir::ValueExpression::Integer(mir::IntegerExpression::Call {
                        ty,
                        function: retain_id,
                        args,
                        span: site,
                    }),
                ),
                mir::Type::Scalar(mir::ScalarType::Bool) => {
                    mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Call {
                        function: retain_id,
                        args,
                        span: site,
                    }))
                }
                mir::Type::String => mir::Rvalue::String(mir::StringExpression::Call {
                    function: retain_id,
                    args,
                    span: site,
                }),
                _ => panic!("result"),
            };
            let mut statements = vec![mir::Statement::ControlFlowPlan(
                mir::ControlFlowPlan::RetainedSources(plan),
            )];
            let mut terminator = mir::Terminator::Return(result.clone());
            let mut continuation = None;
            match position {
                0 => {
                    let target = mir::LocalId(relay.locals.len());
                    relay.locals.push(mir::Local {
                        id: target,
                        name: "result".into(),
                        ty: result_type,
                        owned: false,
                        writable: false,
                        synthetic: true,
                    });
                    statements.push(mir::Statement::AssignLocal {
                        target,
                        value: call,
                    });
                }
                1 => terminator = mir::Terminator::Return(call),
                2 => {
                    let nested = match call {
                        mir::Rvalue::Value(mir::ValueExpression::Integer(left)) => {
                            mir::Rvalue::Value(mir::ValueExpression::Integer(
                                mir::IntegerExpression::Binary {
                                    ty: IntegerType::Int64,
                                    op: mir::IntegerBinaryOp::Add,
                                    left: Box::new(left),
                                    right: Box::new(mir::IntegerExpression::constant(
                                        IntegerValue::from_bits(IntegerType::Int64, 1),
                                    )),
                                    span: site,
                                    right_span: site,
                                },
                            ))
                        }
                        mir::Rvalue::Value(mir::ValueExpression::Bool(value)) => {
                            mir::Rvalue::Value(mir::ValueExpression::Bool(
                                mir::BoolExpression::Not(Box::new(value)),
                            ))
                        }
                        mir::Rvalue::String(value) => {
                            mir::Rvalue::String(mir::StringExpression::Concat(vec![
                                value,
                                mir::StringExpression::Literal("!".into()),
                            ]))
                        }
                        _ => panic!("nested result"),
                    };
                    terminator = mir::Terminator::Return(nested);
                }
                3 => {
                    let condition = match call {
                        mir::Rvalue::Value(value) => mir::BoolExpression::Compare {
                            op: mir::CompareOp::Equal,
                            left: Box::new(value),
                            right: Box::new(match result.clone() {
                                mir::Rvalue::Value(value) => value,
                                _ => unreachable!(),
                            }),
                        },
                        mir::Rvalue::String(value) => mir::BoolExpression::StringCompare {
                            op: mir::CompareOp::Equal,
                            left: Box::new(value),
                            right: Box::new(mir::StringExpression::Literal("stored".into())),
                        },
                        _ => panic!("condition"),
                    };
                    terminator = mir::Terminator::Branch {
                        condition,
                        then_block: mir::BlockId(1),
                        else_block: mir::BlockId(1),
                    };
                    continuation = Some(mir::BasicBlock {
                        id: mir::BlockId(1),
                        statements: Vec::new(),
                        terminator: mir::Terminator::Return(result.clone()),
                    });
                }
                _ => unreachable!(),
            }
            relay.blocks = vec![mir::BasicBlock {
                id: mir::BlockId(0),
                statements,
                terminator,
            }];
            relay.blocks.extend(continuation);
            program.functions.push(relay);
            doriac::mir_validation::validate_program(&program)
                .unwrap_or_else(|error| panic!("{result_type:?} position {position}: {error:?}"));
            let relay = program.functions.last_mut().unwrap();
            let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
                &mut relay.blocks[0].statements[0]
            else {
                panic!("relay contract");
            };
            plan.stores.clear();
            let error = doriac::mir_validation::validate_program(&program).unwrap_err();
            assert!(
                error.message.contains("stored owner loses"),
                "{result_type:?} position {position}: {error:?}"
            );
        }
    }
}

#[test]
fn mir_conditional_removal_requires_the_call_to_execute_on_every_path() {
    let mut program = stored_owner_mir();
    let remove = program
        .functions
        .iter_mut()
        .find(|f| f.name == "retain")
        .unwrap();
    let mir::Terminator::CheckedIo { error, .. } = remove.blocks[0].terminator else {
        panic!("registration");
    };
    let set = remove.params[0];
    let stream = remove.params.remove(1);
    remove.parameter_modes.remove(1);
    let key = remove.params[1];
    let stream_type = remove.locals[stream.0].ty;
    let mir::Type::Class(class) = stream_type else {
        panic!("stream");
    };
    remove.locals[stream.0].synthetic = true;
    let result_type = mir::Type::Scalar(mir::ScalarType::Bool);
    remove.return_type = mir::ReturnType::Value(result_type);
    remove.checked_effects.clear();
    remove.required_checked_effects.clear();
    remove.ambient_checked_effects.clear();
    remove.test_assertion_checked_effects.clear();
    let selection = mir::RetainedStorageSelection {
        destination: set,
        path: Vec::new(),
        selectors: vec![mir::RetainedStorageSelector::Input(key)],
    };
    let plan = mir::RetainedSourcesPlan {
        removals: vec![selection.clone()],
        ..Default::default()
    };
    remove.blocks[0].statements = vec![mir::Statement::ControlFlowPlan(
        mir::ControlFlowPlan::RetainedSources(plan.clone()),
    )];
    let mir::Terminator::CheckedIo {
        operation, result, ..
    } = &mut remove.blocks[0].terminator
    else {
        unreachable!();
    };
    *operation = mir::CheckedIoOperation::UnregisterReadinessStream {
        set,
        key,
        stream_type,
    };
    *result = Some(stream);
    remove.blocks[1].statements = vec![mir::Statement::DropClass {
        class,
        local: stream,
        runs_destructor: true,
    }];
    remove.blocks[1].terminator = mir::Terminator::Return(mir::Rvalue::Value(
        mir::ValueExpression::Bool(mir::BoolExpression::Use {
            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
        }),
    ));
    remove.blocks[2].statements = vec![mir::Statement::DropError { local: error }];
    remove.blocks[2].terminator = mir::Terminator::Unreachable;
    let template = remove.clone();
    doriac::mir_validation::validate_program(&program).unwrap();
    #[derive(Clone, Copy, Debug)]
    enum Guard {
        Binary(mir::BoolBinaryOp, Option<bool>),
        Fallback(Option<bool>, bool),
        NullSafe(Option<bool>),
        NullSafeArguments(Option<bool>),
    }
    for (guard, definite) in [
        (Guard::Binary(mir::BoolBinaryOp::And, None), false),
        (Guard::Binary(mir::BoolBinaryOp::And, Some(false)), false),
        (Guard::Binary(mir::BoolBinaryOp::And, Some(true)), true),
        (Guard::Binary(mir::BoolBinaryOp::Or, None), false),
        (Guard::Binary(mir::BoolBinaryOp::Or, Some(true)), false),
        (Guard::Binary(mir::BoolBinaryOp::Or, Some(false)), true),
        (Guard::Binary(mir::BoolBinaryOp::Xor, None), true),
        (Guard::Fallback(None, false), false),
        (Guard::Fallback(Some(true), false), false),
        (Guard::Fallback(Some(false), false), true),
        (Guard::Fallback(None, true), false),
        (Guard::Fallback(Some(true), true), false),
        (Guard::Fallback(Some(false), true), true),
        (Guard::NullSafe(None), false),
        (Guard::NullSafe(Some(true)), true),
        (Guard::NullSafeArguments(None), false),
        (Guard::NullSafeArguments(Some(false)), false),
        (Guard::NullSafeArguments(Some(true)), true),
    ] {
        let mut program = program.clone();
        let mut relay = template.clone();
        relay.id = mir::FunctionId(program.functions.len());
        relay.name = "conditionalRemove".into();
        let enabled = mir::LocalId(relay.locals.len());
        let enabled_type = if matches!(guard, Guard::Fallback(..)) {
            mir::Type::NullableScalar(mir::ScalarType::Bool)
        } else {
            result_type
        };
        relay.locals.push(mir::Local {
            id: enabled,
            name: "enabled".into(),
            ty: enabled_type,
            owned: false,
            writable: false,
            synthetic: false,
        });
        relay.params.push(enabled);
        relay
            .parameter_modes
            .push(mir::FunctionParameterMode::Readonly);
        let mut plan = plan.clone();
        if !definite {
            plan.removals.clear();
        }
        let right = mir::BoolExpression::Call {
            function: template.id,
            span: relay.source_span,
            args: vec![
                mir::Rvalue::Class(mir::ClassExpression::Local {
                    class: match relay.locals[set.0].ty {
                        mir::Type::Class(class) => class,
                        _ => unreachable!(),
                    },
                    local: set,
                    transfer: false,
                }),
                mir::Rvalue::Value(mir::ValueExpression::Inline(
                    mir::InlineValueExpression::Use {
                        mode: mir::InlineValueUseMode::Borrow,
                        ty: match relay.locals[key.0].ty {
                            mir::Type::Scalar(mir::ScalarType::Inline(ty)) => ty,
                            _ => unreachable!(),
                        },
                        operand: mir::Operand::Local(key),
                    },
                )),
            ],
        };
        let expression = match guard {
            Guard::Binary(op, value) => {
                mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Binary {
                    op,
                    left: Box::new(mir::BoolExpression::Use {
                        operand: value.map_or(mir::Operand::Local(enabled), |value| {
                            mir::Operand::Scalar(mir::ScalarValue::Bool(value))
                        }),
                    }),
                    right: Box::new(right),
                }))
            }
            Guard::Fallback(present, nested) => {
                let ty = mir::ScalarType::Bool;
                let mut left = match present {
                    None => mir::NullableScalarExpression::Local { local: enabled, ty },
                    Some(false) => mir::NullableScalarExpression::Null(ty),
                    Some(true) => mir::NullableScalarExpression::Value(mir::ValueExpression::Bool(
                        mir::BoolExpression::Use {
                            operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
                        },
                    )),
                };
                if nested {
                    left = mir::NullableScalarExpression::Coalesce {
                        ty,
                        left: Box::new(mir::NullableScalarExpression::Null(ty)),
                        right: Box::new(left),
                    };
                }
                mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Coalesce {
                    left: Box::new(left),
                    right: Box::new(right),
                }))
            }
            Guard::NullSafe(present) => {
                let mir::BoolExpression::Call {
                    function,
                    span,
                    mut args,
                } = right
                else {
                    unreachable!();
                };
                let mir::Rvalue::Class(receiver) = args.remove(0) else {
                    unreachable!();
                };
                let class = receiver.class();
                program.functions[function.0].method = Some(mir::MethodIdentity {
                    class,
                    name: "removeMember".into(),
                    return_adapter: None,
                    virtual_family: None,
                });
                program.functions[function.0].receiver_mode = Some(mir::ReceiverMode::Writable);
                let object = match present {
                    Some(true) => mir::NullableClassExpression::Class(receiver),
                    Some(false) => mir::NullableClassExpression::Null(class),
                    None => {
                        relay.locals[set.0].ty = mir::Type::NullableClass(class);
                        mir::NullableClassExpression::Local {
                            class,
                            local: set,
                            transfer: false,
                        }
                    }
                };
                relay.return_type =
                    mir::ReturnType::Value(mir::Type::NullableScalar(mir::ScalarType::Bool));
                mir::Rvalue::NullableScalar(mir::NullableScalarExpression::NullSafeCall {
                    ty: mir::ScalarType::Bool,
                    object: Box::new(object),
                    function,
                    args,
                    span,
                })
            }
            Guard::NullSafeArguments(present) => {
                let mir::Type::Class(class) = relay.locals[set.0].ty else {
                    unreachable!();
                };
                let mut probe = template.clone();
                probe.id = mir::FunctionId(program.functions.len());
                probe.name = "probe".into();
                probe.method = Some(mir::MethodIdentity {
                    class,
                    name: "probe".into(),
                    return_adapter: None,
                    virtual_family: None,
                });
                probe.receiver_mode = Some(mir::ReceiverMode::Readonly);
                probe.parameter_modes[0] = mir::FunctionParameterMode::Readonly;
                probe.locals[set.0].writable = false;
                probe.locals[key.0].ty = result_type;
                probe.blocks = vec![mir::BasicBlock {
                    id: mir::BlockId(0),
                    statements: Vec::new(),
                    terminator: mir::Terminator::Return(mir::Rvalue::Value(
                        mir::ValueExpression::Bool(mir::BoolExpression::Use {
                            operand: mir::Operand::Local(key),
                        }),
                    )),
                }];
                let function = probe.id;
                program.functions.push(probe);
                let receiver = mir::LocalId(relay.locals.len());
                relay.locals.push(mir::Local {
                    id: receiver,
                    name: "receiver".into(),
                    ty: if present.is_none() {
                        mir::Type::NullableClass(class)
                    } else {
                        mir::Type::Class(class)
                    },
                    owned: false,
                    writable: false,
                    synthetic: false,
                });
                relay.params.push(receiver);
                relay
                    .parameter_modes
                    .push(mir::FunctionParameterMode::Readonly);
                let object = match present {
                    Some(false) => mir::NullableClassExpression::Null(class),
                    Some(true) => {
                        mir::NullableClassExpression::Class(mir::ClassExpression::Local {
                            class,
                            local: receiver,
                            transfer: false,
                        })
                    }
                    None => mir::NullableClassExpression::Local {
                        class,
                        local: receiver,
                        transfer: false,
                    },
                };
                relay.return_type =
                    mir::ReturnType::Value(mir::Type::NullableScalar(mir::ScalarType::Bool));
                mir::Rvalue::NullableScalar(mir::NullableScalarExpression::NullSafeCall {
                    ty: mir::ScalarType::Bool,
                    object: Box::new(object),
                    function,
                    args: vec![mir::Rvalue::Value(mir::ValueExpression::Bool(right))],
                    span: relay.source_span,
                })
            }
        };
        relay.blocks = vec![mir::BasicBlock {
            id: mir::BlockId(0),
            statements: vec![mir::Statement::ControlFlowPlan(
                mir::ControlFlowPlan::RetainedSources(plan),
            )],
            terminator: mir::Terminator::Return(expression),
        }];
        relay.id = mir::FunctionId(program.functions.len());
        program.functions.push(relay);
        doriac::mir_validation::validate_program(&program)
            .unwrap_or_else(|error| panic!("{guard:?}: {error:?}"));
        let relay = program.functions.last_mut().unwrap();
        let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
            &mut relay.blocks[0].statements[0]
        else {
            unreachable!();
        };
        if definite {
            plan.removals.clear();
        } else {
            plan.removals.push(selection.clone());
        }
        let error = doriac::mir_validation::validate_program(&program).unwrap_err();
        assert!(
            error.message.contains("removed storage disagrees"),
            "{guard:?}: {error:?}"
        );
    }
}

#[test]
fn mir_rejects_omitted_stored_input_loans() {
    for entire_contract in [false, true] {
        let mut program = stored_owner_mir();
        if entire_contract {
            let function = program
                .functions
                .iter_mut()
                .find(|function| function.name == "retain")
                .unwrap();
            function.blocks[0].statements.retain(|statement| {
                !matches!(
                    statement,
                    mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(_)),
                )
            });
        } else {
            retained_store_plan(&mut program).stores.clear();
        }
        let error = doriac::mir_validation::validate_program(&program).unwrap_err();
        assert!(error.message.contains("stored owner loses"), "{error:?}");
    }
}

#[test]
fn mir_rejects_forged_stored_input_sources() {
    let mut program = stored_owner_mir();
    let plan = retained_store_plan(&mut program);
    plan.stores[0].sources[0].local = plan.stores[0].destination;
    let error = doriac::mir_validation::validate_program(&program).unwrap_err();
    assert!(error.message.contains("stored owner loses"), "{error:?}");
}

#[test]
fn mir_rejects_missing_and_forged_returned_key_identities() {
    for forged in [false, true] {
        let mut program = stored_owner_mir();
        let plan = retained_store_plan(&mut program);
        if forged {
            let mir::RetainedStorageSelector::Member(identity) = &mut plan.returned_selectors[0]
            else {
                panic!("registered member")
            };
            identity.origin.start += 1;
        } else {
            plan.returned_selectors.clear();
        }
        let error = doriac::mir_validation::validate_program(&program).unwrap_err();
        assert!(error.message.contains("ownership selectors"), "{error:?}");
    }
}

#[test]
fn mir_reused_call_results_do_not_keep_a_previous_keys_identity() {
    let mut program = stored_owner_mir();
    let fresh = program
        .functions
        .iter()
        .find(|function| function.name == "fresh")
        .unwrap()
        .id;
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "retain")
        .unwrap();
    let mir::Terminator::CheckedIo {
        result,
        error,
        operation,
        ..
    } = function.blocks[0].terminator.clone()
    else {
        panic!("registration")
    };
    let mir::CheckedIoOperation::RegisterReadinessStream { key, .. } = operation else {
        panic!("registration")
    };
    let return_value = function.blocks[1].terminator.clone();
    function.blocks[1].terminator = mir::Terminator::CheckedCall {
        function: fresh,
        args: vec![mir::Rvalue::Value(mir::ValueExpression::Inline(
            mir::InlineValueExpression::Use {
                ty: key,
                mode: mir::InlineValueUseMode::Borrow,
                operand: mir::Operand::Local(function.params[2]),
            },
        ))],
        result,
        error,
        success: mir::BlockId(3),
        failure: mir::BlockId(2),
        span: function.source_span,
    };
    function.blocks.push(mir::BasicBlock {
        id: mir::BlockId(3),
        statements: Vec::new(),
        terminator: return_value,
    });
    let seed = function.params[2];
    let plan = retained_store_plan(&mut program);
    plan.returned_selectors = vec![mir::RetainedStorageSelector::Input(seed)];
    plan.exceptional_stores = plan.stores.clone();
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn nested_key_forwarding_preserves_exact_removal_without_owning_the_source() {
    let analysis = analyze(&format!(
        r#"{BORROWING_ADAPTER}
function forward<T>(RegistrationKey<T> $key): RegistrationKey<T> {{ return $key; }}
function relay<T>(RegistrationKey<T> $key): RegistrationKey<T> throws IoError {{
    return forward($key);
}}
function release(take BorrowingReady $stream): void {{}}
function main(): void {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    let $forwarded = forward(relay($firstKey));
    release($set->unregister($forwarded));
    $first->add(3);
    echo $set->unregister($secondKey)->getCurrent();
    let $stillNonowning = $forwarded;
}}
"#
    ));
    assert_valid(&analysis);
}

#[test]
fn forwarded_keys_do_not_release_an_unselected_members_source() {
    assert_retained_source_error(
        r#"
function select<T>(RegistrationKey<T> $first, RegistrationKey<T> $second): RegistrationKey<T> {
    return $second;
}
function release(take BorrowingReady $stream): void {}
function main(): void {
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    release($set->unregister(select($firstKey, $secondKey)));
    $first->add(3);
    echo $set->unregister($firstKey)->getCurrent();
}
"#,
        "E0763",
    );
}

#[test]
fn a_key_from_unknown_storage_cannot_discharge_a_known_members_sources() {
    assert_retained_source_error(
        r#"
class KeyBox {
    function __construct(?RegistrationKey<BorrowingReady> $key) {}
}
function release(take BorrowingReady $stream): void {}
function main(): void {
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    let $box = new KeyBox($secondKey);
    release($set->unregister($box->key ?? $firstKey));
    $first->add(3);
    echo $set->unregister($firstKey)->getCurrent();
}
"#,
        "E0763",
    );
}

#[test]
fn generic_key_parameters_properties_and_returns_remain_copy() {
    let analysis = analyze(
        r#"
class KeyBox<T> {
    function __construct(RegistrationKey<T> $key) {}
    function getKey(): RegistrationKey<T> { return $this->key; }
}
function roundTrip<T>(RegistrationKey<T> $key): RegistrationKey<T> {
    let $box = new KeyBox<T>($key);
    let $copy = $key;
    return $box->getKey();
}
function main(): void {}
"#,
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn generic_nonkey_returns_preserve_their_retained_source_loans() {
    let body = r#"
function relay<T implements ReadinessStream>(take T $stream): T { return $stream; }
function main(): void {
    writable List<int> $source = [1];
    let $returned = relay(new BorrowingReady(File::openRead("input"), $source));
    $source->add(2);
    echo $returned->getCurrent();
}
"#;
    assert_retained_source_error(body, "E0763");
    let valid = analyze(&format!(
        "{BORROWING_ADAPTER}\n{}",
        body.replace("$source->add(2);", "")
    ));
    assert!(valid.diagnostics.is_empty(), "{:#?}", valid.diagnostics);
}

#[test]
fn mir_key_forwarding_is_derived_from_actual_inputs_and_nested_calls() {
    let source = format!(
        r#"{IMPORTS}
function beforeDeclaration(RegistrationKey<ReadableFile> $key): RegistrationKey<ReadableFile> {{ return afterDeclaration($key); }}
function afterDeclaration(RegistrationKey<ReadableFile> $key): RegistrationKey<ReadableFile> {{ return $key; }}
function select(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second,
    bool $condition): RegistrationKey<ReadableFile> {{ return $second; }}
function relay(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second,
    bool $condition): RegistrationKey<ReadableFile> {{ return select($first, $second, $condition); }}
function checked(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second,
    bool $condition): RegistrationKey<ReadableFile> throws IoError {{ return relay($first, $second, $condition); }}
function nullable(RegistrationKey<ReadableFile> $key): ?RegistrationKey<ReadableFile> {{ return $key; }}
function absent(): ?RegistrationKey<ReadableFile> {{ return null; }}
function checkedAbsent(?RegistrationKey<ReadableFile> $key): ?RegistrationKey<ReadableFile> {{
    if ($key == null) {{ return $key; }}
    return null;
}}
function unwrap(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> {{
    return nullable($second) ?? $first;
}}
function unwrapAbsent(RegistrationKey<ReadableFile> $first): RegistrationKey<ReadableFile> {{
    return absent() ?? $first;
}}
function narrowed(RegistrationKey<ReadableFile> $first, ?RegistrationKey<ReadableFile> $second,
    RegistrationKey<ReadableFile> $third): RegistrationKey<ReadableFile> {{
    if ($second != null) {{ return $second ?? $first; }}
    return $third;
}}
class KeyBox {{ function __construct(?RegistrationKey<ReadableFile> $key) {{}} }}
function fromStorage(KeyBox $box, RegistrationKey<ReadableFile> $fallback): RegistrationKey<ReadableFile> {{
    return $box->key ?? $fallback;
}}
function main(): void {{}}
"#
    );
    let program = doriac::lower_source_to_mir("key-forwarding.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for name in ["select", "relay", "checked"] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let plan = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match statement {
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                    Some(plan)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            plan.returned_selectors,
            vec![mir::RetainedStorageSelector::Input(function.params[1])]
        );
    }
    for (name, inputs, unknown) in [
        ("unwrap", vec![0, 1], false),
        ("unwrapAbsent", vec![0], false),
        ("narrowed", vec![1, 2], false),
        ("fromStorage", vec![1], true),
        ("checkedAbsent", vec![], false),
        ("beforeDeclaration", vec![0], false),
    ] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let plan = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match statement {
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                    Some(plan)
                }
                _ => None,
            });
        if inputs.is_empty() && !unknown {
            assert!(plan.is_none(), "{name}: empty ownership plans are omitted");
            continue;
        }
        let plan = plan.expect("returned keys require an ownership plan");
        let mut expected = inputs
            .into_iter()
            .map(|index| mir::RetainedStorageSelector::Input(function.params[index]))
            .collect::<Vec<_>>();
        if unknown {
            expected.push(mir::RetainedStorageSelector::Unknown);
        }
        assert_eq!(
            plan.returned_selectors.len(),
            expected.len(),
            "{name}: {plan:?}"
        );
        assert!(
            expected
                .iter()
                .all(|selector| plan.returned_selectors.contains(selector)),
            "{name}: {plan:?}"
        );
    }
    let mut invalid = program.clone();
    let function = invalid
        .functions
        .iter_mut()
        .find(|function| function.name == "fromStorage")
        .unwrap();
    let plan = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                Some(plan)
            }
            _ => None,
        })
        .unwrap();
    plan.returned_selectors
        .retain(|selector| *selector != mir::RetainedStorageSelector::Unknown);
    let error = doriac::mir_validation::validate_program(&invalid).unwrap_err();
    assert!(error.message.contains("ownership selectors"), "{error:?}");
    for fault in 0..4 {
        let mut invalid = program.clone();
        let function = invalid
            .functions
            .iter_mut()
            .find(|function| function.name == "select")
            .unwrap();
        let params = function.params.clone();
        let plan = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match statement {
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                    Some(plan)
                }
                _ => None,
            })
            .unwrap();
        match fault {
            0 => plan.returned_selectors.clear(),
            1 => plan.returned_selectors = vec![mir::RetainedStorageSelector::Input(params[0])],
            2 => plan
                .returned_selectors
                .push(plan.returned_selectors[0].clone()),
            3 => plan.returned_selectors = vec![mir::RetainedStorageSelector::Input(params[2])],
            _ => unreachable!(),
        }
        let error = doriac::mir_validation::validate_program(&invalid).unwrap_err();
        assert!(
            error.message.contains("ownership selector"),
            "fault {fault}: {error:?}"
        );
    }
}

#[test]
fn mir_key_forwarding_includes_virtual_and_interface_implementations() {
    let source = format!(
        r#"{IMPORTS}
open class RootPicker {{
    open function select(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> {{ return $first; }}
}}
class ChildPicker extends RootPicker {{
    override function select(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> {{ return $second; }}
}}
interface KeyPicker {{
    function select(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile>;
    function selectChecked(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> throws IoError;
}}
class FirstPicker implements KeyPicker {{
    function select(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> {{ return $first; }}
    function selectChecked(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> throws IoError {{ return $first; }}
}}
class SecondPicker implements KeyPicker {{
    function select(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> {{ return $second; }}
    function selectChecked(RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> throws IoError {{ return $second; }}
}}
function selectVirtual(RootPicker $picker, RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> {{ return $picker->select($first, $second); }}
function selectErased(KeyPicker $picker, RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> {{ return $picker->select($first, $second); }}
function selectCheckedErased(KeyPicker $picker, RegistrationKey<ReadableFile> $first, RegistrationKey<ReadableFile> $second): RegistrationKey<ReadableFile> throws IoError {{ return $picker->selectChecked($first, $second); }}
function main(): void {{}}
"#
    );
    let program = doriac::lower_source_to_mir("dispatched-keys.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for name in ["selectVirtual", "selectErased", "selectCheckedErased"] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let plan = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match statement {
                mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                    Some(plan)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(plan.returned_selectors.len(), 2, "{name}: {plan:?}");
        for input in [function.params[1], function.params[2]] {
            assert!(
                plan.returned_selectors
                    .contains(&mir::RetainedStorageSelector::Input(input)),
                "{name}: {plan:?}"
            );
        }
    }
}

#[test]
fn mir_registration_then_removal_preserves_new_and_incoming_sources() {
    let mut program = stored_owner_mir();
    let function = program
        .functions
        .iter_mut()
        .find(|f| f.name == "retain")
        .unwrap();
    let mir::Terminator::CheckedIo {
        operation: mir::CheckedIoOperation::RegisterReadinessStream { set, stream, .. },
        result: Some(key),
        error,
        span,
        ..
    } = function.blocks[0].terminator.clone()
    else {
        panic!("registration")
    };
    let stream_type = function.locals[stream.0].ty;
    let mir::Type::Class(class) = stream_type else {
        panic!("stream type")
    };
    let result = mir::LocalId(function.locals.len());
    function.locals.push(mir::Local {
        id: result,
        name: "returned".into(),
        ty: stream_type,
        writable: false,
        owned: true,
        synthetic: true,
    });
    function.return_type = mir::ReturnType::Value(stream_type);
    function.blocks[1].terminator = mir::Terminator::CheckedIo {
        operation: mir::CheckedIoOperation::UnregisterReadinessStream {
            set,
            key,
            stream_type,
        },
        result: Some(result),
        error,
        success: mir::BlockId(3),
        failure: mir::BlockId(2),
        span,
    };
    function.blocks.push(mir::BasicBlock {
        id: mir::BlockId(3),
        statements: Vec::new(),
        terminator: mir::Terminator::Return(mir::Rvalue::Class(mir::ClassExpression::Local {
            class,
            local: result,
            transfer: true,
        })),
    });
    let plan = retained_store_plan(&mut program);
    let member = plan.stores.pop().unwrap();
    let selection = mir::RetainedStorageSelection {
        destination: set,
        path: Vec::new(),
        selectors: vec![mir::RetainedStorageSelector::Member(
            member.identity.clone(),
        )],
    };
    plan.returns = [set, stream]
        .into_iter()
        .map(|local| mir::RetainedSource {
            local,
            inherited: true,
            writable: false,
            lifetime: mir::RetainedLifetime::Cleanup,
            owned_path: Vec::new(),
        })
        .collect();
    plan.returned_selectors.clear();
    plan.removals.push(selection.clone());
    plan.returned_storage = Some(mir::RetainedReturnedStorage {
        direct: member.sources.clone(),
        selections: vec![selection],
        ..Default::default()
    });
    plan.exceptional_stores.push(member);
    doriac::mir_validation::validate_program(&program).unwrap();
    for fault in 0..4 {
        let mut invalid = program.clone();
        let plan = retained_store_plan(&mut invalid);
        match fault {
            0 => plan.returned_storage.as_mut().unwrap().direct.clear(),
            1 => plan.exceptional_stores.clear(),
            2 => plan.returns.retain(|source| source.local != stream),
            3 => plan.exceptional_removals = plan.removals.clone(),
            _ => unreachable!(),
        }
        let error = doriac::mir_validation::validate_program(&invalid).unwrap_err();
        let expected = match fault {
            0 => "returned storage disagrees",
            1 => "retained input source",
            2 => "return loses",
            3 => "removed storage disagrees",
            _ => unreachable!(),
        };
        assert!(error.message.contains(expected), "fault {fault}: {error:?}");
    }
}

#[test]
fn mir_removal_cannot_erase_incoming_members_source_obligations() {
    let mut program = stored_owner_mir();
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "retain")
        .unwrap();
    let mir::Terminator::CheckedIo {
        result: Some(result),
        operation,
        ..
    } = function.blocks[0].terminator.clone()
    else {
        panic!("registration")
    };
    let mir::CheckedIoOperation::RegisterReadinessStream { stream, .. } = operation else {
        panic!("registration")
    };
    let stream_type = function.locals[stream.0].ty;
    let mir::Type::Class(class) = stream_type else {
        panic!("stream type")
    };
    let input_key = function.params[2];
    function.params.remove(1);
    function.parameter_modes.remove(1);
    function.locals[stream.0].owned = false;
    function.locals[stream.0].synthetic = true;
    function.locals[result.0].ty = stream_type;
    function.locals[result.0].owned = true;
    function.return_type = mir::ReturnType::Value(stream_type);
    let mir::Terminator::CheckedIo { operation, .. } = &mut function.blocks[0].terminator else {
        unreachable!()
    };
    *operation = mir::CheckedIoOperation::UnregisterReadinessStream {
        set: function.params[0],
        key: input_key,
        stream_type,
    };
    function.blocks[1].terminator =
        mir::Terminator::Return(mir::Rvalue::Class(mir::ClassExpression::Local {
            class,
            local: result,
            transfer: true,
        }));
    let set = function.params[0];
    let selection = mir::RetainedStorageSelection {
        destination: set,
        path: Vec::new(),
        selectors: vec![mir::RetainedStorageSelector::Input(input_key)],
    };
    *retained_store_plan(&mut program) = mir::RetainedSourcesPlan {
        returns: vec![mir::RetainedSource {
            local: set,
            inherited: true,
            writable: false,
            lifetime: mir::RetainedLifetime::Cleanup,
            owned_path: Vec::new(),
        }],
        removals: vec![selection.clone()],
        returned_storage: Some(mir::RetainedReturnedStorage {
            selections: vec![selection],
            ..Default::default()
        }),
        ..Default::default()
    };
    doriac::mir_validation::validate_program(&program).unwrap();
    for fault in 0..5 {
        let mut invalid = program.clone();
        let plan = retained_store_plan(&mut invalid);
        match fault {
            0 => plan.removals.clear(),
            1 => plan.exceptional_removals = plan.removals.clone(),
            2 => plan.returned_storage.as_mut().unwrap().selections.clear(),
            3 => plan.removals[0].selectors = vec![mir::RetainedStorageSelector::Unknown],
            4 => {
                plan.returned_storage.as_mut().unwrap().selections[0].selectors =
                    vec![mir::RetainedStorageSelector::Unknown]
            }
            _ => unreachable!(),
        }
        let error = doriac::mir_validation::validate_program(&invalid).unwrap_err();
        assert!(
            error.message.contains("storage"),
            "fault {fault}: {error:?}"
        );
    }
    let first = program
        .functions
        .iter()
        .find(|f| f.name == "retain")
        .unwrap()
        .id;
    for depth in 0..2 {
        let callee = if depth == 0 {
            first
        } else {
            mir::FunctionId(program.functions.len() - 1)
        };
        let mut caller = program.functions[callee.0].clone();
        caller.id = mir::FunctionId(program.functions.len());
        caller.name = format!("removalRelay{depth}");
        let set = caller.params[0];
        let mir::Type::Class(set_class) = caller.locals[set.0].ty else {
            unreachable!()
        };
        let key = caller.params[1];
        let mir::Type::Scalar(mir::ScalarType::Inline(key_type)) = caller.locals[key.0].ty else {
            unreachable!()
        };
        let (result, error, success, failure, span) = match caller.blocks[0].terminator.clone() {
            mir::Terminator::CheckedIo {
                result,
                error,
                success,
                failure,
                span,
                ..
            }
            | mir::Terminator::CheckedCall {
                result,
                error,
                success,
                failure,
                span,
                ..
            } => (result, error, success, failure, span),
            _ => unreachable!(),
        };
        caller.blocks[0].terminator = mir::Terminator::CheckedCall {
            function: callee,
            args: vec![
                mir::Rvalue::Class(mir::ClassExpression::Local {
                    class: set_class,
                    local: set,
                    transfer: false,
                }),
                mir::Rvalue::Value(mir::ValueExpression::Inline(
                    mir::InlineValueExpression::Use {
                        ty: key_type,
                        mode: mir::InlineValueUseMode::Borrow,
                        operand: mir::Operand::Local(key),
                    },
                )),
            ],
            result,
            error,
            success,
            failure,
            span,
        };
        program.functions.push(caller);
        doriac::mir_validation::validate_program(&program).unwrap();
    }
    retained_store_plan(&mut program).returns.clear();
    let error = doriac::mir_validation::validate_program(&program).unwrap_err();
    assert!(error.message.contains("return loses"), "{error:?}");
}

#[test]
fn taking_and_returning_a_set_preserves_its_stored_source_loans() {
    assert_retained_source_error(
        r#"
function forward(take ReadinessSet $set): ReadinessSet { return $set; }
function main(): void {
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    $set->register(new BorrowingReady(File::openRead("input"), $source));
    let $returned = forward($set);
    $source->add(2);
}
"#,
        "E0763",
    );
}

#[test]
fn callbacks_returning_sets_preserve_stored_source_loans() {
    for callback in [
        "fn(take ReadinessSet $input) => $input",
        "function (take ReadinessSet $input): ReadinessSet { return $input; }",
    ] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable ReadinessSet $set = new ReadinessSet();
        $set->register(new BorrowingReady(File::openRead("input"), $source));
        let $forward = {callback};
        let $returned = $forward($set);
        $source->add(2);
    }}
    $source->add(3);
}}
function main(): void {{}}
"#
        );
        let analysis = analyze(&source);
        let mutation = source.find("$source->add(2)").unwrap() + IMPORTS.len() + 1;
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code, diagnostic.span.start))
                .collect::<Vec<_>>(),
            vec![("E0763", mutation)],
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        assert_valid(&analyze(&source.replace("        $source->add(2);", "")));
    }
}

#[test]
fn returned_callbacks_keep_captured_stored_owners_sources_until_cleanup() {
    for callback in [
        "fn() with (take $input) => inspect($input)",
        "function (): void with (take $input) { inspect($input); }",
    ] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function inspect(ReadinessSet $set): void {{}}
function keep(take ReadinessSet $input): function(): void {{ return {callback}; }}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable ReadinessSet $set = new ReadinessSet();
        $set->register(new BorrowingReady(File::openRead("input"), $source));
        let $recover = keep($set);
        $recover();
        $source->add(2);
    }}
    $source->add(2);
}}
function main(): void {{}}
"#
        );
        let analysis = analyze(&source);
        let mutation = source.find("$source->add(2)").unwrap() + IMPORTS.len() + 1;
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code, diagnostic.span.start))
                .collect::<Vec<_>>(),
            vec![("E0654", mutation)],
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        let valid = source.replacen("        $source->add(2);", "", 1);
        assert_valid(&analyze(&valid));
        let helper = format!(
            "{IMPORTS}\nfunction inspect(ReadinessSet $set): void {{}}\nfunction keep(take ReadinessSet $input): function(): void {{ return {callback}; }}\nfunction main(): void {{}}"
        );
        let program = doriac::lower_source_to_mir("captured-storage.doria", &helper).unwrap();
        let keep = program.functions.iter().find(|f| f.name == "keep").unwrap();
        assert!(matches!(
            keep.return_borrow,
            Some(mir::ReturnBorrow {
                kind: mir::ReturnBorrowKind::Retained,
                lifetime: doriac::types::RetainedLifetime::Cleanup,
                ..
            })
        ));
        doriac::mir_validation::validate_program(&program).unwrap();
    }
}

#[test]
fn returned_sets_preserve_members_for_independent_removal() {
    for forward in ["forward($set)", "forward(forward($set))"] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function forward(take ReadinessSet $set): ReadinessSet {{ return $set; }}
function release(take BorrowingReady $stream): void {{}}
function exercise(): void throws IoError {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    let writable $returned = {forward};
    release($returned->unregister($firstKey));
    $first->add(3);
    release($returned->unregister($secondKey));
    $second->add(4);
}}
function main(): void {{}}
"#
        );
        let analysis = analyze(&source);
        assert_valid(&analysis);
        let source = source.replace("$first->add(3);", "$first->add(3); $second->add(3);");
        let analysis = analyze(&source);
        let mutation = source.find("$second->add(3)").unwrap() + IMPORTS.len() + 1;
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0763" && diagnostic.span.start == mutation),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn callback_dispatch_preserves_members_for_independent_removal() {
    for callback in [
        "fn(take ReadinessSet $input) => $input",
        "when ($choose) { return fn(take ReadinessSet $input) => $input; } else { return function (take ReadinessSet $input): ReadinessSet { return $input; }; }",
    ] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function release(take BorrowingReady $stream): void {{}}
function exercise(bool $choose): void throws IoError {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    let $forward = {callback};
    let writable $returned = $forward($set);
    release($returned->unregister($firstKey));
    $first->add(3);
    release($returned->unregister($secondKey));
    $second->add(4);
}}
function main(): void {{}}
"#
        );
        assert_valid(&analyze(&source));
        let source = source.replace("$first->add(3);", "$first->add(3); $second->add(3);");
        let analysis = analyze(&source);
        let mutation = source.find("$second->add(3)").unwrap() + IMPORTS.len() + 1;
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code, diagnostic.span.start))
                .collect::<Vec<_>>(),
            vec![("E0763", mutation)],
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn mir_proves_returned_storage_from_helper_bodies() {
    let source = format!("{IMPORTS}\nfunction forward(take ReadinessSet $set): ReadinessSet {{ return $set; }}\nfunction main(): void {{}}");
    let mut program = doriac::lower_source_to_mir("returned-storage.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "forward")
        .unwrap();
    let input = function.params[0];
    let plan = function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                Some(plan)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(plan.returned_storage.as_ref().unwrap().inputs, vec![input]);
    plan.returned_storage = None;
    let error = doriac::mir_validation::validate_program(&program).unwrap_err();
    assert!(
        error.message.contains("returned storage disagrees"),
        "{error:?}"
    );
}

#[test]
fn factory_returns_keep_their_members_separate_from_later_registration() {
    let source = format!(
        r#"{BORROWING_ADAPTER}
function create(take BorrowingReady $stream): ReadinessSet throws IoError {{
    writable ReadinessSet $set = new ReadinessSet();
    $set->register($stream);
    return $set;
}}
function release(take BorrowingReady $stream): void {{}}
function exercise(): void throws IoError {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    let writable $set = create(new BorrowingReady(File::openRead("input"), $first));
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $second));
    release($set->unregister($key));
    $second->add(3);
    $first->add(4);
}}
function main(): void {{}}
"#
    );
    let analysis = analyze(&source);
    let mutation = source.find("$first->add(4)").unwrap() + IMPORTS.len() + 1;
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0763")
            .map(|diagnostic| diagnostic.span.start)
            .collect::<Vec<_>>(),
        vec![mutation],
        "{:#?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "E0763"),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn mir_proves_factory_member_identity_and_each_members_sources() {
    let mut program = stored_owner_mir();
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "retain")
        .unwrap();
    let set = function.params[0];
    let stream = function.params[1];
    let mir::Type::Class(class) = function.locals[set.0].ty else {
        panic!("set type")
    };
    function.locals[set.0].owned = true;
    function.parameter_modes[0] = mir::FunctionParameterMode::Take;
    function.return_type = mir::ReturnType::Value(mir::Type::Class(class));
    function.blocks[1].terminator =
        mir::Terminator::Return(mir::Rvalue::Class(mir::ClassExpression::Local {
            class,
            local: set,
            transfer: true,
        }));
    let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
        &mut function.blocks[0].statements[0]
    else {
        unreachable!()
    };
    let member = plan.stores.remove(0);
    plan.returned_selectors.clear();
    plan.returns = [set, stream]
        .into_iter()
        .map(|local| mir::RetainedSource {
            local,
            inherited: true,
            writable: false,
            lifetime: mir::RetainedLifetime::Cleanup,
            owned_path: Vec::new(),
        })
        .collect();
    plan.returned_storage = Some(mir::RetainedReturnedStorage {
        inputs: vec![set],
        direct: Vec::new(),
        selections: Vec::new(),
        members: vec![mir::RetainedReturnedMember {
            identity: member.identity,
            path: member.path,
            sources: member.sources,
            repeated: false,
        }],
    });
    doriac::mir_validation::validate_program(&program).unwrap();
    for fault in 0..5 {
        let mut invalid = program.clone();
        let plan = retained_store_plan(&mut invalid);
        match fault {
            0 => plan.returned_storage = None,
            1 => plan.returned_storage.as_mut().unwrap().members.clear(),
            2 => plan.returned_storage.as_mut().unwrap().members[0]
                .identity
                .callers
                .push(doriac::source::Span::new(1, 2)),
            3 => plan.returned_storage.as_mut().unwrap().members[0]
                .sources
                .clear(),
            4 => plan.returned_storage.as_mut().unwrap().members[0].repeated = true,
            _ => unreachable!(),
        }
        let error = doriac::mir_validation::validate_program(&invalid).unwrap_err();
        assert!(
            error.message.contains("returned storage disagrees"),
            "fault {fault}: {error:?}"
        );
    }
}

#[test]
fn joined_helper_returns_keep_unpartitioned_sources_after_known_removal() {
    let replacement = r#"
        writable ReadinessSet $other = new ReadinessSet();
        $other->register(new BorrowingReady(File::openRead("input"), $second));
        $set = forward($other);
    "#;
    for (then, otherwise) in [(replacement, ""), ("", replacement)] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function forward(take ReadinessSet $set): ReadinessSet {{ return $set; }}
function release(take BorrowingReady $stream): void {{}}
function exercise(bool $replace): void throws IoError {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $first));
    if ($replace) {{ {then} }} else {{ {otherwise} }}
    release($set->unregister($key));
    $second->add(3);
}}
function main(): void {{}}
"#
        );
        let analysis = analyze(&source);
        let mutation = source.find("$second->add(3)").unwrap() + IMPORTS.len() + 1;
        assert!(
            analysis.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "E0763" && diagnostic.span.start == mutation
            }),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0763"),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn taking_and_returning_an_enclosing_owner_preserves_stored_source_loans() {
    assert_retained_source_error(
        r#"
class Owner {
    writable ReadinessSet $set;
    function __construct() { $this->set = new ReadinessSet(); }
}
function forward(take Owner $owner): Owner { return $owner; }
function main(): void {
    writable List<int> $source = [1];
    let writable $owner = new Owner();
    $owner->set->register(new BorrowingReady(File::openRead("input"), $source));
    let $returned = forward($owner);
    $source->add(2);
}
"#,
        "E0763",
    );
}

#[test]
fn sets_in_owned_collections_preserve_sources_through_take_and_return() {
    let owner = r#"
class Owner {
    writable List<ReadinessSet> $sets;
    function __construct() { $this->sets = [new ReadinessSet()]; }
}
"#;
    for (ty, initial, setup, member) in [
        ("List<ReadinessSet>", "[new ReadinessSet()]", "", "$sets[0]"),
        ("ReadinessSet[]", "[new ReadinessSet()]", "", "$sets[0]"),
        (
            "List<List<ReadinessSet>>",
            "[[new ReadinessSet()]]",
            "",
            "$sets[0][0]",
        ),
        (
            "Dictionary<string, ReadinessSet>",
            "[\"slot\" => new ReadinessSet()]",
            "",
            "$sets[\"slot\"]",
        ),
        (
            "SortedDictionary<string, ReadinessSet>",
            "SortedDictionary::from([])",
            "$sets->set(\"slot\", new ReadinessSet());",
            "$sets[\"slot\"]",
        ),
        ("Owner", "new Owner()", "", "$sets->sets[0]"),
    ] {
        for collection in ["$sets", "forward($sets)", "forward(forward($sets))"] {
            let source = format!(
                r#"{BORROWING_ADAPTER}{owner}
function forward(take {ty} $sets): {ty} {{ return $sets; }}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable {ty} $sets = {initial};
        {setup}
        {member}->register(new BorrowingReady(File::openRead("input"), $source));
        let $returned = {collection};
        $source->add(2);
    }}
    $source->add(3);
}}
function main(): void {{}}
"#
            );
            let analysis = analyze(&source);
            let mutation = source.find("$source->add(2)").unwrap() + IMPORTS.len() + 1;
            assert_eq!(
                analysis
                    .diagnostics
                    .iter()
                    .map(|diagnostic| (diagnostic.code, diagnostic.span.start))
                    .collect::<Vec<_>>(),
                vec![("E0763", mutation)],
                "{source}\n{:#?}",
                analysis.diagnostics
            );
            assert_valid(&analyze(&source.replacen(
                "        $source->add(2);",
                "",
                1,
            )));
        }
        let source = format!(
            "{IMPORTS}{owner}\nfunction forward(take {ty} $sets): {ty} {{ return $sets; }}\nfunction main(): void {{}}"
        );
        let mut program = doriac::lower_source_to_mir("collection-storage.doria", &source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        doriac::mir_validation::validate_program(&program).unwrap();
        let forward = program
            .functions
            .iter_mut()
            .find(|f| f.name == "forward")
            .unwrap();
        forward.blocks.iter_mut().for_each(|block| {
            block.statements.retain(|statement| {
                !matches!(
                    statement,
                    mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(_))
                )
            });
        });
        doriac::mir_validation::validate_program(&program)
            .expect_err("collection forwarding cannot omit its inherited source contract");
    }
}

#[test]
fn borrowed_collection_elements_remain_tied_to_the_container_lifetime() {
    for (ty, value) in [
        ("List<ReadinessSet>", "$sets[0]"),
        ("ReadinessSet[]", "$sets[0]"),
        ("List<List<ReadinessSet>>", "$sets[0][0]"),
        ("Dictionary<string, ReadinessSet>", "$sets[\"slot\"]"),
    ] {
        let source = format!(
            "{IMPORTS}\nfunction first({ty} $sets): ReadinessSet {{ return {value}; }}\nfunction main(): void {{}}"
        );
        let mut program = doriac::lower_source_to_mir("borrowed-collection-storage.doria", &source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        doriac::mir_validation::validate_program(&program).unwrap();
        let first = program
            .functions
            .iter_mut()
            .find(|f| f.name == "first")
            .unwrap();
        assert_eq!(
            first.return_borrow.unwrap().kind,
            mir::ReturnBorrowKind::Value
        );
        first.return_borrow = None;
        doriac::mir_validation::validate_program(&program)
            .expect_err("borrowed elements cannot erase their container lifetime");
    }
}

#[test]
fn collection_extraction_helpers_preserve_retained_sources() {
    for (ty, initial, member, result, extraction) in [
        (
            "List<ReadinessSet>",
            "[new ReadinessSet()]",
            "$sets[0]",
            "ReadinessSet",
            "$sets->removeAt(0)",
        ),
        (
            "List<ReadinessSet>",
            "[new ReadinessSet()]",
            "$sets[0]",
            "?ReadinessSet",
            "$sets->pop()",
        ),
        (
            "Dictionary<string, ReadinessSet>",
            "[\"slot\" => new ReadinessSet()]",
            "$sets[\"slot\"]",
            "?ReadinessSet",
            "$sets->remove(\"slot\")",
        ),
        (
            "List<List<ReadinessSet>>",
            "[[new ReadinessSet()]]",
            "$sets[0][0]",
            "?List<ReadinessSet>",
            "$sets->pop()",
        ),
    ] {
        for body in [
            format!("return {extraction};"),
            format!("let $removed = {extraction}; return relay($removed);"),
        ] {
            let source = format!(
                r#"{BORROWING_ADAPTER}
function relay(take {result} $value): {result} {{ return $value; }}
function extract(take {ty} $incoming): {result} {{
    let writable $sets = $incoming;
    {body}
}}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    {{
        writable {ty} $sets = {initial};
        {member}->register(new BorrowingReady(File::openRead("input"), $source));
        let $removed = extract($sets);
        $source->add(2);
    }}
    $source->add(3);
}}
function main(): void {{}}
"#
            );
            let analysis = analyze(&source);
            let mutation = source.find("$source->add(2)").unwrap() + IMPORTS.len() + 1;
            assert_eq!(
                analysis
                    .diagnostics
                    .iter()
                    .map(|d| (d.code, d.span.start))
                    .collect::<Vec<_>>(),
                vec![("E0763", mutation)],
                "{source}\n{:#?}",
                analysis.diagnostics,
            );
            assert_valid(&analyze(&source.replacen(
                "        $source->add(2);",
                "",
                1,
            )));
        }
        let source = format!("{IMPORTS}\nfunction extract(take {ty} $incoming): {result} {{ let writable $sets = $incoming; return {extraction}; }} function main(): void {{}}");
        let mut program = doriac::lower_source_to_mir("extracted-storage.doria", &source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        doriac::mir_validation::validate_program(&program).unwrap();
        program
            .functions
            .iter_mut()
            .find(|f| f.name == "extract")
            .unwrap()
            .blocks
            .iter_mut()
            .for_each(|block| {
                block.statements.retain(|statement| {
                    !matches!(
                        statement,
                        mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(_))
                    )
                })
            });
        doriac::mir_validation::validate_program(&program)
            .expect_err("element extraction cannot omit its inherited source contract");
    }
}

#[test]
fn collection_clear_retires_stored_sources_through_helpers() {
    for clearing in ["$sets->clear();", "emptySets($sets);", "relay($sets);"] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function emptySets(writable List<ReadinessSet> $sets): void {{ $sets->clear(); }}
function relay(writable List<ReadinessSet> $sets): void {{ emptySets($sets); }}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    writable List<ReadinessSet> $sets = [new ReadinessSet()];
    $sets[0]->register(new BorrowingReady(File::openRead("input"), $source));
    {clearing}
    $source->add(2);
}}
function main(): void {{}}
"#
        );
        assert_valid(&analyze(&source));
    }
    let source = format!(
        r#"{IMPORTS}
function emptySets(writable List<ReadinessSet> $sets): void {{ $sets->clear(); }}
function relay(writable List<ReadinessSet> $sets): void {{ emptySets($sets); }}
function main(): void {{}}
"#
    );
    let program = doriac::lower_source_to_mir("cleared-storage.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for name in ["emptySets", "relay"] {
        let mut forged = program.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|f| f.name == name)
            .unwrap();
        let plan = function
            .blocks
            .iter_mut()
            .flat_map(|b| &mut b.statements)
            .find_map(|s| {
                if let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(
                    plan,
                )) = s
                {
                    Some(plan)
                } else {
                    None
                }
            })
            .expect("clearing must have a proved removal effect");
        plan.removals.clear();
        doriac::mir_validation::validate_program(&forged)
            .expect_err("clearing cannot omit its stored-source removal");
    }
    let mut forged = program.clone();
    let function = forged
        .functions
        .iter_mut()
        .find(|f| f.name == "emptySets")
        .unwrap();
    for block in &mut function.blocks {
        block
            .statements
            .retain(|statement| !matches!(statement, mir::Statement::CollectionClear { .. }));
    }
    doriac::mir_validation::validate_program(&forged)
        .expect_err("a removal annotation cannot replace actual collection cleanup");
}

#[test]
fn collection_element_contracts_cover_nominal_callable_and_nested_carriers() {
    let key = r#"
class Key implements Comparable<Key>, Equatable<Key>, Hashable {
    function compare(Key $other): Ordering { return Ordering::Equal; }
    function equals(Key $other): bool { return true; }
    function hash(): uint64 { return 0; }
}
"#;
    for element in [
        "ReadinessSet",
        "?ReadinessSet",
        "Iterator<int>",
        "?Iterator<int>",
        "function(): int",
        "?function(): int",
        "List<ReadinessSet>",
        "?List<ReadinessSet>",
    ] {
        for (collection, parameters, operation) in [
            (format!("List<{element}>"), "", "$sets->pop()"),
            (format!("Deque<{element}>"), "", "$sets->popFront()"),
            (format!("Deque<{element}>"), "", "$sets->popBack()"),
            (
                format!("Dictionary<Key, {element}>"),
                ", Key $key",
                "$sets->remove($key)",
            ),
            (
                format!("SortedDictionary<Key, {element}>"),
                ", Key $key",
                "$sets->remove($key)",
            ),
        ] {
            let returned = format!("?{}", element.trim_start_matches('?'));
            let source = format!("{IMPORTS}{key}\nfunction extract(take {collection} $incoming{parameters}): {returned} {{ let writable $sets = $incoming; return {operation}; }} function main(): void {{}}");
            let mut program = doriac::lower_source_to_mir("element-carrier.doria", &source)
                .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
            doriac::mir_validation::validate_program(&program)
                .unwrap_or_else(|error| panic!("{source}\n{error:#?}"));
            if element.contains("Iterator") {
                let mut forged = program.clone();
                let mut pruned = false;
                for interface in &mut forged.interface_types {
                    if interface.requirement_origins.iter().any(|origin| {
                        doriac::compiler_known_contracts::IterationOperation::from_requirement(
                            *origin,
                        ) == Some(doriac::compiler_known_contracts::IterationOperation::HasCurrent)
                    }) {
                        assert!(interface.methods.is_empty());
                        interface.requirement_origins.clear();
                        pruned = true;
                    }
                }
                assert!(
                    pruned,
                    "unused iterator contracts must retain nominal identity"
                );
                doriac::mir_validation::validate_program(&forged)
                    .expect_err("pruning methods must not erase iterator source obligations");
            }
            let function = program
                .functions
                .iter_mut()
                .find(|f| f.name == "extract")
                .unwrap();
            function.blocks.iter_mut().for_each(|block| {
                block.statements.retain(|statement| {
                    !matches!(
                        statement,
                        mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(_))
                    )
                })
            });
            doriac::mir_validation::validate_program(&program)
                .expect_err("every retained element must prove its inherited sources");
        }
    }
}

#[test]
fn clearing_one_owned_collection_does_not_retire_other_fields_sources() {
    let source = format!(
        r#"{BORROWING_ADAPTER}
class Owner {{
    writable List<ReadinessSet> $first;
    writable List<ReadinessSet> $second;
    function __construct() {{
        $this->first = [new ReadinessSet()];
        $this->second = [new ReadinessSet()];
    }}
}}
function emptySets(writable List<ReadinessSet> $sets): void {{ $sets->clear(); }}
function exercise(): void throws IoError {{
    writable List<int> $firstSource = [1];
    writable List<int> $secondSource = [2];
    let writable $owner = new Owner();
    $owner->first[0]->register(new BorrowingReady(File::openRead("input"), $firstSource));
    $owner->second[0]->register(new BorrowingReady(File::openRead("input"), $secondSource));
    emptySets($owner->first);
    $firstSource->add(3);
    $secondSource->add(4);
}}
function main(): void {{}}
"#
    );
    let analysis = analyze(&source);
    let mutation = source.find("$secondSource->add(4)").unwrap() + IMPORTS.len() + 1;
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .map(|d| (d.code, d.span.start))
            .collect::<Vec<_>>(),
        vec![("E0763", mutation)],
        "{source}\n{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn removal_helpers_release_only_the_selected_members_sources() {
    for removal in [
        "release(recover($set, $firstKey));",
        "release(relay($set, $firstKey));",
        "discard($set, $firstKey);",
    ] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function release(take BorrowingReady $stream): void {{}}
function recover(writable ReadinessSet $set, RegistrationKey<BorrowingReady> $key): BorrowingReady throws IoError {{
    return $set->unregister($key);
}}
function relay(writable ReadinessSet $set, RegistrationKey<BorrowingReady> $key): BorrowingReady throws IoError {{
    return recover($set, $key);
}}
function discard(writable ReadinessSet $set, RegistrationKey<BorrowingReady> $key): void throws IoError {{
    release(recover($set, $key));
}}
function exercise(): void throws IoError {{
    writable List<int> $first = [1];
    writable List<int> $second = [2];
    writable ReadinessSet $set = new ReadinessSet();
    let $firstKey = $set->register(new BorrowingReady(File::openRead("input"), $first));
    let $secondKey = $set->register(new BorrowingReady(File::openRead("input"), $second));
    {removal}
    $first->add(3);
    release($set->unregister($secondKey));
    $second->add(4);
}}
function main(): void {{}}
"#
        );
        assert_valid(&analyze(&source));
        let source = source.replace("$first->add(3);", "$first->add(3); $second->add(3);");
        let analysis = analyze(&source);
        let mutation = source.find("$second->add(3)").unwrap() + IMPORTS.len() + 1;
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "E0763")
                .map(|diagnostic| diagnostic.span.start)
                .collect::<Vec<_>>(),
            vec![mutation],
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn optional_and_failed_helper_removals_keep_the_members_sources_live() {
    for body in [
        "if ($remove) { release($set->unregister($key)); }",
        "try { release($set->unregister($key)); } catch (IoError) {}",
    ] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function release(take BorrowingReady $stream): void {{}}
function discard(writable ReadinessSet $set, RegistrationKey<BorrowingReady> $key, bool $remove): void throws IoError {{ {body} }}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $source));
    discard($set, $key, true);
    $source->add(2);
}}
function main(): void {{}}
"#
        );
        assert!(
            analyze(&source)
                .diagnostics
                .iter()
                .any(|d| d.code == "E0763"),
            "{source}"
        );
    }
}

#[test]
fn virtual_removal_requires_every_possible_body_to_remove_the_member() {
    for base_removes in [true, false] {
        let body = if base_removes {
            "release($set->unregister($key));"
        } else {
            ""
        };
        let source = format!(
            r#"{BORROWING_ADAPTER}
function release(take BorrowingReady $stream): void {{}}
open class RootRemover {{
    open function discard(writable ReadinessSet $set, RegistrationKey<BorrowingReady> $key): void throws IoError {{ {body} }}
}}
class ChildRemover extends RootRemover {{
    override function discard(writable ReadinessSet $set, RegistrationKey<BorrowingReady> $key): void throws IoError {{ release($set->unregister($key)); }}
}}
function exercise(RootRemover $remover): void throws IoError {{
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $source));
    $remover->discard($set, $key);
    $source->add(2);
}}
function main(): void {{}}
"#
        );
        let analysis = analyze(&source);
        if base_removes {
            assert_valid(&analysis);
        } else {
            assert!(
                analysis.diagnostics.iter().any(|d| d.code == "E0763"),
                "{:#?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn helper_registration_then_removal_preserves_the_new_members_sources() {
    for returned in ["create($set, $source)", "relay($set, $source)"] {
        let source = format!(
            r#"{BORROWING_ADAPTER}
function create(writable ReadinessSet $set, List<int> $source): BorrowingReady throws IoError {{
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $source));
    return $set->unregister($key);
}}
function relay(writable ReadinessSet $set, List<int> $source): BorrowingReady throws IoError {{ return create($set, $source); }}
function exercise(): void throws IoError {{
    writable List<int> $source = [1];
    writable ReadinessSet $set = new ReadinessSet();
    let $returned = {returned};
    $source->add(2);
    echo $returned->getCurrent();
}}
function main(): void {{}}
"#
        );
        let analysis = analyze(&source);
        let mutation = source.find("$source->add(2)").unwrap() + IMPORTS.len() + 1;
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|d| d.code == "E0763")
                .map(|d| d.span.start)
                .collect::<Vec<_>>(),
            vec![mutation],
            "{source}\n{:#?}\n{:#?}",
            analysis.diagnostics,
            analysis
                .info
                .retained_callables
                .iter()
                .filter(|(span, _)| span.source == doriac::source::SourceId(0))
                .collect::<Vec<_>>()
        );
        let source = source
            .replace("$source->add(2);", "release($returned); $source->add(2);")
            .replace("echo $returned->getCurrent();", "");
        let source = format!("{source}\nfunction release(take BorrowingReady $stream): void {{}}");
        assert_valid(&analyze(&source));
    }
}

#[test]
fn removing_through_nested_helpers_preserves_the_returned_members_source() {
    assert_retained_source_error(
        r#"
function recover(writable ReadinessSet $set, RegistrationKey<BorrowingReady> $key): BorrowingReady {
    return $set->unregister($key);
}
function create(List<int> $source): BorrowingReady {
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new BorrowingReady(File::openRead("input"), $source));
    return recover($set, $key);
}
function main(): void {
    writable List<int> $source = [1];
    let $returned = create($source);
    $source->add(2);
    echo $returned->getCurrent();
}
"#,
        "E0763",
    );
}
