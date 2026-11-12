//! Float text in PHP output: canonical display and `%f`, each at the width
//! of the float it prints.

use super::*;
use crate::format_string::FormatSpec;

pub(super) const RUNTIME: &str = include_str!("float_text_runtime.php.inc");

/// Canonical display of an emitted float expression of type `ty`.
pub(super) fn display(value: &str, ty: FloatType) -> String {
    format!("__doria_float_text({value}, {})", ty.bit_width())
}

/// `%f` of an emitted float argument, with the precision, width and flags of
/// its specification. The result is complete text, so the PHP format prints
/// it with a bare `%s`.
pub(super) fn fixed(value: &str, spec: &FormatSpec) -> String {
    format!(
        "__doria_float_fixed({value}, {}, {}, {}, {})",
        spec.precision.unwrap_or(6),
        spec.width.unwrap_or(0),
        spec.left_align,
        spec.zero_pad,
    )
}
