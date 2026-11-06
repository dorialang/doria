#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;
#[path = "common/native_structure.rs"]
mod native_structure;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::mir;

const IMPORTS: &str = r#"
use Doria\Std\Io\{File, ReadinessSet, TransferPlan, TransferGroup, TransferCompletion,
    TransferWaitResult, ReadOutcome, Wait, IoError, IoOperation, IoTarget, IoErrorReason};
use Doria\Std\Time\{Deadline, Duration, ClockError};
use Doria\Std\Concurrency\{CancellationSource, CancellationToken, CancellationError};
function start(writable ReadinessSet $set, take TransferPlan $plan,
    ?Deadline $deadline = null, ?CancellationToken $cancellation = null): TransferGroup throws IoError {
    return $set->begin($plan, deadline: $deadline, cancellation: $cancellation);
}
function detached(): CancellationToken {
    let $source = new CancellationSource();
    return $source->token;
}
"#;

fn source(body: &str) -> String {
    format!("{IMPORTS}\nfunction main(): void throws IoError, ClockError, CancellationError {{ {body} }}")
}

fn assert_parity(source: &str, expected: &str) {
    let program = doriac::lower_source_to_mir("group-controls.doria", source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 0, "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
    let output =
        doriac::compile_source("group-controls.doria", source, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("group.php");
    doriac::backend::publish_output(&path, output).unwrap();
    let result = std::process::Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for group-control parity");
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert_eq!(result.stdout, expected.as_bytes(), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
}

#[test]
fn stopped_controls_leave_bytes_untouched_and_do_not_follow_set_reuse() {
    for (setup, control) in [
        (
            "let writable $source = new CancellationSource(); $source->cancel();",
            "cancellation: $source->token",
        ),
        (
            "let $past = Deadline::fromNow(Duration::fromNanoseconds(-9223372036854775807));",
            "deadline: $past",
        ),
    ] {
        let source = source(&format!(
            r#"
    write_file("input", "ab");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    $buffer[0] = 99;
    {setup}
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan, {control});
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, null))
            || $group->getReadRegion($key) != null
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("stopped transfer fabricated data"); }}
        try {{ $group->resubmitRead($key, 0, 1); panic("stopped group accepted resubmission"); }}
        catch (IoError $error) {{
            if ($error->operation != IoOperation::Configure || $error->target != IoTarget::TransferGroup
                || $error->reason != IoErrorReason::InvalidInput) {{ panic("wrong stopped-group error"); }}
        }}
    }}
    if ($buffer[0] != 99) {{ panic("stopped transfer changed its buffer"); }}
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) {{ panic("set inherited old controls"); }}
    }}
    if ($buffer[0] != 97) {{ panic("set reuse skipped original input"); }}
    echo "stopped and reused\n";
"#
        ));
        assert_parity(&source, "stopped and reused\n");
    }
}

#[test]
fn optional_controls_copy_deadlines_and_retain_detached_temporary_tokens() {
    let source = source(
        r#"
    write_file("input", "abcd");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    writable ?Deadline $deadline = Deadline::fromNow(Duration::fromNanoseconds(9223372036854775807));
    {
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = start($set, $plan, cancellation: detached(), deadline: $deadline);
        $deadline = null;
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) { panic("group retained caller control storage"); }
        $group->resubmitRead($key, 0, 1);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) { panic("group lost detached cancellation state"); }
    }
    if ($buffer[0] != 98) { panic("controlled resubmission lost progress"); }
    {
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan, cancellation: null, deadline: null);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) { panic("explicit null omitted no control"); }
    }
    if ($buffer[0] != 99) { panic("explicit-null group lost progress"); }
    echo "copied and retained\n";
"#,
    );
    assert_parity(&source, "copied and retained\n");
}

#[test]
fn live_group_does_not_borrow_the_cancellation_source_or_token() {
    let source = source(
        r#"
    write_file("input", "a");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    let writable $source = new CancellationSource();
    writable ?CancellationToken $token = $source->token;
    {
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan, cancellation: $token);
        $token = null;
        $source->cancel();
        // A real transfer may already have completed when cancellation arrives.
        let $result = $group->getNextCompletion(Wait::Indefinite);
        if ($result != TransferWaitResult::Completed(TransferCompletion::Read($key, null))
            && $result != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) { panic("cancellation erased actual progress"); }
    }
    if (!$source->cancellationRequested) { panic("group detached cancellation identity"); }
    echo "call-only controls\n";
"#,
    );
    assert_parity(&source, "call-only controls\n");
}

#[test]
fn preparation_error_wins_a_racing_cancellation_and_preserves_its_progress() {
    let adapter = r#"
use Doria\Std\Io\{PendingReadableStream, ReadableFile, ReadinessSource, ReadRequest, WritableByteRegion};
class CancellingReader implements PendingReadableStream {
    internal writable ReadableFile $file;
    internal writable CancellationSource $source;
    function __construct(parameter take ReadableFile $file) {
        $this->file = $file;
        $this->source = new CancellationSource();
    }
    CancellationToken $token { get { return $this->source->token; } }
    ReadinessSource $readinessSource { borrowed get { return $this->file->readinessSource; } }
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError {
        return $this->file->read($destination);
    }
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError {
        try { $this->source->cancel(); } catch (CancellationError) {}
        $destination[0] = 33;
        throw new IoError("controlled original error", IoOperation::Read, IoTarget::File("adapter"),
            IoErrorReason::Unsupported, 37, 1);
    }
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError {
        return $this->file->completeRead($request, $destination);
    }
}
"#;
    let source = format!(
        "{}\n{adapter}",
        source(
            r#"
    write_file("input", "a");
    let writable $set = new ReadinessSet();
    let $reader = new CancellingReader(File::openRead("input"));
    let $token = $reader->token;
    let $registration = $set->register($reader);
    writable Bytes $buffer = Bytes::allocate(1);
    try {
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $group = $set->begin($plan, cancellation: $token);
        panic("original error lost to cancellation");
    } catch (IoError $error) {
        if ($error->message != "controlled original error" || $error->operation != IoOperation::Read
            || $error->target != IoTarget::File("adapter") || $error->reason != IoErrorReason::Unsupported
            || ($error->systemCode ?? -1) != 37 || ($error->transferredByteCount ?? -1) != 1) { panic("original error was reconstructed"); }
    }
    if ($buffer[0] != 33) { panic("preparation progress was discarded"); }
    let writable $returned = $set->unregister($registration);
    if ($returned->read($buffer->getWritableRegion(0, 1)) != ReadOutcome::Data(1) || $buffer[0] != 97) { panic("failed setup leaked a loan or submitted input"); }
    echo "original error won\n";
"#
        )
    );
    assert_parity(&source, "original error won\n");
}

#[test]
fn native_control_transport_uses_the_shared_bridge_without_caller_state_allocation() {
    let program = doriac::lower_source_to_mir("group-control-structure.doria", source("")).unwrap();
    let roots = native_structure::authored_symbols(&program, &["start"]);
    let check = |references: std::collections::BTreeSet<String>| {
        assert!(
            references.contains("dr_v2_checked_transfer_group_begin"),
            "{references:?}"
        );
        assert!(!references.contains("dr_v1_checked_transfer_group_begin"));
        assert!(
            references
                .iter()
                .all(|symbol| !symbol.contains("cancellation") && !symbol.contains("clock")),
            "{references:?}"
        );
    };
    let fast = native_structure::cranelift_references(&program);
    check(native_structure::reachable_references(&fast, roots.clone()));
    #[cfg(feature = "llvm-backend")]
    {
        let (_, release) = native_structure::llvm_references(&program);
        check(native_structure::reachable_references(&release, roots));
    }
}

#[test]
fn mir_retains_only_set_and_plan_loans_and_validates_exact_control_types() {
    let program = doriac::lower_source_to_mir("group-control-mir.doria", source("")).unwrap();
    let function = program
        .functions
        .iter()
        .find(|function| function.name == "start")
        .unwrap();
    let returns = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                Some(&plan.returns)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(returns.len(), 2);
    assert!(returns
        .iter()
        .all(|source| function.params[..2].contains(&source.local)));
    let intrinsic = program
        .functions
        .iter()
        .find(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::CheckedIo {
                        operation: mir::CheckedIoOperation::BeginTransferGroup { .. },
                        ..
                    }
                )
            })
        })
        .unwrap();
    let (deadline, cancellation) = intrinsic
        .blocks
        .iter()
        .find_map(|block| match block.terminator {
            mir::Terminator::CheckedIo {
                operation:
                    mir::CheckedIoOperation::BeginTransferGroup {
                        deadline,
                        cancellation,
                        ..
                    },
                ..
            } => Some((deadline, cancellation)),
            _ => None,
        })
        .unwrap();
    for local in [deadline, cancellation] {
        let mut forged = program.clone();
        forged.functions[intrinsic.id.0].locals[local.0].ty =
            mir::Type::Scalar(mir::ScalarType::Bool);
        assert!(doriac::mir_validation::validate_program(&forged).is_err());
    }
}
