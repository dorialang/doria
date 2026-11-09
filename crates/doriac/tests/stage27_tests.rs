use doriac::diagnostics::Diagnostic;
use doriac::lexer::{Lexer, TokenKind};
use doriac::mir;
use doriac::source::SourceFile;
use doriac::{ast, ast::Item};

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

fn diagnostics(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("stage27.doria", source).expect_err("source should be rejected")
}

// Generic enums are executable, so a valid source has no findings.
fn findings(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("stage27.doria", source)
        .err()
        .unwrap_or_default()
}

fn diagnostic(source: &str, code: &str) -> Diagnostic {
    diagnostics(source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"))
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let mir = doriac::lower_source_to_mir("stage27.doria", source)
        .expect("Stage 27 source should lower through shared MIR");
    doriac::mir_interpreter::interpret(&mir).expect("Stage 27 MIR should execute")
}

#[test]
fn nested_scalar_enum_fields_preserve_adjacent_narrow_values() {
    let source = r#"
enum Status { case Draft; case Published; }
enum Priority: int { case Low = 1; case High = 10; }
enum Label: string { case Short = "short"; case Long = "long"; }
enum Packet { case Value(Status $status, bool $flag, Priority $priority, uint8 $byte, Label $label); }
class Settings { static Packet $value = Packet::Value(Status::Published, true, Priority::High, 255, Label::Long); }
function report(Packet $packet): void {
    let $text = match ($packet) {
        Packet::Value($status, $flag, $priority, $byte, $label) =>
            ($status == Status::Published) . ":{$flag}:{$priority->value}:{$byte}:{$label->value}\n"
    };
    echo $text;
}
function main(): void {
    let $copy = Settings::value;
    report($copy);
}
"#;
    let program = doriac::lower_source_to_mir("nested-scalar-enum-layout.doria", source).unwrap();
    let expected = "true:true:10:255:long\n";
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.exit_status, 0);
    for &profile in doriac::backend::NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
}

#[test]
fn static_payload_enum_reads_preserve_nullable_storage_and_managed_copies() {
    for (field_type, observation) in [("string", "$text"), ("?string", "$text ?? \"missing\"")] {
        let source = r#"
enum State { case Text(FIELD_TYPE $text); }
class Settings {
    static writable State $value = State::Text("first");
    static writable ?State $optional = null;
}
function text(State $state): string {
    return match ($state) { State::Text($text) => OBSERVATION };
}
function optional(?State $state): string {
    if ($state == null) { return "missing"; }
    return text($state);
}
function main(): void {
    Settings::value = State::Text(sprintf("%s", "first"));
    let $copy = Settings::value;
    echo text(Settings::value) . ":" . optional(Settings::optional) . ":";
    Settings::optional = Settings::value;
    let $nullableCopy = Settings::optional;
    Settings::value = State::Text("second");
    Settings::optional = null;
    echo text($copy) . ":" . optional($nullableCopy) . ":";
    echo optional(Settings::value) . ":" . optional(Settings::optional) . "\n";
}
"#
        .replace("FIELD_TYPE", field_type)
        .replace("OBSERVATION", observation);
        let program = doriac::lower_source_to_mir("static-enum-places.doria", &source).unwrap();
        let expected = "first:missing:first:first:second:missing\n";
        let output = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, expected.as_bytes());
        assert_eq!(output.stderr, b"");
        assert_eq!(output.exit_status, 0);
        for &profile in doriac::backend::NativeProfile::AVAILABLE {
            native_execution::assert_native_execution(&program, profile, expected);
        }
        let artifact = doriac::compile_source(
            "static-enum-places.doria",
            &source,
            doriac::backend::BackendTarget::Php,
        )
        .unwrap();
        let directory = native_execution::ExecutionDirectory::new();
        let path = directory.path().join("main.php");
        doriac::backend::publish_output(&path, artifact).unwrap();
        let output = std::process::Command::new("php")
            .args(["-d", "ffi.enable=0"])
            .arg(path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, expected.as_bytes());
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn nullable_payload_results_preserve_source_places_and_once_only_removal() {
    for (payload, value, observation) in [
        ("int", "42", "$payload"),
        ("Item", "new Item()", "$payload->number"),
    ] {
        let source = r#"
class Item { int $number = 42; }
enum Packet { case Value(PAYLOAD $payload); }
class Holder {
    Packet $stored = Packet::Value(VALUE);
    function fetch(): Packet { return $this->stored; }
}
function fromArray(Packet[] $values): ?Packet { return $values[0]; }
function fromList(List<Packet> $values): ?Packet { return $values[0]; }
function fromDictionary(Dictionary<string, Packet> $values): ?Packet { return $values["x"]; }
function fromProperty(Holder $holder): ?Packet { return $holder->stored; }
function fromMethod(Holder $holder): ?Packet { return $holder->fetch(); }
function remove(writable List<Packet> $values): ?Packet { return $values->removeAt(0); }
function observe(?Packet $value): int {
    if ($value == null) { return -1; }
    return match ($value) { Packet::Value($payload) => OBSERVATION };
}
function main(): void {
    Packet[] $array = [Packet::Value(VALUE)];
    writable List<Packet> $list = [Packet::Value(VALUE)];
    Dictionary<string, Packet> $dictionary = ["x" => Packet::Value(VALUE)];
    let $holder = new Holder();
    echo "{observe(fromArray($array))} {observe(fromList($list))} ";
    echo "{observe(fromDictionary($dictionary))} {observe(fromProperty($holder))} ";
    echo "{observe(fromMethod($holder))} {observe(remove($list))} ";
    echo "{$list->count}\n";
}
"#
        .replace("PAYLOAD", payload)
        .replace("VALUE", value)
        .replace("OBSERVATION", observation);
        let program = doriac::lower_source_to_mir("nullable-enum-places.doria", &source).unwrap();
        let output = doriac::mir_interpreter::interpret(&program).unwrap();
        let stdout = "42 42 42 42 42 42 0\n";
        assert_eq!(output.stdout, stdout.as_bytes());
        assert_eq!(output.stderr, b"");
        assert_eq!(output.exit_status, 0);
        for &profile in doriac::backend::NativeProfile::AVAILABLE {
            native_execution::assert_native_execution(&program, profile, stdout);
        }
    }
}

#[test]
fn unit_and_backed_enums_execute_with_nominal_equality_and_value_projection() {
    let output = interpret(
        r#"
enum Status
{
    case Draft;
    case Published;
}

enum Priority: int
{
    case Low = 1;
    case High = 10;
}

enum Transport: string
{
    case Road = "road";
    case Rail = "rail";
}

function main(): void throws Doria\Std\Io\IoError
{
    Status $status = Status::Draft;
    Priority $priority = Priority::High;
    Transport $transport = Transport::Rail;
    echo "{$status == Status::Draft} {$status != Status::Published}\n";
    echo "{$priority->value} {$transport->value}\n";
}
"#,
    );
    assert_eq!(output.stdout, b"true true\n10 rail\n");
    assert_eq!(output.exit_status, 0);
}

#[test]
fn nullable_enum_equality_preserves_nominal_identity_presence_and_payloads() {
    for (declaration, ty, first, second) in [
        (
            "enum Status { case Draft; case Published; }",
            "Status",
            "Status::Draft",
            "Status::Published",
        ),
        (
            "enum Priority: int { case Low = 1; case High = 10; }",
            "Priority",
            "Priority::Low",
            "Priority::High",
        ),
        (
            "enum Label: string { case First = \"first\"; case Second = \"second\"; }",
            "Label",
            "Label::First",
            "Label::Second",
        ),
        (
            "enum Packet { case Text(?string $text); }",
            "Packet",
            "Packet::Text(null)",
            "Packet::Text(\"text\")",
        ),
        (
            "enum Leaf { case Value(int $number); } enum Packet { case Nested(Leaf $value, ?Leaf $optional); }",
            "Packet",
            "Packet::Nested(Leaf::Value(0), null)",
            "Packet::Nested(Leaf::Value(0), Leaf::Value(0))",
        ),
        (
            "enum Packet { case Data(Bytes $bytes); }",
            "Packet",
            "Packet::Data(Bytes::fromArray([1]))",
            "Packet::Data(Bytes::fromArray([2]))",
        ),
        (
            "enum Packet { case Data(?Bytes $bytes); }",
            "Packet",
            "Packet::Data(null)",
            "Packet::Data(Bytes::fromArray([]))",
        ),
        (
            "enum Packet { case Data(?Bytes $bytes); }",
            "Packet",
            "Packet::Data(Bytes::fromArray([1]))",
            "Packet::Data(Bytes::fromArray([2]))",
        ),
    ] {
        let source = format!(
            r#"
{declaration}
function same(?{ty} $left, ?{ty} $right): bool {{ return $left == $right; }}
function different(?{ty} $left, ?{ty} $right): bool {{ return $left != $right; }}
function present({ty} $left, ?{ty} $right): bool {{ return $left == $right && $right == $left; }}
function main(): void {{
    if (!same(null, null) || same(null, {first}) || same({first}, null)
        || !same({first}, {first}) || same({first}, {second})
        || different(null, null) || !different(null, {first}) || !different({first}, null)
        || different({first}, {first}) || !different({first}, {second})
        || !present({first}, {first}) || present({first}, null)
        || present({first}, {second})) {{ panic("nullable enum equality changed"); }}
    echo "nullable equality preserved\n";
}}
"#
        );
        assert_nullable_enum_execution(&source);
    }
}

fn assert_nullable_enum_execution(source: &str) {
    assert_enum_execution(
        "nullable-enum-equality.doria",
        source,
        "nullable equality preserved\n",
    );
}

#[test]
fn nullable_enum_callback_survives_failed_guard_and_keeps_null_distinct() {
    let source = r#"
enum Work { case Ready(function(): int $callback); }
function run(take ?Work $work): int {
    return match (take $work) {
        null => 0,
        Work::Ready($callback) if $callback() == 99 => 99,
        Work::Ready($callback) => $callback(),
    };
}
function main(): void {
    let $answer = 42;
    echo run(Work::Ready(fn() with (take $answer) => $answer));
    echo run(null);
}
"#;
    assert_enum_execution("nullable-enum-callback.doria", source, "420");
}

fn assert_enum_execution(path: &str, source: &str, expected: &str) {
    let program = doriac::lower_source_to_mir(path, source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in doriac::backend::NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
    let artifact =
        doriac::compile_source(path, source, doriac::backend::BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=0"])
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn enum_object_and_error_payloads_preserve_identity_including_nullable_fields() {
    for (declaration, ty, value) in [
        ("class Item {}", "Item", "new Item()"),
        (
            "class Failure implements Error { function __construct(string $message) {} }",
            "Error",
            "new Failure(\"same\")",
        ),
    ] {
        let source = format!(
            r#"
{declaration}
enum Packet {{ case Value({ty} $value, ?{ty} $optional); }}
function same(?Packet $left, ?Packet $right): bool {{ return $left == $right; }}
function different(?Packet $left, ?Packet $right): bool {{ return $left != $right; }}
function main(): void {{
    let $first = Packet::Value({value}, null);
    let $second = Packet::Value({value}, null);
    let $present = Packet::Value({value}, {value});
    if (!same($first, $first) || same($first, $second) || same($first, $present)
        || different($first, $first) || !different($first, $second)
        || !same($present, $present) || different($present, $present)
        || same(null, $first) || same($first, null) || !same(null, null)) {{
        panic("enum payload identity changed");
    }}
    echo "nullable equality preserved\n";
}}
"#
        );
        assert_nullable_enum_execution(&source);
    }
}

#[test]
fn nullable_enum_equality_retains_type_and_payload_capability_diagnostics() {
    for left in ["Status", "?Status"] {
        for right in ["Other", "?Other"] {
            diagnostic(&format!("enum Status: int {{ case Ready = 1; }} enum Other: int {{ case Ready = 1; }} function same({left} $left, {right} $right): bool {{ return $left == $right; }}"), "E0579");
        }
        for right in ["int", "?int"] {
            diagnostic(&format!("enum Status: int {{ case Ready = 1; }} function same({left} $left, {right} $right): bool {{ return $left == $right; }}"), "E0580");
        }
    }
    for left in ["Bucket", "?Bucket"] {
        for right in ["Bucket", "?Bucket"] {
            diagnostic(&format!("enum Bucket {{ case Values(List<int> $values); }} function same({left} $left, {right} $right): bool {{ return $left == $right; }}"), "E0584");
        }
    }
    doriac::check_source("nullable-enum-null-test.doria", "enum Bucket { case Values(List<int> $values); } function absent(?Bucket $value): bool { return $value == null; }").expect("presence tests do not require payload equality");
}

#[test]
fn nullable_enum_keeps_first_case_distinct_from_null_and_supports_narrowing() {
    let output = interpret(
        r#"
enum Status
{
    case Draft;
    case Published;
}

function read(?Status $status): Status
{
    if ($status != null) {
        return $status;
    }
    return Status::Published;
}

function main(): void throws Doria\Std\Io\IoError
{
    ?Status $missing = null;
    ?Status $first = Status::Draft;
    echo "{$missing == null} {$first != null} ";
    echo read($missing) == Status::Published;
    echo " ";
    echo ($first ?? Status::Published) == Status::Draft;
}
"#,
    );
    assert_eq!(output.stdout, b"true true true true");
}

#[test]
fn nullable_backed_enum_value_projection_preserves_presence() {
    let output = interpret(
        r#"
enum Priority: int { case Low = 1; case High = 10; }
enum Transport: string { case Road = "road"; case Rail = "rail"; }

function main(): void throws Doria\Std\Io\IoError
{
    ?Priority $priority = Priority::High;
    ?Priority $missingPriority = null;
    ?Transport $transport = Transport::Rail;
    ?Transport $missingTransport = null;
    ?int $priorityValue = $priority?->value;
    ?int $missingPriorityValue = $missingPriority?->value;
    ?string $transportValue = $transport?->value;
    ?string $missingTransportValue = $missingTransport?->value;
    echo "{$priorityValue ?? -1} {$missingPriorityValue ?? -1} ";
    echo $transportValue ?? "missing";
    echo " ";
    echo $missingTransportValue ?? "missing";
}
"#,
    );
    assert_eq!(output.stdout, b"10 -1 rail missing");
}

#[test]
fn enum_identity_survives_mixed_boxing_and_exact_narrowing() {
    let output = interpret(
        r#"
enum Status { case Draft; case Published; }
enum Other { case Draft; }

function main(): void throws Doria\Std\Io\IoError
{
    mixed $value = Status::Draft;
    if ($value is Status) {
        echo $value == Status::Draft;
    }
    if ($value is Other) {
        echo "wrong";
    }
}
"#,
    );
    assert_eq!(output.stdout, b"true");
}

#[test]
fn unit_cases_are_constants_and_copy_defaults() {
    let output = interpret(
        r#"
enum Status { case Draft; case Published; }
const Status DEFAULT_STATUS = Status::Draft;

function show(Status $status = Status::Draft): void throws Doria\Std\Io\IoError
{
    echo $status == DEFAULT_STATUS;
}

function main(): void throws Doria\Std\Io\IoError
{
    show();
    show(Status::Published);
}
"#,
    );
    assert_eq!(output.stdout, b"truefalse");
}

#[test]
fn payload_execution_and_core_match_are_both_available_after_stage_27() {
    doriac::check_source(
        "stage27.doria",
        r#"
enum Shape
{
    case Circle(float $radius);
    case Rect(float $width, float $height);
}
Shape $shape = Shape::Circle(2.5);
"#,
    )
    .expect("payload construction should execute in Stage 27 Slice 2");

    doriac::check_source(
        "stage28.doria",
        r#"
enum Shape { case Circle(float $radius); }
let $area = match (true) {
    true => 1.0,
    default => 0.0,
};
"#,
    )
    .expect("core match should execute in Stage 28 Slice 1");
}

#[test]
fn enum_specific_diagnostics_do_not_fall_through_to_class_errors() {
    let unit_call = diagnostic(
        "enum Status { case Draft; } Status $status = Status::Draft();",
        "E0575",
    );
    assert!(unit_call.fix.is_some());

    let unknown = diagnostic(
        "enum Status { case Draft; } Status $status = Status::Draf;",
        "E0574",
    );
    assert!(unknown.message.contains("unknown case"));
    assert_eq!(
        unknown.fix.as_ref().map(|fix| fix.replacement.as_str()),
        Some("Draft")
    );

    diagnostic("enum Status { case Draft; } echo Status::Draft;", "E0445");
    diagnostic(
        "enum Priority: int { case Low = 1; } bool $same = Priority::Low == 1;",
        "E0580",
    );
}

#[test]
fn declaration_and_backing_rules_are_enforced() {
    diagnostic("enum Empty {}", "E0562");
    diagnostic("enum Status { case Draft; case Draft; }", "E0565");
    diagnostic(
        "enum Priority: int { case Low = 1; case High = 1; }",
        "E0569",
    );
    diagnostic("enum Priority: bool { case Low = true; }", "E0566");
    diagnostic("enum Status { case Draft = 1; }", "E0568");
    diagnostic("enum Priority: int { case Low; }", "E0567");
    diagnostic(
        "enum Priority: int { case Low(string $label) = 1; }",
        "E0571",
    );
}

#[test]
fn enum_surface_rejects_implicit_conversion_display_and_conformance() {
    diagnostic(
        "enum Left { case Ready; } enum Right { case Ready; } bool $same = Left::Ready == Right::Ready;",
        "E0579",
    );
    diagnostic(
        "enum Priority: int { case Low = 1; } bool $same = Priority::Low == 1;",
        "E0580",
    );
    diagnostic("enum Status { case Ready; } echo Status::Ready;", "E0445");
    diagnostic(
        "enum Status { case Ready; } string $text = \"{Status::Ready}\";",
        "E0415",
    );
    diagnostic(
        "enum Status { case Ready; } Status $status = Status::Ready; echo $status->value;",
        "E0577",
    );
    diagnostic(
        "enum Priority: int { case Low = 1; } Priority::Low->value();",
        "E0575",
    );
    diagnostic(
        "enum Priority: int { case Low = 1; } Priority $priority = Priority::Low; $priority->value = 2;",
        "E0578",
    );
    diagnostic(
        "enum Status { case Ready; } Set<Status> $statuses = [Status::Ready];",
        "E0523",
    );
    diagnostic(
        "enum Status { case Ready; } Dictionary<Status, string> $labels = [Status::Ready => \"ready\"];",
        "E0523",
    );
}

#[test]
fn generic_and_malformed_enum_forms_stop_at_their_owned_boundaries() {
    let generic = findings("enum Optional<T> { case None; case Some(T $value); }");
    assert!(generic.is_empty(), "{generic:?}");

    for source in [
        "enum Status { case Ready }",
        "enum Priority: { case Low = 1; }",
        "enum Status { case ; }",
    ] {
        let found = doriac::parse_source("stage27.doria", source)
            .expect_err("malformed enum syntax should be rejected by the parser");
        assert!(found.iter().any(|diagnostic| diagnostic.code == "P0001"));
    }
}

#[test]
fn generic_enum_scopes_and_type_applications_survive_the_execution_boundary() {
    use doriac::types::{ClassType, ResolvedType};

    let source = r#"
namespace Example;
class T {}
enum Packet<T> { case Value(T $value); }
enum Unused<U> { case Value(U $value); }
enum Container { case Value(Packet<bool> $first, Packet<string> $second); }
enum Outside { case Value(T $value); }
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("generic-enum-scopes.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let definition = |name| {
        analysis
            .info
            .enum_declarations
            .iter()
            .find(|definition| definition.source_type.name == name)
            .unwrap()
    };
    let packet = definition("Example\\Packet");
    let parameter = ResolvedType::TypeParameter("T".into());
    assert_eq!(packet.source_type.arguments, vec![parameter.clone()]);
    assert_eq!(packet.cases[0].payload[0].ty, parameter);
    let fields = &definition("Example\\Container").cases[0].payload;
    for (field, argument) in fields
        .iter()
        .zip([ResolvedType::Bool, ResolvedType::String])
    {
        assert_eq!(
            field.ty,
            ResolvedType::Enum(doriac::enums::EnumType::with_arguments(
                packet.source_type.id,
                "Example\\Packet",
                vec![argument],
            ))
        );
    }
    assert_eq!(fields.len(), 2);
    assert_eq!(
        definition("Example\\Outside").cases[0].payload[0].ty,
        ResolvedType::Class(ClassType::new("Example\\T", Vec::new())),
    );
    assert!(analysis
        .info
        .enums
        .iter()
        .all(|instance| instance.source_type.name != "Example\\Unused"));
    let applications = analysis
        .info
        .enums
        .iter()
        .filter(|instance| instance.source_type.name == "Example\\Packet")
        .collect::<Vec<_>>();
    assert_eq!(applications.len(), 2);
    for argument in [ResolvedType::Bool, ResolvedType::String] {
        let instance = applications
            .iter()
            .find(|instance| instance.source_type.arguments == [argument.clone()])
            .unwrap();
        assert_eq!(instance.cases[0].payload[0].ty, argument);
        assert_eq!(instance.declaration, packet.declaration);
        assert_eq!(instance.cases[0].declaration, packet.cases[0].declaration);
    }
    for (index, instance) in analysis.info.enums.iter().enumerate() {
        assert_eq!(instance.id, doriac::enums::EnumId(index));
        assert_eq!(instance.layout.enum_id, instance.id);
        assert!(instance
            .cases
            .iter()
            .all(|case| case.id.enum_id == instance.id));
    }
}

#[test]
fn enum_constant_types_keep_declaration_identity_when_instance_ids_differ() {
    let source = r#"
enum Packet<T> { case Value(T $value); }
enum Status { case Ready; case Done; }
enum Label { case Text(string $value); }
const Status DEFAULT_STATUS = Status::Ready;
const Label DEFAULT_LABEL = Label::Text("default");
function status(Status $value = Status::Ready): bool {
    return $value == DEFAULT_STATUS;
}
function label(Label $value = Label::Text("default")): bool {
    return $value == DEFAULT_LABEL;
}
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("enum-constant-identity.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for name in ["Status", "Label"] {
        let instance = analysis
            .info
            .enums
            .iter()
            .find(|instance| instance.source_type.name == name)
            .unwrap();
        assert_ne!(instance.id, instance.source_type.id);
        let declaration = analysis
            .info
            .enum_declarations
            .iter()
            .find(|declaration| declaration.source_type == instance.source_type)
            .unwrap();
        assert_eq!(declaration.declaration, instance.declaration);
    }
}

#[test]
fn enum_case_facts_and_cleanup_keep_authored_identity_when_layout_ids_differ() {
    use doriac::enums::EnumCaseSource;
    use doriac::ownership::cleanup::Source;
    use doriac::semantics::ResolvedMatchPattern;

    let source = r#"
enum Deferred<T> { case Value(T $value); }
class Token {}
enum Packet { case Empty; case Value(Token $token); }
function make(): Packet { return Packet::Value(new Token()); }
function empty(): Packet { return Packet::Empty; }
function read(take Packet $packet): int {
    return match (take $packet) { Packet::Value($token) => 1, Packet::Empty => 0 };
}
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("case-source-identity.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let definition = analysis
        .info
        .enums
        .iter()
        .find(|definition| definition.source_type.name == "Packet")
        .unwrap();
    assert_ne!(definition.id, definition.source_type.id);
    let source_case = |index| EnumCaseSource {
        declaration: definition.source_type.id,
        index,
    };
    assert_eq!(
        analysis
            .info
            .enum_case_constructions
            .values()
            .copied()
            .collect::<Vec<_>>(),
        vec![source_case(1)]
    );
    assert_eq!(
        analysis
            .info
            .enum_case_values
            .values()
            .copied()
            .collect::<Vec<_>>(),
        vec![source_case(0)]
    );
    let matched = analysis.info.matches.values().next().unwrap();
    assert_eq!(
        matched.arms[0].pattern,
        ResolvedMatchPattern::EnumCase {
            case_id: source_case(1)
        }
    );
    assert_eq!(
        matched.arms[1].pattern,
        ResolvedMatchPattern::EnumCase {
            case_id: source_case(0)
        }
    );
    assert!(analysis.info.cleanup.values.values().any(|value| matches!(value.source, Source::EnumPayload { case, field: 0, .. } if case == source_case(1))));
    assert!(analysis.info.cleanup.source_flows.iter().any(|(_, target)| matches!(target, Source::EnumPayload { case, field: 0, .. } if *case == source_case(1))));
}

#[test]
fn generic_enum_bounds_use_shared_nominal_argument_checking() {
    for (argument, invalid) in [("Yes", false), ("No", true)] {
        let source = format!(
            r#"
interface Ready {{}}
class Yes implements Ready {{}}
class No {{}}
enum Packet<T implements Ready> {{ case Value(T $value); }}
function accept(Packet<{argument}> $packet): void {{}}
"#
        );
        let found = findings(&source);
        assert_eq!(
            found.iter().any(|diagnostic| diagnostic.code == "E0535"),
            invalid,
            "{found:?}"
        );
        assert!(
            found.iter().all(|diagnostic| diagnostic.code == "E0535"),
            "{found:?}"
        );
    }
}

#[test]
fn generic_call_inference_traverses_enum_arguments_and_expected_results() {
    use doriac::semantics::GenericArgument;
    use doriac::types::{IntegerType, ResolvedType};

    let source = r#"
enum Packet<T> { case Value(T $value); }
function select<T>(Packet<T> $packet): ?T { return null; }
function nested<T>(List<Packet<T>> $packets): ?T { return null; }
function empty<T>(): ?Packet<T> { return null; }
function direct(Packet<int> $packet): ?int { return select(packet: $packet); }
function wrapped(List<Packet<string>> $packets): ?string { return nested(packets: $packets); }
function main(): void { ?Packet<bool> $result = empty(); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("enum-inference.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for (call, argument) in [
        ("select(packet:", ResolvedType::Integer(IntegerType::Int64)),
        ("nested(packets:", ResolvedType::String),
        ("empty();", ResolvedType::Bool),
    ] {
        let start = source.find(call).unwrap();
        let (_, specialization) = analysis
            .info
            .generic_call_specializations
            .iter()
            .find(|(span, _)| span.source == doriac::source::SourceId(0) && span.start == start)
            .unwrap_or_else(|| panic!("missing specialization for {call}"));
        assert_eq!(
            specialization.arguments,
            vec![GenericArgument::Type(argument)]
        );
    }
}

#[test]
fn generic_enum_cases_infer_arguments_then_fill_result_context() {
    use doriac::types::ResolvedType;
    let source = r#"
enum Packet<T, U> { case Empty; case First(T $value); case Both(T $first, U $second); }
function make(): Packet<bool, string> { return Packet::Both(second: "yes", first: true); }
function partial(): Packet<bool, string> { return Packet::First(true); }
function empty(): ?Packet<bool, string> { return (Packet::Empty); }
function accept(Packet<bool, string> $packet): void {}
function main(): void { accept(Packet::Empty); }
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("enum-case-inference.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let spans = analysis
        .info
        .enum_case_constructions
        .keys()
        .chain(analysis.info.enum_case_values.keys())
        .collect::<Vec<_>>();
    assert_eq!(spans.len(), 4);
    for span in spans {
        let ty = &analysis.info.expression_types[span];
        let ResolvedType::Enum(enumeration) = ty else {
            panic!("{ty:?}")
        };
        assert_eq!(enumeration.name, "Packet");
        assert_eq!(
            enumeration.arguments,
            vec![ResolvedType::Bool, ResolvedType::String]
        );
        assert!(!analysis
            .info
            .generic_call_specializations
            .contains_key(span));
        assert!(!analysis.info.call_targets.contains_key(span));
    }
}

#[test]
fn generic_enum_match_binds_substituted_payloads_in_generic_and_concrete_bodies() {
    use doriac::types::ResolvedType;
    let source = r#"
enum Packet<T> { case Empty; case Value(T $value); }
function read<U>(Packet<U> $packet): ?U {
    return match ($packet) { Packet::Value($value) => $value, Packet::Empty => null };
}
function flag(Packet<bool> $packet): bool {
    return match ($packet) { Packet::Value($value) => $value, Packet::Empty => false };
}
function make<U>(take U $value): Packet<U> { return Packet::Value($value); }
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("enum-match-inference.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let mut matches = analysis.info.matches.iter().collect::<Vec<_>>();
    matches.sort_by_key(|(span, _)| **span);
    assert_eq!(matches.len(), 2);
    assert_eq!(
        matches[0].1.arms[0].bindings[0].ty,
        ResolvedType::TypeParameter("U".into())
    );
    assert_eq!(matches[1].1.arms[0].bindings[0].ty, ResolvedType::Bool);
    assert!(!matches[1].1.arms[0].bindings[0].borrowed);
}

#[test]
fn generic_enum_construction_reports_inference_conflicts_and_missing_arguments() {
    for (body, code) in [
        (
            "function make(): Packet<bool> { return Packet::Both(true, \"wrong\"); }",
            "E0532",
        ),
        (
            "function make(): Packet<string> { return Packet::Both(true, true); }",
            "E0404",
        ),
        ("function main(): void { Packet::Empty; }", "E0531"),
    ] {
        let source =
            format!("enum Packet<T> {{ case Empty; case Both(T $first, T $second); }} {body}");
        let found = diagnostics(&source);
        assert!(
            found.iter().any(|d| d.code == code),
            "expected {code}: {found:?}"
        );
    }
}

#[test]
fn generic_enum_nested_empty_case_uses_the_substituted_payload_context() {
    use doriac::types::ResolvedType;
    let source = r#"
enum Optional<T> { case Empty; case Value(T $value); }
enum Packet<T> { case Value(T $value); }
function make(): Packet<Optional<bool>> { return Packet::Value((Optional::Empty)); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("nested-empty-case.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let (span, _) = analysis.info.enum_case_values.iter().next().unwrap();
    let ResolvedType::Enum(enumeration) = &analysis.info.expression_types[span] else {
        panic!("missing empty case type")
    };
    assert_eq!(enumeration.name, "Optional");
    assert_eq!(enumeration.arguments, [ResolvedType::Bool]);
}

#[test]
fn generic_enum_inferred_arguments_check_nominal_bounds() {
    for (name, invalid) in [("Yes", false), ("No", true)] {
        let source = format!(
            r#"
interface Ready {{}}
class Yes implements Ready {{}}
class No {{}}
enum Packet<T implements Ready> {{ case Value(T $value); }}
function make(): Packet<{name}> {{ return Packet::Value(new {name}()); }}
"#
        );
        let found = findings(&source);
        assert_eq!(
            found.iter().any(|d| d.code == "E0535"),
            invalid,
            "{found:?}"
        );
        assert!(found.iter().all(|d| d.code == "E0535"), "{found:?}");
    }
}

#[test]
fn pascal_case_fixes_are_exact_and_suppressed_on_collisions() {
    let enum_name = diagnostic("enum status { case Ready; }", "E0563");
    assert_eq!(
        enum_name.fix.as_ref().map(|fix| fix.replacement.as_str()),
        Some("Status")
    );
    let case_name = diagnostic("enum Status { case ready; }", "E0564");
    assert_eq!(
        case_name.fix.as_ref().map(|fix| fix.replacement.as_str()),
        Some("Ready")
    );

    let enum_collision = diagnostics("class Status {} enum status { case Ready; }");
    assert!(enum_collision
        .iter()
        .find(|diagnostic| diagnostic.code == "E0563")
        .is_some_and(|diagnostic| diagnostic.fix.is_none()));
    let case_collision = diagnostics("enum Status { case ready; case Ready; }");
    assert!(case_collision
        .iter()
        .find(|diagnostic| diagnostic.code == "E0564")
        .is_some_and(|diagnostic| diagnostic.fix.is_none()));

    let namespaced = diagnostics("namespace Acme; enum status { case Ready; }");
    let namespaced_name = namespaced
        .iter()
        .find(|diagnostic| diagnostic.code == "E0563")
        .expect("the authored enum segment should be validated");
    assert_eq!(
        namespaced_name.message,
        "enum name `status` must use PascalCase"
    );
    assert_eq!(
        namespaced_name
            .fix
            .as_ref()
            .map(|fix| fix.replacement.as_str()),
        Some("Status")
    );

    let namespaced_collision =
        diagnostics("namespace Acme; class Status {} enum status { case Ready; }");
    assert!(namespaced_collision
        .iter()
        .find(|diagnostic| diagnostic.code == "E0563")
        .is_some_and(|diagnostic| diagnostic.fix.is_none()));
}

#[test]
fn enum_and_match_keywords_are_real_tokens() {
    let source = SourceFile::new("stage27.doria", "enum case match default");
    let tokens = Lexer::new(&source)
        .lex()
        .expect("Stage 27 keywords should lex");
    assert!(matches!(tokens[0].kind, TokenKind::Enum));
    assert!(matches!(tokens[1].kind, TokenKind::Case));
    assert!(matches!(tokens[2].kind, TokenKind::Match));
    assert!(matches!(tokens[3].kind, TokenKind::Default));
    assert!(!tokens
        .iter()
        .any(|token| matches!(token.kind, TokenKind::Reserved(_))));
}

#[test]
fn checked_enum_metadata_preserves_authored_type_case_and_payload_origins() {
    let source = r#"
namespace Example;
enum Packet { case Empty; case Count(int $count); }
function main(): void { let $value = Packet::Count(7); }
"#;
    let parsed = doriac::parse_source("enum-origins.doria", source).unwrap();
    let declaration = parsed
        .items
        .iter()
        .find_map(|item| match item {
            Item::Enum(declaration) => Some(declaration),
            _ => None,
        })
        .unwrap();
    let (_, analysis) = doriac::analyze_source_for_ide("enum-origins.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let checked = analysis
        .info
        .enums
        .iter()
        .find(|definition| definition.source_type.name == "Example\\Packet")
        .unwrap();
    let template = analysis
        .info
        .enum_declarations
        .iter()
        .find(|definition| definition.source_type == checked.source_type)
        .unwrap();
    assert_eq!(template.declaration, checked.declaration);
    assert_eq!(template.name_span, checked.name_span);
    assert_eq!(template.cases.len(), checked.cases.len());
    for (template, instance) in template.cases.iter().zip(&checked.cases) {
        assert_eq!(template.name, instance.name);
        assert_eq!(template.declaration, instance.declaration);
        assert_eq!(template.name_span, instance.name_span);
        assert_eq!(template.payload, instance.payload);
    }
    assert_eq!(checked.source_type.id, checked.id);
    assert!(checked.source_type.arguments.is_empty());
    assert_eq!(checked.declaration, declaration.span);
    assert_eq!(checked.name_span, declaration.name_span);
    assert_eq!(
        &source[checked.name_span.start..checked.name_span.end],
        "Packet"
    );
    assert_eq!(checked.cases.len(), declaration.cases.len());
    for (case, authored) in checked.cases.iter().zip(&declaration.cases) {
        assert_eq!(case.declaration, authored.span);
        assert_eq!(case.name_span, authored.name_span);
        assert_eq!(&source[case.name_span.start..case.name_span.end], case.name);
        assert_eq!(case.payload.len(), authored.payload.len());
        for (field, authored) in case.payload.iter().zip(&authored.payload) {
            assert_eq!(field.declaration, authored.span);
            assert_eq!(field.name, authored.name);
        }
    }
}

#[test]
fn parser_preserves_unit_backed_payload_generic_and_match_shapes() {
    let program = doriac::parse_source(
        "stage27.doria",
        r#"
enum Status { case Draft; }
enum Priority: int { case High = 10; }
enum Transport: string { case Rail = "rail"; }
enum Shape<T> { case Circle(float $radius); case Value(T $value); }
function main(): void {
    let $label = match (true) {
        Shape::Circle($radius) => $radius,
        null => 0.0,
        default => 1.0,
    };
}
"#,
    )
    .expect("accepted enum and match syntax should parse without recovery");

    let declarations = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(declaration) => Some(declaration),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(declarations.len(), 4);
    assert_eq!(declarations[1].backing_type.as_ref().unwrap().name, "int");
    assert_eq!(
        declarations[2].backing_type.as_ref().unwrap().name,
        "string"
    );
    assert_eq!(declarations[3].type_params.len(), 1);
    assert_eq!(declarations[3].cases[0].payload.len(), 1);

    let Item::Function(main) = &program.items[4] else {
        panic!("expected main function");
    };
    let ast::Stmt::VarDecl(binding) = &main.body.statements()[0] else {
        panic!("expected match binding");
    };
    let ast::Expr::Match { arms, .. } = &binding.initializer else {
        panic!("expected match expression");
    };
    assert_eq!(arms.len(), 3);
    assert!(matches!(
        &arms[0].pattern,
        ast::MatchPattern::EnumCase {
            bindings: Some(bindings),
            ..
        } if bindings.len() == 1
    ));
    assert!(matches!(arms[1].pattern, ast::MatchPattern::Expression(_)));
    assert!(matches!(arms[2].pattern, ast::MatchPattern::Default { .. }));
}

#[test]
fn payload_and_core_match_fixtures_both_pass_semantics() {
    let payload = include_str!("fixtures/stage27/payload_enum.doria");
    doriac::parse_source("stage27.doria", payload)
        .expect("payload syntax should have no lexer or parser diagnostics");
    doriac::check_source("stage27.doria", payload)
        .expect("payload construction should pass Stage 27 checking");

    let matching = include_str!("fixtures/stage27/match_expression.doria");
    doriac::parse_source("stage27.doria", matching)
        .expect("accepted match syntax should have no lexer or parser diagnostics");
    doriac::check_source("stage28.doria", matching)
        .expect("accepted core match fixture should pass Stage 28 checking");
}

#[test]
fn enum_type_namespace_and_case_namespaces_are_checked_globally() {
    diagnostic(
        "enum Status { case Draft; } enum Status { case Other; }",
        "E0560",
    );
    diagnostic("class Status {} enum Status { case Draft; }", "E0561");
    diagnostic("interface Status {} enum Status { case Draft; }", "E0561");
    diagnostic("trait Status {} enum Status { case Draft; }", "E0561");
    doriac::check_source(
        "stage27.doria",
        "enum Left { case Ready; } enum Right { case Ready; }",
    )
    .expect("case names are scoped to their declaring enum");
}

#[test]
fn enum_payload_types_resolve_against_all_declared_classes() {
    for source in [
        "class User {} enum Result { case Ok(User $user); }",
        "enum Result { case Ok(User $user); } class User {}",
    ] {
        doriac::check_source("stage27.doria", source)
            .expect("enum payload types should resolve regardless of declaration order");
    }
}

#[test]
fn payload_construction_copy_constants_defaults_and_generic_storage_execute() {
    let output = interpret(
        r#"
enum Coordinate
{
    case Origin;
    case Point(int $x, int $y);
}

enum Label
{
    case Text(string $value);
}

const Label DEFAULT_LABEL = Label::Text("default");

class Box<T>
{
    function __construct(take T $value)
    {
    }
}

function mark(string $name, int $value): int throws Doria\Std\Io\IoError
{
    echo $name;
    return $value;
}

function defaulted(Label $label = Label::Text("default")): bool
{
    return $label == DEFAULT_LABEL;
}

function main(): void throws Doria\Std\Io\IoError
{
    Coordinate $point = Coordinate::Point(
        y: mark("y", 22),
        x: mark("x", 20),
    );
    Coordinate $copy = $point;
    let $box = new Box<Coordinate>($copy);
    echo " {$point == Coordinate::Point(20, 22)}";
    echo " {$box->value == $point}";
    echo " {defaulted()}\n";
}
"#,
    );
    assert_eq!(output.stdout, b"yx true true true\n");
}

#[test]
fn promoted_properties_own_copies_of_copyable_payload_enums() {
    let program = doriac::lower_source_to_mir(
        "stage27-promoted-payload.doria",
        r#"
enum Label { case Text(string $value); }

class LabelReader
{
    function __construct(Label $label) {}
}

class OptionalLabelReader
{
    function __construct(?Label $label) {}
}

function main(): void
{
    let $reader = new LabelReader(Label::Text("constructor"));
    let $optional = new OptionalLabelReader(Label::Text("optional"));
}
"#,
    )
    .expect("copyable payload enum promotion should lower");

    let promoted_value = |name: &str| {
        program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("missing constructor {name}"))
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match statement {
                mir::Statement::AssignProperty { value, .. } => Some(value),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name} should initialize its promoted property"))
    };

    assert!(matches!(
        promoted_value("LabelReader::__construct"),
        mir::Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            mode: mir::PayloadEnumUseMode::Copy,
            ..
        })
    ));
    assert!(matches!(
        promoted_value("OptionalLabelReader::__construct"),
        mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Use {
            mode: mir::PayloadEnumUseMode::Copy,
            ..
        })
    ));
}

#[test]
fn payload_ownership_layout_equality_and_observation_boundaries_are_checked() {
    let moved = diagnostics(
        r#"
class Document {}
enum LoadResult { case Loaded(Document $document); }
function main(): void throws Doria\Std\Io\IoError
{
    Document $document = new Document();
    LoadResult $result = LoadResult::Loaded($document);
    let $again = $document;
}
"#,
    );
    assert!(moved.iter().any(|diagnostic| diagnostic.code == "E0470"));

    diagnostic("enum Node { case Next(Node $next); }", "E0581");
    diagnostic("enum Node { case Next(?Node $next); }", "E0581");
    diagnostic(
        "enum Left { case Next(Right $right); } enum Right { case Next(Left $left); }",
        "E0581",
    );
    doriac::check_source(
        "stage27.doria",
        "class Link { ?Node $next = null; } enum Node { case Next(Link $link); }",
    )
    .expect("recursion through a pointer-shaped class remains finite");

    diagnostic(
        r#"
enum Bucket { case Values(List<int> $values); }
function main(): void throws Doria\Std\Io\IoError
{
    Bucket $left = Bucket::Values([1]);
    Bucket $right = Bucket::Values([1]);
    bool $same = $left == $right;
}
"#,
        "E0584",
    );
    diagnostic(
        r#"
enum Coordinate { case Point(int $x, int $y); }
function main(): void throws Doria\Std\Io\IoError
{
    Coordinate $point = Coordinate::Point(1, 2);
    echo $point->x;
}
"#,
        "E0577",
    );
}

#[test]
fn payload_case_calls_reuse_the_normal_argument_binding_rules() {
    diagnostic(
        "enum Coordinate { case Point(int $x, int $y); } let $p = Coordinate::Point;",
        "E0583",
    );
    diagnostic(
        "enum Status { case Ready; } let $s = Status::Ready();",
        "E0575",
    );

    for source in [
        "enum Coordinate { case Point(int $x, int $y); } let $p = Coordinate::Point(x: 1);",
        "enum Coordinate { case Point(int $x, int $y); } let $p = Coordinate::Point(z: 1, y: 2);",
        "enum Coordinate { case Point(int $x, int $y); } let $p = Coordinate::Point(x: 1, x: 2);",
        "enum Coordinate { case Point(int $x, int $y); } let $p = Coordinate::Point(x: 1, 2);",
    ] {
        assert!(
            doriac::check_source("stage27.doria", source).is_err(),
            "invalid payload arguments must be rejected: {source}"
        );
    }
}

#[test]
fn enums_cannot_shadow_compiler_known_type_names() {
    for name in [
        "String",
        "Int",
        "Float",
        "Bool",
        "List",
        "Dictionary",
        "Set",
        "Bytes",
        "Displayable",
        "SharedReference",
        "WritableSharedReference",
        "WeakReference",
    ] {
        let source = format!("enum {name} {{ case Value; }}");
        let found = diagnostics(&source);
        assert!(
            found.iter().any(|diagnostic| diagnostic.code == "E0561"),
            "{name} should be rejected as a reserved type name: {found:#?}"
        );
    }
}

#[test]
fn generic_payload_enums_execute_after_specialization() {
    assert_enum_execution(
        "generic-copy-enum.doria",
        r#"
enum Optional<T> { case None; case Some(T $value); }
function unwrap(Optional<int> $value): int {
    return match ($value) { Optional::None => -1, Optional::Some($inner) => $inner, };
}
function main(): void {
    let $some = Optional::Some(7);
    Optional<int> $none = Optional::None;
    echo unwrap($some) . ";" . unwrap($none);
}
"#,
        "7;-1",
    );
    assert_enum_execution(
        "generic-enum-functions.doria",
        r#"
enum Optional<T> { case None; case Some(T $value); }
function wrap<T>(take T $value): Optional<T> { return Optional::Some($value); }
function orElse<T>(take Optional<T> $value, take T $fallback): T {
    return match (take $value) { Optional::None => $fallback, Optional::Some($inner) => $inner, };
}
function main(): void {
    echo orElse(wrap(3), 0) . ";" . orElse(wrap("text"), "none") . ";";
    Optional<string> $empty = Optional::None;
    echo orElse($empty, "fallback");
}
"#,
        "3;text;fallback",
    );
    assert_enum_execution(
        "generic-move-enum.doria",
        r#"
enum Optional<T> { case None; case Some(T $value); }
class Token {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function consume(take Optional<Token> $value): int {
    return match (take $value) { Optional::None => 0, Optional::Some($token) => $token->value, };
}
function main(): void {
    echo consume(Optional::Some(new Token(5))) . ";";
    echo consume(Optional::None);
}
"#,
        "drop5;5;0",
    );
    assert_enum_execution(
        "generic-callback-enum.doria",
        r#"
enum Optional<T> { case None; case Some(T $value); }
function call(Optional<function(): int> $value): int {
    return match ($value) { Optional::None => 0, Optional::Some($run) => $run(), };
}
function main(): void {
    let $base = 40;
    let $callback = fn() with (take $base) => $base + 2;
    echo call(Optional::Some($callback));
}
"#,
        "42",
    );
    assert_enum_execution(
        "generic-nested-enum.doria",
        r#"
enum Optional<T> { case None; case Some(T $value); }
enum Pair<A, B> { case Both(A $left, B $right); }
function depth(Optional<Optional<int>> $value): int {
    return match ($value) {
        Optional::None => 0,
        Optional::Some($inner) => match ($inner) { Optional::None => 1, Optional::Some($number) => $number, },
    };
}
function main(): void {
    echo depth(Optional::Some(Optional::Some(9))) . ";" . depth(Optional::Some(Optional::None)) . ";";
    let $pair = Pair::Both(1, "two");
    echo match ($pair) { Pair::Both($left, $right) => "{$left}{$right}", };
}
"#,
        "9;1;1two",
    );
    // Each application is its own runtime identity across `mixed`.
    assert_enum_execution(
        "generic-enum-mixed-identity.doria",
        r#"
enum Optional<T> { case Some(T $value); case Empty; }
function main(): void {
    mixed $number = Optional::Some(1);
    echo ($number is Optional<int>) ? "a" : "b";
    echo ($number is Optional<string>) ? "c" : "d";
    mixed $text = Optional::Some("x");
    echo ($text is Optional<string>) ? "e" : "f";
}
"#,
        "ade",
    );
    // A Copy payload holding a region keeps its loan until its last use.
    assert_enum_execution(
        "generic-enum-region-payload.doria",
        r#"
use Doria\Std\Io\ByteRegion;
enum Optional<T> { case None; case Some(T $value); }
function first(Optional<ByteRegion> $value): uint8 {
    return match ($value) { Optional::None => 0, Optional::Some($region) => $region[0], };
}
function main(): void {
    let writable $bytes = Bytes::allocate(2);
    $bytes[0] = 7;
    {
        let $wrapped = Optional::Some($bytes->getRegion(0, 1));
        echo first($wrapped);
    }
    $bytes[0] = 9;
    echo ";" . $bytes[0];
}
"#,
        "7;9",
    );
}

#[test]
fn generic_payload_specializations_keep_concrete_enum_loan_rules() {
    // A specialization applies the concrete enum rules after substitution.
    // Payloads transport source loans; borrow-bound closures stay gated.
    let optional = "enum Optional<T> { case None; case Some(T $value); }";
    diagnostic(
        &format!("{optional} function wrap<T>(take T $value): Optional<T> {{ return Optional::Some($value); }} function main(): void {{ let $base = 40; let $callback = fn() with ($base) => $base + 2; let $wrapped = wrap($callback); }}"),
        "E0658",
    );
    diagnostic(
        &format!("{optional} function main(): void {{ let $base = 40; let $wrapped = Optional::Some(fn() with ($base) => $base + 2); }}"),
        "E0658",
    );
    assert!(findings(&format!("use Doria\\Std\\Io\\ByteRegion; {optional} function main(): void {{ let $bytes = Bytes::allocate(2); let $view = $bytes->getRegion(0, 1); let $wrapped = Optional::Some($view); }}")).is_empty());
    diagnostic(
        &format!("use Doria\\Std\\Io\\ByteRegion; {optional} function escape(): Optional<ByteRegion> {{ let $bytes = Bytes::allocate(2); return Optional::Some($bytes->getRegion(0, 1)); }} function main(): void {{}}"),
        "E0762",
    );
    diagnostic(
        &format!("use Doria\\Std\\Io\\ByteRegion; {optional} function main(): void {{ let writable $bytes = Bytes::allocate(2); let $wrapped = Optional::Some($bytes->getRegion(0, 1)); $bytes[0] = 9; let $again = $wrapped; }}"),
        "E0763",
    );
    // Move-ness is per application: a Copy instance can be reused, a Move
    // instance of the same declaration cannot.
    assert!(findings(&format!("{optional} function main(): void {{ let $a = Optional::Some(1); let $b = $a; echo match ($a) {{ Optional::None => 0, Optional::Some($x) => $x, }}; }}")).is_empty());
    diagnostic(
        &format!("{optional} class Token {{ function __construct(int $id) {{}} }} function main(): void {{ let $a = Optional::Some(new Token(1)); let $b = $a; echo match ($a) {{ Optional::None => 0, Optional::Some($x) => $x->id, }}; }}"),
        "E0470",
    );
    // `take` in a generic body is required for move specializations and
    // compiles as an ordinary match for Copy ones.
    assert!(
        findings(&format!("{optional} function orElse<T>(take Optional<T> $value, take T $fallback): T {{ return match (take $value) {{ Optional::None => $fallback, Optional::Some($inner) => $inner, }}; }}")).is_empty()
    );
}
