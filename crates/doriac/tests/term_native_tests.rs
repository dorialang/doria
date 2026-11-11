//! The Stage 46 acceptance test (Decision 0140): one program, compiled
//! natively and run inside a pseudo-terminal, shows the same screens on
//! every tier-1 platform. The expected screens come from the interpreter's
//! scripted terminal, so the native backends, the runtime's platform
//! backend and the interpreter are all held to one answer.

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[allow(dead_code)]
#[path = "common/native_execution.rs"]
mod native_execution;

use doria_pseudo_terminal::{Screen, Session};
use doria_terminal::wire::{ColorSupport, Key, Observation};
use doriac::backend::NativeProfile;
use doriac::mir_interpreter::{interpret_with_io, MirIo, MirTerminal};
use native_execution::{stage_executable, ExecutionDirectory};

const MENU: &str = include_str!("../../../examples/future/stage46/menu.doria");
const COLUMNS: u16 = 80;
const ROWS: u16 = 24;

const FRAME: &[u8] = b"\x1b[2J";
const LEAVE: &[u8] = b"\x1b[?25h";

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|found| found + from)
}

/// One thing that happens to the running menu.
#[derive(Clone, Copy)]
enum Step {
    Press(Key),
    /// The window changes to this many columns and rows.
    Resize(u16, u16),
}

fn presses(keys: &[Key]) -> Vec<Step> {
    keys.iter().copied().map(Step::Press).collect()
}

/// What the interpreter says the terminal shows: the alternate screen after
/// each frame the menu draws, the main screen once it has ended, and the
/// exit status.
struct Expected {
    frames: Vec<String>,
    ended: String,
    status: i32,
}

fn expected(program: &doriac::mir::Program, steps: &[Step]) -> Expected {
    let output = interpret_with_io(
        program,
        MirIo {
            terminal: Some(MirTerminal {
                columns: u32::from(COLUMNS),
                rows: u32::from(ROWS),
                color_support: ColorSupport::Indexed,
                events: steps
                    .iter()
                    .map(|step| match *step {
                        Step::Press(key) => Observation::Key { key, modifiers: 0 },
                        Step::Resize(columns, rows) => Observation::Resized {
                            columns: u32::from(columns),
                            rows: u32::from(rows),
                        },
                    })
                    .collect(),
                cursor: None,
            }),
            ..MirIo::default()
        },
    )
    .unwrap()
    .output;
    let drawn = output.stdout;
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(start) = find(&drawn, FRAME, from) {
        starts.push(start);
        from = start + FRAME.len();
    }
    assert_eq!(starts.len(), steps.len(), "one frame before each step");
    let leave = find(&drawn, LEAVE, *starts.last().unwrap()).expect("the menu leaves");
    let mut screen = Screen::new(COLUMNS, ROWS);
    let mut frames = Vec::new();
    let mut fed = 0;
    for (frame, end) in starts[1..].iter().copied().chain([leave]).enumerate() {
        // The frame after a resize is drawn on the resized window.
        if let Some(Step::Resize(columns, rows)) = frame.checked_sub(1).map(|step| steps[step]) {
            screen.resize(columns, rows);
        }
        screen.feed(&drawn[fed..end]);
        fed = end;
        assert!(screen.on_alternate_screen());
        frames.push(screen.text());
    }
    screen.feed(&drawn[fed..]);
    assert!(!screen.on_alternate_screen() && screen.cursor_is_visible());
    Expected {
        frames,
        ended: screen.text(),
        status: output.exit_status,
    }
}

fn key_bytes(key: Key) -> &'static [u8] {
    match key {
        Key::Up => b"\x1b[A",
        Key::Down => b"\x1b[B",
        Key::Enter => b"\r",
        Key::Escape => b"\x1b",
        other => panic!("no bytes for {other:?}"),
    }
}

/// One way of running the compiled menu.
struct Launch {
    what: String,
    program: std::path::PathBuf,
    arguments: Vec<String>,
    /// Keeps the built files in place until the run is over.
    _directory: ExecutionDirectory,
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
fn php() -> Launch {
    let artifact = doriac::compile_source("menu.doria", MENU, doriac::backend::BackendTarget::Php)
        .unwrap_or_else(|diagnostics| panic!("the menu does not compile to PHP: {diagnostics:#?}"));
    let directory = ExecutionDirectory::new();
    let script = directory.path().join("menu.php");
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

/// Runs the compiled menu in a pseudo-terminal, takes `steps` one frame at a
/// time, and holds every screen to the interpreter's.
fn menu_matches_the_interpreter(
    launch: impl FnOnce(&doriac::mir::Program) -> Launch,
    steps: &[Step],
) {
    let program = doriac::lower_source_to_mir("menu.doria", MENU)
        .unwrap_or_else(|diagnostics| panic!("the menu does not compile: {diagnostics:#?}"));
    let expected = expected(&program, steps);
    let launch = launch(&program);
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
        assert!(
            !screen.cursor_is_visible(),
            "{what}: the menu hides the cursor"
        );
        match *step {
            Step::Press(key) => session.type_bytes(key_bytes(key)),
            Step::Resize(columns, rows) => {
                session.resize(columns, rows);
                screen.resize(columns, rows);
            }
        }
    }
    let (exit, drawn) = session.finish();
    screen.feed(&drawn[fed..]);
    assert_eq!(exit.code, Some(expected.status), "{what}");
    assert!(
        !screen.on_alternate_screen(),
        "{what}: the first screen is back"
    );
    assert!(screen.cursor_is_visible(), "{what}: the cursor is back");
    assert_eq!(screen.text(), expected.ended, "{what}");
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
    menu_matches_the_interpreter(|_| php(), &presses(&[Key::Down, Key::Down, Key::Enter]));
}

#[test]
fn php_output_restores_the_terminal_when_the_menu_is_cancelled() {
    menu_matches_the_interpreter(|_| php(), &presses(&[Key::Up, Key::Escape]));
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
