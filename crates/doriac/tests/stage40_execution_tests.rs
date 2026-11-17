//! Stage 40 execution (Decision 0148): the unsafe and foreign forms each
//! execution target builds, run through the interpreter and every native
//! profile this compiler build has.

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution_runner.rs"]
mod native_execution;

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

const POINTER: &str = "use Doria\\Std\\Ffi\\{MutPtr, Ptr};\n";

#[test]
fn pointer_addresses_offset_cast_and_compare() {
    let source = format!(
        "{POINTER}
function step(Ptr<int32> $pointer, int $count): Ptr<int32>
{{
    return unsafe($pointer->offset($count));
}}

function main(): void
{{
    unsafe {{
        ?Ptr<int32> $absent = Ptr::fromAddress(0);
        echo $absent == null;
        echo \" \";
        ?Ptr<int32> $maybe = Ptr::fromAddress(4096);
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
            Ptr<uint8> $bytes = Ptr::cast($base);
            echo $bytes->offset(5)->address;
            echo \" \";
            MutPtr<int64> $wide = MutPtr::cast($base);
            echo $wide->offset(1)->address;
            echo \" \";
            Ptr<int64> $readonly = Ptr::from($wide);
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

#[test]
fn an_aligned_access_at_a_misaligned_address_panics_with_p1702() {
    for body in [
        "?Ptr<int32> $pointer = Ptr::fromAddress(4098); if ($pointer != null) { echo $pointer->read(); }",
        "?Ptr<float64> $pointer = Ptr::fromAddress(4100); if ($pointer != null) { MutPtr<float64> $mutable = MutPtr::cast($pointer); $mutable->write(1.5); }",
    ] {
        let source = format!("{POINTER}function main(): void {{ unsafe {{ {body} }} }}");
        assert_panics_natively(&source, "P1702", "Pointer Access Is Misaligned");
    }
}

#[test]
fn an_offset_that_leaves_the_address_space_panics_with_p1703() {
    for body in [
        // Below zero.
        "?Ptr<uint8> $pointer = Ptr::fromAddress(8); if ($pointer != null) { echo $pointer->byteOffset(-16)->address; }",
        // To zero, which no `Ptr<T>` holds.
        "?Ptr<int64> $pointer = Ptr::fromAddress(16); if ($pointer != null) { echo $pointer->offset(-2)->address; }",
        // Past the largest address.
        "?Ptr<uint8> $pointer = Ptr::fromAddress(18446744073709551608); if ($pointer != null) { echo $pointer->byteOffset(8)->address; }",
        // The scaled count overflows before the address moves.
        "?Ptr<int64> $pointer = Ptr::fromAddress(4096); if ($pointer != null) { echo $pointer->offset(2305843009213693952)->address; }",
    ] {
        let source = format!("{POINTER}function main(): void {{ unsafe {{ {body} }} }}");
        assert_panics_natively(&source, "P1703", "Pointer Arithmetic Left The Address Space");
    }
}

#[test]
fn the_interpreter_and_php_output_report_pointer_operations() {
    let source = format!(
        "{POINTER}function main(): void {{ unsafe {{ ?Ptr<int32> $pointer = Ptr::fromAddress(4096); echo $pointer == null; }} }}"
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
        let $pointer = MutPtr::to($value);
        $pointer->write(7);
        echo $pointer->read();
        echo \" \";
        echo $value;
    }}
    echo \" \";
    writable float64 $ratio = 0.0;
    unsafe {{
        let $place = MutPtr::to($ratio);
        $place->writeUnaligned(2.5);
        echo Float::toInt($place->readUnaligned() * 2.0);
    }}
    echo \" \";
    writable bool $flag = false;
    unsafe {{
        MutPtr::to($flag)->write(true);
    }}
    echo $flag;
    echo \" \";
    writable int64 $wide = 0;
    unsafe {{
        MutPtr<uint8> $low = MutPtr::cast(MutPtr::to($wide));
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
        "{POINTER}function bump(writable int32 $count): void {{ unsafe {{ MutPtr::to($count)->write(8); }} echo $count . \" \"; }}
function main(): void {{
    writable int32 $value = 1;
    bump($value);
    echo $value . \" \";
    let $set = function(writable int32 $target): void {{ unsafe {{ MutPtr::to($target)->write(5); }} echo $target . \" \"; }};
    $set($value);
    echo $value . \" \";
    writable int32 $captured = 2;
    writable function writable(): void $raise = function(): void with (writable $captured) {{
        unsafe {{ MutPtr::to($captured)->write(9); }}
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
use Doria\Std\Ffi\{CChar, CLong, CSize, LinkName, MutPtr, Opaque, Ptr};

extern "C"
{
    /** @safety Any value but the least `int32`. */
    function abs(int32 $value): int32;

    /** @safety Any value but the least `CLong`. */
    function labs(CLong $value): CLong;

    /** @safety `$text` points to a NUL-terminated string. */
    function strlen(Ptr<CChar> $text): CSize;

    /** @safety The memory is released once, with `free`. */
    function malloc(CSize $size): ?MutPtr<Opaque>;

    /** @safety `$memory` is null, or came from `malloc` and is not used again. */
    function free(?MutPtr<Opaque> $memory): void;

    /** @safety `$memory` is writable for `$count` bytes. */
    #[LinkName("memset")]
    function fill(MutPtr<Opaque> $memory, int32 $value, CSize $count): MutPtr<Opaque>;

    /** @safety `$memory` is writable for `$count` bytes. */
    #[LinkName("memset")]
    function clear(MutPtr<Opaque> $memory, int32 $value, CSize $count): void;
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
        ?MutPtr<Opaque> $memory = malloc(CSize::from(16));
        if ($memory != null) {{
            let $block = $memory;
            clear($block, 0, CSize::from(16));
            MutPtr<uint8> $bytes = MutPtr::cast($block);
            $bytes->write(UInt8::from(68));
            $bytes->offset(1)->write(UInt8::from(111));
            $bytes->offset(2)->write(UInt8::from(114));
            $bytes->offset(3)->write(UInt8::from(105));
            $bytes->offset(4)->write(UInt8::from(97));
            Ptr<CChar> $text = Ptr::cast(Ptr::from($bytes));
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

#[test]
fn foreign_aggregates_report_where_they_cannot_be_built_yet() {
    for (body, position) in [
        ("class Holder { Color $color; function __construct(Color $initial) { $this->color = $initial; } }", "a property"),
        ("function main(): void { ?Color $maybe = null; }", "a nullable value"),
        ("function keep(Color[] $colors): void {}", "a nullable value"),
        ("enum Paint { case Solid(Color $color); }", "an enum payload"),
        ("function main(): void { let $show = function(Color $color): void {}; }", "a closure parameter"),
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
        let $whole = MutPtr::to($pixel);
        Pixel $copy = $whole->read();
        echo $copy->x . \" \" . $copy->color->blue . \" \";
        writable Pixel $changed = $copy;
        $changed->y = 40;
        $whole->write($changed);
        echo $pixel->y . \" \";
        let $green = MutPtr::to($pixel->color->green);
        $green->write(UInt8::from(77));
        echo $pixel->color->green . \" \";
        let $color = MutPtr::to($pixel->color);
        Color $read = $color->read();
        echo $read->green . \" \" . $whole->read()->color->green . \" \";
        Ptr<Pixel> $base = Ptr::from($whole);
        echo $base->offset(1)->address - $base->address;
        let $y = MutPtr::to($pixel->y);
        $y->writeUnaligned(9);
        echo \" \" . $y->readUnaligned() . \" \" . $pixel->y . \" \";
        MutPtr<uint8> $bytes = MutPtr::cast($whole);
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
        "?Ptr<Sample> $pointer = Ptr::fromAddress(4100); if ($pointer != null) { Sample $sample = $pointer->read(); echo $sample->tag; }",
        "?Ptr<Sample> $pointer = Ptr::fromAddress(4100); if ($pointer != null) { MutPtr<Sample> $mutable = MutPtr::cast($pointer); $mutable->write(new Sample(tag: 1, value: 2, extra: 3)); }",
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
    MutPtr<Opaque> $memory;
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
    function fill(Fill $fill, CSize $count): MutPtr<Opaque>;

    /** @safety Any value. */
    #[LinkName(\"ldexp\")]
    function scale(Scale $value, int32 $exponent): float64;
}}

function main(): void
{{
    writable int64 $target = 0;
    unsafe {{
        MutPtr<Opaque> $memory = MutPtr::cast(MutPtr::to($target));
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
        MutPtr<uint16> $text = MutPtr::cast(MutPtr::to($tip));
        echo $text->read() . \" \" . $text->offset(2)->read() . \" \";
        $text->offset(1)->write(UInt16::from(9));
        Ptr<int32> $length = Ptr::cast(Ptr::from($text)->byteOffset(8));
        echo $length->read() . \" \" . Ptr::from($text)->offset(1)->read();
    }}
}}
"
    );
    // `uint16[3]` takes bytes 0 to 5; `int32` is aligned to byte 8.
    assert_runs_natively(&source, "12 7 0 0 7 9");
}

#[test]
fn a_region_pointer_gives_c_the_storage_itself() {
    let source = format!(
        "{POINTER}use Doria\\Std\\Ffi\\CSize;
use Doria\\Std\\Io\\WritableByteRegion;

extern \"C\"
{{
    /** @safety `$memory` is writable for `$count` bytes. */
    function memset(MutPtr<uint8> $memory, int32 $value, CSize $count): MutPtr<uint8>;
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
