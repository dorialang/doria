//! The Stage 46 acceptance test (Decision 0140): one program, compiled
//! natively and run inside a pseudo-terminal, shows the same screens on
//! every tier-1 platform. The menu's expected screens come from the
//! interpreter's scripted terminal, so the native backends, the runtime's
//! platform backend and the interpreter are all held to one answer.
//!
//! A transfer group that watches the terminal is held to the interpreter
//! the same way. So is the relay, which also waits on a command's output:
//! the order its one wait reports things in is written here in words, the
//! interpreter's screens must be those words, and every backend's screens
//! must be the interpreter's.

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[allow(dead_code)]
#[path = "common/native_execution.rs"]
mod native_execution;

use doria_pseudo_terminal::{Screen, Session};
use doria_terminal::wire::{
    ColorSupport, Key, Observation, MODIFIER_ALT, MODIFIER_CONTROL, MODIFIER_SHIFT,
};
use doriac::backend::NativeProfile;
use doriac::mir_interpreter::{
    interpret_with_io, InterpreterOutput, MirExitStatus, MirIo, MirProgram, MirTerminal,
};
use native_execution::{stage_executable, ExecutionDirectory};

const MENU_PATH: &str = "menu.doria";
const MENU: &str = include_str!("../../../examples/future/stage46/menu.doria");
const RELAY_PATH: &str = "command_output_and_keys.doria";
const RELAY: &str = include_str!("../../../examples/future/stage46/command_output_and_keys.doria");
const COLUMNS: u16 = 80;
const ROWS: u16 = 24;

const FRAME: &[u8] = b"\x1b[2J";
/// How a program's last frame ends: it shows the cursor again itself, or it
/// ends and the runtime puts the terminal back, first screen first.
const LEAVE: [&[u8]; 2] = [b"\x1b[?25h", b"\x1b[?1049l"];

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|found| found + from)
}

/// One thing that happens to the running program.
#[derive(Clone, Copy)]
enum Step {
    Press(Key),
    /// These bytes are typed, which a terminal reports as this key with
    /// these modifiers.
    Typed(&'static [u8], Key, u8),
    /// The window changes to this many columns and rows.
    Resize(u16, u16),
}

fn presses(keys: &[Key]) -> Vec<Step> {
    keys.iter().copied().map(Step::Press).collect()
}

/// What the terminal shows: the alternate screen the program has drawn when
/// it waits for each step, the main screen once it has ended, and the exit
/// status.
struct Expected {
    frames: Vec<String>,
    ended: String,
    status: i32,
}

/// What a step is to the interpreter's scripted terminal.
fn observed(step: Step) -> Observation {
    match step {
        Step::Press(key) => Observation::Key { key, modifiers: 0 },
        Step::Typed(_, key, modifiers) => Observation::Key { key, modifiers },
        Step::Resize(columns, rows) => Observation::Resized {
            columns: u32::from(columns),
            rows: u32::from(rows),
        },
    }
}

/// The terminal the interpreter scripts: `events` is what it observes.
fn scripted(events: Vec<Observation>) -> MirTerminal {
    MirTerminal {
        columns: u32::from(COLUMNS),
        rows: u32::from(ROWS),
        color_support: ColorSupport::Indexed,
        events,
        cursor: None,
    }
}

/// The screens of an interpreter run: the alternate screen after each frame
/// the program drew, the main screen once it has ended, and the exit status.
/// `shown[step]` is how many frames the program had drawn when that step was
/// taken, so a resize is in effect for every frame after those.
fn replayed(output: &InterpreterOutput, steps: &[Step], shown: &[usize]) -> Expected {
    let drawn = &output.stdout;
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(start) = find(drawn, FRAME, from) {
        starts.push(start);
        from = start + FRAME.len();
    }
    let last = *starts.last().expect("the program draws");
    let leave = LEAVE
        .iter()
        .filter_map(|ending| find(drawn, ending, last))
        .min()
        .expect("the program leaves");
    let mut screen = Screen::new(COLUMNS, ROWS);
    let mut frames = Vec::new();
    let mut fed = 0;
    for (frame, end) in starts[1..].iter().copied().chain([leave]).enumerate() {
        for (step, taken_after) in steps.iter().zip(shown) {
            if let (Step::Resize(columns, rows), true) = (*step, *taken_after == frame) {
                screen.resize(columns, rows);
            }
        }
        screen.feed(&drawn[fed..end]);
        fed = end;
        assert!(screen.on_alternate_screen());
        frames.push(screen.text());
    }
    screen.feed(&drawn[fed..]);
    // A report on standard error follows on the same terminal, which by then
    // starts each new line at the left edge again.
    for line in output.stderr.split_inclusive(|byte| *byte == b'\n') {
        match line.strip_suffix(b"\n") {
            Some(text) => {
                screen.feed(text);
                screen.feed(b"\r\n");
            }
            None => screen.feed(line),
        }
    }
    assert!(!screen.on_alternate_screen() && screen.cursor_is_visible());
    Expected {
        frames,
        ended: screen.text(),
        status: output.exit_status,
    }
}

/// What the interpreter says a program shows when it draws one frame, then
/// waits for a step. `events` is what its scripted terminal observes.
fn interpreted(
    program: &doriac::mir::Program,
    events: Vec<Observation>,
    steps: &[Step],
) -> Expected {
    let output = interpret_with_io(
        program,
        MirIo {
            terminal: Some(scripted(events)),
            ..MirIo::default()
        },
    )
    .unwrap()
    .output;
    let shown = (1..=steps.len()).collect::<Vec<_>>();
    let expected = replayed(&output, steps, &shown);
    assert_eq!(
        expected.frames.len(),
        steps.len(),
        "one frame before each step"
    );
    expected
}

fn key_bytes(key: Key) -> Vec<u8> {
    match key {
        Key::Up => b"\x1b[A".to_vec(),
        Key::Down => b"\x1b[B".to_vec(),
        Key::Enter => b"\r".to_vec(),
        Key::Escape => b"\x1b".to_vec(),
        Key::Character(character) => character.to_string().into_bytes(),
        other => panic!("no bytes for {other:?}"),
    }
}

/// One way of running a compiled program.
struct Launch {
    what: String,
    program: std::path::PathBuf,
    arguments: Vec<String>,
    /// Keeps the built files in place until the run is over.
    _directory: ExecutionDirectory,
}

impl Launch {
    /// The same launch with these arguments for the program itself.
    fn with_arguments(mut self, arguments: &[&str]) -> Self {
        self.arguments
            .extend(arguments.iter().map(|argument| argument.to_string()));
        self
    }
}

fn native(program: &doriac::mir::Program, profile: NativeProfile) -> Launch {
    let directory = ExecutionDirectory::new();
    Launch {
        what: format!("{profile:?}"),
        program: stage_executable(program, profile, &directory),
        arguments: Vec::new(),
        _directory: directory,
    }
}

/// PHP output beside the native runtime it loads.
fn php(path: &str, source: &str) -> Launch {
    let artifact = doriac::compile_source(path, source, doriac::backend::BackendTarget::Php)
        .unwrap_or_else(|diagnostics| panic!("{path} does not compile to PHP: {diagnostics:#?}"));
    let directory = ExecutionDirectory::new();
    let script = directory
        .path()
        .join(std::path::Path::new(path).with_extension("php"));
    doriac::backend::publish_output(&script, artifact).unwrap();
    Launch {
        what: "PHP".into(),
        program: "php".into(),
        arguments: vec![
            "-d".into(),
            "ffi.enable=1".into(),
            script.to_string_lossy().into_owned(),
        ],
        _directory: directory,
    }
}

fn lower(path: &str, source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir(path, source)
        .unwrap_or_else(|diagnostics| panic!("{path} does not compile: {diagnostics:#?}"))
}

/// Runs a compiled program in a pseudo-terminal: waits for the screen
/// expected before each step, takes the step, and checks how the run ends.
fn shows_the_expected_screens(launch: Launch, expected: &Expected, steps: &[Step]) {
    assert_eq!(expected.frames.len(), steps.len());
    let what = &launch.what;
    let arguments = launch
        .arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut session = Session::start(
        &launch.program,
        &arguments,
        &[("TERM", "xterm-256color")],
        COLUMNS,
        ROWS,
    );
    let mut screen = Screen::new(COLUMNS, ROWS);
    let mut fed = 0;
    for (number, (frame, step)) in expected.frames.iter().zip(steps).enumerate() {
        session.wait_until(
            &format!("frame {} ({what}):\n{frame}", number + 1),
            |drawn| {
                screen.feed(&drawn[fed..]);
                fed = drawn.len();
                screen.on_alternate_screen() && screen.text() == *frame
            },
        );
        assert!(!screen.cursor_is_visible(), "{what}: the cursor is hidden");
        match *step {
            Step::Press(key) => session.type_bytes(&key_bytes(key)),
            Step::Typed(bytes, ..) => session.type_bytes(bytes),
            Step::Resize(columns, rows) => {
                session.resize(columns, rows);
                screen.resize(columns, rows);
            }
        }
    }
    let (exit, drawn) = session.finish();
    screen.feed(&drawn[fed..]);
    assert_eq!(
        exit.code,
        Some(expected.status),
        "{what}: the program ended showing\n{}",
        screen.main_text()
    );
    assert!(
        !screen.on_alternate_screen(),
        "{what}: the first screen is back"
    );
    assert!(screen.cursor_is_visible(), "{what}: the cursor is back");
    assert!(screen.line_wrap_is_on(), "{what}: lines wrap again");
    assert_eq!(screen.text(), expected.ended, "{what}");
}

/// Runs a compiled program in a pseudo-terminal, takes `steps` one frame at
/// a time, and holds every screen to the interpreter's.
fn matches_the_interpreter(
    path: &str,
    source: &str,
    launch: impl FnOnce(&doriac::mir::Program) -> Launch,
    steps: &[Step],
) {
    let program = lower(path, source);
    let expected = interpreted(
        &program,
        steps.iter().copied().map(observed).collect(),
        steps,
    );
    shows_the_expected_screens(launch(&program), &expected, steps);
}

fn menu_matches_the_interpreter(
    launch: impl FnOnce(&doriac::mir::Program) -> Launch,
    steps: &[Step],
) {
    matches_the_interpreter(MENU_PATH, MENU, launch, steps);
}

#[test]
fn arrow_keys_move_the_menu_and_enter_chooses_on_every_platform() {
    menu_matches_the_interpreter(
        |program| native(program, NativeProfile::Fast),
        &presses(&[Key::Down, Key::Down, Key::Enter]),
    );
}

#[test]
fn the_menu_wraps_at_its_ends_and_escape_cancels_on_every_platform() {
    menu_matches_the_interpreter(
        |program| native(program, NativeProfile::Fast),
        &presses(&[Key::Up, Key::Escape]),
    );
}

#[cfg(feature = "llvm-backend")]
#[test]
fn the_release_profile_shows_the_same_menu() {
    menu_matches_the_interpreter(
        |program| native(program, NativeProfile::Release),
        &presses(&[Key::Down, Key::Down, Key::Enter]),
    );
}

/// PHP output drives the terminal through the same native runtime, so the
/// same program shows the same screens when it is run by PHP.
#[test]
fn php_output_shows_the_same_menu() {
    menu_matches_the_interpreter(
        |_| php(MENU_PATH, MENU),
        &presses(&[Key::Down, Key::Down, Key::Enter]),
    );
}

#[test]
fn php_output_restores_the_terminal_when_the_menu_is_cancelled() {
    menu_matches_the_interpreter(|_| php(MENU_PATH, MENU), &presses(&[Key::Up, Key::Escape]));
}

/// A window too short for the whole menu: the menu is told about the new
/// size, draws again inside it, and scrolls to keep the focused item in view.
#[test]
fn the_menu_redraws_and_scrolls_when_the_window_is_resized_on_every_platform() {
    menu_matches_the_interpreter(
        |program| native(program, NativeProfile::Fast),
        &[
            Step::Press(Key::Down),
            Step::Resize(40, 8),
            Step::Press(Key::Down),
            Step::Press(Key::Down),
            Step::Press(Key::Down),
            Step::Press(Key::Down),
            Step::Press(Key::Enter),
        ],
    );
}

/// How this host spells a command that copies its input to its output until
/// its input ends. `child_process_tests.rs` runs the same two commands.
#[cfg(unix)]
const COPY_COMMAND: &[&str] = &["cat"];

// `-InputFormat None` keeps PowerShell from reading its input as commands,
// so the script is the only reader of the pipe.
#[cfg(windows)]
const COPY_COMMAND: &[&str] = &[
    "powershell",
    "-NoProfile",
    "-NonInteractive",
    "-InputFormat",
    "None",
    "-Command",
    "$i = [Console]::OpenStandardInput(); $o = [Console]::OpenStandardOutput(); $i.CopyTo($o); $o.Flush()",
];

const RELAY_TITLE: &str = "Each character typed sends a mark. q or Escape ends.";

/// The relay's screen: its title, then the lines of its journal.
fn journal(lines: &[&str]) -> String {
    let mut screen = String::from(RELAY_TITLE);
    for (number, line) in lines.iter().enumerate() {
        screen.push_str(if number == 0 { "\n\n" } else { "\n" });
        screen.push_str(line);
    }
    screen
}

/// One step of a run of the relay, and the lines its journal gains before
/// the step is taken. The relay appends a line for each thing its one wait
/// reports, in the order the wait reports them, and draws a frame after each,
/// so the journal is that order.
type RelayStep = (&'static [&'static str], Step);

/// A run of the relay: its steps, the line the last step adds as the relay
/// ends, and what the interpreter's scripted terminal observes meanwhile.
struct RelayRun {
    steps: &'static [RelayStep],
    last: &'static str,
    events: &'static [Observation],
}

const fn typed(character: char) -> Observation {
    Observation::Key {
        key: Key::Character(character),
        modifiers: 0,
    }
}

/// The command copies what the relay sends it, so its output exists only
/// once the relay has written, and each step is taken only once the relay
/// shows what came before. Nothing here is a race.
///
/// The command's answer to the relay's greeting is reported before any key,
/// because nothing has been typed. The typed `a` is reported before the
/// mark the command copies back for it, while the read of the command's
/// output is still waiting. The resize is reported by the same watch. `q` ends the relay.
///
/// The scripted terminal is quiet where the real one is: when the first wait
/// asks it, so the greeting's answer comes first; at the end of each drain;
/// and when the wait after the `a` asks it, so the copied mark is reported.
const RELAY_RUN: RelayRun = RelayRun {
    steps: &[
        (&["command wrote: >"], Step::Press(Key::Character('a'))),
        (&["key a", "command wrote: ."], Step::Resize(60, ROWS)),
        (&["window is now 60x24"], Step::Press(Key::Character('q'))),
    ],
    last: "key q",
    events: &[
        Observation::TimedOut,
        typed('a'),
        Observation::TimedOut,
        Observation::TimedOut,
        Observation::Resized {
            columns: 60,
            rows: ROWS as u32,
        },
        Observation::TimedOut,
        typed('q'),
    ],
};

/// A lone Escape is a key only once its delay has run out with nothing
/// following it, so the group's wait has to wake for that moment by itself.
const ESCAPE_RUN: RelayRun = RelayRun {
    steps: &[(&["command wrote: >"], Step::Press(Key::Escape))],
    last: "key Escape",
    events: &[
        Observation::TimedOut,
        Observation::Key {
            key: Key::Escape,
            modifiers: 0,
        },
    ],
};

/// Runs the relay with a command that copies its input: in the interpreter,
/// where the command is scripted, and compiled in a pseudo-terminal with the
/// host's real one. The interpreter must draw the journal one line at a time
/// in the order stated, and the compiled relay must show the interpreter's
/// screen before each step. The relay ends with status 0 and one line on the
/// first screen; a wait that reported the terminal ready with nothing waiting
/// would add a sentence to that line.
fn relay_shows_its_journal(launch: impl FnOnce(&doriac::mir::Program) -> Launch, run: &RelayRun) {
    let program = lower(RELAY_PATH, RELAY);
    let steps = run.steps.iter().map(|(_, step)| *step).collect::<Vec<_>>();
    // Every frame the relay draws: its empty journal, then one more line each.
    let mut lines: Vec<&str> = Vec::new();
    let mut journals = vec![journal(&lines)];
    let mut shown = Vec::new();
    for (gained, _) in run.steps {
        for line in *gained {
            lines.push(*line);
            journals.push(journal(&lines));
        }
        shown.push(journals.len());
    }
    lines.push(run.last);
    journals.push(journal(&lines));

    let output = interpret_with_io(
        &program,
        MirIo {
            args: COPY_COMMAND.iter().map(|part| part.to_string()).collect(),
            programs: [(
                COPY_COMMAND[0].to_string(),
                MirProgram {
                    echo_input: true,
                    exit: Some(MirExitStatus::Exited(0)),
                    ..MirProgram::default()
                },
            )]
            .into(),
            terminal: Some(scripted(run.events.to_vec())),
            ..MirIo::default()
        },
    )
    .unwrap()
    .output;
    let interpreter = replayed(&output, &steps, &shown);
    assert_eq!(interpreter.frames, journals);
    assert_eq!(interpreter.ended, "The command exited with status 0.");
    assert_eq!(interpreter.status, 0);

    let expected = Expected {
        frames: shown
            .iter()
            .map(|drawn| interpreter.frames[drawn - 1].clone())
            .collect(),
        ended: interpreter.ended,
        status: interpreter.status,
    };
    shows_the_expected_screens(
        launch(&program).with_arguments(COPY_COMMAND),
        &expected,
        &steps,
    );
}

/// One transfer group waits on a child's output and the terminal: whichever
/// has something first is reported first, a resize arrives through the
/// watch, and the terminal is put back when the program ends.
#[test]
fn one_wait_reports_command_output_keys_and_a_resize_as_they_arrive_on_every_platform() {
    relay_shows_its_journal(|program| native(program, NativeProfile::Fast), &RELAY_RUN);
}

#[test]
fn the_wait_on_a_command_and_the_terminal_wakes_for_a_lone_escape_on_every_platform() {
    relay_shows_its_journal(|program| native(program, NativeProfile::Fast), &ESCAPE_RUN);
}

#[cfg(feature = "llvm-backend")]
#[test]
fn the_release_profile_waits_on_a_command_and_the_terminal_together() {
    relay_shows_its_journal(
        |program| native(program, NativeProfile::Release),
        &RELAY_RUN,
    );
}

/// PHP output reaches the same transfer group and the same terminal watch
/// through the native runtime.
#[test]
fn php_output_waits_on_a_command_and_the_terminal_together() {
    relay_shows_its_journal(|_| php(RELAY_PATH, RELAY), &RELAY_RUN);
}

/// A group whose only entry watches the terminal. It checks what a watch
/// refuses, shows that on one frame, and waits for a key; it then checks what
/// the group reports around that key, shows it on a second frame, and waits
/// for a last key through the raw mode itself while the group still watches.
const LONE_WATCH_PATH: &str = "lone_watch.doria";
const LONE_WATCH: &str = r#"
use Doria\Std\Io\{IoError, IoErrorReason, IoOperation, IoTarget, ReadinessSet, TransferCompletion,
    TransferPlan, TransferWaitResult, Wait};
use Doria\Std\Term\{Console, RawMode, TerminalError, TerminalEvent, TerminalWaitResult};

function describe(TransferWaitResult $result): string {
    return match ($result) {
        TransferWaitResult::Completed($completion) => match ($completion) {
            TransferCompletion::Ready($key) => "ready",
            default => "a transfer",
        },
        TransferWaitResult::TimedOut => "timed out",
        TransferWaitResult::Exhausted => "exhausted",
    };
}

function refusal(IoError $error, IoOperation $operation, IoTarget $target): string {
    if ($error->operation == $operation && $error->target == $target
        && $error->reason == IoErrorReason::InvalidInput
        && $error->systemCode == null && $error->transferredByteCount == null) {
        return $error->message;
    }
    return "another failure: {$error->message}";
}

function isKey(TerminalWaitResult $result): bool {
    return match ($result) {
        TerminalWaitResult::Event($event) => match ($event) {
            TerminalEvent::Key($key, $modifiers) => true,
            TerminalEvent::Resized($size) => false,
        },
        default => false,
    };
}

// Takes every waiting event and counts the keys among them; -1 says the
// terminal has gone away.
function takeKeys(RawMode $raw): int throws TerminalError {
    let writable $keys = 0;
    while (true) {
        let $result = $raw->getNextEvent(Wait::Immediate);
        if ($result == TerminalWaitResult::TimedOut) {
            return $keys;
        }
        if ($result == TerminalWaitResult::Closed) {
            return -1;
        }
        if (isKey($result)) {
            $keys += 1;
        }
    }
}

function show(List<string> $lines): void throws TerminalError {
    Console::clear();
    let writable $row = 0;
    while ($row < $lines->count) {
        Console::writeAt(0, $row, $lines[$row]);
        $row++;
    }
    Console::flush();
}

function main(): void throws IoError, TerminalError {
    {
        let $raw = Console::enterRawMode();
        let $screen = Console::enterAlternateScreen();
        Console::hideCursor();
        writable List<string> $lines = [];
        let writable $set = new ReadinessSet();
        {
            let $plan = TransferPlan::watch($raw);
            let $key = $plan->getWatchKey(0);
            try {
                $plan->getWatchKey(1);
                $lines->add("position 1 gave a watch key");
            } catch (IoError $error) {
                $lines->add("no watch at 1: " . refusal($error, IoOperation::Access, IoTarget::TransferPlan));
            }
            try {
                $plan->getReadKey(0);
                $lines->add("the watch gave a read key");
            } catch (IoError $error) {
                $lines->add("no read at 0: " . refusal($error, IoOperation::Access, IoTarget::TransferPlan));
            }
            let writable $group = $set->begin($plan);
            if ($group->observedWatchKey == null) {
                $lines->add("nothing observed yet");
            } else {
                $lines->add("a watch was observed before any fetch");
            }
            try {
                $group->resubmitWatch($key);
                $lines->add("an armed watch was armed again");
            } catch (IoError $error) {
                $lines->add("still armed: " . refusal($error, IoOperation::Configure, IoTarget::TransferGroup));
            }
            show($lines);

            // A window may announce its size before any key is typed, so
            // the wait is repeated until a key is among the events.
            let writable $keys = 0;
            while ($keys == 0) {
                let $result = $group->getNextCompletion(Wait::Indefinite);
                if ($result != TransferWaitResult::Completed(TransferCompletion::Ready($key))
                    || $group->observedWatchKey != $key || $group->observedReadKey != null
                    || $group->observedWriteKey != null) {
                    $lines->add("the wait reported " . describe($result));
                }
                $keys = takeKeys($raw);
                if ($keys == 0) {
                    $group->resubmitWatch($key);
                }
            }
            $lines->add("ready with {$keys} key");
            let $retired = $group->getNextCompletion(Wait::Immediate);
            $lines->add("left alone: " . describe($retired));
            if ($group->observedWatchKey == null) {
                $lines->add("nothing observed");
            } else {
                $lines->add("the watch key outlived its fetch");
            }
            $group->resubmitWatch($key);
            let $quiet = $group->getNextCompletion(Wait::Immediate);
            $lines->add("armed again: " . describe($quiet));
            show($lines);
            let $last = $raw->getNextEvent(Wait::Indefinite);
        }
        Console::showCursor();
    }
    echo "watched\n";
}
"#;

/// The lone watch's second frame, on every backend.
const LONE_WATCH_SCREEN: &str = "\
no watch at 1: failed to access transfer plan: invalid input\n\
no read at 0: failed to access transfer plan: invalid input\n\
nothing observed yet\n\
still armed: failed to configure transfer group: invalid input\n\
ready with 1 key\n\
left alone: exhausted\n\
nothing observed\n\
armed again: timed out";

/// Runs the compiled lone watch in a pseudo-terminal and holds every screen
/// to the interpreter's. The scripted terminal has a quiet moment where the
/// native one does: at the end of the drain, and when the armed watch is
/// asked at once.
fn the_lone_watch_matches_the_interpreter(launch: impl FnOnce(&doriac::mir::Program) -> Launch) {
    let program = lower(LONE_WATCH_PATH, LONE_WATCH);
    let steps = presses(&[Key::Character('x'), Key::Character('q')]);
    let expected = interpreted(
        &program,
        vec![
            observed(steps[0]),
            Observation::TimedOut,
            Observation::TimedOut,
            observed(steps[1]),
        ],
        &steps,
    );
    assert_eq!(expected.frames[1], LONE_WATCH_SCREEN);
    assert_eq!(expected.ended, "watched");
    assert_eq!(expected.status, 0);
    shows_the_expected_screens(launch(&program), &expected, &steps);
}

/// A plan of one watch, on its own readiness set: the keys a plan gives,
/// what the group refuses, what it reports before and after `Ready`, and the
/// raw mode's own wait while the group watches it.
#[test]
fn a_group_that_only_watches_the_terminal_matches_the_interpreter_on_every_platform() {
    the_lone_watch_matches_the_interpreter(|program| native(program, NativeProfile::Fast));
}

#[cfg(feature = "llvm-backend")]
#[test]
fn the_release_profile_watches_the_terminal_the_same_way() {
    the_lone_watch_matches_the_interpreter(|program| native(program, NativeProfile::Release));
}

#[test]
fn php_output_watches_the_terminal_the_same_way() {
    the_lone_watch_matches_the_interpreter(|_| php(LONE_WATCH_PATH, LONE_WATCH));
}

const KEY_EVENTS_PATH: &str = "key_events.doria";
const KEY_EVENTS: &str = include_str!("fixtures/terminal/key_events.doria");

/// What a terminal sends for each key a program can be told about, with the
/// key and modifiers it must be reported as. The Unix backend decodes these
/// bytes itself; on Windows the console turns them into key records first.
const EVERY_KEY: &[Step] = &[
    Step::Typed(b"a", Key::Character('a'), 0),
    Step::Typed("é".as_bytes(), Key::Character('é'), 0),
    // Outside the Basic Multilingual Plane: two UTF-16 units on Windows.
    Step::Typed("🎮".as_bytes(), Key::Character('🎮'), 0),
    Step::Typed(b"\r", Key::Enter, 0),
    Step::Typed(b"\t", Key::Tab, 0),
    Step::Typed(b"\x7f", Key::Backspace, 0),
    Step::Typed(b"\x1b[A", Key::Up, 0),
    Step::Typed(b"\x1b[B", Key::Down, 0),
    Step::Typed(b"\x1b[C", Key::Right, 0),
    Step::Typed(b"\x1b[D", Key::Left, 0),
    Step::Typed(b"\x1b[H", Key::Home, 0),
    Step::Typed(b"\x1b[F", Key::End, 0),
    Step::Typed(b"\x1b[5~", Key::PageUp, 0),
    Step::Typed(b"\x1b[6~", Key::PageDown, 0),
    Step::Typed(b"\x1b[2~", Key::Insert, 0),
    Step::Typed(b"\x1b[3~", Key::Delete, 0),
    Step::Typed(b"\x1bOP", Key::Function(1), 0),
    Step::Typed(b"\x1b[15~", Key::Function(5), 0),
    Step::Typed(b"\x1b[24~", Key::Function(12), 0),
    // In raw mode Control+C is a key, not an interrupt.
    Step::Typed(b"\x03", Key::Character('c'), MODIFIER_CONTROL),
    Step::Typed(b"\x1b[1;5C", Key::Right, MODIFIER_CONTROL),
    Step::Typed(b"\x1b[1;2A", Key::Up, MODIFIER_SHIFT),
    Step::Typed(b"\x1b[1;3D", Key::Left, MODIFIER_ALT),
    Step::Resize(60, 30),
    Step::Typed(b"\x1b", Key::Escape, 0),
    Step::Typed(b"q", Key::Character('q'), 0),
];

/// Every key, the modifiers and a resize arrive as the same typed events on
/// each platform's own input path.
#[test]
fn every_key_and_modifier_is_reported_the_same_on_every_platform() {
    matches_the_interpreter(
        KEY_EVENTS_PATH,
        KEY_EVENTS,
        |program| native(program, NativeProfile::Fast),
        EVERY_KEY,
    );
}

const ENDINGS_PATH: &str = "endings.doria";
const ENDINGS: &str = include_str!("fixtures/terminal/endings.doria");

/// A program that changed the terminal and returns without putting anything
/// back leaves it as it found it: the guards and the runtime do the rest.
#[test]
fn returning_from_main_puts_the_terminal_back_on_every_platform() {
    matches_the_interpreter(
        ENDINGS_PATH,
        ENDINGS,
        |program| native(program, NativeProfile::Fast),
        &[Step::Press(Key::Character('r'))],
    );
}

/// A panic runs no guard. The runtime puts the terminal back first, so the
/// report is on the first screen, where the user can read it.
#[test]
fn a_panic_puts_the_terminal_back_before_its_report_on_every_platform() {
    matches_the_interpreter(
        ENDINGS_PATH,
        ENDINGS,
        |program| native(program, NativeProfile::Fast),
        &[Step::Press(Key::Character('p'))],
    );
}

#[test]
fn php_output_puts_the_terminal_back_before_a_panic_report() {
    matches_the_interpreter(
        ENDINGS_PATH,
        ENDINGS,
        |_| php(ENDINGS_PATH, ENDINGS),
        &[Step::Press(Key::Character('p'))],
    );
}
