#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/native_structure.rs"]
mod native_structure;

use doriac::backend::{BackendOutput, BackendTarget, NativeProfile};
use doriac::mir;

const SOURCE: &str = include_str!("../../../examples/native/main_read_transfer_plan.doria");
const EXPECTED: &str = include_str!("fixtures/native_io/main_read_transfer_plan/expected_stdout");
const WRITE_SOURCE: &str = include_str!("../../../examples/native/main_write_transfer_plan.doria");
const WRITE_EXPECTED: &str =
    include_str!("fixtures/native_io/main_write_transfer_plan/expected_stdout");

const COMPLETION_HELPERS: &str = r#"
function makeRead(ReadTransferKey $key, ?ReadOutcome $outcome): TransferWaitResult {
    return TransferWaitResult::Completed(TransferCompletion::Read($key, $outcome));
}
function makeWrite(WriteTransferKey $key, ?WriteOutcome $outcome): TransferWaitResult {
    return TransferWaitResult::Completed(TransferCompletion::Write($key, $outcome));
}
function copyResult(TransferWaitResult $result): TransferWaitResult {
    let $copy = $result;
    if ($copy != $result) { panic("result copy changed"); }
    return $result;
}
function verifyRead(TransferWaitResult $result, ReadTransferKey $expected, ?ReadOutcome $actual): bool {
    return match ($result) {
        TransferWaitResult::Completed($completion) => match ($completion) {
            TransferCompletion::Read($key, $outcome) => $key == $expected && $outcome == $actual,
            TransferCompletion::Write($key, $outcome) => false,
            TransferCompletion::Ready($key) => false,
        },
        TransferWaitResult::TimedOut => false,
        TransferWaitResult::Exhausted => false,
    };
}
function verifyWrite(TransferWaitResult $result, WriteTransferKey $expected, ?WriteOutcome $actual): bool {
    return match ($result) {
        TransferWaitResult::Completed($completion) => match ($completion) {
            TransferCompletion::Read($key, $outcome) => false,
            TransferCompletion::Write($key, $outcome) => $key == $expected && $outcome == $actual,
            TransferCompletion::Ready($key) => false,
        },
        TransferWaitResult::TimedOut => false,
        TransferWaitResult::Exhausted => false,
    };
}
function optional(?TransferWaitResult $result): ?TransferWaitResult { return $result; }
"#;

fn completion_source() -> String {
    format!(
        r#"use Doria\Std\Io\{{ReadTransferKey, WriteTransferKey, ReadOutcome, WriteOutcome,
    TransferCompletion, TransferWaitResult}};
{COMPLETION_HELPERS}
function main(): void {{}}"#
    )
}

#[test]
fn completion_and_wait_results_are_nominal_nested_inline_copy_values() {
    let program =
        doriac::lower_source_to_mir("completion-shape.doria", completion_source()).unwrap();
    for (name, cases) in [
        (doriac::compiler_known_stream::TRANSFER_COMPLETION, 3),
        (doriac::compiler_known_stream::TRANSFER_WAIT_RESULT, 3),
    ] {
        let definition = program
            .enums
            .iter()
            .find(|value| value.name == name)
            .unwrap();
        assert_eq!(definition.cases.len(), cases);
        assert!(definition.capabilities.copy && definition.capabilities.trivial_copy);
        assert!(!definition.capabilities.needs_drop);
        let payload = definition.payload_type().unwrap();
        for ty in [
            mir::Type::PayloadEnum(payload),
            mir::Type::NullablePayloadEnum(payload),
        ] {
            assert!(!ty.has_move_ownership());
            assert!(!doriac::native_closure_abi::needs_drop(ty));
        }
        for width in [4, 8] {
            let layout =
                doriac::native_closure_abi::type_layout(mir::Type::PayloadEnum(payload), width);
            assert_eq!(layout.size, definition.layout.size);
            assert_eq!(layout.align, definition.layout.align);
        }
    }
    let names = ["makeRead", "makeWrite", "optional"];
    let mut allowed = std::collections::BTreeSet::from([
        "memset".to_owned(),
        "memcpy".to_owned(),
        "llvm.memset.p0.i32".to_owned(),
        "llvm.memset.p0.i64".to_owned(),
        "llvm.memcpy.p0.p0.i32".to_owned(),
        "llvm.memcpy.p0.p0.i64".to_owned(),
    ]);
    for function in program
        .functions
        .iter()
        .filter(|function| names.contains(&function.name.as_str()))
    {
        for label in ["function_name", "source_path", "source_text"] {
            allowed.insert(format!("doria_{label}_{}", function.id.0));
        }
    }
    // These functions transport only inline metadata values, not buffer
    // addresses. Their fixed value copies and static diagnostics need no owner.
    let graph = native_structure::cranelift_references(&program);
    let references = native_structure::reachable_references(
        &graph,
        native_structure::authored_symbols(&program, &names),
    );
    assert!(references.is_subset(&allowed), "{references:?}");
    #[cfg(feature = "llvm-backend")]
    {
        let (_, graph) = native_structure::llvm_references(&program);
        let references = native_structure::reachable_references(
            &graph,
            native_structure::authored_symbols(&program, &names),
        );
        assert!(references.is_subset(&allowed), "{references:?}");
    }
}

#[test]
fn completion_construction_requires_exact_keys_outcomes_and_wait_cases() {
    let source = completion_source();
    for invalid in [
        source.replace(
            "ReadTransferKey $key, ?ReadOutcome",
            "WriteTransferKey $key, ?ReadOutcome",
        ),
        source.replace("?ReadOutcome $outcome", "?WriteOutcome $outcome"),
        source.replace(
            "TransferCompletion::Write($key, $outcome)",
            "TransferCompletion::Read($key, $outcome)",
        ),
        source.replace(
            "TransferWaitResult::Completed(TransferCompletion::Read($key, $outcome))",
            "TransferCompletion::Read($key, $outcome)",
        ),
        source.replace(
            "TransferWaitResult::TimedOut => false,",
            "TransferCompletion::TimedOut => false,",
        ),
        source.replace("TransferWaitResult::Exhausted => false,", ""),
        source.replace("TransferCompletion::Ready($key) => false,", ""),
        source.replace(
            "TransferCompletion::Ready($key) => false,",
            "TransferCompletion::Ready($key, $outcome) => false,",
        ),
    ] {
        assert!(
            doriac::check_source("invalid-completion.doria", &invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn completion_copy_and_matching_preserve_keys_nullable_outcomes_and_wait_states() {
    let plan_source = WRITE_SOURCE.replace("ReadTransferKey, WriteTransferKey};", "ReadTransferKey, WriteTransferKey, ReadOutcome, WriteOutcome, TransferCompletion, TransferWaitResult};");
    let mut source = format!("{plan_source}\n{COMPLETION_HELPERS}");
    let mut checks = String::new();
    for (key, maker, verifier, outcomes) in [
        (
            "$read",
            "makeRead",
            "verifyRead",
            &[
                "null",
                "ReadOutcome::Data(0)",
                "ReadOutcome::Data(7)",
                "ReadOutcome::WouldBlock",
                "ReadOutcome::EndOfStream",
                "ReadOutcome::TimedOut",
            ][..],
        ),
        (
            "$copied",
            "makeWrite",
            "verifyWrite",
            &[
                "null",
                "WriteOutcome::Accepted(0)",
                "WriteOutcome::Accepted(9)",
                "WriteOutcome::WouldBlock",
            ][..],
        ),
    ] {
        for outcome in outcomes {
            checks.push_str(&format!("if (!{verifier}(copyResult({maker}({key}, {outcome})), {key}, {outcome})) {{ panic(\"completion transport changed\"); }}\n"));
        }
    }
    checks.push_str(
        r#"
        if (verifyRead(TransferWaitResult::TimedOut, $read, null)
            || verifyRead(TransferWaitResult::Exhausted, $read, null)
            || verifyWrite(TransferWaitResult::TimedOut, $copied, null)
            || verifyWrite(TransferWaitResult::Exhausted, $copied, null)
            || copyResult(TransferWaitResult::TimedOut) == TransferWaitResult::Exhausted
            || optional(null) != null
            || optional(makeRead($read, null)) == null) { panic("wait state collapsed"); }
        echo "completion values preserved\n";
    "#,
    );
    source = source.replace(
        "echo \"typed keys survive appends\\n\";",
        &format!("{checks}\necho \"typed keys survive appends\\n\";"),
    );
    let program = doriac::lower_source_to_mir("completion-transport.doria", &source).unwrap();
    let expected = format!("completion values preserved\n{WRITE_EXPECTED}");
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, &expected);
    }
    let artifact =
        doriac::compile_source("completion-transport.doria", &source, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("completion.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

fn lower() -> mir::Program {
    doriac::lower_source_to_mir("read-plan.doria", SOURCE).unwrap()
}

#[test]
fn key_lookup_requires_exact_positions_and_directional_result_types() {
    for invalid in [
        WRITE_SOURCE.replace("getWriteKey(position: 0)", "getWriteKey(position: 0.0)"),
        WRITE_SOURCE.replace(
            "WriteTransferKey $copied = $plan->getWriteKey(0)",
            "ReadTransferKey $copied = $plan->getWriteKey(0)",
        ),
        WRITE_SOURCE.replace(
            "ReadTransferKey $read = $plan->getReadKey(2)",
            "WriteTransferKey $read = $plan->getReadKey(2)",
        ),
        WRITE_SOURCE.replace(
            "ReadTransferKey $read = $plan->getReadKey(2)",
            "int $read = $plan->getReadKey(2)",
        ),
        WRITE_SOURCE.replace("getWriteKey(position: 0)", "getWriteKey(index: 0)"),
    ] {
        assert!(
            doriac::check_source("invalid-key-lookup.doria", &invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn independent_validation_rejects_a_registration_key_as_a_transfer_lookup_result() {
    let mut program = doriac::lower_source_to_mir("key-lookup.doria", WRITE_SOURCE).unwrap();
    let registration = program
        .functions
        .iter()
        .flat_map(|function| &function.locals)
        .find_map(|local| match local.ty {
            mir::Type::Scalar(mir::ScalarType::Inline(
                key @ doriac::inline_value::InlineValueType::RegistrationKey(_),
            )) => Some(key),
            _ => None,
        })
        .unwrap();
    let mut altered = false;
    for function in &mut program.functions {
        for block in &mut function.blocks {
            if let mir::Terminator::CheckedIo {
                operation: mir::CheckedIoOperation::TransferPlanKey { key, .. },
                ..
            } = &mut block.terminator
            {
                *key = registration;
                altered = true;
            }
        }
    }
    assert!(altered);
    assert!(doriac::mir_validation::validate_program(&program).is_err());
    assert!(doriac::mir_interpreter::interpret(&program).is_err());
    assert!(doriac::codegen_cranelift::lower_mir_to_object(&program).is_err());
    #[cfg(feature = "llvm-backend")]
    assert!(doriac::codegen_llvm::lower_mir_to_object(&program).is_err());
}

#[test]
fn only_the_approved_read_and_write_spellings_are_available() {
    for (source, approved, old) in [
        (SOURCE, "andReadFrom", "andRead"),
        (SOURCE, "TransferPlan::read", "TransferPlan::readFrom"),
        (WRITE_SOURCE, "TransferPlan::writeTo", "TransferPlan::write"),
        (WRITE_SOURCE, "andWriteTo", "andWrite"),
    ] {
        assert!(source.contains(approved));
        assert!(
            doriac::check_source("old-plan-spelling.doria", source.replace(approved, old)).is_err()
        );
    }
}

#[test]
fn write_and_mixed_plans_retain_shared_sources_without_performing_io() {
    let program = doriac::lower_source_to_mir("write-plan.doria", WRITE_SOURCE).unwrap();
    let output = doriac::mir_interpreter::interpret_with_io(
        &program,
        doriac::mir_interpreter::MirIo::default(),
    )
    .unwrap();
    assert_eq!(output.output.stdout, WRITE_EXPECTED.as_bytes());
    assert!(output.output.stderr.is_empty());
    assert_eq!(output.output.exit_status, 0);
    assert_eq!((output.trace.file_reads, output.trace.file_writes), (0, 0));
    assert_eq!(output.files.get("input").unwrap(), b"unchanged\n");
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, WRITE_EXPECTED);
    }
    let artifact =
        doriac::compile_source("write-plan.doria", WRITE_SOURCE, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("write-plan.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, WRITE_EXPECTED.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read(directory.path().join("input")).unwrap(),
        b"unchanged\n"
    );
}

#[test]
fn write_plans_reject_source_mutation_escape_and_nonwritable_stream_capabilities() {
    for invalid in [
        WRITE_SOURCE.replace("echo \"shared", "$source[0] = 7; echo \"shared"),
        WRITE_SOURCE.replace(
            "File::openReadWrite(\"input\", FileCreation::OpenExisting, FileWriteMode::Positioned)",
            "File::openRead(\"input\")",
        ),
        WRITE_SOURCE.replace("function main(): void {", r#"
function escape<T implements PendingWritableStream>(RegistrationKey<T> $key): TransferPlan throws IoError {
    writable Bytes $local = Bytes::allocate(2);
    return TransferPlan::writeTo($key, $local->getRegion(0, 2));
}
function main(): void {"#),
    ] {
        assert!(doriac::check_source("invalid-write-plan.doria", &invalid).is_err(), "{invalid}");
    }
}

#[test]
fn independent_validation_rejects_forged_plan_direction() {
    for source in [SOURCE, WRITE_SOURCE] {
        let mut program = doriac::lower_source_to_mir("plan-direction.doria", source).unwrap();
        let mut altered = false;
        for function in &mut program.functions {
            for block in &mut function.blocks {
                if let mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::ConfigureTransferPlan { reading, .. },
                    ..
                } = &mut block.terminator
                {
                    *reading = !*reading;
                    altered = true;
                }
            }
        }
        assert!(altered);
        assert!(doriac::mir_validation::validate_program(&program).is_err());
        assert!(doriac::mir_interpreter::interpret(&program).is_err());
        assert!(doriac::codegen_cranelift::lower_mir_to_object(&program).is_err());
        #[cfg(feature = "llvm-backend")]
        assert!(doriac::codegen_llvm::lower_mir_to_object(&program).is_err());
    }
}

#[test]
fn chained_plans_forward_all_retained_destination_loans_through_generic_helpers() {
    let program = lower();
    doriac::mir_validation::validate_program(&program).unwrap();
    let plan = program
        .classes
        .iter()
        .find(|class| class.declaration_name == doriac::compiler_known_stream::TRANSFER_PLAN)
        .unwrap();
    assert_eq!(
        plan.runtime_payload,
        Some(mir::RuntimeClassPayload::TransferPlan)
    );
    for function in program.functions.iter().filter(|function| {
        function.name.starts_with("append") || function.name.starts_with("configure")
    }) {
        let retained = function
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
        assert!(!retained.returns.is_empty(), "{}", function.name);
        assert!(retained
            .returns
            .iter()
            .all(|source| source.lifetime == mir::RetainedLifetime::Cleanup));
    }
}

#[test]
fn unstarted_setup_and_consuming_failure_match_interpreter_and_both_native_profiles() {
    let program = lower();
    let interpreted = doriac::mir_interpreter::interpret_with_io(
        &program,
        doriac::mir_interpreter::MirIo::default(),
    )
    .unwrap();
    assert_eq!(interpreted.output.stdout, EXPECTED.as_bytes());
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.output.exit_status, 0);
    assert_eq!(interpreted.trace.file_reads, 0);
    assert_eq!(interpreted.trace.file_writes, 0);
    assert_eq!(interpreted.files.get("input").unwrap(), b"unread bytes\n");
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, EXPECTED);
    }
}

#[test]
fn php_libraries_retain_runtime_dependencies_for_plan_and_cleanup_storage() {
    for ty in [
        "TransferPlan",
        "?TransferPlan",
        "List<TransferPlan>",
        "ReadableFile",
        "ReadinessSet",
    ] {
        let source = format!(
            r#"
use Doria\Std\Io\{{TransferPlan, ReadableFile, ReadinessSet}};
function discard(take {ty} $value): void {{}}
"#
        );
        let artifact =
            doriac::compile_source("runtime-library.doria", source, BackendTarget::Php).unwrap();
        assert!(
            matches!(artifact, BackendOutput::TextWithRuntime { .. }),
            "{ty}"
        );
    }
    for source in [
        r#"use Doria\Std\Io\TransferPlan;
function forward(take TransferPlan $plan): TransferPlan { return $plan; }"#,
        r#"use Doria\Std\Io\TransferPlan;
class Holder {
    function __construct(take TransferPlan $plan) {}
}"#,
        r#"use Doria\Std\Io\ReadableFile;
enum Holder { case File(ReadableFile $file); }
function discard(take Holder $value): void {}"#,
    ] {
        let artifact =
            doriac::compile_source("plan-library.doria", source, BackendTarget::Php).unwrap();
        assert!(matches!(artifact, BackendOutput::TextWithRuntime { .. }));
    }
    for ty in ["int", "ReadTransferKey", "RegistrationKey<ReadableFile>"] {
        let source = format!(
            r#"
use Doria\Std\Io\{{ReadTransferKey, RegistrationKey, ReadableFile}};
function forward({ty} $value): {ty} {{ return $value; }}
"#
        );
        let artifact =
            doriac::compile_source("metadata-library.doria", source, BackendTarget::Php).unwrap();
        assert!(matches!(artifact, BackendOutput::Text { .. }), "{ty}");
    }
}

#[test]
fn php_setup_uses_the_shared_runtime_and_matches_checked_failure() {
    let php = doriac::compile_source("read-plan.doria", SOURCE, BackendTarget::Php).unwrap();
    let contents = match &php {
        BackendOutput::TextWithRuntime { contents, .. } => contents,
        _ => panic!("plan configuration needs the checked native runtime"),
    };
    assert!(contents.contains("dr_v1_host_transfer_plan_append"));
    let directory = native_execution::ExecutionDirectory::new();
    std::fs::write(directory.path().join("input"), b"unread bytes\n").unwrap();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, php).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), EXPECTED);
    assert!(output.stderr.is_empty());
}

#[test]
fn source_rejects_access_to_each_chained_destination_while_the_plan_is_live() {
    for buffer in ["first", "second", "empty"] {
        let invalid = SOURCE.replace(
            "echo \"configured without reading\\n\";",
            &format!("echo ${buffer}->length;"),
        );
        assert!(
            doriac::check_source("live-plan.doria", &invalid).is_err(),
            "{buffer}"
        );
    }
}

#[test]
fn setup_does_not_accept_a_readiness_only_key() {
    let invalid = SOURCE.replace(
        "RegistrationKey<T> $key",
        "RegistrationKey<Doria\\Std\\Io\\ReadinessStream> $key",
    );
    assert!(doriac::check_source("wrong-key.doria", &invalid).is_err());
}

#[test]
fn consumed_plan_cannot_be_used_again() {
    let invalid = SOURCE.replace(
        "panic(\"foreign key accepted\");",
        "let $again = forward($plan);",
    );
    assert!(doriac::check_source("consumed-plan.doria", &invalid).is_err());
}

#[test]
fn a_plan_cannot_escape_with_a_function_local_destination() {
    let invalid = SOURCE.replace(
        "function main(): void {",
        r#"
function escape(RegistrationKey<Doria\Std\Io\ReadableFile> $key): TransferPlan throws IoError {
    writable Bytes $local = Bytes::allocate(2);
    return TransferPlan::read($key, $local->getWritableRegion(0, 2));
}
function main(): void {"#,
    );
    assert!(doriac::check_source("escaped-plan.doria", &invalid).is_err());
}

#[test]
fn independent_validation_rejects_an_unowned_consumed_plan() {
    let mut program = lower();
    for function in &mut program.functions {
        for block in &function.blocks {
            if let mir::Terminator::CheckedIo {
                operation:
                    mir::CheckedIoOperation::ConfigureTransferPlan {
                        previous: Some(previous),
                        ..
                    },
                ..
            } = &block.terminator
            {
                function.locals[previous.0].owned = false;
            }
        }
    }
    assert!(doriac::mir_validation::validate_program(&program).is_err());
}

#[test]
fn independent_validation_rejects_a_readonly_destination() {
    let mut program = lower();
    for function in &mut program.functions {
        for block in &function.blocks {
            if let mir::Terminator::CheckedIo {
                operation: mir::CheckedIoOperation::ConfigureTransferPlan { region, .. },
                ..
            } = &block.terminator
            {
                function.locals[region.0].writable = false;
            }
        }
    }
    assert!(doriac::mir_validation::validate_program(&program).is_err());
}

#[test]
fn independent_validation_rejects_forged_plan_return_loans() {
    let mut program = lower();
    let mut cleared = false;
    for function in &mut program.functions {
        if function.blocks.iter().any(|block| {
            matches!(
                block.terminator,
                mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::ConfigureTransferPlan { .. },
                    ..
                }
            )
        }) {
            for block in &mut function.blocks {
                for statement in &mut block.statements {
                    if let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(
                        plan,
                    )) = statement
                    {
                        plan.returns.clear();
                        cleared = true;
                    }
                }
            }
        }
    }
    assert!(cleared);
    assert!(doriac::mir_validation::validate_program(&program).is_err());
}

#[test]
fn plan_loans_survive_an_indirect_creator() {
    let source = SOURCE.replace("let $plan = configure($firstKey, $first->getWritableRegion(0, 2));", r#"
        let $create = fn(RegistrationKey<Doria\Std\Io\ReadableFile> $key, writable WritableByteRegion $destination) => TransferPlan::read($key, $destination);
        let $plan = $create($firstKey, $first->getWritableRegion(0, 2));"#);
    let program = doriac::lower_source_to_mir("indirect-plan.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, EXPECTED.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, EXPECTED);
    }
    let invalid = source.replace(
        "let $replacement = $plan->andReadFrom",
        "$first[0] = 1; let $replacement = $plan->andReadFrom",
    );
    assert!(doriac::check_source("indirect-conflict.doria", &invalid).is_err());
}

#[test]
fn checked_failure_does_not_restore_the_consumed_receiver() {
    let source = SOURCE
        .replace(
            "try {\n        let $plan = configure($firstKey, $first->getWritableRegion(0, 2));",
            "let $plan = configure($firstKey, $first->getWritableRegion(0, 2));\n    try {",
        )
        .replace(
            "catch (IoError $error) {",
            "catch (IoError $error) { let $again = forward($plan);",
        );
    assert!(doriac::check_source("failed-consumption.doria", &source).is_err());
}

fn assert_configuration_references(
    program: &mir::Program,
    references: &native_structure::FunctionReferences,
) {
    let names = program
        .functions
        .iter()
        .filter(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::CheckedIo {
                        operation: mir::CheckedIoOperation::ConfigureTransferPlan { .. }
                            | mir::CheckedIoOperation::TransferPlanKey { .. },
                        ..
                    }
                )
            })
        })
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    let roots = native_structure::authored_symbols(program, &names);
    let calls = native_structure::reachable_references(references, roots);
    for required in [
        "dr_v1_checked_transfer_plan_new",
        "dr_v1_checked_transfer_plan_append",
    ] {
        assert!(calls.contains(required), "{calls:?}");
    }
    if program.functions.iter().any(|function| {
        function.blocks.iter().any(|block| {
            matches!(
                block.terminator,
                mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::TransferPlanKey { .. },
                    ..
                }
            )
        })
    }) {
        assert!(
            calls.contains("dr_v1_checked_transfer_plan_key"),
            "{calls:?}"
        );
    }
    for forbidden in [
        "dr_v1_checked_file_io",
        "dr_v1_pending_request_configure_file",
        doriac::native_abi::BYTES_COPY,
        doriac::native_abi::BYTES_FROM_COLLECTION,
        doriac::native_abi::BYTES_TO_COLLECTION,
        doriac::native_abi::SHARED_RETAIN,
        doriac::native_abi::WRITABLE_SHARED_RETAIN,
        doriac::native_abi::CLOSURE_ENVIRONMENT_ALLOCATE,
    ] {
        assert!(
            !calls.contains(forbidden),
            "configuration introduced {forbidden}: {calls:?}"
        );
    }
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("unresolved text target")),
        "{calls:?}"
    );
}

#[test]
fn cranelift_configuration_does_not_transfer_copy_or_share_buffers() {
    for source in [SOURCE, WRITE_SOURCE] {
        let program = doriac::lower_source_to_mir("plan-structure.doria", source).unwrap();
        assert_configuration_references(
            &program,
            &native_structure::cranelift_references(&program),
        );
    }
}

#[test]
#[cfg(feature = "llvm-backend")]
fn llvm_configuration_does_not_transfer_copy_or_share_buffers() {
    for source in [SOURCE, WRITE_SOURCE] {
        let program = doriac::lower_source_to_mir("plan-structure.doria", source).unwrap();
        let (_, references) = native_structure::llvm_references(&program);
        assert_configuration_references(&program, &references);
    }
}
