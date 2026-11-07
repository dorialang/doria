//! Exercise public membership through ordinary lowering and shared validated MIR.
use crate as doriac;
use doriac::mir;
use doriac::mir_interpreter::{interpret_with_io, InterpreterIoOutput, MirIo};

use crate::execution_directory;

const PRELUDE: &str = r#"
use Doria\Std\Io\{ReadinessSet, ReadinessStream, ReadinessSource, RegistrationKey,
    ReadableFile, File, IoError, IoOperation, IoTarget, IoErrorReason};
class ReadyOnly implements ReadinessStream {
    internal ReadableFile $source;
    function __construct(parameter take ReadableFile $source, string $label) {
        $this->source = $source;
    }
    ReadinessSource $readinessSource { borrowed get => $this->source->readinessSource; }
    function getLabel(): string { return $this->label; }
    function __destruct() { try { echo "drop {$this->label}\n"; } catch (IoError) {} }
}
"#;

fn lower(source: &str) -> mir::Program {
    let source = format!("{PRELUDE}\n{source}");
    let program = doriac::lower_source_to_mir("membership-mir.doria", &source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

fn execute(source: &str) -> InterpreterIoOutput {
    let program = lower(source);
    let output = interpret_with_io(
        &program,
        MirIo {
            files: [("input".into(), b"ABC".to_vec())].into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output.output.exit_status, 0, "{output:#?}");
    assert!(output.output.stderr.is_empty(), "{output:#?}");
    for &profile in doriac::backend::NativeProfile::AVAILABLE {
        assert_native_matches(&program, &output.output, profile);
    }
    assert_php_matches(source, &output.output);
    output
}

fn assert_php_matches(source: &str, expected: &doriac::mir_interpreter::InterpreterOutput) {
    let artifact = doriac::compile_source(
        "membership-mir.doria",
        format!("{PRELUDE}\n{source}"),
        doriac::backend::BackendTarget::Php,
    )
    .unwrap();
    let directory = execution_directory::ExecutionDirectory::new();
    std::fs::write(directory.path().join("input"), b"ABC").unwrap();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let actual = std::process::Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for exact readiness membership parity");
    assert_eq!(
        actual.status.code(),
        Some(expected.exit_status),
        "{actual:?}"
    );
    assert_eq!(actual.stdout, expected.stdout, "{actual:?}");
    assert_eq!(actual.stderr, expected.stderr, "{actual:?}");
}

fn assert_native_matches(
    program: &mir::Program,
    expected: &doriac::mir_interpreter::InterpreterOutput,
    profile: doriac::backend::NativeProfile,
) {
    use std::process::{Command, Stdio};
    let bytes = doriac::codegen_native::generate_executable(program, profile).unwrap();
    let directory = execution_directory::ExecutionDirectory::new();
    std::fs::write(directory.path().join("input"), b"ABC").unwrap();
    let executable = directory.path().join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    std::fs::write(&executable, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let actual = doriac::native_process::spawn(
        Command::new(&executable)
            .current_dir(directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap()
    .wait_with_output()
    .unwrap();
    assert_eq!(
        actual.status.code(),
        Some(expected.exit_status),
        "{profile:?}: {actual:?}"
    );
    assert_eq!(actual.stdout, expected.stdout, "{profile:?}: {actual:?}");
    assert_eq!(actual.stderr, expected.stderr, "{profile:?}: {actual:?}");
}

const ROUND_TRIP: &str = r#"
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    let $stream = new ReadyOnly(File::openRead("input"), "original");
    let $key = $set->register($stream);
    let $copy = $key;
    let $returned = $set->unregister($copy);
    echo $returned->getLabel() . "\n";
}
"#;

#[cfg(feature = "llvm-backend")]
#[test]
fn llvm_membership_object_uses_checked_transport_without_transfer_callbacks() {
    let program = lower(ROUND_TRIP);
    let ir = doriac::codegen_llvm::lower_mir_to_llvm_ir(&program).unwrap();
    assert!(
        ir.contains("call i8 @dr_v1_checked_readiness_set_add_owned("),
        "{ir}"
    );
    assert!(
        ir.contains("call i8 @dr_v1_checked_readiness_set_remove_owned("),
        "{ir}"
    );
    let callbacks = ir
        .lines()
        .filter(|line| line.starts_with("define ") && line.contains("@__doria_stream_callback_"))
        .collect::<Vec<_>>();
    assert_eq!(callbacks.len(), 1, "{callbacks:?}");
    assert!(callbacks[0].contains("_ReadinessSource("), "{callbacks:?}");
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
}

#[cfg(feature = "llvm-backend")]
#[test]
fn llvm_membership_object_accepts_unconstrained_removal_key_declarations() {
    let program = lower(&format!(
        r#"
function removeInt(writable ReadinessSet $set, RegistrationKey<int> $key): int throws IoError {{
    return $set->unregister($key);
}}
{ROUND_TRIP}
"#,
    ));
    assert!(program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .any(|block| {
            matches!(
                block.terminator,
                mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::UnregisterReadinessStream {
                        stream_type: mir::Type::Scalar(mir::ScalarType::Integer(
                            doriac::numeric::IntegerType::Int64
                        )),
                        ..
                    },
                    ..
                }
            )
        }));
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .unwrap()
        .is_empty());
}

#[test]
fn readiness_only_round_trip_returns_the_original_owner_and_copy_key() {
    let output = execute(ROUND_TRIP);
    assert_eq!(output.output.stdout, b"original\ndrop original\n");
    assert_eq!(output.trace.file_close_attempts, 1);
}

#[test]
fn file_round_trip_preserves_pending_capabilities_and_transfer_storage() {
    let output = execute(
        r#"
use Doria\Std\Io\ReadOutcome;
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(File::openRead("input"));
    let writable $returned = $set->unregister($key);
    writable Bytes $buffer = Bytes::allocate(3);
    {
        let writable $region = $buffer->getWritableRegion(0, 3);
        echo match ($returned->read($region)) {
            ReadOutcome::Data($count) => "data {$count}\n",
            default => "wrong outcome\n",
        };
    }
    echo "bytes {$buffer[0]} {$buffer[1]} {$buffer[2]}\n";
    $returned->close();
}
"#,
    );
    assert_eq!(output.output.stdout, b"data 3\nbytes 65 66 67\n");
    assert_eq!(output.trace.file_close_attempts, 1);
}

#[test]
fn erased_round_trip_keeps_the_original_method_view() {
    let source = r#"
interface NamedReady extends ReadinessStream { function getLabel(): string; }
class ErasedReady implements NamedReady {
    internal ReadableFile $source;
    function __construct(parameter take ReadableFile $source) { $this->source = $source; }
    ReadinessSource $readinessSource { borrowed get => $this->source->readinessSource; }
    function getLabel(): string { return "erased original"; }
    function __destruct() { try { echo "drop erased\n"; } catch (IoError) {} }
}
function retain(writable ReadinessSet $set, take NamedReady $stream): NamedReady {
    let $key = $set->register($stream);
    return $set->unregister($key);
}
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    let $returned = retain($set, new ErasedReady(File::openRead("input")));
    echo $returned->getLabel() . "\n";
}
"#;
    let output = execute(source);
    assert_eq!(output.output.stdout, b"erased original\ndrop erased\n");
    assert_eq!(output.trace.file_close_attempts, 1);
}

#[test]
fn open_round_trip_keeps_dynamic_dispatch_and_child_cleanup() {
    let output = execute(
        r#"
open class Base implements ReadinessStream {
    internal ReadableFile $source;
    function __construct(parameter take ReadableFile $source) { $this->source = $source; }
    ReadinessSource $readinessSource { borrowed get => $this->source->readinessSource; }
    open function getLabel(): string { return "base"; }
    function __destruct() { try { echo "drop base\n"; } catch (IoError) {} }
}
class Child extends Base {
    function __construct(parameter take ReadableFile $source) { parent::__construct($source); }
    override function getLabel(): string { return "child"; }
    function __destruct() { try { echo "drop child\n"; } catch (IoError) {} }
}
function create(take ReadableFile $source): Base { return new Child($source); }
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(create(File::openRead("input")));
    let $returned = $set->unregister($key);
    echo $returned->getLabel() . "\n";
}
"#,
    );
    assert_eq!(output.output.stdout, b"child\ndrop child\ndrop base\n");
    assert_eq!(output.trace.file_close_attempts, 1);
}

#[test]
fn foreign_and_stale_keys_leave_other_members_untouched() {
    let output = execute(
        r#"
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    writable ReadinessSet $other = new ReadinessSet();
    let $key = $set->register(new ReadyOnly(File::openRead("input"), "original"));
    try { let $wrong = $other->unregister($key); }
    catch (IoError $error) {
        echo "foreign " . ($error->operation == IoOperation::Unregister) . " " .
            ($error->target == IoTarget::ReadinessSet) . " " .
            ($error->reason == IoErrorReason::InvalidInput) . "\n";
    }
    { let $returned = $set->unregister($key); echo $returned->getLabel() . "\n"; }
    let $newKey = $set->register(new ReadyOnly(File::openRead("input"), "new"));
    try { let $stale = $set->unregister($key); }
    catch (IoError $error) { echo "stale " . ($error->reason == IoErrorReason::InvalidInput) . "\n"; }
    let $new = $set->unregister($newKey);
    echo $new->getLabel() . "\n";
}
"#,
    );
    assert_eq!(
        output.output.stdout,
        b"foreign true true true\noriginal\ndrop original\nstale true\nnew\ndrop new\n"
    );
    assert_eq!(output.trace.file_close_attempts, 2);
}

#[test]
fn set_cleanup_preserves_reverse_acquisition_after_removal() {
    let output = execute(
        r#"
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    let $first = $set->register(new ReadyOnly(File::openRead("input"), "first"));
    let $second = $set->register(new ReadyOnly(File::openRead("input"), "second"));
    { let $removed = $set->unregister($second); }
    let $third = $set->register(new ReadyOnly(File::openRead("input"), "third"));
    echo "end\n";
}
"#,
    );
    assert_eq!(
        output.output.stdout,
        b"drop second\nend\ndrop third\ndrop first\n"
    );
    assert_eq!(output.trace.file_close_attempts, 3);
}

#[test]
fn escaping_checked_error_drops_set_members_before_reporting() {
    let source = r#"
function main(): void throws IoError {
    writable ReadinessSet $set = new ReadinessSet();
    let $first = $set->register(new ReadyOnly(File::openRead("input"), "first"));
    let $second = $set->register(new ReadyOnly(File::openRead("input"), "second"));
    throw new IoError("stop", IoOperation::Wait, IoTarget::ReadinessSet, IoErrorReason::Other);
}
"#;
    let program = lower(source);
    let output = interpret_with_io(
        &program,
        MirIo {
            files: [("input".into(), b"ABC".to_vec())].into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output.output.exit_status, 70);
    assert_eq!(output.output.stdout, b"drop second\ndrop first\n");
    for &profile in doriac::backend::NativeProfile::AVAILABLE {
        assert_native_matches(&program, &output.output, profile);
    }
    assert_php_matches(source, &output.output);
    let diagnostic = String::from_utf8(output.output.stderr).unwrap();
    assert!(
        diagnostic.contains("R1000") && diagnostic.contains("stop"),
        "{diagnostic}"
    );
    assert_eq!(output.trace.file_close_attempts, 2);
}

#[test]
fn registered_destructor_preserves_current_cleanup_call_path() {
    for exit in [
        "",
        "return;",
        "throw new IoError(\"stop\", IoOperation::Wait, IoTarget::ReadinessSet, IoErrorReason::Other);",
    ] {
        let source = format!(
            r#"
class PanicOnDrop implements ReadinessStream {{
    internal ReadableFile $source;
    function __construct(parameter take ReadableFile $source) {{ $this->source = $source; }}
    ReadinessSource $readinessSource {{ borrowed get => $this->source->readinessSource; }}
    function __destruct() {{ panic("registered cleanup"); }}
}}
function finish(): void {{
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new PanicOnDrop(File::openRead("input")));
    {exit}
}}
function main(): void {{ finish(); }}
"#,
        );
        let program = lower(&source);
        let output = interpret_with_io(
            &program,
            MirIo {
                files: [("input".into(), b"ABC".to_vec())].into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.output.exit_status, 101);
        assert!(output.output.stdout.is_empty());
        let diagnostic = String::from_utf8_lossy(&output.output.stderr);
        for name in ["PanicOnDrop::__destruct", "finish", "main"] {
            assert!(diagnostic.contains(name), "{diagnostic}");
        }
        for &profile in doriac::backend::NativeProfile::AVAILABLE {
        assert_native_matches(
                &program,
                &output.output,
                profile,
            );
        }
        assert_php_matches(&source, &output.output);
    }
}

#[test]
fn getter_failure_drops_taken_adapter_before_catch_and_preserves_error() {
    let output = execute(
        r#"
function refuse(): void throws IoError {
    throw new IoError("getter refused", IoOperation::Read, IoTarget::File("input"),
        IoErrorReason::Other, 9, 3);
}
class FailingReady implements ReadinessStream {
    internal ReadableFile $source;
    function __construct(parameter take ReadableFile $source) { $this->source = $source; }
    ReadinessSource $readinessSource {
        borrowed get { refuse(); return $this->source->readinessSource; }
    }
    function __destruct() { try { echo "drop failing\n"; } catch (IoError) {} }
}
function main(): void {
    writable ReadinessSet $set = new ReadinessSet();
    try { let $key = $set->register(new FailingReady(File::openRead("input"))); }
    catch (IoError $error) {
        echo "{$error->message} {$error->systemCode ?? -1} {$error->transferredByteCount ?? -1}\n";
    }
    let $key = $set->register(new ReadyOnly(File::openRead("input"), "survivor"));
    let $returned = $set->unregister($key);
    echo $returned->getLabel() . "\n";
}
"#,
    );
    assert_eq!(
        output.output.stdout,
        b"drop failing\ngetter refused 9 3\nsurvivor\ndrop survivor\n"
    );
    assert_eq!(output.trace.file_close_attempts, 2);
}

#[test]
fn membership_mir_rejects_wrong_access_ownership_and_key_identity() {
    let valid = lower(ROUND_TRIP);
    for fault in 0..5 {
        let mut invalid = valid.clone();
        let function = invalid
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedIo {
                            operation: mir::CheckedIoOperation::RegisterReadinessStream { .. },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let (set, stream, key, result) =
            function
                .blocks
                .iter_mut()
                .find_map(|block| {
                    let mir::Terminator::CheckedIo {
                        operation:
                            mir::CheckedIoOperation::RegisterReadinessStream { set, stream, key },
                        result,
                        ..
                    } = &mut block.terminator
                    else {
                        return None;
                    };
                    if fault == 4 {
                        *key = doriac::inline_value::InlineValueType::Duration;
                    }
                    Some((*set, *stream, *key, result.unwrap()))
                })
                .unwrap();
        match fault {
            0 => function.locals[set.0].writable = false,
            1 => function.locals[stream.0].owned = false,
            2 => function.locals[result.0].owned = true,
            3 => function.locals[stream.0].ty = mir::Type::String,
            4 => function.locals[result.0].ty = mir::Type::Scalar(mir::ScalarType::Inline(key)),
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "fault {fault}"
        );
    }
}
