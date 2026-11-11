use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::backend::{BackendError, BackendOutput};
use crate::builtins::Builtin;
use crate::const_eval::{ConstKey, ConstValue, Evaluation, ParameterDefaultKey};
use crate::diagnostics::Diagnostic;
use crate::format_string::{self, FormatConversion, FormatPiece};
use crate::hir::*;
use crate::mir;
use crate::numeric::{parse_decimal_magnitude, FloatType, IntegerType};
use crate::php_closure::{PhpClosureDescriptor, PhpClosurePlan};
#[cfg(test)]
use crate::php_execution;
use crate::semantics::{
    GivenSemanticInfo, MatchSemanticInfo, ResolvedMatchPattern, SemanticInfo, WhenSemanticInfo,
};
use crate::source::Span;
use crate::symbols::{BindingId, BuiltinInterface};
use crate::types::{ResolvedType, TypeRef};

mod assignment_place;
mod bytes;
mod concurrency;
mod core_collection;
mod core_value;
mod diagnostic_sites;
mod expression_region;
mod integers;
mod interface;
mod native_runtime;
mod numeric_place;
mod property_hooks;
mod shared;
mod specialization;
mod standard_io;
mod streams;
mod term;
mod value_argument;
mod value_result;

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
    public function __construct(
        public string $typeName,
        public bool $hasTransferredByteCount = false,
    ) {}
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

    public function ensureOrigin(int $origin, string $callable): void
    {
        $error = $this->error();
        $error->__doriaEnsureErrorOrigin($origin, $callable);
        $this->origin = $error->__doriaErrorOrigin();
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

// PHP catch bindings are function-scoped; forwarding transfers their carrier.
function __doria_rethrow_checked_error(?__DoriaCheckedError &$caught): never
{
    $forwarded = $caught ?? throw new LogicException('missing Doria checked-error carrier');
    $caught = null;
    throw $forwarded;
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

function __doria_error_descriptor(string $typeName, bool $hasTransferredByteCount = false): __DoriaErrorDescriptor
{
    static $descriptors = [];
    return $descriptors[$typeName] ??= new __DoriaErrorDescriptor($typeName, $hasTransferredByteCount);
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

function __doria_assertion_bytes_difference(__DoriaBytes $actual, __DoriaBytes $expected): string
{
    $common = min(count($actual), count($expected));
    for ($index = 0; $index < $common; ++$index) {
        $actualByte = $actual->get($index, 0, 0, "");
        $expectedByte = $expected->get($index, 0, 0, "");
        if ($actualByte !== $expectedByte) {
            return "First Differing Byte: " . $index . "\nExpected Byte: " .
                sprintf('%02x', $expectedByte) . "\nActual Byte: " .
                sprintf('%02x', $actualByte);
        }
    }
    return "Expected Byte Length: " . count($expected) . "\nActual Byte Length: " .
        count($actual) . "\nDelta: " . (count($actual) - count($expected));
}

function __doria_write_error_outcome_v3(
    __DoriaCheckedError $caught,
    string $path,
    bool $withTransferProgress = false,
): bool
{
    $type = $caught->descriptor->typeName;
    $message = $caught->error()->message;
    $origin = $caught->origin;
    $known = $origin !== 0;
    [$sourcePath, $sourceText, $start, $end] = __doria_error_source_location($origin);
    $function = $known ? $caught->error()->__doriaErrorCallable() : "";
    $record = ($withTransferProgress ? "DORIAO5\0" : "DORIAO3\0") .
        pack("vV", $withTransferProgress ? 5 : 3, strlen($type)) .
        __doria_u64_le(strlen($message)) .
        pack("VVVC", strlen($sourcePath), strlen($sourceText), strlen($function), $known ? 1 : 0) .
        __doria_u64_le($start) . __doria_u64_le($end);
    if ($withTransferProgress) {
        $count = $caught->descriptor->hasTransferredByteCount
            ? $caught->error()->transferredByteCount : null;
        $value = $count ?? 0;
        $record .= pack("CV2", $count !== null ? 1 : 0,
            $value & 0xffffffff, ($value >> 32) & 0xffffffff);
    }
    $record .= $type . $message . $sourcePath . $sourceText . $function;
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
    [$sourcePath, $sourceText, $start, $end] = __doria_error_source_location($origin);
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
        __doria_u64_le($start) . __doria_u64_le($end) .
        pack("v", 0) . $type . $matcher . $actualType . $actualPresentation .
        $expectedType . $expectedPresentation . $difference . $userMessage .
        $sourcePath . $sourceText . $function;
    return __doria_publish_outcome($path, $record);
}

function __doria_report_unhandled_error(__DoriaCheckedError $caught): void
{
    $type = $caught->descriptor->typeName;
    $origin = $caught->origin;
    $site = __doria_error_site($origin);
    [$sourcePath] = __doria_error_source_location($origin);
    if (!$site["known"]) { $sourcePath = "<source unavailable>"; }
    $line = $site["line"];
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
    $v5 = getenv("DORIA_RUNTIME_OUTCOME_V5");
    if (!$assertion && is_string($v5) && __doria_write_error_outcome_v3($caught, $v5, true)) {
        $caught->dropError();
        exit(70);
    }
    $v3 = getenv("DORIA_RUNTIME_OUTCOME_V3");
    if (!$assertion && !is_string($v5) && is_string($v3) && __doria_write_error_outcome_v3($caught, $v3)) {
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
            $sourcePath . " · line " . $line . " · " . $error->__doriaErrorCallable() . "\n" .
            __doria_source_excerpt($site, "Assertion Failed Here") .
            ($expectedPresent ? "\n\nExpected\n  " . $expectedPresentation : "") .
            ($actualPresent ? "\n\nActual\n  " . $actualPresentation : "") .
            ($differencePresent ? "\n\nDifference\n  " . __doria_safe_error_message($difference) : "") .
            "\n\nWhy\n  " . __doria_safe_error_message($error->message) .
            "\n\nProcess Exited With Status 70\n";
        __doria_terminal_ending();
        @fwrite(STDERR, $message);
        $caught->dropError();
        exit(70);
    }
    global $__doria_io_message_vocabulary;
    $transferred = $caught->descriptor->hasTransferredByteCount
        ? $error->transferredByteCount : null;
    $progress = $transferred === null ? "" :
        "\n\n" . $__doria_io_message_vocabulary["progressHeading"] . "\n  " . $transferred;
    $message = "Error[R1000]: Unhandled " . $type . "\n\nWhere\n" .
        $sourcePath . " · line " . $line . " · " .
        $error->__doriaErrorCallable() . "\n" .
        __doria_source_excerpt($site, "This Error Was First Thrown Here") .
        $progress . "\n\nWhy\n  " .
        __doria_safe_error_message($error->message) .
        "\n\nProcess Exited With Status 70\n";
    __doria_terminal_ending();
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
    return is_array($collection) || $collection instanceof Countable
        ? count($collection) : $collection->count;
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
            __doria_rethrow_checked_error($error);
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

    public function __doriaBorrowCell(mixed $key, int $start, int $end, string $callable, bool $optional = false): __DoriaCell
    {
        [$found, $index] = $this->locate($key);
        if (!$found && $optional) { return new __DoriaCell(null); }
        if (!$found) { __doria_panic('P1312', $start, $end, null, $callable); }
        return __DoriaCell::borrowed($this->entries[$index][1]);
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
        if (!$found) { __doria_panic('P1312', 0, 0); }
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
    public function __doriaBorrowElementCell(string $member): __DoriaCell
    {
        if (!$this->values) { return new __DoriaCell(null); }
        $index = $member === 'last' ? array_key_last($this->values) : array_key_first($this->values);
        return __DoriaCell::borrowed($this->values[$index]);
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
    public function __doriaBorrowElementCell(string $member): __DoriaCell
    {
        return $this->heap ? __DoriaCell::borrowed($this->heap[0]) : new __DoriaCell(null);
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
            __doria_rethrow_checked_error($error);
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
    public function __doriaBorrowElementCell(string $member): __DoriaCell
    {
        if ($this->count === 0) { return new __DoriaCell(null); }
        $index = $member === 'peekBack' ? ($this->head + $this->count - 1) % $this->capacity : $this->head;
        return __DoriaCell::borrowed($this->values[$index]);
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

interface __DoriaManagedCopy
{
    public function __doriaCopy(): mixed;
    public function __doriaRelease(): void;
}

function __doria_copy_value(mixed $value): mixed
{
    return $value instanceof __DoriaManagedCopy ? $value->__doriaCopy() : $value;
}

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

class __DoriaCell
{
    public bool $live = true;
    public bool $borrowed = false;

    public function __construct(public mixed $value)
    {
    }

    // The checker owns access and lifetime permission. This private backend
    // reference transports that exact slot; it is neither an owner nor a copy.
    public static function borrowed(mixed &$slot): self
    {
        $cell = new self(null);
        $cell->value =& $slot;
        $cell->borrowed = true;
        return $cell;
    }

    public function __doriaProjectCell(): self
    {
        return $this->value instanceof __DoriaMixedValue
            ? self::borrowed($this->value->value()) : $this;
    }

    public function __doriaIsPresent(): bool
    {
        return $this->value !== null;
    }

    public function __doriaDropValue(): void
    {
        __doria_drop_value($this->value);
    }

    public function __doriaLoadFields(): void
    {
        __DoriaValueResult::clear();
        $value = $this->value;
        if ($value !== null) { $value->__doriaPublishFields(); }
    }

    public function __doriaStoreFields(): void
    {
        $value = __DoriaValueResult::boxed();
        __DoriaValueResult::clear();
        $this->value = $value;
        $this->live = true;
    }
}

function __doria_project_cell(__DoriaCell $cell): __DoriaCell
{
    return $cell->__doriaProjectCell();
}

function __doria_take_cell(__DoriaCell $cell): mixed
{
    if ($cell->borrowed) {
        throw new LogicException("compiler invariant violated: borrowed Doria place was moved");
    }
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
    } elseif ($value instanceof __DoriaManagedCopy) {
        $value->__doriaRelease();
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
    if ($cell->borrowed) { return; }
    $cell->__doriaDropValue();
}

function __doria_replace_cell(__DoriaCell $cell, mixed $replacement): void
{
    $previous = $cell->live ? $cell->value : null;
    $cell->value = $replacement;
    $cell->live = true;
    __doria_drop_value($previous);
}

function __doria_collection_index(array|SortedDictionary $values, int|string $key, bool $dictionary, int $start, int $end, string $callable): mixed
{
    if ($values instanceof SortedDictionary) {
        if (!$values->containsKey($key)) { __doria_panic("P1312", $start, $end, null, $callable); }
        return $values->get($key);
    }
    if (!array_key_exists($key, $values)) {
        __doria_panic($dictionary ? "P1312" : "P1310", $start, $end, null, $callable,
            $dictionary ? [] : ["index" => [1, $key], "length" => [2, count($values)]]);
    }
    return $values[$key];
}

function __doria_collection_cell(__DoriaCell $owner, int|string $key, bool $dictionary, int $start, int $end, string $callable): __DoriaCell
{
    if (!$owner->live) {
        throw new LogicException("compiler invariant violated: moved Doria collection was borrowed");
    }
    if (!array_key_exists($key, $owner->value)) {
        __doria_panic($dictionary ? "P1312" : "P1310", $start, $end, null, $callable,
            $dictionary ? [] : ["index" => [1, $key], "length" => [2, count($owner->value)]]);
    }
    return __DoriaCell::borrowed($owner->value[$key]);
}

function __doria_collection_optional_cell(__DoriaCell $owner, string $member, int|string|null $key): __DoriaCell
{
    if (!$owner->live) {
        throw new LogicException("compiler invariant violated: moved Doria collection was borrowed");
    }
    if ($owner->value === null || $owner->value === []) { return new __DoriaCell(null); }
    if ($member !== 'get') {
        $key = $member === 'last' ? array_key_last($owner->value) : array_key_first($owner->value);
    }
    return array_key_exists($key, $owner->value) ? __DoriaCell::borrowed($owner->value[$key]) : new __DoriaCell(null);
}

function __doria_list_remove_at(__DoriaCell $owner, int $index, int $start, int $end, string $callable): mixed
{
    if (!$owner->live) {
        throw new LogicException("compiler invariant violated: moved Doria collection was borrowed");
    }
    if (!array_key_exists($index, $owner->value)) {
        __doria_panic("P1310", $start, $end, null, $callable,
            ["index" => [1, $index], "length" => [2, count($owner->value)]]);
    }
    $removed = array_splice($owner->value, $index, 1);
    return $removed[0];
}

function __doria_list_insert_at(__DoriaCell $owner, int $index, mixed $value, int $start, int $end, string $callable): void
{
    if (!$owner->live) {
        throw new LogicException("compiler invariant violated: moved Doria collection was borrowed");
    }
    if ($index < 0 || $index > count($owner->value)) {
        __doria_panic("P1310", $start, $end, null, $callable,
            ["index" => [1, $index], "length" => [2, count($owner->value)]]);
    }
    array_splice($owner->value, $index, 0, [$value]);
}

function __doria_dictionary_get(?array $values, int|string $key): mixed
{
    return $values[$key] ?? null;
}

function __doria_list_pop(__DoriaCell $owner): mixed
{
    if (!$owner->live) {
        throw new LogicException("compiler invariant violated: moved Doria collection was borrowed");
    }
    return array_pop($owner->value);
}

function __doria_dictionary_remove(__DoriaCell $owner, int|string $key): mixed
{
    if (!$owner->live) {
        throw new LogicException("compiler invariant violated: moved Doria collection was borrowed");
    }
    $removed = $owner->value[$key] ?? null;
    unset($owner->value[$key]);
    return $removed;
}

function __doria_collection_set(array|SortedDictionary &$values, int|string $key, mixed $replacement, bool $dictionary, int $start, int $end, string $callable): void
{
    if ($values instanceof SortedDictionary) {
        $previous = $values->get($key);
        $values->set($key, $replacement);
        __doria_drop_value($previous);
        return;
    }
    if (!$dictionary && !array_key_exists($key, $values)) {
        __doria_panic("P1310", $start, $end, null, $callable, ["index" => [1, $key], "length" => [2, count($values)]]);
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
    output.push_str(term::RUNTIME);
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
    let field_parameters = specialization
        .function_argument_fields(function_type)
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            ty.as_ref()
                .map(|ty| value_result::Place::parameter_in(ty, index, specialization))
        })
        .collect::<Vec<_>>();
    let field_result = specialization
        .function_field_result(function_type)
        .map(|ty| {
            value_result::Place::parameter_in(&ty, function_type.parameters.len(), specialization)
        });
    for transport in [
        value_argument::Transport::Boxed,
        value_argument::Transport::FieldArguments,
        value_argument::Transport::FieldResult,
        value_argument::Transport::FieldArgumentsAndResult,
    ] {
        let field_arguments = transport.arguments();
        if field_arguments && !field_parameters.iter().any(Option::is_some) {
            continue;
        }
        if transport.result() && field_result.is_none() {
            continue;
        }
        write_indent(output, 1);
        output.push_str(&format!("public function {}(", transport.callback_entry()));
        output.push_str(
            &function_type
                .parameters
                .iter()
                .enumerate()
                .flat_map(|(index, parameter)| {
                    if let Some(place) =
                        field_parameters[index].as_ref().filter(|_| field_arguments)
                    {
                        return place.declarations();
                    }
                    let ty = if crate::php_closure::parameter_uses_cell(function_type, index) {
                        "__DoriaCell".to_string()
                    } else {
                        php_mir_type(parameter.ty, program, specialization)
                    };
                    vec![format!("{ty} $argument{index}")]
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push(')');
        if transport.result() {
            output.push_str(": void");
        } else if let mir::ReturnType::Value(ty) = function_type.return_type {
            output.push_str(": ");
            let hint = php_mir_type(ty, program, specialization);
            output.push_str(
                if function_type
                    .return_borrow
                    .is_some_and(|borrow| borrow.kind == mir::ReturnBorrowKind::Value)
                    && ty.has_move_ownership()
                    && !matches!(
                        ty,
                        mir::Type::ByteRegion(_) | mir::Type::NullableByteRegion(_)
                    )
                {
                    "__DoriaCell"
                } else {
                    &hint
                },
            );
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
        if !field_arguments {
            for (index, place) in field_parameters.iter().enumerate() {
                if let Some(place) = place {
                    value_result::unpack_stable(
                        &format!("argument{index}"),
                        place,
                        output,
                        2,
                        specialization,
                    );
                }
            }
        }
        let helper = descriptor.owner_class.as_ref().map_or_else(
            || descriptor.helper_name.clone(),
            |class| format!("{class}::{}", descriptor.helper_name),
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
                .flat_map(|(index, _)| {
                    field_parameters[index].as_ref().map_or_else(
                        || vec![format!("$argument{index}")],
                        |place| place.names().map(|name| format!("${name}")).collect(),
                    )
                }),
        );
        let call = format!("{helper}({})", arguments.join(", "));
        if descriptor.invocation_mode == mir::FunctionInvocationMode::Once
            && descriptor.environment_name.is_some()
        {
            writeln(output, 2, "$environment = $this->__doriaEnvironment;");
            writeln(output, 2, "$this->__doriaEnvironment = null;");
            let call = call.replace("$this->__doriaEnvironment", "$environment");
            writeln(output, 2, "try {");
            if let Some(place) = &field_result {
                writeln(output, 3, &format!("{call};"));
                place.capture(output, 3);
            } else if matches!(function_type.return_type, mir::ReturnType::Value(_)) {
                writeln(output, 3, &format!("$result = {call};"));
            } else {
                writeln(output, 3, &format!("{call};"));
            }
            writeln(output, 2, "} catch (__DoriaCheckedError $error) {");
            writeln(output, 3, "$environment->__doriaDrop();");
            if field_result.is_some() {
                value_result::clear(output, 3);
            }
            writeln(output, 3, "__doria_rethrow_checked_error($error);");
            writeln(output, 2, "}");
            writeln(output, 2, "$environment->__doriaDrop();");
            if field_result.is_none()
                && matches!(function_type.return_type, mir::ReturnType::Value(_))
            {
                writeln(output, 2, "return $result;");
            }
        } else if let Some(place) = &field_result {
            writeln(output, 2, &format!("{call};"));
            place.capture(output, 2);
        } else if matches!(function_type.return_type, mir::ReturnType::Value(_)) {
            writeln(output, 2, &format!("return {call};"));
        } else {
            writeln(output, 2, &format!("{call};"));
        }
        if let Some(place) = &field_result {
            if transport.result() {
                place.publish(output, 2);
            } else {
                writeln(output, 2, &format!("return {};", place.read()));
            }
        }
        writeln(output, 1, "}");
        output.push('\n');
    }
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
        mir::Type::ByteRegion(_) => "__DoriaByteRegion".to_string(),
        mir::Type::NullableByteRegion(_) => "?__DoriaByteRegion".to_string(),
        mir::Type::Scalar(mir::ScalarType::Float(_)) => "float".to_string(),
        mir::Type::Scalar(mir::ScalarType::Bool) => "bool".to_string(),
        mir::Type::Scalar(mir::ScalarType::Enum(id)) => specialization.enum_symbol_by_id(id),
        mir::Type::Scalar(mir::ScalarType::Inline(ty)) => php_type_symbol(ty.name()),
        mir::Type::String => "string".to_string(),
        mir::Type::Mixed => "mixed".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Integer(_)) => "?int".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Float(_)) => "?float".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Bool) => "?bool".to_string(),
        mir::Type::NullableScalar(mir::ScalarType::Enum(id)) => {
            format!("?{}", specialization.enum_symbol_by_id(id))
        }
        mir::Type::NullableScalar(mir::ScalarType::Inline(ty)) => {
            format!("?{}", php_type_symbol(ty.name()))
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
        mir::Type::PayloadEnum(ty) => specialization.enum_symbol_by_id(ty.id),
        mir::Type::NullablePayloadEnum(ty) => {
            format!("?{}", specialization.enum_symbol_by_id(ty.id))
        }
        mir::Type::Function(_) => "__DoriaFunctionValue".to_string(),
        mir::Type::NullableFunction(_) => "?__DoriaFunctionValue".to_string(),
        mir::Type::Collection(id) | mir::Type::NullableCollection(id) => {
            let nullable = matches!(ty, mir::Type::NullableCollection(_));
            let definition = &program.collection_types[id.0];
            let base = if definition.uses_core_operations() {
                "__DoriaCoreCollection".to_string()
            } else {
                match definition.kind {
                    mir::CollectionKind::Bytes => "__DoriaBytes".to_string(),
                    mir::CollectionKind::SortedDictionary => php_type_symbol("SortedDictionary"),
                    mir::CollectionKind::SortedSet => php_type_symbol("SortedSet"),
                    mir::CollectionKind::PriorityQueue => php_type_symbol("PriorityQueue"),
                    mir::CollectionKind::Deque => php_type_symbol("Deque"),
                    _ => "array".to_string(),
                }
            };
            if nullable {
                php_nullable_type_hint(base)
            } else {
                base
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
    match generate_artifact(program, mir)? {
        BackendOutput::Text { contents, .. } | BackendOutput::TextWithRuntime { contents, .. } => {
            Ok(contents)
        }
        _ => unreachable!("PHP emission always produces text"),
    }
}

pub fn generate_artifact(
    program: &Program,
    mir: Option<&mir::Program>,
) -> Result<BackendOutput, BackendError> {
    let mut closure_plan = PhpClosurePlan::build(program, mir);
    let specialization = Rc::new(specialization::Plan::build(program, &closure_plan)?);
    validate_program(program, &specialization)?;
    specialization.prepare_call_sites(&mut closure_plan, &HashMap::new());
    let runtime = (native_runtime::required(program) || specialization.native_standard_io)
        .then(crate::runtime_artifact::shared_runtime_dependency)
        .transpose()?;
    for descriptor in closure_plan.descriptors.values_mut() {
        let class = match descriptor.source_instance {
            mir::ClosureOwner::Callable(id) => specialization.callable(id).class,
            mir::ClosureOwner::PropertyInitializer(property) => Some(property.class),
        };
        // Emission consumes the specialized PHP identity, not a source name
        // that should be encoded a second time.
        descriptor.owner_class = class.map(|class| specialization.class_symbol(class).to_string());
    }
    let closure_plan = Rc::new(closure_plan);

    let mut output = String::from(
        "<?php\n\ninterface __DoriaDisplayable\n{\n    public function toString(): string;\n}\n\ninterface __DoriaValueEquatable\n{\n    public function __doriaEquals(mixed $other): bool;\n}\n\nfinal class __DoriaMixedValue\n{\n    public function __construct(\n        private readonly string $typeTag,\n        private mixed $value,\n    ) {\n    }\n\n    public function is(string $typeTag): bool { return $this->typeTag === $typeTag; }\n    public function &value(): mixed { return $this->value; }\n}\n\nfunction __doria_box_mixed(string $typeTag, mixed $value): __DoriaMixedValue\n{\n    if ($typeTag === 'float32') { $value = unpack('G', pack('G', $value))[1]; }\n    return new __DoriaMixedValue($value === null ? 'null' : $typeTag, $value);\n}\n\nfunction __doria_mixed_is(mixed $value, string $typeTag): bool\n{\n    return $value instanceof __DoriaMixedValue && $value->is($typeTag);\n}\n\nfunction __doria_mixed_value(mixed $value): mixed\n{\n    return $value instanceof __DoriaMixedValue ? $value->value() : $value;\n}\n\nfunction __doria_equal(mixed $left, mixed $right): bool\n{\n    if ($left instanceof __DoriaValueEquatable) { return $left->__doriaEquals($right); }\n    if ($right instanceof __DoriaValueEquatable) { return $right->__doriaEquals($left); }\n    return $left === $right;\n}\n\nfunction __doria_display(string|int|float|bool|__DoriaDisplayable $value): string\n{\n    if ($value instanceof __DoriaDisplayable) { return $value->toString(); }\n    if (is_bool($value)) { return $value ? 'true' : 'false'; }\n    return (string) $value;\n}\n\nfunction __doria_less(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) < 0; }\n    return $left < $right;\n}\n\nfunction __doria_less_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) <= 0; }\n    return $left <= $right;\n}\n\nfunction __doria_greater(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) > 0; }\n    return $left > $right;\n}\n\nfunction __doria_greater_equal(string|int|float|bool $left, string|int|float|bool $right): bool\n{\n    if (is_string($left) && is_string($right)) { return strcmp($left, $right) >= 0; }\n    return $left >= $right;\n}\n\n",
    );
    value_result::emit_runtime(program, &mut output, &specialization);
    emit_primitive_comparison_runtime(&mut output);
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
    interface::emit_declarations(&program.semantic_info, &specialization, &mut output);
    output.push_str(PHP_STAGE26_COLLECTION_HELPERS);
    output.push_str(core_collection::RUNTIME);
    if let Some(runtime) = &runtime {
        native_runtime::emit(&mut output, program, runtime);
    }
    standard_io::emit_runtime(&mut output, runtime.is_some());
    bytes::emit_runtime(&mut output, runtime.is_some());
    output.push_str(integers::RUNTIME);
    output.push_str(numeric_place::RUNTIME);
    streams::emit_runtime(
        &mut output,
        program
            .semantic_info
            .classes
            .iter()
            .any(|class| class.declaration_name == crate::compiler_known_stream::CHILD),
    );
    emit_php_closure_runtime(&closure_plan, &specialization, mir, &mut output);
    value_result::emit_region_runtime(&mut output, &specialization);
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
    let diagnostic_sites_offset = output.len();
    output.push_str(diagnostic_sites::RUNTIME);
    output.push_str("$__doria_catalogue = [\n");
    for entry in doria_diagnostic_catalogue::RUNTIME_CATALOGUE {
        output.push_str(&format!(
            "    {} => [{}, {}, {}, [{}]],\n",
            emit_php_string_literal(entry.code),
            emit_php_string_literal(entry.title),
            emit_php_string_literal(entry.primary_label),
            emit_php_string_literal(entry.explanation),
            entry
                .fact_names
                .iter()
                .map(|name| emit_php_string_literal(name))
                .collect::<Vec<_>>()
                .join(", "),
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
    ?array $operation = null,
)
{
    global $__doria_catalogue, $__doria_panicking;
    if (!isset($__doria_catalogue[$code]) || !__doria_panic_facts_match($code, $facts)) {
        $code = "P1001";
        $facts = [];
        $message = null;
    }
    [$title, $label, $why] = $__doria_catalogue[$code];
    if ($code === "P1501") { $why = $facts["conflictReason"][1]; }
    [$sourcePath, $sourceText, $sourceStart] = __doria_source_location($start);
    [, , $sourceEnd] = __doria_source_location($end);
    $site = __doria_diagnostic_site($start, $end);
    $line = $site["line"];
    if (!$site["known"]) { $sourcePath = "<source unavailable>"; }
    // Primary labels and operation frames are separate compiler-owned spans.
    // Human rendering and private V2 transport consume this same snapshot.
    $frames = __doria_panic_frames($operation ?? $site);
    $function = $callable ?? "main";
    if ($callable === null && isset($frames[0])) {
        $function = $frames[0][0];
    }
    $channel = getenv("DORIA_RUNTIME_OUTCOME_V2");
    if (is_string($channel) && __doria_write_panic_outcome_v2(
        $channel, $code, $message, $site, $function, $facts, $frames,
    )) {
        $__doria_panicking = true;
        exit(101);
    }
    __doria_terminal_ending();
    @fwrite(STDERR, "Panic[" . $code . "]: " . $title . "\n\nWhere\n");
    @fwrite(STDERR, $sourcePath . " · line " . $line . " · " . $function . "\n");
    @fwrite(STDERR, __doria_source_excerpt($site, $label));
    @fwrite(STDERR, "\n\nWhy\n" . $why);
    if ($code === "P1000" && $message !== null) {
        @fwrite(STDERR, "\n\nNote\n" . $message);
    }
    @fwrite(STDERR, "\n\nCall Path");
    foreach ($frames as [$name, $framePath, $frameStart, $frameEnd, $frameLine]) {
        @fwrite(STDERR, "\n" . $name . " · " . $framePath . ":" . $frameLine);
    }
    @fwrite(STDERR, "\n\nProcess Exited With Status 101\n");
    $__doria_panicking = true;
    exit(101);
}

function __doria_read_line(string $prompt, int $start, int $end, string $callable): ?string
{
    if (__DORIA_NATIVE_STANDARD_IO) {
        return __DoriaNativeStandardIo::get()->intrinsic(0, $prompt, null, $start, $end, $callable);
    }
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
            __doria_checked_origin($start, $end),
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
            __doria_checked_origin($start, $end),
            $callable,
        );
    }
    return $line;
}

function __doria_read_file(string $path, int $start, int $end, string $callable): string
{
    if (str_contains($path, "\0")) {
        __doria_throw_io_validation(__DoriaStdIoIoOperation::Open, $path, __doria_checked_origin($start, $end), $callable);
    }
    error_clear_last();
    $file = @fopen($path, "rb");
    if ($file === false) {
        __doria_throw_io(
            __DoriaStdIoIoOperation::Open,
            __DoriaStdIoIoTarget::File($path),
            error_get_last(),
            __doria_checked_origin($start, $end),
            $callable,
        );
    }
    $contents = "";
    while (!feof($file)) {
        error_clear_last();
        $chunk = @fread($file, 8192);
        if ($chunk === false) {
            // The failed call's internal consumption is unknown. Preserve the
            // original error across close, without reporting a prefix as an
            // exact transferred-byte total.
            $hostError = error_get_last();
            @fclose($file);
            __doria_throw_io(
                __DoriaStdIoIoOperation::Read,
                __DoriaStdIoIoTarget::File($path),
                $hostError,
                __doria_checked_origin($start, $end),
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
            __doria_checked_origin($start, $end),
            $callable,
            strlen($contents),
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
            __doria_checked_origin($start, $end),
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
        __doria_throw_io_validation(__DoriaStdIoIoOperation::Open, $path, __doria_checked_origin($start, $end), $callable);
    }
    error_clear_last();
    $file = @fopen($path, $append ? "ab" : "wb");
    if ($file === false) {
        __doria_throw_io(
            __DoriaStdIoIoOperation::Open,
            __DoriaStdIoIoTarget::File($path),
            error_get_last(),
            __doria_checked_origin($start, $end),
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
            __doria_checked_origin($start, $end),
            $callable,
            strlen($contents),
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
    return $__doria_io_message_vocabulary["operations"][$operation->name];
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
    if ($target->__doriaMatchesCase(3)) { return $__doria_io_message_vocabulary["stderr"]; }
    if ($target->__doriaMatchesCase(4)) { return $__doria_io_message_vocabulary["readinessSet"]; }
    if ($target->__doriaMatchesCase(5)) { return $__doria_io_message_vocabulary["transferPlan"]; }
    if ($target->__doriaMatchesCase(6)) { return $__doria_io_message_vocabulary["transferGroup"]; }
    $suffix = match (true) {
        $target->__doriaMatchesCase(8) => "processInputSuffix",
        $target->__doriaMatchesCase(9) => "processOutputSuffix",
        $target->__doriaMatchesCase(10) => "processErrorsSuffix",
        default => "processSuffix",
    };
    return $__doria_io_message_vocabulary["processPrefix"] . $target->__doriaPayloadAt(0) .
        $__doria_io_message_vocabulary[$suffix];
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
    ?int $transferredByteCount = null,
    ?__DoriaStdIoIoErrorReason $reason = null,
): void {
    $systemCode = __doria_system_code($hostError);
    $reason ??= __doria_io_reason($hostError);
    __doria_throw_io_facts($operation, $target, $reason, $systemCode,
        $transferredByteCount, $origin, $callable);
}

function __doria_throw_io_facts(
    __DoriaStdIoIoOperation $operation, __DoriaStdIoIoTarget $target,
    __DoriaStdIoIoErrorReason $reason, ?int $systemCode, ?int $transferredByteCount,
    int $origin, string $callable,
): void {
    global $__doria_io_message_vocabulary;
    $message = $__doria_io_message_vocabulary["ioPrefix"] .
        __doria_io_operation_word($operation) . " " . __doria_io_target_name($target) .
        $__doria_io_message_vocabulary["separator"] . __doria_io_reason_words($reason);
    __doria_throw(new __DoriaStdIoIoError(
        $message,
        $operation,
        $target,
        $reason,
        $systemCode,
        $transferredByteCount,
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
            if (($target->__doriaMatchesCase(2) || $target->__doriaMatchesCase(3)) &&
                __doria_is_broken_pipe(error_get_last())) { exit(0); }
            // Zero establishes no progress in this call; FALSE does not.
            __doria_throw_io($operation, $target, error_get_last(), __doria_checked_origin($start, $end), $callable,
                $written === 0 ? $offset : null);
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
        __doria_checked_origin($start, $end),
        $callable,
    );
}

function __doria_write_stdout(string $value, int $start, int $end, string $callable): void
{
    if (__DORIA_NATIVE_STANDARD_IO) {
        __DoriaNativeStandardIo::get()->intrinsic(6, null, $value, $start, $end, $callable);
        return;
    }
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
    if (__DORIA_NATIVE_STANDARD_IO) {
        __DoriaNativeStandardIo::get()->intrinsic(7, null, $value, $start, $end, $callable);
        return;
    }
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
                ClassMember::Property(property) if property.is_static && property.has_storage() => {
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
        // Payload cases belong to the declaration, shared by its specializations.
        .map(|definition| definition.source_type.id)
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
                                if property_hooks::has_runtime_initializer(
                                    property, &class.name, &program.semantic_info, &specialization,
                                )
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
            move_enum_names: crate::ownership::MoveEnums::new(
                program
                    .semantic_info
                    .enums
                    .iter()
                    .filter(|definition| {
                        definition.source_type.arguments.is_empty() && !definition.capabilities.copy
                    })
                    .map(|definition| definition.name.clone()),
                program.semantic_info.enums.iter().map(|definition| {
                    (
                        definition.source_type.clone(),
                        !definition.capabilities.copy,
                    )
                }),
            ),
        },
        closure_plan,
    );
    scopes.closure_owner = mir
        .and_then(|program| program.selected_entry)
        .map(mir::ClosureOwner::Callable);
    scopes.matches = program.semantic_info.matches.clone();
    scopes.specialization = specialization;
    scopes.whens = program.semantic_info.whens.clone();
    scopes.given_preludes = program.semantic_info.given_preludes.clone();
    scopes.expression_types = php_expression_types(&program.semantic_info);
    scopes.flow_facts = Rc::new(program.semantic_info.flow_facts.clone());
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
                .map(|case| {
                    (
                        crate::enums::EnumCaseSource {
                            declaration: definition.source_type.id,
                            index: case.id.index,
                        },
                        case.tag,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    emit_closure_entries(None, &mut output, 0, &scopes);
    // PHP binds a class that implements an interface or extends a parent when
    // execution reaches it, so every declaration precedes the statements that
    // run, whichever source order the program lists them in.
    let (statements, declarations): (Vec<_>, Vec<_>) = program
        .items
        .iter()
        .partition(|item| matches!(item, Item::Statement(_)));
    for item in declarations.into_iter().chain(statements) {
        emit_item(item, &program.semantic_info, &mut output, 0, &mut scopes);
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push('\n');
    }
    scopes.core_helpers.borrow().emit(&mut output);
    output.insert_str(
        diagnostic_sites_offset,
        &scopes.diagnostic_sites.borrow().emit_metadata(program)?,
    );
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
    Ok(match runtime {
        Some(runtime) => BackendOutput::TextWithRuntime {
            extension: "php".into(),
            contents: output,
            runtime: Box::new(runtime),
        },
        None => BackendOutput::Text {
            extension: "php".into(),
            contents: output,
        },
    })
}

struct PhpValidation<'a> {
    semantic: &'a SemanticInfo,
    plan: &'a specialization::Plan,
    substitutions: &'a HashMap<String, ResolvedType>,
    expression_types: HashMap<Span, ResolvedType>,
    shared_construction_types: HashMap<Span, ResolvedType>,
    #[cfg(test)]
    expression_visits: Cell<usize>,
    #[cfg(test)]
    expression_visit_limit: Option<usize>,
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
            #[cfg(test)]
            expression_visits: Cell::new(0),
            #[cfg(test)]
            expression_visit_limit: None,
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
                    for method in declaration.members.iter().flat_map(ClassMember::callables) {
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
            Item::Enum(declaration) if !declaration.type_params.is_empty() => {
                for instance in plan.enum_specializations(&declaration.name) {
                    let substitutions = declaration
                        .type_params
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .zip(instance.arguments.iter().cloned())
                        .collect();
                    let context = PhpValidation::new(&program.semantic_info, plan, &substitutions);
                    validate_item(item, &context)?;
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
                if property.is_static && property.has_storage() {
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
    // A destructor's receiver is writable by the lifecycle protocol (decision
    // log entry 42); a constructor's never is.
    if is_method
        && matches!(function.name.as_str(), "__construct" | "__destruct")
        && (function.is_static || (function.writable_this && function.name == "__construct"))
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
                    function: function.span,
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
        || ty.name == "Bytes"
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
            crate::inline_value::InlineValueKind::for_name(&class.name).is_some_and(|kind| kind.requires_owned_cleanup()) || concurrency::has_source_payload(class) || streams::has_payload(class) || streams::has_child_payload(class) || streams::has_set_payload(class) || streams::has_plan_payload(class) || streams::has_group_payload(class) || class.members.iter().any(
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
                ClassMember::Property(property)
                    if !property.is_static && property.has_storage() =>
                {
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
        ResolvedType::ByteRegion(_) => Ok(()),
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
                        validate_increment(increment, semantic_info)?;
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
                let iterable = foreach
                    .dictionary_projection()
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
            if let Some((dictionary, _)) = foreach.dictionary_projection() {
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
        Stmt::Increment(increment) => validate_increment(increment, semantic_info),
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
    if bytes::supports_rmw(&assignment.target, semantic_info) {
        return Ok(());
    }

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
        AssignOp::AddAssign
        | AssignOp::SubAssign
        | AssignOp::MulAssign
        | AssignOp::DivAssign
        | AssignOp::ModAssign
        | AssignOp::ShiftLeftAssign
        | AssignOp::ShiftRightAssign
        | AssignOp::BitwiseAndAssign
        | AssignOp::BitwiseOrAssign
        | AssignOp::BitwiseXorAssign
            if integers::ty(semantic_info.expression_type(assignment.value.span())).is_some() =>
        {
            None
        }
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

fn validate_increment(
    increment: &IncrementStmt,
    semantic_info: &PhpValidation<'_>,
) -> Result<(), BackendError> {
    if bytes::supports_rmw(&increment.target, semantic_info)
        || integers::ty(semantic_info.expression_type(increment.target.span())).is_some()
    {
        validate_expr(&increment.target, semantic_info)
    } else {
        Err(unsupported_increment(increment))
    }
}

fn validate_expr(expr: &Expr, semantic_info: &PhpValidation<'_>) -> Result<(), BackendError> {
    #[cfg(test)]
    {
        let visits = semantic_info.expression_visits.get() + 1;
        semantic_info.expression_visits.set(visits);
        assert!(
            semantic_info
                .expression_visit_limit
                .is_none_or(|limit| visits <= limit),
            "PHP validation revisited an expression subtree"
        );
    }
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
                Some(ResolvedType::Set(_))
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
                && !receiver_type.is_some_and(|ty| php_collection_property(ty, property))
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
                    "length" | "byteLength" | "isEmpty" | "bytes" | "byteRegion"
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
            null_safe,
            ..
        } => {
            validate_expr(object, semantic_info)?;
            validate_arguments(args, semantic_info)?;
            let receiver_type = semantic_info.expression_type(object.span());
            if receiver_type.is_some_and(is_stage23_runtime_type)
                && !receiver_type.is_some_and(core_collection::uses_receiver)
                && !matches!(receiver_type, Some(ResolvedType::List(_)) if method == "add")
                && !matches!(receiver_type, Some(ResolvedType::List(_)) if method == "insertAt" && !null_safe)
                && !matches!(receiver_type, Some(ResolvedType::List(_)) if method == "removeAt" && !null_safe)
                && !matches!(receiver_type, Some(ResolvedType::List(_)) if method == "pop" && !null_safe)
                && !matches!(receiver_type, Some(ResolvedType::List(_) | ResolvedType::Dictionary(_, _)) if method == "clear" && !null_safe)
                && !matches!(receiver_type, Some(ResolvedType::Dictionary(_, _)) if method == "get")
                && !matches!(receiver_type, Some(ResolvedType::Dictionary(_, _)) if method == "set" && !null_safe)
                && !matches!(receiver_type, Some(ResolvedType::Dictionary(_, _)) if method == "remove" && !null_safe)
                && !(receiver_type.is_some_and(bytes::uses) && method == "toArray")
                && !receiver_type.is_some_and(|ty| bytes::region_method(ty, method))
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
        Expr::FunctionCall { args, .. } => {
            validate_arguments(args, semantic_info)?;
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
            // UTF-8 validation is exact with PCRE, which every PHP build has.
            if class_name == "String" && method == "fromBytes" && args.len() == 1 {
                return validate_arguments(args, semantic_info);
            }
            if class_name == "String" {
                validate_arguments(args, semantic_info)?;
                return Err(unsupported_string_runtime_shape(
                    *span,
                    format!("Unicode String operation `String::{method}`"),
                ));
            }
            if (class_name == "Bytes"
                && matches!(method.as_str(), "fromArray" | "allocate")
                && args.len() == 1)
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
            // A widening conversion cannot fail and PHP integers hold every
            // Doria integer up to 64 bits, so it is the value itself.
            if php_widening_integer_conversion(class_name, method, args, semantic_info) {
                return validate_arguments(args, semantic_info);
            }
            // A conversion that can fail is checked against the target's
            // range, from any source whose width the compiler knows.
            if IntegerType::from_companion_name(class_name).is_some()
                && method == "from"
                && args.len() == 1
                && integers::ty(semantic_info.expression_type(args[0].value.span())).is_some()
            {
                return validate_arguments(args, semantic_info);
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
                UnaryOp::Negate
                    if integers::ty(semantic_info.expression_type(expr.span())).is_some() =>
                {
                    None
                }
                UnaryOp::Negate => Some("checked integer overflow behavior for unary `-`"),
                UnaryOp::BitwiseNot
                    if integers::ty(semantic_info.expression_type(expr.span())).is_some() =>
                {
                    None
                }
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
                validate_display_type(left, semantic_info)?;
                validate_display_type(right, semantic_info)?;
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
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::BitwiseAnd
                | BinaryOp::BitwiseXor
                | BinaryOp::BitwiseOr
                    if integers::ty(semantic_info.expression_type(left.span())).is_some() =>
                {
                    None
                }
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
    validate_display_type(expr, semantic_info)?;
    validate_expr(expr, semantic_info)
}

fn validate_display_type(
    expr: &Expr,
    semantic_info: &PhpValidation<'_>,
) -> Result<(), BackendError> {
    if semantic_info.float_type(expr.span()).is_some() {
        return Err(unsupported_numeric_shape(
            expr.span(),
            "canonical float display formatting",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn concatenation_validation_visits_each_expression_once() {
        let terms = 32;
        let expression = vec!["\"x\""; terms].join(" . ");
        let source = format!("function main(): void {{ echo {expression}; }}");
        let program = crate::lower_source("concat-visits.doria", source).unwrap();
        let closures = PhpClosurePlan::build(&program, None);
        let plan = specialization::Plan::build(&program, &closures).unwrap();
        let substitutions = HashMap::new();
        let mut context = PhpValidation::new(&program.semantic_info, &plan, &substitutions);
        let expected = terms * 2 - 1;
        // Fail on repeated traversal, without a wall-clock threshold.
        context.expression_visit_limit = Some(expected);
        let function = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Function(function) if function.name == "main" => Some(function),
                _ => None,
            })
            .unwrap();
        let Stmt::Echo { expr, .. } = &function.body.statements[0] else {
            unreachable!();
        };
        validate_display_expr(expr, &context).unwrap();
        assert_eq!(context.expression_visits.get(), expected);
    }
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

fn emit_method_arguments_for_call(
    object: &Expr,
    method: &str,
    arguments: &[Argument],
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    let ingested = scopes
        .expression_types
        .get(&object.span())
        .and_then(crate::types::CollectionFamily::from_resolved)
        .map_or(&[][..], |family| family.ingested_arguments(method));
    if ingested.is_empty() {
        return emit_arguments_for_call(arguments, span, scopes);
    }
    arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            if ingested.contains(&index) {
                emit_owned_expr(&argument.value, scopes)
            } else {
                emit_expr(&argument.value, scopes)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn emit_receiver_borrow_home(object: &Expr, null_safe: bool, scopes: &PhpNameScopes) -> String {
    if scopes
        .expression_types
        .get(&object.span())
        .and_then(shared::kind)
        .is_some()
    {
        format!(
            "{}{}payloadCell()",
            emit_member_receiver(object, scopes),
            if null_safe { "?->" } else { "->" }
        )
    } else {
        if let Some(home) = scopes
            .prepared_receiver
            .as_ref()
            .and_then(|receiver| receiver.cell(object))
        {
            return project_borrowed_argument_cell(
                object,
                normalize_borrowed_result_cell(object, home),
                scopes,
            );
        }
        borrowed_argument_cell(object, scopes).unwrap_or_else(|| {
            core_collection::temporary_cell(
                emit_owned_expr(object, scopes),
                object.span(),
                "__doria_receiver_",
                scopes,
            )
        })
    }
}

fn emit_method_with_receiver_home(
    object: &Expr,
    method: &str,
    arguments: impl FnOnce() -> String,
    span: Span,
    null_safe: bool,
    scopes: &PhpNameScopes,
) -> String {
    let home = emit_receiver_borrow_home(object, null_safe, scopes);
    let receiver = scopes.expression_temp("__doria_receiver_home_", span);
    scopes.track_expression_temporary(PhpExpressionTemporary::Alias(receiver.clone()));
    let arguments = arguments();
    let operator = if null_safe { "?->" } else { "->" };
    format!(
        "(${receiver} = {home}){operator}value{operator}{method}(${receiver}{})",
        if arguments.is_empty() {
            String::new()
        } else {
            format!(", {arguments}")
        }
    )
}

fn emit_call_argument_values(
    arguments: &[Argument],
    span: Span,
    scopes: &PhpNameScopes,
) -> Vec<String> {
    if let Some((active, values)) = &scopes.field_arguments {
        if *active == span {
            return values.clone();
        }
    }
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
            let value = emit_call_argument(&argument.value, parameter, scopes);
            match &argument.name {
                Some(name) => format!("{}: {value}", name.text),
                None => value,
            }
        })
        .collect()
}

fn emit_call_argument(
    value: &Expr,
    parameter: Option<&crate::php_closure::PhpCallableParameter>,
    scopes: &PhpNameScopes,
) -> String {
    value_argument::Input::named(value, parameter, scopes).expression(value, scopes)
}

fn stage_owned_argument_cell(span: Span, emitted: String, scopes: &PhpNameScopes) -> String {
    let temporary = scopes.expression_temp("__doria_argument_claim_", span);
    scopes.track_expression_temporary(PhpExpressionTemporary::Owned(temporary.clone()));
    format!("(${temporary} = {emitted})")
}

/// Stage an owned receiver before arguments. Until invocation begins, an
/// argument's checked failure must release that receiver in the caller.
fn emit_consuming_method_call(
    receiver: String,
    method: &str,
    arguments: &[Argument],
    span: Span,
    null_safe: bool,
    parent: bool,
    scopes: &PhpNameScopes,
) -> String {
    let plan = scopes
        .closure_plan
        .callable_at(span)
        .expect("checked consuming call");
    let receiver_name = scopes.expression_temp("__doria_consumed_receiver_", span);
    let mut body = format!("${receiver_name} = {receiver}; ");
    if null_safe {
        body.push_str(&format!(
            "if (${receiver_name} === null) {{ return null; }} "
        ));
    }
    let mut cleanup = vec![receiver_name.clone()];
    let mut passed = Vec::new();
    body.push_str("try { ");
    for (index, argument) in arguments.iter().enumerate() {
        let parameter = argument
            .name
            .as_ref()
            .and_then(|name| {
                plan.parameters
                    .iter()
                    .find(|parameter| parameter.name == name.text)
            })
            .or_else(|| plan.parameters.get(index));
        let name = scopes.expression_temp(&format!("__doria_consumed_argument_{index}_"), span);
        let value = emit_call_argument(&argument.value, parameter, scopes);
        body.push_str(&format!("${name} = {value}; "));
        if parameter.is_some_and(|parameter| parameter.take) {
            cleanup.push(name.clone());
        }
        passed.push(match &argument.name {
            Some(label) => format!("{}: ${name}", label.text),
            None => format!("${name}"),
        });
    }
    let started = scopes.expression_temp("__doria_consuming_started_", span);
    body.push_str(&format!("${started} = true; return "));
    if parent {
        body.push_str(&format!("parent::{method}({}); ", passed.join(", ")));
    } else {
        body.push_str(&format!(
            "${receiver_name}->{method}({}); ",
            passed.join(", ")
        ));
    }
    body.push_str(&format!("}} finally {{ if (!isset(${started})) {{ "));
    for name in cleanup.iter().rev() {
        body.push_str(&format!(
            "if (isset(${name})) {{ __doria_drop_value(${name}); }} "
        ));
    }
    body.push_str("} }");
    emit_expression_body(scopes, &body, &[])
}

fn is_payload_enum_expression(expr: &Expr, semantic_info: &SemanticInfo) -> bool {
    let Some(ResolvedType::Enum(enum_type)) = semantic_info.expression_type(expr.span()) else {
        return false;
    };
    semantic_info
        .enums
        .iter()
        .find(|definition| definition.source_type.id == enum_type.id)
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

fn php_widening_integer_conversion(
    class_name: &str,
    method: &str,
    args: &[Argument],
    semantic_info: &SemanticInfo,
) -> bool {
    let Some(target) = IntegerType::from_companion_name(class_name) else {
        return false;
    };
    method == "from"
        && args.len() == 1
        && integers::ty(semantic_info.expression_type(args[0].value.span())).is_some_and(|source| {
            target.min_value() <= source.min_value() && source.max_value() <= target.max_value()
        })
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

fn php_collection_property(ty: &ResolvedType, property: &str) -> bool {
    match ty {
        ResolvedType::Bytes | ResolvedType::ByteRegion(_) | ResolvedType::TypedArray(_) => {
            property == "length"
        }
        ResolvedType::List(_) | ResolvedType::Set(_) => {
            matches!(property, "count" | "isEmpty" | "first" | "last")
        }
        ResolvedType::Dictionary(_, _) => {
            matches!(property, "count" | "isEmpty")
        }
        ResolvedType::Nullable(inner) => php_collection_property(inner, property),
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

fn php_builtin_value_equality(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Bytes => true,
        ResolvedType::Class(class) => class.inline_copy_type().is_some(),
        ResolvedType::Nullable(inner) => php_builtin_value_equality(inner),
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
    move_enum_names: crate::ownership::MoveEnums,
}

#[derive(Debug, Clone)]
struct PhpNameScopes {
    diagnostic_sites: Rc<RefCell<diagnostic_sites::Sites>>,
    core_helpers: Rc<RefCell<core_value::Helpers>>,
    scopes: Vec<HashMap<String, String>>,
    mixed_bindings: Vec<HashSet<String>>,
    used_php_names: HashSet<String>,
    next_mangled_id: Rc<Cell<usize>>,
    next_label_id: Rc<Cell<usize>>,
    symbols: PhpScopeSymbols,
    payload_case_tags: HashMap<crate::enums::EnumCaseSource, u32>,
    matches: HashMap<Span, MatchSemanticInfo>,
    whens: HashMap<Span, WhenSemanticInfo>,
    given_preludes: HashMap<Span, GivenSemanticInfo>,
    expression_types: HashMap<Span, ResolvedType>,
    flow_facts: Rc<crate::narrowing::FactsByUse>,
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
    expression_inputs: Vec<String>,
    current_callable: Option<String>,
    compiler_owned_callable: bool,
    /// The current specialization's executable effects, when it has its own.
    executable_effects:
        Option<std::sync::Arc<crate::semantics::executable_effects::InstanceEffects>>,
    current_class: Option<String>,
    closure_owner: Option<mir::ClosureOwner>,
    current_class_id: Option<crate::class_layout::ClassId>,
    file_payload: bool,
    readiness_set_payload: bool,
    specialization: Rc<specialization::Plan>,
    substitutions: HashMap<String, ResolvedType>,
    // The enum specialization whose PHP class is being emitted.
    enum_instance: Option<crate::enums::EnumType>,
    returns_borrow: bool,
    returns_borrow_cell: bool,
    expression_temporaries: Option<Rc<RefCell<Vec<PhpExpressionTemporary>>>>,
    expression_region: Option<Rc<RefCell<expression_region::Root>>>,
    yield_temporaries: Option<Rc<RefCell<Vec<PhpExpressionTemporary>>>>,
    pending_returns: Vec<Rc<RefCell<Vec<String>>>>,
    value_return: Option<value_result::Return>,
    field_call: Option<Span>,
    field_arguments: Option<(Span, Vec<String>)>,
    prepared_call: Option<Rc<value_argument::Prepared>>,
    prepared_receiver: Option<Rc<value_argument::Receiver>>,
    scalar_operands: HashMap<*const Expr, String>,
}

#[derive(Debug, Clone)]
enum PhpBindingPlace {
    Direct(String),
    Cell(String),
    Fields(value_result::Place),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PhpExpressionTemporary {
    Owned(String),
    Alias(String),
    Pending { name: String, started: String },
}

impl PhpExpressionTemporary {
    fn name(&self) -> &str {
        match self {
            Self::Owned(name) | Self::Alias(name) | Self::Pending { name, .. } => name,
        }
    }

    fn cleanup(&self) -> String {
        let name = self.name();
        match self {
            Self::Owned(_) => {
                format!("if (isset(${name})) {{ __doria_drop_value(${name}); unset(${name}); }}")
            }
            Self::Alias(_) => format!("unset(${name});"),
            Self::Pending { started, .. } => format!(
                "if (isset(${name}) && !(${started} ?? false)) {{ __doria_drop_value(${name}); }} unset(${name});"
            ),
        }
    }
}

impl PhpBindingPlace {
    fn read(&self) -> String {
        match self {
            Self::Direct(name) => format!("${name}"),
            Self::Cell(name) => format!("${name}->value"),
            Self::Fields(place) => place.read(),
        }
    }

    fn cell(&self) -> Option<String> {
        match self {
            Self::Cell(name) => Some(format!("${name}")),
            Self::Direct(_) | Self::Fields(_) => None,
        }
    }
}

impl PhpNameScopes {
    /// Whether an operation can raise in this specialization, which decides
    /// whether it needs an origin guard.
    fn has_checked_effects(&self, span: Span) -> bool {
        match self
            .executable_effects
            .as_ref()
            .and_then(|effects| effects.site(span))
        {
            Some(effects) => !effects.is_empty(),
            None => self.specialization.has_checked_effects(span),
        }
    }

    fn new(symbols: PhpScopeSymbols, closure_plan: Rc<PhpClosurePlan>) -> Self {
        Self {
            diagnostic_sites: Rc::default(),
            core_helpers: Rc::default(),
            scopes: vec![HashMap::new()],
            mixed_bindings: vec![HashSet::new()],
            used_php_names: HashSet::new(),
            next_mangled_id: Rc::new(Cell::new(0)),
            next_label_id: Rc::new(Cell::new(0)),
            symbols,
            payload_case_tags: HashMap::new(),
            matches: HashMap::new(),
            whens: HashMap::new(),
            given_preludes: HashMap::new(),
            expression_types: HashMap::new(),
            flow_facts: Rc::default(),
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
            expression_inputs: Vec::new(),
            current_callable: None,
            compiler_owned_callable: false,
            executable_effects: None,
            current_class: None,
            closure_owner: None,
            current_class_id: None,
            file_payload: false,
            readiness_set_payload: false,
            specialization: Rc::default(),
            substitutions: HashMap::new(),
            enum_instance: None,
            returns_borrow: false,
            returns_borrow_cell: false,
            expression_temporaries: None,
            expression_region: None,
            yield_temporaries: None,
            pending_returns: Vec::new(),
            value_return: None,
            field_call: None,
            field_arguments: None,
            prepared_call: None,
            prepared_receiver: None,
            scalar_operands: HashMap::new(),
        }
    }

    fn expression_scope(&self) -> Self {
        let mut scopes = Self::new(self.symbols.clone(), Rc::clone(&self.closure_plan));
        scopes.diagnostic_sites = Rc::clone(&self.diagnostic_sites);
        scopes.core_helpers = Rc::clone(&self.core_helpers);
        scopes.payload_case_tags = self.payload_case_tags.clone();
        scopes.matches = self.matches.clone();
        scopes.whens = self.whens.clone();
        scopes.given_preludes = self.given_preludes.clone();
        scopes.expression_types = self.expression_types.clone();
        scopes.flow_facts = Rc::clone(&self.flow_facts);
        scopes.file_payload = self.file_payload;
        scopes.readiness_set_payload = self.readiness_set_payload;
        scopes.interface_conversion_types = self.interface_conversion_types.clone();
        scopes.type_test_types = self.type_test_types.clone();
        scopes.mixed_box_plans = self.mixed_box_plans.clone();
        scopes.throw_error_types = self.throw_error_types.clone();
        scopes.catch_error_types = self.catch_error_types.clone();
        scopes.direct_parent_calls = self.direct_parent_calls.clone();
        scopes.const_evaluation = self.const_evaluation.clone();
        scopes.current_callable = self.current_callable.clone();
        scopes.compiler_owned_callable = self.compiler_owned_callable;
        scopes.executable_effects = self.executable_effects.clone();
        scopes.current_class = self.current_class.clone();
        scopes.closure_owner = self.closure_owner;
        scopes.current_class_id = self.current_class_id;
        scopes.specialization = Rc::clone(&self.specialization);
        scopes.substitutions = self.substitutions.clone();
        scopes.returns_borrow = self.returns_borrow;
        scopes.returns_borrow_cell = self.returns_borrow_cell;
        scopes
    }

    fn call_symbol(&self, span: Span) -> Option<&str> {
        if self
            .field_arguments
            .as_ref()
            .is_some_and(|(active, _)| *active == span)
        {
            let symbol = self.specialization.call_symbol(span, &self.substitutions)?;
            Some(
                self.specialization
                    .argument_symbol(symbol, self.field_call == Some(span)),
            )
        } else if self.field_call == Some(span) {
            self.specialization
                .field_call_symbol(span, &self.substitutions)
        } else {
            self.specialization.call_symbol(span, &self.substitutions)
        }
    }

    fn transport_symbol<'a>(&'a self, span: Span, symbol: &'a str) -> &'a str {
        if self
            .field_arguments
            .as_ref()
            .is_some_and(|(active, _)| *active == span)
        {
            self.specialization
                .argument_symbol(symbol, self.field_call == Some(span))
        } else if self.field_call == Some(span) {
            self.specialization.result_symbol(symbol)
        } else {
            symbol
        }
    }

    fn source_location(&self, span: Span, offset: usize) -> u64 {
        self.diagnostic_sites.borrow_mut().register(span);
        php_source_location(span, offset)
    }

    fn checked_origin(&self, span: Span) -> usize {
        self.diagnostic_sites.borrow_mut().register(span)
    }

    fn is_static_property(&self, class_name: &str, member: &str) -> bool {
        self.symbols
            .static_properties
            .contains(&(class_name.to_string(), member.to_string()))
    }

    fn static_member_class_symbol(&self, class_name: &str, span: Span) -> String {
        self.specialization
            .class_symbol_for_static_member(span, &self.substitutions)
            .map(str::to_string)
            .or_else(|| {
                self.specialization
                    .enum_case_symbol(span, &self.substitutions)
            })
            .unwrap_or_else(|| php_type_symbol(class_name))
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
                    && declaration
                        .span
                        .is_some_and(|declared| span.contains(declared))
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
            let id = self.next_mangled_id.get() + 1;
            self.next_mangled_id.set(id);
            let candidate = format!("{prefix}__doria{id}");
            if !self.used_php_names.contains(&candidate) {
                self.used_php_names.insert(candidate.clone());
                return candidate;
            }
        }
    }

    fn fresh_label(&self, prefix: &str) -> String {
        let id = self.next_label_id.get();
        self.next_label_id.set(id + 1);
        format!("{prefix}__doria{id}")
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

    fn track_expression_temporary(&self, temporary: PhpExpressionTemporary) {
        let mut temporaries = self
            .expression_temporaries
            .as_ref()
            .expect("expression temporary has a cleanup scope")
            .borrow_mut();
        if let Some(previous) = temporaries
            .iter()
            .find(|item| item.name() == temporary.name())
        {
            assert_eq!(previous, &temporary, "temporary ownership must be stable");
        } else {
            temporaries.push(temporary);
        }
    }

    fn captured_php_names(&self) -> Vec<String> {
        let mut names = self
            .scopes
            .iter()
            .flat_map(|scope| scope.values().cloned())
            .chain(self.expression_inputs.iter().cloned())
            .collect::<HashSet<_>>();
        for place in self.binding_places.iter().flat_map(HashMap::values) {
            if let PhpBindingPlace::Fields(place) = place {
                names.extend(place.names().map(str::to_string));
            }
        }
        if let Some(temporaries) = &self.expression_temporaries {
            names.extend(
                temporaries
                    .borrow()
                    .iter()
                    .map(|item| item.name().to_string()),
            );
        }
        let mut names = names.into_iter().collect::<Vec<_>>();
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
            let id = self.next_mangled_id.get() + 1;
            self.next_mangled_id.set(id);
            let candidate = format!("{name}__doria{id}");
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
        Item::Enum(enum_decl) => emit_enum_specializations(enum_decl, output, indent, scopes),
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
            &scopes.specialization,
            output,
            indent,
        ),
        Item::Statement(statement) => emit_statement(statement, output, indent, scopes),
    }
}

/// A generic declaration emits one PHP class per concrete specialization, so
/// runtime type identity, `mixed` tags and payload layout stay per application.
fn emit_enum_specializations(
    enum_decl: &EnumDecl,
    output: &mut String,
    indent: usize,
    scopes: &PhpNameScopes,
) {
    if enum_decl.type_params.is_empty() {
        emit_enum(enum_decl, output, indent, scopes);
        return;
    }
    for instance in scopes.specialization.enum_specializations(&enum_decl.name) {
        let mut inner = scopes.clone();
        inner.substitutions = enum_decl
            .type_params
            .iter()
            .map(|parameter| parameter.name.clone())
            .zip(instance.arguments.iter().cloned())
            .collect();
        inner.enum_instance = Some(instance);
        emit_enum(enum_decl, output, indent, &inner);
    }
}

fn enum_class_symbol(enum_decl: &EnumDecl, scopes: &PhpNameScopes) -> String {
    scopes.enum_instance.as_ref().map_or_else(
        || php_type_symbol(&enum_decl.name),
        |instance| scopes.specialization.enum_symbol(instance),
    )
}

fn emit_enum(enum_decl: &EnumDecl, output: &mut String, indent: usize, scopes: &PhpNameScopes) {
    if enum_decl.cases.iter().any(|case| !case.payload.is_empty()) {
        emit_payload_enum(enum_decl, output, indent, scopes);
        return;
    }
    write_indent(output, indent);
    output.push_str("enum ");
    output.push_str(&enum_class_symbol(enum_decl, scopes));
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
        ", __DoriaOwnedObject, __DoriaManagedCopy"
    } else {
        ""
    };
    writeln(
        output,
        indent,
        &format!(
            "final class {} implements __DoriaValueEquatable{owned_marker}",
            enum_class_symbol(enum_decl, scopes)
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

    for line in [
        "public function __doriaCopy(): self",
        "{",
        "    $payload = [];",
        "    foreach ($this->__doriaPayload as $field) { $payload[] = __doria_copy_value($field); }",
        "    return new self($this->__doriaTag, $payload);",
        "}",
        "public function __doriaRelease(): void { $this->__destruct(); }",
    ] {
        writeln(output, indent + 1, line);
    }

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

    value_result::storage::emit_boxed_writer(enum_decl, output, indent + 1, scopes);
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
        if !value_result::emit_boxed_factory(enum_decl, case, tag, output, indent + 2, scopes) {
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
        }
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
    value_result::emit_writers(enum_decl, output, indent, scopes);
    value_result::storage::emit_cell(enum_decl, output, indent, scopes);
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
        let mut scopes =
            if crate::inline_value::InlineValueKind::for_name(&class_decl.name).is_some() {
                scopes.expression_scope()
            } else {
                let constructor = scopes
                    .specialization
                    .callables
                    .iter()
                    .find(|callable| {
                        callable.class == Some(class.id) && callable.symbol == "__construct"
                    })
                    .expect("each owned class has a checked constructor instance");
                scopes.specialization.scope(scopes, constructor)
            };
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
    if concurrency::emit_token_class(class_decl, output, indent) {
        return;
    }
    let mut class_scopes = scopes.clone();
    class_scopes.current_class = Some(class_decl.name.clone());
    class_scopes.file_payload = streams::has_payload(class_decl);
    class_scopes.readiness_set_payload = streams::has_set_payload(class_decl);
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
                    && property.has_storage()
                    && !scopes.current_class_id.is_some_and(|class| {
                        scopes
                            .specialization
                            .property_storage_symbols(class, &property.name)
                            .is_some()
                    })
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
                        && !scopes.current_class_id.is_some_and(|class| {
                            scopes
                                .specialization
                                .property_storage_symbols(class, &parameter.name)
                                .is_some()
                        })
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
                if property_hooks::has_runtime_initializer(
                    property,
                    &class_decl.name,
                    semantic_info,
                    &scopes.specialization,
                ) =>
            {
                Some(property)
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
    if crate::inline_value::InlineValueKind::for_name(&class_decl.name).is_some() {
        interfaces.push("__DoriaValueEquatable".to_string());
    }
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
    if let Some(inline) = crate::inline_value::InlineValueKind::for_name(&class_decl.name) {
        writeln(
            output,
            indent + 1,
            "private readonly int $__doriaInlineBits;",
        );
        if inline.storage_bytes() > 8 {
            writeln(
                output,
                indent + 1,
                "private readonly int $__doriaInlineHighBits;",
            );
        }
        let parameters = if inline.storage_bytes() > 8 {
            "int $__doriaInlineBits, int $__doriaInlineHighBits"
        } else {
            "int $__doriaInlineBits"
        };
        writeln(
            output,
            indent + 1,
            &format!("private function __construct({parameters})"),
        );
        writeln(output, indent + 1, "{");
        writeln(
            output,
            indent + 2,
            "$this->__doriaInlineBits = $__doriaInlineBits;",
        );
        if inline.storage_bytes() > 8 {
            writeln(
                output,
                indent + 2,
                "$this->__doriaInlineHighBits = $__doriaInlineHighBits;",
            );
        }
        writeln(output, indent + 1, "}");
        if !inline.requires_owned_cleanup() {
            for line in [
                "public function __doriaLowWord(): int",
                "{",
                "    return $this->__doriaInlineBits;",
                "}",
                "public function __doriaHighWord(): int",
                "{",
                if inline.storage_bytes() > 8 {
                    "    return $this->__doriaInlineHighBits;"
                } else {
                    "    return 0;"
                },
                "}",
            ] {
                writeln(output, indent + 1, line);
            }
        }
        writeln(
            output,
            indent + 1,
            "public function __doriaEquals(mixed $other): bool",
        );
        writeln(output, indent + 1, "{");
        let high = if inline.storage_bytes() > 8 {
            " && $this->__doriaInlineHighBits === $other->__doriaInlineHighBits"
        } else {
            ""
        };
        writeln(
            output,
            indent + 2,
            &format!(
                "return $other instanceof self && $this->__doriaInlineBits === $other->__doriaInlineBits{high};"
            ),
        );
        writeln(output, indent + 1, "}");
        if inline == crate::inline_value::InlineValueKind::RegistrationKey {
            streams::emit_registration_key_members(output, indent + 1);
        } else if matches!(
            inline,
            crate::inline_value::InlineValueKind::StandardInput
                | crate::inline_value::InlineValueKind::StandardOutput
                | crate::inline_value::InlineValueKind::StandardError
        ) {
            for line in [
                "public static function __doriaStandardView(): self",
                "{",
                "    static $view = null;",
                "    return $view ??= new self(0);",
                "}",
            ] {
                writeln(output, indent + 1, line);
            }
        } else if matches!(
            inline,
            crate::inline_value::InlineValueKind::ReadTransferKey
                | crate::inline_value::InlineValueKind::WriteTransferKey
        ) {
            for line in [
                "public static function __doriaFromTransferKey(int $low, int $high): self",
                "{",
                "    return new self($low, $high);",
                "}",
                "public function __doriaNativeTransferKey(): FFI\\CData",
                "{",
                "    $key = __DoriaNativeIo::get()->new('int64_t[2]');",
                "    $this->__doriaWriteNativeTransferKey($key);",
                "    return $key;",
                "}",
                "public function __doriaWriteNativeTransferKey(FFI\\CData $key): void",
                "{",
                "    $key[0] = $this->__doriaInlineBits;",
                "    $key[1] = $this->__doriaInlineHighBits;",
                "}",
            ] {
                writeln(output, indent + 1, line);
            }
        }
    }
    if concurrency::has_source_payload(class_decl) {
        concurrency::emit_source_payload(output, indent + 1);
    } else if scopes.file_payload {
        streams::emit_payload_members(output, indent + 1);
    } else if streams::has_child_payload(class_decl) {
        streams::emit_child_payload_members(output, indent + 1);
    } else if streams::has_set_payload(class_decl) {
        streams::emit_set_payload_members(output, indent + 1);
    } else if streams::has_plan_payload(class_decl) {
        streams::emit_plan_payload_members(output, indent + 1);
    } else if streams::has_group_payload(class_decl) {
        streams::emit_group_members(output, indent + 1);
    } else if streams::has_readiness_payload(class_decl) {
        streams::emit_readiness_payload_members(output, indent + 1);
    } else if streams::has_request_payload(class_decl) {
        streams::emit_request_payload_members(output, indent + 1);
    }
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
                "return __doria_error_descriptor({}, {});",
                emit_php_string_literal(&class_decl.name),
                if class_decl.name == crate::compiler_known_io::IO_ERROR {
                    "true"
                } else {
                    "false"
                },
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
        let manual_promotions = constructor_uses_manual_promotions(class_decl, method);
        for (index, parameter) in method.params.iter().enumerate() {
            let Some(access) = parameter.constructor_role.promoted_access() else {
                continue;
            };
            if !manual_promotions && !callable_parameter_uses_cell(callable_plan, index) {
                continue;
            }
            if value_result::storage::emit_property(
                &parameter.name,
                &parameter.ty,
                &access,
                method.span.source,
                output,
                indent + 1,
                scopes,
            ) {
                continue;
            }
            writeln(
                output,
                indent + 1,
                &format!(
                    "{} {} ${};",
                    emit_member_access(&access, method.span.source),
                    php_type(&parameter.ty, scopes),
                    parameter.name
                ),
            );
        }
    }
    for member in &class_decl.members {
        match member {
            ClassMember::Property(property) => {
                if property.has_storage() {
                    emit_property(
                        property,
                        &class_decl.name,
                        &semantic_info.const_evaluation,
                        semantic_info,
                        output,
                        indent + 1,
                        scopes,
                    );
                }
                if let Some(hooks) = &property.hooks {
                    for accessor in &hooks.accessors {
                        emit_function(
                            &accessor.function,
                            semantic_info,
                            output,
                            indent + 1,
                            scopes,
                            PhpFunctionEmission {
                                is_method: true,
                                ..Default::default()
                            },
                        );
                    }
                }
            }
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
                        exact_method: false,
                        property_initializers: initializers,
                        invoke_parent_constructor: parent_has_constructor
                            && emitted_method.name == "__construct",
                        invoke_parent_destructor: parent_has_destructor
                            && emitted_method.name == "__destruct",
                        manual_promotions: constructor_uses_manual_promotions(
                            class_decl,
                            &emitted_method,
                        ),
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
                &scopes.specialization,
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
        let frame_body_start = output.len();
        if let Some(parent_completed) = &parent_completed {
            writeln(output, indent + 2, &format!("${parent_completed} = false;"));
        }
        if cleanup {
            writeln(output, indent + 2, "try {");
        }
        let body_indent = indent + 2 + usize::from(cleanup);
        if parent_has_constructor {
            writeln(output, body_indent, "parent::__construct();");
        }
        // A parent phase with no executable PHP constructor still completes
        // before this phase starts. A throwing parent call never reaches here.
        if let Some(parent_completed) = &parent_completed {
            writeln(output, body_indent, &format!("${parent_completed} = true;"));
        }
        for property in &instance_initializers {
            property_hooks::emit_initializer(
                property,
                output,
                body_indent,
                &mut constructor_scopes,
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
        let Some(mir::ClosureOwner::Callable(constructor)) = constructor_scopes.closure_owner
        else {
            unreachable!("generated constructor has a checked concrete callable instance");
        };
        let constructor_span = constructor_scopes
            .specialization
            .callable(constructor)
            .declaration;
        let frame_body = output.split_off(frame_body_start);
        diagnostic_sites::emit_frame_body(
            output,
            &frame_body,
            indent + 1,
            constructor_span,
            &constructor_scopes,
        );
        writeln(output, indent + 1, "}");
        output.push('\n');
    }
    // A consuming field take releases the rest of its receiver without the
    // receiver's own destructor: disarm it, then drop the owned fields.
    if scopes
        .symbols
        .classes_with_php_destructors
        .contains(&class_decl.name)
        && !concurrency::has_source_payload(class_decl)
        && !scopes.file_payload
        && !streams::uses_child_payload(scopes)
        && !streams::uses_set_payload(scopes)
        && !streams::uses_plan_payload(scopes)
        && !streams::uses_group_payload(scopes)
    {
        writeln(
            output,
            indent + 1,
            "public function __doriaReleaseTaken(): void",
        );
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
        if concurrency::has_source_payload(class_decl) {
            concurrency::emit_source_drop(output, indent + 2);
        } else if scopes.file_payload {
            writeln(output, indent + 2, "$this->__doriaFileOwner->drop();");
            writeln(output, indent + 2, "unset($this->__doriaFileOwner);");
        } else if streams::uses_child_payload(scopes) {
            streams::emit_child_drop(output, indent + 2);
        } else if streams::uses_set_payload(scopes) {
            streams::emit_set_drop(output, indent + 2);
        } else if streams::uses_plan_payload(scopes) {
            streams::emit_plan_drop(output, indent + 2);
        } else if streams::uses_group_payload(scopes) {
            writeln(output, indent + 2, "$this->__doriaTransferGroup->close();");
            writeln(output, indent + 2, "unset($this->__doriaTransferGroup);");
        }
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
    scopes
        .specialization
        .emit_dispatch_bridges(output, indent + 1, scopes);
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
                        &scopes.specialization,
                    )
                ),
            );
        }
        writeln(
            output,
            indent,
            &format!(
                "}}, null, {}::class))();",
                php_type_symbol(&class_decl.name)
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
    scopes.returns_borrow = shared_scopes
        .closure_plan
        .function_type(descriptor.function_type)
        .return_borrow
        .is_some_and(|borrow| borrow.kind == mir::ReturnBorrowKind::Value);
    scopes.returns_borrow_cell =
        scopes.returns_borrow && borrowed_result_uses_cell(&semantic.inferred_return_type);
    scopes.push();

    write_indent(output, indent);
    if descriptor.owner_class.is_some() {
        output.push_str("public static ");
    }
    output.push_str("function ");
    output.push_str(&descriptor.helper_name);
    output.push('(');
    let mut parameters = Vec::new();
    let environment_parameter = descriptor
        .environment_name
        .as_ref()
        .map(|environment_name| {
            scopes.used_php_names.extend(
                closure
                    .parameters
                    .iter()
                    .map(|parameter| parameter.name.clone()),
            );
            let name = scopes.fresh_temp("__doriaEnvironment");
            scopes.expression_inputs.push(name.clone());
            parameters.push(format!("{environment_name} ${name}"));
            name
        });
    let field_types = scopes
        .specialization
        .function_argument_fields(scopes.closure_plan.function_type(descriptor.function_type));
    let field_result = scopes
        .specialization
        .function_field_result(scopes.closure_plan.function_type(descriptor.function_type));
    let mut field_places = Vec::new();
    for (index, parameter) in closure.parameters.iter().enumerate() {
        let binding = scopes.binding_for_declaration(&parameter.name, parameter.span);
        let fields = field_types[index]
            .as_ref()
            .map(|ty| value_result::Place::parameter(ty, index, &mut scopes));
        let uses_cell = scopes
            .closure_plan
            .parameter_uses_cell(descriptor.function_type, index);
        let parameter_type = if uses_cell {
            "__DoriaCell".to_string()
        } else {
            php_type(&parameter.ty, &scopes)
        };
        if let Some(fields) = &fields {
            parameters.extend(fields.declarations());
        } else {
            parameters.push(format!("{parameter_type} ${}", parameter.name));
        }
        scopes.declare_unmangled(&parameter.name);
        if let Some(binding) = binding {
            let place = if uses_cell || scopes.needs_cell(binding) {
                PhpBindingPlace::Cell(parameter.name.clone())
            } else if let Some(fields) = &fields {
                PhpBindingPlace::Fields(fields.clone())
            } else {
                PhpBindingPlace::Direct(parameter.name.clone())
            };
            scopes.bind_place(binding, place);
        }
        field_places.push(fields);
    }
    output.push_str(&parameters.join(", "));
    output.push_str("): ");
    if field_result.is_some() {
        output.push_str("void");
    } else if scopes.returns_borrow_cell {
        output.push_str("__DoriaCell");
    } else {
        output.push_str(&php_resolved_type(&semantic.inferred_return_type, &scopes));
    }
    output.push('\n');
    writeln(output, indent, "{");
    if let Some(layout_id) = descriptor.environment_layout {
        let fields = scopes.closure_plan.layout(layout_id).fields.clone();
        for field in fields {
            scopes.bind_place(
                field.environment_binding,
                PhpBindingPlace::Cell(format!(
                    "{}->field{}",
                    environment_parameter
                        .as_ref()
                        .expect("capturing closure must have an environment parameter"),
                    field.id.0
                )),
            );
        }
    }
    let frame_body_start = output.len();
    if let Some(ty) = &field_result {
        scopes.value_return = Some(value_result::Return::new(ty, &mut scopes));
    }
    for (index, parameter) in closure.parameters.iter().enumerate() {
        let Some(binding) = scopes.binding_for_declaration(&parameter.name, parameter.span) else {
            continue;
        };
        let uses_cell = scopes
            .closure_plan
            .parameter_uses_cell(descriptor.function_type, index);
        if scopes.needs_cell(binding)
            && !uses_cell
            && !value_result::storage::initialize_parameter(
                &parameter.name,
                binding,
                field_places[index].as_ref(),
                output,
                indent + 1,
                &mut scopes,
            )
        {
            let value = field_places[index]
                .as_ref()
                .map_or_else(|| format!("${}", parameter.name), value_result::Place::read);
            writeln(
                output,
                indent + 1,
                &format!("${} = new __DoriaCell({value});", parameter.name),
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
            ClosureBody::Expression(expr)
                if semantic.inferred_return_type == ResolvedType::Void =>
            {
                emit_statement(
                    &Stmt::Expr {
                        expr: *expr.clone(),
                        span: expr.span(),
                    },
                    output,
                    indent,
                    scopes,
                );
                writeln(output, indent, "return;");
            }
            ClosureBody::Expression(expr) => {
                if value_result::emit_return(expr, output, indent, scopes) {
                    return;
                }
                let result = scopes.fresh_temp("__doria_closure_result");
                let owns_result = returned_value_owns_carrier(
                    &semantic.inferred_return_type,
                    scopes.returns_borrow,
                    scopes,
                );
                writeln(
                    output,
                    indent,
                    &format!(
                        "${result} = {};",
                        if owns_result {
                            emit_owned_expr(expr, scopes)
                        } else {
                            emit_returned_expr(expr, scopes)
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
    value_result::emit_return_epilogue(output, indent + 1, &scopes);
    scopes.pop();
    let frame_body = output.split_off(frame_body_start);
    diagnostic_sites::emit_frame_body(output, &frame_body, indent, closure.span, &scopes);
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
    if !property.is_static
        && value_result::storage::emit_property(
            &property.name,
            &property.ty,
            &property.access,
            property.span.source,
            output,
            indent,
            shared_scopes,
        )
    {
        return;
    }
    let visibility = emit_member_access(&property.access, property.span.source);
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
            property_hooks::has_runtime_initializer(
                property,
                class_name,
                semantic_info,
                &shared_scopes.specialization,
            )
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
                &shared_scopes.specialization,
            ));
        } else if !runtime_initializer {
            output.push_str(
                &property_hooks::constant_default(initializer, semantic_info)
                    .expect("admitted PHP constant property default")
                    .emit(shared_scopes),
            );
        }
    }
    output.push_str(";\n");
}

fn emit_constant(
    constant: &ConstDecl,
    class_name: Option<&str>,
    evaluation: &Evaluation,
    plan: &specialization::Plan,
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
        let (definition, _) = checked_const_enum(&payload.value, plan);
        write_indent(output, indent);
        if class_name.is_some() {
            output.push_str(emit_member_access(&constant.access, constant.span.source));
            output.push(' ');
        }
        output.push_str(if class_name.is_some() {
            "static function "
        } else {
            "function "
        });
        output.push_str(&class_name.map_or_else(
            || format!("__doria_const_{}", php_constant_symbol(&constant.name)),
            |_| format!("__doriaConst{}", constant.name),
        ));
        output.push_str("(): ");
        output.push_str(&plan.enum_symbol(&definition.source_type));
        output.push('\n');
        writeln(output, indent, "{");
        writeln(output, indent + 1, "static $value = null;");
        writeln(
            output,
            indent + 1,
            &format!("return $value ??= {};", emit_const_value(value, plan)),
        );
        writeln(output, indent, "}");
        return;
    }
    write_indent(output, indent);
    if class_name.is_some() {
        output.push_str(emit_member_access(&constant.access, constant.span.source));
        output.push(' ');
    }
    output.push_str("const ");
    output.push_str(&class_name.map_or_else(
        || php_top_level_constant_name(&constant.name),
        |_| constant.name.clone(),
    ));
    output.push_str(" = ");
    output.push_str(&emit_const_value(value, plan));
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

fn checked_const_enum<'a>(
    value: &crate::const_eval::ConstEnumValue,
    plan: &'a specialization::Plan,
) -> (
    &'a crate::semantics::EnumSemanticInfo,
    &'a crate::semantics::EnumCaseSemanticInfo,
) {
    let definition = plan
        .enum_definition_for_type(&value.enum_type)
        .expect("checked enum constant has a concrete application");
    let case = definition
        .resolve_typed_case(&value.enum_type, value.case)
        .expect("checked enum constant belongs to its concrete application");
    let case = definition
        .cases
        .iter()
        .find(|candidate| candidate.id == case)
        .expect("checked enum constant names a declared case");
    (definition, case)
}

fn emit_const_value(value: &ConstValue, plan: &specialization::Plan) -> String {
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
        ConstValue::Enum(value) => {
            let (definition, case) = checked_const_enum(value, plan);
            format!(
                "{}::{}",
                plan.enum_symbol(&definition.source_type),
                case.name
            )
        }
        ConstValue::PayloadEnum(value) => {
            let (definition, case) = checked_const_enum(&value.value, plan);
            assert_eq!(
                value.fields.len(),
                case.payload.len(),
                "checked enum constant payload"
            );
            let method = php_payload_case_method(&case.name, !value.fields.is_empty());
            format!(
                "{}::{method}({})",
                plan.enum_symbol(&definition.source_type),
                value
                    .fields
                    .iter()
                    .map(|field| emit_const_value(field, plan))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PhpFunctionEmission<'a> {
    is_method: bool,
    exact_method: bool,
    property_initializers: &'a [&'a PropertyDecl],
    invoke_parent_constructor: bool,
    invoke_parent_destructor: bool,
    manual_promotions: bool,
    drop_properties: &'a [String],
    construction_properties: &'a [String],
    parent_drop_on_construction_failure: bool,
}

fn promoted_property_owns_copy_claim(param: &Param, scopes: &PhpNameScopes) -> bool {
    param.constructor_role.is_promoted()
        && !param.borrow
        && scopes
            .specialization
            .resolve_type(&param.ty, &scopes.substitutions)
            .is_some_and(|ty| owns_copy_claim(&ty, scopes))
}

fn constructor_uses_manual_promotions(class: &ClassDecl, function: &FunctionDecl) -> bool {
    function.name == "__construct"
        && (class.parent.is_some()
            || function
                .params
                .iter()
                .any(|param| param.constructor_role.is_promoted()))
}

fn promoted_property_value(param: &Param, scopes: &PhpNameScopes) -> String {
    let place = scopes
        .binding_for_declaration(&param.name, param.span)
        .and_then(|binding| scopes.place(binding))
        .expect("promoted parameter has a checked storage place");
    if promoted_property_owns_copy_claim(param, scopes) {
        // Copy promotion acquires a field claim without consuming the parameter.
        return format!("__doria_copy_value({})", place.read());
    }
    if param.take {
        if let PhpBindingPlace::Cell(name) = place {
            return format!("__doria_take_cell(${name})");
        }
    }
    place.read()
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
        if shared_scopes
            .specialization
            .exact_symbol(callable.id)
            .is_some()
        {
            output.push('\n');
            emit_function_instance(
                function,
                semantic_info,
                output,
                indent,
                &scopes,
                PhpFunctionEmission {
                    exact_method: true,
                    ..emission
                },
            );
        }
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
    let Some(mir::ClosureOwner::Callable(instance)) = shared_scopes.closure_owner else {
        unreachable!();
    };
    let callable = shared_scopes.specialization.callable(instance);
    let mut transports = vec![value_argument::Transport::Boxed];
    if callable.field_result.is_some() {
        transports.push(value_argument::Transport::FieldResult);
    }
    if callable.field_parameters.iter().any(Option::is_some) {
        transports.push(value_argument::Transport::FieldArguments);
        if callable.field_result.is_some() {
            transports.push(value_argument::Transport::FieldArgumentsAndResult);
        }
    }
    for (index, transport) in transports.into_iter().enumerate() {
        if index != 0 {
            output.push('\n');
        }
        emit_function_instance_body(
            function,
            semantic_info,
            output,
            indent,
            shared_scopes,
            emission,
            transport,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_function_instance_body(
    function: &FunctionDecl,
    semantic_info: &SemanticInfo,
    output: &mut String,
    indent: usize,
    shared_scopes: &PhpNameScopes,
    emission: PhpFunctionEmission<'_>,
    transport: value_argument::Transport,
) {
    let field_body = transport.result();
    let PhpFunctionEmission {
        is_method,
        exact_method,
        property_initializers,
        invoke_parent_constructor,
        invoke_parent_destructor,
        manual_promotions,
        drop_properties,
        construction_properties,
        parent_drop_on_construction_failure,
    } = emission;
    let mut scopes = shared_scopes.expression_scope();
    scopes.returns_borrow = semantic_info
        .return_borrows
        .get(&function.span)
        .is_some_and(|borrow| borrow.kind == crate::types::ReturnBorrowKind::Value);
    scopes.returns_borrow_cell = scopes.returns_borrow
        && function.return_type.as_ref().is_some_and(|ty| {
            scopes
                .specialization
                .resolve_type(ty, &scopes.substitutions)
                .is_some_and(|ty| borrowed_result_uses_cell(&ty))
        });
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
    if let Some(plan) = &callable_plan {
        scopes.returns_borrow = plan.returns_borrow;
        scopes.returns_borrow_cell = scopes.returns_borrow
            && function.return_type.as_ref().is_some_and(|ty| {
                scopes
                    .specialization
                    .resolve_type(ty, &scopes.substitutions)
                    .is_some_and(|ty| borrowed_result_uses_cell(&ty))
            });
    }
    let Some(mir::ClosureOwner::Callable(instance)) = scopes.closure_owner else {
        unreachable!("authored callable emission has a checked concrete instance");
    };
    let parameter_types = if transport.arguments() {
        scopes
            .specialization
            .callable(instance)
            .field_parameters
            .clone()
    } else {
        vec![None; function.params.len()]
    };
    assert_eq!(
        parameter_types.len(),
        function.params.len(),
        "checked field parameter arity"
    );
    let mut parameter_places = Vec::new();
    for (index, param) in function.params.iter().enumerate() {
        scopes.declare_unmangled(&param.name);
        let place = parameter_types[index]
            .as_ref()
            .map(|ty| value_result::Place::parameter(ty, index, &mut scopes));
        if param.ty.name == "mixed" {
            scopes.mark_mixed(&param.name);
        }
        if let Some(binding) = scopes.binding_for_declaration(&param.name, param.span) {
            let uses_cell = callable_parameter_uses_cell(callable_plan.as_ref(), index);
            scopes.bind_place(
                binding,
                if uses_cell || scopes.needs_cell(binding) {
                    PhpBindingPlace::Cell(param.name.clone())
                } else if let Some(place) = &place {
                    PhpBindingPlace::Fields(place.clone())
                } else {
                    PhpBindingPlace::Direct(param.name.clone())
                },
            );
        }
        parameter_places.push(place);
    }

    let receiver_home = callable_plan
        .as_ref()
        .is_some_and(|plan| plan.receiver_home)
        .then(|| scopes.fresh_temp("__doria_receiver_home"));
    let internal_receiver_home = callable_plan
        .as_ref()
        .is_some_and(|plan| plan.receiver_home && plan.lifecycle_receiver);
    if let Some(home) = &receiver_home {
        scopes.expression_inputs.push(home.clone());
    }

    write_indent(output, indent);
    if is_method {
        output.push_str(emit_member_access(&function.access, function.span.source));
        output.push(' ');
        if function.is_static {
            output.push_str("static ");
        }
    }
    output.push_str("function ");
    let callable = scopes.specialization.callable(instance);
    let field_result_type = callable.field_result.clone();
    let implementation_symbol = if exact_method {
        scopes
            .specialization
            .exact_symbol(instance)
            .expect("checked exact implementation symbol")
    } else {
        &callable.symbol
    }
    .to_string();
    let result_symbol = if transport.arguments() {
        scopes
            .specialization
            .argument_symbol(&implementation_symbol, true)
    } else {
        scopes.specialization.result_symbol(&implementation_symbol)
    }
    .to_string();
    let symbol = if transport.arguments() {
        scopes
            .specialization
            .argument_symbol(&implementation_symbol, field_body)
    } else if field_body {
        &result_symbol
    } else {
        &implementation_symbol
    };
    output.push_str(symbol);
    output.push('(');
    if let Some(home) = receiver_home.as_ref().filter(|_| !internal_receiver_home) {
        output.push_str(&format!("__DoriaCell ${home}"));
        if !function.params.is_empty() {
            output.push_str(", ");
        }
    }
    if function.intrinsic
        == Some(crate::ast::CompilerIntrinsic::File(
            crate::compiler_known_stream::FileFunction::UnavailableConstructor,
        ))
        && scopes.file_payload
    {
        output.push_str("__DoriaFileOwner $__doriaFileOwner");
    } else if function.intrinsic
        == Some(crate::ast::CompilerIntrinsic::File(
            crate::compiler_known_stream::FileFunction::UnavailableConstructor,
        ))
        && streams::uses_child_payload(&scopes)
    {
        output.push_str("FFI\\CData $__doriaChild");
    } else if function.intrinsic
        == Some(crate::ast::CompilerIntrinsic::File(
            crate::compiler_known_stream::FileFunction::UnavailableConstructor,
        ))
        && streams::uses_readiness_payload(&scopes)
    {
        output.push_str("FFI\\CData $__doriaReadinessSource");
    } else if function.intrinsic
        == Some(crate::ast::CompilerIntrinsic::File(
            crate::compiler_known_stream::FileFunction::UnavailableConstructor,
        ))
        && streams::uses_request_payload(&scopes)
    {
        output.push_str("__DoriaPendingRequest $__doriaPendingRequest");
    } else if function.intrinsic
        == Some(crate::ast::CompilerIntrinsic::File(
            crate::compiler_known_stream::FileFunction::UnavailableConstructor,
        ))
        && streams::uses_plan_payload(&scopes)
    {
        output.push_str("FFI\\CData $__doriaTransferPlan, array $__doriaTransferOperations");
    } else if function.intrinsic
        == Some(crate::ast::CompilerIntrinsic::File(
            crate::compiler_known_stream::FileFunction::UnavailableConstructor,
        ))
        && streams::uses_group_payload(&scopes)
    {
        output.push_str("__DoriaHostedTransferGroup $__doriaTransferGroup");
    } else {
        output.push_str(
            &function
                .params
                .iter()
                .enumerate()
                .map(|(parameter_index, param)| {
                    if let Some(place) = &parameter_places[parameter_index] {
                        return place.declarations().join(", ");
                    }
                    let uses_cell =
                        callable_parameter_uses_cell(callable_plan.as_ref(), parameter_index);
                    emit_param(
                        param,
                        (!transport.arguments())
                            .then(|| {
                                semantic_info.parameter_defaults.get(&ParameterDefaultKey {
                                    function: function.span,
                                    parameter_index,
                                })
                            })
                            .flatten(),
                        &scopes,
                        uses_cell,
                        !manual_promotions,
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    output.push(')');
    let is_lifecycle_method =
        is_method && matches!(function.name.as_str(), "__construct" | "__destruct");
    if let Some(return_type) = function
        .return_type
        .as_ref()
        .filter(|_| !is_lifecycle_method)
    {
        output.push_str(": ");
        if field_body {
            output.push_str("void");
        } else if scopes.returns_borrow_cell {
            output.push_str("__DoriaCell");
        } else {
            output.push_str(&php_type(return_type, &scopes));
        }
    }
    output.push('\n');
    writeln(output, indent, "{");
    let mut forwarded_arguments = Vec::new();
    if let Some(home) = receiver_home.as_ref().filter(|_| !internal_receiver_home) {
        forwarded_arguments.push(format!("${home}"));
    }
    for (param, place) in function.params.iter().zip(&parameter_places) {
        if let Some(place) = place {
            forwarded_arguments.extend(place.names().map(|name| format!("${name}")));
        } else {
            forwarded_arguments.push(format!("${}", param.name));
        }
    }
    if !field_body {
        if let Some(ty) = &field_result_type {
            if value_result::supports_intrinsic(function.intrinsic) {
                writeln(
                    output,
                    indent + 1,
                    &format!(
                        "{}{result_symbol}({});",
                        if is_method { "self::" } else { "" },
                        forwarded_arguments.join(", ")
                    ),
                );
                value_result::emit_boxed_return(ty, output, indent + 1, &mut scopes);
                writeln(output, indent, "}");
                return;
            }
        }
    } else {
        scopes.value_return = Some(value_result::Return::new(
            field_result_type.as_ref().unwrap(),
            &mut scopes,
        ));
        if !value_result::supports_intrinsic(function.intrinsic) {
            let call = format!(
                "{}{}({})",
                if is_method { "self::" } else { "" },
                implementation_symbol,
                forwarded_arguments.join(", ")
            );
            value_result::emit_forwarded_result(&call, output, indent + 1, &mut scopes);
            value_result::emit_return_epilogue(output, indent + 1, &scopes);
            writeln(output, indent, "}");
            return;
        }
    }
    if let Some(intrinsic) = function.intrinsic {
        match intrinsic {
            crate::ast::CompilerIntrinsic::Concurrency(intrinsic) => {
                concurrency::emit_body(intrinsic, output, indent + 1);
            }
            crate::ast::CompilerIntrinsic::Term(intrinsic) => term::emit_body(
                intrinsic,
                function,
                &parameter_places,
                output,
                indent + 1,
                &mut scopes,
            ),
            crate::ast::CompilerIntrinsic::File(intrinsic) => streams::emit_body(
                intrinsic,
                function,
                &parameter_places,
                output,
                indent + 1,
                &mut scopes,
            ),
            crate::ast::CompilerIntrinsic::Readiness(intrinsic) => {
                streams::emit_set_body(intrinsic, function, output, indent + 1, &scopes);
            }
            crate::ast::CompilerIntrinsic::Process(intrinsic) => {
                streams::emit_process_body(intrinsic, function, output, indent + 1, &scopes);
            }
            crate::ast::CompilerIntrinsic::TransferPlan(intrinsic) => {
                streams::emit_plan_body(intrinsic, function, output, indent + 1, &scopes);
            }
            crate::ast::CompilerIntrinsic::TransferGroup(intrinsic) => {
                streams::emit_group_body(
                    intrinsic,
                    function,
                    &parameter_places,
                    output,
                    indent + 1,
                    &mut scopes,
                );
            }
            crate::ast::CompilerIntrinsic::Time(intrinsic) => {
                use crate::compiler_known_time::TimeFunction;
                let body = match intrinsic {
                    TimeFunction::DurationFromNanoseconds => "return new self($nanoseconds);",
                    TimeFunction::DurationNanoseconds => "return $this->__doriaInlineBits;",
                    TimeFunction::DeadlineFromNow => {
                        let getter = scopes
                            .specialization
                            .accessor_symbol("nanoseconds", crate::ast::PropertyHookKind::Get);
                        let call = format!(
                            "$status = $runtime->dr_v1_deadline_from_now($duration->{getter}(), $low, $high, $code, $hasCode);"
                        );
                        for line in [
                            "$runtime = __DoriaNativeIo::get();",
                            "$low = $runtime->new('uint64_t[1]');",
                            "$high = $runtime->new('int64_t[1]');",
                            "$code = $runtime->new('int64_t[1]');",
                            "$hasCode = $runtime->new('uint8_t[1]');",
                            &call,
                            "if ($status !== 0 && $status !== 1) { __DoriaNativeIo::fail('protocol'); }",
                        ] {
                            writeln(output, indent + 1, line);
                        }
                        writeln(
                            output,
                            indent + 1,
                            &format!(
                                "if ($status === 1) {{ __doria_throw(new {}({}, $hasCode[0] === 0 ? null : $code[0]), 0, ''); }}",
                                php_type_symbol(crate::compiler_known_time::CLOCK_ERROR),
                                emit_php_string_literal(
                                    crate::compiler_known_time::CLOCK_FAILURE_MESSAGE
                                ),
                            ),
                        );
                        "return new self($runtime->cast('int64_t[1]', $low)[0], $high[0]);"
                    }
                };
                writeln(output, indent + 1, body);
            }
        }
        writeln(output, indent, "}");
        return;
    }
    let frame_body_start = output.len();
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
    emit_owned_scope(
        output,
        body_indent,
        &mut scopes,
        |output, body_indent, scopes| {
            if let Some(home) = &receiver_home {
                if internal_receiver_home {
                    writeln(
                        output,
                        body_indent,
                        &format!("${home} = new __DoriaCell($this);"),
                    );
                    writeln(output, body_indent, &format!("${home}->borrowed = true;"));
                }
                let binding = scopes
                    .closure_plan
                    .binding_resolution
                    .receiver_for_callable(function.span)
                    .expect("borrowed receiver has a checked binding");
                scopes.bind_place(binding, PhpBindingPlace::Cell(home.clone()));
            }
            if function.consuming_this {
                let binding = scopes
                    .closure_plan
                    .binding_resolution
                    .receiver_for_callable(function.span)
                    .expect("consuming method has a checked receiver binding");
                let receiver = scopes.fresh_temp("__doria_receiver");
                scopes.insert_current("this", receiver.clone());
                writeln(
                    output,
                    body_indent,
                    &format!("${receiver} = new __DoriaCell($this);"),
                );
                scopes.bind_place(binding, PhpBindingPlace::Cell(receiver.clone()));
                scopes.own_cell(receiver);
            }
            for (parameter_index, param) in function.params.iter().enumerate() {
                if transport.arguments() {
                    continue;
                }
                let Some(default @ ConstValue::PayloadEnum(_)) =
                    semantic_info.parameter_defaults.get(&ParameterDefaultKey {
                        function: function.span,
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
                        emit_const_value(default, &scopes.specialization)
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
                let Some(binding) = scopes.binding_for_declaration(&param.name, param.span) else {
                    continue;
                };
                let uses_cell = callable_parameter_uses_cell(callable_plan.as_ref(), index);
                if scopes.needs_cell(binding)
                    && !uses_cell
                    && !value_result::storage::initialize_parameter(
                        &param.name,
                        binding,
                        parameter_places[index].as_ref(),
                        output,
                        body_indent,
                        scopes,
                    )
                {
                    if let Some(place) = &parameter_places[index] {
                        writeln(
                            output,
                            body_indent,
                            &format!("${} = {};", param.name, place.read()),
                        );
                    }
                    let owns_copy = scopes
                        .source_type(binding)
                        .is_some_and(|ty| owns_copy_claim(ty, scopes));
                    writeln(
                        output,
                        body_indent,
                        &format!(
                            "${0} = new __DoriaCell({1}${0}{2});",
                            param.name,
                            if owns_copy { "__doria_copy_value(" } else { "" },
                            if owns_copy { ")" } else { "" }
                        ),
                    );
                    if owns_copy {
                        scopes.own_cell(param.name.clone());
                    }
                }
                if uses_cell
                    && !transport.arguments()
                    && semantic_info
                        .parameter_defaults
                        .contains_key(&ParameterDefaultKey {
                            function: function.span,
                            parameter_index: index,
                        })
                {
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
                    let value = promoted_property_value(param, scopes);
                    writeln(
                        output,
                        body_indent,
                        &format!("$this->{} = {value};", param.name),
                    );
                }
                if (uses_cell || scopes.needs_cell(binding)) && param.take {
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
            }
            // Construction completion belongs to the Doria parent phase,
            // not to the existence of a host constructor method.
            if let Some(parent_completed) = &parent_completed {
                writeln(output, body_indent, &format!("${parent_completed} = true;"));
            }
            if construction_cleanup {
                writeln(output, body_indent, "try {");
            }
            let phase_indent = body_indent + usize::from(construction_cleanup);
            for property in property_initializers {
                property_hooks::emit_initializer(property, output, phase_indent, scopes);
            }
            if manual_promotions {
                for param in &function.params {
                    if !param.constructor_role.is_promoted() {
                        continue;
                    }
                    let value = promoted_property_value(param, scopes);
                    let assignment = if param.borrow && param.writable {
                        "=&"
                    } else {
                        "="
                    };
                    writeln(
                        output,
                        phase_indent,
                        &format!("$this->{} {assignment} {value};", param.name),
                    );
                }
            }
            let routes_destructor_cleanup = invoke_parent_destructor || !drop_properties.is_empty();
            let statement_indent = if routes_destructor_cleanup {
                writeln(output, phase_indent, "try");
                writeln(output, phase_indent, "{");
                phase_indent + 1
            } else {
                phase_indent
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
            if routes_destructor_cleanup || (is_method && function.name == "__construct") {
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
            if construction_cleanup {
                emit_constructor_catch(
                    construction_properties,
                    parent_completed.as_deref(),
                    output,
                    body_indent,
                    scopes,
                );
            }
        },
    );
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
    value_result::emit_return_epilogue(output, indent + 1, &scopes);
    let frame_body = output.split_off(frame_body_start);
    diagnostic_sites::emit_frame_body(output, &frame_body, indent, function.span, &scopes);
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
        writeln(
            output,
            indent,
            &format!(
                "if (isset($this->{property})) {{ ${temporary} = $this->{property}; unset($this->{property}); __doria_drop_value(${temporary}); }}"
            ),
        );
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
    writeln(
        output,
        indent + 1,
        &format!("__doria_rethrow_checked_error(${error});"),
    );
    writeln(output, indent, "}");
}

fn emit_param(
    param: &Param,
    evaluated_default: Option<&ConstValue>,
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
        output.push_str(emit_member_access(&access, param.span.source));
        output.push(' ');
    }
    let payload_default = matches!(evaluated_default, Some(ConstValue::PayloadEnum(_)));
    if uses_cell && evaluated_default.is_none() {
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
    if let Some(default) = evaluated_default {
        output.push_str(" = ");
        if payload_default {
            output.push_str("[]");
        } else {
            output.push_str(&emit_const_value(default, &scopes.specialization));
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
    if php_type == "mixed" {
        return php_type;
    }
    let mut members = vec![php_type.trim_start_matches('?').to_string()];
    if payload_default {
        members.push("array".to_string());
    }
    if php_type.starts_with('?') {
        members.push("null".to_string());
    }
    members.join("|")
}

fn php_top_level_constant_name(name: &str) -> String {
    format!("__DORIA_CONST_{}", php_constant_symbol(name))
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
        &format!("if (${forwarded} !== null) {{ __doria_rethrow_checked_error(${forwarded}); }}"),
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
    let previous_root = scopes.expression_region.take();
    emit_statement_region(statement, output, indent, scopes);
    scopes.expression_region = previous_root;
}

fn emit_statement_region(
    statement: &Stmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    diagnostic_sites::emit_set_site(
        output,
        indent,
        diagnostic_sites::statement_span(statement),
        scopes,
    );
    let root = match statement {
        Stmt::VarDecl(decl) => Some(&decl.initializer),
        Stmt::Assignment(assignment)
            if assignment.op == AssignOp::Assign
                && matches!(
                    expression_region::ungrouped(&assignment.target),
                    Expr::Variable { .. } | Expr::StaticMember { .. }
                ) =>
        {
            Some(&assignment.value)
        }
        Stmt::Return {
            expr: Some(expr), ..
        }
        | Stmt::Echo { expr, .. }
        | Stmt::Expr { expr, .. } => Some(expr),
        Stmt::Throw(statement) => Some(&statement.expr),
        _ => None,
    };
    if let Some(root) = root {
        let previous_temporaries = scopes.expression_temporaries.clone();
        // Select the yield's statement owner before staging its operands.
        if matches!(statement, Stmt::Return { .. }) && scopes.returns_borrow {
            if let Some(temporaries) = &scopes.yield_temporaries {
                scopes.expression_temporaries = Some(Rc::clone(temporaries));
            }
        }
        expression_region::emit(root, output, indent, scopes, |output, indent, scopes| {
            emit_statement_inner(statement, output, indent, scopes);
        });
        scopes.expression_temporaries = previous_temporaries;
        return;
    }
    if scopes.expression_temporaries.is_some()
        || !matches!(
            statement,
            Stmt::VarDecl(_) | Stmt::Assignment(_) | Stmt::Increment(_) | Stmt::Return { .. }
        )
    {
        emit_statement_inner(statement, output, indent, scopes);
        return;
    }
    let temporaries = Rc::new(RefCell::new(Vec::new()));
    scopes.expression_temporaries = Some(Rc::clone(&temporaries));
    let mut body = String::new();
    emit_statement_inner(statement, &mut body, indent, scopes);
    scopes.expression_temporaries = None;
    let temporaries = temporaries.borrow();
    if temporaries.is_empty() {
        output.push_str(&body);
        return;
    }
    // Decision 0081 keeps expression owners through statement-result binding.
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
    for temporary in temporaries.iter().rev() {
        writeln(output, indent + 1, &temporary.cleanup());
    }
    writeln(output, indent, "}");
}

fn emit_statement_inner(
    statement: &Stmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    match statement {
        Stmt::Block(block) => emit_block(block, output, indent, scopes),
        Stmt::VarDecl(decl) => {
            if value_result::emit_local(decl, output, indent, scopes) {
                return;
            }
            let borrowed_cell = decl
                .bindings
                .first()
                .and_then(|binding| scopes.binding_for_declaration(&binding.name, binding.span))
                .is_some_and(|binding| {
                    !scopes.owns_binding(binding)
                        && scopes.source_type(binding).is_some_and(|ty| {
                            borrowed_result_uses_cell(&crate::types::substitute_resolved_type(
                                ty,
                                &scopes.substitutions,
                            ))
                        })
                });
            let owns_value = decl
                .bindings
                .iter()
                .any(|binding| binding_needs_php_drop(binding, scopes));
            let staged = (!borrowed_cell)
                .then(|| {
                    value_result::statement_value(
                        &decl.initializer,
                        owns_value,
                        output,
                        indent,
                        scopes,
                    )
                })
                .flatten();
            let initializer = if let Some(value) = staged {
                value
            } else if borrowed_cell {
                borrowed_argument_cell(&decl.initializer, scopes)
                    .expect("checked borrowed binding has a storage home")
            } else if owns_value {
                emit_owned_expr(&decl.initializer, scopes)
            } else {
                emit_expr(&decl.initializer, scopes)
            };
            let binding_is_mixed = decl.ty.as_ref().is_some_and(|ty| ty.name == "mixed")
                || (decl.ty.is_none()
                    && scopes
                        .expression_types
                        .get(&decl.initializer.span())
                        .is_some_and(is_mixed_storage_type));
            expression_region::consume(output, indent, scopes);
            if decl.bindings.len() == 1 {
                emit_local_binding(
                    &decl.bindings[0],
                    &initializer,
                    binding_is_mixed,
                    borrowed_cell,
                    output,
                    indent,
                    scopes,
                );
            } else {
                let temporary = scopes.fresh_temp("__doria_grouped_value");
                writeln(output, indent, &format!("${temporary} = {initializer};"));
                for (index, binding) in decl.bindings.iter().enumerate() {
                    let value = if index > 0 && binding_needs_php_drop(binding, scopes) {
                        format!("__doria_copy_value(${temporary})")
                    } else {
                        format!("${temporary}")
                    };
                    emit_local_binding(
                        binding,
                        &value,
                        binding_is_mixed,
                        borrowed_cell,
                        output,
                        indent,
                        scopes,
                    );
                }
                writeln(output, indent, &format!("unset(${temporary});"));
            }
        }
        Stmt::Assignment(assignment) => {
            if bytes::emit_assignment(assignment, output, indent, scopes) {
                return;
            }
            if value_result::emit_assignment(assignment, output, indent, scopes) {
                return;
            }
            if property_hooks::emit_assignment(assignment, output, indent, scopes) {
                return;
            }
            if numeric_place::emit_assignment_statement(assignment, output, indent, scopes) {
                return;
            }
            let expression = emit_assignment(assignment, scopes);
            expression_region::consume(output, indent, scopes);
            writeln(output, indent, &format!("{expression};"));
        }
        Stmt::Echo { expr, span } => {
            let expression = with_expression_temporaries(scopes, |scopes| {
                format!(
                    "__doria_write_stdout({}, {}, {}, {})",
                    emit_display_expr(expr, scopes),
                    scopes.source_location(*span, span.start),
                    scopes.source_location(*span, span.end),
                    scopes.callable_identity(),
                )
            });
            expression_region::consume(output, indent, scopes);
            writeln(output, indent, &format!("{expression};"));
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                if value_result::emit_return(expr, output, indent, scopes) {
                    return;
                }
                let result_scopes = scopes.clone();
                let owns_result = scopes.expression_types.get(&expr.span()).is_some_and(|ty| {
                    returned_value_owns_carrier(ty, scopes.returns_borrow, scopes)
                });
                if owns_result || scopes.has_owned_cells() {
                    let result = scopes.fresh_temp("__doria_return");
                    if owns_result {
                        for pending in &scopes.pending_returns {
                            pending.borrow_mut().push(result.clone());
                        }
                    }
                    let staged = (!scopes.returns_borrow)
                        .then(|| {
                            value_result::statement_value(expr, owns_result, output, indent, scopes)
                        })
                        .flatten();
                    let value = if let Some(value) = staged {
                        value
                    } else if owns_result {
                        emit_owned_expr(expr, &result_scopes)
                    } else {
                        emit_returned_expr(expr, &result_scopes)
                    };
                    expression_region::consume(output, indent, &result_scopes);
                    writeln(output, indent, &format!("${result} = {value};"));
                    writeln(output, indent, &format!("return ${result};"));
                } else {
                    let staged = (!scopes.returns_borrow)
                        .then(|| value_result::statement_value(expr, false, output, indent, scopes))
                        .flatten();
                    let value = staged.unwrap_or_else(|| emit_returned_expr(expr, &result_scopes));
                    expression_region::consume(output, indent, &result_scopes);
                    writeln(output, indent, &format!("return {value};"));
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
                            emit_if(if_stmt, output, indent, None, scopes);
                        }
                    });
                } else if if_stmt.given.is_some() {
                    emit_given_if(if_stmt, output, indent, scopes);
                } else {
                    emit_if(if_stmt, output, indent, None, scopes);
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
            if numeric_place::emit_increment_statement(increment, output, indent, scopes) {
                return;
            }
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
            expression_region::consume(output, indent, scopes);
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
        ResolvedType::Enum(ty) if ty.arguments.is_empty() => ty.name.clone(),
        ResolvedType::Enum(ty) => format!(
            "{}<{}>",
            ty.name,
            ty.arguments
                .iter()
                .map(assertion_type_name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
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
    let error_class = php_type_symbol(crate::compiler_known_test::ASSERTION_ERROR);
    let matcher = assertion.matcher.fact_name();
    let origin = scopes.checked_origin(assertion.span);
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
                        resolved_actual_type.as_ref().unwrap(),
                        scopes,
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
    scopes: &mut PhpNameScopes,
) {
    debug_assert!(scopes.throw_error_types.contains_key(&statement.span));
    let value = value_result::statement_value(&statement.expr, true, output, indent, scopes)
        .unwrap_or_else(|| emit_owned_expr(&statement.expr, scopes));
    expression_region::consume(output, indent, scopes);
    writeln(
        output,
        indent,
        &format!(
            "__doria_throw({}, {}, {});",
            value,
            scopes.checked_origin(statement.span),
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
                .unwrap_or_else(|| php_type_symbol(&class.name));
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
            writeln(output, indent + 2, &format!("unset(${caught});"));
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
            writeln(
                output,
                indent + 2,
                &format!("__doria_rethrow_checked_error(${caught});"),
            );
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
    let predicates = while_stmt
        .given
        .as_ref()
        .map(|given| emit_given_setup(given, output, indent, scopes))
        .unwrap_or_default();
    writeln(output, indent, "while (true) {");
    let gate = emit_predicate_gate(&predicates, output, indent + 1, scopes);
    let condition = emit_condition(
        &while_stmt.condition,
        gate.as_deref(),
        output,
        indent + 1,
        scopes,
    );
    writeln(
        output,
        indent + 1,
        &format!("if (!{condition}) {{ break; }}"),
    );
    emit_block(&while_stmt.body, output, indent + 1, scopes);
    writeln(output, indent, "}");
}

fn emit_do_while(
    do_while: &DoWhileStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let first = scopes.fresh_temp("__doria_do_first");
    writeln(output, indent, &format!("${first} = true;"));
    writeln(output, indent, "while (true) {");
    // The next iteration's head evaluates the tail even after continue.
    writeln(output, indent + 1, &format!("if (!${first}) {{"));
    let condition = expression_region::value(&do_while.condition, output, indent + 2, scopes);
    writeln(
        output,
        indent + 2,
        &format!("if (!{condition}) {{ break; }}"),
    );
    writeln(output, indent + 1, "}");
    writeln(output, indent + 1, &format!("${first} = false;"));
    emit_block(&do_while.body, output, indent + 1, scopes);
    writeln(output, indent, "}");
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
            scopes.source_location(span, span.start),
            scopes.source_location(span, span.end),
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
        let first = for_stmt
            .increment
            .as_ref()
            .map(|_| scopes.fresh_temp("__doria_for_first"));
        if let Some(first) = &first {
            writeln(output, indent, &format!("${first} = true;"));
        }
        writeln(output, indent, "while (true) {");
        if let (Some(first), Some(increment)) = (&first, &for_stmt.increment) {
            writeln(output, indent + 1, &format!("if (!${first}) {{"));
            let statement = match increment {
                ForIncrement::Increment(increment) => Stmt::Increment(increment.clone()),
                ForIncrement::Assignment(assignment) => Stmt::Assignment(assignment.clone()),
            };
            emit_statement(&statement, output, indent + 2, scopes);
            writeln(output, indent + 1, "}");
            writeln(output, indent + 1, &format!("${first} = false;"));
        }
        if let Some(condition) = &for_stmt.condition {
            let condition = expression_region::value(condition, output, indent + 1, scopes);
            writeln(
                output,
                indent + 1,
                &format!("if (!{condition}) {{ break; }}"),
            );
        }
        emit_block(&for_stmt.body, output, indent + 1, scopes);
        writeln(output, indent, "}");
    });
    scopes.pop();
}

fn resolved_type_needs_php_drop(ty: &ResolvedType, scopes: &PhpNameScopes) -> bool {
    match ty {
        ResolvedType::Interface(_)
        | ResolvedType::InterfaceSelf(_)
        | ResolvedType::SharedHandle(_, _) => true,
        ResolvedType::TraitSelf(_) => {
            unreachable!("unsubstituted trait self reached executable PHP lowering")
        }
        ResolvedType::Mixed
        | ResolvedType::Bytes
        | ResolvedType::Error
        | ResolvedType::Function(_)
        | ResolvedType::TypeParameter(_) => true,
        ResolvedType::Enum(enum_type) => scopes
            .symbols
            .payload_enums_with_php_destructors
            .contains(&enum_type.name),
        ResolvedType::Class(class) => class
            .inline_copy_type()
            .is_none_or(|kind| kind.requires_owned_cleanup()),
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
        | ResolvedType::ByteRegion(_)
        | ResolvedType::Integer(_)
        | ResolvedType::Float(_)
        | ResolvedType::String
        | ResolvedType::Bool
        | ResolvedType::Null
        | ResolvedType::Unsupported => false,
    }
}

fn returned_value_owns_carrier(
    ty: &ResolvedType,
    returns_borrow: bool,
    scopes: &PhpNameScopes,
) -> bool {
    // Carrier ownership follows the checked return contract for every Move
    // family. Retained capture lifetimes do not turn a new closure into a loan.
    let ty = crate::types::substitute_resolved_type(ty, &scopes.substitutions);
    !returns_borrow
        && (crate::ownership::resolved_type_is_move_type(&ty, &scopes.symbols.move_enum_names)
            || resolved_type_needs_php_drop(&ty, scopes))
}

fn assignment_target_needs_php_drop(target: &Expr, scopes: &PhpNameScopes) -> bool {
    match target {
        Expr::Grouped { expr, .. } => assignment_target_needs_php_drop(expr, scopes),
        Expr::Variable { span, .. } => scopes
            .binding_for_use(*span)
            .and_then(|binding| scopes.source_type(binding))
            .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes)),
        _ => scopes
            .expression_types
            .get(&target.span())
            .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes)),
    }
}

fn binding_needs_php_drop(declaration: &VarBinding, scopes: &PhpNameScopes) -> bool {
    scopes
        .binding_for_declaration(&declaration.name, declaration.span)
        .filter(|binding| scopes.owns_binding(*binding))
        .and_then(|binding| scopes.source_type(binding))
        .is_some_and(|ty| resolved_type_needs_php_drop(ty, scopes))
}

fn emit_local_binding(
    declaration: &VarBinding,
    initializer: &str,
    binding_is_mixed: bool,
    borrowed_cell: bool,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let owns_value = binding_needs_php_drop(declaration, scopes);
    let php_name = scopes.declare(&declaration.name);
    let binding = scopes.binding_for_declaration(&declaration.name, declaration.span);
    if binding_is_mixed {
        scopes.mark_mixed(&declaration.name);
    }
    if borrowed_cell {
        writeln(output, indent, &format!("${php_name} = {initializer};"));
        scopes.bind_place(
            binding.expect("checked borrowed binding exists"),
            PhpBindingPlace::Cell(php_name),
        );
    } else if owns_value || binding.is_some_and(|binding| scopes.needs_cell(binding)) {
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
        Expr::Variable { span, .. } | Expr::This { span } => {
            scopes.place_for_use(*span).and_then(PhpBindingPlace::cell)
        }
        _ => None,
    }
}

/// Borrowing needs initialized storage; assignment may initialize a fresh slot.
/// Keep projections out of assignment and ownership-transfer target resolution.
fn borrowed_argument_cell(target: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    if scopes
        .interface_conversion_types
        .contains_key(&target.span())
        && scopes.expression_types.get(&target.span()) != Some(&ResolvedType::Null)
        && !expression_has_move_ownership(target, scopes)
    {
        let value = emit_owned_expr(target, scopes);
        return Some(stage_owned_argument_cell(
            target.span(),
            format!("new __DoriaCell({value})"),
            scopes,
        ));
    }
    let cell = borrowed_argument_cell_unprojected(target, scopes)?;
    Some(project_borrowed_argument_cell(target, cell, scopes))
}

fn project_borrowed_argument_cell(target: &Expr, cell: String, scopes: &PhpNameScopes) -> String {
    if scopes
        .expression_types
        .get(&target.span())
        .is_some_and(|ty| !is_mixed_storage_type(ty))
    {
        format!("__doria_project_cell({cell})")
    } else {
        cell
    }
}

fn normalize_borrowed_result_cell(target: &Expr, cell: String) -> String {
    if matches!(
        expression_region::ungrouped(target),
        Expr::MethodCall {
            null_safe: true,
            ..
        } | Expr::PropertyAccess {
            null_safe: true,
            ..
        }
    ) {
        format!("({cell} ?? new __DoriaCell(null))")
    } else {
        cell
    }
}

fn borrowed_argument_cell_unprojected(target: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    if let Some(cell) = scopes
        .prepared_receiver
        .as_ref()
        .and_then(|receiver| receiver.cell(target))
    {
        return Some(normalize_borrowed_result_cell(target, cell));
    }
    if let Some(cell) = value_result::storage::cell(target, scopes) {
        return Some(cell);
    }
    if let Some(cell) = core_collection::borrowed_cell(target, scopes) {
        return Some(expression_region::operation(target, cell, scopes));
    }
    if expression_returns_borrow_cell(target, scopes) {
        let cell = expression_region::guarded(
            target,
            emit_expr_unboxed_impl(target, scopes),
            true,
            scopes,
        );
        return Some(normalize_borrowed_result_cell(target, cell));
    }
    match target {
        Expr::Grouped { expr, .. } => borrowed_argument_cell(expr, scopes),
        Expr::Binary {
            op: BinaryOp::Coalesce,
            ..
        } if expression_has_move_ownership(target, scopes)
            && scopes
                .expression_types
                .get(&target.span())
                .is_some_and(borrowed_result_uses_cell) =>
        {
            Some(emit_borrowed_result_cell(target, scopes))
        }
        Expr::Variable { span, .. } | Expr::This { span } => {
            scopes.place_for_use(*span).map(|place| match place {
                PhpBindingPlace::Cell(name) => format!("${name}"),
                PhpBindingPlace::Direct(name) => format!("__DoriaCell::borrowed(${name})"),
                PhpBindingPlace::Fields(place) => {
                    format!("__DoriaCell::borrowed({})", place.read())
                }
            })
        }
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            ..
        } if property == "referencedValue"
            && scopes
                .expression_types
                .get(&object.span())
                .and_then(shared::kind)
                == Some(crate::types::SharedHandleKind::SharedReference) =>
        {
            let home = emit_receiver_borrow_home(object, *null_safe, scopes);
            Some(if *null_safe {
                format!("({home} ?? new __DoriaCell(null))")
            } else {
                home
            })
        }
        Expr::StaticMember {
            class_name,
            member,
            span,
        } if scopes.is_static_property(class_name, member) => Some(format!(
            "__DoriaCell::borrowed({}::${member})",
            scopes.static_member_class_symbol(class_name, *span),
        )),
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            span,
            ..
        } if scopes
            .specialization
            .accessor(
                *span,
                crate::ast::PropertyHookKind::Get,
                &scopes.substitutions,
            )
            .is_none() =>
        {
            // Retain the authored class scope for internal stored properties.
            Some(format!(
                "(static function({}object $receiver): __DoriaCell {{ {}return __DoriaCell::borrowed($receiver->{property}); }})({})",
                if *null_safe { "?" } else { "" },
                if *null_safe {
                    "if ($receiver === null) { return new __DoriaCell(null); } "
                } else {
                    ""
                },
                property_hooks::receiver(object, *null_safe, scopes),
            ))
        }
        Expr::Index {
            collection,
            index,
            span,
        } => {
            let source_type = scopes.expression_types.get(&collection.span())?;
            let source_type = match source_type {
                ResolvedType::Nullable(inner) => inner.as_ref(),
                ty => ty,
            };
            let location = format!(
                "{}, {}, {}",
                scopes.source_location(*span, span.start),
                scopes.source_location(*span, span.end),
                scopes.callable_identity(),
            );
            if core_collection::uses_receiver(source_type)
                || matches!(source_type, ResolvedType::SortedDictionary(_, _))
                || bytes::indexed(source_type)
            {
                return Some(diagnostic_sites::operation(
                    format!(
                        "{}->__doriaBorrowCell({}, {location})",
                        core_collection::receiver(collection, false, scopes),
                        emit_expr(index, scopes),
                    ),
                    *span,
                    scopes,
                ));
            }
            if !matches!(
                source_type,
                ResolvedType::TypedArray(_)
                    | ResolvedType::List(_)
                    | ResolvedType::Dictionary(_, _)
            ) {
                return None;
            }
            let owner = borrowed_argument_cell(collection, scopes)?;
            Some(diagnostic_sites::operation(
                format!(
                    "__doria_collection_cell({owner}, {}, {}, {location})",
                    emit_expr(index, scopes),
                    matches!(source_type, ResolvedType::Dictionary(_, _)),
                ),
                *span,
                scopes,
            ))
        }
        _ => None,
    }
}

fn borrowed_result_uses_cell(ty: &ResolvedType) -> bool {
    // Borrowed Move results transport their storage, not a carrier snapshot.
    match ty {
        ResolvedType::Nullable(inner) => borrowed_result_uses_cell(inner),
        _ => crate::php_closure::requires_writable_cell(ty),
    }
}

fn expression_returns_borrow_cell(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    if !scopes
        .expression_types
        .get(&expr.span())
        .is_some_and(borrowed_result_uses_cell)
    {
        return false;
    }
    match expr {
        Expr::FunctionCall { span, .. }
        | Expr::MethodCall { span, .. }
        | Expr::StaticCall { span, .. } => scopes
            .closure_plan
            .callable_at(*span)
            .is_some_and(|plan| plan.returns_borrow),
        Expr::PropertyAccess { span, .. } => scopes
            .specialization
            .accessor(
                *span,
                crate::ast::PropertyHookKind::Get,
                &scopes.substitutions,
            )
            .is_some_and(|accessor| accessor.callable.returns_borrow),
        Expr::CallableCall(call) => scopes
            .closure_plan
            .callable_value_type(scopes.closure_owner, call.span)
            .return_borrow
            .is_some_and(|borrow| borrow.kind == mir::ReturnBorrowKind::Value),
        _ => false,
    }
}

fn emit_returned_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    if !scopes.returns_borrow_cell {
        return emit_expr(expr, scopes);
    }
    let cell = emit_borrowed_result_cell(expr, scopes);
    match (
        scopes.expression_types.get(&expr.span()),
        scopes.interface_conversion_types.get(&expr.span()),
    ) {
        (Some(actual), Some(expected)) => interface::convert_typed_collection(
            &crate::types::substitute_resolved_type(actual, &scopes.substitutions),
            &crate::types::substitute_resolved_type(expected, &scopes.substitutions),
            cell,
            false,
            true,
        ),
        _ => cell,
    }
}

fn emit_borrowed_result_cell(expr: &Expr, scopes: &PhpNameScopes) -> String {
    if let Some(cell) = value_result::borrowed_cell_expression(expr, scopes) {
        return cell;
    }
    match expr {
        Expr::Grouped { expr, .. } => emit_borrowed_result_cell(expr, scopes),
        Expr::Null { .. } => "new __DoriaCell(null)".into(),
        Expr::Binary {
            left,
            op: BinaryOp::Coalesce,
            right,
            span,
        } => {
            let mut source = left.as_ref();
            while let Expr::Grouped { expr, .. } = source {
                source = expr;
            }
            match scopes.flow_facts.get(&source.span()) {
                Some(
                    crate::narrowing::Fact::NonNull
                    | crate::narrowing::Fact::Exact(_)
                    | crate::narrowing::Fact::Constructed { .. },
                ) => {
                    return emit_borrowed_result_cell(left, scopes);
                }
                Some(crate::narrowing::Fact::Null) => {
                    return emit_expression_body(
                        scopes,
                        &format!(
                            "{}; return {};",
                            emit_expr(left, scopes),
                            emit_borrowed_result_cell(right, scopes)
                        ),
                        &[],
                    );
                }
                None if matches!(source, Expr::Null { .. }) => {
                    return emit_borrowed_result_cell(right, scopes);
                }
                None => {}
            }
            let cell = scopes.expression_temp("__doria_coalesced_loan_", *span);
            emit_expression_body(
                scopes,
                &format!(
                    "${cell} = {}; return ${cell}->value !== null ? ${cell} : {};",
                    emit_borrowed_result_cell(left, scopes),
                    emit_borrowed_result_cell(right, scopes),
                ),
                &[],
            )
        }
        _ => borrowed_argument_cell(expr, scopes)
            .expect("checked returned collection loan has a stable storage home"),
    }
}

fn emit_owned_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    if scopes.expression_temporaries.is_none() {
        return with_expression_temporaries(scopes, |scopes| emit_owned_expr(expr, scopes));
    }
    let value = emit_owned_expr_unconverted(expr, scopes);
    interface::convert_collection(expr, value, true, scopes)
}

fn expression_has_move_ownership(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    let ty = scopes
        .expression_types
        .get(&expr.span())
        .expect("checked ownership expression has a source type");
    let ty = crate::types::substitute_resolved_type(ty, &scopes.substitutions);
    crate::ownership::resolved_type_is_move_type(&ty, &scopes.symbols.move_enum_names)
}

fn emit_owned_expr_unconverted(expr: &Expr, scopes: &PhpNameScopes) -> String {
    match expr {
        Expr::This { span } => scopes
            .place_for_use(*span)
            .and_then(PhpBindingPlace::cell)
            .map_or_else(
                || emit_expr_unboxed(expr, scopes),
                |cell| format!("__doria_take_cell({cell})"),
            ),
        Expr::Grouped { expr: inner, .. } => emit_mixed_box_plan(
            expr,
            format!("({})", emit_owned_expr(inner, scopes)),
            scopes,
        ),
        Expr::Variable { name, span } => {
            let emitted = scopes
                .place_for_use(*span)
                .filter(|_| expression_has_move_ownership(expr, scopes))
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
            emit_mixed_box_plan(expr, acquire_copy_claim(expr, emitted, scopes), scopes)
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
            value_result::match_expression(expr, true, scopes)
                .unwrap_or_else(|| emit_match_expression(scrutinee, arms, *span, true, scopes)),
            scopes,
        ),
        Expr::When(when) => emit_mixed_box_plan(
            expr,
            emit_when_expression(when, true, false, scopes),
            scopes,
        ),
        _ => emit_mixed_box_plan(
            expr,
            acquire_copy_claim(expr, emit_expr_unboxed(expr, scopes), scopes),
            scopes,
        ),
    }
}

fn owns_copy_claim(ty: &ResolvedType, scopes: &PhpNameScopes) -> bool {
    let ty = crate::types::substitute_resolved_type(ty, &scopes.substitutions);
    !crate::ownership::resolved_type_is_move_type(&ty, &scopes.symbols.move_enum_names)
        && resolved_type_needs_php_drop(&ty, scopes)
}

fn acquire_copy_claim(expr: &Expr, emitted: String, scopes: &PhpNameScopes) -> String {
    if scopes
        .expression_types
        .get(&expr.span())
        .is_some_and(|ty| owns_copy_claim(ty, scopes))
        && !expression_creates_owner(expr, scopes)
    {
        format!("__doria_copy_value({emitted})")
    } else {
        emitted
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
        diagnostic_sites::operation(
            emit_collection_set(collection, index, &assignment.value, *span, scopes),
            *span,
            scopes,
        )
    }))
}

fn emit_collection_set(
    collection: &Expr,
    index: &Expr,
    value: &Expr,
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    if scopes
        .expression_types
        .get(&collection.span())
        .is_some_and(bytes::indexed)
    {
        return format!(
            "{}->set({}, {}, {}, {}, {})",
            emit_member_receiver(collection, scopes),
            emit_expr(index, scopes),
            emit_expr(value, scopes),
            scopes.source_location(span, span.start),
            scopes.source_location(span, span.end),
            scopes.callable_identity(),
        );
    }
    if let Some(value) = core_collection::assignment(collection, index, value, scopes) {
        return value;
    }
    format!(
        "__doria_collection_set({}, {}, {}, {}, {}, {}, {})",
        emit_assignment_target(collection, scopes),
        emit_expr(index, scopes),
        emit_owned_expr(value, scopes),
        matches!(
            scopes.expression_types.get(&collection.span()),
            Some(ResolvedType::Dictionary(_, _))
        ),
        scopes.source_location(span, span.start),
        scopes.source_location(span, span.end),
        scopes.callable_identity(),
    )
}

fn emit_assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> String {
    if let Some(value) = value_result::assignment_expression(assignment, scopes) {
        return value;
    }
    if scopes.expression_temporaries.is_none() {
        return with_expression_temporaries(scopes, |scopes| emit_assignment(assignment, scopes));
    }
    if let Some(expression) = property_hooks::assignment(assignment, scopes) {
        return expression;
    }
    if let Some(expression) = emit_index_assignment(assignment, scopes) {
        return expression;
    }
    if let Some(expression) = integers::assignment(assignment, scopes) {
        return expression;
    }
    if let Some(expression) = numeric_place::float_assignment(assignment, scopes) {
        return expression;
    }
    let byte_op = bytes::assign_operator(&assignment.op);
    if let Some(op) = byte_op {
        if let Some(value) = bytes::rmw(
            &assignment.target,
            &emit_expr(&assignment.value, scopes),
            op,
            assignment.span,
            assignment.value.span(),
            scopes,
        ) {
            return value;
        }
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
            return format!(
                "(function(object $receiver, mixed $replacement): void {{ $previous = $receiver->{property} ?? null; $receiver->{property} = $replacement; __doria_drop_value($previous); }})({}, {replacement})",
                emit_member_receiver(object, scopes)
            );
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
    if scopes.expression_temporaries.is_none() {
        return with_expression_temporaries(scopes, |scopes| emit_increment(increment, scopes));
    }
    let byte_op = match increment.op {
        IncrementOp::Increment => "+",
        IncrementOp::Decrement => "-",
    };
    if let Some(integer) = integers::ty(scopes.expression_types.get(&increment.target.span())) {
        let assignment = Assignment {
            target: increment.target.clone(),
            op: match increment.op {
                IncrementOp::Increment => AssignOp::AddAssign,
                IncrementOp::Decrement => AssignOp::SubAssign,
            },
            value: Expr::Int {
                value: "1".to_string(),
                span: increment.span,
            },
            span: increment.span,
        };
        if let Some(expression) = property_hooks::assignment(&assignment, scopes) {
            return expression;
        }
        return integers::update(
            &increment.target,
            None,
            byte_op,
            integer,
            increment.span,
            scopes,
        );
    }
    if let Some(value) = bytes::rmw(
        &increment.target,
        "1",
        byte_op,
        increment.span,
        increment.span,
        scopes,
    ) {
        return value;
    }
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
        _ if expression_returns_borrow_cell(expr, scopes) => format!(
            "({})->value",
            borrowed_argument_cell(expr, scopes)
                .expect("checked returned collection loan has a stable storage home"),
        ),
        _ => emit_expr(expr, scopes),
    }
}

fn emit_if(
    if_stmt: &IfStmt,
    output: &mut String,
    indent: usize,
    gate: Option<&str>,
    scopes: &mut PhpNameScopes,
) {
    let condition = emit_condition(&if_stmt.condition, gate, output, indent, scopes);
    writeln(output, indent, &format!("if ({condition})"));
    emit_block(&if_stmt.then_block, output, indent, scopes);

    if let Some(else_branch) = &if_stmt.else_branch {
        match else_branch {
            ElseBranch::If(else_if) => {
                writeln(output, indent, "else {");
                emit_if(else_if, output, indent + 1, gate, scopes);
                writeln(output, indent, "}");
            }
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
    let gate = emit_predicate_gate(&predicates, output, indent, scopes);
    emit_if(if_stmt, output, indent, gate.as_deref(), scopes);
}

fn emit_given_setup(
    given: &GivenPrelude,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Vec<expression_region::Value> {
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
            predicates.push(expression_region::prepare(expr, scopes));
        } else {
            emit_statement(statement, output, indent, scopes);
        }
    }
    predicates
}

fn emit_predicate_gate(
    predicates: &[expression_region::Value],
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<String> {
    if predicates.is_empty() {
        return None;
    }
    let gate = scopes.fresh_temp("__doria_given_gate");
    writeln(output, indent, &format!("${gate} = true;"));
    for predicate in predicates {
        writeln(output, indent, &format!("if (${gate}) {{"));
        predicate.emit(output, indent + 1);
        writeln(
            output,
            indent + 1,
            &format!("${gate} = {};", predicate.value),
        );
        writeln(output, indent, "}");
    }
    Some(format!("${gate}"))
}

fn emit_condition(
    expr: &Expr,
    gate: Option<&str>,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> String {
    let Some(gate) = gate else {
        return expression_region::value(expr, output, indent, scopes);
    };
    let condition = scopes.fresh_temp("__doria_gated_condition");
    writeln(output, indent, &format!("${condition} = false;"));
    writeln(output, indent, &format!("if ({gate}) {{"));
    let value = expression_region::value(expr, output, indent + 1, scopes);
    writeln(output, indent + 1, &format!("${condition} = {value};"));
    writeln(output, indent, "}");
    format!("${condition}")
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
    let iterable = if let Some((dictionary, projection)) = foreach.dictionary_projection() {
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
            writeln(output, indent + 1, &temporary.cleanup());
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
    let iterable_type = scopes.expression_types[&foreach.iterable.span()].clone();
    let (acquire, acquire_plan, acquire_symbol) = scopes
        .specialization
        .iteration_callable(
            &iterable_type,
            crate::compiler_known_contracts::IterationOperation::Acquire,
        )
        .expect("checked public iterable has an acquisition callable");
    let cursor_type = acquire.signature.return_type.clone();
    let acquire_call = emit_iteration_call(iterable, acquire_symbol, acquire_plan);
    let call = |operation| {
        let (_, plan, symbol) = scopes
            .specialization
            .iteration_callable(&cursor_type, operation)
            .expect("checked public iterator has its protocol callables");
        emit_iteration_call(&format!("${cursor}"), symbol, plan)
    };
    let has_current = call(crate::compiler_known_contracts::IterationOperation::HasCurrent);
    let advance = call(crate::compiler_known_contracts::IterationOperation::Advance);
    let (current_requirement, current_plan, current_symbol) = scopes
        .specialization
        .iteration_callable(
            &cursor_type,
            crate::compiler_known_contracts::IterationOperation::GetCurrent,
        )
        .expect("checked public iterator has a current-value callable");
    let returned_cell = current_plan.returns_borrow
        && borrowed_result_uses_cell(&current_requirement.signature.return_type);
    let current_owns = returned_value_owns_carrier(
        &current_requirement.signature.return_type,
        current_plan.returns_borrow,
        scopes,
    );
    let mut current = emit_iteration_call(&format!("${cursor}"), current_symbol, current_plan);
    let binding =
        scopes.binding_for_declaration(&foreach.value_binding.name, foreach.value_binding.span);
    let cell = current_owns || binding.is_some_and(|binding| scopes.needs_cell(binding));
    let current_home =
        (cell && !returned_cell && !current_owns).then(|| scopes.fresh_temp("__doria_current"));
    if returned_cell && !cell {
        current = format!("({current})->value");
    }
    if let Some(binding) = binding {
        scopes.bind_place(
            binding,
            if cell {
                PhpBindingPlace::Cell(value.clone())
            } else {
                PhpBindingPlace::Direct(value.clone())
            },
        );
    }
    writeln(output, indent, &format!("${cursor} = {acquire_call};"));
    writeln(output, indent, "try {");
    writeln(
        output,
        indent + 1,
        &format!("for (; {has_current}; {advance}) {{"),
    );
    if current_owns {
        writeln(
            output,
            indent + 2,
            &format!("${value} = new __DoriaCell({current});"),
        );
        scopes.own_cell(value.clone());
    } else if let Some(home) = &current_home {
        writeln(output, indent + 2, &format!("${home} = {current};"));
        writeln(
            output,
            indent + 2,
            &format!("${value} = __DoriaCell::borrowed(${home});"),
        );
    } else {
        writeln(output, indent + 2, &format!("${value} = {current};"));
    }
    writeln(output, indent + 2, "try {");
    emit_owned_scope(output, indent + 3, scopes, |output, indent, scopes| {
        for statement in &foreach.body.statements {
            emit_statement(statement, output, indent, scopes);
        }
    });
    writeln(output, indent + 2, "} finally {");
    writeln(output, indent + 3, &format!("unset(${value});"));
    if let Some(home) = &current_home {
        writeln(output, indent + 3, &format!("unset(${home});"));
    }
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

fn emit_iteration_call(
    receiver: &str,
    symbol: &str,
    plan: &crate::php_closure::PhpCallablePlan,
) -> String {
    let arguments = if plan.caller_receiver_home() {
        format!("__DoriaCell::borrowed({receiver})")
    } else {
        String::new()
    };
    format!("({receiver})->{symbol}({arguments})")
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
    if let Some(value) = scopes
        .prepared_receiver
        .as_ref()
        .and_then(|receiver| receiver.value(expr))
    {
        return value;
    }
    if let Some(value) = scopes
        .scalar_operands
        .get(&(expression_region::ungrouped(expr) as *const Expr))
    {
        return value.clone();
    }
    if let Some(value) = emit_primitive_core_call(expr, scopes) {
        return value;
    }
    if let Expr::MethodCall { object, span, .. } = expr {
        if scopes.specialization.constrained_display_call(*span) {
            // Semantics checks this call as its receiver's display conversion,
            // and MIR lowers it as one; a primitive receiver has no method.
            return emit_display_expr(object, scopes);
        }
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
    emit_expr_value(expr, emit_expr_unboxed(expr, scopes), scopes)
}

fn emit_expr_value(expr: &Expr, emitted: String, scopes: &PhpNameScopes) -> String {
    let emitted = emit_mixed_box_plan(expr, emitted, scopes);
    let emitted = interface::convert_collection(expr, emitted, false, scopes);
    if expression_creates_owner(expr, scopes) {
        let temporary = scopes.expression_temp("__doria_owned_expression_", expr.span());
        scopes.track_expression_temporary(PhpExpressionTemporary::Owned(temporary.clone()));
        format!("(${temporary} = {emitted})")
    } else {
        emitted
    }
}

fn expression_creates_owner(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    let Some(ty) = scopes.expression_types.get(&expr.span()) else {
        return false;
    };
    if !resolved_type_needs_php_drop(ty, scopes) {
        return false;
    }
    match expr {
        Expr::PropertyAccess { span, .. } => scopes
            .specialization
            .accessor(
                *span,
                crate::ast::PropertyHookKind::Get,
                &scopes.substitutions,
            )
            .is_some_and(|accessor| {
                returned_value_owns_carrier(ty, accessor.callable.returns_borrow, scopes)
            }),
        Expr::New { .. } | Expr::Closure(_) | Expr::Array { .. } => true,
        Expr::FunctionCall { span, .. }
        | Expr::MethodCall { span, .. }
        | Expr::StaticCall { span, .. } => {
            returned_value_owns_carrier(
                ty,
                scopes
                    .closure_plan
                    .callable_at(*span)
                    .is_some_and(|plan| plan.returns_borrow),
                scopes,
            ) && !matches!(expr, Expr::MethodCall { object, method, .. } if scopes.expression_types.get(&object.span()).is_some_and(|ty| php_collection_borrowed_get(ty, method)))
        }
        Expr::CallableCall(call) => returned_value_owns_carrier(
            ty,
            scopes
                .closure_plan
                .callable_value_type(scopes.closure_owner, call.span)
                .return_borrow
                .is_some_and(|borrow| borrow.kind == mir::ReturnBorrowKind::Value),
            scopes,
        ),
        _ => false,
    }
}

fn php_collection_borrowed_get(ty: &ResolvedType, method: &str) -> bool {
    crate::types::CollectionFamily::from_resolved(ty)
        .is_some_and(|family| family.borrows_element(method))
}

fn with_expression_temporaries(
    scopes: &PhpNameScopes,
    emit: impl FnOnce(&PhpNameScopes) -> String,
) -> String {
    if scopes.expression_region.is_some() && scopes.expression_temporaries.is_some() {
        return emit(scopes);
    }
    let mut nested = scopes.clone();
    let temporaries = Rc::new(RefCell::new(Vec::new()));
    nested.expression_temporaries = Some(Rc::clone(&temporaries));
    let expression = emit(&nested);
    let temporaries = temporaries.borrow();
    if temporaries.is_empty() {
        return expression;
    }
    emit_expression_body(scopes, &format!("return {expression};"), &temporaries)
}

fn emit_expression_body(
    scopes: &PhpNameScopes,
    body: &str,
    temporaries: &[PhpExpressionTemporary],
) -> String {
    let captures = expression_capture_list(scopes);
    let cleanup = temporaries
        .iter()
        .rev()
        .map(PhpExpressionTemporary::cleanup)
        .collect::<Vec<_>>()
        .join(" ");
    format!("(function(){captures} {{ try {{ {body} }} finally {{ {cleanup} }} }})()")
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
        emit_php_string_literal(&php_mixed_type_tag(
            &crate::types::substitute_resolved_type(&plan.source_type, &scopes.substitutions)
        ))
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
                    if let Some(copy) = source.and_then(|place| {
                        value_result::storage::copy_capture(
                            acquisition.source_binding_id,
                            place,
                            scopes,
                        )
                    }) {
                        return copy;
                    }
                    if field.ty.requires_owned_cleanup() {
                        format!("new __DoriaCell(__doria_copy_value({read}))")
                    } else {
                        format!("new __DoriaCell({read})")
                    }
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
    let function_type = scopes
        .closure_plan
        .callable_value_type(scopes.closure_owner, call.span);
    let field_arguments = scopes
        .field_arguments
        .as_ref()
        .filter(|(span, _)| *span == call.span);
    let arguments = field_arguments.map_or_else(
        || {
            call.args
                .iter()
                .enumerate()
                .map(|(index, argument)| {
                    emit_callable_argument(&argument.value, index, function_type, scopes)
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
        |(_, values)| values.join(", "),
    );
    let callee = if function_type.invocation_mode == mir::FunctionInvocationMode::Once {
        assignment_target_cell(&call.callee, scopes)
            .map(|cell| format!("__doria_take_cell({cell})"))
            .unwrap_or_else(|| emit_expr(&call.callee, scopes))
    } else {
        emit_expr(&call.callee, scopes)
    };
    let entry = value_argument::Transport::for_call(
        field_arguments.is_some(),
        scopes.field_call == Some(call.span),
    )
    .callback_entry();
    format!("({callee})->{entry}({arguments})")
}

fn emit_callable_argument(
    argument: &Expr,
    index: usize,
    function_type: &mir::FunctionType,
    scopes: &PhpNameScopes,
) -> String {
    value_argument::Input::callback(argument, index, function_type, scopes)
        .expression(argument, scopes)
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
                let value = if owns_copy_claim(&call.element_type, scopes) {
                    "__doria_copy_value($__doriaAlgorithmElement)".into()
                } else if clone {
                    let call = emit_core_method(
                        "$__doriaAlgorithmElement",
                        &[],
                        &call.element_type,
                        crate::compiler_known_contracts::CoreValueOperation::Clone,
                        scopes,
                    );
                    format!("($__doriaAlgorithmElement === null ? null : {call})")
                } else {
                    "$__doriaAlgorithmElement".into()
                };
                body.push_str(&format!("if (($__doriaAlgorithmCallback)($__doriaAlgorithmElement)) {{ $__doriaAlgorithmResult[] = {value}; }} "));
            }
            body.push_str("} } catch (__DoriaCheckedError $__doriaAlgorithmError) { __doria_drop_value($__doriaAlgorithmResult); __doria_rethrow_checked_error($__doriaAlgorithmError); } return $__doriaAlgorithmResult; ");
        }
        ListAlgorithmKind::Reduce => {
            body.push_str("$__doriaAlgorithmAccumulator = new __DoriaCell($__doriaAlgorithmInitial); try { foreach ($__doriaAlgorithmSource as $__doriaAlgorithmElement) { ($__doriaAlgorithmCallback)($__doriaAlgorithmAccumulator, $__doriaAlgorithmElement); } } catch (__DoriaCheckedError $__doriaAlgorithmError) { __doria_drop_cell($__doriaAlgorithmAccumulator); __doria_rethrow_checked_error($__doriaAlgorithmError); } return __doria_take_cell($__doriaAlgorithmAccumulator); ");
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

fn emit_core_method(
    receiver: &str,
    arguments: &[&str],
    ty: &ResolvedType,
    operation: crate::compiler_known_contracts::CoreValueOperation,
    scopes: &PhpNameScopes,
) -> String {
    let ty = crate::types::substitute_resolved_type(ty, &scopes.substitutions);
    let (method, plan) = scopes
        .specialization
        .core_callable(&ty, operation)
        .expect("checked core operation has a specialized callable plan");
    let mut inputs = Vec::new();
    if plan.caller_receiver_home() {
        inputs.push(format!("__DoriaCell::borrowed({receiver})"));
    }
    inputs.extend(arguments.iter().enumerate().map(|(index, argument)| {
        if plan.parameters[index].cell {
            format!("__DoriaCell::borrowed({argument})")
        } else {
            (*argument).to_string()
        }
    }));
    let call = format!("{receiver}->{method}({})", inputs.join(", "));
    if plan.returns_borrow && borrowed_result_uses_cell(&ty) {
        format!("({call})->value")
    } else {
        call
    }
}

fn emit_core_equality(
    left: &str,
    right: &str,
    ty: &ResolvedType,
    scopes: &PhpNameScopes,
) -> String {
    let helper = core_value::helper(
        ty,
        crate::compiler_known_contracts::CoreValueOperation::Equal,
        scopes,
    );
    format!("{helper}({left}, {right})")
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
        let helper = core_value::helper(
            ty.unwrap(),
            crate::compiler_known_contracts::CoreValueOperation::Equal,
            scopes,
        );
        emit_php_string_literal(&helper)
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
    // Places remain places. Computed operations alone get metadata scopes;
    // their original receiver, argument, cell and ownership emission is intact.
    let diagnostic = matches!(
        expr,
        Expr::FunctionCall { .. }
            | Expr::MethodCall { .. }
            | Expr::StaticCall { .. }
            | Expr::New { .. }
            | Expr::CallableCall(_)
            | Expr::ListAlgorithmCall(_)
            | Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Index { .. }
            | Expr::ArrayRepeat { .. }
            | Expr::InterpolatedString { .. }
    ) || matches!(expr, Expr::PropertyAccess { span, .. }
        if scopes.specialization.accessor(*span, crate::ast::PropertyHookKind::Get,
            &scopes.substitutions).is_some());
    let value = expression_region::guarded(
        expr,
        emit_expr_unboxed_impl(expr, scopes),
        diagnostic,
        scopes,
    );
    if expression_returns_borrow_cell(expr, scopes) {
        format!("({value})?->value")
    } else {
        value
    }
}

fn emit_expr_unboxed_impl(expr: &Expr, scopes: &PhpNameScopes) -> String {
    if let Some(value) = value_result::null_comparison(expr, scopes) {
        return value;
    }
    if let Some(value) = value_argument::expression(expr, scopes) {
        return value;
    }
    if let Some(value) = bytes::expression(expr, scopes) {
        return value;
    }
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
            format!("__doria_const_{}()", php_constant_symbol(name))
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
                .is_some_and(|ty| owns_copy_claim(ty, scopes))
            {
                "__doria_copy_value($source)".into()
            } else if scopes
                .expression_types
                .get(&value.span())
                .is_some_and(|ty| {
                    scopes.specialization.core_operation(
                        value.span(),
                        crate::compiler_known_contracts::CoreValueOperation::Clone,
                        ty,
                        &scopes.substitutions,
                    )
                })
            {
                let call = emit_core_method(
                    "$source",
                    &[],
                    &scopes.expression_types[&value.span()],
                    crate::compiler_known_contracts::CoreValueOperation::Clone,
                    scopes,
                );
                format!("($source === null ? null : {call})")
            } else {
                "$source".into()
            };
            format!(
                "(static function ($source, int $count) {{ if ($count < 0) {{ __doria_panic(\"P1311\", {}, {}, null, {}, [\"count\" => [1, $count]]); }} $result = []; try {{ for ($index = 0; $index < $count; ++$index) {{ $result[] = {duplicate}; }} }} catch (__DoriaCheckedError $error) {{ __doria_drop_value($result); __doria_rethrow_checked_error($error); }} return $result; }})({}, {})",
                scopes.source_location(count.span(), count.span().start),
                scopes.source_location(count.span(), count.span().end),
                scopes.callable_identity(),
                emit_expr(value, scopes),
                emit_expr(count, scopes)
            )
        }
        Expr::Index {
            collection,
            index,
            span,
        } if scopes
            .expression_types
            .get(&collection.span())
            .is_some_and(bytes::indexed) =>
        {
            format!(
                "{}->get({}, {}, {}, {})",
                emit_member_receiver(collection, scopes),
                emit_expr(index, scopes),
                scopes.source_location(*span, span.start),
                scopes.source_location(*span, span.end),
                scopes.callable_identity(),
            )
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
            scopes.source_location(*span, span.start),
            scopes.source_location(*span, span.end),
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
            .is_some_and(|ty| php_collection_property(ty, property)) =>
        {
            let receiver = emit_expr(object, scopes);
            if matches!(property.as_str(), "first" | "last") {
                let key = if property == "first" {
                    "array_key_first"
                } else {
                    "array_key_last"
                };
                let value = format!("($sequence === [] ? null : $sequence[{key}($sequence)])");
                let value = if *null_safe {
                    format!("($sequence === null ? null : {value})")
                } else {
                    value
                };
                return format!("(static fn($sequence) => {value})({receiver})");
            }
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
        // Extraction reads and unsets the field, then destroys the rest of the
        // consumed receiver; its class has no destructor to observe the gap.
        Expr::PropertyAccess {
            object,
            property,
            take: true,
            span,
            ..
        } => {
            let field = emit_expr(
                &Expr::PropertyAccess {
                    object: object.clone(),
                    property: property.clone(),
                    null_safe: false,
                    take: false,
                    span: *span,
                },
                scopes,
            );
            let receiver = emit_owned_expr_unconverted(object, scopes);
            let release =
                if scopes.current_class.as_ref().is_some_and(|class| {
                    scopes.symbols.classes_with_php_destructors.contains(class)
                }) {
                    "$__doriaTakenOwner->__doriaReleaseTaken();"
                } else {
                    "__doria_drop_value($__doriaTakenOwner);"
                };
            emit_expression_body(
                scopes,
                &format!(
                    "$__doriaTaken = {field}; unset({field}); $__doriaTakenOwner = {receiver}; {release} return $__doriaTaken;"
                ),
                &[],
            )
        }
        Expr::PropertyAccess {
            object,
            property,
            null_safe,
            span,
            ..
        } => {
            if let Some(value) = property_hooks::getter(object, *null_safe, *span, scopes) {
                return value;
            }
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
            span,
            null_safe: false,
            ..
        } if matches!(
            scopes.expression_types.get(&object.span()),
            Some(ResolvedType::List(_))
        ) && matches!(method.as_str(), "removeAt" | "pop" | "insertAt" | "clear")
            || matches!(
                scopes.expression_types.get(&object.span()),
                Some(ResolvedType::Dictionary(_, _))
            ) && matches!(method.as_str(), "remove" | "clear") =>
        {
            let owner = borrowed_argument_cell(object, scopes).unwrap_or_else(|| {
                core_collection::temporary_cell(
                    emit_owned_expr(object, scopes),
                    object.span(),
                    "__doria_collection_receiver_",
                    scopes,
                )
            });
            if method == "clear" {
                return format!("__doria_replace_cell({owner}, [])");
            }
            if method == "pop" {
                return format!("__doria_list_pop({owner})");
            }
            if method == "insertAt" {
                return format!(
                    "__doria_list_insert_at({owner}, {}, {}, {}, {}, {})",
                    emit_expr(&args[0].value, scopes),
                    emit_owned_expr(&args[1].value, scopes),
                    scopes.source_location(*span, span.start),
                    scopes.source_location(*span, span.end),
                    scopes.callable_identity(),
                );
            }
            if method == "remove" {
                return format!(
                    "__doria_dictionary_remove({owner}, {})",
                    emit_expr(&args[0].value, scopes)
                );
            }
            format!(
                "__doria_list_remove_at({owner}, {}, {}, {}, {})",
                emit_expr(&args[0].value, scopes),
                scopes.source_location(*span, span.start),
                scopes.source_location(*span, span.end),
                scopes.callable_identity(),
            )
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
            span,
            null_safe: false,
        } if matches!(
            scopes.expression_types.get(&object.span()),
            Some(ResolvedType::Dictionary(_, _))
        ) && method == "set" =>
        {
            emit_collection_set(object, &args[0].value, &args[1].value, *span, scopes)
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
                if scopes
                    .closure_plan
                    .callable_at(*span)
                    .is_some_and(|plan| plan.consumes_receiver)
                {
                    return emit_consuming_method_call(
                        emit_owned_expr(object, scopes),
                        scopes.call_symbol(*span).unwrap_or(method),
                        args,
                        *span,
                        *null_safe,
                        false,
                        scopes,
                    );
                }
                if scopes
                    .closure_plan
                    .callable_at(*span)
                    .is_some_and(|plan| plan.caller_receiver_home())
                {
                    return emit_method_with_receiver_home(
                        object,
                        scopes.call_symbol(*span).unwrap_or(method),
                        || emit_method_arguments_for_call(object, method, args, *span, scopes),
                        *span,
                        *null_safe,
                        scopes,
                    );
                }
                format!(
                    "{}{}{}({})",
                    emit_member_receiver(object, scopes),
                    if *null_safe { "?->" } else { "->" },
                    scopes.call_symbol(*span).unwrap_or(method),
                    emit_method_arguments_for_call(object, method, args, *span, scopes)
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
                if element.is_some_and(|ty| owns_copy_claim(ty, scopes)) {
                    return format!(
                        "{class_name}::from({}, {}\"__doria_copy_value\")",
                        emit_expr(&args[0].value, scopes),
                        if class_name == "SortedDictionary" {
                            format!("{}, ", php_unsigned_collection(expr, scopes))
                        } else {
                            String::new()
                        }
                    );
                }
                if element.is_some_and(|ty| {
                    scopes.specialization.core_operation(
                        args[0].value.span(),
                        crate::compiler_known_contracts::CoreValueOperation::Clone,
                        ty,
                        &scopes.substitutions,
                    )
                }) {
                    let clone = core_value::helper(
                        element.unwrap(),
                        crate::compiler_known_contracts::CoreValueOperation::Clone,
                        scopes,
                    );
                    return format!(
                        "{class_name}::from({}, {}{})",
                        emit_expr(&args[0].value, scopes),
                        if class_name == "SortedDictionary" {
                            format!("{}, ", php_unsigned_collection(expr, scopes))
                        } else {
                            String::new()
                        },
                        emit_php_string_literal(&clone),
                    );
                }
            }
            if let (Some(target), "from", [argument]) = (
                IntegerType::from_companion_name(class_name),
                method.as_str(),
                args.as_slice(),
            ) {
                let source = integers::ty(scopes.expression_types.get(&argument.value.span()))
                    .expect("PHP capability validation admits typed integer conversions only");
                return integers::convert(&argument.value, source, target, *span, scopes);
            }
            if class_name == "String" && method == "fromBytes" && args.len() == 1 {
                return format!("{}->__doriaUtf8String()", emit_expr(&args[0].value, scopes));
            }
            if class_name == "Bytes" && method == "fromArray" && args.len() == 1 {
                return format!(
                    "__DoriaBytes::fromArray({}, {}, {}, {})",
                    emit_expr(&args[0].value, scopes),
                    scopes.source_location(*span, span.start),
                    scopes.source_location(*span, span.end),
                    scopes.callable_identity(),
                );
            }
            if class_name == "Bytes" && method == "allocate" && args.len() == 1 {
                let length = &args[0].value;
                let span = length.span();
                return format!(
                    "__DoriaBytes::allocate({}, {}, {}, {})",
                    emit_expr(length, scopes),
                    scopes.source_location(span, span.start),
                    scopes.source_location(span, span.end),
                    scopes.callable_identity(),
                );
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
                    .or_else(|| {
                        scopes
                            .specialization
                            .enum_case_symbol(*span, &scopes.substitutions)
                    })
                    .unwrap_or_else(|| php_type_symbol(class_name))
            };
            let method = scopes.call_symbol(*span).unwrap_or(method);
            if scopes.direct_parent_calls.contains(span)
                && scopes
                    .closure_plan
                    .callable_at(*span)
                    .is_some_and(|plan| plan.consumes_receiver)
            {
                let receiver = scopes
                    .lookup("this")
                    .map(|cell| format!("__doria_take_cell(${cell})"))
                    .expect("consuming parent call requires an owned receiver");
                return emit_consuming_method_call(
                    receiver, method, args, *span, false, true, scopes,
                );
            }
            let arguments = if scopes.is_payload_enum_expression(expr) {
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
            };
            let arguments = if scopes.direct_parent_calls.contains(span)
                && scopes
                    .closure_plan
                    .callable_at(*span)
                    .is_some_and(|plan| plan.caller_receiver_home())
            {
                let Some(mir::ClosureOwner::Callable(instance)) = scopes.closure_owner else {
                    unreachable!("parent call belongs to a method");
                };
                let binding = scopes
                    .closure_plan
                    .binding_resolution
                    .receiver_for_callable(scopes.specialization.callable(instance).declaration)
                    .and_then(|binding| scopes.place(binding))
                    .and_then(PhpBindingPlace::cell)
                    .expect("checked parent receiver has a storage home");
                format!(
                    "{binding}{}",
                    if arguments.is_empty() {
                        String::new()
                    } else {
                        format!(", {arguments}")
                    }
                )
            } else {
                arguments
            };
            format!("{}::{method}({})", qualifier, arguments)
        }
        Expr::StaticMember {
            class_name, span, ..
        } if scopes
            .specialization
            .accessor(
                *span,
                crate::ast::PropertyHookKind::Get,
                &scopes.substitutions,
            )
            .is_some() =>
        {
            let accessor = scopes
                .specialization
                .accessor(
                    *span,
                    crate::ast::PropertyHookKind::Get,
                    &scopes.substitutions,
                )
                .expect("checked static getter");
            let symbol = scopes.transport_symbol(*span, accessor.symbol);
            format!(
                "{}::{symbol}()",
                scopes.static_member_class_symbol(class_name, *span)
            )
        }
        Expr::StaticMember {
            class_name,
            member,
            span,
        } if scopes.is_static_property(class_name, member) => {
            format!(
                "{}::${member}",
                scopes.static_member_class_symbol(class_name, *span)
            )
        }
        Expr::StaticMember {
            class_name,
            member,
            span,
        } if scopes.is_payload_unit_case(class_name, member) => {
            format!(
                "{}::{}()",
                scopes
                    .specialization
                    .enum_case_symbol(*span, &scopes.substitutions)
                    .unwrap_or_else(|| php_type_symbol(class_name)),
                php_payload_case_method(member, false)
            )
        }
        Expr::StaticMember {
            class_name,
            member,
            span,
        } if scopes.is_payload_class_constant(class_name, member) => {
            format!(
                "{}::__doriaConst{member}()",
                scopes.static_member_class_symbol(class_name, *span)
            )
        }
        Expr::StaticMember {
            class_name,
            member,
            span,
        } => format!(
            "{}::{member}",
            scopes.static_member_class_symbol(class_name, *span)
        ),
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
                .unwrap_or_else(|| php_type_symbol(&class_type.name));
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
        Expr::Unary { op, expr, span } => match op {
            UnaryOp::Not => format!("!({})", emit_expr(expr, scopes)),
            UnaryOp::Negate if integer_literal_magnitude(expr) == Some((i64::MAX as u128) + 1) => {
                "(-9223372036854775807 - 1)".to_string()
            }
            UnaryOp::Negate
                if integer_literal_magnitude(expr).is_none()
                    && integers::ty(scopes.expression_types.get(&expr.span())).is_some() =>
            {
                integers::operation(
                    "neg",
                    &emit_expr(expr, scopes),
                    "0",
                    integers::ty(scopes.expression_types.get(&expr.span())).unwrap(),
                    *span,
                    scopes,
                )
            }
            UnaryOp::Negate => format!("-({})", emit_expr(expr, scopes)),
            UnaryOp::BitwiseNot => integers::operation(
                "~",
                &emit_expr(expr, scopes),
                "0",
                integers::ty(scopes.expression_types.get(&expr.span()))
                    .expect("PHP capability validation admits `~` on a typed integer only"),
                *span,
                scopes,
            ),
        },
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                if scopes.expression_types.get(&left.span())
                    == Some(&ResolvedType::Float(FloatType::Float64)) =>
            {
                numeric_place::float_binary(left, emit_binary_op(op), right, scopes)
            }
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::ShiftLeft
            | BinaryOp::ShiftRight
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseOr
                if integers::ty(scopes.expression_types.get(&left.span())).is_some() =>
            {
                integers::binary(
                    left,
                    emit_binary_op(op),
                    right,
                    integers::ty(scopes.expression_types.get(&left.span())).unwrap(),
                    *span,
                    scopes,
                )
            }
            BinaryOp::Equal | BinaryOp::NotEqual
                if scopes.expression_types.get(&left.span()).is_some_and(|ty| {
                    php_builtin_value_equality(&crate::types::substitute_resolved_type(
                        ty,
                        &scopes.substitutions,
                    ))
                }) || scopes
                    .expression_types
                    .get(&right.span())
                    .is_some_and(|ty| {
                        php_builtin_value_equality(&crate::types::substitute_resolved_type(
                            ty,
                            &scopes.substitutions,
                        ))
                    }) =>
            {
                format!(
                    "{}__doria_equal({}, {})",
                    if *op == BinaryOp::NotEqual { "!" } else { "" },
                    emit_expr(left, scopes),
                    emit_expr(right, scopes),
                )
            }
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
                let negate = if *op == BinaryOp::NotEqual { "!" } else { "" };
                format!(
                    "{negate}{}",
                    emit_core_equality(
                        &emit_expr(left, scopes),
                        &emit_expr(right, scopes),
                        &scopes.expression_types[&left.span()],
                        scopes
                    )
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
        } => value_result::match_expression(expr, false, scopes)
            .unwrap_or_else(|| emit_match_expression(scrutinee, arms, *span, false, scopes)),
        Expr::When(when) => emit_when_expression(when, false, false, scopes),
    }
}

fn emit_when_expression(
    when: &WhenExpression,
    owned_result: bool,
    borrowed_cell_result: bool,
    scopes: &PhpNameScopes,
) -> String {
    scopes
        .whens
        .get(&when.span)
        .expect("checked when expression must have a semantic plan");
    let mut when_scopes = scoped_when(scopes, owned_result, borrowed_cell_result);
    let mut body = String::new();
    emit_when_body(when, &mut body, 1, &mut when_scopes);
    let capture_list = expression_capture_list(scopes);
    format!("(function(){capture_list} {{\n{body}}})()")
}

fn scoped_when(
    scopes: &PhpNameScopes,
    owned_result: bool,
    borrowed_cell_result: bool,
) -> PhpNameScopes {
    let mut when_scopes = scopes.clone();
    when_scopes.value_return = None;
    when_scopes.field_call = None;
    when_scopes.pending_returns.clear();
    when_scopes.expression_temporaries = None;
    when_scopes.expression_region = None;
    when_scopes.yield_temporaries = if owned_result {
        None
    } else {
        scopes.expression_temporaries.clone()
    };
    when_scopes.returns_borrow = !owned_result;
    when_scopes.returns_borrow_cell = borrowed_cell_result;
    when_scopes.push();
    when_scopes
}

fn emit_when_body(
    when: &WhenExpression,
    body: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    emit_owned_scope(body, indent, scopes, |body, indent, when_scopes| {
        if let Some(finally) = &when.finally {
            emit_finalizer_error_boundary(
                body,
                indent,
                when_scopes,
                |body, indent, when_scopes| {
                    writeln(body, indent, "try");
                    writeln(body, indent, "{");
                    emit_when_branches(
                        when.given.as_ref(),
                        &when.branches,
                        body,
                        indent + 1,
                        when_scopes,
                    );
                    writeln(body, indent, "}");
                    writeln(body, indent, "finally");
                    emit_block(&finally.block, body, indent, when_scopes);
                },
            );
        } else {
            emit_when_branches(
                when.given.as_ref(),
                &when.branches,
                body,
                indent,
                when_scopes,
            );
        }
    });
}

fn expression_capture_list(scopes: &PhpNameScopes) -> String {
    let mut captures = scopes.captured_php_names();
    if let Some(name) = scopes
        .expression_region
        .as_ref()
        .and_then(|region| region.borrow().captured_name().map(str::to_owned))
    {
        captures.push(name);
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
    let gate = emit_predicate_gate(&predicates, body, indent, scopes);
    emit_when_branch_chain(branches, gate.as_deref(), body, indent, scopes);
}

fn emit_when_branch_chain(
    branches: &[WhenBranch],
    gate: Option<&str>,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let Some((branch, rest)) = branches.split_first() else {
        return;
    };
    if let Some(condition) = &branch.condition {
        let condition = emit_condition(condition, gate, output, indent, scopes);
        writeln(output, indent, &format!("if ({condition})"));
        emit_block(&branch.block, output, indent, scopes);
        if !rest.is_empty() {
            writeln(output, indent, "else {");
            emit_when_branch_chain(rest, gate, output, indent + 1, scopes);
            writeln(output, indent, "}");
        }
    } else {
        emit_block(&branch.block, output, indent, scopes);
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
            let passed = arm_scopes.fresh_temp("__doria_match_guard");
            let mut guard_scopes = arm_scopes.clone();
            guard_scopes.push();
            emit_owned_scope(
                &mut output,
                0,
                &mut guard_scopes,
                |output, indent, guard_scopes| {
                    emit_php_match_bindings(
                        output,
                        &temporary,
                        &arm.pattern,
                        arm_info,
                        &info.scrutinee_type,
                        guard_scopes,
                        false,
                        false,
                    );
                    writeln(
                        output,
                        indent,
                        &format!("${passed} = {};", emit_expr(&guard.condition, guard_scopes)),
                    );
                },
            );
            output.push_str(&format!("if (${passed} === true) {{ "));
        }
        emit_owned_scope(
            &mut output,
            0,
            &mut arm_scopes,
            |output, indent, arm_scopes| {
                emit_php_match_bindings(
                    output,
                    &temporary,
                    &arm.pattern,
                    arm_info,
                    &info.scrutinee_type,
                    arm_scopes,
                    false,
                    consumed,
                );
                let value = if owned_result {
                    emit_owned_expr(&arm.value, arm_scopes)
                } else {
                    emit_expr(&arm.value, arm_scopes)
                };
                writeln(output, indent, &format!("return {value};"));
            },
        );
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
            let pattern = emit_const_value(constant, &scopes.specialization);
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
                Some(format!(
                    "{value} === {}::{case}",
                    php_type_symbol(qualifier)
                ))
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
        info.bindings
            .iter()
            .find(|binding| binding.name == name)
            .is_some_and(|binding| {
                !binding.borrowed
                    && resolved_type_needs_php_drop(&binding.ty, arm_scopes)
                    && (consumed || owns_copy_claim(&binding.ty, arm_scopes))
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
                if reuse
                    && owned
                    && info
                        .bindings
                        .iter()
                        .find(|field| field.name == binding.name)
                        .is_some_and(|field| owns_copy_claim(&field.ty, arm_scopes))
                {
                    output.push_str(&format!("__doria_drop_cell(${name}); "));
                }
                let method = if owned && consumed {
                    "__doriaTakePayload"
                } else {
                    "__doriaPayloadAt"
                };
                emit_php_match_binding(
                    output,
                    &name,
                    binding,
                    if owned && !consumed {
                        format!("__doria_copy_value(${temporary}->{method}({index}))")
                    } else {
                        format!("${temporary}->{method}({index})")
                    },
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
            let mut value = if is_mixed_storage_type(scrutinee_type) {
                format!("__doria_mixed_value(${temporary})")
            } else {
                format!("${temporary}")
            };
            let owned = ownership.get(&binding.name).copied().unwrap_or(false);
            if owned && !consumed {
                value = format!("__doria_copy_value({value})");
            }
            if reuse
                && owned
                && info
                    .bindings
                    .iter()
                    .find(|field| field.name == binding.name)
                    .is_some_and(|field| owns_copy_claim(&field.ty, arm_scopes))
            {
                output.push_str(&format!("__doria_drop_cell(${name}); "));
            }
            emit_php_match_binding(output, &name, binding, value, owned, arm_scopes);
            if owned && consumed {
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
    let exact = &crate::types::substitute_resolved_type(exact, &scopes.substitutions);
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
                    "__doria_mixed_is({value}, {})",
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
        ResolvedType::Enum(ty) if ty.arguments.is_empty() => format!("enum:{}", ty.name),
        ResolvedType::Enum(_) => resolved_type_identity(ty),
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
        ResolvedType::ByteRegion(kind) => kind.source_name().to_string(),
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
        ResolvedType::TraitSelf(_) => {
            unreachable!("unsubstituted trait self reached executable PHP lowering")
        }
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
                        "{}-{}-parameter-{index}",
                        match borrow.kind {
                            crate::types::ReturnBorrowKind::Value => "value",
                            crate::types::ReturnBorrowKind::Retained => "retained",
                        },
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
        ResolvedType::Enum(ty) if ty.arguments.is_empty() => {
            format!("enum:{}:{}", ty.name.len(), ty.name)
        }
        ResolvedType::Enum(ty) => format!(
            "enum:{}:{}[{}]",
            ty.name.len(),
            ty.name,
            list(ty.arguments.iter().map(resolved_type_identity))
        ),
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
        IO_PROCESS_ERRORS_SUFFIX, IO_PROCESS_INPUT_SUFFIX, IO_PROCESS_MESSAGE_PREFIX,
        IO_PROCESS_MESSAGE_SUFFIX, IO_PROCESS_OUTPUT_SUFFIX, IO_READINESS_SET_NAME,
        IO_STANDARD_ERROR_NAME, IO_STANDARD_INPUT_NAME, IO_STANDARD_OUTPUT_NAME,
        IO_TRANSFERRED_BYTE_COUNT_HEADING, IO_TRANSFER_GROUP_NAME, IO_TRANSFER_PLAN_NAME,
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
        ("readinessSet", IO_READINESS_SET_NAME),
        ("transferPlan", IO_TRANSFER_PLAN_NAME),
        ("transferGroup", IO_TRANSFER_GROUP_NAME),
        ("processPrefix", IO_PROCESS_MESSAGE_PREFIX),
        ("processSuffix", IO_PROCESS_MESSAGE_SUFFIX),
        ("processInputSuffix", IO_PROCESS_INPUT_SUFFIX),
        ("processOutputSuffix", IO_PROCESS_OUTPUT_SUFFIX),
        ("processErrorsSuffix", IO_PROCESS_ERRORS_SUFFIX),
        ("invalidUtf8Prefix", INVALID_UTF8_MESSAGE_PREFIX),
        ("progressHeading", IO_TRANSFERRED_BYTE_COUNT_HEADING),
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
            .map(|operation| {
                format!(
                    "{} => {}",
                    emit_php_string_literal(operation.case_name()),
                    emit_php_string_literal(operation.word()),
                )
            })
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

fn emit_primitive_comparison_runtime(output: &mut String) {
    let ordering = php_type_symbol("Ordering");
    for (name, less) in [
        (
            "__doria_compare_primitive",
            "__doria_less($left, $right)".to_string(),
        ),
        (
            "__doria_compare_uint64",
            emit_uint64_comparison("$left", "<", "$right"),
        ),
    ] {
        output.push_str(&format!(
            "function {name}($left, $right)\n{{\n    return $left === $right ? {ordering}::Equal : ({less} ? {ordering}::Less : {ordering}::Greater);\n}}\n\n"
        ));
    }
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
            let helper = if is_uint64(Some(&receiver)) {
                "__doria_compare_uint64"
            } else {
                "__doria_compare_primitive"
            };
            format!("{helper}({left}, {right})")
        }
        CoreValueOperation::Clone => return None,
    })
}

fn emit_display_expr(expr: &Expr, scopes: &PhpNameScopes) -> String {
    let value = emit_expr(expr, scopes);
    let displayed = if is_uint64(scopes.expression_types.get(&expr.span())) {
        format!("sprintf(\"%u\", {value})")
    } else {
        format!("__doria_display({value})")
    };
    expression_region::operation(expr, displayed, scopes)
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
    if Builtin::from_name(name).is_some_and(Builtin::uses_bytes) {
        return standard_io::emit_byte_intrinsic(name, args, span, scopes);
    }
    let helper = match name {
        "read_line" => "__doria_read_line".to_string(),
        "read_file" => "__doria_read_file".to_string(),
        "write_file" => "__doria_write_file".to_string(),
        "append_file" => "__doria_append_file".to_string(),
        "write_stderr" => "__doria_write_stderr".to_string(),
        "sprintf" => "__doria_sprintf".to_string(),
        "printf" => "__doria_printf".to_string(),
        _ => scopes
            .call_symbol(span)
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
                            diagnostic_sites::operation(
                                format!("__doria_display({argument})"),
                                source.value.span(),
                                scopes,
                            )
                        };
                    }
                }
            }
        }
    }
    if name == "printf" {
        emitted.insert(0, scopes.source_location(span, span.end).to_string());
        emitted.insert(0, scopes.source_location(span, span.start).to_string());
        emitted.insert(2, scopes.callable_identity());
    } else if matches!(
        name,
        "read_line" | "read_file" | "write_file" | "append_file" | "write_stderr"
    ) {
        if name == "read_line" && emitted.is_empty() {
            emitted.push("\"\"".to_string());
        }
        emitted.push(format!(
            "start: {}",
            scopes.source_location(span, span.start)
        ));
        emitted.push(format!("end: {}", scopes.source_location(span, span.end)));
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

/// Doria's checker owns visibility. Classes of one compiler-owned standard
/// source share internal members, which PHP `private` would forbid.
fn emit_member_access(access: &MemberAccess, source: crate::source::SourceId) -> &'static str {
    match access {
        MemberAccess::External => "public",
        MemberAccess::Internal if source.is_compiler_owned() => "public",
        MemberAccess::Internal => "private",
    }
}

fn php_type(ty: &TypeRef, scopes: &PhpNameScopes) -> String {
    if let Some(grouped) = &ty.grouped {
        // Grouping preserves syntax only; nullability can belong to any wrapper.
        let mut inner = grouped.inner.clone();
        inner.nullable |= ty.nullable;
        return php_type(&inner, scopes);
    }
    if let Some(resolved) = scopes
        .specialization
        .resolve_type(ty, &scopes.substitutions)
        .filter(core_collection::uses)
    {
        return php_resolved_type(&resolved, scopes);
    }
    if scopes.substitutions.contains_key(&ty.name) {
        let resolved = scopes
            .specialization
            .resolve_type(ty, &scopes.substitutions)
            .expect("checked specialization has a concrete type");
        return php_resolved_type(&resolved, scopes);
    }
    if let Some(symbol) = scopes
        .specialization
        .class_symbol_for_type(ty, &scopes.substitutions)
    {
        return format!("{}{symbol}", if ty.nullable { "?" } else { "" });
    }
    if let Some(resolved) = scopes
        .specialization
        .resolve_type(ty, &scopes.substitutions)
        .filter(|resolved| match resolved {
            ResolvedType::Nullable(inner) => matches!(inner.as_ref(), ResolvedType::Enum(_)),
            resolved => matches!(resolved, ResolvedType::Enum(_)),
        })
    {
        return php_resolved_type(&resolved, scopes);
    }
    let name = if ty.function.is_some() {
        "__DoriaFunctionValue".to_string()
    } else if IntegerType::from_source_name(&ty.name).is_some() {
        "int".to_string()
    } else if FloatType::from_source_name(&ty.name).is_some() {
        "float".to_string()
    } else if crate::types::ByteRegionKind::from_source_name(&ty.name).is_some() {
        "__DoriaByteRegion".to_string()
    } else if crate::types::SharedHandleKind::from_source_name(&ty.name).is_some() {
        "__DoriaSharedHandle".to_string()
    } else if scopes.symbols.interface_names.contains(&ty.name) {
        interface::declaration_name(&ty.name)
    } else {
        match ty.name.as_str() {
            "Bytes" => "__DoriaBytes".to_string(),
            "List" | "Dictionary" | "Set" | "[]" => "array".to_string(),
            "Error" => "__DoriaErrorValue".to_string(),
            "bool" | "string" | "mixed" | "void" | "self" | "parent" => ty.name.clone(),
            name => php_type_symbol(name),
        }
    };
    if ty.nullable {
        php_nullable_type_hint(name)
    } else {
        name
    }
}

fn php_nullable_type_hint(name: String) -> String {
    // PHP's host hint includes null in `mixed`; Doria's separate nullable
    // carrier and exact mixed payload tag remain defined by generated code.
    if name == "mixed" || name == "null" {
        "mixed".to_string()
    } else {
        format!("?{name}")
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
        ResolvedType::TraitSelf(_) => {
            unreachable!("unsubstituted trait self reached executable PHP lowering")
        }
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
        ResolvedType::Enum(ty) => scopes
            .specialization
            .enum_symbol(&substituted_enum(ty, scopes)),
        ResolvedType::Class(ty) => scopes
            .specialization
            .class_symbols
            .get(ty)
            .cloned()
            .unwrap_or_else(|| php_type_symbol(&ty.name)),
        ResolvedType::Nullable(inner) => php_nullable_type_hint(php_resolved_type(inner, scopes)),
        ResolvedType::Bytes => "__DoriaBytes".to_string(),
        ResolvedType::ByteRegion(_) => "__DoriaByteRegion".to_string(),
        ResolvedType::TypedArray(_)
        | ResolvedType::List(_)
        | ResolvedType::Dictionary(_, _)
        | ResolvedType::Set(_) => "array".to_string(),
        ResolvedType::SortedDictionary(_, _) => php_type_symbol("SortedDictionary"),
        ResolvedType::SortedSet(_) => php_type_symbol("SortedSet"),
        ResolvedType::PriorityQueue(_) => php_type_symbol("PriorityQueue"),
        ResolvedType::Deque(_) => php_type_symbol("Deque"),
        ResolvedType::SharedHandle(_, _) => "__DoriaSharedHandle".to_string(),
    }
}

/// An enum type under the current specialization's substitutions.
fn substituted_enum(ty: &crate::enums::EnumType, scopes: &PhpNameScopes) -> crate::enums::EnumType {
    if ty.arguments.is_empty() || scopes.substitutions.is_empty() {
        return ty.clone();
    }
    match crate::types::substitute_resolved_type(
        &ResolvedType::Enum(ty.clone()),
        &scopes.substitutions,
    ) {
        ResolvedType::Enum(ty) => ty,
        _ => ty.clone(),
    }
}

fn php_type_symbol(name: &str) -> String {
    match name {
        "SortedDictionary" | "SortedSet" | "PriorityQueue" | "Deque" => name.to_string(),
        crate::compiler_known_io::IO_OPERATION => "__DoriaStdIoIoOperation".to_string(),
        crate::compiler_known_io::IO_TARGET => "__DoriaStdIoIoTarget".to_string(),
        crate::compiler_known_io::IO_ERROR_REASON => "__DoriaStdIoIoErrorReason".to_string(),
        crate::compiler_known_io::UTF8_INPUT_SOURCE => "__DoriaStdIoUtf8InputSource".to_string(),
        crate::compiler_known_io::IO_ERROR => "__DoriaStdIoIoError".to_string(),
        crate::compiler_known_io::INVALID_UTF8_ERROR => "__DoriaStdIoInvalidUtf8Error".to_string(),
        _ if name.contains('\\') => {
            format!("__DoriaQualified_{}", hex_bytes(name.as_bytes()))
        }
        // Keep Doria's case-sensitive type identity independent of PHP's
        // reserved words, built-in classes, and case-insensitive type names.
        _ => format!("__DoriaType_{}", hex_bytes(name.as_bytes())),
    }
}

fn php_constant_symbol(name: &str) -> String {
    if name.contains('\\') {
        format!("__DoriaQualified_{}", hex_bytes(name.as_bytes()))
    } else {
        name.to_string()
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
            _ if character.is_ascii_control() => {
                // Keep generated PHP textual without changing the string's
                // bytes. Two hex digits cannot consume following payload digits.
                output.push_str(&format!("\\x{:02x}", character as u32));
            }
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

#[cfg(test)]
mod string_literal_tests {
    use super::{emit_php_string_literal, php_execution};

    #[test]
    fn php_string_literals_roundtrip_controls_and_utf8_without_binary_source() {
        let mut value = (0_u8..=127).map(char::from).collect::<String>();
        value.push_str("\x000123abcdef\0\0Dória👩‍💻\\x00\\0\"$amount");
        let literal = emit_php_string_literal(&value);
        assert!(!literal
            .chars()
            .any(|character| character.is_ascii_control()));
        assert!(literal.contains("\\x000123abcdef"));
        let output = php_execution::Script::new(&format!("echo {literal};"))
            .command(&["-d", "ffi.enable=0"])
            .output()
            .expect("PHP is required for exact string-literal serialization parity");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.stdout, value.as_bytes());
    }
}

#[cfg(test)]
mod type_hint_tests {
    use super::*;

    fn scopes_for_type_hints() -> (PhpNameScopes, TypeRef) {
        let program = crate::lower_source(
            "type-hints.doria",
            "function accepts(function(int): int $call): void {} function main(): void {}",
        )
        .unwrap();
        let callable = program
            .items
            .iter()
            .find_map(|item| match item {
                Item::Function(function) if function.name == "accepts" => {
                    Some(function.params[0].ty.clone())
                }
                _ => None,
            })
            .unwrap();
        let scopes = PhpNameScopes::new(
            PhpScopeSymbols {
                interface_names: HashSet::from(["Readable".to_string()]),
                static_properties: HashSet::new(),
                payload_unit_cases: HashSet::new(),
                payload_top_constants: HashSet::new(),
                payload_class_constants: HashSet::new(),
                payload_enum_expressions: HashSet::new(),
                classes_with_php_constructors: HashSet::new(),
                classes_with_php_destructors: HashSet::new(),
                payload_enums_with_php_destructors: HashSet::new(),
                move_enum_names: crate::ownership::MoveEnums::default(),
            },
            Rc::new(PhpClosurePlan::build(&program, None)),
        );
        (scopes, callable)
    }

    #[test]
    fn grouped_host_hints_preserve_inner_and_outer_nullability_across_transport_families() {
        let (scopes, callable) = scopes_for_type_hints();
        let integer = || TypeRef::named("int");
        let token = || TypeRef::named("Token");
        let cases = [
            (integer(), "int".to_string()),
            (TypeRef::named("float32"), "float".to_string()),
            (TypeRef::named("bool"), "bool".to_string()),
            (TypeRef::named("string"), "string".to_string()),
            (TypeRef::named("mixed"), "mixed".to_string()),
            (TypeRef::named("Bytes"), "__DoriaBytes".to_string()),
            (
                TypeRef::named(crate::types::ByteRegionKind::Readonly.source_name()),
                "__DoriaByteRegion".to_string(),
            ),
            (
                TypeRef::named(crate::types::ByteRegionKind::Writable.source_name()),
                "__DoriaByteRegion".to_string(),
            ),
            (TypeRef::named("Error"), "__DoriaErrorValue".to_string()),
            (token(), php_type_symbol("Token")),
            (
                TypeRef::named("Readable"),
                interface::declaration_name("Readable"),
            ),
            (callable, "__DoriaFunctionValue".to_string()),
            (TypeRef::array_of(integer()), "array".to_string()),
            (
                TypeRef::generic("List", vec![integer()]),
                "array".to_string(),
            ),
            (
                TypeRef::generic("Dictionary", vec![integer(), integer()]),
                "array".to_string(),
            ),
            (
                TypeRef::generic("Set", vec![integer()]),
                "array".to_string(),
            ),
            (
                TypeRef::generic("Set", vec![token()]),
                "__DoriaCoreCollection".to_string(),
            ),
            (
                TypeRef::generic("SortedDictionary", vec![integer(), integer()]),
                "SortedDictionary".to_string(),
            ),
            (
                TypeRef::generic("SortedSet", vec![integer()]),
                "SortedSet".to_string(),
            ),
            (
                TypeRef::generic("PriorityQueue", vec![integer()]),
                "PriorityQueue".to_string(),
            ),
            (
                TypeRef::generic("Deque", vec![integer()]),
                "Deque".to_string(),
            ),
            (
                TypeRef::generic("SharedReference", vec![token()]),
                "__DoriaSharedHandle".to_string(),
            ),
        ];
        let span = Span::new(0, 0);
        for (ty, hint) in cases {
            assert_eq!(
                php_type(&TypeRef::grouped(ty.clone(), span, span), &scopes),
                hint
            );
            let nullable_hint = php_nullable_type_hint(hint);
            for grouped in [
                TypeRef::grouped(ty.clone().nullable(), span, span),
                TypeRef::grouped(ty.clone(), span, span).nullable(),
                TypeRef::grouped(
                    TypeRef::grouped(ty.nullable(), span, span).nullable(),
                    span,
                    span,
                )
                .nullable(),
            ] {
                assert_eq!(php_type(&grouped, &scopes), nullable_hint, "{grouped}");
                let union = if nullable_hint == "mixed" {
                    "mixed".to_string()
                } else {
                    format!("{}|null", nullable_hint.strip_prefix('?').unwrap())
                };
                assert_eq!(php_parameter_value_type(&grouped, false, &scopes), union);
            }
        }
    }

    #[test]
    fn grouped_host_hints_preserve_nullable_substitutions_and_class_specializations() {
        let (mut scopes, _) = scopes_for_type_hints();
        let integer = crate::types::nullable_of(ResolvedType::Integer(IntegerType::Int64));
        scopes
            .substitutions
            .insert("T".to_string(), integer.clone());
        Rc::make_mut(&mut scopes.specialization)
            .class_symbols
            .insert(
                crate::types::ClassType::new("Box", vec![integer]),
                "__DoriaSpecializedBox".to_string(),
            );
        let span = Span::new(0, 0);
        for ty in [
            TypeRef::grouped(TypeRef::named("T"), span, span),
            TypeRef::grouped(TypeRef::named("T").nullable(), span, span).nullable(),
        ] {
            assert_eq!(php_type(&ty, &scopes), "?int");
            assert_eq!(php_parameter_value_type(&ty, false, &scopes), "int|null");
        }
        let ty = TypeRef::generic("Box", vec![TypeRef::named("T")]);
        assert_eq!(
            php_type(&TypeRef::grouped(ty.clone(), span, span), &scopes),
            "__DoriaSpecializedBox"
        );
        assert_eq!(
            php_type(&TypeRef::grouped(ty.nullable(), span, span), &scopes),
            "?__DoriaSpecializedBox"
        );
    }

    #[test]
    fn nullable_host_hints_keep_mixed_standalone() {
        assert_eq!(php_nullable_type_hint("mixed".to_string()), "mixed");
        assert_eq!(php_nullable_type_hint("null".to_string()), "mixed");
        assert_eq!(php_nullable_type_hint("int".to_string()), "?int");
        assert_eq!(
            php_nullable_type_hint("__DoriaBytes".to_string()),
            "?__DoriaBytes"
        );
    }
}
