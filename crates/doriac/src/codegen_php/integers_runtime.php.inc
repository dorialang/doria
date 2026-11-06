// Compiler-private checked arithmetic over the existing PHP integer carriers.
// UInt64 uses all 64 bits of a signed PHP int; negative carriers are not negative
// Doria values. No string, float, boxed owner, or host arithmetic wrap is used
// to determine its mathematical result or overflow.
function __doria_integer_operation(string $op, int $left, int $right, int $bits,
    bool $signed, int $start, int $end, string $callable): int
{
    $code = match ($op) {
        "+" => "P1101", "-" => "P1102", "*" => "P1103", "neg" => "P1104",
        default => throw new LogicException("MIR checked integer operation is unknown"),
    };
    if (($bits !== 8 && $bits !== 16 && $bits !== 32 && $bits !== 64)
        || ($op === "neg" && !$signed)) {
        throw new LogicException("MIR checked integer operation has an invalid type");
    }

    if ($signed || $bits !== 64) {
        $minimum = $signed ? ($bits === 64 ? PHP_INT_MIN : -(1 << ($bits - 1))) : 0;
        $maximum = $bits === 64 ? PHP_INT_MAX : (1 << ($signed ? $bits - 1 : $bits)) - 1;
        $value = match ($op) {
            "+" => $left + $right, "-" => $left - $right,
            "*" => $left * $right, "neg" => -$left,
        };
        // PHP promotes host signed overflow to float. Reject that result before
        // comparison or return can round it or coerce it back into an integer.
        if (!is_int($value) || $value < $minimum || $value > $maximum) {
            __doria_panic($code, $start, $end, null, $callable);
        }
        return $value;
    }

    if ($op === "+" || $op === "-") {
        // Two unsigned 32-bit limbs. The largest intermediate is below 2^33,
        // so every sum, difference and carry/borrow remains a host integer.
        $leftLow = $left & 0xffffffff;
        $rightLow = $right & 0xffffffff;
        $leftHigh = ($left >> 32) & 0xffffffff;
        $rightHigh = ($right >> 32) & 0xffffffff;
        if ($op === "+") {
            $low = $leftLow + $rightLow;
            $high = $leftHigh + $rightHigh + ($low >> 32);
            if ($high > 0xffffffff) {
                __doria_panic($code, $start, $end, null, $callable);
            }
        } else {
            $low = $leftLow - $rightLow;
            $high = $leftHigh - $rightHigh - ($low < 0 ? 1 : 0);
            if ($high < 0) {
                __doria_panic($code, $start, $end, null, $callable);
            }
        }
        // Shifts and OR assemble the carrier bits, not wrapping arithmetic.
        return ($high << 32) | ($low & 0xffffffff);
    }

    // Four unsigned 16-bit limbs per operand. A product column has at most
    // four 16x16 products plus carry (< 2^35), never a floating intermediate.
    $a0 = $left & 0xffff;
    $a1 = ($left >> 16) & 0xffff;
    $a2 = ($left >> 32) & 0xffff;
    $a3 = ($left >> 48) & 0xffff;
    $b0 = $right & 0xffff;
    $b1 = ($right >> 16) & 0xffff;
    $b2 = ($right >> 32) & 0xffff;
    $b3 = ($right >> 48) & 0xffff;

    $column = $a0 * $b0;
    $word0 = $column & 0xffff;
    $column = ($column >> 16) + $a0 * $b1 + $a1 * $b0;
    $word1 = $column & 0xffff;
    $column = ($column >> 16) + $a0 * $b2 + $a1 * $b1 + $a2 * $b0;
    $word2 = $column & 0xffff;
    $column = ($column >> 16) + $a0 * $b3 + $a1 * $b2 + $a2 * $b1 + $a3 * $b0;

    // All omitted high columns are nonnegative. Any high product or final
    // carry proves overflow; there can be no cancellation between columns.
    if (($column >> 16) !== 0
        || ($a1 * $b3 + $a2 * $b2 + $a3 * $b1) !== 0
        || ($a2 * $b3 + $a3 * $b2) !== 0
        || $a3 * $b3 !== 0) {
        __doria_panic($code, $start, $end, null, $callable);
    }
    return $word0 | ($word1 << 16) | ($word2 << 32) | (($column & 0xffff) << 48);
}
