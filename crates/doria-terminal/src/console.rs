//! What each `Console` operation means: which sequences it renders and what
//! it records in the ledger. It is pure and written against a [`Host`], so
//! the runtime's device-backed terminal and the debug interpreter's scripted
//! terminal perform the same operations and cannot drift.
use super::render;
use super::wire::{self, operation, ColorSupport, Style};

/// Why an operation did not happen: a `TerminalErrorReason` and the native
/// code behind it, when there is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub status: u8,
    pub system_code: Option<i64>,
}

impl Refusal {
    pub const fn not_a_terminal() -> Self {
        Self {
            status: wire::STATUS_NOT_A_TERMINAL,
            system_code: None,
        }
    }

    pub const fn in_use() -> Self {
        Self {
            status: wire::STATUS_IN_USE,
            system_code: None,
        }
    }

    pub const fn unsupported() -> Self {
        Self {
            status: wire::STATUS_UNSUPPORTED,
            system_code: None,
        }
    }

    /// The terminal went away or failed while it was being used.
    pub const fn device_failure() -> Self {
        Self {
            status: wire::STATUS_DEVICE_FAILURE,
            system_code: None,
        }
    }

    pub const fn to_result(self) -> i64 {
        wire::failure(self.status, self.system_code)
    }
}

/// Packs an operation's outcome as `wire` defines.
pub const fn result(outcome: Result<u64, Refusal>) -> i64 {
    match outcome {
        Ok(payload) => wire::success(payload),
        Err(refusal) => refusal.to_result(),
    }
}

/// What standard output is, decided once: a redirected stream stays
/// redirected for the life of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputClass {
    pub terminal: bool,
    /// The terminal accepts the renderer's sequences. Only an old Windows
    /// console refuses them.
    pub sequences: bool,
    pub color: ColorSupport,
}

impl OutputClass {
    pub const NOT_A_TERMINAL: Self = Self {
        terminal: false,
        sequences: false,
        color: ColorSupport::None,
    };

    /// A terminal that takes the renderer's sequences.
    pub const fn terminal(color: ColorSupport) -> Self {
        Self {
            terminal: true,
            sequences: true,
            color,
        }
    }

    const KNOWN: u8 = 0x80;

    /// Never zero, so zero can mean "not yet classified".
    pub const fn to_bits(self) -> u8 {
        Self::KNOWN
            | ((self.terminal as u8) << 3)
            | ((self.sequences as u8) << 2)
            | self.color as u8
    }

    pub const fn from_bits(bits: u8) -> Self {
        Self {
            terminal: bits & 8 != 0,
            sequences: bits & 4 != 0,
            color: ColorSupport::from_code(bits & 3),
        }
    }
}

/// The ledger: one bit for each terminal setting the program has moved away
/// from the value it had at start. Cursor visibility, line wrap and the screen
/// in use cannot be read back from a terminal, so their starting values are
/// the ones every terminal starts a program with: visible, wrapping, main
/// screen. The input mode and the Windows output mode are read before they
/// are changed and kept by the platform.
pub mod ledger {
    pub const CURSOR_HIDDEN: u32 = 1;
    pub const LINE_WRAP_DISABLED: u32 = 1 << 1;
    pub const ALTERNATE_SCREEN: u32 = 1 << 2;
    pub const RAW_MODE: u32 = 1 << 3;
    pub const OUTPUT_MODE: u32 = 1 << 4;
    /// The entries undone by writing a sequence to the terminal.
    pub const RENDERED: u32 = CURSOR_HIDDEN | LINE_WRAP_DISABLED | ALTERNATE_SCREEN;
}

/// The sequences that undo ledger entries, or make them again. Fixed
/// storage: a signal handler builds this too.
pub struct Undo {
    bytes: [u8; 32],
    length: usize,
}

impl Undo {
    /// `interrupted` says a console write was cut short, so the terminal may
    /// be inside an escape sequence or a styled run that must end first.
    pub fn new(entries: u32, interrupted: bool) -> Self {
        let mut undo = Self {
            bytes: [0; 32],
            length: 0,
        };
        if entries & ledger::RENDERED == 0 {
            return undo;
        }
        if interrupted {
            undo.push(render::ABORT_SEQUENCE);
        }
        if entries & ledger::ALTERNATE_SCREEN != 0 {
            undo.push(render::LEAVE_ALTERNATE_SCREEN);
        }
        if entries & ledger::LINE_WRAP_DISABLED != 0 {
            undo.push(render::ENABLE_LINE_WRAP);
        }
        if entries & ledger::CURSOR_HIDDEN != 0 {
            undo.push(render::SHOW_CURSOR);
        }
        undo
    }

    /// The sequences that make `entries` again, for a program continued
    /// after a stop that undid them.
    pub fn again(entries: u32) -> Self {
        let mut redo = Self {
            bytes: [0; 32],
            length: 0,
        };
        if entries & ledger::ALTERNATE_SCREEN != 0 {
            redo.push(render::ENTER_ALTERNATE_SCREEN);
        }
        if entries & ledger::LINE_WRAP_DISABLED != 0 {
            redo.push(render::DISABLE_LINE_WRAP);
        }
        if entries & ledger::CURSOR_HIDDEN != 0 {
            redo.push(render::HIDE_CURSOR);
        }
        redo
    }

    fn push(&mut self, sequence: &[u8]) {
        self.bytes[self.length..self.length + sequence.len()].copy_from_slice(sequence);
        self.length += sequence.len();
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

/// The terminal an operation acts on.
pub trait Host {
    /// Takes rendered output, in order.
    fn append(&mut self, bytes: &[u8]) -> Result<(), Refusal>;
    /// Records a change before it is made.
    fn mark(&mut self, entry: u32);
    /// Removes an entry whose change was never made.
    fn unmark(&mut self, entry: u32);
    /// The entry's undoing sequence has been appended. The entry leaves the
    /// ledger when that output reaches the terminal, not before.
    fn unmark_when_written(&mut self, entry: u32);
    /// The entry is in effect and the program has not already undone it.
    fn active(&self, entry: u32) -> bool;
    /// Columns and rows.
    fn size(&mut self) -> Result<(u32, u32), Refusal>;
    /// Starts a new alternate-screen generation and returns it.
    fn enter_screen(&mut self) -> u32;
    /// The generation of the screen entered last.
    fn screen(&self) -> u32;
}

fn distance(argument: i64) -> u32 {
    argument.clamp(0, wire::COORDINATE_LIMIT as i64) as u32
}

fn write<H: Host>(
    host: &mut H,
    class: OutputClass,
    style: Style,
    text: &[u8],
) -> Result<(), Refusal> {
    let prefix = render::style_prefix(style, class.color);
    host.append(prefix.as_bytes())?;
    host.append(text)?;
    if !prefix.is_empty() {
        host.append(render::STYLE_SUFFIX)?;
    }
    Ok(())
}

/// Performs one output, screen or cursor operation. Color support, flushing
/// and the raw-mode operations belong to the host and never arrive here.
pub fn operate<H: Host>(
    host: &mut H,
    class: OutputClass,
    operation: u32,
    first: i64,
    second: i64,
    text: &[u8],
) -> Result<u64, Refusal> {
    if operation == operation::WRITE {
        write(host, class, Style::from_bits(first), text)?;
        return Ok(0);
    }
    if !class.terminal {
        // Nothing to put back on a stream that was never a terminal.
        return if operation == operation::RESET {
            Ok(0)
        } else {
            Err(Refusal::not_a_terminal())
        };
    }
    if !class.sequences {
        return Err(Refusal::unsupported());
    }
    match operation {
        operation::SIZE => {
            let (columns, rows) = host.size()?;
            return Ok(wire::pair(columns, rows));
        }
        operation::WRITE_AT => {
            let cell = render::move_to(wire::cell_column(first), wire::cell_row(first));
            host.append(cell.as_bytes())?;
            write(host, class, Style::from_bits(second), text)?;
        }
        operation::CLEAR => host.append(render::CLEAR)?,
        operation::CLEAR_LINE => host.append(render::CLEAR_LINE)?,
        operation::CLEAR_LINE_FROM_CURSOR => host.append(render::CLEAR_LINE_FROM_CURSOR)?,
        operation::ERASE_AT => {
            let cell = render::move_to(wire::cell_column(first), wire::cell_row(first));
            host.append(cell.as_bytes())?;
            host.append(render::ERASE_CHARACTER)?;
        }
        operation::SET_TITLE => {
            host.append(render::TITLE_PREFIX)?;
            for run in render::title_runs(text) {
                host.append(run)?;
            }
            host.append(render::TITLE_SUFFIX)?;
        }
        operation::SET_LINE_WRAP => {
            if first == 0 {
                host.append(render::ENABLE_LINE_WRAP)?;
                host.unmark_when_written(ledger::LINE_WRAP_DISABLED);
            } else {
                host.mark(ledger::LINE_WRAP_DISABLED);
                host.append(render::DISABLE_LINE_WRAP)?;
            }
        }
        operation::MOVE_TO => {
            let cell = render::move_to(wire::cell_column(first), wire::cell_row(first));
            host.append(cell.as_bytes())?;
        }
        operation::MOVE_UP
        | operation::MOVE_DOWN
        | operation::MOVE_LEFT
        | operation::MOVE_RIGHT => {
            let direction = match operation {
                operation::MOVE_UP => render::Direction::Up,
                operation::MOVE_DOWN => render::Direction::Down,
                operation::MOVE_LEFT => render::Direction::Left,
                _ => render::Direction::Right,
            };
            host.append(render::move_by(direction, distance(first)).as_bytes())?;
        }
        operation::SHOW_CURSOR => {
            host.append(render::SHOW_CURSOR)?;
            host.unmark_when_written(ledger::CURSOR_HIDDEN);
        }
        operation::HIDE_CURSOR => {
            host.mark(ledger::CURSOR_HIDDEN);
            host.append(render::HIDE_CURSOR)?;
        }
        operation::ENTER_ALTERNATE_SCREEN => {
            if host.active(ledger::ALTERNATE_SCREEN) {
                return Err(Refusal::in_use());
            }
            host.mark(ledger::ALTERNATE_SCREEN);
            if let Err(refusal) = host.append(render::ENTER_ALTERNATE_SCREEN) {
                host.unmark(ledger::ALTERNATE_SCREEN);
                return Err(refusal);
            }
            return Ok(u64::from(host.enter_screen()));
        }
        operation::LEAVE_ALTERNATE_SCREEN => {
            if host.active(ledger::ALTERNATE_SCREEN) && host.screen() == first as u32 {
                host.append(render::LEAVE_ALTERNATE_SCREEN)?;
                host.unmark_when_written(ledger::ALTERNATE_SCREEN);
            }
        }
        operation::RESET => {
            host.append(render::SHOW_CURSOR)?;
            host.append(render::ENABLE_LINE_WRAP)?;
            host.append(render::STYLE_SUFFIX)?;
            host.unmark_when_written(ledger::CURSOR_HIDDEN | ledger::LINE_WRAP_DISABLED);
        }
        _ => return Err(Refusal::unsupported()),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal in memory: what a scripted terminal is.
    #[derive(Default)]
    struct Recorded {
        drawn: Vec<u8>,
        entries: u32,
        undone: u32,
        screen: u32,
    }

    impl Host for Recorded {
        fn append(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
            self.drawn.extend_from_slice(bytes);
            Ok(())
        }
        fn mark(&mut self, entry: u32) {
            self.undone &= !entry;
            self.entries |= entry;
        }
        fn unmark(&mut self, entry: u32) {
            self.entries &= !entry;
        }
        fn unmark_when_written(&mut self, entry: u32) {
            self.undone |= entry;
        }
        fn active(&self, entry: u32) -> bool {
            self.entries & !self.undone & entry != 0
        }
        fn size(&mut self) -> Result<(u32, u32), Refusal> {
            Ok((80, 24))
        }
        fn enter_screen(&mut self) -> u32 {
            self.screen += 1;
            self.screen
        }
        fn screen(&self) -> u32 {
            self.screen
        }
    }

    const TERMINAL: OutputClass = OutputClass::terminal(ColorSupport::Indexed);

    #[test]
    fn a_refusal_keeps_its_reason_and_native_code_through_the_wire() {
        let refused = result(Err(Refusal {
            status: wire::STATUS_DEVICE_FAILURE,
            system_code: Some(5),
        }));
        assert_eq!(wire::status(refused), wire::STATUS_DEVICE_FAILURE);
        assert_eq!(wire::system_code(refused), Some(5));
        for (refusal, status) in [
            (Refusal::not_a_terminal(), wire::STATUS_NOT_A_TERMINAL),
            (Refusal::in_use(), wire::STATUS_IN_USE),
            (Refusal::unsupported(), wire::STATUS_UNSUPPORTED),
            (Refusal::device_failure(), wire::STATUS_DEVICE_FAILURE),
        ] {
            assert_eq!(wire::status(refusal.to_result()), status);
            assert_eq!(wire::system_code(refusal.to_result()), None);
        }
        assert_eq!(wire::payload(result(Ok(7))), 7);
    }

    #[test]
    fn an_output_class_survives_its_cached_form() {
        for class in [
            OutputClass::NOT_A_TERMINAL,
            OutputClass {
                terminal: true,
                sequences: false,
                color: ColorSupport::None,
            },
            OutputClass::terminal(ColorSupport::TrueColor),
            OutputClass::terminal(ColorSupport::Basic),
        ] {
            assert_ne!(class.to_bits(), 0);
            assert_eq!(OutputClass::from_bits(class.to_bits()), class);
        }
    }

    #[test]
    fn undo_writes_only_what_the_program_changed() {
        assert!(Undo::new(0, true).as_bytes().is_empty());
        assert!(Undo::new(ledger::RAW_MODE | ledger::OUTPUT_MODE, true)
            .as_bytes()
            .is_empty());
        assert_eq!(
            Undo::new(ledger::CURSOR_HIDDEN, false).as_bytes(),
            b"\x1b[?25h"
        );
        assert_eq!(
            Undo::new(ledger::RENDERED, false).as_bytes(),
            b"\x1b[?1049l\x1b[?7h\x1b[?25h"
        );
        assert_eq!(
            Undo::new(ledger::RENDERED | ledger::RAW_MODE, true).as_bytes(),
            b"\x18\x1b[0m\x1b[?1049l\x1b[?7h\x1b[?25h"
        );
        assert!(Undo::again(ledger::RAW_MODE).as_bytes().is_empty());
        assert_eq!(
            Undo::again(ledger::RENDERED).as_bytes(),
            b"\x1b[?1049h\x1b[?7l\x1b[?25l"
        );
    }

    #[test]
    fn a_redirected_stream_takes_plain_text_and_refuses_the_rest() {
        let mut host = Recorded::default();
        let loud = Style {
            foreground: wire::Color::Rgb(1, 2, 3),
            background: wire::Color::Indexed(4),
            attributes: wire::ATTRIBUTE_BOLD,
        };
        let class = OutputClass::NOT_A_TERMINAL;
        assert_eq!(
            operate(
                &mut host,
                class,
                operation::WRITE,
                loud.to_bits(),
                0,
                b"plain"
            ),
            Ok(0)
        );
        assert_eq!(
            operate(&mut host, class, operation::RESET, 0, 0, b""),
            Ok(0)
        );
        for refused in [
            operation::SIZE,
            operation::WRITE_AT,
            operation::CLEAR,
            operation::MOVE_TO,
            operation::HIDE_CURSOR,
            operation::SET_TITLE,
            operation::ENTER_ALTERNATE_SCREEN,
        ] {
            assert_eq!(
                operate(&mut host, class, refused, 0, 0, b"x"),
                Err(Refusal::not_a_terminal())
            );
        }
        assert_eq!(host.drawn, b"plain");
        assert_eq!(host.entries, 0);
    }

    #[test]
    fn a_terminal_that_refuses_sequences_still_takes_plain_text() {
        let mut host = Recorded::default();
        let class = OutputClass {
            terminal: true,
            sequences: false,
            color: ColorSupport::None,
        };
        assert_eq!(
            operate(&mut host, class, operation::WRITE, 0, 0, b"text"),
            Ok(0)
        );
        assert_eq!(
            operate(&mut host, class, operation::CLEAR, 0, 0, b""),
            Err(Refusal::unsupported())
        );
        assert_eq!(host.drawn, b"text");
    }

    #[test]
    fn the_ledger_follows_what_the_program_changed_and_undid() {
        let mut host = Recorded::default();
        operate(&mut host, TERMINAL, operation::HIDE_CURSOR, 0, 0, b"").unwrap();
        operate(&mut host, TERMINAL, operation::SET_LINE_WRAP, 1, 0, b"").unwrap();
        assert!(host.active(ledger::CURSOR_HIDDEN | ledger::LINE_WRAP_DISABLED));
        operate(&mut host, TERMINAL, operation::SHOW_CURSOR, 0, 0, b"").unwrap();
        // Undone but not yet written: still an entry, no longer active.
        assert!(!host.active(ledger::CURSOR_HIDDEN));
        assert_ne!(host.entries & ledger::CURSOR_HIDDEN, 0);
        operate(&mut host, TERMINAL, operation::HIDE_CURSOR, 0, 0, b"").unwrap();
        assert!(host.active(ledger::CURSOR_HIDDEN));
        operate(&mut host, TERMINAL, operation::RESET, 0, 0, b"").unwrap();
        assert!(!host.active(ledger::CURSOR_HIDDEN | ledger::LINE_WRAP_DISABLED));
        assert_eq!(
            host.drawn,
            b"\x1b[?25l\x1b[?7l\x1b[?25h\x1b[?25l\x1b[?25h\x1b[?7h\x1b[0m"
        );
    }

    #[test]
    fn one_alternate_screen_at_a_time_and_a_stale_guard_ends_nothing() {
        let mut host = Recorded::default();
        let enter = operation::ENTER_ALTERNATE_SCREEN;
        let leave = operation::LEAVE_ALTERNATE_SCREEN;
        let first = operate(&mut host, TERMINAL, enter, 0, 0, b"").unwrap();
        assert_eq!(
            operate(&mut host, TERMINAL, enter, 0, 0, b""),
            Err(Refusal::in_use())
        );
        operate(&mut host, TERMINAL, leave, first as i64, 0, b"").unwrap();
        // Left but not yet written out: the screen is free to enter again.
        let second = operate(&mut host, TERMINAL, enter, 0, 0, b"").unwrap();
        assert_ne!(first, second);
        operate(&mut host, TERMINAL, leave, first as i64, 0, b"").unwrap();
        assert!(host.active(ledger::ALTERNATE_SCREEN));
        operate(&mut host, TERMINAL, leave, second as i64, 0, b"").unwrap();
        assert!(!host.active(ledger::ALTERNATE_SCREEN));
        assert_eq!(host.drawn, b"\x1b[?1049h\x1b[?1049l\x1b[?1049h\x1b[?1049l");
    }

    #[test]
    fn a_frame_renders_in_the_order_it_was_drawn() {
        let mut host = Recorded::default();
        let red_bold = Style {
            foreground: wire::Color::Named(1),
            background: wire::Color::Default,
            attributes: wire::ATTRIBUTE_BOLD,
        };
        let size = operate(&mut host, TERMINAL, operation::SIZE, 0, 0, b"").unwrap();
        assert_eq!((wire::pair_first(size), wire::pair_second(size)), (80, 24));
        operate(&mut host, TERMINAL, operation::CLEAR, 0, 0, b"").unwrap();
        operate(
            &mut host,
            TERMINAL,
            operation::WRITE_AT,
            wire::cell(2, 1),
            red_bold.to_bits(),
            b"hi",
        )
        .unwrap();
        operate(
            &mut host,
            TERMINAL,
            operation::MOVE_TO,
            wire::cell(0, 3),
            0,
            b"",
        )
        .unwrap();
        operate(&mut host, TERMINAL, operation::MOVE_RIGHT, 4, 0, b"").unwrap();
        operate(&mut host, TERMINAL, operation::MOVE_LEFT, 0, 0, b"").unwrap();
        operate(
            &mut host,
            TERMINAL,
            operation::ERASE_AT,
            wire::cell(7, 0),
            0,
            b"",
        )
        .unwrap();
        operate(
            &mut host,
            TERMINAL,
            operation::SET_TITLE,
            0,
            0,
            b"a\x1bb\x07c",
        )
        .unwrap();
        assert_eq!(
            operate(&mut host, TERMINAL, 9999, 0, 0, b""),
            Err(Refusal::unsupported())
        );
        assert_eq!(
            host.drawn,
            b"\x1b[2J\x1b[H\x1b[2;3H\x1b[1;31mhi\x1b[0m\x1b[4;1H\x1b[4C\x1b[1;8H\x1b[X\x1b]2;abc\x07"
        );
    }
}
