//! Decision 0110 child processes: spawning, pipes, bounded collection, the
//! mandatory cleanup policy and process errors.
#[path = "common/execution_directory.rs"]
mod execution_directory;
#[path = "common/native_execution.rs"]
mod native_execution;

use std::collections::BTreeMap;

use doriac::backend::NativeProfile;

use doriac::mir_interpreter::{MirExitStatus, MirIo, MirProgram};

const PATH: &str = "child-process.doria";

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir(PATH, source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"))
}

fn programs(entries: &[(&str, MirProgram)]) -> MirIo {
    MirIo {
        programs: entries
            .iter()
            .map(|(name, program)| (name.to_string(), program.clone()))
            .collect::<BTreeMap<_, _>>(),
        ..MirIo::default()
    }
}

fn interpret(source: &str, io: MirIo) -> doriac::mir_interpreter::InterpreterIoOutput {
    let program = lower(source);
    let observed = doriac::mir_interpreter::interpret_with_io(&program, io)
        .unwrap_or_else(|error| panic!("{source}\n{error:?}"));
    assert!(
        observed.output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&observed.output.stderr)
    );
    observed
}

const STATUS: &str = r#"
function describe(ExitStatus $status): string {
    return match ($status) {
        ExitStatus::Exited($code) => "exited {$code}",
        ExitStatus::Signaled($signal) => "signaled {$signal}",
    };
}
function text(Bytes $bytes): string {
    let $text = String::fromBytes($bytes);
    return $text ?? "<invalid>";
}
"#;

#[test]
fn collect_drains_both_streams_and_reports_the_exit_status() {
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus}};
{STATUS}
function main(): void {{
    writable Command $command = Command::executable("/usr/bin/greet");
    $command->argument("world");
    $command->environment("GREETING", "hi");
    $command->environment("GREETING", "hello");
    $command->removeEnvironment("HOME");
    $command->directory("/tmp");
    let $collected = $command->spawn(ChildCleanup::Wait)->collect(1024);
    echo describe($collected->status) . "|" . text($collected->output) . "|" . text($collected->errors);
    echo $collected->truncated ? "|truncated\n" : "|complete\n";
    let $short = Command::executable("/usr/bin/greet")->spawn(ChildCleanup::Wait)->collect(3);
    echo text($short->output) . "|" . text($short->errors) . ($short->truncated ? "|truncated\n" : "|complete\n");
}}
"#
    );
    let observed = interpret(
        &source,
        programs(&[(
            "/usr/bin/greet",
            MirProgram {
                output: b"hello world\n".to_vec(),
                errors: b"note".to_vec(),
                exit: Some(MirExitStatus::Exited(3)),
                ..MirProgram::default()
            },
        )]),
    );
    assert_eq!(
        String::from_utf8_lossy(&observed.output.stdout),
        "exited 3|hello world\n|note|complete\nhel|not|truncated\n"
    );
    let spawn = &observed.trace.spawns[0];
    assert_eq!(spawn.program, "/usr/bin/greet");
    assert!(!spawn.shell);
    assert_eq!(spawn.arguments, ["world"]);
    assert_eq!(
        spawn.environment,
        [("GREETING".to_string(), "hello".to_string())]
    );
    assert_eq!(spawn.removed_environment, ["HOME"]);
    assert_eq!(spawn.directory.as_deref(), Some("/tmp"));
    assert!(observed.trace.cleanups.is_empty());
}

#[test]
fn input_pipes_half_close_and_output_reads_to_the_end() {
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus}};
use Doria\Std\Io\ReadOutcome;
{STATUS}
function main(): void {{
    let writable $child = Command::executable("cat")->spawn(ChildCleanup::Terminate);
    echo $child->getId() > 0 ? "id|" : "no id|";
    writable Bytes $message = Bytes::fromArray([97, 98, 99]);
    let $written = $child->getInput()->write($message->getRegion(0, 3));
    $child->getInput()->closeInput();
    $child->getInput()->closeInput();
    writable Bytes $buffer = Bytes::allocate(8);
    let writable $total = 0;
    while (true) {{
        let $count = match ($child->getOutput()->read($buffer->getWritableRegion($total, 8 - $total))) {{
            ReadOutcome::Data($bytes) => $bytes,
            ReadOutcome::EndOfStream => -1,
            ReadOutcome::WouldBlock => 0,
            ReadOutcome::TimedOut => 0,
        }};
        if ($count < 0) {{
            break;
        }}
        $total += $count;
    }}
    echo "{{$total}}|" . describe($child->wait()) . "\n";
}}
"#
    );
    let observed = interpret(
        &source,
        programs(&[(
            "cat",
            MirProgram {
                echo_input: true,
                exit: Some(MirExitStatus::Exited(0)),
                ..MirProgram::default()
            },
        )]),
    );
    assert_eq!(
        String::from_utf8_lossy(&observed.output.stdout),
        "id|3|exited 0\n"
    );
}

#[test]
fn missing_programs_and_invalid_requests_fail_to_spawn() {
    let source = r#"use Doria\Std\Process\{Command, ChildCleanup};
use Doria\Std\Io\{IoError, IoOperation, IoTarget, IoErrorReason};
function attempt(Command $command): void {
    try {
        let $child = $command->spawn(ChildCleanup::Wait);
        echo "spawned\n";
    } catch (IoError $error) {
        echo $error->message . "\n";
        if ($error->operation != IoOperation::Spawn) { echo "wrong operation\n"; }
    }
}
function main(): void {
    attempt(Command::executable("/missing/tool"));
    attempt(Command::shell("echo hi"));
    writable Command $invalid = Command::executable("/usr/bin/greet");
    $invalid->environment("A=B", "1");
    attempt($invalid);
}
"#;
    let observed = interpret(
        source,
        programs(&[("/usr/bin/greet", MirProgram::default())]),
    );
    let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
    assert_eq!(
        String::from_utf8_lossy(&observed.output.stdout),
        format!(
            "failed to spawn process `/missing/tool`: not found\nfailed to spawn process `{shell}`: not found\nfailed to spawn process `/usr/bin/greet`: invalid input\n"
        )
    );
    assert_eq!(observed.trace.spawns.len(), 3);
    assert!(observed.trace.spawns[1].shell);
}

#[test]
fn cleanup_policies_run_once_and_explicit_resolution_skips_them() {
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus, WaitOutcome}};
use Doria\Std\Time\{{Deadline, Duration}};
{STATUS}
function main(): void {{
    {{
        let $terminated = Command::executable("server")->spawn(ChildCleanup::Terminate);
    }}
    {{
        let $detached = Command::executable("server")->spawn(ChildCleanup::Detach);
    }}
    {{
        let $waited = Command::executable("done")->spawn(ChildCleanup::Wait);
    }}
    let writable $child = Command::executable("server")->spawn(ChildCleanup::Wait);
    let $outcome = $child->waitUntil(Deadline::fromNow(Duration::fromNanoseconds(1000000)));
    echo match ($outcome) {{
        WaitOutcome::Exited($status) => describe($status),
        WaitOutcome::TimedOut => "timed out",
    }};
    echo "|" . describe($child->terminate()) . "\n";
    let writable $finished = Command::executable("done")->spawn(ChildCleanup::Terminate);
    let $exited = $finished->waitUntil(Deadline::fromNow(Duration::fromNanoseconds(1000000)));
    echo match ($exited) {{
        WaitOutcome::Exited($status) => describe($status),
        WaitOutcome::TimedOut => "timed out",
    }} . "\n";
    Command::executable("server")->spawn(ChildCleanup::Wait)->detach();
}}
"#
    );
    let observed = interpret(
        &source,
        programs(&[
            ("server", MirProgram::default()),
            (
                "done",
                MirProgram {
                    exit: Some(MirExitStatus::Signaled(2)),
                    ..MirProgram::default()
                },
            ),
        ]),
    );
    let terminated = if cfg!(windows) {
        "exited 1"
    } else {
        "signaled 9"
    };
    assert_eq!(
        String::from_utf8_lossy(&observed.output.stdout),
        format!("timed out|{terminated}\nsignaled 2\n")
    );
    assert_eq!(observed.trace.cleanups, ["Terminate", "Detach", "Wait"]);
}

/// Real children through /bin/sh on Unix hosts.
#[cfg(unix)]
#[test]
fn native_children_collect_stream_and_report_exit_status() {
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus, WaitOutcome}};
use Doria\Std\Io\{{ReadOutcome, IoError}};
use Doria\Std\Time\{{Deadline, Duration}};
{STATUS}
function main(): void {{
    let $collected = Command::shell("printf out; printf err >&2; exit 3")->spawn(ChildCleanup::Wait)->collect(1024);
    echo describe($collected->status) . "|" . text($collected->output) . "|" . text($collected->errors) . "\n";

    // Both streams far exceed a pipe buffer: draining must be concurrent.
    let $large = Command::shell("head -c 300000 /dev/zero; head -c 300000 /dev/zero >&2")->spawn(ChildCleanup::Wait)->collect(10);
    echo "{{$large->output->length}}/{{$large->errors->length}}" . ($large->truncated ? "|truncated\n" : "|complete\n");

    writable Command $env = Command::executable("sh");
    $env->argument("-c");
    $env->argument('printf "%s:%s:%s" "$GREETING" "${{HOME-unset}}" "$(pwd)"');
    $env->environment("GREETING", "hi");
    $env->removeEnvironment("HOME");
    $env->directory("/");
    echo text($env->spawn(ChildCleanup::Wait)->collect(256)->output) . "\n";

    let writable $cat = Command::executable("cat")->spawn(ChildCleanup::Terminate);
    writable Bytes $message = Bytes::fromArray([97, 98, 99]);
    let $written = $cat->getInput()->write($message->getRegion(0, 3));
    $cat->getInput()->closeInput();
    writable Bytes $buffer = Bytes::allocate(8);
    let writable $total = 0;
    while (true) {{
        let $count = match ($cat->getOutput()->read($buffer->getWritableRegion($total, 8 - $total))) {{
            ReadOutcome::Data($bytes) => $bytes,
            ReadOutcome::EndOfStream => -1,
            ReadOutcome::WouldBlock => 0,
            ReadOutcome::TimedOut => 0,
        }};
        if ($count < 0) {{
            break;
        }}
        $total += $count;
    }}
    echo "cat {{$total}}|" . describe($cat->wait()) . "\n";

    writable Command $sleep = Command::executable("sleep");
    $sleep->argument("30");
    let writable $sleeper = $sleep->spawn(ChildCleanup::Terminate);
    echo ($sleeper->getId() > 0 ? "id|" : "no id|");
    echo match ($sleeper->waitUntil(Deadline::fromNow(Duration::fromNanoseconds(20000000)))) {{
        WaitOutcome::Exited($status) => describe($status),
        WaitOutcome::TimedOut => "timed out",
    }};
    echo "|" . describe($sleeper->terminate()) . "\n";

    try {{
        let $missing = Command::executable("doria-no-such-program")->spawn(ChildCleanup::Wait);
    }} catch (IoError $error) {{
        echo $error->message . "\n";
    }}
}}
"#
    );
    let program = lower(&source);
    let expected = "exited 3|out|err\n10/10|truncated\nhi:unset:/\ncat 3|exited 0\nid|timed out|signaled 9\nfailed to spawn process `doria-no-such-program`: not found\n";
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected);
}

/// PHP reaches the same runtime through FFI (Decision 0110, PHP Native Runtime).
fn assert_php_execution(source: &str, expected: &str) {
    let directory = native_execution::ExecutionDirectory::new();
    let artifact =
        doriac::compile_source(PATH, source, doriac::backend::BackendTarget::Php).unwrap();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .current_dir(directory.path())
        .arg(path)
        .output()
        .expect("PHP is required for process compatibility parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    assert!(output.stderr.is_empty(), "{output:?}");
}

/// Pipe failures name the child's pipe; the Wait policy at destruction closes
/// the pipes first, so a child blocked on its input still exits.
#[cfg(unix)]
#[test]
fn native_pipe_failures_name_the_pipe_and_destruction_waits_without_deadlock() {
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus}};
use Doria\Std\Io\{{IoError, IoOperation}};
{STATUS}
function main(): void {{
    let writable $cat = Command::executable("cat")->spawn(ChildCleanup::Wait);
    $cat->getInput()->closeInput();
    writable Bytes $byte = Bytes::fromArray([120]);
    try {{
        let $written = $cat->getInput()->write($byte->getRegion(0, 1));
        echo "wrote after close\n";
    }} catch (IoError $error) {{
        echo $error->message . ($error->operation == IoOperation::Write ? "|write\n" : "|other\n");
    }}
    echo describe($cat->wait()) . "\n";
    {{
        // `cat` blocks reading its input; destruction closes it, then waits.
        let $reader = Command::executable("cat")->spawn(ChildCleanup::Wait);
    }}
    try {{
        writable Command $shell = Command::shell("true");
        $shell->argument("ignored");
        let $child = $shell->spawn(ChildCleanup::Wait);
    }} catch (IoError $error) {{
        echo $error->message . "\n";
    }}
    let writable $partial = Command::shell("printf abcdef")->spawn(ChildCleanup::Wait);
    writable Bytes $two = Bytes::allocate(2);
    let $first = $partial->getOutput()->read($two->getWritableRegion(0, 2));
    echo text($partial->collect(64)->output) . "\n";
    echo "done\n";
}}
"#
    );
    let program = lower(&source);
    let expected = "failed to write process `cat` input: closed|write\nexited 0\nfailed to spawn process `/bin/sh`: invalid input\ncdef\ndone\n";
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected);
}
