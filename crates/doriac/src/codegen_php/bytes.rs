//! Fixed-length owned byte storage for the PHP compatibility backend.
use super::*;

pub(super) fn uses(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Bytes => true,
        ResolvedType::Nullable(inner) => uses(inner),
        _ => false,
    }
}

pub(super) fn indexed(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::ByteRegion(_) => true,
        ResolvedType::Nullable(inner) => indexed(inner),
        _ => uses(ty),
    }
}

pub(super) fn region_method(ty: &ResolvedType, method: &str) -> bool {
    match ty {
        ResolvedType::Nullable(inner) => region_method(inner, method),
        ResolvedType::ByteRegion(crate::types::ByteRegionKind::Writable)
            if matches!(method, "copyFrom" | "copyWithin") =>
        {
            true
        }
        _ => crate::types::ByteRegionKind::method_result(ty, method).is_some(),
    }
}

fn indexed_place(expr: &Expr) -> Option<(&Expr, &Expr, Span)> {
    match expr {
        Expr::Grouped { expr, .. } => indexed_place(expr),
        Expr::Index {
            collection,
            index,
            span,
        } => Some((collection, index, *span)),
        _ => None,
    }
}

pub(super) fn supports_rmw(target: &Expr, context: &PhpValidation<'_>) -> bool {
    indexed_place(target)
        .is_some_and(|(source, _, _)| context.expression_type(source.span()).is_some_and(indexed))
}

pub(super) fn expression(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    match expr {
        Expr::Index {
            collection,
            index,
            span,
        } => {
            if let Some(region) = value_result::flat_region(collection, scopes) {
                let [storage, offset, length] = region.region_parts().unwrap();
                return Some(format!(
                    "__DoriaByteRegion::__doriaGet({storage}, {offset}, {length}, {}, {}, {}, {})",
                    emit_expr(index, scopes),
                    scopes.source_location(*span, span.start),
                    scopes.source_location(*span, span.end),
                    scopes.callable_identity()
                ));
            }
        }
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            ..
        } if property == "length" => {
            if let Some(region) = value_result::flat_region(object, scopes) {
                let [_, _, length] = region.region_parts().unwrap();
                return Some(if *null_safe {
                    format!("({} ? {length} : null)", region.present())
                } else {
                    length
                });
            }
        }
        _ => {}
    }
    let Expr::MethodCall {
        object,
        method,
        args,
        null_safe,
        span,
        ..
    } = expr
    else {
        return None;
    };
    let ty = scopes.expression_types.get(&object.span())?;
    if !region_method(ty, method) {
        return None;
    }
    Some(format!(
        "{}{}{method}({}, start: {}, end: {}, callable: {})",
        emit_member_receiver(object, scopes),
        if *null_safe { "?->" } else { "->" },
        emit_arguments_for_call(args, *span, scopes),
        scopes.source_location(*span, span.start),
        scopes.source_location(*span, span.end),
        scopes.callable_identity()
    ))
}

pub(super) fn emit_fields(
    expr: &Expr,
    destination: &value_result::Place,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let Expr::MethodCall {
        object,
        method,
        args,
        null_safe,
        span,
        ..
    } = expr
    else {
        return false;
    };
    let Some(ty) = scopes.expression_types.get(&object.span()).cloned() else {
        return false;
    };
    let Some(kind) = crate::types::ByteRegionKind::method_result(&ty, method) else {
        return false;
    };
    let region = value_result::region_argument(object, output, indent, scopes);
    let owner = if region.is_none() {
        let name = scopes.fresh_temp("__doria_region_owner");
        scopes.track_expression_temporary(PhpExpressionTemporary::Alias(name.clone()));
        let value = expression_region::value(object, output, indent, scopes);
        writeln(output, indent, &format!("${name} = {value};"));
        Some(name)
    } else {
        None
    };
    let present = region.as_ref().map_or_else(
        || format!("${} !== null", owner.as_ref().unwrap()),
        value_result::Place::present,
    );
    if *null_safe {
        destination.absent(output, indent);
        writeln(output, indent, &format!("if ({present}) {{"));
    }
    let body_indent = indent + usize::from(*null_safe);
    let arguments = emit_arguments_for_call(args, *span, scopes);
    expression_region::consume(output, body_indent, scopes);
    let writable = if kind.is_writable() { "true" } else { "false" };
    let location = format!(
        "start: {}, end: {}, callable: {}",
        scopes.source_location(*span, span.start),
        scopes.source_location(*span, span.end),
        scopes.callable_identity()
    );
    let producer = if let Some(region) = region {
        let [storage, offset, length] = region.region_parts().unwrap();
        let source_writable = if region.region_kind().unwrap().is_writable() {
            "true"
        } else {
            "false"
        };
        format!("__DoriaByteRegion::__doriaSubregionFields({storage}, {offset}, {length}, {source_writable}, {arguments}, writable: {writable}, {location})")
    } else {
        format!(
            "${}->__doriaRegionFields({arguments}, writable: {writable}, {location})",
            owner.unwrap()
        )
    };
    writeln(output, body_indent, &format!("{producer};"));
    destination.capture(output, body_indent);
    if *null_safe {
        writeln(output, indent, "}");
    }
    true
}

pub(super) fn prepare_scalar(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<String> {
    let (source, index, null_safe) = match expr {
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            ..
        } if property == "length" => (object.as_ref(), None, *null_safe),
        Expr::Index {
            collection, index, ..
        } => (collection.as_ref(), Some(index.as_ref()), false),
        _ => return None,
    };
    let region = value_result::region_argument(source, output, indent, scopes)?;
    let [storage, offset, length] = region.region_parts().unwrap();
    if let Some(index) = index {
        let index = expression_region::value(index, output, indent, scopes);
        let span = expr.span();
        Some(format!(
            "__DoriaByteRegion::__doriaGet({storage}, {offset}, {length}, {index}, {}, {}, {})",
            scopes.source_location(span, span.start),
            scopes.source_location(span, span.end),
            scopes.callable_identity()
        ))
    } else if null_safe {
        Some(format!("({} ? {length} : null)", region.present()))
    } else {
        Some(length)
    }
}

pub(super) fn emit_assignment(
    assignment: &Assignment,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let Some((source, index, access_span)) = indexed_place(&assignment.target) else {
        return false;
    };
    let Some(region) = value_result::region_argument(source, output, indent, scopes) else {
        return false;
    };
    let [storage, offset, length] = region.region_parts().unwrap();
    let position = scopes.fresh_temp("__doria_region_index");
    let index = expression_region::value(index, output, indent, scopes);
    writeln(output, indent, &format!("${position} = {index};"));
    let location = format!(
        "{}, {}, {}",
        scopes.source_location(access_span, access_span.start),
        scopes.source_location(access_span, access_span.end),
        scopes.callable_identity()
    );
    let previous = assign_operator(&assignment.op).map(|_| {
        let name = scopes.fresh_temp("__doria_region_previous");
        writeln(output, indent, &format!("${name} = __DoriaByteRegion::__doriaGet({storage}, {offset}, {length}, ${position}, {location});"));
        name
    });
    let right = expression_region::value(&assignment.value, output, indent, scopes);
    let value = if let Some(op) = assign_operator(&assignment.op) {
        format!(
            "__doria_uint8_operation({op:?}, ${}, {right}, {}, {}, {}, {}, {})",
            previous.unwrap(),
            scopes.source_location(assignment.span, assignment.span.start),
            scopes.source_location(assignment.span, assignment.span.end),
            scopes.source_location(assignment.value.span(), assignment.value.span().start),
            scopes.source_location(assignment.value.span(), assignment.value.span().end),
            scopes.callable_identity()
        )
    } else {
        right
    };
    let writable = if region.region_kind().unwrap().is_writable() {
        "true"
    } else {
        "false"
    };
    writeln(output, indent, &format!("__DoriaByteRegion::__doriaSet({storage}, {offset}, {length}, {writable}, ${position}, {value}, {location});"));
    true
}

pub(super) fn assign_operator(op: &AssignOp) -> Option<&'static str> {
    match op {
        AssignOp::Assign => None,
        AssignOp::AddAssign => Some("+"),
        AssignOp::SubAssign => Some("-"),
        AssignOp::MulAssign => Some("*"),
        AssignOp::DivAssign => Some("/"),
        AssignOp::ModAssign => Some("%"),
        AssignOp::ShiftLeftAssign => Some("<<"),
        AssignOp::ShiftRightAssign => Some(">>"),
        AssignOp::BitwiseAndAssign => Some("&"),
        AssignOp::BitwiseOrAssign => Some("|"),
        AssignOp::BitwiseXorAssign => Some("^"),
    }
}

pub(super) fn rmw(
    target: &Expr,
    right: &str,
    op: &str,
    span: Span,
    right_span: Span,
    scopes: &PhpNameScopes,
) -> Option<String> {
    let (source, index, access_span) = indexed_place(target)?;
    if !scopes
        .expression_types
        .get(&source.span())
        .is_some_and(indexed)
    {
        return None;
    }
    let receiver = scopes.expression_temp("__doria_byte_receiver_", span);
    let position = scopes.expression_temp("__doria_byte_index_", span);
    let previous = scopes.expression_temp("__doria_byte_previous_", span);
    let location = format!(
        "{}, {}, {}",
        scopes.source_location(access_span, access_span.start),
        scopes.source_location(access_span, access_span.end),
        scopes.callable_identity()
    );
    let body = format!("${receiver} = {}; ${position} = {}; ${previous} = ${receiver}->get(${position}, {location}); ${receiver}->set(${position}, __doria_uint8_operation({op:?}, ${previous}, {right}, {}, {}, {}, {}, {}), {location}); return null;",
        emit_member_receiver(source, scopes), emit_expr(index, scopes),
        scopes.source_location(span, span.start), scopes.source_location(span, span.end),
        scopes.source_location(right_span, right_span.start), scopes.source_location(right_span, right_span.end), scopes.callable_identity());
    Some(emit_expression_body(scopes, &body, &[]))
}

pub(super) fn emit_runtime(output: &mut String, native: bool) {
    let storage = if native {
        "__DoriaNativeIo::get()->new(\"uint8_t[\" . $length . \"]\")"
    } else {
        "array_fill(0, $length, 0)"
    };
    output.push_str(
        &RUNTIME
            .replace("__DORIA_BYTE_STORAGE__", storage)
            .replace(
                "__DORIA_REGION_FIELDS__",
                &value_result::REGION_FIELDS.to_string(),
            )
            .replace(
                "__DORIA_READONLY_REGION_TYPE__",
                &value_result::region_result_type(crate::types::ByteRegionKind::Readonly)
                    .to_string(),
            )
            .replace(
                "__DORIA_WRITABLE_REGION_TYPE__",
                &value_result::region_result_type(crate::types::ByteRegionKind::Writable)
                    .to_string(),
            ),
    );
}

const RUNTIME: &str = r#"
final class __DoriaBytes implements Countable, IteratorAggregate, __DoriaValueEquatable
{
    private ?FFI\CData $nativeBase = null;
    private readonly WeakReference $regionStorage;

    private function __construct(private array|FFI\CData $values, private readonly int $length,
        private ?FFI\CData $nativeOwner = null)
    {
        $this->regionStorage = WeakReference::create($this);
    }

    public function __doriaRegionFields(int $offset, int $length, bool $writable,
        int $start, int $end, string $callable): void
    {
        __DoriaByteRegion::__doriaSubregionFields($this->regionStorage, 0, $this->length,
            true, $offset, $length, $writable, $start, $end, $callable);
    }

    public static function __doriaFromNativeBytes(FFI\CData $owner): self
    {
        $runtime = __DoriaNativeIo::get();
        $length = $runtime->dr_v1_bytes_length($owner);
        return new self($length === 0 ? [] : $runtime->cast('uint8_t *', $runtime->dr_v1_bytes_data($owner)), $length, $owner);
    }

    public function __destruct()
    {
        if ($this->nativeOwner !== null) {
            $owner = $this->nativeOwner;
            $this->nativeOwner = null;
            __DoriaNativeIo::get()->dr_v1_bytes_free($owner);
        }
    }

    public static function allocate(int $length, int $start, int $end, string $callable): self
    {
        if ($length < 0) {
            __doria_panic("P1303", $start, $end, null, $callable, ["length" => [1, $length]]);
        }
        try {
            return new self($length === 0 ? [] : __DORIA_BYTE_STORAGE__, $length);
        } catch (ValueError|FFI\Exception) {
            __doria_panic("P1302", $start, $end, null, $callable);
        }
    }

    public static function fromArray(array $values, int $start, int $end, string $callable): self
    {
        $copy = self::allocate(count($values), $start, $end, $callable);
        $index = 0;
        foreach ($values as $value) {
            $copy->values[$index++] = $value;
        }
        return $copy;
    }

    /** String::fromBytes: the bytes as a string when they are valid UTF-8, else null. */
    public function __doriaUtf8String(): ?string
    {
        $text = '';
        for ($index = 0; $index < $this->length; $index++) {
            $text .= chr($this->values[$index]);
        }
        // PCRE rejects overlong forms, surrogates and code points past U+10FFFF,
        // matching the native validator.
        return preg_match('//u', $text) === 1 ? $text : null;
    }

    public function toArray(): array
    {
        $copy = [];
        $length = $this->length;
        for ($index = 0; $index < $length; ++$index) {
            $copy[] = $this->values[$index];
        }
        return $copy;
    }

    public function count(): int
    {
        return $this->length;
    }

    public function getRegion(int $offset, int $length, int $start, int $end, string $callable): __DoriaByteRegion
    {
        return __DoriaByteRegion::fromBytes($this, $offset, $length, false, $start, $end, $callable);
    }

    public function getWritableRegion(int $offset, int $length, int $start, int $end, string $callable): __DoriaByteRegion
    {
        return __DoriaByteRegion::fromBytes($this, $offset, $length, true, $start, $end, $callable);
    }

    public function getIterator(): Traversable
    {
        // Index the owner instead of retaining a snapshot of its storage.
        $length = $this->length;
        for ($index = 0; $index < $length; ++$index) {
            yield $index => $this->values[$index];
        }
    }

    private function checkIndex(int $index, int $start, int $end, string $callable): void
    {
        $length = $this->length;
        if ($index < 0 || $index >= $length) {
            __doria_panic("P1301", $start, $end, null, $callable,
                ["index" => [1, $index], "length" => [2, $length]]);
        }
    }

    public function get(int $index, int $start, int $end, string $callable): int
    {
        $this->checkIndex($index, $start, $end, $callable);
        return $this->values[$index];
    }

    public function set(int $index, int $value, int $start, int $end, string $callable): void
    {
        $this->checkIndex($index, $start, $end, $callable);
        $this->values[$index] = $value;
    }

    public function __doriaBorrowCell(int $index, int $start, int $end, string $callable): __DoriaCell
    {
        $this->checkIndex($index, $start, $end, $callable);
        // PHP references to CData elements detach from the native slot.
        // Project through the owner for both storage representations instead.
        return new __DoriaByteCell($this, $index, $start, $end, $callable);
    }

    /**
     * The one address view of this storage. A native call names a range by this
     * base, this owner's count, and integers; the runtime checks the range and
     * forms the address, so no call allocates an interior view. PHP-array and
     * empty storage have no base, and the runtime rejects a nonempty range there.
     */
    public function __doriaNativeBase(): ?FFI\CData
    {
        if ($this->length === 0 || !($this->values instanceof FFI\CData)) { return null; }
        return $this->nativeBase ??= FFI::addr($this->values[0]);
    }

    /** WritableByteRegion::copyFrom storage step; the caller checked both ranges. */
    public function __doriaCopyFrom(int $offset, self $source, int $sourceOffset, int $count): void
    {
        if ($count === 0) { return; }
        if ($this->values instanceof FFI\CData && $source->values instanceof FFI\CData) {
            // The runtime copies correctly when copyWithin's ranges overlap.
            __DoriaNativeIo::get()->dr_v1_host_byte_range_copy(
                $this->__doriaNativeBase(), $this->length, $offset,
                $source->__doriaNativeBase(), $source->length, $sourceOffset, $count);
            return;
        }
        $bytes = [];
        for ($index = 0; $index < $count; ++$index) {
            $bytes[] = $source->values[$sourceOffset + $index];
        }
        foreach ($bytes as $index => $value) {
            $this->values[$offset + $index] = $value;
        }
    }

    public function __doriaEquals(mixed $other): bool
    {
        if (!($other instanceof self)) { return false; }
        $length = $this->length;
        if ($length !== $other->length) { return false; }
        for ($index = 0; $index < $length; ++$index) {
            if ($this->values[$index] !== $other->values[$index]) { return false; }
        }
        return true;
    }
}

final class __DoriaByteCell extends __DoriaCell
{
    private readonly WeakReference $storage;

    public function __construct(__DoriaBytes $source, private readonly int $index,
        private readonly int $start, private readonly int $end, private readonly string $callable)
    {
        parent::__construct(null);
        unset($this->value);
        $this->borrowed = true;
        $this->storage = WeakReference::create($source);
    }

    private function source(): __DoriaBytes
    {
        if (!$this->live) { throw new LogicException("MIR byte borrow was used after cleanup"); }
        return $this->storage->get() ?? throw new LogicException("MIR byte borrow outlived its source");
    }

    public function __get(string $name): mixed
    {
        if ($name !== "value") { throw new LogicException("unknown byte cell member"); }
        return $this->source()->get($this->index, $this->start, $this->end, $this->callable);
    }

    public function __set(string $name, mixed $value): void
    {
        if ($name !== "value") { throw new LogicException("unknown byte cell member"); }
        $this->source()->set($this->index, $value, $this->start, $this->end, $this->callable);
    }
}

final class __DoriaByteRegion implements Countable
{
    private function __construct(private readonly WeakReference $storage,
        private readonly int $offset, private readonly int $length,
        private readonly bool $writable) {}

    public static function __doriaFromFields(WeakReference $storage, int $offset, int $length, bool $writable): self
    {
        return new self($storage, $offset, $length, $writable);
    }

    public function __doriaRegionStorage(): WeakReference { return $this->storage; }
    public function __doriaRegionOffset(): int { return $this->offset; }

    public static function __doriaSubregionFields(WeakReference $storage, int $sourceOffset,
        int $sourceLength, bool $sourceWritable, int $offset, int $length, bool $writable,
        int $start, int $end, string $callable): void
    {
        if ($writable && !$sourceWritable) { throw new LogicException("MIR region upgraded readonly access"); }
        self::checkRegion($sourceLength, $offset, $length, $start, $end, $callable);
        __DoriaValueResult::clear();
        __DoriaValueResult::$field0 = $storage;
        __DoriaValueResult::$field1 = $sourceOffset + $offset;
        __DoriaValueResult::$field2 = $length;
        __DoriaValueResult::$fields = __DORIA_REGION_FIELDS__;
        __DoriaValueResult::$type = $writable ? __DORIA_WRITABLE_REGION_TYPE__ : __DORIA_READONLY_REGION_TYPE__;
        __DoriaValueResult::$tag = 0;
        __DoriaValueResult::$present = true;
    }

    public function __doriaPublishFields(): void
    {
        self::__doriaSubregionFields($this->storage, $this->offset, $this->length,
            $this->writable, 0, $this->length, $this->writable, 0, 0, '');
    }

    /** The live owner of a scalar region; the caller holds it through the native call. */
    public static function __doriaNativeSource(WeakReference $storage): __DoriaBytes
    {
        return $storage->get() ?? throw new LogicException("MIR byte region outlived its source");
    }

    public static function __doriaGet(WeakReference $storage, int $offset, int $length,
        int $index, int $start, int $end, string $callable): int
    {
        if ($index < 0 || $index >= $length) {
            __doria_panic("P1301", $start, $end, null, $callable, ["index" => [1, $index], "length" => [2, $length]]);
        }
        $source = $storage->get() ?? throw new LogicException("MIR byte region outlived its source");
        return $source->get($offset + $index, $start, $end, $callable);
    }

    public static function __doriaSet(WeakReference $storage, int $offset, int $length,
        bool $writable, int $index, int $value, int $start, int $end, string $callable): void
    {
        if (!$writable) { throw new LogicException("MIR region wrote through readonly access"); }
        if ($index < 0 || $index >= $length) {
            __doria_panic("P1301", $start, $end, null, $callable, ["index" => [1, $index], "length" => [2, $length]]);
        }
        $source = $storage->get() ?? throw new LogicException("MIR byte region outlived its source");
        $source->set($offset + $index, $value, $start, $end, $callable);
    }

    private static function checkRegion(int $sourceLength, int $offset, int $length,
        int $start, int $end, string $callable): void
    {
        if ($offset < 0 || $length < 0 || $offset > $sourceLength || $length > $sourceLength - $offset) {
            __doria_panic("P1304", $start, $end, null, $callable,
                ["offset" => [1, $offset], "requestedLength" => [1, $length], "length" => [2, $sourceLength]]);
        }
    }

    public static function fromBytes(__DoriaBytes $source, int $offset, int $length,
        bool $writable, int $start, int $end, string $callable): self
    {
        self::checkRegion(count($source), $offset, $length, $start, $end, $callable);
        return new self(WeakReference::create($source), $offset, $length, $writable);
    }

    private function source(): __DoriaBytes
    {
        return $this->storage->get() ?? throw new LogicException("MIR byte region outlived its source");
    }

    public function count(): int { return $this->length; }

    /** The live owner a native transfer addresses; the caller holds it through the call. */
    public function __doriaNativeStorage(bool $writable): __DoriaBytes
    {
        if ($writable && !$this->writable) {
            throw new LogicException("MIR native transfer upgraded readonly access");
        }
        return $this->source();
    }

    public function __doriaWriteBytes(string $bytes): void
    {
        $length = strlen($bytes);
        if (!$this->writable || $length > $this->length) {
            throw new LogicException("MIR byte transfer exceeded its writable region");
        }
        $source = $this->source();
        for ($index = 0; $index < $length; ++$index) {
            $source->set($this->offset + $index, ord($bytes[$index]), 0, 0, "");
        }
    }

    public function getRegion(int $offset, int $length, int $start, int $end, string $callable): self
    {
        self::checkRegion($this->length, $offset, $length, $start, $end, $callable);
        return new self($this->storage, $this->offset + $offset, $length, false);
    }

    public function copyWithin(int $offset, int $length, int $destination, int $start, int $end, string $callable): void
    {
        if (!$this->writable) { throw new LogicException("MIR region copy wrote through readonly access"); }
        // Same order as the native runtime: source range first, then destination.
        self::checkRegion($this->length, $offset, $length, $start, $end, $callable);
        self::checkRegion($this->length, $destination, $length, $start, $end, $callable);
        $storage = $this->source();
        $storage->__doriaCopyFrom($this->offset + $destination, $storage, $this->offset + $offset, $length);
    }

    public function copyFrom(self $source, int $start, int $end, string $callable): int
    {
        if (!$this->writable) { throw new LogicException("MIR region copy wrote through readonly access"); }
        $count = min($this->length, $source->length);
        $this->source()->__doriaCopyFrom($this->offset, $source->source(), $source->offset, $count);
        return $count;
    }

    public function getWritableRegion(int $offset, int $length, int $start, int $end, string $callable): self
    {
        if (!$this->writable) { throw new LogicException("MIR region upgraded readonly access"); }
        self::checkRegion($this->length, $offset, $length, $start, $end, $callable);
        return new self($this->storage, $this->offset + $offset, $length, true);
    }

    private function checkIndex(int $index, int $start, int $end, string $callable): void
    {
        if ($index < 0 || $index >= $this->length) {
            __doria_panic("P1301", $start, $end, null, $callable,
                ["index" => [1, $index], "length" => [2, $this->length]]);
        }
    }

    public function get(int $index, int $start, int $end, string $callable): int
    {
        $this->checkIndex($index, $start, $end, $callable);
        return $this->source()->get($this->offset + $index, $start, $end, $callable);
    }

    public function set(int $index, int $value, int $start, int $end, string $callable): void
    {
        if (!$this->writable) { throw new LogicException("MIR region wrote through readonly access"); }
        $this->checkIndex($index, $start, $end, $callable);
        $this->source()->set($this->offset + $index, $value, $start, $end, $callable);
    }

    public function __doriaBorrowCell(int $index, int $start, int $end, string $callable): __DoriaCell
    {
        if (!$this->writable) { throw new LogicException("MIR region borrowed readonly access as writable"); }
        $this->checkIndex($index, $start, $end, $callable);
        return $this->source()->__doriaBorrowCell($this->offset + $index, $start, $end, $callable);
    }
}

function __doria_uint8_operation(string $op, int $left, int $right, int $start,
    int $end, int $rightStart, int $rightEnd, string $callable): int
{
    if ($op === "+" || $op === "-" || $op === "*") {
        return __doria_integer_operation($op, $left, $right, 8, false, $start, $end, $callable);
    }
    if (($op === "/" || $op === "%") && $right === 0) {
        __doria_panic($op === "/" ? "P1105" : "P1107", $rightStart, $rightEnd, null, $callable,
            [], __doria_diagnostic_site($start, $end));
    }
    if (($op === "<<" || $op === ">>") && ($right < 0 || $right >= 8)) {
        __doria_panic("P1108", $rightStart, $rightEnd, null, $callable,
            [], __doria_diagnostic_site($start, $end));
    }
    $value = match ($op) {
        "/" => intdiv($left, $right), "%" => $left % $right,
        "<<" => ($left << $right) & 255, ">>" => $left >> $right,
        "&" => $left & $right, "|" => $left | $right, "^" => $left ^ $right,
        default => throw new LogicException("MIR byte operation is unknown"),
    };
    return $value;
}
"#;
