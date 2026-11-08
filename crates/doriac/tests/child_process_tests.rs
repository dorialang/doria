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
function prefix(Bytes $bytes, int $length): string {
    writable Bytes $copy = Bytes::allocate($length);
    {
        let writable $destination = $copy->getWritableRegion(0, $length);
        $destination->copyFrom($bytes->getRegion(0, $length));
    }
    return text($copy);
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

/// How each native scenario's child is spelled on this host. The scenarios,
/// their Doria source and their expected output are written once; only the
/// programs, the shell, the filesystem root and the platform's report for a
/// terminated child differ. Unix hosts use POSIX tools; Windows hosts use
/// cmd.exe for the shell form and Windows PowerShell for executables, both
/// present on every Windows host.
struct Host {
    /// Doria declarations the expressions below use.
    prelude: &'static str,
    /// A Command that writes `out` to its output and `err` to its error
    /// output, then exits with status 3.
    report: &'static str,
    /// A Command that writes far more than a pipe buffer to both outputs.
    flood: &'static str,
    /// A Command that writes `$GREETING:<DORIA_CHILD_INHERITED or unset>:<cwd>`.
    environment: &'static str,
    /// A Command that copies its input to its output until end of input.
    copy: &'static str,
    /// The program name a failure of `copy` reports.
    copy_name: &'static str,
    /// A Command that sleeps for 30 seconds.
    sleep: &'static str,
    /// A shell-form Command that writes `abcdef`.
    partial: &'static str,
    /// Any shell script, for the shell form's argument rejection.
    shell_script: &'static str,
    /// The program a shell-form failure reports.
    shell_name: &'static str,
    /// How `describe` reports a child ended by `terminate`.
    terminated: &'static str,
    /// A Command that waits, then writes `ready` and exits.
    slow: &'static str,
    /// A Command that reads its input to the end and writes how many bytes.
    count: &'static str,
}

#[cfg(unix)]
const HOST: Host = Host {
    prelude: r#"
function sh(string $script): Command {
    writable Command $command = Command::executable("sh");
    $command->argument("-c");
    $command->argument($script);
    return $command;
}
function sleeper(): Command {
    writable Command $command = Command::executable("sleep");
    $command->argument("30");
    return $command;
}
"#,
    report: "Command::shell('printf out; printf err >&2; exit 3')",
    flood: "Command::shell('head -c 300000 /dev/zero; head -c 300000 /dev/zero >&2')",
    environment: r#"sh('printf "%s:%s:%s" "$GREETING" "${DORIA_CHILD_INHERITED-unset}" "$(pwd)"')"#,
    copy: "Command::executable('cat')",
    copy_name: "cat",
    sleep: "sleeper()",
    partial: "Command::shell('printf abcdef')",
    shell_script: "true",
    shell_name: "/bin/sh",
    terminated: "signaled 9",
    slow: "Command::shell('sleep 0.3; printf ready')",
    count: "Command::shell('wc -c | tr -d \" \\n\"')",
};

// `-InputFormat None` keeps PowerShell from reading its input as commands, so
// a child whose input pipe stays open still exits; `copy` reads the input
// stream itself.
#[cfg(windows)]
const HOST: Host = Host {
    prelude: r#"
function powershell(string $script): Command {
    writable Command $command = Command::executable("powershell");
    $command->argument("-NoProfile");
    $command->argument("-NonInteractive");
    $command->argument("-InputFormat");
    $command->argument("None");
    $command->argument("-Command");
    $command->argument($script);
    return $command;
}
"#,
    report: "Command::shell('<nul set /p =out&1>&2 <nul set /p =err&exit /b 3')",
    flood: "powershell('$b = New-Object byte[] 300000; $o = [Console]::OpenStandardOutput(); $o.Write($b, 0, $b.Length); $o.Flush(); $e = [Console]::OpenStandardError(); $e.Write($b, 0, $b.Length); $e.Flush()')",
    environment: r"powershell('$inherited = if (Test-Path env:DORIA_CHILD_INHERITED) { \'set\' } else { \'unset\' }; [Console]::Out.Write($env:GREETING + \':\' + $inherited + \':\' + (Get-Location).Path)')",
    copy: "powershell('$i = [Console]::OpenStandardInput(); $o = [Console]::OpenStandardOutput(); $i.CopyTo($o); $o.Flush()')",
    copy_name: "powershell",
    sleep: "powershell('Start-Sleep -Seconds 30')",
    partial: "Command::shell('<nul set /p =abcdef')",
    shell_script: "exit /b 0",
    shell_name: "cmd.exe",
    terminated: "exited 1",
    slow: r"powershell('Start-Sleep -Milliseconds 300; [Console]::Out.Write(\'ready\')')",
    count: "powershell('$i = [Console]::OpenStandardInput(); $b = New-Object byte[] 65536; $n = 0; while (($r = $i.Read($b, 0, $b.Length)) -gt 0) { $n += $r }; [Console]::Out.Write($n)')",
};

/// The filesystem root the environment scenario runs its child in.
fn root() -> String {
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        format!("{drive}\\")
    } else {
        "/".to_string()
    }
}

/// A Doria single-quoted string literal for `text`.
fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// An inherited variable the child-process program removes from one child.
const INHERITED: (&str, &str) = ("DORIA_CHILD_INHERITED", "present");

/// Real children of the host: the shell form, an executable with arguments,
/// an edited environment and directory, input half-close, terminate and a
/// missing program, on every native profile and through PHP.
#[test]
fn native_children_collect_stream_and_report_exit_status() {
    let Host {
        prelude,
        report,
        flood,
        environment,
        copy,
        sleep,
        terminated,
        ..
    } = HOST;
    let root = root();
    let directory = literal(&root);
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus, WaitOutcome}};
use Doria\Std\Io\{{ReadOutcome, IoError}};
use Doria\Std\Time\{{Deadline, Duration}};
{STATUS}
{prelude}
function main(): void {{
    let $collected = {report}->spawn(ChildCleanup::Wait)->collect(1024);
    echo describe($collected->status) . "|" . text($collected->output) . "|" . text($collected->errors) . "\n";

    // Both streams far exceed a pipe buffer: draining must be concurrent.
    let $large = {flood}->spawn(ChildCleanup::Wait)->collect(10);
    echo "{{$large->output->length}}/{{$large->errors->length}}" . ($large->truncated ? "|truncated\n" : "|complete\n");

    writable Command $env = {environment};
    $env->environment("GREETING", "hi");
    $env->removeEnvironment("DORIA_CHILD_INHERITED");
    $env->directory({directory});
    echo text($env->spawn(ChildCleanup::Wait)->collect(256)->output) . "\n";

    let writable $cat = {copy}->spawn(ChildCleanup::Terminate);
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

    let writable $sleeper = {sleep}->spawn(ChildCleanup::Terminate);
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
    let expected = format!(
        "exited 3|out|err\n10/10|truncated\nhi:unset:{root}\ncat 3|exited 0\nid|timed out|{terminated}\nfailed to spawn process `doria-no-such-program`: not found\n"
    );
    let environment = [(INHERITED.0, std::ffi::OsStr::new(INHERITED.1))];
    for &profile in NativeProfile::AVAILABLE {
        let directory = native_execution::ExecutionDirectory::new();
        let result = native_execution::execute_native_in_directory_with_env(
            &program,
            profile,
            &[],
            &directory,
            &environment,
        );
        assert_eq!(result.status.code(), Some(0), "{profile:?}: {result:?}");
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            expected,
            "{profile:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty(), "{profile:?}: {result:?}");
    }
    assert_php_execution(&source, &expected, &environment);
}

/// PHP reaches the same runtime through FFI (Decision 0110, PHP Native Runtime).
fn assert_php_execution(source: &str, expected: &str, environment: &[(&str, &std::ffi::OsStr)]) {
    let directory = native_execution::ExecutionDirectory::new();
    let artifact =
        doriac::compile_source(PATH, source, doriac::backend::BackendTarget::Php).unwrap();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .current_dir(directory.path())
        .envs(environment.iter().copied())
        .arg(path)
        .output()
        .expect("PHP is required for process compatibility parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    assert!(output.stderr.is_empty(), "{output:?}");
}

/// Pipe failures name the child's pipe; the Wait policy at destruction closes
/// the pipes first, so a child blocked on its input still exits.
#[test]
fn native_pipe_failures_name_the_pipe_and_destruction_waits_without_deadlock() {
    let Host {
        prelude,
        copy,
        copy_name,
        partial,
        shell_script,
        shell_name,
        ..
    } = HOST;
    let shell_script = literal(shell_script);
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus}};
use Doria\Std\Io\{{IoError, IoOperation}};
{STATUS}
{prelude}
function main(): void {{
    let writable $cat = {copy}->spawn(ChildCleanup::Wait);
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
        // The copier blocks reading its input; destruction closes it, then waits.
        let $reader = {copy}->spawn(ChildCleanup::Wait);
    }}
    try {{
        writable Command $shell = Command::shell({shell_script});
        $shell->argument("ignored");
        let $child = $shell->spawn(ChildCleanup::Wait);
    }} catch (IoError $error) {{
        echo $error->message . "\n";
    }}
    let writable $partial = {partial}->spawn(ChildCleanup::Wait);
    writable Bytes $two = Bytes::allocate(2);
    let $first = $partial->getOutput()->read($two->getWritableRegion(0, 2));
    echo text($partial->collect(64)->output) . "\n";
    echo "done\n";
}}
"#
    );
    let program = lower(&source);
    let expected = format!(
        "failed to write process `{copy_name}` input: closed|write\nexited 0\nfailed to spawn process `{shell_name}`: invalid input\ncdef\ndone\n"
    );
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, &expected);
    }
    assert_php_execution(&source, &expected, &[]);
}

/// Doria shared by the transfer-group scenarios: drain a registered reader
/// until the end of the stream, reusing one read transfer.
const GROUP_DRAIN: &str = r#"
function drainRead(writable TransferGroup $group, ReadTransferKey $key, int $size, writable Bytes $received, int $start): int throws IoError {
    let writable $total = $start;
    while (true) {
        let $count = match ($group->getNextCompletion(Wait::Indefinite)) {
            TransferWaitResult::Completed($completion) => match ($completion) {
                TransferCompletion::Read($finished, $outcome) => match ($outcome) {
                    ReadOutcome::Data($bytes) => $bytes,
                    ReadOutcome::EndOfStream => -1,
                    default => 0,
                },
                default => 0,
            },
            default => 0,
        };
        if ($count < 0) {
            return $total;
        }
        {
            let $region = $group->getReadRegion($key);
            if ($region != null) {
                let writable $index = 0;
                while ($index < $count) {
                    $received[$total] = $region[$index];
                    $total += 1;
                    $index += 1;
                }
            }
        }
        $group->resubmitRead($key, 0, $size);
    }
}
"#;

const GROUP_IMPORTS: &str = r"use Doria\Std\Process\{Command, ChildCleanup, ExitStatus, ChildInput, ChildOutput};
use Doria\Std\Io\{BorrowedBufferedReader, BorrowedBufferedWriter, ReadinessSet, TransferPlan,
    TransferGroup, TransferWaitResult, TransferCompletion, Wait, ReadOutcome, WriteOutcome,
    ReadTransferKey, IoError, IoErrorReason};";

/// A child's output through a readiness set and a transfer plan: on the
/// interpreter's scripted child, then on real children below.
fn group_read_program(command: &str, prelude: &str) -> String {
    format!(
        r#"{GROUP_IMPORTS}
{STATUS}
{prelude}
{GROUP_DRAIN}
function main(): void throws IoError {{
    let writable $child = {command}->spawn(ChildCleanup::Wait);
    writable Bytes $received = Bytes::allocate(64);
    let writable $total = 0;
    {{
        let writable $reader = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 64);
        writable ReadinessSet $set = new ReadinessSet();
        let $registration = $set->register($reader);
        writable Bytes $storage = Bytes::allocate(4);
        {{
            let $plan = TransferPlan::read($registration, $storage->getWritableRegion(0, 4));
            let $key = $plan->getReadKey(0);
            let writable $group = $set->begin($plan);
            $total = drainRead($group, $key, 4, $received, 0);
        }}
        let $returned = $set->unregister($registration);
    }}
    let $seen = prefix($received, $total);
    echo "{{$total}}:{{$seen}}|" . describe($child->wait()) . "\n";
}}
"#
    )
}

#[test]
fn a_scripted_childs_output_joins_a_transfer_group() {
    let observed = interpret(
        &group_read_program("Command::executable('/usr/bin/report')", ""),
        programs(&[(
            "/usr/bin/report",
            MirProgram {
                output: b"abcdefghij".to_vec(),
                exit: Some(MirExitStatus::Exited(0)),
                ..MirProgram::default()
            },
        )]),
    );
    assert_eq!(
        String::from_utf8_lossy(&observed.output.stdout),
        "10:abcdefghij|exited 0\n"
    );
}

/// A real child's output is read through the shared transfer machinery on
/// every native profile and through PHP, and the child still resolves.
#[test]
fn native_child_output_joins_a_transfer_group() {
    let Host {
        prelude, partial, ..
    } = HOST;
    let source = group_read_program(partial, prelude);
    let program = lower(&source);
    let expected = "6:abcdef|exited 0\n";
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected, &[]);
}

/// A plain read on a child pipe waits for a slow child and never reports
/// would-block, and the child's own input stays blocking while it waits.
#[test]
fn native_plain_reads_wait_on_a_slow_child() {
    let Host {
        prelude,
        copy,
        slow,
        ..
    } = HOST;
    let source = format!(
        r#"use Doria\Std\Process\{{Command, ChildCleanup, ExitStatus, WaitOutcome}};
use Doria\Std\Io\{{ReadOutcome, IoError}};
use Doria\Std\Time\{{Deadline, Duration, ClockError}};
{STATUS}
{prelude}
function main(): void throws IoError, ClockError {{
    let writable $child = {slow}->spawn(ChildCleanup::Wait);
    writable Bytes $buffer = Bytes::allocate(16);
    let writable $total = 0;
    let writable $blocked = false;
    while (true) {{
        let $count = match ($child->getOutput()->read($buffer->getWritableRegion($total, 16 - $total))) {{
            ReadOutcome::Data($bytes) => $bytes,
            ReadOutcome::EndOfStream => -1,
            ReadOutcome::WouldBlock => -2,
            ReadOutcome::TimedOut => -3,
        }};
        if ($count == -1) {{ break; }}
        if ($count < 0) {{ $blocked = true; break; }}
        $total += $count;
    }}
    echo prefix($buffer, $total) . ($blocked ? "|would block\n" : "|waited\n");
    echo describe($child->wait()) . "\n";

    // The copier reads its input while the parent is busy elsewhere. Its
    // input is the blocking end of the pipe, so waiting does not end it.
    let writable $cat = {copy}->spawn(ChildCleanup::Terminate);
    let writable $timer = {slow}->spawn(ChildCleanup::Terminate);
    echo match ($timer->waitUntil(Deadline::fromNow(Duration::fromNanoseconds(100000000)))) {{
        WaitOutcome::Exited($status) => "ended early\n",
        WaitOutcome::TimedOut => "parent waited\n",
    }};
    writable Bytes $message = Bytes::fromArray([120, 121, 122]);
    let $written = $cat->getInput()->write($message->getRegion(0, 3));
    $cat->getInput()->closeInput();
    writable Bytes $copied = Bytes::allocate(8);
    let writable $seen = 0;
    while (true) {{
        let $count = match ($cat->getOutput()->read($copied->getWritableRegion($seen, 8 - $seen))) {{
            ReadOutcome::Data($bytes) => $bytes,
            default => -1,
        }};
        if ($count < 0) {{ break; }}
        $seen += $count;
    }}
    echo prefix($copied, $seen) . "|" . describe($cat->wait()) . "\n";
}}
"#
    );
    let program = lower(&source);
    let expected = "ready|waited\nexited 0\nparent waited\nxyz|exited 0\n";
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected, &[]);
}

/// A write larger than the pipe's capacity: a plain write waits for the child
/// to take every byte, and a pending write is resubmitted until it has done so.
#[test]
fn native_writes_past_pipe_capacity_deliver_every_byte() {
    let Host { prelude, count, .. } = HOST;
    let source = format!(
        r#"{GROUP_IMPORTS}
{STATUS}
{prelude}
function counted(writable ChildOutput $output): string throws IoError {{
    writable Bytes $buffer = Bytes::allocate(32);
    let writable $total = 0;
    while (true) {{
        let $read = match ($output->read($buffer->getWritableRegion($total, 32 - $total))) {{
            ReadOutcome::Data($bytes) => $bytes,
            default => -1,
        }};
        if ($read < 0) {{ break; }}
        $total += $read;
    }}
    return prefix($buffer, $total);
}}
function main(): void throws IoError {{
    writable Bytes $large = Bytes::allocate(1048576);

    let writable $plain = {count}->spawn(ChildCleanup::Wait);
    let $accepted = match ($plain->getInput()->write($large->getRegion(0, 1048576))) {{
        WriteOutcome::Accepted($bytes) => $bytes,
        WriteOutcome::WouldBlock => -1,
    }};
    $plain->getInput()->closeInput();
    let $plainCount = counted($plain->getOutput());
    echo "plain {{$accepted}}|{{$plainCount}}|" . describe($plain->wait()) . "\n";

    let writable $pending = {count}->spawn(ChildCleanup::Wait);
    let writable $sent = 0;
    {{
        let writable $writer = new BorrowedBufferedWriter<ChildInput>($pending->getInput(), 64);
        writable ReadinessSet $set = new ReadinessSet();
        let $registration = $set->register($writer);
        {{
            let $plan = TransferPlan::writeTo($registration, $large->getRegion(0, 1048576));
            let $key = $plan->getWriteKey(0);
            let writable $group = $set->begin($plan);
            while ($sent < 1048576) {{
                $sent += match ($group->getNextCompletion(Wait::Indefinite)) {{
                    TransferWaitResult::Completed($completion) => match ($completion) {{
                        TransferCompletion::Write($finished, $outcome) => match ($outcome) {{
                            WriteOutcome::Accepted($bytes) => $bytes,
                            default => 0,
                        }},
                        default => 0,
                    }},
                    default => 0,
                }};
                if ($sent < 1048576) {{
                    $group->resubmitWrite($key, $sent, 1048576 - $sent);
                }}
            }}
        }}
        let $returned = $set->unregister($registration);
    }}
    $pending->getInput()->closeInput();
    let $pendingCount = counted($pending->getOutput());
    echo "pending {{$sent}}|{{$pendingCount}}|" . describe($pending->wait()) . "\n";
}}
"#
    );
    let program = lower(&source);
    let expected = "plain 1048576|1048576|exited 0\npending 1048576|1048576|exited 0\n";
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected, &[]);
}

/// A half-closed input is a checked Closed failure at registration, not a
/// runtime failure, and every outcome is checked on each backend.
#[test]
fn a_closed_child_input_is_rejected_as_closed() {
    let Host { prelude, copy, .. } = HOST;
    let source = format!(
        r#"{GROUP_IMPORTS}
{STATUS}
{prelude}
function main(): void {{
    let writable $child = {copy}->spawn(ChildCleanup::Wait);
    $child->getInput()->closeInput();
    try {{
        let writable $writer = new BorrowedBufferedWriter<ChildInput>($child->getInput(), 64);
        writable ReadinessSet $set = new ReadinessSet();
        let $registration = $set->register($writer);
        echo "registered\n";
    }} catch (IoError $error) {{
        echo ($error->reason == IoErrorReason::Closed ? "closed\n" : "other\n");
    }}
    try {{
        echo describe($child->wait()) . "\n";
    }} catch (IoError $error) {{
        echo "wait failed\n";
    }}
}}
"#
    );
    let program = lower(&source);
    let expected = "closed\nexited 0\n";
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    assert_php_execution(&source, expected, &[]);
}

/// One child's input and output in one group: the bytes written come back
/// through the child while the group waits on both pipes.
fn input_and_output_program(copy: &str, prelude: &str, total: usize) -> String {
    format!(
        r#"{GROUP_IMPORTS}
{STATUS}
{prelude}
function writtenBytes(TransferWaitResult $result): int {{
    return match ($result) {{
        TransferWaitResult::Completed($completion) => match ($completion) {{
            TransferCompletion::Write($finished, $outcome) => match ($outcome) {{
                WriteOutcome::Accepted($bytes) => $bytes,
                default => 0,
            }},
            default => -1,
        }},
        default => -1,
    }};
}}
function readBytes(TransferWaitResult $result): int {{
    return match ($result) {{
        TransferWaitResult::Completed($completion) => match ($completion) {{
            TransferCompletion::Read($finished, $outcome) => match ($outcome) {{
                ReadOutcome::Data($bytes) => $bytes,
                default => 0,
            }},
            default => -1,
        }},
        default => -1,
    }};
}}
function main(): void throws IoError {{
    let writable $child = {copy}->spawn(ChildCleanup::Wait);
    writable Bytes $source = Bytes::allocate({total});
    let writable $index = 0;
    while ($index < {total}) {{
        $source[$index] = 7;
        $index += 251;
    }}
    writable Bytes $storage = Bytes::allocate(4096);
    let writable $sent = 0;
    let writable $received = 0;
    let writable $sum = 0;
    {{
        let writable $writer = new BorrowedBufferedWriter<ChildInput>($child->getInput(), 64);
        let writable $reader = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 64);
        writable ReadinessSet $set = new ReadinessSet();
        let $sink = $set->register($writer);
        let $tap = $set->register($reader);
        {{
            let $plan = TransferPlan::writeTo($sink, $source->getRegion(0, {total}))
                ->andReadFrom($tap, $storage->getWritableRegion(0, 4096));
            let $writeKey = $plan->getWriteKey(0);
            let $readKey = $plan->getReadKey(1);
            let writable $group = $set->begin($plan);
            while ($sent < {total} || $received < {total}) {{
                let $result = $group->getNextCompletion(Wait::Indefinite);
                let $wrote = writtenBytes($result);
                if ($wrote >= 0) {{
                    $sent += $wrote;
                    if ($sent < {total}) {{
                        $group->resubmitWrite($writeKey, $sent, {total} - $sent);
                    }}
                }}
                let $count = readBytes($result);
                if ($count >= 0) {{
                    {{
                        let $region = $group->getReadRegion($readKey);
                        if ($region != null) {{
                            let writable $at = 0;
                            while ($at < $count) {{
                                $sum += Int::from($region[$at]);
                                $at += 1;
                            }}
                        }}
                    }}
                    $received += $count;
                    if ($received < {total}) {{
                        $group->resubmitRead($readKey, 0, 4096);
                    }}
                }}
            }}
        }}
        let $firstBack = $set->unregister($sink);
        let $secondBack = $set->unregister($tap);
    }}
    $child->getInput()->closeInput();
    echo "{{$sent}}|{{$received}}|{{$sum}}|" . describe($child->wait()) . "\n";
}}
"#
    )
}

/// The expected sum of the pattern `input_and_output_program` sends.
fn pattern_sum(total: usize) -> usize {
    (0..total).filter(|index| index % 251 == 0).count() * 7
}

#[test]
fn a_scripted_childs_input_and_output_share_a_transfer_group() {
    let total = 200;
    let source = input_and_output_program("Command::executable('cat')", "", total);
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
        format!("{total}|{total}|{}|exited 0\n", pattern_sum(total))
    );
}

/// Writing a child's input while reading its output: the classic case that
/// deadlocks when each side blocks on a full pipe.
#[test]
fn native_child_input_and_output_share_a_transfer_group() {
    let Host { prelude, copy, .. } = HOST;
    let total = 300_000;
    let source = input_and_output_program(copy, prelude, total);
    let program = lower(&source);
    let expected = format!("{total}|{total}|{}|exited 0\n", pattern_sum(total));
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, &expected);
    }
    assert_php_execution(&source, &expected, &[]);
}

/// A slot is exclusive and the owner stays whole: a second loan of the same
/// pipe, or resolving the child while a pipe loan lives, is still rejected.
#[test]
fn child_pipe_loans_are_disjoint_only_between_different_pipes() {
    let prefix = r#"use Doria\Std\Process\{Command, ChildCleanup, ChildInput, ChildOutput};
use Doria\Std\Io\{BorrowedBufferedReader, BorrowedBufferedWriter, IoError};
function main(): void throws IoError {
    let writable $child = Command::executable('cat')->spawn(ChildCleanup::Wait);
    let writable $reader = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 8);
"#;
    let accepted = format!(
        "{prefix}    let writable $writer = new BorrowedBufferedWriter<ChildInput>($child->getInput(), 8);\n    let writable $errors = new BorrowedBufferedReader<ChildOutput>($child->getErrors(), 8);\n}}\n"
    );
    lower(&accepted);
    // The same holds when the child lives in a stored property.
    lower(
        r#"use Doria\Std\Process\{Command, ChildCleanup, Child, ChildInput, ChildOutput};
use Doria\Std\Io\{BorrowedBufferedReader, BorrowedBufferedWriter, IoError};
class Holder {
    writable Child $child;
    function __construct(parameter take Child $child) { $this->child = $child; }
    writable function both(): void throws IoError {
        let writable $reader = new BorrowedBufferedReader<ChildOutput>($this->child->getOutput(), 8);
        let writable $writer = new BorrowedBufferedWriter<ChildInput>($this->child->getInput(), 8);
        $reader->fill();
    }
}
function main(): void {}
"#,
    );
    for rejected in [
        "    let writable $again = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 8);\n    $reader->fill();\n",
        "    let $status = $child->wait();\n    $reader->fill();\n",
    ] {
        let source = format!("{prefix}{rejected}}}\n");
        let errors = doriac::lower_source_to_mir(PATH, &source)
            .expect_err("the conflicting pipe loan must be rejected");
        assert!(
            errors.iter().any(|error| error.code == "E0763"),
            "{source}\n{errors:#?}"
        );
    }
}
