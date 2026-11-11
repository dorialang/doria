#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::backend::NativeProfile;
use doriac::numeric::IntegerType;
use std::process::Output;

const NARROWED_MIXED_UPDATES: &str = r#"
function main(): void {
    writable mixed $dynamic = 7;
    if ($dynamic is int) { $dynamic += 2; }
    if ($dynamic is int) { $dynamic++; }
    if ($dynamic is int) { echo $dynamic; }
    $dynamic = "text";
    if ($dynamic is string) { echo ";{$dynamic}"; }
}
"#;

fn php(source: &str) -> Output {
    let php = doriac::compile_source_to_php("checked-integers.doria", source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap()
}

fn assert_parity(source: &str, expected: &str) {
    assert_native_parity(source, expected);
    let run = php(source);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(run.stdout, expected.as_bytes(), "{run:?}");
    assert!(run.stderr.is_empty(), "{run:?}");
}

fn assert_native_parity(source: &str, expected: &str) {
    let program = doriac::lower_source_to_mir("checked-integers.doria", source).unwrap();
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.exit_status, 0);
    assert_eq!(interpreted.stdout, expected.as_bytes());
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
}

#[test]
fn checked_integer_loops_reuse_direct_places_without_sequencing_closures() {
    let source = r#"
function hot(): int {
    let writable $sum = 0;
    for (let writable $index = 0; $index < 5; $index++) {
        $sum += $index;
    }
    $sum -= 1;
    $sum *= 2;
    $sum++;
    --$sum;
    return $sum + 0;
}
function main(): void { echo hot(); }
"#;
    assert_parity(source, "18");
    let php = doriac::compile_source_to_php("integer-storage.doria", source).unwrap();
    let start = php.find("function __DoriaFunction_686f74(").unwrap();
    let body = php[start..].split_once("\n}\n").unwrap().0;
    for allocation in [
        "__DoriaCell",
        "__doria_integer_left_",
        "__doria_integer_previous_",
    ] {
        assert!(
            !body.contains(allocation),
            "unexpected integer sequencing/storage {allocation}: {body}"
        );
    }
    assert!(body.contains("__doria_numeric_assign("), "{body}");
    // A `for` lowers to a first-iteration flag so its condition carries an
    // operation site; the update still assigns the loop variable in place.
    assert!(body.contains("__doria_numeric_assign($index, "), "{body}");
    assert_eq!(body.matches("(function()").count(), 0, "{body}");
}

#[test]
fn checked_integer_updates_snapshot_before_rhs_and_project_narrowed_storage() {
    assert_parity(
        r#"
class State { static writable int $value = 10; }
function right(): int { State::value = 100; return 3; }
function main(): void {
    State::value += right();
    echo "{State::value};";
    writable ?int $nullable = 40;
    if ($nullable != null) { $nullable += 1; $nullable++; echo "{$nullable};"; }
    $nullable = null;
    if ($nullable == null) { echo "null;"; }
}
"#,
        "13;42;null;",
    );
}

#[test]
fn checked_integer_mixed_updates_preserve_projected_php_storage() {
    let run = php(NARROWED_MIXED_UPDATES);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(run.stdout, b"10;text", "{run:?}");
    assert!(run.stderr.is_empty(), "{run:?}");
}

#[test]
fn checked_integer_mixed_updates_preserve_projected_storage_across_backends() {
    assert_parity(NARROWED_MIXED_UPDATES, "10;text");
}

#[test]
fn mixed_updates_rebox_every_integer_width_and_nullable_storage() {
    let mut source = String::new();
    let mut main = String::from("function main(): void {");
    let mut expected = String::new();
    for integer in IntegerType::ALL {
        let ty = integer.source_name();
        source.push_str(&format!(
            "function mixed_{ty}({ty} $input): void {{\n\
             writable mixed $value = $input;\n\
             if ($value is {ty}) {{ $value += 2; }}\n\
             if ($value is {ty}) {{ $value++; }}\n\
             if ($value is {ty}) {{ --$value; }}\n\
             if ($value is {ty}) {{ $value *= 3; }}\n\
             if ($value is {ty}) {{ $value -= 1; }}\n\
             if ($value is {ty}) {{ echo $value . \";\"; }}\n\
             writable ?mixed $optional = $input;\n\
             if ($optional is {ty}) {{ $optional += 1; }}\n\
             if ($optional is {ty}) {{ $optional++; }}\n\
             if ($optional is {ty}) {{ echo $optional . \";\"; }}\n\
             $optional = null; if ($optional == null) {{ echo \"null;\"; }} }}\n"
        ));
        main.push_str(&format!("mixed_{ty}(7);"));
        expected.push_str("26;9;null;");
    }
    source.push_str(
        r#"
function crossing(uint64 $input): void {
    writable mixed $value = $input;
    if ($value is uint64) { $value++; }
    if ($value is uint64) { echo $value; }
}
"#,
    );
    main.push_str("crossing(9223372036854775807);}");
    source.push_str(&main);
    expected.push_str("9223372036854775808");
    assert_parity(&source, &expected);
}

#[test]
fn mixed_updates_preserve_writable_parameters_and_closure_homes() {
    assert_parity(
        r#"
function update(writable mixed $value): void {
    if ($value is int) { $value += 2; }
}
function optional(writable ?mixed $value): void {
    if ($value is int) { $value++; }
}
function main(): void {
    writable mixed $value = 7;
    update($value);
    let writable $run = function(): void with (writable $value) {
        if ($value is int) { $value *= 3; }
    };
    $run();
    let $change = function(writable mixed $target): void {
        if ($target is int) { --$target; }
    };
    $change($value);
    if ($value is int) { echo $value; }
    $value = "replaced";
    if ($value is string) { echo ";{$value};"; }
    writable ?mixed $nullable = 10;
    optional($nullable);
    if ($nullable is int) { echo $nullable; }
    $nullable = null;
}
"#,
        "26;replaced;11",
    );
}

#[test]
fn mixed_and_float_updates_commit_only_after_successful_rhs() {
    assert_parity(
        r#"
class Failure implements Error { string $message = "stop"; }
function failed(): int throws Failure { throw new Failure(); }
function failedFloat(): float64 throws Failure { throw new Failure(); }
function main(): void {
    writable mixed $integer = 10;
    if ($integer is int) { try { $integer += failed(); } catch (Failure) {} }
    if ($integer is int) { echo $integer . ";"; }
    writable mixed $number = 1.25;
    if ($number is float64) { try { $number += failedFloat(); } catch (Failure) {} }
    if ($number is float64) { echo $number == 1.25; }
}
"#,
        "10;true",
    );
}

#[test]
fn float64_updates_snapshot_and_preserve_boxed_nullable_and_indexed_places() {
    assert_parity(
        r#"
class State { static writable float64 $value = 1.25; }
function right(): float64 { State::value = 100.0; return 2.5; }
function main(): void {
    State::value += right();
    echo State::value == 3.75;
    writable mixed $value = 1.25;
    if ($value is float64) { $value += 2.5; }
    if ($value is float64) { $value *= 2.0; }
    if ($value is float64) { $value -= 0.5; }
    if ($value is float64) { $value /= 2.0; }
    if ($value is float64) { echo $value == 3.5; }
    writable ?mixed $boxed = 1.25;
    if ($boxed is float64) { $boxed += 2.5; }
    if ($boxed is float64) { echo $boxed == 3.75; }
    $boxed = null;
    writable ?float64 $optional = 1.25;
    if ($optional != null) { $optional += 2.5; echo $optional == 3.75; }
    writable List<float64> $values = [1.25];
    $values[0] += 2.5; echo $values[0] == 3.75;
}
"#,
        "truetruetruetruetrue",
    );
}

#[test]
fn native_mixed_float_widths_and_increments_keep_php_precision_boundaries() {
    for ty in ["float32", "float64"] {
        let source = format!(
            r#"
function update({ty} $input): void {{
    writable mixed $value = $input;
    if ($value is {ty}) {{ $value += 2.5; }}
    if ($value is {ty}) {{ $value++; }}
    if ($value is {ty}) {{ --$value; }}
    if ($value is {ty}) {{ echo $value == 3.75; }}
}}
function main(): void {{ update(1.25); }}
"#
        );
        assert_native_parity(&source, "true");
        let errors = doriac::compile_source_to_php("float-boundary.doria", &source).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code == "B1301"),
            "{errors:?}"
        );
    }
}

#[test]
fn numeric_update_temporary_owners_drop_rhs_before_target() {
    assert_parity(
        r#"
class Operand {
    writable int $integer = 10;
    writable float64 $number = 1.25;
    function __construct(int $id) {}
    function __destruct() { try { echo "drop {$this->id};"; } catch (Doria\Std\Io\IoError) {} }
}
function create(int $id): Operand throws Doria\Std\Io\IoError {
    echo "make {$id};"; return new Operand($id);
}
function main(): void {
    writable List<int> $integers = [10];
    $integers[create(1)->id - 1] += create(2)->integer;
    writable List<float64> $numbers = [1.25];
    $numbers[create(3)->id - 3] += create(4)->number;
    echo (create(5)->number + create(6)->number) == 2.5;
}
"#,
        "make 1;make 2;drop 2;drop 1;make 3;make 4;drop 4;drop 3;make 5;make 6;truedrop 6;drop 5;",
    );
}

#[test]
fn checked_arithmetic_preserves_every_integer_width_and_unsigned_carrier() {
    let mut source = String::new();
    let mut main = String::from("function main(): void {");
    let mut expected = String::new();
    for integer in IntegerType::ALL {
        let ty = integer.source_name();
        source.push_str(&format!(
            "function compute_{ty}({ty} $left, {ty} $right): {ty} {{\n\
             writable {ty} $value = $left + $right; $value -= $right;\n\
             $value *= $right; $value++; --$value; return $value; }}\n"
        ));
        main.push_str(&format!("echo compute_{ty}(7, 3) . \";\";"));
        expected.push_str("21;");
    }
    source.push_str(
        r#"
function unsignedCrossing(uint64 $value): uint64 { return $value + 1; }
function unsignedDifference(uint64 $value): uint64 { return $value - 1; }
function unsignedProduct(uint64 $value): uint64 { return $value * 3; }
function negate(int $value): int { return -$value; }
"#,
    );
    main.push_str(
        r#"
echo unsignedCrossing(9223372036854775807) . ";";
echo unsignedDifference(18446744073709551615) . ";";
echo unsignedProduct(6148914691236517205) . ";";
echo negate(-9223372036854775807);
}"#,
    );
    source.push_str(&main);
    expected.push_str(
        "9223372036854775808;18446744073709551614;18446744073709551615;9223372036854775807",
    );
    assert_parity(&source, &expected);
}

#[test]
fn checked_integer_updates_preserve_places_hooks_and_evaluation_order() {
    assert_parity(r#"
class Counter {
    internal writable int $stored = 10;
    writable int $getterCalls = 0;
    writable int $setterCalls = 0;
    writable int $count = 10 {
        writable get { $this->getterCalls++; return $this->count; }
        set (int $value) { $this->setterCalls++; $this->count = $value; }
    }
    writable function update(): void throws Doria\Std\Io\IoError {
        $this->stored += right(); $this->stored++; --$this->stored;
        echo "stored {$this->stored};";
    }
}
function position(): int throws Doria\Std\Io\IoError { echo "I;"; return 0; }
function right(): int throws Doria\Std\Io\IoError { echo "R;"; return 3; }
function change(writable int $value): void throws Doria\Std\Io\IoError {
    $value += right(); echo "copy {$value};";
}
function main(): void {
    let writable $counter = new Counter();
    $counter->update();
    $counter->count += right(); $counter->count++; --$counter->count;
    echo "count {$counter->count};";
    echo "hooks {$counter->getterCalls} {$counter->setterCalls};";
    writable int $local = 10; change($local); echo "local {$local};";
    writable int[] $array = [10]; $array[position()] += right();
    echo "array {$array[0]};";
    writable List<int> $list = [10]; $list[position()] *= right();
    echo "list {$list[0]};";
    writable List<List<int>> $nested = [[10]]; $nested[0][position()] += right();
    echo "nested {$nested[0][0]};";
    writable Dictionary<string, int> $dictionary = ["key" => 10];
    $dictionary["key"] -= right(); $dictionary["key"]++;
    echo "dictionary {$dictionary["key"]};";
    writable Bytes $bytes = Bytes::allocate(1); $bytes[0] = 10;
    $bytes[position()] += 3; $bytes[0]++; --$bytes[0];
    echo "byte {$bytes[0]};";
}
"#, "R;stored 13;R;count 13;hooks 4 3;R;copy 13;local 10;I;R;array 13;I;R;list 30;I;R;nested 13;R;dictionary 8;I;byte 13;");
}

#[test]
fn checked_integer_overflow_uses_catalogue_codes_and_aborts_without_cleanup() {
    for integer in IntegerType::ALL {
        let ty = integer.source_name();
        let max = integer.max_value();
        let min = integer.min_value();
        let mut operations = vec![
            ("$value += 1;".to_string(), max, "P1101"),
            ("$value++;".to_string(), max, "P1101"),
            ("$value -= 1;".to_string(), min, "P1102"),
            ("--$value;".to_string(), min, "P1102"),
            ("$value *= 2;".to_string(), max, "P1103"),
        ];
        if integer.is_signed() {
            operations.push(("let $negated = -$value;".to_string(), min, "P1104"));
        }
        for (operation, initial, code) in operations {
            let source = format!(
                r#"
class Guard {{ function __destruct() {{ try {{ echo "drop;"; }} catch (Doria\Std\Io\IoError) {{}} }} }}
function overflow({ty} $input): void {{
    let $guard = new Guard(); writable {ty} $value = $input;
    {operation}
}}
function main(): void {{ overflow({initial}); }}
"#
            );
            let run = php(&source);
            assert_eq!(run.status.code(), Some(101), "{ty} {operation}: {run:?}");
            assert!(run.stdout.is_empty(), "{ty} {operation}: {run:?}");
            let stderr = String::from_utf8(run.stderr).unwrap();
            assert!(
                stderr.contains(&format!("Panic[{code}]")),
                "{ty} {operation}: {stderr}"
            );
            assert!(
                stderr.contains("checked-integers.doria") && stderr.contains("overflow"),
                "{stderr}"
            );
            assert!(
                !stderr.contains("PHP Fatal") && !stderr.contains("Stack trace"),
                "{stderr}"
            );
        }
    }
}

#[test]
fn integer_index_bounds_are_checked_before_the_right_operand() {
    let run = php(r#"
function position(): int throws Doria\Std\Io\IoError { echo "index;"; return 1; }
function right(): int throws Doria\Std\Io\IoError { echo "right;"; return 1; }
function main(): void { writable List<int> $list = [1]; $list[position()] += right(); }
"#);
    assert_eq!(run.status.code(), Some(101), "{run:?}");
    assert_eq!(run.stdout, b"index;");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("Panic[P1310]"),
        "{run:?}"
    );
}

#[test]
fn checked_integer_operands_run_once_in_order_and_release_temporary_owners() {
    assert_parity(
        r#"
class Operand {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop {$this->value};"; } catch (Doria\Std\Io\IoError) {} }
}
function create(int $value): Operand throws Doria\Std\Io\IoError {
    echo "make {$value};"; return new Operand($value);
}
function main(): void { echo (create(20)->value + create(22)->value) . ";"; }
"#,
        "make 20;make 22;42;drop 22;drop 20;",
    );
}

#[test]
fn checked_integer_updates_do_not_write_if_the_right_operand_throws() {
    assert_parity(
        r#"
class Failure implements Error { string $message = "stop"; }
class Counter {
    writable int $setterCalls = 0;
    writable int $count = 10 {
        get => $this->count;
        set (int $value) { $this->setterCalls++; $this->count = $value; }
    }
}

function failed(): int throws Failure { throw new Failure(); }
function main(): void {
    let writable $counter = new Counter();
    writable List<int> $values = [10];
    try { $counter->count += failed(); } catch (Failure) {}
    try { $values[0] += failed(); } catch (Failure) {}
    echo "{$counter->count} {$counter->setterCalls} {$values[0]}";
}
"#,
        "10 0 10",
    );
}

#[test]
fn fixed_width_constants_and_defaults_keep_their_checked_source_types() {
    assert_parity(
        r#"
const int8 MINIMUM = -128;
class Limits { const uint32 MAXIMUM = 4294967295; }
function increment(int8 $value = MINIMUM): int8 { return $value + 1; }
function main(): void { echo increment() . ";" . Limits::MAXIMUM; }
"#,
        "-127;4294967295",
    );
}

#[test]
fn writable_collection_and_region_updates_preserve_generic_and_erased_parameter_abi() {
    assert_parity(
        r#"
use Doria\Std\Io\{ReadableStream, WritableByteRegion, ReadOutcome};
interface Updater { writable function update(writable List<int> $values): void; }
class Change implements Updater {
    writable function update(writable List<int> $values): void { $values[0] += 3; }
}
class Reader implements ReadableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome {
        $destination[0] += 3;
        return ReadOutcome::Data(1);
    }
}
function changeGeneric<T implements Updater>(writable T $change, writable List<int> $values): void {
    $change->update($values);
}
function changeErased(writable Updater $change, writable List<int> $values): void {
    $change->update($values);
}
function readGeneric<T implements ReadableStream>(writable T $reader, writable WritableByteRegion $destination): void {
    $reader->read($destination);
}
function readErased(writable ReadableStream $reader, writable WritableByteRegion $destination): void {
    $reader->read($destination);
}
function main(): void {
    let writable $change = new Change();
    writable List<int> $values = [10];
    $change->update($values); changeGeneric($change, $values); changeErased($change, $values);
    echo "list {$values[0]};";
    let writable $reader = new Reader();
    let writable $bytes = Bytes::allocate(1); $bytes[0] = 10;
    {
        let writable $destination = $bytes->getWritableRegion(0, 1);
        $reader->read($destination); readGeneric($reader, $destination); readErased($reader, $destination);
    }
    echo "byte {$bytes[0]};";
}
"#,
        "list 19;byte 19;",
    );
}

/// The interpreter's answer is the reference; the native profiles and PHP
/// must print the same.
fn assert_backends_agree(source: &str) -> String {
    let program = doriac::lower_source_to_mir("checked-integers.doria", source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.exit_status, 0, "{source}");
    let expected = String::from_utf8(interpreted.stdout).unwrap();
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, &expected);
    }
    let run = php(source);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{source}");
    assert!(run.stderr.is_empty(), "{run:?}");
    expected
}

#[test]
fn division_remainder_shifts_and_bitwise_operators_preserve_every_integer_width() {
    let mut source = String::new();
    let mut main = String::from("function main(): void {\n");
    for integer in IntegerType::ALL {
        let ty = integer.source_name();
        let bits = integer.bit_width();
        let min = integer.min_value();
        let max = integer.max_value();
        source.push_str(&format!(
            "function divide_{ty}({ty} $left, {ty} $right): void {{\n\
             writable {ty} $quotient = $left; $quotient /= $right;\n\
             writable {ty} $remainder = $left; $remainder %= $right;\n\
             echo ($left / $right) . \",\" . ($left % $right) . \",\" . $quotient . \",\" . $remainder . \";\";\n\
             }}\n\
             function shift_{ty}({ty} $value, {ty} $count): void {{\n\
             writable {ty} $left = $value; $left <<= $count;\n\
             writable {ty} $right = $value; $right >>= $count;\n\
             echo ($value << $count) . \",\" . ($value >> $count) . \",\" . $left . \",\" . $right . \";\";\n\
             }}\n\
             function mask_{ty}({ty} $left, {ty} $right): void {{\n\
             writable {ty} $both = $left; $both &= $right;\n\
             writable {ty} $either = $left; $either |= $right;\n\
             writable {ty} $one = $left; $one ^= $right;\n\
             echo ($left & $right) . \",\" . ($left | $right) . \",\" . ($left ^ $right) . \",\" . (~$left)\n\
             . \",\" . $both . \",\" . $either . \",\" . $one . \";\";\n\
             }}\n"
        ));
        let half = max / 2 + 1;
        let mut values = vec![min, max, 0, 1, 7, half, max - 1];
        if integer.is_signed() {
            values.extend([-1, -7, min + 1]);
        }
        for &left in &values {
            for &right in &values {
                main.push_str(&format!("mask_{ty}({left}, {right});\n"));
                if right != 0 && !(integer.is_signed() && left == min && right == -1) {
                    main.push_str(&format!("divide_{ty}({left}, {right});\n"));
                }
            }
            for count in [0, 1, bits / 2, bits - 1] {
                main.push_str(&format!("shift_{ty}({left}, {count});\n"));
            }
        }
    }
    // The one remainder whose quotient does not exist is zero.
    for integer in IntegerType::ALL.into_iter().filter(|ty| ty.is_signed()) {
        let ty = integer.source_name();
        source.push_str(&format!(
            "function least_{ty}({ty} $left, {ty} $right): {ty} {{ return $left % $right; }}\n"
        ));
        main.push_str(&format!(
            "echo least_{ty}({}, -1) . \";\";\n",
            integer.min_value()
        ));
    }
    main.push_str("}\n");
    let program = source + &main;
    let printed = assert_backends_agree(&program);
    assert!(printed.ends_with("0;0;0;0;"), "{printed}");
}

#[test]
fn division_and_shift_faults_use_catalogue_codes_in_php_output() {
    for integer in IntegerType::ALL {
        let ty = integer.source_name();
        let bits = integer.bit_width();
        let min = integer.min_value();
        let mut operations = vec![
            ("let $result = $value / $other;".to_string(), 7, 0, "P1105"),
            ("$value /= $other;".to_string(), 7, 0, "P1105"),
            ("let $result = $value % $other;".to_string(), 7, 0, "P1107"),
            ("$value %= $other;".to_string(), 7, 0, "P1107"),
            (
                "let $result = $value << $other;".to_string(),
                1,
                i128::from(bits),
                "P1108",
            ),
            (
                "let $result = $value >> $other;".to_string(),
                1,
                integer.max_value(),
                "P1108",
            ),
        ];
        if integer.is_signed() {
            operations.push((
                "let $result = $value / $other;".to_string(),
                min,
                -1,
                "P1106",
            ));
            operations.push((
                "let $result = $value << $other;".to_string(),
                1,
                -1,
                "P1108",
            ));
        }
        for (operation, initial, other, code) in operations {
            let source = format!(
                r#"
function fault({ty} $input, {ty} $other): void {{
    writable {ty} $value = $input;
    {operation}
}}
function main(): void {{ fault({initial}, {other}); }}
"#
            );
            let program = doriac::lower_source_to_mir("checked-integers.doria", &source)
                .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
            let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
            assert_eq!(interpreted.exit_status, 101, "{ty} {operation}");
            let reference = String::from_utf8(interpreted.stderr).unwrap();
            assert!(reference.contains(&format!("Panic[{code}]")), "{reference}");
            let run = php(&source);
            assert_eq!(run.status.code(), Some(101), "{ty} {operation}: {run:?}");
            assert!(run.stdout.is_empty(), "{ty} {operation}: {run:?}");
            // The same report, down to the operand the panic points at.
            assert_eq!(
                String::from_utf8(run.stderr).unwrap(),
                reference,
                "{ty} {operation}"
            );
        }
    }
}

#[test]
fn checked_conversions_hold_every_pair_of_integer_widths() {
    let mut source = String::new();
    let mut main = String::from("function main(): void {\n");
    let mut faults = Vec::new();
    for from in IntegerType::ALL {
        for to in IntegerType::ALL {
            let (source_name, target_name) = (from.source_name(), to.source_name());
            let companion = to.companion_name();
            source.push_str(&format!(
                "function {source_name}_to_{target_name}({source_name} $value): {target_name} {{ return {companion}::from($value); }}\n"
            ));
            for value in [from.min_value(), from.max_value(), 0, 1, 127, 128, 255, 256] {
                if value < from.min_value() || value > from.max_value() {
                    continue;
                }
                if value < to.min_value() || value > to.max_value() {
                    faults.push((source_name, target_name, companion, value));
                } else {
                    main.push_str(&format!(
                        "echo {source_name}_to_{target_name}({value}) . \";\";\n"
                    ));
                }
            }
        }
    }
    main.push_str("}\n");
    assert_backends_agree(&(source + &main));

    // One value outside each target, taken from each source that can hold it.
    let mut seen = std::collections::HashSet::new();
    for (source_name, target_name, companion, value) in faults {
        if !seen.insert((source_name, target_name)) {
            continue;
        }
        let program = format!(
            "function convert({source_name} $value): {target_name} {{ return {companion}::from($value); }}\n\
             function main(): void {{ let $converted = convert({value}); }}\n"
        );
        let mir = doriac::lower_source_to_mir("checked-integers.doria", &program)
            .unwrap_or_else(|errors| panic!("{program}\n{errors:#?}"));
        let interpreted = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(interpreted.exit_status, 101, "{program}");
        let reference = String::from_utf8(interpreted.stderr).unwrap();
        assert!(reference.contains("Panic[P1109]"), "{reference}");
        let run = php(&program);
        assert_eq!(run.status.code(), Some(101), "{program}: {run:?}");
        assert_eq!(
            String::from_utf8(run.stderr).unwrap(),
            reference,
            "{program}"
        );
    }
}
