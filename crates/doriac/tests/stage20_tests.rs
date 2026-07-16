use doriac::ast::{ClassMember, Item, MemberAccess, StaticQualifier};
use doriac::const_eval::{ConstKey, ConstValue};
use doriac::mir;

const PARSER_EXAMPLE: &str = include_str!("../../../examples/native/main_stage20_parser.doria");
const DISPLAYABLE_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage20_displayable.doria");
const STATICS_EXAMPLE: &str = include_str!("../../../examples/native/main_stage20_statics.doria");
const SELF_EXAMPLE: &str = include_str!("../../../examples/native/main_stage20_self.doria");
const STATIC_CONSTRUCTOR_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage20_static_constructor.doria");
const PROPERTY_READ_MODIFY_WRITE_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage20_property_read_modify_write.doria");

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage20.doria", source).expect_err("source should be rejected")
}

fn assert_diagnostic(source: &str, code: &str) {
    let found = diagnostics(source);
    assert!(
        found.iter().any(|diagnostic| diagnostic.code == code),
        "expected {code}, got {found:#?}"
    );
}

fn diagnostic_snapshot(source: &str, code: &str) -> String {
    let diagnostic = diagnostics(source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"));
    let mut snapshot = format!(
        "code: {}\nmessage: {}\nhelp: {}\nspan: {}..{}\n",
        diagnostic.code,
        diagnostic.message,
        diagnostic.help.as_deref().unwrap_or(""),
        diagnostic.span.start,
        diagnostic.span.end,
    );
    if let Some(fix) = diagnostic.fix {
        snapshot.push_str(&format!(
            "fix: {}..{} -> {:?}\n",
            fix.span.start, fix.span.end, fix.replacement
        ));
    }
    for related in diagnostic.related {
        snapshot.push_str(&format!(
            "related: {}..{}: {}\n",
            related.span.start, related.span.end, related.message
        ));
    }
    snapshot
}

fn lower(source: &str) -> mir::Program {
    doriac::lower_source_to_mir("stage20.doria", source).expect("source should lower to MIR")
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    doriac::mir_interpreter::interpret(&lower(source)).expect("MIR should interpret")
}

#[test]
fn parses_constants_static_members_and_explicit_method_identity() {
    let program = doriac::parse_source(
        "surface.doria",
        r#"
const int TOP_LIMIT = 40;

class Counter
{
    internal const STEP = TOP_LIMIT / 20;
    static int $initial = TOP_LIMIT;
    internal static writable int $value = Counter::initial;

    static function read(): int { return Counter::value; }
    writable function increment(): void { return; }
}
"#,
    )
    .expect("Stage 20 declarations should parse");

    assert!(matches!(program.items.first(), Some(Item::Constant(_))));
    let class = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .expect("Counter class");
    assert!(class.members.iter().any(|member| matches!(
        member,
        ClassMember::Constant(constant)
            if constant.name == "STEP" && constant.access == MemberAccess::Internal
    )));
    assert!(class.members.iter().any(|member| matches!(
        member,
        ClassMember::Property(property)
            if property.name == "value" && property.is_static && property.writable
    )));
    assert!(class.members.iter().any(|member| matches!(
        member,
        ClassMember::Method(method)
            if method.name == "read" && method.is_static && !method.writable_this
    )));
    assert!(class.members.iter().any(|member| matches!(
        member,
        ClassMember::Method(method)
            if method.name == "increment" && !method.is_static && method.writable_this
    )));
}

#[test]
fn evaluates_typed_inferred_and_forward_constants_without_runtime_storage() {
    let hir = doriac::lower_source(
        "constants.doria",
        r#"
const int ANSWER = LATER + 1;
const LATER = 41;
const string LABEL = "Dor" . "ia";

class Limits
{
    const DOUBLE = ANSWER * 2;
}

function main(): void
{
    echo LABEL;
    echo Limits::DOUBLE;
}
"#,
    )
    .expect("forward constants should lower");
    let values = &hir.semantic_info.const_evaluation.values;
    let answer = &values[&ConstKey::TopLevel("ANSWER".to_string())].value;
    let later = &values[&ConstKey::TopLevel("LATER".to_string())].value;
    let doubled = &values[&ConstKey::Class {
        class_name: "Limits".to_string(),
        name: "DOUBLE".to_string(),
    }]
        .value;
    assert!(matches!(answer, ConstValue::Integer(value) if value.signed_value() == 42));
    assert!(matches!(later, ConstValue::Integer(value) if value.signed_value() == 41));
    assert!(matches!(doubled, ConstValue::Integer(value) if value.signed_value() == 84));

    let mir = doriac::mir_lowering::lower_program(&hir).expect("constants should fold into MIR");
    assert!(
        mir.statics.is_empty(),
        "constants must not allocate static storage"
    );
    let output = doriac::mir_interpreter::interpret(&mir).expect("folded constants should run");
    assert_eq!(output.stdout, b"Doria84");
}

#[test]
fn constant_evaluation_preserves_runtime_expression_semantics() {
    let hir = doriac::lower_source(
        "constant-semantics.doria",
        r#"
const bool SHORT_AND = false && (1 / 0 == 0);
const bool SHORT_OR = true || (1 / 0 == 0);
const int8 MIN_INT8 = -128;
const int MIN_INT = -9223372036854775808;
const string LABEL = "v" . 1 . ":" . true . ":" . 1.5;
"#,
    )
    .expect("valid constant expressions should evaluate");
    let values = &hir.semantic_info.const_evaluation.values;

    assert!(matches!(
        values[&ConstKey::TopLevel("SHORT_AND".to_string())].value,
        ConstValue::Bool(false)
    ));
    assert!(matches!(
        values[&ConstKey::TopLevel("SHORT_OR".to_string())].value,
        ConstValue::Bool(true)
    ));
    assert!(matches!(
        values[&ConstKey::TopLevel("MIN_INT8".to_string())].value,
        ConstValue::Integer(value) if value.signed_value() == -128
    ));
    assert!(matches!(
        values[&ConstKey::TopLevel("MIN_INT".to_string())].value,
        ConstValue::Integer(value) if value.signed_value() == i64::MIN as i128
    ));
    assert!(matches!(
        &values[&ConstKey::TopLevel("LABEL".to_string())].value,
        ConstValue::String(value) if value == "v1:true:1.5"
    ));
}

#[test]
fn rejects_invalid_constant_dependencies_operations_and_names() {
    let cases = [
        ("const FIRST = SECOND; const SECOND = FIRST;", "E0482"),
        ("const int8 TOO_LARGE = 127 + 1;", "E0485"),
        (
            "function runtime(): int { return 1; } const VALUE = runtime();",
            "E0485",
        ),
        ("const int VALUE = \"wrong\";", "E0484"),
        ("const not_upper = 1;", "E0490"),
        ("const VALUE = 1; const VALUE = 2;", "E0481"),
        (
            "const HUGE = 999999999999999999999999999999999999999999999999999999999999999999999999999999999999;",
            "E0417",
        ),
    ];
    for (source, code) in cases {
        assert_diagnostic(source, code);
    }

    let cycle = diagnostics("const FIRST = SECOND; const SECOND = FIRST;");
    assert!(cycle.iter().any(
        |diagnostic| diagnostic.message.contains("FIRST -> SECOND -> FIRST")
            || diagnostic.message.contains("SECOND -> FIRST -> SECOND")
    ));
}

#[test]
fn constant_initializers_use_normal_static_access_diagnostics() {
    let cases = [
        ("const VALUE = self::OTHER;", "E0492"),
        ("class Base { const VALUE = parent::OTHER; }", "E0496"),
        ("class Base { const VALUE = static::OTHER; }", "E0495"),
        ("class Base { static int $value = static::OTHER; }", "E0495"),
        (
            "class Base { const OTHER = 1; const VALUE = Base::$OTHER; }",
            "E0494",
        ),
    ];
    for (source, code) in cases {
        assert_diagnostic(source, code);
    }
}

#[test]
fn static_methods_reject_receiver_mutability() {
    let found = diagnostics("class Counter { static writable function increment(): void {} }");
    assert!(found.iter().any(|diagnostic| {
        diagnostic.code == "E0497" && diagnostic.message.contains("have no `$this` receiver")
    }));
}

#[test]
fn enforces_static_initialization_and_copy_type_rules() {
    doriac::check_source(
        "valid-statics.doria",
        r#"
class Counter
{
    static int $initial = 40;
    static writable int $value = Counter::initial + 2;
    static string $label = "ready";
}

function main(): void
{
    Counter::value = 43;
    echo Counter::initial;
    echo Counter::value;
    echo Counter::label;
}
"#,
    )
    .expect("Copy statics with const-evaluable initializers should be accepted");

    let independent = interpret(
        r#"
class Left { static writable int $value = 1; }
class Right { static writable int $value = 2; }
function main(): void
{
    Left::value = 3;
    echo Left::value;
    echo Right::value;
}
"#,
    );
    assert_eq!(independent.stdout, b"32");

    assert_diagnostic(
        "class Counter { static int $value = 1; } function main(): void { Counter::value = 2; }",
        "E0202",
    );
    assert_diagnostic(
        "class Item {} class Store { static Item $item = new Item(); }",
        "E0486",
    );

    let runtime = diagnostics(
        "function value(): int { return 1; } class Store { static int $value = value(); }",
    );
    assert!(runtime.iter().any(|diagnostic| {
        diagnostic.code == "E0485"
            && diagnostic
                .message
                .contains("runtime-initialized statics require a future accepted decision record")
    }));
    assert_diagnostic(
        "class Store { static int $left = Store::right; static int $right = Store::left; }",
        "E0482",
    );
    let mutable_dependency = diagnostics(
        "class Store { static writable int $source = 1; static int $copy = Store::source; }",
    );
    assert!(mutable_dependency.iter().any(|diagnostic| {
        diagnostic.code == "E0485"
            && diagnostic
                .message
                .contains("constant evaluation cannot read writable static `Store::source`")
    }));
}

#[test]
fn internal_access_is_limited_to_the_declaring_class_for_every_member_kind() {
    doriac::check_source(
        "same-class.doria",
        r#"
class Vault
{
    internal const CODE = 2;
    internal int $secret = 40;
    internal static int $offset = Vault::CODE;

    internal function reveal(): int { return $this->secret; }
    internal static function staticOffset(): int { return Vault::offset; }

    function total(): int
    {
        return $this->reveal() + Vault::staticOffset();
    }
}
"#,
    )
    .expect("a class may access its own internal members");

    for (source, code) in [
        (
            "class Vault { internal int $secret = 1; } function main(): void { let $vault = new Vault(); echo $vault->secret; }",
            "E0306",
        ),
        (
            "class Vault { internal function reveal(): int { return 1; } } function expose(Vault $vault): int { return $vault->reveal(); }",
            "E0307",
        ),
        (
            "class Vault { internal static function reveal(): int { return 1; } } function main(): void { echo Vault::reveal(); }",
            "E0307",
        ),
        (
            "class Vault { internal const CODE = 1; } class Other { function expose(): int { return Vault::CODE; } }",
            "E0307",
        ),
        (
            "class Vault { internal static int $value = 1; } function main(): void { echo Vault::value; }",
            "E0307",
        ),
        (
            "class Vault { internal function __construct() {} } function main(): void { let $vault = new Vault(); }",
            "E0307",
        ),
    ] {
        assert_diagnostic(source, code);
    }
}

#[test]
fn lifecycle_methods_remain_non_static_and_non_callable() {
    for (source, code) in [
        ("class Item { static function __construct() {} }", "E0465"),
        ("class Item { static function __destruct() {} }", "E0465"),
        (
            "class Item { function __construct() {} } function main(): void { Item::__construct(); }",
            "E0414",
        ),
        (
            "class Item { function __destruct() {} } function main(): void { let $item = new Item(); $item->__destruct(); }",
            "E0414",
        ),
    ] {
        let found = diagnostics(source);
        assert!(
            found.iter().any(|diagnostic| diagnostic.code == code),
            "expected lifecycle diagnostic, got {found:#?}"
        );
    }
}

#[test]
fn method_calls_support_recursion_class_returns_moves_and_deterministic_drops() {
    let source = r#"
class Token
{
    function __construct(string $name) {}
    function __destruct() { echo "drop " . $this->name . "\n"; }
}

class Worker
{
    function sum(int $value): int
    {
        if ($value <= 0) { return 0; }
        return $value + $this->sum($value - 1);
    }

    function make(string $name): Token { return new Token($name); }
    function relay(take Token $token): Token { return $token; }
    function inspect(Token $token): string { return $token->name; }

    function leaveEarly(): void
    {
        let $local = new Token("local");
        return;
    }
}

function main(): void
{
    let $worker = new Worker();
    let $token = $worker->relay($worker->make("owned"));
    echo $worker->sum(6);
    echo ":" . $worker->inspect($token) . "\n";
    $worker->leaveEarly();
}
"#;
    let output = interpret(source);
    assert_eq!(output.stdout, b"21:owned\ndrop local\ndrop owned\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    assert!(
        !doriac::codegen_cranelift::lower_mir_to_object(&lower(source))
            .expect("method ownership source should lower to Cranelift")
            .is_empty()
    );
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&lower(source))
        .expect("method ownership source should lower to LLVM")
        .is_empty());
}

#[test]
fn owned_property_replacement_remains_behind_the_writable_path_move_boundary() {
    assert_diagnostic(
        r#"
class Token {}

class Box
{
    writable Token $token = new Token();

    writable function replace(take Token $replacement): void
    {
        $this->token = $replacement;
    }
}
"#,
        "E0472",
    );
}

#[test]
fn property_initializers_may_call_internal_static_methods_of_the_declaring_class() {
    let source = r#"
class Message
{
    string $text = Message::defaultText();

    internal static function defaultText(): string
    {
        return "ready";
    }
}

function main(): void
{
    let $message = new Message();
    echo $message->text;
}
"#;
    let output = interpret(source);
    assert_eq!(output.stdout, b"ready");
    assert!(output.stderr.is_empty());
}

#[test]
fn panic_inside_a_method_keeps_the_accepted_no_cleanup_behavior() {
    let source = r#"
class Token
{
    function __destruct() { echo "unexpected cleanup"; }
}

class Runner
{
    function fail(): void
    {
        let $token = new Token();
        panic("method failed");
    }
}

function main(): void
{
    let $runner = new Runner();
    $runner->fail();
}
"#;
    let output = interpret(source);
    assert!(output.stdout.is_empty());
    assert_eq!(output.exit_status, 101);
    let stderr = String::from_utf8(output.stderr).expect("panic output is UTF-8");
    assert!(stderr.contains("Panic: method failed"));
    assert!(stderr.contains("Runner::fail"));
}

#[test]
fn writable_method_calls_require_writable_receivers() {
    let source = r#"
class Counter
{
    writable int $value = 0;
    writable function increment(): void { $this->value++; }
}

function main(): void
{
    let $counter = new Counter();
    $counter->increment();
}
"#;
    assert_diagnostic(source, "E0203");
}

#[test]
fn property_read_modify_write_requires_a_writable_numeric_place() {
    doriac::check_source(
        "property-read-modify-write.doria",
        PROPERTY_READ_MODIFY_WRITE_EXAMPLE,
    )
    .expect("writable numeric property and static places should be accepted");

    let cases = [
        (
            r#"
class Counter
{
    int $value = 0;
    writable function increment(): void { $this->value++; }
}
"#,
            "E0202",
        ),
        (
            r#"
class Counter
{
    writable int $value = 0;
    function increment(): void { $this->value++; }
}
"#,
            "E0201",
        ),
        (
            r#"
class Counter
{
    static int $value = 0;
    static function increment(): void { Counter::value++; }
}
"#,
            "E0202",
        ),
        (
            r#"
class Counter
{
    writable string $value = "zero";
    writable function increment(): void { $this->value++; }
}
"#,
            "E0423",
        ),
    ];
    for (source, code) in cases {
        assert_diagnostic(source, code);
    }
}

#[test]
fn property_read_modify_write_runs_in_the_mir_interpreter() {
    let output = interpret(PROPERTY_READ_MODIFY_WRITE_EXAMPLE);
    assert_eq!(output.stdout, b"3:2.5:3\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn mir_records_receiver_modes_and_rejects_malformed_method_calls() {
    let program = lower(PARSER_EXAMPLE);
    let create = program
        .functions
        .iter()
        .find(|function| function.name.ends_with("::create"))
        .expect("static create method");
    let parse = program
        .functions
        .iter()
        .find(|function| function.name.ends_with("::parse"))
        .expect("writable parse method");
    let parse_program = program
        .functions
        .iter()
        .find(|function| function.name.ends_with("::parseProgram"))
        .expect("readonly parseProgram method");
    assert_eq!(create.receiver_mode, None);
    assert_eq!(parse.receiver_mode, Some(mir::ReceiverMode::Writable));
    assert_eq!(
        parse_program.receiver_mode,
        Some(mir::ReceiverMode::Readonly)
    );

    let mut missing_receiver = program.clone();
    let parse = missing_receiver
        .functions
        .iter_mut()
        .find(|function| function.name.ends_with("::parse"))
        .expect("writable parse method");
    parse.params.remove(0);
    let error = doriac::mir_validation::validate_program(&missing_receiver)
        .expect_err("method without a receiver parameter must be rejected");
    assert!(error.message.contains("has no receiver parameter"));

    let mut readonly_receiver = program;
    let main = readonly_receiver
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("main function");
    let parser = main
        .locals
        .iter_mut()
        .find(|local| local.name == "parser")
        .expect("parser local");
    parser.writable = false;
    let error = doriac::mir_validation::validate_program(&readonly_receiver)
        .expect_err("writable method call through readonly MIR must be rejected");
    assert!(error.message.contains("requires a writable class value"));
}

#[test]
fn malformed_static_writes_are_rejected_by_shared_mir_validation() {
    let mut program = lower(STATICS_EXAMPLE);
    let value = program
        .statics
        .iter_mut()
        .find(|property| property.name == "value")
        .expect("writable value static");
    value.writable = false;
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("MIR cannot assign to a readonly static");
    assert!(error.message.contains("assignment targets readonly static"));
}

#[test]
fn stage20_acceptance_examples_run_through_the_shared_interpreter() {
    for (source, expected) in [
        (PARSER_EXAMPLE, b"Doria:parser\n".as_slice()),
        (DISPLAYABLE_EXAMPLE, b"L!R![LR]L!R!LRL!LL!R!LR\n".as_slice()),
        (STATICS_EXAMPLE, b"40:42:42:44:S:ready\n".as_slice()),
    ] {
        let output = interpret(source);
        assert_eq!(output.stdout, expected);
        assert!(output.stderr.is_empty());
        assert_eq!(output.exit_status, 0);
    }
}

#[test]
fn self_scope_and_type_forms_resolve_before_mir() {
    let ast = doriac::parse_source("self.doria", SELF_EXAMPLE).expect("self forms should parse");
    let counter = ast
        .items
        .iter()
        .find_map(|item| match item {
            Item::Class(class) if class.name == "Counter" => Some(class),
            _ => None,
        })
        .expect("Counter class");
    let next = counter
        .members
        .iter()
        .find_map(|member| match member {
            ClassMember::Method(method) if method.name == "next" => Some(method),
            _ => None,
        })
        .expect("next method");
    assert!(matches!(
        &next.body.statements[0],
        doriac::ast::Stmt::Assignment(doriac::ast::Assignment {
            target: doriac::ast::Expr::StaticMember {
                qualifier: StaticQualifier::SelfType,
                ..
            },
            ..
        })
    ));

    let hir = doriac::lower_source("self.doria", SELF_EXAMPLE).expect("self should lower");
    let message = hir
        .items
        .iter()
        .find_map(|item| match item {
            doriac::hir::Item::Class(class) if class.name == "Message" => Some(class),
            _ => None,
        })
        .expect("Message class");
    let with_name = message
        .members
        .iter()
        .find_map(|member| match member {
            doriac::hir::ClassMember::Method(method) if method.name == "withName" => Some(method),
            _ => None,
        })
        .expect("withName method");
    assert_eq!(
        with_name.return_type.as_ref().map(|ty| ty.name.as_str()),
        Some("Message")
    );

    let output = doriac::mir_interpreter::interpret(
        &doriac::mir_lowering::lower_program(&hir).expect("self should lower to MIR"),
    )
    .expect("self fixture should run");
    assert_eq!(
        output.stdout,
        b"2:3:second\nreleased:second\nreleased:first\n"
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);

    assert_diagnostic(
        "class Other {} class Message { function replace(): self { return new Other(); } }",
        "E0404",
    );
}

#[test]
fn self_access_preserves_internal_and_writable_rules() {
    doriac::check_source(
        "self-internal.doria",
        r#"
class Vault
{
    internal const STEP = 1;
    internal static writable int $value = 0;
    internal static function advance(): int
    {
        self::value = self::value + self::STEP;
        return self::value;
    }
    static function reveal(): int { return self::advance(); }
}
function main(): void { echo Vault::reveal(); }
"#,
    )
    .expect("self access inside the declaring class should reach internal members");

    assert_diagnostic(
        "class Counter { static int $value = 1; static function change(): void { self::value = 2; } }",
        "E0202",
    );
    assert_diagnostic(
        "class Vault { internal static function secret(): int { return 1; } } function main(): void { echo Vault::secret(); }",
        "E0307",
    );
}

#[test]
fn constructor_static_mutation_is_ordinary_mutation() {
    let output = interpret(STATIC_CONSTRUCTOR_EXAMPLE);
    assert_eq!(output.stdout, b"37\nmessage released\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    assert_diagnostic(
        "class Counter { static int $value = 1; function __construct() { Counter::value = 2; } }",
        "E0202",
    );
}

#[test]
fn static_access_identity_nevers_have_precise_machine_fixes() {
    let sigil_source = r#"
class Foo
{
    static int $prop = 1;
    function read(): int { return Foo::$prop; }
}
"#;
    let sigil = diagnostics(sigil_source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == "E0494")
        .expect("sigil-carrying access diagnostic");
    let dollar = sigil_source.rfind("$prop").expect("access sigil");
    assert_eq!(sigil.span, doriac::source::Span::new(dollar, dollar + 1));
    assert_eq!(sigil.fix.as_ref().map(|fix| fix.span), Some(sigil.span));
    assert_eq!(
        sigil.fix.as_ref().map(|fix| fix.replacement.as_str()),
        Some("")
    );

    let static_source = r#"
class Foo
{
    static function create(): int { return 1; }
    function read(): int { return static::create(); }
}
"#;
    let late_static = diagnostics(static_source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == "E0495")
        .expect("late-static-binding diagnostic");
    let qualifier = static_source.rfind("static::").expect("static qualifier");
    assert_eq!(
        late_static.fix.as_ref().map(|fix| fix.span),
        Some(doriac::source::Span::new(qualifier, qualifier + 6))
    );
    assert_eq!(
        late_static.fix.as_ref().map(|fix| fix.replacement.as_str()),
        Some("self")
    );
    assert!(!late_static.message.contains("Stage"));
}

#[test]
fn static_identity_diagnostics_match_snapshots() {
    let sigil = "class Foo { static int $prop = 1; function read(): int { return Foo::$prop; } }";
    assert_eq!(
        diagnostic_snapshot(sigil, "E0494"),
        include_str!("fixtures/diagnostics/stage20_static_property_sigil_fix.txt")
            .replace("\r\n", "\n")
    );

    let late_static = "class Foo { static function create(): int { return 1; } function read(): int { return static::create(); } }";
    assert_eq!(
        diagnostic_snapshot(late_static, "E0495"),
        include_str!("fixtures/diagnostics/stage20_late_static_binding_fix.txt")
            .replace("\r\n", "\n")
    );
}

#[test]
fn class_members_share_one_namespace_with_both_declaration_spans() {
    for (source, first_kind, second_kind) in [
        (
            "class Example { const FOO = 1; static int $FOO = 2; }",
            "class constant",
            "static property",
        ),
        (
            "class Example { string $bar = \"\"; function bar(): string { return $this->bar; } }",
            "instance property",
            "instance method",
        ),
        (
            "class Example { function bar(): string { return \"\"; } string $bar = \"\"; }",
            "instance method",
            "instance property",
        ),
        (
            "class Example { static int $value = 1; static function value(): int { return 1; } }",
            "static property",
            "static method",
        ),
        (
            "class Example { static function value(): int { return 1; } int $value = 1; }",
            "static method",
            "instance property",
        ),
        (
            "class Example { function value(): int { return 1; } const value = 1; }",
            "instance method",
            "class constant",
        ),
        (
            "class Example { string $__construct = \"\"; function __construct() {} }",
            "instance property",
            "instance method",
        ),
    ] {
        let duplicate = diagnostics(source)
            .into_iter()
            .find(|diagnostic| diagnostic.code == "E0481")
            .expect("duplicate member diagnostic");
        assert!(duplicate.message.contains(first_kind));
        assert!(duplicate.message.contains(second_kind));
        assert_eq!(duplicate.related.len(), 1);
        assert_ne!(duplicate.span, duplicate.related[0].span);
    }

    assert_eq!(
        diagnostic_snapshot(
            "class Example { const FOO = 1; static int $FOO = 2; }",
            "E0481"
        ),
        include_str!("fixtures/diagnostics/stage20_duplicate_constant_static_property.txt")
            .replace("\r\n", "\n")
    );
    assert_eq!(
        diagnostic_snapshot(
            "class Example { string $bar = \"\"; function bar(): string { return $this->bar; } }",
            "E0481"
        ),
        include_str!("fixtures/diagnostics/stage20_duplicate_property_method.txt")
            .replace("\r\n", "\n")
    );
}

#[test]
fn reserved_and_two_clock_qualifiers_are_structural_not_parser_errors() {
    let reserved_source = "class self {}";
    let reserved = diagnostics(reserved_source);
    assert!(reserved.iter().any(|diagnostic| {
        diagnostic.code == "E0309" && diagnostic.message.contains("reserved")
    }));
    assert_eq!(
        diagnostic_snapshot(reserved_source, "E0309"),
        include_str!("fixtures/diagnostics/stage20_reserved_self_class.txt").replace("\r\n", "\n")
    );

    let parent_source = "class Child { function save(): void { parent::save(); } }";
    let parsed_parent = doriac::parse_source("parent.doria", parent_source)
        .expect("generalized parent calls should parse");
    assert!(matches!(
        &parsed_parent.items[0],
        Item::Class(class)
            if matches!(
                &class.members[0],
                ClassMember::Method(method)
                    if matches!(
                        &method.body.statements[0],
                        doriac::ast::Stmt::Expr {
                            expr: doriac::ast::Expr::StaticCall {
                                qualifier: StaticQualifier::Parent,
                                ..
                            },
                            ..
                        }
                    )
            )
    ));
    let parent = diagnostics(parent_source);
    assert_eq!(
        parent
            .iter()
            .filter(|diagnostic| diagnostic.code == "P0001")
            .count(),
        0
    );
    assert!(parent.iter().any(|diagnostic| {
        diagnostic.code == "E0496" && diagnostic.message.contains("Stage 34")
    }));

    let trait_source = r#"
trait UsesLimit
{
    function limit(): int { return self::MAX_DEPTH; }
}
"#;
    let parsed = doriac::parse_source("trait.doria", trait_source)
        .expect("trait self access should parse structurally");
    assert!(matches!(
        parsed.items.first(),
        Some(Item::Trait(trait_decl))
            if matches!(
                &trait_decl.members[0],
                ClassMember::Method(method)
                    if matches!(
                        &method.body.statements[0],
                        doriac::ast::Stmt::Return {
                            expr: Some(doriac::ast::Expr::StaticMember {
                                qualifier: StaticQualifier::SelfType,
                                ..
                            }),
                            ..
                        }
                    )
            )
    ));
    let trait_diagnostics = diagnostics(trait_source);
    assert_eq!(
        trait_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "P0001")
            .count(),
        0
    );
    assert!(trait_diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0493" && diagnostic.message.contains("Stage 35")
    }));
}
