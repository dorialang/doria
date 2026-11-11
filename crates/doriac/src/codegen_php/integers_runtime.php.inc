// Compiler-private integer operations over the existing PHP integer carriers.
// UInt64 uses all 64 bits of a signed PHP int; negative carriers are not negative
// Doria values. No string, float, boxed owner, or host arithmetic wrap is used
// to determine a mathematical result, an overflow or a panic.
//
// A zero divisor and a shift count outside the width are faults of the right
// operand. Their panic points at it, inside the whole operation, when the
// caller passes its span.
function __doria_integer_right_operand_panic(string $code, int $start, int $end,
    int $rightStart, int $rightEnd, string $callable): void
{
    if ($rightStart < 0) { __doria_panic($code, $start, $end, null, $callable); }
    __doria_panic($code, $rightStart, $rightEnd, null, $callable,
        [], __doria_diagnostic_site($start, $end));
}

function __doria_integer_operation(string $op, int $left, int $right, int $bits,
    bool $signed, int $start, int $end, string $callable,
    int $rightStart = -1, int $rightEnd = -1): int
{
    if (($bits !== 8 && $bits !== 16 && $bits !== 32 && $bits !== 64)
        || ($op === "neg" && !$signed)) {
        throw new LogicException("MIR integer operation has an invalid type");
    }

    // A signed carrier is sign-extended and a narrow unsigned one is
    // zero-extended, so the host's bitwise operators already answer in the
    // type's own width.
    if ($op === "&") { return $left & $right; }
    if ($op === "|") { return $left | $right; }
    if ($op === "^") { return $left ^ $right; }
    if ($op === "~") {
        return $signed || $bits === 64 ? ~$left : ~$left & ((1 << $bits) - 1);
    }

    if ($op === "<<" || $op === ">>") {
        // A UInt64 count with its top bit set is far past the width, too.
        if ($right < 0 || $right >= $bits) {
            __doria_integer_right_operand_panic("P1108", $start, $end,
                $rightStart, $rightEnd, $callable);
        }
        if ($op === "<<") {
            // Bits shifted past the width are discarded, never an overflow.
            $value = $left << $right;
            if ($bits === 64) { return $value; }
            $value &= (1 << $bits) - 1;
            return $signed && ($value >> ($bits - 1)) === 1 ? $value - (1 << $bits) : $value;
        }
        if ($signed || $bits !== 64 || $right === 0) { return $left >> $right; }
        // The host shift is arithmetic; UInt64 needs the vacated bits clear.
        return ($left >> $right) & (PHP_INT_MAX >> ($right - 1));
    }

    if ($op === "/" || $op === "%") {
        if ($right === 0) {
            __doria_integer_right_operand_panic($op === "/" ? "P1105" : "P1107",
                $start, $end, $rightStart, $rightEnd, $callable);
        }
        if ($signed || $bits !== 64 || ($left >= 0 && $right > 0)) {
            $minimum = $bits === 64 ? PHP_INT_MIN : -(1 << ($bits - 1));
            if ($signed && $right === -1 && $left === $minimum) {
                // The one quotient outside the type. Its remainder is zero.
                if ($op === "/") { __doria_panic("P1106", $start, $end, null, $callable); }
                return 0;
            }
            return $op === "/" ? intdiv($left, $right) : $left % $right;
        }
        if ($right < 0) {
            // A divisor of 2^63 or more goes into the dividend once or never.
            // Two carriers with the top bit set order as their values do.
            $once = $left < 0 && $left >= $right;
            if ($op === "/") { return $once ? 1 : 0; }
            return $once ? $left - $right : $left;
        }
        // The dividend is 2^63 or more and the divisor is below it. Divide
        // half the dividend, then restore the halved-away bit; every
        // intermediate stays inside a host integer.
        $half = ($left >> 1) & PHP_INT_MAX;
        $quotient = intdiv($half, $right) << 1;
        $remainder = (($half % $right) << 1) | ($left & 1);
        if ($remainder < 0) {
            $remainder = (($remainder & PHP_INT_MAX) - $right) + PHP_INT_MAX + 1;
            $quotient |= 1;
        } elseif ($remainder >= $right) {
            $remainder -= $right;
            $quotient |= 1;
        }
        return $op === "/" ? $quotient : $remainder;
    }

    $code = match ($op) {
        "+" => "P1101", "-" => "P1102", "*" => "P1103", "neg" => "P1104",
        default => throw new LogicException("MIR integer operation is unknown"),
    };

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

// `Target::from(value)` for a conversion that can fail: the value must be one
// the target type holds. The panic points at the value, inside the conversion.
function __doria_integer_convert(int $value, int $sourceBits, bool $sourceSigned,
    int $targetBits, bool $targetSigned, int $start, int $end,
    int $valueStart, int $valueEnd, string $callable): int
{
    $fits = match (true) {
        // A UInt64 carrier with its top bit set is 2^63 or more.
        !$sourceSigned && $sourceBits === 64 && $value < 0 => !$targetSigned && $targetBits === 64,
        $targetBits === 64 => $targetSigned || $value >= 0,
        $targetSigned => $value >= -(1 << ($targetBits - 1)) && $value < (1 << ($targetBits - 1)),
        default => $value >= 0 && $value < (1 << $targetBits),
    };
    if (!$fits) {
        __doria_panic("P1109", $valueStart, $valueEnd, null, $callable,
            [], __doria_diagnostic_site($start, $end));
    }
    return $value;
}
