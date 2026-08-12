use doriac::diagnostics::Diagnostic;
use doriac::lexer::{Lexer, TokenKind};
use doriac::source::SourceFile;
use doriac::{ast, ast::Item};

fn diagnostics(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("stage27.doria", source).expect_err("source should be rejected")
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

function main(): void
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

function main(): void
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

function main(): void
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

function main(): void
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

function show(Status $status = Status::Draft): void
{
    echo $status == DEFAULT_STATUS;
}

function main(): void
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
    let generic = diagnostic(
        "enum Optional<T> { case None; case Some(T $value); }",
        "E0572",
    );
    assert_eq!(generic.title, "Generic Enums Are Not Implemented");

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
    let ast::Stmt::VarDecl(binding) = &main.body.statements[0] else {
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

function mark(string $name, int $value): int
{
    echo $name;
    return $value;
}

function defaulted(Label $label = Label::Text("default")): bool
{
    return $label == DEFAULT_LABEL;
}

function main(): void
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
fn payload_ownership_layout_equality_and_observation_boundaries_are_checked() {
    let moved = diagnostics(
        r#"
class Document {}
enum LoadResult { case Loaded(Document $document); }
function main(): void
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
function main(): void
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
function main(): void
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
