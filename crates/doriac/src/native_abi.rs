//! Shared helpers for the implementation-private native function ABI.

use crate::mir;

pub const STRING_FROM_UTF8: &str = "dr_v1_string_from_utf8";
pub const STRING_RETAIN: &str = "dr_v1_string_retain";
pub const STRING_RELEASE: &str = "dr_v1_string_release";
pub const STRING_CONCAT: &str = "dr_v1_string_concat";
pub const STRING_COMPARE: &str = "dr_v1_string_compare";
pub const STRING_DATA: &str = "dr_v1_string_data";
pub const STRING_LENGTH: &str = "dr_v1_string_length";
pub const STRING_WRITE_STDOUT: &str = "dr_v1_write_string_stdout";
pub const STRING_WRITE_STDERR: &str = "dr_v1_write_string_stderr";
pub const READ_STDIN_LINE: &str = "dr_v1_read_stdin_line";
pub const NULLABLE_STRING_EQUAL: &str = "dr_v1_nullable_string_equal";
pub const FORMAT_STRING: &str = "dr_v1_format_string";
pub const FORMAT_I64: &str = "dr_v1_format_i64";
pub const FORMAT_U64: &str = "dr_v1_format_u64";
pub const FORMAT_F32: &str = "dr_v1_format_f32";
pub const FORMAT_F64: &str = "dr_v1_format_f64";
pub const READ_FILE: &str = "dr_v1_read_file";
pub const WRITE_FILE: &str = "dr_v1_write_file";
pub const STRING_FROM_I64: &str = "dr_v1_string_from_i64";
pub const STRING_FROM_U64: &str = "dr_v1_string_from_u64";
pub const STRING_FROM_F32: &str = "dr_v1_string_from_f32";
pub const STRING_FROM_F64: &str = "dr_v1_string_from_f64";
pub const STRING_FROM_BOOL: &str = "dr_v1_string_from_bool";
pub const CLASS_ALLOCATE: &str = "dr_v1_class_allocate";
pub const CLASS_FREE: &str = "dr_v1_class_free";

pub fn function_symbol(function: &mir::Function) -> String {
    let sanitized = function
        .name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("__doria_fn_{}_{}", function.id.0, sanitized)
}
