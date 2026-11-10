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

/// What the interpreter says the terminal shows: the alternate screen after
/// each frame the menu draws, the main screen once it has ended, and the
/// exit status.
struct Expected {
    frames: Vec<String>,
    ended: String,
    status: i32,
}

fn expected(program: &doriac::mir::Program, keys: &[Key]) -> Expected {
    let output = interpret_with_io(
        program,
        MirIo {
            terminal: Some(MirTerminal {
                columns: u32::from(COLUMNS),
                rows: u32::from(ROWS),
                color_support: ColorSupport::Indexed,
                events: keys
                    .iter()
                    .map(|key| Observation::Key {
                        key: *key,
                        modifiers: 0,
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
    assert_eq!(starts.len(), keys.len(), "one frame before each key");
    let leave = find(&drawn, LEAVE, *starts.last().unwrap()).expect("the menu leaves");
    let mut screen = Screen::new(COLUMNS, ROWS);
    let mut frames = Vec::new();
    let mut fed = 0;
    for end in starts[1..].iter().copied().chain([leave]) {
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

/// Runs the natively compiled menu in a pseudo-terminal, types `keys` one
/// frame at a time, and holds every screen to the interpreter's.
fn menu_matches_the_interpreter(profile: NativeProfile, keys: &[Key]) {
    let program = doriac::lower_source_to_mir("menu.doria", MENU)
        .unwrap_or_else(|diagnostics| panic!("the menu does not compile: {diagnostics:#?}"));
    let expected = expected(&program, keys);
    let directory = ExecutionDirectory::new();
    let executable = stage_executable(&program, profile, &directory);
    let mut session = Session::start(
        &executable,
        &[],
        &[("TERM", "xterm-256color")],
        COLUMNS,
        ROWS,
    );
    let mut screen = Screen::new(COLUMNS, ROWS);
    let mut fed = 0;
    for (number, (frame, key)) in expected.frames.iter().zip(keys).enumerate() {
        session.wait_until(
            &format!("frame {} ({profile:?}):\n{frame}", number + 1),
            |drawn| {
                screen.feed(&drawn[fed..]);
                fed = drawn.len();
                screen.on_alternate_screen() && screen.text() == *frame
            },
        );
        assert!(
            !screen.cursor_is_visible(),
            "{profile:?}: the menu hides the cursor"
        );
        session.type_bytes(key_bytes(*key));
    }
    let (exit, drawn) = session.finish();
    screen.feed(&drawn[fed..]);
    assert_eq!(exit.code, Some(expected.status), "{profile:?}");
    assert!(
        !screen.on_alternate_screen(),
        "{profile:?}: the first screen is back"
    );
    assert!(
        screen.cursor_is_visible(),
        "{profile:?}: the cursor is back"
    );
    assert_eq!(screen.text(), expected.ended, "{profile:?}");
}

#[test]
fn arrow_keys_move_the_menu_and_enter_chooses_on_every_platform() {
    menu_matches_the_interpreter(NativeProfile::Fast, &[Key::Down, Key::Down, Key::Enter]);
}

#[test]
fn the_menu_wraps_at_its_ends_and_escape_cancels_on_every_platform() {
    menu_matches_the_interpreter(NativeProfile::Fast, &[Key::Up, Key::Escape]);
}

#[cfg(feature = "llvm-backend")]
#[test]
fn the_release_profile_shows_the_same_menu() {
    menu_matches_the_interpreter(NativeProfile::Release, &[Key::Down, Key::Down, Key::Enter]);
}
