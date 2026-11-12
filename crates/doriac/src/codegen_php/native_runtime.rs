//! Lazy, verified loading of the compiler-owned PHP runtime dependency.
use super::*;
use crate::runtime_artifact::SharedRuntimeDependency;
use crate::source::SourceFile;

pub(super) fn required(program: &Program) -> bool {
    let runtime_classes = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class)
                if runtime_declaration(class.span.source)
                    && (mir::RuntimeClassPayload::for_declaration(
                        &class.name,
                        class.span.source,
                    )
                    .is_some()
                        || crate::compiler_known_stream::is_pending_request(&class.name)
                        || crate::inline_value::InlineValueKind::for_name(&class.name)
                            .is_some_and(|kind| kind.requires_owned_cleanup())) =>
            {
                Some(class.name.as_str())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    term::used(program)
        || program.semantic_info.call_targets.values().any(|target| {
        matches!(target, crate::semantics::CallableTarget::Method { class_type, method_name, .. }
            if class_type.name == crate::compiler_known_time::DEADLINE && method_name == "fromNow")
    }) || program
        .semantic_info
        .expression_types
        .iter()
        .any(|(span, ty)| {
            !runtime_declaration(span.source) && uses_runtime_storage(ty, &runtime_classes)
        })
        || program
            .semantic_info
            .callable_signatures
            .iter()
            .any(|(span, signature)| {
                !runtime_declaration(span.source)
                    && signature
                        .parameters
                        .iter()
                        .map(|parameter| &parameter.r#type)
                        .chain(std::iter::once(&signature.return_type))
                        .any(|ty| uses_runtime_storage(ty, &runtime_classes))
            })
        || program
            .semantic_info
            .property_families
            .values()
            .any(|property| {
                !runtime_declaration(property.root_declaration.source)
                    && uses_runtime_storage(&property.ty, &runtime_classes)
            })
        // A standard enum carrying a runtime class (a lock outcome returning
        // the file) needs the runtime only when authored code holds that class,
        // which the expression and signature checks above already see.
        || program.semantic_info.enums.iter().any(|definition| {
            !runtime_declaration(definition.declaration.source)
                && definition
                    .cases
                    .iter()
                    .flat_map(|case| &case.payload)
                    .any(|payload| uses_runtime_storage(&payload.ty, &runtime_classes))
        })
}

fn runtime_declaration(source: crate::source::SourceId) -> bool {
    source == crate::compiler_known_stream::SOURCE_ID
        || source == crate::compiler_known_concurrency::SOURCE_ID
}

fn uses_runtime_storage(ty: &ResolvedType, runtime_classes: &HashSet<&str>) -> bool {
    match ty {
        ResolvedType::Class(class) => runtime_classes.contains(class.name.as_str()),
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner)
        | ResolvedType::SortedSet(inner)
        | ResolvedType::PriorityQueue(inner)
        | ResolvedType::Deque(inner)
        | ResolvedType::SharedHandle(_, inner) => uses_runtime_storage(inner, runtime_classes),
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            uses_runtime_storage(key, runtime_classes)
                || uses_runtime_storage(value, runtime_classes)
        }
        ResolvedType::Function(function) => {
            uses_runtime_storage(&function.return_type, runtime_classes)
                || function
                    .parameters
                    .iter()
                    .any(|parameter| uses_runtime_storage(&parameter.ty, runtime_classes))
        }
        _ => false,
    }
}

pub(super) fn emit(output: &mut String, program: &Program, runtime: &SharedRuntimeDependency) {
    let source = SourceFile::new(&program.source_path, &program.source_text);
    output.push_str("$__doria_native_failures = [\n");
    for (key, explanation) in [
        ("ffi", "This PHP deployment does not permit FFI. Native-backed Doria operations require the PHP FFI extension to be enabled by the operator."),
        ("missing", "The native runtime library or its identity sidecar is missing or unreadable."),
        ("identity", "The native runtime identity or bytes do not match this generated PHP program."),
        ("load", "PHP could not load the verified native runtime on this host. Its architecture and native dependencies must match the deployment."),
        ("layout", "The native storage layout cannot be represented safely by this PHP FFI deployment."),
        ("protocol", "The verified native runtime returned an invalid operation status."),
    ] {
        let diagnostic = Diagnostic::new("B3602", "PHP native runtime dependency is unavailable", Span::default())
            .with_explanation(explanation)
            .with_help("Deploy the generated PHP output together with its compiler-published .doria-runtime directory on a matching platform.");
        output.push_str(&format!("    {} => {},\n", emit_php_string_literal(key),
            emit_php_string_literal(&(diagnostic.render(&source) + "\n"))));
    }
    output.push_str("];\n");
    output.push_str(&format!(
        "$__doria_native_metadata = json_decode(hex2bin({}), true);\n",
        emit_php_string_literal(&hex_bytes(runtime.metadata_json().as_bytes()))
    ));
    let code = RUNTIME
        .replace(
            "__DORIA_LIBRARY_PATH__",
            &emit_php_string_literal(&runtime.relative_library_path().to_string_lossy()),
        )
        .replace(
            "__DORIA_METADATA_PATH__",
            &emit_php_string_literal(&runtime.relative_metadata_path().to_string_lossy()),
        );
    output.push_str(&code);
}

const RUNTIME: &str = r#"
final class __DoriaNativeIo
{
    private static ?FFI $runtime = null;

    // `$detail` says what the runtime answered when the failure is one of
    // its answers. A protocol failure also names the operation that saw it,
    // since the diagnostic alone cannot tell forty call sites apart.
    public static function fail(string $reason, string $detail = ''): void
    {
        global $__doria_native_failures;
        // The report belongs on the screen the user returns to.
        __doria_terminal_ending();
        // Leaving through `exit` unwinds the callers, and nothing they hold
        // may run a Doria drop against a runtime that has just failed.
        global $__doria_shutting_down;
        $__doria_shutting_down = true;
        fwrite(STDERR, $__doria_native_failures[$reason]);
        if ($reason === "protocol") {
            $caller = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 2)[1] ?? [];
            $operation = ($caller["class"] ?? "") === "" ? ($caller["function"] ?? "")
                : $caller["class"] . "::" . ($caller["function"] ?? "");
            if ($operation !== "") {
                fwrite(STDERR, "The operation that failed was " . $operation .
                    ($detail === "" ? "" : ": " . $detail) . ".\n");
            }
        }
        exit(1);
    }

    public static function get(): FFI
    {
        if (self::$runtime !== null) { return self::$runtime; }
        if (!class_exists(FFI::class)) { self::fail("ffi"); }
        global $__doria_native_metadata;
        $path = __DIR__ . DIRECTORY_SEPARATOR . __DORIA_LIBRARY_PATH__;
        $sidecar = __DIR__ . DIRECTORY_SEPARATOR . __DORIA_METADATA_PATH__;
        $document = @file_get_contents($sidecar);
        if ($document === false || !is_file($path) || !is_readable($path)) {
            self::fail("missing");
        }
        $metadata = json_decode($document, true);
        if (!is_array($metadata)) { self::fail("identity"); }
        foreach ($__doria_native_metadata as $field => $expected) {
            if (!array_key_exists($field, $metadata) || $metadata[$field] !== $expected) {
                self::fail("identity");
            }
        }
        if (@filesize($path) !== $metadata["bytes"] ||
            @hash_file("sha256", $path) !== $metadata["sha256"]) {
            self::fail("identity");
        }
        try {
            FFI::cdef("");
        } catch (Throwable) {
            self::fail("ffi");
        }
        try {
            $runtime = FFI::cdef(<<<'DORIA_NATIVE_ABI'
typedef struct {
    uint64_t value;
    int64_t system_code;
    size_t transferred_byte_count;
    uint8_t reason;
    uint8_t has_system_code;
    uint8_t has_transferred_byte_count;
} DrIoStatusV1;
typedef struct { uint64_t low; int64_t high; } DrDeadlineV1;
typedef struct { uint64_t kind; DrDeadlineV1 deadline; } DrWaitBudgetV1;
typedef struct { size_t count; uint8_t tag; uint8_t has_count; } DrAdapterOutcomeV1;
typedef struct {
    DrIoStatusV1 transfer, wait_failure, cancellation_failure;
    uint8_t stop, has_cancellation_failure, wait_operation, cancellation_operation;
} DrCompletionStatusV1;
typedef struct DrStringV1 DrStringV1;
typedef struct DrBytesV1 DrBytesV1;
typedef struct DrCancellationV1 DrCancellationV1;
DrCancellationV1 *dr_v1_cancellation_new(void);
DrCancellationV1 *dr_v1_cancellation_retain(DrCancellationV1 *state);
void dr_v1_cancellation_release(DrCancellationV1 *state);
uint8_t dr_v1_cancellation_is_requested(const DrCancellationV1 *state);
uint8_t dr_v1_cancellation_request(const DrCancellationV1 *state, uint8_t *reason,
    int64_t *system_code, uint8_t *has_system_code);
typedef struct { void *object; const void *vtable; } DrInterfaceCarrierV1;
const uint8_t *dr_v1_runtime_identity(size_t *length);
DrStringV1 *dr_v1_string_from_utf8(const char *bytes, size_t length);
const uint8_t *dr_v1_string_data(const DrStringV1 *string);
size_t dr_v1_string_length(const DrStringV1 *string);
void dr_v1_string_release(DrStringV1 *string);
size_t dr_v1_bytes_length(const DrBytesV1 *bytes);
const uint8_t *dr_v1_bytes_data(const DrBytesV1 *bytes);
void dr_v1_bytes_free(DrBytesV1 *bytes);
uint8_t dr_v4_checked_io(const void *frame, uint8_t operation, DrStringV1 *path_or_prompt,
    const char *contents, size_t length, void **result, DrStringV1 **message,
    DrStringV1 **path, int64_t *system_code, size_t *valid_byte_count,
    size_t *invalid_byte_count, size_t *transferred_byte_count, uint64_t *meta);
void dr_v1_file_object_layout(size_t *size, size_t *alignment);
const void *dr_v1_file_object_readiness_source(const void *file);
uint8_t dr_v1_file_object_open(DrStringV1 *path, uint8_t access,
    uint8_t creation, uint8_t append, void *file, DrIoStatusV1 *status);
uint8_t dr_v1_host_file_object_read(void *file, uint8_t *base, size_t capacity, size_t offset, size_t length, DrIoStatusV1 *status);
uint8_t dr_v1_host_file_object_write(void *file, const uint8_t *base, size_t capacity, size_t offset, size_t length, DrIoStatusV1 *status);
void dr_v1_host_byte_range_copy(uint8_t *destination, size_t destination_capacity, size_t destination_offset,
    const uint8_t *source, size_t source_capacity, size_t source_offset, size_t count);
uint8_t dr_v1_file_object_close(void *file, DrIoStatusV1 *status);
uint8_t dr_v1_file_object_control_int(void *file, uint8_t operation, int64_t offset, uint8_t origin, DrIoStatusV1 *status);
void dr_v1_file_object_drop(const void *frame, void *file);
DrStringV1 *dr_v1_file_object_path(const void *file);
typedef struct DrCollectionV1 DrCollectionV1;
DrCollectionV1 *dr_v1_collection_new(size_t length, uint8_t keyed, uint8_t fixed, uint8_t value_width);
void dr_v1_collection_push(DrCollectionV1 *collection, uint64_t value);
void dr_v1_collection_free(DrCollectionV1 *collection);
uint8_t dr_v1_checked_process(const void *frame, uint8_t operation, void *child,
    const int64_t *operands, size_t operand_count, int64_t *result, DrStringV1 **message,
    DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
void dr_v1_child_process_drop(const void *frame, void *child);
void dr_v1_file_object_lock_layout(size_t *size, size_t *alignment);
uint8_t dr_v1_file_object_try_lock(void *file, uint8_t mode, uint8_t ownership,
    void *guard, DrIoStatusV1 *status);
void *dr_v1_file_object_lock_file(void *guard);
DrStringV1 *dr_v1_file_object_lock_path(const void *guard);
uint8_t dr_v1_file_object_lock_release(void *guard, void **file, DrIoStatusV1 *status);
void *dr_v1_file_object_lock_drop(void *guard);
const void *dr_v1_standard_stream_readiness_source(uint8_t stream);
uint8_t dr_v1_host_standard_stream_io(const void *frame, uint8_t stream,
    uint8_t operation, uint8_t *base, size_t capacity, size_t offset, size_t length, uint64_t *result_tag,
    size_t *result_value, DrStringV1 **message, DrStringV1 **error_path,
    int64_t *system_code, size_t *transferred, uint64_t *meta);
void *dr_v1_readiness_set_new(const void *frame, DrIoStatusV1 *status);
void dr_v1_readiness_set_drop(const void *frame, void *set);
void dr_v1_transfer_key_layout(size_t *size, size_t *alignment);
uint8_t dr_v1_readiness_set_add_source(void *set, const void *source, void *key, DrIoStatusV1 *status);
uint8_t dr_v1_readiness_set_add_host_pending_source(void *set, const void *source,
    uint8_t readable, uint8_t writable, void *key, DrIoStatusV1 *status);
uint8_t dr_v1_readiness_set_remove_source(void *set, const void *key, DrIoStatusV1 *status);
uint8_t dr_v1_readiness_set_get_source(void *set, const void *key, DrIoStatusV1 *status);
uint8_t dr_v1_checked_pending_request_configure_file(const void *frame, void *request,
    void *file, const void *fresh_source, uint8_t *data, size_t length, DrInterfaceCarrierV1 *error,
    DrStringV1 **message, DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
void dr_v1_pending_request_complete(void *request, uint8_t tag, size_t count);
void dr_v1_host_adapter_error_layout(size_t *slot_size, size_t *slot_alignment,
    size_t *terminal_size, size_t *terminal_alignment);
void dr_v1_host_adapter_error_initialize(void *slot, size_t identity);
uint8_t dr_v1_host_adapter_error_claim(const void *slot, void *terminal,
    uint8_t has_count, size_t count, size_t bound);
uint8_t dr_v1_host_adapter_error_disposition(const void *slot);
uint8_t dr_v1_host_adapter_error_acknowledge(const void *slot);
size_t dr_v1_host_adapter_error_return(void *terminal);
void dr_v1_host_adapter_error_discard(void *terminal);
void dr_v1_host_pending_view_layout(size_t *size, size_t *alignment);
uint8_t dr_v1_host_pending_action(void *record, void *view, uint8_t **caller,
    size_t *length, uint8_t *reading);
uint8_t dr_v1_host_pending_source_reply(void *record, const void *source, void *error);
uint8_t dr_v1_host_pending_prepare_reply(void *view, void *error);
uint8_t dr_v1_host_pending_complete_reply(void *view, uint8_t tag, size_t count, void *error);
void dr_v1_transfer_outcome_layout(size_t *size, size_t *alignment);
void dr_v1_transfer_adapter_outcome_layout(size_t *size, size_t *alignment);
uint8_t dr_v1_wait_budget_init(uint8_t kind, uint64_t nanoseconds, DrWaitBudgetV1 *budget,
    uint8_t *reason, int64_t *system_code, uint8_t *has_system_code);
uint8_t dr_v1_wait_budget_init_deadline(uint64_t low, int64_t high, DrWaitBudgetV1 *budget,
    uint8_t *reason, int64_t *system_code, uint8_t *has_system_code);
void *dr_v1_host_transfer_plan_configure(const void *frame, void *set, void *plan,
    const DrWaitBudgetV1 *budget, DrIoStatusV1 *status);
void *dr_v2_host_transfer_plan_configure(const void *frame, void *set, void *plan,
    const DrDeadlineV1 *deadline, DrCancellationV1 *cancellation, DrIoStatusV1 *status);
uint8_t dr_v1_host_transfer_group_observe(void *owner, const DrWaitBudgetV1 *budget, DrIoStatusV1 *status);
uint8_t dr_v1_host_transfer_group_observe_wait(void *owner, uint8_t kind,
    uint64_t low, int64_t high, DrIoStatusV1 *status);
uint8_t dr_v1_host_transfer_group_resubmit(void *owner, const void *key, uint8_t reading,
    size_t offset, size_t length, DrIoStatusV1 *status);
uint8_t dr_v1_host_transfer_group_resubmit_watch(void *owner, const void *key, DrIoStatusV1 *status);
void dr_v1_host_transfer_group_close(void *owner);
uint8_t dr_v1_host_transfer_group_poll(const void *frame, void *owner, uint8_t *kind,
    size_t *index, DrAdapterOutcomeV1 *outcome, void *terminal, void *control);
uint8_t dr_v1_host_transfer_group_action(const void *owner, size_t *index, void **record,
    void *view, uint8_t **caller, size_t *length, uint8_t *reading);
uint8_t dr_v1_host_pending_request_configure_file(const void *frame, void *owner, void *view,
    void *file, const void *fresh_source, uint8_t *base, size_t capacity, size_t offset, size_t length, DrInterfaceCarrierV1 *error,
    DrStringV1 **message, DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
uint8_t dr_v1_host_pending_request_configure_standard(const void *frame, void *owner, void *view,
    uint8_t stream, const void *fresh_source, uint8_t *base, size_t capacity, size_t offset, size_t length, DrInterfaceCarrierV1 *error,
    DrStringV1 **message, DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
uint8_t dr_v1_host_transfer_group_read_region(const void *owner, const void *key,
    const uint8_t **data, size_t *length, DrIoStatusV1 *status);
uint8_t dr_v1_host_transfer_group_observed(const void *owner, uint8_t kind, void *key);
void dr_v1_host_transfer_group_drop(const void *frame, void *owner);
uint8_t dr_v1_checked_pending_request_outcome(const void *frame, const void *request,
    uint64_t *tag, size_t *count, DrStringV1 **message, DrStringV1 **path,
    int64_t *system_code, size_t *transferred, uint64_t *meta);
uint8_t dr_v1_host_transfer_plan_new(const void *frame, const void *key,
    uint8_t *base, size_t capacity, size_t offset, size_t length, uint8_t reading, void **result, DrStringV1 **message,
    DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
uint8_t dr_v1_host_transfer_plan_append(const void *frame, void *plan, const void *key,
    uint8_t *base, size_t capacity, size_t offset, size_t length, uint8_t reading, void **result, DrStringV1 **message,
    DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
uint8_t dr_v1_checked_transfer_plan_watch_new(const void *frame, uint64_t source, void **result,
    DrStringV1 **message, DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
uint8_t dr_v1_checked_transfer_plan_watch_append(const void *frame, void *plan, uint64_t source,
    void **result, DrStringV1 **message, DrStringV1 **path, int64_t *system_code, size_t *transferred,
    uint64_t *meta);
void dr_v1_transfer_plan_drop(void *plan);
uint8_t dr_v1_checked_transfer_plan_key(const void *frame, const void *plan,
    int64_t position, uint8_t kind, void *result, DrStringV1 **message,
    DrStringV1 **path, int64_t *system_code, size_t *transferred, uint64_t *meta);
uint8_t dr_v1_deadline_from_now(int64_t duration, uint64_t *low, int64_t *high,
    int64_t *system_code, uint8_t *has_system_code);
int64_t dr_v1_host_terminal_operate(uint32_t operation, int64_t first, int64_t second,
    const char *text, size_t length);
int64_t dr_v1_terminal_await_event(uint32_t generation, uint8_t wait_kind, uint64_t wait_low,
    int64_t wait_high, DrCancellationV1 *cancellation);
void dr_v1_terminal_process_ending(void);
DORIA_NATIVE_ABI, $path);
        } catch (Throwable) {
            // Configuration failures are backend diagnostics, never Doria IoError.
            self::fail("load");
        }
        $length = $runtime->new("size_t[1]");
        $identity = $runtime->dr_v1_runtime_identity($length);
        if ($identity === null || $length[0] < 1 || $length[0] > 65536) {
            self::fail("identity");
        }
        $loaded = json_decode(FFI::string($runtime->cast("char *", $identity), $length[0]), true);
        foreach (["abiVersion", "runtimeRevision", "targetTriple", "profile", "featureSet", "artifactKind"] as $field) {
            if (!is_array($loaded) || !array_key_exists($field, $loaded) ||
                $loaded[$field] !== $metadata[$field]) {
                self::fail("identity");
            }
        }
        $layout = $runtime->new("size_t[2]");
        $key = $runtime->new("int64_t[2]");
        $runtime->dr_v1_transfer_key_layout(FFI::addr($layout[0]), FFI::addr($layout[1]));
        if ($layout[0] !== FFI::sizeof($key) || $layout[1] !== FFI::alignof($key)) {
            self::fail("layout");
        }
        return self::$runtime = $runtime;
    }
}
"#;
