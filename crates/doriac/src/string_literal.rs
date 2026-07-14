/// Finds the closing brace for an interpolation beginning at `open`.
///
/// This scanner only identifies the interpolation boundary. The contents are
/// still tokenized and parsed by the ordinary Doria lexer and expression parser.
pub fn interpolation_close(input: &str, open: usize) -> Option<usize> {
    if input.as_bytes().get(open) != Some(&b'{') {
        return None;
    }

    let bytes = input.as_bytes();
    let mut cursor = open + 1;
    let mut depth = 1_usize;
    let mut quote = None;

    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                cursor += 1;
                if cursor < bytes.len() {
                    cursor += input[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is on a UTF-8 boundary")
                        .len_utf8();
                }
                continue;
            }
            if byte == active_quote {
                quote = None;
                cursor += 1;
                continue;
            }
            cursor += input[cursor..]
                .chars()
                .next()
                .expect("cursor is on a UTF-8 boundary")
                .len_utf8();
            continue;
        }

        match byte {
            b'/' if bytes.get(cursor + 1) == Some(&b'/') => {
                cursor += 2;
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    cursor += 1;
                }
            }
            b'#' => {
                cursor += 1;
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    cursor += 1;
                }
            }
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                cursor += 2;
                while cursor + 1 < bytes.len()
                    && !(bytes[cursor] == b'*' && bytes[cursor + 1] == b'/')
                {
                    cursor += input[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is on a UTF-8 boundary")
                        .len_utf8();
                }
                if cursor + 1 >= bytes.len() {
                    return None;
                }
                cursor += 2;
            }
            b'\'' | b'"' => {
                quote = Some(byte);
                cursor += 1;
            }
            b'{' => {
                depth += 1;
                cursor += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
                cursor += 1;
            }
            _ => {
                cursor += input[cursor..]
                    .chars()
                    .next()
                    .expect("cursor is on a UTF-8 boundary")
                    .len_utf8();
            }
        }
    }

    None
}

pub fn decode_escape(character: char) -> Option<char> {
    match character {
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        '\\' => Some('\\'),
        '\'' => Some('\''),
        '"' => Some('"'),
        '{' => Some('{'),
        '}' => Some('}'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_balanced_interpolation_around_nested_strings_and_braces() {
        let text = r#"{sprintf("{%s}", "value") + grouped({1})} tail"#;
        assert_eq!(interpolation_close(text, 0), text.find("} tail"));
    }

    #[test]
    fn reports_unterminated_interpolation() {
        assert_eq!(interpolation_close("{a() + b()", 0), None);
    }

    #[test]
    fn ignores_braces_inside_ordinary_expression_comments() {
        for text in [
            "{left() /* } { */ + right()} tail",
            "{left() // }\n + right()} tail",
            "{left() # }\n + right()} tail",
        ] {
            assert_eq!(interpolation_close(text, 0), text.find("} tail"));
        }
    }
}
