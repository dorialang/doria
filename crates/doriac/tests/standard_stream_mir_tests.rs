#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;
#[path = "common/native_structure.rs"]
mod native_structure;

use doriac::backend::NativeProfile;
use doriac::mir::{self, CheckedIoOperation, StandardIoStream};
use doriac::mir_interpreter::{interpret_with_io, MirIo, MirIoFaults, MirIoWriteFailure};

fn standard_program(stream: StandardIoStream, body: &str) -> mir::Program {
    transfer_program(stream, body, true)
}

fn transfer_program(stream: StandardIoStream, body: &str, standard: bool) -> mir::Program {
    transfer_program_with_declarations(stream, body, standard, "")
}

fn transfer_program_with_declarations(
    stream: StandardIoStream,
    body: &str,
    standard: bool,
    declarations: &str,
) -> mir::Program {
    let source = format!(
        r#"
use Doria\Std\Io\{{File, FileCreation, FileWriteMode, ReadableFile, WritableFile,
    ReadOutcome, WriteOutcome, WritableByteRegion, ByteRegion, IoError, IoOperation,
    IoTarget, IoErrorReason, ReadinessSet, TransferPlan, Wait, TransferWaitResult,
    TransferCompletion}};
function receive(writable ReadableFile $file, writable WritableByteRegion $region): ReadOutcome throws IoError {{
    return $file->read($region);
}}
function send(writable WritableFile $file, ByteRegion $region): WriteOutcome throws IoError {{
    return $file->write($region);
}}
{declarations}
function main(): void throws IoError {{
    write_file("unused-input", "");
    writable ReadableFile $reader = File::openRead("unused-input");
    writable WritableFile $writer = File::openWrite("unused-output", FileCreation::CreateOrTruncate, FileWriteMode::Positioned);
    writable Bytes $bytes = Bytes::allocate(5);
    $bytes[0] = 7; $bytes[1] = 255; $bytes[2] = 0; $bytes[3] = 81; $bytes[4] = 9;
    {body}
}}
"#
    );
    let mut program = doriac::lower_source_to_mir("standard-stream-mir.doria", &source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    if !standard {
        return program;
    }
    let (reads, writes) = retarget_standard_transfers(&mut program, stream);
    assert!(reads != 0 && writes != 0);
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

fn retarget_standard_transfers(
    program: &mut mir::Program,
    stream: StandardIoStream,
) -> (usize, usize) {
    let (mut reads, mut writes) = (0, 0);
    // Isolate private transfer operations in this file-shaped template. Public
    // standard-view regressions below lower their authored declarations directly.
    for function in &mut program.functions {
        for block in &mut function.blocks {
            let mir::Terminator::CheckedIo { operation, .. } = &mut block.terminator else {
                continue;
            };
            *operation = match *operation {
                CheckedIoOperation::ReadOwnedFile {
                    destination,
                    outcome,
                    ..
                } => {
                    reads += 1;
                    CheckedIoOperation::TransferStandardBytes {
                        stream: StandardIoStream::Input,
                        region: destination,
                        outcome,
                    }
                }
                CheckedIoOperation::WriteOwnedFile {
                    source, outcome, ..
                } => {
                    writes += 1;
                    CheckedIoOperation::TransferStandardBytes {
                        stream,
                        region: source,
                        outcome,
                    }
                }
                _ => continue,
            };
        }
    }
    (reads, writes)
}

fn standard_pending_program(stream: StandardIoStream, body: &str) -> mir::Program {
    // Programs include only the standard declarations they name (decision
    // log entry 41): name the inline standard views these mutations target.
    standard_pending_program_with_declarations(
        stream,
        body,
        "function standardViews(Doria\\Std\\Io\\StandardInput $input, Doria\\Std\\Io\\StandardOutput $output, Doria\\Std\\Io\\StandardError $error): void {}\n",
    )
}

fn standard_pending_program_with_declarations(
    stream: StandardIoStream,
    body: &str,
    declarations: &str,
) -> mir::Program {
    let mut program = transfer_program_with_declarations(stream, body, false, declarations);
    retarget_standard_transfers(&mut program, stream);
    let mut projections = 0;
    let mut selections = 0;
    for function in &mut program.functions {
        for block in &mut function.blocks {
            // These private MIR fixtures exercise the real opaque-source and
            // pending ABI through file-shaped fixture receivers as well as
            // the separately lowered public inline standard views.
            if let mir::Terminator::Return(mir::Rvalue::Class(
                value @ mir::ClassExpression::FileReadinessSource { .. },
            )) = &mut block.terminator
            {
                let mir::ClassExpression::FileReadinessSource { class, file } = *value else {
                    unreachable!()
                };
                let mir::Type::Class(owner) = function.locals[file.0].ty else {
                    unreachable!()
                };
                let channel = match program.classes[owner.0].name.as_str() {
                    "Doria\\Std\\Io\\ReadableFile" => StandardIoStream::Input,
                    "Doria\\Std\\Io\\WritableFile" => stream,
                    _ => continue,
                };
                *value = mir::ClassExpression::StandardReadinessSource {
                    class,
                    stream: channel,
                    borrowed_from: file,
                };
                projections += 1;
            }
            if let mir::Terminator::CheckedIo {
                operation: operation @ CheckedIoOperation::PreparePendingFile { .. },
                ..
            } = &mut block.terminator
            {
                let CheckedIoOperation::PreparePendingFile {
                    file,
                    request,
                    region,
                } = *operation
                else {
                    unreachable!()
                };
                let mir::Type::Class(owner) = function.locals[file.0].ty else {
                    unreachable!()
                };
                let channel = match program.classes[owner.0].name.as_str() {
                    "Doria\\Std\\Io\\ReadableFile" => StandardIoStream::Input,
                    "Doria\\Std\\Io\\WritableFile" => stream,
                    _ => continue,
                };
                *operation = CheckedIoOperation::PreparePendingStandard {
                    stream: channel,
                    request,
                    region,
                };
                selections += 1;
            }
        }
    }
    assert_eq!((projections, selections), (2, 2));
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

fn assert_standard_parity(program: &mir::Program, input: &[u8], stdout: &[u8], stderr: &[u8]) {
    let result =
        interpret_with_io(
            program,
            MirIo {
                stdin: input.to_vec(),
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| {
            panic!("{error:?}\n{}", program.functions.iter()
        .filter(|function| function.source_span.source.0 == 0 && function.name != "main")
        .map(|function| function.to_string()).collect::<Vec<_>>().join("\n"))
        });
    assert_eq!(result.output.exit_status, 0, "{result:?}");
    assert_eq!(result.output.stdout, stdout, "{result:?}");
    assert_eq!(result.output.stderr, stderr, "{result:?}");
    for &profile in NativeProfile::AVAILABLE {
        let result = native_execution::execute_native_with_input(program, profile, Some(input));
        assert_eq!(result.status.code(), Some(0), "{profile:?}: {result:?}");
        assert_eq!(result.stdout, stdout, "{profile:?}: {result:?}");
        assert_eq!(result.stderr, stderr, "{profile:?}: {result:?}");
    }
}

fn assert_php_parity(
    hir: &doriac::hir::Program,
    program: &mir::Program,
    input: &[u8],
    stdout: &[u8],
    stderr: &[u8],
) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    let artifact = doriac::codegen_php::generate_artifact(hir, Some(program)).unwrap();
    doriac::backend::publish_output(&path, artifact).unwrap();
    let mut child = Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP is required for standard-stream parity");
    child.stdin.take().unwrap().write_all(input).unwrap();
    let result = child.wait_with_output().unwrap();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert_eq!(result.stdout, stdout, "{result:?}");
    assert_eq!(result.stderr, stderr, "{result:?}");
}

#[test]
fn public_standard_getters_preserve_copy_transport_without_native_owners() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, StandardInput, StandardOutput, StandardError};
class Computed { static int $value { get { return 7; } } }
function input(StandardInput $value): void { echo "i"; }
function output(StandardOutput $value): void { echo "o"; }
function error(StandardError $value): void { echo "e"; }
function same<T>(T $value): T { return $value; }
function main(): void {
    let $input = StandardStreams::input;
    let $inputCopy = $input;
    input($input); input(same($inputCopy));
    let $output = StandardStreams::output;
    let $outputCopy = $output;
    output($output); output(same($outputCopy));
    let $error = StandardStreams::error;
    let $errorCopy = $error;
    error($error); error(same($errorCopy));
    echo Computed::value;
}
"#;
    let hir = doriac::lower_source("standard-getters.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(hir.semantic_info.property_accessor_calls.len(), 4);
    assert!(hir
        .semantic_info
        .property_accessor_calls
        .values()
        .all(|calls| {
            calls
                .getter
                .as_ref()
                .is_none_or(|call| call.receiver_mode.is_none())
        }));
    let program =
        doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let facade = hir
        .semantic_info
        .class_member_surfaces
        .iter()
        .find(|surface| surface.receiver.name == "Doria\\Std\\Io\\StandardStreams")
        .expect("standard facade has a checked member surface");
    for (property, result) in [
        ("input", "StandardInput"),
        ("output", "StandardOutput"),
        ("error", "StandardError"),
    ] {
        let member = facade
            .members
            .iter()
            .find(|member| member.name == property)
            .unwrap();
        let hooks = member
            .hooks
            .as_ref()
            .expect("computed static getter contract");
        assert_eq!(
            hooks.storage,
            Some(doriac::property_hooks::PropertyHookStorage::Computed)
        );
        let getter = hooks.getter.as_ref().unwrap();
        assert!(getter.receiver_mode.is_none());
        assert!(getter.signature.parameters.is_empty());
        assert!(
            matches!(&getter.signature.return_type, doriac::types::ResolvedType::Class(class)
            if class.name == format!("Doria\\Std\\Io\\{result}"))
        );
        assert!(hooks.setter.is_none());
    }
    for name in ["StandardInput", "StandardOutput", "StandardError"] {
        let definition = program
            .classes
            .iter()
            .find(|class| class.name.ends_with(&format!("\\{name}")))
            .unwrap();
        let ty = definition
            .inline_copy_type()
            .expect("standard view is inline Copy");
        assert_eq!(ty.storage_bytes(), 8);
        assert!(!ty.requires_owned_cleanup());
        assert!(definition.runtime_payload.is_none());
        assert!(definition.destructor.is_none());
        assert!(
            program
                .stream_callback(
                    mir::Type::Scalar(mir::ScalarType::Inline(ty)),
                    doriac::compiler_known_stream::StreamOperation::ReadinessSource,
                )
                .is_some(),
            "{name}: {definition:#?}\n{:#?}",
            program.interface_vtables
        );
    }
    assert_standard_parity(&program, b"", b"iiooee7", b"");
    assert_php_parity(&hir, &program, b"", b"iiooee7", b"");
}

#[test]
fn public_standard_interface_views_keep_zero_payload_present() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, StandardInput, ReadinessStream, PendingReadableStream};
function present(?ReadinessStream $stream): bool { return $stream != null; }
function borrowed(ReadinessStream $stream): ReadinessStream { return $stream; }
function optional(?ReadinessStream $stream): ?ReadinessStream { return $stream; }
function erased(): PendingReadableStream { return StandardStreams::input; }
function main(): void {
    let writable $stream = erased();
    if (!present($stream)) { panic("present erased stream"); }
    let $source = $stream->readinessSource;
    ?StandardInput $missing = null;
    if (present($missing)) { panic("absent inline stream"); }
    ?StandardInput $input = StandardStreams::input;
    if (!present($input)) { panic("present inline stream"); }
    let $plain = StandardStreams::input;
    ReadinessStream $owner = $plain;
    let $alias = borrowed($owner);
    if (!present($alias)) { panic("borrowed inline stream"); }
    let $aliasSource = $alias->readinessSource;
    ?ReadinessStream $optionalOwner = $input;
    let $optional = optional($optionalOwner);
    if (!present($optional)) { panic("nullable inline home"); }
    ?ReadinessStream $missingOwner = $missing;
    let $absent = optional($missingOwner);
    if (present($absent)) { panic("absent inline home"); }
    let $nested = borrowed($alias);
    if (!present($nested)) { panic("nested inline home"); }
    echo "present";
}
"#;
    let hir = doriac::lower_source("standard-interface.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let program =
        doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_standard_parity(&program, b"", b"present", b"");
}

#[test]
fn copy_interface_temporary_borrows_cannot_escape_their_statement() {
    let declarations = r#"
use Doria\Std\Io\{StandardStreams, StandardInput, ReadinessStream};
function borrowed(ReadinessStream $stream): ReadinessStream { return $stream; }
function optional(?ReadinessStream $stream): ?ReadinessStream { return $stream; }
function writableBorrow(writable ReadinessStream $stream): ReadinessStream { return $stream; }
function writableOptional(writable ?ReadinessStream $stream): ?ReadinessStream { return $stream; }
function named(bool $ignored, ReadinessStream $stream): ReadinessStream { return $stream; }
function generic<T>(take T $ignored, ReadinessStream $stream): ReadinessStream { return $stream; }
class Lender<T> {
    function lend(ReadinessStream $stream): ReadinessStream { return $stream; }
    function generic<U>(take U $ignored, ReadinessStream $stream): ReadinessStream { return $stream; }
}
class StaticLender {
    static function select(ReadinessStream $stream): ReadinessStream { return $stream; }
}
"#;
    for call in [
        "borrowed($input)",
        "borrowed(($input))",
        "borrowed(StandardStreams::input)",
        "optional($input)",
        "optional($present)",
        "optional($missing)",
        "writableBorrow($input)",
        "writableOptional($input)",
        "named(stream: $input, ignored: false)",
        "generic(stream: $input, ignored: 1)",
        "$lender->lend($input)",
        "StaticLender::select($input)",
        "$lender->generic(stream: $input, ignored: 1)",
        "$select($input)",
        "borrowed($input)->readinessSource",
    ] {
        let source = format!(
            r#"{declarations}
function main(): void {{
    let writable $input = StandardStreams::input;
    ?StandardInput $present = $input;
    ?StandardInput $missing = null;
    let $lender = new Lender<int>();
    let $select = function(ReadinessStream $stream): ReadinessStream {{ return $stream; }};
    let $view = {call};
}}
"#
        );
        let (_, analysis) =
            doriac::analyze_source_for_ide("copy-interface-temporary.doria", &source)
                .unwrap_or_else(|errors| panic!("{call}: {errors:#?}"));
        let errors = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.severity == doriac::diagnostics::DiagnosticSeverity::Error
            })
            .collect::<Vec<_>>();
        assert!(
            !errors.is_empty() && errors.iter().all(|error| error.code == "E0478"),
            "{call}: {errors:#?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("temporary")),
            "{call}: {errors:#?}"
        );
    }
}

#[test]
fn copy_interface_temporary_borrows_allow_immediate_calls() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, StandardInput, ReadinessStream};
function borrowed(ReadinessStream $stream): ReadinessStream { return $stream; }
function optional(?ReadinessStream $stream): ?ReadinessStream { return $stream; }
function present(?ReadinessStream $stream): bool { return $stream != null; }
function clear(writable ?ReadinessStream $stream): void { $stream = null; }
function replace(writable ReadinessStream $stream): void { $stream = StandardStreams::input; }
function retain(ReadinessStream $stream): function(): void {
    return function(): void with ($stream) { let $source = $stream->readinessSource; };
}
function main(): void {
    let writable $input = StandardStreams::input;
    ?StandardInput $missing = null;
    if (!present(borrowed($input))) { panic("immediate erased borrow"); }
    if (!present(borrowed(borrowed($input)))) { panic("nested immediate erased borrow"); }
    if (!present(optional($input))) { panic("immediate nullable erasure"); }
    if (present(optional($missing))) { panic("absent nullable erasure"); }
    retain($input)();
    clear($input);
    replace($input);
    if (!present(borrowed($input))) { panic("temporary replacement changed original Copy"); }
    $input = StandardStreams::input;
    ReadinessStream $owner = $input;
    let $view = borrowed($owner);
    $input = StandardStreams::input;
    if (!present($view)) { panic("independent Copy source"); }
    let $callback = retain($owner);
    $input = StandardStreams::input;
    $callback();
}
"#;
    let program = doriac::lower_source_to_mir("copy-interface-immediate.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_standard_parity(&program, b"", b"", b"");
}

#[test]
fn copy_interface_writable_snapshots_preserve_original_in_php_and_native() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, ReadinessStream};
function present(?ReadinessStream $stream): bool { return $stream != null; }
function optional(?ReadinessStream $stream): ?ReadinessStream { return $stream; }
function clear(writable ?ReadinessStream $stream): void { $stream = null; }
function replace(writable ReadinessStream $stream): void { $stream = StandardStreams::input; }
function main(): void {
    let $absent = optional(null);
    if (present($absent)) { panic("owner-free null borrow"); }
    let writable $input = StandardStreams::input;
    clear(stream: $input);
    replace($input);
    if (!present($input)) { panic("direct call changed Copy source"); }
    let $clear = function(writable ?ReadinessStream $stream): void { $stream = null; };
    $clear($input);
    if (!present($input)) { panic("indirect call changed Copy source"); }
    writable ?ReadinessStream $owner = $input;
    clear($owner);
    if (present($owner) || !present($input)) { panic("stored owner replacement"); }
}
"#;
    let hir = doriac::lower_source("copy-interface-snapshot.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let program =
        doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_standard_parity(&program, b"", b"", b"");
    assert_php_parity(&hir, &program, b"", b"", b"");
}

#[test]
fn copy_projection_erasure_returns_independent_owned_interface_snapshots() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, StandardInput, ReadinessStream};
class ConcreteHolder {
    function __construct(StandardInput $stream) {}
    function erased(): ReadinessStream { return $this->stream; }
}
class Holder<T implements ReadinessStream> {
    function __construct(take T $stream) {}
    function erased(): ReadinessStream { return $this->stream; }
}
class OptionalHolder<T implements ReadinessStream> {
    function __construct(take ?T $stream) {}
    function erased(): ?ReadinessStream { return $this->stream; }
}
class Lens<T implements ReadinessStream> {
    function __construct(take T $stream) {}
    T $value { borrowed get { return $this->stream; } }
}
class NestedOwner<T implements ReadinessStream> {
    function __construct(take T $stream) {}
    function erased(): ReadinessStream { return $this->stream; }
}
class Shell<T implements ReadinessStream> {
    function __construct(take ?NestedOwner<T> $nested) {}
}
function project<T implements ReadinessStream>(Holder<T> $holder): ReadinessStream {
    let $local = $holder->erased();
    return $local;
}
function fromGetter<T implements ReadinessStream>(Lens<T> $holder): ReadinessStream {
    return $holder->value;
}
function first<T implements ReadinessStream>(List<T> $streams): ReadinessStream { return $streams[0]; }
function present(?ReadinessStream $stream): bool { return $stream != null; }
function consume(take ReadinessStream $stream): void {}
function consumeNullable(take ?ReadinessStream $stream): void {}
function consumeConcrete(take ConcreteHolder $holder): void {}
function consumeHolder<T implements ReadinessStream>(take Holder<T> $holder): void {}
function consumeOptional<T implements ReadinessStream>(take OptionalHolder<T> $holder): void {}
function consumeLens<T implements ReadinessStream>(take Lens<T> $holder): void {}
function main(): void {
    let $input = StandardStreams::input;
    let $shell = new Shell<StandardInput>(null);
    let $concrete = new ConcreteHolder($input);
    let $concreteOwner = $concrete->erased();
    consumeConcrete($concrete);
    if (!present($concreteOwner)) { panic("concrete field snapshot"); }
    consume($concreteOwner);
    let $holder = new Holder<StandardInput>($input);
    let $fieldOwner = project(holder: $holder);
    consumeHolder($holder);
    if (!present($fieldOwner)) { panic("generic field snapshot"); }
    consume($fieldOwner);
    writable List<StandardInput> $streams = [$input];
    let $elementOwner = first(streams: $streams);
    $streams->clear();
    if (!present($elementOwner)) { panic("element snapshot"); }
    consume($elementOwner);
    let $lens = new Lens<StandardInput>($input);
    let $getterOwner = fromGetter($lens);
    consumeLens($lens);
    if (!present($getterOwner)) { panic("getter snapshot"); }
    consume($getterOwner);
    ?StandardInput $some = $input;
    ?StandardInput $none = null;
    let $optional = new OptionalHolder<StandardInput>($some);
    let $nullableOwner = $optional->erased();
    consumeOptional($optional);
    if (!present($nullableOwner)) { panic("nullable field snapshot"); }
    consumeNullable($nullableOwner);
    let $absent = new OptionalHolder<StandardInput>($none);
    let $absentOwner = $absent->erased();
    consumeOptional($absent);
    if (present($absentOwner)) { panic("absent field snapshot"); }
    consumeNullable($absentOwner);
    ReadinessStream $erased = $input;
    let $moveHolder = new Holder<ReadinessStream>($erased);
    let $fieldLoan = project($moveHolder);
    if (!present($fieldLoan)) { panic("Move field loan"); }
    writable List<ReadinessStream> $moveStreams = [$input];
    let $elementLoan = first($moveStreams);
    if (!present($elementLoan)) { panic("Move element loan"); }
}
"#;
    let hir = doriac::lower_source("copy-projection-erasure.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    for invalid in [
        source.replace(
            "if (!present($fieldLoan))",
            "consumeHolder($moveHolder); if (!present($fieldLoan))",
        ),
        source.replace(
            "if (!present($elementLoan))",
            "$moveStreams->clear(); if (!present($elementLoan))",
        ),
    ] {
        assert!(doriac::check_source("move-projection-live-loan.doria", &invalid).is_err());
    }
    for name in [
        "concreteOwner",
        "fieldOwner",
        "elementOwner",
        "getterOwner",
        "nullableOwner",
        "absentOwner",
        "fieldLoan",
        "elementLoan",
    ] {
        let binding = hir
            .semantic_info
            .binding_resolution
            .declarations_by_id
            .values()
            .find(|binding| binding.name == name)
            .unwrap();
        assert_eq!(
            binding.ownership,
            if name.ends_with("Loan") {
                doriac::symbols::BindingOwnership::ReadonlyBorrow
            } else {
                doriac::symbols::BindingOwnership::Owned
            },
            "{name}"
        );
    }
    let program =
        doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_standard_parity(&program, b"", b"", b"");
    assert_php_parity(&hir, &program, b"", b"", b"");
}

#[test]
fn owned_projection_results_supply_writable_temporary_receivers() {
    let source = r#"
use Doria\Std\Io\{StandardOutput, WritableStream, ByteRegion, IoError};
class OutputHolder<T implements WritableStream> {
    function __construct(take T $stream) {}
    function erased(): WritableStream { return $this->stream; }
}
function mutate(OutputHolder<StandardOutput> $holder, ByteRegion $bytes): void throws IoError {
    $holder->erased()->write($bytes);
}
function main(): void {}
"#;
    let hir = doriac::lower_source("copy-projection-writable-receiver.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let invalid = source.replace(
        "OutputHolder<StandardOutput> $holder",
        "OutputHolder<WritableStream> $holder",
    );
    assert!(doriac::check_source("move-projection-readonly-receiver.doria", &invalid).is_err());
}

#[test]
fn generic_copy_erasure_returns_owned_snapshots_and_preserves_move_loans() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, StandardInput, ReadinessStream};
function erase<T implements ReadinessStream>(T $stream): ReadinessStream { return $stream; }
function forward<T implements ReadinessStream>(T $stream): ReadinessStream {
    let $local = erase($stream);
    return $local;
}
function callbackErase<T implements ReadinessStream>(T $stream): ReadinessStream {
    let $callback = function(T $value): ReadinessStream { return $value; };
    return $callback($stream);
}
function nested<T implements ReadinessStream>(T $stream): ReadinessStream {
    return borrowed($stream);
}
function borrowed(ReadinessStream $stream): ReadinessStream { return $stream; }
function optional<T implements ReadinessStream>(?T $stream): ?ReadinessStream { return $stream; }
function present(?ReadinessStream $stream): bool { return $stream != null; }
function consume(take ReadinessStream $stream): void {}
class Projector<T implements ReadinessStream> {
    function erase(T $stream): ReadinessStream { return $stream; }
}
trait ProjectorMethods {
    function forward<T implements ReadinessStream>(T $stream): ReadinessStream {
        let $local = erase($stream);
        return $local;
    }
}
class TraitProjector { uses ProjectorMethods; }
function main(): void {
    let writable $input = StandardStreams::input;
    let $copyOwner = erase($input);
    $input = StandardStreams::input;
    if (!present($copyOwner)) { panic("generic Copy result"); }
    consume($copyOwner);
    let $forwardOwner = forward($input);
    consume($forwardOwner);
    let $callbackOwner = callbackErase($input);
    consume($callbackOwner);
    if (!present($input)) { panic("original Copy was moved"); }
    ?StandardInput $some = $input;
    ?StandardInput $none = null;
    let $nullableOwner = optional($some);
    let $absentOwner = optional($none);
    if (!present($nullableOwner) || present($absentOwner)) { panic("generic nullable erasure"); }
    let $projector = new Projector<StandardInput>();
    let $methodOwner = $projector->erase($input);
    consume($methodOwner);
    let $traitProjector = new TraitProjector();
    let $traitOwner = $traitProjector->forward($input);
    consume($traitOwner);
    ReadinessStream $owner = $input;
    let $alias = erase($owner);
    if (!present($alias)) { panic("generic Move loan"); }
    let $nestedAlias = nested($owner);
    if (!present($nestedAlias)) { panic("nested generic Move loan"); }
    let $callbackAlias = callbackErase($owner);
    if (!present($callbackAlias)) { panic("generic callback Move loan"); }
    let $traitAlias = $traitProjector->forward($owner);
    if (!present($traitAlias)) { panic("generic trait Move loan"); }
}
"#;
    let hir = doriac::lower_source("generic-copy-erasure.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    for name in [
        "copyOwner",
        "nullableOwner",
        "absentOwner",
        "methodOwner",
        "forwardOwner",
        "callbackOwner",
        "traitOwner",
        "alias",
        "nestedAlias",
        "callbackAlias",
        "traitAlias",
    ] {
        let binding = hir
            .semantic_info
            .binding_resolution
            .declarations_by_id
            .values()
            .find(|binding| binding.name == name)
            .unwrap();
        assert_eq!(
            binding.ownership,
            if matches!(
                name,
                "alias" | "nestedAlias" | "callbackAlias" | "traitAlias"
            ) {
                doriac::symbols::BindingOwnership::ReadonlyBorrow
            } else {
                doriac::symbols::BindingOwnership::Owned
            },
            "{name}"
        );
    }
    let program =
        doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_standard_parity(&program, b"", b"", b"");
    assert_php_parity(&hir, &program, b"", b"", b"");
}

#[test]
fn generic_copy_erasure_rejects_escaping_nested_temporary_borrows() {
    for (body, expected_code) in [
        ("return borrowed($stream);", "E0478"),
        ("let $alias = borrowed($stream); return $alias;", "E0478"),
        ("return borrowed(erase($stream));", "E0478"),
        ("let $callback = function(T $value): ReadinessStream { return borrowed($value); }; return $callback($stream);", "E0478"),
        ("let $callback = fn(T $value) => borrowed($value); return $callback($stream);", "E0478"),
        (
            "let $owner = erase($stream); let $alias = borrowed($owner); return $alias;",
            "E0474",
        ),
    ] {
        let source = format!(
            r#"
use Doria\Std\Io\{{StandardStreams, ReadinessStream}};
function borrowed(ReadinessStream $stream): ReadinessStream {{ return $stream; }}
function erase<T implements ReadinessStream>(T $stream): ReadinessStream {{ return $stream; }}
function relay<T implements ReadinessStream>(T $stream): ReadinessStream {{ {body} }}
function main(): void {{
    let $input = StandardStreams::input;
    let $view = relay($input);
    let $source = $view->readinessSource;
}}
"#
        );
        let diagnostics = match doriac::lower_source("generic-nested-copy-borrow.doria", &source) {
            Err(diagnostics) => diagnostics,
            Ok(_) => panic!("a generic temporary borrow escaped: {body}"),
        };
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == expected_code),
            "{body}: {diagnostics:#?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.code.starts_with('I')
                    && !diagnostic.code.starts_with('B')),
            "{diagnostics:#?}"
        );
    }
}

#[test]
fn generic_trait_copy_erasure_rejects_escaping_temporary_borrows() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, ReadinessStream};
function borrowed(ReadinessStream $stream): ReadinessStream { return $stream; }
trait RelayMethods {
    function relay<T implements ReadinessStream>(T $stream): ReadinessStream {
        return borrowed($stream);
    }
}
class Relay { uses RelayMethods; }
function main(): void {
    let $input = StandardStreams::input;
    let $relay = new Relay();
    let $view = $relay->relay($input);
    let $source = $view->readinessSource;
}
"#;
    let diagnostics = match doriac::lower_source("generic-trait-copy-borrow.doria", source) {
        Err(diagnostics) => diagnostics,
        Ok(_) => panic!("a composed generic method returned a temporary borrow"),
    };
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0478"),
        "{diagnostics:#?}"
    );
}

#[test]
fn retained_closures_cannot_escape_copy_interface_temporaries() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, ReadinessStream};
function retain(ReadinessStream $stream): function(): void {
    return function(): void with ($stream) { let $source = $stream->readinessSource; };
}
function main(): void {
    let $input = StandardStreams::input;
    let $callback = retain($input);
    $callback();
}
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("copy-interface-retained.doria", source).unwrap();
    let errors = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == doriac::diagnostics::DiagnosticSeverity::Error)
        .collect::<Vec<_>>();
    assert!(
        !errors.is_empty() && errors.iter().all(|error| error.code == "E0658"),
        "{errors:#?}"
    );
}

#[test]
fn public_standard_owned_interface_aliases_preserve_zero_presence_and_nullable_storage() {
    let source = r#"
use Doria\Std\Io\{StandardStreams, StandardInput, ReadinessStream, PendingWritableStream,
    WritableStream, ByteRegion, WriteOutcome, IoError};
function present(?ReadinessStream $stream): bool { return $stream != null; }
function borrowed(ReadinessStream $stream): ReadinessStream { return $stream; }
function optional(?ReadinessStream $stream): ?ReadinessStream { return $stream; }
function borrowedWriter(writable PendingWritableStream $stream): PendingWritableStream { return $stream; }
function send<T implements WritableStream>(writable T $stream, ByteRegion $source): WriteOutcome throws IoError {
    return $stream->write($source);
}
function main(): void throws IoError {
    let writable $plain = StandardStreams::input;
    ReadinessStream $owner = $plain;
    let $alias = borrowed($owner);
    $plain = StandardStreams::input;
    if (!present($alias)) { panic("borrowed erased stream"); }
    let $source = $alias->readinessSource;
    ?StandardInput $input = StandardStreams::input;
    ?ReadinessStream $optionalOwner = $input;
    let $optional = optional($optionalOwner);
    if (!present($optional)) { panic("nullable erased home"); }
    ?StandardInput $missing = null;
    ?ReadinessStream $missingOwner = $missing;
    let $absent = optional($missingOwner);
    if (present($absent)) { panic("absent erased home"); }
    let $nested = borrowed($alias);
    if (!present($nested)) { panic("nested erased home"); }
    let writable $writer = StandardStreams::output;
    writable PendingWritableStream $writerOwner = $writer;
    let writable $view = borrowedWriter($writerOwner);
    writable Bytes $bytes = Bytes::allocate(0);
    if (send($view, $bytes->getRegion(0, 0)) != WriteOutcome::Accepted(0)) { panic("borrowed erased writer"); }
}
"#;
    let hir = doriac::lower_source("standard-owned-interface-alias.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let program =
        doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_standard_parity(&program, b"", b"", b"");
}

#[test]
fn standard_pending_groups_share_binary_queue_completion_reuse_and_intrinsic_access() {
    for stream in [StandardIoStream::Output, StandardIoStream::Error] {
        let target = if stream == StandardIoStream::Output {
            "StandardOutput"
        } else {
            "StandardError"
        };
        let intrinsic = if stream == StandardIoStream::Output {
            "write_stdout_bytes"
        } else {
            "write_stderr_bytes"
        };
        let body = format!(
            r#"
    if (read_line() != "prefix") {{ panic("input prefix"); }}
    writable ReadinessSet $set = new ReadinessSet();
    let $readerKey = $set->register(File::openRead("unused-input"));
    let $writerKey = $set->register(File::openWrite("unused-output", FileCreation::OpenExisting, FileWriteMode::Positioned));
    writable Bytes $destination = Bytes::allocate(5);
    $destination[0] = 7; $destination[4] = 9;
    {{
        let $plan = TransferPlan::read($readerKey, $destination->getWritableRegion(1, 3))
            ->andWriteTo($writerKey, $bytes->getRegion(1, 3));
        let $read = $plan->getReadKey(0);
        let $write = $plan->getWriteKey(1);
        let writable $group = $set->begin($plan);
        writable bool $sawRead = false;
        writable bool $sawWrite = false;
        foreach (0..<2 as int $position) {{
            let $completion = $group->getNextCompletion(Wait::Indefinite);
            if ($completion == TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::Data(3)))) {{
                if ($sawRead) {{ panic("read repeated"); }} $sawRead = true;
            }} else if ($completion == TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted(3)))) {{
                if ($sawWrite) {{ panic("write repeated"); }} $sawWrite = true;
            }} else {{ panic("completion changed"); }}
        }}
        if (!$sawRead || !$sawWrite) {{ panic("completion missing"); }}
        try {{ read_line(); panic("stdin claim released early"); }} catch (IoError $error) {{
            if ($error->target != IoTarget::StandardInput || $error->reason != IoErrorReason::InvalidInput
                || $error->transferredByteCount != null) {{ panic("stdin conflict facts"); }}
        }}
        try {{ {intrinsic}($bytes); panic("intrinsic claim bypassed"); }} catch (IoError $error) {{
            if ($error->target != IoTarget::{target} || $error->reason != IoErrorReason::InvalidInput
                || $error->transferredByteCount != null) {{ panic("output conflict facts"); }}
        }}
        try {{ send($writer, $bytes->getRegion(1, 3)); panic("direct claim bypassed"); }} catch (IoError) {{}}
        $group->resubmitWrite($write, 0, 0);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted(0)))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("empty reuse"); }}
    }}
    if ($destination[0] != 7 || $destination[1] != 255 || $destination[2] != 0
        || $destination[3] != 81 || $destination[4] != 9) {{ panic("selected region"); }}
    if (read_line() != "line") {{ panic("shared input queue"); }}
    if (send($writer, $bytes->getRegion(1, 3)) != WriteOutcome::Accepted(3)) {{ panic("claim retained after drop"); }}
    let $returnedReader = $set->unregister($readerKey);
    let $returnedWriter = $set->unregister($writerKey);
"#
        );
        let program = standard_pending_program(stream, &body);
        let bytes = b"\xff\0Q\xff\0Q";
        assert_standard_parity(
            &program,
            b"prefix\n\xff\0Qline\n",
            if stream == StandardIoStream::Output {
                bytes
            } else {
                b""
            },
            if stream == StandardIoStream::Error {
                bytes
            } else {
                b""
            },
        );
    }
}

#[test]
fn public_standard_views_register_transfer_borrow_and_return_concrete_and_erased_values() {
    for erased in [false, true] {
        for stream in [StandardIoStream::Output, StandardIoStream::Error] {
            let (property, target, intrinsic) = if stream == StandardIoStream::Output {
                ("output", "StandardOutput", "write_stdout_bytes")
            } else {
                ("error", "StandardError", "write_stderr_bytes")
            };
            let registration = if erased {
                "PendingReadableStream $ownedReader = $reader; PendingWritableStream $ownedWriter = $writer;\nlet $readerKey = $set->register($ownedReader); let $writerKey = $set->register($ownedWriter);"
            } else {
                "let $readerKey = $set->register($reader); let $writerKey = $set->register($writer);"
            };
            let source = format!(
                r#"
use Doria\Std\Io\{{StandardStreams, PendingReadableStream, PendingWritableStream, WritableStream,
    ByteRegion, ReadinessSet, TransferPlan, Wait, TransferWaitResult, TransferCompletion,
    ReadOutcome, WriteOutcome, IoError, IoTarget, IoErrorReason}};
function send<T implements WritableStream>(writable T $stream, ByteRegion $source): WriteOutcome throws IoError {{
    return $stream->write($source);
}}
function borrowedWriter(writable PendingWritableStream $stream): PendingWritableStream {{ return $stream; }}
function main(): void throws IoError {{
    if (read_line() != "prefix") {{ panic("input prefix"); }}
    let $reader = StandardStreams::input;
    let writable $writer = StandardStreams::{property};
    writable ReadinessSet $set = new ReadinessSet();
    {registration}
    writable Bytes $bytes = Bytes::allocate(5);
    $bytes[0] = 7; $bytes[1] = 255; $bytes[2] = 0; $bytes[3] = 81; $bytes[4] = 9;
    {{
        writable PendingWritableStream $aliasOwner = $writer;
        let writable $alias = borrowedWriter($aliasOwner);
        if (send($alias, $bytes->getRegion(0, 0)) != WriteOutcome::Accepted(0)) {{ panic("borrowed inline writer"); }}
    }}
    {{
        let writable $view = $set->getWritableStream($writerKey);
        if (send($view, $bytes->getRegion(0, 0)) != WriteOutcome::Accepted(0)) {{ panic("writable access"); }}
        let $readonly = $set->getStream($writerKey);
        let $source = $readonly->readinessSource;
    }}
    writable Bytes $destination = Bytes::allocate(5);
    $destination[0] = 7; $destination[4] = 9;
    {{
        let $plan = TransferPlan::read($readerKey, $destination->getWritableRegion(1, 3))
            ->andWriteTo($writerKey, $bytes->getRegion(1, 3));
        let $read = $plan->getReadKey(0);
        let $write = $plan->getWriteKey(1);
        let writable $group = $set->begin($plan);
        writable bool $sawRead = false;
        writable bool $sawWrite = false;
        foreach (0..<2 as int $position) {{
            let $completion = $group->getNextCompletion(Wait::Indefinite);
            if ($completion == TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::Data(3)))) {{
                if ($sawRead) {{ panic("read repeated"); }} $sawRead = true;
            }} else if ($completion == TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted(3)))) {{
                if ($sawWrite) {{ panic("write repeated"); }} $sawWrite = true;
            }} else {{ panic("completion changed"); }}
        }}
        if (!$sawRead || !$sawWrite) {{ panic("completion missing"); }}
        try {{ read_line(); panic("stdin claim released early"); }} catch (IoError $error) {{
            if ($error->target != IoTarget::StandardInput || $error->reason != IoErrorReason::InvalidInput) {{ panic("input conflict"); }}
        }}
        try {{ {intrinsic}($bytes); panic("intrinsic claim bypassed"); }} catch (IoError $error) {{
            if ($error->target != IoTarget::{target} || $error->reason != IoErrorReason::InvalidInput) {{ panic("output conflict"); }}
        }}
        try {{ send($writer, $bytes->getRegion(1, 3)); panic("view claim bypassed"); }} catch (IoError) {{}}
        $group->resubmitWrite($write, 0, 0);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted(0)))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("empty reuse"); }}
    }}
    if ($destination[0] != 7 || $destination[1] != 255 || $destination[2] != 0
        || $destination[3] != 81 || $destination[4] != 9) {{ panic("selected region"); }}
    if (read_line() != "line") {{ panic("shared input queue"); }}
    if (send($writer, $bytes->getRegion(1, 3)) != WriteOutcome::Accepted(3)) {{ panic("claim retained after drop"); }}
    let writable $returnedWriter = $set->unregister($writerKey);
    if (send($returnedWriter, $bytes->getRegion(0, 0)) != WriteOutcome::Accepted(0)) {{ panic("returned value"); }}
    let $returnedReader = $set->unregister($readerKey);
    try {{ $set->getStream($readerKey); panic("stale key"); }} catch (IoError $error) {{
        if ($error->target != IoTarget::ReadinessSet || $error->reason != IoErrorReason::InvalidInput) {{ panic("stale key facts"); }}
    }}
}}
"#
            );
            let hir = doriac::lower_source("public-standard-registration.doria", &source)
                .unwrap_or_else(|errors| panic!("erased={erased}, {stream:?}: {errors:#?}"));
            let program = doriac::mir_lowering::lower_program(&hir)
                .unwrap_or_else(|errors| panic!("erased={erased}, {stream:?}: {errors:#?}"));
            let bytes = b"\xff\0Q\xff\0Q";
            assert_standard_parity(
                &program,
                b"prefix\n\xff\0Qline\n",
                if stream == StandardIoStream::Output {
                    bytes
                } else {
                    b""
                },
                if stream == StandardIoStream::Error {
                    bytes
                } else {
                    b""
                },
            );
            assert_php_parity(
                &hir,
                &program,
                b"prefix\n\xff\0Qline\n",
                if stream == StandardIoStream::Output {
                    bytes
                } else {
                    b""
                },
                if stream == StandardIoStream::Error {
                    bytes
                } else {
                    b""
                },
            );
        }
    }
}

#[test]
fn standard_groups_coalesce_aliases_in_plan_order_and_retain_claims_until_teardown() {
    let program = standard_pending_program(
        StandardIoStream::Output,
        r#"
    writable ReadinessSet $set = new ReadinessSet();
    let $first = $set->register(File::openWrite("unused-output", FileCreation::OpenExisting, FileWriteMode::Positioned));
    let $second = $set->register(File::openWrite("unused-output", FileCreation::OpenExisting, FileWriteMode::Positioned));
    writable ReadinessSet $other = new ReadinessSet();
    let $foreign = $other->register(File::openWrite("unused-output", FileCreation::OpenExisting, FileWriteMode::Positioned));
    {
        let $plan = TransferPlan::writeTo($first, $bytes->getRegion(0, 1))->andWriteTo($second, $bytes->getRegion(1, 3));
        let $firstKey = $plan->getWriteKey(0);
        let $secondKey = $plan->getWriteKey(1);
        let writable $group = $set->begin($plan);
        foreach (0..<2 as int $position) {
            let $completion = $group->getNextCompletion(Wait::Indefinite);
            if ($completion != TransferWaitResult::Completed(TransferCompletion::Write($firstKey, WriteOutcome::Accepted(1)))
                && $completion != TransferWaitResult::Completed(TransferCompletion::Write($secondKey, WriteOutcome::Accepted(3)))) { panic("alias completion"); }
        }
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) { panic("alias completion repeated"); }
        if (send($writer, $bytes->getRegion(0, 0)) != WriteOutcome::Accepted(0)) { panic("empty direct access claimed device"); }
        try {
            let $competing = $other->begin(TransferPlan::writeTo($foreign, $bytes->getRegion(0, 1)));
            panic("competing group admitted");
        } catch (IoError $error) {
            if ($error->target != IoTarget::TransferGroup || $error->reason != IoErrorReason::InvalidInput) { panic("competing claim facts"); }
        }
    }
    {
        let $plan = TransferPlan::writeTo($foreign, $bytes->getRegion(0, 1));
        let $key = $plan->getWriteKey(0);
        let writable $group = $other->begin($plan);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($key, WriteOutcome::Accepted(1)))) { panic("claim was not released"); }
    }
"#,
    );
    assert_standard_parity(&program, b"", b"\x07\xff\0Q\x07", b"");
}

#[test]
fn standard_source_and_pending_mir_validate_nominal_loans_direction_and_selected_storage() {
    let original = standard_pending_program(StandardIoStream::Output, "");
    for mutation in 0..8 {
        let mut program = original.clone();
        if mutation < 6 {
            let inline = mutation >= 3;
            let function = program
                .functions
                .iter_mut()
                .find(|function| {
                    function.blocks.iter().any(|block| {
                        matches!(
                            &block.terminator,
                            mir::Terminator::Return(mir::Rvalue::Class(
                                mir::ClassExpression::StandardReadinessSource { borrowed_from, .. }
                            )) if matches!(function.locals[borrowed_from.0].ty,
                                mir::Type::Scalar(mir::ScalarType::Inline(_))) == inline
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
                        mir::Terminator::Return(mir::Rvalue::Class(
                            mir::ClassExpression::StandardReadinessSource { .. }
                        ))
                    )
                })
                .unwrap();
            let mir::Terminator::Return(mir::Rvalue::Class(
                mir::ClassExpression::StandardReadinessSource {
                    class,
                    borrowed_from,
                    ..
                },
            )) = &mut block.terminator
            else {
                unreachable!()
            };
            match mutation % 3 {
                0 => {
                    let Some(mir::ImplementingType::Class(owner)) = function.locals
                        [borrowed_from.0]
                        .ty
                        .interface_implementing_type()
                    else {
                        unreachable!()
                    };
                    *class = owner;
                }
                1 => *borrowed_from = mir::LocalId(usize::MAX),
                2 => function.locals[borrowed_from.0].ty = mir::Type::Scalar(mir::ScalarType::Bool),
                _ => unreachable!(),
            }
        } else {
            let function = program
                .functions
                .iter_mut()
                .find(|function| {
                    function.blocks.iter().any(|block| {
                        matches!(
                            block.terminator,
                            mir::Terminator::CheckedIo {
                                operation: CheckedIoOperation::PreparePendingStandard {
                                    stream: StandardIoStream::Input,
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
                            operation: CheckedIoOperation::PreparePendingStandard {
                                stream: StandardIoStream::Input,
                                ..
                            },
                            ..
                        }
                    )
                })
                .unwrap();
            let mir::Terminator::CheckedIo {
                operation: CheckedIoOperation::PreparePendingStandard { stream, region, .. },
                ..
            } = &mut block.terminator
            else {
                unreachable!()
            };
            if mutation == 6 {
                *stream = StandardIoStream::Output;
            } else {
                function.locals[region.0].writable = false;
            }
        }
        assert!(
            doriac::mir_validation::validate_program(&program).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn standard_projection_and_pending_lowering_use_canonical_sources_and_configurators() {
    let program = standard_pending_program(StandardIoStream::Output, "");
    let source_roots = program
        .functions
        .iter()
        .filter(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::Return(mir::Rvalue::Class(
                        mir::ClassExpression::StandardReadinessSource { .. }
                    ))
                )
            })
        })
        .map(|function| {
            doriac::native_abi::function_symbol(function)
                .trim_start_matches('_')
                .to_owned()
        })
        .collect::<Vec<_>>();
    let prepare_roots = program
        .functions
        .iter()
        .filter(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::CheckedIo {
                        operation: CheckedIoOperation::PreparePendingStandard { .. },
                        ..
                    }
                )
            })
        })
        .map(|function| {
            doriac::native_abi::function_symbol(function)
                .trim_start_matches('_')
                .to_owned()
        })
        .collect::<Vec<_>>();
    let assert_graph = |graph: &native_structure::FunctionReferences| {
        let sources = native_structure::reachable_references(graph, source_roots.clone());
        assert!(sources.contains(doriac::native_abi::STANDARD_READINESS_SOURCE));
        assert!(sources.contains(doriac::native_abi::STANDARD_READINESS_SOURCE_HOME));
        assert!(!sources.contains(doriac::native_abi::FILE_READINESS_SOURCE));
        assert!(!sources.contains(doriac::native_abi::CLASS_ALLOCATE));
        let preparations = native_structure::reachable_references(graph, prepare_roots.clone());
        assert!(preparations.contains(doriac::native_abi::PENDING_REQUEST_CONFIGURE_STANDARD));
        assert!(!preparations.contains(doriac::native_abi::PENDING_REQUEST_CONFIGURE_FILE));
    };
    assert_graph(&native_structure::cranelift_references(&program));
    #[cfg(feature = "llvm-backend")]
    assert_graph(&native_structure::llvm_references(&program).1);
}

#[test]
fn opaque_standard_sources_preserve_nullable_coalescing_and_borrowed_capture_transport() {
    let declarations = r#"
use Doria\Std\Io\ReadinessSource;
class OptionalSource {
    internal ?ReadableFile $file;
    function __construct(parameter take ?ReadableFile $file) { $this->file = $file; }
    ?ReadinessSource $present { borrowed get { return $this->file?->readinessSource; } }
}
function inspect(ReadinessSource $source): void {}
function inspectNullable(?ReadinessSource $source): void { if ($source != null) { inspect($source); } }
"#;
    let body = r#"
    let $holder = new OptionalSource(File::openRead("unused-input"));
    let $empty = new OptionalSource(null);
    {
        let $source = $holder->present;
        if ($source == null) { panic("source became absent"); }
        let $callback = function(): void with ($source) {
            if ($source != null) { inspect($source); } else { panic("captured source absent"); }
        };
        $callback();
    }
    inspect($holder->present ?? $reader->readinessSource);
    inspect($empty->present ?? $reader->readinessSource);
    inspectNullable($empty->present ?? $holder->present);
    {
        let $stored = $empty->present ?? $holder->present;
        let $read = function(): void with ($stored) {
            if ($stored != null) { inspect($stored); } else { panic("stored source absent"); }
        };
        $read();
        let $direct = $empty->present ?? $reader->readinessSource;
        inspect($direct);
    }
"#;
    for standard in [false, true] {
        let program = if standard {
            standard_pending_program_with_declarations(StandardIoStream::Output, body, declarations)
        } else {
            transfer_program_with_declarations(StandardIoStream::Output, body, false, declarations)
        };
        assert_standard_parity(&program, b"", b"", b"");
    }
}

#[test]
fn queued_standard_input_progress_wins_later_preparation_cancellation() {
    let declarations = r#"
use Doria\Std\Concurrency\{CancellationSource, CancellationToken, CancellationError};
use Doria\Std\Io\{PendingReadableStream, ReadRequest, ReadinessSource};
class CancelPreparation implements PendingReadableStream {
    internal ReadableFile $file;
    internal writable CancellationSource $source;
    function __construct(parameter take ReadableFile $file) {
        $this->file = $file;
        $this->source = new CancellationSource();
    }
    CancellationToken $token { get { return $this->source->token; } }
    ReadinessSource $readinessSource { borrowed get => $this->file->readinessSource; }
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError { return ReadOutcome::Data(0); }
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError {
        try { $this->source->cancel(); } catch (CancellationError) {}
        $request->complete(ReadOutcome::Data(0));
    }
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError { return $request->outcome; }
}
"#;
    for cancel_first in [false, true] {
        let (plan, physical, immediate, outcome, byte, remainder) = if cancel_first {
            ("TransferPlan::read($cancel, $empty->getWritableRegion(0, 0))->andReadFrom($read, $destination->getWritableRegion(0, 1))", 1, 0, "null", 99, "abc")
        } else {
            ("TransferPlan::read($read, $destination->getWritableRegion(0, 1))->andReadFrom($cancel, $empty->getWritableRegion(0, 0))", 0, 1, "ReadOutcome::Data(1)", 97, "bc")
        };
        let body = format!(
            r#"
    if (read_line() != "prefix") {{ panic("prefix"); }}
    writable ReadinessSet $set = new ReadinessSet();
    let $read = $set->register(File::openRead("unused-input"));
    let $adapter = new CancelPreparation(File::openRead("unused-input"));
    let $token = $adapter->token;
    let $cancel = $set->register($adapter);
    writable Bytes $destination = Bytes::allocate(1); $destination[0] = 99;
    writable Bytes $empty = Bytes::allocate(0);
    {{
        let $plan = {plan};
        let $physical = $plan->getReadKey({physical});
        let $immediate = $plan->getReadKey({immediate});
        let writable $group = $set->begin($plan, cancellation: $token);
        writable bool $sawPhysical = false; writable bool $sawImmediate = false;
        foreach (0..<2 as int $position) {{
            let $result = $group->getNextCompletion(Wait::Indefinite);
            if ($result == TransferWaitResult::Completed(TransferCompletion::Read($physical, {outcome}))) {{
                if ($sawPhysical) {{ panic("physical completion repeated"); }} $sawPhysical = true;
            }} else if ($result == TransferWaitResult::Completed(TransferCompletion::Read($immediate, ReadOutcome::Data(0)))) {{
                if ($sawImmediate) {{ panic("immediate completion repeated"); }} $sawImmediate = true;
            }} else {{ panic("cancelled queue outcome"); }}
        }}
        if (!$sawPhysical || !$sawImmediate || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("cancelled completion lost"); }}
    }}
    if ($destination[0] != {byte} || read_line() != "{remainder}") {{ panic("cancelled queue progress"); }}
"#
        );
        let program = standard_pending_program_with_declarations(
            StandardIoStream::Output,
            &body,
            declarations,
        );
        assert_standard_parity(&program, b"prefix\nabc\n", b"", b"");
    }
}

#[test]
fn standard_transfers_share_intrinsic_cursor_and_preserve_binary_eof_empty_and_region_edges() {
    let body = r#"
    if (receive($reader, $bytes->getWritableRegion(1, 3)) != ReadOutcome::Data(3)) { panic("read count"); }
    if ($bytes[0] != 7 || $bytes[1] != 255 || $bytes[2] != 0 || $bytes[3] != 81 || $bytes[4] != 9) { panic("region edges"); }
    if (send($writer, $bytes->getRegion(1, 3)) != WriteOutcome::Accepted(3)) { panic("write count"); }
    if (read_line() != "line") { panic("shared cursor"); }
    if (receive($reader, $bytes->getWritableRegion(1, 1)) != ReadOutcome::EndOfStream) { panic("EOF"); }
    if (receive($reader, $bytes->getWritableRegion(5, 0)) != ReadOutcome::Data(0)) { panic("empty read"); }
    if (send($writer, $bytes->getRegion(5, 0)) != WriteOutcome::Accepted(0)) { panic("empty write"); }
"#;
    for stream in [StandardIoStream::Output, StandardIoStream::Error] {
        let program = standard_program(stream, body);
        let expected_stdout = if stream == StandardIoStream::Output {
            &b"\xff\0Q"[..]
        } else {
            &b""[..]
        };
        let expected_stderr = if stream == StandardIoStream::Error {
            &b"\xff\0Q"[..]
        } else {
            &b""[..]
        };
        let interpreted = interpret_with_io(
            &program,
            MirIo {
                stdin: b"\xff\0Qline\n".to_vec(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(interpreted.output.exit_status, 0, "{interpreted:?}");
        assert_eq!(interpreted.output.stdout, expected_stdout);
        assert_eq!(interpreted.output.stderr, expected_stderr);
        for &profile in NativeProfile::AVAILABLE {
            let output = native_execution::execute_native_with_input(
                &program,
                profile,
                Some(b"\xff\0Qline\n"),
            );
            assert_eq!(output.status.code(), Some(0), "{profile:?}: {output:?}");
            assert_eq!(output.stdout, expected_stdout, "{profile:?}");
            assert_eq!(output.stderr, expected_stderr, "{profile:?}");
        }
    }
}

#[test]
fn standard_transfer_mir_rejects_forged_direction_access_outcomes_and_result_slots() {
    let original = standard_program(StandardIoStream::Output,
        "receive($reader, $bytes->getWritableRegion(1, 3)); send($writer, $bytes->getRegion(1, 3));");
    for mutation in 0..5 {
        let mut program = original.clone();
        let function = program
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedIo {
                            operation: CheckedIoOperation::TransferStandardBytes {
                                stream: StandardIoStream::Input,
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
                        operation: CheckedIoOperation::TransferStandardBytes {
                            stream: StandardIoStream::Input,
                            ..
                        },
                        ..
                    }
                )
            })
            .unwrap();
        let mir::Terminator::CheckedIo {
            operation:
                CheckedIoOperation::TransferStandardBytes {
                    stream,
                    region,
                    outcome,
                },
            result,
            ..
        } = &mut block.terminator
        else {
            unreachable!()
        };
        match mutation {
            0 => *stream = StandardIoStream::Output,
            1 => function.locals[region.0].writable = false,
            2 => {
                outcome.id = program
                    .enums
                    .iter()
                    .find(|definition| definition.name == "Doria\\Std\\Io\\WriteOutcome")
                    .unwrap()
                    .id
            }
            3 => function.locals[result.unwrap().0].synthetic = false,
            4 => function.locals[region.0].ty = mir::Type::Scalar(mir::ScalarType::Bool),
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&program).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn interpreter_known_eof_after_partial_intrinsic_failure_preserves_the_queue_cursor() {
    let program = standard_program(
        StandardIoStream::Output,
        r#"
    try { read_stdin_bytes(); panic("expected partial intrinsic failure"); } catch (IoError) {}
    if (read_line() != null) { panic("known EOF"); }
    if (receive($reader, $bytes->getWritableRegion(0, 1)) != ReadOutcome::EndOfStream) { panic("EOF was lost"); }
    if (send($writer, $bytes->getRegion(0, 0)) != WriteOutcome::Accepted(0)) { panic("empty output"); }
"#,
    );
    let result = interpret_with_io(
        &program,
        MirIo {
            stdin: b"abc".to_vec(),
            faults: MirIoFaults {
                stdin_bytes_read: true,
                transferred_byte_count: Some(3),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.output.exit_status, 0, "{result:?}");
    assert!(result.output.stdout.is_empty() && result.output.stderr.is_empty());
}

#[test]
fn standard_transfer_lowering_calls_the_shared_runtime_without_whole_buffer_intrinsics() {
    let program = standard_program(StandardIoStream::Output,
        "receive($reader, $bytes->getWritableRegion(1, 3)); send($writer, $bytes->getRegion(1, 3));");
    let roots = native_structure::authored_symbols(&program, &["receive", "send"]);
    let graphs = [
        native_structure::cranelift_references(&program),
        #[cfg(feature = "llvm-backend")]
        native_structure::llvm_references(&program).1,
    ];
    for graph in graphs {
        let references = native_structure::reachable_references(&graph, roots.clone());
        assert!(
            references.contains(doriac::native_abi::CHECKED_STANDARD_STREAM_IO),
            "{references:?}"
        );
        for forbidden in [
            doriac::native_abi::READ_STDIN_BYTES,
            doriac::native_abi::WRITE_STDOUT_BYTES,
            doriac::native_abi::WRITE_STDERR_BYTES,
            doriac::native_abi::BYTES_COPY,
            doriac::native_abi::BYTES_ALLOCATE,
        ] {
            assert!(
                !references.contains(forbidden),
                "{forbidden}: {references:?}"
            );
        }
    }
}

#[test]
fn checked_file_and_standard_results_cannot_acquire_owned_copy_outcome_slots() {
    for standard in [false, true] {
        let original = transfer_program(StandardIoStream::Output,
            "receive($reader, $bytes->getWritableRegion(1, 3)); send($writer, $bytes->getRegion(1, 3));", standard);
        doriac::mir_validation::validate_program(&original).unwrap();
        let sites = original
            .functions
            .iter()
            .enumerate()
            .flat_map(|(function_index, function)| {
                function
                    .blocks
                    .iter()
                    .filter_map(move |block| match block.terminator {
                        mir::Terminator::CheckedIo {
                            operation:
                                CheckedIoOperation::ReadOwnedFile { .. }
                                | CheckedIoOperation::WriteOwnedFile { .. }
                                | CheckedIoOperation::TransferStandardBytes { .. },
                            result: Some(result),
                            ..
                        } => Some((function_index, result)),
                        _ => None,
                    })
            })
            .collect::<Vec<_>>();
        assert!(sites.len() >= 2);
        for (function, result) in sites {
            let mut program = original.clone();
            program.functions[function].locals[result.0].owned = true;
            assert!(doriac::mir_validation::validate_program(&program).is_err());
        }
    }
}

#[test]
fn empty_standard_transfers_ignore_faults_and_default_native_input_still_reports_eof() {
    for stream in [StandardIoStream::Output, StandardIoStream::Error] {
        let program = standard_program(
            stream,
            r#"
    if (receive($reader, $bytes->getWritableRegion(5, 0)) != ReadOutcome::Data(0)) { panic("empty read"); }
    if (send($writer, $bytes->getRegion(5, 0)) != WriteOutcome::Accepted(0)) { panic("empty write"); }
    if ($bytes[0] != 7 || $bytes[4] != 9) { panic("empty edges"); }
"#,
        );
        let output = interpret_with_io(
            &program,
            MirIo {
                stdin: b"saved".to_vec(),
                faults: MirIoFaults {
                    stdin_bytes_read: true,
                    stdout_write: Some(MirIoWriteFailure::Other),
                    stderr_write: Some(MirIoWriteFailure::Other),
                    transferred_byte_count: Some(3),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.output.exit_status, 0, "{output:?}");
        assert!(output.output.stdout.is_empty() && output.output.stderr.is_empty());
    }
    let program = standard_program(
        StandardIoStream::Output,
        r#"
    if (receive($reader, $bytes->getWritableRegion(1, 1)) != ReadOutcome::EndOfStream) { panic("default input EOF"); }
    if (receive($reader, $bytes->getWritableRegion(5, 0)) != ReadOutcome::Data(0)) { panic("empty read"); }
    if (send($writer, $bytes->getRegion(5, 0)) != WriteOutcome::Accepted(0)) { panic("empty write"); }
"#,
    );
    native_execution::assert_native_execution(&program, NativeProfile::Fast, "");
}

#[test]
fn interpreter_standard_failures_preserve_prefix_zero_and_absent_progress_and_real_targets() {
    for progress in [None, Some(0), Some(1), Some(3)] {
        let count = progress.unwrap_or(0);
        let expected = progress.map_or("-1".to_owned(), |value| value.to_string());
        let body = format!(
            r#"
    try {{ receive($reader, $bytes->getWritableRegion(1, 3)); panic("missing failure"); }}
    catch (IoError $error) {{
        if ($error->operation != IoOperation::Read || $error->target != IoTarget::StandardInput ||
            $error->reason != IoErrorReason::Other || ($error->systemCode ?? -1) != 0 ||
            ($error->transferredByteCount ?? -1) != {expected}) {{ panic("failure fields"); }}
    }}
    if (read_line() != "{}") {{ panic("failure cursor"); }}
    if ($bytes[0] != 7 || $bytes[4] != 9) {{ panic("failure edges"); }}
    if ($bytes[1] != {first} || $bytes[2] != {second} || $bytes[3] != {third}) {{ panic("failure prefix"); }}
"#,
            &"abc"[count..],
            first = if count >= 1 { 97 } else { 255 },
            second = if count >= 2 { 98 } else { 0 },
            third = if count >= 3 { 99 } else { 81 },
        );
        let program = standard_program(StandardIoStream::Output, &body);
        let output = interpret_with_io(
            &program,
            MirIo {
                stdin: b"abc\n".to_vec(),
                faults: MirIoFaults {
                    stdin_bytes_read: true,
                    system_code: Some(0),
                    transferred_byte_count: progress,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.output.exit_status, 0, "{output:?}");
        assert!(output.output.stdout.is_empty() && output.output.stderr.is_empty());
        for stream in [StandardIoStream::Output, StandardIoStream::Error] {
            let target = if stream == StandardIoStream::Output {
                "StandardOutput"
            } else {
                "StandardError"
            };
            let body = format!(
                r#"
    try {{ send($writer, $bytes->getRegion(1, 3)); panic("missing failure"); }}
    catch (IoError $error) {{
        if ($error->operation != IoOperation::Write || $error->target != IoTarget::{target} ||
            ($error->systemCode ?? -1) != 0 || ($error->transferredByteCount ?? -1) != {expected}) {{ panic("failure fields"); }}
    }}
"#
            );
            let program = standard_program(stream, &body);
            for failure in [MirIoWriteFailure::Other, MirIoWriteFailure::BrokenPipe] {
                let mut faults = MirIoFaults {
                    system_code: Some(0),
                    transferred_byte_count: progress,
                    ..Default::default()
                };
                if stream == StandardIoStream::Output {
                    faults.stdout_write = Some(failure);
                } else {
                    faults.stderr_write = Some(failure);
                }
                let output = interpret_with_io(
                    &program,
                    MirIo {
                        faults,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(output.output.exit_status, 0, "{output:?}");
                assert_eq!(
                    output.output.stdout,
                    if stream == StandardIoStream::Output {
                        &b"\xff\0Q"[..count]
                    } else {
                        b""
                    }
                );
                assert_eq!(
                    output.output.stderr,
                    if stream == StandardIoStream::Error {
                        &b"\xff\0Q"[..count]
                    } else {
                        b""
                    }
                );
            }
        }
    }
}
