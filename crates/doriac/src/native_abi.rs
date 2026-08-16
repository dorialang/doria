//! Shared helpers for the implementation-private native function ABI.

use crate::mir;

pub const STRING_FROM_UTF8: &str = "dr_v1_string_from_utf8";
pub const PROCESS_EXIT: &str = "dr_v1_exit_process";
pub const STRING_RETAIN: &str = "dr_v1_string_retain";
pub const STRING_RELEASE: &str = "dr_v1_string_release";
pub const STRING_CONCAT: &str = "dr_v1_string_concat";
pub const STRING_COMPARE: &str = "dr_v1_string_compare";
pub const STRING_DATA: &str = "dr_v1_string_data";
pub const STRING_LENGTH: &str = "dr_v1_string_length";
pub const STRING_BYTE_LENGTH: &str = "dr_v1_string_byte_length";
pub const STRING_GRAPHEME_LENGTH: &str = "dr_v1_string_grapheme_length";
pub const STRING_IS_EMPTY: &str = "dr_v1_string_is_empty";
pub const STRING_TO_BYTES: &str = "dr_v1_string_to_bytes";
pub const STRING_TRIM: &str = "dr_v2_string_trim";
pub const STRING_TRIM_START: &str = "dr_v2_string_trim_start";
pub const STRING_TRIM_END: &str = "dr_v2_string_trim_end";
pub const STRING_LOWER: &str = "dr_v2_string_lower";
pub const STRING_UPPER: &str = "dr_v2_string_upper";
pub const STRING_LOWER_FIRST: &str = "dr_v2_string_lower_first";
pub const STRING_UPPER_FIRST: &str = "dr_v2_string_upper_first";
pub const STRING_CONTAINS: &str = "dr_v1_string_contains";
pub const STRING_STARTS_WITH: &str = "dr_v1_string_starts_with";
pub const STRING_ENDS_WITH: &str = "dr_v1_string_ends_with";
pub const STRING_CONTAINS_IGNORE_CASE: &str = "dr_v2_string_contains_ignore_case";
pub const STRING_STARTS_WITH_IGNORE_CASE: &str = "dr_v2_string_starts_with_ignore_case";
pub const STRING_ENDS_WITH_IGNORE_CASE: &str = "dr_v2_string_ends_with_ignore_case";
pub const STRING_EQUALS_IGNORE_CASE: &str = "dr_v2_string_equals_ignore_case";
pub const STRING_INDEX_OF: &str = "dr_v1_string_index_of";
pub const STRING_LAST_INDEX_OF: &str = "dr_v1_string_last_index_of";
pub const STRING_INDEX_OF_IGNORE_CASE: &str = "dr_v2_string_index_of_ignore_case";
pub const STRING_LAST_INDEX_OF_IGNORE_CASE: &str = "dr_v2_string_last_index_of_ignore_case";
pub const STRING_COUNT_OCCURRENCES: &str = "dr_v2_string_count_occurrences";
pub const STRING_REPLACE: &str = "dr_v2_string_replace";
pub const STRING_SPLIT: &str = "dr_v2_string_split";
pub const STRING_JOIN: &str = "dr_v2_string_join";
pub const STRING_SLICE: &str = "dr_v2_string_slice";
pub const STRING_REPEAT: &str = "dr_v2_string_repeat";
pub const STRING_PAD_START: &str = "dr_v2_string_pad_start";
pub const STRING_PAD_END: &str = "dr_v2_string_pad_end";
pub const STRING_FROM_BYTES: &str = "dr_v1_string_from_bytes";
pub const STRING_WRITE_STDOUT: &str = "dr_v2_write_string_stdout";
pub const STRING_WRITE_STDERR: &str = "dr_v2_write_string_stderr";
pub const READ_STDIN_LINE: &str = "dr_v2_read_stdin_line";
/// Writes a prompt, flushes stdout, then reads one line. The single runtime
/// operation behind `read_line(string $prompt = ""): ?string`.
pub const READ_STDIN_LINE_PROMPTED: &str = "dr_v2_read_stdin_line_prompted";
pub const INT_PARSE: &str = "dr_v1_int_parse";
pub const FLOAT_PARSE: &str = "dr_v1_float_parse";
pub const NULLABLE_STRING_EQUAL: &str = "dr_v1_nullable_string_equal";
pub const FORMAT_STRING: &str = "dr_v1_format_string";
pub const FORMAT_I64: &str = "dr_v1_format_i64";
pub const FORMAT_U64: &str = "dr_v1_format_u64";
pub const FORMAT_F32: &str = "dr_v1_format_f32";
pub const FORMAT_F64: &str = "dr_v1_format_f64";
pub const READ_FILE: &str = "dr_v2_read_file";
pub const WRITE_FILE: &str = "dr_v2_write_file";
pub const APPEND_FILE: &str = "dr_v2_append_file";
pub const BYTES_COPY: &str = "dr_v1_bytes_copy";
pub const BYTES_FREE: &str = "dr_v1_bytes_free";
pub const BYTES_LENGTH: &str = "dr_v1_bytes_length";
pub const BYTES_GET: &str = "dr_v2_bytes_get";
pub const BYTES_SET: &str = "dr_v2_bytes_set";
pub const BYTES_EQUAL: &str = "dr_v1_bytes_equal";
pub const BYTES_FROM_COLLECTION: &str = "dr_v1_bytes_from_collection";
pub const BYTES_TO_COLLECTION: &str = "dr_v1_bytes_to_collection";
pub const READ_FILE_BYTES: &str = "dr_v2_read_file_bytes";
pub const WRITE_FILE_BYTES: &str = "dr_v2_write_file_bytes";
pub const APPEND_FILE_BYTES: &str = "dr_v2_append_file_bytes";
pub const READ_STDIN_BYTES: &str = "dr_v2_read_stdin_bytes";
pub const WRITE_STDOUT_BYTES: &str = "dr_v2_write_stdout_bytes";
pub const WRITE_STDERR_BYTES: &str = "dr_v2_write_stderr_bytes";
pub const STRING_FROM_I64: &str = "dr_v1_string_from_i64";
pub const STRING_FROM_U64: &str = "dr_v1_string_from_u64";
pub const STRING_FROM_F32: &str = "dr_v1_string_from_f32";
pub const STRING_FROM_F64: &str = "dr_v1_string_from_f64";
pub const STRING_FROM_BOOL: &str = "dr_v1_string_from_bool";
pub const CLASS_ALLOCATE: &str = "dr_v2_class_allocate";
pub const CLASS_FREE: &str = "dr_v1_class_free";
pub const SHARED_CREATE: &str = "dr_v2_shared_create";
pub const SHARED_RETAIN: &str = "dr_v2_shared_retain";
pub const SHARED_RELEASE: &str = "dr_v2_shared_release";
pub const SHARED_CREATE_WEAK: &str = "dr_v2_shared_create_weak";
pub const SHARED_RELEASE_WEAK: &str = "dr_v1_shared_release_weak";
pub const SHARED_ACQUIRE: &str = "dr_v2_shared_acquire";
pub const SHARED_PAYLOAD: &str = "dr_v1_shared_payload";
pub const WRITABLE_SHARED_CREATE: &str = "dr_v2_writable_shared_create";
pub const WRITABLE_SHARED_RETAIN: &str = "dr_v2_writable_shared_retain";
pub const WRITABLE_SHARED_RELEASE: &str = "dr_v2_writable_shared_release";
pub const WRITABLE_SHARED_CREATE_WEAK: &str = "dr_v2_writable_shared_create_weak";
pub const WRITABLE_SHARED_RELEASE_WEAK: &str = "dr_v1_writable_shared_release_weak";
pub const WRITABLE_SHARED_ACQUIRE: &str = "dr_v2_writable_shared_acquire";
pub const WRITABLE_SHARED_ACQUIRE_READONLY_ACCESS: &str =
    "dr_v2_writable_shared_acquire_readonly_access";
pub const WRITABLE_SHARED_ACQUIRE_WRITABLE_ACCESS: &str =
    "dr_v2_writable_shared_acquire_writable_access";
pub const WRITABLE_SHARED_RELEASE_READONLY_ACCESS: &str =
    "dr_v2_writable_shared_release_readonly_access";
pub const WRITABLE_SHARED_RELEASE_WRITABLE_ACCESS: &str =
    "dr_v2_writable_shared_release_writable_access";
pub const WRITABLE_SHARED_READONLY_PAYLOAD: &str = "dr_v1_writable_shared_readonly_payload";
pub const WRITABLE_SHARED_WRITABLE_PAYLOAD: &str = "dr_v1_writable_shared_writable_payload";
pub const MIXED_NEW: &str = "dr_v1_mixed_new";
pub const MIXED_NEW_BORROWED: &str = "dr_v1_mixed_new_borrowed";
pub const MIXED_NEW_AGGREGATE: &str = "dr_v2_mixed_new_aggregate";
pub const MIXED_CLONE_OWNED: &str = "dr_v1_mixed_clone_owned";
pub const MIXED_RELEASE_OWNED: &str = "dr_v1_mixed_release_owned";
pub const MIXED_FREE: &str = "dr_v1_mixed_free";
pub const MIXED_TAG: &str = "dr_v1_mixed_tag";
pub const MIXED_TYPE_ID: &str = "dr_v1_mixed_type_id";
pub const MIXED_PAYLOAD: &str = "dr_v1_mixed_payload";
pub const COLLECTION_NEW: &str = "dr_v1_collection_new";
pub const COLLECTION_STAGE26_NEW: &str = "dr_v2_collection_new";
pub const COLLECTION_AGGREGATE_NEW: &str = "dr_v4_collection_new_aggregate";
pub const COLLECTION_AGGREGATE_VALUE_AT: &str = "dr_v4_collection_aggregate_value_at";
pub const COLLECTION_AGGREGATE_PUSH_SLOT: &str = "dr_v4_collection_aggregate_push_slot";
pub const COLLECTION_AGGREGATE_PUSH_FRONT_SLOT: &str = "dr_v4_collection_aggregate_push_front_slot";
pub const COLLECTION_AGGREGATE_INSERT_SLOT: &str = "dr_v4_collection_aggregate_insert_slot";
pub const COLLECTION_AGGREGATE_REMOVE_AT_INTO: &str = "dr_v4_collection_aggregate_remove_at_into";
pub const COLLECTION_AGGREGATE_POP_INTO: &str = "dr_v4_collection_aggregate_pop_into";
pub const COLLECTION_AGGREGATE_KEYED_SET_SLOT: &str = "dr_v4_collection_aggregate_keyed_set_slot";
pub const COLLECTION_AGGREGATE_KEYED_REMOVE_INTO: &str =
    "dr_v4_collection_aggregate_keyed_remove_into";
pub const COLLECTION_AGGREGATE_NULLABLE_ACCESS_INTO: &str =
    "dr_v4_collection_aggregate_nullable_access_into";

pub const fn nullable_collection_access_code(
    access: crate::mir::NullableCollectionAccess,
) -> Option<u8> {
    match access {
        crate::mir::NullableCollectionAccess::Get => Some(0),
        crate::mir::NullableCollectionAccess::Index => None,
        crate::mir::NullableCollectionAccess::Remove => Some(1),
        crate::mir::NullableCollectionAccess::First => Some(2),
        crate::mir::NullableCollectionAccess::Last => Some(3),
        crate::mir::NullableCollectionAccess::Pop => Some(4),
        crate::mir::NullableCollectionAccess::PopFront => Some(5),
        crate::mir::NullableCollectionAccess::PopBack => Some(6),
        crate::mir::NullableCollectionAccess::At => Some(7),
    }
}
pub const COLLECTION_STAGE26_FINALIZE: &str = "dr_v2_collection_finalize";
pub const COLLECTION_STAGE26_FROM_COPY: &str = "dr_v2_collection_from_copy";
pub const COLLECTION_FILL_WORD: &str = "dr_v2_collection_fill_word";
pub const COLLECTION_FILL_STRING: &str = "dr_v2_collection_fill_string";
pub const COLLECTION_FREE: &str = "dr_v1_collection_free";
pub const COLLECTION_RESET_AFTER_CLEANUP: &str = "dr_v2_collection_reset_after_cleanup";
pub const COLLECTION_DETACH_FOR_CLEANUP: &str = "dr_v3_collection_detach_for_cleanup";
pub const COLLECTION_FINISH_DETACHED_CLEANUP: &str = "dr_v3_collection_finish_detached_cleanup";
pub const COLLECTION_LENGTH: &str = "dr_v1_collection_length";
pub const COLLECTION_PUSH: &str = "dr_v1_collection_push";
pub const COLLECTION_PUSH_NULLABLE: &str = "dr_v2_collection_push_nullable";
pub const COLLECTION_PUSH_FRONT: &str = "dr_v2_collection_push_front";
pub const COLLECTION_PUSH_FRONT_NULLABLE: &str = "dr_v2_collection_push_front_nullable";
pub const COLLECTION_INSERT_AT: &str = "dr_v2_collection_insert_at";
pub const COLLECTION_INSERT_AT_NULLABLE: &str = "dr_v2_collection_insert_at_nullable";
pub const COLLECTION_REMOVE_AT: &str = "dr_v2_collection_remove_at";
pub const COLLECTION_POP: &str = "dr_v1_collection_pop";
pub const COLLECTION_PUSH_UNIQUE: &str = "dr_v2_collection_push_unique";
pub const COLLECTION_REMOVE_VALUE: &str = "dr_v2_collection_remove_value";
pub const COLLECTION_INDEX_OF: &str = "dr_v2_collection_index_of";
pub const COLLECTION_SET_ALGEBRA: &str = "dr_v1_collection_set_algebra";
pub const COLLECTION_VALUE_AT: &str = "dr_v2_collection_value_at";
pub const COLLECTION_KEY_AT: &str = "dr_v2_collection_key_at";
pub const COLLECTION_SET_AT: &str = "dr_v2_collection_set_at";
pub const COLLECTION_SET_AT_NULLABLE: &str = "dr_v2_collection_set_at_nullable";
pub const COLLECTION_KEYED_GET: &str = "dr_v1_collection_keyed_get";
pub const COLLECTION_KEYED_GET_NULLABLE: &str = "dr_v2_collection_keyed_get_nullable";
pub const COLLECTION_KEYED_SET: &str = "dr_v1_collection_keyed_set";
pub const COLLECTION_KEYED_SET_NULLABLE: &str = "dr_v2_collection_keyed_set_nullable";
pub const COLLECTION_KEYED_HAS: &str = "dr_v1_collection_keyed_has";
pub const COLLECTION_KEYED_REMOVE: &str = "dr_v1_collection_keyed_remove";
pub const COLLECTION_NULLABLE_ACCESS: &str = "dr_v1_collection_nullable_access";
pub const COLLECTION_CONTAINS: &str = "dr_v2_collection_contains";

pub const COLLECTION_COMPARE_WORD: u8 = 0;
pub const COLLECTION_COMPARE_STRING: u8 = 1;
pub const COLLECTION_COMPARE_FLOAT32: u8 = 2;
pub const COLLECTION_COMPARE_FLOAT64: u8 = 3;
pub const COLLECTION_COMPARE_SIGNED_8: u8 = 4;
pub const COLLECTION_COMPARE_SIGNED_16: u8 = 5;
pub const COLLECTION_COMPARE_SIGNED_32: u8 = 6;
pub const COLLECTION_COMPARE_SIGNED_64: u8 = 7;
pub const COLLECTION_COMPARE_UNSIGNED_8: u8 = 8;
pub const COLLECTION_COMPARE_UNSIGNED_16: u8 = 9;
pub const COLLECTION_COMPARE_UNSIGNED_32: u8 = 10;
pub const COLLECTION_COMPARE_UNSIGNED_64: u8 = 11;
pub const COLLECTION_COMPARE_BOOL: u8 = 12;

pub const COLLECTION_KIND_SORTED_DICTIONARY: u8 = 1;
pub const COLLECTION_KIND_SORTED_SET: u8 = 2;
pub const COLLECTION_KIND_PRIORITY_QUEUE: u8 = 3;
pub const COLLECTION_KIND_DEQUE: u8 = 4;

pub const COLLECTION_LENGTH_FIELD: u32 = 0;
pub const COLLECTION_CAPACITY_FIELD: u32 = 1;
pub const COLLECTION_KEYS_FIELD: u32 = 2;
pub const COLLECTION_VALUES_FIELD: u32 = 3;
pub const COLLECTION_KEYED_FIELD: u32 = 4;
pub const COLLECTION_FIXED_FIELD: u32 = 5;
pub const COLLECTION_VALUE_WIDTH_FIELD: u32 = 6;
pub const COLLECTION_KIND_FIELD: u32 = 7;
pub const COLLECTION_HEAD_FIELD: u32 = 11;
pub const COLLECTION_INDEX_FIELD: u32 = 12;
pub const COLLECTION_VALUE_SIZE_FIELD: u32 = 16;
pub const COLLECTION_VALUE_STRIDE_FIELD: u32 = 17;
pub const COLLECTION_VALUE_ALIGNMENT_FIELD: u32 = 18;
pub const COLLECTION_AGGREGATE_FIELD: u32 = 19;

pub const fn collection_header_size(pointer_bytes: u32) -> u32 {
    let prefix = align_to(4 * pointer_bytes + 7, pointer_bytes);
    let legacy = align_to(prefix + 3 * pointer_bytes + 2, pointer_bytes);
    align_to(legacy + 3 * pointer_bytes + 1, pointer_bytes)
}

const fn align_to(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

pub const fn stage26_collection_kind(kind: mir::CollectionKind) -> Option<u8> {
    match kind {
        mir::CollectionKind::SortedDictionary => Some(COLLECTION_KIND_SORTED_DICTIONARY),
        mir::CollectionKind::SortedSet => Some(COLLECTION_KIND_SORTED_SET),
        mir::CollectionKind::PriorityQueue => Some(COLLECTION_KIND_PRIORITY_QUEUE),
        mir::CollectionKind::Deque => Some(COLLECTION_KIND_DEQUE),
        _ => None,
    }
}

pub const fn collection_comparator_code(comparator: mir::CollectionComparator) -> u8 {
    match comparator {
        mir::CollectionComparator::SignedInteger(8) => COLLECTION_COMPARE_SIGNED_8,
        mir::CollectionComparator::SignedInteger(16) => COLLECTION_COMPARE_SIGNED_16,
        mir::CollectionComparator::SignedInteger(32) => COLLECTION_COMPARE_SIGNED_32,
        mir::CollectionComparator::SignedInteger(64) => COLLECTION_COMPARE_SIGNED_64,
        mir::CollectionComparator::UnsignedInteger(8) => COLLECTION_COMPARE_UNSIGNED_8,
        mir::CollectionComparator::UnsignedInteger(16) => COLLECTION_COMPARE_UNSIGNED_16,
        mir::CollectionComparator::UnsignedInteger(32) => COLLECTION_COMPARE_UNSIGNED_32,
        mir::CollectionComparator::UnsignedInteger(64) => COLLECTION_COMPARE_UNSIGNED_64,
        mir::CollectionComparator::Bool => COLLECTION_COMPARE_BOOL,
        mir::CollectionComparator::StringBytes => COLLECTION_COMPARE_STRING,
        mir::CollectionComparator::SignedInteger(_)
        | mir::CollectionComparator::UnsignedInteger(_) => COLLECTION_COMPARE_WORD,
    }
}

pub const MIXED_TAG_BOOL: u8 = 1;
pub const MIXED_TAG_INT8: u8 = 2;
pub const MIXED_TAG_INT16: u8 = 3;
pub const MIXED_TAG_INT32: u8 = 4;
pub const MIXED_TAG_INT64: u8 = 5;
pub const MIXED_TAG_UINT8: u8 = 6;
pub const MIXED_TAG_UINT16: u8 = 7;
pub const MIXED_TAG_UINT32: u8 = 8;
pub const MIXED_TAG_UINT64: u8 = 9;
pub const MIXED_TAG_FLOAT32: u8 = 10;
pub const MIXED_TAG_FLOAT64: u8 = 11;
pub const MIXED_TAG_STRING: u8 = 12;
pub const MIXED_TAG_CLASS: u8 = 13;
pub const MIXED_TAG_ENUM: u8 = 14;
pub const MIXED_TAG_PAYLOAD_ENUM: u8 = 15;
pub const MIXED_TAG_ERROR: u8 = 16;

pub const fn collection_value_width(ty: mir::Type, pointer_width: u8) -> Option<u8> {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Bool) => Some(1),
        mir::Type::Scalar(mir::ScalarType::Integer(ty)) => Some(ty.storage_bytes() as u8),
        mir::Type::Scalar(mir::ScalarType::Float(crate::numeric::FloatType::Float32)) => Some(4),
        mir::Type::Scalar(mir::ScalarType::Float(crate::numeric::FloatType::Float64)) => Some(8),
        mir::Type::Scalar(mir::ScalarType::Enum(_)) => Some(4),
        mir::Type::String
        | mir::Type::Mixed
        | mir::Type::Class(_)
        | mir::Type::NullableString
        | mir::Type::NullableMixed
        | mir::Type::NullableClass(_)
        | mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_)
        | mir::Type::Collection(_)
        | mir::Type::NullableCollection(_) => Some(pointer_width),
        mir::Type::Error | mir::Type::NullableError => pointer_width.checked_mul(2),
        mir::Type::NullableScalar(_) => Some(16),
        mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_) => None,
    }
}

pub const fn nullable_payload_type(ty: mir::Type) -> Option<mir::Type> {
    match ty {
        mir::Type::NullableScalar(value) => Some(mir::Type::Scalar(value)),
        mir::Type::NullableString => Some(mir::Type::String),
        mir::Type::NullableMixed => Some(mir::Type::Mixed),
        mir::Type::NullableError => Some(mir::Type::Error),
        mir::Type::NullableClass(value) => Some(mir::Type::Class(value)),
        mir::Type::NullableSharedReference(value) => Some(mir::Type::SharedReference(value)),
        mir::Type::NullableWeakReference(value) => Some(mir::Type::WeakReference(value)),
        mir::Type::NullableWritableSharedReference(value) => {
            Some(mir::Type::WritableSharedReference(value))
        }
        mir::Type::NullableWritableWeakReference(value) => {
            Some(mir::Type::WritableWeakReference(value))
        }
        mir::Type::NullableReadonlySharedReferenceAccess(value) => {
            Some(mir::Type::ReadonlySharedReferenceAccess(value))
        }
        mir::Type::NullableWritableSharedReferenceAccess(value) => {
            Some(mir::Type::WritableSharedReferenceAccess(value))
        }
        mir::Type::NullableCollection(value) => Some(mir::Type::Collection(value)),
        _ => None,
    }
}

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

#[cfg(test)]
mod tests {
    use super::collection_header_size;

    #[test]
    fn collection_cleanup_scratch_matches_the_runtime_header() {
        assert_eq!(
            collection_header_size(size_of::<usize>() as u32) as usize,
            doria_rt::DR_COLLECTION_SIZE
        );
        assert_eq!(align_of::<usize>(), doria_rt::DR_COLLECTION_ALIGN);
    }
}
