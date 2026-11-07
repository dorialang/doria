#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;
#[path = "common/native_structure.rs"]
mod native_structure;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::mir::Type;

const OUTCOME_SOURCE: &str = r#"
use Doria\Std\Io\{ReadOutcome, WriteOutcome, FlushOutcome};
function data(int $byteCount): ReadOutcome { return ReadOutcome::Data($byteCount); }
function accepted(int $byteCount): WriteOutcome { return WriteOutcome::Accepted($byteCount); }
function retry(): ReadOutcome { return ReadOutcome::WouldBlock; }
function reuse(ReadOutcome $outcome): ReadOutcome { return $outcome; }
function flushed(): FlushOutcome { return FlushOutcome::Complete; }
function pendingFlush(): FlushOutcome { return FlushOutcome::Pending; }
function reuseFlush(FlushOutcome $outcome): FlushOutcome { return $outcome; }
function main(): void {
    let $outcome = reuse(data(0)); let $written = accepted(0); let $retry = retry();
    let $complete = reuseFlush(flushed()); let $pending = reuseFlush(pendingFlush());
}
"#;

const SOURCE: &str = include_str!("../../../examples/native/main_stream_capabilities.doria");
const EXPECTED: &str = include_str!("fixtures/native_io/main_stream_capabilities/expected_stdout");

const CONDITIONAL_FORWARDER: &str = include_str!("fixtures/conditional_stream_forwarder.doria");

#[test]
fn conditional_stream_forwarders_preserve_each_nominal_capability_tier() {
    let capabilities = [
        "ReadableStream",
        "WritableStream",
        "DuplexStream",
        "FlushableStream",
        "ReadinessStream",
        "PendingReadableStream",
        "PendingWritableStream",
    ];
    let sources = [
        ("SyncRead", [true, false, false, false, false, false, false]),
        (
            "SyncWrite",
            [false, true, false, false, false, false, false],
        ),
        ("SyncDuplex", [true, true, true, true, false, false, false]),
        (
            "StandardInput",
            [true, false, false, false, true, true, false],
        ),
        (
            "StandardOutput",
            [false, true, false, false, true, false, true],
        ),
        ("DuplexFile", [true, true, true, false, true, true, true]),
        (
            "ReadableStream",
            [true, false, false, false, false, false, false],
        ),
        (
            "PendingReadableStream",
            [true, false, false, false, true, true, false],
        ),
        (
            "PendingWritableStream",
            [false, true, false, false, true, false, true],
        ),
        ("Lookalike", [false; 7]),
    ];
    for (source_type, expected) in sources {
        for (capability, available) in capabilities.into_iter().zip(expected) {
            let source = format!(
                "{CONDITIONAL_FORWARDER}\nfunction requireCapability(writable {capability} $stream): void {{}}\n\
                function inspect(writable Forward<Forward<{source_type}>> $stream): void {{ requireCapability($stream); }}"
            );
            let (_, analysis) =
                doriac::analyze_source_for_ide("conditional-stream-tiers.doria", &source).unwrap();
            if available {
                assert!(
                    analysis.diagnostics.is_empty(),
                    "{source_type} -> {capability}: {:?}",
                    analysis.diagnostics
                );
            } else {
                assert_eq!(
                    analysis.diagnostics.len(),
                    1,
                    "{source_type} -> {capability}: {:?}",
                    analysis.diagnostics
                );
                let diagnostic = &analysis.diagnostics[0];
                assert_eq!(diagnostic.code, "E0408", "{diagnostic:?}");
                assert_eq!(
                    &source[diagnostic.span.start..diagnostic.span.end],
                    "$stream"
                );
            }
        }
    }
}

#[test]
fn conditional_stream_forwarders_execute_nested_concrete_and_erased_transfers() {
    use doriac::mir::{
        EnumExpression, IndirectCallee, PayloadEnumExpression, Rvalue, Statement, Terminator,
        ValueExpression,
    };

    let source = format!(
        r#"{CONDITIONAL_FORWARDER}
function readGeneric<T implements ReadableStream>(writable T $stream, writable WritableByteRegion $destination): ReadOutcome {{
    return $stream->read($destination);
}}
function writeErased(writable WritableStream $stream, ByteRegion $source): WriteOutcome {{ return $stream->write($source); }}
function flushErased(writable FlushableStream $stream): FlushOutcome {{ return $stream->flush(); }}
function main(): void {{
    let writable $stream = new Forward<Forward<SyncDuplex>>(new Forward<SyncDuplex>(new SyncDuplex()));
    writable Bytes $bytes = Bytes::allocate(1);
    {{
        let writable $destination = $bytes->getWritableRegion(0, 1);
        echo match (readGeneric($stream, $destination)) {{ ReadOutcome::Data($count) => $count, default => -1 }};
    }}
    {{
        let $source = $bytes->getRegion(0, 1);
        echo ":" . $source[0] . ":";
        echo match (writeErased($stream, $source)) {{ WriteOutcome::Accepted($count) => $count, default => -1 }};
    }}
    echo match (flushErased($stream)) {{ FlushOutcome::Pending => ":pending", default => ":wrong" }};
    echo match ($stream->flush()) {{ FlushOutcome::Complete => ":complete\n", default => ":wrong\n" }};
}}
"#
    );
    let expected = "1:42:1:pending:complete\n";
    let program = lower(&source);
    doriac::mir_validation::validate_program(&program).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected);

    let concrete = program
        .functions
        .iter()
        .filter(|function| {
            function.name.starts_with("readGeneric")
                || function.method.as_ref().is_some_and(|method| {
                    matches!(method.name.as_str(), "read" | "write" | "flush")
                        && program.classes[method.class.0].name.starts_with("Forward")
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        concrete.len(),
        7,
        "three methods in two layers and one generic caller"
    );
    for function in &concrete {
        assert!(
            function.blocks.iter().all(|block| !matches!(
                block.terminator,
                Terminator::IndirectCall { .. } | Terminator::CheckedIndirectCall { .. }
            )),
            "conditional concrete forwarding must stay static: {}",
            function.name
        );
        // Infallible specialized targets return directly through typed expressions.
        let outcome_call = |value: &Rvalue| match value {
            Rvalue::PayloadEnum(PayloadEnumExpression::Call { function, .. })
            | Rvalue::Value(ValueExpression::Enum(EnumExpression::Call { function, .. })) => {
                Some(*function)
            }
            _ => None,
        };
        let calls = function
            .blocks
            .iter()
            .flat_map(|block| {
                block
                    .statements
                    .iter()
                    .filter_map(|statement| match statement {
                        Statement::AssignLocal { value, .. } => outcome_call(value),
                        _ => None,
                    })
                    .chain(match &block.terminator {
                        Terminator::CheckedCall { function, .. } => Some(*function),
                        Terminator::Return(value) => outcome_call(value),
                        _ => None,
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            calls.len(),
            1,
            "one forwarded call in {}: {function:?}",
            function.name
        );
        let expected_method = function
            .method
            .as_ref()
            .map_or("read", |method| method.name.as_str());
        assert_eq!(
            program.functions[calls[0].0].method.as_ref().unwrap().name,
            expected_method
        );
    }
    for name in ["writeErased", "flushErased"] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let indirect = function
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator {
                Terminator::IndirectCall { callee, .. }
                | Terminator::CheckedIndirectCall { callee, .. } => Some(callee),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(indirect.len(), 1, "one erased boundary in {name}");
        assert!(matches!(
            indirect[0],
            IndirectCallee::InterfaceMethod { .. }
        ));
    }
    let names = concrete
        .iter()
        .map(|function| function.name.as_str())
        .chain(["writeErased", "flushErased"])
        .collect::<Vec<_>>();
    assert_cranelift_functions_do_not_allocate(&program, &names);
    #[cfg(feature = "llvm-backend")]
    assert_llvm_functions_do_not_allocate(&program, &names);
}

#[test]
fn conditional_stream_forwarders_execute_pending_readiness_and_retirement() {
    let source = format!(
        r#"{CONDITIONAL_FORWARDER}
use Doria\Std\Io\{{File, ReadableFile, ReadinessSet, TransferPlan, TransferWaitResult, TransferCompletion, Wait}};
function main(): void {{
    write_file("conditional-input", "abc");
    let writable $adapter = new Forward<Forward<ReadableFile>>(new Forward<ReadableFile>(File::openRead("conditional-input")));
    {{
        let $source = $adapter->readinessSource;
        if (!($source is ReadinessSource)) {{ panic("conditional readiness source lost"); }}
    }}
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register($adapter);
    writable Bytes $bytes = Bytes::allocate(2);
    {{
        let $plan = TransferPlan::read($registration, $bytes->getWritableRegion(0, 2));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(2)))) {{
            panic("conditional preparation or completion lost");
        }}
        {{
            let $region = $group->getReadRegion($key);
            if ($region == null || $region->length != 2 || $region[0] != 97 || $region[1] != 98) {{ panic("pending bytes changed"); }}
        }}
        $group->resubmitRead($key, 0, 1);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) {{
            panic("conditional resubmission lost");
        }}
        {{
            let $region = $group->getReadRegion($key);
            if ($region == null || $region->length != 1 || $region[0] != 99) {{ panic("resubmitted bytes changed"); }}
        }}
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("completion repeated"); }}
    }}
    $bytes[0] = 9;
    let writable $returned = $set->unregister($registration);
    if ($returned->read($bytes->getWritableRegion(0, 1)) != ReadOutcome::EndOfStream) {{ panic("owner or position changed"); }}
    echo "conditional pending ownership preserved\n";
}}
"#
    );
    let expected = "conditional pending ownership preserved\n";
    let program = lower(&source);
    let observed =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(observed.output.stdout, expected.as_bytes());
    assert!(observed.output.stderr.is_empty(), "{:?}", observed.output);
    assert_eq!(observed.output.exit_status, 0);
    assert_eq!(
        observed.trace.file_opens,
        observed.trace.file_close_attempts
    );
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected);
}

#[test]
fn conditional_stream_forwarders_execute_pending_writes_and_cancellation() {
    let source = format!(
        r#"{CONDITIONAL_FORWARDER}
use Doria\Std\Io\{{File, WritableFile, FileCreation, FileWriteMode, ReadinessSet, TransferPlan,
    TransferWaitResult, TransferCompletion, Wait, IoOperation, IoTarget, IoErrorReason}};
use Doria\Std\Concurrency\{{CancellationSource, CancellationToken, CancellationError}};
class CancellingWriter implements PendingWritableStream {{
    internal writable WritableFile $file;
    internal writable CancellationSource $cancellation;
    static writable int $prepares = 0;
    static writable int $completes = 0;
    function __construct(parameter take WritableFile $file) {{
        $this->file = $file;
        $this->cancellation = new CancellationSource();
    }}
    CancellationToken $token {{ get => $this->cancellation->token; }}
    ReadinessSource $readinessSource {{ borrowed get => $this->file->readinessSource; }}
    writable function write(ByteRegion $source): WriteOutcome throws IoError {{ return $this->file->write($source); }}
    writable function prepareWrite(writable WriteRequest $request, ByteRegion $source): void throws IoError {{
        self::prepares += 1;
        $this->file->prepareWrite($request, $source);
        try {{ $this->cancellation->cancel(); }} catch (CancellationError) {{}}
    }}
    writable function completeWrite(WriteRequest $request, ByteRegion $source): ?WriteOutcome throws IoError {{
        self::completes += 1;
        return $this->file->completeWrite($request, $source);
    }}
}}
function main(): void throws CancellationError {{
    let $writer = new CancellingWriter(File::openWrite("conditional-output", FileCreation::CreateOrTruncate, FileWriteMode::Positioned));
    let $token = $writer->token;
    let $adapter = new Forward<Forward<CancellingWriter>>(new Forward<CancellingWriter>($writer));
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register($adapter);
    writable Bytes $bytes = Bytes::allocate(2);
    $bytes[0] = 97;
    $bytes[1] = 98;
    let writable $stopped = new CancellationSource();
    $stopped->cancel();
    {{
        let $plan = TransferPlan::writeTo($registration, $bytes->getRegion(0, 2));
        let $key = $plan->getWriteKey(0);
        let writable $group = $set->begin($plan, cancellation: $stopped->token);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($key, null))) {{ panic("pre-cancelled write fabricated progress"); }}
    }}
    if (CancellingWriter::prepares != 0 || CancellingWriter::completes != 0 || read_file("conditional-output") != "") {{ panic("pre-cancelled write executed"); }}
    {{
        let $plan = TransferPlan::writeTo($registration, $bytes->getRegion(0, 2));
        let $key = $plan->getWriteKey(0);
        let writable $group = $set->begin($plan, cancellation: $token);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($key, null))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("cancelled selection fabricated progress"); }}
        try {{ $group->resubmitWrite($key, 0, 1); panic("cancelled group accepted resubmission"); }}
        catch (IoError $error) {{
            if ($error->operation != IoOperation::Configure || $error->target != IoTarget::TransferGroup
                || $error->reason != IoErrorReason::InvalidInput) {{ panic("wrong cancelled resubmission error"); }}
        }}
    }}
    if (CancellingWriter::prepares != 1 || CancellingWriter::completes != 1 || read_file("conditional-output") != "") {{ panic("cancelled preparation did not retire exactly once"); }}
    {{
        let $plan = TransferPlan::writeTo($registration, $bytes->getRegion(0, 2));
        let $key = $plan->getWriteKey(0);
        let writable $group = $set->begin($plan);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($key, WriteOutcome::Accepted(2)))) {{ panic("new group inherited cancellation"); }}
        $group->resubmitWrite($key, 1, 1);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($key, WriteOutcome::Accepted(1)))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("pending write resubmission changed"); }}
    }}
    if (CancellingWriter::prepares != 3 || CancellingWriter::completes != 3) {{ panic("write callbacks were skipped or repeated"); }}
    $bytes[0] = 99;
    let writable $returned = $set->unregister($registration);
    if ($returned->write($bytes->getRegion(0, 1)) != WriteOutcome::Accepted(1)
        || read_file("conditional-output") != "abbc") {{ panic("write owner, position or buffer loan changed"); }}
    echo "conditional write cancellation preserved\n";
}}
"#
    );
    let expected = "conditional write cancellation preserved\n";
    let program = lower(&source);
    let observed =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(observed.output.exit_status, 0, "{:?}", observed.output);
    assert_eq!(observed.output.stdout, expected.as_bytes());
    assert!(observed.output.stderr.is_empty());
    assert_eq!(observed.files["conditional-output"], b"abbc");
    assert_eq!(
        observed.trace.file_opens,
        observed.trace.file_close_attempts
    );
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(&source, expected);
    assert_eq!(
        std::fs::read(directory.path().join("conditional-output")).unwrap(),
        b"abbc"
    );
}

#[test]
fn conditional_stream_forwarders_retire_prepared_writes_after_setup_failure() {
    let source = format!(
        r#"{CONDITIONAL_FORWARDER}
use Doria\Std\Io\{{File, WritableFile, FileCreation, FileWriteMode, ReadinessSet, TransferPlan,
    IoOperation, IoTarget, IoErrorReason}};
class SetupWriter implements PendingWritableStream {{
    internal writable WritableFile $file;
    internal bool $fail;
    static writable int $prepares = 0;
    static writable int $completes = 0;
    static writable int $drops = 0;
    function __construct(parameter take WritableFile $file, parameter bool $fail) {{
        $this->file = $file;
        $this->fail = $fail;
    }}
    ReadinessSource $readinessSource {{ borrowed get => $this->file->readinessSource; }}
    writable function write(ByteRegion $source): WriteOutcome throws IoError {{ return $this->file->write($source); }}
    writable function prepareWrite(writable WriteRequest $request, ByteRegion $source): void throws IoError {{
        self::prepares += 1;
        if ($this->fail) {{
            throw new IoError("conditional preparation error", IoOperation::Write, IoTarget::File("adapter"),
                IoErrorReason::Unsupported, 37, 2);
        }}
        $this->file->prepareWrite($request, $source);
    }}
    writable function completeWrite(WriteRequest $request, ByteRegion $source): ?WriteOutcome throws IoError {{
        self::completes += 1;
        if ($this->fail) {{ panic("throwing preparation received completion"); }}
        if ($request->outcome != null) {{ panic("failed setup submitted a write"); }}
        return $this->file->completeWrite($request, $source);
    }}
    function __destruct() {{ self::drops++; }}
}}
function main(): void {{
    writable ReadinessSet $set = new ReadinessSet();
    let $first = $set->register(new Forward<Forward<SetupWriter>>(new Forward<SetupWriter>(
        new SetupWriter(File::openWrite("conditional-first", FileCreation::CreateOrTruncate, FileWriteMode::Positioned), false))));
    let $second = $set->register(new Forward<Forward<SetupWriter>>(new Forward<SetupWriter>(
        new SetupWriter(File::openWrite("conditional-second", FileCreation::CreateOrTruncate, FileWriteMode::Positioned), true))));
    writable Bytes $a = Bytes::allocate(1);
    writable Bytes $b = Bytes::allocate(1);
    $a[0] = 97;
    $b[0] = 98;
    try {{
        let $plan = TransferPlan::writeTo($first, $a->getRegion(0, 1))
            ->andWriteTo($second, $b->getRegion(0, 1));
        let $group = $set->begin($plan);
        panic("throwing preparation succeeded");
    }} catch (IoError $error) {{
        if ($error->message != "conditional preparation error" || $error->operation != IoOperation::Write
            || $error->target != IoTarget::File("adapter") || $error->reason != IoErrorReason::Unsupported
            || ($error->systemCode ?? 0) != 37 || ($error->transferredByteCount ?? 0) != 2) {{
            panic("conditional forwarding replaced the original error");
        }}
    }}
    if (SetupWriter::prepares != 2 || SetupWriter::completes != 1 || SetupWriter::drops != 0
        || read_file("conditional-first") != "" || read_file("conditional-second") != "") {{
        panic("setup cleanup changed callbacks, owners or files");
    }}
    $a[0] = 99;
    $b[0] = 100;
    {{
        let writable $returned = $set->unregister($first);
        if ($returned->write($a->getRegion(0, 1)) != WriteOutcome::Accepted(1)) {{ panic("first owner not reusable"); }}
    }}
    {{
        let writable $returned = $set->unregister($second);
        if ($returned->write($b->getRegion(0, 1)) != WriteOutcome::Accepted(1)) {{ panic("second owner not reusable"); }}
    }}
    if (SetupWriter::prepares != 2 || SetupWriter::completes != 1 || SetupWriter::drops != 2
        || read_file("conditional-first") != "c" || read_file("conditional-second") != "d") {{
        panic("setup failure retained loans, advanced positions or repeated cleanup");
    }}
    echo "conditional setup cleanup preserved\n";
}}
"#
    );
    let expected = "conditional setup cleanup preserved\n";
    let program = lower(&source);
    let observed =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(observed.output.exit_status, 0, "{:?}", observed.output);
    assert_eq!(observed.output.stdout, expected.as_bytes());
    assert!(observed.output.stderr.is_empty());
    assert_eq!(observed.files["conditional-first"], b"c");
    assert_eq!(observed.files["conditional-second"], b"d");
    assert_eq!(
        observed.trace.file_opens,
        observed.trace.file_close_attempts
    );
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(&source, expected);
    for (name, contents) in [("conditional-first", b"c"), ("conditional-second", b"d")] {
        assert_eq!(
            std::fs::read(directory.path().join(name)).unwrap(),
            contents
        );
    }
}

const FINISH_OUTCOMES: &str = include_str!("fixtures/finish_outcomes.doria");

#[test]
fn finish_outcomes_specialize_ownership_cleanup_and_copy_per_application() {
    let expected = "complete1;pending2;retry3;copy4;1:3\n";
    let program = lower(FINISH_OUTCOMES);
    doriac::mir_validation::validate_program(&program).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(FINISH_OUTCOMES, expected);

    // One specialization per application; Copy only when every payload is Copy.
    let applications = |declaration: &str| {
        program
            .enums
            .iter()
            .filter(|definition| {
                definition
                    .name
                    .strip_prefix(declaration)
                    .is_some_and(|rest| rest.starts_with('<'))
            })
            .collect::<Vec<_>>()
    };
    let finish = applications("Doria\\Std\\Io\\FinishOutcome");
    assert_eq!(finish.len(), 2, "{finish:#?}");
    let (copy, owned): (Vec<_>, Vec<_>) = finish
        .into_iter()
        .partition(|definition| definition.capabilities.copy);
    assert_eq!((copy.len(), owned.len()), (1, 1));
    assert!(!copy[0].capabilities.needs_drop);
    assert!(owned[0].capabilities.needs_drop);
    let borrowed = applications("Doria\\Std\\Io\\BorrowedFinishOutcome");
    assert_eq!(borrowed.len(), 1, "{borrowed:#?}");
    assert!(!borrowed[0].capabilities.copy);
    assert!(borrowed[0].capabilities.needs_drop);
    assert_eq!(
        borrowed[0]
            .cases
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        ["Complete", "Pending"]
    );
}

const SCOPED_CLASS_BORROWING: &str = include_str!("fixtures/scoped_class_borrowing.doria");
const BORROWED_FINISH_TRANSPORT: &str = include_str!("fixtures/borrowed_finish_transport.doria");

fn assert_every_backend(source: &str, expected: &str) {
    let program = lower(source);
    doriac::mir_validation::validate_program(&program).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);
}

const BUFFERED_ADAPTERS: &str = include_str!("fixtures/buffered_adapters.doria");
const BUFFERED_PENDING_ADAPTERS: &str = include_str!("fixtures/buffered_pending_adapters.doria");
const FILE_LOCKS: &str = include_str!("fixtures/file_locks.doria");
const LINE_READERS: &str = include_str!("fixtures/line_readers.doria");

#[test]
fn line_readers_split_utf8_bound_lines_and_report_invalid_input_on_every_backend() {
    // Multibyte characters split across refills, CRLF, an empty line and an
    // unterminated last line; a bounded line reports TooLong without
    // consuming; invalid and incomplete UTF-8 keep exact stream offsets.
    assert_every_backend(
        LINE_READERS,
        "[h\u{e9}llo][wor\u{20ac}ld][][last]end;longlong;[ok]invalid UTF-8 in stream:3:1;6;1:incomplete;3\n",
    );
    let mut rendered = Vec::new();
    doria_diagnostic_catalogue::visit_invalid_utf8_message_parts(
        doria_diagnostic_catalogue::Utf8MessageSource::Stream,
        |part| rendered.extend_from_slice(part),
    );
    assert!(
        doriac::compiler_known_stream::SOURCE_TEXT
            .contains(&format!("\"{}\"", String::from_utf8(rendered).unwrap())),
        "the adapter's message must be the catalogue's stream rendering"
    );
}

#[test]
fn file_locks_contend_release_and_return_ownership_on_every_backend() {
    // Exclusive excludes shared; shared admits shared; explicit unlock and
    // guard destruction both release; a second acquisition through the same
    // file fails with IoOperation::Lock; the owning guard returns its file.
    assert_every_backend(FILE_LOCKS, "c0a;aca;Lca;oca\n");
}

#[test]
fn file_lock_guards_keep_their_file_and_stay_compiler_owned() {
    let imports = "use Doria\\Std\\Io\\{File, FileLockMode, FileLock, ReadableFile, IoError};\n";
    for (body, expected) in [
        ("function main(): void { let writable $f = File::openRead(\"x\"); let writable $g = $f->lock(FileLockMode::Shared); $f->close(); $g->unlock(); }", "E0763"),
        ("function escape(): FileLock<ReadableFile> throws IoError { let writable $f = File::openRead(\"x\"); return $f->lock(FileLockMode::Shared); } function main(): void {}", "E0762"),
        ("function main(): void { let writable $f = File::openRead(\"x\"); let $g = new FileLock<ReadableFile>($f); }", "E0307"),
        ("function main(): void { let writable $f = File::openRead(\"x\"); $f->acquireLock(FileLockMode::Shared); }", "E0307"),
        ("function lockIt(ReadableFile $f): void throws IoError { let $g = $f->lock(FileLockMode::Shared); } function main(): void {}", "E0203"),
    ] {
        let source = format!("{imports}{body}");
        let diagnostics = doriac::check_source("file-lock.doria", source.as_str())
            .expect_err("lock misuse must be rejected");
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code == expected),
            "{source}\n{diagnostics:#?}"
        );
    }
}

#[test]
fn buffered_adapters_preserve_bytes_partial_progress_and_finish_ownership() {
    // Short reads refill a bounded buffer; partial and would-block writes drain
    // through flush and a retried consuming finish; borrowed forms write
    // through to the caller's stream and release it.
    assert_every_backend(BUFFERED_ADAPTERS, "100;21816;2;20;1;10;5;3;5\n");
}

#[test]
fn buffered_adapters_forward_readiness_and_pending_tiers_on_every_backend() {
    // Decision log entry 44: buffered bytes complete pending reads at once and
    // an empty buffer passes transfers through in plan order; a writer fills
    // free space at once, drains a full buffer through one transfer and
    // rejects a second concurrent write on itself.
    assert_every_backend(
        BUFFERED_PENDING_ADAPTERS,
        "reader;borrowed reader;a buffered writer carries one pending write at a time; \
         plan one write per buffered writer and resubmit it after its completion;\
         writer;borrowed writer\n",
    );
    // The tiers are conditional: a synchronous-only source gives none.
    let source = r#"
use Doria\Std\Io\{BufferedReader, BufferedWriter, ReadableStream, WritableStream, ReadinessSet,
    ReadOutcome, WriteOutcome, ByteRegion, WritableByteRegion};
class SyncRead implements ReadableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome { return ReadOutcome::EndOfStream; }
}
class SyncWrite implements WritableStream {
    writable function write(ByteRegion $source): WriteOutcome { return WriteOutcome::Accepted($source->length); }
}
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    let $reader = $set->register(new BufferedReader<SyncRead>(new SyncRead(), 4));
    let $writer = $set->register(new BufferedWriter<SyncWrite>(new SyncWrite(), 4));
}
"#;
    let diagnostics = doriac::check_source("sync-buffered.doria", source)
        .expect_err("synchronous sources give buffered adapters no readiness tier");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0535")
            .count(),
        2,
        "{diagnostics:#?}"
    );
}

#[test]
fn conditional_conformance_rechecks_do_not_reuse_source_pass_class_identities() {
    // The source pass numbers one template per generic class; execution
    // numbers each concrete instance. Pair<int> and Pair<string> shift Cond's
    // execution id, so a source-pass summary of `view` joined into the
    // execution recheck named a property of Pair<string> (malformed MIR).
    assert_every_backend(
        r#"use Doria\Std\Io\{File, ReadableFile, ReadinessStream, ReadinessSource, ByteRegion};
class Pair<T> {
    internal T $first;
    function __construct(parameter take T $first) { $this->first = $first; }
}
class Cond<S> implements ReadinessStream where (S implements ReadinessStream) {
    internal writable S $source;
    internal writable Bytes $buffer;
    function __construct(parameter take S $source) { $this->source = $source; $this->buffer = Bytes::allocate(4); }
    ByteRegion $view { borrowed get => $this->buffer->getRegion(0, 2); }
    ReadinessSource $readinessSource where (S implements ReadinessStream) {
        borrowed get => $this->source->readinessSource;
    }
}
function main(): void {
    write_file("cond-input", "x");
    let $left = new Pair<int>(1);
    let $right = new Pair<string>("two");
    let $cond = new Cond<ReadableFile>(File::openRead("cond-input"));
    if ($cond->view->length != 2) { panic("view lost"); }
    echo "ok\n";
}
"#,
        "ok\n",
    );
}

#[test]
fn dropped_writers_flush_and_destructors_may_mutate_the_dying_object() {
    // Decision log entry 42: a destructor has writable access to the dying
    // object, so a buffered writer flushes when it is dropped.
    assert_every_backend(
        r#"use Doria\Std\Io\{File, FileCreation, FileWriteMode, BufferedWriter, BorrowedBufferedWriter, ReadOutcome};
class Tally {
    internal writable int $count = 0;
    writable function bump(): void { $this->count++; }
    function __destruct(): void {
        $this->bump();
        try {
            echo "tally {$this->count};";
        } catch (Doria\Std\Io\IoError) {}
    }
}
function main(): void {
    {
        let writable $writer = new BufferedWriter<Doria\Std\Io\WritableFile>(
            File::openWrite("owned.txt", FileCreation::CreateOrTruncate, FileWriteMode::Positioned), 64);
        writable Bytes $text = Bytes::fromArray([111, 107]);
        let $accepted = $writer->write($text->getRegion(0, 2));
    }
    {
        let writable $file = File::openWrite("borrowed.txt", FileCreation::CreateOrTruncate, FileWriteMode::Positioned);
        {
            let writable $writer = new BorrowedBufferedWriter<Doria\Std\Io\WritableFile>($file, 64);
            writable Bytes $text = Bytes::fromArray([104, 105]);
            let $accepted = $writer->write($text->getRegion(0, 2));
        }
    }
    echo read_file("owned.txt") . read_file("borrowed.txt") . ";";
    {
        let $tally = new Tally();
    }
    echo "\n";
}
"#,
        "okhi;tally 1;\n",
    );
}

#[test]
fn unused_generic_templates_keep_sibling_field_borrows_disjoint() {
    // Neither class is instantiated: template bodies still check precisely.
    for source in [
        "use Doria\\Std\\Io\\{ReadableStream, WritableByteRegion, IoError};\n\
         class R<S implements ReadableStream> {\n\
             internal writable S $source;\n\
             internal writable Bytes $buffer;\n\
             function __construct(parameter take S $source) { $this->source = $source; $this->buffer = Bytes::allocate(4); }\n\
             writable function go(): void throws IoError { let writable $r = $this->buffer->getWritableRegion(0, 1); let $o = $this->source->read($r); }\n\
         }\nfunction main(): void {}",
        "use Doria\\Std\\Io\\{WritableByteRegion};\n\
         class Sink { writable function fill(writable WritableByteRegion $r): int { return 0; } }\n\
         class Holder {\n\
             internal writable Bytes $buffer;\n\
             function __construct(borrow writable Sink $sink) { $this->buffer = Bytes::allocate(4); }\n\
             writable function go(): void { let writable $r = $this->buffer->getWritableRegion(0, 1); let $n = $this->sink->fill($r); }\n\
         }\nfunction main(): void { let writable $s = new Sink(); { let writable $h = new Holder($s); $h->go(); } }",
    ] {
        let program = lower(source);
        doriac::mir_validation::validate_program(&program).unwrap();
    }
}

#[test]
fn ordinary_classes_retain_readonly_and_exclusive_writable_sources() {
    // Writes through the retained field reach the caller's source; the
    // wrapper's cleanup never destroys it.
    assert_every_backend(SCOPED_CLASS_BORROWING, "4;2;3;1\n");
    // A pending borrowed finish moves its wrapper, and that wrapper's loan,
    // through a generic payload; returned payloads keep parameter loans.
    assert_every_backend(BORROWED_FINISH_TRANSPORT, "some;pending1;3;2\n");
}

#[test]
fn scoped_class_borrows_reject_conflicts_escapes_rebinding_and_extraction() {
    let declarations = r#"
class Counter { function __construct(writable int $value) {} writable function bump(): void { $this->value++; } }
class Handle { function __construct(borrow writable Counter $counter) {} writable function bump(): void { $this->counter->bump(); } }
class View { function __construct(borrow Counter $counter) {} function read(): int { return $this->counter->value; } }
enum Optional<T> { case Some(T $value); case Empty; }
"#;
    for (body, expected) in [
        // Readonly retention excludes writes; writable retention excludes all other access.
        ("function main(): void { let writable $c = new Counter(1); let $v = new View($c); $c->bump(); echo $v->read(); }", "E0763"),
        ("function main(): void { let writable $c = new Counter(1); let writable $h = new Handle($c); echo $c->value; $h->bump(); }", "E0763"),
        ("function main(): void { let writable $c = new Counter(1); let writable $h = new Handle($c); let writable $m = $h; echo $c->value; $m->bump(); }", "E0763"),
        ("function main(): void { let writable $c = new Counter(1); let $o = Optional::Some(new Handle($c)); echo $c->value; let $again = $o; }", "E0763"),
        ("function escape(): Handle { let writable $c = new Counter(1); return new Handle($c); } function main(): void {}", "E0762"),
        ("function escape(): Optional<Handle> { let writable $c = new Counter(1); return Optional::Some(new Handle($c)); } function main(): void {}", "E0762"),
        // The field is bound once and cannot be taken.
        ("class Rebind { function __construct(borrow writable Counter $counter) {} writable function reset(take Counter $other): void { $this->counter = $other; } } function main(): void {}", "E0202"),
        ("class Steal { function __construct(borrow writable Counter $counter) {} consuming function steal(): Counter { return $this->counter; } } function main(): void {}", "E0472"),
        // Readonly retention grants no write-through, and writable retention needs a writable place.
        ("class Reader { function __construct(borrow Counter $counter) {} writable function bump(): void { $this->counter->bump(); } } function main(): void {}", "E0203"),
        ("function main(): void { let $c = new Counter(1); let $h = new Handle($c); }", "E0479"),
    ] {
        let source = format!("{declarations}{body}");
        let diagnostics = doriac::check_source("scoped-borrowing.doria", &source).unwrap_err();
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code == expected),
            "{body}: {diagnostics:?}"
        );
    }
    let accepted = format!("{declarations}function wrap(writable Counter $c): Optional<Handle> {{ return Optional::Some(new Handle($c)); }} function main(): void {{ let writable $c = new Counter(1); {{ let $o = wrap($c); }} $c->bump(); echo $c->value; }}");
    doriac::check_source("scoped-borrowing.doria", accepted).unwrap();
}

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("stream-capabilities.doria", source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
}

const READINESS_SOURCE: &str =
    include_str!("../../../examples/native/main_stream_readiness_source.doria");

#[test]
fn hosted_standard_transfers_preserve_binary_bytes_outcomes_and_canonical_errors() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut php = doriac::compile_source(
        "hosted-standard-transfers.doria",
        r#"
use Doria\Std\Io\ReadinessSet;
function main(): void { let $set = new ReadinessSet(); }
"#,
        BackendTarget::Php,
    )
    .unwrap();
    let doriac::backend::BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
        panic!("standard transfer ABI requires the verified shared runtime");
    };
    contents.push_str(r#"
$runtime = __DoriaNativeIo::get();
$tag = $runtime->new('uint64_t[1]');
$count = $runtime->new('size_t[1]');
$message = $runtime->new('DrStringV1 *[1]');
$path = $runtime->new('DrStringV1 *[1]');
$code = $runtime->new('int64_t[1]');
$progress = $runtime->new('size_t[1]');
$meta = $runtime->new('uint64_t[1]');
$bytes = $runtime->new('uint8_t[3]');
$isNull = static fn(?FFI\CData $value): bool => $value === null || FFI::isNull($value);
$transfer = static function(int $stream, int $operation, ?FFI\CData $data, int $length) use
    ($runtime, $tag, $count, $message, $path, $code, $progress, $meta): int {
    $tag[0] = 99; $count[0] = 99; $code[0] = -99; $progress[0] = 99; $meta[0] = 99;
    return $runtime->dr_v1_checked_standard_stream_io(null, $stream, $operation, $data, $length,
        $tag, $count, $message, $path, $code, $progress, $meta);
};
$success = static function(int $expectedTag, int $expectedCount) use
    ($tag, $count, $message, $path, $code, $progress, $meta, $isNull): void {
    if ($tag[0] !== $expectedTag || $count[0] !== $expectedCount || !$isNull($message[0]) ||
        !$isNull($path[0]) || $code[0] !== 0 || $progress[0] !== 0 || $meta[0] !== 0) {
        throw new LogicException('standard transfer changed outcome or invented failure facts');
    }
};
try {
    if ($transfer(0, 1, FFI::addr($bytes[0]), 3) !== 0) { throw new LogicException('standard read failed'); }
    $success(0, 3);
    if (FFI::string(FFI::addr($bytes[0]), 3) !== "\xff\0Q") { throw new LogicException('binary input changed'); }
    if ($transfer(1, 2, FFI::addr($bytes[0]), 3) !== 0) { throw new LogicException('standard write failed'); }
    $success(0, 3);
    if ($transfer(0, 1, FFI::addr($bytes[0]), 1) !== 0) { throw new LogicException('EOF read failed'); }
    $success(2, 0);
    foreach ([0, 1, 2] as $stream) {
        if ($transfer($stream, $stream === 0 ? 1 : 2, null, 0) !== 0) { throw new LogicException('empty transfer failed'); }
        $success(0, 0);
    }
    if ($transfer(1, 1, FFI::addr($bytes[0]), 1) !== 1 || $tag[0] !== 0 || $count[0] !== 0 ||
        $isNull($message[0]) || !$isNull($path[0]) || $code[0] !== 0 || $progress[0] !== 0 ||
        $meta[0] !== (1 | (2 << 8) | (2 << 16)) ||
        FFI::string($runtime->dr_v1_string_data($message[0]), $runtime->dr_v1_string_length($message[0])) !==
        'failed to read standard output: invalid input') {
        throw new LogicException('standard failure changed canonical target, reason or optional fields');
    }
} finally {
    $runtime->dr_v1_string_release($message[0]);
    $runtime->dr_v1_string_release($path[0]);
}
"#);
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("hosted-standard-transfers.php");
    doriac::backend::publish_output(&path, php).unwrap();
    let mut child = Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP/FFI is required for standard-device parity");
    child.stdin.take().unwrap().write_all(b"\xff\0Q").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"\xff\0Q");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn hosted_file_locks_preserve_ffi_storage_path_and_original_file_identity() {
    let mut php = doriac::compile_source(
        "hosted-file-locks.doria",
        r#"
use Doria\Std\Io\ReadinessSet;
function main(): void { let $set = new ReadinessSet(); }
"#,
        BackendTarget::Php,
    )
    .unwrap();
    let doriac::backend::BackendOutput::TextWithRuntime { contents, .. } = &mut php else {
        panic!("file-object lock ABI requires the verified shared runtime");
    };
    contents.push_str(r#"
$runtime = __DoriaNativeIo::get();
$allocate = static function(string $operation) use ($runtime): FFI\CData {
    $layout = $runtime->new('size_t[2]');
    $runtime->$operation(FFI::addr($layout[0]), FFI::addr($layout[1]));
    $word = $runtime->new('uint64_t');
    if ($layout[0] < 1 || $layout[1] < 1 || $layout[1] > FFI::alignof($word)) {
        throw new LogicException('unsupported opaque lock layout');
    }
    return $runtime->new('uint64_t[' . intdiv($layout[0] + FFI::sizeof($word) - 1, FFI::sizeof($word)) . ']');
};
$isNull = static fn(?FFI\CData $value): bool => $value === null || FFI::isNull($value);
// Read pointer storage explicitly: PHP casts void* scalars through the pointee.
$same = static fn(FFI\CData $a, FFI\CData $b): bool =>
    $runtime->cast('uintptr_t *', FFI::addr($a))[0] ===
    $runtime->cast('uintptr_t *', FFI::addr($b))[0];
$binary = "A\0B";
$native = $runtime->dr_v1_string_from_utf8($binary, strlen($binary));
try {
    if (FFI::string($runtime->dr_v1_string_data($native), $runtime->dr_v1_string_length($native)) !== $binary) {
        throw new LogicException('native bytes were converted to a NUL-terminated C string');
    }
} finally {
    $runtime->dr_v1_string_release($native);
}
file_put_contents('lock-input', 'original');
$path = $runtime->dr_v1_string_from_utf8('lock-input', 10);
try {
    foreach ([0, 1] as $ownership) {
        foreach ([false, true] as $explicit) {
            $file = $allocate('dr_v1_file_object_layout');
            $guard = $allocate('dr_v1_file_object_lock_layout');
            $other = $allocate('dr_v1_file_object_layout');
            $waiting = $allocate('dr_v1_file_object_lock_layout');
            $status = $runtime->new('DrIoStatusV1');
            $returned = $runtime->new('void *[1]');
            $fileHome = FFI::addr($file[0]);
            $guardHome = FFI::addr($guard[0]);
            $otherHome = FFI::addr($other[0]);
            $fileLive = false;
            $otherLive = false;
            $guardLive = false;
            $waitingLive = false;
            try {
                if ($runtime->dr_v1_file_object_open($path, 2, 0, 0, $fileHome, FFI::addr($status)) !== 0) {
                    throw new LogicException('primary file did not open');
                }
                $fileLive = true;
                if ($runtime->dr_v1_file_object_open($path, 2, 0, 0, $otherHome, FFI::addr($status)) !== 0) {
                    throw new LogicException('competing file did not open');
                }
                $otherLive = true;
                $source = $runtime->dr_v1_file_object_readiness_source($fileHome);
                if ($runtime->dr_v1_file_object_try_lock($fileHome, 1, $ownership, $guardHome, FFI::addr($status)) !== 0) {
                    throw new LogicException('guard acquisition failed');
                }
                $guardLive = true;
                if ($ownership === 1) { $fileLive = false; }
                $projected = $runtime->dr_v1_file_object_lock_file($guardHome);
                $target = $runtime->dr_v1_file_object_lock_path($guardHome);
                if (!$same($projected, $fileHome)) {
                    throw new LogicException('guard changed original file identity');
                }
                if (!$same($runtime->dr_v1_file_object_readiness_source($projected), $source)) {
                    throw new LogicException('guard changed readiness source identity');
                }
                if (FFI::string($runtime->dr_v1_string_data($target), $runtime->dr_v1_string_length($target)) !== 'lock-input') {
                    throw new LogicException('guard changed opening path');
                }
                $code = $runtime->dr_v1_file_object_try_lock($otherHome, 1, 1,
                    FFI::addr($waiting[0]), FFI::addr($status));
                if ($code === 0) { $waitingLive = true; $otherLive = false; }
                if ($code !== 1 || $status->has_system_code !== 0 || $status->has_transferred_byte_count !== 0) {
                    throw new LogicException('native contention became failure or consumed an owner');
                }
                if ($explicit) {
                    if ($runtime->dr_v1_file_object_lock_release($guardHome, $returned, FFI::addr($status)) !== 0) {
                        throw new LogicException('explicit guard release failed');
                    }
                    $guardLive = false;
                    if (($ownership === 1 && !$same($returned[0], $fileHome)) ||
                        ($ownership === 0 && !$isNull($returned[0]))) {
                        throw new LogicException('guard release changed ownership');
                    }
                    $fileLive = true;
                    $bytes = $runtime->new('uint8_t[8]');
                    if ($runtime->dr_v1_file_object_read($fileHome, $bytes, 8, FFI::addr($status)) !== 0 ||
                        $status->value !== 8 || FFI::string(FFI::addr($bytes[0]), 8) !== 'original') {
                        throw new LogicException('released original file is unusable');
                    }
                } else {
                    $destroyed = $runtime->dr_v1_file_object_lock_drop($guardHome);
                    $guardLive = false;
                    if (($ownership === 1 && !$same($destroyed, $fileHome)) ||
                        ($ownership === 0 && !$isNull($destroyed))) {
                        throw new LogicException('guard cleanup changed backing-storage ownership');
                    }
                    // Native fields were dropped only for the owning form.
                    // PHP still owns $file's FFI memory; no native class free.
                }
                if ($runtime->dr_v1_file_object_try_lock($otherHome, 1, 1,
                    FFI::addr($waiting[0]), FFI::addr($status)) !== 0) {
                    throw new LogicException('guard did not release its native lock');
                }
                $waitingLive = true;
                $otherLive = false;
                $destroyed = $runtime->dr_v1_file_object_lock_drop(FFI::addr($waiting[0]));
                $waitingLive = false;
                if (!$same($destroyed, $otherHome)) {
                    throw new LogicException('owned cleanup did not return caller-owned FFI storage');
                }
            } finally {
                if ($waitingLive) { $runtime->dr_v1_file_object_lock_drop(FFI::addr($waiting[0])); }
                if ($guardLive) { $runtime->dr_v1_file_object_lock_drop($guardHome); }
                if ($otherLive) { $runtime->dr_v1_file_object_drop(null, $otherHome); }
                if ($fileLive) { $runtime->dr_v1_file_object_drop(null, $fileHome); }
            }
        }
    }
} finally {
    $runtime->dr_v1_string_release($path);
}
if (file_get_contents('lock-input') !== 'original') { throw new LogicException('file contents changed'); }
echo "canonical hosted file locks; original FFI storage\n";
"#);
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("hosted-file-locks.php");
    doriac::backend::publish_output(&path, php).unwrap();
    let output = std::process::Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP/FFI is required for file-lock ownership parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        output.stdout,
        b"canonical hosted file locks; original FFI storage\n"
    );
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn large_regular_file_copy_reuses_one_bounded_buffer_and_preserves_exact_bytes() {
    let source = r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode, ReadableFile, WritableFile, ReadOutcome, WriteOutcome, IoError};
function copyRegularFile(writable ReadableFile $input, writable WritableFile $output,
    writable Bytes $buffer): int throws IoError {
    let writable $total = 0;
    let $capacity = $buffer->length;
    while (true) {
        let $outcome = $input->read($buffer->getWritableRegion(0, $capacity));
        if ($outcome == ReadOutcome::EndOfStream) { return $total; }
        if ($outcome == ReadOutcome::WouldBlock) { panic("regular copy unexpectedly would block"); }
        if ($outcome == ReadOutcome::TimedOut) { panic("regular copy unexpectedly timed out"); }
        let $count = match ($outcome) {
            ReadOutcome::Data($count) => $count,
            ReadOutcome::WouldBlock => 0,
            ReadOutcome::TimedOut => 0,
            ReadOutcome::EndOfStream => 0
        };
        if ($count <= 0) { panic("regular copy made no progress"); }
        let writable $offset = 0;
        while ($offset < $count) {
            let $outcome = $output->write($buffer->getRegion($offset, $count - $offset));
            if ($outcome == WriteOutcome::WouldBlock) { panic("regular write unexpectedly would block"); }
            let $written = match ($outcome) {
                WriteOutcome::Accepted($count) => $count,
                WriteOutcome::WouldBlock => 0
            };
            if ($written <= 0) { panic("regular write made no progress"); }
            $offset += $written;
        }
        $total += $count;
    }
}
function main(): void {
    let writable $input = File::openRead("input.bin");
    let writable $output = File::openWrite("output.bin", FileCreation::CreateNew, FileWriteMode::Positioned);
    writable Bytes $buffer = Bytes::allocate(8192);
    echo copyRegularFile($input, $output, $buffer) . "\n";
    $output->close();
    $input->close();
}
"#;
    let input = (0..1_048_833)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let expected = format!("{}\n", input.len());
    let program = lower(source);
    let interpreted = doriac::mir_interpreter::interpret_with_io(
        &program,
        doriac::mir_interpreter::MirIo {
            files: [("input.bin".to_owned(), input.clone())].into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert_eq!(interpreted.output.exit_status, 0);
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.files["input.bin"], input);
    assert_eq!(interpreted.files["output.bin"], input);
    let chunks = input.len().div_ceil(8192);
    assert_eq!(interpreted.trace.file_reads, chunks + 1);
    assert_eq!(interpreted.trace.file_writes, chunks);
    assert_eq!(interpreted.trace.file_close_attempts, 2);
    assert_cranelift_functions_do_not_allocate(&program, &["copyRegularFile"]);
    #[cfg(feature = "llvm-backend")]
    assert_llvm_functions_do_not_allocate(&program, &["copyRegularFile"]);
    for &profile in NativeProfile::AVAILABLE {
        let directory = native_execution::ExecutionDirectory::new();
        std::fs::write(directory.path().join("input.bin"), &input).unwrap();
        let result =
            native_execution::execute_native_in_directory(&program, profile, &[], &directory);
        assert_eq!(result.status.code(), Some(0), "{profile:?}: {result:?}");
        assert!(result.stderr.is_empty(), "{profile:?}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes());
        assert_eq!(
            std::fs::read(directory.path().join("input.bin")).unwrap(),
            input
        );
        assert_eq!(
            std::fs::read(directory.path().join("output.bin")).unwrap(),
            input
        );
    }
    let directory = native_execution::ExecutionDirectory::new();
    std::fs::write(directory.path().join("input.bin"), &input).unwrap();
    assert_php_execution_in_directory(source, &expected, &directory);
    assert_eq!(
        std::fs::read(directory.path().join("input.bin")).unwrap(),
        input
    );
    assert_eq!(
        std::fs::read(directory.path().join("output.bin")).unwrap(),
        input
    );
}

#[test]
fn file_resize_executes_with_zero_growth_independent_cursors_and_append_semantics() {
    let source = include_str!("../../../examples/native/main_stream_resize.doria");
    let expected = include_str!("fixtures/native_io/main_stream_resize/expected_stdout");
    let program = lower(source);
    let interpreted =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert_eq!(interpreted.output.exit_status, 0);
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.files["resize.bin"], b"abX\n");
    assert_eq!(interpreted.trace.file_close_attempts, 3);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(source, expected);
    assert_eq!(
        std::fs::read(directory.path().join("resize.bin")).unwrap(),
        b"abX\n"
    );
}

#[test]
fn file_resize_requires_typed_length_writable_access_and_nonblocking_hook_safety() {
    let prefix = "use Doria\\Std\\Io\\{ReadableFile, WritableFile, DuplexFile, IoError};";
    for body in [
        "function run(ReadableFile $file): void throws IoError { $file->setLength(1); }",
        "function run(writable ReadableFile $file): void throws IoError { $file->setLength(1); }",
        "function run(WritableFile $file): void throws IoError { $file->setLength(1); }",
        "function run(DuplexFile $file): void throws IoError { $file->setLength(1); }",
        "function run(writable WritableFile $file): void throws IoError { $file->setLength(); }",
        "function run(writable WritableFile $file): void throws IoError { $file->setLength(\"1\"); }",
        "function run(writable WritableFile $file): int throws IoError { return $file->setLength(1); }",
        "function run(writable WritableFile $file): void throws IoError { $file->truncate(1); }",
    ] {
        assert!(doriac::check_source("resize-rejection.doria", format!("{prefix} {body}")).is_err(), "{body}");
    }
    for owner in ["WritableFile", "DuplexFile"] {
        doriac::check_source("resize-acceptance.doria", format!("{prefix} function run(writable {owner} $file): void throws IoError {{ $file->setLength(length: -1); }}")).unwrap();
    }
    for call in ["$this->file->setLength(1);", "resize($this->file);"] {
        let source = format!(
            r#"
{prefix}
function resize(writable WritableFile $file): void throws IoError {{ $file->setLength(1); }}
class Writer {{
    internal writable WritableFile $file;
    function __construct(parameter take WritableFile $file) {{ $this->file = $file; }}
    int $count {{ writable get throws IoError {{ {call} return 0; }} }}
}}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("resize-hook.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0770"),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn independent_mir_validation_rejects_forged_resize_lengths_and_access() {
    use doriac::mir::{CheckedIoOperation, FileControlOperation, Terminator};
    let original = lower(include_str!(
        "../../../examples/native/main_stream_resize.doria"
    ));
    for bad_length in [true, false] {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        Terminator::CheckedIo {
                            operation: CheckedIoOperation::ControlOwnedFile {
                                control: FileControlOperation::SetLength { .. },
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
                    Terminator::CheckedIo {
                        operation: CheckedIoOperation::ControlOwnedFile {
                            control: FileControlOperation::SetLength { .. },
                            ..
                        },
                        ..
                    }
                )
            })
            .unwrap();
        if let Terminator::CheckedIo {
            operation:
                CheckedIoOperation::ControlOwnedFile {
                    file,
                    control: FileControlOperation::SetLength { length },
                },
            ..
        } = &mut block.terminator
        {
            if bad_length {
                *length = *file;
            } else {
                function.locals[file.0].writable = false;
            }
        }
        let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
        if bad_length {
            assert!(error.message.contains("canonical-int length"), "{error:?}");
        }
    }
}

#[test]
fn native_file_resize_uses_shared_control_without_fill_or_cursor_restoration() {
    use doriac::mir::{CheckedIoOperation, FileControlOperation, ReturnType, Terminator};
    let program = lower(include_str!(
        "../../../examples/native/main_stream_resize.doria"
    ));
    let graphs = [
        native_structure::cranelift_references(&program),
        #[cfg(feature = "llvm-backend")]
        native_structure::llvm_references(&program).1,
    ];
    let methods = program
        .functions
        .iter()
        .filter(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    Terminator::CheckedIo {
                        operation: CheckedIoOperation::ControlOwnedFile {
                            control: FileControlOperation::SetLength { .. },
                            ..
                        },
                        ..
                    }
                )
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    for function in methods {
        assert_eq!(function.return_type, ReturnType::Void);
        let symbol = doriac::native_abi::function_symbol(function);
        for graph in &graphs {
            let references = &graph[symbol.trim_start_matches('_')];
            assert!(
                references.contains("dr_v1_checked_file_control"),
                "{references:?}"
            );
            assert!(
                !references.iter().any(|name| name.contains("readiness")
                    || name.contains("file_write")
                    || name.contains("file_seek")
                    || name.contains("executor")
                    || name.contains("worker")),
                "{references:?}"
            );
        }
    }
}

#[test]
fn file_synchronization_executes_with_void_results_and_preserves_file_state() {
    let source = include_str!("../../../examples/native/main_stream_synchronize.doria");
    let expected = include_str!("fixtures/native_io/main_stream_synchronize/expected_stdout");
    let program = lower(source);
    let interpreted =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert_eq!(interpreted.output.exit_status, 0);
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.files["synchronize.bin"], b"abcZ\n");
    assert_eq!(interpreted.trace.file_close_attempts, 2);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(source, expected);
    assert_eq!(
        std::fs::read(directory.path().join("synchronize.bin")).unwrap(),
        b"abcZ\n"
    );
}

#[test]
fn file_synchronization_requires_writable_access_and_keeps_hooks_nonblocking() {
    for method in ["synchronizeData", "synchronizeMetadata"] {
        let prefix = "use Doria\\Std\\Io\\{WritableFile, DuplexFile, ReadableFile, IoError};";
        for body in [
            format!("function run(WritableFile $file): void throws IoError {{ $file->{method}(); }}"),
            format!("function run(DuplexFile $file): void throws IoError {{ $file->{method}(); }}"),
            format!("function run(writable ReadableFile $file): void throws IoError {{ $file->{method}(); }}"),
            format!("function run(writable WritableFile $file): int throws IoError {{ return $file->{method}(); }}"),
        ] {
            assert!(doriac::check_source("sync-rejection.doria", format!("{prefix} {body}")).is_err(), "{body}");
        }
        for owner in ["WritableFile", "DuplexFile"] {
            doriac::check_source("sync-acceptance.doria",
                format!("{prefix} function run(writable {owner} $file): void throws IoError {{ $file->{method}(); }}")).unwrap();
        }
        for direct in [true, false] {
            let call = if direct {
                format!("$this->file->{method}();")
            } else {
                "synchronize($this->file);".to_owned()
            };
            let source = format!(
                r#"
{prefix}
function synchronize(writable WritableFile $file): void throws IoError {{ $file->{method}(); }}
class Writer {{
    internal writable WritableFile $file;
    function __construct(parameter take WritableFile $file) {{ $this->file = $file; }}
    int $count {{ writable get throws IoError {{ {call} return 0; }} }}
}}
"#
            );
            let (_, analysis) = doriac::analyze_source_for_ide("sync-hook.doria", source).unwrap();
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0770"),
                "{:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn independent_mir_validation_rejects_readonly_and_readable_only_synchronization() {
    use doriac::mir::{CheckedIoOperation, FileControlOperation, Terminator};
    let original = lower(include_str!(
        "../../../examples/native/main_stream_synchronize.doria"
    ));
    for control in [
        FileControlOperation::SynchronizeData,
        FileControlOperation::SynchronizeMetadata,
    ] {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(&block.terminator, Terminator::CheckedIo {
                operation: CheckedIoOperation::ControlOwnedFile { control: operation, .. }, ..
            } if *operation == control)
                })
            })
            .unwrap();
        let file = function
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                Terminator::CheckedIo {
                    operation:
                        CheckedIoOperation::ControlOwnedFile {
                            file,
                            control: operation,
                        },
                    ..
                } if *operation == control => Some(*file),
                _ => None,
            })
            .unwrap();
        function.locals[file.0].writable = false;
        assert!(doriac::mir_validation::validate_program(&forged).is_err());

        let mut forged = lower(
            r#"
use Doria\Std\Io\{ReadableFile, IoError, SeekOrigin};
function seek(writable ReadableFile $file): int throws IoError { return $file->seek(0, SeekOrigin::Start); }
function main(): void {}
"#,
        );
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        &block.terminator,
                        Terminator::CheckedIo {
                            operation: CheckedIoOperation::ControlOwnedFile {
                                control: FileControlOperation::Seek { .. },
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
                    &block.terminator,
                    Terminator::CheckedIo {
                        operation: CheckedIoOperation::ControlOwnedFile { .. },
                        ..
                    }
                )
            })
            .unwrap();
        if let Terminator::CheckedIo {
            operation:
                CheckedIoOperation::ControlOwnedFile {
                    control: operation, ..
                },
            result,
            ..
        } = &mut block.terminator
        {
            *operation = control;
            *result = None;
        }
        let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
        assert!(error.message.contains("writable file type"), "{error:?}");
    }
}

#[test]
fn native_synchronization_uses_shared_controls_without_eager_infrastructure() {
    use doriac::mir::{CheckedIoOperation, FileControlOperation, ReturnType, Terminator};
    let program = lower(include_str!(
        "../../../examples/native/main_stream_synchronize.doria"
    ));
    let graphs = [
        native_structure::cranelift_references(&program),
        #[cfg(feature = "llvm-backend")]
        native_structure::llvm_references(&program).1,
    ];
    let mut methods = 0;
    for function in &program.functions {
        if !function.blocks.iter().any(|block| {
            matches!(
                &block.terminator,
                Terminator::CheckedIo {
                    operation: CheckedIoOperation::ControlOwnedFile {
                        control: FileControlOperation::SynchronizeData
                            | FileControlOperation::SynchronizeMetadata,
                        ..
                    },
                    ..
                }
            )
        }) {
            continue;
        }
        methods += 1;
        assert_eq!(function.return_type, ReturnType::Void);
        for block in &function.blocks {
            if let Terminator::CheckedIo {
                operation: CheckedIoOperation::ControlOwnedFile { .. },
                result,
                ..
            } = &block.terminator
            {
                assert!(result.is_none());
            }
        }
        let symbol = doriac::native_abi::function_symbol(function);
        for graph in &graphs {
            let references = &graph[symbol.trim_start_matches('_')];
            assert!(
                references.contains("dr_v1_checked_file_control"),
                "{references:?}"
            );
            assert!(
                !references.iter().any(|name| name.contains("readiness")
                    || name.contains("deadline")
                    || name.contains("cancellation")
                    || name.contains("executor")
                    || name.contains("worker")),
                "{references:?}"
            );
        }
    }
    assert_eq!(methods, 4);
}

#[test]
fn seek_and_readonly_inspection_execute_across_profiles_with_original_errors() {
    let source = include_str!("../../../examples/native/main_stream_seek.doria");
    let expected = "6 2 1 4 4 6 seek-error 101 102 6\n8 9 9 0 0\n0 10 10\n";
    let program = lower(source);
    let interpreted =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert_eq!(interpreted.output.exit_status, 0);
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.files["seek.bin"], b"abcdef\0\0Z!");
    assert_eq!(interpreted.trace.file_close_attempts, 3);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(source, expected);
    assert_eq!(
        std::fs::read(directory.path().join("seek.bin")).unwrap(),
        b"abcdef\0\0Z!"
    );
}

#[test]
fn seek_requires_explicit_typed_origin_and_writable_receiver_but_inspection_does_not() {
    let prefix = "use Doria\\Std\\Io\\{ReadableFile, SeekOrigin, IoError};";
    for body in [
        "function run(ReadableFile $file): int throws IoError { return $file->seek(0, SeekOrigin::Start); }",
        "function run(writable ReadableFile $file): int throws IoError { return $file->seek(0); }",
        "function run(writable ReadableFile $file): int throws IoError { return $file->seek(0, 0); }",
        "function run(writable ReadableFile $file): int throws IoError { $file->position = 2; return 0; }",
        "function run(writable ReadableFile $file): int throws IoError { $file->length = 2; return 0; }",
        "function run(ReadableFile $file): int throws IoError { return $file->position; }",
        "function run(ReadableFile $file): int throws IoError { return $file->length; }",
    ] {
        assert!(doriac::check_source("seek-rejection.doria", format!("{prefix} {body}")).is_err(), "{body}");
    }
    doriac::check_source("readonly-file.doria", format!("{prefix} function run(ReadableFile $file): int throws IoError {{ return $file->getPosition() + $file->getLength(); }}")).unwrap();
}

#[test]
fn readonly_file_inspection_observes_other_open_writes_without_moving_its_cursor() {
    let source = r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode};
function main(): void {
    write_file("fresh.bin", "abc");
    let $reader = File::openRead("fresh.bin");
    echo $reader->getLength() . " ";
    let writable $writer = File::openWrite("fresh.bin", FileCreation::OpenExisting, FileWriteMode::Append);
    writable Bytes $marker = Bytes::allocate(1);
    $marker[0] = 100;
    $writer->write($marker->getRegion(0, 1));
    echo $reader->getLength() . " " . $reader->getPosition() . "\n";
    $writer->close();
    $reader->close();
}
"#;
    let expected = "3 4 0\n";
    let program = lower(source);
    let interpreted =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert_eq!(interpreted.output.exit_status, 0);
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.files["fresh.bin"], b"abcd");
    assert_eq!(interpreted.trace.file_close_attempts, 2);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(source, expected);
    assert_eq!(
        std::fs::read(directory.path().join("fresh.bin")).unwrap(),
        b"abcd"
    );
}

#[test]
fn fresh_file_inspection_methods_keep_the_global_nonblocking_hook_rule() {
    for method in ["getPosition", "getLength"] {
        let source = format!(
            r#"
use Doria\Std\Io\{{ReadableFile, IoError}};
function sample(ReadableFile $file): int throws IoError {{ return $file->{method}(); }}
class Inspector {{
    internal ReadableFile $file;
    function __construct(parameter take ReadableFile $file) {{ $this->file = $file; }}
    int $value {{ get throws IoError => sample($this->file); }}
}}
"#
        );
        let (_, analysis) =
            doriac::analyze_source_for_ide("file-inspection-hook.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0770"),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn independent_mir_validation_checks_seek_types_and_receiver_access() {
    use doriac::mir::{CheckedIoOperation, FileControlOperation, Terminator};
    let original = lower(include_str!(
        "../../../examples/native/main_stream_seek.doria"
    ));
    for invalid_offset in [true, false] {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        Terminator::CheckedIo {
                            operation: CheckedIoOperation::ControlOwnedFile {
                                control: FileControlOperation::Seek { .. },
                                ..
                            },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let (offset, _, file) = function
            .blocks
            .iter()
            .find_map(|block| match block.terminator {
                Terminator::CheckedIo {
                    operation:
                        CheckedIoOperation::ControlOwnedFile {
                            file,
                            control: FileControlOperation::Seek { offset, origin },
                        },
                    ..
                } => Some((offset, origin, file)),
                _ => None,
            })
            .unwrap();
        let control = function
            .blocks
            .iter_mut()
            .find_map(|block| match &mut block.terminator {
                Terminator::CheckedIo {
                    operation:
                        CheckedIoOperation::ControlOwnedFile {
                            control: control @ FileControlOperation::Seek { .. },
                            ..
                        },
                    ..
                } => Some(control),
                _ => None,
            })
            .unwrap();
        let FileControlOperation::Seek {
            offset: input,
            origin: option,
        } = control
        else {
            unreachable!()
        };
        if invalid_offset {
            *input = file;
        } else {
            *option = offset;
        }
        let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
        assert!(
            error.message.contains(if invalid_offset {
                "canonical-int offset"
            } else {
                "canonical enum"
            }),
            "{error:?}, {file:?}"
        );
    }
    let mut forged = original;
    let function = forged
        .functions
        .iter_mut()
        .find(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    Terminator::CheckedIo {
                        operation: CheckedIoOperation::ControlOwnedFile {
                            control: FileControlOperation::Seek { .. },
                            ..
                        },
                        ..
                    }
                )
            })
        })
        .unwrap();
    let file = function
        .blocks
        .iter()
        .find_map(|block| match block.terminator {
            Terminator::CheckedIo {
                operation: CheckedIoOperation::ControlOwnedFile { file, .. },
                ..
            } => Some(file),
            _ => None,
        })
        .unwrap();
    function.locals[file.0].writable = false;
    assert!(doriac::mir_validation::validate_program(&forged).is_err());
}

#[test]
fn ordinary_class_interface_and_collection_loan_aliases_preserve_native_ownership() {
    let source = r#"
interface Named { int $value { get; } }
class Item implements Named {
    int $value = 7 { get => $this->value; }
    function __destruct() { try { echo "drop\n"; } catch (Doria\Std\Io\IoError) {} }
}
function fromClass(Item $item): Item { return $item; }
function fromNullable(?Item $item): ?Item { return $item; }
function fromInterface(Named $item): Named { return $item; }
function fromList(List<int> $items): List<int> { return $items; }
function fromArray(int[] $items): int[] { return $items; }
function main(): void {
    let $owner = new Item();
    let $direct = fromClass($owner);
    let $alias = (($direct));
    let $maybe = fromNullable($owner);
    let $maybeAlias = $maybe;
    let $erased = fromInterface($owner);
    let $erasedAlias = $erased;
    List<int> $list = [9];
    let $listView = fromList($list);
    let $listAlias = $listView;
    int[] $array = [11];
    let $arrayView = fromArray($array);
    let $arrayAlias = $arrayView;
    echo "{$alias->value} {$maybeAlias?->value ?? -1} {$erasedAlias->value} {$listAlias[0]} {$arrayAlias[0]}\n";
}
"#;
    let expected = "7 7 7 9 11\ndrop\n";
    let program = lower(source);
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);
}

#[test]
fn readiness_sources_forward_ordinary_loans_without_adapter_bypass_or_extra_owners() {
    let expected = "source\nread 3\nsource\nread 3\nadapter 1\n";
    let program = lower(READINESS_SOURCE);
    let result = doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(result.output.stdout, expected.as_bytes());
    assert!(result.output.stderr.is_empty());
    assert_eq!(result.output.exit_status, 0);
    assert_eq!(result.trace.file_opens, 2);
    assert_eq!(result.trace.file_reads, 2);
    assert_eq!(result.trace.file_close_attempts, 2);
    assert_eq!(result.files["ready.txt"], b"abc\n");
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(READINESS_SOURCE, expected);
    let getters = program
        .functions
        .iter()
        .filter(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    doriac::mir::Terminator::Return(doriac::mir::Rvalue::Class(
                        doriac::mir::ClassExpression::FileReadinessSource { .. }
                    ))
                )
            })
        })
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(getters.len(), 3);
    let required = [
        doriac::native_abi::FILE_READINESS_SOURCE,
        doriac::native_abi::FILE_READINESS_SOURCE_HOME,
    ];
    assert_cranelift_runtime_references(&program, &getters, &required);
    #[cfg(feature = "llvm-backend")]
    assert_llvm_runtime_references(&program, &getters, &required);
}

#[test]
fn readiness_source_loans_cannot_be_constructed_taken_or_returned_from_local_owners() {
    for body in [
        "function main(): void { let $source = new ReadinessSource(); }",
        r#"function escape(): ReadinessSource {
            let $file = File::openRead("ready.txt");
            return $file->readinessSource;
        }"#,
        r#"function consume(take ReadinessSource $source): void {}
        function main(): void {
            let $file = File::openRead("ready.txt");
            consume($file->readinessSource);
        }"#,
        r#"function inspect(ReadinessSource $source): void {}
        function main(): void {
            let $file = File::openRead("ready.txt");
            let $source = $file->readinessSource;
            $file->close();
            inspect($source);
        }"#,
    ] {
        let source = format!("use Doria\\Std\\Io\\{{File, ReadinessSource}}; {body}");
        let diagnostics = doriac::check_source("invalid-readiness-loan.doria", &source)
            .expect_err("an opaque source cannot escape its ordinary owner loan");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with('E')),
            "{source}: {diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.code.starts_with('N')
                    && !diagnostic.code.starts_with('P')),
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn readiness_source_mir_rejects_forged_result_types_owners_and_allocations() {
    use doriac::mir::{ClassExpression, Rvalue, Terminator};
    let original = lower(READINESS_SOURCE);
    let source = original
        .classes
        .iter()
        .find(|class| class.name == doriac::compiler_known_stream::READINESS_SOURCE)
        .unwrap();
    let (source_class, constructor) = (source.id, source.constructor);
    for mutation in 0..3 {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        Terminator::Return(Rvalue::Class(
                            ClassExpression::FileReadinessSource { .. }
                        ))
                    )
                })
            })
            .unwrap();
        let receiver = function.params[0];
        let Type::Class(owner_class) = function.locals[receiver.0].ty else {
            unreachable!()
        };
        let block = function
            .blocks
            .iter_mut()
            .find(|block| {
                matches!(
                    block.terminator,
                    Terminator::Return(Rvalue::Class(ClassExpression::FileReadinessSource { .. }))
                )
            })
            .unwrap();
        match mutation {
            0 => {
                function.locals[receiver.0].ty = Type::Class(source_class);
            }
            1 => {
                block.terminator =
                    Terminator::Return(Rvalue::Class(ClassExpression::FileReadinessSource {
                        class: owner_class,
                        file: receiver,
                    }));
            }
            2 => {
                block.terminator = Terminator::Return(Rvalue::Class(ClassExpression::New {
                    span: function.source_span,
                    class: source_class,
                    concrete_class: source_class,
                    constructor,
                    properties: Vec::new(),
                    args: Vec::new(),
                }));
            }
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&forged).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn owned_writable_files_preserve_cursors_append_and_erased_dispatch() {
    let source = include_str!("../../../examples/native/main_stream_writable_file.doria");
    let expected = include_str!("fixtures/native_io/main_stream_writable_file/expected_stdout");
    let program = lower(source);
    let result = doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(result.output.stdout, expected.as_bytes());
    assert!(result.output.stderr.is_empty());
    assert_eq!(result.output.exit_status, 0);
    assert_eq!(result.files["positioned.bin"], b"XBCDXXX");
    assert_eq!(result.files["duplex.bin"], b"ABXX");
    assert_eq!(result.trace.file_opens, 6);
    assert_eq!(result.trace.file_reads, 6);
    assert_eq!(result.trace.file_writes, 6);
    assert_eq!(result.trace.file_close_attempts, 6);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_cranelift_functions_do_not_allocate(&program, &["writeErased"]);
    #[cfg(feature = "llvm-backend")]
    assert_llvm_functions_do_not_allocate(&program, &["writeErased"]);
}

#[test]
fn writable_file_creation_matrix_preserves_exact_modes_and_failure_contents() {
    let mut source = String::from(
        r#"use Doria\Std\Io\{File, FileCreation, FileWriteMode, IoError, IoErrorReason};
function main(): void {
    writable Bytes $buffer = Bytes::allocate(1);
    $buffer[0] = 90;
    let $region = $buffer->getRegion(0, 1);
"#,
    );
    let mut expected = String::new();
    let mut files = std::collections::BTreeMap::new();
    let mut opens = 0;
    let mut index = 0;
    for factory in ["openWrite", "openReadWrite"] {
        for creation in [
            "OpenExisting",
            "OpenOrCreate",
            "CreateNew",
            "TruncateExisting",
            "CreateOrTruncate",
        ] {
            for mode in ["Positioned", "Append"] {
                for existing in [false, true] {
                    let path = format!("file-{index}.bin");
                    if existing {
                        source.push_str(&format!("write_file(\"{path}\", \"AB\");\n"));
                    }
                    source.push_str(&format!(
                        r#"echo "{index} ";
try {{
    let writable $file = File::{factory}("{path}", FileCreation::{creation}, FileWriteMode::{mode});
    $file->write($region);
    echo "ok ";
}} catch (IoError $error) {{
    echo match ($error->reason) {{
        IoErrorReason::NotFound => "missing ",
        IoErrorReason::Other => "exists ",
        default => "wrong error ",
    }};
}}
try {{ echo read_file("{path}") . "\n"; }}
catch (IoError) {{ echo "absent\n"; }}
"#
                    ));
                    let failed_missing =
                        !existing && matches!(creation, "OpenExisting" | "TruncateExisting");
                    let failed_existing = existing && creation == "CreateNew";
                    let contents = if failed_missing {
                        "absent"
                    } else if failed_existing {
                        "AB"
                    } else if !existing
                        || matches!(creation, "TruncateExisting" | "CreateOrTruncate")
                    {
                        "Z"
                    } else if mode == "Append" {
                        "ABZ"
                    } else {
                        "ZB"
                    };
                    let status = if failed_missing {
                        "missing"
                    } else if failed_existing {
                        "exists"
                    } else {
                        opens += 1;
                        "ok"
                    };
                    expected.push_str(&format!("{index} {status} {contents}\n"));
                    if !failed_missing {
                        files.insert(path, contents.as_bytes().to_vec());
                    }
                    index += 1;
                }
            }
        }
    }
    source.push_str("}\n");
    let program = lower(&source);
    let result = doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(result.output.stdout, expected.as_bytes());
    assert!(result.output.stderr.is_empty());
    assert_eq!(result.output.exit_status, 0);
    assert_eq!(result.files, files);
    assert_eq!(result.trace.file_opens, opens);
    assert_eq!(result.trace.file_writes, opens);
    assert_eq!(result.trace.file_close_attempts, opens);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, &expected);
    }
    let php = assert_php_execution(&source, &expected);
    for (path, contents) in files {
        assert_eq!(std::fs::read(php.path().join(path)).unwrap(), contents);
    }
}

#[test]
fn file_write_failure_preserves_unknown_zero_and_positive_progress() {
    use doriac::mir_interpreter::{interpret_with_io, MirIo, MirIoFaults};
    let source = r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode, IoError, IoOperation, IoTarget, WriteOutcome};
function main(): void {
    let writable $file = File::openWrite("input.bin", FileCreation::OpenExisting, FileWriteMode::Positioned);
    writable Bytes $bytes = Bytes::allocate(3);
    $bytes[0] = 88; $bytes[1] = 89; $bytes[2] = 90;
    let $source = $bytes->getRegion(0, 3);
    let $empty = $source->getRegion(3, 0);
    if ($file->write($empty) == WriteOutcome::Accepted(0)) { echo "empty\n"; }
    try { $file->write($source); }
    catch (IoError $error) {
        if ($error->operation == IoOperation::Write && ($error->systemCode ?? -1) == 5) {
            echo match ($error->target) {
                IoTarget::File($path) => "write {$path}:{$error->transferredByteCount ?? -1}\n",
                default => "wrong target\n",
            };
        }
    }
    try { $file->close(); }
    catch (IoError $error) {
        if ($error->operation == IoOperation::Close && $error->transferredByteCount == null) {
            echo "close\n";
        }
    }
}
"#;
    let program = lower(source);
    for (progress, contents) in [(None, "ABC"), (Some(0), "ABC"), (Some(2), "XYC")] {
        let result = interpret_with_io(
            &program,
            MirIo {
                files: [("input.bin".into(), b"ABC".to_vec())].into(),
                faults: MirIoFaults {
                    file_write: true,
                    file_close: true,
                    transferred_byte_count: progress,
                    system_code: Some(5),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.output.stdout,
            format!(
                "empty\nwrite input.bin:{}\nclose\n",
                progress.map_or(-1, |count| count as i64)
            )
            .as_bytes()
        );
        assert!(result.output.stderr.is_empty());
        assert_eq!(result.output.exit_status, 0);
        assert_eq!(result.files["input.bin"], contents.as_bytes());
        assert_eq!(result.trace.file_opens, 1);
        assert_eq!(result.trace.file_writes, 1);
        assert_eq!(result.trace.file_close_attempts, 1);
    }
}

#[test]
fn writable_file_modes_are_required_and_capabilities_are_static() {
    let imports = "use Doria\\Std\\Io\\{File, FileCreation, FileWriteMode};";
    for factory in ["openWrite", "openReadWrite"] {
        let source = format!(
            "{imports} function main(): void {{ File::{factory}(\"input.bin\", FileCreation::OpenExisting); }}"
        );
        let diagnostics = doriac::check_source("missing-file-mode.doria", source)
            .expect_err("write mode has no default");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0409"),
            "{diagnostics:?}"
        );
    }
    for (factory, options, call) in [
        ("openRead", "", "$file->write($source)"),
        (
            "openWrite",
            ", FileCreation::OpenExisting, FileWriteMode::Positioned",
            "$file->read($destination)",
        ),
    ] {
        let source = format!(
            r#"{imports}
function main(): void {{
    let writable $file = File::{factory}("input.bin"{options});
    writable Bytes $bytes = Bytes::allocate(1);
    let $source = $bytes->getRegion(0, 0);
    let writable $destination = $bytes->getWritableRegion(0, 0);
    {call};
}}
"#
        );
        let diagnostics = doriac::check_source("wrong-file-capability.doria", source)
            .expect_err("file capabilities cannot be widened");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0304"),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn php_file_policies_execute_without_the_former_writable_backend_boundary() {
    for source in [
        r#"use Doria\Std\Io\WritableFile as Writer;
function inspect(?Writer $file): void {} function main(): void {}"#
            .to_owned(),
        r#"use Doria\Std\Io\DuplexFile;
function inspect(List<DuplexFile> $files): void {} function main(): void {}"#
            .to_owned(),
    ] {
        let output = doriac::compile_source("php-writable-file.doria", source, BackendTarget::Php)
            .expect("file types and containers are supported by PHP");
        assert!(matches!(
            output,
            doriac::backend::BackendOutput::Text { .. }
                | doriac::backend::BackendOutput::TextWithRuntime { .. }
        ));
    }
    assert_php_execution(
        include_str!("../../../examples/native/main_stream_writable_file.doria"),
        include_str!("fixtures/native_io/main_stream_writable_file/expected_stdout"),
    );
    assert_php_execution(
        include_str!("../../../examples/native/main_stream_readable_file.doria"),
        include_str!("fixtures/native_io/main_stream_readable_file/expected_stdout"),
    );
}

#[test]
fn named_file_options_use_ordinary_aliases_and_evaluate_once_in_source_order() {
    let source = r#"
use Doria\Std\Io\File as Files;
use Doria\Std\Io\FileCreation as Creation;
use Doria\Std\Io\FileWriteMode as Mode;
function choosePath(): string { echo "path\n"; return "named.bin"; }
function chooseMode(): Mode { echo "mode\n"; return Mode::Append; }
function chooseCreation(): Creation { echo "creation\n"; return Creation::CreateNew; }
function main(): void {
    let writable $file = Files::openWrite(mode: chooseMode(), path: choosePath(), creation: chooseCreation());
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 88;
    let $source = $bytes->getRegion(0, 1);
    $file->write($source);
    $file->close();
    echo read_file("named.bin") . "\n";
}
"#;
    let program = lower(source);
    let expected = "mode\npath\ncreation\nX\n";
    let result = doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(result.output.stdout, expected.as_bytes());
    assert!(result.output.stderr.is_empty());
    assert_eq!(result.output.exit_status, 0);
    assert_eq!(result.trace.file_opens, 1);
    assert_eq!(result.trace.file_close_attempts, 1);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);
}

#[test]
fn writable_file_mir_rejects_forged_options_regions_and_outcomes() {
    use doriac::mir::{CheckedIoOperation, Terminator};
    let original = lower(include_str!(
        "../../../examples/native/main_stream_writable_file.doria"
    ));
    for missing_creation in [false, true] {
        let mut forged = original.clone();
        let operation = forged
            .functions
            .iter_mut()
            .flat_map(|function| &mut function.blocks)
            .find_map(|block| match &mut block.terminator {
                Terminator::CheckedIo {
                    operation:
                        operation @ CheckedIoOperation::OpenOwnedFile {
                            creation: Some(_), ..
                        },
                    ..
                } => Some(operation),
                _ => None,
            })
            .unwrap();
        let CheckedIoOperation::OpenOwnedFile { creation, mode, .. } = operation else {
            unreachable!()
        };
        if missing_creation {
            *creation = None;
        } else {
            *mode = None;
        }
        let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
        assert!(
            error
                .message
                .contains("exact creation and write-mode options"),
            "{error:?}"
        );
    }
    let mut forged = original.clone();
    let operation = forged
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .find_map(|block| match &mut block.terminator {
            Terminator::CheckedIo {
                operation:
                    operation @ CheckedIoOperation::OpenOwnedFile {
                        creation: Some(_), ..
                    },
                ..
            } => Some(operation),
            _ => None,
        })
        .unwrap();
    let CheckedIoOperation::OpenOwnedFile { creation, mode, .. } = operation else {
        unreachable!()
    };
    *mode = *creation;
    let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
    assert!(error.message.contains("canonical enum"), "{error:?}");

    let mut forged = original.clone();
    let operation = forged
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .find_map(|block| match &mut block.terminator {
            Terminator::CheckedIo {
                operation: operation @ CheckedIoOperation::WriteOwnedFile { .. },
                ..
            } => Some(operation),
            _ => None,
        })
        .unwrap();
    let CheckedIoOperation::WriteOwnedFile { source, file, .. } = operation else {
        unreachable!()
    };
    *source = *file;
    let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
    assert!(
        error.message.contains("readonly borrowed byte storage"),
        "{error:?}"
    );

    let mut forged = original;
    forged
        .enums
        .iter_mut()
        .find(|definition| definition.name == "Doria\\Std\\Io\\WriteOutcome")
        .unwrap()
        .cases[0]
        .name = "Data".into();
    let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
    assert!(
        error.message.contains("canonical WriteOutcome"),
        "{error:?}"
    );
}

#[test]
fn checked_io_mir_requires_ownership_for_every_owned_file_and_byte_result() {
    use doriac::mir::{CheckedIoOperation, Statement, Terminator};
    let original = lower(
        r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode};
function main(): void {
    let $reader = File::openRead("input.bin");
    let $writer = File::openWrite("output.bin", FileCreation::OpenOrCreate, FileWriteMode::Positioned);
    let $duplex = File::openReadWrite("duplex.bin", FileCreation::OpenOrCreate, FileWriteMode::Append);
    let $bytes = read_file_bytes("input.bin");
    let $stdin = read_stdin_bytes();
}
"#,
    );
    let mut checked_results = 0;
    for (function_index, function) in original.functions.iter().enumerate() {
        for block in &function.blocks {
            let Terminator::CheckedIo {
                operation,
                result: Some(result),
                ..
            } = &block.terminator
            else {
                continue;
            };
            if !matches!(
                operation,
                CheckedIoOperation::OpenOwnedFile { .. }
                    | CheckedIoOperation::ReadFile { bytes: true, .. }
                    | CheckedIoOperation::ReadStdinBytes
            ) {
                continue;
            }
            let mut forged = original.clone();
            let function = &mut forged.functions[function_index];
            function.locals[result.0].owned = false;
            for block in &mut function.blocks {
                block.statements.retain(|statement| {
                    !matches!(statement,
                        Statement::DropClass { local, .. } | Statement::DropCollection { local, .. }
                            if local == result
                    )
                });
            }
            let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
            assert!(
                error
                    .message
                    .contains("success slot must own its returned value"),
                "{error:?}"
            );
            checked_results += 1;
        }
    }
    assert_eq!(checked_results, 5);
}

#[test]
fn owned_readable_files_reuse_regions_and_ordinary_checked_calls() {
    let source = include_str!("../../../examples/native/main_stream_readable_file.doria");
    let expected = include_str!("fixtures/native_io/main_stream_readable_file/expected_stdout");
    let program = lower(source);
    let interpreted =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.output.exit_status, 0);
    assert_eq!(interpreted.files["stream-read.txt"], b"ABC\n");
    assert_eq!(interpreted.trace.file_opens, 2);
    assert_eq!(interpreted.trace.file_reads, 4);
    assert_eq!(interpreted.trace.file_close_attempts, 2);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(source, expected);
    assert_eq!(
        std::fs::read(directory.path().join("stream-read.txt")).unwrap(),
        b"ABC\n"
    );
}

#[test]
fn nullable_files_and_owning_containers_preserve_one_close_obligation() {
    let source = include_str!("../../../examples/native/main_stream_file_ownership.doria");
    let expected = include_str!("fixtures/native_io/main_stream_file_ownership/expected_stdout");
    let program = lower(source);
    let interpreted =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.output.exit_status, 0);
    assert_eq!(interpreted.trace.file_opens, 4);
    assert_eq!(interpreted.trace.file_close_attempts, 4);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let directory = assert_php_execution(source, expected);
    assert_eq!(
        std::fs::read(directory.path().join("stream-owned.txt")).unwrap(),
        b"AB\n"
    );
}

#[test]
fn checked_file_failure_preserves_progress_path_and_disposes_owner_once() {
    use doriac::mir_interpreter::{interpret_with_io, MirIo, MirIoFailureReason, MirIoFaults};
    let source = r#"
use Doria\Std\Io\{File, IoError, IoTarget, IoOperation, ReadOutcome};
function main(): void throws IoError {
    let writable $file = File::openRead("input.bin");
    writable Bytes $bytes = Bytes::allocate(3);
    {
        let writable $empty = $bytes->getWritableRegion(3, 0);
        if ($file->read($empty) == ReadOutcome::Data(0)) { echo "empty\n"; }
        let writable $destination = $bytes->getWritableRegion(0, 3);
        try { $file->read($destination); }
        catch (IoError $error) {
            if ($error->operation == IoOperation::Read) {
                echo "progress {$error->transferredByteCount ?? -1}\n";
            }
        }
    }
    echo "bytes {$bytes[0]} {$bytes[1]} {$bytes[2]}\n";
    try { $file->close(); }
    catch (IoError $error) {
        if ($error->operation == IoOperation::Close && $error->transferredByteCount == null) {
            echo match ($error->target) {
                IoTarget::File($path) => "close {$path}\n",
                default => "wrong target\n",
            };
        }
    }
}
"#;
    let program = lower(source);
    let output = interpret_with_io(
        &program,
        MirIo {
            files: [("input.bin".into(), b"ABC".to_vec())].into(),
            faults: MirIoFaults {
                file_read: true,
                file_close: true,
                failure_reason: MirIoFailureReason::Other,
                transferred_byte_count: Some(2),
                system_code: Some(5),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        output.output.stdout,
        b"empty\nprogress 2\nbytes 65 66 0\nclose input.bin\n"
    );
    assert!(output.output.stderr.is_empty());
    assert_eq!(output.output.exit_status, 0);
    assert_eq!(output.trace.file_opens, 1);
    assert_eq!(output.trace.file_reads, 1);
    assert_eq!(output.trace.file_close_attempts, 1);
}

#[test]
fn consumed_file_is_unavailable_on_success_and_checked_failure_paths() {
    for close in [
        "$file->close();",
        "try { $file->close(); } catch (IoError) {}",
    ] {
        let source = format!(
            r#"
use Doria\Std\Io\{{File, IoError}};
function main(): void throws IoError {{
    let writable $file = File::openRead("input.txt");
    writable Bytes $bytes = Bytes::allocate(1);
    {close}
    let writable $destination = $bytes->getWritableRegion(0, 1);
    $file->read($destination);
}}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("consumed-file.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0470"),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn file_mir_rejects_forged_outcomes_storage_and_post_close_access() {
    use doriac::mir::{CheckedIoOperation, ClassExpression, Rvalue, Statement, Terminator};
    let source = include_str!("../../../examples/native/main_stream_readable_file.doria");
    let original = lower(source);
    let mut forged = original.clone();
    forged
        .enums
        .iter_mut()
        .find(|definition| definition.name == "Doria\\Std\\Io\\ReadOutcome")
        .unwrap()
        .cases[0]
        .name = "Accepted".into();
    let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
    assert!(error.message.contains("canonical ReadOutcome"), "{error:?}");

    for failure_path in [false, true] {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function
                    .method
                    .as_ref()
                    .is_some_and(|method| method.name == "close")
            })
            .unwrap();
        let (file, target) = function
            .blocks
            .iter()
            .find_map(|block| match block.terminator {
                Terminator::CheckedIo {
                    operation: CheckedIoOperation::CloseOwnedFile { file },
                    success,
                    failure,
                    ..
                } => Some((file, if failure_path { failure } else { success })),
                _ => None,
            })
            .unwrap();
        let Type::Class(class) = function.locals[file.0].ty else {
            unreachable!()
        };
        function.blocks[target.0].statements.insert(
            0,
            Statement::AssignLocal {
                target: file,
                value: Rvalue::Class(ClassExpression::Local {
                    class,
                    local: file,
                    transfer: false,
                }),
            },
        );
        let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
        assert!(error.message.contains("after closing"), "{error:?}");
    }

    let mut forged = original;
    let function = forged
        .functions
        .iter_mut()
        .find(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    Terminator::CheckedIo {
                        operation: CheckedIoOperation::OpenOwnedFile { .. },
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
                Terminator::CheckedIo {
                    operation: CheckedIoOperation::OpenOwnedFile { .. },
                    ..
                }
            )
        })
        .unwrap();
    let Terminator::CheckedIo {
        operation: CheckedIoOperation::OpenOwnedFile { class, .. },
        result: Some(result),
        success,
        ..
    } = block.terminator
    else {
        unreachable!()
    };
    let constructor = forged.classes[class.0].constructor;
    block.statements.push(Statement::AssignLocal {
        target: result,
        value: Rvalue::Class(ClassExpression::New {
            span: function.source_span,
            class,
            concrete_class: class,
            constructor,
            properties: Vec::new(),
            args: Vec::new(),
        }),
    });
    block.terminator = Terminator::Jump(success);
    let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
    assert!(
        error.message.contains("ordinary class allocation"),
        "{error:?}"
    );
}

#[test]
fn compiler_owned_file_calls_cannot_bypass_nonblocking_hook_checks() {
    for body in [
        r#"class Value { ReadableFile $file { get throws IoError => File::openRead("input.bin"); } }"#,
        r#"class Value {
            internal writable ReadableFile $file;
            function __construct(parameter take ReadableFile $file) { $this->file = $file; }
            int $count {
                writable get throws IoError {
                    writable Bytes $bytes = Bytes::allocate(1);
                    let writable $destination = $bytes->getWritableRegion(0, 1);
                    $this->file->read($destination);
                    return 0;
                }
            }
        }"#,
    ] {
        let source = format!("use Doria\\Std\\Io\\{{File, ReadableFile, IoError}}; {body}");
        let (_, analysis) = doriac::analyze_source_for_ide("file-hook.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0770"),
            "{:?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0770"),
            "{:?}",
            analysis.diagnostics
        );
    }
    let (_, analysis) = doriac::analyze_source_for_ide(
        "nonblocking-hook.doria",
        "class Value { int $count { get => 1; } }",
    )
    .unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn readiness_errors_extend_the_existing_family_without_changing_prior_tags() {
    let source = include_str!("../../../examples/native/main_stream_readiness_error.doria");
    let expected = include_str!("fixtures/native_io/main_stream_readiness_error/expected_stdout");
    let program = lower(source);
    let target = program
        .enums
        .iter()
        .find(|definition| definition.name == "Doria\\Std\\Io\\IoTarget")
        .unwrap();
    assert_eq!(
        target
            .cases
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        [
            "File",
            "StandardInput",
            "StandardOutput",
            "StandardError",
            "ReadinessSet",
            "TransferPlan",
            "TransferGroup",
            "Process",
            "ProcessInput",
            "ProcessOutput",
            "ProcessErrors"
        ]
    );
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);

    let exhaustive = include_str!("../../../examples/native/main_stream_close_error.doria");
    for (case, word) in [
        ("Register", "register"),
        ("Unregister", "unregister"),
        ("Wait", "wait"),
        ("Configure", "configure"),
        ("Access", "access"),
        ("Seek", "seek"),
        ("SynchronizeData", "synchronizeData"),
        ("SynchronizeMetadata", "synchronizeMetadata"),
        ("Resize", "resize"),
        ("Lock", "lock"),
        ("Unlock", "unlock"),
        ("Spawn", "spawn"),
        ("Terminate", "terminate"),
    ] {
        let comma = if case == "Terminate" { "" } else { "," };
        let source = exhaustive.replace(
            &format!("        IoOperation::{case} => \"{word}\"{comma}\n"),
            "",
        );
        let diagnostics = doriac::check_source("readiness-exhaustiveness.doria", source)
            .expect_err("every added operation needs an exhaustive match arm");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0585"
                    && diagnostic
                        .explanation
                        .as_deref()
                        .is_some_and(|text| text.contains(&format!("IoOperation::{case}")))),
            "{diagnostics:?}"
        );
    }

    let exhaustive_target = r#"
use Doria\Std\Io\IoTarget;
function name(IoTarget $target): string {
    return match ($target) {
        IoTarget::File($path) => $path,
        IoTarget::StandardInput => "input",
        IoTarget::StandardOutput => "output",
        IoTarget::StandardError => "error",
        IoTarget::ReadinessSet => "readiness",
        IoTarget::TransferPlan => "plan",
        IoTarget::TransferGroup => "group",
        IoTarget::Process($program) => $program,
        IoTarget::ProcessInput($program) => $program,
        IoTarget::ProcessOutput($program) => $program,
        IoTarget::ProcessErrors($program) => $program
    };
}
function main(): void {}
"#;
    doriac::check_source("readiness-target-exhaustiveness.doria", exhaustive_target).unwrap();
    let missing_target =
        exhaustive_target.replace("        IoTarget::ReadinessSet => \"readiness\",\n", "");
    let diagnostics = doriac::check_source("readiness-target-exhaustiveness.doria", missing_target)
        .expect_err("the added target needs an exhaustive match arm");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0585"
                && diagnostic
                    .explanation
                    .as_deref()
                    .is_some_and(|text| text.contains("IoTarget::ReadinessSet"))),
        "{diagnostics:?}"
    );
}

#[test]
fn transfer_group_errors_use_the_existing_transport_and_require_an_exhaustive_target_arm() {
    let source = r#"
use Doria\Std\Io\{IoError, IoErrorReason, IoOperation, IoTarget};
function access(): void throws IoError {
    throw new IoError("failed to access transfer group: invalid input",
        IoOperation::Access, IoTarget::TransferGroup, IoErrorReason::InvalidInput);
}
function main(): void {
    try {
        access();
    } catch (IoError $error) {
        echo ($error->operation == IoOperation::Access) . " ";
        echo match ($error->target) {
            IoTarget::File($path) => $path,
            IoTarget::StandardInput => "input",
            IoTarget::StandardOutput => "output",
            IoTarget::StandardError => "error",
            IoTarget::ReadinessSet => "set",
            IoTarget::TransferPlan => "plan",
            IoTarget::TransferGroup => "group",
            IoTarget::Process($program) => $program,
            IoTarget::ProcessInput($program) => $program,
            IoTarget::ProcessOutput($program) => $program,
            IoTarget::ProcessErrors($program) => $program
        };
        echo " " . ($error->reason == IoErrorReason::InvalidInput);
        echo " " . ($error->systemCode == null);
        echo " " . ($error->transferredByteCount == null) . "\n";
        echo $error->message . "\n";
    }
}
"#;
    for (operation, verb) in [("Access", "access"), ("Configure", "configure")] {
        let fixture = source
            .replace("IoOperation::Access", &format!("IoOperation::{operation}"))
            .replace(
                "failed to access transfer group",
                &format!("failed to {verb} transfer group"),
            );
        let expected =
            format!("true group true true true\nfailed to {verb} transfer group: invalid input\n");
        let program = lower(&fixture);
        let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(interpreted.stdout, expected.as_bytes());
        assert!(interpreted.stderr.is_empty());
        assert_eq!(interpreted.exit_status, 0);
        for &profile in NativeProfile::AVAILABLE {
            native_execution::assert_native_execution(&program, profile, &expected);
        }
        assert_php_execution(&fixture, &expected);
    }
    let missing = source
        .replace("            IoTarget::TransferPlan => \"plan\",\n", "")
        .replace("            IoTarget::TransferGroup => \"group\",\n", "");
    let diagnostics = doriac::check_source("group-target-exhaustiveness.doria", missing)
        .expect_err("both appended targets require exhaustive match arms");
    for target in ["TransferPlan", "TransferGroup"] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0585"
                    && diagnostic
                        .explanation
                        .as_deref()
                        .is_some_and(|text| text.contains(&format!("IoTarget::{target}")))),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn transfer_plan_configuration_uses_existing_checked_error_transport() {
    let source = r#"
use Doria\Std\Io\{IoError, IoErrorReason, IoOperation, IoTarget};
function configure(): void throws IoError {
    throw new IoError("failed to configure transfer plan: resource exhausted",
        IoOperation::Configure, IoTarget::TransferPlan,
        IoErrorReason::ResourceExhausted, 0);
}
function main(): void {
    try {
        configure();
    } catch (IoError $error) {
        echo ($error->operation == IoOperation::Configure) . " ";
        echo match ($error->target) {
            IoTarget::TransferPlan => "plan",
            default => "wrong target"
        };
        echo " " . ($error->reason == IoErrorReason::ResourceExhausted);
        echo " " . (($error->systemCode ?? -1) == 0);
        echo " " . ($error->transferredByteCount == null) . "\n";
        echo $error->message . "\n";
    }
}
"#;
    let expected =
        "true plan true true true\nfailed to configure transfer plan: resource exhausted\n";
    let program = lower(source);
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);

    let exhaustive_target = r#"
use Doria\Std\Io\IoTarget;
function describe(IoTarget $target): string {
    return match ($target) {
        IoTarget::File($path) => $path,
        IoTarget::StandardInput => "input",
        IoTarget::StandardOutput => "output",
        IoTarget::StandardError => "error",
        IoTarget::ReadinessSet => "set"
    };
}
function main(): void {}
"#;
    let diagnostics = doriac::check_source("plan-target-exhaustiveness.doria", exhaustive_target)
        .expect_err("TransferPlan requires an exhaustive target match arm");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0585"
                && diagnostic
                    .explanation
                    .as_deref()
                    .is_some_and(|text| text.contains("IoTarget::TransferPlan"))),
        "{diagnostics:?}"
    );
}

#[test]
fn close_reuses_canonical_io_error_and_preserves_existing_operation_tags() {
    let source = include_str!("../../../examples/native/main_stream_close_error.doria");
    let expected = include_str!("fixtures/native_io/main_stream_close_error/expected_stdout");
    let program = lower(source);
    let operation = program
        .enums
        .iter()
        .find(|definition| definition.name == "Doria\\Std\\Io\\IoOperation")
        .unwrap();
    assert_eq!(
        operation
            .cases
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Open",
            "Read",
            "Write",
            "Append",
            "Flush",
            "Close",
            "Register",
            "Unregister",
            "Wait",
            "Configure",
            "Access",
            "Seek",
            "SynchronizeData",
            "SynchronizeMetadata",
            "Resize",
            "Lock",
            "Unlock",
            "Spawn",
            "Terminate"
        ]
    );
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);

    let source = source.replace("        IoOperation::Close => \"close\",", "");
    let diagnostics = doriac::check_source("close-exhaustiveness.doria", source)
        .expect_err("an exhaustive operation match must now account for Close");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0585"
                && diagnostic
                    .explanation
                    .as_deref()
                    .is_some_and(|text| text.contains("IoOperation::Close"))),
        "{diagnostics:?}"
    );
}

#[test]
fn file_creation_choices_use_ordinary_namespaced_copy_enum_contracts() {
    let source = include_str!("../../../examples/native/main_stream_file_creation.doria");
    let expected = include_str!("fixtures/native_io/main_stream_file_creation/expected_stdout");
    let program = lower(source);
    let creation = program
        .enums
        .iter()
        .find(|definition| definition.name == "Doria\\Std\\Io\\FileCreation")
        .unwrap();
    assert!(creation.capabilities.copy);
    assert!(!creation.capabilities.needs_drop);
    assert_eq!(
        creation
            .cases
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        [
            "OpenExisting",
            "OpenOrCreate",
            "CreateNew",
            "TruncateExisting",
            "CreateOrTruncate"
        ]
    );
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);

    assert!(doriac::check_source(
        "unqualified-file-creation.doria",
        "function inspect(FileCreation $creation): void {}"
    )
    .is_err());
    let (_, analysis) = doriac::analyze_source_for_ide("aliased-file-creation.doria",
        "use Doria\\Std\\Io\\FileCreation as Creation; function inspect(Creation $creation): void {}").unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn transfer_outcomes_reuse_ordinary_inline_copy_enum_layout() {
    let program = lower(SOURCE);
    for name in ["ReadOutcome", "WriteOutcome"] {
        let definition = program
            .enums
            .iter()
            .find(|definition| definition.name == format!("Doria\\Std\\Io\\{name}"))
            .unwrap();
        assert!(definition.capabilities.copy);
        assert!(definition.capabilities.trivial_copy);
        assert!(!definition.capabilities.needs_drop);
        assert_eq!(definition.layout.size, 16);
        assert_eq!(definition.layout.align, 8);
        let ty = Type::PayloadEnum(definition.payload_type().unwrap());
        assert!(!ty.has_move_ownership());
        assert!(!doriac::native_closure_abi::needs_drop(ty));
        for width in [4, 8] {
            assert_eq!(doriac::native_closure_abi::type_layout(ty, width).size, 16);
        }
        let nullable = Type::NullablePayloadEnum(definition.payload_type().unwrap());
        assert!(!nullable.has_move_ownership());
        assert!(!doriac::native_closure_abi::needs_drop(nullable));
        assert_eq!(
            program
                .functions
                .iter()
                .find(|function| function.name
                    == if name == "ReadOutcome" {
                        "forwardRead"
                    } else {
                        "forwardWrite"
                    })
                .unwrap()
                .return_type,
            doriac::mir::ReturnType::Value(nullable)
        );
    }
}

#[test]
fn flush_outcomes_use_ordinary_scalar_copy_enum_transport() {
    let program = lower(OUTCOME_SOURCE);
    let definition = program
        .enums
        .iter()
        .find(|definition| definition.name == "Doria\\Std\\Io\\FlushOutcome")
        .unwrap();
    assert!(definition.capabilities.copy);
    assert!(definition.capabilities.trivial_copy);
    assert!(!definition.capabilities.needs_drop);
    assert!(definition.payload_type().is_none());
    assert_eq!(
        definition
            .cases
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        ["Complete", "Pending"]
    );
    let ty = Type::Scalar(doriac::mir::ScalarType::Enum(definition.id));
    assert!(!ty.has_move_ownership());
    assert!(!doriac::native_closure_abi::needs_drop(ty));
    for name in ["flushed", "pendingFlush", "reuseFlush"] {
        assert_eq!(
            program
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap()
                .return_type,
            doriac::mir::ReturnType::Value(ty)
        );
    }
}

#[test]
fn flush_contract_requires_outcome_and_exclusive_interface_access() {
    let imports = "use Doria\\Std\\Io\\{FlushableStream, FlushOutcome, IoError};";
    doriac::check_source("flush-result.doria", format!(
        "{imports} class Ready implements FlushableStream {{ writable function flush(): FlushOutcome throws IoError {{ return FlushOutcome::Complete; }} }}"
    )).unwrap();
    let diagnostics = doriac::check_source("old-flush-result.doria", format!(
        "{imports} class Old implements FlushableStream {{ writable function flush(): void throws IoError {{}} }}"
    )).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0755"),
        "{diagnostics:?}"
    );
    assert!(doriac::check_source("readonly-flush.doria", format!(
        "{imports} function inspect(FlushableStream $stream): FlushOutcome throws IoError {{ return $stream->flush(); }}"
    )).is_err());
}

#[test]
fn concrete_and_erased_streams_reuse_regions_and_preserve_every_outcome() {
    let program = lower(SOURCE);
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, EXPECTED.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, EXPECTED);
    }
    assert_php_execution(SOURCE, EXPECTED);
    assert_cranelift_functions_do_not_allocate(&program, &["forwardRead", "forwardWrite"]);
    #[cfg(feature = "llvm-backend")]
    assert_llvm_functions_do_not_allocate(&program, &["forwardRead", "forwardWrite"]);
}

#[test]
fn nested_concrete_adapters_keep_static_calls_and_erasure_adds_only_one_dispatch() {
    use doriac::mir::{IndirectCallee, Terminator};

    let source = r#"
use Doria\Std\Io\{File, FileCreation, FileWriteMode, WritableFile, ByteRegion, WritableStream, ReadOutcome, WriteOutcome, IoError};

class MeteredWriter<S implements WritableStream> implements WritableStream {
    internal writable S $inner;
    writable int $calls = 0;

    function __construct(parameter take S $inner) { $this->inner = $inner; }

    writable function write(ByteRegion $source): WriteOutcome throws IoError {
        $this->calls++;
        return $this->inner->write($source);
    }
}

function writeConcrete<S implements WritableStream>(writable S $stream, ByteRegion $source): WriteOutcome throws IoError {
    return $stream->write($source);
}
function writeErased(writable WritableStream $stream, ByteRegion $source): WriteOutcome throws IoError {
    return $stream->write($source);
}
function checkFile(string $path, Bytes $seed): void {
    let writable $input = File::openRead($path);
    writable Bytes $actual = Bytes::allocate(4);
    let writable $total = 0;
    let writable $seedIndex = 0;
    while (true) {
        let writable $count = 0;
        {
            let writable $destination = $actual->getWritableRegion(0, 4);
            $count = match ($input->read($destination)) {
                ReadOutcome::Data($byteCount) => $byteCount,
                ReadOutcome::EndOfStream => 0,
                default => -1,
            };
        }
        if ($count == 0) { break; }
        if ($count < 0) { panic("unexpected read outcome"); }
        for (let writable $index = 0; $index < $count; $index++) {
            if ($actual[$index] != $seed[$seedIndex]) { panic("wrong byte"); }
            $seedIndex++;
            if ($seedIndex == 4) { $seedIndex = 0; }
        }
        $total += $count;
    }
    $input->close();
    if ($total != 128) { panic("wrong length"); }
    echo "verified\n";
}
function main(): void {
    writable Bytes $buffer = Bytes::allocate(4);
    $buffer[0] = 65;
    $buffer[1] = 0;
    $buffer[2] = 255;
    $buffer[3] = 10;
    let $payload = $buffer->getRegion(0, 4);
    {
        let writable $writer = new MeteredWriter<MeteredWriter<WritableFile>>(new MeteredWriter<WritableFile>(
            File::openWrite("concrete.bin", FileCreation::CreateNew, FileWriteMode::Positioned)
        ));
        for (let writable $index = 0; $index < 32; $index++) {
            if (writeConcrete($writer, $payload) != WriteOutcome::Accepted(4)) { panic("wrong concrete outcome"); }
        }
        echo "concrete {$writer->calls}\n";
    }
    checkFile("concrete.bin", $buffer);
    {
        let writable $writer = new MeteredWriter<MeteredWriter<WritableFile>>(new MeteredWriter<WritableFile>(
            File::openWrite("erased.bin", FileCreation::CreateNew, FileWriteMode::Positioned)
        ));
        for (let writable $index = 0; $index < 32; $index++) {
            if (writeErased($writer, $payload) != WriteOutcome::Accepted(4)) { panic("wrong erased outcome"); }
        }
        echo "erased {$writer->calls}\n";
    }
    checkFile("erased.bin", $buffer);
}
"#;
    let expected = "concrete 32\nverified\nerased 32\nverified\n";
    let program = lower(source);
    let interpreted =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(interpreted.output.stdout, expected.as_bytes());
    assert!(interpreted.output.stderr.is_empty());
    assert_eq!(interpreted.output.exit_status, 0);
    assert_eq!(interpreted.trace.file_opens, 4);
    assert_eq!(interpreted.trace.file_writes, 64);
    assert_eq!(interpreted.trace.file_close_attempts, 4);
    for path in ["concrete.bin", "erased.bin"] {
        assert_eq!(interpreted.files[path], [65, 0, 255, 10].repeat(32));
    }
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);

    let concrete = program
        .functions
        .iter()
        .filter(|function| {
            function.name.starts_with("writeConcrete")
                || function.method.as_ref().is_some_and(|method| {
                    method.name == "write"
                        && program.classes[method.class.0]
                            .name
                            .starts_with("MeteredWriter")
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(concrete.len(), 3, "two adapters and the generic caller");
    for function in &concrete {
        assert!(
            function.blocks.iter().all(|block| !matches!(
                block.terminator,
                Terminator::IndirectCall { .. } | Terminator::CheckedIndirectCall { .. }
            )),
            "concrete adapter must stay specialized: {}",
            function.name
        );
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator, Terminator::CheckedCall { .. }))
                .count(),
            1,
            "one forwarded call in {}",
            function.name
        );
    }
    let erased = program
        .functions
        .iter()
        .find(|function| function.name == "writeErased")
        .unwrap();
    assert_eq!(
        erased
            .blocks
            .iter()
            .filter(|block| matches!(
                block.terminator,
                Terminator::CheckedIndirectCall {
                    callee: IndirectCallee::InterfaceMethod { .. },
                    ..
                }
            ))
            .count(),
        1
    );
    let names = concrete
        .iter()
        .map(|function| function.name.as_str())
        .chain(std::iter::once(erased.name.as_str()))
        .collect::<Vec<_>>();
    assert_cranelift_functions_do_not_allocate(&program, &names);
    #[cfg(feature = "llvm-backend")]
    assert_llvm_functions_do_not_allocate(&program, &names);
}

#[test]
fn caller_owned_offset_preserves_partial_writes_and_retries_without_copying_suffixes() {
    let source = include_str!("../../../examples/native/main_stream_partial_write.doria");
    let expected = include_str!("fixtures/native_io/main_stream_partial_write/expected_stdout");
    let program = lower(source);
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_cranelift_functions_do_not_allocate(&program, &["writeNextPart"]);
    #[cfg(feature = "llvm-backend")]
    assert_llvm_functions_do_not_allocate(&program, &["writeNextPart"]);

    assert_php_execution(source, expected);
}

#[test]
fn owned_registration_transfers_returns_and_drops_erased_streams_once() {
    let source = include_str!("../../../examples/native/main_stream_owned_registration.doria");
    let expected =
        include_str!("fixtures/native_io/main_stream_owned_registration/expected_stdout");
    let program = lower(source);
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);
}

#[test]
fn typed_registration_returns_complete_file_owners_and_cleans_up_rejected_values() {
    let source = include_str!("../../../examples/native/main_stream_typed_registration.doria");
    let expected =
        include_str!("fixtures/native_io/main_stream_typed_registration/expected_stdout");
    let program = lower(source);
    let result = doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(result.output.stdout, expected.as_bytes());
    assert!(result.output.stderr.is_empty());
    assert_eq!(result.output.exit_status, 0);
    assert_eq!(result.files["registered.txt"], b"abc\n");
    assert_eq!(result.trace.file_opens, 5);
    assert_eq!(result.trace.file_reads, 3);
    assert_eq!(result.trace.file_close_attempts, 5);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    // This ordinary generic control is not the public native-backed readiness set.
    let diagnostics = doriac::compile_source_to_php("typed-registration.doria", source)
        .expect_err("PHP retains its array-backed Dictionary method boundary");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "B2301" && diagnostic.message.contains("containsKey")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn readiness_access_borrows_original_concrete_and_erased_streams() {
    let source = include_str!("../../../examples/native/main_readiness_access.doria");
    let expected = include_str!("fixtures/native_io/main_readiness_access/expected_stdout");
    let program = lower(source);
    let result = doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(result.output.stdout, expected.as_bytes());
    assert!(result.output.stderr.is_empty());
    assert_eq!(result.output.exit_status, 0);
    assert_eq!(result.trace.file_opens, 2);
    assert_eq!(result.trace.file_reads, 3);
    assert_eq!(result.trace.file_close_attempts, 2);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);
}

#[test]
fn typed_registration_rejects_a_key_for_another_registered_type() {
    let source = include_str!("../../../examples/native/main_stream_typed_registration.doria");
    let source = source.replace(
        "$registry->remove($second)",
        "$registry->remove(new FixtureKey<Doria\\Std\\Io\\WritableFile>($second->name))",
    );
    let diagnostics = doriac::check_source("wrong-registration-key.doria", source).unwrap_err();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0408"
                && diagnostic.message.contains("FixtureKey<")
                && diagnostic.message.contains("ReadableFile")
                && diagnostic.message.contains("WritableFile")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn failed_registration_does_not_restore_the_callers_moved_file() {
    let source = include_str!("../../../examples/native/main_stream_typed_registration.doria");
    let source = source.replace(
        "try { $registry->register(\"first\", File::openRead(\"registered.txt\")); }",
        "let $rejected = File::openRead(\"registered.txt\");\ntry { $registry->register(\"first\", $rejected); }",
    ).replace(
        "report($registry->remove($first), $buffer, \"first\");",
        "$rejected->close();\nreport($registry->remove($first), $buffer, \"first\");",
    );
    let diagnostics = doriac::check_source("failed-registration-move.doria", source).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"),
        "{diagnostics:#?}"
    );
}

#[test]
fn owned_registration_does_not_leave_a_callable_stream_alias() {
    let source = include_str!("../../../examples/native/main_stream_owned_registration.doria");
    let source = source.replace("let $first =", "let writable $first =").replace(
        "$registry->register($first);",
        "$registry->register($first);\nlet writable $destination = $buffer->getWritableRegion(0, 1);\n$first->read($destination);",
    );
    let (_, analysis) = doriac::analyze_source_for_ide("moved-stream.doria", &source)
        .expect("the ownership regression must parse and type-check");
    let diagnostics = analysis.diagnostics;
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"),
        "{diagnostics:#?}"
    );
}

#[test]
fn owned_stream_temporaries_preserve_exclusive_access_and_statement_cleanup() {
    let source =
        include_str!("../../../examples/native/main_stream_owned_temporary_receivers.doria");
    let expected =
        include_str!("fixtures/native_io/main_stream_owned_temporary_receivers/expected_stdout");
    let program = lower(source);
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected.as_bytes());
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(source, expected);
}

#[test]
fn readonly_stream_results_do_not_gain_exclusive_temporary_access() {
    for receiver in ["borrowReader($reader)", "(new Holder())->reader"] {
        let source = format!(
            r#"
use Doria\Std\Io\{{ReadableStream, WritableByteRegion, ReadOutcome}};
class Reader implements ReadableStream {{
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        return ReadOutcome::Data(0);
    }}
}}
class Holder {{ ReadableStream $reader = new Reader(); }}
function borrowReader(ReadableStream $reader): ReadableStream {{ return $reader; }}
function main(): void {{
    let writable $reader = new Reader();
    writable Bytes $buffer = Bytes::allocate(1);
    let writable $destination = $buffer->getWritableRegion(0, 1);
    {receiver}->read($destination);
}}
"#
        );
        let diagnostics = doriac::check_source("readonly-stream-result.doria", source)
            .expect_err("readonly returned borrows and properties cannot become writable");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0203"),
            "{receiver}: {diagnostics:?}"
        );
    }
}

#[test]
fn writable_interface_calls_preserve_nested_collection_property_access() {
    for (ty, values, index) in [
        ("List<ReadableStream>", "[new Reader()]", "0"),
        ("ReadableStream[]", "[new Reader()]", "0"),
        (
            "Dictionary<string, ReadableStream>",
            "[\"reader\" => new Reader()]",
            "\"reader\"",
        ),
        ("List<List<ReadableStream>>", "[[new Reader()]]", "0][0"),
    ] {
        let source = format!(
            r#"
use Doria\Std\Io\{{ReadableStream, WritableByteRegion, ReadOutcome}};
class Reader implements ReadableStream {{
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        $destination[0] = 7;
        return ReadOutcome::Data(1);
    }}
}}
class Registry {{
    internal writable {ty} $streams = {values};
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        return $this->streams[{index}]->read($destination);
    }}
}}
function main(): void {{
    let writable $registry = new Registry();
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let writable $destination = $buffer->getWritableRegion(0, 1);
        $registry->read($destination);
    }}
    echo $buffer[0];
}}
"#
        );
        let program = lower(&source);
        let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(interpreted.stdout, b"7", "{ty}");
        assert!(interpreted.stderr.is_empty(), "{ty}");
        assert_eq!(interpreted.exit_status, 0, "{ty}");
        for &profile in NativeProfile::AVAILABLE {
            native_execution::assert_native_execution(&program, profile, "7");
        }

        for readonly in [
            source.replace("internal writable", "internal"),
            source.replace(
                "writable function read(writable WritableByteRegion $destination): ReadOutcome {\n        return",
                "function read(writable WritableByteRegion $destination): ReadOutcome {\n        return",
            ),
        ] {
            let diagnostics = doriac::check_source("readonly-registration.doria", readonly)
                .expect_err("readonly collection paths must not grant writable interface access");
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic.code == "E0203"),
                "{ty}: {diagnostics:#?}"
            );
        }
    }
}

#[test]
fn writable_stream_arguments_preserve_nested_collection_property_access() {
    for (ty, values, index) in [
        ("List<ReadableStream>", "[new Reader()]", "0"),
        ("ReadableStream[]", "[new Reader()]", "0"),
        (
            "Dictionary<string, ReadableStream>",
            "[\"reader\" => new Reader()]",
            "\"reader\"",
        ),
        ("List<List<ReadableStream>>", "[[new Reader()]]", "0][0"),
    ] {
        for callable in ["readStream", "$read"] {
            let call = format!("{callable}($this->streams[{index}], $destination)");
            let source = format!(
                r#"
use Doria\Std\Io\{{ReadableStream, WritableByteRegion, ReadOutcome}};
class Reader implements ReadableStream {{
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        $destination[0] = 7;
        return ReadOutcome::Data(1);
    }}
}}
function readStream(writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome {{
    return $stream->read($destination);
}}
class Registry {{
    internal writable {ty} $streams = {values};
    writable function read(writable WritableByteRegion $destination): ReadOutcome {{
        let $read = function (writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome {{
            return $stream->read($destination);
        }};
        return {call};
    }}
}}
function main(): void {{
    let writable $registry = new Registry();
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let writable $destination = $buffer->getWritableRegion(0, 1);
        $registry->read($destination);
    }}
    echo $buffer[0];
}}
"#
            );
            let program = lower(&source);
            let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
            assert_eq!(interpreted.stdout, b"7", "{call}");
            assert!(interpreted.stderr.is_empty(), "{call}");
            assert_eq!(interpreted.exit_status, 0, "{call}");
            for &profile in NativeProfile::AVAILABLE {
                native_execution::assert_native_execution(&program, profile, "7");
            }

            for readonly in [
            source.replace("internal writable", "internal"),
            source.replace(
                "writable function read(writable WritableByteRegion $destination): ReadOutcome {\n        let",
                "function read(writable WritableByteRegion $destination): ReadOutcome {\n        let",
            ),
        ] {
                let (_, analysis) =
                    doriac::analyze_source_for_ide("readonly-stream-argument.doria", readonly)
                        .expect("readonly controls use accepted grammar");
                let expected_code = if callable == "$read" { "E0651" } else { "E0479" };
                assert!(
                    analysis
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == expected_code),
                "{call}: {:?}",
                analysis.diagnostics
            );
        }
        }
    }
}

fn assert_php_execution(source: &str, expected: &str) -> native_execution::ExecutionDirectory {
    let directory = native_execution::ExecutionDirectory::new();
    assert_php_execution_in_directory(source, expected, &directory);
    directory
}

fn assert_php_execution_in_directory(
    source: &str,
    expected: &str,
    directory: &native_execution::ExecutionDirectory,
) {
    let artifact =
        doriac::compile_source("stream-capabilities.doria", source, BackendTarget::Php).unwrap();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .current_dir(directory.path())
        .arg(path)
        .output()
        .expect("PHP is required for stream compatibility parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn capabilities_resolve_through_aliases_without_unqualified_aliases() {
    let source = r#"
use Doria\Std\Io\ReadableStream as Reader;
use Doria\Std\Io\ReadOutcome as Progress;
use Doria\Std\Io\{WritableByteRegion, IoError};
function read(writable Reader $stream, writable WritableByteRegion $destination): Progress throws IoError {
    return $stream->read($destination);
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("aliased-stream.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for name in [
        "ReadOutcome",
        "WriteOutcome",
        "FlushOutcome",
        "ReadableStream",
        "WritableStream",
        "FlushableStream",
    ] {
        let source = format!("function inspect({name} $value): void {{}}");
        assert!(doriac::check_source("unqualified-stream.doria", source).is_err());
    }
}

#[test]
fn stream_names_cannot_be_redeclared_as_any_nominal_kind() {
    for canonical in doriac::compiler_known_stream::CANONICAL_TYPES {
        let (namespace, name) = canonical.rsplit_once('\\').unwrap();
        for kind in ["class", "interface", "enum", "trait"] {
            let source = format!("namespace {namespace}; {kind} {name} {{}}");
            let diagnostics = doriac::check_source("reserved-stream.doria", &source).unwrap_err();
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0640"),
                "{source}: {diagnostics:?}"
            );
        }
    }
}

#[test]
fn read_contract_preserves_receiver_variance_and_exact_storage_modes() {
    let imports = "use Doria\\Std\\Io\\{ReadableStream, ReadOutcome, WritableByteRegion, IoError};";
    doriac::check_source("readonly-implementation.doria", format!(
        "{imports} class ReadonlySource implements ReadableStream {{ function read(writable WritableByteRegion $destination): ReadOutcome throws IoError {{ return ReadOutcome::EndOfStream; }} }}"
    )).expect("a readonly implementation may satisfy a writable requirement under Decision 0134");
    for signature in [
        "writable function read(WritableByteRegion $destination): ReadOutcome throws IoError",
        "writable function read(take WritableByteRegion $destination): ReadOutcome throws IoError",
    ] {
        let source = format!(
            "{imports} class Wrong implements ReadableStream {{ {signature} {{ return ReadOutcome::EndOfStream; }} }}"
        );
        let diagnostics = doriac::check_source("wrong-stream-mode.doria", &source).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.code.starts_with('P')),
            "{source}: {diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0755"),
            "{source}: {diagnostics:?}"
        );
    }
    let source = format!(
        "{imports} function inspect(ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome throws IoError {{ return $stream->read($destination); }}"
    );
    assert!(doriac::check_source("readonly-stream.doria", source).is_err());
}

#[test]
fn stream_io_failures_remain_ambient_with_checked_runtime_transport() {
    let source = r#"
use Doria\Std\Io\{ReadableStream, ReadOutcome, WritableByteRegion};
function read(writable ReadableStream $stream, writable WritableByteRegion $destination): ReadOutcome {
    return $stream->read($destination);
}
"#;
    doriac::check_source("ambient-stream.doria", source).unwrap();
    let program = lower(&format!("{source}\nfunction main(): void {{}}"));
    let io_error = program
        .error_descriptors
        .iter()
        .find(|descriptor| descriptor.type_name == doriac::compiler_known_io::IO_ERROR)
        .unwrap()
        .id;
    for signature in &program.function_types {
        assert!(!signature
            .checked_effects
            .contains(&doriac::mir::CheckedEffect::Concrete(io_error)));
        assert!(signature
            .ambient_checked_effects
            .contains(&doriac::mir::CheckedEffect::Concrete(io_error)));
    }
}

#[test]
fn cranelift_outcome_construction_and_transport_do_not_reference_heap_operations() {
    assert_cranelift_functions_do_not_allocate(
        &lower(OUTCOME_SOURCE),
        &[
            "data",
            "accepted",
            "retry",
            "reuse",
            "flushed",
            "pendingFlush",
            "reuseFlush",
            "main",
        ],
    );
}

#[test]
fn transfer_allocation_guards_detect_allocations_and_copies_in_forwarded_helpers() {
    let program = lower(
        r#"
function allocateInCallee(int $size): int {
    let $bytes = Bytes::allocate($size);
    return $bytes->length;
}
function forwardAllocation(int $size): int {
    return allocateInCallee($size);
}
function copyInCallee(Bytes $bytes): int {
    let $copy = $bytes->toArray();
    return $copy->length;
}
function forwardCopy(Bytes $bytes): int {
    return copyInCallee($bytes);
}
function main(): void {
    if (forwardAllocation(4) != 4) { panic("wrong length"); }
    let $bytes = Bytes::allocate(4);
    if (forwardCopy($bytes) != 4) { panic("wrong copy length"); }
}
"#,
    );
    for name in ["forwardAllocation", "forwardCopy"] {
        assert!(std::panic::catch_unwind(|| {
            assert_cranelift_functions_do_not_allocate(&program, &[name]);
        })
        .is_err());
        #[cfg(feature = "llvm-backend")]
        assert!(std::panic::catch_unwind(|| {
            assert_llvm_functions_do_not_allocate(&program, &[name]);
        })
        .is_err());
    }
}

fn assert_cranelift_functions_do_not_allocate(program: &doriac::mir::Program, names: &[&str]) {
    assert_cranelift_runtime_references(program, names, &[]);
}

fn assert_cranelift_runtime_references(
    program: &doriac::mir::Program,
    names: &[&str],
    required: &[&str],
) {
    let graph = native_structure::cranelift_references(program);
    assert_transfer_runtime_references(program, names, required, &graph);
}

fn assert_transfer_runtime_references(
    program: &doriac::mir::Program,
    names: &[&str],
    required: &[&str],
    graph: &native_structure::FunctionReferences,
) {
    let mut graph = graph.clone();
    // Checked file bodies materialize IoError on failure. Runtime probes cover
    // their ordinary transfer path; this guard follows authored forwarding code
    // up to that compiler-owned boundary, not its permitted Error allocation.
    for function in &program.functions {
        if function.method.as_ref().is_some_and(|method| {
            program.classes[method.class.0].runtime_payload
                == Some(doriac::mir::RuntimeClassPayload::OwnedFile)
        }) && function
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, doriac::mir::Terminator::CheckedIo { .. }))
        {
            let symbol = doriac::native_abi::function_symbol(function);
            graph.remove(symbol.trim_start_matches('_'));
        }
    }
    for symbol in native_structure::authored_symbols(program, names) {
        let references = native_structure::reachable_references(&graph, vec![symbol.clone()]);
        for target in &references {
            // Erased calls may invoke authored bodies with their own costs;
            // this guard covers generated direct helpers, not runtime counts.
            assert_no_transfer_allocation(target);
            assert!(
                !target.starts_with("unresolved text target at"),
                "cannot establish helper reachability for {symbol}: {target}"
            );
        }
        for required in required {
            assert!(
                graph[&symbol].contains(*required),
                "{symbol} does not use its runtime-owned projection {required}: {:?}",
                graph[&symbol]
            );
        }
    }
}

fn assert_no_transfer_allocation(target: &str) {
    use doriac::native_abi::*;
    for forbidden in [
        CLASS_ALLOCATE,
        CLOSURE_ENVIRONMENT_ALLOCATE,
        COLLECTION_NEW,
        COLLECTION_ITERATOR_NEW,
        COLLECTION_STAGE26_NEW,
        COLLECTION_AGGREGATE_NEW,
        CORE_COLLECTION_NEW,
        COLLECTION_STAGE26_FROM_COPY,
        MIXED_NEW,
        MIXED_NEW_BORROWED,
        MIXED_NEW_AGGREGATE,
        MIXED_NEW_AGGREGATE_BORROWED,
        BYTES_ALLOCATE,
        BYTES_COPY,
        BYTES_FROM_COLLECTION,
        BYTES_TO_COLLECTION,
        SHARED_RETAIN,
        WRITABLE_SHARED_RETAIN,
    ] {
        assert_ne!(target, forbidden, "transfer helper references {forbidden}");
    }
}

#[cfg(feature = "llvm-backend")]
#[test]
fn llvm_outcome_construction_and_transport_do_not_reference_heap_operations() {
    assert_llvm_functions_do_not_allocate(
        &lower(OUTCOME_SOURCE),
        &[
            "data",
            "accepted",
            "retry",
            "reuse",
            "flushed",
            "pendingFlush",
            "reuseFlush",
            "main",
        ],
    );
}

#[cfg(feature = "llvm-backend")]
fn assert_llvm_functions_do_not_allocate(program: &doriac::mir::Program, names: &[&str]) {
    assert_llvm_runtime_references(program, names, &[]);
}

#[cfg(feature = "llvm-backend")]
fn assert_llvm_runtime_references(
    program: &doriac::mir::Program,
    names: &[&str],
    required: &[&str],
) {
    let (_, graph) = native_structure::llvm_references(program);
    assert_transfer_runtime_references(program, names, required, &graph);
}
