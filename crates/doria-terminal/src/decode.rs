//! The one mapping from terminal input bytes to key events. Unix terminals
//! send these bytes directly; the Windows backend turns console key records
//! into the same bytes (it asks the console for VT input) and the debug
//! interpreter's scripted terminal feeds them from its script, so every
//! backend reports the same portable key set.
//!
//! The decoder is a pure function of the bytes in front of it. The caller
//! owns the buffer, the escape delay and the device.

use super::wire::{Key, MODIFIER_ALT, MODIFIER_CONTROL, MODIFIER_SHIFT};

/// How long a lone Escape byte is held before it is reported as the Escape
/// key. A sequence sent by the terminal arrives in one burst, far inside this.
pub const ESCAPE_DELAY_NANOSECONDS: u64 = 50_000_000;

/// A control sequence longer than this without a final byte is not one the
/// portable key set contains; it is dropped instead of being waited for.
const SEQUENCE_LIMIT: usize = 32;

const ESCAPE: u8 = 0x1b;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The bytes may be the start of a longer key. Read more, or decode again
    /// with `settled` once the escape delay has passed or input has ended.
    Incomplete,
    /// `length` bytes form input outside the portable key set. Drop them.
    Ignored { length: usize },
    /// `length` bytes form one key.
    Key {
        length: usize,
        key: Key,
        modifiers: u8,
    },
}

/// Decodes the key at the front of `bytes`. `settled` says no further byte
/// belongs with these: the escape delay has passed or input has ended.
pub fn decode(bytes: &[u8], settled: bool) -> Step {
    match bytes.first() {
        None => Step::Incomplete,
        Some(&ESCAPE) => escape(bytes, settled, true),
        Some(_) => plain(bytes, settled),
    }
}

const fn key(length: usize, key: Key, modifiers: u8) -> Step {
    Step::Key {
        length,
        key,
        modifiers,
    }
}

/// A key that does not start with Escape.
fn plain(bytes: &[u8], settled: bool) -> Step {
    let first = bytes[0];
    let control = |scalar: u8| key(1, Key::Character(scalar as char), MODIFIER_CONTROL);
    match first {
        b'\r' => key(1, Key::Enter, 0),
        b'\t' => key(1, Key::Tab, 0),
        0x7f | 0x08 => key(1, Key::Backspace, 0),
        0x00 => control(b' '),
        // Control with a letter. Tab, Enter and Backspace took their codes
        // above; a terminal cannot tell those apart from Control+I, +M, +H.
        0x01..=0x1a => control(b'a' + first - 1),
        0x1c => control(b'\\'),
        0x1d => control(b']'),
        0x1e => control(b'^'),
        0x1f => control(b'_'),
        0x20..=0x7e => key(1, Key::Character(first as char), 0),
        _ => scalar(bytes, settled),
    }
}

/// A character outside ASCII. Input that is not UTF-8 is reported as U+FFFD,
/// one per maximal invalid part, the same substitution the string API makes.
fn scalar(bytes: &[u8], settled: bool) -> Step {
    let expected = match bytes[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 1,
    };
    let available = bytes.len().min(expected);
    match core::str::from_utf8(&bytes[..available]) {
        Ok(text) => match text.chars().next() {
            Some(scalar) => key(available, Key::Character(scalar), 0),
            None => Step::Incomplete,
        },
        Err(error) => match error.error_len() {
            Some(length) => key(length, Key::Character(char::REPLACEMENT_CHARACTER), 0),
            None if settled => key(available, Key::Character(char::REPLACEMENT_CHARACTER), 0),
            None => Step::Incomplete,
        },
    }
}

fn with_alt(step: Step) -> Step {
    match step {
        Step::Incomplete => Step::Incomplete,
        Step::Ignored { length } => Step::Ignored { length: length + 1 },
        Step::Key {
            length,
            key,
            modifiers,
        } => Step::Key {
            length: length + 1,
            key,
            modifiers: modifiers | MODIFIER_ALT,
        },
    }
}

/// `bytes` starts with Escape. `alt_allowed` is false for the key that
/// follows an Alt prefix, so Alt applies once.
fn escape(bytes: &[u8], settled: bool, alt_allowed: bool) -> Step {
    let Some(&second) = bytes.get(1) else {
        return if settled {
            key(1, Key::Escape, 0)
        } else {
            Step::Incomplete
        };
    };
    match second {
        b'[' => control_sequence(bytes, settled),
        b'O' => single_shift(bytes, settled),
        ESCAPE if alt_allowed => with_alt(escape(&bytes[1..], settled, false)),
        // Escape, then the start of something else.
        ESCAPE => key(1, Key::Escape, 0),
        _ if alt_allowed => with_alt(plain(&bytes[1..], settled)),
        _ => key(1, Key::Escape, 0),
    }
}

/// The modifier parameter of a control sequence: one more than a bit set of
/// Shift (1), Alt (2) and Control (4). Other bits name keys outside the
/// portable set and are dropped.
fn modifiers_from(parameter: Option<u32>) -> u8 {
    let Some(bits) = parameter.and_then(|value| value.checked_sub(1)) else {
        return 0;
    };
    let mut modifiers = 0;
    if bits & 1 != 0 {
        modifiers |= MODIFIER_SHIFT;
    }
    if bits & 2 != 0 {
        modifiers |= MODIFIER_ALT;
    }
    if bits & 4 != 0 {
        modifiers |= MODIFIER_CONTROL;
    }
    modifiers
}

/// Up to two decimal parameters separated by `;`. `None` when the parameters
/// hold anything else (private markers, intermediates, a third parameter).
fn parameters(bytes: &[u8]) -> Option<(Option<u32>, Option<u32>)> {
    let mut values = [None, None];
    let mut index = 0;
    for byte in bytes {
        match byte {
            b'0'..=b'9' => {
                let digit = u32::from(byte - b'0');
                let value: u32 = values[index].unwrap_or(0);
                values[index] = Some(value.saturating_mul(10).saturating_add(digit));
            }
            b';' if index == 0 => index = 1,
            _ => return None,
        }
    }
    Some((values[0], values[1]))
}

/// `bytes` starts with `ESC [`.
fn control_sequence(bytes: &[u8], settled: bool) -> Step {
    if bytes.len() == 2 {
        return if settled {
            key(2, Key::Character('['), MODIFIER_ALT)
        } else {
            Step::Incomplete
        };
    }
    if bytes[2] == b'[' {
        // The Linux console's F1 to F5: `ESC [ [ A` to `ESC [ [ E`.
        return match bytes.get(3) {
            Some(letter @ b'A'..=b'E') => key(4, Key::Function(letter - b'A' + 1), 0),
            Some(_) => Step::Ignored { length: 3 },
            None if settled => Step::Ignored { length: 3 },
            None => Step::Incomplete,
        };
    }
    let mut end = 2;
    let last = loop {
        match bytes.get(end) {
            // Parameter and intermediate bytes.
            Some(0x20..=0x3f) => end += 1,
            Some(last @ 0x40..=0x7e) => break *last,
            // Not a control sequence after all. Drop what was read; the
            // byte that broke it starts the next key.
            Some(_) => return Step::Ignored { length: end },
            None if settled || end > SEQUENCE_LIMIT => return Step::Ignored { length: end },
            None => return Step::Incomplete,
        }
    };
    let length = end + 1;
    let Some((first, second)) = parameters(&bytes[2..end]) else {
        return Step::Ignored { length };
    };
    let modifiers = modifiers_from(second);
    let named = match last {
        b'~' => match first {
            Some(1 | 7) => Some(Key::Home),
            Some(2) => Some(Key::Insert),
            Some(3) => Some(Key::Delete),
            Some(4 | 8) => Some(Key::End),
            Some(5) => Some(Key::PageUp),
            Some(6) => Some(Key::PageDown),
            Some(number @ 11..=15) => Some(Key::Function(number as u8 - 10)),
            Some(number @ 17..=21) => Some(Key::Function(number as u8 - 11)),
            Some(number @ 23..=24) => Some(Key::Function(number as u8 - 12)),
            _ => None,
        },
        b'Z' if first.is_none() && second.is_none() => {
            return key(length, Key::Tab, MODIFIER_SHIFT);
        }
        // A letter key carries no first parameter, or 1 before a modifier.
        // Anything else that ends in one of these letters is a report (a
        // cursor position reply ends in `R`), not a key.
        _ if !matches!(first, None | Some(1)) => None,
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        b'P' => Some(Key::Function(1)),
        b'Q' => Some(Key::Function(2)),
        b'R' => Some(Key::Function(3)),
        b'S' => Some(Key::Function(4)),
        _ => None,
    };
    match named {
        Some(named) => key(length, named, modifiers),
        None => Step::Ignored { length },
    }
}

/// `bytes` starts with `ESC O`: the application-mode form of the arrow,
/// Home, End and first four function keys.
fn single_shift(bytes: &[u8], settled: bool) -> Step {
    let Some(&last) = bytes.get(2) else {
        return if settled {
            key(2, Key::Character('O'), MODIFIER_ALT)
        } else {
            Step::Incomplete
        };
    };
    let named = match last {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'P' => Key::Function(1),
        b'Q' => Key::Function(2),
        b'R' => Key::Function(3),
        b'S' => Key::Function(4),
        0x40..=0x7e => return Step::Ignored { length: 3 },
        // Alt+O, then the start of something else.
        _ => return key(2, Key::Character('O'), MODIFIER_ALT),
    };
    key(3, named, 0)
}

/// A cursor position reply found in input: `ESC [ row ; column R`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorReport {
    /// Where the reply starts in the searched bytes.
    pub start: usize,
    pub length: usize,
    /// Zero-based.
    pub column: u32,
    /// Zero-based.
    pub row: u32,
}

/// Finds the first complete cursor position reply in `bytes`. Keys typed
/// while the question was in flight may sit on either side of it; the caller
/// removes the reply and leaves them in place.
pub fn find_cursor_report(bytes: &[u8]) -> Option<CursorReport> {
    let mut start = 0;
    while start + 1 < bytes.len() {
        if bytes[start] == ESCAPE && bytes[start + 1] == b'[' {
            if let Some(report) = cursor_report_at(bytes, start) {
                return Some(report);
            }
        }
        start += 1;
    }
    None
}

fn cursor_report_at(bytes: &[u8], start: usize) -> Option<CursorReport> {
    let mut index = start + 2;
    let mut number = |terminator: u8| -> Option<u32> {
        let mut value: u32 = 0;
        let mut digits = 0;
        loop {
            match *bytes.get(index)? {
                digit @ b'0'..=b'9' => {
                    value = value
                        .saturating_mul(10)
                        .saturating_add(u32::from(digit - b'0'));
                    digits += 1;
                    index += 1;
                }
                byte if byte == terminator && digits > 0 => {
                    index += 1;
                    return Some(value);
                }
                _ => return None,
            }
        }
    };
    let row = number(b';')?;
    let column = number(b'R')?;
    Some(CursorReport {
        start,
        length: index - start,
        column: column.saturating_sub(1),
        row: row.saturating_sub(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: u8 = 0;
    const CONTROL: u8 = MODIFIER_CONTROL;
    const ALT: u8 = MODIFIER_ALT;
    const SHIFT: u8 = MODIFIER_SHIFT;

    /// Decodes a whole burst the way the raw-mode reader does once input has
    /// settled, returning the keys and how many bytes were dropped.
    fn keys(mut bytes: &[u8]) -> (Vec<(Key, u8)>, usize) {
        let mut decoded = Vec::new();
        let mut ignored = 0;
        while !bytes.is_empty() {
            match decode(bytes, true) {
                Step::Incomplete => panic!("settled input never stays incomplete: {bytes:?}"),
                Step::Ignored { length } => {
                    assert!(length > 0);
                    ignored += length;
                    bytes = &bytes[length..];
                }
                Step::Key {
                    length,
                    key,
                    modifiers,
                } => {
                    assert!(length > 0);
                    decoded.push((key, modifiers));
                    bytes = &bytes[length..];
                }
            }
        }
        (decoded, ignored)
    }

    fn one(bytes: &[u8]) -> (Key, u8) {
        let (decoded, ignored) = keys(bytes);
        assert_eq!(ignored, 0, "{bytes:?}");
        assert_eq!(decoded.len(), 1, "{bytes:?} decoded to {decoded:?}");
        decoded[0]
    }

    #[test]
    fn printable_and_control_bytes_are_single_keys() {
        assert_eq!(one(b"a"), (Key::Character('a'), NONE));
        assert_eq!(one(b"Z"), (Key::Character('Z'), NONE));
        assert_eq!(one(b" "), (Key::Character(' '), NONE));
        assert_eq!(one(b"~"), (Key::Character('~'), NONE));
        assert_eq!(one(b"\r"), (Key::Enter, NONE));
        assert_eq!(one(b"\t"), (Key::Tab, NONE));
        assert_eq!(one(b"\x7f"), (Key::Backspace, NONE));
        assert_eq!(one(b"\x08"), (Key::Backspace, NONE));
        assert_eq!(one(b"\x03"), (Key::Character('c'), CONTROL));
        assert_eq!(one(b"\x01"), (Key::Character('a'), CONTROL));
        assert_eq!(one(b"\x1a"), (Key::Character('z'), CONTROL));
        assert_eq!(one(b"\n"), (Key::Character('j'), CONTROL));
        assert_eq!(one(b"\x00"), (Key::Character(' '), CONTROL));
        assert_eq!(one(b"\x1c"), (Key::Character('\\'), CONTROL));
        assert_eq!(one(b"\x1d"), (Key::Character(']'), CONTROL));
        assert_eq!(one(b"\x1e"), (Key::Character('^'), CONTROL));
        assert_eq!(one(b"\x1f"), (Key::Character('_'), CONTROL));
    }

    #[test]
    fn characters_are_whole_unicode_scalars() {
        assert_eq!(one("\u{e9}".as_bytes()), (Key::Character('\u{e9}'), NONE));
        assert_eq!(
            one("\u{20ac}".as_bytes()),
            (Key::Character('\u{20ac}'), NONE)
        );
        assert_eq!(
            one("\u{1f600}".as_bytes()),
            (Key::Character('\u{1f600}'), NONE)
        );
        let (decoded, ignored) = keys("a\u{e9}\u{1f600}b".as_bytes());
        assert_eq!(ignored, 0);
        assert_eq!(
            decoded,
            [
                (Key::Character('a'), NONE),
                (Key::Character('\u{e9}'), NONE),
                (Key::Character('\u{1f600}'), NONE),
                (Key::Character('b'), NONE),
            ]
        );
    }

    #[test]
    fn a_scalar_split_across_reads_waits_for_its_remaining_bytes() {
        let euro = "\u{20ac}".as_bytes();
        assert_eq!(decode(&euro[..1], false), Step::Incomplete);
        assert_eq!(decode(&euro[..2], false), Step::Incomplete);
        assert_eq!(
            decode(euro, false),
            key(3, Key::Character('\u{20ac}'), NONE)
        );
        // Input ended inside the scalar.
        assert_eq!(
            decode(&euro[..2], true),
            key(2, Key::Character(char::REPLACEMENT_CHARACTER), NONE)
        );
    }

    #[test]
    fn bytes_that_are_not_utf8_become_replacement_characters() {
        let replacement = (Key::Character(char::REPLACEMENT_CHARACTER), NONE);
        assert_eq!(keys(b"\xff").0, [replacement]);
        assert_eq!(keys(b"\x80").0, [replacement]);
        // A lead byte followed by a byte that cannot continue it: one
        // replacement, then the following key on its own.
        assert_eq!(keys(b"\xe2a").0, [replacement, (Key::Character('a'), NONE)]);
        assert_eq!(
            keys(b"\xe2\x82a").0,
            [replacement, (Key::Character('a'), NONE)]
        );
        // Overlong and surrogate encodings.
        assert_eq!(keys(b"\xc0\x80").0, [replacement, replacement]);
        assert_eq!(
            keys(b"\xed\xa0\x80").0,
            [replacement, replacement, replacement]
        );
        assert_eq!(decode(b"\xe2a", false), key(1, replacement.0, NONE));
    }

    #[test]
    fn a_lone_escape_is_the_escape_key_only_once_input_settles() {
        assert_eq!(decode(b"\x1b", false), Step::Incomplete);
        assert_eq!(decode(b"\x1b", true), key(1, Key::Escape, NONE));
        assert_eq!(decode(b"\x1b[", false), Step::Incomplete);
        assert_eq!(decode(b"\x1b[", true), key(2, Key::Character('['), ALT));
        assert_eq!(decode(b"\x1bO", false), Step::Incomplete);
        assert_eq!(decode(b"\x1bO", true), key(2, Key::Character('O'), ALT));
        assert_eq!(decode(b"\x1b[1;5", false), Step::Incomplete);
        assert_eq!(decode(b"\x1b[1;5", true), Step::Ignored { length: 5 });
        assert_eq!(decode(b"", false), Step::Incomplete);
        assert_eq!(decode(b"", true), Step::Incomplete);
    }

    #[test]
    fn alt_is_an_escape_prefix_applied_once() {
        assert_eq!(one(b"\x1ba"), (Key::Character('a'), ALT));
        assert_eq!(one(b"\x1b\r"), (Key::Enter, ALT));
        assert_eq!(one(b"\x1b\x7f"), (Key::Backspace, ALT));
        assert_eq!(one(b"\x1b\x03"), (Key::Character('c'), CONTROL | ALT));
        assert_eq!(
            one("\x1b\u{e9}".as_bytes()),
            (Key::Character('\u{e9}'), ALT)
        );
        assert_eq!(one(b"\x1b\x1b[A"), (Key::Up, ALT));
        assert_eq!(one(b"\x1b\x1bOP"), (Key::Function(1), ALT));
        assert_eq!(one(b"\x1b\x1b"), (Key::Escape, ALT));
        assert_eq!(
            keys(b"\x1b\x1b\x1b").0,
            [(Key::Escape, ALT), (Key::Escape, NONE)]
        );
        assert_eq!(
            keys(b"\x1b\x1b\x1ba").0,
            [(Key::Escape, ALT), (Key::Character('a'), ALT)]
        );
        // An Alt prefix in front of a split scalar still waits.
        assert_eq!(decode(b"\x1b\xe2\x82", false), Step::Incomplete);
        assert_eq!(decode(b"\x1b\x1b", false), Step::Incomplete);
        assert_eq!(decode(b"\x1b\x1b[", false), Step::Incomplete);
    }

    #[test]
    fn named_keys_decode_from_both_cursor_key_forms() {
        for (bytes, expected) in [
            (&b"\x1b[A"[..], Key::Up),
            (b"\x1b[B", Key::Down),
            (b"\x1b[C", Key::Right),
            (b"\x1b[D", Key::Left),
            (b"\x1b[H", Key::Home),
            (b"\x1b[F", Key::End),
            (b"\x1bOA", Key::Up),
            (b"\x1bOB", Key::Down),
            (b"\x1bOC", Key::Right),
            (b"\x1bOD", Key::Left),
            (b"\x1bOH", Key::Home),
            (b"\x1bOF", Key::End),
            (b"\x1b[1~", Key::Home),
            (b"\x1b[7~", Key::Home),
            (b"\x1b[2~", Key::Insert),
            (b"\x1b[3~", Key::Delete),
            (b"\x1b[4~", Key::End),
            (b"\x1b[8~", Key::End),
            (b"\x1b[5~", Key::PageUp),
            (b"\x1b[6~", Key::PageDown),
        ] {
            assert_eq!(one(bytes), (expected, NONE), "{bytes:?}");
            assert_eq!(decode(bytes, false), key(bytes.len(), expected, NONE));
        }
    }

    #[test]
    fn function_keys_decode_from_every_common_form() {
        let forms: [&[&[u8]]; 12] = [
            &[b"\x1bOP", b"\x1b[P", b"\x1b[11~", b"\x1b[[A"],
            &[b"\x1bOQ", b"\x1b[Q", b"\x1b[12~", b"\x1b[[B"],
            &[b"\x1bOR", b"\x1b[R", b"\x1b[13~", b"\x1b[[C"],
            &[b"\x1bOS", b"\x1b[S", b"\x1b[14~", b"\x1b[[D"],
            &[b"\x1b[15~", b"\x1b[[E"],
            &[b"\x1b[17~"],
            &[b"\x1b[18~"],
            &[b"\x1b[19~"],
            &[b"\x1b[20~"],
            &[b"\x1b[21~"],
            &[b"\x1b[23~"],
            &[b"\x1b[24~"],
        ];
        for (index, sequences) in forms.iter().enumerate() {
            for bytes in *sequences {
                assert_eq!(
                    one(bytes),
                    (Key::Function(index as u8 + 1), NONE),
                    "{bytes:?}"
                );
            }
        }
    }

    #[test]
    fn modifier_parameters_set_shift_alt_and_control() {
        assert_eq!(one(b"\x1b[1;2A"), (Key::Up, SHIFT));
        assert_eq!(one(b"\x1b[1;3B"), (Key::Down, ALT));
        assert_eq!(one(b"\x1b[1;5C"), (Key::Right, CONTROL));
        assert_eq!(one(b"\x1b[1;8D"), (Key::Left, SHIFT | ALT | CONTROL));
        assert_eq!(one(b"\x1b[1;5H"), (Key::Home, CONTROL));
        assert_eq!(one(b"\x1b[3;5~"), (Key::Delete, CONTROL));
        assert_eq!(one(b"\x1b[5;2~"), (Key::PageUp, SHIFT));
        assert_eq!(one(b"\x1b[15;6~"), (Key::Function(5), SHIFT | CONTROL));
        assert_eq!(one(b"\x1b[1;2P"), (Key::Function(1), SHIFT));
        assert_eq!(one(b"\x1b[Z"), (Key::Tab, SHIFT));
        // The Meta bit is outside the portable set and is dropped.
        assert_eq!(one(b"\x1b[1;9A"), (Key::Up, NONE));
        assert_eq!(one(b"\x1b[1;1A"), (Key::Up, NONE));
    }

    #[test]
    fn input_outside_the_portable_key_set_is_dropped_whole() {
        for bytes in [
            &b"\x1b[200~"[..], // bracketed paste start
            b"\x1b[I",         // focus in
            b"\x1b[<0;10;10M", // SGR mouse
            b"\x1b[?1;2c",     // device attributes
            b"\x1b[97;5u",     // extended keyboard report
            b"\x1b[24;80R",    // an unrequested cursor position reply
            b"\x1b[1;2;3A",    // a third parameter
            b"\x1b[25~",       // F13
            b"\x1bOM",         // keypad Enter in application mode
        ] {
            assert_eq!(keys(bytes), (Vec::new(), bytes.len()), "{bytes:?}");
        }
        // Not a Linux console function key: the introducer is dropped and
        // the letter is typed text.
        assert_eq!(keys(b"\x1b[[Z"), (vec![(Key::Character('Z'), NONE)], 3));
        assert_eq!(decode(b"\x1b[200~", false), Step::Ignored { length: 6 });
        assert_eq!(decode(b"\x1b[24;80R", false), Step::Ignored { length: 8 });
        assert_eq!(decode(b"\x1bOM", false), Step::Ignored { length: 3 });
    }

    #[test]
    fn a_broken_sequence_never_swallows_the_key_after_it() {
        assert_eq!(
            keys(b"\x1b[1\x1b[A"),
            (vec![(Key::Up, NONE)], 3),
            "an escape inside a control sequence starts a new key"
        );
        assert_eq!(keys(b"\x1b[\r"), (vec![(Key::Enter, NONE)], 2));
        assert_eq!(
            keys(b"\x1bO\x1b[B"),
            (vec![(Key::Character('O'), ALT), (Key::Down, NONE)], 0)
        );
        assert_eq!(keys(b"\x1b[[\x1b[C"), (vec![(Key::Right, NONE)], 3));
    }

    #[test]
    fn an_endless_control_sequence_is_dropped_not_awaited() {
        let mut bytes = b"\x1b[".to_vec();
        bytes.extend(core::iter::repeat_n(b'1', SEQUENCE_LIMIT - 2));
        assert_eq!(decode(&bytes, false), Step::Incomplete);
        bytes.push(b'1');
        assert_eq!(
            decode(&bytes, false),
            Step::Ignored {
                length: bytes.len()
            }
        );
        // Saturating parameters never overflow.
        let mut huge = b"\x1b[".to_vec();
        huge.extend(core::iter::repeat_n(b'9', 20));
        huge.push(b'~');
        assert_eq!(decode(&huge, false), Step::Ignored { length: huge.len() });
    }

    #[test]
    fn a_burst_decodes_in_order() {
        let (decoded, ignored) = keys(b"hi\x1b[A\x03\x1bx\x1b[1;5D\r");
        assert_eq!(ignored, 0);
        assert_eq!(
            decoded,
            [
                (Key::Character('h'), NONE),
                (Key::Character('i'), NONE),
                (Key::Up, NONE),
                (Key::Character('c'), CONTROL),
                (Key::Character('x'), ALT),
                (Key::Left, CONTROL),
                (Key::Enter, NONE),
            ]
        );
    }

    #[test]
    fn every_prefix_of_every_key_either_waits_or_decodes_the_same_key() {
        // A key must never decode differently because a read split it.
        for bytes in [
            &b"\x1b[1;5A"[..],
            b"\x1b[15;6~",
            b"\x1bOP",
            b"\x1b[[E",
            b"\x1b\x1b[1;2H",
            "\x1b\u{1f600}".as_bytes(),
            "\u{1f600}".as_bytes(),
        ] {
            let whole = decode(bytes, false);
            assert!(matches!(whole, Step::Key { length, .. } if length == bytes.len()));
            for end in 1..bytes.len() {
                assert_eq!(
                    decode(&bytes[..end], false),
                    Step::Incomplete,
                    "{:?}",
                    &bytes[..end]
                );
            }
        }
    }

    #[test]
    fn a_cursor_report_is_found_among_typed_keys() {
        assert_eq!(
            find_cursor_report(b"\x1b[24;80R"),
            Some(CursorReport {
                start: 0,
                length: 8,
                column: 79,
                row: 23
            })
        );
        assert_eq!(
            find_cursor_report(b"ab\x1b[A\x1b[1;1Rcd"),
            Some(CursorReport {
                start: 5,
                length: 6,
                column: 0,
                row: 0
            })
        );
        for bytes in [
            &b""[..],
            b"\x1b",
            b"\x1b[",
            b"\x1b[24;80",
            b"\x1b[24;R",
            b"\x1b[;80R",
            b"\x1b[24R",
            b"\x1b[A",
            b"24;80R",
        ] {
            assert_eq!(find_cursor_report(bytes), None, "{bytes:?}");
        }
        // A terminal that answers zero is treated as the origin.
        assert_eq!(
            find_cursor_report(b"\x1b[0;0R").map(|report| (report.column, report.row)),
            Some((0, 0))
        );
    }
}
