use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::backend::BackendError;
use crate::builtins::Builtin;
use crate::const_eval::{ConstKey, ConstValue, Evaluation, ParameterDefaultKey};
use crate::diagnostics::Diagnostic;
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::hir::*;
use crate::mir;
use crate::numeric::{parse_decimal_magnitude, FloatType, IntegerType};
use crate::php_closure::{PhpClosureDescriptor, PhpClosurePlan};
use crate::semantics::{
    GivenSemanticInfo, MatchSemanticInfo, ResolvedMatchPattern, SemanticInfo, WhenSemanticInfo,
};
use crate::source::Span;
use crate::symbols::{BindingId, BuiltinInterface};
use crate::types::{ResolvedType, TypeRef};

mod core_collection;
mod interface;
mod shared;
mod specialization;

const PHP_INTEGER_UNSUPPORTED_CODE: &str = "B1301";
const PHP_CONSTANT_UNSUPPORTED_CODE: &str = "B2001";
const PHP_COLLECTION_UNSUPPORTED_CODE: &str = "B2301";
const PHP_STRING_RUNTIME_UNSUPPORTED_CODE: &str = "B2501";

const PHP_ERROR_CONTRACT: &str = r#"
interface __DoriaErrorValue
{
    public static function __doriaErrorType(): __DoriaErrorDescriptor;
    public function __doriaErrorDescriptor(): __DoriaErrorDescriptor;
    public function __doriaEnsureErrorOrigin(int $origin, string $callable): void;
    public function __doriaErrorOrigin(): int;
    public function __doriaErrorCallable(): string;
}
"#;

const PHP_CHECKED_ERROR_HELPERS: &str = r#"
final class __DoriaErrorDescriptor
{
    public function __construct(public string $typeName) {}
}

final class __DoriaCheckedError extends Exception
{
    private bool $__doriaLive = true;

    public function __construct(
        private ?__DoriaErrorValue $error,
        public __DoriaErrorDescriptor $descriptor,
        public int $origin,
    ) {
        parent::__construct("");
    }

    public function error(): __DoriaErrorValue
    {
        if (!$this->__doriaLive || $this->error === null) {
            throw new LogicException("compiler invariant violated: moved Doria error was used");
        }
        return $this->error;
    }

    public function takeError(): __DoriaErrorValue
    {
        $error = $this->error();
        $this->error = null;
        $this->__doriaLive = false;
        return $error;
    }

    public function dropError(): void
    {
        if (!$this->__doriaLive || $this->error === null) {
            return;
        }
        $error = $this->error;
        $this->error = null;
        $this->__doriaLive = false;
        if (function_exists("__doria_drop_value")) {
            __doria_drop_value($error);
        }
    }

    public function __destruct()
    {
        if (__doria_cleanup_enabled()) { $this->dropError(); }
    }
}

function __doria_detach_checked_error(__DoriaCheckedError $caught): __DoriaCheckedError
{
    $previous = $caught->getPrevious();
    while ($previous instanceof __DoriaCheckedError) {
        $previous->dropError();
        $previous = $previous->getPrevious();
    }
    return new __DoriaCheckedError(
        $caught->takeError(),
        $caught->descriptor,
        $caught->origin,
    );
}

function __doria_error_descriptor(string $typeName): __DoriaErrorDescriptor
{
    static $descriptors = [];
    return $descriptors[$typeName] ??= new __DoriaErrorDescriptor($typeName);
}

function __doria_throw(__DoriaErrorValue $error, int $origin, string $callable): void
{
    $error->__doriaEnsureErrorOrigin($origin, $callable);
    throw new __DoriaCheckedError(
        $error,
        $error->__doriaErrorDescriptor(),
        $error->__doriaErrorOrigin(),
    );
}

function __doria_safe_error_message(string $message): string
{
    $safe = "";
    $length = strlen($message);
    for ($index = 0; $index < $length; ++$index) {
        $byte = ord($message[$index]);
        $safe .= match ($byte) {
            0 => "\\u0000",
            10 => "\n  ",
            13 => "\\r",
            9 => "\\t",
            default => $byte < 0x20 || $byte === 0x7f
                ? sprintf("\\u00%02x", $byte)
                : $message[$index],
        };
    }
    return $safe;
}

function __doria_bound_assertion_text(string $presentation): string
{
    if (strlen($presentation) <= 4096) {
        return $presentation;
    }
    $marker = "...<truncated>";
    $prefix = substr($presentation, 0, 4096 - strlen($marker));
    while ($prefix !== "" && preg_match('//u', $prefix) !== 1) {
        $prefix = substr($prefix, 0, -1);
    }
    return $prefix . $marker;
}

function __doria_assertion_error_message(string $message): string
{
    $escaped = "";
    $length = strlen($message);
    for ($index = 0; $index < $length; ++$index) {
        $byte = ord($message[$index]);
        $escaped .= match ($byte) {
            34 => "\\\"",
            92 => "\\\\",
            10 => "\\n",
            13 => "\\r",
            9 => "\\t",
            default => $byte < 0x20 || $byte === 0x7f
                ? sprintf("\\u00%02x", $byte)
                : $message[$index],
        };
    }
    return __doria_bound_assertion_text($escaped);
}

function __doria_assertion_presentation(mixed $value, string $type): string
{
    if ($value === null) {
        $presentation = "null";
    } elseif ($type === "string" || $type === "?string") {
        $encoded = json_encode($value, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
        $presentation = $encoded === false ? "\"<invalid string>\"" : $encoded;
    } elseif ($type === "uint64" || $type === "?uint64") {
        $presentation = sprintf('%u', $value);
    } elseif ($type === "Bytes" || str_ends_with($type, "[]") ||
        preg_match('/^(List|Dictionary|SortedDictionary|Set|SortedSet|PriorityQueue|Deque)</', $type) === 1
    ) {
        return __doria_assertion_collection_presentation($value, $type);
    } elseif (is_bool($value)) {
        $presentation = $value ? "true" : "false";
    } elseif (is_int($value) || is_float($value)) {
        $presentation = __doria_display($value);
    } elseif ($value instanceof \UnitEnum) {
        $presentation = $type . "::" . $value->name;
    } elseif (str_starts_with($type, "class ")) {
        $presentation = "<" . substr($type, 6) . ">";
    } else {
        $presentation = "<" . $type . ">";
    }
    return __doria_bound_assertion_text($presentation);
}

function __doria_assertion_type_arguments(string $type): array
{
    $open = strpos($type, '<');
    if ($open === false || !str_ends_with($type, '>')) { return []; }
    $arguments = [];
    $start = $open + 1;
    $depth = 0;
    $length = strlen($type) - 1;
    for ($index = $start; $index < $length; ++$index) {
        $character = $type[$index];
        if ($character === '<') { ++$depth; }
        elseif ($character === '>') { --$depth; }
        elseif ($character === ',' && $depth === 0) {
            $arguments[] = trim(substr($type, $start, $index - $start));
            $start = $index + 1;
        }
    }
    $arguments[] = trim(substr($type, $start, $length - $start));
    return $arguments;
}

function __doria_assertion_item_presentation(mixed $value, string $type): string
{
    if ($value === null) { return "null"; }
    $type = ltrim($type, '?');
    if ($type === 'uint64') { return sprintf('%u', $value); }
    if ($type === 'string') {
        $encoded = json_encode($value, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
        return $encoded === false ? '"<invalid string>"' : $encoded;
    }
    if (is_bool($value)) { return $value ? 'true' : 'false'; }
    if (is_int($value) || is_float($value)) { return __doria_display($value); }
    if ($value instanceof \UnitEnum) { return $type . '::' . $value->name; }
    return '<' . $type . '>';
}

function __doria_assertion_collection_presentation(mixed $value, string $type): string
{
    $count = __doria_assertion_collection_count($value);
    if ($type === 'Bytes') {
        $items = [];
        foreach ($value as $byte) {
            if (count($items) === 8) { break; }
            $items[] = sprintf('%02x', $byte);
        }
        $suffix = $count > 8 ? ' ...<truncated>' : '';
        return __doria_bound_assertion_text(
            'Bytes(length: ' . $count . ', hex: "' . implode(' ', $items) . $suffix . '")'
        );
    }
    if (str_starts_with($type, 'PriorityQueue<')) {
        return __doria_bound_assertion_text($type . '(count: ' . $count . ')');
    }

    $dictionary = str_starts_with($type, 'Dictionary<') ||
        str_starts_with($type, 'SortedDictionary<');
    $set = str_starts_with($type, 'Set<') || str_starts_with($type, 'SortedSet<');
    $typedArray = str_ends_with($type, '[]');
    $arguments = __doria_assertion_type_arguments($type);
    $keyType = $dictionary ? ($arguments[0] ?? 'mixed') : '';
    $valueType = $typedArray
        ? substr($type, 0, -2)
        : ($arguments[$dictionary ? 1 : 0] ?? 'mixed');
    $presentation = $typedArray
        ? '['
        : $type . '(count: ' . $count . ') ' . (($dictionary || $set) ? '{' : '[');
    $shown = 0;
    foreach ($value as $key => $item) {
        if ($shown === 8) { break; }
        if ($shown !== 0) { $presentation .= ', '; }
        if ($dictionary) {
            $presentation .= __doria_assertion_item_presentation($key, $keyType) . ' => ';
        }
        $presentation .= __doria_assertion_item_presentation($item, $valueType);
        ++$shown;
    }
    if ($count > 8) {
        if ($shown !== 0) { $presentation .= ', '; }
        $presentation .= '...<truncated>';
    }
    $presentation .= ($dictionary || $set) ? '}' : ']';
    return __doria_bound_assertion_text($presentation);
}

function __doria_assertion_string_difference(string $actual, string $expected, int $mode): string
{
    preg_match_all('/\X/u', $actual, $actualMatches);
    preg_match_all('/\X/u', $expected, $expectedMatches);
    $actualGraphemes = $actualMatches[0];
    $expectedGraphemes = $expectedMatches[0];
    if ($mode === 2) {
        $common = min(count($actualGraphemes), count($expectedGraphemes));
        $actualGraphemes = array_slice($actualGraphemes, count($actualGraphemes) - $common);
        $expectedGraphemes = array_slice($expectedGraphemes, count($expectedGraphemes) - $common);
    }
    $common = min(count($actualGraphemes), count($expectedGraphemes));
    $index = $common;
    for ($candidate = 0; $candidate < $common; ++$candidate) {
        if ($actualGraphemes[$candidate] !== $expectedGraphemes[$candidate]) {
            $index = $candidate;
            break;
        }
    }
    $relation = $mode === 1 ? 'Prefix' : ($mode === 2 ? 'Suffix' : 'Value');
    $actualText = __doria_assertion_presentation($actual, 'string');
    $expectedText = __doria_assertion_presentation($expected, 'string');
    $difference = 'First Differing Grapheme: ' . $index . "\n" .
        'Expected ' . $relation . ': ' . $expectedText . "\n" .
        'Actual ' . $relation . ': ' . $actualText . "\n" .
        'Expected Grapheme Length: ' . count($expectedMatches[0]) . "\n" .
        'Actual Grapheme Length: ' . count($actualMatches[0]);
    if (strlen($difference) <= 4096) { return $difference; }
    $marker = '...<truncated>';
    $prefix = substr($difference, 0, 4096 - strlen($marker));
    while ($prefix !== '' && preg_match('//u', $prefix) !== 1) {
        $prefix = substr($prefix, 0, -1);
    }
    return $prefix . $marker;
}

function __doria_assertion_decimal_add(string $left, string $right): string
{
    $carry = 0;
    $result = '';
    for ($leftIndex = strlen($left) - 1, $rightIndex = strlen($right) - 1;
        $leftIndex >= 0 || $rightIndex >= 0 || $carry !== 0;
        --$leftIndex, --$rightIndex
    ) {
        $sum = $carry + ($leftIndex >= 0 ? ord($left[$leftIndex]) - 48 : 0) +
            ($rightIndex >= 0 ? ord($right[$rightIndex]) - 48 : 0);
        $result = chr(48 + ($sum % 10)) . $result;
        $carry = intdiv($sum, 10);
    }
    return $result;
}

function __doria_assertion_count_difference(mixed $collection, int $expected): string
{
    $actual = __doria_assertion_collection_count($collection);
    $delta = $expected < 0 && $actual > PHP_INT_MAX + $expected
        ? __doria_assertion_decimal_add((string) $actual, substr((string) $expected, 1))
        : (string) ($actual - $expected);
    return "Expected Count: " . $expected . "\nActual Count: " . $actual . "\nDelta: " . $delta;
}

function __doria_assertion_bytes_difference(array $actual, array $expected): string
{
    $common = min(count($actual), count($expected));
    for ($index = 0; $index < $common; ++$index) {
        if ($actual[$index] !== $expected[$index]) {
            return "First Differing Byte: " . $index . "\nExpected Byte: " .
                sprintf('%02x', $expected[$index]) . "\nActual Byte: " .
                sprintf('%02x', $actual[$index]);
        }
    }
    return "Expected Byte Length: " . count($expected) . "\nActual Byte Length: " .
        count($actual) . "\nDelta: " . (count($actual) - count($expected));
}

function __doria_u64_le(int $value): string
{
    return pack("V2", $value & 0xffffffff, intdiv($value, 4294967296));
}

function __doria_publish_outcome(string $path, string $record): bool
{
    return @file_put_contents($path, $record, LOCK_EX) === strlen($record);
}

function __doria_write_error_outcome_v3(__DoriaCheckedError $caught, string $path): bool
{
    $type = $caught->descriptor->typeName;
    $message = $caught->error()->message;
    $origin = $caught->origin;
    $known = $origin !== 0;
    [$sourcePath, $sourceText, $start] = $known
        ? __doria_source_location($origin)
        : ["", "", 0];
    $function = $known ? $caught->error()->__doriaErrorCallable() : "";
    $record = "DORIAO3\0" . pack("vV", 3, strlen($type)) .
        __doria_u64_le(strlen($message)) .
        pack("VVVC", strlen($sourcePath), strlen($sourceText), strlen($function), $known ? 1 : 0) .
        __doria_u64_le($start) . __doria_u64_le($start + ($known ? 1 : 0)) .
        $type . $message . $sourcePath . $sourceText . $function;
    return __doria_publish_outcome($path, $record);
}

function __doria_write_assertion_outcome_v4(
    __DoriaCheckedError $caught,
    array $facts,
    string $path,
): bool {
    [$matcher, $negated, $actualPresent, $actualType, $actualPresentation,
        $expectedPresent, $expectedType, $expectedPresentation,
        $differencePresent, $difference, $userMessagePresent, $userMessage] = $facts;
    $type = $caught->descriptor->typeName;
    $origin = $caught->origin;
    $known = $origin !== 0;
    [$sourcePath, $sourceText, $start] = $known
        ? __doria_source_location($origin)
        : ["", "", 0];
    $function = $known ? $caught->error()->__doriaErrorCallable() : "";
    if (strlen($type) > 4096 || strlen($matcher) > 64 ||
        strlen($actualType) > 4096 || strlen($actualPresentation) > 4096 ||
        strlen($expectedType) > 4096 || strlen($expectedPresentation) > 4096 ||
        strlen($difference) > 4096 || strlen($userMessage) > 65536 ||
        strlen($sourcePath) > 4096 || strlen($sourceText) > 4194304 ||
        strlen($function) > 1024
    ) {
        return false;
    }
    $record = "DORIAO4\0" . pack("vVVCVCCVV", 4, strlen($type), 70, 1,
        strlen($matcher), $negated ? 1 : 0, $actualPresent ? 1 : 0,
        strlen($actualType), strlen($actualPresentation)) .
        pack("CVVCVCVVVVC", $expectedPresent ? 1 : 0, strlen($expectedType),
            strlen($expectedPresentation), $differencePresent ? 1 : 0,
            strlen($difference), $userMessagePresent ? 1 : 0, strlen($userMessage),
            strlen($sourcePath), strlen($sourceText), strlen($function), $known ? 1 : 0) .
        __doria_u64_le($start) . __doria_u64_le($start + ($known ? 1 : 0)) .
        pack("v", 0) . $type . $matcher . $actualType . $actualPresentation .
        $expectedType . $expectedPresentation . $difference . $userMessage .
        $sourcePath . $sourceText . $function;
    return __doria_publish_outcome($path, $record);
}

function __doria_report_unhandled_error(__DoriaCheckedError $caught): void
{
    $type = $caught->descriptor->typeName;
    $origin = $caught->origin;
    [$sourcePath, $sourceText, $sourceOffset] = __doria_source_location($origin);
    $line = __doria_source_line($origin);
    $before = substr($sourceText, 0, max(0, $sourceOffset));
    $lineStart = strrpos($before, "\n");
    $lineStart = $lineStart === false ? 0 : $lineStart + 1;
    $lineEnd = strpos($sourceText, "\n", max(0, $sourceOffset));
    $lineEnd = $lineEnd === false ? strlen($sourceText) : $lineEnd;
    $lineText = rtrim(substr($sourceText, $lineStart, $lineEnd - $lineStart), "\r");
    $markerOffset = max(0, $sourceOffset - $lineStart);
    $error = $caught->error();
    $assertion = $type === "Doria\\Std\\Test\\AssertionError" &&
        method_exists($error, "__doriaAssertionFacts");
    if ($assertion) {
        $facts = $error->__doriaAssertionFacts();
        $v4 = getenv("DORIA_RUNTIME_OUTCOME_V4");
        if (is_string($v4) && __doria_write_assertion_outcome_v4($caught, $facts, $v4)) {
            $caught->dropError();
            exit(70);
        }
    }
    $v3 = getenv("DORIA_RUNTIME_OUTCOME_V3");
    if (!$assertion && is_string($v3) && __doria_write_error_outcome_v3($caught, $v3)) {
        $caught->dropError();
        exit(70);
    }
    if ($assertion && !is_string(getenv("DORIA_RUNTIME_OUTCOME_V4")) &&
        is_string($v3) && __doria_write_error_outcome_v3($caught, $v3)) {
        $caught->dropError();
        exit(70);
    }
    if ($assertion) {
        [, , $actualPresent, , $actualPresentation,
            $expectedPresent, , $expectedPresentation,
            $differencePresent, $difference] = $facts;
        $message = "Error[R1001]: Assertion Failed\n\nWhere\n" .
            $sourcePath . " · line " . $line . " · " . $error->__doriaErrorCallable() . "\n\n" .
            $lineText . "\n" . str_repeat(" ", $markerOffset) . "^\nAssertion Failed Here" .
            ($expectedPresent ? "\n\nExpected\n  " . $expectedPresentation : "") .
            ($actualPresent ? "\n\nActual\n  " . $actualPresentation : "") .
            ($differencePresent ? "\n\nDifference\n  " . __doria_safe_error_message($difference) : "") .
            "\n\nWhy\n  " . __doria_safe_error_message($error->message) .
            "\n\nProcess Exited With Status 70\n";
        @fwrite(STDERR, $message);
        $caught->dropError();
        exit(70);
    }
    $message = "Error[R1000]: Unhandled " . $type . "\n\nWhere\n" .
        $sourcePath . " · line " . $line . " · " .
        $error->__doriaErrorCallable() . "\n\n" .
        $lineText . "\n" . str_repeat(" ", $markerOffset) . "^\n" .
        "This Error Was First Thrown Here\n\nWhy\n  " .
        __doria_safe_error_message($error->message) .
        "\n\nProcess Exited With Status 70\n";
    @fwrite(STDERR, $message);
    $caught->dropError();
    exit(70);
}

"#;

const PHP_STAGE26_COLLECTION_HELPERS: &str = r#"
abstract class __DoriaOrderedCollection
{
    public function __construct(protected bool $unsigned = false) {}

    abstract public function clear(): void;

    protected function compare(mixed $left, mixed $right): int
    {
        if ($this->unsigned) { return ($left ^ PHP_INT_MIN) <=> ($right ^ PHP_INT_MIN); }
        if (is_string($left)) { return strcmp($left, $right); }
        if (is_bool($left)) { return ((int) $left) <=> ((int) $right); }
        return $left <=> $right;
    }

    protected static function releaseValuesInReverse(array &$values): void
    {
        $keys = array_keys($values);
        for ($index = count($keys) - 1; $index >= 0; --$index) {
            __doria_drop_value($values[$keys[$index]]);
            unset($values[$keys[$index]]);
        }
    }

    protected static function releasePairsInReverse(array &$pairs): void
    {
        $keys = array_keys($pairs);
        for ($index = count($keys) - 1; $index >= 0; --$index) {
            $key = $keys[$index];
            __doria_drop_value($pairs[$key][1]);
            __doria_drop_value($pairs[$key][0]);
            unset($pairs[$key][1]);
            unset($pairs[$key][0]);
            unset($pairs[$key]);
        }
    }
}

function __doria_assertion_collection_count(mixed $collection): int
{
    return is_array($collection) ? count($collection) : $collection->count;
}

function __doria_collection_index_of(mixed $collection, mixed $value, ?callable $equals = null): ?int
{
    $index = 0;
    foreach ($collection as $candidate) {
        if ($equals === null ? __doria_equal($candidate, $value) : $equals($candidate, $value)) { return $index; }
        ++$index;
    }
    return null;
}

function __doria_primitive_set_from(array $source): array
{
    $seen = [];
    $result = [];
    foreach ($source as $value) {
        $key = is_string($value) ? "s:" . $value : (is_bool($value) ? "b:" . (int) $value : "i:" . $value);
        if (!isset($seen[$key])) { $seen[$key] = true; $result[] = $value; }
    }
    return $result;
}

function __doria_list_remove(array &$collection, mixed $value, ?callable $equals = null): bool
{
    $index = __doria_collection_index_of($collection, $value, $equals);
    if ($index === null) { return false; }
    $removed = array_splice($collection, $index, 1);
    __doria_drop_value($removed[0]);
    return true;
}

function __doria_assertion_collection_contains(mixed $collection, mixed $value, ?callable $equals = null): bool
{
    if ($collection instanceof __DoriaCoreCollection) { return $collection->contains($value); }
    if ($equals === null && !is_array($collection)) { return $collection->contains($value); }
    return __doria_collection_index_of($collection, $value, $equals) !== null;
}

function __doria_assertion_dictionary_has_key(mixed $collection, mixed $key): bool
{
    return is_array($collection)
        ? array_key_exists($key, $collection)
        : $collection->containsKey($key);
}

function __doria_assertion_dictionary_has_value(mixed $collection, mixed $value, ?callable $equals = null): bool
{
    if ($equals === null && !is_array($collection)) { return $collection->containsValue($value); }
    return __doria_collection_index_of($collection, $value, $equals) !== null;
}

final class SortedDictionary extends __DoriaOrderedCollection implements ArrayAccess, IteratorAggregate
{
    private array $entries = [];

    public static function from(array $source, bool $unsigned = false, ?callable $duplicate = null): self
    {
        $result = new self($unsigned);
        try {
            foreach ($source as $key => $value) {
                $result->entries[] = [$key, $duplicate === null ? $value : $duplicate($value)];
            }
            usort($result->entries, fn($left, $right) => $result->compare($left[0], $right[0]));
        } catch (__DoriaCheckedError $error) {
            $result->clear();
            throw $error;
        }
        return $result;
    }

    public static function fromPairs(array $pairs, bool $unsigned = false): self
    {
        $result = new self($unsigned);
        foreach ($pairs as $pair) { $result->set($pair[0], $pair[1]); }
        return $result;
    }

    private function locate(mixed $key): array
    {
        $low = 0;
        $high = count($this->entries);
        while ($low < $high) {
            $middle = $low + intdiv($high - $low, 2);
            $order = $this->compare($this->entries[$middle][0], $key);
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
        if ($name === 'keys' || $name === 'values') {
            $projection = [];
            $index = $name === 'keys' ? 0 : 1;
            foreach ($this->entries as $entry) { $projection[] = $entry[$index]; }
            return $projection;
        }
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

    public static function from(array $source, bool $unsigned = false): self
    {
        $result = new self($unsigned);
        foreach ($source as $value) { $result->add($value); }
        return $result;
    }

    private function locate(mixed $value): array
    {
        $low = 0;
        $high = count($this->values);
        while ($low < $high) {
            $middle = $low + intdiv($high - $low, 2);
            $order = $this->compare($this->values[$middle], $value);
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
        $result = new self($this->unsigned);
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
    public static function from(array $source, bool $unsigned = false): self
    {
        $result = new self($unsigned);
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
            $child = $right < $length && $this->compare($this->heap[$right], $this->heap[$left]) < 0
                ? $right : $left;
            if ($this->compare($this->heap[$parent], $this->heap[$child]) <= 0) { return; }
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
            if ($this->compare($this->heap[$parent], $this->heap[$child]) <= 0) { break; }
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
    public function contains(mixed $value): bool
    {
        foreach ($this->heap as $candidate) {
            if ($candidate === $value) { return true; }
        }
        return false;
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
    public static function from(array $source, ?callable $duplicate = null): self
    {
        $result = new self();
        try {
            foreach ($source as $value) { $result->pushBack($duplicate === null ? $value : $duplicate($value)); }
        } catch (__DoriaCheckedError $error) {
            $result->clear();
            throw $error;
        }
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
    public function contains(mixed $value): bool
    {
        for ($offset = 0; $offset < $this->count; ++$offset) {
            if ($this->values[($this->head + $offset) % $this->capacity] === $value) {
                return true;
            }
        }
        return false;
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
            __doria_drop_value($values[($head + $offset) % $capacity]);
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

const PHP_CLOSURE_BASE_RUNTIME: &str = r#"
$__doria_panicking = false;
$__doria_shutting_down = false;
register_shutdown_function(static function (): void {
    global $__doria_shutting_down;
    $__doria_shutting_down = true;
});

function __doria_cleanup_enabled(): bool
{
    global $__doria_panicking, $__doria_shutting_down;
    // Host shutdown may reclaim leaked graphs, but must not execute Doria drops.
    return !$__doria_panicking && !$__doria_shutting_down;
}

interface __DoriaFunctionValue
{
    public function __doriaDrop(): void;
}

interface __DoriaOwnedObject {}

function __doria_begin_destroy(object $value, string $class): bool
{
    if (!__doria_cleanup_enabled()) { return false; }
    static $destroyed;
    $destroyed ??= new WeakMap();
    $completed = $destroyed[$value] ?? [];
    if (isset($completed[$class])) { return false; }
    $completed[$class] = true;
    $destroyed[$value] = $completed;
    return true;
}

final class __DoriaCell
{
    public bool $live = true;

    public function __construct(public mixed $value)
    {
    }
}

function __doria_take_cell(__DoriaCell $cell): mixed
{
    if (!$cell->live) {
        throw new LogicException("compiler invariant violated: moved Doria place was used");
    }
    $value = $cell->value;
    $cell->value = null;
    $cell->live = false;
    return $value;
}

function __doria_drop_value(mixed &$value): void
{
    if (!__doria_cleanup_enabled()) { return; }
    if ($value instanceof __DoriaMixedValue) {
        $payload = $value->value();
        __doria_drop_value($payload);
    } elseif ($value instanceof __DoriaFunctionValue) {
        $value->__doriaDrop();
    } elseif ($value instanceof __DoriaCell) {
        __doria_drop_cell($value);
    } elseif ($value instanceof __DoriaSharedHandle) {
        $value->drop();
    } elseif ($value instanceof __DoriaOrderedCollection) {
        $value->clear();
    } elseif ($value instanceof __DoriaOwnedObject) {
        $value->__destruct();
    } elseif (is_array($value)) {
        foreach (array_reverse(array_keys($value)) as $key) {
            __doria_drop_value($value[$key]);
            unset($value[$key]);
        }
    }
    $value = null;
}

function __doria_drop_cell(__DoriaCell $cell): void
{
    if (!$cell->live) { return; }
    $cell->live = false;
    __doria_drop_value($cell->value);
}

function __doria_replace_cell(__DoriaCell $cell, mixed $replacement): void
{
    $previous = $cell->live ? $cell->value : null;
    $cell->value = $replacement;
    $cell->live = true;
    __doria_drop_value($previous);
}

function __doria_collection_index(array $values, int|string $key, bool $dictionary, int $start, int $end, string $callable): mixed
{
    if (!array_key_exists($key, $values)) {
        __doria_panic($dictionary ? "P1312" : "P1310", $start, $end, null, $callable,
            $dictionary ? [] : ["index" => $key, "length" => count($values)]);
    }
    return $values[$key];
}

function __doria_dictionary_get(?array $values, int|string $key): mixed
{
    return $values[$key] ?? null;
}

function __doria_collection_set(array &$values, int|string $key, mixed $replacement, bool $dictionary, int $start, int $end, string $callable): void
{
    if (!$dictionary && !array_key_exists($key, $values)) {
        __doria_panic("P1310", $start, $end, null, $callable, ["index" => $key, "length" => count($values)]);
    }
    $previous = $values[$key] ?? null;
    $values[$key] = $replacement;
    __doria_drop_value($previous);
}

"#;

fn emit_php_closure_runtime(
    plan: &PhpClosurePlan,
    specialization: &specialization::Plan,
    program: Option<&mir::Program>,
    output: &mut String,
) {
    output.push_str(PHP_CLOSURE_BASE_RUNTIME);
    if plan.descriptors.is_empty() {
        return;
    }
    let program = program.expect("validated MIR must back executable PHP closures");
    let mut descriptors = plan.descriptors.values().collect::<Vec<_>>();
    descriptors.sort_by_key(|descriptor| descriptor.descriptor.0);
    for descriptor in descriptors {
        if let (Some(layout_id), Some(environment_name)) = (
            descriptor.environment_layout,
            descriptor.environment_name.as_ref(),
        ) {
            emit_php_closure_environment(environment_name, plan.layout(layout_id), output);
        }
        emit_php_closure_carrier(
            descriptor,
            plan.function_type(descriptor.function_type),
            program,
            specialization,
            output,
        );
    }
}

fn emit_php_closure_environment(
    name: &str,
    layout: &mir::ClosureEnvironmentLayout,
    output: &mut String,
) {
    writeln(output, 0, &format!("final class {name}"));
    writeln(output, 0, "{");
    writeln(output, 1, "private bool $__doriaLive = true;");
    let mut constructor_fields = layout.fields.iter().collect::<Vec<_>>();
    constructor_fields.sort_by_key(|field| field.logical_index);
    for field in &layout.fields {
        writeln(
            output,
            1,
            &format!("public __DoriaCell $field{};", field.id.0),
        );
    }
    output.push('\n');
    write_indent(output, 1);
    output.push_str("public function __construct(");
    output.push_str(
        &constructor_fields
            .iter()
            .map(|field| format!("__DoriaCell $field{}", field.id.0))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str(")\n");
    writeln(output, 1, "{");
    for field in &constructor_fields {
        writeln(
            output,
            2,
            &format!("$this->field{0} = $field{0};", field.id.0),
        );
    }
    writeln(output, 1, "}");
    output.push('\n');
    writeln(output, 1, "public function __doriaDrop(): void");
    writeln(output, 1, "{");
    writeln(output, 2, "if (!$this->__doriaLive) { return; }");
    writeln(output, 2, "$this->__doriaLive = false;");
    for logical_index in &layout.logical_release_order {
        let field = layout
            .fields
            .iter()
            .find(|field| field.logical_index == *logical_index)
            .expect("validated closure release order names a field");
        if field.storage == mir::ClosureEnvironmentStorage::Owned {
            writeln(
                output,
                2,
                &format!("__doria_drop_cell($this->field{});", field.id.0),
            );
        }
    }
    writeln(output, 1, "}");
    output.push('\n');
    writeln(output, 1, "public function __destruct()");
    writeln(output, 1, "{");
    writeln(output, 2, "if (!__doria_cleanup_enabled()) { return; }");
    writeln(
        output,
        2,
        "$this->__doriaDrop(); // Defensive only; compiler-emitted drops define Doria cleanup.",
    );
    writeln(output, 1, "}");
    writeln(output, 0, "}");
    output.push('\n');
}

fn emit_php_closure_carrier(
    descriptor: &PhpClosureDescriptor,
    function_type: &mir::FunctionType,
    program: &mir::Program,
    specialization: &specialization::Plan,
    output: &mut String,
) {
    writeln(
        output,
        0,
        &format!(
            "final class {} implements __DoriaFunctionValue",
            descriptor.carrier_name
        ),
    );
    writeln(output, 0, "{");
    writeln(output, 1, "private bool $__doriaLive = true;");
    if let Some(environment_name) = &descriptor.environment_name {
        writeln(
            output,
            1,
            &format!("private ?{environment_name} $__doriaEnvironment;"),
        );
        writeln(
            output,
            1,
            &format!("public function __construct({environment_name} $environment)"),
        );
        writeln(output, 1, "{");
        writeln(output, 2, "$this->__doriaEnvironment = $environment;");
        writeln(output, 1, "}");
    } else {
        writeln(output, 1, "public function __construct()");
        writeln(output, 1, "{");
        writeln(output, 1, "}");
    }
    output.push('\n');
    write_indent(output, 1);
    output.push_str("public function __invoke(");
    output.push_str(
        &function_type
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let ty = if parameter.mode == mir::FunctionParameterMode::Writable {
                    "__DoriaCell".to_string()
                } else {
                    php_mir_type(parameter.ty, program, specialization)
                };
                format!("{ty} $argument{index}")
            })
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push(')');
    if let mir::ReturnType::Value(ty) = function_type.return_type {
        output.push_str(": ");
        output.push_str(&php_mir_type(ty, program, specialization));
    } else {
        output.push_str(": void");
    }
    output.push('\n');
    writeln(output, 1, "{");
    writeln(
        output,
        2,
        "if (!$this->__doriaLive) { throw new LogicException(\"compiler invariant violated: consumed Doria closure was invoked\"); }",
    );
    if descriptor.invocation_mode == mir::FunctionInvocationMode::Once {
        writeln(output, 2, "$this->__doriaLive = false;");
    }
    let helper = descriptor.owner_class.as_ref().map_or_else(
        || descriptor.helper_name.clone(),
        |class| format!("{}::{}", php_symbol_name(class), descriptor.helper_name),
    );
    let mut arguments = Vec::new();
    if descriptor.environment_name.is_some() {
        arguments.push("$this->__doriaEnvironment".to_string());
    }
    arguments.extend(
        function_type
            .parameters
            .iter()
            .enumerate()
            .map(|(index, _)| format!("$argument{index}")),
    );
    let call = format!("{helper}({})", arguments.join(", "));
    if descriptor.invocation_mode == mir::FunctionInvocationMode::Once
        && descriptor.environment_name.is_some()
    {
        writeln(output, 2, "$environment = $this->__doriaEnvironment;");
        writeln(output, 2, "$this->__doriaEnvironment = null;");
        let call = call.replace("$this->__doriaEnvironment", "$environment");
        if matches!(function_type.return_type, mir::ReturnType::Value(_)) {
            writeln(output, 2, "try {");
            writeln(output, 3, &format!("$result = {call};"));
            writeln(output, 2, "} catch (__DoriaCheckedError $error) {");
            writeln(output, 3, "$environment->__doriaDrop();");
            writeln(output, 3, "throw $error;");
            writeln(output, 2, "}");
            writeln(output, 2, "$environment->__doriaDrop();");
            writeln(output, 2, "return $result;");
        } else {
            writeln(output, 2, "try {");
            writeln(output, 3, &format!("{call};"));
            writeln(output, 2, "} catch (__DoriaCheckedError $error) {");
            writeln(output, 3, "$environment->__doriaDrop();");
            writeln(output, 3, "throw $error;");
            writeln(output, 2, "}");
            writeln(output, 2, "$environment->__doriaDrop();");
        }
    } else if matches!(function_type.return_type, mir::ReturnType::Value(_)) {
        writeln(output, 2, &format!("return {call};"));
    } else {
        writeln(output, 2, &format!("{call};"));
    }
    writeln(output, 1, "}");
    output.push('\n');
    writeln(output, 1, "public function __doriaDrop(): void");
    writeln(output, 1, "{");
    writeln(output, 2, "if (!$this->__doriaLive) { return; }");
    writeln(output, 2, "$this->__doriaLive = false;");
    if descriptor.environment_name.is_some() {
        writeln(
            output,
            2,
            "if ($this->__doriaEnvironment !== null) { $this->__doriaEnvironment->__doriaDrop(); $this->__doriaEnvironment = null; }",
        );
    }
    writeln(output, 1, "}");
    output.push('\n');
    writeln(output, 1, "public function __destruct()");
    writeln(output, 1, "{");
    writeln(output, 2, "if (!__doria_cleanup_enabled()) { return; }");
    writeln(
        output,
        2,
        "$this->__doriaDrop(); // Defensive only; compiler-emitted drops define Doria cleanup.",
    );
    writeln(output, 1, "}");
    writeln(output, 0, "}");
    output.push('\n');
}

fn php_mir_type(
    ty: mir::Type,
    program: &mir::Program,
    specialization: &specialization::Plan,
) -> String {
    match ty {
        mir::Type::Scalar(mir::ScalarType::Integer(_)) => "int".to_string(),
        mir::Type::Scalar(mir::ScalarType::Float(_)) => "float".to_string(),
        mir::Type::Scalar(mir::ScalarType::Bool) => "bool".to_string(),
        mir::Type::Scalar(mir::ScalarType::Enum(id)) => program.enums[id.0].name.clone(),
        mir::Type::String => "string".to_string(),
        mir::Type::Mixed => "mixed".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Integer(_)) => "?int".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Float(_)) => "?float".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Bool) => "?bool".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Enum(id)) => {
            format!("?{}", program.enums[id.0].name)
        }
        mir::Type::NullableString => "?string".to_string(),
        mir::Type::NullableMixed => "mixed".to_string(),
        // The closure descriptor validates the exact Doria contract. This PHP
        // invocation shim accepts its nominal object carrier, not only Error.
        mir::Type::Interface(_) => "object".to_string(),
        mir::Type::NullableInterface(_) => "?object".to_string(),
        mir::Type::Class(id) => specialization.class_symbol(id).to_string(),
        mir::Type::NullableClass(id) => {
            format!("?{}", specialization.class_symbol(id))
        }
        mir::Type::PayloadEnum(ty) => php_symbol_name(&program.enums[ty.id.0].name),
        mir::Type::NullablePayloadEnum(ty) => {
            format!("?{}", php_symbol_name(&program.enums[ty.id.0].name))
        }
        mir::Type::Function(_) => "__DoriaFunctionValue".to_string(),
        mir::Type::NullableFunction(_) => "?__DoriaFunctionValue".to_string(),
        mir::Type::Collection(id) | mir::Type::NullableCollection(id) => {
            let nullable = matches!(ty, mir::Type::NullableCollection(_));
            let definition = &program.collection_types[id.0];
            let base = if definition.uses_core_operations() {
                "__DoriaCoreCollection"
            } else {
                match definition.kind {
                    mir::CollectionKind::SortedDictionary => "__DoriaSortedDictionary",
                    mir::CollectionKind::SortedSet => "__DoriaSortedSet",
                    mir::CollectionKind::PriorityQueue => "__DoriaPriorityQueue",
                    mir::CollectionKind::Deque => "__DoriaDeque",
                    _ => "array",
                }
            };
            if nullable {
                format!("?{base}")
            } else {
                base.to_string()
            }
        }
        mir::Type::SharedReference(_)
        | mir::Type::WeakReference(_)
        | mir::Type::WritableSharedReference(_)
        | mir::Type::WritableWeakReference(_)
        | mir::Type::ReadonlySharedReferenceAccess(_)
        | mir::Type::WritableSharedReferenceAccess(_) => "__DoriaSharedHandle".to_string(),
        mir::Type::NullableSharedReference(_)
        | mir::Type::NullableWeakReference(_)
        | mir::Type::NullableWritableSharedReference(_)
        | mir::Type::NullableWritableWeakReference(_)
        | mir::Type::NullableReadonlySharedReferenceAccess(_)
        | mir::Type::NullableWritableSharedReferenceAccess(_) => "?__DoriaSharedHandle".to_string(),
        mir::Type::ClosureEnvironment(_) => "mixed".to_string(),
    }
}

pub fn generate(program: &Program, mir: Option<&mir::Program>) -> Result<String, BackendError> {
    let mut closure_plan = PhpClosurePlan::build(program, mir);
    let specialization = Rc::new(specialization::Plan::build(program, &closure_plan)?);
    validate_program(program, &specialization)?;
    for descriptor in closure_plan.descriptors.values_mut() {
        let class = match descriptor.source_instance {
            mir::ClosureOwner::Callable(id) => specialization.callable(id).class,
            mir::ClosureOwner::PropertyInitializer(property) => Some(property.class),
        };
        descriptor.owner_class = class.map(|class| specialization.class_symbol(class).to_string());
    }
    let closure_plan = Rc::new(closure_plan);

    let mut output = String::from(
        "<?php\n\ninterface __DoriaDisplayable\n{\n    public function toString(): string;\n}\n\ninterface __DoriaValueEquatable\n{\n    public function __doriaEquals(mixed $other): bool;\n}\n\nfinal class __DoriaMixedValue\n{\n    public function __construct(\n        private readonly string $typeTag,\n        private mixed $value,\n    ) {\n    }\n\n    public function is(string $typeTag): bool { return $this->typeTag === $typeTag; }\n    public function value(): mixed { return $this->value; }\n}\n\nfunction __doria_box_mixed(string $typeTag, mixed $value): __DoriaMixedValue\n{\n    if ($typeTag === 'float32') { $value = unpack('G', pack('G', $value))[1]; }\n    return new __DoriaMixedValue($value === null ? 'null' : $typeTag, $value);\n}\n\nfunction __doria_mixed_is(mixed $value, string $typeTag): bool\n{\n    return $value instanceof __DoriaMixedValue && $value->is($typeTag);\n}\n\nfunction __doria_mixed_value(mixed $value): mixed\n{\n    return $value instanceof __DoriaMixedValue ? $value->value() : $value;\n}\n\nfunction __doria_equal(mixed $left, mixed $right): bool\n{\n    if ($left instanceof __DoriaValueEquatable) { return $left->__doriaEquals($right); }\n    if ($right instanceof __DoriaValueEquatable) { return $right->__doriaEquals($left); }\n    return $left === $right;\n}\n\nfunction __doria_display(string|int|float|bool|__DoriaDisplayable $value): string\n{\n    if ($value instanceof __DoriaDisplayable) { return $value->toString(); }\n    if (is_bool($value)) { return $value ? 'true' : 'false'; }\n    return (string) $value;\n}\n\nfunction __doria_less(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) < 0; }\n    return $left < $right;\n}\n\nfunction __doria_less_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) <= 0; }\n    return $left <= $right;\n}\n\nfunction __doria_greater(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) > 0; }\n    return $left > $right;\n}\n\nfunction __doria_greater_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) >= 0; }\n    return $left >= $right;\n}\n\n",
    );
    output.push_str(
        "function __doria_box_nullable_mixed(string $typeTag, mixed $value): mixed\n{\n    return $value === null ? null : __doria_box_mixed($typeTag, $value);\n}\n\n",
    );
    output.push_str(PHP_ERROR_CONTRACT);
    if program
        .semantic_info
        .classes
        .iter()
        .any(|class| class.implements(BuiltinInterface::Error))
    {
        output.push_str(PHP_CHECKED_ERROR_HELPERS);
    }
    interface::emit_declarations(&program.semantic_info, &mut output);
    output.push_str(PHP_STAGE26_COLLECTION_HELPERS);
    output.push_str(core_collection::RUNTIME);
    emit_php_closure_runtime(&closure_plan, &specialization, mir, &mut output);
    output.push_str(shared::RUNTIME);
    output.push_str("$__doria_sources = [\n");
    if program.sources.is_empty() {
        output.push_str(&format!(
            "    0 => [{}, hex2bin({})],\n",
            emit_php_string_literal(&program.source_path),
            emit_php_string_literal(&hex_bytes(program.source_text.as_bytes())),
        ));
    } else {
        let mut sources = program.sources.iter().collect::<Vec<_>>();
        sources.sort_by_key(|source| source.id);
        for source in sources {
            output.push_str(&format!(
                "    {} => [{}, hex2bin({})],\n",
                source.id.0,
                emit_php_string_literal(&source.display_path),
                emit_php_string_literal(&hex_bytes(source.source.text.as_bytes())),
            ));
        }
    }
    output.push_str("];\n");
    output.push_str("$__doria_catalogue = [\n");
    for entry in doria_diagnostic_catalogue::RUNTIME_CATALOGUE
        .iter()
        .filter(|entry| {
            matches!(
                entry.code,
                "P1000"
                    | "P1001"
                    | "P1311"
                    | "P1310"
                    | "P1312"
                    | "P1401"
                    | "P1402"
                    | "P1403"
                    | "P1404"
                    | "P1405"
                    | "P1406"
                    | "P1407"
                    | "P1501"
                    | "P1502"
                    | "P1503"
                    | "P1504"
                    | "P1505"
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
                emit_php_string_literal(&php_function_name(&function.name)),
                php_source_location(function.span, function.span.start),
            )),
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(function) = member {
                        output.push_str(&format!(
                            "    {} => {},\n",
                            emit_php_string_literal(&format!(
                                "{}::{}",
                                php_symbol_name(&class.name),
                                function.name
                            )),
                            php_source_location(function.span, function.span.start),
                        ));
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    output.push_str("];\n$__doria_function_names = [\n");
    for item in &program.items {
        match item {
            Item::Function(function) => output.push_str(&format!(
                "    {} => {},\n",
                emit_php_string_literal(&php_function_name(&function.name)),
                emit_php_string_literal(&function.name),
            )),
            Item::Class(class) => {
                for member in &class.members {
                    if let ClassMember::Method(function) = member {
                        let php_name =
                            format!("{}::{}", php_symbol_name(&class.name), function.name);
                        let source_name = format!("{}::{}", class.name, function.name);
                        output.push_str(&format!(
                            "    {} => {},\n",
                            emit_php_string_literal(&php_name),
                            emit_php_string_literal(&source_name),
                        ));
                    }
                }
            }
            Item::Enum(_) | Item::Constant(_) | Item::Statement(_) => {}
        }
    }
    output.push_str("];\n$__doria_generated_closure_frames = [\n");
    let mut closure_descriptors = closure_plan.descriptors.values().collect::<Vec<_>>();
    closure_descriptors.sort_by_key(|descriptor| descriptor.descriptor.0);
    for descriptor in closure_descriptors {
        let helper = descriptor.owner_class.as_ref().map_or_else(
            || descriptor.helper_name.clone(),
            |class| format!("{}::{}", php_symbol_name(class), descriptor.helper_name),
        );
        output.push_str(&format!(
            "    {},\n    {},\n",
            emit_php_string_literal(&helper),
            emit_php_string_literal(&format!("{}::__invoke", descriptor.carrier_name)),
        ));
    }
    output.push_str("];\n\n");
    emit_checked_io_message_vocabulary(&mut output);
    output.push_str(
        r#"function __doria_source_location(int $location): array
{
    global $__doria_sources;
    $sourceId = intdiv($location, 4294967296);
    $offset = $location % 4294967296;
    [$path, $text] = $__doria_sources[$sourceId] ?? ["<unknown>", ""];
    return [$path, $text, $offset];
}

function __doria_source_line(int $location): int
{
    [, $text, $offset] = __doria_source_location($location);
    return substr_count(substr($text, 0, max(0, $offset)), "\n") + 1;
}

function __doria_panic(
    string $code,
    int $start,
    int $end,
    ?string $message = null,
    ?string $callable = null,
    array $facts = [],
)
{
    global $__doria_catalogue, $__doria_function_spans, $__doria_function_names, $__doria_generated_closure_frames, $__doria_panicking;
    if (!isset($__doria_catalogue[$code])) { $code = "P1001"; }
    [$title, $label, $why] = $__doria_catalogue[$code];
    if ($code === "P1501" && isset($facts["conflictReason"])) { $why = $facts["conflictReason"]; }
    [$sourcePath, $sourceText, $sourceStart] = __doria_source_location($start);
    [, , $sourceEnd] = __doria_source_location($end);
    $line = __doria_source_line($start);
    $helperFunctions = [
        "__doria_panic",
        "__doria_source_location",
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
        "__doria_collection_index",
        "__doria_collection_set",
        "__doria_replace_cell",
        "__doria_drop_cell",
        "__doria_drop_value",
        "__doria_shared_retain",
        "__doria_shared_release_count",
    ];
    $frames = [];
    foreach (debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS) as $frame) {
        if (!isset($frame["function"])) { continue; }
        if (in_array($frame["class"] ?? null, ["__DoriaSharedControl", "__DoriaSharedHandle"], true)
            || $frame["function"] === "{closure}" || str_starts_with($frame["function"], "{closure:")) { continue; }
        $frameName = isset($frame["class"])
            ? $frame["class"] . "::" . $frame["function"]
            : $frame["function"];
        if (!isset($frame["class"]) && in_array($frame["function"], $helperFunctions, true)) {
            continue;
        }
        if (in_array($frameName, $__doria_generated_closure_frames, true)) { continue; }
        $frames[] = $frame;
    }
    $function = $callable ?? "main";
    if ($callable === null && isset($frames[0])) {
        $phpFunction = isset($frames[0]["class"])
            ? $frames[0]["class"] . "::" . $frames[0]["function"]
            : $frames[0]["function"];
        $function = $__doria_function_names[$phpFunction] ?? $phpFunction;
    }
    @fwrite(STDERR, "Panic[" . $code . "]: " . $title . "\n\nWhere\n");
    @fwrite(STDERR, $sourcePath . " · line " . $line . " · " . $function . "\n\n");
    $before = substr($sourceText, 0, max(0, $sourceStart));
    $lineStart = strrpos($before, "\n");
    $lineStart = $lineStart === false ? 0 : $lineStart + 1;
    $lineEnd = strpos($sourceText, "\n", max(0, $sourceStart));
    $lineEnd = $lineEnd === false ? strlen($sourceText) : $lineEnd;
    $lineText = rtrim(substr($sourceText, $lineStart, $lineEnd - $lineStart), "\r");
    $prefix = substr($sourceText, $lineStart, max(0, $sourceStart - $lineStart));
    $selected = substr($sourceText, max(0, $sourceStart), max(1, $sourceEnd - $sourceStart));
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
    if ($facts !== [] && $code !== "P1501") {
        @fwrite(STDERR, "\n\nFacts");
        foreach ($facts as $name => $value) { @fwrite(STDERR, "\n" . $name . ": " . $value); }
    }
    @fwrite(STDERR, "\n\nCall Path");
    foreach ($frames as $index => $frame) {
        $phpName = isset($frame["class"])
            ? $frame["class"] . "::" . $frame["function"]
            : $frame["function"];
        $name = $__doria_function_names[$phpName] ?? $phpName;
        $frameOffset = $index === 0 ? $start : ($__doria_function_spans[$phpName] ?? $start);
        [$framePath] = __doria_source_location($frameOffset);
        @fwrite(
            STDERR,
            "\n" . $name . " · " . $framePath . ":" . __doria_source_line($frameOffset)
        );
    }
    @fwrite(STDERR, "\n\nProcess Exited With Status 101\n");
    $__doria_panicking = true;
    exit(101);
}

function __doria_read_line(string $prompt, int $start, int $end, string $callable): ?string
{
    if ($prompt !== "") {
        __doria_write_all(
            STDOUT,
            $prompt,
            $start,
            $end,
            __DoriaStdIoIoTarget::__doriaCaseStandardOutput(),
            __DoriaStdIoIoOperation::Write,
            $callable,
        );
    }
    __doria_flush_stdout($start, $end, $callable);
    error_clear_last();
    $line = @fgets(STDIN);
    if ($line === false) {
        if (feof(STDIN)) { return null; }
        __doria_throw_io(
            __DoriaStdIoIoOperation::Read,
            __DoriaStdIoIoTarget::__doriaCaseStandardInput(),
            error_get_last(),
            $start,
            $callable,
        );
    }
    if (str_ends_with($line, "\n")) {
        $line = substr($line, 0, -1);
        if (str_ends_with($line, "\r")) { $line = substr($line, 0, -1); }
    }
    $invalid = __doria_invalid_utf8($line);
    if ($invalid !== null) {
        [$valid, $length] = $invalid;
        __doria_throw(
            new __DoriaStdIoInvalidUtf8Error(
                __doria_invalid_utf8_message(
                    __DoriaStdIoUtf8InputSource::__doriaCaseStandardInput()
                ),
                __DoriaStdIoUtf8InputSource::__doriaCaseStandardInput(),
                $valid,
                $length,
            ),
            $start,
            $callable,
        );
    }
    return $line;
}

function __doria_read_file(string $path, int $start, int $end, string $callable): string
{
    if (str_contains($path, "\0")) {
        __doria_throw_io_validation(__DoriaStdIoIoOperation::Open, $path, $start, $callable);
    }
    error_clear_last();
    $file = @fopen($path, "rb");
    if ($file === false) {
        __doria_throw_io(
            __DoriaStdIoIoOperation::Open,
            __DoriaStdIoIoTarget::File($path),
            error_get_last(),
            $start,
            $callable,
        );
    }
    $contents = "";
    while (!feof($file)) {
        error_clear_last();
        $chunk = @fread($file, 8192);
        if ($chunk === false) {
            @fclose($file);
            __doria_throw_io(
                __DoriaStdIoIoOperation::Read,
                __DoriaStdIoIoTarget::File($path),
                error_get_last(),
                $start,
                $callable,
            );
        }
        $contents .= $chunk;
    }
    error_clear_last();
    if (!@fclose($file)) {
        __doria_throw_io(
            __DoriaStdIoIoOperation::Read,
            __DoriaStdIoIoTarget::File($path),
            error_get_last(),
            $start,
            $callable,
        );
    }
    $invalid = __doria_invalid_utf8($contents);
    if ($invalid !== null) {
        [$valid, $length] = $invalid;
        __doria_throw(
            new __DoriaStdIoInvalidUtf8Error(
                __doria_invalid_utf8_message(__DoriaStdIoUtf8InputSource::File($path)),
                __DoriaStdIoUtf8InputSource::File($path),
                $valid,
                $length,
            ),
            $start,
            $callable,
        );
    }
    return $contents;
}

function __doria_write_file(
    string $path,
    string $contents,
    int $start,
    int $end,
    string $callable,
): void
{
    __doria_write_file_mode($path, $contents, false, $start, $end, $callable);
}

function __doria_append_file(
    string $path,
    string $contents,
    int $start,
    int $end,
    string $callable,
): void
{
    __doria_write_file_mode($path, $contents, true, $start, $end, $callable);
}

function __doria_write_file_mode(
    string $path,
    string $contents,
    bool $append,
    int $start,
    int $end,
    string $callable,
): void {
    if (str_contains($path, "\0")) {
        __doria_throw_io_validation(__DoriaStdIoIoOperation::Open, $path, $start, $callable);
    }
    error_clear_last();
    $file = @fopen($path, $append ? "ab" : "wb");
    if ($file === false) {
        __doria_throw_io(
            __DoriaStdIoIoOperation::Open,
            __DoriaStdIoIoTarget::File($path),
            error_get_last(),
            $start,
            $callable,
        );
    }
    __doria_write_all(
        $file,
        $contents,
        $start,
        $end,
        __DoriaStdIoIoTarget::File($path),
        $append ? __DoriaStdIoIoOperation::Append : __DoriaStdIoIoOperation::Write,
        $callable,
    );
    error_clear_last();
    if (!@fclose($file)) {
        __doria_throw_io(
            $append ? __DoriaStdIoIoOperation::Append : __DoriaStdIoIoOperation::Write,
            __DoriaStdIoIoTarget::File($path),
            error_get_last(),
            $start,
            $callable,
        );
    }
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

function __doria_system_code(?array $error): ?int
{
    if ($error === null || !isset($error["message"])) { return null; }
    if (preg_match('/\berrno=(\d+)\b/', $error["message"], $match) !== 1) { return null; }
    return (int) $match[1];
}

function __doria_io_reason(?array $error): __DoriaStdIoIoErrorReason
{
    $code = __doria_system_code($error);
    $reason = PHP_OS_FAMILY === "Windows" ? match ($code) {
            2, 3 => __DoriaStdIoIoErrorReason::NotFound,
            5, 32 => __DoriaStdIoIoErrorReason::PermissionDenied,
            1, 87, 123, 206 => __DoriaStdIoIoErrorReason::InvalidInput,
            995 => __DoriaStdIoIoErrorReason::Interrupted,
            4, 8, 14, 39, 112 => __DoriaStdIoIoErrorReason::ResourceExhausted,
            50 => __DoriaStdIoIoErrorReason::Unsupported,
            6, 109, 232 => __DoriaStdIoIoErrorReason::Closed,
            default => __DoriaStdIoIoErrorReason::Other,
        } : match ($code) {
        2 => __DoriaStdIoIoErrorReason::NotFound,
        1, 13 => __DoriaStdIoIoErrorReason::PermissionDenied,
        22, 36, 63 => __DoriaStdIoIoErrorReason::InvalidInput,
        4 => __DoriaStdIoIoErrorReason::Interrupted,
        12, 23, 24, 28 => __DoriaStdIoIoErrorReason::ResourceExhausted,
        38, 45, 78, 95 => __DoriaStdIoIoErrorReason::Unsupported,
        9, 32 => __DoriaStdIoIoErrorReason::Closed,
        default => __DoriaStdIoIoErrorReason::Other,
    };
    if ($reason !== __DoriaStdIoIoErrorReason::Other) { return $reason; }
    if ($error === null || !isset($error["message"])) { return $reason; }

    $message = strtolower($error["message"]);
    if (str_contains($message, "no such file or directory") ||
        str_contains($message, "cannot find the file") ||
        str_contains($message, "cannot find the path")) {
        return __DoriaStdIoIoErrorReason::NotFound;
    }
    if (str_contains($message, "permission denied") ||
        str_contains($message, "access is denied")) {
        return __DoriaStdIoIoErrorReason::PermissionDenied;
    }
    if (str_contains($message, "invalid argument") ||
        str_contains($message, "filename, directory name, or volume label syntax is incorrect")) {
        return __DoriaStdIoIoErrorReason::InvalidInput;
    }
    if (str_contains($message, "interrupted system call")) {
        return __DoriaStdIoIoErrorReason::Interrupted;
    }
    if (str_contains($message, "too many open files") ||
        str_contains($message, "no space left") ||
        str_contains($message, "not enough memory") ||
        str_contains($message, "insufficient system resources")) {
        return __DoriaStdIoIoErrorReason::ResourceExhausted;
    }
    if (str_contains($message, "operation not supported") ||
        str_contains($message, "not supported")) {
        return __DoriaStdIoIoErrorReason::Unsupported;
    }
    if (str_contains($message, "bad file descriptor") ||
        str_contains($message, "broken pipe") ||
        str_contains($message, "pipe has been ended") ||
        str_contains($message, "closed stream")) {
        return __DoriaStdIoIoErrorReason::Closed;
    }
    return $reason;
}

function __doria_io_operation_word(__DoriaStdIoIoOperation $operation): string
{
    global $__doria_io_message_vocabulary;
    return match ($operation) {
        __DoriaStdIoIoOperation::Open => $__doria_io_message_vocabulary["operations"][0],
        __DoriaStdIoIoOperation::Read => $__doria_io_message_vocabulary["operations"][1],
        __DoriaStdIoIoOperation::Write => $__doria_io_message_vocabulary["operations"][2],
        __DoriaStdIoIoOperation::Append => $__doria_io_message_vocabulary["operations"][3],
        __DoriaStdIoIoOperation::Flush => $__doria_io_message_vocabulary["operations"][4],
    };
}

function __doria_io_reason_words(__DoriaStdIoIoErrorReason $reason): string
{
    global $__doria_io_message_vocabulary;
    return match ($reason) {
        __DoriaStdIoIoErrorReason::NotFound => $__doria_io_message_vocabulary["reasons"][0],
        __DoriaStdIoIoErrorReason::PermissionDenied => $__doria_io_message_vocabulary["reasons"][1],
        __DoriaStdIoIoErrorReason::InvalidInput => $__doria_io_message_vocabulary["reasons"][2],
        __DoriaStdIoIoErrorReason::Interrupted => $__doria_io_message_vocabulary["reasons"][3],
        __DoriaStdIoIoErrorReason::ResourceExhausted => $__doria_io_message_vocabulary["reasons"][4],
        __DoriaStdIoIoErrorReason::Unsupported => $__doria_io_message_vocabulary["reasons"][5],
        __DoriaStdIoIoErrorReason::Closed => $__doria_io_message_vocabulary["reasons"][6],
        __DoriaStdIoIoErrorReason::Other => $__doria_io_message_vocabulary["reasons"][7],
    };
}

function __doria_io_target_name(__DoriaStdIoIoTarget $target): string
{
    global $__doria_io_message_vocabulary;
    if ($target->__doriaMatchesCase(0)) {
        return $__doria_io_message_vocabulary["filePrefix"] .
            $target->__doriaPayloadAt(0) . $__doria_io_message_vocabulary["fileSuffix"];
    }
    if ($target->__doriaMatchesCase(1)) { return $__doria_io_message_vocabulary["stdin"]; }
    if ($target->__doriaMatchesCase(2)) { return $__doria_io_message_vocabulary["stdout"]; }
    return $__doria_io_message_vocabulary["stderr"];
}

function __doria_invalid_utf8_message(__DoriaStdIoUtf8InputSource $source): string
{
    global $__doria_io_message_vocabulary;
    $target = $source->__doriaMatchesCase(0)
        ? $__doria_io_message_vocabulary["filePrefix"] . $source->__doriaPayloadAt(0) .
            $__doria_io_message_vocabulary["fileSuffix"]
        : $__doria_io_message_vocabulary["stdin"];
    return $__doria_io_message_vocabulary["invalidUtf8Prefix"] . $target;
}

function __doria_throw_io(
    __DoriaStdIoIoOperation $operation,
    __DoriaStdIoIoTarget $target,
    ?array $hostError,
    int $origin,
    string $callable,
): void {
    global $__doria_io_message_vocabulary;
    $systemCode = __doria_system_code($hostError);
    $reason = __doria_io_reason($hostError);
    $message = $__doria_io_message_vocabulary["ioPrefix"] .
        __doria_io_operation_word($operation) . " " . __doria_io_target_name($target) .
        $__doria_io_message_vocabulary["separator"] . __doria_io_reason_words($reason);
    __doria_throw(new __DoriaStdIoIoError(
        $message,
        $operation,
        $target,
        $reason,
        $systemCode,
    ), $origin, $callable);
}

function __doria_throw_io_validation(
    __DoriaStdIoIoOperation $operation,
    string $path,
    int $origin,
    string $callable,
): void {
    global $__doria_io_message_vocabulary;
    $target = __DoriaStdIoIoTarget::File($path);
    $reason = __DoriaStdIoIoErrorReason::InvalidInput;
    $message = $__doria_io_message_vocabulary["ioPrefix"] .
        __doria_io_operation_word($operation) . " " . __doria_io_target_name($target) .
        $__doria_io_message_vocabulary["separator"] . __doria_io_reason_words($reason);
    __doria_throw(new __DoriaStdIoIoError(
        $message,
        $operation,
        $target,
        $reason,
        null,
    ), $origin, $callable);
}

function __doria_invalid_utf8(string $value): ?array
{
    $length = strlen($value);
    for ($index = 0; $index < $length;) {
        $first = ord($value[$index]);
        if ($first <= 0x7f) { ++$index; continue; }
        if ($first >= 0xc2 && $first <= 0xdf) {
            $width = 2; $secondMin = 0x80; $secondMax = 0xbf;
        } elseif ($first === 0xe0) {
            $width = 3; $secondMin = 0xa0; $secondMax = 0xbf;
        } elseif (($first >= 0xe1 && $first <= 0xec) || ($first >= 0xee && $first <= 0xef)) {
            $width = 3; $secondMin = 0x80; $secondMax = 0xbf;
        } elseif ($first === 0xed) {
            $width = 3; $secondMin = 0x80; $secondMax = 0x9f;
        } elseif ($first === 0xf0) {
            $width = 4; $secondMin = 0x90; $secondMax = 0xbf;
        } elseif ($first >= 0xf1 && $first <= 0xf3) {
            $width = 4; $secondMin = 0x80; $secondMax = 0xbf;
        } elseif ($first === 0xf4) {
            $width = 4; $secondMin = 0x80; $secondMax = 0x8f;
        } else {
            return [$index, 1];
        }
        if ($index + $width > $length) { return [$index, null]; }
        $second = ord($value[$index + 1]);
        if ($second < $secondMin || $second > $secondMax) { return [$index, 1]; }
        for ($offset = 2; $offset < $width; ++$offset) {
            $continuation = ord($value[$index + $offset]);
            if ($continuation < 0x80 || $continuation > 0xbf) { return [$index, 1]; }
        }
        $index += $width;
    }
    return null;
}

function __doria_write_all(
    mixed $stream,
    string $value,
    int $start,
    int $end,
    __DoriaStdIoIoTarget $target,
    __DoriaStdIoIoOperation $operation,
    string $callable,
): void
{
    $offset = 0;
    $length = strlen($value);
    while ($offset < $length) {
        error_clear_last();
        $written = @fwrite($stream, substr($value, $offset));
        if ($written === false || $written === 0) {
            if (__doria_is_broken_pipe(error_get_last())) { exit(0); }
            __doria_throw_io($operation, $target, error_get_last(), $start, $callable);
        }
        $offset += $written;
    }
}

function __doria_flush_stdout(int $start, int $end, string $callable): void
{
    error_clear_last();
    if (@fflush(STDOUT)) { return; }
    if (__doria_is_broken_pipe(error_get_last())) { exit(0); }
    __doria_throw_io(
        __DoriaStdIoIoOperation::Flush,
        __DoriaStdIoIoTarget::__doriaCaseStandardOutput(),
        error_get_last(),
        $start,
        $callable,
    );
}

function __doria_write_stdout(string $value, int $start, int $end, string $callable): void
{
    __doria_write_all(
        STDOUT,
        $value,
        $start,
        $end,
        __DoriaStdIoIoTarget::__doriaCaseStandardOutput(),
        __DoriaStdIoIoOperation::Write,
        $callable,
    );
}

function __doria_write_stderr(string $value, int $start, int $end, string $callable): void
{
    __doria_write_all(
        STDERR,
        $value,
        $start,
        $end,
        __DoriaStdIoIoTarget::__doriaCaseStandardError(),
        __DoriaStdIoIoOperation::Write,
        $callable,
    );
}

function __doria_sprintf(string $format, mixed ...$values): string
{
    return sprintf($format, ...$values);
}

function __doria_printf(
    int $start,
    int $end,
    string $callable,
    string $format,
    mixed ...$values,
): void
{
    $value = sprintf($format, ...$values);
    __doria_write_stdout($value, $start, $end, $callable);
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
    let mut classes_with_php_constructors = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class)
                if class.members.iter().any(|member| {
                    matches!(member, ClassMember::Method(method) if method.name == "__construct")
                        || matches!(
                            member,
                            ClassMember::Property(property)
                                if !property.is_static
                                    && property.initializer.as_ref().is_some_and(|value| {
                                        is_payload_enum_expression(value, &program.semantic_info)
                                            || requires_php_runtime_property_initializer(
                                                value,
                                                &program.semantic_info,
                                            )
                                            .is_some()
                                    })
                        )
                }) =>
            {
                Some(class.name.clone())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    let (mut classes_with_php_destructors, payload_enums_with_php_destructors) =
        php_explicit_drop_types(program);
    loop {
        let mut changed = false;
        for class in program.items.iter().filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        }) {
            let Some(parent) = &class.parent else {
                continue;
            };
            if classes_with_php_constructors.contains(&parent.name) {
                changed |= classes_with_php_constructors.insert(class.name.clone());
            }
            if classes_with_php_destructors.contains(&parent.name) {
                changed |= classes_with_php_destructors.insert(class.name.clone());
            }
        }
        if !changed {
            break;
        }
    }
    let mut scopes = PhpNameScopes::new(
        PhpScopeSymbols {
            interface_names: program
                .semantic_info
                .contracts
                .interfaces
                .iter()
                .map(|interface| interface.name.clone())
                .collect(),
            static_properties,
            payload_unit_cases,
            payload_top_constants,
            payload_class_constants,
            payload_enum_expressions,
            classes_with_php_constructors,
            classes_with_php_destructors,
            payload_enums_with_php_destructors,
        },
        closure_plan,
    );
    scopes.matches = program.semantic_info.matches.clone();
    scopes.specialization = specialization;
    scopes.whens = program.semantic_info.whens.clone();
    scopes.given_preludes = program.semantic_info.given_preludes.clone();
    scopes.expression_types = php_expression_types(&program.semantic_info);
    scopes.interface_conversion_types = program.semantic_info.interface_conversion_types.clone();
    scopes.type_test_types = program.semantic_info.type_test_types.clone();
    scopes.mixed_box_plans = program.semantic_info.mixed_box_plans.clone();
    scopes.throw_error_types = program.semantic_info.throw_error_types.clone();
    scopes.catch_error_types = program.semantic_info.catch_error_types.clone();
    scopes.direct_parent_calls = program
        .semantic_info
        .call_targets
        .iter()
        .filter_map(|(span, target)| match target {
            crate::semantics::CallableTarget::Method {
                direct_parent: true,
                ..
            } => Some(*span),
            _ => None,
        })
        .collect();
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
    emit_closure_entries(None, &mut output, 0, &scopes);
    for item in &program.items {
        emit_item(item, &program.semantic_info, &mut output, 0, &mut scopes);
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push('\n');
    }
    if let Some(entry) = program.items.iter().find_map(|item| match item {
        Item::Function(function) if crate::names::source_name_is(&function.name, "main") => {
            Some(function)
        }
        _ => None,
    }) {
        let arguments = if entry.params.is_empty() {
            String::new()
        } else {
            "array_slice($_SERVER['argv'] ?? [], 1)".to_string()
        };
        let invocation = format!("{}({arguments})", php_function_name(&entry.name));
        output.push_str("if (isset($_SERVER['SCRIPT_FILENAME']) && realpath($_SERVER['SCRIPT_FILENAME']) === __FILE__) {\n    ");
        if entry
            .return_type
            .as_ref()
            .is_some_and(|return_type| return_type.name == "int")
        {
            output.push_str("exit(");
            output.push_str(&invocation);
            output.push_str(");");
        } else {
            output.push_str(&invocation);
            output.push(';');
        }
        output.push_str("\n}\n");
    }
    Ok(output)
}

struct PhpValidation<'a> {
    semantic: &'a SemanticInfo,
    plan: &'a specialization::Plan,
    substitutions: &'a HashMap<String, ResolvedType>,
    expression_types: HashMap<Span, ResolvedType>,
    shared_construction_types: HashMap<Span, ResolvedType>,
}

fn php_expression_types(semantic: &SemanticInfo) -> HashMap<Span, ResolvedType> {
    let mut types = semantic.expression_types.clone();
    types.extend(semantic.collection_construction_types.clone());
    for (span, integer) in &semantic.integer_expression_types {
        if matches!(types.get(span), Some(ResolvedType::Integer(_))) {
            types.insert(*span, ResolvedType::Integer(*integer));
        }
    }
    types
}

impl std::ops::Deref for PhpValidation<'_> {
    type Target = SemanticInfo;
    fn deref(&self) -> &Self::Target {
        self.semantic
    }
}

impl<'a> PhpValidation<'a> {
    fn new(
        semantic: &'a SemanticInfo,
        plan: &'a specialization::Plan,
        substitutions: &'a HashMap<String, ResolvedType>,
    ) -> Self {
        let specialize = |types: &HashMap<Span, ResolvedType>| {
            types
                .iter()
                .map(|(span, ty)| {
                    (
                        *span,
                        crate::types::substitute_resolved_type(ty, substitutions),
                    )
                })
                .collect()
        };
        Self {
            semantic,
            plan,
            substitutions,
            expression_types: specialize(&php_expression_types(semantic)),
            shared_construction_types: specialize(&semantic.shared_construction_types),
        }
    }

    fn expression_type(&self, span: Span) -> Option<&ResolvedType> {
        self.expression_types.get(&span)
    }

    fn float_type(&self, span: Span) -> Option<FloatType> {
        match self.expression_type(span) {
            Some(ResolvedType::Float(ty)) => Some(*ty),
            _ => self.semantic.float_type(span),
        }
    }
}

fn validate_program(program: &Program, plan: &specialization::Plan) -> Result<(), BackendError> {
    let empty = HashMap::new();
    let context = PhpValidation::new(&program.semantic_info, plan, &empty);
    for item in &program.items {
        match item {
            Item::Function(function) => {
                for callable in plan.callables.iter().filter(|callable| {
                    callable.class.is_none() && callable.declaration == function.span
                }) {
                    let context =
                        PhpValidation::new(&program.semantic_info, plan, &callable.substitutions);
                    validate_function(function, &context, false)?;
                }
            }
            Item::Class(declaration) => {
                for class in program
                    .semantic_info
                    .classes
                    .iter()
                    .filter(|class| class.declaration_name == declaration.name)
                {
                    let substitutions = declaration
                        .type_params
                        .iter()
                        .zip(&class.arguments)
                        .map(|(parameter, ty)| (parameter.name.clone(), ty.clone()))
                        .collect();
                    let context = PhpValidation::new(&program.semantic_info, plan, &substitutions);
                    validate_class(declaration, &context)?;
                    for method in declaration
                        .members
                        .iter()
                        .filter_map(|member| match member {
                            ClassMember::Method(method) => Some(method),
                            _ => None,
                        })
                    {
                        for callable in plan.callables.iter().filter(|callable| {
                            callable.class == Some(class.id) && callable.declaration == method.span
                        }) {
                            let context = PhpValidation::new(
                                &program.semantic_info,
                                plan,
                                &callable.substitutions,
                            );
                            validate_function(method, &context, true)?;
                        }
                    }
                }
            }
            _ => validate_item(item, &context)?,
        }
    }
    Ok(())
}

fn validate_class(
    class_decl: &ClassDecl,
    semantic_info: &PhpValidation<'_>,
) -> Result<(), BackendError> {
    for member in &class_decl.members {
        match member {
            ClassMember::Property(property) => {
                validate_type(&property.ty, property.span, semantic_info)?;
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
                }
            }
            ClassMember::Method(_) => {}
            ClassMember::Constant(constant) => {
                validate_php_class_constant_name(constant)?;
                if let Some(ty) = &constant.ty {
                    validate_type(ty, constant.span, semantic_info)?;
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

fn validate_item(item: &Item, semantic_info: &PhpValidation<'_>) -> Result<(), BackendError> {
    match item {
        Item::Class(_) | Item::Function(_) => {
            unreachable!("callables are validated per concrete instance")
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
                    validate_type(&field.ty, field.span, semantic_info)?;
                }
                if let Some(value) = &case.backing_value {
                    validate_expr(value, semantic_info)?;
                }
            }
            Ok(())
        }
        Item::Constant(constant) => {
            if let Some(ty) = &constant.ty {
                validate_type(ty, constant.span, semantic_info)?;
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
        ConstValue::Integer(value)
            if !value.ty.is_default_int() && value.ty != IntegerType::UInt64 =>
        {
            Err(unsupported_integer_shape(
                span,
                format!(
                    "Doria `{}` width and signedness with PHP's single signed integer type",
                    value.ty.source_name()
                ),
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
    semantic_info: &PhpValidation<'_>,
    is_method: bool,
) -> Result<(), BackendError> {
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
        validate_type(&param.ty, param.span, semantic_info)?;
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
        validate_type(return_type, function.span, semantic_info)?;
    }
    validate_block(&function.body, semantic_info)
}

fn php_type_ref_needs_explicit_drop(
    ty: &TypeRef,
    classes: &HashSet<String>,
    payload_enums: &HashSet<String>,
    type_parameters: &HashSet<String>,
    interfaces: &HashSet<String>,
) -> bool {
    ty.function.is_some()
        || ty.name == "mixed"
        || ty.name == "Error"
        || interfaces.contains(&ty.name)
        || crate::types::SharedHandleKind::from_source_name(&ty.name).is_some()
        || classes.contains(&ty.name)
        || payload_enums.contains(&ty.name)
        || type_parameters.contains(&ty.name)
        || ty.type_arguments().any(|argument| {
            php_type_ref_needs_explicit_drop(
                argument,
                classes,
                payload_enums,
                type_parameters,
                interfaces,
            )
        })
}

fn php_explicit_drop_types(program: &Program) -> (HashSet<String>, HashSet<String>) {
    let interfaces = program
        .semantic_info
        .contracts
        .interfaces
        .iter()
        .map(|interface| interface.name.clone())
        .collect::<HashSet<_>>();
    let classes = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .collect::<Vec<_>>();
    let enums = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(declaration) => Some(declaration),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut drop_classes = classes
        .iter()
        .filter(|class| {
            class.members.iter().any(
                |member| matches!(member, ClassMember::Method(method) if method.name == "__destruct"),
            )
        })
        .map(|class| class.name.clone())
        .collect::<HashSet<_>>();
    let mut drop_enums = HashSet::new();

    loop {
        let mut changed = false;
        for declaration in &enums {
            let type_parameters = declaration
                .type_params
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect::<HashSet<_>>();
            if declaration.cases.iter().any(|case| {
                case.payload.iter().any(|field| {
                    php_type_ref_needs_explicit_drop(
                        &field.ty,
                        &drop_classes,
                        &drop_enums,
                        &type_parameters,
                        &interfaces,
                    )
                })
            }) {
                changed |= drop_enums.insert(declaration.name.clone());
            }
        }
        for class in &classes {
            let type_parameters = class
                .type_params
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect::<HashSet<_>>();
            let owns_observable_value = class.members.iter().any(|member| match member {
                ClassMember::Property(property) if !property.is_static => {
                    php_type_ref_needs_explicit_drop(
                        &property.ty,
                        &drop_classes,
                        &drop_enums,
                        &type_parameters,
                        &interfaces,
                    )
                }
                ClassMember::Method(method) if method.name == "__construct" => {
                    method.params.iter().any(|parameter| {
                        parameter.constructor_role.is_promoted()
                            && !parameter.borrow
                            && php_type_ref_needs_explicit_drop(
                                &parameter.ty,
                                &drop_classes,
                                &drop_enums,
                                &type_parameters,
                                &interfaces,
                            )
                    })
                }
                _ => false,
            });
            if owns_observable_value {
                changed |= drop_classes.insert(class.name.clone());
            }
        }
        for class in &classes {
            let Some(parent) = &class.parent else {
                continue;
            };
            if drop_classes.contains(&class.name) || drop_classes.contains(&parent.name) {
                changed |= drop_classes.insert(class.name.clone());
                changed |= drop_classes.insert(parent.name.clone());
            }
        }
        if !changed {
            return (drop_classes, drop_enums);
        }
    }
}

fn validate_type(
    ty: &TypeRef,
    span: Span,
    context: &PhpValidation<'_>,
) -> Result<(), BackendError> {
    if let Some(ty) = context.plan.resolve_type(ty, context.substitutions) {
        validate_resolved_type(&ty, span)?;
    }
    Ok(())
}

fn validate_resolved_type(ty: &ResolvedType, span: Span) -> Result<(), BackendError> {
    match ty {
        ResolvedType::SharedHandle(_, payload) => {
            if !shared::supported_payload(ty) {
                return Err(unsupported_shared_ownership(span));
            }
            validate_resolved_type(payload, span)
        }
        ResolvedType::Nullable(inner)
        | ResolvedType::TypedArray(inner)
        | ResolvedType::List(inner)
        | ResolvedType::Set(inner)
        | ResolvedType::SortedSet(inner)
        | ResolvedType::PriorityQueue(inner)
        | ResolvedType::Deque(inner) => validate_resolved_type(inner, span),
        ResolvedType::Class(class) | ResolvedType::Interface(class) => {
            for ty in &class.arguments {
                validate_resolved_type(ty, span)?;
            }
            Ok(())
        }
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            validate_resolved_type(key, span)?;
            validate_resolved_type(value, span)
        }
        ResolvedType::Function(function) => {
            for parameter in &function.parameters {
                validate_resolved_type(&parameter.ty, span)?;
            }
            validate_resolved_type(&function.return_type, span)
        }
        _ => Ok(()),
    }
}

fn unsupported_shared_ownership(span: Span) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::unsupported_stage(
        PHP_COLLECTION_UNSUPPORTED_CODE,
        "PHP compatibility backend cannot preserve Doria shared ownership; use the `native` or `debug` target for this valid Doria program",
        span,
    )])
}

fn validate_block(block: &Block, semantic_info: &PhpValidation<'_>) -> Result<(), BackendError> {
    for statement in &block.statements {
        validate_statement(statement, semantic_info)?;
    }
    Ok(())
}

fn validate_statement(
    statement: &Stmt,
    semantic_info: &PhpValidation<'_>,
) -> Result<(), BackendError> {
    match statement {
        Stmt::Block(block) => validate_block(block, semantic_info),
        Stmt::VarDecl(decl) => {
            if let Some(ty) = &decl.ty {
                validate_type(ty, decl.span, semantic_info)?;
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
            if let Some(given) = &while_stmt.given {
                validate_given(given, semantic_info)?;
            }
            validate_expr(&while_stmt.condition, semantic_info)?;
            validate_block(&while_stmt.body, semantic_info)?;
            if let Some(finally) = &while_stmt.finally {
                validate_block(&finally.block, semantic_info)?;
            }
            Ok(())
        }
        Stmt::DoWhile(do_while) => {
            validate_block(&do_while.body, semantic_info)?;
            validate_expr(&do_while.condition, semantic_info)?;
            if let Some(finally) = &do_while.finally {
                validate_block(&finally.block, semantic_info)?;
            }
            Ok(())
        }
        Stmt::For(for_stmt) => {
            if let Some(initializer) = &for_stmt.initializer {
                match initializer {
                    ForInitializer::VarDecl(decl) => {
                        if let Some(ty) = &decl.ty {
                            validate_type(ty, decl.span, semantic_info)?;
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
            if foreach.value_binding.writable {
                let iterable = dictionary_foreach_projection(&foreach.iterable)
                    .map_or(&foreach.iterable, |(dictionary, _)| dictionary);
                let supported = semantic_info
                    .expression_type(iterable.span())
                    .is_some_and(|ty| {
                        matches!(
                            ty,
                            ResolvedType::SortedDictionary(_, _) | ResolvedType::Deque(_)
                        ) || core_collection::uses_receiver(ty)
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
            if let Some(first_binding) = &foreach.first_binding {
                if let Some(ty) = &first_binding.ty {
                    validate_type(ty, first_binding.span, semantic_info)?;
                }
            }
            if let Some(ty) = &foreach.value_binding.ty {
                validate_type(ty, foreach.value_binding.span, semantic_info)?;
            }
            validate_block(&foreach.body, semantic_info)
        }
        Stmt::Increment(increment) => Err(unsupported_increment(increment)),
        Stmt::Expr { expr, .. } => validate_expr(expr, semantic_info),
        Stmt::Throw(statement) => validate_expr(&statement.expr, semantic_info),
        Stmt::Try(statement) => {
            validate_block(&statement.body, semantic_info)?;
            for catch in &statement.catches {
                validate_block(&catch.body, semantic_info)?;
            }
            if let Some(finally) = &statement.finally {
                validate_block(&finally.body, semantic_info)?;
            }
            Ok(())
        }
    }
}

fn validate_if(if_stmt: &IfStmt, semantic_info: &PhpValidation<'_>) -> Result<(), BackendError> {
    if let Some(given) = &if_stmt.given {
        validate_given(given, semantic_info)?;
    }
    validate_expr(&if_stmt.condition, semantic_info)?;
    validate_block(&if_stmt.then_block, semantic_info)?;
    if let Some(else_branch) = &if_stmt.else_branch {
        match else_branch {
            ElseBranch::If(else_if) => validate_if(else_if, semantic_info)?,
            ElseBranch::Block(block) => validate_block(block, semantic_info)?,
        }
    }
    if let Some(finally) = &if_stmt.finally {
        validate_block(&finally.block, semantic_info)?;
    }
    Ok(())
}

fn validate_given(
    given: &GivenPrelude,
    semantic_info: &PhpValidation<'_>,
) -> Result<(), BackendError> {
    for statement in &given.block.statements {
        validate_statement(statement, semantic_info)?;
    }
    Ok(())
}

fn validate_assignment(
    assignment: &Assignment,
    semantic_info: &PhpValidation<'_>,
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

fn validate_expr(expr: &Expr, semantic_info: &PhpValidation<'_>) -> Result<(), BackendError> {
    match expr {
        Expr::Assertion(assertion) => {
            for operand in [
                assertion.actual.as_deref(),
                assertion.expected.as_deref(),
                assertion.user_message.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                validate_expr(operand, semantic_info)?;
            }
            Ok(())
        }
        Expr::Closure(closure) => {
            for parameter in &closure.parameters {
                validate_type(&parameter.ty, parameter.span, semantic_info)?;
            }
            if let Some(return_type) = &closure.return_type {
                validate_type(return_type, closure.span, semantic_info)?;
            }
            match &closure.body {
                ClosureBody::Expression(body) => validate_expr(body, semantic_info),
                ClosureBody::Block(body) => validate_block(body, semantic_info),
            }
        }
        Expr::CallableCall(call) => {
            validate_expr(&call.callee, semantic_info)?;
            validate_arguments(&call.args, semantic_info)
        }
        Expr::ListAlgorithmCall(call) => {
            validate_expr(&call.receiver, semantic_info)?;
            validate_arguments(&call.arguments, semantic_info)
        }
        Expr::Variable { .. }
        | Expr::This { .. }
        | Expr::Identifier { .. }
        | Expr::String { .. }
        | Expr::Float { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. } => Ok(()),
        Expr::Int { value, span } => {
            if parse_decimal_magnitude(value).is_some_and(|value| value > i64::MAX as u128)
                && !is_uint64(semantic_info.expression_type(*span))
            {
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
        Expr::ArrayRepeat { value, count, .. } => {
            validate_expr(value, semantic_info)?;
            validate_expr(count, semantic_info)
        }
        Expr::Index {
            collection,
            index,
            span,
        } => {
            validate_expr(collection, semantic_info)?;
            validate_expr(index, semantic_info)?;
            if matches!(
                semantic_info.expression_type(collection.span()),
                Some(ResolvedType::Bytes | ResolvedType::Set(_))
            ) {
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
            let receiver_type = semantic_info.expression_type(object.span());
            if receiver_type.is_some_and(is_stage23_runtime_type)
                && !receiver_type.is_some_and(core_collection::uses_receiver)
                && !receiver_type.is_some_and(|ty| php_sequence_property(ty, property))
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
            let receiver_type = semantic_info.expression_type(object.span());
            if receiver_type.is_some_and(is_stage23_runtime_type)
                && !receiver_type.is_some_and(core_collection::uses_receiver)
                && !matches!(receiver_type, Some(ResolvedType::List(_)) if method == "add")
                && !matches!(receiver_type, Some(ResolvedType::Dictionary(_, _)) if method == "get")
                && !php_collection_search(receiver_type, method)
                && !matches!(semantic_info.plan.target(*span, semantic_info.substitutions),
                    Some(crate::semantics::CallableTarget::ConstrainedMethod { requirement, .. })
                    if crate::compiler_known_contracts::IterationOperation::from_requirement(requirement) == Some(crate::compiler_known_contracts::IterationOperation::Acquire))
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
            if (*shared
                || crate::types::SharedHandleKind::from_source_name(&class_type.name).is_some())
                && !semantic_info
                    .shared_construction_types
                    .get(span)
                    .or_else(|| semantic_info.expression_type(*span))
                    .is_some_and(shared::supported_payload)
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
            ..
        } => {
            if class_name == "String" {
                validate_arguments(args, semantic_info)?;
                return Err(unsupported_string_runtime_shape(
                    *span,
                    format!("Unicode String operation `String::{method}`"),
                ));
            }
            if (class_name == "Bytes" && method == "fromArray" && args.len() == 1)
                || (class_name == "Set" && method == "from" && args.len() == 1)
            {
                return validate_arguments(args, semantic_info);
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
                validate_type(ty, *span, semantic_info)
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
                .get(span)
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
        Expr::When(when) => {
            if let Some(given) = &when.given {
                validate_given(given, semantic_info)?;
            }
            for branch in &when.branches {
                if let Some(condition) = &branch.condition {
                    validate_expr(condition, semantic_info)?;
                }
                validate_block(&branch.block, semantic_info)?;
            }
            if let Some(finally) = &when.finally {
                validate_block(&finally.block, semantic_info)?;
            }
            Ok(())
        }
    }
}

fn validate_display_expr(
    expr: &Expr,
    semantic_info: &PhpValidation<'_>,
) -> Result<(), BackendError> {
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
    semantic_info: &PhpValidation<'_>,
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
    semantic_info: &PhpValidation<'_>,
) -> Result<(), BackendError> {
    for argument in arguments {
        validate_expr(&argument.value, semantic_info)?;
    }
    Ok(())
}

fn emit_arguments_for_call(arguments: &[Argument], span: Span, scopes: &PhpNameScopes) -> String {
    emit_call_argument_values(arguments, span, scopes).join(", ")
}

fn emit_call_argument_values(
    arguments: &[Argument],
    span: Span,
    scopes: &PhpNameScopes,
) -> Vec<String> {
    let plan = scopes.closure_plan.callable_at(span);
    arguments
        .iter()
        .enumerate()
        .map(|(written_index, argument)| {
            let parameter = plan.and_then(|plan| {
                argument
                    .name
                    .as_ref()
                    .and_then(|name| {
                        plan.parameters
                            .iter()
                            .find(|parameter| parameter.name == name.text)
                    })
                    .or_else(|| plan.parameters.get(written_index))
            });
            let value = if parameter.is_some_and(|parameter| parameter.cell) {
                assignment_target_cell(&argument.value, scopes).unwrap_or_else(|| {
                    format!(
                        "new __DoriaCell({})",
                        if parameter.is_some_and(|parameter| parameter.take) {
                            emit_owned_expr(&argument.value, scopes)
                        } else {
                            emit_expr(&argument.value, scopes)
                        }
                    )
                })
            } else if parameter.is_some_and(|parameter| parameter.take) {
                emit_owned_expr(&argument.value, scopes)
            } else {
                emit_expr(&argument.value, scopes)
            };
            match &argument.name {
                Some(name) => format!("{}: {value}", name.text),
                None => value,
            }
        })
        .collect()
}

// PHP property defaults accept only constant expressions. Doria instance
// initializers are executable and run before the constructor body, so anything
// outside that subset is emitted as the first statement in the constructor.
fn requires_php_runtime_property_initializer(
    expr: &Expr,
    semantic_info: &SemanticInfo,
) -> Option<(Span, &'static str)> {
    if is_payload_enum_expression(expr, semantic_info) {
        return None;
    }
    match expr {
        Expr::Assertion(_) => Some((expr.span(), "test assertion execution")),
        Expr::Closure(_) | Expr::CallableCall(_) => Some((expr.span(), "closure execution")),
        Expr::ListAlgorithmCall(call) => Some((call.span, "List algorithm execution")),
        Expr::StaticMember {
            class_name,
            member,
            span,
            ..
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
                .and_then(|key| requires_php_runtime_property_initializer(key, semantic_info))
                .or_else(|| {
                    requires_php_runtime_property_initializer(&element.value, semantic_info)
                })
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
            requires_php_runtime_property_initializer(expr, semantic_info)
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
        Expr::Binary { left, right, .. } => {
            requires_php_runtime_property_initializer(left, semantic_info)
                .or_else(|| requires_php_runtime_property_initializer(right, semantic_info))
        }
        Expr::Range { span, .. } => {
            Some((*span, "range expressions in instance property initializers"))
        }
        Expr::Match { span, .. } => {
            Some((*span, "match expressions in instance property initializers"))
        }
        Expr::When(when) => Some((
            when.span,
            "when expressions in instance property initializers",
        )),
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

fn php_sequence_property(ty: &ResolvedType, property: &str) -> bool {
    match ty {
        ResolvedType::TypedArray(_) => property == "length",
        ResolvedType::List(_) => matches!(property, "count" | "isEmpty"),
        ResolvedType::Nullable(inner) => php_sequence_property(inner, property),
        _ => false,
    }
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

#[derive(Debug, Clone)]
struct PhpScopeSymbols {
    interface_names: HashSet<String>,
    static_properties: HashSet<(String, String)>,
    payload_unit_cases: HashSet<(String, String)>,
    payload_top_constants: HashSet<String>,
    payload_class_constants: HashSet<(String, String)>,
    payload_enum_expressions: HashSet<Span>,
    classes_with_php_constructors: HashSet<String>,
    classes_with_php_destructors: HashSet<String>,
    payload_enums_with_php_destructors: HashSet<String>,
}

#[derive(Debug, Clone)]
struct PhpNameScopes {
    scopes: Vec<HashMap<String, String>>,
    mixed_bindings: Vec<HashSet<String>>,
    used_php_names: HashSet<String>,
    next_mangled_id: usize,
    symbols: PhpScopeSymbols,
    payload_case_tags: HashMap<crate::enums::EnumCaseId, u32>,
    matches: HashMap<Span, MatchSemanticInfo>,
    whens: HashMap<Span, WhenSemanticInfo>,
    given_preludes: HashMap<Span, GivenSemanticInfo>,
    expression_types: HashMap<Span, ResolvedType>,
    interface_conversion_types: HashMap<Span, ResolvedType>,
    type_test_types: HashMap<Span, ResolvedType>,
    mixed_box_plans: HashMap<Span, crate::semantics::MixedBoxPlan>,
    throw_error_types: HashMap<Span, ResolvedType>,
    catch_error_types: HashMap<Span, ResolvedType>,
    direct_parent_calls: HashSet<Span>,
    const_evaluation: Evaluation,
    closure_plan: Rc<PhpClosurePlan>,
    binding_places: Vec<HashMap<BindingId, PhpBindingPlace>>,
    owned_cells: Vec<Vec<String>>,
    current_callable: Option<String>,
    current_class: Option<String>,
    closure_owner: Option<mir::ClosureOwner>,
    current_class_id: Option<crate::class_layout::ClassId>,
    specialization: Rc<specialization::Plan>,
    substitutions: HashMap<String, ResolvedType>,
    returns_borrow: bool,
    expression_temporaries: Option<Rc<RefCell<Vec<String>>>>,
    yield_temporaries: Option<Rc<RefCell<Vec<String>>>>,
    pending_returns: Vec<Rc<RefCell<Vec<String>>>>,
}

#[derive(Debug, Clone)]
enum PhpBindingPlace {
    Direct(String),
    Cell(String),
}

impl PhpBindingPlace {
    fn read(&self) -> String {
        match self {
            Self::Direct(name) => format!("${name}"),
            Self::Cell(name) => format!("${name}->value"),
        }
    }

    fn cell(&self) -> Option<String> {
        match self {
            Self::Cell(name) => Some(format!("${name}")),
            Self::Direct(_) => None,
        }
    }
}

impl PhpNameScopes {
    fn new(symbols: PhpScopeSymbols, closure_plan: Rc<PhpClosurePlan>) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            mixed_bindings: vec![HashSet::new()],
            used_php_names: HashSet::new(),
            next_mangled_id: 0,
            symbols,
            payload_case_tags: HashMap::new(),
            matches: HashMap::new(),
            whens: HashMap::new(),
            given_preludes: HashMap::new(),
            expression_types: HashMap::new(),
            interface_conversion_types: HashMap::new(),
            type_test_types: HashMap::new(),
            mixed_box_plans: HashMap::new(),
            throw_error_types: HashMap::new(),
            catch_error_types: HashMap::new(),
            direct_parent_calls: HashSet::new(),
            const_evaluation: Evaluation::default(),
            closure_plan,
            binding_places: vec![HashMap::new()],
            owned_cells: vec![Vec::new()],
            current_callable: None,
            current_class: None,
            closure_owner: None,
            current_class_id: None,
            specialization: Rc::default(),
            substitutions: HashMap::new(),
            returns_borrow: false,
            expression_temporaries: None,
            yield_temporaries: None,
            pending_returns: Vec::new(),
        }
    }

    fn expression_scope(&self) -> Self {
        let mut scopes = Self::new(self.symbols.clone(), Rc::clone(&self.closure_plan));
        scopes.payload_case_tags = self.payload_case_tags.clone();
        scopes.matches = self.matches.clone();
        scopes.whens = self.whens.clone();
        scopes.given_preludes = self.given_preludes.clone();
        scopes.expression_types = self.expression_types.clone();
        scopes.interface_conversion_types = self.interface_conversion_types.clone();
        scopes.type_test_types = self.type_test_types.clone();
        scopes.mixed_box_plans = self.mixed_box_plans.clone();
        scopes.throw_error_types = self.throw_error_types.clone();
        scopes.catch_error_types = self.catch_error_types.clone();
        scopes.direct_parent_calls = self.direct_parent_calls.clone();
        scopes.const_evaluation = self.const_evaluation.clone();
        scopes.current_callable = self.current_callable.clone();
        scopes.current_class = self.current_class.clone();
        scopes.closure_owner = self.closure_owner;
        scopes.current_class_id = self.current_class_id;
        scopes.specialization = Rc::clone(&self.specialization);
        scopes.substitutions = self.substitutions.clone();
        scopes.returns_borrow = self.returns_borrow;
        scopes
    }

    fn is_static_property(&self, class_name: &str, member: &str) -> bool {
        self.symbols
            .static_properties
            .contains(&(class_name.to_string(), member.to_string()))
    }

    fn is_payload_unit_case(&self, enum_name: &str, case_name: &str) -> bool {
        self.symbols
            .payload_unit_cases
            .contains(&(enum_name.to_string(), case_name.to_string()))
    }

    fn is_payload_top_constant(&self, name: &str) -> bool {
        self.symbols.payload_top_constants.contains(name)
    }

    fn is_payload_class_constant(&self, class_name: &str, name: &str) -> bool {
        self.symbols
            .payload_class_constants
            .contains(&(class_name.to_string(), name.to_string()))
    }

    fn is_payload_enum_expression(&self, expr: &Expr) -> bool {
        self.symbols.payload_enum_expressions.contains(&expr.span())
    }

    fn push(&mut self) {
        self.scopes.push(HashMap::new());
        self.mixed_bindings.push(HashSet::new());
        self.binding_places.push(HashMap::new());
        self.owned_cells.push(Vec::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
        self.mixed_bindings.pop();
        self.binding_places.pop();
        self.owned_cells.pop();
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

    fn bind_place(&mut self, binding: BindingId, place: PhpBindingPlace) {
        self.binding_places
            .last_mut()
            .expect("PHP emitter always has a binding scope")
            .insert(binding, place);
    }

    fn binding_for_declaration(&self, name: &str, span: Span) -> Option<BindingId> {
        self.closure_plan
            .binding_resolution
            .declarations_by_id
            .values()
            .find(|declaration| {
                declaration.name == name
                    && declaration.span.is_some_and(|declared| {
                        declared.start >= span.start && declared.end <= span.end
                    })
            })
            .map(|declaration| declaration.id)
    }

    fn binding_for_use(&self, span: Span) -> Option<BindingId> {
        self.closure_plan
            .binding_resolution
            .uses_by_span
            .get(&span)
            .copied()
    }

    fn place(&self, binding: BindingId) -> Option<&PhpBindingPlace> {
        self.binding_places
            .iter()
            .rev()
            .find_map(|scope| scope.get(&binding))
    }

    fn place_for_use(&self, span: Span) -> Option<&PhpBindingPlace> {
        self.binding_for_use(span)
            .and_then(|binding| self.place(binding))
    }

    fn source_type(&self, binding: BindingId) -> Option<&ResolvedType> {
        self.closure_plan
            .binding_resolution
            .declarations_by_id
            .get(&binding)
            .and_then(|declaration| declaration.source_type.as_ref())
    }

    fn needs_cell(&self, binding: BindingId) -> bool {
        self.closure_plan.cell_bindings.contains(&binding)
    }

    fn owns_binding(&self, binding: BindingId) -> bool {
        self.closure_plan
            .binding_resolution
            .declarations_by_id
            .get(&binding)
            .is_some_and(|declaration| {
                declaration.ownership == crate::symbols::BindingOwnership::Owned
            })
    }

    fn own_cell(&mut self, php_name: String) {
        self.owned_cells
            .last_mut()
            .expect("PHP emitter always has an ownership scope")
            .push(php_name);
    }

    fn current_owned_cells(&self) -> &[String] {
        self.owned_cells
            .last()
            .expect("PHP emitter always has an ownership scope")
    }

    fn has_owned_cells(&self) -> bool {
        self.owned_cells.iter().any(|scope| !scope.is_empty())
    }

    fn callable_identity(&self) -> String {
        self.current_callable
            .as_ref()
            .map(|identity| emit_php_string_literal(identity))
            .unwrap_or_else(|| "__METHOD__".to_string())
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
        Item::Function(function) => emit_function(
            function,
            semantic_info,
            output,
            indent,
            scopes,
            PhpFunctionEmission::default(),
        ),
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
        emit_payload_enum(enum_decl, output, indent, scopes);
        return;
    }
    write_indent(output, indent);
    output.push_str("enum ");
    output.push_str(&php_symbol_name(&enum_decl.name));
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

fn emit_payload_enum(
    enum_decl: &EnumDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    let owned_marker = if scopes
        .symbols
        .payload_enums_with_php_destructors
        .contains(&enum_decl.name)
    {
        ", __DoriaOwnedObject"
    } else {
        ""
    };
    writeln(
        output,
        indent,
        &format!(
            "final class {} implements __DoriaValueEquatable{owned_marker}",
            php_symbol_name(&enum_decl.name)
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
    writeln(
        output,
        indent + 1,
        "public function __doriaTakePayload(int $index): mixed",
    );
    writeln(output, indent + 1, "{");
    writeln(
        output,
        indent + 2,
        "$value = $this->__doriaPayload[$index];",
    );
    writeln(output, indent + 2, "$this->__doriaPayload[$index] = null;");
    writeln(output, indent + 2, "return $value;");
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
                .map(|field| format!("{} ${}", php_type(&field.ty, scopes), field.name))
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
        "if (!__doria_begin_destroy($this, self::class)) { return; }",
    );
    writeln(
        output,
        indent + 2,
        "for ($index = count($this->__doriaPayload) - 1; $index >= 0; --$index) { if (function_exists('__doria_drop_value')) { __doria_drop_value($this->__doriaPayload[$index]); } unset($this->__doriaPayload[$index]); }",
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
    for class in semantic_info
        .classes
        .iter()
        .filter(|class| class.declaration_name == class_decl.name)
    {
        let constructor = scopes
            .specialization
            .callables
            .iter()
            .find(|callable| callable.class == Some(class.id) && callable.symbol == "__construct")
            .expect("each concrete class has a checked constructor instance");
        let mut scopes = scopes.specialization.scope(scopes, constructor);
        scopes.current_class_id = Some(class.id);
        emit_class_instance(class_decl, semantic_info, output, indent, &scopes);
    }
}

fn emit_class_instance(
    class_decl: &ClassDecl,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    let mut class_scopes = scopes.clone();
    class_scopes.current_class = Some(class_decl.name.clone());
    let scopes = &class_scopes;
    let type_parameters = class_decl
        .type_params
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<HashSet<_>>();
    let mut owned_properties = class_decl
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Property(property)
                if !property.is_static
                    && php_type_ref_needs_explicit_drop(
                        &property.ty,
                        &scopes.symbols.classes_with_php_destructors,
                        &scopes.symbols.payload_enums_with_php_destructors,
                        &type_parameters,
                        &scopes.symbols.interface_names,
                    ) =>
            {
                Some(property.name.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    owned_properties.extend(class_decl.members.iter().flat_map(|member| {
        match member {
            ClassMember::Method(method) if method.name == "__construct" => method
                .params
                .iter()
                .filter(|parameter| {
                    parameter.constructor_role.is_promoted()
                        && !parameter.borrow
                        && php_type_ref_needs_explicit_drop(
                            &parameter.ty,
                            &scopes.symbols.classes_with_php_destructors,
                            &scopes.symbols.payload_enums_with_php_destructors,
                            &type_parameters,
                            &scopes.symbols.interface_names,
                        )
                })
                .map(|parameter| parameter.name.clone())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        }
    }));
    let instance_initializers = class_decl
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Property(property)
                if !property.is_static
                    && property.initializer.as_ref().is_some_and(|value| {
                        is_payload_enum_expression(value, semantic_info)
                            || requires_php_runtime_property_initializer(value, semantic_info)
                                .is_some()
                    }) =>
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
    let mut has_destructor = false;
    let parent_has_constructor = class_decl.parent.as_ref().is_some_and(|parent| {
        scopes
            .symbols
            .classes_with_php_constructors
            .contains(&parent.name)
    });
    let parent_has_destructor = class_decl.parent.as_ref().is_some_and(|parent| {
        scopes
            .symbols
            .classes_with_php_destructors
            .contains(&parent.name)
    });
    let checked_class = semantic_info
        .classes
        .iter()
        .find(|class| Some(class.id) == scopes.current_class_id);
    let is_error = checked_class.is_some_and(|class| class.implements(BuiltinInterface::Error));
    let class_type = crate::types::ClassType::new(
        class_decl.name.clone(),
        checked_class.unwrap().arguments.clone(),
    );
    let mut interfaces = interface::class_views(semantic_info, &class_type);
    if scopes
        .symbols
        .classes_with_php_destructors
        .contains(&class_decl.name)
    {
        interfaces.push("__DoriaOwnedObject".to_string());
    }
    if checked_class.is_some_and(|class| class.implements(BuiltinInterface::Displayable)) {
        interfaces.push("__DoriaDisplayable".to_string());
    }
    if is_error {
        interfaces.push("__DoriaErrorValue".to_string());
    }
    interfaces.sort();
    interfaces.dedup();
    let implements = if interfaces.is_empty() {
        String::new()
    } else {
        format!(" implements {}", interfaces.join(", "))
    };
    let extends = checked_class
        .unwrap()
        .parent
        .as_ref()
        .map_or_else(String::new, |parent| {
            format!(
                " extends {}",
                scopes
                    .specialization
                    .class_symbols
                    .get(parent)
                    .expect("checked parent instance")
            )
        });
    writeln(
        output,
        indent,
        &format!(
            "class {}{extends}{implements}",
            scopes
                .specialization
                .class_symbols
                .get(&class_type)
                .expect("checked class instance")
        ),
    );
    writeln(output, indent, "{");
    if is_error {
        writeln(output, indent + 1, "private int $__doriaErrorOrigin = 0;");
        writeln(
            output,
            indent + 1,
            "private string $__doriaErrorCallable = \"main\";",
        );
        writeln(
            output,
            indent + 1,
            "public static function __doriaErrorType(): __DoriaErrorDescriptor",
        );
        writeln(output, indent + 1, "{");
        writeln(
            output,
            indent + 2,
            &format!(
                "return __doria_error_descriptor({});",
                emit_php_string_literal(&class_decl.name)
            ),
        );
        writeln(output, indent + 1, "}");
        writeln(
            output,
            indent + 1,
            "public function __doriaErrorDescriptor(): __DoriaErrorDescriptor",
        );
        writeln(output, indent + 1, "{");
        writeln(output, indent + 2, "return self::__doriaErrorType();");
        writeln(output, indent + 1, "}");
        writeln(
            output,
            indent + 1,
            "public function __doriaEnsureErrorOrigin(int $origin, string $callable): void",
        );
        writeln(output, indent + 1, "{");
        writeln(
            output,
            indent + 2,
            "if ($this->__doriaErrorOrigin === 0) { $this->__doriaErrorOrigin = $origin; $this->__doriaErrorCallable = $callable; }",
        );
        writeln(output, indent + 1, "}");
        writeln(
            output,
            indent + 1,
            "public function __doriaErrorOrigin(): int",
        );
        writeln(output, indent + 1, "{");
        writeln(output, indent + 2, "return $this->__doriaErrorOrigin;");
        writeln(output, indent + 1, "}");
        writeln(
            output,
            indent + 1,
            "public function __doriaErrorCallable(): string",
        );
        writeln(output, indent + 1, "{");
        writeln(output, indent + 2, "return $this->__doriaErrorCallable;");
        writeln(output, indent + 1, "}");
        output.push('\n');
        if class_decl.name == crate::compiler_known_test::ASSERTION_ERROR {
            writeln(
                output,
                indent + 1,
                "public function __doriaAssertionFacts(): array",
            );
            writeln(output, indent + 1, "{");
            writeln(
                output,
                indent + 2,
                "return [$this->__assertionMatcher, $this->__assertionNegated, $this->__assertionActualPresent, $this->__assertionActualType, $this->__assertionActualPresentation, $this->__assertionExpectedPresent, $this->__assertionExpectedType, $this->__assertionExpectedPresentation, $this->__assertionDifferencePresent, $this->__assertionDifference, $this->__assertionUserMessagePresent, $this->__assertionUserMessage];",
            );
            writeln(output, indent + 1, "}");
            output.push('\n');
        }
    }
    for member in &class_decl.members {
        let ClassMember::Method(method) = member else {
            continue;
        };
        let callable_plan = scopes.closure_plan.callable_definition(method.span);
        let manual_promotions = class_decl.parent.is_some() && method.name == "__construct";
        for (index, parameter) in method.params.iter().enumerate() {
            let Some(access) = parameter.constructor_role.promoted_access() else {
                continue;
            };
            if !manual_promotions && !callable_parameter_uses_cell(callable_plan, index) {
                continue;
            }
            writeln(
                output,
                indent + 1,
                &format!(
                    "{} {} ${};",
                    emit_member_access(&access),
                    php_type(&parameter.ty, scopes),
                    parameter.name
                ),
            );
        }
    }
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
                has_destructor |= method.name == "__destruct";
                let initializers = if method.name == "__construct" {
                    instance_initializers.as_slice()
                } else {
                    &[]
                };
                let mut emitted_method = method.clone();
                if class_decl.name == crate::compiler_known_test::ASSERTION_ERROR
                    && emitted_method.name == "__construct"
                {
                    // The generated assertion helper constructs this compiler-owned
                    // class outside its PHP class body. Doria source still cannot
                    // invoke the internal constructor.
                    emitted_method.access = MemberAccess::External;
                }
                emit_function(
                    &emitted_method,
                    semantic_info,
                    output,
                    indent + 1,
                    scopes,
                    PhpFunctionEmission {
                        is_method: true,
                        property_initializers: initializers,
                        invoke_parent_constructor: parent_has_constructor
                            && emitted_method.name == "__construct",
                        invoke_parent_destructor: parent_has_destructor
                            && emitted_method.name == "__destruct",
                        manual_promotions: class_decl.parent.is_some()
                            && emitted_method.name == "__construct",
                        drop_properties: if emitted_method.name == "__destruct" {
                            owned_properties.as_slice()
                        } else {
                            &[]
                        },
                        construction_properties: if emitted_method.name == "__construct" {
                            owned_properties.as_slice()
                        } else {
                            &[]
                        },
                        parent_drop_on_construction_failure: parent_has_destructor
                            && emitted_method.name == "__construct",
                    },
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
    if !has_constructor && (!instance_initializers.is_empty() || parent_has_constructor) {
        let mut constructor_scopes = scopes.expression_scope();
        constructor_scopes.current_callable = Some(format!("{}::__construct", class_decl.name));
        let parent_completed = parent_has_destructor
            .then(|| constructor_scopes.fresh_temp("__doria_parent_completed"));
        let cleanup = !owned_properties.is_empty() || parent_has_destructor;
        writeln(output, indent + 1, "public function __construct()");
        writeln(output, indent + 1, "{");
        if let Some(parent_completed) = &parent_completed {
            writeln(output, indent + 2, &format!("${parent_completed} = false;"));
        }
        if cleanup {
            writeln(output, indent + 2, "try {");
        }
        let body_indent = indent + 2 + usize::from(cleanup);
        if parent_has_constructor {
            writeln(output, body_indent, "parent::__construct();");
            if let Some(parent_completed) = &parent_completed {
                writeln(output, body_indent, &format!("${parent_completed} = true;"));
            }
        }
        for (name, initializer) in &instance_initializers {
            writeln(
                output,
                body_indent,
                &format!(
                    "$this->{name} = {};",
                    emit_owned_expr(
                        initializer,
                        &scopes
                            .specialization
                            .property_scope(&constructor_scopes, name)
                    )
                ),
            );
        }
        if cleanup {
            emit_constructor_catch(
                &owned_properties,
                parent_completed.as_deref(),
                output,
                indent + 2,
                &mut constructor_scopes,
            );
        }
        writeln(output, indent + 1, "}");
        output.push('\n');
    }
    if !has_destructor
        && scopes
            .symbols
            .classes_with_php_destructors
            .contains(&class_decl.name)
    {
        writeln(output, indent + 1, "public function __destruct()");
        writeln(output, indent + 1, "{");
        writeln(
            output,
            indent + 2,
            "if (!__doria_begin_destroy($this, __CLASS__)) { return; }",
        );
        for property in owned_properties.iter().rev() {
            writeln(
                output,
                indent + 2,
                &format!(
                    "if (isset($this->{property})) {{ $__doriaPropertyValue = $this->{property}; unset($this->{property}); __doria_drop_value($__doriaPropertyValue); }}"
                ),
            );
        }
        if parent_has_destructor {
            writeln(output, indent + 2, "parent::__destruct();");
        }
        writeln(output, indent + 1, "}");
        output.push('\n');
    }
    emit_closure_entries(
        Some(
            scopes
                .specialization
                .class_symbol(checked_class.unwrap().id),
        ),
        output,
        indent + 1,
        scopes,
    );
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
            &format!(
                "}}, null, {}::class))();",
                php_symbol_name(&class_decl.name)
            ),
        );
    }
}

fn constructor_starts_with_parent_call(
    function: &FunctionDecl,
    semantic_info: &SemanticInfo,
) -> bool {
    matches!(
        function.body.statements.first(),
        Some(Stmt::Expr {
            expr:
                Expr::StaticCall {
                    method,
                    span,
                    ..
                },
            ..
        }) if method == "__construct"
            && matches!(
                semantic_info.call_target(*span),
                Some(crate::semantics::CallableTarget::Method {
                    direct_parent: true,
                    ..
                })
            )
    )
}

fn emit_closure_entries(
    owner_class: Option<&str>,
    output: &mut String,
    indent: usize,
    shared_scopes: &PhpNameScopes,
) {
    let mut descriptors = shared_scopes
        .closure_plan
        .descriptors
        .values()
        .filter(|descriptor| descriptor.owner_class.as_deref() == owner_class)
        .cloned()
        .collect::<Vec<_>>();
    descriptors.sort_by_key(|descriptor| descriptor.descriptor.0);
    for descriptor in descriptors {
        let closure = shared_scopes
            .closure_plan
            .closures
            .get(&descriptor.closure_id);
        let Some(closure) = closure.cloned() else {
            continue;
        };
        emit_closure_entry(&descriptor, &closure, output, indent, shared_scopes);
        output.push('\n');
    }
}

fn emit_closure_entry(
    descriptor: &PhpClosureDescriptor,
    closure: &ClosureExpression,
    output: &mut String,
    indent: usize,
    shared_scopes: &PhpNameScopes,
) {
    let specialized_scopes;
    let shared_scopes = if let mir::ClosureOwner::Callable(instance) = descriptor.source_instance {
        specialized_scopes = shared_scopes.specialization.scope(
            shared_scopes,
            shared_scopes.specialization.callable(instance),
        );
        &specialized_scopes
    } else {
        shared_scopes
    };
    let semantic = shared_scopes
        .closure_plan
        .semantic_closures
        .get(&closure.closure_id)
        .expect("checked closure must have semantic facts");
    let mut scopes = shared_scopes.expression_scope();
    scopes.current_callable = Some(descriptor.debug_identity.clone());
    scopes.closure_owner = Some(descriptor.source_instance);
    scopes.returns_borrow = matches!(&semantic.function_type, ResolvedType::Function(function) if function.return_borrow.is_some());
    scopes.push();

    write_indent(output, indent);
    if descriptor.owner_class.is_some() {
        output.push_str("public static ");
    }
    output.push_str("function ");
    output.push_str(&descriptor.helper_name);
    output.push('(');
    let mut parameters = Vec::new();
    if let Some(environment_name) = &descriptor.environment_name {
        parameters.push(format!("{environment_name} $__doriaEnvironment"));
    }
    for parameter in &closure.parameters {
        let binding = scopes.binding_for_declaration(&parameter.name, parameter.span);
        let parameter_type = if parameter.writable {
            "__DoriaCell".to_string()
        } else {
            php_type(&parameter.ty, &scopes)
        };
        parameters.push(format!("{parameter_type} ${}", parameter.name));
        scopes.declare_unmangled(&parameter.name);
        if let Some(binding) = binding {
            let place = if parameter.writable || scopes.needs_cell(binding) {
                PhpBindingPlace::Cell(parameter.name.clone())
            } else {
                PhpBindingPlace::Direct(parameter.name.clone())
            };
            scopes.bind_place(binding, place);
        }
    }
    output.push_str(&parameters.join(", "));
    output.push_str("): ");
    output.push_str(&php_resolved_type(&semantic.inferred_return_type, &scopes));
    output.push('\n');
    writeln(output, indent, "{");
    if let Some(layout_id) = descriptor.environment_layout {
        let fields = scopes.closure_plan.layout(layout_id).fields.clone();
        for field in fields {
            scopes.bind_place(
                field.environment_binding,
                PhpBindingPlace::Cell(format!("__doriaEnvironment->field{}", field.id.0)),
            );
        }
    }
    for parameter in &closure.parameters {
        let Some(binding) = scopes.binding_for_declaration(&parameter.name, parameter.span) else {
            continue;
        };
        if scopes.needs_cell(binding) && !parameter.writable {
            writeln(
                output,
                indent + 1,
                &format!("${0} = new __DoriaCell(${0});", parameter.name),
            );
        }
        if parameter.take && scopes.needs_cell(binding) {
            scopes.own_cell(parameter.name.clone());
        }
    }

    emit_owned_scope(
        output,
        indent + 1,
        &mut scopes,
        |output, indent, scopes| match &closure.body {
            ClosureBody::Expression(expr) => {
                let result = scopes.fresh_temp("__doria_closure_result");
                let owns_result = !scopes.returns_borrow
                    && resolved_type_needs_php_drop(&semantic.inferred_return_type, scopes);
                writeln(
                    output,
                    indent,
                    &format!(
                        "${result} = {};",
                        if owns_result {
                            emit_owned_expr(expr, scopes)
                        } else {
                            emit_expr(expr, scopes)
                        }
                    ),
                );
                writeln(output, indent, &format!("return ${result};"));
            }
            ClosureBody::Block(block) => {
                for statement in &block.statements {
                    emit_statement(statement, output, indent, scopes);
                }
            }
        },
    );
    scopes.pop();
    writeln(output, indent, "}");
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
    let ty = php_type(&property.ty, shared_scopes);
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
        let runtime_initializer = if property.is_static {
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
                || requires_php_runtime_property_initializer(initializer, semantic_info).is_some()
        };
        if !runtime_initializer {
            output.push_str(" = ");
        }
        if property.is_static && !runtime_initializer {
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
        } else if !runtime_initializer {
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
            || format!("__doria_const_{}", php_symbol_name(&constant.name)),
            |_| format!("__doriaConst{}", constant.name),
        ));
        output.push_str("(): ");
        output.push_str(&php_symbol_name(enum_name));
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
        ConstValue::Integer(value) if value.ty == IntegerType::UInt64 => {
            emit_integer_bits(value.mathematical_value() as u64 as i64)
        }
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
                (*candidate == *value)
                    .then(|| format!("{}::{case_name}", php_symbol_name(enum_name)))
            })
            .expect("checked enum constant must name a declared case"),
        ConstValue::PayloadEnum(value) => {
            let (enum_name, case_name) = evaluation
                .payload_case_name(value.enum_id, value.case_id)
                .expect("checked payload enum constant must name a declared case");
            let method = php_payload_case_method(case_name, !value.fields.is_empty());
            format!(
                "{}::{method}({})",
                php_symbol_name(enum_name),
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

#[derive(Debug, Clone, Copy, Default)]
struct PhpFunctionEmission<'a> {
    is_method: bool,
    property_initializers: &'a [(&'a str, &'a Expr)],
    invoke_parent_constructor: bool,
    invoke_parent_destructor: bool,
    manual_promotions: bool,
    drop_properties: &'a [String],
    construction_properties: &'a [String],
    parent_drop_on_construction_failure: bool,
}

fn emit_function(
    function: &FunctionDecl,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    shared_scopes: &PhpNameScopes,
    emission: PhpFunctionEmission<'_>,
) {
    for callable in shared_scopes
        .specialization
        .callables
        .iter()
        .filter(|callable| {
            callable.declaration == function.span
                && callable.class == shared_scopes.current_class_id
        })
    {
        let scopes = shared_scopes.specialization.scope(shared_scopes, callable);
        emit_function_instance(function, semantic_info, output, indent, &scopes, emission);
    }
}

fn emit_function_instance(
    function: &FunctionDecl,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    shared_scopes: &PhpNameScopes,
    emission: PhpFunctionEmission<'_>,
) {
    let PhpFunctionEmission {
        is_method,
        property_initializers,
        invoke_parent_constructor,
        invoke_parent_destructor,
        manual_promotions,
        drop_properties,
        construction_properties,
        parent_drop_on_construction_failure,
    } = emission;
    let mut scopes = shared_scopes.expression_scope();
    scopes.returns_borrow = semantic_info.return_borrows.contains_key(&function.span);
    scopes.current_callable = Some(if is_method {
        format!(
            "{}::{}",
            scopes.current_class.as_deref().expect("method has a class"),
            function.name
        )
    } else {
        function.name.clone()
    });
    let callable_plan = scopes
        .closure_plan
        .callable_definition(function.span)
        .cloned();
    for (index, param) in function.params.iter().enumerate() {
        scopes.declare_unmangled(&param.name);
        if param.ty.name == "mixed" {
            scopes.mark_mixed(&param.name);
        }
        if let Some(binding) = scopes.binding_for_declaration(&param.name, param.span) {
            let uses_cell = callable_parameter_uses_cell(callable_plan.as_ref(), index);
            scopes.bind_place(
                binding,
                if uses_cell || scopes.needs_cell(binding) {
                    PhpBindingPlace::Cell(param.name.clone())
                } else {
                    PhpBindingPlace::Direct(param.name.clone())
                },
            );
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
    let Some(mir::ClosureOwner::Callable(instance)) = scopes.closure_owner else {
        unreachable!("authored callable emission has a checked concrete instance");
    };
    output.push_str(&scopes.specialization.callable(instance).symbol);
    output.push('(');
    output.push_str(
        &function
            .params
            .iter()
            .enumerate()
            .map(|(parameter_index, param)| {
                let uses_cell =
                    callable_parameter_uses_cell(callable_plan.as_ref(), parameter_index);
                emit_param(
                    param,
                    semantic_info.parameter_defaults.get(&ParameterDefaultKey {
                        function_start: function.span.start,
                        parameter_index,
                    }),
                    &semantic_info.const_evaluation,
                    &scopes,
                    uses_cell,
                    !manual_promotions,
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
        output.push_str(&php_type(return_type, &scopes));
    }
    output.push('\n');
    writeln(output, indent, "{");
    let is_entry = !is_method && crate::names::source_name_is(&function.name, "main");
    if is_method && function.name == "__destruct" {
        writeln(
            output,
            indent + 1,
            "if (!__doria_begin_destroy($this, __CLASS__)) { return; }",
        );
    }
    let checked_entry = is_entry && !function.checked_effects.is_empty();
    let body_indent = if checked_entry {
        indent + 2
    } else {
        indent + 1
    };
    if checked_entry {
        writeln(output, indent + 1, "try");
        writeln(output, indent + 1, "{");
    }
    scopes.push();
    let parent_completed =
        parent_drop_on_construction_failure.then(|| scopes.fresh_temp("__doria_parent_completed"));
    if let Some(parent_completed) = &parent_completed {
        writeln(
            output,
            body_indent,
            &format!("${parent_completed} = false;"),
        );
    }
    let construction_cleanup =
        !construction_properties.is_empty() || parent_drop_on_construction_failure;
    if construction_cleanup {
        writeln(output, body_indent, "try {");
    }
    emit_owned_scope(
        output,
        body_indent + usize::from(construction_cleanup),
        &mut scopes,
        |output, body_indent, scopes| {
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
                writeln(output, body_indent, &format!("if (${name} === []) {{"));
                writeln(
                    output,
                    body_indent + 1,
                    &format!(
                        "${name} = {};",
                        emit_const_value(default, &semantic_info.const_evaluation)
                    ),
                );
                if param.constructor_role.is_promoted() && !manual_promotions {
                    writeln(
                        output,
                        body_indent + 1,
                        &format!("$this->{name} = ${name};"),
                    );
                }
                writeln(output, body_indent, "}");
            }
            for (index, param) in function.params.iter().enumerate() {
                if scopes
                    .binding_for_declaration(&param.name, param.span)
                    .is_none()
                {
                    continue;
                }
                let uses_cell = callable_parameter_uses_cell(callable_plan.as_ref(), index);
                if uses_cell && param.default.is_some() {
                    writeln(
                        output,
                        body_indent,
                        &format!(
                    "if (!(${0} instanceof __DoriaCell)) {{ ${0} = new __DoriaCell(${0}); }}",
                    param.name
                ),
                    );
                }
                if param.constructor_role.is_promoted() && uses_cell && !manual_promotions {
                    let value = if uses_cell && param.take {
                        format!("__doria_take_cell(${})", param.name)
                    } else if uses_cell {
                        format!("${}->value", param.name)
                    } else {
                        format!("${}", param.name)
                    };
                    writeln(
                        output,
                        body_indent,
                        &format!("$this->{} = {value};", param.name),
                    );
                }
                if uses_cell && param.take {
                    scopes.own_cell(param.name.clone());
                }
            }
            let explicit_parent_constructor = invoke_parent_constructor
                && constructor_starts_with_parent_call(function, semantic_info);
            if invoke_parent_constructor {
                if explicit_parent_constructor {
                    emit_statement(
                        function
                            .body
                            .statements
                            .first()
                            .expect("explicit parent constructor call is present"),
                        output,
                        body_indent,
                        scopes,
                    );
                } else {
                    writeln(output, body_indent, "parent::__construct();");
                }
                if let Some(parent_completed) = &parent_completed {
                    writeln(output, body_indent, &format!("${parent_completed} = true;"));
                }
            }
            if manual_promotions {
                for (index, param) in function.params.iter().enumerate() {
                    if !param.constructor_role.is_promoted() {
                        continue;
                    }
                    let uses_cell = callable_parameter_uses_cell(callable_plan.as_ref(), index);
                    let value = if uses_cell && param.take {
                        format!("__doria_take_cell(${})", param.name)
                    } else if uses_cell {
                        format!("${}->value", param.name)
                    } else {
                        format!("${}", param.name)
                    };
                    writeln(
                        output,
                        body_indent,
                        &format!("$this->{} = {value};", param.name),
                    );
                }
            }
            for (name, initializer) in property_initializers {
                writeln(
                    output,
                    body_indent,
                    &format!(
                        "$this->{name} = {};",
                        emit_owned_expr(
                            initializer,
                            &scopes.specialization.property_scope(scopes, name)
                        )
                    ),
                );
            }
            let routes_destructor_cleanup = invoke_parent_destructor || !drop_properties.is_empty();
            let statement_indent = if routes_destructor_cleanup {
                writeln(output, body_indent, "try");
                writeln(output, body_indent, "{");
                body_indent + 1
            } else {
                body_indent
            };
            let emit_statements =
                |output: &mut String, indent: usize, scopes: &mut PhpNameScopes| {
                    for statement in function
                        .body
                        .statements
                        .iter()
                        .skip(usize::from(explicit_parent_constructor))
                    {
                        emit_statement(statement, output, indent, scopes);
                    }
                };
            if routes_destructor_cleanup {
                scopes.push();
                emit_owned_scope(output, statement_indent, scopes, emit_statements);
                scopes.pop();
            } else {
                emit_statements(output, statement_indent, scopes);
            }
            if routes_destructor_cleanup {
                writeln(output, body_indent, "}");
                writeln(output, body_indent, "finally");
                writeln(output, body_indent, "{");
                let cleanup_indent = if invoke_parent_destructor && !drop_properties.is_empty() {
                    writeln(output, body_indent + 1, "try");
                    writeln(output, body_indent + 1, "{");
                    body_indent + 2
                } else {
                    body_indent + 1
                };
                for property in drop_properties.iter().rev() {
                    let temporary = scopes.fresh_temp("__doria_property_value");
                    writeln(
                output,
                cleanup_indent,
                &format!(
                    "if (isset($this->{property})) {{ ${temporary} = $this->{property}; unset($this->{property}); __doria_drop_value(${temporary}); }}"
                ),
            );
                }
                if invoke_parent_destructor && !drop_properties.is_empty() {
                    writeln(output, body_indent + 1, "}");
                    writeln(output, body_indent + 1, "finally");
                    writeln(output, body_indent + 1, "{");
                    writeln(output, body_indent + 2, "parent::__destruct();");
                    writeln(output, body_indent + 1, "}");
                } else if invoke_parent_destructor {
                    writeln(output, body_indent + 1, "parent::__destruct();");
                }
                writeln(output, body_indent, "}");
            }
        },
    );
    if construction_cleanup {
        emit_constructor_catch(
            construction_properties,
            parent_completed.as_deref(),
            output,
            body_indent,
            &mut scopes,
        );
    }
    scopes.pop();
    if checked_entry {
        writeln(output, indent + 1, "}");
        writeln(output, indent + 1, "catch (__DoriaCheckedError $error)");
        writeln(output, indent + 1, "{");
        writeln(
            output,
            indent + 2,
            "__doria_report_unhandled_error($error);",
        );
        writeln(output, indent + 1, "}");
    }
    writeln(output, indent, "}");
}

fn emit_property_drops(
    properties: &[String],
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    for property in properties.iter().rev() {
        let temporary = scopes.fresh_temp("__doria_property_value");
        writeln(output, indent, &format!(
            "if (isset($this->{property})) {{ ${temporary} = $this->{property}; unset($this->{property}); __doria_drop_value(${temporary}); }}"
        ));
    }
}

fn emit_constructor_catch(
    properties: &[String],
    parent_completed: Option<&str>,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let error = scopes.fresh_temp("__doria_construction_failure");
    writeln(
        output,
        indent,
        &format!("}} catch (__DoriaCheckedError ${error}) {{"),
    );
    emit_property_drops(properties, output, indent + 1, scopes);
    if let Some(parent_completed) = parent_completed {
        writeln(
            output,
            indent + 1,
            &format!("if (${parent_completed}) {{ parent::__destruct(); }}"),
        );
    }
    writeln(output, indent + 1, &format!("throw ${error};"));
    writeln(output, indent, "}");
}

fn emit_param(
    param: &Param,
    evaluated_default: Option<&ConstValue>,
    evaluation: &Evaluation,
    scopes: &PhpNameScopes,
    uses_cell: bool,
    emit_promotion: bool,
) -> String {
    let mut output = String::new();
    if let Some(access) = param
        .constructor_role
        .promoted_access()
        .filter(|_| !uses_cell && emit_promotion)
    {
        output.push_str(emit_member_access(&access));
        output.push(' ');
    }
    let payload_default = matches!(evaluated_default, Some(ConstValue::PayloadEnum(_)));
    if uses_cell && param.default.is_none() {
        output.push_str("__DoriaCell");
    } else if uses_cell {
        let value_type = php_parameter_value_type(&param.ty, payload_default, scopes);
        if value_type == "mixed" {
            output.push_str("mixed");
        } else {
            output.push_str("__DoriaCell|");
            output.push_str(&value_type);
        }
    } else if payload_default {
        output.push_str(&php_parameter_value_type(&param.ty, true, scopes));
    } else {
        output.push_str(&php_type(&param.ty, scopes));
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

fn callable_parameter_uses_cell(
    callable_plan: Option<&crate::php_closure::PhpCallablePlan>,
    parameter_index: usize,
) -> bool {
    callable_plan
        .and_then(|plan| plan.parameters.get(parameter_index))
        .is_some_and(|parameter| parameter.cell)
}

fn php_parameter_value_type(ty: &TypeRef, payload_default: bool, scopes: &PhpNameScopes) -> String {
    let php_type = php_type(ty, scopes);
    let mut members = vec![php_type.trim_start_matches('?').to_string()];
    if payload_default {
        members.push("array".to_string());
    }
    if ty.nullable {
        members.push("null".to_string());
    }
    members.join("|")
}

fn php_top_level_constant_name(name: &str) -> String {
    format!("__DORIA_CONST_{}", php_symbol_name(name))
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
    emit_owned_scope(output, indent + 1, scopes, |output, indent, scopes| {
        for statement in &block.statements {
            emit_statement(statement, output, indent, scopes);
        }
    });
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_owned_scope(
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
    emit: impl FnOnce(&mut String, usize, &mut PhpNameScopes),
) {
    let mut body = String::new();
    emit(&mut body, indent, scopes);
    if scopes.current_owned_cells().is_empty() {
        output.push_str(&body);
        return;
    }
    writeln(output, indent, "try");
    writeln(output, indent, "{");
    for line in body.lines() {
        output.push_str("    ");
        output.push_str(line);
        output.push('\n');
    }
    writeln(output, indent, "}");
    writeln(output, indent, "finally");
    writeln(output, indent, "{");
    for cell in scopes.current_owned_cells().iter().rev() {
        // An exception may precede a declaration, including during its initializer.
        writeln(
            output,
            indent + 1,
            &format!("if (isset(${cell})) {{ __doria_drop_cell(${cell}); }}"),
        );
    }
    writeln(output, indent, "}");
}

fn emit_finalizer_error_boundary(
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
    emit_body: impl FnOnce(&mut String, usize, &mut PhpNameScopes),
) {
    let forwarded = scopes.fresh_temp("__doria_finalizer_error");
    let caught = scopes.fresh_temp("__doria_finalizer_caught");
    writeln(output, indent, &format!("${forwarded} = null;"));
    writeln(output, indent, "try");
    writeln(output, indent, "{");
    let pending = Rc::new(RefCell::new(Vec::new()));
    scopes.pending_returns.push(Rc::clone(&pending));
    emit_body(output, indent + 1, scopes);
    scopes.pending_returns.pop();
    writeln(output, indent, "}");
    writeln(
        output,
        indent,
        &format!("catch (__DoriaCheckedError ${caught})"),
    );
    writeln(output, indent, "{");
    for result in pending.borrow().iter().rev() {
        writeln(
            output,
            indent + 1,
            &format!("if (isset(${result})) {{ __doria_drop_value(${result}); }}"),
        );
    }
    writeln(
        output,
        indent + 1,
        &format!("${forwarded} = __doria_detach_checked_error(${caught});"),
    );
    writeln(output, indent + 1, &format!("unset(${caught});"));
    writeln(output, indent, "}");
    writeln(
        output,
        indent,
        &format!("if (${forwarded} !== null) {{ throw ${forwarded}; }}"),
    );
}

fn emit_with_finally(
    finally: &ControlFlowFinally,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
    emit_body: impl FnOnce(&mut String, usize, &mut PhpNameScopes),
) {
    emit_finalizer_error_boundary(output, indent, scopes, |output, indent, scopes| {
        writeln(output, indent, "try");
        writeln(output, indent, "{");
        emit_body(output, indent + 1, scopes);
        writeln(output, indent, "}");
        writeln(output, indent, "finally");
        emit_block(&finally.block, output, indent, scopes);
    });
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
            let binding_ownership = decl
                .bindings
                .iter()
                .map(|binding| {
                    scopes
                        .binding_for_declaration(&binding.name, binding.span)
                        .filter(|binding| scopes.owns_binding(*binding))
                        .and_then(|binding| scopes.source_type(binding))
                        .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes))
                })
                .collect::<Vec<_>>();
            let owns_value = binding_ownership.iter().copied().any(|owns| owns);
            let initializer = if owns_value {
                emit_owned_expr(&decl.initializer, scopes)
            } else {
                emit_expr(&decl.initializer, scopes)
            };
            let binding_is_mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                || (decl.ty.is_none()
                    && matches!(
                        scopes.expression_types.get(&decl.initializer.span()),
                        Some(ResolvedType::Mixed)
                    ));
            if decl.bindings.len() == 1 {
                emit_local_binding(
                    &decl.bindings[0],
                    &initializer,
                    binding_is_mixed,
                    binding_ownership[0],
                    output,
                    indent,
                    scopes,
                );
            } else {
                let temporary = scopes.fresh_temp("__doria_grouped_value");
                writeln(output, indent, &format!("${temporary} = {initializer};"));
                for (binding, owns_binding) in decl.bindings.iter().zip(binding_ownership) {
                    emit_local_binding(
                        binding,
                        &format!("${temporary}"),
                        binding_is_mixed,
                        owns_binding,
                        output,
                        indent,
                        scopes,
                    );
                }
                writeln(output, indent, &format!("unset(${temporary});"));
            }
        }
        Stmt::Assignment(assignment) => {
            writeln(
                output,
                indent,
                &format!("{};", emit_assignment(assignment, scopes)),
            );
        }
        Stmt::Echo { expr, span } => {
            let expression = with_expression_temporaries(scopes, |scopes| {
                format!(
                    "__doria_write_stdout({}, {}, {}, {})",
                    emit_display_expr(expr, scopes),
                    php_source_location(*span, span.start),
                    php_source_location(*span, span.end),
                    scopes.callable_identity(),
                )
            });
            writeln(output, indent, &format!("{expression};"));
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                let mut result_scopes = scopes.clone();
                if scopes.returns_borrow && scopes.yield_temporaries.is_some() {
                    result_scopes.expression_temporaries = scopes.yield_temporaries.clone();
                }
                let owns_result = !scopes.returns_borrow
                    && scopes
                        .expression_types
                        .get(&expr.span())
                        .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes));
                if owns_result || scopes.has_owned_cells() {
                    let result = scopes.fresh_temp("__doria_return");
                    if owns_result {
                        for pending in &scopes.pending_returns {
                            pending.borrow_mut().push(result.clone());
                        }
                    }
                    writeln(
                        output,
                        indent,
                        &format!(
                            "${result} = {};",
                            if owns_result {
                                emit_owned_expr(expr, &result_scopes)
                            } else {
                                emit_expr(expr, &result_scopes)
                            }
                        ),
                    );
                    writeln(output, indent, &format!("return ${result};"));
                } else {
                    writeln(
                        output,
                        indent,
                        &format!("return {};", emit_expr(expr, &result_scopes)),
                    );
                }
            } else {
                writeln(output, indent, "return;");
            }
        }
        Stmt::If(if_stmt) => {
            scopes.push();
            emit_owned_scope(output, indent, scopes, |output, indent, scopes| {
                if let Some(finally) = &if_stmt.finally {
                    emit_with_finally(finally, output, indent, scopes, |output, indent, scopes| {
                        if if_stmt.given.is_some() {
                            emit_given_if(if_stmt, output, indent, scopes);
                        } else {
                            emit_if(if_stmt, output, indent, "if", None, scopes);
                        }
                    });
                } else if if_stmt.given.is_some() {
                    emit_given_if(if_stmt, output, indent, scopes);
                } else {
                    emit_if(if_stmt, output, indent, "if", None, scopes);
                }
            });
            scopes.pop();
        }
        Stmt::While(while_stmt) => {
            scopes.push();
            emit_owned_scope(output, indent, scopes, |output, indent, scopes| {
                if let Some(finally) = &while_stmt.finally {
                    emit_with_finally(finally, output, indent, scopes, |output, indent, scopes| {
                        emit_while(while_stmt, output, indent, scopes);
                    });
                    return;
                }
                emit_while(while_stmt, output, indent, scopes);
            });
            scopes.pop();
        }
        Stmt::DoWhile(do_while) => {
            if let Some(finally) = &do_while.finally {
                emit_with_finally(finally, output, indent, scopes, |output, indent, scopes| {
                    emit_do_while(do_while, output, indent, scopes);
                });
                return;
            }
            emit_do_while(do_while, output, indent, scopes);
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
            if let Expr::Assertion(assertion) = expr {
                emit_assertion_statement(assertion, output, indent, scopes);
                return;
            }
            if let Expr::FunctionCall { name, args, span } = expr {
                if name == "panic" && args.len() == 1 {
                    emit_panic(&args[0].value, *span, output, indent, scopes);
                    return;
                }
            }
            let expression = with_expression_temporaries(scopes, |scopes| emit_expr(expr, scopes));
            writeln(output, indent, &format!("{expression};"));
        }
        Stmt::Throw(statement) => emit_throw_statement(statement, output, indent, scopes),
        Stmt::Try(statement) => emit_try_statement(statement, output, indent, scopes),
    }
}

fn assertion_type_name(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Integer(ty) => ty.source_name().to_string(),
        ResolvedType::Float(ty) => ty.source_name().to_string(),
        ResolvedType::String => "string".to_string(),
        ResolvedType::Bool => "bool".to_string(),
        ResolvedType::Null => "null".to_string(),
        ResolvedType::Mixed => "mixed".to_string(),
        ResolvedType::Error => "Error".to_string(),
        ResolvedType::Enum(ty) => ty.name.clone(),
        ResolvedType::Class(ty) => {
            if ty.arguments.is_empty() {
                ty.name.clone()
            } else {
                format!(
                    "{}<{}>",
                    ty.name,
                    ty.arguments
                        .iter()
                        .map(assertion_type_name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        ResolvedType::Nullable(inner) => format!("?{}", assertion_type_name(inner)),
        ResolvedType::TypedArray(element) => format!("{}[]", assertion_type_name(element)),
        ResolvedType::List(element) => format!("List<{}>", assertion_type_name(element)),
        ResolvedType::Dictionary(key, value) => format!(
            "Dictionary<{}, {}>",
            assertion_type_name(key),
            assertion_type_name(value)
        ),
        ResolvedType::SortedDictionary(key, value) => format!(
            "SortedDictionary<{}, {}>",
            assertion_type_name(key),
            assertion_type_name(value)
        ),
        ResolvedType::Set(element) => format!("Set<{}>", assertion_type_name(element)),
        ResolvedType::SortedSet(element) => {
            format!("SortedSet<{}>", assertion_type_name(element))
        }
        ResolvedType::PriorityQueue(element) => {
            format!("PriorityQueue<{}>", assertion_type_name(element))
        }
        ResolvedType::Deque(element) => format!("Deque<{}>", assertion_type_name(element)),
        other => resolved_type_identity(other),
    }
}

fn emit_assertion_statement(
    assertion: &Assertion,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let resolved_actual_type = assertion
        .actual_type
        .as_ref()
        .map(|ty| crate::types::substitute_resolved_type(ty, &scopes.substitutions));
    let resolved_expected_type = assertion
        .expected_type
        .as_ref()
        .map(|ty| crate::types::substitute_resolved_type(ty, &scopes.substitutions));
    let error_class = php_symbol_name(crate::compiler_known_test::ASSERTION_ERROR);
    let matcher = assertion.matcher.fact_name();
    let origin = php_source_location(assertion.span, assertion.span.start.saturating_add(1));
    if assertion.matcher == crate::assertions::AssertionMatcher::Fail {
        let message = scopes.fresh_temp("__doria_assertion_message");
        let value = assertion
            .user_message
            .as_deref()
            .expect("checked fail assertion has a message");
        writeln(
            output,
            indent,
            &format!("${message} = {};", emit_expr(value, scopes)),
        );
        writeln(
            output,
            indent,
            &format!(
                "__doria_throw(new {error_class}(${message}, {}, false, false, \"\", \"\", false, \"\", \"\", false, \"\", true, ${message}), {origin}, {});",
                emit_php_string_literal(matcher),
                scopes.callable_identity(),
            ),
        );
        return;
    }

    let actual = scopes.fresh_temp("__doria_assertion_actual");
    let actual_expr = assertion
        .actual
        .as_deref()
        .expect("checked matcher assertion has an actual operand");
    writeln(
        output,
        indent,
        &format!("${actual} = {};", emit_expr(actual_expr, scopes)),
    );
    let expected = assertion.expected.as_deref().map(|value| {
        let name = scopes.fresh_temp("__doria_assertion_expected");
        writeln(
            output,
            indent,
            &format!("${name} = {};", emit_expr(value, scopes)),
        );
        name
    });
    let throw_facts = if assertion.matcher == crate::assertions::AssertionMatcher::Throws {
        let actual_type = scopes.fresh_temp("__doria_assertion_throw_actual_type");
        let actual_presentation = scopes.fresh_temp("__doria_assertion_throw_actual_presentation");
        let expected_type = scopes.fresh_temp("__doria_assertion_throw_expected_type");
        let expected_presentation =
            scopes.fresh_temp("__doria_assertion_throw_expected_presentation");
        let difference = scopes.fresh_temp("__doria_assertion_throw_difference");
        let expected_error_type = match resolved_expected_type.as_ref() {
            Some(ResolvedType::Function(function)) => match &function
                .parameters
                .first()
                .expect("checked throw inspector has one parameter")
                .ty
            {
                ResolvedType::Class(class) => class.name.as_str(),
                ResolvedType::Interface(interface_type) => interface_type.name.as_str(),
                ResolvedType::Error => "Error",
                _ => unreachable!("checked throw inspector parameter implements Error"),
            },
            _ => "Error",
        };
        writeln(output, indent, &format!("${actual_type} = \"NoError\";"));
        writeln(
            output,
            indent,
            &format!("${actual_presentation} = \"No Checked Error\";"),
        );
        writeln(
            output,
            indent,
            &format!(
                "${expected_type} = {};",
                emit_php_string_literal(if assertion.negated {
                    "NoError"
                } else {
                    expected_error_type
                })
            ),
        );
        writeln(
            output,
            indent,
            &format!(
                "${expected_presentation} = {};",
                emit_php_string_literal(if assertion.negated {
                    "No Checked Error"
                } else if expected_error_type == "Error" {
                    "A Checked Error"
                } else {
                    expected_error_type
                })
            ),
        );
        writeln(
            output,
            indent,
            &format!("${difference} = \"No Checked Error Was Produced\";"),
        );
        Some((
            actual_type,
            actual_presentation,
            expected_type,
            expected_presentation,
            difference,
        ))
    } else {
        None
    };
    let comparison = if assertion.matcher == crate::assertions::AssertionMatcher::Throws {
        let (throw_actual_type, throw_actual_presentation, _, _, throw_difference) =
            throw_facts.as_ref().expect("throw facts");
        let threw = scopes.fresh_temp("__doria_assertion_threw");
        let result = scopes.fresh_temp("__doria_assertion_result");
        let caught = scopes.fresh_temp("__doria_assertion_caught");
        writeln(output, indent, &format!("${threw} = false;"));
        writeln(output, indent, "try");
        writeln(output, indent, "{");
        writeln(output, indent + 1, &format!("${result} = (${actual})();"));
        writeln(
            output,
            indent + 1,
            &format!("__doria_drop_value(${result});"),
        );
        writeln(output, indent, "}");
        writeln(
            output,
            indent,
            &format!("catch (__DoriaCheckedError ${caught})"),
        );
        writeln(output, indent, "{");
        let caught_error = scopes.fresh_temp("__doria_assertion_error");
        writeln(
            output,
            indent + 1,
            &format!("${caught_error} = ${caught}->error();"),
        );
        writeln(
            output,
            indent + 1,
            &format!("${throw_actual_type} = ${caught}->descriptor->typeName;"),
        );
        writeln(
            output,
            indent + 1,
            &format!(
                "${throw_actual_presentation} = __doria_bound_assertion_text(${throw_actual_type} . {} . __doria_assertion_error_message(${caught_error}->message));",
                emit_php_string_literal(": ")
            ),
        );
        writeln(
            output,
            indent + 1,
            &format!(
                "${throw_difference} = {};",
                emit_php_string_literal(if assertion.negated {
                    "A Checked Error Was Produced"
                } else {
                    "The Checked Error Type Did Not Match"
                })
            ),
        );
        if let Some(inspector) = expected.as_ref() {
            let parameter = match resolved_expected_type.as_ref() {
                Some(ResolvedType::Function(function)) => function.parameters.first(),
                _ => None,
            }
            .expect("checked throw inspector has one parameter");
            let error = format!("${caught_error}");
            if parameter.ty != ResolvedType::Error {
                writeln(
                    output,
                    indent + 1,
                    &format!("if ({})", php_error_matches(&error, &parameter.ty, scopes)),
                );
                writeln(output, indent + 1, "{");
                writeln(output, indent + 2, &format!("(${inspector})({error});"));
                writeln(output, indent + 2, &format!("${threw} = true;"));
                writeln(output, indent + 1, "}");
            } else {
                writeln(output, indent + 1, &format!("(${inspector})({error});"));
                writeln(output, indent + 1, &format!("${threw} = true;"));
            }
        } else {
            writeln(output, indent + 1, &format!("${threw} = true;"));
        }
        writeln(output, indent + 1, &format!("unset(${caught_error});"));
        writeln(output, indent + 1, &format!("unset(${caught});"));
        writeln(output, indent, "}");
        format!("${threw}")
    } else {
        match assertion.matcher {
            crate::assertions::AssertionMatcher::GreaterThan
            | crate::assertions::AssertionMatcher::GreaterThanOrEqual
            | crate::assertions::AssertionMatcher::LessThan
            | crate::assertions::AssertionMatcher::LessThanOrEqual
                if is_uint64(resolved_actual_type.as_ref()) =>
            {
                let operator = match assertion.matcher {
                    crate::assertions::AssertionMatcher::GreaterThan => ">",
                    crate::assertions::AssertionMatcher::GreaterThanOrEqual => ">=",
                    crate::assertions::AssertionMatcher::LessThan => "<",
                    _ => "<=",
                };
                emit_uint64_comparison(
                    &format!("${actual}"),
                    operator,
                    &format!("${}", expected.as_ref().unwrap()),
                )
            }
            crate::assertions::AssertionMatcher::Equal => {
                if resolved_actual_type.as_ref().is_some_and(|ty| {
                    scopes.specialization.core_operation(
                        assertion.span,
                        crate::compiler_known_contracts::CoreValueOperation::Equal,
                        ty,
                        &scopes.substitutions,
                    )
                }) {
                    emit_core_equality(
                        &format!("${actual}"),
                        &format!("${}", expected.as_ref().unwrap()),
                        "equals",
                    )
                } else {
                    format!("__doria_equal(${actual}, ${})", expected.as_ref().unwrap())
                }
            }
            crate::assertions::AssertionMatcher::Null => format!("${actual} === null"),
            crate::assertions::AssertionMatcher::True => format!("${actual} === true"),
            crate::assertions::AssertionMatcher::False => format!("${actual} === false"),
            crate::assertions::AssertionMatcher::GreaterThan => {
                format!(
                    "__doria_greater(${actual}, ${})",
                    expected.as_ref().unwrap()
                )
            }
            crate::assertions::AssertionMatcher::GreaterThanOrEqual => format!(
                "__doria_greater_equal(${actual}, ${})",
                expected.as_ref().unwrap()
            ),
            crate::assertions::AssertionMatcher::LessThan => {
                format!("__doria_less(${actual}, ${})", expected.as_ref().unwrap())
            }
            crate::assertions::AssertionMatcher::LessThanOrEqual => format!(
                "__doria_less_equal(${actual}, ${})",
                expected.as_ref().unwrap()
            ),
            crate::assertions::AssertionMatcher::StringContains => {
                format!("str_contains(${actual}, ${})", expected.as_ref().unwrap())
            }
            crate::assertions::AssertionMatcher::StringStartsWith => {
                format!(
                    "str_starts_with(${actual}, ${})",
                    expected.as_ref().unwrap()
                )
            }
            crate::assertions::AssertionMatcher::StringEndsWith => {
                format!("str_ends_with(${actual}, ${})", expected.as_ref().unwrap())
            }
            crate::assertions::AssertionMatcher::StringEmpty => format!("${actual} === \"\""),
            crate::assertions::AssertionMatcher::CollectionContains => format!(
                "__doria_assertion_collection_contains(${actual}, ${}, {})",
                expected.as_ref().unwrap(),
                php_equality_callback(assertion.span, resolved_expected_type.as_ref(), scopes)
            ),
            crate::assertions::AssertionMatcher::CollectionEmpty => {
                format!("__doria_assertion_collection_count(${actual}) === 0")
            }
            crate::assertions::AssertionMatcher::CollectionCount => format!(
                "__doria_assertion_collection_count(${actual}) === ${}",
                expected.as_ref().unwrap()
            ),
            crate::assertions::AssertionMatcher::DictionaryHasKey => format!(
                "__doria_assertion_dictionary_has_key(${actual}, ${})",
                expected.as_ref().unwrap()
            ),
            crate::assertions::AssertionMatcher::DictionaryHasValue => format!(
                "__doria_assertion_dictionary_has_value(${actual}, ${}, {})",
                expected.as_ref().unwrap(),
                php_equality_callback(assertion.span, resolved_expected_type.as_ref(), scopes)
            ),
            crate::assertions::AssertionMatcher::Throws => unreachable!(),
            crate::assertions::AssertionMatcher::Fail => unreachable!(),
        }
    };
    let passed = if assertion.negated {
        format!("!({comparison})")
    } else {
        comparison
    };
    writeln(output, indent, &format!("if (!({passed}))"));
    writeln(output, indent, "{");
    let (actual_type, actual_presentation) =
        if let Some((throw_actual_type, throw_actual_presentation, _, _, _)) = throw_facts.as_ref()
        {
            (
                format!("${throw_actual_type}"),
                throw_actual_presentation.clone(),
            )
        } else {
            let ty = resolved_actual_type
                .as_ref()
                .map(assertion_type_name)
                .unwrap_or_default();
            let presentation = scopes.fresh_temp("__doria_assertion_actual_text");
            writeln(
                output,
                indent + 1,
                &format!(
                    "${presentation} = __doria_assertion_presentation(${actual}, {});",
                    emit_php_string_literal(&ty)
                ),
            );
            (emit_php_string_literal(&ty), presentation)
        };
    let (expected_present, expected_type, expected_presentation) =
        if let Some((_, _, throw_expected_type, throw_expected_presentation, _)) =
            throw_facts.as_ref()
        {
            (
                true,
                format!("${throw_expected_type}"),
                format!("${throw_expected_presentation}"),
            )
        } else if let Some(ref expected) = expected {
            let ty = resolved_expected_type
                .as_ref()
                .map(assertion_type_name)
                .unwrap_or_default();
            let presentation = scopes.fresh_temp("__doria_assertion_expected_text");
            writeln(
                output,
                indent + 1,
                &format!(
                    "${presentation} = __doria_assertion_presentation(${expected}, {});",
                    emit_php_string_literal(&ty)
                ),
            );
            (
                true,
                emit_php_string_literal(&ty),
                format!("${presentation}"),
            )
        } else {
            match assertion.matcher {
                crate::assertions::AssertionMatcher::Null => (
                    true,
                    emit_php_string_literal("null"),
                    emit_php_string_literal("null"),
                ),
                crate::assertions::AssertionMatcher::True => (
                    true,
                    emit_php_string_literal("bool"),
                    emit_php_string_literal("true"),
                ),
                crate::assertions::AssertionMatcher::False => (
                    true,
                    emit_php_string_literal("bool"),
                    emit_php_string_literal("false"),
                ),
                crate::assertions::AssertionMatcher::StringEmpty => (
                    true,
                    emit_php_string_literal("string"),
                    emit_php_string_literal("\"\""),
                ),
                _ => (
                    false,
                    emit_php_string_literal(""),
                    emit_php_string_literal(""),
                ),
            }
        };
    let static_difference =
        crate::assertions::stable_difference(assertion.matcher, assertion.negated);
    let dynamic_difference = match assertion.matcher {
        crate::assertions::AssertionMatcher::Equal
            if matches!(resolved_actual_type, Some(ResolvedType::String))
                && matches!(resolved_expected_type, Some(ResolvedType::String)) =>
        {
            expected.as_ref().map(|expected| {
                format!("__doria_assertion_string_difference(${actual}, ${expected}, 0)")
            })
        }
        crate::assertions::AssertionMatcher::StringStartsWith => {
            expected.as_ref().map(|expected| {
                format!("__doria_assertion_string_difference(${actual}, ${expected}, 1)")
            })
        }
        crate::assertions::AssertionMatcher::StringEndsWith => expected.as_ref().map(|expected| {
            format!("__doria_assertion_string_difference(${actual}, ${expected}, 2)")
        }),
        crate::assertions::AssertionMatcher::Equal
            if matches!(resolved_actual_type, Some(ResolvedType::Bytes))
                && matches!(resolved_expected_type, Some(ResolvedType::Bytes)) =>
        {
            expected.as_ref().map(|expected| {
                format!("__doria_assertion_bytes_difference(${actual}, ${expected})")
            })
        }
        crate::assertions::AssertionMatcher::CollectionCount => expected
            .as_ref()
            .map(|expected| format!("__doria_assertion_count_difference(${actual}, ${expected})")),
        _ => None,
    };
    let throw_difference = throw_facts
        .as_ref()
        .map(|(_, _, _, _, difference)| format!("${difference}"));
    let difference_present =
        throw_difference.is_some() || dynamic_difference.is_some() || static_difference.is_some();
    let difference = throw_difference
        .or(dynamic_difference)
        .unwrap_or_else(|| emit_php_string_literal(static_difference.unwrap_or_default()));
    writeln(
        output,
        indent + 1,
        &format!(
            "__doria_throw(new {error_class}({}, {}, {}, true, {}, ${actual_presentation}, {}, {}, {expected_presentation}, {}, {}, false, \"\"), {origin}, {});",
            emit_php_string_literal(crate::assertions::stable_message(
                assertion.matcher,
                assertion.negated,
            )),
            emit_php_string_literal(matcher),
            if assertion.negated { "true" } else { "false" },
            actual_type,
            if expected_present { "true" } else { "false" },
            expected_type,
            if difference_present { "true" } else { "false" },
            difference,
            scopes.callable_identity(),
        ),
    );
    writeln(output, indent, "}");
}

fn emit_throw_statement(
    statement: &ThrowStmt,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    debug_assert!(scopes.throw_error_types.contains_key(&statement.span));
    writeln(
        output,
        indent,
        &format!(
            "__doria_throw({}, {}, {});",
            emit_owned_expr(&statement.expr, scopes),
            php_source_location(statement.span, statement.span.start.saturating_add(1),),
            scopes.callable_identity(),
        ),
    );
}

fn emit_try_statement(
    statement: &TryStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if statement.finally.is_some() {
        emit_finalizer_error_boundary(output, indent, scopes, |output, indent, scopes| {
            emit_try_statement_inner(statement, output, indent, scopes);
        });
    } else {
        emit_try_statement_inner(statement, output, indent, scopes);
    }
}

fn php_error_matches(value: &str, error_type: &ResolvedType, scopes: &PhpNameScopes) -> String {
    match error_type {
        ResolvedType::Error => "true".to_string(),
        ResolvedType::Class(class) => {
            let symbol = scopes
                .specialization
                .class_symbols
                .get(class)
                .cloned()
                .unwrap_or_else(|| php_symbol_name(&class.name));
            format!("{value} instanceof {symbol}")
        }
        ResolvedType::Interface(interface_type) => format!(
            "{value} instanceof {}",
            interface::specialization_name(interface_type)
        ),
        _ => unreachable!("semantic checking restricts checked errors to Error values"),
    }
}

fn emit_try_statement_inner(
    statement: &TryStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    writeln(output, indent, "try");
    emit_block(&statement.body, output, indent, scopes);

    if !statement.catches.is_empty() {
        let caught = scopes.fresh_temp("__doria_checked_error");
        writeln(
            output,
            indent,
            &format!("catch (__DoriaCheckedError ${caught})"),
        );
        writeln(output, indent, "{");
        scopes.push();
        let mut has_catch_all = false;
        for (index, clause) in statement.catches.iter().enumerate() {
            let error_type = scopes
                .catch_error_types
                .get(&clause.span)
                .unwrap_or(&clause.error_type);
            let condition = match error_type {
                ResolvedType::Error => {
                    has_catch_all = true;
                    "true".to_string()
                }
                ResolvedType::Class(_) | ResolvedType::Interface(_) => {
                    php_error_matches(&format!("${caught}->error()"), error_type, scopes)
                }
                _ => unreachable!("semantic checking restricts catch types to Error values"),
            };
            writeln(
                output,
                indent + 1,
                &format!("{} ({condition})", if index == 0 { "if" } else { "elseif" }),
            );
            writeln(output, indent + 1, "{");
            scopes.push();
            let binding = match &clause.binding {
                Some(binding) => scopes.declare(&binding.name),
                None => scopes.fresh_temp("__doria_caught_value"),
            };
            writeln(
                output,
                indent + 2,
                &format!("${binding} = new __DoriaCell(${caught}->takeError());"),
            );
            if let Some(source) = &clause.binding {
                let id = scopes
                    .binding_for_declaration(&source.name, source.span)
                    .expect("checked catch binding exists");
                scopes.bind_place(id, PhpBindingPlace::Cell(binding.clone()));
            }
            scopes.own_cell(binding);
            emit_owned_scope(output, indent + 2, scopes, |output, indent, scopes| {
                for body_statement in &clause.body.statements {
                    emit_statement(body_statement, output, indent, scopes);
                }
            });
            scopes.pop();
            writeln(output, indent + 1, "}");
        }
        if !has_catch_all {
            writeln(output, indent + 1, "else");
            writeln(output, indent + 1, "{");
            writeln(output, indent + 2, &format!("throw ${caught};"));
            writeln(output, indent + 1, "}");
        }
        scopes.pop();
        writeln(output, indent, "}");
    }

    if let Some(finally) = &statement.finally {
        writeln(output, indent, "finally");
        emit_block(&finally.body, output, indent, scopes);
    }
}

fn emit_while(
    while_stmt: &WhileStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if let Some(given) = &while_stmt.given {
        let mut predicates = emit_given_setup(given, output, indent, scopes);
        predicates.push(emit_expr(&while_stmt.condition, scopes));
        write_indent(output, indent);
        output.push_str("while (");
        output.push_str(&emit_bool_chain(predicates.iter().map(String::as_str)));
        output.push_str(")\n");
        emit_block(&while_stmt.body, output, indent, scopes);
        return;
    }
    write_indent(output, indent);
    output.push_str("while (");
    output.push_str(&emit_expr(&while_stmt.condition, scopes));
    output.push_str(")\n");
    emit_block(&while_stmt.body, output, indent, scopes);
}

fn emit_do_while(
    do_while: &DoWhileStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    write_indent(output, indent);
    output.push_str("do\n");
    emit_block(&do_while.body, output, indent, scopes);
    writeln(
        output,
        indent,
        &format!("while ({});", emit_expr(&do_while.condition, scopes)),
    );
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
            "__doria_panic(\"P1000\", {}, {}, {}, {});",
            php_source_location(span, span.start),
            php_source_location(span, span.end),
            emit_expr(message, scopes),
            scopes.callable_identity(),
        ),
    );
}

fn emit_for(for_stmt: &ForStmt, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    scopes.push();
    emit_owned_scope(output, indent, scopes, |output, indent, scopes| {
        if let Some(initializer) = &for_stmt.initializer {
            let statement = match initializer {
                ForInitializer::VarDecl(decl) => Stmt::VarDecl(decl.clone()),
                ForInitializer::Assignment(assignment) => Stmt::Assignment(assignment.clone()),
            };
            emit_statement(&statement, output, indent, scopes);
        }
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
        output.push_str("for (; ");
        output.push_str(&condition);
        output.push_str("; ");
        output.push_str(&increment);
        output.push_str(")\n");
        emit_block(&for_stmt.body, output, indent, scopes);
    });
    scopes.pop();
}

fn emit_for_increment(increment: &ForIncrement, scopes: &PhpNameScopes) -> String {
    match increment {
        ForIncrement::Increment(increment) => emit_increment(increment, scopes),
        ForIncrement::Assignment(assignment) => emit_assignment(assignment, scopes),
    }
}

fn resolved_type_needs_php_drop(ty: &ResolvedType, scopes: &PhpNameScopes) -> bool {
    match ty {
        ResolvedType::Interface(_)
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::SharedHandle(_, _) => true,
        ResolvedType::TraitSelf(_) => unreachable!("trait execution is pending Stage 35 Slice 4"),
        ResolvedType::Mixed
        | ResolvedType::Error
        | ResolvedType::Function(_)
        | ResolvedType::TypeParameter(_) => true,
        ResolvedType::Enum(enum_type) => scopes
            .symbols
            .payload_enums_with_php_destructors
            .contains(&enum_type.name),
        ResolvedType::Class(_) => true,
        ResolvedType::TypedArray(element)
        | ResolvedType::List(element)
        | ResolvedType::Set(element)
        | ResolvedType::SortedSet(element)
        | ResolvedType::PriorityQueue(element)
        | ResolvedType::Deque(element) => resolved_type_needs_php_drop(element, scopes),
        ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
            resolved_type_needs_php_drop(key, scopes) || resolved_type_needs_php_drop(value, scopes)
        }
        ResolvedType::Nullable(inner) => resolved_type_needs_php_drop(inner, scopes),
        ResolvedType::Void
        | ResolvedType::Bytes
        | ResolvedType::Integer(_)
        | ResolvedType::Float(_)
        | ResolvedType::String
        | ResolvedType::Bool
        | ResolvedType::Null
        | ResolvedType::Unsupported => false,
    }
}

fn assignment_target_needs_php_drop(target: &Expr, scopes: &PhpNameScopes) -> bool {
    match target {
        Expr::Grouped { expr, .. } => assignment_target_needs_php_drop(expr, scopes),
        Expr::Variable { span, .. } => scopes
            .binding_for_use(*span)
            .and_then(|binding| scopes.source_type(binding))
            .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes)),
        Expr::PropertyAccess { span, .. } => scopes
            .closure_plan
            .property_write_types
            .get(span)
            .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes)),
        _ => scopes
            .expression_types
            .get(&target.span())
            .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes)),
    }
}

fn emit_local_binding(
    declaration: &VarBinding,
    initializer: &str,
    binding_is_mixed: bool,
    owns_value: bool,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let php_name = scopes.declare(&declaration.name);
    let binding = scopes.binding_for_declaration(&declaration.name, declaration.span);
    if binding_is_mixed {
        scopes.mark_mixed(&declaration.name);
    }
    if owns_value || binding.is_some_and(|binding| scopes.needs_cell(binding)) {
        writeln(
            output,
            indent,
            &format!("${php_name} = new __DoriaCell({initializer});"),
        );
        scopes.bind_place(
            binding.expect("checked binding exists"),
            PhpBindingPlace::Cell(php_name.clone()),
        );
        if owns_value {
            scopes.own_cell(php_name);
        }
    } else {
        writeln(output, indent, &format!("${php_name} = {initializer};"));
        if let Some(binding) = binding {
            scopes.bind_place(binding, PhpBindingPlace::Direct(php_name));
        }
    }
}

fn assignment_target_cell(target: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    match target {
        Expr::Grouped { expr, .. } => assignment_target_cell(expr, scopes),
        Expr::Variable { span, .. } => scopes.place_for_use(*span).and_then(PhpBindingPlace::cell),
        _ => None,
    }
}

fn emit_owned_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    if scopes.expression_temporaries.is_none() {
        return with_expression_temporaries(scopes, |scopes| emit_owned_expr(expr, scopes));
    }
    let value = emit_owned_expr_unconverted(expr, scopes);
    interface::convert_collection(expr, value, true, scopes)
}

fn emit_owned_expr_unconverted(expr: &Expr, scopes: &PhpNameScopes) -> String {
    match expr {
        Expr::Grouped { expr: inner, .. } => emit_mixed_box_plan(
            expr,
            format!("({})", emit_owned_expr(inner, scopes)),
            scopes,
        ),
        Expr::Variable { name, span } => {
            let emitted = scopes
                .place_for_use(*span)
                .filter(|_| {
                    scopes
                        .binding_for_use(*span)
                        .and_then(|binding| scopes.source_type(binding))
                        .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes))
                })
                .and_then(PhpBindingPlace::cell)
                .map_or_else(
                    || emit_expr_unboxed(expr, scopes),
                    |cell| format!("__doria_take_cell({cell})"),
                );
            let emitted = if scopes.is_mixed_binding(name)
                && scopes
                    .expression_types
                    .get(span)
                    .is_some_and(|ty| !is_mixed_storage_type(ty))
            {
                format!("__doria_mixed_value({emitted})")
            } else {
                emitted
            };
            emit_mixed_box_plan(expr, emitted, scopes)
        }
        Expr::Binary {
            left,
            op: BinaryOp::Coalesce,
            right,
            ..
        } => emit_mixed_box_plan(
            expr,
            format!(
                "({} ?? {})",
                emit_owned_expr(left, scopes),
                emit_owned_expr(right, scopes)
            ),
            scopes,
        ),
        Expr::Match {
            scrutinee,
            arms,
            span,
            ..
        } => emit_mixed_box_plan(
            expr,
            emit_match_expression(scrutinee, arms, *span, true, scopes),
            scopes,
        ),
        Expr::When(when) => emit_mixed_box_plan(
            expr,
            emit_when_expression(
                when.given.as_ref(),
                &when.branches,
                when.finally.as_ref(),
                when.span,
                true,
                scopes,
            ),
            scopes,
        ),
        _ => emit_mixed_box_plan(expr, emit_expr_unboxed(expr, scopes), scopes),
    }
}

fn emit_index_assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> Option<String> {
    if assignment.op != AssignOp::Assign {
        return None;
    }
    let mut target = &assignment.target;
    while let Expr::Grouped { expr, .. } = target {
        target = expr;
    }
    let Expr::Index {
        collection,
        index,
        span,
    } = target
    else {
        return None;
    };
    Some(with_expression_temporaries(scopes, |scopes| {
        if let Some(value) =
            core_collection::assignment(collection, index, &assignment.value, scopes)
        {
            return value;
        }
        format!(
            "__doria_collection_set({}, {}, {}, {}, {}, {}, {})",
            emit_assignment_target(collection, scopes),
            emit_expr(index, scopes),
            emit_owned_expr(&assignment.value, scopes),
            matches!(
                scopes.expression_types.get(&collection.span()),
                Some(ResolvedType::Dictionary(_, _))
            ),
            php_source_location(*span, span.start),
            php_source_location(*span, span.end),
            scopes.callable_identity(),
        )
    }))
}

fn emit_assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> String {
    if scopes.expression_temporaries.is_none() {
        return with_expression_temporaries(scopes, |scopes| emit_assignment(assignment, scopes));
    }
    if let Some(expression) = emit_index_assignment(assignment, scopes) {
        return expression;
    }
    if assignment.op == AssignOp::Assign
        && assignment_target_needs_php_drop(&assignment.target, scopes)
    {
        let replacement = emit_owned_expr(&assignment.value, scopes);
        if let Some(cell) = assignment_target_cell(&assignment.target, scopes) {
            return format!("__doria_replace_cell({cell}, {replacement})");
        }
        let mut target = &assignment.target;
        while let Expr::Grouped { expr, .. } = target {
            target = expr;
        }
        if let Expr::PropertyAccess {
            object, property, ..
        } = target
        {
            // Keep internal-property access in the authored lexical scope.
            return format!("(function(object $receiver, mixed $replacement): void {{ $previous = $receiver->{property} ?? null; $receiver->{property} = $replacement; __doria_drop_value($previous); }})({}, {replacement})", emit_member_receiver(object, scopes));
        }
        return format!("{} = {replacement}", emit_assignment_target(target, scopes));
    }
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
        Expr::Index {
            collection, index, ..
        } => format!(
            "{}[{}]",
            emit_assignment_target(collection, scopes),
            emit_expr(index, scopes)
        ),
        _ => emit_expr(expr, scopes),
    }
}

fn emit_if(
    if_stmt: &IfStmt,
    output: &mut String,
    indent: usize,
    keyword: &str,
    gate: Option<&str>,
    scopes: &mut PhpNameScopes,
) {
    write_indent(output, indent);
    output.push_str(keyword);
    output.push_str(" (");
    if let Some(gate) = gate {
        output.push_str(gate);
        output.push_str(" && ");
    }
    output.push_str(&emit_expr(&if_stmt.condition, scopes));
    output.push_str(")\n");
    emit_block(&if_stmt.then_block, output, indent, scopes);

    if let Some(else_branch) = &if_stmt.else_branch {
        match else_branch {
            ElseBranch::If(else_if) => emit_if(else_if, output, indent, "else if", gate, scopes),
            ElseBranch::Block(block) => {
                write_indent(output, indent);
                output.push_str("else\n");
                emit_block(block, output, indent, scopes);
            }
        }
    }
}

fn emit_given_if(if_stmt: &IfStmt, output: &mut String, indent: usize, scopes: &mut PhpNameScopes) {
    let given = if_stmt
        .given
        .as_ref()
        .expect("given-if emission requires a prelude");
    let predicates = emit_given_setup(given, output, indent, scopes);
    let gate = if predicates.is_empty() {
        None
    } else {
        let gate = scopes.fresh_temp("__doria_given_gate");
        writeln(
            output,
            indent,
            &format!(
                "${gate} = {};",
                emit_bool_chain(predicates.iter().map(String::as_str))
            ),
        );
        Some(format!("${gate}"))
    };
    emit_if(if_stmt, output, indent, "if", gate.as_deref(), scopes);
}

fn emit_given_setup(
    given: &GivenPrelude,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Vec<String> {
    let info = scopes
        .given_preludes
        .get(&given.span)
        .cloned()
        .expect("checked given prelude must have a semantic plan");
    let predicate_indices = info
        .predicate_statement_indices
        .into_iter()
        .collect::<HashSet<_>>();
    let mut predicates = Vec::new();
    for (index, statement) in given.block.statements.iter().enumerate() {
        if predicate_indices.contains(&index) {
            let Stmt::Expr { expr, .. } = statement else {
                unreachable!("checked given predicate must be an expression statement")
            };
            predicates.push(emit_expr(expr, scopes));
        } else {
            emit_statement(statement, output, indent, scopes);
        }
    }
    predicates
}

fn emit_bool_chain<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let values = values.map(|value| format!("({value})")).collect::<Vec<_>>();
    if values.is_empty() {
        "true".to_string()
    } else {
        values.join(" && ")
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

    let mut iterable_scopes = scopes.clone();
    let temporaries = Rc::new(RefCell::new(Vec::new()));
    iterable_scopes.expression_temporaries = Some(Rc::clone(&temporaries));
    let emit_iterable = |expr: &Expr| {
        if iterable_scopes
            .expression_types
            .get(&expr.span())
            .is_some_and(core_collection::uses_receiver)
        {
            core_collection::receiver(expr, false, &iterable_scopes)
        } else {
            emit_expr(expr, &iterable_scopes)
        }
    };
    let iterable =
        if let Some((dictionary, projection)) = dictionary_foreach_projection(&foreach.iterable) {
            format!(
                "__doria_collection_projection({}, {})",
                emit_iterable(dictionary),
                if projection == DictionaryForeachProjection::Keys {
                    "true"
                } else {
                    "false"
                }
            )
        } else {
            emit_iterable(&foreach.iterable)
        };
    let temporaries = temporaries.borrow();
    if temporaries.is_empty() {
        emit_foreach_body(foreach, &iterable, output, indent, scopes);
    } else {
        writeln(output, indent, "try");
        writeln(output, indent, "{");
        emit_foreach_body(foreach, &iterable, output, indent + 1, scopes);
        writeln(output, indent, "}");
        writeln(output, indent, "finally");
        writeln(output, indent, "{");
        for temporary in temporaries.iter().rev() {
            writeln(
                output,
                indent + 1,
                &format!("if (isset(${temporary})) {{ __doria_drop_value(${temporary}); }}"),
            );
        }
        writeln(output, indent, "}");
    }
}

fn emit_foreach_body(
    foreach: &ForeachStmt,
    iterable: &str,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if foreach.iterable_family == crate::semantics::ForeachIterableFamily::PublicIterable {
        emit_public_foreach(foreach, iterable, output, indent, scopes);
        return;
    }
    scopes.push();
    let first_name = foreach
        .first_binding
        .as_ref()
        .map(|binding| scopes.declare(&binding.name));
    let value_name = scopes.declare(&foreach.value_binding.name);
    let sequence_index = (foreach.iteration_kind
        == crate::semantics::ForeachIterationKind::SequenceIndex)
        .then(|| scopes.fresh_temp("__doria_sequence_index"));

    if let Some(sequence_index) = &sequence_index {
        writeln(output, indent, &format!("${sequence_index} = 0;"));
    }

    write_indent(output, indent);
    output.push_str("foreach (");
    output.push_str(iterable);
    output.push_str(" as ");
    if foreach.iteration_kind == crate::semantics::ForeachIterationKind::DictionaryKey {
        if let Some(first_name) = &first_name {
            output.push('$');
            output.push_str(first_name);
            output.push_str(" => ");
        }
    }
    if foreach.value_binding.writable {
        output.push('&');
    }
    output.push('$');
    output.push_str(&value_name);
    output.push_str(")\n");
    writeln(output, indent, "{");
    if let (Some(first_name), Some(sequence_index)) = (&first_name, &sequence_index) {
        writeln(
            output,
            indent + 1,
            &format!("${first_name} = ${sequence_index}++;"),
        );
    }
    emit_owned_scope(output, indent + 1, scopes, |output, indent, scopes| {
        for statement in &foreach.body.statements {
            emit_statement(statement, output, indent, scopes);
        }
    });
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_public_foreach(
    foreach: &ForeachStmt,
    iterable: &str,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    scopes.push();
    let cursor = scopes.fresh_temp("__doria_iterator");
    let value = scopes.declare(&foreach.value_binding.name);
    writeln(
        output,
        indent,
        &format!("${cursor} = ({iterable})->iterator();"),
    );
    writeln(output, indent, "try {");
    writeln(
        output,
        indent + 1,
        &format!("for (; ${cursor}->hasCurrent(); ${cursor}->advance()) {{"),
    );
    writeln(
        output,
        indent + 2,
        &format!("${value} = ${cursor}->getCurrent();"),
    );
    writeln(output, indent + 2, "try {");
    emit_owned_scope(output, indent + 3, scopes, |output, indent, scopes| {
        for statement in &foreach.body.statements {
            emit_statement(statement, output, indent, scopes);
        }
    });
    writeln(output, indent + 2, "} finally {");
    writeln(output, indent + 3, &format!("unset(${value});"));
    writeln(output, indent + 2, "}");
    writeln(output, indent + 1, "}");
    writeln(output, indent, "} finally {");
    writeln(
        output,
        indent + 1,
        &format!("__doria_drop_value(${cursor});"),
    );
    writeln(output, indent, "}");
    scopes.pop();
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
    let value_name = scopes.declare(&foreach.value_binding.name);
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
    emit_owned_scope(output, indent + 1, scopes, |output, indent, scopes| {
        for statement in &foreach.body.statements {
            emit_statement(statement, output, indent, scopes);
        }
    });
    scopes.pop();
    writeln(output, indent, "}");
}

fn emit_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    if let Some(value) = emit_primitive_core_call(expr, scopes) {
        return value;
    }
    if scopes.expression_temporaries.is_none() {
        return with_expression_temporaries(scopes, |scopes| {
            interface::convert_collection(
                expr,
                emit_mixed_box_plan(expr, emit_expr_unboxed(expr, scopes), scopes),
                false,
                scopes,
            )
        });
    }
    let emitted = emit_expr_unboxed(expr, scopes);
    let emitted = emit_mixed_box_plan(expr, emitted, scopes);
    let emitted = interface::convert_collection(expr, emitted, false, scopes);
    if expression_creates_owner(expr, scopes) {
        let temporary = scopes.expression_temp("__doria_owned_expression_", expr.span());
        let mut temporaries = scopes.expression_temporaries.as_ref().unwrap().borrow_mut();
        if !temporaries.contains(&temporary) {
            temporaries.push(temporary.clone());
        }
        format!("(${temporary} = {emitted})")
    } else {
        emitted
    }
}

fn expression_creates_owner(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    if !scopes
        .expression_types
        .get(&expr.span())
        .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes))
    {
        return false;
    }
    match expr {
        Expr::New { .. } | Expr::Closure(_) | Expr::Array { .. } => true,
        Expr::FunctionCall { span, .. } | Expr::MethodCall { span, .. } | Expr::StaticCall { span, .. } => {
            !scopes.closure_plan.callable_at(*span).is_some_and(|plan| plan.returns_borrow)
                && !matches!(expr, Expr::MethodCall { object, method, .. } if scopes.expression_types.get(&object.span()).is_some_and(|ty| php_collection_borrowed_get(ty, method)))
        }
        Expr::CallableCall(call) => scopes.closure_plan.callable_value_calls.get(&call.span)
            .is_some_and(|call| matches!(&call.function_type, ResolvedType::Function(function) if function.return_borrow.is_none())),
        _ => false,
    }
}

fn php_collection_borrowed_get(ty: &ResolvedType, method: &str) -> bool {
    match ty {
        ResolvedType::Nullable(inner) => php_collection_borrowed_get(inner, method),
        ResolvedType::Dictionary(_, _) | ResolvedType::SortedDictionary(_, _) => method == "get",
        _ => false,
    }
}

fn with_expression_temporaries(
    scopes: &PhpNameScopes,
    emit: impl FnOnce(&PhpNameScopes) -> String,
) -> String {
    let mut nested = scopes.clone();
    let temporaries = Rc::new(RefCell::new(Vec::new()));
    nested.expression_temporaries = Some(Rc::clone(&temporaries));
    let expression = emit(&nested);
    let temporaries = temporaries.borrow();
    if temporaries.is_empty() {
        return expression;
    }
    let captures = scopes
        .captured_php_names()
        .into_iter()
        .filter(|name| name != "this")
        .map(|name| format!("&${name}"))
        .collect::<Vec<_>>();
    let captures = if captures.is_empty() {
        String::new()
    } else {
        format!(" use ({})", captures.join(", "))
    };
    let cleanup = temporaries
        .iter()
        .rev()
        .map(|name| format!("if (isset(${name})) {{ __doria_drop_value(${name}); }}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("(function(){captures} {{ try {{ return {expression}; }} finally {{ {cleanup} }} }})()")
}

fn emit_mixed_box_plan(expr: &Expr, emitted: String, scopes: &PhpNameScopes) -> String {
    let Some(plan) = scopes.mixed_box_plans.get(&expr.span()) else {
        return emitted;
    };
    let helper = if plan.nullable_target {
        "__doria_box_nullable_mixed"
    } else {
        "__doria_box_mixed"
    };
    format!(
        "{helper}({}, {emitted})",
        emit_php_string_literal(&php_mixed_type_tag(&plan.source_type))
    )
}

fn emit_closure_expression(closure: &ClosureExpression, scopes: &PhpNameScopes) -> String {
    let descriptor = scopes
        .closure_plan
        .descriptor(closure.closure_id, scopes.closure_owner);
    let Some(environment_name) = &descriptor.environment_name else {
        return format!("new {}()", descriptor.carrier_name);
    };
    let ownership = scopes
        .closure_plan
        .ownership
        .get(&closure.closure_id)
        .expect("checked closure must have an ownership plan");
    let layout = scopes.closure_plan.layout(
        descriptor
            .environment_layout
            .expect("capturing closure must have an environment layout"),
    );
    let mut fields = layout.fields.iter().collect::<Vec<_>>();
    fields.sort_by_key(|field| field.logical_index);
    let captured = fields
        .into_iter()
        .map(|field| {
            let acquisition = ownership
                .acquisitions
                .iter()
                .find(|acquisition| acquisition.environment_binding_id == field.environment_binding)
                .expect("validated environment field must have an acquisition");
            let source = scopes.place(acquisition.source_binding_id);
            let read = source.map(PhpBindingPlace::read).unwrap_or_else(|| {
                let declaration = scopes
                    .closure_plan
                    .binding_resolution
                    .declarations_by_id
                    .get(&acquisition.source_binding_id)
                    .expect("closure source binding must exist");
                if declaration.name == "this" {
                    "$this".to_string()
                } else {
                    format!("${}", scopes.php_name(&declaration.name))
                }
            });
            match acquisition.kind {
                crate::ownership::CaptureAcquisitionKind::ReadonlyLease
                | crate::ownership::CaptureAcquisitionKind::WritableLease => source
                    .and_then(PhpBindingPlace::cell)
                    .unwrap_or_else(|| format!("new __DoriaCell({read})")),
                crate::ownership::CaptureAcquisitionKind::CopyIntoEnvironment => {
                    format!("new __DoriaCell({read})")
                }
                crate::ownership::CaptureAcquisitionKind::MoveIntoEnvironment => {
                    let cell = source
                        .and_then(PhpBindingPlace::cell)
                        .expect("move capture source must use a compiler-owned PHP cell");
                    format!("new __DoriaCell(__doria_take_cell({cell}))")
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "new {}(new {environment_name}({captured}))",
        descriptor.carrier_name
    )
}

fn emit_callable_call(call: &CallableCall, scopes: &PhpNameScopes) -> String {
    let call_info = scopes
        .closure_plan
        .callable_value_calls
        .get(&call.span)
        .expect("checked callable call must have semantic metadata");
    let function_type = match &call_info.function_type {
        ResolvedType::Function(function_type) => Some(function_type),
        ResolvedType::Nullable(inner) => match inner.as_ref() {
            ResolvedType::Function(function_type) => Some(function_type),
            _ => None,
        },
        _ => None,
    }
    .expect("checked callable call must have a structural function type");
    let arguments = call
        .args
        .iter()
        .zip(&function_type.parameters)
        .map(|(argument, parameter)| match parameter.ownership_mode {
            crate::types::FunctionTypeParameterMode::Writable => {
                assignment_target_cell(&argument.value, scopes)
                    .unwrap_or_else(|| emit_expr(&argument.value, scopes))
            }
            crate::types::FunctionTypeParameterMode::Take => {
                emit_owned_expr(&argument.value, scopes)
            }
            _ => emit_expr(&argument.value, scopes),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let callee = if call_info.invocation_mode == crate::types::FunctionInvocationMode::Once {
        assignment_target_cell(&call.callee, scopes)
            .map(|cell| format!("__doria_take_cell({cell})"))
            .unwrap_or_else(|| emit_expr(&call.callee, scopes))
    } else {
        emit_expr(&call.callee, scopes)
    };
    format!("({callee})({arguments})")
}

fn emit_list_algorithm_call(call: &ListAlgorithmCall, scopes: &PhpNameScopes) -> String {
    let source = emit_expr(&call.receiver, scopes);
    let callback_index = usize::from(call.kind == ListAlgorithmKind::Reduce);
    let callback = emit_expr(&call.arguments[callback_index].value, scopes);
    let mut body = if call.kind == ListAlgorithmKind::Reduce {
        "(function ($__doriaAlgorithmSource, $__doriaAlgorithmInitial, $__doriaAlgorithmCallback) { ".to_string()
    } else {
        "(function ($__doriaAlgorithmSource, $__doriaAlgorithmCallback) { ".to_string()
    };
    match call.kind {
        ListAlgorithmKind::Map | ListAlgorithmKind::Filter => {
            body.push_str("$__doriaAlgorithmResult = []; try { foreach ($__doriaAlgorithmSource as $__doriaAlgorithmElement) { ");
            if call.kind == ListAlgorithmKind::Map {
                body.push_str("$__doriaAlgorithmValue = ($__doriaAlgorithmCallback)($__doriaAlgorithmElement); $__doriaAlgorithmResult[] = $__doriaAlgorithmValue; ");
            } else {
                let clone = scopes.specialization.core_operation(
                    call.span,
                    crate::compiler_known_contracts::CoreValueOperation::Clone,
                    &call.element_type,
                    &scopes.substitutions,
                );
                let value = if clone {
                    "($__doriaAlgorithmElement === null ? null : $__doriaAlgorithmElement->clone())"
                } else {
                    "$__doriaAlgorithmElement"
                };
                body.push_str(&format!("if (($__doriaAlgorithmCallback)($__doriaAlgorithmElement)) {{ $__doriaAlgorithmResult[] = {value}; }} "));
            }
            body.push_str("} } catch (__DoriaCheckedError $__doriaAlgorithmError) { __doria_drop_value($__doriaAlgorithmResult); throw $__doriaAlgorithmError; } return $__doriaAlgorithmResult; ");
        }
        ListAlgorithmKind::Reduce => {
            body.push_str("$__doriaAlgorithmAccumulator = new __DoriaCell($__doriaAlgorithmInitial); try { foreach ($__doriaAlgorithmSource as $__doriaAlgorithmElement) { ($__doriaAlgorithmCallback)($__doriaAlgorithmAccumulator, $__doriaAlgorithmElement); } } catch (__DoriaCheckedError $__doriaAlgorithmError) { __doria_drop_cell($__doriaAlgorithmAccumulator); throw $__doriaAlgorithmError; } return __doria_take_cell($__doriaAlgorithmAccumulator); ");
        }
    }
    body.push_str("})(");
    body.push_str(&source);
    body.push_str(", ");
    if call.kind == ListAlgorithmKind::Reduce {
        body.push_str(&emit_owned_expr(&call.arguments[0].value, scopes));
        body.push_str(", ");
    }
    body.push_str(&callback);
    body.push(')');
    body
}

fn emit_core_equality(left: &str, right: &str, method: &str) -> String {
    format!("(static function ($left, $right) {{ return $left === null || $right === null ? $left === $right : $left->{method}($right); }})({left}, {right})")
}

fn php_equality_callback(span: Span, ty: Option<&ResolvedType>, scopes: &PhpNameScopes) -> String {
    if ty.is_some_and(|ty| {
        scopes.specialization.core_operation(
            span,
            crate::compiler_known_contracts::CoreValueOperation::Equal,
            ty,
            &scopes.substitutions,
        )
    }) {
        "static function ($left, $right): bool { return $left === null || $right === null ? $left === $right : $left->equals($right); }".into()
    } else {
        "null".into()
    }
}

fn php_collection_search(ty: Option<&ResolvedType>, method: &str) -> bool {
    matches!(
        (ty, method),
        (
            Some(ResolvedType::List(_)),
            "contains" | "indexOf" | "remove"
        ) | (
            Some(
                ResolvedType::TypedArray(_)
                    | ResolvedType::Deque(_)
                    | ResolvedType::PriorityQueue(_)
            ),
            "contains"
        ) | (
            Some(ResolvedType::Dictionary(_, _) | ResolvedType::SortedDictionary(_, _)),
            "containsValue"
        )
    )
}

fn emit_expr_unboxed(expr: &Expr, scopes: &PhpNameScopes) -> String {
    if let Some(value) = interface::builtin_acquire(expr, scopes) {
        return value;
    }
    if let Some(value) = core_collection::expression(expr, scopes) {
        return value;
    }
    match expr {
        Expr::Assertion(_) => {
            unreachable!("checked assertions are emitted only from terminal statement position")
        }
        Expr::Closure(closure) => emit_closure_expression(closure, scopes),
        Expr::CallableCall(call) => emit_callable_call(call, scopes),
        Expr::ListAlgorithmCall(call) => emit_list_algorithm_call(call, scopes),
        Expr::Variable { name, span } => {
            let value = scopes
                .place_for_use(*span)
                .map(PhpBindingPlace::read)
                .unwrap_or_else(|| format!("${}", scopes.php_name(name)));
            if scopes.is_mixed_binding(name)
                && scopes
                    .expression_types
                    .get(span)
                    .is_some_and(|ty| !is_mixed_storage_type(ty))
            {
                format!("__doria_mixed_value({value})")
            } else {
                value
            }
        }
        Expr::This { span } => scopes
            .place_for_use(*span)
            .map(PhpBindingPlace::read)
            .unwrap_or_else(|| "$this".to_string()),
        Expr::Identifier { name, .. } if scopes.is_payload_top_constant(name) => {
            format!("__doria_const_{}()", php_symbol_name(name))
        }
        Expr::Identifier { name, .. } => php_top_level_constant_name(name),
        Expr::String { value, .. } => emit_php_string_literal(value),
        Expr::InterpolatedString { parts, .. } => emit_interpolated_string(parts, scopes),
        Expr::Int { value, span } if is_uint64(scopes.expression_types.get(span)) => {
            let value = parse_decimal_magnitude(value).expect("checked uint64 literal") as u64;
            emit_integer_bits(value as i64)
        }
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
                            emit_owned_expr(&element.value, scopes)
                        )
                    } else {
                        emit_owned_expr(&element.value, scopes)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        Expr::ArrayRepeat { value, count, .. } => {
            let duplicate = if scopes
                .expression_types
                .get(&value.span())
                .is_some_and(|ty| {
                    scopes.specialization.core_operation(
                        value.span(),
                        crate::compiler_known_contracts::CoreValueOperation::Clone,
                        ty,
                        &scopes.substitutions,
                    )
                }) {
                "($source === null ? null : $source->clone())"
            } else {
                "$source"
            };
            format!("(static function ($source, int $count) {{ if ($count < 0) {{ __doria_panic(\"P1311\", {}, {}, null, {}, [\"count\" => $count]); }} $result = []; try {{ for ($index = 0; $index < $count; ++$index) {{ $result[] = {duplicate}; }} }} catch (__DoriaCheckedError $error) {{ __doria_drop_value($result); throw $error; }} return $result; }})({}, {})",
                count.span().start, count.span().end, scopes.callable_identity(), emit_expr(value, scopes), emit_expr(count, scopes))
        }
        Expr::Index {
            collection,
            index,
            span,
        } => format!(
            "__doria_collection_index({}, {}, {}, {}, {}, {})",
            emit_expr(collection, scopes),
            emit_expr(index, scopes),
            matches!(
                scopes.expression_types.get(&collection.span()),
                Some(ResolvedType::Dictionary(_, _))
            ),
            php_source_location(*span, span.start),
            php_source_location(*span, span.end),
            scopes.callable_identity()
        ),
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            ..
        } if scopes
            .expression_types
            .get(&object.span())
            .is_some_and(|ty| php_sequence_property(ty, property)) =>
        {
            let receiver = emit_expr(object, scopes);
            let count = format!(
                "count({})",
                if *null_safe { "$sequence" } else { &receiver }
            );
            let value = if property == "isEmpty" {
                format!("({count} === 0)")
            } else {
                count
            };
            if *null_safe {
                format!("(static fn($sequence) => $sequence === null ? null : {value})({receiver})")
            } else {
                value
            }
        }
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            ..
        } => {
            let receiver = emit_member_receiver(object, scopes);
            let operator = if *null_safe { "?->" } else { "->" };
            if scopes
                .expression_types
                .get(&object.span())
                .and_then(shared::kind)
                .is_some()
            {
                if property == "referencedValue"
                    && scopes
                        .expression_types
                        .get(&object.span())
                        .and_then(shared::kind)
                        == Some(crate::types::SharedHandleKind::SharedReference)
                {
                    format!("{receiver}{operator}payload()")
                } else {
                    format!("{receiver}{operator}payload()->{property}")
                }
            } else {
                format!("{receiver}{operator}{property}")
            }
        }
        Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
            ..
        } if php_collection_search(scopes.expression_types.get(&object.span()), method) => {
            let receiver_type = scopes.expression_types.get(&object.span()).unwrap();
            let compared = match receiver_type {
                ResolvedType::List(ty)
                | ResolvedType::TypedArray(ty)
                | ResolvedType::Deque(ty)
                | ResolvedType::PriorityQueue(ty)
                | ResolvedType::Dictionary(_, ty)
                | ResolvedType::SortedDictionary(_, ty) => ty.as_ref(),
                _ => unreachable!("checked collection search receiver"),
            };
            let callback = php_equality_callback(*span, Some(compared), scopes);
            let object = emit_expr(object, scopes);
            let value = emit_expr(&args[0].value, scopes);
            match method.as_str() {
                "remove" => format!("__doria_list_remove({object}, {value}, {callback})"),
                "indexOf" => format!("__doria_collection_index_of({object}, {value}, {callback})"),
                _ => {
                    format!("(__doria_collection_index_of({object}, {value}, {callback}) !== null)")
                }
            }
        }
        Expr::MethodCall {
            object,
            method,
            args,
            null_safe: false,
            ..
        } if matches!(
            scopes.expression_types.get(&object.span()),
            Some(ResolvedType::List(_))
        ) && method == "add" =>
        {
            let value = args
                .first()
                .map(|argument| emit_owned_expr(&argument.value, scopes))
                .expect("semantic checking requires one List::add argument");
            format!("({}[] = {value})", emit_expr(object, scopes))
        }
        Expr::MethodCall {
            object,
            method,
            args,
            null_safe: false,
            ..
        } if matches!(
            scopes.expression_types.get(&object.span()),
            Some(ResolvedType::Dictionary(_, _))
        ) && method == "get" =>
        {
            format!(
                "__doria_dictionary_get({}, {})",
                emit_expr(object, scopes),
                emit_expr(&args[0].value, scopes)
            )
        }
        Expr::MethodCall {
            object,
            method,
            args,
            null_safe,
            span,
            ..
        } => shared::emit_method(object, method, args, *null_safe, *span, scopes).unwrap_or_else(
            || {
                format!(
                    "{}{}{}({})",
                    emit_member_receiver(object, scopes),
                    if *null_safe { "?->" } else { "->" },
                    scopes
                        .specialization
                        .call_symbol(*span, &scopes.substitutions)
                        .unwrap_or(method),
                    emit_arguments_for_call(args, *span, scopes)
                )
            },
        ),
        Expr::FunctionCall { name, args, span } => emit_function_call(name, args, *span, scopes),
        Expr::StaticCall {
            class_name,
            method,
            args,
            span,
            ..
        } => {
            if matches!(class_name.as_str(), "Deque" | "SortedDictionary")
                && method == "from"
                && args.len() == 1
            {
                let ty = scopes.expression_types.get(&expr.span());
                let element = match ty {
                    Some(
                        ResolvedType::Deque(element) | ResolvedType::SortedDictionary(_, element),
                    ) => Some(element.as_ref()),
                    _ => None,
                };
                if element.is_some_and(|ty| {
                    scopes.specialization.core_operation(
                        args[0].value.span(),
                        crate::compiler_known_contracts::CoreValueOperation::Clone,
                        ty,
                        &scopes.substitutions,
                    )
                }) {
                    return format!("{class_name}::from({}, {}static fn($value) => $value === null ? null : $value->clone())",
                        emit_expr(&args[0].value, scopes),
                        if class_name == "SortedDictionary" { format!("{}, ", php_unsigned_collection(expr, scopes)) } else { String::new() });
                }
            }
            if class_name == "Bytes" && method == "fromArray" && args.len() == 1 {
                return format!("array_values({})", emit_expr(&args[0].value, scopes));
            }
            if class_name == "Set" && method == "from" && args.len() == 1 {
                return format!(
                    "__doria_primitive_set_from({})",
                    emit_expr(&args[0].value, scopes)
                );
            }
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
                        return format!(
                            "SortedDictionary::fromPairs([{pairs}], {})",
                            php_unsigned_collection(expr, scopes)
                        );
                    }
                }
            }
            if matches!(
                class_name.as_str(),
                "SortedDictionary" | "SortedSet" | "PriorityQueue"
            ) && method == "from"
                && args.len() == 1
            {
                return format!(
                    "{class_name}::from({}, {})",
                    emit_expr(&args[0].value, scopes),
                    php_unsigned_collection(expr, scopes)
                );
            }
            let qualifier = if scopes.direct_parent_calls.contains(span) {
                "parent".to_string()
            } else {
                scopes
                    .specialization
                    .class_symbol_for_call(*span, &scopes.substitutions)
                    .map(str::to_string)
                    .unwrap_or_else(|| php_symbol_name(class_name))
            };
            let method = scopes
                .specialization
                .call_symbol(*span, &scopes.substitutions)
                .unwrap_or(method);
            format!(
                "{}::{method}({})",
                qualifier,
                if scopes.is_payload_enum_expression(expr) {
                    args.iter()
                        .map(|argument| {
                            let value = emit_owned_expr(&argument.value, scopes);
                            match &argument.name {
                                Some(name) => format!("{}: {value}", name.text),
                                None => value,
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    emit_arguments_for_call(args, *span, scopes)
                }
            )
        }
        Expr::StaticMember {
            class_name, member, ..
        } if scopes.is_static_property(class_name, member) => {
            format!("{}::${member}", php_symbol_name(class_name))
        }
        Expr::StaticMember {
            class_name, member, ..
        } if scopes.is_payload_unit_case(class_name, member) => {
            format!(
                "{}::{}()",
                php_symbol_name(class_name),
                php_payload_case_method(member, false)
            )
        }
        Expr::StaticMember {
            class_name, member, ..
        } if scopes.is_payload_class_constant(class_name, member) => {
            format!("{}::__doriaConst{member}()", php_symbol_name(class_name))
        }
        Expr::StaticMember {
            class_name, member, ..
        } => format!("{}::{member}", php_symbol_name(class_name)),
        Expr::New {
            class_type,
            args,
            span,
            shared,
        } => {
            let class_symbol = scopes
                .specialization
                .class_symbol_for_call(*span, &scopes.substitutions)
                .or_else(|| {
                    scopes
                        .specialization
                        .class_symbol_for_type(class_type, &scopes.substitutions)
                })
                .map(str::to_string)
                .unwrap_or_else(|| php_symbol_name(&class_type.name));
            if *shared {
                format!(
                    "new __DoriaSharedHandle(new __DoriaSharedControl(new {}({})), 0)",
                    class_symbol,
                    emit_arguments_for_call(args, *span, scopes)
                )
            } else if crate::types::SharedHandleKind::from_source_name(&class_type.name)
                == Some(crate::types::SharedHandleKind::WritableSharedReference)
            {
                format!(
                    "new __DoriaSharedHandle(new __DoriaSharedControl({}), 2)",
                    emit_owned_expr(&args[0].value, scopes)
                )
            } else {
                format!(
                    "new {}({})",
                    class_symbol,
                    emit_arguments_for_call(args, *span, scopes)
                )
            }
        }
        Expr::Grouped { expr, .. } => format!("({})", emit_expr(expr, scopes)),
        Expr::IsType { expr, ty: _, span } => {
            let value = emit_expr(expr, scopes);
            let source_type = scopes
                .expression_types
                .get(&expr.span())
                .expect("checked type test must preserve its source type");
            let exact_type = scopes
                .type_test_types
                .get(span)
                .expect("checked type test must preserve its resolved exact type");
            let test = php_exact_type_test_for_source(&value, source_type, exact_type, scopes);
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
            left,
            op,
            right,
            span,
        } => match op {
            BinaryOp::Equal | BinaryOp::NotEqual
                if scopes.expression_types.get(&left.span()).is_some_and(|ty| {
                    scopes.specialization.core_operation(
                        *span,
                        crate::compiler_known_contracts::CoreValueOperation::Equal,
                        ty,
                        &scopes.substitutions,
                    )
                }) && matches!(
                    scopes
                        .expression_types
                        .get(&left.span())
                        .map(|ty| match ty {
                            ResolvedType::Nullable(inner) => inner.as_ref(),
                            other => other,
                        }),
                    Some(ResolvedType::Class(_) | ResolvedType::Interface(_))
                ) =>
            {
                let method = scopes
                    .specialization
                    .call_symbol(*span, &scopes.substitutions)
                    .unwrap_or("equals");
                let negate = if *op == BinaryOp::NotEqual { "!" } else { "" };
                format!(
                    "{negate}{}",
                    emit_core_equality(&emit_expr(left, scopes), &emit_expr(right, scopes), method)
                )
            }
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
                "{} . {}",
                emit_display_expr(left, scopes),
                emit_display_expr(right, scopes)
            ),
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual
                if is_uint64(scopes.expression_types.get(&left.span())) =>
            {
                emit_uint64_comparison(
                    &emit_expr(left, scopes),
                    emit_binary_op(op),
                    &emit_expr(right, scopes),
                )
            }
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
        } => emit_match_expression(scrutinee, arms, *span, false, scopes),
        Expr::When(when) => emit_when_expression(
            when.given.as_ref(),
            &when.branches,
            when.finally.as_ref(),
            when.span,
            false,
            scopes,
        ),
    }
}

fn emit_when_expression(
    given: Option<&GivenPrelude>,
    branches: &[WhenBranch],
    finally: Option<&ControlFlowFinally>,
    span: Span,
    owned_result: bool,
    scopes: &PhpNameScopes,
) -> String {
    scopes
        .whens
        .get(&span)
        .expect("checked when expression must have a semantic plan");
    let mut when_scopes = scopes.clone();
    when_scopes.pending_returns.clear();
    when_scopes.expression_temporaries = None;
    when_scopes.yield_temporaries = if owned_result {
        None
    } else {
        scopes.expression_temporaries.clone()
    };
    when_scopes.returns_borrow = !owned_result;
    when_scopes.push();
    let mut body = String::new();
    emit_owned_scope(
        &mut body,
        1,
        &mut when_scopes,
        |body, indent, when_scopes| {
            if let Some(finally) = finally {
                emit_finalizer_error_boundary(
                    body,
                    indent,
                    when_scopes,
                    |body, indent, when_scopes| {
                        writeln(body, indent, "try");
                        writeln(body, indent, "{");
                        emit_when_branches(given, branches, body, indent + 1, when_scopes);
                        writeln(body, indent, "}");
                        writeln(body, indent, "finally");
                        emit_block(&finally.block, body, indent, when_scopes);
                    },
                );
            } else {
                emit_when_branches(given, branches, body, indent, when_scopes);
            }
        },
    );
    when_scopes.pop();
    let capture_list = expression_capture_list(scopes);
    format!("(function(){capture_list} {{\n{body}}})()")
}

fn expression_capture_list(scopes: &PhpNameScopes) -> String {
    let mut captures = scopes.captured_php_names();
    if let Some(temporaries) = &scopes.expression_temporaries {
        for temporary in temporaries.borrow().iter() {
            if !captures.contains(temporary) {
                captures.push(temporary.clone());
            }
        }
    }
    captures.retain(|name| name != "this");
    if captures.is_empty() {
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
    }
}

fn emit_when_branches(
    given: Option<&GivenPrelude>,
    branches: &[WhenBranch],
    body: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let predicates = given
        .map(|given| emit_given_setup(given, body, indent, scopes))
        .unwrap_or_default();
    let gate = if predicates.is_empty() {
        None
    } else {
        let gate = scopes.fresh_temp("__doria_given_gate");
        writeln(
            body,
            indent,
            &format!(
                "${gate} = {};",
                emit_bool_chain(predicates.iter().map(String::as_str))
            ),
        );
        Some(format!("${gate}"))
    };

    for (index, branch) in branches.iter().enumerate() {
        write_indent(body, indent);
        if let Some(condition) = &branch.condition {
            body.push_str(if index == 0 { "if (" } else { "else if (" });
            if let Some(gate) = &gate {
                body.push_str(gate);
                body.push_str(" && ");
            }
            body.push_str(&emit_expr(condition, scopes));
            body.push_str(")\n");
        } else {
            body.push_str("else\n");
        }
        emit_block(&branch.block, body, indent, scopes);
    }
}

fn emit_match_expression(
    scrutinee: &Expr,
    arms: &[MatchArm],
    span: Span,
    owned_result: bool,
    scopes: &PhpNameScopes,
) -> String {
    let info = scopes
        .matches
        .get(&span)
        .expect("checked match must have a semantic plan");
    let temporary = scopes.expression_temp("__doriaMatch", span);
    let consumed = matches!(info.mode, crate::ast::MatchMode::Consumed { .. });
    let drop_scrutinee = consumed && resolved_type_needs_php_drop(&info.scrutinee_type, scopes);
    let mut output = String::new();
    if drop_scrutinee {
        output.push_str("try { ");
    }

    for (index, (arm, arm_info)) in arms.iter().zip(&info.arms).enumerate() {
        let mut arm_scopes = scopes.clone();
        arm_scopes.pending_returns.clear();
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
                arm_info,
                &info.scrutinee_type,
                &mut arm_scopes,
                false,
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
            arm_info,
            &info.scrutinee_type,
            &mut arm_scopes,
            arm.guard.is_some(),
            consumed,
        );
        emit_owned_scope(&mut output, 0, &mut arm_scopes, |output, indent, scopes| {
            let value = if owned_result {
                emit_owned_expr(&arm.value, scopes)
            } else {
                emit_expr(&arm.value, scopes)
            };
            writeln(output, indent, &format!("return {value};"));
        });
        if arm.guard.is_some() {
            output.push_str("} ");
        }
        if condition.is_some() {
            output.push_str("} ");
        }
    }
    if drop_scrutinee {
        output.push_str(&format!(
            "}} finally {{ __doria_drop_value(${temporary}); }} "
        ));
    }
    let capture_list = expression_capture_list(scopes);
    output.insert_str(0, &format!("(function(${temporary}){capture_list} {{ "));
    output.push_str("})(");
    output.push_str(&if consumed {
        emit_owned_expr(scrutinee, scopes)
    } else {
        emit_expr(scrutinee, scopes)
    });
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
        ResolvedMatchPattern::ExactType(ty) => Some(php_exact_type_test_for_source(
            &value,
            scrutinee_type,
            ty,
            scopes,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_php_match_bindings(
    output: &mut String,
    temporary: &str,
    pattern: &MatchPattern,
    info: &crate::semantics::MatchArmSemanticInfo,
    scrutinee_type: &ResolvedType,
    arm_scopes: &mut PhpNameScopes,
    reuse: bool,
    consumed: bool,
) {
    let owns = |name: &str| {
        consumed
            && info
                .bindings
                .iter()
                .find(|binding| binding.name == name)
                .is_some_and(|binding| {
                    !binding.borrowed && resolved_type_needs_php_drop(&binding.ty, arm_scopes)
                })
    };
    let ownership = info
        .bindings
        .iter()
        .map(|binding| (binding.name.clone(), owns(&binding.name)))
        .collect::<HashMap<_, _>>();
    match (pattern, &info.pattern) {
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
                let owned = ownership.get(&binding.name).copied().unwrap_or(false);
                let method = if owned {
                    "__doriaTakePayload"
                } else {
                    "__doriaPayloadAt"
                };
                emit_php_match_binding(
                    output,
                    &name,
                    binding,
                    format!("${temporary}->{method}({index})"),
                    owned,
                    arm_scopes,
                );
            }
        }
        (MatchPattern::TypeBinding { binding, .. }, ResolvedMatchPattern::ExactType(_)) => {
            let name = if reuse {
                arm_scopes.declare_or_reuse_current(&binding.name)
            } else {
                arm_scopes.declare(&binding.name)
            };
            let value = if is_mixed_storage_type(scrutinee_type) {
                format!("__doria_mixed_value(${temporary})")
            } else {
                format!("${temporary}")
            };
            let owned = ownership.get(&binding.name).copied().unwrap_or(false);
            emit_php_match_binding(output, &name, binding, value, owned, arm_scopes);
            if owned {
                output.push_str(&format!("${temporary} = null; "));
            }
        }
        _ => {}
    }
}

fn emit_php_match_binding(
    output: &mut String,
    name: &str,
    binding: &crate::hir::MatchBinding,
    value: String,
    owned: bool,
    scopes: &mut PhpNameScopes,
) {
    let place = if owned {
        output.push_str(&format!("${name} = new __DoriaCell({value}); "));
        scopes.own_cell(name.to_string());
        PhpBindingPlace::Cell(name.to_string())
    } else {
        output.push_str(&format!("${name} = {value}; "));
        PhpBindingPlace::Direct(name.to_string())
    };
    if let Some(id) = scopes.binding_for_declaration(&binding.name, binding.span) {
        scopes.bind_place(id, place);
    }
}

fn php_exact_type_test_for_source(
    value: &str,
    source: &ResolvedType,
    exact: &ResolvedType,
    scopes: &PhpNameScopes,
) -> String {
    if let Some(target) = interface::nominal_type_name(exact, scopes) {
        let value = if is_mixed_storage_type(source) {
            format!("__doria_mixed_value({value})")
        } else {
            value.to_string()
        };
        return format!("{value} instanceof {target}");
    }
    match source {
        ResolvedType::Mixed => format!(
            "__doria_mixed_is({value}, {})",
            emit_php_string_literal(&php_mixed_type_tag(exact))
        ),
        ResolvedType::Nullable(inner) if matches!(inner.as_ref(), ResolvedType::Mixed) => {
            if matches!(exact, ResolvedType::Null) {
                format!("{value} === null")
            } else {
                format!(
                    "{value} !== null && __doria_mixed_is({value}, {})",
                    emit_php_string_literal(&php_mixed_type_tag(exact))
                )
            }
        }
        ResolvedType::Nullable(inner) if inner.as_ref() == exact => format!("{value} !== null"),
        ResolvedType::Nullable(_) => "false".to_string(),
        _ => (source == exact).to_string(),
    }
}

fn is_mixed_storage_type(ty: &ResolvedType) -> bool {
    matches!(ty, ResolvedType::Mixed)
        || matches!(ty, ResolvedType::Nullable(inner) if matches!(inner.as_ref(), ResolvedType::Mixed))
}

fn php_mixed_type_tag(ty: &ResolvedType) -> String {
    match ty {
        ResolvedType::Integer(integer) => integer.source_name().to_string(),
        ResolvedType::Float(float) => float.source_name().to_string(),
        ResolvedType::String => "string".to_string(),
        ResolvedType::Bool => "bool".to_string(),
        ResolvedType::Null => "null".to_string(),
        ResolvedType::Error => "error".to_string(),
        ResolvedType::Function(_) => resolved_type_identity(ty),
        ResolvedType::Enum(ty) => format!("enum:{}", ty.name),
        ResolvedType::Class(ty) => format!("class:{}", ty.name),
        ResolvedType::Interface(ty) => format!("interface:{}", interface::specialization_name(ty)),
        ResolvedType::Nullable(inner) => php_mixed_type_tag(inner),
        _ => unreachable!("only exact Doria runtime values cross a PHP mixed boundary"),
    }
}

fn resolved_type_identity(ty: &ResolvedType) -> String {
    fn list(values: impl IntoIterator<Item = String>) -> String {
        values
            .into_iter()
            .map(|value| format!("{}:{value}", value.len()))
            .collect::<Vec<_>>()
            .join("")
    }

    match ty {
        ResolvedType::Void => "void".to_string(),
        ResolvedType::Integer(ty) => ty.source_name().to_string(),
        ResolvedType::Float(ty) => ty.source_name().to_string(),
        ResolvedType::String => "string".to_string(),
        ResolvedType::Bytes => "Bytes".to_string(),
        ResolvedType::Bool => "bool".to_string(),
        ResolvedType::Null => "null".to_string(),
        ResolvedType::Mixed => "mixed".to_string(),
        ResolvedType::Error => "Error".to_string(),
        ResolvedType::TypeParameter(name) => format!("type:{}:{name}", name.len()),
        ResolvedType::Interface(ty) => format!(
            "interface:{}:{}[{}]",
            ty.name.len(),
            ty.name,
            list(ty.arguments.iter().map(resolved_type_identity))
        ),
        ResolvedType::InterfaceSelf(name) => format!("self:{}:{name}", name.len()),
        ResolvedType::TraitSelf(_) => unreachable!("trait execution is pending Stage 35 Slice 4"),
        ResolvedType::Function(function) => {
            let invocation = match function.invocation_mode {
                crate::types::FunctionInvocationMode::Readonly => "readonly",
                crate::types::FunctionInvocationMode::Writable => "writable",
                crate::types::FunctionInvocationMode::Once => "once",
            };
            let parameters = list(function.parameters.iter().map(|parameter| {
                let ownership = match parameter.ownership_mode {
                    crate::types::FunctionTypeParameterMode::Readonly => "readonly",
                    crate::types::FunctionTypeParameterMode::Writable => "writable",
                    crate::types::FunctionTypeParameterMode::Take => "take",
                };
                format!("{ownership}:{}", resolved_type_identity(&parameter.ty))
            }));
            let effects = list(function.checked_effects.iter().map(resolved_type_identity));
            let return_borrow = function.return_borrow.map_or_else(
                || "owned".to_string(),
                |borrow| {
                    let crate::types::FunctionBorrowSource::Parameter(index) = borrow.source;
                    format!(
                        "{}-parameter-{index}",
                        if borrow.writable {
                            "writable"
                        } else {
                            "readonly"
                        }
                    )
                },
            );
            format!(
                "function:{invocation}:params[{parameters}]:return[{}]:effects[{effects}]:borrow[{return_borrow}]",
                resolved_type_identity(&function.return_type)
            )
        }
        ResolvedType::Enum(ty) => format!("enum:{}:{}", ty.name.len(), ty.name),
        ResolvedType::Nullable(inner) => format!("nullable[{}]", resolved_type_identity(inner)),
        ResolvedType::Class(ty) => format!(
            "class:{}:{}[{}]",
            ty.name.len(),
            ty.name,
            list(ty.arguments.iter().map(resolved_type_identity))
        ),
        ResolvedType::TypedArray(element) => {
            format!("array[{}]", resolved_type_identity(element))
        }
        ResolvedType::List(element) => format!("List[{}]", resolved_type_identity(element)),
        ResolvedType::Dictionary(key, value) => format!(
            "Dictionary[{}][{}]",
            resolved_type_identity(key),
            resolved_type_identity(value)
        ),
        ResolvedType::SortedDictionary(key, value) => format!(
            "SortedDictionary[{}][{}]",
            resolved_type_identity(key),
            resolved_type_identity(value)
        ),
        ResolvedType::Set(element) => format!("Set[{}]", resolved_type_identity(element)),
        ResolvedType::SortedSet(element) => {
            format!("SortedSet[{}]", resolved_type_identity(element))
        }
        ResolvedType::PriorityQueue(element) => {
            format!("PriorityQueue[{}]", resolved_type_identity(element))
        }
        ResolvedType::Deque(element) => format!("Deque[{}]", resolved_type_identity(element)),
        ResolvedType::SharedHandle(kind, payload) => format!(
            "{}[{}]",
            kind.source_name(),
            resolved_type_identity(payload)
        ),
        ResolvedType::Unsupported => "unsupported".to_string(),
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
                emitted.push(emit_display_expr(expr, scopes));
            }
        }
    }

    match emitted.len() {
        0 => emit_php_string_literal(""),
        1 if has_expr => format!("{} . {}", emit_php_string_literal(""), emitted[0]),
        _ => emitted.join(" . "),
    }
}

fn emit_checked_io_message_vocabulary(output: &mut String) {
    use doria_diagnostic_catalogue::{
        IoMessageOperation, IoMessageReason, INVALID_UTF8_MESSAGE_PREFIX, IO_ERROR_MESSAGE_PREFIX,
        IO_ERROR_MESSAGE_SEPARATOR, IO_FILE_MESSAGE_PREFIX, IO_FILE_MESSAGE_SUFFIX,
        IO_STANDARD_ERROR_NAME, IO_STANDARD_INPUT_NAME, IO_STANDARD_OUTPUT_NAME,
    };

    output.push_str("$__doria_io_message_vocabulary = [\n");
    for (key, value) in [
        ("ioPrefix", IO_ERROR_MESSAGE_PREFIX),
        ("separator", IO_ERROR_MESSAGE_SEPARATOR),
        ("filePrefix", IO_FILE_MESSAGE_PREFIX),
        ("fileSuffix", IO_FILE_MESSAGE_SUFFIX),
        ("stdin", IO_STANDARD_INPUT_NAME),
        ("stdout", IO_STANDARD_OUTPUT_NAME),
        ("stderr", IO_STANDARD_ERROR_NAME),
        ("invalidUtf8Prefix", INVALID_UTF8_MESSAGE_PREFIX),
    ] {
        output.push_str(&format!(
            "    {} => {},\n",
            emit_php_string_literal(key),
            emit_php_string_literal(value),
        ));
    }
    output.push_str("    \"operations\" => [");
    output.push_str(
        &IoMessageOperation::ALL
            .iter()
            .map(|operation| emit_php_string_literal(operation.word()))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str("],\n    \"reasons\" => [");
    output.push_str(
        &IoMessageReason::ALL
            .iter()
            .map(|reason| emit_php_string_literal(reason.words()))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str("],\n];\n\n");
}

fn emit_php_string_literal(value: &str) -> String {
    format!("\"{}\"", escape_php_string(value))
}

fn is_uint64(ty: Option<&ResolvedType>) -> bool {
    matches!(ty, Some(ResolvedType::Integer(IntegerType::UInt64)))
}

fn php_unsigned_collection(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    match scopes.expression_types.get(&expr.span()) {
        Some(ResolvedType::SortedDictionary(key, _)) => is_uint64(Some(key)),
        Some(ResolvedType::SortedSet(value) | ResolvedType::PriorityQueue(value)) => {
            is_uint64(Some(value))
        }
        _ => false,
    }
}

fn emit_integer_bits(value: i64) -> String {
    if value == i64::MIN {
        "(-9223372036854775807 - 1)".to_string()
    } else {
        value.to_string()
    }
}

fn emit_uint64_comparison(left: &str, operator: &str, right: &str) -> String {
    format!("((({left}) ^ PHP_INT_MIN) {operator} (({right}) ^ PHP_INT_MIN))")
}

fn emit_primitive_core_call(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    use crate::compiler_known_contracts::CoreValueOperation;
    let Expr::MethodCall {
        object, args, span, ..
    } = expr
    else {
        return None;
    };
    let crate::semantics::CallableTarget::ConstrainedMethod {
        receiver,
        requirement,
        ..
    } = scopes.specialization.target(*span, &scopes.substitutions)?
    else {
        return None;
    };
    if !matches!(
        receiver,
        ResolvedType::Integer(_)
            | ResolvedType::Float(_)
            | ResolvedType::Bool
            | ResolvedType::String
    ) {
        return None;
    }
    let operation = CoreValueOperation::from_requirement(requirement)?;
    let left = emit_expr(object, scopes);
    Some(match operation {
        CoreValueOperation::Equal => format!("({left} === {})", emit_expr(&args[0].value, scopes)),
        CoreValueOperation::Hash => {
            if receiver == ResolvedType::String {
                format!("unpack('J', hash('fnv1a64', {left}, true))[1]")
            } else {
                format!("(int)({left})")
            }
        }
        CoreValueOperation::Compare => {
            let right = emit_expr(&args[0].value, scopes);
            let less = if receiver == ResolvedType::String {
                "strcmp($left, $right) < 0".to_string()
            } else if is_uint64(Some(&receiver)) {
                emit_uint64_comparison("$left", "<", "$right")
            } else {
                "$left < $right".to_string()
            };
            format!("(static function ($left, $right) {{ return $left === $right ? Ordering::Equal : ({less} ? Ordering::Less : Ordering::Greater); }})({left}, {right})")
        }
        CoreValueOperation::Clone => return None,
    })
}

fn emit_display_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    let value = emit_expr(expr, scopes);
    if is_uint64(scopes.expression_types.get(&expr.span())) {
        format!("sprintf(\"%u\", {value})")
    } else {
        format!("__doria_display({value})")
    }
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
        "read_line" => "__doria_read_line".to_string(),
        "read_file" => "__doria_read_file".to_string(),
        "write_file" => "__doria_write_file".to_string(),
        "append_file" => "__doria_append_file".to_string(),
        "write_stderr" => "__doria_write_stderr".to_string(),
        "sprintf" => "__doria_sprintf".to_string(),
        "printf" => "__doria_printf".to_string(),
        _ => scopes
            .specialization
            .call_symbol(span, &scopes.substitutions)
            .map(str::to_string)
            .unwrap_or_else(|| php_function_name(name)),
    };
    let mut emitted = emit_call_argument_values(args, span, scopes);
    if matches!(name, "sprintf" | "printf") {
        if let Some(Expr::String { value, span }) = args.first().map(|argument| &argument.value) {
            if let Ok(pieces) = format_string::parse(value, *span) {
                emitted[0] = emit_php_string_literal(&php_format_from_plan(&pieces, args, scopes));
                let conversions = pieces.iter().filter_map(|piece| match piece {
                    FormatPiece::Argument { spec, .. } => Some(spec.conversion),
                    FormatPiece::Literal(_) => None,
                });
                for ((argument, source), conversion) in emitted
                    .iter_mut()
                    .skip(1)
                    .zip(args.iter().skip(1))
                    .zip(conversions)
                {
                    if conversion == FormatConversion::Display {
                        *argument = if is_uint64(scopes.expression_types.get(&source.value.span()))
                        {
                            format!("sprintf(\"%u\", {argument})")
                        } else {
                            format!("__doria_display({argument})")
                        };
                    }
                }
            }
        }
    }
    if name == "printf" {
        emitted.insert(0, php_source_location(span, span.end).to_string());
        emitted.insert(0, php_source_location(span, span.start).to_string());
        emitted.insert(2, scopes.callable_identity());
    } else if matches!(
        name,
        "read_line" | "read_file" | "write_file" | "append_file" | "write_stderr"
    ) {
        if name == "read_line" && emitted.is_empty() {
            emitted.push("\"\"".to_string());
        }
        emitted.push(format!("start: {}", php_source_location(span, span.start)));
        emitted.push(format!("end: {}", php_source_location(span, span.end)));
        emitted.push(format!("callable: {}", scopes.callable_identity()));
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

fn php_source_location(span: Span, offset: usize) -> u64 {
    (u64::from(span.source.0) << 32)
        | (u64::try_from(offset).expect("source offsets fit in u64") & 0xffff_ffff)
}

fn php_format_from_plan(
    pieces: &[FormatPiece],
    args: &[Argument],
    scopes: &PhpNameScopes,
) -> String {
    let mut format = String::new();
    for piece in pieces {
        match piece {
            FormatPiece::Literal(value) => format.push_str(&value.replace('%', "%%")),
            FormatPiece::Argument { index, spec } => {
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
                    FormatConversion::Decimal
                        if args.get(*index as usize + 1).is_some_and(|argument| {
                            is_uint64(scopes.expression_types.get(&argument.value.span()))
                        }) =>
                    {
                        'u'
                    }
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

fn php_type(ty: &TypeRef, scopes: &PhpNameScopes) -> String {
    if let Some(resolved) = scopes
        .specialization
        .resolve_type(ty, &scopes.substitutions)
        .filter(core_collection::uses)
    {
        return php_resolved_type(&resolved, scopes);
    }
    if scopes.substitutions.contains_key(&ty.name) {
        let resolved =
            crate::types::resolved_type_ref_with_substitutions(ty, &scopes.substitutions)
                .expect("checked specialization has a concrete type");
        return php_resolved_type(&resolved, scopes);
    }
    if let Some(symbol) = scopes
        .specialization
        .class_symbol_for_type(ty, &scopes.substitutions)
    {
        return format!("{}{symbol}", if ty.nullable { "?" } else { "" });
    }
    let name = if ty.function.is_some() {
        "__DoriaFunctionValue".to_string()
    } else if IntegerType::from_source_name(&ty.name).is_some() {
        "int".to_string()
    } else if FloatType::from_source_name(&ty.name).is_some() {
        "float".to_string()
    } else if crate::types::SharedHandleKind::from_source_name(&ty.name).is_some() {
        "__DoriaSharedHandle".to_string()
    } else if scopes.symbols.interface_names.contains(&ty.name) {
        interface::declaration_name(&ty.name)
    } else {
        match ty.name.as_str() {
            "List" | "Dictionary" | "Set" | "[]" => "array".to_string(),
            "Error" => "__DoriaErrorValue".to_string(),
            name => php_symbol_name(name),
        }
    };
    if ty.nullable {
        format!("?{name}")
    } else {
        name
    }
}

fn php_resolved_type(ty: &ResolvedType, scopes: &PhpNameScopes) -> String {
    if core_collection::uses(ty) {
        return format!(
            "{}__DoriaCoreCollection",
            if matches!(ty, ResolvedType::Nullable(_)) {
                "?"
            } else {
                ""
            }
        );
    }
    match ty {
        ResolvedType::Interface(ty) => interface::declaration_name(&ty.name),
        ResolvedType::InterfaceSelf(name) => interface::declaration_name(name),
        ResolvedType::TraitSelf(_) => unreachable!("trait execution is pending Stage 35 Slice 4"),
        ResolvedType::Void => "void".to_string(),
        ResolvedType::Integer(_) => "int".to_string(),
        ResolvedType::Float(_) => "float".to_string(),
        ResolvedType::String => "string".to_string(),
        ResolvedType::Bool => "bool".to_string(),
        ResolvedType::Null => "null".to_string(),
        ResolvedType::Mixed | ResolvedType::Unsupported | ResolvedType::TypeParameter(_) => {
            "mixed".to_string()
        }
        ResolvedType::Error => "__DoriaErrorValue".to_string(),
        ResolvedType::Function(_) => "__DoriaFunctionValue".to_string(),
        ResolvedType::Enum(ty) => php_symbol_name(&ty.name),
        ResolvedType::Class(ty) => scopes
            .specialization
            .class_symbols
            .get(ty)
            .cloned()
            .unwrap_or_else(|| php_symbol_name(&ty.name)),
        ResolvedType::Nullable(inner) => {
            let inner = php_resolved_type(inner, scopes);
            if inner == "mixed" || inner == "null" {
                "mixed".to_string()
            } else {
                format!("?{inner}")
            }
        }
        ResolvedType::Bytes
        | ResolvedType::TypedArray(_)
        | ResolvedType::List(_)
        | ResolvedType::Dictionary(_, _)
        | ResolvedType::Set(_) => "array".to_string(),
        ResolvedType::SortedDictionary(_, _) => "__DoriaSortedDictionary".to_string(),
        ResolvedType::SortedSet(_) => "__DoriaSortedSet".to_string(),
        ResolvedType::PriorityQueue(_) => "__DoriaPriorityQueue".to_string(),
        ResolvedType::Deque(_) => "__DoriaDeque".to_string(),
        ResolvedType::SharedHandle(_, _) => "__DoriaSharedHandle".to_string(),
    }
}

fn php_symbol_name(name: &str) -> String {
    match name {
        crate::compiler_known_io::IO_OPERATION => "__DoriaStdIoIoOperation".to_string(),
        crate::compiler_known_io::IO_TARGET => "__DoriaStdIoIoTarget".to_string(),
        crate::compiler_known_io::IO_ERROR_REASON => "__DoriaStdIoIoErrorReason".to_string(),
        crate::compiler_known_io::UTF8_INPUT_SOURCE => "__DoriaStdIoUtf8InputSource".to_string(),
        crate::compiler_known_io::IO_ERROR => "__DoriaStdIoIoError".to_string(),
        crate::compiler_known_io::INVALID_UTF8_ERROR => "__DoriaStdIoInvalidUtf8Error".to_string(),
        _ if name.contains('\\') => {
            format!("__DoriaQualified_{}", hex_bytes(name.as_bytes()))
        }
        _ => name.to_string(),
    }
}

fn php_function_name(name: &str) -> String {
    format!("__DoriaFunction_{}", hex_bytes(name.as_bytes()))
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
