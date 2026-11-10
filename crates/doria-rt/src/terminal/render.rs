//! The one mapping from terminal operations to terminal output. Every backend
//! (native, PHP through FFI, the debug interpreter's scripted terminal, and
//! Windows once VT processing is on) emits exactly these bytes, so no public
//! `Doria\Std\Term` type ever has to carry an escape sequence.
//!
//! Nothing here allocates or touches a device: callers append the returned
//! bytes to the standard output device's console buffer.

use super::wire::{
    Color, ColorSupport, Style, ATTRIBUTE_BOLD, ATTRIBUTE_DIM, ATTRIBUTE_ITALIC, ATTRIBUTE_REVERSE,
    ATTRIBUTE_STRIKETHROUGH, ATTRIBUTE_UNDERLINE, COORDINATE_LIMIT,
};

/// Ends a styled write: back to the default attributes.
pub const STYLE_SUFFIX: &[u8] = b"\x1b[0m";
/// Erases the screen and homes the cursor. Scrollback is left alone.
pub const CLEAR: &[u8] = b"\x1b[2J\x1b[H";
pub const CLEAR_LINE: &[u8] = b"\x1b[2K";
pub const CLEAR_LINE_FROM_CURSOR: &[u8] = b"\x1b[K";
/// Erases the cell under the cursor without moving it.
pub const ERASE_CHARACTER: &[u8] = b"\x1b[X";
pub const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
pub const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
pub const ENABLE_LINE_WRAP: &[u8] = b"\x1b[?7h";
pub const DISABLE_LINE_WRAP: &[u8] = b"\x1b[?7l";
pub const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
pub const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
/// Asks the terminal for the cursor position; the reply arrives as input.
pub const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
pub const TITLE_PREFIX: &[u8] = b"\x1b]2;";
pub const TITLE_SUFFIX: &[u8] = b"\x07";
/// Cancels a control sequence the terminal may be in the middle of, then
/// returns to the default attributes. Used when restoration interrupts a
/// device write at an unknown byte.
pub const ABORT_SEQUENCE: &[u8] = b"\x18\x1b[0m";

/// The longest sequence built here is a style prefix with all six attributes
/// and two RGB colors: 48 bytes.
const CAPACITY: usize = 64;

/// A short rendered control sequence, held inline.
#[derive(Clone, Copy)]
pub struct Sequence {
    bytes: [u8; CAPACITY],
    length: usize,
}

impl Sequence {
    const fn empty() -> Self {
        Self {
            bytes: [0; CAPACITY],
            length: 0,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    fn push(&mut self, byte: u8) {
        self.bytes[self.length] = byte;
        self.length += 1;
    }

    fn push_all(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.push(*byte);
        }
    }

    fn push_decimal(&mut self, mut value: u32) {
        let mut digits = [0u8; 10];
        let mut count = 0;
        loop {
            digits[count] = b'0' + (value % 10) as u8;
            count += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        while count > 0 {
            count -= 1;
            self.push(digits[count]);
        }
    }

    /// Appends one SGR parameter, separated from the previous one.
    fn parameter(&mut self, value: u32) {
        if self.length > 2 {
            self.push(b';');
        }
        self.push_decimal(value);
    }
}

const ATTRIBUTE_CODES: [(u8, u32); 6] = [
    (ATTRIBUTE_BOLD, 1),
    (ATTRIBUTE_DIM, 2),
    (ATTRIBUTE_ITALIC, 3),
    (ATTRIBUTE_UNDERLINE, 4),
    (ATTRIBUTE_REVERSE, 7),
    (ATTRIBUTE_STRIKETHROUGH, 9),
];

/// What a styled write emits before its text. Empty when the write is plain:
/// the style is the default one, or the terminal shows no color. A non-empty
/// prefix is always closed with [`STYLE_SUFFIX`] after the text.
pub fn style_prefix(style: Style, support: ColorSupport) -> Sequence {
    let mut sequence = Sequence::empty();
    if support == ColorSupport::None {
        return sequence;
    }
    sequence.push_all(b"\x1b[");
    for (attribute, code) in ATTRIBUTE_CODES {
        if style.attributes & attribute != 0 {
            sequence.parameter(code);
        }
    }
    push_color(&mut sequence, downgrade(style.foreground, support), 0);
    push_color(&mut sequence, downgrade(style.background, support), 10);
    if sequence.length == 2 {
        return Sequence::empty();
    }
    sequence.push(b'm');
    sequence
}

/// `layer` is 0 for the foreground and 10 for the background.
fn push_color(sequence: &mut Sequence, color: Color, layer: u32) {
    match color {
        Color::Default => {}
        Color::Named(index) => {
            let index = u32::from(index & 15);
            sequence.parameter(if index < 8 {
                30 + layer + index
            } else {
                90 + layer + index - 8
            });
        }
        Color::Indexed(index) => {
            sequence.parameter(38 + layer);
            sequence.parameter(5);
            sequence.parameter(u32::from(index));
        }
        Color::Rgb(red, green, blue) => {
            sequence.parameter(38 + layer);
            sequence.parameter(2);
            sequence.parameter(u32::from(red));
            sequence.parameter(u32::from(green));
            sequence.parameter(u32::from(blue));
        }
    }
}

/// The xterm default palette for the sixteen named colors.
const NAMED_PALETTE: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The channel levels of the 6x6x6 cube at indices 16..232.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Maps a color the terminal cannot show to the nearest one it can.
pub fn downgrade(color: Color, support: ColorSupport) -> Color {
    match (support, color) {
        (ColorSupport::None, _) => Color::Default,
        (_, Color::Default | Color::Named(_)) | (ColorSupport::TrueColor, _) => color,
        (ColorSupport::Indexed, Color::Indexed(_)) => color,
        (ColorSupport::Indexed, Color::Rgb(red, green, blue)) => {
            Color::Indexed(nearest_indexed(red, green, blue))
        }
        (ColorSupport::Basic, Color::Indexed(index)) => {
            if index < 16 {
                Color::Named(index)
            } else {
                let (red, green, blue) = indexed_channels(index);
                Color::Named(nearest_named(red, green, blue))
            }
        }
        (ColorSupport::Basic, Color::Rgb(red, green, blue)) => {
            Color::Named(nearest_named(red, green, blue))
        }
    }
}

fn distance(left: (u8, u8, u8), right: (u8, u8, u8)) -> u32 {
    let channel = |a: u8, b: u8| {
        let difference = u32::from(a.abs_diff(b));
        difference * difference
    };
    channel(left.0, right.0) + channel(left.1, right.1) + channel(left.2, right.2)
}

fn nearest_named(red: u8, green: u8, blue: u8) -> u8 {
    let mut best = 0;
    let mut best_distance = u32::MAX;
    for (index, entry) in NAMED_PALETTE.iter().enumerate() {
        let candidate = distance(*entry, (red, green, blue));
        if candidate < best_distance {
            best = index as u8;
            best_distance = candidate;
        }
    }
    best
}

fn nearest_cube_step(value: u8) -> u8 {
    let mut best = 0;
    let mut best_distance = u8::MAX;
    for (step, level) in CUBE_LEVELS.iter().enumerate() {
        let candidate = level.abs_diff(value);
        if candidate < best_distance {
            best = step as u8;
            best_distance = candidate;
        }
    }
    best
}

/// The nearest of the color cube and the grayscale ramp. The first sixteen
/// indices are never chosen: terminals let the user repaint them.
fn nearest_indexed(red: u8, green: u8, blue: u8) -> u8 {
    let steps = (
        nearest_cube_step(red),
        nearest_cube_step(green),
        nearest_cube_step(blue),
    );
    let cube = (
        CUBE_LEVELS[steps.0 as usize],
        CUBE_LEVELS[steps.1 as usize],
        CUBE_LEVELS[steps.2 as usize],
    );
    let average = (u32::from(red) + u32::from(green) + u32::from(blue)) / 3;
    // The ramp is 8, 18, ... 238; round the average to its nearest entry.
    let gray_step = (average.saturating_sub(3) / 10).min(23) as u8;
    let gray = 8 + 10 * gray_step;
    if distance((gray, gray, gray), (red, green, blue)) < distance(cube, (red, green, blue)) {
        232 + gray_step
    } else {
        16 + 36 * steps.0 + 6 * steps.1 + steps.2
    }
}

/// The channels of an indexed color at or above 16.
fn indexed_channels(index: u8) -> (u8, u8, u8) {
    if index >= 232 {
        let gray = 8 + 10 * (index - 232);
        (gray, gray, gray)
    } else {
        let cell = (index - 16) as usize;
        (
            CUBE_LEVELS[cell / 36],
            CUBE_LEVELS[(cell / 6) % 6],
            CUBE_LEVELS[cell % 6],
        )
    }
}

/// Moves the cursor to a zero-based cell.
pub fn move_to(column: u32, row: u32) -> Sequence {
    let mut sequence = Sequence::empty();
    sequence.push_all(b"\x1b[");
    sequence.push_decimal(row.min(COORDINATE_LIMIT) + 1);
    sequence.push(b';');
    sequence.push_decimal(column.min(COORDINATE_LIMIT) + 1);
    sequence.push(b'H');
    sequence
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Moves the cursor by `count` cells. A count of zero is no movement and
/// renders nothing; a terminal would read a zero parameter as one.
pub fn move_by(direction: Direction, count: u32) -> Sequence {
    let mut sequence = Sequence::empty();
    if count == 0 {
        return sequence;
    }
    sequence.push_all(b"\x1b[");
    sequence.push_decimal(count.min(COORDINATE_LIMIT));
    sequence.push(match direction {
        Direction::Up => b'A',
        Direction::Down => b'B',
        Direction::Right => b'C',
        Direction::Left => b'D',
    });
    sequence
}

/// The parts of a window title that may be sent to a terminal: everything
/// except C0 controls, DEL and C1 controls. A control inside a title would
/// end the title sequence and let the remaining text act as terminal input.
pub fn title_runs(title: &[u8]) -> TitleRuns<'_> {
    TitleRuns { rest: title }
}

pub struct TitleRuns<'a> {
    rest: &'a [u8],
}

fn control_length(bytes: &[u8]) -> usize {
    match bytes {
        [byte, ..] if *byte < 0x20 || *byte == 0x7f => 1,
        // U+0080..U+009F encode as C2 80..C2 9F.
        [0xc2, second, ..] if (0x80..=0x9f).contains(second) => 2,
        _ => 0,
    }
}

impl<'a> Iterator for TitleRuns<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        loop {
            let skipped = control_length(self.rest);
            if skipped == 0 {
                break;
            }
            self.rest = &self.rest[skipped..];
        }
        if self.rest.is_empty() {
            return None;
        }
        let mut end = 1;
        while end < self.rest.len() && control_length(&self.rest[end..]) == 0 {
            end += 1;
        }
        let (run, rest) = self.rest.split_at(end);
        self.rest = rest;
        Some(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(foreground: Color, background: Color, attributes: u8) -> Style {
        Style {
            foreground,
            background,
            attributes,
        }
    }

    fn prefix(style: Style, support: ColorSupport) -> Vec<u8> {
        style_prefix(style, support).as_bytes().to_vec()
    }

    #[test]
    fn a_plain_write_renders_no_style_bytes() {
        for support in [
            ColorSupport::None,
            ColorSupport::Basic,
            ColorSupport::Indexed,
            ColorSupport::TrueColor,
        ] {
            assert!(style_prefix(Style::DEFAULT, support).is_empty());
        }
        let loud = style(Color::Rgb(1, 2, 3), Color::Named(4), 63);
        assert!(style_prefix(loud, ColorSupport::None).is_empty());
        // Attribute bits outside the six known ones render nothing.
        assert!(style_prefix(
            style(Color::Default, Color::Default, 64),
            ColorSupport::TrueColor
        )
        .is_empty());
    }

    #[test]
    fn attributes_and_colors_render_as_one_sgr_sequence() {
        assert_eq!(
            prefix(
                style(Color::Default, Color::Default, ATTRIBUTE_BOLD),
                ColorSupport::Basic
            ),
            b"\x1b[1m"
        );
        assert_eq!(
            prefix(
                style(Color::Named(1), Color::Named(12), 0),
                ColorSupport::Basic
            ),
            b"\x1b[31;104m"
        );
        assert_eq!(
            prefix(
                style(Color::Named(9), Color::Named(7), 0),
                ColorSupport::Basic
            ),
            b"\x1b[91;47m"
        );
        assert_eq!(
            prefix(
                style(Color::Indexed(200), Color::Indexed(17), ATTRIBUTE_UNDERLINE),
                ColorSupport::Indexed
            ),
            b"\x1b[4;38;5;200;48;5;17m"
        );
        assert_eq!(
            prefix(
                style(Color::Default, Color::Rgb(0, 128, 255), 0),
                ColorSupport::TrueColor
            ),
            b"\x1b[48;2;0;128;255m"
        );
    }

    #[test]
    fn the_longest_style_prefix_fits_inline() {
        let longest = prefix(
            style(Color::Rgb(255, 255, 255), Color::Rgb(255, 255, 255), 63),
            ColorSupport::TrueColor,
        );
        assert_eq!(
            longest,
            b"\x1b[1;2;3;4;7;9;38;2;255;255;255;48;2;255;255;255m"
        );
        assert!(longest.len() <= CAPACITY);
        assert!(move_to(u32::MAX, u32::MAX).as_bytes().len() <= CAPACITY);
        assert!(move_by(Direction::Left, u32::MAX).as_bytes().len() <= CAPACITY);
    }

    #[test]
    fn colors_fall_to_the_nearest_one_the_terminal_shows() {
        // True color passes everything through.
        let rgb = Color::Rgb(12, 34, 56);
        assert_eq!(downgrade(rgb, ColorSupport::TrueColor), rgb);
        assert_eq!(
            downgrade(Color::Indexed(99), ColorSupport::TrueColor),
            Color::Indexed(99)
        );

        // 256 colors: exact cube and ramp entries map to themselves.
        assert_eq!(
            downgrade(Color::Rgb(0, 0, 0), ColorSupport::Indexed),
            Color::Indexed(16)
        );
        assert_eq!(
            downgrade(Color::Rgb(255, 255, 255), ColorSupport::Indexed),
            Color::Indexed(231)
        );
        assert_eq!(
            downgrade(Color::Rgb(255, 0, 0), ColorSupport::Indexed),
            Color::Indexed(196)
        );
        assert_eq!(
            downgrade(Color::Rgb(95, 135, 175), ColorSupport::Indexed),
            Color::Indexed(16 + 36 + 6 * 2 + 3)
        );
        assert_eq!(
            downgrade(Color::Rgb(128, 128, 128), ColorSupport::Indexed),
            Color::Indexed(244)
        );
        assert_eq!(
            downgrade(Color::Rgb(8, 8, 8), ColorSupport::Indexed),
            Color::Indexed(232)
        );
        for index in 16..=255u8 {
            let (red, green, blue) = indexed_channels(index);
            let Color::Indexed(mapped) =
                downgrade(Color::Rgb(red, green, blue), ColorSupport::Indexed)
            else {
                panic!("an RGB color must map to an indexed one");
            };
            assert_eq!(
                indexed_channels(mapped),
                (red, green, blue),
                "index {index}"
            );
        }

        // 16 colors.
        assert_eq!(
            downgrade(Color::Rgb(255, 0, 0), ColorSupport::Basic),
            Color::Named(9)
        );
        assert_eq!(
            downgrade(Color::Rgb(200, 10, 10), ColorSupport::Basic),
            Color::Named(1)
        );
        assert_eq!(
            downgrade(Color::Rgb(0, 0, 0), ColorSupport::Basic),
            Color::Named(0)
        );
        assert_eq!(
            downgrade(Color::Rgb(250, 250, 250), ColorSupport::Basic),
            Color::Named(15)
        );
        assert_eq!(
            downgrade(Color::Indexed(3), ColorSupport::Basic),
            Color::Named(3)
        );
        assert_eq!(
            downgrade(Color::Indexed(196), ColorSupport::Basic),
            Color::Named(9)
        );
        assert_eq!(
            downgrade(Color::Indexed(255), ColorSupport::Basic),
            Color::Named(7)
        );
        assert_eq!(
            downgrade(Color::Named(5), ColorSupport::Basic),
            Color::Named(5)
        );
        assert_eq!(
            downgrade(Color::Named(5), ColorSupport::None),
            Color::Default
        );
        assert_eq!(
            prefix(
                style(Color::Rgb(255, 0, 0), Color::Default, 0),
                ColorSupport::Basic
            ),
            b"\x1b[91m"
        );
    }

    #[test]
    fn cursor_movement_is_one_based_on_the_wire_and_row_first() {
        assert_eq!(move_to(0, 0).as_bytes(), b"\x1b[1;1H");
        assert_eq!(move_to(79, 23).as_bytes(), b"\x1b[24;80H");
        assert_eq!(move_by(Direction::Up, 1).as_bytes(), b"\x1b[1A");
        assert_eq!(move_by(Direction::Down, 12).as_bytes(), b"\x1b[12B");
        assert_eq!(move_by(Direction::Right, 3).as_bytes(), b"\x1b[3C");
        assert_eq!(move_by(Direction::Left, 400).as_bytes(), b"\x1b[400D");
        assert!(move_by(Direction::Left, 0).is_empty());
    }

    #[test]
    fn a_title_cannot_carry_a_control_character() {
        let runs =
            |title: &[u8]| -> Vec<Vec<u8>> { title_runs(title).map(<[u8]>::to_vec).collect() };
        assert_eq!(runs(b"plain title"), [b"plain title".to_vec()]);
        assert!(runs(b"").is_empty());
        assert!(runs(b"\x07\x1b\x7f").is_empty());
        assert_eq!(
            runs(b"a\x07b\x1b]2;c\x9c"),
            // 0x9c alone is not valid UTF-8 and is not a C1 encoding; the C1
            // string terminator is C2 9C.
            [b"a".to_vec(), b"b".to_vec(), b"]2;c\x9c".to_vec()]
        );
        assert_eq!(
            runs("caf\u{e9} \u{9c}x\u{80}\u{a0}".as_bytes()),
            [
                "caf\u{e9} ".as_bytes().to_vec(),
                b"x".to_vec(),
                "\u{a0}".as_bytes().to_vec()
            ]
        );
    }
}
