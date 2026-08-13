use std::collections::{HashMap, HashSet};

use crate::backend::BackendError;
use crate::builtins::Builtin;
use crate::const_eval::{ConstKey, ConstValue, Evaluation, ParameterDefaultKey};
use crate::diagnostics::Diagnostic;
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::hir::*;
use crate::numeric::{parse_decimal_magnitude, FloatType, IntegerType};
use crate::semantics::{MatchSemanticInfo, ResolvedMatchPattern, SemanticInfo};
use crate::source::Span;
use crate::types::{ResolvedType, TypeRef};

const PHP_INTEGER_UNSUPPORTED_CODE: &str = "B1301";
const PHP_OWNERSHIP_UNSUPPORTED_CODE: &str = "B1901";
const PHP_CONSTANT_UNSUPPORTED_CODE: &str = "B2001";
const PHP_COLLECTION_UNSUPPORTED_CODE: &str = "B2301";
const PHP_GENERICS_UNSUPPORTED_CODE: &str = "B2401";
const PHP_STRING_RUNTIME_UNSUPPORTED_CODE: &str = "B2501";

const PHP_STAGE26_COLLECTION_HELPERS: &str = r#"
abstract class __DoriaOrderedCollection
{
    protected static function compare(mixed $left, mixed $right): int
    {
        if (is_string($left)) { return strcmp($left, $right); }
        if (is_bool($left)) { return ((int) $left) <=> ((int) $right); }
        return $left <=> $right;
    }

    protected static function releaseValuesInReverse(array &$values): void
    {
        $keys = array_keys($values);
        for ($index = count($keys) - 1; $index >= 0; --$index) {
            unset($values[$keys[$index]]);
        }
    }

    protected static function releasePairsInReverse(array &$pairs): void
    {
        $keys = array_keys($pairs);
        for ($index = count($keys) - 1; $index >= 0; --$index) {
            $key = $keys[$index];
            unset($pairs[$key][1]);
            unset($pairs[$key][0]);
            unset($pairs[$key]);
        }
    }
}

final class SortedDictionary extends __DoriaOrderedCollection implements ArrayAccess, IteratorAggregate
{
    private array $entries = [];

    public static function from(array $source): self
    {
        $pairs = [];
        foreach ($source as $key => $value) { $pairs[] = [$key, $value]; }
        return self::fromPairs($pairs);
    }

    public static function fromPairs(array $pairs): self
    {
        $result = new self();
        foreach ($pairs as $pair) { $result->set($pair[0], $pair[1]); }
        return $result;
    }

    private function locate(mixed $key): array
    {
        $low = 0;
        $high = count($this->entries);
        while ($low < $high) {
            $middle = $low + intdiv($high - $low, 2);
            $order = self::compare($this->entries[$middle][0], $key);
            if ($order < 0) { $low = $middle + 1; }
            elseif ($order > 0) { $high = $middle; }
            else { return [true, $middle]; }
        }
        return [false, $low];
    }

    public function set(mixed $key, mixed $value): void
    {
        [$found, $index] = $this->locate($key);
        if ($found) { $this->entries[$index][1] = $value; return; }
        array_splice($this->entries, $index, 0, [[$key, $value]]);
    }

    public function get(mixed $key): mixed
    {
        [$found, $index] = $this->locate($key);
        return $found ? $this->entries[$index][1] : null;
    }

    public function containsKey(mixed $key): bool { return $this->locate($key)[0]; }

    public function containsValue(mixed $value): bool
    {
        foreach ($this->entries as $entry) {
            if ($entry[1] === $value) { return true; }
        }
        return false;
    }

    public function remove(mixed $key): mixed
    {
        [$found, $index] = $this->locate($key);
        if (!$found) { return null; }
        $value = $this->entries[$index][1];
        array_splice($this->entries, $index, 1);
        return $value;
    }

    public function clear(): void
    {
        $entries = $this->entries;
        $this->entries = [];
        self::releasePairsInReverse($entries);
    }

    public function __get(string $name): mixed
    {
        if ($name === 'count') { return count($this->entries); }
        if ($name === 'isEmpty') { return count($this->entries) === 0; }
        if ($name === 'keys') { return array_map(fn($entry) => $entry[0], $this->entries); }
        if ($name === 'values') { return array_map(fn($entry) => $entry[1], $this->entries); }
        return null;
    }

    public function offsetExists(mixed $offset): bool { return $this->containsKey($offset); }
    public function offsetGet(mixed $offset): mixed
    {
        [$found, $index] = $this->locate($offset);
        if (!$found) { __doria_panic('P1311', 0, 0); }
        return $this->entries[$index][1];
    }
    public function offsetSet(mixed $offset, mixed $value): void { $this->set($offset, $value); }
    public function offsetUnset(mixed $offset): void { $this->remove($offset); }

    public function &getIterator(): Traversable
    {
        foreach ($this->entries as $index => $entry) {
            yield $entry[0] => $this->entries[$index][1];
        }
    }
}

final class SortedSet extends __DoriaOrderedCollection implements IteratorAggregate
{
    private array $values = [];

    public static function from(array $source): self
    {
        $result = new self();
        foreach ($source as $value) { $result->add($value); }
        return $result;
    }

    private function locate(mixed $value): array
    {
        $low = 0;
        $high = count($this->values);
        while ($low < $high) {
            $middle = $low + intdiv($high - $low, 2);
            $order = self::compare($this->values[$middle], $value);
            if ($order < 0) { $low = $middle + 1; }
            elseif ($order > 0) { $high = $middle; }
            else { return [true, $middle]; }
        }
        return [false, $low];
    }

    public function add(mixed $value): bool
    {
        [$found, $index] = $this->locate($value);
        if ($found) { return false; }
        array_splice($this->values, $index, 0, [$value]);
        return true;
    }

    public function remove(mixed $value): bool
    {
        [$found, $index] = $this->locate($value);
        if (!$found) { return false; }
        array_splice($this->values, $index, 1);
        return true;
    }

    public function contains(mixed $value): bool { return $this->locate($value)[0]; }

    public function clear(): void
    {
        $values = $this->values;
        $this->values = [];
        self::releaseValuesInReverse($values);
    }

    private function algebra(self $other, string $operation): self
    {
        $result = new self();
        foreach ($this->values as $value) {
            if ($operation === 'union' || ($operation === 'intersect') === $other->contains($value)) {
                $result->add($value);
            }
        }
        if ($operation === 'union') {
            foreach ($other->values as $value) { $result->add($value); }
        }
        return $result;
    }

    public function union(self $other): self { return $this->algebra($other, 'union'); }
    public function intersect(self $other): self { return $this->algebra($other, 'intersect'); }
    public function difference(self $other): self { return $this->algebra($other, 'difference'); }
    public function __get(string $name): mixed
    {
        if ($name === 'count') { return count($this->values); }
        if ($name === 'isEmpty') { return count($this->values) === 0; }
        if ($name === 'first') { return $this->values[0] ?? null; }
        if ($name === 'last') {
            return $this->values ? $this->values[count($this->values) - 1] : null;
        }
        return null;
    }
    public function getIterator(): Traversable { yield from $this->values; }
}

final class PriorityQueue extends __DoriaOrderedCollection
{
    private array $heap = [];
    public static function from(array $source): self
    {
        $result = new self();
        $result->heap = array_values($source);
        for ($root = intdiv(count($result->heap), 2) - 1; $root >= 0; --$root) {
            $result->siftDown($root);
        }
        return $result;
    }
    private function siftDown(int $parent): void
    {
        $length = count($this->heap);
        while (($left = $parent * 2 + 1) < $length) {
            $right = $left + 1;
            $child = $right < $length && self::compare($this->heap[$right], $this->heap[$left]) < 0
                ? $right : $left;
            if (self::compare($this->heap[$parent], $this->heap[$child]) <= 0) { return; }
            [$this->heap[$parent], $this->heap[$child]] = [$this->heap[$child], $this->heap[$parent]];
            $parent = $child;
        }
    }
    public function push(mixed $value): void
    {
        $this->heap[] = $value;
        $child = count($this->heap) - 1;
        while ($child > 0) {
            $parent = intdiv($child - 1, 2);
            if (self::compare($this->heap[$parent], $this->heap[$child]) <= 0) { break; }
            [$this->heap[$parent], $this->heap[$child]] = [$this->heap[$child], $this->heap[$parent]];
            $child = $parent;
        }
    }
    public function pop(): mixed
    {
        if (!$this->heap) { return null; }
        $value = $this->heap[0];
        $last = array_pop($this->heap);
        if ($this->heap) { $this->heap[0] = $last; $this->siftDown(0); }
        return $value;
    }
    public function clear(): void
    {
        $heap = $this->heap;
        $this->heap = [];
        self::releaseValuesInReverse($heap);
    }
    public function __get(string $name): mixed
    {
        if ($name === 'count') { return count($this->heap); }
        if ($name === 'isEmpty') { return count($this->heap) === 0; }
        if ($name === 'peek') { return $this->heap[0] ?? null; }
        return null;
    }
}

final class Deque extends __DoriaOrderedCollection implements IteratorAggregate
{
    private array $values = [];
    private int $head = 0;
    private int $count = 0;
    private int $capacity = 4;
    public static function from(array $source): self
    {
        $result = new self();
        foreach ($source as $value) { $result->pushBack($value); }
        return $result;
    }
    private function grow(): void
    {
        $next = [];
        for ($index = 0; $index < $this->count; ++$index) {
            $next[$index] = $this->values[($this->head + $index) % $this->capacity];
        }
        $this->values = $next;
        $this->head = 0;
        $this->capacity *= 2;
    }
    public function pushBack(mixed $value): void
    {
        if ($this->count === $this->capacity) { $this->grow(); }
        $this->values[($this->head + $this->count) % $this->capacity] = $value;
        ++$this->count;
    }
    public function pushFront(mixed $value): void
    {
        if ($this->count === $this->capacity) { $this->grow(); }
        $this->head = ($this->head + $this->capacity - 1) % $this->capacity;
        $this->values[$this->head] = $value;
        ++$this->count;
    }
    public function popFront(): mixed
    {
        if ($this->count === 0) { return null; }
        $value = $this->values[$this->head];
        unset($this->values[$this->head]);
        $this->head = ($this->head + 1) % $this->capacity;
        --$this->count;
        return $value;
    }
    public function popBack(): mixed
    {
        if ($this->count === 0) { return null; }
        $index = ($this->head + $this->count - 1) % $this->capacity;
        $value = $this->values[$index];
        unset($this->values[$index]);
        --$this->count;
        return $value;
    }
    public function clear(): void
    {
        $values = $this->values;
        $head = $this->head;
        $count = $this->count;
        $capacity = $this->capacity;
        $this->values = [];
        $this->head = 0;
        $this->count = 0;
        for ($offset = $count - 1; $offset >= 0; --$offset) {
            unset($values[($head + $offset) % $capacity]);
        }
    }
    public function __get(string $name): mixed
    {
        if ($name === 'count') { return $this->count; }
        if ($name === 'isEmpty') { return $this->count === 0; }
        if ($name === 'peekFront') { return $this->count ? $this->values[$this->head] : null; }
        if ($name === 'peekBack') {
            return $this->count ? $this->values[($this->head + $this->count - 1) % $this->capacity] : null;
        }
        return null;
    }
    public function &getIterator(): Traversable
    {
        for ($offset = 0; $offset < $this->count; ++$offset) {
            $index = ($this->head + $offset) % $this->capacity;
            yield $offset => $this->values[$index];
        }
    }
}

function __doria_collection_projection(mixed $collection, bool $keys): array
{
    if (is_array($collection)) { return $keys ? array_keys($collection) : array_values($collection); }
    return $keys ? $collection->keys : $collection->values;
}

"#;

pub fn generate(program: &Program) -> Result<String, BackendError> {
    validate_program(program)?;

    let mut output = String::from(
        "<?php\n\ninterface __DoriaDisplayable\n{\n    public function toString(): string;\n}\n\ninterface __DoriaValueEquatable\n{\n    public function __doriaEquals(mixed $other): bool;\n}\n\nfinal class __DoriaMixedValue\n{\n    public function __construct(\n        private readonly string $typeTag,\n        private mixed $value,\n    ) {\n    }\n\n    public function is(string $typeTag): bool { return $this->typeTag === $typeTag; }\n    public function value(): mixed { return $this->value; }\n}\n\nfunction __doria_box_mixed(string $typeTag, mixed $value): __DoriaMixedValue\n{\n    if ($typeTag === 'float32') { $value = unpack('G', pack('G', $value))[1]; }\n    return new __DoriaMixedValue($value === null ? 'null' : $typeTag, $value);\n}\n\nfunction __doria_mixed_is(mixed $value, string $typeTag): bool\n{\n    return $value instanceof __DoriaMixedValue && $value->is($typeTag);\n}\n\nfunction __doria_mixed_value(mixed $value): mixed\n{\n    return $value instanceof __DoriaMixedValue ? $value->value() : $value;\n}\n\nfunction __doria_equal(mixed $left, mixed $right): bool\n{\n    if ($left instanceof __DoriaValueEquatable) { return $left->__doriaEquals($right); }\n    if ($right instanceof __DoriaValueEquatable) { return $right->__doriaEquals($left); }\n    return $left === $right;\n}\n\nfunction __doria_display(string|int|float|bool|__DoriaDisplayable $value): string\n{\n    if ($value instanceof __DoriaDisplayable) { return $value->toString(); }\n    if (is_bool($value)) { return $value ? 'true' : 'false'; }\n    return (string) $value;\n}\n\nfunction __doria_less(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) < 0; }\n    return $left < $right;\n}\n\nfunction __doria_less_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) <= 0; }\n    return $left <= $right;\n}\n\nfunction __doria_greater(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) > 0; }\n    return $left > $right;\n}\n\nfunction __doria_greater_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) >= 0; }\n    return $left >= $right;\n}\n\n",
    );
    output.push_str(PHP_STAGE26_COLLECTION_HELPERS);
    output.push_str(&format!(
        "$__doria_source_path = {};\n$__doria_source_text = hex2bin({});\n",
        emit_php_string_literal(&program.source_path),
        emit_php_string_literal(&hex_bytes(program.source_text.as_bytes())),
    ));
    output.push_str("$__doria_catalogue = [\n");
    for entry in doria_diagnostic_catalogue::RUNTIME_CATALOGUE
        .iter()
        .filter(|entry| {
            matches!(
                entry.code,
                "P1000"
                    | "P1001"
                    | "P1311"
                    | "P1401"
                    | "P1402"
                    | "P1403"
                    | "P1404"
                    | "P1405"
                    | "P1406"
                    | "P1407"
            )
        })
    {
        output.push_str(&format!(
            "    {} => [{}, {}, {}],\n",
            emit_php_string_literal(entry.code),
            emit_php_string_literal(entry.title),
            emit_php_string_literal(entry.primary_label),
            emit_php_string_literal(entry.explanation),
        ));
    }
    output.push_str("];\n$__doria_function_spans = [\n");
    for item in &program.items {
        match item {
            Item::Function(function) => output.push_str(&format!(
                "    {} => {},\n",
                emit_php_string_literal(&function.name),
                function.span.start,
            )),
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(function) = member {
                        output.push_str(&format!(
                            "    {} => {},\n",
                            emit_php_string_literal(&format!("{}::{}", class.name, function.name)),
                            function.span.start,
                        ));
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    output.push_str("];\n\n");
    output.push_str(
        r#"function __doria_source_line(int $offset): int
{
    global $__doria_source_text;
    return substr_count(substr($__doria_source_text, 0, max(0, $offset)), "\n") + 1;
}

function __doria_panic(string $code, int $start, int $end, ?string $message = null)
{
    global $__doria_catalogue, $__doria_source_path, $__doria_source_text, $__doria_function_spans;
    if (!isset($__doria_catalogue[$code])) { $code = "P1001"; }
    [$title, $label, $why] = $__doria_catalogue[$code];
    $line = __doria_source_line($start);
    $helperFunctions = [
        "__doria_panic",
        "__doria_source_line",
        "__doria_read_line",
        "__doria_read_file",
        "__doria_write_file",
        "__doria_append_file",
        "__doria_is_broken_pipe",
        "__doria_write_all",
        "__doria_flush_stdout",
        "__doria_write_stdout",
        "__doria_write_stderr",
        "__doria_sprintf",
        "__doria_printf",
    ];
    $frames = [];
    foreach (debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS) as $frame) {
        if (isset($frame["function"]) &&
            (isset($frame["class"]) || !in_array($frame["function"], $helperFunctions, true))) {
            $frames[] = $frame;
        }
    }
    $function = "main";
    if (isset($frames[0])) {
        $function = isset($frames[0]["class"])
            ? $frames[0]["class"] . "::" . $frames[0]["function"]
            : $frames[0]["function"];
    }
    @fwrite(STDERR, "Panic[" . $code . "]: " . $title . "\n\nWhere\n");
    @fwrite(STDERR, $__doria_source_path . " · line " . $line . " · " . $function . "\n\n");
    $before = substr($__doria_source_text, 0, max(0, $start));
    $lineStart = strrpos($before, "\n");
    $lineStart = $lineStart === false ? 0 : $lineStart + 1;
    $lineEnd = strpos($__doria_source_text, "\n", max(0, $start));
    $lineEnd = $lineEnd === false ? strlen($__doria_source_text) : $lineEnd;
    $lineText = rtrim(substr($__doria_source_text, $lineStart, $lineEnd - $lineStart), "\r");
    $prefix = substr($__doria_source_text, $lineStart, max(0, $start - $lineStart));
    $selected = substr($__doria_source_text, max(0, $start), max(1, $end - $start));
    $prefixWidth = preg_match_all('/\X/u', $prefix, $unused);
    $markerWidth = preg_match_all('/\X/u', $selected, $unused);
    if ($prefixWidth === false) { $prefixWidth = strlen($prefix); }
    if ($markerWidth === false || $markerWidth === 0) { $markerWidth = 1; }
    @fwrite(STDERR, "  " . $line . "  " . $lineText . "\n");
    $gutter = str_repeat(" ", 4 + strlen((string) $line) + $prefixWidth);
    @fwrite(STDERR, $gutter . str_repeat("^", $markerWidth) . "\n");
    @fwrite(STDERR, $gutter . $label . "\n\nWhy\n" . $why);
    if ($code === "P1000" && $message !== null) {
        @fwrite(STDERR, "\n\nNote\n" . $message);
    }
    @fwrite(STDERR, "\n\nCall Path");
    foreach ($frames as $index => $frame) {
        $name = isset($frame["class"])
            ? $frame["class"] . "::" . $frame["function"]
            : $frame["function"];
        $frameOffset = $index === 0 ? $start : ($__doria_function_spans[$name] ?? $start);
        @fwrite(
            STDERR,
            "\n" . $name . " · " . $__doria_source_path . ":" . __doria_source_line($frameOffset)
        );
    }
    @fwrite(STDERR, "\n\nProcess Exited With Status 101\n");
    exit(101);
}

function __doria_read_line(string $prompt, int $start, int $end): ?string
{
    if ($prompt !== "") { __doria_write_all(STDOUT, $prompt, $start, $end); }
    __doria_flush_stdout($start, $end);
    $line = @fgets(STDIN);
    if ($line === false) {
        if (feof(STDIN)) { return null; }
        __doria_panic("P1403", $start, $end);
    }
    if (preg_match('//u', $line) !== 1) { __doria_panic("P1404", $start, $end); }
    if (str_ends_with($line, "\n")) {
        $line = substr($line, 0, -1);
        if (str_ends_with($line, "\r")) { $line = substr($line, 0, -1); }
    }
    return $line;
}

function __doria_read_file(string $path, int $start, int $end): string
{
    if (str_contains($path, "\0")) { __doria_panic("P1405", $start, $end); }
    $contents = @file_get_contents($path);
    if ($contents === false) { __doria_panic("P1401", $start, $end); }
    if (preg_match('//u', $contents) !== 1) { __doria_panic("P1406", $start, $end); }
    return $contents;
}

function __doria_write_file(string $path, string $contents, int $start, int $end): void
{
    if (str_contains($path, "\0")) { __doria_panic("P1405", $start, $end); }
    $written = @file_put_contents($path, $contents);
    if ($written === false || $written !== strlen($contents)) { __doria_panic("P1402", $start, $end); }
}

function __doria_append_file(string $path, string $contents, int $start, int $end): void
{
    if (str_contains($path, "\0")) { __doria_panic("P1405", $start, $end); }
    $written = @file_put_contents($path, $contents, FILE_APPEND);
    if ($written === false || $written !== strlen($contents)) { __doria_panic("P1402", $start, $end); }
}

function __doria_is_broken_pipe(?array $error): bool
{
    if ($error === null || !isset($error["message"])) { return false; }
    $message = $error["message"];
    if (preg_match('/\berrno=32\b/', $message) === 1) { return true; }
    if (PHP_OS_FAMILY === "Windows" && preg_match('/\berrno=(?:109|232)\b/', $message) === 1) {
        return true;
    }
    return stripos($message, "broken pipe") !== false;
}

function __doria_write_all(mixed $stream, string $value, int $start, int $end): void
{
    $offset = 0;
    $length = strlen($value);
    while ($offset < $length) {
        error_clear_last();
        $written = @fwrite($stream, substr($value, $offset));
        if ($written === false || $written === 0) {
            if (__doria_is_broken_pipe(error_get_last())) { exit(0); }
            __doria_panic("P1407", $start, $end);
        }
        $offset += $written;
    }
}

function __doria_flush_stdout(int $start, int $end): void
{
    error_clear_last();
    if (@fflush(STDOUT)) { return; }
    if (__doria_is_broken_pipe(error_get_last())) { exit(0); }
    __doria_panic("P1407", $start, $end);
}

function __doria_write_stdout(string $value, int $start, int $end): void
{
    __doria_write_all(STDOUT, $value, $start, $end);
}

function __doria_write_stderr(string $value, int $start, int $end): void
{
    __doria_write_all(STDERR, $value, $start, $end);
}

function __doria_sprintf(string $format, mixed ...$values): string
{
    return sprintf($format, ...$values);
}

function __doria_printf(int $start, int $end, string $format, mixed ...$values): void
{
    $value = sprintf($format, ...$values);
    __doria_write_all(STDOUT, $value, $start, $end);
}

"#,
    );
    let static_properties = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .flat_map(|class| {
            class.members.iter().filter_map(move |member| match member {
                ClassMember::Property(property) if property.is_static => {
                    Some((class.name.clone(), property.name.clone()))
                }
                _ => None,
            })
        })
        .collect();
    let payload_enums = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(declaration)
                if declaration
                    .cases
                    .iter()
                    .any(|case| !case.payload.is_empty()) =>
            {
                Some(declaration)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let payload_unit_cases = payload_enums
        .iter()
        .flat_map(|declaration| {
            declaration.cases.iter().filter_map(move |case| {
                case.payload
                    .is_empty()
                    .then_some((declaration.name.clone(), case.name.clone()))
            })
        })
        .collect();
    let payload_top_constants = program
        .semantic_info
        .const_evaluation
        .values
        .iter()
        .filter_map(|(key, value)| {
            matches!(value.value, ConstValue::PayloadEnum(_))
                .then(|| match key {
                    ConstKey::TopLevel(name) => Some(name.clone()),
                    _ => None,
                })
                .flatten()
        })
        .collect();
    let payload_class_constants = program
        .semantic_info
        .const_evaluation
        .values
        .iter()
        .filter_map(|(key, value)| {
            matches!(value.value, ConstValue::PayloadEnum(_))
                .then(|| match key {
                    ConstKey::Class { class_name, name } => {
                        Some((class_name.clone(), name.clone()))
                    }
                    _ => None,
                })
                .flatten()
        })
        .collect();
    let payload_enum_ids = program
        .semantic_info
        .enums
        .iter()
        .filter(|definition| definition.cases.iter().any(|case| !case.payload.is_empty()))
        .map(|definition| definition.id)
        .collect::<HashSet<_>>();
    let payload_enum_expressions = program
        .semantic_info
        .expression_types
        .iter()
        .filter_map(|(span, ty)| {
            let ty = match ty {
                ResolvedType::Nullable(inner) => inner.as_ref(),
                ty => ty,
            };
            matches!(ty, ResolvedType::Enum(enum_ty) if payload_enum_ids.contains(&enum_ty.id))
                .then_some(*span)
        })
        .collect();
    let mut scopes = PhpNameScopes::new(
        static_properties,
        payload_unit_cases,
        payload_top_constants,
        payload_class_constants,
        payload_enum_expressions,
    );
    scopes.matches = program.semantic_info.matches.clone();
    scopes.expression_types = program.semantic_info.expression_types.clone();
    scopes.type_test_types = program.semantic_info.type_test_types.clone();
    scopes.mixed_box_types = program.semantic_info.mixed_box_types.clone();
    scopes.const_evaluation = program.semantic_info.const_evaluation.clone();
    scopes.payload_case_tags = program
        .semantic_info
        .enums
        .iter()
        .filter(|definition| definition.cases.iter().any(|case| !case.payload.is_empty()))
        .flat_map(|definition| {
            definition
                .cases
                .iter()
                .map(|case| (case.id, case.tag))
                .collect::<Vec<_>>()
        })
        .collect();
    for item in &program.items {
        emit_item(item, &program.semantic_info, &mut output, 0, &mut scopes);
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push('\n');
    }
    Ok(output)
}

fn validate_program(program: &Program) -> Result<(), BackendError> {
    for item in &program.items {
        validate_item(item, &program.semantic_info)?;
    }
    Ok(())
}

fn validate_item(item: &Item, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    match item {
        Item::Class(class_decl) => {
            if !class_decl.type_params.is_empty() {
                return Err(BackendError::from_diagnostics(vec![Diagnostic::new(
                    PHP_GENERICS_UNSUPPORTED_CODE,
                    "PHP compatibility output does not support generic class specialization; compile this program for a native target",
                    class_decl.span,
                )]));
            }
            for member in &class_decl.members {
                match member {
                    ClassMember::Property(property) => {
                        validate_type(&property.ty, property.span)?;
                        if property.is_static {
                            validate_evaluated_value(
                                semantic_info,
                                &ConstKey::Static {
                                    class_name: class_decl.name.clone(),
                                    name: property.name.clone(),
                                },
                                property.span,
                            )?;
                        } else if let Some(initializer) = &property.initializer {
                            validate_expr(initializer, semantic_info)?;
                            if let Some((span, feature)) =
                                unsupported_php_property_default(initializer, semantic_info)
                            {
                                return Err(unsupported_constant_shape(span, feature));
                            }
                        }
                    }
                    ClassMember::Method(method) => validate_function(method, semantic_info, true)?,
                    ClassMember::Constant(constant) => {
                        validate_php_class_constant_name(constant)?;
                        if let Some(ty) = &constant.ty {
                            validate_type(ty, constant.span)?;
                        }
                        validate_evaluated_value(
                            semantic_info,
                            &ConstKey::Class {
                                class_name: class_decl.name.clone(),
                                name: constant.name.clone(),
                            },
                            constant.span,
                        )?;
                    }
                }
            }
            Ok(())
        }
        Item::Enum(enum_decl) => {
            if !enum_decl.type_params.is_empty() {
                return Err(BackendError::from_diagnostics(vec![Diagnostic::new(
                    "B0003",
                    "PHP compatibility output for generic enums requires a future generic-enum stage",
                    enum_decl.span,
                )]));
            }
            for case in &enum_decl.cases {
                for field in &case.payload {
                    validate_type(&field.ty, field.span)?;
                }
                if let Some(value) = &case.backing_value {
                    validate_expr(value, semantic_info)?;
                }
            }
            Ok(())
        }
        Item::Function(function) => validate_function(function, semantic_info, false),
        Item::Constant(constant) => {
            if let Some(ty) = &constant.ty {
                validate_type(ty, constant.span)?;
            }
            validate_evaluated_value(
                semantic_info,
                &ConstKey::TopLevel(constant.name.clone()),
                constant.span,
            )
        }
        Item::Statement(statement) => validate_statement(statement, semantic_info),
    }
}

fn validate_php_class_constant_name(constant: &ConstDecl) -> Result<(), BackendError> {
    if constant.name.eq_ignore_ascii_case("class") {
        return Err(unsupported_constant_shape(
            constant.span,
            format!(
                "class constant `{}` because PHP reserves `class` for class-name fetching",
                constant.name
            ),
        ));
    }
    Ok(())
}

fn validate_evaluated_value(
    semantic_info: &SemanticInfo,
    key: &ConstKey,
    span: Span,
) -> Result<(), BackendError> {
    validate_const_value(evaluated_value(&semantic_info.const_evaluation, key), span)
}

fn validate_const_value(value: &ConstValue, span: Span) -> Result<(), BackendError> {
    match value {
        ConstValue::Integer(value) if !value.ty.is_default_int() => Err(unsupported_integer_shape(
            span,
            format!(
                "Doria `{}` width and signedness with PHP's single signed integer type",
                value.ty.source_name()
            ),
        )),
        ConstValue::Integer(value) if value.mathematical_value() > i64::MAX as i128 => {
            Err(unsupported_integer_shape(
                span,
                "an integer constant outside PHP's signed integer range",
            ))
        }
        ConstValue::Float(value) if !value.ty.is_default_float() => Err(unsupported_numeric_shape(
            span,
            "Doria `float32` precision with PHP's `float` type",
        )),
        _ => Ok(()),
    }
}

fn validate_function(
    function: &FunctionDecl,
    semantic_info: &SemanticInfo,
    is_method: bool,
) -> Result<(), BackendError> {
    if !function.type_params.is_empty() {
        return Err(BackendError::from_diagnostics(vec![Diagnostic::new(
            PHP_GENERICS_UNSUPPORTED_CODE,
            "PHP compatibility output does not support generic function specialization; compile this program for a native target",
            function.span,
        )]));
    }
    if is_method && function.name == "__destruct" {
        return Err(unsupported_ownership_shape(
            function.span,
            "deterministic scope-based `__destruct` timing",
        ));
    }
    if is_method
        && matches!(function.name.as_str(), "__construct" | "__destruct")
        && (function.is_static || function.writable_this)
    {
        return Err(BackendError::new(format!(
            "compiler invariant violated: invalid lifecycle method `{}` reached PHP emission after semantic validation",
            function.name
        )));
    }
    for (parameter_index, param) in function.params.iter().enumerate() {
        if param.take && is_move_type(&param.ty, semantic_info) {
            return Err(unsupported_ownership_shape(
                param.span,
                format!("ownership transfer through `take ${}`", param.name),
            ));
        }
        validate_type(&param.ty, param.span)?;
        if param.default.is_some() {
            let default = semantic_info
                .parameter_defaults
                .get(&ParameterDefaultKey {
                    function_start: function.span.start,
                    parameter_index,
                })
                .ok_or_else(|| {
                    BackendError::new(format!(
                        "compiler invariant violated: checked default for parameter `${}` has no folded value",
                        param.name
                    ))
                })?;
            validate_const_value(default, param.span)?;
        }
    }
    if let Some(return_type) = &function.return_type {
        validate_type(return_type, function.span)?;
    }
    validate_block(&function.body, semantic_info)
}

fn is_move_type(ty: &TypeRef, semantic_info: &SemanticInfo) -> bool {
    ty.name == "mixed"
        || matches!(
            ty.name.as_str(),
            "[]" | "List"
                | "Dictionary"
                | "Set"
                | "SortedDictionary"
                | "SortedSet"
                | "PriorityQueue"
                | "Deque"
        )
        || semantic_info
            .classes
            .iter()
            .any(|class| class.name == ty.name)
}

fn validate_type(ty: &TypeRef, span: Span) -> Result<(), BackendError> {
    if crate::types::SharedHandleKind::from_source_name(&ty.name).is_some() {
        return Err(unsupported_shared_ownership(span));
    }
    if ty.name == "Bytes" {
        return Err(unsupported_collection_shape(
            span,
            "the native `Bytes` runtime representation",
        ));
    }
    for argument in ty.type_arguments() {
        validate_type(argument, span)?;
    }
    Ok(())
}

fn unsupported_shared_ownership(span: Span) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::unsupported_stage(
        PHP_COLLECTION_UNSUPPORTED_CODE,
        "PHP compatibility backend cannot preserve Doria shared ownership; use the `native` or `debug` target for this valid Doria program",
        span,
    )])
}

fn validate_block(block: &Block, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    for statement in &block.statements {
        validate_statement(statement, semantic_info)?;
    }
    Ok(())
}

fn validate_statement(statement: &Stmt, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    match statement {
        Stmt::Block(block) => validate_block(block, semantic_info),
        Stmt::VarDecl(decl) => {
            if let Some(ty) = &decl.ty {
                validate_type(ty, decl.span)?;
            }
            validate_expr(&decl.initializer, semantic_info)
        }
        Stmt::Assignment(assignment) => validate_assignment(assignment, semantic_info),
        Stmt::Echo { expr, .. } => validate_display_expr(expr, semantic_info),
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                validate_expr(expr, semantic_info)?;
            }
            Ok(())
        }
        Stmt::If(if_stmt) => validate_if(if_stmt, semantic_info),
        Stmt::While(while_stmt) => {
            validate_expr(&while_stmt.condition, semantic_info)?;
            validate_block(&while_stmt.body, semantic_info)
        }
        Stmt::For(for_stmt) => {
            if let Some(initializer) = &for_stmt.initializer {
                match initializer {
                    ForInitializer::VarDecl(decl) => {
                        if let Some(ty) = &decl.ty {
                            validate_type(ty, decl.span)?;
                        }
                        validate_expr(&decl.initializer, semantic_info)?;
                    }
                    ForInitializer::Assignment(assignment) => {
                        validate_assignment(assignment, semantic_info)?;
                    }
                }
            }
            if let Some(condition) = &for_stmt.condition {
                validate_expr(condition, semantic_info)?;
            }
            if let Some(increment) = &for_stmt.increment {
                match increment {
                    ForIncrement::Increment(increment) => {
                        return Err(unsupported_increment(increment));
                    }
                    ForIncrement::Assignment(assignment) => {
                        validate_assignment(assignment, semantic_info)?;
                    }
                }
            }
            validate_block(&for_stmt.body, semantic_info)
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => Ok(()),
        Stmt::Foreach(foreach) => {
            if foreach.value.writable {
                let iterable = dictionary_foreach_projection(&foreach.iterable)
                    .map_or(&foreach.iterable, |(dictionary, _)| dictionary);
                let supported = semantic_info
                    .expression_type(iterable.span())
                    .is_some_and(|ty| {
                        matches!(
                            ty,
                            ResolvedType::SortedDictionary(_, _) | ResolvedType::Deque(_)
                        )
                    });
                if !supported {
                    return Err(unsupported_collection_shape(
                        foreach.span,
                        "writable collection element iteration",
                    ));
                }
            }
            if let Some((dictionary, _)) = dictionary_foreach_projection(&foreach.iterable) {
                validate_expr(dictionary, semantic_info)?;
            } else {
                validate_expr(&foreach.iterable, semantic_info)?;
            }
            if let Some(key) = &foreach.key {
                if let Some(ty) = &key.ty {
                    validate_type(ty, foreach.span)?;
                }
            }
            if let Some(ty) = &foreach.value.ty {
                validate_type(ty, foreach.span)?;
            }
            validate_block(&foreach.body, semantic_info)
        }
        Stmt::Increment(increment) => Err(unsupported_increment(increment)),
        Stmt::Expr { expr, .. } => validate_expr(expr, semantic_info),
    }
}

fn validate_if(if_stmt: &IfStmt, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    validate_expr(&if_stmt.condition, semantic_info)?;
    validate_block(&if_stmt.then_block, semantic_info)?;
    if let Some(else_branch) = &if_stmt.else_branch {
        match else_branch {
            ElseBranch::If(else_if) => validate_if(else_if, semantic_info)?,
            ElseBranch::Block(block) => validate_block(block, semantic_info)?,
        }
    }
    Ok(())
}

fn validate_assignment(
    assignment: &Assignment,
    semantic_info: &SemanticInfo,
) -> Result<(), BackendError> {
    validate_expr(&assignment.target, semantic_info)?;
    validate_expr(&assignment.value, semantic_info)?;

    // Semantic checking has already required compound-assignment operands to
    // have one compatible numeric type. The value metadata is sufficient here
    // because assignment targets are not expression-valued in Doria IR.
    let float_assignment = semantic_info.float_type(assignment.value.span()).is_some();
    let feature = match assignment.op {
        AssignOp::Assign => None,
        AssignOp::AddAssign if float_assignment => None,
        AssignOp::SubAssign if float_assignment => None,
        AssignOp::MulAssign if float_assignment => None,
        AssignOp::DivAssign if float_assignment => None,
        AssignOp::AddAssign => Some("checked integer overflow behavior for `+=`"),
        AssignOp::SubAssign => Some("checked integer overflow behavior for `-=`"),
        AssignOp::MulAssign => Some("checked integer overflow behavior for `*=`"),
        AssignOp::DivAssign => Some("Doria integer division semantics for `/=`"),
        AssignOp::ModAssign => Some("Doria integer remainder semantics for `%=`"),
        AssignOp::ShiftLeftAssign => Some("Doria integer shift semantics for `<<=`"),
        AssignOp::ShiftRightAssign => Some("Doria integer shift semantics for `>>=`"),
        AssignOp::BitwiseAndAssign => Some("fixed-width Doria bitwise semantics for `&=`"),
        AssignOp::BitwiseOrAssign => Some("fixed-width Doria bitwise semantics for `|=`"),
        AssignOp::BitwiseXorAssign => Some("fixed-width Doria bitwise semantics for `^=`"),
    };
    if let Some(feature) = feature {
        return Err(unsupported_integer_shape(assignment.span, feature));
    }
    Ok(())
}

fn unsupported_increment(increment: &IncrementStmt) -> BackendError {
    let operator = match increment.op {
        IncrementOp::Increment => "++",
        IncrementOp::Decrement => "--",
    };
    unsupported_integer_shape(
        increment.span,
        format!("checked integer overflow behavior for `{operator}`"),
    )
}

fn validate_expr(expr: &Expr, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    match expr {
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. } => Ok(()),
        Expr::Int { value, span } => {
            if parse_decimal_magnitude(value).is_some_and(|value| value > i64::MAX as u128) {
                return Err(unsupported_integer_shape(
                    *span,
                    format!(
                        "integer literal `{value}` outside PHP's signed integer range; the `uint64` maximum must not become a PHP float"
                    ),
                ));
            }
            Ok(())
        }
        Expr::InterpolatedString { parts, .. } => {
            for part in parts {
                if let InterpolatedStringPart::Expr(expr) = part {
                    validate_display_expr(expr, semantic_info)?;
                }
            }
            Ok(())
        }
        Expr::Array { elements, .. } => {
            for element in elements {
                if let Some(key) = &element.key {
                    validate_expr(key, semantic_info)?;
                }
                validate_expr(&element.value, semantic_info)?;
            }
            Ok(())
        }
        Expr::ArrayRepeat { span, .. } => Err(unsupported_collection_shape(
            *span,
            "sequence fill literals require the native collection runtime",
        )),
        Expr::Index {
            collection,
            index,
            span,
        } => {
            validate_expr(collection, semantic_info)?;
            validate_expr(index, semantic_info)?;
            if semantic_info
                .expression_type(collection.span())
                .is_some_and(is_stage23_runtime_type)
            {
                return Err(unsupported_collection_shape(
                    *span,
                    "assertive collection indexed access",
                ));
            }
            Ok(())
        }
        Expr::PropertyAccess {
            object,
            property,
            span,
            ..
        } => {
            validate_expr(object, semantic_info)?;
            if semantic_info
                .expression_type(object.span())
                .is_some_and(is_stage23_runtime_type)
            {
                return Err(unsupported_collection_shape(
                    *span,
                    format!("collection property `{property}`"),
                ));
            }
            if semantic_info
                .expression_type(object.span())
                .is_some_and(|ty| matches!(ty, ResolvedType::String))
                && matches!(
                    property.as_str(),
                    "length" | "byteLength" | "isEmpty" | "bytes"
                )
            {
                return Err(unsupported_string_runtime_shape(
                    *span,
                    format!("String intrinsic property `{property}`"),
                ));
            }
            Ok(())
        }
        Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            validate_expr(object, semantic_info)?;
            validate_arguments(args, semantic_info)?;
            if semantic_info
                .expression_type(object.span())
                .is_some_and(is_stage23_runtime_type)
            {
                return Err(unsupported_collection_shape(
                    *span,
                    format!("collection method `{method}`"),
                ));
            }
            Ok(())
        }
        Expr::FunctionCall { name, args, .. } if matches!(name.as_str(), "sprintf" | "printf") => {
            validate_php_format_call(args, semantic_info)
        }
        Expr::FunctionCall {
            name, args, span, ..
        } => {
            validate_arguments(args, semantic_info)?;
            if Builtin::from_name(name).is_some_and(Builtin::uses_bytes) {
                return Err(unsupported_collection_shape(
                    *span,
                    format!("byte I/O intrinsic `{name}`"),
                ));
            }
            Ok(())
        }
        Expr::New {
            class_type,
            args,
            shared,
            span,
        } => {
            // The PHP compatibility backend cannot express Doria's shared-ownership
            // families: PHP object references are not `SharedReference<T>`, and PHP's
            // refcounting does not implement the writable family's access rules.
            if *shared
                || crate::types::SharedHandleKind::from_source_name(&class_type.name).is_some()
            {
                return Err(unsupported_shared_ownership(*span));
            }
            validate_arguments(args, semantic_info)
        }
        Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            if class_name == "String" {
                validate_arguments(args, semantic_info)?;
                return Err(unsupported_string_runtime_shape(
                    *span,
                    format!("Unicode String operation `String::{method}`"),
                ));
            }
            if matches!(class_name.as_str(), "Bytes" | "Set") {
                validate_arguments(args, semantic_info)?;
                return Err(unsupported_collection_shape(
                    *span,
                    format!("collection constructor `{class_name}::{method}`"),
                ));
            }
            if (class_name == "Int" && method == "toFloat")
                || (class_name == "Float" && method == "toInt")
            {
                return Err(unsupported_numeric_shape(
                    *span,
                    format!("exact Doria conversion semantics for `{class_name}::{method}(...)`"),
                ));
            }
            if IntegerType::from_companion_name(class_name).is_some() && method == "from" {
                return Err(unsupported_integer_shape(
                    *span,
                    format!(
                        "checked Doria integer conversion semantics for `{class_name}::from(...)`"
                    ),
                ));
            }
            validate_arguments(args, semantic_info)
        }
        Expr::StaticMember { .. } => Ok(()),
        Expr::Grouped { expr, .. } => validate_expr(expr, semantic_info),
        Expr::IsType { expr, ty, span } => {
            validate_expr(expr, semantic_info)?;
            if matches!(
                semantic_info.expression_type(expr.span()),
                Some(ResolvedType::Mixed)
            ) {
                Ok(())
            } else {
                validate_type(ty, *span)
            }
        }
        Expr::Unary { op, expr, span } => {
            if *op == UnaryOp::Negate {
                if let Some(magnitude) = integer_literal_magnitude(expr) {
                    if magnitude <= (i64::MAX as u128) + 1 {
                        return Ok(());
                    }
                    return Err(unsupported_integer_shape(
                        *span,
                        "an integer literal outside PHP's signed integer range",
                    ));
                }
            }
            let feature = match op {
                UnaryOp::Not => None,
                UnaryOp::Negate if semantic_info.float_type(expr.span()).is_some() => None,
                UnaryOp::Negate => Some("checked integer overflow behavior for unary `-`"),
                UnaryOp::BitwiseNot => Some("fixed-width Doria bitwise semantics for `~`"),
            };
            if let Some(feature) = feature {
                return Err(unsupported_integer_shape(*span, feature));
            }
            validate_expr(expr, semantic_info)
        }
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => {
            validate_expr(left, semantic_info)?;
            validate_expr(right, semantic_info)?;
            if *op == BinaryOp::Concat {
                validate_display_expr(left, semantic_info)?;
                validate_display_expr(right, semantic_info)?;
            }
            let float_operands = matches!(
                (
                    semantic_info.float_type(left.span()),
                    semantic_info.float_type(right.span()),
                ),
                (Some(left), Some(right)) if left == right
            );
            let float32_operands = matches!(
                (
                    semantic_info.float_type(left.span()),
                    semantic_info.float_type(right.span()),
                ),
                (Some(FloatType::Float32), Some(FloatType::Float32))
            );
            let feature = match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div
                    if float32_operands =>
                {
                    return Err(unsupported_numeric_shape(
                        *span,
                        "Doria `float32` arithmetic with binary32 rounding after each operation",
                    ));
                }
                BinaryOp::Add if float_operands => None,
                BinaryOp::Sub if float_operands => None,
                BinaryOp::Mul if float_operands => None,
                BinaryOp::Div if float_operands => None,
                BinaryOp::Add => Some("checked integer overflow behavior for `+`"),
                BinaryOp::Sub => Some("checked integer overflow behavior for `-`"),
                BinaryOp::Mul => Some("checked integer overflow behavior for `*`"),
                BinaryOp::Div => Some("Doria integer division semantics for `/`"),
                BinaryOp::Mod => Some("Doria integer remainder semantics for `%`"),
                BinaryOp::ShiftLeft => Some("Doria integer shift semantics for `<<`"),
                BinaryOp::ShiftRight => Some("Doria integer shift semantics for `>>`"),
                BinaryOp::BitwiseAnd => Some("fixed-width Doria bitwise semantics for `&`"),
                BinaryOp::BitwiseXor => Some("fixed-width Doria bitwise semantics for `^`"),
                BinaryOp::BitwiseOr => Some("fixed-width Doria bitwise semantics for `|`"),
                BinaryOp::Concat
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Xor
                | BinaryOp::Coalesce => None,
            };
            if let Some(feature) = feature {
                return Err(unsupported_integer_shape(*span, feature));
            }
            Ok(())
        }
        Expr::Range { start, end, .. } => {
            validate_expr(start, semantic_info)?;
            validate_expr(end, semantic_info)
        }
        Expr::Match {
            scrutinee,
            arms,
            span,
            ..
        } => {
            validate_expr(scrutinee, semantic_info)?;
            let info = semantic_info
                .matches
                .get(&(span.start, span.end))
                .ok_or_else(|| BackendError::new("checked match has no semantic plan"))?;
            for (arm, arm_info) in arms.iter().zip(&info.arms) {
                match (&arm.pattern, &arm_info.pattern) {
                    (MatchPattern::Expression(pattern), ResolvedMatchPattern::Condition) => {
                        validate_expr(pattern, semantic_info)?;
                    }
                    (_, ResolvedMatchPattern::Constant(value)) => {
                        validate_const_value(value, arm.span)?;
                    }
                    _ => {}
                }
                if let Some(guard) = &arm.guard {
                    validate_expr(&guard.condition, semantic_info)?;
                }
                validate_expr(&arm.value, semantic_info)?;
            }
            Ok(())
        }
    }
}

fn validate_display_expr(expr: &Expr, semantic_info: &SemanticInfo) -> Result<(), BackendError> {
    if semantic_info.float_type(expr.span()).is_some() {
        return Err(unsupported_numeric_shape(
            expr.span(),
            "canonical float display formatting",
        ));
    }
    validate_expr(expr, semantic_info)
}

fn validate_php_format_call(
    args: &[Argument],
    semantic_info: &SemanticInfo,
) -> Result<(), BackendError> {
    let Some(format) = args.first().map(|argument| &argument.value) else {
        return Ok(());
    };
    validate_expr(format, semantic_info)?;
    let Expr::String { value, span } = format else {
        return validate_arguments(&args[1..], semantic_info);
    };
    let Ok(pieces) = format_string::parse(value, *span) else {
        return validate_arguments(&args[1..], semantic_info);
    };
    let conversions = pieces.iter().filter_map(|piece| match piece {
        FormatPiece::Argument { spec, .. } => Some(spec.conversion),
        FormatPiece::Literal(_) => None,
    });
    for (argument, conversion) in args[1..].iter().zip(conversions) {
        let argument = &argument.value;
        match conversion {
            FormatConversion::Display => validate_display_expr(argument, semantic_info)?,
            FormatConversion::Float => {
                return Err(unsupported_numeric_shape(
                    argument.span(),
                    "canonical `%f` float formatting",
                ));
            }
            _ => validate_expr(argument, semantic_info)?,
        }
    }
    Ok(())
}

fn validate_arguments(
    arguments: &[Argument],
    semantic_info: &SemanticInfo,
) -> Result<(), BackendError> {
    for argument in arguments {
        validate_expr(&argument.value, semantic_info)?;
    }
    Ok(())
}

/// Emit a call argument list. PHP 8 spells named arguments `name: value`
/// identically to Doria and evaluates arguments in written order, so the
/// arguments are emitted as written; no reordering is needed on this backend.
fn emit_arguments(arguments: &[Argument], scopes: &PhpNameScopes) -> String {
    arguments
        .iter()
        .map(|argument| match &argument.name {
            Some(name) => format!("{}: {}", name.text, emit_expr(&argument.value, scopes)),
            None => emit_expr(&argument.value, scopes),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// Instance initializers are currently emitted in PHP property-default syntax.
// Keep that syntax boundary as an allow-list so executable Doria expressions
// cannot reach a PHP constant-expression context unnoticed.
fn unsupported_php_property_default(
    expr: &Expr,
    semantic_info: &SemanticInfo,
) -> Option<(Span, &'static str)> {
    if is_payload_enum_expression(expr, semantic_info) {
        return None;
    }
    match expr {
        Expr::StaticMember {
            class_name,
            member,
            span,
        } if semantic_info
            .const_evaluation
            .values
            .contains_key(&ConstKey::Static {
                class_name: class_name.clone(),
                name: member.clone(),
            }) =>
        {
            Some((
                *span,
                "instance property initializers that read static properties",
            ))
        }
        Expr::Variable { span, .. } | Expr::This { span } => {
            Some((*span, "runtime values in instance property initializers"))
        }
        Expr::InterpolatedString { span, .. } => {
            Some((*span, "interpolated instance property initializers"))
        }
        Expr::Array { elements, .. } => elements.iter().find_map(|element| {
            element
                .key
                .as_ref()
                .and_then(|key| unsupported_php_property_default(key, semantic_info))
                .or_else(|| unsupported_php_property_default(&element.value, semantic_info))
        }),
        Expr::ArrayRepeat { span, .. } => {
            Some((*span, "sequence fill instance property initializers"))
        }
        Expr::Index { span, .. } => {
            Some((*span, "indexed access in instance property initializers"))
        }
        Expr::PropertyAccess { span, .. } => {
            Some((*span, "instance property initializers that read properties"))
        }
        Expr::MethodCall { span, .. } => {
            Some((*span, "instance property initializers that call methods"))
        }
        Expr::FunctionCall { span, .. } => {
            Some((*span, "instance property initializers that call functions"))
        }
        Expr::StaticCall { span, .. } => Some((
            *span,
            "instance property initializers that call static methods",
        )),
        Expr::New { span, .. } => Some((
            *span,
            "object construction in instance property initializers",
        )),
        Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } => {
            unsupported_php_property_default(expr, semantic_info)
        }
        Expr::Binary {
            op:
                BinaryOp::Div
                | BinaryOp::Concat
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual,
            span,
            ..
        } => Some((
            *span,
            "instance property initializers that require runtime helper calls",
        )),
        Expr::Binary { left, right, .. } => unsupported_php_property_default(left, semantic_info)
            .or_else(|| unsupported_php_property_default(right, semantic_info)),
        Expr::Range { span, .. } => {
            Some((*span, "range expressions in instance property initializers"))
        }
        Expr::Match { span, .. } => {
            Some((*span, "match expressions in instance property initializers"))
        }
        Expr::IsType { span, .. } => Some((*span, "type tests in instance property initializers")),
        Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::StaticMember { .. } => None,
    }
}

fn is_payload_enum_expression(expr: &Expr, semantic_info: &SemanticInfo) -> bool {
    let Some(ResolvedType::Enum(enum_type)) = semantic_info.expression_type(expr.span()) else {
        return false;
    };
    semantic_info
        .enums
        .iter()
        .find(|definition| definition.id == enum_type.id)
        .is_some_and(|definition| definition.cases.iter().any(|case| !case.payload.is_empty()))
}

fn integer_literal_magnitude(expr: &Expr) -> Option<u128> {
    match expr {
        Expr::Int { value, .. } => parse_decimal_magnitude(value),
        Expr::Grouped { expr, .. } => integer_literal_magnitude(expr),
        _ => None,
    }
}

fn unsupported_integer_shape(span: Span, feature: impl Into<String>) -> BackendError {
    unsupported_numeric_shape(span, feature)
}

fn unsupported_numeric_shape(span: Span, feature: impl Into<String>) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        PHP_INTEGER_UNSUPPORTED_CODE,
        format!(
            "PHP compatibility backend cannot preserve {} exactly; use the `native` or `debug` target for this valid Doria program",
            feature.into()
        ),
        span,
    )])
}

fn unsupported_ownership_shape(span: Span, feature: impl Into<String>) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        PHP_OWNERSHIP_UNSUPPORTED_CODE,
        format!(
            "PHP compatibility backend cannot preserve {} exactly; use the `native` or `debug` target for this valid Doria program",
            feature.into()
        ),
        span,
    )])
}

fn unsupported_constant_shape(span: Span, feature: impl Into<String>) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        PHP_CONSTANT_UNSUPPORTED_CODE,
        format!(
            "PHP compatibility backend cannot preserve {} exactly; use the `native` or `debug` target for this valid Doria program",
            feature.into()
        ),
        span,
    )])
}

fn unsupported_collection_shape(span: Span, feature: impl Into<String>) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        PHP_COLLECTION_UNSUPPORTED_CODE,
        format!(
            "PHP compatibility backend cannot preserve {} exactly; use the `native` or `debug` target for this valid Doria program",
            feature.into()
        ),
        span,
    )])
}

fn unsupported_string_runtime_shape(span: Span, feature: impl Into<String>) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        PHP_STRING_RUNTIME_UNSUPPORTED_CODE,
        format!(
            "PHP compatibility backend cannot preserve {} exactly without optional extensions; use the `native` or `debug` target for this valid Doria program",
            feature.into()
        ),
        span,
    )])
}

fn is_stage23_runtime_type(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Bytes
        | ResolvedType::TypedArray(_)
        | ResolvedType::List(_)
        | ResolvedType::Dictionary(_, _)
        | ResolvedType::Set(_) => true,
        ResolvedType::Nullable(inner) => is_stage23_runtime_type(inner),
        _ => false,
    }
}

#[derive(Debug, Clone, Default)]
struct PhpNameScopes {
    scopes: Vec<HashMap<String, String>>,
    mixed_bindings: Vec<HashSet<String>>,
    used_php_names: HashSet<String>,
    next_mangled_id: usize,
    static_properties: HashSet<(String, String)>,
    payload_unit_cases: HashSet<(String, String)>,
    payload_top_constants: HashSet<String>,
    payload_class_constants: HashSet<(String, String)>,
    payload_enum_expressions: HashSet<(usize, usize)>,
    payload_case_tags: HashMap<crate::enums::EnumCaseId, u32>,
    matches: HashMap<(usize, usize), MatchSemanticInfo>,
    expression_types: HashMap<(usize, usize), ResolvedType>,
    type_test_types: HashMap<(usize, usize), ResolvedType>,
    mixed_box_types: HashMap<(usize, usize), ResolvedType>,
    const_evaluation: Evaluation,
}

impl PhpNameScopes {
    fn new(
        static_properties: HashSet<(String, String)>,
        payload_unit_cases: HashSet<(String, String)>,
        payload_top_constants: HashSet<String>,
        payload_class_constants: HashSet<(String, String)>,
        payload_enum_expressions: HashSet<(usize, usize)>,
    ) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            mixed_bindings: vec![HashSet::new()],
            used_php_names: HashSet::new(),
            next_mangled_id: 0,
            static_properties,
            payload_unit_cases,
            payload_top_constants,
            payload_class_constants,
            payload_enum_expressions,
            payload_case_tags: HashMap::new(),
            matches: HashMap::new(),
            expression_types: HashMap::new(),
            type_test_types: HashMap::new(),
            mixed_box_types: HashMap::new(),
            const_evaluation: Evaluation::default(),
        }
    }

    fn expression_scope(&self) -> Self {
        let mut scopes = Self::new(
            self.static_properties.clone(),
            self.payload_unit_cases.clone(),
            self.payload_top_constants.clone(),
            self.payload_class_constants.clone(),
            self.payload_enum_expressions.clone(),
        );
        scopes.payload_case_tags = self.payload_case_tags.clone();
        scopes.matches = self.matches.clone();
        scopes.expression_types = self.expression_types.clone();
        scopes.type_test_types = self.type_test_types.clone();
        scopes.mixed_box_types = self.mixed_box_types.clone();
        scopes.const_evaluation = self.const_evaluation.clone();
        scopes
    }

    fn is_static_property(&self, class_name: &str, member: &str) -> bool {
        self.static_properties
            .contains(&(class_name.to_string(), member.to_string()))
    }

    fn is_payload_unit_case(&self, enum_name: &str, case_name: &str) -> bool {
        self.payload_unit_cases
            .contains(&(enum_name.to_string(), case_name.to_string()))
    }

    fn is_payload_top_constant(&self, name: &str) -> bool {
        self.payload_top_constants.contains(name)
    }

    fn is_payload_class_constant(&self, class_name: &str, name: &str) -> bool {
        self.payload_class_constants
            .contains(&(class_name.to_string(), name.to_string()))
    }

    fn is_payload_enum_expression(&self, expr: &Expr) -> bool {
        self.payload_enum_expressions
            .contains(&(expr.span().start, expr.span().end))
    }

    fn push(&mut self) {
        self.scopes.push(HashMap::new());
        self.mixed_bindings.push(HashSet::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
        self.mixed_bindings.pop();
    }

    fn declare(&mut self, name: &str) -> String {
        let shadows_outer = self.lookup(name).is_some();
        let php_name = if shadows_outer || self.used_php_names.contains(name) {
            self.next_mangled_name(name)
        } else {
            name.to_string()
        };
        self.insert_current(name, php_name.clone());
        php_name
    }

    fn declare_or_reuse_current(&mut self, name: &str) -> String {
        if let Some(existing) = self.scopes.last().and_then(|scope| scope.get(name)) {
            return existing.clone();
        }
        self.declare(name)
    }

    fn declare_unmangled(&mut self, name: &str) -> String {
        let php_name = name.to_string();
        self.insert_current(name, php_name.clone());
        php_name
    }

    fn fresh_temp(&mut self, prefix: &str) -> String {
        loop {
            self.next_mangled_id += 1;
            let candidate = format!("{prefix}__doria{}", self.next_mangled_id);
            if !self.used_php_names.contains(&candidate) {
                self.used_php_names.insert(candidate.clone());
                return candidate;
            }
        }
    }

    fn expression_temp(&self, prefix: &str, span: Span) -> String {
        let base = format!("{prefix}{}", span.start);
        if !self.used_php_names.contains(&base) {
            return base;
        }
        (1..)
            .map(|suffix| format!("{base}_{suffix}"))
            .find(|candidate| !self.used_php_names.contains(candidate))
            .expect("an unused PHP expression temporary name must exist")
    }

    fn captured_php_names(&self) -> Vec<String> {
        let mut names = self
            .scopes
            .iter()
            .flat_map(|scope| scope.values().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn lookup(&self, name: &str) -> Option<&str> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .map(String::as_str)
    }

    fn php_name(&self, name: &str) -> String {
        self.lookup(name).unwrap_or(name).to_string()
    }

    fn mark_mixed(&mut self, name: &str) {
        if let Some(scope) = self.mixed_bindings.last_mut() {
            scope.insert(name.to_string());
        }
    }

    fn is_mixed_binding(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .zip(self.mixed_bindings.iter().rev())
            .find_map(|(scope, mixed)| scope.contains_key(name).then(|| mixed.contains(name)))
            .unwrap_or(false)
    }

    fn insert_current(&mut self, name: &str, php_name: String) {
        self.used_php_names.insert(php_name.clone());
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), php_name);
        }
    }

    fn next_mangled_name(&mut self, name: &str) -> String {
        loop {
            self.next_mangled_id += 1;
            let candidate = format!("{name}__doria{}", self.next_mangled_id);
            if !self.used_php_names.contains(&candidate) {
                return candidate;
            }
        }
    }
}

fn emit_item(
    item: &Item,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    match item {
        Item::Class(class_decl) => emit_class(class_decl, semantic_info, output, indent, scopes),
        Item::Enum(enum_decl) => emit_enum(enum_decl, output, indent, scopes),
        Item::Function(function) => {
            emit_function(function, semantic_info, output, indent, false, scopes, &[])
        }
        Item::Constant(constant) => emit_constant(
            constant,
            None,
            &semantic_info.const_evaluation,
            output,
            indent,
        ),
        Item::Statement(statement) => emit_statement(statement, output, indent, scopes),
    }
}

fn emit_enum(enum_decl: &EnumDecl, output: &mut String, indent: usize, scopes: &PhpNameScopes) {
    if enum_decl.cases.iter().any(|case| !case.payload.is_empty()) {
        emit_payload_enum(enum_decl, output, indent);
        return;
    }
    write_indent(output, indent);
    output.push_str("enum ");
    output.push_str(&enum_decl.name);
    if let Some(backing) = &enum_decl.backing_type {
        output.push_str(": ");
        output.push_str(&backing.name);
    }
    output.push('\n');
    writeln(output, indent, "{");
    for case in &enum_decl.cases {
        write_indent(output, indent + 1);
        output.push_str("case ");
        output.push_str(&case.name);
        if let Some(value) = &case.backing_value {
            output.push_str(" = ");
            output.push_str(&emit_expr(value, scopes));
        }
        output.push_str(";\n");
    }
    writeln(output, indent, "}");
}

fn emit_payload_enum(enum_decl: &EnumDecl, output: &mut String, indent: usize) {
    writeln(
        output,
        indent,
        &format!(
            "final class {} implements __DoriaValueEquatable",
            enum_decl.name
        ),
    );
    writeln(output, indent, "{");
    writeln(output, indent + 1, "private int $__doriaTag;");
    writeln(output, indent + 1, "private array $__doriaPayload;");
    output.push('\n');
    writeln(
        output,
        indent + 1,
        "private function __construct(int $tag, array $payload)",
    );
    writeln(output, indent + 1, "{");
    writeln(output, indent + 2, "$this->__doriaTag = $tag;");
    writeln(output, indent + 2, "$this->__doriaPayload = $payload;");
    writeln(output, indent + 1, "}");

    output.push('\n');
    writeln(
        output,
        indent + 1,
        "public function __doriaMatchesCase(int $tag): bool",
    );
    writeln(output, indent + 1, "{");
    writeln(output, indent + 2, "return $this->__doriaTag === $tag;");
    writeln(output, indent + 1, "}");
    output.push('\n');
    writeln(
        output,
        indent + 1,
        "public function __doriaPayloadAt(int $index): mixed",
    );
    writeln(output, indent + 1, "{");
    writeln(output, indent + 2, "return $this->__doriaPayload[$index];");
    writeln(output, indent + 1, "}");

    for (tag, case) in enum_decl.cases.iter().enumerate() {
        output.push('\n');
        let method = php_payload_case_method(&case.name, !case.payload.is_empty());
        write_indent(output, indent + 1);
        output.push_str("public static function ");
        output.push_str(&method);
        output.push('(');
        output.push_str(
            &case
                .payload
                .iter()
                .map(|field| format!("{} ${}", php_type(&field.ty), field.name))
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push_str("): self\n");
        writeln(output, indent + 1, "{");
        writeln(
            output,
            indent + 2,
            &format!(
                "return new self({tag}, [{}]);",
                case.payload
                    .iter()
                    .map(|field| format!("${}", field.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
        writeln(output, indent + 1, "}");
    }

    output.push('\n');
    writeln(
        output,
        indent + 1,
        "public function __doriaEquals(mixed $other): bool",
    );
    writeln(output, indent + 1, "{");
    writeln(
        output,
        indent + 2,
        "if (!$other instanceof self || $this->__doriaTag !== $other->__doriaTag) { return false; }",
    );
    writeln(
        output,
        indent + 2,
        "foreach ($this->__doriaPayload as $index => $value) {",
    );
    writeln(
        output,
        indent + 3,
        "if (!__doria_equal($value, $other->__doriaPayload[$index])) { return false; }",
    );
    writeln(output, indent + 2, "}");
    writeln(output, indent + 2, "return true;");
    writeln(output, indent + 1, "}");
    output.push('\n');
    writeln(output, indent + 1, "public function __destruct()");
    writeln(output, indent + 1, "{");
    writeln(
        output,
        indent + 2,
        "for ($index = count($this->__doriaPayload) - 1; $index >= 0; --$index) { unset($this->__doriaPayload[$index]); }",
    );
    writeln(output, indent + 1, "}");
    writeln(output, indent, "}");
}

fn emit_class(
    class_decl: &ClassDecl,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    let instance_initializers = class_decl
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Property(property)
                if !property.is_static
                    && property
                        .initializer
                        .as_ref()
                        .is_some_and(|value| is_payload_enum_expression(value, semantic_info)) =>
            {
                Some((
                    property.name.as_str(),
                    property.initializer.as_ref().unwrap(),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut has_constructor = false;
    let implements = if class_decl
        .implements
        .iter()
        .any(|interface| interface == "Displayable")
    {
        " implements __DoriaDisplayable"
    } else {
        ""
    };
    writeln(
        output,
        indent,
        &format!("class {}{implements}", class_decl.name),
    );
    writeln(output, indent, "{");
    for member in &class_decl.members {
        match member {
            ClassMember::Property(property) => emit_property(
                property,
                &class_decl.name,
                &semantic_info.const_evaluation,
                semantic_info,
                output,
                indent + 1,
                scopes,
            ),
            ClassMember::Method(method) => {
                has_constructor |= method.name == "__construct";
                let initializers = if method.name == "__construct" {
                    instance_initializers.as_slice()
                } else {
                    &[]
                };
                emit_function(
                    method,
                    semantic_info,
                    output,
                    indent + 1,
                    true,
                    scopes,
                    initializers,
                )
            }
            ClassMember::Constant(constant) => emit_constant(
                constant,
                Some(&class_decl.name),
                &semantic_info.const_evaluation,
                output,
                indent + 1,
            ),
        }
        output.push('\n');
    }
    if !has_constructor && !instance_initializers.is_empty() {
        writeln(output, indent + 1, "public function __construct()");
        writeln(output, indent + 1, "{");
        for (name, initializer) in &instance_initializers {
            writeln(
                output,
                indent + 2,
                &format!(
                    "$this->{name} = {};",
                    emit_expr(initializer, &scopes.expression_scope())
                ),
            );
        }
        writeln(output, indent + 1, "}");
        output.push('\n');
    }
    writeln(output, indent, "}");
    let static_payload_initializers = class_decl.members.iter().filter_map(|member| match member {
        ClassMember::Property(property)
            if property.is_static
                && matches!(
                    semantic_info
                        .const_evaluation
                        .values
                        .get(&ConstKey::Static {
                            class_name: class_decl.name.clone(),
                            name: property.name.clone(),
                        })
                        .map(|value| &value.value),
                    Some(ConstValue::PayloadEnum(_))
                ) =>
        {
            Some(property)
        }
        _ => None,
    });
    let static_payload_initializers = static_payload_initializers.collect::<Vec<_>>();
    if !static_payload_initializers.is_empty() {
        writeln(
            output,
            indent,
            "(\\Closure::bind(static function (): void {",
        );
        for property in static_payload_initializers {
            writeln(
                output,
                indent + 1,
                &format!(
                    "self::${} = {};",
                    property.name,
                    emit_const_value(
                        evaluated_value(
                            &semantic_info.const_evaluation,
                            &ConstKey::Static {
                                class_name: class_decl.name.clone(),
                                name: property.name.clone(),
                            },
                        ),
                        &semantic_info.const_evaluation,
                    )
                ),
            );
        }
        writeln(
            output,
            indent,
            &format!("}}, null, {}::class))();", class_decl.name),
        );
    }
}

fn emit_property(
    property: &PropertyDecl,
    class_name: &str,
    evaluation: &Evaluation,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    shared_scopes: &PhpNameScopes,
) {
    let visibility = emit_member_access(&property.access);
    let ty = php_type(&property.ty);
    write_indent(output, indent);
    output.push_str(visibility);
    output.push(' ');
    if property.is_static {
        output.push_str("static ");
    }
    output.push_str(&ty);
    output.push_str(" $");
    output.push_str(&property.name);
    if let Some(initializer) = &property.initializer {
        let payload_initializer = if property.is_static {
            matches!(
                evaluation
                    .values
                    .get(&ConstKey::Static {
                        class_name: class_name.to_string(),
                        name: property.name.clone(),
                    })
                    .map(|value| &value.value),
                Some(ConstValue::PayloadEnum(_))
            )
        } else {
            is_payload_enum_expression(initializer, semantic_info)
        };
        if !payload_initializer {
            output.push_str(" = ");
        }
        if property.is_static && !payload_initializer {
            output.push_str(&emit_const_value(
                evaluated_value(
                    evaluation,
                    &ConstKey::Static {
                        class_name: class_name.to_string(),
                        name: property.name.clone(),
                    },
                ),
                evaluation,
            ));
        } else if !payload_initializer {
            output.push_str(&emit_expr(initializer, &shared_scopes.expression_scope()));
        }
    }
    output.push_str(";\n");
}

fn emit_constant(
    constant: &ConstDecl,
    class_name: Option<&str>,
    evaluation: &Evaluation,
    output: &mut String,
    indent: usize,
) {
    let key = class_name.map_or_else(
        || ConstKey::TopLevel(constant.name.clone()),
        |class_name| ConstKey::Class {
            class_name: class_name.to_string(),
            name: constant.name.clone(),
        },
    );
    let value = evaluated_value(evaluation, &key);
    if let ConstValue::PayloadEnum(payload) = value {
        let (enum_name, _) = evaluation
            .payload_case_name(payload.enum_id, payload.case_id)
            .expect("checked payload constant must name a payload enum");
        write_indent(output, indent);
        if class_name.is_some() {
            output.push_str(emit_member_access(&constant.access));
            output.push(' ');
        }
        output.push_str(if class_name.is_some() {
            "static function "
        } else {
            "function "
        });
        output.push_str(&class_name.map_or_else(
            || format!("__doria_const_{}", constant.name),
            |_| format!("__doriaConst{}", constant.name),
        ));
        output.push_str("(): ");
        output.push_str(enum_name);
        output.push('\n');
        writeln(output, indent, "{");
        writeln(output, indent + 1, "static $value = null;");
        writeln(
            output,
            indent + 1,
            &format!("return $value ??= {};", emit_const_value(value, evaluation)),
        );
        writeln(output, indent, "}");
        return;
    }
    write_indent(output, indent);
    if class_name.is_some() {
        output.push_str(emit_member_access(&constant.access));
        output.push(' ');
    }
    output.push_str("const ");
    output.push_str(&class_name.map_or_else(
        || php_top_level_constant_name(&constant.name),
        |_| constant.name.clone(),
    ));
    output.push_str(" = ");
    output.push_str(&emit_const_value(value, evaluation));
    output.push_str(";\n");
}

fn evaluated_value<'a>(evaluation: &'a Evaluation, key: &ConstKey) -> &'a ConstValue {
    &evaluation
        .values
        .get(key)
        .unwrap_or_else(|| {
            panic!(
                "checked declaration `{}` has no evaluated value",
                key.display()
            )
        })
        .value
}

fn emit_const_value(value: &ConstValue, evaluation: &Evaluation) -> String {
    match value {
        ConstValue::Integer(value)
            if value.ty.is_default_int() && value.mathematical_value() == i64::MIN as i128 =>
        {
            "(-9223372036854775807 - 1)".to_string()
        }
        ConstValue::Integer(value) => value.display(),
        ConstValue::Float(value) => {
            let value = value.display();
            match value.as_str() {
                "NaN" => "NAN".to_string(),
                "Infinity" => "INF".to_string(),
                "-Infinity" => "-INF".to_string(),
                _ if !value.contains('.') && !value.contains('e') && !value.contains('E') => {
                    format!("{value}.0")
                }
                _ => value,
            }
        }
        ConstValue::String(value) => emit_php_string_literal(value),
        ConstValue::Bool(value) => value.to_string(),
        ConstValue::Null => "null".to_string(),
        ConstValue::Enum(value) => evaluation
            .enum_cases
            .iter()
            .find_map(|((enum_name, case_name), candidate)| {
                (*candidate == *value).then(|| format!("{enum_name}::{case_name}"))
            })
            .expect("checked enum constant must name a declared case"),
        ConstValue::PayloadEnum(value) => {
            let (enum_name, case_name) = evaluation
                .payload_case_name(value.enum_id, value.case_id)
                .expect("checked payload enum constant must name a declared case");
            let method = php_payload_case_method(case_name, !value.fields.is_empty());
            format!(
                "{enum_name}::{method}({})",
                value
                    .fields
                    .iter()
                    .map(|field| emit_const_value(field, evaluation))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

fn emit_function(
    function: &FunctionDecl,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    is_method: bool,
    shared_scopes: &PhpNameScopes,
    property_initializers: &[(&str, &Expr)],
) {
    let mut scopes = shared_scopes.expression_scope();
    for param in &function.params {
        scopes.declare_unmangled(&param.name);
        if param.ty.name == "mixed" {
            scopes.mark_mixed(&param.name);
        }
    }

    write_indent(output, indent);
    if is_method {
        output.push_str(emit_member_access(&function.access));
        output.push(' ');
        if function.is_static {
            output.push_str("static ");
        }
    }
    output.push_str("function ");
    output.push_str(&function.name);
    output.push('(');
    output.push_str(
        &function
            .params
            .iter()
            .enumerate()
            .map(|(parameter_index, param)| {
                emit_param(
                    param,
                    semantic_info.parameter_defaults.get(&ParameterDefaultKey {
                        function_start: function.span.start,
                        parameter_index,
                    }),
                    &semantic_info.const_evaluation,
                    &scopes,
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push(')');
    let is_lifecycle_method =
        is_method && matches!(function.name.as_str(), "__construct" | "__destruct");
    if let Some(return_type) = function
        .return_type
        .as_ref()
        .filter(|_| !is_lifecycle_method)
    {
        output.push_str(": ");
        output.push_str(&php_type(return_type));
    }
    output.push('\n');
    writeln(output, indent, "{");
    scopes.push();
    for (name, initializer) in property_initializers {
        writeln(
            output,
            indent + 1,
            &format!("$this->{name} = {};", emit_expr(initializer, &scopes)),
        );
    }
    for (parameter_index, param) in function.params.iter().enumerate() {
        let Some(default @ ConstValue::PayloadEnum(_)) =
            semantic_info.parameter_defaults.get(&ParameterDefaultKey {
                function_start: function.span.start,
                parameter_index,
            })
        else {
            continue;
        };
        let name = scopes.php_name(&param.name);
        writeln(output, indent + 1, &format!("if (${name} === []) {{"));
        writeln(
            output,
            indent + 2,
            &format!(
                "${name} = {};",
                emit_const_value(default, &semantic_info.const_evaluation)
            ),
        );
        if param.promoted_access.is_some() {
            writeln(output, indent + 2, &format!("$this->{name} = ${name};"));
        }
        writeln(output, indent + 1, "}");
    }
    for statement in &function.body.statements {
        emit_statement(statement, output, indent + 1, &mut scopes);
    }
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_param(
    param: &Param,
    evaluated_default: Option<&ConstValue>,
    evaluation: &Evaluation,
    scopes: &PhpNameScopes,
) -> String {
    let mut output = String::new();
    if let Some(access) = &param.promoted_access {
        output.push_str(emit_member_access(access));
        output.push(' ');
    }
    let payload_default = matches!(evaluated_default, Some(ConstValue::PayloadEnum(_)));
    if payload_default {
        let payload_type = php_type(&param.ty);
        output.push_str(payload_type.trim_start_matches('?'));
        output.push_str("|array");
        if param.ty.nullable {
            output.push_str("|null");
        }
    } else {
        output.push_str(&php_type(&param.ty));
    }
    output.push_str(" $");
    output.push_str(&scopes.php_name(&param.name));
    if param.default.is_some() {
        output.push_str(" = ");
        let default =
            evaluated_default.expect("checked Copy parameter default must have an evaluated value");
        if payload_default {
            output.push_str("[]");
        } else {
            output.push_str(&emit_const_value(default, evaluation));
        }
    }
    output
}

fn php_top_level_constant_name(name: &str) -> String {
    format!("__DORIA_CONST_{name}")
}

fn php_payload_case_method(case_name: &str, has_payload: bool) -> String {
    if has_payload {
        case_name.to_string()
    } else {
        format!("__doriaCase{case_name}")
    }
}

fn emit_block(block: &Block, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    writeln(output, indent, "{");
    scopes.push();
    for statement in &block.statements {
        emit_statement(statement, output, indent + 1, scopes);
    }
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_statement(
    statement: &Stmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    match statement {
        Stmt::Block(block) => emit_block(block, output, indent, scopes),
        Stmt::VarDecl(decl) => {
            let initializer = emit_expr(&decl.initializer, scopes);
            let binding_is_mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                || (decl.ty.is_none()
                    && matches!(
                        scopes
                            .expression_types
                            .get(&(decl.initializer.span().start, decl.initializer.span().end)),
                        Some(ResolvedType::Mixed)
                    ));
            if decl.bindings.len() == 1 {
                let php_name = scopes.declare(&decl.bindings[0].name);
                if binding_is_mixed {
                    scopes.mark_mixed(&decl.bindings[0].name);
                }
                writeln(output, indent, &format!("${php_name} = {initializer};"));
            } else {
                let temporary = scopes.fresh_temp("__doria_grouped_value");
                writeln(output, indent, &format!("${temporary} = {initializer};"));
                for binding in &decl.bindings {
                    let php_name = scopes.declare(&binding.name);
                    if binding_is_mixed {
                        scopes.mark_mixed(&binding.name);
                    }
                    writeln(output, indent, &format!("${php_name} = ${temporary};"));
                }
                writeln(output, indent, &format!("unset(${temporary});"));
            }
        }
        Stmt::Assignment(assignment) => {
            if assignment.op == AssignOp::DivAssign {
                let target = emit_assignment_target(&assignment.target, scopes);
                writeln(
                    output,
                    indent,
                    &format!(
                        "{target} = fdiv({target}, {});",
                        emit_expr(&assignment.value, scopes)
                    ),
                );
                return;
            }
            let op = match assignment.op {
                AssignOp::Assign => "=",
                AssignOp::AddAssign => "+=",
                AssignOp::SubAssign => "-=",
                AssignOp::MulAssign => "*=",
                AssignOp::DivAssign => "/=",
                AssignOp::ModAssign => "%=",
                AssignOp::ShiftLeftAssign => "<<=",
                AssignOp::ShiftRightAssign => ">>=",
                AssignOp::BitwiseAndAssign => "&=",
                AssignOp::BitwiseOrAssign => "|=",
                AssignOp::BitwiseXorAssign => "^=",
            };
            writeln(
                output,
                indent,
                &format!(
                    "{} {} {};",
                    emit_assignment_target(&assignment.target, scopes),
                    op,
                    emit_expr(&assignment.value, scopes)
                ),
            );
        }
        Stmt::Echo { expr, span } => {
            writeln(
                output,
                indent,
                &format!(
                    "__doria_write_stdout(__doria_display({}), {}, {});",
                    emit_expr(expr, scopes),
                    span.start,
                    span.end,
                ),
            );
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                writeln(
                    output,
                    indent,
                    &format!("return {};", emit_expr(expr, scopes)),
                );
            } else {
                writeln(output, indent, "return;");
            }
        }
        Stmt::If(if_stmt) => emit_if(if_stmt, output, indent, "if", scopes),
        Stmt::While(while_stmt) => {
            write_indent(output, indent);
            output.push_str("while (");
            output.push_str(&emit_expr(&while_stmt.condition, scopes));
            output.push_str(")\n");
            emit_block(&while_stmt.body, output, indent, scopes);
        }
        Stmt::For(for_stmt) => emit_for(for_stmt, output, indent, scopes),
        Stmt::Break { .. } => {
            writeln(output, indent, "break;");
        }
        Stmt::Continue { .. } => {
            writeln(output, indent, "continue;");
        }
        Stmt::Foreach(foreach) => emit_foreach(foreach, output, indent, scopes),
        Stmt::Increment(increment) => {
            writeln(
                output,
                indent,
                &format!("{};", emit_increment(increment, scopes)),
            );
        }
        Stmt::Expr { expr, .. } => {
            if let Expr::FunctionCall { name, args, span } = expr {
                if name == "panic" && args.len() == 1 {
                    emit_panic(&args[0].value, *span, output, indent, scopes);
                    return;
                }
            }
            writeln(output, indent, &format!("{};", emit_expr(expr, scopes)));
        }
    }
}

fn emit_panic(
    message: &Expr,
    span: Span,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    writeln(
        output,
        indent,
        &format!(
            "__doria_panic(\"P1000\", {}, {}, {});",
            span.start,
            span.end,
            emit_expr(message, scopes),
        ),
    );
}

fn emit_for(for_stmt: &ForStmt, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    scopes.push();
    let initializer = for_stmt
        .initializer
        .as_ref()
        .map(|initializer| emit_for_initializer(initializer, scopes))
        .unwrap_or_default();
    let condition = for_stmt
        .condition
        .as_ref()
        .map(|condition| emit_expr(condition, scopes))
        .unwrap_or_default();
    let increment = for_stmt
        .increment
        .as_ref()
        .map(|increment| emit_for_increment(increment, scopes))
        .unwrap_or_default();

    write_indent(output, indent);
    output.push_str("for (");
    output.push_str(&initializer);
    output.push_str("; ");
    output.push_str(&condition);
    output.push_str("; ");
    output.push_str(&increment);
    output.push_str(")\n");
    emit_block(&for_stmt.body, output, indent, scopes);
    scopes.pop();
}

fn emit_for_initializer(initializer: &ForInitializer, scopes: &mut PhpNameScopes) -> String {
    match initializer {
        ForInitializer::VarDecl(decl) => {
            let initializer = emit_expr(&decl.initializer, scopes);
            let binding_is_mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                || (decl.ty.is_none()
                    && matches!(
                        scopes
                            .expression_types
                            .get(&(decl.initializer.span().start, decl.initializer.span().end)),
                        Some(ResolvedType::Mixed)
                    ));
            if decl.bindings.len() == 1 {
                let php_name = scopes.declare(&decl.bindings[0].name);
                if binding_is_mixed {
                    scopes.mark_mixed(&decl.bindings[0].name);
                }
                format!("${php_name} = {initializer}")
            } else {
                let temporary = scopes.fresh_temp("__doria_grouped_value");
                let mut expressions = vec![format!("${temporary} = {initializer}")];
                expressions.extend(decl.bindings.iter().map(|binding| {
                    let php_name = scopes.declare(&binding.name);
                    if binding_is_mixed {
                        scopes.mark_mixed(&binding.name);
                    }
                    format!("${php_name} = ${temporary}")
                }));
                // PHP's for-initializer accepts a comma-separated expression
                // list. Clearing the collision-safe temporary releases its
                // string handle after the ordered copies, matching Doria's
                // statement-end temporary lifetime without a chained assign.
                expressions.push(format!("${temporary} = null"));
                expressions.join(", ")
            }
        }
        ForInitializer::Assignment(assignment) => emit_assignment(assignment, scopes),
    }
}

fn emit_for_increment(increment: &ForIncrement, scopes: &PhpNameScopes) -> String {
    match increment {
        ForIncrement::Increment(increment) => emit_increment(increment, scopes),
        ForIncrement::Assignment(assignment) => emit_assignment(assignment, scopes),
    }
}

fn emit_assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> String {
    if assignment.op == AssignOp::DivAssign {
        let target = emit_assignment_target(&assignment.target, scopes);
        return format!(
            "{target} = fdiv({target}, {})",
            emit_expr(&assignment.value, scopes)
        );
    }
    let op = match assignment.op {
        AssignOp::Assign => "=",
        AssignOp::AddAssign => "+=",
        AssignOp::SubAssign => "-=",
        AssignOp::MulAssign => "*=",
        AssignOp::DivAssign => "/=",
        AssignOp::ModAssign => "%=",
        AssignOp::ShiftLeftAssign => "<<=",
        AssignOp::ShiftRightAssign => ">>=",
        AssignOp::BitwiseAndAssign => "&=",
        AssignOp::BitwiseOrAssign => "|=",
        AssignOp::BitwiseXorAssign => "^=",
    };
    format!(
        "{} {} {}",
        emit_assignment_target(&assignment.target, scopes),
        op,
        emit_expr(&assignment.value, scopes)
    )
}

fn emit_increment(increment: &IncrementStmt, scopes: &PhpNameScopes) -> String {
    let target = emit_assignment_target(&increment.target, scopes);
    let op = match increment.op {
        IncrementOp::Increment => "++",
        IncrementOp::Decrement => "--",
    };
    match increment.position {
        IncrementPosition::Pre => format!("{op}{target}"),
        IncrementPosition::Post => format!("{target}{op}"),
    }
}

fn emit_assignment_target(expr: &Expr, scopes: &PhpNameScopes) -> String {
    match expr {
        Expr::Grouped { expr, .. } => emit_assignment_target(expr, scopes),
        _ => emit_expr(expr, scopes),
    }
}

fn emit_if(
    if_stmt: &IfStmt,
    output: &mut String,
    indent: usize,
    keyword: &str,
    scopes: &mut PhpNameScopes,
) {
    write_indent(output, indent);
    output.push_str(keyword);
    output.push_str(" (");
    output.push_str(&emit_expr(&if_stmt.condition, scopes));
    output.push_str(")\n");
    emit_block(&if_stmt.then_block, output, indent, scopes);

    if let Some(else_branch) = &if_stmt.else_branch {
        match else_branch {
            ElseBranch::If(else_if) => emit_if(else_if, output, indent, "else if", scopes),
            ElseBranch::Block(block) => {
                write_indent(output, indent);
                output.push_str("else\n");
                emit_block(block, output, indent, scopes);
            }
        }
    }
}

fn emit_foreach(
    foreach: &ForeachStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if let Some((start, end, inclusive)) = grouped_range_expr(&foreach.iterable) {
        emit_range_foreach(foreach, start, end, inclusive, output, indent, scopes);
        return;
    }

    if let Some((dictionary, projection)) = dictionary_foreach_projection(&foreach.iterable) {
        let iterable = format!(
            "__doria_collection_projection({}, {})",
            emit_expr(dictionary, scopes),
            if projection == DictionaryForeachProjection::Keys {
                "true"
            } else {
                "false"
            }
        );
        scopes.push();
        let value_name = scopes.declare(&foreach.value.name);

        write_indent(output, indent);
        output.push_str("foreach (");
        output.push_str(&iterable);
        output.push_str(" as ");
        if foreach.value.writable {
            output.push('&');
        }
        output.push('$');
        output.push_str(&value_name);
        output.push_str(")\n");
        writeln(output, indent, "{");
        for statement in &foreach.body.statements {
            emit_statement(statement, output, indent + 1, scopes);
        }
        scopes.pop();
        writeln(output, indent, "}");
        return;
    }

    let iterable = emit_expr(&foreach.iterable, scopes);
    scopes.push();
    let key_name = foreach.key.as_ref().map(|key| scopes.declare(&key.name));
    let value_name = scopes.declare(&foreach.value.name);

    write_indent(output, indent);
    output.push_str("foreach (");
    output.push_str(&iterable);
    output.push_str(" as ");
    if let Some(key_name) = key_name {
        output.push('$');
        output.push_str(&key_name);
        output.push_str(" => ");
    }
    if foreach.value.writable {
        output.push('&');
    }
    output.push('$');
    output.push_str(&value_name);
    output.push_str(")\n");
    writeln(output, indent, "{");
    for statement in &foreach.body.statements {
        emit_statement(statement, output, indent + 1, scopes);
    }
    scopes.pop();
    writeln(output, indent, "}");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DictionaryForeachProjection {
    Keys,
    Values,
}

fn dictionary_foreach_projection(expr: &Expr) -> Option<(&Expr, DictionaryForeachProjection)> {
    match expr {
        Expr::Grouped { expr, .. } => dictionary_foreach_projection(expr),
        Expr::PropertyAccess {
            object,
            property,
            null_safe: false,
            ..
        } => match property.as_str() {
            "keys" => Some((object, DictionaryForeachProjection::Keys)),
            "values" => Some((object, DictionaryForeachProjection::Values)),
            _ => None,
        },
        _ => None,
    }
}

fn grouped_range_expr(expr: &Expr) -> Option<(&Expr, &Expr, bool)> {
    match expr {
        Expr::Grouped { expr, .. } => grouped_range_expr(expr),
        Expr::Range {
            start,
            end,
            inclusive,
            ..
        } => Some((start, end, *inclusive)),
        _ => None,
    }
}

fn emit_range_foreach(
    foreach: &ForeachStmt,
    start: &Expr,
    end: &Expr,
    inclusive: bool,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let start_expr = emit_expr(start, scopes);
    let end_expr = emit_expr(end, scopes);
    let start_temp = scopes.fresh_temp("__doria_range_start");
    let end_temp = scopes.fresh_temp("__doria_range_end");

    writeln(output, indent, &format!("${start_temp} = {start_expr};"));
    writeln(output, indent, &format!("${end_temp} = {end_expr};"));

    scopes.push();
    let value_name = scopes.declare(&foreach.value.name);
    let done_temp = inclusive.then(|| scopes.fresh_temp("__doria_range_done"));
    if let Some(done_temp) = &done_temp {
        writeln(output, indent, &format!("${done_temp} = false;"));
    }

    write_indent(output, indent);
    output.push_str("for ($");
    output.push_str(&value_name);
    output.push_str(" = $");
    output.push_str(&start_temp);
    output.push_str("; ");
    if let Some(done_temp) = &done_temp {
        output.push_str("!$");
        output.push_str(done_temp);
        output.push_str(" && ");
    }
    output.push('$');
    output.push_str(&value_name);
    output.push(' ');
    output.push_str(if inclusive { "<=" } else { "<" });
    output.push_str(" $");
    output.push_str(&end_temp);
    output.push_str("; ");
    if let Some(done_temp) = &done_temp {
        output.push('$');
        output.push_str(&value_name);
        output.push_str(" < $");
        output.push_str(&end_temp);
        output.push_str(" ? $");
        output.push_str(&value_name);
        output.push_str("++ : ($");
        output.push_str(done_temp);
        output.push_str(" = true)");
    } else {
        output.push('$');
        output.push_str(&value_name);
        output.push_str("++");
    }
    output.push_str(")\n");
    writeln(output, indent, "{");
    for statement in &foreach.body.statements {
        emit_statement(statement, output, indent + 1, scopes);
    }
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    let emitted = emit_expr_unboxed(expr, scopes);
    let Some(source_type) = scopes
        .mixed_box_types
        .get(&(expr.span().start, expr.span().end))
    else {
        return emitted;
    };
    format!(
        "__doria_box_mixed({}, {emitted})",
        emit_php_string_literal(&php_mixed_type_tag(source_type))
    )
}

fn emit_expr_unboxed(expr: &Expr, scopes: &PhpNameScopes) -> String {
    match expr {
        Expr::Variable { name, span } => {
            let value = format!("${}", scopes.php_name(name));
            if scopes.is_mixed_binding(name)
                && !matches!(
                    scopes.expression_types.get(&(span.start, span.end)),
                    Some(ResolvedType::Mixed) | None
                )
            {
                format!("__doria_mixed_value({value})")
            } else {
                value
            }
        }
        Expr::This { .. } => "$this".to_string(),
        Expr::Identifier { name, .. } if scopes.is_payload_top_constant(name) => {
            format!("__doria_const_{name}()")
        }
        Expr::Identifier { name, .. } => php_top_level_constant_name(name),
        Expr::String { value, .. } => emit_php_string_literal(value),
        Expr::InterpolatedString { parts, .. } => emit_interpolated_string(parts, scopes),
        Expr::Int { value, .. } | Expr::Float { value, .. } => value.clone(),
        Expr::Bool { value, .. } => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Expr::Null { .. } => "null".to_string(),
        Expr::Array { elements, .. } => {
            let inner = elements
                .iter()
                .map(|element| {
                    if let Some(key) = &element.key {
                        format!(
                            "{} => {}",
                            emit_expr(key, scopes),
                            emit_expr(&element.value, scopes)
                        )
                    } else {
                        emit_expr(&element.value, scopes)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        Expr::ArrayRepeat { .. } => {
            unreachable!("PHP validation rejects native sequence fill literals")
        }
        Expr::Index {
            collection, index, ..
        } => format!(
            "{}[{}]",
            emit_expr(collection, scopes),
            emit_expr(index, scopes)
        ),
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            ..
        } => format!(
            "{}{}{property}",
            emit_member_receiver(object, scopes),
            if *null_safe { "?->" } else { "->" }
        ),
        Expr::MethodCall {
            object,
            method,
            args,
            null_safe,
            ..
        } => format!(
            "{}{}{method}({})",
            emit_member_receiver(object, scopes),
            if *null_safe { "?->" } else { "->" },
            emit_arguments(args, scopes)
        ),
        Expr::FunctionCall { name, args, span } => emit_function_call(name, args, *span, scopes),
        Expr::StaticCall {
            class_name,
            method,
            args,
            ..
        } => {
            if class_name == "SortedDictionary" && method == "from" && args.len() == 1 {
                if let Expr::Array { elements, .. } = &args[0].value {
                    if elements.iter().all(|element| element.key.is_some()) {
                        let pairs = elements
                            .iter()
                            .map(|element| {
                                format!(
                                    "[{}, {}]",
                                    emit_expr(
                                        element.key.as_ref().expect("checked keyed entry"),
                                        scopes
                                    ),
                                    emit_expr(&element.value, scopes)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        return format!("SortedDictionary::fromPairs([{pairs}])");
                    }
                }
            }
            format!("{class_name}::{method}({})", emit_arguments(args, scopes))
        }
        Expr::StaticMember {
            class_name, member, ..
        } if scopes.is_static_property(class_name, member) => {
            format!("{class_name}::${member}")
        }
        Expr::StaticMember {
            class_name, member, ..
        } if scopes.is_payload_unit_case(class_name, member) => {
            format!("{class_name}::{}()", php_payload_case_method(member, false))
        }
        Expr::StaticMember {
            class_name, member, ..
        } if scopes.is_payload_class_constant(class_name, member) => {
            format!("{class_name}::__doriaConst{member}()")
        }
        Expr::StaticMember {
            class_name, member, ..
        } => format!("{class_name}::{member}"),
        Expr::New {
            class_type, args, ..
        } => format!("new {class_type}({})", emit_arguments(args, scopes)),
        Expr::Grouped { expr, .. } => format!("({})", emit_expr(expr, scopes)),
        Expr::IsType { expr, ty: _, span } => {
            let value = emit_expr(expr, scopes);
            let source_type = scopes
                .expression_types
                .get(&(expr.span().start, expr.span().end))
                .expect("checked type test must preserve its source type");
            let exact_type = scopes
                .type_test_types
                .get(&(span.start, span.end))
                .expect("checked type test must preserve its resolved exact type");
            let test = php_exact_type_test_for_source(&value, source_type, exact_type);
            format!("({test})")
        }
        Expr::Unary { op, expr, .. } => match op {
            UnaryOp::Not => format!("!({})", emit_expr(expr, scopes)),
            UnaryOp::Negate if integer_literal_magnitude(expr) == Some((i64::MAX as u128) + 1) => {
                "(-9223372036854775807 - 1)".to_string()
            }
            UnaryOp::Negate => format!("-({})", emit_expr(expr, scopes)),
            UnaryOp::BitwiseNot => {
                unreachable!("unsupported integer unary operator passed PHP capability validation")
            }
        },
        Expr::Binary {
            left, op, right, ..
        } => match op {
            BinaryOp::Div => format!(
                "fdiv({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::And => format!(
                "(({}) && ({}))",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Or => format!(
                "(({}) || ({}))",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Xor => format!(
                "(({}) !== ({}))",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Equal
                if scopes.is_payload_enum_expression(left)
                    || scopes.is_payload_enum_expression(right) =>
            {
                format!(
                    "__doria_equal({}, {})",
                    emit_expr(left, scopes),
                    emit_expr(right, scopes)
                )
            }
            BinaryOp::NotEqual
                if scopes.is_payload_enum_expression(left)
                    || scopes.is_payload_enum_expression(right) =>
            {
                format!(
                    "!__doria_equal({}, {})",
                    emit_expr(left, scopes),
                    emit_expr(right, scopes)
                )
            }
            BinaryOp::Concat => format!(
                "__doria_display({}) . __doria_display({})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Less => format!(
                "__doria_less({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::LessEqual => format!(
                "__doria_less_equal({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::Greater => format!(
                "__doria_greater({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            BinaryOp::GreaterEqual => format!(
                "__doria_greater_equal({}, {})",
                emit_expr(left, scopes),
                emit_expr(right, scopes)
            ),
            _ => format!(
                "{} {} {}",
                emit_expr(left, scopes),
                emit_binary_op(op),
                emit_expr(right, scopes)
            ),
        },
        Expr::Range { start, end, .. } => format!(
            "null /* unsupported range expression {}..{} */",
            emit_expr(start, scopes),
            emit_expr(end, scopes)
        ),
        Expr::Match {
            scrutinee,
            arms,
            span,
            ..
        } => emit_match_expression(scrutinee, arms, *span, scopes),
    }
}

fn emit_match_expression(
    scrutinee: &Expr,
    arms: &[MatchArm],
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    let info = scopes
        .matches
        .get(&(span.start, span.end))
        .expect("checked match must have a semantic plan");
    let temporary = scopes.expression_temp("__doriaMatch", span);
    let captures = scopes.captured_php_names();
    let capture_list = if captures.is_empty() {
        String::new()
    } else {
        format!(
            " use ({})",
            captures
                .iter()
                .map(|name| format!("&${name}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut output = format!("(function(${temporary}){capture_list} {{ ");

    for (index, (arm, arm_info)) in arms.iter().zip(&info.arms).enumerate() {
        let mut arm_scopes = scopes.clone();
        arm_scopes.push();
        // Semantic analysis proves every checked match exhaustive. Once all
        // earlier arms fail, the final arm is therefore the remaining case;
        // emitting it unconditionally keeps PHP from inventing a Doria runtime
        // "no arm matched" path that cannot exist in valid source.
        let condition = (index + 1 != arms.len()).then(|| {
            emit_php_match_condition(
                &temporary,
                &arm.pattern,
                &arm_info.pattern,
                &info.scrutinee_type,
                &arm_scopes,
            )
            .expect("only the final checked match arm may be unconditional")
        });
        if let Some(ref condition) = condition {
            output.push_str("if (");
            output.push_str(condition);
            output.push_str(") { ");
        }
        if let Some(guard) = &arm.guard {
            emit_php_match_bindings(
                &mut output,
                &temporary,
                &arm.pattern,
                &arm_info.pattern,
                &info.scrutinee_type,
                &mut arm_scopes,
                false,
            );
            output.push_str("if ((");
            output.push_str(&emit_expr(&guard.condition, &arm_scopes));
            output.push_str(") === true) { ");
        }
        emit_php_match_bindings(
            &mut output,
            &temporary,
            &arm.pattern,
            &arm_info.pattern,
            &info.scrutinee_type,
            &mut arm_scopes,
            arm.guard.is_some(),
        );
        output.push_str("return ");
        output.push_str(&emit_expr(&arm.value, &arm_scopes));
        output.push_str("; ");
        if arm.guard.is_some() {
            output.push_str("} ");
        }
        if condition.is_some() {
            output.push_str("} ");
        }
    }
    output.push_str("})(");
    output.push_str(&emit_expr(scrutinee, scopes));
    output.push(')');
    output
}

fn emit_php_match_condition(
    temporary: &str,
    pattern: &MatchPattern,
    resolved: &ResolvedMatchPattern,
    scrutinee_type: &ResolvedType,
    scopes: &PhpNameScopes,
) -> Option<String> {
    let value = format!("${temporary}");
    match resolved {
        ResolvedMatchPattern::Default => None,
        ResolvedMatchPattern::Condition => {
            let MatchPattern::Expression(condition) = pattern else {
                unreachable!("checked condition match must preserve its expression pattern")
            };
            Some(format!("({}) === true", emit_expr(condition, scopes)))
        }
        ResolvedMatchPattern::Null if matches!(scrutinee_type, ResolvedType::Mixed) => {
            Some(format!("__doria_mixed_is({value}, 'null')"))
        }
        ResolvedMatchPattern::Null => Some(format!("{value} === null")),
        ResolvedMatchPattern::Constant(constant) => {
            let pattern = emit_const_value(constant, &scopes.const_evaluation);
            if matches!(constant, ConstValue::PayloadEnum(_)) {
                Some(format!("__doria_equal({value}, {pattern})"))
            } else {
                Some(format!("{value} === {pattern}"))
            }
        }
        ResolvedMatchPattern::EnumCase { case_id, .. } => {
            if let Some(tag) = scopes.payload_case_tags.get(case_id) {
                Some(format!("{value}->__doriaMatchesCase({tag})"))
            } else {
                let MatchPattern::EnumCase {
                    qualifier, case, ..
                } = pattern
                else {
                    unreachable!("checked enum-case match must preserve enum syntax")
                };
                Some(format!("{value} === {qualifier}::{case}"))
            }
        }
        ResolvedMatchPattern::ExactType(ty) if matches!(scrutinee_type, ResolvedType::Mixed) => {
            Some(format!(
                "__doria_mixed_is({value}, {})",
                emit_php_string_literal(&php_mixed_type_tag(ty))
            ))
        }
        ResolvedMatchPattern::ExactType(ty) => Some(php_exact_type_test(&value, ty)),
    }
}

fn emit_php_match_bindings(
    output: &mut String,
    temporary: &str,
    pattern: &MatchPattern,
    resolved: &ResolvedMatchPattern,
    scrutinee_type: &ResolvedType,
    arm_scopes: &mut PhpNameScopes,
    reuse: bool,
) {
    match (pattern, resolved) {
        (
            MatchPattern::EnumCase {
                bindings: Some(bindings),
                ..
            },
            ResolvedMatchPattern::EnumCase { case_id, .. },
        ) if arm_scopes.payload_case_tags.contains_key(case_id) => {
            for (index, binding) in bindings.iter().enumerate() {
                let name = if reuse {
                    arm_scopes.declare_or_reuse_current(&binding.name)
                } else {
                    arm_scopes.declare(&binding.name)
                };
                output.push_str(&format!(
                    "${name} = ${temporary}->__doriaPayloadAt({index}); "
                ));
            }
        }
        (MatchPattern::TypeBinding { binding, .. }, ResolvedMatchPattern::ExactType(_)) => {
            let name = if reuse {
                arm_scopes.declare_or_reuse_current(&binding.name)
            } else {
                arm_scopes.declare(&binding.name)
            };
            let value = if matches!(scrutinee_type, ResolvedType::Mixed) {
                format!("__doria_mixed_value(${temporary})")
            } else {
                format!("${temporary}")
            };
            output.push_str(&format!("${name} = {value}; "));
        }
        _ => {}
    }
}

fn php_exact_type_test(value: &str, ty: &ResolvedType) -> String {
    php_host_exact_type_test(value, &php_mixed_type_tag(ty))
}

fn php_exact_type_test_for_source(
    value: &str,
    source: &ResolvedType,
    exact: &ResolvedType,
) -> String {
    match source {
        ResolvedType::Mixed => format!(
            "__doria_mixed_is({value}, {})",
            emit_php_string_literal(&php_mixed_type_tag(exact))
        ),
        ResolvedType::Nullable(inner) if inner.as_ref() == exact => format!("{value} !== null"),
        ResolvedType::Nullable(_) => "false".to_string(),
        _ => (source == exact).to_string(),
    }
}

fn php_host_exact_type_test(value: &str, type_tag: &str) -> String {
    match type_tag {
        "int" | "int8" | "int16" | "int32" | "uint8" | "uint16" | "uint32" | "uint64" => {
            format!("is_int({value})")
        }
        "float" | "float32" => format!("is_float({value})"),
        "string" => format!("is_string({value})"),
        "bool" => format!("is_bool({value})"),
        "null" => format!("{value} === null"),
        tag if tag.starts_with("enum:") => format!("{value} instanceof {}", &tag[5..]),
        tag if tag.starts_with("class:") => format!("{value} instanceof {}", &tag[6..]),
        _ => unreachable!("semantic checking rejects non-narrowable exact PHP type tests"),
    }
}

fn php_mixed_type_tag(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Integer(integer) => integer.source_name().to_string(),
        ResolvedType::Float(float) => float.source_name().to_string(),
        ResolvedType::String => "string".to_string(),
        ResolvedType::Bool => "bool".to_string(),
        ResolvedType::Null => "null".to_string(),
        ResolvedType::Enum(ty) => format!("enum:{}", ty.name),
        ResolvedType::Class(ty) => format!("class:{}", ty.name),
        ResolvedType::Nullable(inner) => php_mixed_type_tag(inner),
        _ => unreachable!("only exact Doria runtime values cross a PHP mixed boundary"),
    }
}

fn emit_member_receiver(expr: &Expr, scopes: &PhpNameScopes) -> String {
    let emitted = emit_expr(expr, scopes);
    match expr {
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::PropertyAccess { .. }
        | Expr::MethodCall { .. }
        | Expr::FunctionCall { .. }
        | Expr::StaticCall { .. }
        | Expr::StaticMember { .. }
        | Expr::New { .. }
        | Expr::Grouped { .. } => emitted,
        _ => format!("({emitted})"),
    }
}

fn emit_interpolated_string(parts: &[InterpolatedStringPart], scopes: &PhpNameScopes) -> String {
    let mut emitted = Vec::new();
    let mut has_expr = false;

    for part in parts {
        match part {
            InterpolatedStringPart::Text { value: text, .. } => {
                if !text.is_empty() {
                    emitted.push(emit_php_string_literal(text));
                }
            }
            InterpolatedStringPart::Expr(expr) => {
                has_expr = true;
                emitted.push(format!("__doria_display({})", emit_expr(expr, scopes)));
            }
        }
    }

    match emitted.len() {
        0 => emit_php_string_literal(""),
        1 if has_expr => format!("{} . {}", emit_php_string_literal(""), emitted[0]),
        _ => emitted.join(" . "),
    }
}

fn emit_php_string_literal(value: &str) -> String {
    format!("\"{}\"", escape_php_string(value))
}

fn emit_binary_op(op: &BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::ShiftLeft => "<<",
        BinaryOp::ShiftRight => ">>",
        BinaryOp::BitwiseAnd => "&",
        BinaryOp::BitwiseXor => "^",
        BinaryOp::BitwiseOr => "|",
        BinaryOp::Concat => ".",
        BinaryOp::Equal => "===",
        BinaryOp::NotEqual => "!==",
        BinaryOp::Less => "<",
        BinaryOp::LessEqual => "<=",
        BinaryOp::Greater => ">",
        BinaryOp::GreaterEqual => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::Xor => unreachable!("xor is emitted by the boolean-specialized binary branch"),
        BinaryOp::Coalesce => "??",
    }
}

fn emit_function_call(name: &str, args: &[Argument], span: Span, scopes: &PhpNameScopes) -> String {
    let helper = match name {
        "read_line" => "__doria_read_line",
        "read_file" => "__doria_read_file",
        "write_file" => "__doria_write_file",
        "append_file" => "__doria_append_file",
        "write_stderr" => "__doria_write_stderr",
        "sprintf" => "__doria_sprintf",
        "printf" => "__doria_printf",
        _ => name,
    };
    let mut emitted = args
        .iter()
        .map(|argument| match &argument.name {
            Some(name) => format!("{}: {}", name.text, emit_expr(&argument.value, scopes)),
            None => emit_expr(&argument.value, scopes),
        })
        .collect::<Vec<_>>();
    if matches!(name, "sprintf" | "printf") {
        if let Some(Expr::String { value, span }) = args.first().map(|argument| &argument.value) {
            if let Ok(pieces) = format_string::parse(value, *span) {
                emitted[0] = emit_php_string_literal(&php_format_from_plan(&pieces));
                let conversions = pieces.iter().filter_map(|piece| match piece {
                    FormatPiece::Argument { spec, .. } => Some(spec.conversion),
                    FormatPiece::Literal(_) => None,
                });
                for (argument, conversion) in emitted.iter_mut().skip(1).zip(conversions) {
                    if conversion == FormatConversion::Display {
                        *argument = format!("__doria_display({argument})");
                    }
                }
            }
        }
    }
    if name == "printf" {
        emitted.insert(0, span.end.to_string());
        emitted.insert(0, span.start.to_string());
    } else if matches!(
        name,
        "read_line" | "read_file" | "write_file" | "append_file" | "write_stderr"
    ) {
        if name == "read_line" && emitted.is_empty() {
            emitted.push("\"\"".to_string());
        }
        emitted.push(format!("start: {}", span.start));
        emitted.push(format!("end: {}", span.end));
    }
    format!("{helper}({})", emitted.join(", "))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn php_format_from_plan(pieces: &[FormatPiece]) -> String {
    let mut format = String::new();
    for piece in pieces {
        match piece {
            FormatPiece::Literal(value) => format.push_str(&value.replace('%', "%%")),
            FormatPiece::Argument { spec, .. } => {
                format.push('%');
                if spec.left_align {
                    format.push('-');
                }
                if spec.zero_pad {
                    format.push('0');
                }
                if let Some(width) = spec.width {
                    format.push_str(&width.to_string());
                }
                if let Some(precision) = spec.precision {
                    format.push('.');
                    format.push_str(&precision.to_string());
                }
                format.push(match spec.conversion {
                    FormatConversion::Display => 's',
                    FormatConversion::Decimal => 'd',
                    FormatConversion::Float => 'F',
                    FormatConversion::HexLower => 'x',
                    FormatConversion::HexUpper => 'X',
                    FormatConversion::Octal => 'o',
                    FormatConversion::Binary => 'b',
                });
            }
        }
    }
    format
}

fn emit_member_access(access: &MemberAccess) -> &'static str {
    match access {
        MemberAccess::External => "public",
        MemberAccess::Internal => "private",
    }
}

fn php_type(ty: &TypeRef) -> String {
    let name = if IntegerType::from_source_name(&ty.name).is_some() {
        "int".to_string()
    } else if FloatType::from_source_name(&ty.name).is_some() {
        "float".to_string()
    } else {
        match ty.name.as_str() {
            "List" | "Dictionary" | "Set" | "[]" => "array".to_string(),
            name => name.to_string(),
        }
    };
    if ty.nullable {
        format!("?{name}")
    } else {
        name
    }
}

fn escape_php_string(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '$' => output.push_str("\\$"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ => output.push(character),
        }
    }
    output
}

fn writeln(output: &mut String, indent: usize, line: &str) {
    write_indent(output, indent);
    output.push_str(line);
    output.push('\n');
}

fn write_indent(output: &mut String, indent: usize) {
    for _ in 0..indent {
        output.push_str("    ");
    }
}
