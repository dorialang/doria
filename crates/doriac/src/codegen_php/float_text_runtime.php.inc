// Compiler-private float text. Canonical display is the shortest decimal that
// identifies the value at its own width, laid out as the native runtime lays
// it out; `%f` is the exact value rounded to the requested fraction digits,
// ties to even. Both work from the value's exact decimal expansion, held as a
// string of digits: a binary float m * 2^e is the integer m * 2^e when e is
// not negative and m * 5^-e over 10^-e otherwise, so no division and no host
// float formatting takes part.

// The decimal digits of $number * 2^$power ($power >= 0) or of
// $number * 5^-$power ($power < 0), without leading zeros. $number is not
// negative and is below 2^56.
function __doria_float_scaled_digits(int $number, int $power): string
{
    // Little-endian limbs in base 10^7; a limb times a factor below 2^24
    // plus a carry stays far inside a PHP integer.
    $limbs = [];
    for ($rest = $number; $rest > 0; $rest = intdiv($rest, 10000000)) {
        $limbs[] = $rest % 10000000;
    }
    [$base, $step, $count] = $power >= 0 ? [2, 23, $power] : [5, 10, -$power];
    while ($count > 0 && $limbs !== []) {
        $take = min($step, $count);
        $factor = $base ** $take;
        $carry = 0;
        foreach ($limbs as $index => $limb) {
            $product = $limb * $factor + $carry;
            $limbs[$index] = $product % 10000000;
            $carry = intdiv($product, 10000000);
        }
        while ($carry > 0) {
            $limbs[] = $carry % 10000000;
            $carry = intdiv($carry, 10000000);
        }
        $count -= $take;
    }
    if ($limbs === []) { return "0"; }
    $digits = (string) array_pop($limbs);
    for ($index = count($limbs) - 1; $index >= 0; $index--) {
        $digits .= str_pad((string) $limbs[$index], 7, "0", STR_PAD_LEFT);
    }
    return $digits;
}

// Whether the digit string $left is below ($result < 0), equal to or above
// the digit string $right as integers. Neither has leading zeros.
function __doria_float_compare_digits(string $left, string $right): int
{
    return strlen($left) <=> strlen($right) ?: strcmp($left, $right);
}

// The sign, the integer significand and the binary exponent of a finite
// value at $bits (32 or 64) of width, and whether its lower neighbor is half
// as far away as its upper one (the value is a power of two).
function __doria_float_parts(float $value, int $bits): array
{
    if ($bits === 32) {
        $word = unpack('N', pack('G', $value))[1];
        $exponent = ($word >> 23) & 0xff;
        $fraction = $word & 0x7fffff;
        return $exponent === 0
            ? [($word >> 31) & 1, $fraction, -149, false]
            : [($word >> 31) & 1, $fraction | 0x800000, $exponent - 150, $fraction === 0 && $exponent > 1];
    }
    $word = unpack('J', pack('E', $value))[1];
    $exponent = ($word >> 52) & 0x7ff;
    $fraction = $word & 0xfffffffffffff;
    return $exponent === 0
        ? [($word >> 63) & 1, $fraction, -1074, false]
        : [($word >> 63) & 1, $fraction | (1 << 52), $exponent - 1075, $fraction === 0 && $exponent > 1];
}

// Canonical display of a float carried at $bits (32 or 64) of width.
function __doria_float_text(float $value, int $bits): string
{
    if (is_nan($value)) { return "NaN"; }
    if ($value === INF) { return "Infinity"; }
    if ($value === -INF) { return "-Infinity"; }
    [$negative, $significand, $exponent, $closer] = __doria_float_parts($value, $bits);
    if ($significand === 0) { return $negative ? "-0" : "0"; }

    // The value and the two ends of the interval that rounds to it, as
    // integers over one shared power of ten. The ends belong to the interval
    // when the significand is even, since a tie rounds to even.
    $power = $exponent - 2;
    $exact = __doria_float_scaled_digits(4 * $significand, $power);
    $low = __doria_float_scaled_digits(4 * $significand - ($closer ? 1 : 2), $power);
    $high = __doria_float_scaled_digits(4 * $significand + 2, $power);
    $inclusive = ($significand & 1) === 0;
    $length = strlen($exact);
    $inside = static function (string $candidate) use ($low, $high, $inclusive): bool {
        $above = __doria_float_compare_digits($candidate, $low);
        $below = __doria_float_compare_digits($candidate, $high);
        return $inclusive ? $above >= 0 && $below <= 0 : $above > 0 && $below < 0;
    };

    // The shortest decimal in the interval is the value cut to some number of
    // leading digits, or that cut plus one in its last place; the first count
    // that puts either inside is the answer, and the nearer one wins.
    $chosen = $exact;
    for ($count = 1; $count < $length; $count++) {
        $head = substr($exact, 0, $count);
        $tail = substr($exact, $count);
        $zeros = str_repeat("0", $length - $count);
        if (trim($tail, "0") === "") { break; }
        $down = $head . $zeros;
        $raised = $head;
        for ($index = $count - 1; $index >= 0 && $raised[$index] === "9"; $index--) {
            $raised[$index] = "0";
        }
        if ($index >= 0) {
            $raised[$index] = (string) ((int) $raised[$index] + 1);
        } else {
            $raised = "1" . $raised;
        }
        $up = $raised . $zeros;
        $downInside = $inside($down);
        $upInside = $inside($up);
        if ($downInside && $upInside) {
            // Both strings have one length, so they compare as numbers.
            $half = strcmp($tail, "5" . str_repeat("0", strlen($tail) - 1));
            $chosen = $half > 0 || ($half === 0 && ((int) $head[$count - 1] & 1) === 1) ? $up : $down;
            break;
        }
        if ($downInside) { $chosen = $down; break; }
        if ($upInside) { $chosen = $up; break; }
    }

    // $chosen * 10^$scale is the value's shortest decimal; $digits are its
    // significant digits and $point is where the decimal point falls in them.
    $scale = $power < 0 ? $power : 0;
    $digits = rtrim($chosen, "0");
    $point = strlen($chosen) + $scale;
    $count = strlen($digits);
    [$plain, $small] = $bits === 32 ? [13, -6] : [16, -5];
    if ($point >= $count && $point <= $plain) {
        $text = $digits . str_repeat("0", $point - $count) . ".0";
    } elseif ($point > 0 && $point <= $plain) {
        $text = substr($digits, 0, $point) . "." . substr($digits, $point);
    } elseif ($point > $small && $point <= 0) {
        $text = "0." . str_repeat("0", -$point) . $digits;
    } elseif ($count === 1) {
        $text = $digits . "e" . ($point - 1);
    } else {
        $text = $digits[0] . "." . substr($digits, 1) . "e" . ($point - 1);
    }
    return $negative ? "-" . $text : $text;
}

// `%f`: the exact value with $precision fraction digits, ties to even, then
// padded to $width. A float32 is carried as the binary64 number equal to it,
// so its exact value needs no width.
function __doria_float_fixed(float $value, int $precision, int $width, bool $left, bool $zero): string
{
    if (is_nan($value)) {
        $text = "NaN";
    } elseif ($value === INF) {
        $text = "Infinity";
    } elseif ($value === -INF) {
        $text = "-Infinity";
    } else {
        [$negative, $significand, $exponent] = __doria_float_parts($value, 64);
        $digits = __doria_float_scaled_digits($significand, $exponent);
        $fraction = $exponent < 0 ? -$exponent : 0;
        if (strlen($digits) <= $fraction) {
            $digits = str_repeat("0", $fraction - strlen($digits) + 1) . $digits;
        }
        $whole = substr($digits, 0, strlen($digits) - $fraction);
        $part = substr($digits, strlen($digits) - $fraction);
        if ($precision >= $fraction) {
            $part .= str_repeat("0", $precision - $fraction);
        } else {
            $dropped = substr($part, $precision);
            $kept = $whole . substr($part, 0, $precision);
            $half = strcmp($dropped, "5" . str_repeat("0", strlen($dropped) - 1));
            if ($half > 0 || ($half === 0 && ((int) $kept[strlen($kept) - 1] & 1) === 1)) {
                for ($index = strlen($kept) - 1; $index >= 0 && $kept[$index] === "9"; $index--) {
                    $kept[$index] = "0";
                }
                if ($index >= 0) {
                    $kept[$index] = (string) ((int) $kept[$index] + 1);
                } else {
                    $kept = "1" . $kept;
                }
            }
            $whole = substr($kept, 0, strlen($kept) - $precision);
            $part = substr($kept, strlen($kept) - $precision);
        }
        $text = ($negative ? "-" : "") . $whole . ($precision > 0 ? "." . $part : "");
    }
    $padding = $width - strlen($text);
    if ($padding <= 0) { return $text; }
    if ($left) { return $text . str_repeat(" ", $padding); }
    if (!$zero) { return str_repeat(" ", $padding) . $text; }
    return $text[0] === "-"
        ? "-" . str_repeat("0", $padding) . substr($text, 1)
        : str_repeat("0", $padding) . $text;
}
