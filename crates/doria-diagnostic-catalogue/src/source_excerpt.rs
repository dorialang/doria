//! Allocation-free source-preview policy shared by hosted and native diagnostics.
//!
//! Consumers supply their existing Unicode grapheme boundaries and display-width
//! implementation. This layer owns tab stops, clipping, and marker placement;
//! it neither allocates nor introduces another Unicode dependency.

pub const DEFAULT_TERMINAL_WIDTH: usize = 100;
pub const TAB_WIDTH: usize = 4;
pub const MINIMUM_GUTTER_WIDTH: usize = 3;
pub const GUTTER_GAP: &str = "      ";

pub const fn source_width(terminal_width: usize) -> usize {
    let width = terminal_width.saturating_sub(12);
    if width < 8 {
        8
    } else {
        width
    }
}

struct Unit<'a> {
    text: &'a str,
    byte_start: usize,
    byte_end: usize,
    column: usize,
    width: usize,
}

fn units<'a>(
    text: &'a str,
    mut boundaries: impl Iterator<Item = usize>,
    measure: impl Fn(&str) -> usize,
    tab_width: usize,
) -> impl Iterator<Item = Unit<'a>> {
    let mut start = boundaries.next().unwrap_or(0);
    let mut column = 0;
    let mut tab_spaces = 0;
    let mut tab_start = 0;
    let tab_width = tab_width.max(1);
    core::iter::from_fn(move || {
        if tab_spaces == 0 {
            let end = boundaries.next()?;
            let value = &text[start..end];
            if value != "\t" {
                let unit = Unit {
                    text: value,
                    byte_start: start,
                    byte_end: end,
                    column,
                    width: measure(value),
                };
                start = end;
                column += unit.width;
                return Some(unit);
            }
            tab_start = start;
            start = end;
            tab_spaces = tab_width - column % tab_width;
        }
        // Individual spaces allow a clipped window to start inside a tab stop.
        let unit = Unit {
            text: " ",
            byte_start: tab_start,
            byte_end: start,
            column,
            width: 1,
        };
        column += 1;
        tab_spaces -= 1;
        Some(unit)
    })
}

pub fn display_width_with_tabs<'a, I: Iterator<Item = usize>>(
    text: &'a str,
    tab_width: usize,
    boundaries: impl Fn(&'a str) -> I,
    measure: impl Fn(&str) -> usize,
) -> usize {
    units(text, boundaries(text), measure, tab_width)
        .map(|unit| unit.width)
        .sum()
}

pub struct SourceExcerpt<'a, B, W> {
    text: &'a str,
    boundaries: B,
    measure: W,
    start_column: usize,
    end_column: usize,
    left_ellipsis: bool,
    right_ellipsis: bool,
    pub marker_offset: usize,
    pub marker_width: usize,
}

impl<'a, I, B, W> SourceExcerpt<'a, B, W>
where
    I: Iterator<Item = usize>,
    B: Fn(&'a str) -> I,
    W: Fn(&str) -> usize,
{
    /// Marker offsets are line-relative UTF-8 bytes. `None` renders context.
    pub fn new(
        text: &'a str,
        marker: Option<(usize, usize)>,
        terminal_width: usize,
        boundaries: B,
        measure: W,
    ) -> Self {
        let maximum_width = source_width(terminal_width);
        let mut total_width = 0;
        let mut marker_start = 0;
        let mut marker_end = 0;
        for unit in units(text, boundaries(text), &measure, TAB_WIDTH) {
            total_width = unit.column + unit.width;
            if let Some((start, end)) = marker {
                if unit.byte_end <= start {
                    marker_start = total_width;
                }
                if unit.byte_start < end {
                    marker_end = total_width;
                }
            }
        }
        let marker_width = marker_end.saturating_sub(marker_start).max(1);
        let (mut start_column, mut end_column) = (0, total_width);
        let mut left_ellipsis = false;
        let mut right_ellipsis = false;
        if total_width > maximum_width {
            let content_width = if marker.is_some() {
                maximum_width - 2
            } else {
                maximum_width - 1
            };
            let desired_start = if marker.is_some() {
                marker_start
                    .saturating_sub(content_width / 3)
                    .min(total_width.saturating_sub(content_width))
            } else {
                0
            };
            let mut found_start = false;
            end_column = desired_start;
            for unit in units(text, boundaries(text), &measure, TAB_WIDTH) {
                let next = unit.column + unit.width;
                if !found_start && next > desired_start {
                    start_column = unit.column;
                    found_start = true;
                }
                if found_start {
                    if next > start_column + content_width {
                        break;
                    }
                    end_column = next;
                }
            }
            left_ellipsis = start_column > 0;
            right_ellipsis = end_column < total_width;
        }
        let adjusted = marker_start.saturating_sub(start_column) + usize::from(left_ellipsis);
        let capacity = maximum_width.saturating_sub(adjusted).max(1);
        let marker_offset = adjusted.min(maximum_width.saturating_sub(marker_width.min(capacity)));
        Self {
            text,
            boundaries,
            measure,
            start_column,
            end_column,
            left_ellipsis,
            right_ellipsis,
            marker_offset,
            marker_width: marker_width
                .min(maximum_width.saturating_sub(marker_offset))
                .max(1),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn write(&self, mut write: impl FnMut(&[u8])) {
        if self.left_ellipsis {
            write("…".as_bytes());
        }
        for unit in units(
            self.text,
            (self.boundaries)(self.text),
            &self.measure,
            TAB_WIDTH,
        ) {
            if unit.column >= self.start_column && unit.column + unit.width <= self.end_column {
                write(unit.text.as_bytes());
            }
            if unit.column + unit.width > self.end_column {
                break;
            }
        }
        if self.right_ellipsis {
            write("…".as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
        text.char_indices()
            .map(|(index, _)| index)
            .chain(core::iter::once(text.len()))
    }

    #[test]
    fn default_window_and_late_marker_are_bounded() {
        let text = std::string::String::from("0123456789").repeat(20);
        let excerpt = SourceExcerpt::new(
            &text,
            Some((170, 200)),
            DEFAULT_TERMINAL_WIDTH,
            boundaries,
            str::len,
        );
        let mut output = std::vec::Vec::new();
        excerpt.write(|bytes| output.extend_from_slice(bytes));
        let output = std::string::String::from_utf8(output).unwrap();
        assert_eq!(source_width(DEFAULT_TERMINAL_WIDTH), 88);
        assert_eq!(output, std::format!("…{}", &text[114..]));
        assert_eq!(excerpt.marker_offset, 57);
        assert_eq!(excerpt.marker_width, 30);
        assert!(excerpt.marker_offset + excerpt.marker_width <= 88);
    }

    #[test]
    fn context_truncation_tabs_and_empty_markers_share_the_policy() {
        let text = "a\tb\tend";
        let excerpt = SourceExcerpt::new(text, Some((1, 2)), 100, boundaries, str::len);
        let mut output = std::vec::Vec::new();
        excerpt.write(|bytes| output.extend_from_slice(bytes));
        assert_eq!(output, b"a   b   end");
        assert_eq!((excerpt.marker_offset, excerpt.marker_width), (1, 3));
        let empty = SourceExcerpt::new("", Some((0, 0)), 1, boundaries, str::len);
        assert!(empty.is_empty());
        assert_eq!((empty.marker_offset, empty.marker_width), (0, 1));
        let context = SourceExcerpt::new("0123456789", None, 1, boundaries, str::len);
        output.clear();
        context.write(|bytes| output.extend_from_slice(bytes));
        assert_eq!(output, "0123456…".as_bytes());
    }
}
