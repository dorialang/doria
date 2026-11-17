//! Stage 40 checking (Decision 0148): unsafe contexts, foreign declarations,
//! C-ABI functions, C function pointer types and their diagnostics.

use doriac::diagnostics::{Diagnostic, DiagnosticSeverity};

/// Every finding of checking `source`: its errors, or its warnings when it
/// has no error.
fn findings(source: &str) -> Vec<Diagnostic> {
    match doriac::check_source_with_warnings("ffi.doria", source) {
        Ok((_, warnings)) => warnings,
        Err(diagnostics) => diagnostics,
    }
}

fn codes(source: &str) -> Vec<&'static str> {
    findings(source)
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn only(source: &str, code: &str) -> Diagnostic {
    let found = findings(source);
    assert!(
        found.iter().all(|diagnostic| diagnostic.code == code),
        "expected only {code}: {found:#?}"
    );
    found
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("expected {code}, found nothing"))
}

const ABS: &str = r#"
extern "C"
{
    /** @safety Any value is valid. */
    function abs(int32 $value): int32;
}
"#;

#[test]
fn a_foreign_call_needs_an_unsafe_context() {
    let outside = format!("{ABS}function main(): void {{ echo abs(-3); }}");
    let diagnostic = only(&outside, "E0785");
    assert_eq!(
        diagnostic.title,
        "Unsafe Operation Outside An Unsafe Context"
    );
    assert!(diagnostic.message.contains("calling `abs`"));

    for body in [
        "unsafe { echo abs(-3); }",
        "let $value = unsafe(abs(-3)); echo $value;",
        "unsafe { let $value = abs(-3); if ($value > 0) { echo abs($value); } }",
    ] {
        let source = format!("{ABS}function main(): void {{ {body} }}");
        assert!(
            codes(&source).is_empty(),
            "{body}: {:#?}",
            findings(&source)
        );
    }
}

#[test]
fn an_unsafe_context_stops_at_a_closure_body() {
    let source = format!(
        "{ABS}function main(): void {{ unsafe {{ let $call = fn(int32 $value) => abs($value); echo $call(-2); }} }}"
    );
    let found = findings(&source);
    let unsafe_call = found
        .iter()
        .find(|diagnostic| diagnostic.code == "E0785")
        .unwrap_or_else(|| panic!("closure call needs its own context: {found:#?}"));
    assert!(unsafe_call
        .notes
        .iter()
        .any(|note| note.contains("does not extend into a closure body")));
    let inner = format!(
        "{ABS}function main(): void {{ let $call = fn(int32 $value) => unsafe(abs($value)); echo $call(-2); }}"
    );
    assert!(codes(&inner).is_empty(), "{:#?}", findings(&inner));
}

#[test]
fn unused_and_nested_unsafe_contexts_warn() {
    let unused = only("function main(): void { unsafe { echo 1; } }", "E0797");
    assert_eq!(unused.severity, DiagnosticSeverity::Warning);
    assert_eq!(unused.title, "Unused Unsafe Context");

    let nested = format!("{ABS}function main(): void {{ unsafe {{ echo unsafe(abs(-1)); }} }}");
    let found = findings(&nested);
    assert_eq!(
        found
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        ["E0797"],
        "{found:#?}"
    );
    assert!(found[0].message.contains("inside another unsafe context"));
}

#[test]
fn unsafe_functions_and_methods_need_an_unsafe_context_at_every_call() {
    let source = r#"
interface Device
{
    /** @safety The device is open. */
    unsafe function reset(): void;
}

class Port implements Device
{
    function reset(): void {}

    /** @safety The port is open. */
    unsafe function flush(): void {}

    /** @safety Any address. */
    static unsafe function at(int $address): Port
    {
        return new Port();
    }
}

/** @safety Any value. */
unsafe function prepare(int $value): int
{
    return $value;
}

function restart(Device $device): void
{
    $device->reset();
}

function main(): void
{
    let $port = new Port();
    $port->reset();
    $port->flush();
    let $other = Port::at(1);
    echo prepare(3);
    unsafe {
        $port->flush();
        echo prepare(4);
    }
}
"#;
    let found = findings(source);
    let unsafe_calls = found
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0785")
        .count();
    assert_eq!(
        unsafe_calls, 4,
        "the interface call, `flush`, `Port::at` and `prepare` outside a context: {found:#?}"
    );
    assert!(
        found.iter().all(|diagnostic| diagnostic.code == "E0785"),
        "a safe implementation of an unsafe requirement is valid: {found:#?}"
    );
}

#[test]
fn an_unsafe_function_body_is_an_ordinary_context() {
    let source = format!(
        "{ABS}/** @safety None. */ unsafe function wrapper(): int32 {{ return abs(-1); }} function main(): void {{ echo unsafe(wrapper()); }}"
    );
    only(&source, "E0785");
}

#[test]
fn an_override_cannot_add_unsafe() {
    let source = r#"
open class Base
{
    open function reset(): void {}
}

class Derived extends Base
{
    /** @safety Never. */
    override unsafe function reset(): void {}
}

function main(): void {}
"#;
    let diagnostic = only(source, "E0794");
    assert_eq!(diagnostic.title, "Override Cannot Add Unsafe");

    let dropping = r#"
open class Base
{
    /** @safety Never. */
    open unsafe function reset(): void {}
}

class Derived extends Base
{
    override function reset(): void {}
}

function main(): void {}
"#;
    assert!(codes(dropping).is_empty(), "{:#?}", findings(dropping));
}

#[test]
fn foreign_signatures_use_ffi_safe_types() {
    let source = r#"
extern "C"
{
    /** @safety Any text. */
    function puts(string $text): int32;
}

function main(): void {}
"#;
    let diagnostic = only(source, "E0786");
    assert_eq!(diagnostic.title, "Type Cannot Cross The C Boundary");
    assert!(
        diagnostic.helps.iter().any(|help| help.contains("CString")),
        "{diagnostic:#?}"
    );

    let safe = r#"
use Doria\Std\Ffi\{CChar, CLong, CSize, MutPtr, Opaque, Ptr};

extern "C"
{
    /** @safety Pointers are valid for the call. */
    function copy(MutPtr<uint8> $destination, Ptr<uint8> $source, CSize $length): ?MutPtr<Opaque>;

    /** @safety Pointers are valid for the call. */
    function length(Ptr<CChar> $text): CLong;

    /** @safety Pointers are valid for the call. */
    function sort(MutPtr<Opaque> $base, CSize $count, extern "C" function(Ptr<Opaque>, Ptr<Opaque>): int32 $compare): void;
}

function main(): void {}
"#;
    assert!(codes(safe).is_empty(), "{:#?}", findings(safe));

    let pointee =
        "use Doria\\Std\\Ffi\\Ptr; function main(): void { ?Ptr<string> $pointer = null; }";
    only(pointee, "E0786");
}

#[test]
fn opaque_is_only_a_pointee() {
    let diagnostic = only(
        "use Doria\\Std\\Ffi\\Opaque; function main(): void { Opaque $value = 0; }",
        "E0787",
    );
    assert_eq!(diagnostic.title, "Foreign Type Is Not Valid Here");
}

#[test]
fn c_is_the_only_abi() {
    let diagnostic = only(
        "extern \"system\" { /** @safety None. */ function tick(): void; } function main(): void {}",
        "E0788",
    );
    assert_eq!(diagnostic.fixes.len(), 1);
}

#[test]
fn foreign_signatures_have_no_effects_modes_or_type_parameters() {
    for (body, title) in [
        (
            "class Failure implements Error { function __construct(string $message) {} } extern \"C\" { /** @safety None. */ function tick(): void throws Failure; }",
            "C Function Cannot Throw",
        ),
        (
            "extern \"C\" { /** @safety None. */ function fill(writable int32 $value): void; }",
            "Foreign Parameter Has A Mode",
        ),
        (
            "extern \"C\" function compare<T>(int32 $value): int32 { return $value; }",
            "Foreign Function Cannot Be Generic",
        ),
        (
            "class Failure implements Error { function __construct(string $message) {} } extern \"C\" function callback(int32 $value): int32 throws Failure { return $value; }",
            "C Function Cannot Throw",
        ),
    ] {
        let source = format!("{body} function main(): void {{}}");
        let found = findings(&source);
        assert!(
            found
                .iter()
                .any(|diagnostic| diagnostic.code == "E0789" && diagnostic.title == title),
            "{body}: {found:#?}"
        );
    }
}

#[test]
fn foreign_symbols_follow_the_link_name_rules() {
    let printf = only(
        "use Doria\\Std\\Ffi\\{CChar, Ptr}; extern \"C\" { /** @safety None. */ function printf(Ptr<CChar> $format): int32; } function main(): void {}",
        "E0791",
    );
    assert!(printf
        .helps
        .iter()
        .any(|help| help.contains("#[LinkName(\"printf\")]")));

    only(
        "extern \"C\" { /** @safety None. */ function dr_v1_flush(): void; } function main(): void {}",
        "E0792",
    );

    let conflict = r#"
use Doria\Std\Ffi\LinkName;

extern "C"
{
    /** @safety Any value. */
    function abs(int32 $value): int32;

    /** @safety Any value. */
    #[LinkName("abs")]
    function absolute(int64 $value): int64;
}

function main(): void {}
"#;
    only(conflict, "E0790");

    let linked = r#"
use Doria\Std\Ffi\{CChar, LinkName, Opaque, Ptr};

extern "C"
{
    /** @safety Any value. */
    #[LinkName("objc_msgSend")]
    function sendNoArguments(Ptr<Opaque> $receiver, Ptr<Opaque> $selector): Ptr<Opaque>;

    /** @safety Any value. */
    #[LinkName("objc_msgSend")]
    function sendText(Ptr<Opaque> $receiver, Ptr<Opaque> $selector, Ptr<CChar> $text): Ptr<Opaque>;
}

function main(): void {}
"#;
    assert!(codes(linked).is_empty(), "{:#?}", findings(linked));
}

#[test]
fn variadic_bindings_declare_promoted_types() {
    let promoted = r#"
use Doria\Std\Ffi\{CChar, LinkName, Ptr, Variadic};

extern "C"
{
    /** @safety `$format` is a NUL-terminated format. */
    #[LinkName("printf")]
    #[Variadic(fixedParameters: 1)]
    function printSmall(Ptr<CChar> $format, int16 $value, float32 $ratio): int32;
}

function main(): void {}
"#;
    let found = findings(promoted);
    assert_eq!(
        found
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        ["E0793", "E0793"],
        "{found:#?}"
    );
    assert!(found.iter().all(|diagnostic| diagnostic.fixes.len() == 1));

    let range = r#"
use Doria\Std\Ffi\{CChar, LinkName, Ptr, Variadic};

extern "C"
{
    /** @safety `$format` is a NUL-terminated format. */
    #[LinkName("printf")]
    #[Variadic(fixedParameters: 3)]
    function printOne(Ptr<CChar> $format, int32 $value): int32;
}

function main(): void {}
"#;
    let diagnostic = only(range, "E0793");
    assert_eq!(
        diagnostic.title,
        "Variadic Fixed Parameter Count Is Out Of Range"
    );
}

#[test]
fn foreign_attributes_apply_only_to_their_targets() {
    let library = only(
        "use Doria\\Std\\Ffi\\Library; #[Library(\"zlib\")] function main(): void {}",
        "E0795",
    );
    assert_eq!(library.title, "Foreign Attribute Is Not Valid Here");
    let link_name = only(
        "use Doria\\Std\\Ffi\\LinkName; #[LinkName(\"abs\")] extern \"C\" { /** @safety None. */ function abs(int32 $value): int32; } function main(): void {}",
        "E0795",
    );
    assert!(link_name.message.contains("LinkName"));
    let placed = r#"
use Doria\Std\Ffi\{Library, NonBlocking};

#[Library("m")]
extern "C"
{
    /** @safety Any value. */
    #[NonBlocking]
    function cbrt(float64 $value): float64;
}

function main(): void {}
"#;
    assert!(codes(placed).is_empty(), "{:#?}", findings(placed));
}

#[test]
fn unsafe_and_foreign_declarations_state_their_safety_sections() {
    let missing = r#"
extern "C"
{
    function abs(int32 $value): int32;
}

unsafe function release(int $address): void {}

class Port
{
    unsafe function flush(): void {}
}

function main(): void {}
"#;
    let found = findings(missing);
    assert_eq!(
        found
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        ["E0796", "E0796", "E0796"],
        "{found:#?}"
    );
    assert!(found
        .iter()
        .all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning));

    let shared = r#"
/** @safety Every pointer argument stays valid for the call. */
extern "C"
{
    function abs(int32 $value): int32;
    function labs(int64 $value): int64;
}

function main(): void {}
"#;
    assert!(codes(shared).is_empty(), "{:#?}", findings(shared));
}

#[test]
fn calling_a_c_function_pointer_needs_an_unsafe_context() {
    let source = r#"
use Doria\Std\Ffi\Opaque;

function invoke(extern "C" function(int32): int32 $callback, ?extern "C" function(int32): int32 $maybe): void
{
    echo $callback(1);
    echo unsafe($callback(2));
    if ($maybe != null) {
        echo unsafe($maybe(3));
    }
}

function main(): void {}
"#;
    let found = findings(source);
    assert_eq!(
        found
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        ["E0785"],
        "{found:#?}"
    );
    assert!(found[0].message.contains("C function pointer"));
}

#[test]
fn foreign_calls_build_natively_and_report_what_cannot_be_built_yet() {
    let source = format!("{ABS}function main(): void {{ echo unsafe(abs(-3)); }}");
    assert!(codes(&source).is_empty(), "{:#?}", findings(&source));
    let diagnostics = doriac::compile_source(
        "ffi.doria",
        source.clone(),
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("the interpreter calls C through the native companion");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Foreign Calls Cannot Run In The Interpreter Yet"),
        "{diagnostics:#?}"
    );
    let diagnostics =
        doriac::compile_source("ffi.doria", source, doriac::backend::BackendTarget::Php)
            .expect_err("PHP output calls C through the native companion");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Foreign Calls Cannot Be Built For PHP Output Yet"),
        "{diagnostics:#?}"
    );

    for (declaration, title) in [
        (
            "#[Library(\"zlib\")] extern \"C\" { /** @safety Any value. */ function zlibVersion(): Ptr<CChar>; }",
            "Native Libraries Cannot Be Linked Yet",
        ),
        (
            "extern \"C\" { /** @safety Any value. */ #[LinkName(\"printf\")] #[Variadic(fixedParameters: 1)] function printInteger(Ptr<CChar> $format, int32 $value): int32; }",
            "Variadic Foreign Functions Cannot Be Built Yet",
        ),
        (
            "extern \"C\" { /** @safety Any value. */ #[CaptureSystemCode(SystemCodeSource::Errno)] function close(int32 $descriptor): int32; }",
            "System Code Capture Cannot Be Built Yet",
        ),
    ] {
        let source = format!(
            "use Doria\\Std\\Ffi\\{{CaptureSystemCode, CChar, Library, LinkName, Ptr, SystemCodeSource, Variadic}};\n{declaration}\nfunction main(): void {{}}"
        );
        assert!(codes(&source).is_empty(), "{title}: {:#?}", findings(&source));
        let diagnostics = doriac::lower_source_to_mir("ffi.doria", source)
            .expect_err("the binding cannot be built yet");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0784" && diagnostic.title == title),
            "{title}: {diagnostics:#?}"
        );
    }

    let unsafe_only = "/** @safety None. */ unsafe function tick(): int { return 1; } function main(): void { echo unsafe(tick()); }";
    let output = doriac::compile_source_to_debug("tick.doria", unsafe_only)
        .expect("an unsafe function with no foreign code builds");
    assert!(!output.is_empty());
}

const COLOR: &str = r#"
extern "C" struct Color
{
    uint8 $red;
    uint8 $green;
    uint8 $blue;
    uint8 $alpha = 255;
}

extern "C" union Value
{
    int32 $number;
    float32 $ratio;
    Color $color;
}

extern "C" struct Stream;
"#;

#[test]
fn foreign_structs_are_built_from_every_field_without_a_default() {
    for body in [
        "let $color = new Color(red: 1, green: 2, blue: 3); echo $color->alpha;",
        "let $color = new Color(1, 2, 3, 4); echo $color->red;",
        "writable Color $color = new Color(red: 1, green: 2, blue: 3); $color->alpha = 9; echo $color->alpha;",
        "let $size = Color::SIZE; let $alignment = Color::ALIGNMENT; echo $size + $alignment;",
    ] {
        let source = format!("{COLOR}function main(): void {{ {body} }}");
        assert!(codes(&source).is_empty(), "{body}: {:#?}", findings(&source));
    }
    let missing = format!("{COLOR}function main(): void {{ let $color = new Color(red: 1); }}");
    assert!(!codes(&missing).is_empty(), "missing fields are reported");
    let readonly = format!(
        "{COLOR}function main(): void {{ let $color = new Color(red: 1, green: 2, blue: 3); $color->red = 4; }}"
    );
    only(&readonly, "E0479");
}

#[test]
fn a_union_is_built_zeroed_or_from_one_named_member() {
    for body in [
        "let $value = new Value();",
        "let $value = new Value(number: 3);",
        "writable Value $value = new Value(); $value->ratio = 1.5;",
        "let $value = new Value(number: 3); echo unsafe($value->number);",
    ] {
        let source = format!("{COLOR}function main(): void {{ {body} }}");
        assert!(
            codes(&source).is_empty(),
            "{body}: {:#?}",
            findings(&source)
        );
    }
    for body in [
        "let $value = new Value(number: 3, ratio: 1.5);",
        "let $value = new Value(3);",
    ] {
        let source = format!("{COLOR}function main(): void {{ {body} }}");
        let diagnostic = only(&source, "E0799");
        assert_eq!(diagnostic.title, "Union Construction Names One Member");
    }
    let read = format!(
        "{COLOR}function main(): void {{ let $value = new Value(number: 3); echo $value->number; }}"
    );
    let diagnostic = only(&read, "E0785");
    assert!(diagnostic.message.contains("union member `$number`"));
}

#[test]
fn foreign_aggregates_have_fields_and_constants_only() {
    for (body, title) in [
        (
            "let $color = new Color(red: 1, green: 2, blue: 3); echo $color->hue;",
            "Foreign Aggregate Has No Such Field",
        ),
        (
            "let $color = new Color(red: 1, green: 2, blue: 3); $color->blend();",
            "Foreign Aggregate Has No Methods",
        ),
        (
            "echo Color::COUNT;",
            "Foreign Aggregate Has No Such Constant",
        ),
    ] {
        let source = format!("{COLOR}function main(): void {{ {body} }}");
        let diagnostic = only(&source, "E0798");
        assert_eq!(diagnostic.title, title, "{body}");
    }
    for (declaration, title) in [
        (
            "extern \"C\" struct Pair { int32 $left; int32 $left; }",
            "Foreign Field Is Declared Twice",
        ),
        (
            "extern \"C\" union Either { int32 $left = 1; float32 $right; }",
            "Union Member Cannot Have A Default",
        ),
        (
            "extern \"C\" struct Empty { }",
            "Foreign Aggregate Has No Fields",
        ),
    ] {
        let source = format!("{declaration} function main(): void {{}}");
        let diagnostic = only(&source, "E0798");
        assert_eq!(diagnostic.title, title, "{declaration}");
    }
    let unsafe_field = "extern \"C\" struct Named { string $name; } function main(): void {}";
    only(unsafe_field, "E0786");
    let recursive =
        "extern \"C\" struct Node { int32 $value; Node $next; } function main(): void {}";
    only(recursive, "E0786");
}

#[test]
fn an_incomplete_struct_is_used_only_through_a_pointer() {
    let pointer = format!(
        "use Doria\\Std\\Ffi\\MutPtr; {COLOR} extern \"C\" {{ /** @safety Valid streams. */ function finish(MutPtr<Stream> $stream): int32; }} function main(): void {{}}"
    );
    assert!(codes(&pointer).is_empty(), "{:#?}", findings(&pointer));
    let by_value =
        format!("{COLOR}function consume(Stream $stream): void {{}} function main(): void {{}}");
    let diagnostic = only(&by_value, "E0787");
    assert!(diagnostic
        .helps
        .iter()
        .any(|help| help.contains("Ptr<Stream>")));
}

#[test]
fn foreign_aggregates_cross_the_c_boundary_by_value_and_by_pointer() {
    let source = format!(
        "use Doria\\Std\\Ffi\\Ptr; {COLOR} extern \"C\" {{ /** @safety Any color. */ function blend(Color $left, Ptr<Color> $right): Color; }} function main(): void {{}}"
    );
    assert!(codes(&source).is_empty(), "{:#?}", findings(&source));
}

#[test]
fn fixed_length_array_fields_are_zero_filled_until_regions_land() {
    let declaration = "extern \"C\" struct Tip { uint16[128] $text; int32 $length; }";
    let built = format!("{declaration} function main(): void {{ let $tip = new Tip(length: 0); echo $tip->length; }}");
    assert!(codes(&built).is_empty(), "{:#?}", findings(&built));
    let read = format!("{declaration} function main(): void {{ let $tip = new Tip(length: 0); let $text = $tip->text; }}");
    let diagnostic = only(&read, "E0784");
    assert_eq!(
        diagnostic.title,
        "Fixed-Length Array Fields Cannot Be Read Yet"
    );
}

const POINTERS: &str = r#"
use Doria\Std\Ffi\{CChar, CLong, CSize, MutPtr, Opaque, Ptr};

extern "C" struct Point
{
    int32 $x;
    int32 $y;
}

extern "C" struct Handle;

extern "C"
{
    /** @safety Any size. */
    function allocate(CSize $size): MutPtr<Opaque>;
    /** @safety A handle from `openHandle`. */
    function openHandle(): MutPtr<Handle>;
    /** @safety A valid callback. */
    function find(Ptr<CChar> $name): Ptr<Opaque>;
}
"#;

fn pointer_program(body: &str) -> String {
    format!("{POINTERS}function main(): void {{ writable int32 $value = 3; writable Point $point = new Point(x: 1, y: 2); {body} }}")
}

/// `pointer_program` with a class holding an FFI-safe property and helpers
/// that read, change and take values.
fn pin_program(body: &str) -> String {
    format!(
        "class Box {{ writable int32 $count = 0; writable int32 $other = 0; writable function reset(): void {{ $this->count = 0; }} }} function consume(take Box $box): void {{}} function show(Point $point): void {{ echo $point->x; }} function shift(writable Point $point): void {{ $point->x = 9; }} {}",
        pointer_program(body)
    )
}

#[test]
fn pointer_members_have_their_types() {
    for body in [
        "unsafe { let $pointer = MutPtr::to($value); $pointer->write(7); int32 $read = $pointer->read(); echo $read; }",
        "unsafe { let $pointer = MutPtr::to($value); $pointer->writeUnaligned(7); int32 $read = $pointer->readUnaligned(); echo $read; }",
        "unsafe { let $pointer = Ptr::to($value); Ptr<int32> $next = $pointer->offset(1); Ptr<int32> $byte = $pointer->byteOffset(4); echo $next == $byte; }",
        "unsafe { let $field = MutPtr::to($point->y); $field->write(5); } unsafe { Point $copy = Ptr::to($point)->read(); echo $copy->x; }",
        "unsafe { let $pointer = MutPtr::to($value); Ptr<uint8> $bytes = Ptr::cast($pointer); MutPtr<uint8> $writable = MutPtr::cast($pointer); echo $bytes->address == $writable->address; }",
        "unsafe { ?Ptr<int64> $absent = Ptr::fromAddress(0); echo $absent == null; }",
        "unsafe { let $address = find(Ptr::cast(Ptr::to($value))); extern \"C\" function(int32): int32 $callback = Ptr::toFunction($address); echo $callback(3); }",
        "unsafe { let $memory = allocate(CSize::from(8)); MutPtr<uint8> $bytes = MutPtr::cast($memory); let $skipped = $memory->byteOffset(4); echo $bytes->address < $skipped->address; }",
        "let $readonly = Ptr::from(unsafe(MutPtr::to($value))); echo $readonly->address;",
        "unsafe { let $pointer = MutPtr::to($value); let $same = MutPtr::to($value); echo $pointer == $same; }",
    ] {
        let source = pointer_program(body);
        assert!(codes(&source).is_empty(), "{body}: {:#?}", findings(&source));
    }
}

#[test]
fn pointer_operations_need_an_unsafe_context() {
    for (body, operation) in [
        ("let $pointer = Ptr::to($value);", "forming a pointer"),
        (
            "let $pointer = unsafe(Ptr::to($value)); echo $pointer->read();",
            "reading through a pointer",
        ),
        (
            "let $pointer = unsafe(MutPtr::to($value)); $pointer->write(1);",
            "writing through a pointer",
        ),
        (
            "let $pointer = unsafe(Ptr::to($value)); let $next = $pointer->offset(1);",
            "offsetting a pointer",
        ),
        (
            "let $pointer = unsafe(Ptr::to($value)); Ptr<uint8> $bytes = Ptr::cast($pointer);",
            "converting a pointer",
        ),
        (
            "?Ptr<int32> $pointer = Ptr::fromAddress(16);",
            "converting an address to a pointer",
        ),
    ] {
        let diagnostic = only(&pointer_program(body), "E0785");
        assert!(
            diagnostic.message.contains(operation),
            "{body}: {diagnostic:#?}"
        );
    }
    let safe = pointer_program(
        "let $pointer = unsafe(MutPtr::to($value)); echo $pointer->address; let $readonly = Ptr::from($pointer); echo $readonly == Ptr::from($pointer);",
    );
    assert!(codes(&safe).is_empty(), "{:#?}", findings(&safe));
}

#[test]
fn reading_writing_and_offsetting_need_a_pointee_with_a_size() {
    for body in [
        "unsafe { let $memory = allocate(CSize::from(8)); echo $memory->read(); }",
        "unsafe { let $memory = allocate(CSize::from(8)); let $next = $memory->offset(1); }",
        "unsafe { let $handle = openHandle(); let $copy = $handle->readUnaligned(); }",
    ] {
        let diagnostic = only(&pointer_program(body), "E0800");
        assert_eq!(
            diagnostic.title,
            "Pointer Operation Needs A Pointee With A Size"
        );
    }
    let bytes = pointer_program(
        "unsafe { let $handle = openHandle(); let $next = $handle->byteOffset(16); echo $next->address; }",
    );
    assert!(codes(&bytes).is_empty(), "{:#?}", findings(&bytes));
}

#[test]
fn a_pointer_addresses_a_place_of_an_ffi_safe_type() {
    for body in [
        "unsafe { let $pointer = Ptr::to(3); }",
        "unsafe { let $pointer = Ptr::to(Int32::from(3)); }",
    ] {
        let diagnostic = only(&pointer_program(body), "E0801");
        assert_eq!(diagnostic.title, "Pointer Source Is Not A Place");
    }
    let text = pointer_program("let $text = \"zlib\"; unsafe { let $pointer = Ptr::to($text); }");
    assert!(codes(&text).contains(&"E0786"), "{:#?}", findings(&text));
}

#[test]
fn a_conversion_takes_its_target_from_the_expected_type() {
    for body in [
        "unsafe { let $pointer = MutPtr::to($value); let $bytes = Ptr::cast($pointer); }",
        "unsafe { let $pointer = MutPtr::to($value); int $number = Ptr::cast($pointer); }",
        "unsafe { let $address = Ptr::fromAddress(16); }",
        "unsafe { let $function = Ptr::toFunction(find(Ptr::cast(Ptr::to($value)))); }",
    ] {
        let diagnostic = only(&pointer_program(body), "E0802");
        assert_eq!(diagnostic.title, "Pointer Conversion Needs A Target Type");
    }
}

#[test]
fn pointers_have_only_their_own_members() {
    let write = only(
        &pointer_program("unsafe { let $pointer = Ptr::to($value); $pointer->write(1); }"),
        "E0304",
    );
    assert!(
        write.helps.iter().any(|help| help.contains("MutPtr<T>")),
        "{write:#?}"
    );
    let property = only(
        &pointer_program("let $pointer = unsafe(Ptr::to($value)); echo $pointer->adress;"),
        "E0303",
    );
    assert_eq!(property.title, "Unknown Property");
    let ordering = only(
        &pointer_program(
            "let $pointer = unsafe(Ptr::to($value)); let $less = $pointer < $pointer;",
        ),
        "E0441",
    );
    assert_eq!(
        ordering.title,
        "Foreign Value Has No Arithmetic Or Ordering"
    );
}

#[test]
fn a_pin_stops_moves_and_replacement_but_not_field_access() {
    // Decision 0148 section 7, amended 2026-11-16: "Pins stop moves, not
    // field access".
    let conflicting = [
        "unsafe { let $pointer = Ptr::to($value); $value = 4; }",
        "unsafe { let $pointer = MutPtr::to($value); $value++; }",
        "unsafe { let $pointer = Ptr::to($point); $point = new Point(x: 3, y: 4); }",
        "unsafe { let $field = MutPtr::to($point->y); $point = new Point(x: 3, y: 4); }",
        "let writable $box = new Box(); unsafe { let $field = Ptr::to($box->count); $box->reset(); }",
        "let writable $box = new Box(); unsafe { let $field = MutPtr::to($box->count); $box->count = 2; }",
        "let writable $box = new Box(); unsafe { let $field = Ptr::to($box->count); consume($box); }",
    ];
    for body in conflicting {
        let found = findings(&pin_program(body));
        assert!(
            found.len() == 1 && found[0].code == "E0477",
            "{body}: {found:#?}"
        );
        let diagnostic = found.into_iter().next().expect("one finding");
        assert_eq!(
            diagnostic.title, "Pinned Value Cannot Be Moved Or Replaced",
            "{body}"
        );
        assert!(
            diagnostic
                .related
                .iter()
                .any(|related| related.message == "Pointer Formed Here"),
            "{body}: {diagnostic:#?}"
        );
    }
    for body in [
        "unsafe { let $pointer = MutPtr::to($value); $pointer->write(4); } $value = 5; echo $value;",
        "unsafe { let $pointer = MutPtr::to($value); let $again = MutPtr::to($value); let $readonly = Ptr::to($value); }",
        "unsafe { let $pointer = Ptr::to($value); echo $value; }",
        "unsafe { let $pointer = MutPtr::to($value); echo $value; }",
        "unsafe { let $pointer = Ptr::to($value); let $writable = MutPtr::to($value); }",
        "writable int $round = 0; unsafe { while ($round < 2) { $round++; echo $value; let $pointer = MutPtr::to($value); } }",
        "unsafe { let $pointer = MutPtr::to($point); $point->x = 5; echo $point->y; $pointer->write(new Point(x: 1, y: 1)); echo $point->x; }",
        "unsafe { let $field = MutPtr::to($point->y); $point->x = 3; echo $point->y; }",
        "unsafe { let $pointer = Ptr::to($point); show($point); }",
        // A `writable` parameter of a Copy type receives its own copy.
        "unsafe { let $pointer = Ptr::to($point); shift($point); }",
        "let writable $box = new Box(); unsafe { let $field = MutPtr::to($box->count); $box->other = 2; echo $box->count; }",
    ] {
        let source = pin_program(body);
        assert!(codes(&source).is_empty(), "{body}: {:#?}", findings(&source));
    }
    let capture = pointer_program(
        "unsafe { let $pointer = Ptr::to($value); let $bump = function(): void with (writable $value) { $value = 1; }; }",
    );
    assert!(
        codes(&capture).contains(&"E0654"),
        "{:#?}",
        findings(&capture)
    );
    let inner = only(
        &pointer_program(
            "unsafe { { writable int32 $inner = 1; let $pointer = MutPtr::to($inner); } }",
        ),
        "E0762",
    );
    assert_eq!(inner.title, "Value Cannot Outlive Borrowed Source");
    let readonly = only(
        &pointer_program("int32 $fixed = 1; unsafe { let $pointer = MutPtr::to($fixed); }"),
        "E0479",
    );
    assert!(readonly.message.contains("$fixed"));
}

#[test]
fn boundary_values_convert_explicitly_and_compare_only_for_equality() {
    for body in [
        "let $long = CLong::from(5); ?CLong $maybe = CLong::tryFrom(-1); echo $long == CLong::from(5); echo $maybe == null;",
        "let $size = CSize::from($value); int64 $length = Int64::from($size); uint8 $small = UInt8::from($size); echo $length;",
    ] {
        let source = pointer_program(body);
        assert!(codes(&source).is_empty(), "{body}: {:#?}", findings(&source));
    }
    let boundary = only(
        &pointer_program("let $long = CLong::from(CSize::from(1));"),
        "E0443",
    );
    assert!(
        boundary
            .helps
            .iter()
            .any(|help| help.contains("Int64::from")),
        "{boundary:#?}"
    );
    only(&pointer_program("let $long = CLong::from(1.5);"), "E0443");
    let arithmetic = only(
        &pointer_program("let $long = CLong::from(5); let $next = $long + $long;"),
        "E0441",
    );
    assert_eq!(
        arithmetic.title,
        "Foreign Value Has No Arithmetic Or Ordering"
    );
    only(
        &pointer_program("let $long = CLong::parse(\"5\");"),
        "E0304",
    );
}

#[test]
fn pointer_operations_report_that_they_cannot_be_built_yet() {
    let source = "use Doria\\Std\\Ffi\\{CLong, Ptr}; function main(): void { writable int32 $value = 1; let $pointer = unsafe(Ptr::to($value)); let $long = CLong::from(2); }";
    assert!(codes(source).is_empty(), "{:#?}", findings(source));
    let diagnostics = doriac::compile_source(
        "ffi.doria",
        source.to_string(),
        doriac::backend::BackendTarget::Debug,
    )
    .expect_err("pointer operations cannot be built yet");
    let titles = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0784")
        .map(|diagnostic| diagnostic.title.as_str())
        .collect::<Vec<_>>();
    // Every target builds boundary conversions and the native targets
    // build pointer operations, including a pointer to a local; the
    // interpreter runs those through the native companion.
    assert_eq!(
        titles,
        ["Pointer Operations Cannot Run In The Interpreter Yet"],
        "{diagnostics:#?}"
    );
    // A pointer to a foreign struct field builds; one to a class property
    // does not yet.
    let property = "use Doria\\Std\\Ffi\\Ptr; class Box { int32 $count = 0; } function main(): void { let $box = new Box(); let $pointer = unsafe(Ptr::to($box->count)); }";
    let diagnostics = doriac::compile_source(
        "ffi.doria",
        property.to_string(),
        doriac::backend::BackendTarget::Native,
    )
    .expect_err("a pointer to a property cannot be built yet");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Pointers To Properties Cannot Be Built Yet"),
        "{diagnostics:#?}"
    );
}
