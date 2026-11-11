//! `Doria\Std\Term` (Decision 0140) against the interpreter's scripted
//! terminal: the authored module, its packed transport and the shared
//! operation core, checked on the exact bytes a terminal would receive.

use doria_terminal::wire::{ColorSupport, Key, Observation};
use doriac::mir;
use doriac::mir_interpreter::{
    interpret_with_io, InterpreterOutput, MirExitStatus, MirIo, MirProgram, MirTerminal,
};

fn terminal(events: Vec<Observation>) -> MirTerminal {
    MirTerminal {
        columns: 80,
        rows: 24,
        color_support: ColorSupport::Indexed,
        events,
        cursor: Some((9, 4)),
    }
}

/// Runs a program in the interpreter. A run the interpreter cannot finish
/// gives its reason instead.
fn interpret(source: &str, io: MirIo) -> Result<InterpreterOutput, String> {
    let program = match doriac::lower_source_to_mir("term.doria", source) {
        Ok(program) => program,
        Err(diagnostics) => panic!("the program does not compile: {diagnostics:#?}"),
    };
    interpret_with_io(&program, io)
        .map(|run| run.output)
        .map_err(|error| error.message)
}

fn run(source: &str, terminal: Option<MirTerminal>) -> InterpreterOutput {
    interpret(
        source,
        MirIo {
            terminal,
            ..MirIo::default()
        },
    )
    .unwrap()
}

fn drawn(output: &InterpreterOutput) -> String {
    String::from_utf8_lossy(&output.stdout).replace('\x1b', "ESC")
}

#[test]
fn a_frame_is_rendered_in_order_with_ordinary_output() {
    let output = run(
        r#"
use Doria\Std\Term\{Color, Console, Style, TerminalError};

function main(): void throws TerminalError {
    Console::hideCursor();
    Console::writeAt(2, 1, "hi", Style::plain->withForeground(Color::Red)->withBold());
    Console::moveTo(0, 3);
    Console::moveRight(4);
    Console::clearLineFromCursor();
    Console::setTitle("menu");
    Console::write("A");
    echo "B";
    Console::write("C", Style::plain->withForeground(Color::Rgb(255, 0, 0)));
    let $size = Console::getSize();
    echo "{$size->columns}x{$size->rows}";
    Console::showCursor();
}
"#,
        Some(terminal(Vec::new())),
    );
    assert_eq!(
        output.exit_status,
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        drawn(&output),
        "ESC[?25lESC[2;3HESC[1;31mhiESC[0mESC[4;1HESC[4CESC[KESC]2;menu\x07AB\
         ESC[38;5;196mCESC[0m80x24ESC[?25h"
    );
}

#[test]
fn keys_and_resizes_arrive_as_typed_events_and_the_script_end_closes_the_terminal() {
    let output = run(
        r#"
use Doria\Std\Io\Wait;
use Doria\Std\Term\{Console, Key, TerminalError, TerminalEvent, TerminalWaitResult};

function describe(TerminalWaitResult $result): string {
    return match ($result) {
        TerminalWaitResult::Event($event) => match ($event) {
            TerminalEvent::Key($key, $modifiers) => match ($key) {
                Key::Character($text) => "char {$text} control={$modifiers->control} alt={$modifiers->alt} shift={$modifiers->shift}",
                Key::Up => "up",
                Key::Enter => "enter",
                Key::Function($number) => "f{$number}",
                default => "other key",
            },
            TerminalEvent::Resized($size) => "resized {$size->columns}x{$size->rows}",
        },
        TerminalWaitResult::TimedOut => "timed out",
        TerminalWaitResult::Cancelled => "cancelled",
        TerminalWaitResult::Closed => "closed",
    };
}

function main(): void throws TerminalError {
    let $keys = Console::enterRawMode();
    let writable $open = true;
    while ($open) {
        let $result = $keys->getNextEvent(Wait::Indefinite);
        echo describe($result) . "\n";
        if ($result == TerminalWaitResult::Closed) {
            $open = false;
        }
    }
    let $position = $keys->getCursorPosition();
    echo "cursor {$position->column},{$position->row}\n";
    $keys->leave();
}
"#,
        Some(terminal(vec![
            Observation::Key {
                key: Key::Character('é'),
                modifiers: 0,
            },
            Observation::Key {
                key: Key::Character('c'),
                modifiers: 1,
            },
            Observation::Key {
                key: Key::Up,
                modifiers: 0,
            },
            Observation::Key {
                key: Key::Function(5),
                modifiers: 0,
            },
            Observation::Resized {
                columns: 100,
                rows: 30,
            },
            Observation::TimedOut,
            Observation::Key {
                key: Key::Enter,
                modifiers: 0,
            },
        ])),
    );
    assert_eq!(
        output.exit_status,
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "char é control=false alt=false shift=false\n\
         char c control=true alt=false shift=false\n\
         up\n\
         f5\n\
         resized 100x30\n\
         timed out\n\
         enter\n\
         closed\n\
         cursor 9,4\n"
    );
}

fn pressed(key: Key) -> Observation {
    Observation::Key { key, modifiers: 0 }
}

/// The Stage 46 acceptance program: one source for every platform.
const MENU: &str = include_str!("../../../examples/future/stage46/menu.doria");

#[test]
fn the_menu_moves_with_the_arrow_keys_and_enter_chooses_the_focused_item() {
    let output = run(
        MENU,
        Some(terminal(vec![
            pressed(Key::Down),
            pressed(Key::Down),
            pressed(Key::Enter),
        ])),
    );
    let drawn = drawn(&output);
    assert_eq!(
        output.exit_status,
        0,
        "{drawn}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The alternate screen is entered and left, the cursor hidden and shown.
    assert!(drawn.starts_with("ESC[?1049hESC[?25l"), "{drawn}");
    assert!(
        drawn.ends_with("ESC[?25hESC[?1049lYou chose: macOS arm64\n"),
        "{drawn}"
    );
    // Three frames: the focus starts on the first item and moves down twice.
    for focused in ["> Linux x86-64", "> Linux arm64", "> macOS arm64"] {
        assert!(
            drawn.contains(&format!("ESC[7m{focused}ESC[0m")),
            "no frame focuses {focused:?}: {drawn}"
        );
    }
    assert_eq!(drawn.matches("ESC[2JESC[H").count(), 3, "{drawn}");
}

#[test]
fn the_menu_cancels_on_escape_and_wraps_at_its_ends() {
    let output = run(
        MENU,
        Some(terminal(vec![pressed(Key::Up), pressed(Key::Escape)])),
    );
    let drawn = drawn(&output);
    assert_eq!(output.exit_status, 1, "{drawn}");
    // Up from the first item wraps to the last one.
    assert!(drawn.contains("ESC[7m> WebAssemblyESC[0m"), "{drawn}");
    assert!(drawn.ends_with("Nothing chosen.\n"), "{drawn}");
}

#[test]
fn the_menu_reports_a_missing_terminal_instead_of_drawing_garbage() {
    let output = run(MENU, None);
    assert_eq!(output.exit_status, 2);
    let drawn = drawn(&output);
    assert!(
        drawn.starts_with("This program needs a terminal: "),
        "{drawn}"
    );
    assert!(!drawn.contains("ESC"), "{drawn}");
}

const INTERACTIVE: &str = r#"
use Doria\Std\Io\StandardStreams;

function main(): void {
    let $input = StandardStreams::input;
    let $output = StandardStreams::output;
    let $error = StandardStreams::error;
    echo "{$input->isInteractive} {$output->isInteractive} {$error->isInteractive}";
}
"#;

#[test]
fn the_standard_views_say_whether_they_are_attached_to_a_terminal() {
    let attached = run(INTERACTIVE, Some(terminal(Vec::new())));
    assert_eq!(drawn(&attached), "true true true");
    let redirected = run(INTERACTIVE, None);
    assert_eq!(drawn(&redirected), "false false false");
}

#[test]
fn raw_mode_owns_standard_input_until_it_is_left() {
    let output = run(
        r#"
use Doria\Std\Io\IoError;
use Doria\Std\Term\{Console, TerminalError};

function attempt(): string {
    try {
        let $line = read_line();
        return "read";
    } catch (IoError $error) {
        return "refused";
    }
}

function main(): void throws TerminalError {
    let $raw = Console::enterRawMode();
    echo attempt();
    $raw->leave();
    echo " ";
    echo attempt();
}
"#,
        Some(terminal(Vec::new())),
    );
    assert_eq!(drawn(&output), "refused read");
}

// Terminal events in a transfer-group wait (Decision 0140, "Waiting On A
// Terminal And Other Sources"). The scripted terminal decides what each
// readiness check sees: a scripted `TimedOut` is one moment in which the
// terminal had nothing, used up by whoever asks, the group included.

/// One word per fetch, so a transcript shows the order completions arrive in.
const DESCRIBE_COMPLETION: &str = r#"
function describe(TransferWaitResult $result): string {
    return match ($result) {
        TransferWaitResult::Completed($completion) => match ($completion) {
            TransferCompletion::Read($key, $outcome) => match ($outcome) {
                ReadOutcome::Data($count) => "read {$count}",
                ReadOutcome::EndOfStream => "read end",
                default => "read without data",
            },
            TransferCompletion::Write($key, $outcome) => "write",
            TransferCompletion::Ready($key) => "ready",
        },
        TransferWaitResult::TimedOut => "timed out",
        TransferWaitResult::Exhausted => "exhausted",
    };
}
"#;

/// What a program does after `Ready`: take every waiting event.
const DRAIN_EVENTS: &str = r#"
function drain(RawMode $raw): string throws TerminalError {
    let writable $seen = "";
    while (true) {
        let $result = $raw->getNextEvent(Wait::Immediate);
        if ($result == TerminalWaitResult::TimedOut) {
            return $seen . "quiet";
        }
        if ($result == TerminalWaitResult::Closed) {
            return $seen . "closed";
        }
        $seen = $seen . "event ";
    }
}
"#;

const WATCH_IMPORTS: &str = r#"
use Doria\Std\Io\{File, IoError, IoErrorReason, IoOperation, IoTarget, ReadOutcome, ReadinessSet,
    TransferCompletion, TransferPlan, TransferWaitResult, Wait};
use Doria\Std\Term\{Console, RawMode, TerminalError, TerminalWaitResult};
"#;

fn watch_program(body: &str) -> String {
    format!("{WATCH_IMPORTS}{DESCRIBE_COMPLETION}{DRAIN_EVENTS}{body}")
}

fn transcript(source: &str, events: Vec<Observation>) -> String {
    transcript_with(
        source,
        MirIo {
            terminal: Some(terminal(events)),
            ..MirIo::default()
        },
    )
}

fn transcript_with(source: &str, io: MirIo) -> String {
    let output = interpret(source, io).unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.exit_status,
        0,
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
fn a_plan_of_one_watch_reports_ready_once_and_is_then_exhausted() {
    let source = watch_program(
        r#"
function main(): void throws IoError, TerminalError {
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    {
        let $plan = TransferPlan::watch($raw);
        let $key = $plan->getWatchKey(0);
        let writable $group = $set->begin($plan);
        if ($group->observedWatchKey != null) { panic("a watch was observed before any fetch"); }
        let $first = $group->getNextCompletion(Wait::Indefinite);
        echo describe($first) . "\n";
        if ($first != TransferWaitResult::Completed(TransferCompletion::Ready($key))) { panic("Ready carried another key"); }
        if ($group->observedWatchKey != $key || $group->observedReadKey != null
            || $group->observedWriteKey != null) { panic("Ready did not identify its watch alone"); }
        let $second = $group->getNextCompletion(Wait::Immediate);
        echo describe($second) . "\n";
        if ($group->observedWatchKey != null) { panic("an exhausted fetch kept the watch key"); }
        let $third = $group->getNextCompletion(Wait::Indefinite);
        echo describe($third) . "\n";
        echo drain($raw) . "\n";
    }
    $raw->leave();
}
"#,
    );
    // Reporting takes nothing from the event queue: the key is still there.
    assert_eq!(
        transcript(&source, vec![pressed(Key::Enter)]),
        "ready\nexhausted\nexhausted\nevent closed\n"
    );
}

#[test]
fn a_watch_is_armed_again_only_by_resubmission_after_its_events_are_drained() {
    let source = watch_program(
        r#"
function main(): void throws IoError, TerminalError {
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    {
        let $plan = TransferPlan::watch($raw);
        let $key = $plan->getWatchKey(0);
        let writable $group = $set->begin($plan);
        let $first = $group->getNextCompletion(Wait::Indefinite);
        echo describe($first) . "\n";
        echo drain($raw) . "\n";
        $group->resubmitWatch($key);
        let $second = $group->getNextCompletion(Wait::Immediate);
        echo describe($second) . "\n";
        if ($group->observedWatchKey != null) { panic("a timed out fetch kept the watch key"); }
        let $third = $group->getNextCompletion(Wait::Indefinite);
        echo describe($third) . "\n";
        if ($group->observedWatchKey != $key) { panic("the armed watch lost its key"); }
        echo drain($raw) . "\n";
        $group->resubmitWatch($key);
        let $fourth = $group->getNextCompletion(Wait::Immediate);
        echo describe($fourth) . "\n";
        let $fifth = $group->getNextCompletion(Wait::Indefinite);
        echo describe($fifth) . "\n";
    }
    $raw->leave();
}
"#,
    );
    assert_eq!(
        transcript(
            &source,
            vec![
                pressed(Key::Character('a')),
                Observation::TimedOut,
                Observation::TimedOut,
                pressed(Key::Enter),
            ]
        ),
        // The drain takes the key and one quiet moment; the armed watch then
        // finds the next moment quiet too, and the Enter after it ready. A
        // terminal that has gone away is ready as well: its wait says Closed.
        "ready\nevent quiet\ntimed out\nready\nevent closed\nready\nexhausted\n"
    );
}

#[test]
fn a_wait_with_only_a_quiet_watch_times_out_unless_it_is_indefinite() {
    let source = watch_program(
        r#"
use Doria\Std\Time\Duration;
function main(): void throws IoError, TerminalError {
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    {
        let writable $group = $set->begin(TransferPlan::watch($raw));
        let $immediate = $group->getNextCompletion(Wait::Immediate);
        echo describe($immediate) . "\n";
        let $bounded = $group->getNextCompletion(Wait::For(Duration::fromNanoseconds(1)));
        echo describe($bounded) . "\n";
        let $indefinite = $group->getNextCompletion(Wait::Indefinite);
        echo describe($indefinite) . "\n";
        echo drain($raw) . "\n";
    }
    $raw->leave();
}
"#,
    );
    assert_eq!(
        transcript(
            &source,
            vec![
                Observation::TimedOut,
                Observation::TimedOut,
                Observation::TimedOut,
                Observation::Resized {
                    columns: 100,
                    rows: 30,
                },
            ]
        ),
        "timed out\ntimed out\nready\nevent closed\n"
    );
}

/// A read of an in-memory file beside a watch. `PLAN` builds the plan and
/// `READ` and `WATCH` are the two entries' positions in it.
const READ_BESIDE_WATCH: &str = r#"
function main(): void throws IoError, TerminalError {
    write_file("input", "ab");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(2);
    let $raw = Console::enterRawMode();
    {
        let $plan = PLAN;
        let $read = $plan->getReadKey(READ);
        let $watch = $plan->getWatchKey(WATCH);
        let writable $group = $set->begin($plan);
        let writable $fetches = 0;
        while ($fetches < 3) {
            let $result = $group->getNextCompletion(Wait::Indefinite);
            echo describe($result) . "\n";
            if ($result == TransferWaitResult::Completed(TransferCompletion::Ready($watch))) {
                if ($group->observedWatchKey != $watch || $group->observedReadKey != null) { panic("Ready observed a transfer key"); }
            } else if ($result == TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::Data(2)))) {
                if ($group->observedReadKey != $read || $group->observedWatchKey != null) { panic("a read observed the watch key"); }
            } else if ($result != TransferWaitResult::Exhausted || $group->observedWatchKey != null
                || $group->observedReadKey != null) { panic("an unexpected completion"); }
            $fetches += 1;
        }
    }
    if ($buffer[0] != 97 || $buffer[1] != 98) { panic("the read did not fill its buffer"); }
    $raw->leave();
}
"#;

#[test]
fn a_ready_watch_is_reported_before_a_read_that_is_also_complete() {
    for (plan, read, watch) in [
        (
            "TransferPlan::read($registration, $buffer->getWritableRegion(0, 2))->andWatch($raw)",
            "0",
            "1",
        ),
        (
            "TransferPlan::watch($raw)->andReadFrom($registration, $buffer->getWritableRegion(0, 2))",
            "1",
            "0",
        ),
    ] {
        let source = watch_program(
            &READ_BESIDE_WATCH
                .replace("PLAN", plan)
                .replace("READ", read)
                .replace("WATCH", watch),
        );
        assert_eq!(
            transcript(&source, vec![pressed(Key::Enter)]),
            "ready\nread 2\nexhausted\n",
            "{plan}"
        );
        // A terminal with nothing yet lets the read report first; the key
        // that arrives afterwards is reported by the next fetch.
        assert_eq!(
            transcript(&source, vec![Observation::TimedOut, pressed(Key::Enter)]),
            "read 2\nready\nexhausted\n",
            "{plan}"
        );
    }
}

/// Whether a failure carries the labels a rejected plan or group request has.
const REFUSED: &str = r#"
function refused(IoError $error, IoOperation $operation, IoTarget $target): bool {
    return $error->operation == $operation && $error->target == $target
        && $error->reason == IoErrorReason::InvalidInput
        && $error->systemCode == null && $error->transferredByteCount == null;
}
"#;

#[test]
fn a_watch_key_is_found_only_at_the_position_of_a_watch() {
    let source = watch_program(&format!(
        r#"{REFUSED}
function main(): void throws IoError, TerminalError {{
    write_file("input", "ab");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(2);
    let $raw = Console::enterRawMode();
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 2))->andWatch($raw);
        let $read = $plan->getReadKey(0);
        let $watch = $plan->getWatchKey(1);
        let writable $refusals = 0;
        try {{ $plan->getWatchKey(0); panic("a read position gave a watch key"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Access, IoTarget::TransferPlan)) {{ panic("wrong failure for a read position"); }}
            echo $error->message . "\n";
            $refusals += 1;
        }}
        try {{ $plan->getWatchKey(2); panic("a position past the plan gave a watch key"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Access, IoTarget::TransferPlan)) {{ panic("wrong failure past the plan"); }}
            $refusals += 1;
        }}
        try {{ $plan->getWatchKey(-1); panic("a negative position gave a watch key"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Access, IoTarget::TransferPlan)) {{ panic("wrong failure for a negative position"); }}
            $refusals += 1;
        }}
        try {{ $plan->getReadKey(1); panic("a watch position gave a read key"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Access, IoTarget::TransferPlan)) {{ panic("wrong read-key failure for a watch"); }}
            $refusals += 1;
        }}
        try {{ $plan->getWriteKey(1); panic("a watch position gave a write key"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Access, IoTarget::TransferPlan)) {{ panic("wrong write-key failure for a watch"); }}
            $refusals += 1;
        }}
        if ($plan->getWatchKey(1) != $watch || $plan->getReadKey(0) != $read) {{ panic("a key changed between lookups"); }}
        echo "{{$refusals}} refused\n";
    }}
    $raw->leave();
}}
"#
    ));
    assert_eq!(
        transcript(&source, Vec::new()),
        "failed to access transfer plan: invalid input\n5 refused\n"
    );
}

#[test]
fn a_watch_is_resubmitted_only_by_its_own_group_after_its_ready_was_fetched() {
    let source = watch_program(&format!(
        r#"{REFUSED}
use Doria\Std\Concurrency\{{CancellationError, CancellationSource}};
function main(): void throws IoError, TerminalError, CancellationError {{
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    let $other = TransferPlan::watch($raw);
    let $foreign = $other->getWatchKey(0);
    let writable $source = new CancellationSource();
    {{
        let $plan = TransferPlan::watch($raw);
        let $key = $plan->getWatchKey(0);
        let writable $group = $set->begin($plan, cancellation: $source->token);
        let writable $refusals = 0;
        try {{ $group->resubmitWatch($key); panic("an armed watch was resubmitted"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Configure, IoTarget::TransferGroup)) {{ panic("wrong failure for an armed watch"); }}
            echo $error->message . "\n";
            $refusals += 1;
        }}
        let $first = $group->getNextCompletion(Wait::Indefinite);
        echo describe($first) . "\n";
        try {{ $group->resubmitWatch($foreign); panic("another plan's watch key was accepted"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Configure, IoTarget::TransferGroup)) {{ panic("wrong failure for a foreign key"); }}
            $refusals += 1;
        }}
        $group->resubmitWatch($key);
        try {{ $group->resubmitWatch($key); panic("a watch was armed twice"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Configure, IoTarget::TransferGroup)) {{ panic("wrong failure for a second arming"); }}
            $refusals += 1;
        }}
        $source->cancel();
        let $stopped = $group->getNextCompletion(Wait::Indefinite);
        echo describe($stopped) . "\n";
        if ($group->observedWatchKey != $key) {{ panic("the stopped watch lost its key"); }}
        let $after = $group->getNextCompletion(Wait::Indefinite);
        echo describe($after) . "\n";
        try {{ $group->resubmitWatch($key); panic("a stopped group armed a watch"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Configure, IoTarget::TransferGroup)) {{ panic("wrong failure for a stopped group"); }}
            $refusals += 1;
        }}
        let $last = $group->getNextCompletion(Wait::Immediate);
        echo describe($last) . "\n";
        echo "{{$refusals}} refused\n";
    }}
    // The other plan still watches the raw mode, so it is not left here: it
    // ends with its scope, after that plan.
}}
"#
    ));
    assert_eq!(
        transcript(&source, vec![pressed(Key::Enter)]),
        "failed to configure transfer group: invalid input\nready\nready\nexhausted\nexhausted\n\
         4 refused\n"
    );
}

#[test]
fn a_stopped_group_reports_an_armed_watch_once_and_then_its_pending_read() {
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
        let source = watch_program(&format!(
            r#"{REFUSED}
use Doria\Std\Concurrency\{{CancellationError, CancellationSource}};
use Doria\Std\Time\{{ClockError, Deadline, Duration}};
function main(): void throws IoError, TerminalError, CancellationError, ClockError {{
    write_file("input", "ab");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(2);
    $buffer[0] = 9;
    let $raw = Console::enterRawMode();
    {setup}
    {{
        let $plan = TransferPlan::watch($raw);
        let $key = $plan->getWatchKey(0);
        let writable $group = $set->begin($plan, {control});
        let $first = $group->getNextCompletion(Wait::Immediate);
        echo describe($first) . "\n";
        if ($first != TransferWaitResult::Completed(TransferCompletion::Ready($key))
            || $group->observedWatchKey != $key) {{ panic("the stopped watch was not identified"); }}
        let $second = $group->getNextCompletion(Wait::Indefinite);
        echo describe($second) . "\n";
        try {{ $group->resubmitWatch($key); panic("a stopped group armed a watch"); }}
        catch (IoError $error) {{
            if (!refused($error, IoOperation::Configure, IoTarget::TransferGroup)) {{ panic("wrong failure for a stopped group"); }}
        }}
    }}
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 2))->andWatch($raw);
        let $read = $plan->getReadKey(0);
        let writable $group = $set->begin($plan, {control});
        let writable $fetches = 0;
        while ($fetches < 3) {{
            let $result = $group->getNextCompletion(Wait::Indefinite);
            echo describe($result) . "\n";
            $fetches += 1;
        }}
        if ($group->getReadRegion($read) != null) {{ panic("a stopped read exposed bytes"); }}
    }}
    if ($buffer[0] != 9) {{ panic("a stopped read changed its buffer"); }}
    // The stopped groups asked the terminal nothing: its quiet moment is still there.
    echo drain($raw) . "\n";
    $raw->leave();
}}
"#
        ));
        assert_eq!(
            transcript(&source, vec![Observation::TimedOut]),
            "ready\nexhausted\nready\nread without data\nexhausted\nquiet\n",
            "{control}"
        );
    }
}

#[test]
fn the_raw_mode_stays_usable_while_a_group_watches_it() {
    let source = watch_program(
        r#"
function main(): void throws IoError, TerminalError {
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    {
        let writable $group = $set->begin(TransferPlan::watch($raw));
        let $position = $raw->getCursorPosition();
        echo "cursor {$position->column},{$position->row}\n";
        echo drain($raw) . "\n";
        let $result = $group->getNextCompletion(Wait::Immediate);
        echo describe($result) . "\n";
    }
    $raw->leave();
}
"#,
    );
    assert_eq!(
        transcript(&source, vec![pressed(Key::Up), Observation::TimedOut]),
        "cursor 9,4\nevent quiet\nready\n"
    );
}

#[test]
fn independent_validation_rejects_forged_watch_operands() {
    let source = watch_program(
        r#"
function main(): void throws IoError, TerminalError {
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    let $plan = TransferPlan::watch($raw);
    let $key = $plan->getWatchKey(0);
    let writable $group = $set->begin($plan);
    $group->resubmitWatch($key);
}
"#,
    );
    let program = doriac::lower_source_to_mir("watch.doria", &source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    // A watch takes a borrowed source and that source's identity, in that
    // order; a resubmission takes a watch key, never the group itself.
    for forge_plan in [true, false] {
        let mut forged = program.clone();
        let mut altered = false;
        for function in &mut forged.functions {
            for block in &mut function.blocks {
                let mir::Terminator::CheckedIo { operation, .. } = &mut block.terminator else {
                    continue;
                };
                match operation {
                    mir::CheckedIoOperation::WatchTransferPlan {
                        source, identity, ..
                    } if forge_plan => {
                        std::mem::swap(source, identity);
                        altered = true;
                    }
                    mir::CheckedIoOperation::ResubmitWatch { group, key } if !forge_plan => {
                        *key = *group;
                        altered = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(altered);
        assert!(doriac::mir_validation::validate_program(&forged).is_err());
    }
}

/// The diagnostics of a program that must not compile, as code, message and help.
fn rejected(source: &str) -> Vec<(String, String, String)> {
    match doriac::check_source("watch.doria", source) {
        Ok(_) => panic!("the program compiles: {source}"),
        Err(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.code.to_string(),
                    diagnostic.message.clone(),
                    diagnostic.help.clone().unwrap_or_default(),
                )
            })
            .collect(),
    }
}

#[test]
fn leaving_or_moving_a_watched_raw_mode_is_rejected_with_the_fix() {
    for (label, body) in [
        (
            "a plan watches it",
            r#"
    let $raw = Console::enterRawMode();
    let $plan = TransferPlan::watch($raw);
    $raw->leave();
"#,
        ),
        (
            "a group watches it",
            r#"
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    let writable $group = $set->begin(TransferPlan::watch($raw));
    $raw->leave();
"#,
        ),
        (
            "a group watches it beside a read",
            r#"
    write_file("input", "ab");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(2);
    let $raw = Console::enterRawMode();
    let writable $group = $set->begin(
        TransferPlan::read($registration, $buffer->getWritableRegion(0, 2))->andWatch($raw));
    $raw->leave();
"#,
        ),
        (
            "it is given away while a group watches it",
            r#"
    let $raw = Console::enterRawMode();
    let writable $set = new ReadinessSet();
    let writable $group = $set->begin(TransferPlan::watch($raw));
    finish($raw);
"#,
        ),
    ] {
        let source = watch_program(&format!(
            r#"
function finish(take RawMode $raw): void throws TerminalError {{ $raw->leave(); }}
function main(): void throws IoError, TerminalError {{{body}}}
"#
        ));
        let diagnostics = rejected(&source);
        assert!(
            diagnostics
                .iter()
                .any(|(code, message, help)| code == "E0763"
                    && message.contains("readonly use by a borrow-bound value")
                    && help.contains("end the borrow-bound value before giving the source away")
                    && help.contains("closing the block that declares it")),
            "{label}: {diagnostics:#?}"
        );
    }
}

#[test]
fn a_group_cannot_outlive_the_raw_mode_it_watches() {
    let source = watch_program(
        r#"
use Doria\Std\Io\TransferGroup;
function watching(writable ReadinessSet $set): TransferGroup throws IoError, TerminalError {
    let $raw = Console::enterRawMode();
    return $set->begin(TransferPlan::watch($raw));
}
function main(): void {}
"#,
    );
    let diagnostics = rejected(&source);
    assert!(
        diagnostics
            .iter()
            .any(|(code, _, help)| (code == "E0762" || code == "E0763") && !help.is_empty()),
        "{diagnostics:#?}"
    );
}

#[test]
fn only_the_standard_library_implements_watchable() {
    for (label, declarations) in [
        ("directly", "class Panel implements Watchable {"),
        (
            "through an interface that extends it",
            "interface Screen extends Watchable {}\nclass Panel implements Screen {",
        ),
    ] {
        let source = format!(
            r#"
use Doria\Std\Io\{{Watchable, WatchSource}};
use Doria\Std\Term\RawMode;
{declarations}
    internal RawMode $raw;
    function __construct(parameter take RawMode $raw) {{ $this->raw = $raw; }}
    WatchSource $watchSource {{ get => $this->raw->watchSource; }}
}}
function main(): void {{}}
"#
        );
        let diagnostics = rejected(&source);
        assert!(
            diagnostics
                .iter()
                .any(|(code, message, help)| code == "E0781"
                    && message
                        .contains("class `Panel` cannot implement `Doria\\Std\\Io\\Watchable`")
                    && help.contains("`TransferPlan::watch` or `andWatch`")
                    && help.contains("passes it instead of implementing the interface")),
            "{label}: {diagnostics:#?}"
        );
    }
}

#[test]
fn a_program_cannot_make_a_watch_source_of_its_own() {
    for expression in ["new WatchSource()", "WatchSource::fromBits(4294967297)"] {
        let source = format!(
            r#"
use Doria\Std\Io\WatchSource;
function main(): void {{
    let $source = {expression};
}}
"#
        );
        let diagnostics = rejected(&source);
        assert!(
            diagnostics
                .iter()
                .any(|(code, message, _)| code == "E0307" && message.contains("is internal")),
            "{expression}: {diagnostics:#?}"
        );
    }
}

// A read of a command's output in a group (Decision 0140): while the command
// is alive and has written nothing more, the read stays pending, as it does
// natively, and a later fetch completes it once the command has written or
// ended. Only a wait that nothing but the program itself could end is refused.

/// A scripted terminal beside a scripted command named `copy` that copies its
/// input to its output and ends when its input does, like `cat`.
fn with_copy_command(events: Vec<Observation>) -> MirIo {
    MirIo {
        terminal: Some(terminal(events)),
        programs: [(
            "copy".to_string(),
            MirProgram {
                echo_input: true,
                exit: Some(MirExitStatus::Exited(0)),
                ..MirProgram::default()
            },
        )]
        .into(),
        ..MirIo::default()
    }
}

const COMMAND_IMPORTS: &str = r#"
use Doria\Std\Io\BorrowedBufferedReader;
use Doria\Std\Process\{ChildCleanup, ChildOutput, Command};
"#;

/// The reason the interpreter gives for a wait only the program could end.
const DEAD_END: &str =
    "reading child output that only this program's own later writes could produce";

#[test]
fn a_read_of_a_quiet_command_stays_pending_beside_a_watch_until_the_command_writes() {
    let source = watch_program(&format!(
        r#"{COMMAND_IMPORTS}
function main(): void throws IoError, TerminalError {{
    let $raw = Console::enterRawMode();
    let writable $child = Command::executable("copy")->spawn(ChildCleanup::Terminate);
    writable Bytes $message = Bytes::allocate(2);
    $message[0] = 104;
    $message[1] = 105;
    writable Bytes $storage = Bytes::allocate(8);
    {{
        let writable $reader = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 8);
        let writable $set = new ReadinessSet();
        let $tap = $set->register($reader);
        {{
            let $plan = TransferPlan::read($tap, $storage->getWritableRegion(0, 8))->andWatch($raw);
            let $read = $plan->getReadKey(0);
            let $watch = $plan->getWatchKey(1);
            let writable $group = $set->begin($plan);
            let $quiet = $group->getNextCompletion(Wait::Immediate);
            echo describe($quiet) . "\n";
            if ($group->getReadRegion($read) != null) {{ panic("a waiting read exposed bytes"); }}
            let $first = $group->getNextCompletion(Wait::Indefinite);
            echo describe($first) . "\n";
            if ($first != TransferWaitResult::Completed(TransferCompletion::Ready($watch))
                || $group->observedReadKey != null) {{ panic("the key was not reported by the watch"); }}
            echo drain($raw) . "\n";
            let $still = $group->getNextCompletion(Wait::Immediate);
            echo describe($still) . "\n";
            let $written = $child->getInput()->write($message->getRegion(0, 2));
            let $second = $group->getNextCompletion(Wait::Indefinite);
            echo describe($second) . "\n";
            if ($second != TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::Data(2)))
                || $group->observedReadKey != $read || $group->observedWatchKey != null) {{ panic("the read did not report the copied bytes"); }}
            {{ let $region = $group->getReadRegion($read); if ($region == null || $region->length != 2) {{ panic("the copied bytes are not readable"); }} }}
            $group->resubmitRead($read, 0, 8);
            let $again = $group->getNextCompletion(Wait::Immediate);
            echo describe($again) . "\n";
            $child->getInput()->closeInput();
            let $ended = $group->getNextCompletion(Wait::Indefinite);
            echo describe($ended) . "\n";
            let $last = $group->getNextCompletion(Wait::Indefinite);
            echo describe($last) . "\n";
        }}
        let $returned = $set->unregister($tap);
    }}
    if ($storage[0] != 104 || $storage[1] != 105) {{ panic("the copied bytes did not reach the buffer"); }}
    $raw->leave();
}}
"#
    ));
    assert_eq!(
        transcript_with(
            &source,
            with_copy_command(vec![
                Observation::TimedOut,
                pressed(Key::Character('x')),
                Observation::TimedOut,
            ])
        ),
        // Nothing is ready at first. The key is reported while the read still
        // waits; the read reports once the command has copied the program's
        // write, waits again when resubmitted, and ends with the command.
        "timed out\nready\nevent quiet\ntimed out\nread 2\ntimed out\nread end\nexhausted\n"
    );
}

#[test]
fn a_stopped_group_retires_a_read_that_was_still_waiting_on_a_command() {
    let source = watch_program(&format!(
        r#"{COMMAND_IMPORTS}
use Doria\Std\Concurrency\{{CancellationError, CancellationSource}};
function main(): void throws IoError, TerminalError, CancellationError {{
    let $raw = Console::enterRawMode();
    let writable $child = Command::executable("copy")->spawn(ChildCleanup::Terminate);
    writable Bytes $storage = Bytes::allocate(8);
    $storage[0] = 9;
    let writable $source = new CancellationSource();
    {{
        let writable $reader = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 8);
        let writable $set = new ReadinessSet();
        let $tap = $set->register($reader);
        {{
            let $plan = TransferPlan::read($tap, $storage->getWritableRegion(0, 8))->andWatch($raw);
            let $read = $plan->getReadKey(0);
            let writable $group = $set->begin($plan, cancellation: $source->token);
            let $quiet = $group->getNextCompletion(Wait::Immediate);
            echo describe($quiet) . "\n";
            $source->cancel();
            let writable $fetches = 0;
            while ($fetches < 3) {{
                let $result = $group->getNextCompletion(Wait::Indefinite);
                echo describe($result) . "\n";
                $fetches += 1;
            }}
            if ($group->getReadRegion($read) != null) {{ panic("a stopped read exposed bytes"); }}
        }}
        let $returned = $set->unregister($tap);
    }}
    if ($storage[0] != 9) {{ panic("a stopped read changed its buffer"); }}
    echo drain($raw) . "\n";
    $raw->leave();
}}
"#
    ));
    assert_eq!(
        transcript_with(
            &source,
            with_copy_command(vec![Observation::TimedOut, Observation::TimedOut])
        ),
        "timed out\nready\nread without data\nexhausted\nquiet\n"
    );
}

/// A group whose only entry reads a command that has written nothing. `BEGIN`
/// begins the group.
const READ_OF_A_QUIET_COMMAND: &str = r#"
use Doria\Std\Time\{ClockError, Deadline, Duration};
function main(): void throws IoError, ClockError {
    let writable $child = Command::executable("copy")->spawn(ChildCleanup::Terminate);
    writable Bytes $storage = Bytes::allocate(8);
    let $far = Deadline::fromNow(Duration::fromNanoseconds(9223372036854775807));
    {
        let writable $reader = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 8);
        let writable $set = new ReadinessSet();
        let $tap = $set->register($reader);
        {
            let $plan = TransferPlan::read($tap, $storage->getWritableRegion(0, 8));
            let writable $group = BEGIN;
            let $quiet = $group->getNextCompletion(Wait::Immediate);
            echo describe($quiet) . "\n";
            let $bounded = $group->getNextCompletion(Wait::For(Duration::fromNanoseconds(1)));
            echo describe($bounded) . "\n";
            let $first = $group->getNextCompletion(Wait::Indefinite);
            echo describe($first) . "\n";
            let $last = $group->getNextCompletion(Wait::Indefinite);
            echo describe($last) . "\n";
        }
        let $returned = $set->unregister($tap);
    }
}
"#;

#[test]
fn an_indefinite_wait_on_a_quiet_command_ends_only_at_the_group_deadline() {
    let program = |begin: &str| {
        watch_program(&format!(
            "{COMMAND_IMPORTS}{}",
            READ_OF_A_QUIET_COMMAND.replace("BEGIN", begin)
        ))
    };
    // Waits take no time in the interpreter, so a wait that only the group's
    // deadline can end reaches that deadline: the read is retired unread.
    assert_eq!(
        transcript_with(
            &program("$set->begin($plan, deadline: $far)"),
            with_copy_command(Vec::new())
        ),
        "timed out\ntimed out\nread without data\nexhausted\n"
    );
    // Without a deadline nothing but the program's own later write could end
    // the wait, and the program is waiting: the run stops with that reason.
    assert_eq!(
        interpret(
            &program("$set->begin($plan)"),
            with_copy_command(Vec::new())
        ),
        Err(DEAD_END.to_string())
    );
}

#[test]
fn a_wait_only_the_program_could_end_is_still_refused() {
    // A watch that is not armed cannot end the wait either.
    let beside_an_idle_watch = watch_program(&format!(
        r#"{COMMAND_IMPORTS}
function main(): void throws IoError, TerminalError {{
    let $raw = Console::enterRawMode();
    let writable $child = Command::executable("copy")->spawn(ChildCleanup::Terminate);
    writable Bytes $storage = Bytes::allocate(8);
    let writable $reader = new BorrowedBufferedReader<ChildOutput>($child->getOutput(), 8);
    let writable $set = new ReadinessSet();
    let $tap = $set->register($reader);
    let $plan = TransferPlan::read($tap, $storage->getWritableRegion(0, 8))->andWatch($raw);
    let writable $group = $set->begin($plan);
    let $first = $group->getNextCompletion(Wait::Indefinite);
    echo describe($first) . "\n";
    let $never = $group->getNextCompletion(Wait::Indefinite);
}}
"#
    ));
    assert_eq!(
        interpret(
            &beside_an_idle_watch,
            with_copy_command(vec![pressed(Key::Enter)])
        ),
        Err(DEAD_END.to_string())
    );
    // A plain read has no group to keep it pending.
    let plain_read = watch_program(&format!(
        r#"{COMMAND_IMPORTS}
function main(): void throws IoError {{
    let writable $child = Command::executable("copy")->spawn(ChildCleanup::Terminate);
    writable Bytes $storage = Bytes::allocate(8);
    let $outcome = $child->getOutput()->read($storage->getWritableRegion(0, 8));
}}
"#
    ));
    assert_eq!(
        interpret(&plain_read, with_copy_command(Vec::new())),
        Err(DEAD_END.to_string())
    );
}
