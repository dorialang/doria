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
fn checked_foreign_code_reports_that_it_cannot_be_built_yet() {
    let source = format!("{ABS}function main(): void {{ echo unsafe(abs(-3)); }}");
    assert!(codes(&source).is_empty(), "{:#?}", findings(&source));
    let diagnostics =
        doriac::compile_source("ffi.doria", source, doriac::backend::BackendTarget::Debug)
            .expect_err("foreign calls cannot be built yet");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0784"
                && diagnostic.title == "Extern Blocks Cannot Be Built Yet"),
        "{diagnostics:#?}"
    );

    let unsafe_only = "/** @safety None. */ unsafe function tick(): int { return 1; } function main(): void { echo unsafe(tick()); }";
    let output = doriac::compile_source_to_debug("tick.doria", unsafe_only)
        .expect("an unsafe function with no foreign code builds");
    assert!(!output.is_empty());
}
