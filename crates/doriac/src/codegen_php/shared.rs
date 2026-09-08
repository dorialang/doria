//! Explicit PHP control blocks for Doria's existing shared-ownership families.

use super::*;
use crate::types::SharedHandleKind;

pub(super) fn interface_payload(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::SharedHandle(_, payload) => matches!(
            payload.as_ref(),
            ResolvedType::Interface(_) | ResolvedType::Error
        ),
        ResolvedType::Nullable(inner) => interface_payload(inner),
        _ => false,
    }
}

pub(super) fn kind(ty: &ResolvedType) -> Option<SharedHandleKind> {
    match ty {
        ResolvedType::SharedHandle(kind, _) => Some(*kind),
        ResolvedType::Nullable(inner) => kind(inner),
        _ => None,
    }
}

pub(super) fn emit_method(
    object: &Expr,
    method: &str,
    args: &[Argument],
    null_safe: bool,
    span: Span,
    scopes: &PhpNameScopes,
) -> Option<String> {
    let receiver = scopes.expression_types.get(&object.span()).and_then(kind)?;
    let operation = match (receiver, method) {
        (
            SharedHandleKind::SharedReference | SharedHandleKind::WritableSharedReference,
            "share",
        ) => Some("share"),
        (
            SharedHandleKind::SharedReference | SharedHandleKind::WritableSharedReference,
            "createWeakReference",
        ) => Some("weak"),
        (SharedHandleKind::WeakReference | SharedHandleKind::WritableWeakReference, "acquire") => {
            Some("acquire")
        }
        (
            SharedHandleKind::WritableSharedReference,
            "acquireReadonlyAccess" | "acquireWritableAccess",
        ) => Some("access"),
        _ => None,
    };
    let operator = if null_safe { "?->" } else { "->" };
    let object = emit_member_receiver(object, scopes);
    Some(if let Some(operation) = operation {
        let writable = if operation == "access" {
            if method == "acquireWritableAccess" {
                "true, "
            } else {
                "false, "
            }
        } else {
            ""
        };
        format!(
            "{object}{operator}{operation}({writable}{}, {}, {})",
            php_source_location(span, span.start),
            php_source_location(span, span.end),
            scopes.callable_identity()
        )
    } else {
        let method = scopes
            .specialization
            .call_symbol(span, &scopes.substitutions)
            .unwrap_or(method);
        format!(
            "{object}{operator}payload()->{method}({})",
            emit_arguments_for_call(args, span, scopes)
        )
    })
}

pub(super) const RUNTIME: &str = r#"
function __doria_shared_retain(int &$count, string $code, int $start, int $end, string $callable): void
{
    // Store an unsigned machine word in PHP's signed integer bit pattern.
    if ($count === -1) { __doria_panic($code, $start, $end, null, $callable); }
    $count = $count === PHP_INT_MAX ? PHP_INT_MIN : $count + 1;
}

function __doria_shared_release_count(int &$count): void
{
    if ($count === 0) { throw new LogicException("compiler invariant violated: shared count underflow"); }
    $count = $count === PHP_INT_MIN ? PHP_INT_MAX : $count - 1;
}

final class __DoriaSharedControl
{
    public int $strong = 1;
    public int $weak = 0;
    public int $readers = 0;
    public bool $writer = false;

    public function __construct(public ?object $payload) {}

    public function release(): void
    {
        __doria_shared_release_count($this->strong);
        if ($this->strong !== 0) { return; }
        if ($this->readers !== 0 || $this->writer) {
            throw new LogicException("compiler invariant violated: shared payload dropped with an active lease");
        }
        $payload = $this->payload;
        $this->payload = null;
        __doria_drop_value($payload);
    }
}

final class __DoriaSharedHandle
{
    private bool $live = true;

    // Kinds: readonly owner/weak, writable owner/weak, readonly/writable access.
    public function __construct(private __DoriaSharedControl $control, private int $kind) {}

    private function checked(): __DoriaSharedControl
    {
        if (!$this->live) { throw new LogicException("compiler invariant violated: released shared handle used"); }
        return $this->control;
    }

    public function payload(): object
    {
        $control = $this->checked();
        if ($control->payload === null
            || !in_array($this->kind, [0, 4, 5], true)
            || ($this->kind === 4 && $control->readers === 0)
            || ($this->kind === 5 && !$control->writer)) {
            throw new LogicException("compiler invariant violated: shared payload used without access");
        }
        return $control->payload;
    }

    public function share(int $start, int $end, string $callable): self
    {
        $control = $this->checked();
        __doria_shared_retain($control->strong, "P1503", $start, $end, $callable);
        return new self($control, $this->kind);
    }

    public function weak(int $start, int $end, string $callable): self
    {
        $control = $this->checked();
        __doria_shared_retain($control->weak, "P1504", $start, $end, $callable);
        return new self($control, $this->kind === 0 ? 1 : 3);
    }

    public function acquire(int $start, int $end, string $callable): ?self
    {
        $control = $this->checked();
        if ($control->strong === 0) { return null; }
        __doria_shared_retain($control->strong, "P1503", $start, $end, $callable);
        return new self($control, $this->kind === 1 ? 0 : 2);
    }

    public function access(bool $writable, int $start, int $end, string $callable): self
    {
        $control = $this->checked();
        if ($writable && $control->readers !== 0) {
            __doria_panic("P1501", $start, $end, null, $callable, ["conflictReason" => "Cannot Acquire Writable Access While Readonly Access Is Active"]);
        }
        if ($control->writer) {
            __doria_panic("P1501", $start, $end, null, $callable, ["conflictReason" => $writable
                ? "Cannot Acquire Writable Access While Writable Access Is Active"
                : "Cannot Acquire Readonly Access While Writable Access Is Active"]);
        }
        __doria_shared_retain($control->strong, "P1503", $start, $end, $callable);
        if ($writable) { $control->writer = true; }
        else { __doria_shared_retain($control->readers, "P1505", $start, $end, $callable); }
        return new self($control, $writable ? 5 : 4);
    }

    public function drop(): void
    {
        if (!$this->live) { return; }
        $this->live = false;
        if ($this->kind === 1 || $this->kind === 3) {
            __doria_shared_release_count($this->control->weak);
            return;
        }
        if ($this->kind === 4) { __doria_shared_release_count($this->control->readers); }
        if ($this->kind === 5) { $this->control->writer = false; }
        $this->control->release();
    }

    public function __destruct()
    {
        global $__doria_panicking;
        if (!$__doria_panicking) { $this->drop(); }
    }
}
"#;
