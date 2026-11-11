//! Integer transport between the authored `Doria\Std\Term` module and the
//! terminal layer. The runtime, the debug interpreter's scripted terminal and
//! the PHP host all pack and unpack with these definitions, so the three
//! backends cannot drift. This is private ABI, never a stable interface.

/// Result status carried in bits 0..8 of every result.
pub const STATUS_OK: u8 = 0;
pub const STATUS_NOT_A_TERMINAL: u8 = 1;
pub const STATUS_IN_USE: u8 = 2;
pub const STATUS_UNSUPPORTED: u8 = 3;
pub const STATUS_DEVICE_FAILURE: u8 = 4;

const STATUS_BITS: u32 = 8;
const SYSTEM_CODE_PRESENT: i64 = 1 << 40;

/// Operation selectors of `dr_v1_terminal_operate`.
pub mod operation {
    pub const SIZE: u32 = 0;
    pub const COLOR_SUPPORT: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const WRITE_AT: u32 = 3;
    pub const FLUSH: u32 = 4;
    pub const CLEAR: u32 = 5;
    pub const CLEAR_LINE: u32 = 6;
    pub const CLEAR_LINE_FROM_CURSOR: u32 = 7;
    pub const ERASE_AT: u32 = 8;
    pub const SET_TITLE: u32 = 9;
    pub const SET_LINE_WRAP: u32 = 10;
    pub const MOVE_TO: u32 = 11;
    pub const MOVE_UP: u32 = 12;
    pub const MOVE_DOWN: u32 = 13;
    pub const MOVE_LEFT: u32 = 14;
    pub const MOVE_RIGHT: u32 = 15;
    pub const SHOW_CURSOR: u32 = 16;
    pub const HIDE_CURSOR: u32 = 17;
    /// Returns the guard's generation. `LEAVE_RAW_MODE` takes it back and
    /// does nothing for a generation that has already left, so a guard's
    /// `leave()` and its destructor share one operation.
    pub const ENTER_RAW_MODE: u32 = 18;
    pub const LEAVE_RAW_MODE: u32 = 19;
    pub const ENTER_ALTERNATE_SCREEN: u32 = 20;
    pub const LEAVE_ALTERNATE_SCREEN: u32 = 21;
    pub const RESET: u32 = 22;
    pub const CURSOR_POSITION: u32 = 23;
    /// Whether a standard stream (`first`: 0 input, 1 output, 2 error) is
    /// attached to a terminal. The payload is 1 or 0; it never fails.
    pub const STREAM_IS_INTERACTIVE: u32 = 24;
}

/// The largest column, row or movement count the layer transports. Larger
/// requests clamp here; no terminal has a dimension near it.
pub const COORDINATE_LIMIT: u32 = (1 << 24) - 1;

pub const fn success(payload: u64) -> i64 {
    (payload << STATUS_BITS) as i64
}

pub const fn failure(status: u8, system_code: Option<i64>) -> i64 {
    match system_code {
        // Native codes are errno values or Win32 error codes: 32 bits.
        Some(code) => status as i64 | ((code as u32 as i64) << STATUS_BITS) | SYSTEM_CODE_PRESENT,
        None => status as i64,
    }
}

pub const fn status(result: i64) -> u8 {
    result as u8
}

pub const fn payload(result: i64) -> u64 {
    (result as u64) >> STATUS_BITS
}

pub const fn system_code(result: i64) -> Option<i64> {
    if result & SYSTEM_CODE_PRESENT != 0 {
        Some(((result >> STATUS_BITS) as u32) as i64)
    } else {
        None
    }
}

const fn clamp(value: u32) -> u64 {
    if value > COORDINATE_LIMIT {
        COORDINATE_LIMIT as u64
    } else {
        value as u64
    }
}

/// A size (`columns`, `rows`) or a zero-based position (`column`, `row`).
pub const fn pair(first: u32, second: u32) -> u64 {
    clamp(first) | (clamp(second) << 24)
}

pub const fn pair_first(payload: u64) -> u32 {
    (payload & COORDINATE_LIMIT as u64) as u32
}

pub const fn pair_second(payload: u64) -> u32 {
    ((payload >> 24) & COORDINATE_LIMIT as u64) as u32
}

/// A cell argument of `WRITE_AT`, `ERASE_AT` and `MOVE_TO`: column in the
/// high word, row in the low word, both already non-negative.
pub const fn cell(column: u32, row: u32) -> i64 {
    ((column as i64) << 32) | row as i64
}

pub const fn cell_column(argument: i64) -> u32 {
    (argument >> 32) as u32
}

pub const fn cell_row(argument: i64) -> u32 {
    argument as u32
}

/// `ColorSupport`, in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ColorSupport {
    None = 0,
    Basic = 1,
    Indexed = 2,
    TrueColor = 3,
}

impl ColorSupport {
    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Basic,
            2 => Self::Indexed,
            3 => Self::TrueColor,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Default,
    /// One of the sixteen named colors, `0..16`.
    Named(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    const fn to_bits(self) -> u64 {
        match self {
            Self::Default => 0,
            Self::Named(index) => 1 | ((index as u64 & 15) << 2),
            Self::Indexed(index) => 2 | ((index as u64) << 2),
            Self::Rgb(red, green, blue) => {
                3 | ((red as u64) << 2) | ((green as u64) << 10) | ((blue as u64) << 18)
            }
        }
    }

    const fn from_bits(bits: u64) -> Self {
        let value = bits >> 2;
        match bits & 3 {
            0 => Self::Default,
            1 => Self::Named((value & 15) as u8),
            2 => Self::Indexed(value as u8),
            _ => Self::Rgb(value as u8, (value >> 8) as u8, (value >> 16) as u8),
        }
    }
}

pub const ATTRIBUTE_BOLD: u8 = 1;
pub const ATTRIBUTE_DIM: u8 = 2;
pub const ATTRIBUTE_ITALIC: u8 = 4;
pub const ATTRIBUTE_UNDERLINE: u8 = 8;
pub const ATTRIBUTE_REVERSE: u8 = 16;
pub const ATTRIBUTE_STRIKETHROUGH: u8 = 32;
const ATTRIBUTE_MASK: u8 = 63;

const COLOR_BITS: u32 = 26;
const COLOR_MASK: u64 = (1 << COLOR_BITS) - 1;

/// A `Style` value. Zero is the default style, so an absent `?Style` and the
/// default style share one encoding and a styled write needs no presence bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub foreground: Color,
    pub background: Color,
    pub attributes: u8,
}

impl Style {
    pub const DEFAULT: Self = Self {
        foreground: Color::Default,
        background: Color::Default,
        attributes: 0,
    };

    pub const fn to_bits(self) -> i64 {
        (self.foreground.to_bits()
            | (self.background.to_bits() << COLOR_BITS)
            | (((self.attributes & ATTRIBUTE_MASK) as u64) << (2 * COLOR_BITS))) as i64
    }

    pub const fn from_bits(bits: i64) -> Self {
        let bits = bits as u64;
        Self {
            foreground: Color::from_bits(bits & COLOR_MASK),
            background: Color::from_bits((bits >> COLOR_BITS) & COLOR_MASK),
            attributes: (bits >> (2 * COLOR_BITS)) as u8 & ATTRIBUTE_MASK,
        }
    }
}

/// `Key`, in declaration order. `Character` carries a Unicode scalar and
/// `Function` a function-key number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Character(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Function(u8),
}

impl Key {
    const fn code(self) -> (u64, u64) {
        match self {
            Self::Character(scalar) => (0, scalar as u64),
            Self::Enter => (1, 0),
            Self::Escape => (2, 0),
            Self::Backspace => (3, 0),
            Self::Tab => (4, 0),
            Self::Up => (5, 0),
            Self::Down => (6, 0),
            Self::Left => (7, 0),
            Self::Right => (8, 0),
            Self::Home => (9, 0),
            Self::End => (10, 0),
            Self::PageUp => (11, 0),
            Self::PageDown => (12, 0),
            Self::Insert => (13, 0),
            Self::Delete => (14, 0),
            Self::Function(number) => (15, number as u64),
        }
    }

    const fn from_code(code: u64, value: u64) -> Option<Self> {
        Some(match code {
            0 => match char::from_u32(value as u32) {
                Some(scalar) => Self::Character(scalar),
                None => return None,
            },
            1 => Self::Enter,
            2 => Self::Escape,
            3 => Self::Backspace,
            4 => Self::Tab,
            5 => Self::Up,
            6 => Self::Down,
            7 => Self::Left,
            8 => Self::Right,
            9 => Self::Home,
            10 => Self::End,
            11 => Self::PageUp,
            12 => Self::PageDown,
            13 => Self::Insert,
            14 => Self::Delete,
            15 => Self::Function(value as u8),
            _ => return None,
        })
    }
}

pub const MODIFIER_CONTROL: u8 = 1;
pub const MODIFIER_ALT: u8 = 2;
pub const MODIFIER_SHIFT: u8 = 4;

/// What one event wait observed. Expiry, closure and cancellation are typed
/// observations of the wait, never failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    TimedOut,
    Closed,
    Cancelled,
    Key { key: Key, modifiers: u8 },
    Resized { columns: u32, rows: u32 },
}

impl Observation {
    pub const fn to_payload(self) -> u64 {
        match self {
            Self::TimedOut => 0,
            Self::Closed => 1,
            Self::Cancelled => 2,
            Self::Key { key, modifiers } => {
                let (code, value) = key.code();
                3 | (code << 4) | (((modifiers & 7) as u64) << 12) | (value << 16)
            }
            Self::Resized { columns, rows } => 4 | (pair(columns, rows) << 4),
        }
    }

    pub const fn from_payload(payload: u64) -> Option<Self> {
        Some(match payload & 15 {
            0 => Self::TimedOut,
            1 => Self::Closed,
            2 => Self::Cancelled,
            3 => match Key::from_code((payload >> 4) & 255, (payload >> 16) & 0x1f_ffff) {
                Some(key) => Self::Key {
                    key,
                    modifiers: (payload >> 12) as u8 & 7,
                },
                None => return None,
            },
            4 => Self::Resized {
                columns: pair_first(payload >> 4),
                rows: pair_second(payload >> 4),
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_status_payload_and_native_code_apart() {
        let size = success(pair(132, 43));
        assert_eq!(status(size), STATUS_OK);
        assert_eq!(pair_first(payload(size)), 132);
        assert_eq!(pair_second(payload(size)), 43);
        assert_eq!(system_code(size), None);
        assert!(size >= 0);

        let plain = failure(STATUS_IN_USE, None);
        assert_eq!((status(plain), system_code(plain)), (STATUS_IN_USE, None));
        for code in [0, 5, 25, 6, i64::from(u32::MAX)] {
            let coded = failure(STATUS_DEVICE_FAILURE, Some(code));
            assert_eq!(status(coded), STATUS_DEVICE_FAILURE);
            assert_eq!(system_code(coded), Some(code));
            assert!(coded >= 0);
        }
    }

    #[test]
    fn pairs_and_cells_clamp_instead_of_wrapping() {
        let clamped = pair(u32::MAX, COORDINATE_LIMIT + 1);
        assert_eq!(pair_first(clamped), COORDINATE_LIMIT);
        assert_eq!(pair_second(clamped), COORDINATE_LIMIT);
        let argument = cell(7, 9);
        assert_eq!((cell_column(argument), cell_row(argument)), (7, 9));
    }

    #[test]
    fn the_default_style_is_zero_and_every_style_round_trips() {
        assert_eq!(Style::DEFAULT.to_bits(), 0);
        assert_eq!(Style::from_bits(0), Style::DEFAULT);
        let colors = [
            Color::Default,
            Color::Named(0),
            Color::Named(15),
            Color::Indexed(0),
            Color::Indexed(255),
            Color::Rgb(0, 0, 0),
            Color::Rgb(255, 128, 1),
        ];
        for foreground in colors {
            for background in colors {
                for attributes in [0, ATTRIBUTE_BOLD, ATTRIBUTE_STRIKETHROUGH, ATTRIBUTE_MASK] {
                    let style = Style {
                        foreground,
                        background,
                        attributes,
                    };
                    let bits = style.to_bits();
                    assert!(bits >= 0);
                    assert_eq!(Style::from_bits(bits), style);
                }
            }
        }
        // A named or indexed black is not the default color.
        assert_ne!(
            Style {
                foreground: Color::Named(0),
                ..Style::DEFAULT
            }
            .to_bits(),
            0
        );
    }

    #[test]
    fn observations_round_trip_through_a_successful_result() {
        let keys = [
            Key::Character('a'),
            Key::Character('\u{10ffff}'),
            Key::Character(' '),
            Key::Enter,
            Key::Escape,
            Key::Backspace,
            Key::Tab,
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
            Key::Insert,
            Key::Delete,
            Key::Function(1),
            Key::Function(12),
        ];
        let mut observations = vec![
            Observation::TimedOut,
            Observation::Closed,
            Observation::Cancelled,
            Observation::Resized {
                columns: 0,
                rows: 0,
            },
            Observation::Resized {
                columns: 80,
                rows: 24,
            },
            Observation::Resized {
                columns: COORDINATE_LIMIT,
                rows: COORDINATE_LIMIT,
            },
        ];
        for key in keys {
            for modifiers in 0..8 {
                observations.push(Observation::Key { key, modifiers });
            }
        }
        for observation in observations {
            let result = success(observation.to_payload());
            assert!(result >= 0, "{observation:?}");
            assert_eq!(status(result), STATUS_OK);
            assert_eq!(
                Observation::from_payload(payload(result)),
                Some(observation)
            );
        }
        // Surrogates are not scalars and unknown kinds are not events.
        assert_eq!(Observation::from_payload(3 | (0xd800 << 16)), None);
        assert_eq!(Observation::from_payload(9), None);
    }
}
