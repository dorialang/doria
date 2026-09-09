//! Compatibility storage for collections whose checked operations call Doria code.
use super::*;
use crate::compiler_known_contracts::CoreValueOperation;

fn shape(ty: &ResolvedType) -> Option<(u8, Option<&ResolvedType>, &ResolvedType)> {
    let object =
        |ty: &ResolvedType| matches!(ty, ResolvedType::Class(_) | ResolvedType::Interface(_));
    match ty {
        ResolvedType::Nullable(inner) => shape(inner),
        ResolvedType::Set(value) if object(value) => Some((0, None, value)),
        ResolvedType::Dictionary(key, value) if object(key) => Some((1, Some(key), value)),
        ResolvedType::SortedSet(value) if object(value) => Some((2, None, value)),
        ResolvedType::SortedDictionary(key, value) if object(key) => Some((3, Some(key), value)),
        ResolvedType::PriorityQueue(value) if object(value) => Some((4, None, value)),
        _ => None,
    }
}

pub(super) fn uses(ty: &ResolvedType) -> bool {
    shape(ty).is_some()
}

fn receiver_type(ty: &ResolvedType) -> &ResolvedType {
    match ty {
        ResolvedType::Nullable(inner) => receiver_type(inner),
        ResolvedType::SharedHandle(
            crate::types::SharedHandleKind::ReadonlySharedReferenceAccess
            | crate::types::SharedHandleKind::WritableSharedReferenceAccess,
            inner,
        ) => receiver_type(inner),
        _ => ty,
    }
}

pub(super) fn uses_receiver(ty: &ResolvedType) -> bool {
    uses(receiver_type(ty))
}

pub(super) fn receiver(expr: &Expr, null_safe: bool, scopes: &PhpNameScopes) -> String {
    let value = emit_member_receiver(expr, scopes);
    if scopes
        .expression_types
        .get(&expr.span())
        .and_then(shared::kind)
        .is_some()
    {
        format!("{value}{}payload()", if null_safe { "?->" } else { "->" })
    } else {
        value
    }
}

fn duplicate(ty: &ResolvedType, scopes: &PhpNameScopes) -> String {
    if scopes
        .specialization
        .core_capability(ty, CoreValueOperation::Clone)
    {
        "static fn($value) => $value === null ? null : $value->clone()".into()
    } else {
        "null".into()
    }
}

fn create(ty: &ResolvedType, scopes: &PhpNameScopes) -> String {
    let (mode, key, value) = shape(ty).expect("checked core collection");
    let compared = key.unwrap_or(value);
    let equals = if scopes
        .specialization
        .core_capability(compared, CoreValueOperation::Equal)
    {
        "static fn($left, $right) => $left->equals($right)"
    } else {
        "static fn($left, $right) => $left === $right"
    };
    let hash = if mode < 2 {
        "static fn($value) => $value->hash()"
    } else {
        "null"
    };
    let compare = if mode >= 2 {
        "static function ($left, $right): int { $order = $left->compare($right); return $order === Ordering::Less ? -1 : ($order === Ordering::Greater ? 1 : 0); }"
    } else {
        "null"
    };
    format!(
        "new __DoriaCoreCollection({mode}, {hash}, {equals}, {compare}, {}, {})",
        key.map_or("null".into(), |ty| duplicate(ty, scopes)),
        duplicate(value, scopes)
    )
}

fn temporary_cell(value: String, span: Span, role: &str, scopes: &PhpNameScopes) -> String {
    let name = scopes.expression_temp(role, span);
    let mut temporaries = scopes
        .expression_temporaries
        .as_ref()
        .expect("expression cleanup scope")
        .borrow_mut();
    if !temporaries.contains(&name) {
        temporaries.push(name.clone());
    }
    format!("(${name} = new __DoriaCell({value}))")
}

fn owned_cell(expr: &Expr, scopes: &PhpNameScopes) -> String {
    temporary_cell(
        emit_owned_expr(expr, scopes),
        expr.span(),
        "__doria_collection_argument_",
        scopes,
    )
}

pub(super) fn expression(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    match expr {
        Expr::Array { elements, span } => {
            let ty = scopes.expression_types.get(span).filter(|ty| uses(ty))?;
            let mut output = temporary_cell(
                create(ty, scopes),
                *span,
                "__doria_collection_literal_",
                scopes,
            );
            for element in elements {
                let key = element
                    .key
                    .as_ref()
                    .map_or("null".into(), |key| owned_cell(key, scopes));
                let value = owned_cell(&element.value, scopes);
                output = format!("__doria_core_insert({output}, {key}, {value})");
            }
            Some(format!("__doria_take_cell({output})"))
        }
        Expr::StaticCall {
            method, args, span, ..
        } if method == "from" && args.len() == 1 => {
            let ty = scopes.expression_types.get(span).filter(|ty| uses(ty))?;
            Some(format!(
                "({})->fromSource({})",
                create(ty, scopes),
                emit_expr(&args[0].value, scopes)
            ))
        }
        Expr::MethodCall {
            object,
            method,
            args,
            null_safe,
            span,
            ..
        } => {
            let ty = scopes
                .expression_types
                .get(&object.span())
                .filter(|ty| uses_receiver(ty))?;
            let ty = receiver_type(ty);
            let (_, key, _) = shape(ty)?;
            let receiver = receiver(object, *null_safe, scopes);
            let operator = if *null_safe { "?->" } else { "->" };
            let (method, arguments) = match method.as_str() {
                "set" if key.is_some() => (
                    "setOwned",
                    args.iter()
                        .map(|arg| owned_cell(&arg.value, scopes))
                        .collect::<Vec<_>>(),
                ),
                "add" | "push" => (
                    if method == "add" {
                        "addOwned"
                    } else {
                        "pushOwned"
                    },
                    vec![owned_cell(&args[0].value, scopes)],
                ),
                "containsValue" => (
                    "containsValue",
                    vec![
                        emit_expr(&args[0].value, scopes),
                        php_equality_callback(*span, shape(ty).map(|(_, _, value)| value), scopes),
                    ],
                ),
                method => (
                    method,
                    args.iter()
                        .map(|arg| emit_expr(&arg.value, scopes))
                        .collect(),
                ),
            };
            Some(format!(
                "{receiver}{operator}{method}({})",
                arguments.join(", ")
            ))
        }
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            ..
        } if property != "referencedValue"
            && scopes
                .expression_types
                .get(&object.span())
                .is_some_and(uses_receiver) =>
        {
            Some(format!(
                "{}{}{property}",
                receiver(object, *null_safe, scopes),
                if *null_safe { "?->" } else { "->" }
            ))
        }
        Expr::Index {
            collection,
            index,
            span,
        } if scopes
            .expression_types
            .get(&collection.span())
            .is_some_and(uses_receiver) =>
        {
            Some(format!(
                "{}->required({}, {}, {}, {})",
                receiver(collection, false, scopes),
                emit_expr(index, scopes),
                php_source_location(*span, span.start),
                php_source_location(*span, span.end),
                scopes.callable_identity()
            ))
        }
        _ => None,
    }
}

pub(super) fn assignment(
    collection: &Expr,
    key: &Expr,
    value: &Expr,
    scopes: &PhpNameScopes,
) -> Option<String> {
    scopes
        .expression_types
        .get(&collection.span())
        .filter(|ty| uses_receiver(ty))?;
    Some(format!(
        "{}->setOwned({}, {})",
        receiver(collection, false, scopes),
        owned_cell(key, scopes),
        owned_cell(value, scopes)
    ))
}

pub(super) const RUNTIME: &str = r#"
function __doria_core_insert(__DoriaCell $owner, ?__DoriaCell $key, __DoriaCell $value): __DoriaCell
{
    if ($key === null) { $owner->value->addOwned($value); }
    else { $owner->value->setOwned($key, $value); }
    return $owner;
}

final class __DoriaCoreCollection extends __DoriaOrderedCollection implements IteratorAggregate
{
    // Modes and callbacks come from checked collection and conformance identities.
    private array $entries = [];
    private array $buckets = [];
    private int $next = 0;
    private static ?string $seed = null;
    public function __construct(private int $mode, private mixed $hashValue,
        private mixed $equalsValue, private mixed $compareValue,
        private mixed $duplicateKey, private mixed $duplicateValue) {}

    private function isMap(): bool { return $this->mode === 1 || $this->mode === 3; }
    private function element(int $index): mixed { return $this->entries[$index][$this->isMap() ? 0 : 1]; }
    private function mix(int $raw): string
    {
        self::$seed ??= random_bytes(16);
        return hash_hmac('sha256', pack('J', $raw), self::$seed);
    }
    private function locate(mixed $key): array
    {
        if ($this->mode < 2) {
            $raw = ($this->hashValue)($key);
            $hash = $this->mix($raw);
            foreach ($this->buckets[$hash] ?? [] as $index => $_) {
                if (($this->equalsValue)($this->element($index), $key)) { return [$index, $hash]; }
            }
            return [-1, $hash];
        }
        $low = 0;
        $high = count($this->entries);
        while ($low < $high) {
            $middle = $low + intdiv($high - $low, 2);
            if (($this->compareValue)($this->element($middle), $key) < 0) { $low = $middle + 1; }
            else { $high = $middle; }
        }
        return [$low < count($this->entries) && ($this->compareValue)($this->element($low), $key) === 0 ? $low : -1, $low];
    }

    private function insert(array $entry, mixed $position): void
    {
        if ($this->mode < 2) {
            $index = $this->next++;
            $entry[2] = $position;
            $this->entries[$index] = $entry;
            $this->buckets[$position][$index] = true;
        } else { array_splice($this->entries, $position, 0, [$entry]); }
    }

    public function setOwned(__DoriaCell $key, __DoriaCell $value): void
    {
        [$index, $position] = $this->locate($key->value);
        if ($index < 0) { $this->insert([__doria_take_cell($key), __doria_take_cell($value)], $position); }
        else {
            $previous = $this->entries[$index][1];
            $this->entries[$index][1] = __doria_take_cell($value);
            __doria_drop_value($previous);
        }
    }

    public function addOwned(__DoriaCell $value): bool
    {
        // Unselected inputs retain their caller's full-expression cleanup home.
        [$index, $position] = $this->locate($value->value);
        if ($index >= 0) { return false; }
        $this->insert([null, __doria_take_cell($value)], $position);
        return true;
    }

    public function get(mixed $key): mixed
    {
        [$index] = $this->locate($key);
        return $index < 0 ? null : $this->entries[$index][1];
    }
    public function required(mixed $key, int $start, int $end, string $callable): mixed
    {
        [$index] = $this->locate($key);
        if ($index < 0) { __doria_panic('P1312', $start, $end, null, $callable); }
        return $this->entries[$index][1];
    }
    public function containsKey(mixed $key): bool { return $this->locate($key)[0] >= 0; }
    public function contains(mixed $key): bool
    {
        if ($this->mode !== 4) { return $this->containsKey($key); }
        foreach ($this->entries as $entry) { if (($this->equalsValue)($entry[1], $key)) { return true; } }
        return false;
    }
    public function containsValue(mixed $value, ?callable $equals = null): bool
    {
        foreach ($this->entries as $entry) {
            if ($equals === null ? __doria_equal($entry[1], $value) : $equals($entry[1], $value)) { return true; }
        }
        return false;
    }
    public function remove(mixed $key): mixed
    {
        [$index] = $this->locate($key);
        if ($index < 0) { return $this->isMap() ? null : false; }
        $entry = $this->entries[$index];
        if ($this->mode < 2) {
            unset($this->entries[$index], $this->buckets[$entry[2]][$index]);
            if (!$this->buckets[$entry[2]]) { unset($this->buckets[$entry[2]]); }
        } else { array_splice($this->entries, $index, 1); }
        if ($this->isMap()) { __doria_drop_value($entry[0]); return $entry[1]; }
        __doria_drop_value($entry[1]);
        return true;
    }
    public function clear(): void
    {
        $entries = $this->entries;
        $this->entries = $this->buckets = [];
        self::releasePairsInReverse($entries);
    }

    // Select the complete path before moving any owning slot. A checked compare
    // failure leaves every existing entry available for ordinary cleanup.
    private static function destination(int $root, int $replacement, int $count, bool $maximum, callable $compare): int
    {
        while (($left = $root * 2 + 1) < $count) {
            $right = $left + 1;
            $direction = $maximum ? 1 : -1;
            $child = $right < $count && $compare($right, $left) === $direction ? $right : $left;
            if ($compare($replacement, $child) !== -$direction) { break; }
            $root = $child;
        }
        return $root;
    }
    private static function rotate(int $root, int $destination, callable $swap): void
    {
        for ($cursor = $destination; $cursor !== $root; $cursor = intdiv($cursor - 1, 2)) { $swap($root, $cursor); }
    }
    private static function heapify(int $count, bool $maximum, callable $compare, callable $swap): void
    {
        for ($root = intdiv($count, 2) - 1; $root >= 0; --$root) {
            self::rotate($root, self::destination($root, $root, $count, $maximum, $compare), $swap);
        }
    }
    private function comparePositions(int $left, int $right): int { return ($this->compareValue)($this->element($left), $this->element($right)); }
    private static function sort(int $count, callable $compare, callable $swap): void
    {
        self::heapify($count, true, $compare, $swap);
        for ($end = $count - 1; $end > 0; --$end) {
            $swap(0, $end);
            self::rotate(0, self::destination(0, 0, $end, true, $compare), $swap);
        }
    }
    private function swap(int $left, int $right): void { [$this->entries[$left], $this->entries[$right]] = [$this->entries[$right], $this->entries[$left]]; }
    public function pushOwned(__DoriaCell $value): void
    {
            $count = count($this->entries);
            $destination = $count;
            while ($destination > 0) {
                $parent = intdiv($destination - 1, 2);
                if (($this->compareValue)($value->value, $this->element($parent)) !== -1) { break; }
                $destination = $parent;
            }
            $this->entries[] = [null, __doria_take_cell($value)];
            for ($cursor = $count; $cursor !== $destination; $cursor = $parent) {
                $parent = intdiv($cursor - 1, 2);
                $this->swap($cursor, $parent);
            }
    }
    public function pop(): mixed
    {
        $last = count($this->entries) - 1;
        if ($last < 0) { return null; }
        $destination = self::destination(0, $last, $last, false, fn($left, $right) => $this->comparePositions($left, $right));
        $this->swap(0, $last);
        $result = array_pop($this->entries);
        self::rotate(0, $destination, fn($left, $right) => $this->swap($left, $right));
        return $result[1];
    }

    private function duplicateEntry(array $entry): array
    {
        $key = null;
        try {
            $key = $this->duplicateKey === null ? $entry[0] : ($this->duplicateKey)($entry[0]);
            $value = $this->duplicateValue === null ? $entry[1] : ($this->duplicateValue)($entry[1]);
            return [$key, $value];
        } catch (__DoriaCheckedError $error) { __doria_drop_value($key); throw $error; }
    }
    public function fromSource(mixed $source): self
    {
        try {
            if ($this->mode === 0) {
                foreach ($source as $value) {
                    [$index, $hash] = $this->locate($value);
                    if ($index < 0) { $this->appendDuplicate([null, $value]); }
                }
            } elseif ($this->mode === 4) {
                foreach ($source as $value) { $this->entries[] = $this->duplicateEntry([null, $value]); }
                self::heapify(count($this->entries), false, fn($left, $right) => $this->comparePositions($left, $right), fn($left, $right) => $this->swap($left, $right));
            } else {
                // The scratch permutation borrows entries; it owns no copied payload.
                $entries = [];
                foreach ($source as $key => $value) { $entries[] = [$this->isMap() ? $key : null, $value]; }
                $count = count($entries);
                $order = $count === 0 ? [] : range(0, $count - 1);
                $component = $this->isMap() ? 0 : 1;
                $compare = function ($left, $right) use (&$order, &$entries, $component): int { return ($this->compareValue)($entries[$order[$left]][$component], $entries[$order[$right]][$component]); };
                $swap = static function ($left, $right) use (&$order): void { [$order[$left], $order[$right]] = [$order[$right], $order[$left]]; };
                self::sort($count, $compare, $swap);
                $previous = null;
                foreach ($order as $index) {
                    if (!$this->isMap() && $previous !== null && ($this->compareValue)($entries[$previous][1], $entries[$index][1]) === 0) { continue; }
                    $this->entries[] = $this->duplicateEntry($entries[$index]);
                    $previous = $index;
                }
            }
            return $this;
        } catch (__DoriaCheckedError $error) { $this->clear(); throw $error; }
    }

    private function appendDuplicate(array $entry): void
    {
        $entry = $this->duplicateEntry($entry);
        try {
            if ($this->mode === 0) {
                $hash = $this->mix(($this->hashValue)($entry[1]));
                $this->insert($entry, $hash);
            } else { $this->entries[] = $entry; }
        } catch (__DoriaCheckedError $error) {
            __doria_drop_value($entry[1]);
            __doria_drop_value($entry[0]);
            throw $error;
        }
    }
    private function algebra(self $other, string $operation): self
    {
        $result = new self($this->mode, $this->hashValue, $this->equalsValue, $this->compareValue, $this->duplicateKey, $this->duplicateValue);
        try {
            foreach ($this->entries as $entry) {
                if ($operation === 'union' || ($operation === 'intersect') === $other->contains($entry[1])) {
                    $result->appendDuplicate($entry);
                }
            }
            if ($operation === 'union') {
                foreach ($other->entries as $entry) {
                    if (!$this->contains($entry[1])) { $result->appendDuplicate($entry); }
                }
            }
            if ($this->mode === 2) {
                self::sort(count($result->entries), fn($left, $right) => $result->comparePositions($left, $right), fn($left, $right) => $result->swap($left, $right));
            }
            return $result;
        } catch (__DoriaCheckedError $error) { $result->clear(); throw $error; }
    }
    public function union(self $other): self { return $this->algebra($other, 'union'); }
    public function intersect(self $other): self { return $this->algebra($other, 'intersect'); }
    public function difference(self $other): self { return $this->algebra($other, 'difference'); }
    public function __get(string $name): mixed
    {
        if ($name === 'count') { return count($this->entries); }
        if ($name === 'isEmpty') { return !$this->entries; }
        if ($name === 'first' || $name === 'peek') { return $this->entries ? $this->entries[array_key_first($this->entries)][1] : null; }
        if ($name === 'last') { return $this->entries ? $this->entries[array_key_last($this->entries)][1] : null; }
        if ($name === 'keys' || $name === 'values') { return $this->projection($name === 'keys' ? 0 : 1); }
        return null;
    }
    private function projection(int $component): Traversable
    {
        foreach ($this->entries as $entry) { yield $entry[$component]; }
    }
    public function &getIterator(): Traversable
    {
        foreach ($this->entries as $index => $entry) { yield ($this->isMap() ? $entry[0] : $index) => $this->entries[$index][1]; }
    }
}
"#;
