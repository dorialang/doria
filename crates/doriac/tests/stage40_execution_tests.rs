//! Stage 40 execution (Decision 0148): the unsafe and foreign forms each
//! execution target builds, run through the interpreter and every native
//! profile this compiler build has.

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution_runner.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::backend::NativeProfile;
use doriac::mir_interpreter::interpret;
use native_execution::execute_native;

fn program(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("stage40.doria", source.to_string())
        .unwrap_or_else(|diagnostics| panic!("the program should build: {diagnostics:#?}"))
}

/// The program prints `stdout` and exits 0 in the interpreter and on every
/// native profile.
fn assert_runs_everywhere(source: &str, stdout: &str) {
    let mir = program(source);
    let interpreted = interpret(&mir).expect("the interpreter should run the program");
    assert_eq!(
        (
            String::from_utf8_lossy(&interpreted.stdout).into_owned(),
            interpreted.exit_status
        ),
        (stdout.to_string(), 0),
        "interpreter: {:?}",
        interpreted.runtime_diagnostic
    );
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(0), "{profile:?}: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "{profile:?}");
        assert!(run.stderr.is_empty(), "{profile:?}: {run:?}");
    }
}

/// The program panics with `code` after printing `stdout`, in the
/// interpreter and on every native profile.
fn assert_panics_everywhere(source: &str, stdout: &str, code: &str) {
    let mir = program(source);
    let interpreted = interpret(&mir).expect("the interpreter should run the program");
    assert_eq!(String::from_utf8_lossy(&interpreted.stdout), stdout);
    assert_eq!(interpreted.exit_status, 101);
    assert_eq!(
        interpreted
            .runtime_diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.code),
        Some(code)
    );
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(101), "{profile:?}: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "{profile:?}");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(stderr.contains(code), "{profile:?}: {stderr}");
    }
}

const BOUNDARY: &str =
    "use Doria\\Std\\Ffi\\{CChar, CLong, CPointerDifference, CSize, CUnsignedLong, CWideChar};\n";

#[test]
fn boundary_values_convert_compare_and_convert_back() {
    let source = format!(
        "{BOUNDARY}
function widen(CSize $size): int64
{{
    return Int64::from($size);
}}

function main(): void
{{
    let $size = CSize::from(42);
    echo widen($size);
    echo \" \";
    echo $size == CSize::from(42);
    echo \" \";
    echo $size != CSize::from(41);
    echo \" \";
    let $long = CLong::from(-7);
    echo Int64::from($long);
    echo \" \";
    let $difference = CPointerDifference::from(-9);
    echo Int64::from($difference);
    echo \" \";
    let $character = CChar::from(65);
    echo UInt8::from($character);
    echo \" \";
    let $wide = CWideChar::from(9731);
    echo UInt32::from($wide);
    echo \" \";
    let $unsigned = CUnsignedLong::from(UInt32::from(4000000000));
    echo UInt64::from($unsigned);
}}
"
    );
    assert_runs_everywhere(&source, "42 true true -7 -9 65 9731 4000000000");
}

#[test]
fn try_from_gives_null_outside_the_range_on_this_target() {
    let source = format!(
        "{BOUNDARY}
function main(): void
{{
    ?CSize $negative = CSize::tryFrom(-1);
    echo $negative == null;
    echo \" \";
    ?CSize $present = CSize::tryFrom(3);
    echo ($present ?? CSize::from(0)) == CSize::from(3);
    echo \" \";
    ?CChar $character = CChar::tryFrom(300);
    echo $character == null;
    echo \" \";
    ?CLong $long = CLong::tryFrom(-5);
    echo Int64::from($long ?? CLong::from(0));
}}
"
    );
    assert_runs_everywhere(&source, "true true true -5");
}

#[test]
fn a_value_outside_the_boundary_range_panics_with_p1701() {
    let source = format!(
        "{BOUNDARY}
function main(): void
{{
    echo \"before \";
    let $size = CSize::from(-1);
    echo $size == CSize::from(0);
}}
"
    );
    assert_panics_everywhere(&source, "before ", "P1701");

    // The fixed-width companion's conversion from a boundary value stays
    // P1109, as for any other integer.
    let narrowing = format!(
        "{BOUNDARY}
function main(): void
{{
    let $size = CSize::from(300);
    echo UInt8::from($size);
}}
"
    );
    assert_panics_everywhere(&narrowing, "", "P1109");
}

#[test]
fn php_output_reports_that_it_does_not_build_boundary_values_yet() {
    let source = format!("{BOUNDARY}function main(): void {{ let $size = CSize::from(1); }}");
    let diagnostics =
        doriac::compile_source("stage40.doria", source, doriac::backend::BackendTarget::Php)
            .expect_err("PHP output does not build boundary values yet");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "C Boundary Values Cannot Be Built For PHP Output Yet"),
        "{diagnostics:#?}"
    );
}

/// The program prints `stdout` and exits 0 on every native profile; the
/// interpreter reports that it runs pointer operations through the native
/// companion library.
fn assert_runs_natively(source: &str, stdout: &str) {
    let mir = program(source);
    assert_interpreter_reports_companion(&mir);
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(0), "{profile:?}: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), stdout, "{profile:?}");
        assert!(run.stderr.is_empty(), "{profile:?}: {run:?}");
    }
}

/// The program panics with `code` and `title` on every native profile.
fn assert_panics_natively(source: &str, code: &str, title: &str) {
    let mir = program(source);
    assert_interpreter_reports_companion(&mir);
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(101), "{profile:?}: {run:?}");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(
            stderr.contains(code) && stderr.contains(title),
            "{profile:?}: {stderr}"
        );
    }
}

fn assert_interpreter_reports_companion(mir: &doriac::mir::Program) {
    let companion = doriac::unsafe_code::companion_operations(mir);
    assert!(
        companion.iter().all(|diagnostic| diagnostic.code == "E0784"
            && [
                "Pointer Operations Cannot Run In The Interpreter Yet",
                "Foreign Calls Cannot Run In The Interpreter Yet",
                "Foreign Structs And Unions Cannot Run In The Interpreter Yet",
            ]
            .contains(&diagnostic.title.as_str())),
        "{companion:#?}"
    );
    assert!(!companion.is_empty());
    let error = interpret(mir).expect_err("the interpreter runs unsafe code through the companion");
    assert!(error.to_string().contains(&companion[0].message), "{error}");
}

const POINTER: &str = "use Doria\\Std\\Ffi\\{Pointer, WritablePointer};\n";

#[test]
fn pointer_addresses_offset_cast_and_compare() {
    let source = format!(
        "{POINTER}
function step(Pointer<int32> $pointer, int $count): Pointer<int32>
{{
    return unsafe($pointer->offset($count));
}}

function main(): void
{{
    unsafe {{
        ?Pointer<int32> $absent = Pointer::fromAddress(0);
        echo $absent == null;
        echo \" \";
        ?Pointer<int32> $maybe = Pointer::fromAddress(4096);
        if ($maybe != null) {{
            let $base = $maybe;
            echo $base->address;
            echo \" \";
            echo step($base, 3)->address;
            echo \" \";
            echo $base->offset(-2)->address;
            echo \" \";
            echo $base->byteOffset(6)->address;
            echo \" \";
            Pointer<uint8> $bytes = Pointer::cast($base);
            echo $bytes->offset(5)->address;
            echo \" \";
            WritablePointer<int64> $wide = WritablePointer::cast($base);
            echo $wide->offset(1)->address;
            echo \" \";
            Pointer<int64> $readonly = Pointer::from($wide);
            echo $readonly->address == $base->address;
            echo \" \";
            echo $base == step($base, 0);
            echo \" \";
            echo $base != step($base, 1);
        }}
    }}
}}
"
    );
    assert_runs_natively(&source, "true 4096 4108 4088 4102 4101 4104 true true true");
}

/// `?->` on `?Pointer<T>` and `?WritablePointer<T>` runs the member only
/// when the pointer is not null, and `is Pointer<T>` holds for a pointer of
/// exactly that type that is not null (Decision 0148 section 3).
#[test]
fn nullable_pointers_reach_members_null_safe_and_narrow_with_is() {
    let source = format!(
        "{POINTER}
extern \"C\" struct Pair
{{
    int32 $first;
    int32 $second;
}}

class Holder
{{
    writable ?Pointer<int32> $maybe = null;
}}

function find(bool $present, Pointer<int32> $pointer): ?Pointer<int32>
{{
    if ($present) {{
        return $pointer;
    }}
    return null;
}}

function main(): void
{{
    unsafe {{
        writable int32 $value = 7;
        writable Pair $pair = new Pair(first: 1, second: 2);
        ?Pointer<int32> $maybe = Pointer::to($value);
        ?Pointer<int32> $absent = Pointer::fromAddress(0);
        ?WritablePointer<int32> $target = WritablePointer::to($value);
        ?WritablePointer<int32> $nowhere = null;
        ?Pointer<Pair> $pairs = Pointer::to($pair);
        echo ($maybe?->read() ?? -1) . \" \" . ($absent?->read() ?? -1) . \" \";
        echo (($maybe?->address ?? 0) != 0) . \" \" . ($absent?->address ?? 0) . \" \";
        $target?->write(9);
        $nowhere?->write(10);
        $nowhere?->writeUnaligned(11);
        echo $value . \" \";
        echo ($maybe?->offset(0)?->readUnaligned() ?? -2) . \" \";
        echo ($absent?->byteOffset(4)?->read() ?? -2) . \" \";
        ?Pair $copy = $pairs?->read();
        echo ($copy?->second ?? -3) . \"\\n\";
        echo (find(true, Pointer::to($value))?->read() ?? -4) . \" \";
        echo (find(false, Pointer::to($value))?->read() ?? -4) . \" \";
        writable Holder $holder = new Holder();
        echo ($holder->maybe?->read() ?? -5) . \" \";
        $holder->maybe = Pointer::to($value);
        echo ($holder->maybe?->read() ?? -5) . \"\\n\";
        if ($maybe is Pointer<int32>) {{
            echo $maybe->read() . \" \";
        }}
        echo ($absent is Pointer<int32>) . \" \" . ($maybe is Pointer<int64>) . \" \";
        echo ($target is Pointer<int32>) . \" \" . ($target is WritablePointer<int32>) . \" \";
        Pointer<int32> $present = Pointer::to($value);
        echo ($present is Pointer<int32>) . \" \" . ($present is Pointer<uint32>);
    }}
}}
"
    );
    assert_runs_natively(
        &source,
        "7 -1 true 0 9 9 -2 2\n9 -4 -5 9\n9 false false false true true false",
    );
}
/// A pointer operation's result flows into `mixed` as every value does
/// (Decision 0148 section 4): the box holds the pointer, and `is` narrows
/// it back.
#[test]
fn a_pointer_operation_result_is_boxed_as_mixed() {
    let source = format!(
        "{POINTER}
function main(): void
{{
    writable int32 $value = 7;
    unsafe {{
        mixed $box = Pointer::to($value);
        if ($box is Pointer<int32>) {{
            echo $box->read();
        }}
    }}
}}
"
    );
    assert_runs_natively(&source, "7");
}

#[test]
fn an_aligned_access_at_a_misaligned_address_panics_with_p1702() {
    for body in [
        "?Pointer<int32> $pointer = Pointer::fromAddress(4098); if ($pointer != null) { echo $pointer->read(); }",
        "?Pointer<float64> $pointer = Pointer::fromAddress(4100); if ($pointer != null) { WritablePointer<float64> $writablePointer = WritablePointer::cast($pointer); $writablePointer->write(1.5); }",
    ] {
        let source = format!("{POINTER}function main(): void {{ unsafe {{ {body} }} }}");
        assert_panics_natively(&source, "P1702", "Pointer Access Is Misaligned");
    }
}

#[test]
fn an_offset_that_leaves_the_address_space_panics_with_p1703() {
    for body in [
        // Below zero.
        "?Pointer<uint8> $pointer = Pointer::fromAddress(8); if ($pointer != null) { echo $pointer->byteOffset(-16)->address; }",
        // To zero, which no `Pointer<T>` holds.
        "?Pointer<int64> $pointer = Pointer::fromAddress(16); if ($pointer != null) { echo $pointer->offset(-2)->address; }",
        // Past the largest address.
        "?Pointer<uint8> $pointer = Pointer::fromAddress(18446744073709551608); if ($pointer != null) { echo $pointer->byteOffset(8)->address; }",
        // The scaled count overflows before the address moves.
        "?Pointer<int64> $pointer = Pointer::fromAddress(4096); if ($pointer != null) { echo $pointer->offset(2305843009213693952)->address; }",
    ] {
        let source = format!("{POINTER}function main(): void {{ unsafe {{ {body} }} }}");
        assert_panics_natively(&source, "P1703", "Pointer Arithmetic Left The Address Space");
    }
}

#[test]
fn the_interpreter_and_php_output_report_pointer_operations() {
    let source = format!(
        "{POINTER}function main(): void {{ unsafe {{ ?Pointer<int32> $pointer = Pointer::fromAddress(4096); echo $pointer == null; }} }}"
    );
    let debug = doriac::compile_source(
        "stage40.doria",
        source.clone(),
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("the interpreter runs pointers through the companion");
    assert!(
        debug.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Pointer Operations Cannot Run In The Interpreter Yet"),
        "{debug:#?}"
    );
    let php = doriac::compile_source("stage40.doria", source, doriac::backend::BackendTarget::Php)
        .expect_err("PHP output runs pointers through the companion");
    assert!(
        php.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Pointers Cannot Be Built For PHP Output Yet"),
        "{php:#?}"
    );
}

#[test]
fn a_pointer_to_a_local_reads_and_writes_its_place() {
    let source = format!(
        "{POINTER}
function main(): void
{{
    writable int32 $value = 41;
    unsafe {{
        let $pointer = WritablePointer::to($value);
        $pointer->write(7);
        echo $pointer->read();
        echo \" \";
        echo $value;
    }}
    echo \" \";
    writable float64 $ratio = 0.0;
    unsafe {{
        let $place = WritablePointer::to($ratio);
        $place->writeUnaligned(2.5);
        echo Float::toInt($place->readUnaligned() * 2.0);
    }}
    echo \" \";
    writable bool $flag = false;
    unsafe {{
        WritablePointer::to($flag)->write(true);
    }}
    echo $flag;
    echo \" \";
    writable int64 $wide = 0;
    unsafe {{
        WritablePointer<uint8> $low = WritablePointer::cast(WritablePointer::to($wide));
        $low->write(UInt8::from(255));
        $low->byteOffset(1)->write(UInt8::from(1));
    }}
    echo $wide;
}}
"
    );
    // Every tier-1 target is little-endian.
    assert_runs_natively(&source, "7 7 5 true 511");

    // A writable Copy parameter of a function is the callee's own copy
    // (SPEC: Copy arguments keep their Copy behavior), so a pointer to it
    // addresses that copy. A writable closure parameter and a writable
    // capture name the caller's place, so a write through a pointer to
    // either is what the caller reads next.
    let parameters = format!(
        "{POINTER}function bump(writable int32 $count): void {{ unsafe {{ WritablePointer::to($count)->write(8); }} echo $count . \" \"; }}
function main(): void {{
    writable int32 $value = 1;
    bump($value);
    echo $value . \" \";
    let $set = function(writable int32 $target): void {{ unsafe {{ WritablePointer::to($target)->write(5); }} echo $target . \" \"; }};
    $set($value);
    echo $value . \" \";
    writable int32 $captured = 2;
    writable function writable(): void $raise = function(): void with (writable $captured) {{
        unsafe {{ WritablePointer::to($captured)->write(9); }}
        echo $captured . \" \";
        $captured = $captured + 1;
    }};
    $raise();
    echo $captured;
}}
"
    );
    assert_runs_natively(&parameters, "8 1 5 5 9 10");
}

const C_LIBRARY: &str = r#"
use Doria\Std\Ffi\{CChar, CLong, CSize, LinkName, Opaque, Pointer, WritablePointer};

extern "C"
{
    /** @safety Any value but the least `int32`. */
    function abs(int32 $value): int32;

    /** @safety Any value but the least `CLong`. */
    function labs(CLong $value): CLong;

    /** @safety `$text` points to a NUL-terminated string. */
    function strlen(Pointer<CChar> $text): CSize;

    /** @safety The memory is released once, with `free`. */
    function malloc(CSize $size): ?WritablePointer<Opaque>;

    /** @safety `$memory` is null, or came from `malloc` and is not used again. */
    function free(?WritablePointer<Opaque> $memory): void;

    /** @safety `$memory` is writable for `$count` bytes. */
    #[LinkName("memset")]
    function fill(WritablePointer<Opaque> $memory, int32 $value, CSize $count): WritablePointer<Opaque>;

    /** @safety `$memory` is writable for `$count` bytes. */
    #[LinkName("memset")]
    function clear(WritablePointer<Opaque> $memory, int32 $value, CSize $count): void;
}
"#;

#[test]
fn foreign_calls_reach_the_platform_c_runtime() {
    let source = format!(
        "{C_LIBRARY}
function main(): void
{{
    unsafe {{
        echo abs(-7);
        echo \" \";
        echo Int64::from(labs(CLong::from(-123456)));
        echo \" \";
        ?WritablePointer<Opaque> $memory = malloc(CSize::from(16));
        if ($memory != null) {{
            let $block = $memory;
            clear($block, 0, CSize::from(16));
            WritablePointer<uint8> $bytes = WritablePointer::cast($block);
            $bytes->write(UInt8::from(68));
            $bytes->offset(1)->write(UInt8::from(111));
            $bytes->offset(2)->write(UInt8::from(114));
            $bytes->offset(3)->write(UInt8::from(105));
            $bytes->offset(4)->write(UInt8::from(97));
            Pointer<CChar> $text = Pointer::cast(Pointer::from($bytes));
            echo UInt64::from(strlen($text));
            echo \" \";
            echo $bytes->offset(4)->read();
            echo \" \";
            let $filled = fill($block, 120, CSize::from(3));
            echo $filled == $block;
            echo \" \";
            echo $bytes->read();
            echo \" \";
            echo $bytes->offset(3)->read();
            echo \" \";
            echo UInt64::from(strlen($text));
        }}
        free($memory);
        free(null);
    }}
}}
"
    );
    // `fill` and `clear` bind one symbol with two signatures: the second
    // calls the address the first imports.
    assert_runs_natively(&source, "7 123456 5 97 true 120 105 5");
}

#[cfg(unix)]
#[test]
fn c_function_pointer_values_are_called_through_their_address() {
    // Decision 0148 section 12: addresses from `dlsym`, converted with
    // `Pointer::toFunction`, called with scalars and a struct result, held
    // nullable, in a foreign struct field, and passed through a function.
    let source = "
use Doria\\Std\\Ffi\\{CChar, Opaque, Pointer, WritablePointer};

extern \"C\" struct Quotient
{
    int32 $quotient;
    int32 $remainder;
}

extern \"C\" struct Table
{
    extern \"C\" function(int32): int32 $absolute;
}

extern \"C\"
{
    /** @safety `$path` is null or a NUL-terminated path. */
    function dlopen(?Pointer<CChar> $path, int32 $mode): ?WritablePointer<Opaque>;

    /** @safety `$handle` came from `dlopen` and `$name` is NUL-terminated. */
    function dlsym(WritablePointer<Opaque> $handle, Pointer<CChar> $name): ?WritablePointer<Opaque>;
}

function find(WritablePointer<Opaque> $handle, uint8[] $name): Pointer<Opaque>
{
    unsafe {
        ?WritablePointer<Opaque> $symbol = dlsym($handle, Pointer::cast($name->getRegion(0, $name->length)->pointer));
        if ($symbol == null) {
            panic(\"no symbol\");
        }
        return Pointer::from($symbol);
    }
}

function apply(extern \"C\" function(int32): int32 $function, int32 $value): int32
{
    return unsafe($function($value));
}

function main(): void
{
    unsafe {
        ?WritablePointer<Opaque> $program = dlopen(null, 1);
        if ($program == null) {
            panic(\"no program handle\");
        }
        let $handle = $program;
        extern \"C\" function(int32): int32 $absolute = Pointer::toFunction(find($handle, [97, 98, 115, 0]));
        echo $absolute(-5);
        extern \"C\" function(int32, int32): Quotient $divide = Pointer::toFunction(find($handle, [100, 105, 118, 0]));
        Quotient $result = $divide(17, 5);
        echo \" \" . $result->quotient . \" \" . $result->remainder;
        ?extern \"C\" function(int32): int32 $maybe = $absolute;
        if ($maybe != null) {
            echo \" \" . $maybe(-9);
        }
        let $table = new Table(absolute: $absolute);
        let $fromField = $table->absolute;
        echo \" \" . $fromField(-11);
        echo \" \" . apply($absolute, -13);
    }
}
";
    assert_runs_natively(source, "5 3 2 9 11 13");
}

#[test]
fn the_last_system_code_starts_at_zero_everywhere() {
    assert_runs_everywhere(
        "use Doria\\Std\\Ffi\\ForeignCall;\nfunction main(): void { echo ForeignCall::lastSystemCode; }",
        "0",
    );
}

#[cfg(unix)]
#[test]
fn a_capturing_declaration_keeps_its_errno_past_later_code() {
    // Decision 0148 section 16: `close(-1)` fails with `EBADF`, 9 on Linux
    // and macOS, and the code survives the allocation that follows.
    let source = "
use Doria\\Std\\Ffi\\{CaptureSystemCode, ForeignCall, SystemCodeSource};

extern \"C\"
{
    /** @safety Any descriptor. */
    #[CaptureSystemCode(SystemCodeSource::Errno)]
    function close(int32 $descriptor): int32;
}

function main(): void
{
    int32 $status = unsafe(close(-1));
    string $between = \"text \" . $status;
    echo $between . \" \" . ForeignCall::lastSystemCode;
}
";
    assert_runs_natively(source, "text -1 9");
}

const AGGREGATES: &str = r#"
extern "C" struct Color
{
    uint8 $red;
    uint8 $green;
    uint8 $blue;
    uint8 $alpha = 255;
}

extern "C" struct Sample
{
    uint8 $tag;
    int64 $value;
    uint16 $extra;
}

extern "C" struct Pixel
{
    int32 $x;
    int32 $y;
    Color $color;
    float64 $weight;
    bool $visible;
}

extern "C" union Word
{
    uint32 $whole;
    uint8 $low;
    float32 $ratio;
    Color $color;
}
"#;

#[test]
fn foreign_structs_and_unions_build_copy_and_change_their_fields() {
    let source = format!(
        "{AGGREGATES}
function brighter(Color $color, uint8 $step): Color
{{
    writable Color $result = $color;
    $result->red = $result->red + $step;
    return $result;
}}

function moved(Pixel $pixel): Pixel
{{
    writable Pixel $copy = $pixel;
    $copy->x += 5;
    $copy->y++;
    $copy->color->blue = 99;
    return $copy;
}}

function main(): void
{{
    let $color = new Color(red: 10, green: 20, blue: 30);
    echo $color->red . \" \" . $color->alpha . \" \";
    let $bright = brighter($color, 5);
    echo $bright->red . \" \" . $color->red . \" \";
    writable Pixel $pixel = new Pixel(x: 1, y: 2, color: $color, weight: 1.5, visible: true);
    let $other = moved($pixel);
    echo $other->x . \" \" . $other->y . \" \" . $other->color->blue . \" \" . $pixel->color->blue . \" \";
    echo $pixel->visible . \" \" . Float::toInt($pixel->weight * 2.0) . \" \";
    $pixel->color = $bright;
    echo $pixel->color->red . \" \" . moved($pixel)->color->red . \" \";
    echo Color::SIZE . \" \" . Color::ALIGNMENT . \" \" . Sample::SIZE . \" \" . Sample::ALIGNMENT . \" \" . Pixel::SIZE . \" \" . Word::SIZE . \" \";
    writable Word $word = new Word();
    unsafe {{
        echo $word->whole . \" \";
    }}
    $word->whole = 258;
    unsafe {{
        echo $word->low . \" \";
    }}
    let $named = new Word(color: new Color(red: 1, green: 2, blue: 3, alpha: 4));
    unsafe {{
        echo $named->whole;
    }}
}}
"
    );
    // Every tier-1 target is little-endian and lays these out alike.
    assert_runs_natively(
        &source,
        "10 255 15 10 6 3 99 30 true 3 15 15 4 1 24 8 32 4 0 2 67305985",
    );
}

/// `?Name` of a foreign struct or union (Decision 0148 section 6) is the
/// shared nullable inline layout: a presence byte, then the C layout. It
/// narrows with `!= null` and `is`, coalesces, reads fields with `?->`, and
/// is a local, parameter, result, property, static property, constant and
/// enum payload field.
#[test]
fn nullable_foreign_aggregates_narrow_coalesce_and_read_fields_null_safe() {
    let source = format!(
        "{AGGREGATES}
extern \"C\" struct Tip
{{
    int32 $id;
    uint16[3] $text;
}}

const ?Color NONE = null;
const ?Color WHITE = new Color(red: 250, green: 251, blue: 252);

class Holder
{{
    writable ?Color $maybe = WHITE;
    static writable ?Color $current = new Color(red: 5, green: 6, blue: 7);
    static writable ?Color $absent = null;
}}

enum Paint
{{
    case None;
    case Maybe(?Color $color);
}}

function find(bool $present): ?Color
{{
    if ($present) {{
        return new Color(red: 1, green: 2, blue: 3);
    }}
    return null;
}}

function pick(?Color $first, ?Color $second): ?Color
{{
    return $first ?? $second;
}}

function describe(Paint $paint): string
{{
    return match ($paint) {{
        Paint::None => \"none\",
        Paint::Maybe($color) => \"maybe \" . ($color?->blue ?? 0),
    }};
}}

function main(): void
{{
    let $none = find(false);
    let $some = find(true);
    echo ($none?->red ?? 7) . \" \" . ($some?->green ?? 7) . \" \";
    let $chosen = $none ?? new Color(red: 9, green: 9, blue: 9);
    echo $chosen->red . \" \";
    if ($some != null) {{
        echo $some->blue . \" \";
    }}
    if ($some is Color) {{
        let $copy = $some;
        echo $copy->alpha . \" \";
    }}
    echo ($chosen is Color) . \" \" . ($none is Color) . \"\\n\";
    writable ?Color $later = null;
    echo ($later == null) . \" \";
    $later = new Color(red: 4, green: 5, blue: 6);
    if ($later != null) {{
        $later->red = 40;
        $later->green += 1;
        echo $later->red . \" \" . $later->green . \" \";
    }}
    echo (pick(null, $some)?->red ?? 0) . \" \" . (pick(null, null) == null) . \" \";
    ?Color $branch = when ($some != null): ?Color {{
        return $some;
    }} else {{
        return null;
    }};
    echo ($branch?->blue ?? 0) . \"\\n\";
    echo (NONE == null) . \" \" . (WHITE?->green ?? 0) . \" \";
    writable Holder $holder = new Holder();
    echo ($holder->maybe?->blue ?? 0) . \" \";
    $holder->maybe = $none;
    echo ($holder->maybe == null) . \" \";
    $holder->maybe = $some;
    echo ($holder->maybe?->red ?? 0) . \" \";
    echo (Holder::current?->blue ?? 0) . \" \" . (Holder::absent == null) . \" \";
    Holder::current = null;
    Holder::absent = $chosen;
    echo (Holder::current?->blue ?? 0) . \" \" . (Holder::absent?->red ?? 0) . \"\\n\";
    echo describe(Paint::Maybe(null)) . \" \" . describe(Paint::Maybe($some)) . \" \" . describe(Paint::None) . \"\\n\";
    writable ?Tip $tip = new Tip(id: 3);
    if ($tip != null) {{
        $tip->text[1] = 77;
        echo $tip->id . \" \" . $tip->text[1] . \" \";
    }}
    echo ($tip?->text?->length ?? -1) . \" \";
    let $noTip = when (false): ?Tip {{
        return new Tip(id: 1);
    }} else {{
        return null;
    }};
    echo ($noTip?->text?->length ?? -1) . \" \" . Color::SIZE;
}}
"
    );
    assert_runs_natively(
        &source,
        "7 2 9 3 255 true false\ntrue 40 6 1 true 3\ntrue 251 252 true 1 7 true 0 9\nmaybe 0 maybe 3 none\n3 77 3 -1 4",
    );
}

#[test]
fn foreign_aggregates_report_where_they_cannot_be_built_yet() {
    for (body, position) in [
        (
            "class Box<T> { function __construct(take T $value) {} } function keep(Box<Color> $box): void {}",
            "a type argument",
        ),
        (
            "class Box<T> { function __construct(take T $value) {} } function keep(Box<?Color> $box): void {}",
            "a type argument",
        ),
        ("function main(): void { let $show = function(Color $color): void {}; }", "a closure parameter"),
        ("function main(): void { let $show = function(?Color $color): void {}; }", "a closure parameter"),
        (
            "function main(): void { let $color = new Color(red: 1, green: 2, blue: 3); let $show = function(): void with ($color) { echo $color->red; }; $show(); }",
            "a closure capture",
        ),
    ] {
        let source = format!("{AGGREGATES}{body}");
        let diagnostics = doriac::compile_source(
            "stage40.doria",
            source,
            doriac::backend::BackendTarget::Native,
        )
        .expect_err("the form cannot be built yet");
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Foreign Aggregates Cannot Be Built Here Yet"
                && diagnostic.message.contains(position)),
            "{body}: {diagnostics:#?}"
        );
    }
    let boxed = format!(
        "{AGGREGATES}function main(): void {{ mixed $value = new Color(red: 1, green: 2, blue: 3); }}"
    );
    let diagnostics = doriac::compile_source(
        "stage40.doria",
        boxed,
        doriac::backend::BackendTarget::Native,
    )
    .expect_err("a foreign aggregate cannot be boxed as `mixed` yet");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Foreign Aggregates Cannot Be Built Here Yet"),
        "{diagnostics:#?}"
    );
}

/// A collection stores foreign structs and unions inline in its element
/// slots (Decision 0148 sections 6 and 8): `List`, `T[]`, the Dictionary
/// families and `Deque`, each also of `?Name`, with indexing, insertion,
/// removal, nullable access and copies between collections.
#[test]
fn foreign_aggregates_are_collection_elements() {
    let source = format!(
        "{AGGREGATES}
function main(): void
{{
    writable List<Color> $colors = [];
    $colors->add(new Color(red: 1, green: 2, blue: 3));
    $colors->add(new Color(red: 4, green: 5, blue: 6));
    $colors->insertAt(0, new Color(red: 7, green: 8, blue: 9));
    echo $colors->count . \" \" . $colors[0]->red . \" \" . $colors[2]->blue . \" \";
    $colors[2] = new Color(red: 10, green: 11, blue: 12, alpha: 13);
    echo $colors[2]->alpha . \" \";
    let $removed = $colors->removeAt(0);
    echo $removed->red . \" \" . $colors->count . \" \";
    echo ($colors->get(5) ?? new Color(red: 99, green: 0, blue: 0))->red . \" \";
    echo ($colors->get(0) ?? new Color(red: 0, green: 0, blue: 0))->green . \" \";
    writable Deque<Color> $copy = Deque::from($colors);
    $colors[0] = new Color(red: 77, green: 0, blue: 0);
    echo ($copy->popFront() ?? new Color(red: 0, green: 0, blue: 0))->red . \" \" . $colors[0]->red . \" \";
    Sample[] $samples = [new Sample(tag: 1, value: -5, extra: 9), new Sample(tag: 2, value: 1234567890123, extra: 10)];
    echo $samples[0]->value . \" \" . $samples[1]->value . \" \" . $samples[1]->extra . \" \";
    writable Dictionary<string, Word> $words = [];
    $words[\"a\"] = new Word(whole: 258);
    $words[\"b\"] = new Word(color: new Color(red: 1, green: 1, blue: 1, alpha: 1));
    unsafe {{
        echo $words[\"a\"]->whole . \" \" . $words[\"b\"]->whole . \" \";
    }}
    echo $words->count . \" \" . ($words->get(\"c\") == null) . \" \";
    let $gone = $words->remove(\"a\");
    unsafe {{
        echo ($gone ?? new Word())->whole . \" \" . $words->count . \" \";
    }}
    writable List<?Color> $maybe = [null, new Color(red: 3, green: 3, blue: 3)];
    $maybe->add(null);
    echo ($maybe[0] == null) . \" \" . ($maybe[1] ?? new Color(red: 0, green: 0, blue: 0))->red . \" \" . $maybe->count . \" \";
    echo ($maybe->removeAt(2) == null) . \" \" . ($maybe->get(1) == null) . \" \";
    writable Deque<Color> $queue = Deque::from([new Color(red: 22, green: 0, blue: 0)]);
    $queue->pushBack(new Color(red: 21, green: 0, blue: 0));
    $queue->pushFront(new Color(red: 20, green: 0, blue: 0));
    echo ($queue->popFront() ?? new Color(red: 0, green: 0, blue: 0))->red . \" \";
    echo ($queue->popBack() ?? new Color(red: 0, green: 0, blue: 0))->red . \" \" . $queue->count;
}}
"
    );
    // Every tier-1 target is little-endian: 258 with its low byte 2 is
    // 0x0102, and four 1 bytes are 0x01010101.
    assert_runs_natively(
        &source,
        "3 7 6 13 7 2 99 2 1 77 -5 1234567890123 10 258 16843009 2 true 258 1 true 3 3 true false 20 21 1",
    );
}

/// A field of a collection or region element is a place in the element's
/// storage: a field write, a nested field write, read-modify-write, a
/// fixed-length array field's region and a writable foreach binding each
/// change the element in place (Decision 0148 sections 6 and 8).
#[test]
fn fields_of_aggregate_elements_change_in_place() {
    let source = format!(
        "{AGGREGATES}
extern \"C\" struct Tip
{{
    int32 $x;
    Color $color;
    uint16[4] $text;
}}

class Palette
{{
    function __construct(
        take writable List<Color> $colors,
    ) {{}}

    writable function brighten(): void
    {{
        $this->colors[0]->red += 1;
    }}
}}

function sum(List<Color> $colors): int
{{
    writable int $total = 0;
    foreach ($colors as Color $color) {{
        $total += Int64::from($color->red);
    }}
    return $total;
}}

function main(): void
{{
    writable Tip[] $tips = [new Tip(x: 1, color: new Color(red: 1, green: 1, blue: 1))];
    $tips[0]->color->blue = 9;
    $tips[0]->x++;
    writable WritableArrayRegion<uint16> $text = $tips[0]->text;
    $text[2] = 44;
    echo $tips[0]->color->blue . \" \" . $tips[0]->x . \" \" . $tips[0]->text[2] . \" \";
    foreach ($tips as writable Tip $tip) {{
        $tip->x = $tip->x * 10;
    }}
    echo $tips[0]->x . \" \";
    writable List<Color> $colors = [new Color(red: 1, green: 2, blue: 3)];
    $colors[0]->green = 50;
    foreach ($colors as writable Color $color) {{
        $color->blue = $color->blue + 100;
    }}
    echo $colors[0]->green . \" \" . $colors[0]->blue . \" \";
    writable Palette $palette = new Palette([new Color(red: 5, green: 0, blue: 0), new Color(red: 6, green: 0, blue: 0)]);
    $palette->brighten();
    echo $palette->colors[0]->red . \" \" . sum($palette->colors) . \" \";
    writable SortedDictionary<int, Color> $sorted = SortedDictionary::from([]);
    $sorted[3] = new Color(red: 3, green: 0, blue: 0);
    $sorted[1] = new Color(red: 1, green: 0, blue: 0);
    $sorted[3]->blue = 33;
    foreach ($sorted as int $key => writable Color $color) {{
        $color->green = UInt8::from($key);
    }}
    foreach ($sorted as int $key => Color $color) {{
        echo $key . \"=\" . $color->red . \"/\" . $color->green . \"/\" . $color->blue . \" \";
    }}
    writable Dictionary<string, Word> $words = [];
    $words[\"a\"] = new Word(whole: 258);
    $words[\"a\"]->low = 7;
    unsafe {{
        echo $words[\"a\"]->whole;
    }}
}}
"
    );
    assert_runs_natively(&source, "9 2 44 20 50 103 6 12 1=1/1/0 3=3/3/33 263");
}

/// A typed array region over foreign structs (Decision 0148 section 8)
/// views the array's elements at their C stride: element reads and writes,
/// field writes, `foreach` and a writable binding reach the array, and C
/// reads the elements through the region's pointer.
#[test]
fn typed_regions_view_aggregate_elements_and_c_reads_them() {
    let source = format!(
        "{POINTER}use Doria\\Std\\Ffi\\{{CSize, Opaque}};
{AGGREGATES}
extern \"C\"
{{
    /** @safety Both ranges hold `$count` bytes and do not overlap. */
    function memcpy(WritablePointer<Opaque> $destination, Pointer<Opaque> $source, CSize $count): WritablePointer<Opaque>;
}}

function main(): void
{{
    writable Sample[] $samples = [new Sample(tag: 1, value: 10, extra: 100), new Sample(tag: 2, value: 20, extra: 200), new Sample(tag: 3, value: 30, extra: 300)];
    let $region = $samples->getRegion(1, 2);
    echo $region->length . \" \" . $region[0]->value . \" \" . $region[1]->extra . \" \";
    foreach ($region as Sample $sample) {{
        echo $sample->tag . \" \";
    }}
    writable WritableArrayRegion<Sample> $writable = $samples->getWritableRegion(0, 3);
    $writable[2]->value = 333;
    $writable[0] = new Sample(tag: 9, value: 90, extra: 900);
    foreach ($writable as writable Sample $sample) {{
        $sample->extra = $sample->extra + 1;
    }}
    echo $samples[0]->tag . \" \" . $samples[2]->value . \" \" . $samples[1]->extra . \" \";
    writable Sample $target = new Sample(tag: 0, value: 0, extra: 0);
    unsafe {{
        let $source = $samples->getRegion(0, 3);
        memcpy(WritablePointer::cast<Opaque>(WritablePointer::to($target)), Pointer::cast<Opaque>($source->pointer->offset(2)), CSize::from(Sample::SIZE));
        echo $target->tag . \" \" . $target->value . \" \" . $target->extra . \" \";
        echo ($source->pointer->offset(1)->address - $source->pointer->address) . \" \" . Sample::SIZE;
    }}
}}
"
    );
    assert_runs_natively(&source, "2 20 300 2 3 9 333 201 3 333 301 24 24");
}

#[test]
fn the_interpreter_and_php_output_report_foreign_aggregates() {
    let source = format!(
        "{AGGREGATES}function main(): void {{ let $color = new Color(red: 1, green: 2, blue: 3); echo $color->red; }}"
    );
    let debug = doriac::compile_source(
        "stage40.doria",
        source.clone(),
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("the interpreter runs foreign aggregates through the companion");
    assert!(
        debug.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Foreign Structs And Unions Cannot Run In The Interpreter Yet"),
        "{debug:#?}"
    );
    let php = doriac::compile_source("stage40.doria", source, doriac::backend::BackendTarget::Php)
        .expect_err("PHP output takes the C data model through the companion");
    assert!(
        php.iter().any(|diagnostic| diagnostic.code == "E0784"),
        "{php:#?}"
    );
    // A construction stored straight into a property is a companion
    // operation too.
    let property = format!(
        "{AGGREGATES}class Holder {{ Color $color = new Color(red: 1, green: 2, blue: 3); }} function main(): void {{ let $holder = new Holder(); }}"
    );
    let debug = doriac::compile_source(
        "stage40.doria",
        property,
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("the interpreter runs foreign aggregates through the companion");
    assert!(
        debug.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Foreign Structs And Unions Cannot Run In The Interpreter Yet"),
        "{debug:#?}"
    );
    // A static property holding one is reported where it is declared.
    let held = format!(
        "{AGGREGATES}class Palette {{ static Color $primary = new Color(red: 1, green: 2, blue: 3); }} function main(): void {{ echo Palette::primary->red; }}"
    );
    let debug = doriac::compile_source(
        "stage40.doria",
        held.clone(),
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("the interpreter runs foreign aggregates through the companion");
    assert!(
        debug.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Foreign Structs And Unions Cannot Run In The Interpreter Yet"),
        "{debug:#?}"
    );
    let php = doriac::compile_source("stage40.doria", held, doriac::backend::BackendTarget::Php)
        .expect_err("PHP output takes the C data model through the companion");
    assert!(
        php.iter().any(|diagnostic| diagnostic.code == "E0784"),
        "{php:#?}"
    );
    // An enum whose payload holds an aggregate is reported where it is
    // held.
    let payload = format!(
        "{AGGREGATES}enum Paint {{ case Solid(Color $color); }} function show(Paint $paint): void {{}} function main(): void {{ show(Paint::Solid(new Color(red: 1, green: 2, blue: 3))); }}"
    );
    let debug = doriac::compile_source(
        "stage40.doria",
        payload,
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("the interpreter runs foreign aggregates through the companion");
    assert!(
        debug.iter().any(|diagnostic| diagnostic.code == "E0784"
            && diagnostic.title == "Foreign Structs And Unions Cannot Run In The Interpreter Yet"),
        "{debug:#?}"
    );
    let constant = format!("{AGGREGATES}function main(): void {{ echo Color::SIZE; }}");
    let php = doriac::compile_source(
        "stage40.doria",
        constant,
        doriac::backend::BackendTarget::Php,
    )
    .expect_err("PHP output takes the C data model through the companion");
    assert!(
        php.iter().any(|diagnostic| diagnostic.code == "E0784"),
        "{php:#?}"
    );
}

#[test]
fn pointers_read_and_write_whole_aggregates_and_their_fields() {
    let source = format!(
        "{POINTER}{AGGREGATES}
function main(): void
{{
    writable Pixel $pixel = new Pixel(x: 1, y: 2, color: new Color(red: 3, green: 4, blue: 5), weight: 0.5, visible: false);
    unsafe {{
        let $whole = WritablePointer::to($pixel);
        Pixel $copy = $whole->read();
        echo $copy->x . \" \" . $copy->color->blue . \" \";
        writable Pixel $changed = $copy;
        $changed->y = 40;
        $whole->write($changed);
        echo $pixel->y . \" \";
        let $green = WritablePointer::to($pixel->color->green);
        $green->write(UInt8::from(77));
        echo $pixel->color->green . \" \";
        let $color = WritablePointer::to($pixel->color);
        Color $read = $color->read();
        echo $read->green . \" \" . $whole->read()->color->green . \" \";
        Pointer<Pixel> $base = Pointer::from($whole);
        echo $base->offset(1)->address - $base->address;
        let $y = WritablePointer::to($pixel->y);
        $y->writeUnaligned(9);
        echo \" \" . $y->readUnaligned() . \" \" . $pixel->y . \" \";
        WritablePointer<uint8> $bytes = WritablePointer::cast($whole);
        echo $bytes->offset(8)->read();
    }}
}}
"
    );
    assert_runs_natively(&source, "1 5 40 77 77 77 32 9 9 3");
}

#[test]
fn an_aligned_aggregate_access_at_a_misaligned_address_panics_with_p1702() {
    for body in [
        "?Pointer<Sample> $pointer = Pointer::fromAddress(4100); if ($pointer != null) { Sample $sample = $pointer->read(); echo $sample->tag; }",
        "?Pointer<Sample> $pointer = Pointer::fromAddress(4100); if ($pointer != null) { WritablePointer<Sample> $writablePointer = WritablePointer::cast($pointer); $writablePointer->write(new Sample(tag: 1, value: 2, extra: 3)); }",
    ] {
        let source = format!("{POINTER}{AGGREGATES}function main(): void {{ unsafe {{ {body} }} }}");
        assert_panics_natively(&source, "P1702", "Pointer Access Is Misaligned");
    }
}

const DIVISION: &str = r#"
use Doria\Std\Ffi\CLong;

extern "C" struct Division
{
    int32 $quotient;
    int32 $remainder;
}

extern "C" struct LongDivision
{
    CLong $quotient;
    CLong $remainder;
}

extern "C" struct WideDivision
{
    int64 $quotient;
    int64 $remainder;
}

extern "C"
{
    /** @safety `$divisor` is not zero and the quotient fits. */
    function div(int32 $dividend, int32 $divisor): Division;

    /** @safety `$divisor` is not zero and the quotient fits. */
    function ldiv(CLong $dividend, CLong $divisor): LongDivision;

    /** @safety `$divisor` is not zero and the quotient fits. */
    function lldiv(int64 $dividend, int64 $divisor): WideDivision;
}
"#;

#[test]
fn foreign_calls_return_structs_by_value() {
    // `div_t` returns in one register on every tier-1 target, `ldiv_t` in
    // one (Windows) or two, and `lldiv_t` in two or, on Windows x64,
    // through the caller's result storage.
    let source = format!(
        "{DIVISION}
function main(): void
{{
    unsafe {{
        let $small = div(17, 5);
        echo $small->quotient . \" \" . $small->remainder . \" \";
        let $long = ldiv(CLong::from(-29), CLong::from(4));
        echo Int64::from($long->quotient) . \" \" . Int64::from($long->remainder) . \" \";
        let $wide = lldiv(9000000000000, 7);
        echo $wide->quotient . \" \" . $wide->remainder;
    }}
}}
"
    );
    assert_runs_natively(&source, "3 2 -7 -1 1285714285714 2");
}

/// System V x86_64 and AArch64 pass a 16-byte struct of two integer
/// words in the two registers its fields would take as separate
/// arguments, and a struct of one `float64` in the register a `float64`
/// takes; Windows x64 passes the first by reference. These bindings call
/// C runtime functions through those equivalent signatures. Arguments that
/// travel on the stack or by reference are tested where Doria links a
/// library that takes them (`#[Library]`).
#[cfg(not(windows))]
#[test]
fn foreign_calls_pass_structs_by_value_in_registers() {
    let source = format!(
        "{POINTER}
use Doria\\Std\\Ffi\\{{CSize, LinkName, Opaque}};

extern \"C\" struct Fill
{{
    WritablePointer<Opaque> $memory;
    int64 $value;
}}

extern \"C\" struct Scale
{{
    float64 $value;
}}

extern \"C\"
{{
    /** @safety `$fill->memory` is writable for `$count` bytes. */
    #[LinkName(\"memset\")]
    function fill(Fill $fill, CSize $count): WritablePointer<Opaque>;

    /** @safety Any value. */
    #[LinkName(\"ldexp\")]
    function scale(Scale $value, int32 $exponent): float64;
}}

function main(): void
{{
    writable int64 $target = 0;
    unsafe {{
        WritablePointer<Opaque> $memory = WritablePointer::cast(WritablePointer::to($target));
        let $filled = fill(new Fill(memory: $memory, value: 1), CSize::from(3));
        echo $filled == $memory;
    }}
    echo \" \" . $target . \" \";
    unsafe {{
        echo Float::toInt(scale(new Scale(value: 1.5), 4));
    }}
}}
"
    );
    // memset writes three bytes of 1: 0x010101.
    assert_runs_natively(&source, "true 65793 24");
}

#[test]
fn an_omitted_fixed_length_array_field_is_zero_filled() {
    let source = format!(
        "{POINTER}
extern \"C\" struct Tip
{{
    uint16[3] $text;
    int32 $length = 7;
}}

function main(): void
{{
    writable Tip $tip = new Tip();
    echo Tip::SIZE . \" \" . $tip->length . \" \";
    unsafe {{
        WritablePointer<uint16> $text = WritablePointer::cast(WritablePointer::to($tip));
        echo $text->read() . \" \" . $text->offset(2)->read() . \" \";
        $text->offset(1)->write(UInt16::from(9));
        Pointer<int32> $length = Pointer::cast(Pointer::from($text)->byteOffset(8));
        echo $length->read() . \" \" . Pointer::from($text)->offset(1)->read();
    }}
}}
"
    );
    // `uint16[3]` takes bytes 0 to 5; `int32` is aligned to byte 8.
    assert_runs_natively(&source, "12 7 0 0 7 9");
}

#[test]
fn null_is_the_constant_default_of_nullable_pointers() {
    // Decision 0148 sections 6 and 15: foreign fields and extern parameters
    // take constant defaults, and `null` is the one constant of a nullable
    // pointer or C function pointer, as it is for an ordinary parameter.
    let source = format!(
        "{POINTER}use Doria\\Std\\Ffi\\{{CChar, CLong, CString}};

extern \"C\" struct Link
{{
    int32 $value;
    ?Pointer<Link> $next = null;
    ?extern \"C\" function(int32): int32 $visit = null;
}}

extern \"C\"
{{
    /** @safety `$text` points to a NUL-terminated string. */
    function strtol(Pointer<CChar> $text, ?WritablePointer<Pointer<CChar>> $end = null, int32 $base = 10): CLong;
}}

function absent(?Pointer<int64> $pointer = null): bool
{{
    return $pointer == null;
}}

function main(): void
{{
    let $link = new Link(value: 3);
    echo ($link->next == null) . \" \" . ($link->visit == null) . \" \" . absent() . \" \";
    let $decimal = CString::from(\"42\");
    let $hexadecimal = CString::from(\"ff\");
    unsafe {{
        echo Int64::from(strtol($decimal->pointer)) . \" \";
        echo Int64::from(strtol($hexadecimal->pointer, base: 16));
    }}
}}
"
    );
    assert_runs_natively(&source, "true true true 42 255");
}

#[test]
fn a_nullable_pointer_is_one_word_and_null_is_address_zero() {
    // Decision 0148 section 3: `?Pointer<T>` is one word and null is address
    // zero wherever it is stored, so C reads and writes it in place: in a
    // foreign field, through `WritablePointer::to` of a local, through a
    // `Pointer<?Pointer<T>>`, and as a C result. Doria storage holds the same word.
    let source = format!(
        "{POINTER}use Doria\\Std\\Ffi\\{{CChar, CLong, CString, CUnsignedLong}};

extern \"C\" struct Stream
{{
    ?Pointer<uint8> $nextIn = null;
    uint32 $availableIn = 0;
    CUnsignedLong $totalIn;
}}

extern \"C\"
{{
    /** @safety `$text` is NUL-terminated; `$end` is null or writable. */
    function strtol(Pointer<CChar> $text, ?WritablePointer<?Pointer<CChar>> $end, int32 $base): CLong;
    /** @safety `$name` is NUL-terminated. */
    function getenv(Pointer<CChar> $name): ?Pointer<CChar>;
}}

class Holder
{{
    function __construct(writable ?Pointer<uint8> $pointer) {{}}
}}

function pass(?Pointer<uint8> $pointer): ?Pointer<uint8>
{{
    return $pointer;
}}

function main(): void
{{
    let $input = \"abc\"->bytes;
    writable Stream $stream = new Stream(totalIn: CUnsignedLong::from(7));
    unsafe {{
        let $from = $input->getRegion(0, 3)->pointer;
        Pointer<uint64> $words = Pointer::cast(Pointer::from(WritablePointer::to($stream)));
        echo $words->read() . \" \" . Stream::SIZE . \" \";
        $stream->nextIn = $from;
        $stream->availableIn = 3;
        echo ($words->read() == $from->address) . \" \" . $words->offset(1)->read() . \" \";
        echo $words->offset(2)->read() . \" \";
        echo (($stream->nextIn ?? $from)->address == $from->address) . \"\\n\";

        let $digits = CString::from(\"42rest\");
        writable ?Pointer<CChar> $end = null;
        echo Int64::from(strtol($digits->pointer, WritablePointer::to($end), 10)) . \" \";
        echo (($end ?? $digits->pointer)->address - $digits->pointer->address) . \" \";
        echo Int64::from(strtol($digits->pointer, null, 10)) . \" \";
        writable ?Pointer<CChar> $later = null;
        let $laterPlace = WritablePointer::to($later);
        if ($later == null) {{
            strtol($digits->pointer, $laterPlace, 10);
            echo (($later ?? $digits->pointer)->address - $digits->pointer->address) . \" \";
        }}
        let $name = CString::from(\"DORIA_STAGE40_NO_SUCH_VARIABLE\");
        echo (getenv($name->pointer) == null) . \"\\n\";

        writable ?Pointer<uint8> $slot = null;
        let $place = WritablePointer::to($slot);
        Pointer<uint64> $raw = Pointer::cast(Pointer::from($place));
        echo ($place->read() == null) . \" \";
        $place->write($from);
        echo ($raw->read() == $from->address) . \" \" . ($slot != null) . \" \";
        $place->write(null);
        echo $raw->read() . \" \" . ($slot == null) . \"\\n\";

        writable Holder $holder = new Holder($from);
        echo ((pass($holder->pointer) ?? $from)->address == $from->address) . \" \";
        $holder->pointer = null;
        echo (pass($holder->pointer) == null) . \" \";
        List<?Pointer<uint8>> $list = [null, $from];
        echo ($list[0] == null) . \" \" . (($list[1] ?? $from)->address == $from->address);
    }}
}}
"
    );
    assert_runs_natively(
        &source,
        "0 24 true 3 7 true\n42 2 42 2 true\ntrue true true 0 true\ntrue true true true",
    );
}

#[test]
fn a_region_pointer_gives_c_the_storage_itself() {
    let source = format!(
        "{POINTER}use Doria\\Std\\Ffi\\CSize;
use Doria\\Std\\Io\\WritableByteRegion;

extern \"C\"
{{
    /** @safety `$memory` is writable for `$count` bytes. */
    function memset(WritablePointer<uint8> $memory, int32 $value, CSize $count): WritablePointer<uint8>;
}}

function main(): void
{{
    writable Bytes $buffer = Bytes::allocate(6);
    unsafe {{
        writable WritableByteRegion $region = $buffer->getWritableRegion(1, 4);
        let $data = $region->writablePointer;
        let $filled = memset($data, 65, CSize::from(4));
        echo $filled == $data;
        echo \" \";
        echo $region->pointer->address == $data->address;
    }}
    echo \" \" . $buffer[0] . \" \" . $buffer[1] . \" \" . $buffer[4] . \" \" . $buffer[5];
    unsafe {{
        let $whole = $buffer->getRegion(0, 6)->pointer;
        let $part = $buffer->getRegion(2, 3)->pointer;
        echo \" \" . ($part->address - $whole->address);
        echo \" \" . $part->read();
    }}
    let $empty = Bytes::allocate(0);
    unsafe {{
        echo \" \" . ($empty->getRegion(0, 0)->pointer->address != UInt64::from(0));
        let $end = $buffer->getRegion(6, 0)->pointer;
        echo \" \" . ($end->address - $buffer->getRegion(0, 6)->pointer->address);
    }}
    let $text = \"Doria\";
    unsafe {{
        echo \" \" . $text->byteRegion->pointer->offset(1)->read();
    }}
}}
"
    );
    // memset writes 65 into bytes 1 to 4 of the buffer itself: C receives
    // the storage's address, and no copy is made.
    assert_runs_natively(&source, "true true 0 65 65 0 2 65 true 6 111");
}

const ARRAY_REGIONS: &str = "
function total(ArrayRegion<int32> $values): int64
{
    writable int64 $sum = 0;
    for (writable int $index = 0; $index < $values->length; $index++) {
        $sum += Int64::from($values[$index]);
    }
    return $sum;
}

function fill(writable WritableArrayRegion<int32> $values, int32 $value): void
{
    for (writable int $index = 0; $index < $values->length; $index++) {
        $values[$index] = $value;
    }
}
";

#[test]
fn foreach_walks_typed_array_regions_in_index_order() {
    // Decision 0148 section 8, amendment Iterating Regions of 2026-11-20:
    // readonly elements, the `int` index, and writable elements changed in
    // place through a writable region binding.
    let source = "function main(): void
{
    writable int32[] $numbers = [1, 2, 3, 4, 5];
    foreach ($numbers->getRegion(0, 3) as int32 $value) {
        echo $value;
    }
    foreach ($numbers->getRegion(2, 3) as int $index => int32 $value) {
        echo \" \" . $index . \":\" . $value;
    }
    {
        let writable $tail = $numbers->getWritableRegion(1, 3);
        foreach ($tail as writable int32 $value) {
            if ($value == 3) {
                continue;
            }
            $value *= 10;
        }
    }
    foreach ($numbers->getRegion(0, 0) as int32 $value) {
        echo \" empty\";
    }
    echo \" \";
    foreach ($numbers as int32 $value) {
        echo $value . \",\";
    }
}
";
    assert_runs_everywhere(source, "123 0:3 1:4 2:5 1,20,3,40,5,");
}

#[test]
fn a_writable_region_foreach_keeps_the_last_change_on_break_and_return() {
    // Decision 0148 section 8: a writable region binding stores back through
    // the foreach body's exit obligation, so leaving the body by `break` or
    // `return` keeps the change made in that iteration.
    let source =
        "function bumpUntil(writable WritableArrayRegion<int32> $values, int32 $limit): int
{
    foreach ($values as int $index => writable int32 $value) {
        $value += 100;
        if ($value > $limit) {
            return $index;
        }
    }
    return -1;
}

function main(): void
{
    writable int32[] $numbers = [1, 2, 3, 4, 5];
    {
        let writable $front = $numbers->getWritableRegion(0, 4);
        foreach ($front as writable int32 $value) {
            $value = 7;
            break;
        }
    }
    {
        let writable $rest = $numbers->getWritableRegion(1, 4);
        echo bumpUntil($rest, 102);
    }
    echo \" \";
    foreach ($numbers as int32 $value) {
        echo $value . \",\";
    }
}
";
    assert_runs_everywhere(source, "1 7,102,103,4,5,");
}

#[test]
fn handle_operations_take_their_value_type_from_a_stated_type_argument() {
    // Decision 0148 section 11 with Decision 0105, Amendment: Type Arguments
    // At The Call: `reclaim<T>` and `borrow<T>` read `T` from the call, so
    // the result needs no declared type.
    let source = r#"
use Doria\Std\Ffi\{Handle, Opaque, WritablePointer};

class Counter
{
    function __construct(writable int $count) {}
}

function main(): void
{
    WritablePointer<Opaque> $handle = Handle::create<Counter>(new Counter(3));
    unsafe {
        let $borrowed = Handle::borrow<Counter>($handle);
        echo $borrowed->count;
        echo " " . Handle::borrow<Counter>($handle)->count;
        let $back = Handle::reclaim<Counter>($handle);
        echo " " . $back->count;
    }
    SharedReference<Counter> $shared = shared new Counter(7);
    WritablePointer<Opaque> $claim = $shared->toHandle();
    unsafe {
        let $returned = SharedReference::reclaim<Counter>($claim);
        echo " " . $returned->count;
    }
}
"#;
    assert_runs_natively(source, "3 3 3 7");
}

#[test]
fn pointer_conversions_take_a_stated_target() {
    // `Pointer::cast<T>` and `Pointer::fromAddress<T>` state the pointee, so
    // a read needs no typed local.
    let source = r#"
use Doria\Std\Ffi\{Opaque, Pointer};

/** @safety `$pointer` addresses an `int32`. */
unsafe function readThrough(Pointer<Opaque> $pointer): int32
{
    return unsafe(Pointer::cast<int32>($pointer)->read());
}

function main(): void
{
    writable int32 $value = 41;
    unsafe {
        let $opaque = Pointer::cast<Opaque>(Pointer::to($value));
        echo readThrough($opaque);
        let $again = Pointer::fromAddress<int32>($opaque->address) ?? Pointer::cast<int32>($opaque);
        echo " " . $again->read();
    }
}
"#;
    assert_runs_natively(source, "41 41");
}

#[test]
fn typed_array_regions_view_read_write_and_copy_elements_in_place() {
    // Decision 0148 section 8: views over `T[]` that copy nothing.
    let source = format!(
        "{ARRAY_REGIONS}
function main(): void
{{
    writable int32[] $numbers = [1, 2, 3, 4, 5, 6];
    echo total($numbers->getRegion(1, 3));
    {{
        writable WritableArrayRegion<int32> $tail = $numbers->getWritableRegion(4, 2);
        fill($tail, 7);
        $tail[0] += 1;
    }}
    echo \" \" . $numbers[4] . \" \" . $numbers[5];
    let $part = $numbers->getRegion(0, 6)->getRegion(2, 2);
    echo \" \" . $part[0] . \" \" . $part[1] . \" \" . $part->length;
    int32[] $other = [9, 10];
    {{
        writable WritableArrayRegion<int32> $front = $numbers->getWritableRegion(0, 3);
        echo \" \" . $front->copyFrom($other->getRegion(0, 2));
        let $inner = $front->getWritableRegion(2, 1);
        echo \" \" . $inner->length;
    }}
    echo \" \" . $numbers[0] . \" \" . $numbers[1] . \" \" . $numbers[2];
    writable uint16[] $wide = [1, 2, 3];
    {{
        writable WritableArrayRegion<uint16> $middle = $wide->getWritableRegion(1, 1);
        $middle[0] = UInt16::from(65535);
    }}
    echo \" \" . $wide[1] . \" \" . $wide->getRegion(3, 0)->length;
}}
"
    );
    assert_runs_everywhere(&source, "9 8 7 3 4 2 2 1 9 10 3 65535 0");
}

#[test]
fn typed_array_regions_view_float_bool_and_enum_elements() {
    // Decision 0148 section 8: a region over `T[]` for every element type
    // stored bit for bit.
    let source = "
enum Suit
{
    case Hearts;
    case Spades;
}

function letter(Suit $suit): string
{
    if ($suit == Suit::Hearts) {
        return \"h\";
    }
    return \"s\";
}

function flag(bool $value): string
{
    if ($value) {
        return \"t\";
    }
    return \"f\";
}

function sum(ArrayRegion<float> $values): float
{
    writable float $total = 0.0;
    for (writable int $index = 0; $index < $values->length; $index++) {
        $total += $values[$index];
    }
    return $total;
}

function main(): void
{
    writable float[] $wide = [0.5, 1.5, 2.5];
    {
        writable WritableArrayRegion<float> $tail = $wide->getWritableRegion(1, 2);
        $tail[0] += 1.0;
        echo \"{$tail[1]}\";
    }
    echo \" {$wide[1]} {sum($wide->getRegion(0, 3))}\";
    float32[] $narrow = [0.25, 1.25];
    let $narrowRegion = $narrow->getRegion(1, 1);
    float32 $narrowValue = $narrowRegion[0];
    echo \" {$narrowValue}\";
    writable bool[] $flags = [true, false, false];
    {
        writable WritableArrayRegion<bool> $middle = $flags->getWritableRegion(1, 2);
        $middle[0] = true;
    }
    let $flagRegion = $flags->getRegion(0, 3);
    echo \" \" . flag($flagRegion[0]) . flag($flagRegion[1]) . flag($flagRegion[2]);
    writable Suit[] $suits = [Suit::Hearts, Suit::Hearts];
    {
        writable WritableArrayRegion<Suit> $all = $suits->getWritableRegion(0, 2);
        $all[1] = Suit::Spades;
    }
    let $suitRegion = $suits->getRegion(0, 2);
    echo \" \" . letter($suitRegion[0]) . letter($suitRegion[1]);
    float[] $source = [9.5];
    {
        writable WritableArrayRegion<float> $front = $wide->getWritableRegion(0, 1);
        echo \" \" . $front->copyFrom($source->getRegion(0, 1));
    }
    echo \" {$wide[0]}\";
}
";
    assert_runs_everywhere(source, "2.5 2.5 5.5 1.25 ttf hs 1 9.5");
}

#[test]
fn typed_array_regions_view_pointer_elements() {
    // Decision 0148 section 8 over `Pointer<T>[]`.
    let source = format!(
        "{POINTER}
function main(): void
{{
    int32 $first = 4;
    int32 $second = 5;
    unsafe {{
        writable Pointer<int32>[] $pointers = [Pointer::to($first), Pointer::to($first)];
        {{
            writable WritableArrayRegion<Pointer<int32>> $slots = $pointers->getWritableRegion(0, 2);
            $slots[1] = Pointer::to($second);
        }}
        let $view = $pointers->getRegion(0, 2);
        Pointer<int32> $one = $view[0];
        Pointer<int32> $two = $view[1];
        echo $one->read() . \" \" . $two->read();
    }}
}}
"
    );
    assert_runs_natively(&source, "4 5");
    // Fixed-length array fields of float and bool elements (section 6).
    let fields = "
extern \"C\" struct Sample
{
    float32[2] $weights;
    bool[2] $set;
}

function main(): void
{
    float32[] $weights = [0.5, 0.25];
    writable Sample $sample = new Sample(weights: $weights->getRegion(0, 2));
    $sample->set[1] = true;
    $sample->weights[0] += 1.0;
    float32 $first = $sample->weights[0];
    float32 $second = $sample->weights[1];
    echo \"{$first} {$second}\";
    if ($sample->set[1] and not $sample->set[0]) {
        echo \" set\";
    }
}
";
    assert_runs_natively(fields, "1.5 0.25 set");
}

#[test]
fn typed_array_regions_view_nullable_pointer_elements() {
    // Decision 0148 sections 3 and 8: a `?Pointer<T>` element is one word,
    // null at address zero, so C reads the region as `T **`.
    let source = format!(
        "{POINTER}
function main(): void
{{
    int32 $first = 4;
    int32 $second = 5;
    unsafe {{
        writable (?Pointer<int32>)[] $slots = [Pointer::to($first), null, null];
        {{
            writable WritableArrayRegion<?Pointer<int32>> $region = $slots->getWritableRegion(0, 3);
            $region[1] = Pointer::to($second);
            $region[0] = null;
            echo ($region[0] == null) . \" \" . (($region[1] ?? Pointer::to($first))->read()) . \" \";
        }}
        let $view = $slots->getRegion(0, 3);
        Pointer<uint64> $words = Pointer::cast($view->pointer);
        echo $words->read() . \" \" . ($words->offset(1)->read() == Pointer::to($second)->address) . \" \";
        echo $words->offset(2)->read() . \" \";
        Pointer<?Pointer<int32>> $secondSlot = $view->pointer->offset(1);
        echo (($secondSlot->read() ?? Pointer::to($first))->read()) . \" \" . ($view[2] == null);
    }}
}}
"
    );
    assert_runs_natively(&source, "true 5 0 true 0 5 true");
    // A fixed-length array field of nullable pointers (section 6) is a
    // region of words, zero-filled when omitted.
    let fields = format!(
        "{POINTER}
extern \"C\" struct Table
{{
    (?Pointer<int32>)[2] $entries;
    int32 $count = 2;
}}

function main(): void
{{
    int32 $first = 4;
    int32 $second = 5;
    unsafe {{
        writable Table $table = new Table();
        echo Table::SIZE . \" \" . ($table->entries[0] == null) . \" \" . ($table->entries[1] == null) . \" \";
        $table->entries[1] = Pointer::to($second);
        echo (($table->entries[1] ?? Pointer::to($first))->read()) . \" \";
        (?Pointer<int32>)[] $copied = [Pointer::to($first), null];
        writable Table $other = new Table(entries: $copied->getRegion(0, 2));
        echo (($other->entries[0] ?? Pointer::to($second))->read()) . \" \" . ($other->entries[1] == null);
    }}
}}
"
    );
    assert_runs_natively(&fields, "24 true true 5 4 true");
    // Without pointer formation the interpreter reads and writes them too.
    assert_runs_everywhere(
        "use Doria\\Std\\Ffi\\Pointer;
function main(): void
{
    writable (?Pointer<int32>)[] $slots = [null, null];
    writable WritableArrayRegion<?Pointer<int32>> $region = $slots->getWritableRegion(0, 2);
    $region[1] = null;
    echo ($region[0] == null) . \" \" . ($region[1] == null);
}
",
        "true true",
    );
}

#[test]
fn array_region_bounds_panic_as_typed_arrays_do() {
    assert_panics_everywhere(
        "function main(): void { int32[] $numbers = [1, 2, 3]; echo \"a\"; let $region = $numbers->getRegion(2, 2); echo $region->length; }",
        "a",
        "P1305",
    );
    assert_panics_everywhere(
        "function main(): void { int32[] $numbers = [1, 2, 3]; let $region = $numbers->getRegion(1, 2); echo $region[1]; echo $region[2]; }",
        "3",
        "P1310",
    );
}

#[test]
fn array_region_pointers_address_the_typed_array_storage() {
    let source = format!(
        "{POINTER}
function main(): void
{{
    writable int32[] $numbers = [1, 2, 3, 4];
    unsafe {{
        let $first = $numbers->getRegion(1, 2)->pointer;
        echo $first->read() . \" \" . $first->offset(1)->read();
        let $whole = $numbers->getRegion(0, 4)->pointer;
        echo \" \" . ($first->address - $whole->address);
    }}
    unsafe {{
        writable WritableArrayRegion<int32> $region = $numbers->getWritableRegion(3, 1);
        $region->writablePointer->write(40);
    }}
    echo \" \" . $numbers[3];
    int64[] $empty = [];
    unsafe {{
        echo \" \" . ($empty->getRegion(0, 0)->pointer->address % UInt64::from(8));
    }}
}}
"
    );
    assert_runs_natively(&source, "2 3 4 40 0");
}

#[test]
fn array_regions_report_what_cannot_be_built_yet() {
    let strings = "function main(): void { string[] $names = [\"Ada\"]; let $region = $names->getRegion(0, 1); echo $region->length; }";
    let diagnostics = doriac::lower_source_to_mir("stage40.doria", strings.to_string())
        .expect_err("array regions of strings are not built yet");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Array Regions Of This Element Type Cannot Be Built Yet"),
        "{diagnostics:#?}"
    );
    let source = format!("{ARRAY_REGIONS}function main(): void {{ int32[] $numbers = [1]; echo total($numbers->getRegion(0, 1)); }}");
    let diagnostics =
        doriac::compile_source("stage40.doria", source, doriac::backend::BackendTarget::Php)
            .expect_err("PHP output does not build typed array regions yet");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B2301"),
        "{diagnostics:#?}"
    );
}

#[test]
fn fixed_length_array_fields_are_regions_over_the_aggregate() {
    let source = format!(
        "{POINTER}
extern \"C\" struct Tip
{{
    uint16[3] $text;
    int32 $length = 7;
}}

function total(ArrayRegion<uint16> $values): int
{{
    writable int $sum = 0;
    for (writable int $index = 0; $index < $values->length; $index++) {{
        $sum += Int64::from($values[$index]);
    }}
    return $sum;
}}

function supply(ArrayRegion<uint16> $source): Tip
{{
    return new Tip(text: $source);
}}

function main(): void
{{
    uint16[] $letters = [72, 105, 33, 0];
    writable Tip $tip = new Tip(text: $letters->getRegion(0, 3));
    echo $tip->text[0] . \" \" . $tip->text[2] . \" \" . $tip->text->length . \" \" . $tip->length;
    $tip->text[1] = UInt16::from(9);
    $tip->text[2] += UInt16::from(1);
    echo \" \" . total($tip->text->getRegion(0, 3));
    let $copy = $tip;
    $tip->text[0] = UInt16::from(1);
    echo \" \" . $copy->text[0];
    unsafe {{
        Pointer<uint16> $text = $tip->text->pointer;
        Pointer<int32> $length = Pointer::cast($text->byteOffset(8));
        echo \" \" . $text->offset(1)->read() . \" \" . $length->read();
    }}
    let $empty = new Tip();
    echo \" \" . total($empty->text);
    let $fromRegion = supply($letters->getRegion(1, 3));
    echo \" \" . $fromRegion->text[0];
}}
"
    );
    assert_runs_natively(&source, "72 33 3 7 115 72 9 7 0 105");
    let mismatch = format!(
        "{POINTER}
extern \"C\" struct Tip
{{
    uint16[3] $text;
}}

function supply(ArrayRegion<uint16> $source): Tip
{{
    return new Tip(text: $source);
}}

function main(): void
{{
    uint16[] $letters = [72, 105];
    let $tip = supply($letters->getRegion(0, 2));
    echo $tip->text[0];
}}
"
    );
    assert_panics_natively(
        &mismatch,
        "P1704",
        "Array Region Length Does Not Match The Field",
    );
}

#[test]
fn assigning_a_fixed_length_array_field_copies_the_region() {
    // Decision 0148 section 6, amended 2026-11-17.
    let source = "
extern \"C\" struct Tip
{
    uint16[3] $text;
    uint16[3] $other;
}

extern \"C\" struct Outer
{
    int8 $tag;
    Tip $tip;
}

// A writable Copy parameter is the callee's own copy.
function replaced(writable Tip $tip, ArrayRegion<uint16> $source): Tip
{
    $tip->text = $source;
    return $tip;
}

function main(): void
{
    writable uint16[] $letters = [72, 105, 33, 46];
    writable Tip $tip = new Tip();
    $tip->text = $letters->getRegion(1, 3);
    $letters[1] = UInt16::from(0);
    echo $tip->text[0] . \" \" . $tip->text[2];
    $tip->other = $tip->text->getRegion(0, 3);
    $tip->text[0] = UInt16::from(1);
    echo \" \" . $tip->other[0];
    $tip = replaced($tip, $letters->getRegion(0, 3));
    echo \" \" . $tip->text[0] . \" \" . $tip->text[1];
    $tip->text = $tip->text->getRegion(0, 3);
    echo \" \" . $tip->text[0];
    writable Outer $outer = new Outer(tag: 1, tip: new Tip());
    $outer->tip->other = $tip->other->getRegion(0, 3);
    echo \" \" . $outer->tip->other[2] . \" \" . $outer->tip->text[2];
}
";
    assert_runs_natively(source, "105 46 105 72 0 72 46 0");
    let mismatch = "
extern \"C\" struct Tip
{
    uint16[3] $text;
}

function main(): void
{
    uint16[] $letters = [72, 105];
    writable Tip $tip = new Tip();
    echo \"a\";
    $tip->text = $letters->getRegion(0, $letters->length);
    echo $tip->text[0];
}
";
    assert_panics_natively(
        mismatch,
        "P1704",
        "Array Region Length Does Not Match The Field",
    );
}

const FOREIGN_STRINGS: &str = "use Doria\\Std\\Ffi\\{CString, Utf16String};\n";

#[test]
fn foreign_strings_are_built_moved_and_released_everywhere() {
    let source = format!(
        "{FOREIGN_STRINGS}
function keep(take CString $text): CString
{{
    return $text;
}}

function main(): void
{{
    let $name = CString::from(\"héllo\");
    let $kept = keep($name);
    let $wide = Utf16String::from(text: \"A😀\");
    let $empty = CString::from(\"\");
    let $zeros = Bytes::allocate(3);
    let $withNul = String::fromBytes($zeros) ?? \"\";
    ?CString $missing = CString::tryFrom($withNul);
    ?Utf16String $present = Utf16String::tryFrom(\"ok\");
    if ($missing == null) {{
        echo \"absent \";
    }}
    if ($present != null) {{
        echo \"present\";
    }}
}}
"
    );
    assert_runs_everywhere(&source, "absent present");
}

#[test]
fn text_with_nul_panics_with_p1705() {
    for construction in ["CString::from", "Utf16String::from"] {
        let source = format!(
            "{FOREIGN_STRINGS}
function main(): void
{{
    echo \"before \";
    let $zeros = Bytes::allocate(3);
    let $withNul = String::fromBytes($zeros) ?? \"\";
    let $text = {construction}($withNul);
    echo \"after\";
}}
"
        );
        assert_panics_everywhere(&source, "before ", "P1705");
    }
}

#[test]
fn foreign_strings_give_c_their_buffers_and_read_text_back() {
    let source = format!(
        "{C_LIBRARY}
use Doria\\Std\\Ffi\\{{CString, Utf16String}};

function main(): void
{{
    let $name = CString::from(\"héllo\");
    let $wide = Utf16String::from(\"A😀\");
    let writable $bytes = Bytes::allocate(2);
    $bytes[0] = UInt8::from(255);
    unsafe {{
        echo UInt64::from(strlen($name->pointer));
        echo \" \";
        echo String::fromCString($name->pointer) ?? \"invalid\";
        echo \" \";
        echo $wide->pointer->offset(1)->read();
        echo \" \";
        echo $wide->pointer->offset(3)->read();
        echo \" \";
        echo String::fromUtf16($wide->pointer) ?? \"invalid\";
        echo \" \";
        Pointer<CChar> $invalid = Pointer::cast($bytes->getRegion(0, 2)->pointer);
        echo String::fromCString($invalid) ?? \"invalid\";
    }}
}}
"
    );
    assert_runs_natively(&source, "6 héllo 55357 0 A😀 invalid");
}

#[test]
fn php_output_reports_that_it_does_not_build_foreign_strings_yet() {
    let source =
        format!("{FOREIGN_STRINGS}function main(): void {{ let $name = CString::from(\"x\"); }}");
    let diagnostics =
        doriac::compile_source("stage40.doria", source, doriac::backend::BackendTarget::Php)
            .expect_err("PHP output does not build foreign strings yet");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Foreign Strings Cannot Be Built For PHP Output Yet"),
        "{diagnostics:#?}"
    );
}

const SORTING: &str = "use Doria\\Std\\Ffi\\{CSize, Opaque, Pointer, WritablePointer};

extern \"C\" function compareIntegers(Pointer<Opaque> $left, Pointer<Opaque> $right): int32
{
    unsafe {
        Pointer<int32> $first = Pointer::cast($left);
        Pointer<int32> $second = Pointer::cast($right);
        let $a = $first->read();
        let $b = $second->read();
        if ($a < $b) {
            return -1;
        }
        if ($a > $b) {
            return 1;
        }
        return 0;
    }
}

extern \"C\"
{
    /** @safety `$base` holds `$count` elements of `$size` bytes each. */
    function qsort(WritablePointer<Opaque> $base, CSize $count, CSize $size,
        extern \"C\" function(Pointer<Opaque>, Pointer<Opaque>): int32 $compare): void;
}
";

#[test]
fn c_calls_a_c_abi_function_through_its_name_as_a_value() {
    // Decision 0148 section 12: the name of a C-callable function is the
    // address of its C entry, which C calls under the C convention.
    let source = format!(
        "{SORTING}
function main(): void
{{
    writable int32[] $values = [5, -3, 9, 1, 7, 1];
    unsafe {{
        writable WritableArrayRegion<int32> $region = $values->getWritableRegion(0, 6);
        qsort(WritablePointer::cast($region->writablePointer), CSize::from(6), CSize::from(4), compareIntegers);
    }}
    foreach ($values as int32 $value) {{
        echo $value . \" \";
    }}
}}
"
    );
    assert_runs_natively(&source, "-3 1 1 5 7 9 ");
}

#[test]
fn a_c_abi_function_that_catches_its_io_errors_runs_as_a_qsort_callback() {
    // Decision 0148 section 12 (amendment of 2026-11-20): a C-callable function
    // whose `echo` sits inside `try` with a `catch` for its I/O error lets
    // no checked error reach C, so its C entry uses the ordinary call.
    let source = "use Doria\\Std\\Ffi\\{CSize, Opaque, Pointer, WritablePointer};

extern \"C\" function compareLoudly(Pointer<Opaque> $left, Pointer<Opaque> $right): int32
{
    try {
        echo \".\";
    } catch (Doria\\Std\\Io\\IoError $error) {
        return 0;
    }
    unsafe {
        Pointer<int32> $first = Pointer::cast($left);
        Pointer<int32> $second = Pointer::cast($right);
        return $first->read() - $second->read();
    }
}

extern \"C\"
{
    /** @safety `$base` holds `$count` elements of `$size` bytes each. */
    function qsort(WritablePointer<Opaque> $base, CSize $count, CSize $size,
        extern \"C\" function(Pointer<Opaque>, Pointer<Opaque>): int32 $compare): void;
}

function main(): void
{
    writable int32[] $values = [3, 1, 2];
    unsafe {
        writable WritableArrayRegion<int32> $region = $values->getWritableRegion(0, 3);
        qsort(WritablePointer::cast($region->writablePointer), CSize::from(3), CSize::from(4), compareLoudly);
    }
    echo \" \";
    foreach ($values as int32 $value) {
        echo $value;
    }
}
";
    let mir = program(source);
    assert_interpreter_reports_companion(&mir);
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(0), "{profile:?}: {run:?}");
        let stdout = String::from_utf8_lossy(&run.stdout);
        // The number of comparisons is the C library's choice; each one
        // prints a dot.
        let (dots, sorted) = stdout
            .split_once(' ')
            .unwrap_or_else(|| panic!("{profile:?}: {stdout}"));
        assert!(
            !dots.is_empty() && dots.chars().all(|character| character == '.'),
            "{profile:?}: {stdout}"
        );
        assert_eq!(sorted, "123", "{profile:?}");
        assert!(run.stderr.is_empty(), "{profile:?}: {run:?}");
    }
}

#[test]
fn variadic_foreign_functions_are_called_per_arity_on_every_native_profile() {
    // Decision 0148 section 14: one binding per used arity. LLVM calls
    // `snprintf` directly; Cranelift calls it through the runtime's variadic
    // call trampoline (amendment of 2026-11-17). The second binding passes
    // more values than the argument registers hold.
    let source = "use Doria\\Std\\Ffi\\{CChar, CSize, CString, LinkName, Pointer, Variadic, WritablePointer};
use Doria\\Std\\Io\\WritableByteRegion;

extern \"C\"
{
    /** @safety `$buffer` holds `$size` bytes and `$format` matches the values. */
    #[LinkName(\"snprintf\")]
    #[Variadic(fixedParameters: 3)]
    function formatThree(WritablePointer<CChar> $buffer, CSize $size, Pointer<CChar> $format,
        int32 $count, float64 $ratio, int64 $total): int32;

    /** @safety `$buffer` holds `$size` bytes and `$format` matches the values. */
    #[LinkName(\"snprintf\")]
    #[Variadic(fixedParameters: 3)]
    function formatMany(WritablePointer<CChar> $buffer, CSize $size, Pointer<CChar> $format,
        int64 $a, int64 $b, int64 $c, int64 $d, int64 $e, int64 $f,
        float64 $g, float64 $h, float64 $i, float64 $j, float64 $k,
        float64 $l, float64 $m, float64 $n, float64 $o): int32;
}

function main(): void
{
    let writable $bytes = Bytes::allocate(128);
    let $three = CString::from(\"%d %.2f %lld\");
    let $many = CString::from(\"%lld %lld %lld %lld %lld %lld %.1f %.1f %.1f %.1f %.1f %.1f %.1f %.1f %.1f\");
    unsafe {
        writable WritableByteRegion $region = $bytes->getWritableRegion(0, 128);
        WritablePointer<CChar> $buffer = WritablePointer::cast($region->writablePointer);
        let $written = formatThree($buffer, CSize::from(128), $three->pointer, 7, 2.5, -9);
        echo $written . \" \" . (String::fromCString(Pointer::from($buffer)) ?? \"invalid\") . \"\\n\";
        formatMany($buffer, CSize::from(128), $many->pointer, 1, 2, 3, 4, 5, 6,
            0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5);
        echo String::fromCString(Pointer::from($buffer)) ?? \"invalid\";
    }
}
";
    assert_runs_natively(
        source,
        "9 7 2.50 -9\n1 2 3 4 5 6 0.5 1.5 2.5 3.5 4.5 5.5 6.5 7.5 8.5",
    );
}

#[test]
fn a_c_entry_receives_and_returns_every_c_form() {
    // A C function pointer value holding a C entry's address calls it
    // under the C convention, so each value crosses the boundary twice:
    // integers of every width, `bool`, `float32`, `float64`, pointers
    // present and absent, and a struct in and out by value.
    let source = "use Doria\\Std\\Ffi\\{CLong, Opaque, Pointer, WritablePointer};

extern \"C\" struct Pair
{
    int64 $first;
    float64 $second;
}

extern \"C\" struct Wide
{
    int64 $a;
    int64 $b;
    int64 $c;
}

extern \"C\" function describe(int8 $small, uint16 $medium, bool $flag, float32 $ratio,
    ?Pointer<Opaque> $address, CLong $count): int64
{
    writable int64 $total = Int64::from($small) + Int64::from($medium) + Int64::from($count);
    if ($flag) {
        $total += 1000;
    }
    if ($address == null) {
        $total += 20000;
    }
    if ($ratio == 0.5) {
        $total += 300000;
    }
    return $total;
}

extern \"C\" function swap(Pair $pair): Pair
{
    return new Pair(first: $pair->first * 2, second: $pair->second + 1.5);
}

extern \"C\" function widen(Wide $value, bool $negate): Wide
{
    if ($negate) {
        return new Wide(a: -$value->a, b: -$value->b, c: -$value->c);
    }
    return $value;
}

extern \"C\" function absent(?Pointer<Opaque> $address): ?Pointer<Opaque>
{
    return $address;
}

function main(): void
{
    unsafe {
        extern \"C\" function(int8, uint16, bool, float32, ?Pointer<Opaque>, CLong): int64 $describe = describe;
        float32 $ratio = 0.5;
        echo $describe(-5, 60000, true, $ratio, null, CLong::from(7)) . \" \";
        extern \"C\" function(Pair): Pair $swap = swap;
        let $swapped = $swap(new Pair(first: 3, second: 8.0));
        echo $swapped->first . \" \" . $swapped->second . \" \";
        extern \"C\" function(Wide, bool): Wide $widen = widen;
        let $wide = $widen(new Wide(a: 1, b: 2, c: 3), true);
        echo $wide->a;
        echo $wide->b;
        echo $wide->c;
        echo \" \";
        extern \"C\" function(?Pointer<Opaque>): ?Pointer<Opaque> $absent = absent;
        echo $absent(null) == null;
    }
}
";
    assert_runs_natively(source, "381002 6 9.5 -1-2-3 true");
}

#[test]
fn a_panic_in_a_callback_shows_the_foreign_call_in_its_call_path() {
    // Decision 0148 section 13: the C entry links the callback's frame to
    // the Doria frame of the outbound call, through a `[foreign call]`
    // frame.
    let source = format!(
        "{SORTING}
extern \"C\" function refuse(Pointer<Opaque> $left, Pointer<Opaque> $right): int32
{{
    panic(\"no comparisons\");
}}

function sort(writable int32[] $values): void
{{
    unsafe {{
        writable WritableArrayRegion<int32> $region = $values->getWritableRegion(0, 2);
        qsort(WritablePointer::cast($region->writablePointer), CSize::from(2), CSize::from(4), refuse);
    }}
}}

function main(): void
{{
    writable int32[] $values = [2, 1];
    sort($values);
}}
"
    );
    let mir = program(&source);
    for profile in NativeProfile::AVAILABLE {
        let run = execute_native(&mir, *profile);
        assert_eq!(run.status.code(), Some(101), "{profile:?}: {run:?}");
        let stderr = String::from_utf8_lossy(&run.stderr);
        let path = stderr
            .split("Call Path")
            .nth(1)
            .unwrap_or_else(|| panic!("{profile:?}: {stderr}"));
        let frames = path
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| line.split(" \u{b7} ").next().unwrap_or(line))
            .take_while(|line| !line.starts_with("Process Exited"))
            .collect::<Vec<_>>();
        assert_eq!(
            frames,
            ["refuse", "[foreign call]", "sort", "main"],
            "{profile:?}: {stderr}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_callback_on_another_thread_panics_with_p1706() {
    // Decision 0148 section 13: C may call a C-callable function only on the
    // thread running Doria code.
    let source = "use Doria\\Std\\Ffi\\{Opaque, Pointer, WritablePointer};

extern \"C\" function work(?WritablePointer<Opaque> $context): ?WritablePointer<Opaque>
{
    return $context;
}

extern \"C\"
{
    /** @safety `$thread` is writable for one `pthread_t`. */
    function pthread_create(WritablePointer<uint64> $thread, ?Pointer<Opaque> $attributes,
        extern \"C\" function(?WritablePointer<Opaque>): ?WritablePointer<Opaque> $start,
        ?WritablePointer<Opaque> $context): int32;

    /** @safety `$thread` came from `pthread_create` and was not joined. */
    function pthread_join(uint64 $thread, ?WritablePointer<Opaque> $result): int32;
}

function main(): void
{
    writable uint64 $thread = 0;
    unsafe {
        if (pthread_create(WritablePointer::to($thread), null, work, null) == 0) {
            pthread_join($thread, null);
        }
    }
    echo \"joined\";
}
";
    assert_panics_natively(source, "P1706", "C Called Doria From Another Thread");
}

/// A C allocator for the foreign-memory programs (Decision 0148 section
/// 10): `malloc` and `free` from the platform C runtime.
const C_ALLOCATOR: &str = "use Doria\\Std\\Ffi\\{CSize, Opaque, Pointer, WritablePointer};
use Doria\\Std\\Io\\{ByteRegion, IoError};

extern \"C\"
{
    /** @safety The memory is released with `free`. */
    function malloc(CSize $size): ?WritablePointer<Opaque>;

    /** @safety `$memory` came from `malloc` and is released once. */
    function free(WritablePointer<Opaque> $memory): void;
}

function allocate(int $bytes): WritablePointer<Opaque>
{
    unsafe {
        ?WritablePointer<Opaque> $memory = malloc(CSize::from($bytes));
        if ($memory != null) {
            return $memory;
        }
    }
    panic(\"out of memory\");
}
";

#[test]
fn an_owner_class_exposes_memory_a_c_allocator_owns_as_a_region() {
    // Decision 0148 section 10's DecodedImage: the owner's destructor gives
    // the memory back to the library, and the region its method returns
    // borrows `$this` (Decision 0089), so safe code reads it after every
    // unsafe context has ended.
    let source = format!(
        "{C_ALLOCATOR}
class DecodedImage
{{
    function __construct(
        internal WritablePointer<uint8> $pixels,
        internal int $length,
    ) {{}}

    function getPixels(): ByteRegion
    {{
        return unsafe(ByteRegion::fromPointer(
            $this,
            Pointer::from($this->pixels),
            $this->length,
        ));
    }}

    function __destruct()
    {{
        try {{
            echo \" released\";
        }} catch (IoError $error) {{
        }}
        unsafe {{ free(WritablePointer::cast($this->pixels)); }}
    }}
}}

function decode(int $length): DecodedImage
{{
    unsafe {{
        WritablePointer<uint8> $pixels = WritablePointer::cast(allocate($length));
        for (writable int $index = 0; $index < $length; $index++) {{
            $pixels->offset($index)->write(UInt8::from($index * 3));
        }}
        return new DecodedImage($pixels, $length);
    }}
}}

function total(ByteRegion $bytes): int
{{
    writable int $sum = 0;
    for (writable int $index = 0; $index < $bytes->length; $index++) {{
        $sum += Int::from($bytes[$index]);
    }}
    return $sum;
}}

function main(): void
{{
    {{
        let $image = decode(5);
        let $pixels = $image->getPixels();
        echo $pixels->length . \" \" . $pixels[4] . \" \" . total($pixels->getRegion(1, 3));
    }}
    echo \" done\";
}}
"
    );
    assert_runs_natively(&source, "5 12 18 released done");
}

#[test]
fn typed_regions_over_foreign_memory_read_and_write_its_elements() {
    // `ArrayRegion::fromPointer` takes its element type from the pointer;
    // the writable form needs writable access to its owner.
    let source = format!(
        "{C_ALLOCATOR}
class Samples
{{
    function __construct(internal WritablePointer<int32> $values, internal int $count) {{}}

    writable function getWritableValues(): WritableArrayRegion<int32>
    {{
        return unsafe(WritableArrayRegion::fromPointer($this, $this->values, $this->count));
    }}

    function getValues(): ArrayRegion<int32>
    {{
        return unsafe(ArrayRegion::fromPointer($this, Pointer::from($this->values), $this->count));
    }}

    function __destruct()
    {{
        unsafe {{ free(WritablePointer::cast($this->values)); }}
    }}
}}

function main(): void
{{
    let writable $samples = unsafe(new Samples(WritablePointer::cast(allocate(16)), 4));
    {{
        let writable $values = $samples->getWritableValues();
        for (writable int $index = 0; $index < $values->length; $index++) {{
            $values[$index] = Int32::from($index * 10 - 5);
        }}
    }}
    let $values = $samples->getValues();
    for (writable int $index = 0; $index < $values->length; $index++) {{
        echo $values[$index] . \" \";
    }}
    echo $values->getRegion(2, 2)[1];
}}
"
    );
    assert_runs_natively(&source, "-5 5 15 25 25");
}

#[test]
fn a_region_over_foreign_memory_needs_an_aligned_pointer_and_a_range() {
    // An `int32` region at an odd address panics with P1702 before safe
    // code reads it; a negative length is no range (P1703).
    let misaligned = format!(
        "{C_ALLOCATOR}
function main(): void
{{
    let $memory = allocate(16);
    unsafe {{
        Pointer<int32> $values = Pointer::cast(Pointer::from($memory)->byteOffset(1));
        let $region = ArrayRegion::fromPointer($memory, $values, 2);
        echo $region->length;
    }}
}}
"
    );
    assert_panics_natively(&misaligned, "P1702", "Pointer Access Is Misaligned");
    let negative = format!(
        "{C_ALLOCATOR}
function main(): void
{{
    let $memory = allocate(16);
    int $length = -1;
    unsafe {{
        Pointer<uint8> $bytes = Pointer::cast(Pointer::from($memory));
        let $region = ByteRegion::fromPointer($memory, $bytes, $length);
        echo $region->length;
    }}
}}
"
    );
    assert_panics_natively(
        &negative,
        "P1703",
        "Pointer Arithmetic Left The Address Space",
    );
}

#[test]
fn handles_carry_owned_and_shared_values_through_c_and_back() {
    // Decision 0148 section 11: a closed class's handle is its object, an
    // open class's a box that keeps its exact type, and a shared
    // reference's its strong claim. `bsearch` keeps the handle of a
    // `WritableSharedReference` as its key and passes it back to the
    // comparison callback, which borrows it with its runtime access checks.
    let source = r#"
use Doria\Std\Ffi\{CSize, Handle, Opaque, Pointer, WritablePointer};

class Counter
{
    function __construct(writable int $count) {}

    function __destruct()
    {
        try {
            echo " dropped " . $this->count;
        } catch (Doria\Std\Io\IoError $error) {
        }
    }
}

open class Shape
{
    open function describe(): string { return "shape"; }
}

class Circle extends Shape
{
    override function describe(): string { return "circle"; }
}

function show(Counter $counter): void
{
    echo $counter->count;
}

function describe(Shape $shape): void
{
    echo " " . $shape->describe();
}

function bump(WritableSharedReference<Counter> $counter): void
{
    let writable $access = $counter->acquireWritableAccess();
    $access->count++;
}

extern "C" function compareKey(Pointer<Opaque> $key, Pointer<Opaque> $element): int32
{
    unsafe {
        bump(WritableSharedReference::borrow(WritablePointer::cast($key)));
        Pointer<int32> $value = Pointer::cast($element);
        return 5 - $value->read();
    }
}

extern "C"
{
    /** @safety `$base` holds `$count` sorted elements of `$size` bytes. */
    function bsearch(Pointer<Opaque> $key, Pointer<Opaque> $base, CSize $count, CSize $size,
        extern "C" function(Pointer<Opaque>, Pointer<Opaque>): int32 $compare): ?Pointer<Opaque>;
}

function main(): void
{
    WritablePointer<Opaque> $owned = Handle::create(new Counter(3));
    unsafe {
        show(Handle::borrow($owned));
        Counter $back = Handle::reclaim($owned);
        echo " back " . $back->count;
    }
    Shape $shape = new Circle();
    WritablePointer<Opaque> $boxed = Handle::create($shape);
    unsafe {
        describe(Handle::borrow($boxed));
        Shape $again = Handle::reclaim($boxed);
        describe($again);
    }
    SharedReference<Counter> $shared = shared new Counter(7);
    let $second = $shared->share();
    WritablePointer<Opaque> $claim = $shared->toHandle();
    unsafe {
        SharedReference<Counter> $returned = SharedReference::reclaim($claim);
        echo " shared " . $returned->count;
    }
    echo " second " . $second->count;
    let $writable = new WritableSharedReference(new Counter(0));
    let $key = $writable->share();
    int32[] $values = [1, 3, 5, 7];
    unsafe {
        WritablePointer<Opaque> $context = $key->toHandle();
        let $region = $values->getRegion(0, 4);
        ?Pointer<Opaque> $found = bsearch(Pointer::from($context), Pointer::cast($region->pointer), CSize::from(4), CSize::from(4), compareKey);
        echo " found " . ($found != null);
        WritableSharedReference<Counter> $mine = WritableSharedReference::reclaim($context);
    }
    echo " compared " . ($writable->acquireReadonlyAccess()->count > 0);
}
"#;
    assert_runs_natively(
        source,
        "3 back 3 dropped 3 circle circle shared 7 second 7 found true compared true dropped 1 dropped 7",
    );
}

#[test]
fn handles_box_shared_references_to_interfaces() {
    // Decision 0148 section 11 (amendment of 2026-11-20): a shared
    // reference to an interface is a two-word carrier, so its handle boxes
    // both words, `borrow` reads through the box and `reclaim` frees it.
    let source = r#"use Doria\Std\Ffi\{Opaque, WritablePointer};

interface Named
{
    function name(): string;
}

class Person implements Named
{
    function __construct(string $text) {}
    function name(): string { return $this->text; }
    function __destruct()
    {
        try {
            echo " dropped " . $this->text;
        } catch (Doria\Std\Io\IoError $error) {
        }
    }
}

function show(SharedReference<Named> $named): void
{
    echo " " . $named->name();
}

function showWritable(WritableSharedReference<Named> $named): void
{
    echo " " . $named->acquireReadonlyAccess()->name();
}

function main(): void
{
    SharedReference<Named> $named = shared new Person("Ada");
    let $second = $named->share();
    WritablePointer<Opaque> $handle = $named->toHandle();
    unsafe {
        show(SharedReference::borrow($handle));
        SharedReference<Named> $back = SharedReference::reclaim($handle);
        show($back);
    }
    show($second);
    let $writable = new WritableSharedReference<Named>(new Person("Grace"));
    WritablePointer<Opaque> $claim = $writable->toHandle();
    unsafe {
        showWritable(WritableSharedReference::borrow($claim));
        WritableSharedReference<Named> $mine = WritableSharedReference::reclaim($claim);
        showWritable($mine);
    }
    echo " end";
}
"#;
    assert_runs_natively(
        source,
        " Ada Ada Ada Grace Grace dropped Grace end dropped Ada",
    );
}

/// Decision 0148 section 21: a standard stream the process starts without is
/// the null device. A Unix parent can start a program with its streams
/// closed, as a Windows GUI program starts with none.
#[cfg(unix)]
#[test]
fn absent_standard_streams_discard_writes_and_report_end_of_input() {
    const SOURCE: &str = r#"
use Doria\Std\Io\StandardStreams;

function main(): void
{
    echo "discarded";
    let $line = read_line();
    let $output = StandardStreams::output;
    write_stderr("{$output->isInteractive} " . ($line ?? "end") . "\n");
}
"#;
    let without_streams = |program: &std::path::Path, interpreter: Option<&str>| {
        let script = match interpreter {
            Some(interpreter) => format!("exec {interpreter} \"$0\" >&- <&-"),
            None => "exec \"$0\" >&- <&-".to_string(),
        };
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .arg(program)
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap()
    };
    let mir = program(SOURCE);
    for profile in NativeProfile::AVAILABLE {
        let directory = execution_directory::ExecutionDirectory::new();
        let executable = native_execution::stage_executable(&mir, *profile, &directory);
        let run = without_streams(&executable, None);
        assert_eq!(run.status.code(), Some(0), "{profile:?}: {run:?}");
        assert_eq!(
            String::from_utf8_lossy(&run.stderr),
            "false end\n",
            "{profile:?}"
        );
    }

    // PHP output reads the same rule from its own standard streams.
    let php = doriac::compile_source_to_php(
        "stage40.doria",
        "function main(): void { echo \"discarded\"; let $line = read_line(); write_stderr(($line ?? \"end\") . \"\\n\"); }",
    )
    .unwrap();
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let directory = execution_directory::ExecutionDirectory::new();
    let script = directory.path().join("program.php");
    std::fs::write(&script, php).unwrap();
    let run = without_streams(&script, Some("php"));
    assert_eq!(run.status.code(), Some(0), "PHP: {run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stderr), "end\n", "PHP");
}

/// Decision 0148 section 17's example: each platform's window class binds
/// its own libraries, and only the target's branch is emitted and linked
/// (amendment of 2026-11-21: only reachable code is emitted).
const PLATFORM_WINDOWS: &str = r#"
use Doria\Std\Ffi\{Library, Opaque, OperatingSystem, Platform, Pointer};

#[Library("user32")]
extern "C"
{
    /** @safety Any thread may call it. */
    function GetCurrentThreadId(): uint32;
}

#[Library("x11")]
extern "C"
{
    /** @safety `$display` is an open display. */
    function XDefaultScreen(Pointer<Opaque> $display): int32;
}

extern "C"
{
    /** @safety Any value. */
    function abs(int32 $value): int32;
}

class Win32Window
{
    static function open(string $title): void
    {
        echo unsafe(GetCurrentThreadId());
        echo " {$title}";
    }
}

class CocoaWindow
{
    static function open(string $title): void
    {
        echo unsafe(abs(-7));
        echo " {$title}";
    }
}

class X11Window
{
    static function open(string $title): void
    {
        echo " {$title}";
    }

    static function screen(Pointer<Opaque> $display): int32
    {
        return unsafe(XDefaultScreen($display));
    }
}

function main(): void
{
    let $title = "Doria";
    if (Platform::operatingSystem == OperatingSystem::Windows) {
        Win32Window::open($title);
    } else if (Platform::operatingSystem == OperatingSystem::MacOs) {
        CocoaWindow::open($title);
    } else {
        X11Window::open($title);
    }
}
"#;

fn emitted_functions(mir: &doriac::mir::Program, fragment: &str) -> Vec<String> {
    mir.functions
        .iter()
        .filter(|function| function.name.contains(fragment))
        .map(|function| function.name.clone())
        .collect()
}

#[test]
fn only_the_target_platform_branch_is_emitted_and_linked() {
    let mir = program(PLATFORM_WINDOWS);
    let target = if cfg!(windows) {
        "Win32Window"
    } else if cfg!(target_os = "macos") {
        "CocoaWindow"
    } else {
        "X11Window"
    };
    for class in ["Win32Window", "CocoaWindow", "X11Window"] {
        let functions = emitted_functions(&mir, class);
        if class == target {
            assert_eq!(functions.len(), 1, "{functions:?}");
            assert!(functions[0].ends_with("::open"), "{functions:?}");
        } else {
            assert!(functions.is_empty(), "{class}: {functions:?}");
        }
    }
    // `X11Window::screen` is never called, so `XDefaultScreen` is not
    // emitted even where the X11 branch is the target's.
    assert!(emitted_functions(&mir, "XDefaultScreen").is_empty());
    if cfg!(target_os = "macos") {
        // `user32` and `x11` have no link input here, and nothing emitted
        // references them.
        assert_runs_natively(PLATFORM_WINDOWS, "7 Doria");
    }
}

/// A class constructed only in a branch constant false for the target emits
/// none of its members, so the library it binds needs no link input.
const DEAD_HELPER: &str = r#"
use Doria\Std\Ffi\Library;

const bool TRACING = false;

#[Library("tracing")]
extern "C"
{
    /** @safety Any code. */
    function traceMessage(int32 $code): int32;
}

class Tracer
{
    function __construct(int32 $code) {}

    function emit(): int32
    {
        return unsafe(traceMessage($this->code));
    }

    function __destruct()
    {
        unsafe { traceMessage(0); }
    }
}

function trace(int32 $code): int32
{
    let $tracer = new Tracer($code);
    return $tracer->emit();
}

class Limits
{
    static int $count = 3;

    static function describe(): string
    {
        return " limit " . self::count;
    }
}

function main(): void
{
    if (TRACING) {
        echo trace(1);
    }
    echo "ran";
    echo Limits::describe();
}
"#;

#[test]
fn a_helper_class_used_only_from_a_dead_branch_is_not_emitted() {
    let mir = program(DEAD_HELPER);
    for fragment in ["Tracer", "trace", "traceMessage"] {
        assert!(
            emitted_functions(&mir, fragment).is_empty(),
            "{fragment}: {:?}",
            emitted_functions(&mir, fragment)
        );
    }
    assert_runs_everywhere(DEAD_HELPER, "ran limit 3");
    let contents = assert_php_prints(DEAD_HELPER, "ran limit 3");
    assert!(!contents.contains("traceMessage"), "{contents}");
    assert!(!contents.contains("function emit"), "{contents}");
}

/// Dispatch through an interface reaches every class reachable code
/// constructs; an implementation nothing constructs is not emitted.
const SHAPES: &str = r#"
use Doria\Std\Ffi\Library;

const bool EXPERIMENTAL = false;

#[Library("hexagons")]
extern "C"
{
    /** @safety Any side. */
    function hexagonArea(int32 $side): int32;
}

interface Shape
{
    function area(): int;
}

class Square implements Shape
{
    function __construct(int $side) {}

    function area(): int
    {
        return $this->side * $this->side;
    }
}

open class Rectangle implements Shape
{
    function __construct(int $width, int $height) {}

    open function area(): int
    {
        return $this->width * $this->height;
    }
}

class Strip extends Rectangle
{
    function __construct(int $length)
    {
        parent::__construct($length, 1);
    }

    override function area(): int
    {
        return $this->width + 100;
    }
}

class Hexagon implements Shape
{
    function __construct(int32 $side) {}

    function area(): int
    {
        return Int64::from(unsafe(hexagonArea($this->side)));
    }
}

function total(Shape $first, Shape $second, Rectangle $third): int
{
    return $first->area() + $second->area() + $third->area();
}

function main(): void
{
    if (EXPERIMENTAL) {
        let $hexagon = new Hexagon(2);
        echo $hexagon->area();
    }
    echo total(new Square(3), new Rectangle(2, 5), new Strip(7));
}
"#;

#[test]
fn dynamic_dispatch_through_an_interface_reaches_constructed_classes() {
    let mir = program(SHAPES);
    assert!(
        emitted_functions(&mir, "Hexagon").is_empty(),
        "{:?}",
        emitted_functions(&mir, "Hexagon")
    );
    for class in ["Square", "Rectangle", "Strip"] {
        assert!(
            emitted_functions(&mir, class)
                .iter()
                .any(|name| name.ends_with("::area")),
            "{class}: {:?}",
            emitted_functions(&mir, class)
        );
    }
    assert_runs_everywhere(SHAPES, "126");
    let contents = assert_php_prints(SHAPES, "126");
    assert!(!contents.contains("hexagonArea"), "{contents}");
}

/// Reachable code that dispatches through a conformance of a class nothing
/// constructs: iteration, display conversion, equality, hashing and
/// ordering of class-typed values. Naming each class reaches its interface
/// implementations and their witnesses, and nothing else of it.
const NAMED_CONFORMANCES: &str = r#"
class Cursor implements Iterator<int>
{
    writable int $position = 0;
    function hasCurrent(): bool { return $this->position < 2; }
    function getCurrent(): int { return $this->position; }
    writable function advance(): void { $this->position = $this->position + 1; }
}

class Counter implements Iterable<int>
{
    function iterator(): Iterator<int> { return new Cursor(); }
    function unused(): int { return 1; }
}

class Label implements Displayable
{
    function toString(): string { return "label"; }
}

class Key implements Comparable<Key>, Equatable<Key>, Hashable
{
    function __construct(int $value) {}
    function compare(Key $other): Ordering { return Ordering::Equal; }
    function equals(Key $other): bool { return $this->value == $other->value; }
    function hash(): uint64 { return 7; }
}

function total(Counter $counter): int
{
    writable int $sum = 0;
    foreach ($counter as int $value) { $sum += $value; }
    return $sum;
}

function show(Label $label): string { return "{$label}"; }
function same(Key $left, Key $right): bool { return $left == $right; }
function distinct(take Key $key): int
{
    writable Set<Key> $set = Set::from([]);
    let $added = $set->add($key);
    return $set->count;
}
function ordered(take Key $key): int
{
    writable SortedSet<Key> $sorted = SortedSet::from([]);
    let $added = $sorted->add($key);
    return $sorted->count;
}

function main(): void
{
    let $reach = function(Counter $counter, Label $label, Key $left, Key $right,
        take Key $hashed, take Key $sorted): void {
        let $sum = total($counter);
        let $text = show($label);
        let $equal = same($left, $right);
        let $count = distinct($hashed);
        let $sortedCount = ordered($sorted);
    };
    echo "ok";
}
"#;

#[test]
fn naming_a_class_reaches_its_interface_implementations() {
    let mir = program(NAMED_CONFORMANCES);
    for (class, witnesses) in [
        ("Counter", &["::iterator"][..]),
        ("Label", &["::toString"][..]),
        ("Key", &["::compare", "::equals", "::hash"][..]),
    ] {
        let emitted = emitted_functions(&mir, class);
        for witness in witnesses {
            assert!(
                emitted.iter().any(|name| name.ends_with(witness)),
                "{class}{witness}: {emitted:?}"
            );
        }
        assert!(
            !emitted
                .iter()
                .any(|name| name.ends_with("::__construct") || name.ends_with("::unused")),
            "{class}: {emitted:?}"
        );
        let definition = mir
            .classes
            .iter()
            .find(|definition| definition.name == class)
            .unwrap();
        assert!(!definition.constructed && definition.implementations_reached);
    }
    assert_runs_everywhere(NAMED_CONFORMANCES, "ok");
    assert_php_prints(NAMED_CONFORMANCES, "ok");
}

/// PHP output of `source` prints `stdout` when PHP is installed; returns the
/// generated PHP.
fn assert_php_prints(source: &str, stdout: &str) -> String {
    let text = doriac::compile_source_to_php("stage40.doria", source)
        .unwrap_or_else(|errors| panic!("PHP output should build: {errors:#?}"));
    if std::process::Command::new("php")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("PHP unavailable; PHP execution skipped");
        return text;
    }
    let output = php_execution::Script::new(&format!(
        "{}\n__DoriaFunction_6d61696e();",
        text.strip_prefix("<?php").unwrap()
    ))
    .command(&[])
    .output()
    .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), stdout);
    text
}

#[test]
fn foreign_aggregates_are_class_properties() {
    let source = format!(
        "{AGGREGATES}
class Holder
{{
    writable Color $color;
    Pixel $origin = new Pixel(x: 7, y: 8, color: new Color(red: 1, green: 1, blue: 1), weight: 0.5, visible: false);

    function __construct(Color $initial)
    {{
        $this->color = $initial;
    }}

    writable function brighten(uint8 $step): void
    {{
        $this->color->red = $this->color->red + $step;
    }}
}}

function main(): void
{{
    writable Holder $holder = new Holder(new Color(red: 10, green: 20, blue: 30));
    echo $holder->color->red . \" \";
    $holder->brighten(5);
    echo $holder->color->red . \" \";
    let $copy = $holder->color;
    $holder->color = new Color(red: 1, green: 2, blue: 3);
    $holder->color->green += 40;
    echo $copy->red . \" \" . $holder->color->red . \" \" . $holder->color->green . \" \" . $holder->color->alpha . \" \";
    echo $holder->origin->x . \" \" . $holder->origin->color->red;
}}
"
    );
    assert_runs_natively(&source, "10 15 15 1 42 255 7 1");
}

#[test]
fn foreign_aggregate_properties_are_changed_in_place() {
    // Decision 0148 section 6: a union property, a promoted writable
    // property and fixed-length array fields read, indexed, changed and
    // assigned through the object.
    let source = format!(
        "{AGGREGATES}
extern \"C\" struct Tip
{{
    uint16[3] $text;
    int32 $length = 7;
}}

function total(ArrayRegion<uint16> $values): int
{{
    writable int $sum = 0;
    for (writable int $index = 0; $index < $values->length; $index++) {{
        $sum += Int64::from($values[$index]);
    }}
    return $sum;
}}

class Note
{{
    writable Word $word = new Word(whole: 258);

    function __construct(writable Tip $tip) {{}}

    writable function retitle(ArrayRegion<uint16> $text): void
    {{
        $this->tip->text = $text;
    }}
}}

function main(): void
{{
    uint16[] $letters = [72, 105, 33, 9, 8, 7];
    writable Note $note = new Note(new Tip(text: $letters->getRegion(0, 3)));
    echo $note->tip->text[1] . \" \" . $note->tip->length . \" \";
    $note->tip->text[1] = UInt16::from(5);
    $note->tip->text[2] += UInt16::from(1);
    echo total($note->tip->text->getRegion(0, 3)) . \" \";
    $note->retitle($letters->getRegion(3, 3));
    echo $note->tip->text[0] . \" \";
    unsafe {{
        echo $note->word->low . \" \";
    }}
    $note->word->low = 1;
    unsafe {{
        echo $note->word->whole;
    }}
}}
"
    );
    assert_runs_natively(&source, "105 7 111 9 2 257");
}

#[test]
fn foreign_aggregates_are_enum_payloads() {
    // Decision 0148 section 6: a payload field holds the aggregate by
    // value in the enum's storage, through construction, matching, copies
    // and a property of the enum type.
    let source = format!(
        "{AGGREGATES}
enum Paint
{{
    case None;
    case Solid(Color $color);
    case Mixed(Color $first, int32 $weight, Pixel $pixel);
}}

function describe(Paint $paint): string
{{
    return match ($paint) {{
        Paint::None => \"none\",
        Paint::Solid($color) => \"solid {{$color->red}}\",
        Paint::Mixed($first, $weight, $pixel) => \"mixed {{$first->green}} {{$weight}} {{$pixel->color->blue}}\",
    }};
}}

class Canvas
{{
    writable Paint $paint = Paint::None;
}}

function main(): void
{{
    let $color = new Color(red: 10, green: 20, blue: 30);
    echo describe(Paint::None) . \" \";
    let writable $paint = Paint::Solid($color);
    echo describe($paint) . \" \";
    $paint = Paint::Mixed($color, 7, new Pixel(x: 1, y: 2, color: new Color(red: 4, green: 5, blue: 6), weight: 0.5, visible: true));
    echo describe($paint) . \" \";
    writable Canvas $canvas = new Canvas();
    $canvas->paint = $paint;
    $paint = Paint::None;
    echo describe($canvas->paint) . \" \" . describe($paint);
}}
"
    );
    assert_runs_natively(&source, "none solid 10 mixed 20 7 6 mixed 20 7 6 none");
}

#[test]
fn foreign_aggregates_are_static_properties_and_constants() {
    // Decision 0148 section 6: a static property holds a foreign struct or
    // union in its static data, read, written and viewed in place, and the
    // shared constant evaluator builds constant ones from their fields.
    let source = format!(
        "{AGGREGATES}
extern \"C\" struct Tip
{{
    uint16[4] $text;
    uint8 $flags = 7;
}}

extern \"C\" struct Framed
{{
    Color $border = new Color(red: 1, green: 1, blue: 1);
    int32 $width;
}}

enum Paint
{{
    case None;
    case Solid(Color $color);
}}

const Color RED = new Color(red: 255, green: 0, blue: 0);

class Palette
{{
    const Color BLUE = new Color(red: 0, green: 0, blue: 255);
    static Color $primary = new Color(red: 10, green: 20, blue: 30);
    static writable Color $current = new Color(red: 1, green: 2, blue: 3, alpha: 4);
    static Pixel $origin = new Pixel(x: -1, y: 2, color: new Color(red: 5, green: 6, blue: 7), weight: 0.5, visible: true);
    static writable Word $word = new Word(whole: 258);
    static writable Tip $tip = new Tip();
    static Tip $blank = new Tip(flags: 1);
    static Color $copy = self::primary;
    static Paint $paint = Paint::Solid(new Color(red: 9, green: 8, blue: 7));
}}

function describe(Paint $paint): string
{{
    return match ($paint) {{
        Paint::None => \"none\",
        Paint::Solid($color) => \"solid {{$color->red}} {{$color->blue}}\",
    }};
}}

function main(): void
{{
    echo Palette::primary->red . \" \" . Palette::primary->alpha . \" \";
    echo Palette::current->green . \" \";
    Palette::current->green = 9;
    Palette::current->blue += 10;
    Palette::current->red++;
    echo Palette::current->red . \" \" . Palette::current->green . \" \" . Palette::current->blue . \" \" . Palette::current->alpha . \" \";
    let $saved = Palette::current;
    Palette::current = Palette::primary;
    echo Palette::current->red . \" \" . $saved->red . \" \";
    Palette::current = new Color(red: 100, green: 101, blue: 102);
    echo Palette::current->blue . \" \";
    echo Palette::origin->x . \" \" . Palette::origin->color->green . \" \" . Palette::origin->visible . \" \" . Float::toInt(Palette::origin->weight * 4.0) . \" \";
    unsafe {{
        echo Palette::word->low . \" \";
    }}
    Palette::word->low = 1;
    unsafe {{
        echo Palette::word->whole . \" \";
    }}
    echo Palette::tip->flags . \" \" . Palette::tip->text->length . \" \";
    Palette::tip->text[2] = UInt16::from(65);
    echo Palette::tip->text[2] . \" \" . Palette::blank->text[0] . \" \" . Palette::blank->flags . \" \";
    echo Palette::copy->blue . \" \" . RED->red . \" \" . Palette::BLUE->blue . \" \";
    let $framed = new Framed(width: 3);
    echo $framed->border->alpha . \" \" . $framed->width . \" \" . describe(Palette::paint);
}}
"
    );
    assert_runs_natively(
        &source,
        "10 255 2 2 9 13 4 10 2 102 -1 6 true 2 2 257 7 4 65 0 1 30 255 255 255 3 solid 9 7",
    );
}
