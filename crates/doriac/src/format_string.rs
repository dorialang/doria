use crate::diagnostics::Diagnostic;
use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatConversion {
    Display,
    Decimal,
    Float,
    HexLower,
    HexUpper,
    Octal,
    Binary,
}

impl FormatConversion {
    pub const fn specifier(self) -> &'static str {
        match self {
            Self::Display => "%s",
            Self::Decimal => "%d",
            Self::Float => "%f",
            Self::HexLower => "%x",
            Self::HexUpper => "%X",
            Self::Octal => "%o",
            Self::Binary => "%b",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatSpec {
    pub conversion: FormatConversion,
    pub width: Option<u32>,
    pub precision: Option<u32>,
    pub left_align: bool,
    pub zero_pad: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatPiece {
    Literal(String),
    Argument { index: u32, spec: FormatSpec },
}

pub fn parse(format: &str, span: Span) -> Result<Vec<FormatPiece>, Diagnostic> {
    let bytes = format.as_bytes();
    let mut pieces = Vec::new();
    let mut literal_start = 0;
    let mut cursor = 0;
    let mut argument_index = 0_u32;
    while cursor < bytes.len() {
        if bytes[cursor] != b'%' {
            cursor += 1;
            continue;
        }
        if literal_start < cursor {
            pieces.push(FormatPiece::Literal(
                format[literal_start..cursor].to_string(),
            ));
        }
        let specifier_start = cursor;
        cursor += 1;
        if cursor == bytes.len() {
            return Err(error(
                span,
                specifier_start,
                "trailing `%` in format string",
            ));
        }
        if bytes[cursor] == b'%' {
            pieces.push(FormatPiece::Literal("%".to_string()));
            cursor += 1;
            literal_start = cursor;
            continue;
        }

        let mut left_align = false;
        let mut zero_pad = false;
        loop {
            match bytes.get(cursor).copied() {
                Some(b'-') if !left_align => left_align = true,
                Some(b'0') if !zero_pad => zero_pad = true,
                Some(b'-' | b'0') => {
                    return Err(error(span, cursor, "duplicate format flag"));
                }
                _ => break,
            }
            cursor += 1;
        }

        let (width, next) = parse_number(bytes, cursor, span)?;
        cursor = next;
        let precision = if bytes.get(cursor) == Some(&b'.') {
            cursor += 1;
            let (precision, next) = parse_number(bytes, cursor, span)?;
            let Some(precision) = precision else {
                return Err(error(span, cursor, "format precision requires digits"));
            };
            cursor = next;
            Some(precision)
        } else {
            None
        };

        let conversion = match bytes.get(cursor).copied() {
            Some(b's') => FormatConversion::Display,
            Some(b'd') => FormatConversion::Decimal,
            Some(b'f') => FormatConversion::Float,
            Some(b'x') => FormatConversion::HexLower,
            Some(b'X') => FormatConversion::HexUpper,
            Some(b'o') => FormatConversion::Octal,
            Some(b'b') => FormatConversion::Binary,
            Some(other) => {
                return Err(error(
                    span,
                    cursor,
                    format!("unsupported format conversion `%{}`", other as char),
                ));
            }
            None => {
                return Err(error(
                    span,
                    specifier_start,
                    "trailing `%` in format string",
                ))
            }
        };
        if precision.is_some() && conversion != FormatConversion::Float {
            return Err(error(
                span,
                specifier_start,
                "format precision is supported only for `%f`",
            ));
        }
        if zero_pad && conversion == FormatConversion::Display {
            return Err(error(
                span,
                specifier_start,
                "zero padding is invalid for `%s`",
            ));
        }
        cursor += 1;
        pieces.push(FormatPiece::Argument {
            index: argument_index,
            spec: FormatSpec {
                conversion,
                width,
                precision,
                left_align,
                zero_pad: zero_pad && !left_align,
            },
        });
        argument_index = argument_index
            .checked_add(1)
            .ok_or_else(|| error(span, specifier_start, "too many format arguments"))?;
        literal_start = cursor;
    }
    if literal_start < bytes.len() {
        pieces.push(FormatPiece::Literal(format[literal_start..].to_string()));
    }
    Ok(pieces)
}

fn parse_number(
    bytes: &[u8],
    mut cursor: usize,
    span: Span,
) -> Result<(Option<u32>, usize), Diagnostic> {
    let start = cursor;
    let mut value = 0_u32;
    while let Some(digit @ b'0'..=b'9') = bytes.get(cursor).copied() {
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(digit - b'0')))
            .ok_or_else(|| error(span, start, "format width or precision exceeds `u32`"))?;
        cursor += 1;
    }
    Ok(((cursor != start).then_some(value), cursor))
}

fn error(span: Span, offset: usize, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        "E0455",
        format!("{} at format byte {offset}", message.into()),
        span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stage17_matrix_into_a_validated_plan() {
        let pieces =
            parse("%05d|%-6s|%.2f|%x|%X|%o|%b|%%", Span::default()).expect("matrix should parse");
        assert_eq!(
            pieces
                .iter()
                .filter(|piece| matches!(piece, FormatPiece::Argument { .. }))
                .count(),
            7
        );
        assert!(pieces.contains(&FormatPiece::Literal("%".to_string())));
    }

    #[test]
    fn rejects_every_deferred_or_malformed_shape() {
        for format in ["%", "%e", "%g", "%1$s", "%*d", "%.2d", "%00d", "%0s"] {
            assert!(parse(format, Span::default()).is_err(), "{format}");
        }
    }
}
