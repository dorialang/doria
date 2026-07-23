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
pub const APPEND_FILE: &str = "dr_v1_append_file";
pub const BYTES_COPY: &str = "dr_v1_bytes_copy";
pub const BYTES_FREE: &str = "dr_v1_bytes_free";
pub const BYTES_LENGTH: &str = "dr_v1_bytes_length";
pub const BYTES_GET: &str = "dr_v1_bytes_get";
pub const BYTES_SET: &str = "dr_v1_bytes_set";
pub const BYTES_EQUAL: &str = "dr_v1_bytes_equal";
pub const BYTES_FROM_COLLECTION: &str = "dr_v1_bytes_from_collection";
pub const BYTES_TO_COLLECTION: &str = "dr_v1_bytes_to_collection";
pub const READ_FILE_BYTES: &str = "dr_v1_read_file_bytes";
pub const WRITE_FILE_BYTES: &str = "dr_v1_write_file_bytes";
pub const APPEND_FILE_BYTES: &str = "dr_v1_append_file_bytes";
pub const READ_STDIN_BYTES: &str = "dr_v1_read_stdin_bytes";
pub const WRITE_STDOUT_BYTES: &str = "dr_v1_write_stdout_bytes";
pub const WRITE_STDERR_BYTES: &str = "dr_v1_write_stderr_bytes";
pub const STRING_FROM_I64: &str = "dr_v1_string_from_i64";
pub const STRING_FROM_U64: &str = "dr_v1_string_from_u64";
pub const STRING_FROM_F32: &str = "dr_v1_string_from_f32";
pub const STRING_FROM_F64: &str = "dr_v1_string_from_f64";
pub const STRING_FROM_BOOL: &str = "dr_v1_string_from_bool";
pub const CLASS_ALLOCATE: &str = "dr_v1_class_allocate";
pub const CLASS_FREE: &str = "dr_v1_class_free";
pub const COLLECTION_NEW: &str = "dr_v1_collection_new";
pub const COLLECTION_FREE: &str = "dr_v1_collection_free";
pub const COLLECTION_LENGTH: &str = "dr_v1_collection_length";
pub const COLLECTION_PUSH: &str = "dr_v1_collection_push";
pub const COLLECTION_INSERT_AT: &str = "dr_v1_collection_insert_at";
pub const COLLECTION_REMOVE_AT: &str = "dr_v1_collection_remove_at";
pub const COLLECTION_POP: &str = "dr_v1_collection_pop";
pub const COLLECTION_PUSH_UNIQUE: &str = "dr_v1_collection_push_unique";
pub const COLLECTION_REMOVE_VALUE: &str = "dr_v1_collection_remove_value";
pub const COLLECTION_SET_ALGEBRA: &str = "dr_v1_collection_set_algebra";
pub const COLLECTION_VALUE_AT: &str = "dr_v1_collection_value_at";
pub const COLLECTION_KEY_AT: &str = "dr_v1_collection_key_at";
pub const COLLECTION_SET_AT: &str = "dr_v1_collection_set_at";
pub const COLLECTION_KEYED_GET: &str = "dr_v1_collection_keyed_get";
pub const COLLECTION_KEYED_SET: &str = "dr_v1_collection_keyed_set";
pub const COLLECTION_KEYED_HAS: &str = "dr_v1_collection_keyed_has";
pub const COLLECTION_KEYED_REMOVE: &str = "dr_v1_collection_keyed_remove";
pub const COLLECTION_NULLABLE_ACCESS: &str = "dr_v1_collection_nullable_access";
pub const COLLECTION_CONTAINS: &str = "dr_v1_collection_contains";

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
