//! `Doria\Std\Term` (Decision 0140) against the interpreter's scripted
//! terminal: the authored module, its packed transport and the shared
//! operation core, checked on the exact bytes a terminal would receive.

use doria_terminal::wire::{ColorSupport, Key, Observation};
use doriac::mir_interpreter::{interpret_with_io, InterpreterOutput, MirIo, MirTerminal};

fn terminal(events: Vec<Observation>) -> MirTerminal {
    MirTerminal {
        columns: 80,
        rows: 24,
        color_support: ColorSupport::Indexed,
        events,
        cursor: Some((9, 4)),
    }
}

fn run(source: &str, terminal: Option<MirTerminal>) -> InterpreterOutput {
    let program = match doriac::lower_source_to_mir("term.doria", source) {
        Ok(program) => program,
        Err(diagnostics) => panic!("the program does not compile: {diagnostics:#?}"),
    };
    interpret_with_io(
        &program,
        MirIo {
            terminal,
            ..MirIo::default()
        },
    )
    .unwrap()
    .output
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
