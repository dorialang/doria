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
        .expect("Stage 27 Slice 1 source should lower through shared MIR");
    doriac::mir_interpreter::interpret(&mir).expect("Stage 27 Slice 1 MIR should execute")
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
fn payload_and_match_syntax_stop_once_at_their_owned_semantic_boundaries() {
    let payload = diagnostics(
        r#"
enum Shape
{
    case Circle(float $radius);
    case Rect(float $width, float $height);
}
Shape $shape = Shape::Circle(2.5);
"#,
    );
    assert_eq!(payload.len(), 1, "{payload:#?}");
    assert_eq!(payload[0].code, "E0573");

    let match_diagnostics = diagnostics(
        r#"
enum Shape { case Circle(float $radius); }
let $area = match (true) {
    true => 1.0,
    default => 0.0,
};
"#,
    );
    assert_eq!(match_diagnostics.len(), 1, "{match_diagnostics:#?}");
    assert_eq!(match_diagnostics[0].code, "E0576");
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
        ast::MatchPattern::EnumCase { bindings, .. } if bindings.len() == 1
    ));
    assert!(matches!(arms[1].pattern, ast::MatchPattern::Expression(_)));
    assert!(matches!(arms[2].pattern, ast::MatchPattern::Default { .. }));
}

#[test]
fn accepted_pending_fixtures_parse_then_stop_once_in_semantics() {
    for (source, code) in [
        (include_str!("fixtures/stage27/payload_enum.doria"), "E0573"),
        (
            include_str!("fixtures/stage27/match_expression.doria"),
            "E0576",
        ),
    ] {
        doriac::parse_source("stage27.doria", source)
            .expect("accepted pending syntax should have no lexer or parser diagnostics");
        let found = diagnostics(source);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].code, code);
    }
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
