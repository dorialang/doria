use super::*;

pub(super) fn has_source_payload(class: &ClassDecl) -> bool {
    class.name == crate::compiler_known_concurrency::CANCELLATION_SOURCE
        && class.span.source == crate::compiler_known_concurrency::SOURCE_ID
}

pub(super) fn emit_token_class(class: &ClassDecl, output: &mut String, indent: usize) -> bool {
    if class.name != crate::compiler_known_concurrency::CANCELLATION_TOKEN
        || class.span.source != crate::compiler_known_concurrency::SOURCE_ID
    {
        return false;
    }
    writeln(
        output,
        indent,
        &format!(
            "final class {} implements __DoriaManagedCopy, __DoriaValueEquatable",
            php_type_symbol(&class.name)
        ),
    );
    for line in r#"{
    private function __construct(private readonly FFI\CData $state) {}
    public static function __doriaFromState(FFI\CData $state): self { return new self($state); }
    public function __doriaCopy(): self
    {
        $retained = __DoriaNativeIo::get()->dr_v1_cancellation_retain($this->state);
        if ($retained === null || FFI::isNull($retained)) { __doria_panic_at_current_site('P1503'); }
        return $this;
    }
    public function __doriaRelease(): void { __DoriaNativeIo::get()->dr_v1_cancellation_release($this->state); }
    public function __doriaEquals(mixed $other): bool
    {
        // FFI pointer equality compares state addresses, not PHP carrier identity.
        return $other instanceof self && $other->state == $this->state;
    }
    public function __doriaCancellationState(): FFI\CData { return $this->state; }
}"#.lines() {
        writeln(output, indent, line);
    }
    true
}

pub(super) fn emit_source_payload(output: &mut String, indent: usize) {
    let token = php_type_symbol(crate::compiler_known_concurrency::CANCELLATION_TOKEN);
    writeln(
        output,
        indent,
        "private ?FFI\\CData $__doriaCancellationState = null;",
    );
    writeln(
        output,
        indent,
        &format!("private ?{token} $__doriaCancellationToken = null;"),
    );
}

pub(super) fn emit_source_drop(output: &mut String, indent: usize) {
    for line in [
        "if ($this->__doriaCancellationState !== null) {",
        "    $state = $this->__doriaCancellationState;",
        "    $this->__doriaCancellationState = null;",
        "    $this->__doriaCancellationToken = null;",
        "    __DoriaNativeIo::get()->dr_v1_cancellation_release($state);",
        "}",
    ] {
        writeln(output, indent, line);
    }
}

pub(super) fn emit_body(
    intrinsic: crate::compiler_known_concurrency::ConcurrencyFunction,
    output: &mut String,
    indent: usize,
) {
    use crate::compiler_known_concurrency::ConcurrencyFunction;
    match intrinsic {
        ConcurrencyFunction::SourceConstructor => {
            for line in [
                "$state = __DoriaNativeIo::get()->dr_v1_cancellation_new();",
                "if ($state === null || FFI::isNull($state)) { __doria_panic_at_current_site('P1601'); }",
                "$this->__doriaCancellationState = $state;",
            ] { writeln(output, indent, line); }
            writeln(
                output,
                indent,
                &format!(
                    "$this->__doriaCancellationToken = {}::__doriaFromState($state);",
                    php_type_symbol(crate::compiler_known_concurrency::CANCELLATION_TOKEN)
                ),
            );
        }
        ConcurrencyFunction::SourceToken => {
            writeln(
                output,
                indent,
                "return $this->__doriaCancellationToken->__doriaCopy();",
            );
        }
        ConcurrencyFunction::SourceRequested => {
            writeln(output, indent, "return __DoriaNativeIo::get()->dr_v1_cancellation_is_requested($this->__doriaCancellationState) !== 0;");
        }
        ConcurrencyFunction::SourceCancel => {
            for line in [
                "$runtime = __DoriaNativeIo::get();",
                "$reason = $runtime->new('uint8_t[1]');",
                "$code = $runtime->new('int64_t[1]');",
                "$hasCode = $runtime->new('uint8_t[1]');",
                "$status = $runtime->dr_v1_cancellation_request($this->__doriaCancellationState, $reason, $code, $hasCode);",
                "if ($status !== 0 && $status !== 1) { __DoriaNativeIo::fail('protocol'); }",
            ] { writeln(output, indent, line); }
            writeln(output, indent, &format!("if ($status === 1) {{ __doria_throw(new {}({}, $hasCode[0] === 0 ? null : $code[0]), 0, ''); }}",
                php_type_symbol(crate::compiler_known_concurrency::CANCELLATION_ERROR),
                emit_php_string_literal(crate::compiler_known_concurrency::CANCELLATION_NOTIFICATION_FAILURE_MESSAGE)));
        }
    }
}
