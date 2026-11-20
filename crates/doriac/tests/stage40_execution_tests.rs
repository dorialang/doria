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

#[cfg(unix)]
#[test]
fn c_function_pointer_values_are_called_through_their_address() {
    // Decision 0148 section 12: addresses from `dlsym`, converted with
    // `Ptr::toFunction`, called with scalars and a struct result, held
    // nullable, in a foreign struct field, and passed through a function.
    let source = "
use Doria\\Std\\Ffi\\{CChar, MutPtr, Opaque, Ptr};

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
    function dlopen(?Ptr<CChar> $path, int32 $mode): ?MutPtr<Opaque>;

    /** @safety `$handle` came from `dlopen` and `$name` is NUL-terminated. */
    function dlsym(MutPtr<Opaque> $handle, Ptr<CChar> $name): ?MutPtr<Opaque>;
}

function find(MutPtr<Opaque> $handle, uint8[] $name): Ptr<Opaque>
{
    unsafe {
        ?MutPtr<Opaque> $symbol = dlsym($handle, Ptr::cast($name->getRegion(0, $name->length)->pointer));
        if ($symbol == null) {
            panic(\"no symbol\");
        }
        return Ptr::from($symbol);
    }
}

function apply(extern \"C\" function(int32): int32 $function, int32 $value): int32
{
    return unsafe($function($value));
}

function main(): void
{
    unsafe {
        ?MutPtr<Opaque> $program = dlopen(null, 1);
        if ($program == null) {
            panic(\"no program handle\");
        }
        let $handle = $program;
        extern \"C\" function(int32): int32 $absolute = Ptr::toFunction(find($handle, [97, 98, 115, 0]));
        echo $absolute(-5);
        extern \"C\" function(int32, int32): Quotient $divide = Ptr::toFunction(find($handle, [100, 105, 118, 0]));
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
    // Decision 0148 section 8 over `Ptr<T>[]`.
    let source = format!(
        "{POINTER}
function main(): void
{{
    int32 $first = 4;
    int32 $second = 5;
    unsafe {{
        writable Ptr<int32>[] $pointers = [Ptr::to($first), Ptr::to($first)];
        {{
            writable WritableArrayRegion<Ptr<int32>> $slots = $pointers->getWritableRegion(0, 2);
            $slots[1] = Ptr::to($second);
        }}
        let $view = $pointers->getRegion(0, 2);
        Ptr<int32> $one = $view[0];
        Ptr<int32> $two = $view[1];
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
        Ptr<uint16> $text = $tip->text->pointer;
        Ptr<int32> $length = Ptr::cast($text->byteOffset(8));
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
        Ptr<CChar> $invalid = Ptr::cast($bytes->getRegion(0, 2)->pointer);
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

const SORTING: &str = "use Doria\\Std\\Ffi\\{CSize, MutPtr, Opaque, Ptr};

extern \"C\" function compareIntegers(Ptr<Opaque> $left, Ptr<Opaque> $right): int32
{
    unsafe {
        Ptr<int32> $first = Ptr::cast($left);
        Ptr<int32> $second = Ptr::cast($right);
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
    function qsort(MutPtr<Opaque> $base, CSize $count, CSize $size,
        extern \"C\" function(Ptr<Opaque>, Ptr<Opaque>): int32 $compare): void;
}
";

#[test]
fn c_calls_a_c_abi_function_through_its_name_as_a_value() {
    // Decision 0148 section 12: the name of a C-ABI function is the
    // address of its C entry, which C calls under the C convention.
    let source = format!(
        "{SORTING}
function main(): void
{{
    writable int32[] $values = [5, -3, 9, 1, 7, 1];
    unsafe {{
        writable WritableArrayRegion<int32> $region = $values->getWritableRegion(0, 6);
        qsort(MutPtr::cast($region->writablePointer), CSize::from(6), CSize::from(4), compareIntegers);
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
    // Decision 0148 section 12 (amendment of 2026-11-20): a C-ABI function
    // whose `echo` sits inside `try` with a `catch` for its I/O error lets
    // no checked error reach C, so its C entry uses the ordinary call.
    let source = "use Doria\\Std\\Ffi\\{CSize, MutPtr, Opaque, Ptr};

extern \"C\" function compareLoudly(Ptr<Opaque> $left, Ptr<Opaque> $right): int32
{
    try {
        echo \".\";
    } catch (Doria\\Std\\Io\\IoError $error) {
        return 0;
    }
    unsafe {
        Ptr<int32> $first = Ptr::cast($left);
        Ptr<int32> $second = Ptr::cast($right);
        return $first->read() - $second->read();
    }
}

extern \"C\"
{
    /** @safety `$base` holds `$count` elements of `$size` bytes each. */
    function qsort(MutPtr<Opaque> $base, CSize $count, CSize $size,
        extern \"C\" function(Ptr<Opaque>, Ptr<Opaque>): int32 $compare): void;
}

function main(): void
{
    writable int32[] $values = [3, 1, 2];
    unsafe {
        writable WritableArrayRegion<int32> $region = $values->getWritableRegion(0, 3);
        qsort(MutPtr::cast($region->writablePointer), CSize::from(3), CSize::from(4), compareLoudly);
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
fn a_c_entry_receives_and_returns_every_c_form() {
    // A C function pointer value holding a C entry's address calls it
    // under the C convention, so each value crosses the boundary twice:
    // integers of every width, `bool`, `float32`, `float64`, pointers
    // present and absent, and a struct in and out by value.
    let source = "use Doria\\Std\\Ffi\\{CLong, MutPtr, Opaque, Ptr};

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
    ?Ptr<Opaque> $address, CLong $count): int64
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

extern \"C\" function absent(?Ptr<Opaque> $address): ?Ptr<Opaque>
{
    return $address;
}

function main(): void
{
    unsafe {
        extern \"C\" function(int8, uint16, bool, float32, ?Ptr<Opaque>, CLong): int64 $describe = describe;
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
        extern \"C\" function(?Ptr<Opaque>): ?Ptr<Opaque> $absent = absent;
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
extern \"C\" function refuse(Ptr<Opaque> $left, Ptr<Opaque> $right): int32
{{
    panic(\"no comparisons\");
}}

function sort(writable int32[] $values): void
{{
    unsafe {{
        writable WritableArrayRegion<int32> $region = $values->getWritableRegion(0, 2);
        qsort(MutPtr::cast($region->writablePointer), CSize::from(2), CSize::from(4), refuse);
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
    // Decision 0148 section 13: C may call a C-ABI function only on the
    // thread running Doria code.
    let source = "use Doria\\Std\\Ffi\\{MutPtr, Opaque, Ptr};

extern \"C\" function work(?MutPtr<Opaque> $context): ?MutPtr<Opaque>
{
    return $context;
}

extern \"C\"
{
    /** @safety `$thread` is writable for one `pthread_t`. */
    function pthread_create(MutPtr<uint64> $thread, ?Ptr<Opaque> $attributes,
        extern \"C\" function(?MutPtr<Opaque>): ?MutPtr<Opaque> $start,
        ?MutPtr<Opaque> $context): int32;

    /** @safety `$thread` came from `pthread_create` and was not joined. */
    function pthread_join(uint64 $thread, ?MutPtr<Opaque> $result): int32;
}

function main(): void
{
    writable uint64 $thread = 0;
    unsafe {
        if (pthread_create(MutPtr::to($thread), null, work, null) == 0) {
            pthread_join($thread, null);
        }
    }
    echo \"joined\";
}
";
    assert_panics_natively(source, "P1706", "C Called Doria From Another Thread");
}
