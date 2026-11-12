//! A small terminal screen: enough of a VT terminal to show what a program
//! drew, whether the bytes came straight from the program or were re-rendered
//! by ConPTY.

#[derive(Clone)]
struct Grid {
    cells: Vec<char>,
    column: usize,
    row: usize,
}

enum State {
    Ground,
    Escape,
    Control,
    OperatingSystem,
    OperatingSystemEscape,
    Charset,
}

pub struct Screen {
    columns: usize,
    rows: usize,
    main: Grid,
    alternate: Grid,
    on_alternate: bool,
    cursor_visible: bool,
    line_wrap: bool,
    state: State,
    parameters: Vec<u8>,
    scalar: Vec<u8>,
    title: Vec<u8>,
}

impl Screen {
    pub fn new(columns: u16, rows: u16) -> Self {
        let (columns, rows) = (usize::from(columns), usize::from(rows));
        let grid = Grid {
            cells: vec![' '; columns * rows],
            column: 0,
            row: 0,
        };
        Self {
            columns,
            rows,
            main: grid.clone(),
            alternate: grid,
            on_alternate: false,
            cursor_visible: true,
            line_wrap: true,
            state: State::Ground,
            parameters: Vec::new(),
            scalar: Vec::new(),
            title: Vec::new(),
        }
    }

    /// Changes the window size. What was drawn stays where it was, as far as
    /// it still fits; nothing is reflowed. A program that draws its own
    /// screen draws it again when it is told about the new size.
    pub fn resize(&mut self, columns: u16, rows: u16) {
        let (columns, rows) = (usize::from(columns), usize::from(rows));
        let (old_columns, old_rows) = (self.columns, self.rows);
        for grid in [&mut self.main, &mut self.alternate] {
            let mut cells = vec![' '; columns * rows];
            for row in 0..rows.min(old_rows) {
                for column in 0..columns.min(old_columns) {
                    cells[row * columns + column] = grid.cells[row * old_columns + column];
                }
            }
            grid.cells = cells;
            grid.column = grid.column.min(columns - 1);
            grid.row = grid.row.min(rows - 1);
        }
        self.columns = columns;
        self.rows = rows;
    }

    pub fn on_alternate_screen(&self) -> bool {
        self.on_alternate
    }

    pub fn cursor_is_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn line_wrap_is_on(&self) -> bool {
        self.line_wrap
    }

    /// The zero-based column and row of the cursor on the screen in use.
    pub fn cursor(&self) -> (usize, usize) {
        let grid = self.grid();
        (grid.column.min(self.columns - 1), grid.row)
    }

    pub fn title(&self) -> String {
        String::from_utf8_lossy(&self.title).into_owned()
    }

    /// The screen in use as text: rows without trailing spaces, joined by
    /// newlines, without trailing empty rows.
    pub fn text(&self) -> String {
        Self::grid_text(self.grid(), self.columns)
    }

    /// The main screen, whichever screen is in use.
    pub fn main_text(&self) -> String {
        Self::grid_text(&self.main, self.columns)
    }

    fn grid_text(grid: &Grid, columns: usize) -> String {
        let rows: Vec<String> = grid
            .cells
            .chunks(columns)
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect();
        let used = rows
            .iter()
            .rposition(|row| !row.is_empty())
            .map_or(0, |last| last + 1);
        rows[..used].join("\n")
    }

    fn grid(&self) -> &Grid {
        if self.on_alternate {
            &self.alternate
        } else {
            &self.main
        }
    }

    fn grid_mut(&mut self) -> &mut Grid {
        if self.on_alternate {
            &mut self.alternate
        } else {
            &mut self.main
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.accept(byte);
        }
    }

    fn accept(&mut self, byte: u8) {
        match self.state {
            State::Ground => self.ground(byte),
            State::Escape => {
                self.state = State::Ground;
                match byte {
                    b'[' => {
                        self.parameters.clear();
                        self.state = State::Control;
                    }
                    b']' => {
                        self.parameters.clear();
                        self.state = State::OperatingSystem;
                    }
                    b'(' | b')' | b'*' | b'+' => self.state = State::Charset,
                    b'M' => self.move_rows(-1),
                    _ => {}
                }
            }
            State::Charset => self.state = State::Ground,
            State::Control => {
                if (0x40..=0x7e).contains(&byte) {
                    self.state = State::Ground;
                    self.control(byte);
                } else if byte == 0x1b {
                    self.state = State::Escape;
                } else {
                    self.parameters.push(byte);
                }
            }
            State::OperatingSystem => match byte {
                0x07 => self.operating_system(),
                0x1b => self.state = State::OperatingSystemEscape,
                _ => self.parameters.push(byte),
            },
            State::OperatingSystemEscape => self.operating_system(),
        }
    }

    fn operating_system(&mut self) {
        self.state = State::Ground;
        let command = std::mem::take(&mut self.parameters);
        if let Some(title) = command
            .strip_prefix(b"0;")
            .or_else(|| command.strip_prefix(b"2;"))
        {
            self.title = title.to_vec();
        }
    }

    fn ground(&mut self, byte: u8) {
        match byte {
            0x1b => {
                self.scalar.clear();
                self.state = State::Escape;
            }
            b'\r' => self.grid_mut().column = 0,
            b'\n' => self.line_feed(),
            0x08 => {
                let grid = self.grid_mut();
                grid.column = grid.column.saturating_sub(1);
            }
            b'\t' => {
                let last = self.columns - 1;
                let grid = self.grid_mut();
                grid.column = ((grid.column / 8 + 1) * 8).min(last);
            }
            0x00..=0x1f | 0x7f => {}
            _ => {
                self.scalar.push(byte);
                match std::str::from_utf8(&self.scalar) {
                    Ok(text) => {
                        let scalar = text.chars().next().expect("one scalar");
                        self.scalar.clear();
                        self.print(scalar);
                    }
                    Err(error) if error.error_len().is_some() => {
                        self.scalar.clear();
                        self.print(char::REPLACEMENT_CHARACTER);
                    }
                    Err(_) => {}
                }
            }
        }
    }

    fn print(&mut self, scalar: char) {
        let (columns, wrap) = (self.columns, self.line_wrap);
        if self.grid().column >= columns {
            if wrap {
                self.grid_mut().column = 0;
                self.line_feed();
            } else {
                self.grid_mut().column = columns - 1;
            }
        }
        let grid = self.grid_mut();
        let cell = grid.row * columns + grid.column;
        grid.cells[cell] = scalar;
        grid.column += 1;
    }

    fn line_feed(&mut self) {
        let (columns, rows) = (self.columns, self.rows);
        let grid = self.grid_mut();
        if grid.row + 1 < rows {
            grid.row += 1;
        } else {
            grid.cells.drain(..columns);
            grid.cells.extend(std::iter::repeat_n(' ', columns));
        }
    }

    fn move_rows(&mut self, by: i64) {
        let last = self.rows as i64 - 1;
        let grid = self.grid_mut();
        grid.row = (grid.row as i64 + by).clamp(0, last) as usize;
    }

    fn move_columns(&mut self, by: i64) {
        let last = self.columns as i64 - 1;
        let grid = self.grid_mut();
        grid.column = (grid.column.min(last as usize) as i64 + by).clamp(0, last) as usize;
    }

    fn erase(&mut self, from: usize, to: usize) {
        let grid = self.grid_mut();
        let to = to.min(grid.cells.len());
        if from < to {
            grid.cells[from..to].fill(' ');
        }
    }

    fn control(&mut self, last: u8) {
        let parameters = std::mem::take(&mut self.parameters);
        let private = parameters.first() == Some(&b'?');
        let numbers: Vec<usize> = parameters
            .iter()
            .filter(|byte| byte.is_ascii_digit() || **byte == b';')
            .copied()
            .collect::<Vec<u8>>()
            .split(|byte| *byte == b';')
            .map(|digits| {
                digits.iter().fold(0usize, |value, digit| {
                    value * 10 + usize::from(digit - b'0')
                })
            })
            .collect();
        let number = |index: usize, default: usize| match numbers.get(index) {
            Some(0) | None => default,
            Some(value) => *value,
        };
        let (columns, rows) = (self.columns, self.rows);
        if private {
            let on = last == b'h';
            if last == b'h' || last == b'l' {
                for mode in &numbers {
                    match mode {
                        25 => self.cursor_visible = on,
                        7 => self.line_wrap = on,
                        47 | 1047 | 1049 => {
                            if on && !self.on_alternate {
                                self.alternate.cells.fill(' ');
                                self.alternate.column = 0;
                                self.alternate.row = 0;
                            }
                            self.on_alternate = on;
                        }
                        _ => {}
                    }
                }
            }
            return;
        }
        match last {
            b'H' | b'f' => {
                let grid = self.grid_mut();
                grid.row = (number(0, 1) - 1).min(rows - 1);
                grid.column = (number(1, 1) - 1).min(columns - 1);
            }
            b'A' => self.move_rows(-(number(0, 1) as i64)),
            b'B' | b'e' => self.move_rows(number(0, 1) as i64),
            b'C' | b'a' => self.move_columns(number(0, 1) as i64),
            b'D' => self.move_columns(-(number(0, 1) as i64)),
            b'E' => {
                self.move_rows(number(0, 1) as i64);
                self.grid_mut().column = 0;
            }
            b'F' => {
                self.move_rows(-(number(0, 1) as i64));
                self.grid_mut().column = 0;
            }
            b'G' | b'`' => self.grid_mut().column = (number(0, 1) - 1).min(columns - 1),
            b'd' => self.grid_mut().row = (number(0, 1) - 1).min(rows - 1),
            b'J' => {
                let grid = self.grid();
                let cursor = grid.row * columns + grid.column.min(columns - 1);
                match numbers.first().copied().unwrap_or(0) {
                    0 => self.erase(cursor, columns * rows),
                    1 => self.erase(0, cursor + 1),
                    _ => self.erase(0, columns * rows),
                }
            }
            b'K' => {
                let grid = self.grid();
                let start = grid.row * columns;
                let cursor = start + grid.column.min(columns - 1);
                match numbers.first().copied().unwrap_or(0) {
                    0 => self.erase(cursor, start + columns),
                    1 => self.erase(start, cursor + 1),
                    _ => self.erase(start, start + columns),
                }
            }
            b'X' => {
                let grid = self.grid();
                let start = grid.row * columns;
                let cursor = start + grid.column.min(columns - 1);
                self.erase(cursor, (cursor + number(0, 1)).min(start + columns));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(bytes: &[u8]) -> Screen {
        let mut screen = Screen::new(20, 5);
        screen.feed(bytes);
        screen
    }

    #[test]
    fn text_lands_where_the_cursor_was_moved() {
        let screen = shown(b"\x1b[2J\x1b[H\x1b[2;3H\x1b[1;31mhi\x1b[0m\x1b[4;1H\x1b[4Cx");
        assert_eq!(screen.text(), "\n  hi\n\n    x");
        assert_eq!(screen.cursor(), (5, 3));
    }

    #[test]
    fn the_alternate_screen_is_a_second_screen_and_leaving_shows_the_first_again() {
        let mut screen = shown(b"shell$ \x1b[?1049h\x1b[?25l\x1b[3;1Hmenu");
        assert!(screen.on_alternate_screen());
        assert!(!screen.cursor_is_visible());
        assert_eq!(screen.text(), "\n\nmenu");
        assert_eq!(screen.main_text(), "shell$");
        screen.feed(b"\x1b[?25h\x1b[?1049ldone");
        assert!(!screen.on_alternate_screen());
        assert!(screen.cursor_is_visible());
        assert_eq!(screen.text(), "shell$ done");
    }

    #[test]
    fn erasing_clears_a_cell_a_line_or_the_screen() {
        let mut screen = shown(b"abcdef\r\nghijkl\x1b[1;3H\x1b[X");
        assert_eq!(screen.text(), "ab def\nghijkl");
        screen.feed(b"\x1b[2;4H\x1b[K");
        assert_eq!(screen.text(), "ab def\nghi");
        screen.feed(b"\x1b[2K");
        assert_eq!(screen.text(), "ab def");
        screen.feed(b"\x1b[2J");
        assert_eq!(screen.text(), "");
    }

    #[test]
    fn a_title_and_sequences_it_does_not_know_never_reach_the_screen() {
        let screen = shown(b"\x1b]2;menu\x07a\x1b]0;other\x1b\\b\x1b[?9001h\x1b[?1004h\x1b(Bc\x1b[38;5;196mD\x1b[m");
        assert_eq!(screen.text(), "abcD");
        assert_eq!(screen.title(), "other");
    }

    #[test]
    fn text_wraps_at_the_right_edge_unless_wrap_is_off_and_the_last_row_scrolls() {
        let mut screen = Screen::new(4, 2);
        screen.feed(b"abcdef");
        assert_eq!(screen.text(), "abcd\nef");
        screen.feed(b"\r\nxy\r\nz");
        assert_eq!(screen.text(), "xy\nz");
        let mut screen = Screen::new(4, 2);
        screen.feed(b"\x1b[?7labcdef");
        assert!(!screen.line_wrap_is_on());
        assert_eq!(screen.text(), "abcf");
    }

    #[test]
    fn characters_outside_ascii_take_one_cell_however_the_bytes_are_split() {
        let mut screen = Screen::new(10, 1);
        let text = "é🎮z".as_bytes();
        for byte in text {
            screen.feed(&[*byte]);
        }
        assert_eq!(screen.text(), "é🎮z");
    }

    #[test]
    fn a_resize_keeps_what_still_fits_and_new_writes_use_the_new_size() {
        let mut screen = shown(b"\x1b[1;1Habcdefgh\x1b[5;1Hlast");
        screen.resize(4, 3);
        assert_eq!(screen.text(), "abcd");
        assert_eq!(screen.cursor(), (3, 2));
        screen.feed(b"\x1b[3;1Hrow3");
        assert_eq!(screen.text(), "abcd\n\nrow3");
        screen.resize(30, 6);
        screen.feed(b"\x1b[6;25Hwide");
        assert_eq!(
            screen.text(),
            "abcd\n\nrow3\n\n\n                        wide"
        );
    }
}
