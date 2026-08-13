use doriac::diagnostics::Diagnostic;

fn diagnostics(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("stage28.doria", source).expect_err("source should be rejected")
}

fn assert_code(source: &str, code: &str) {
    let found = diagnostics(source);
    assert!(
        found.iter().any(|diagnostic| diagnostic.code == code),
        "expected {code}, got {found:#?}"
    );
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let mir = doriac::lower_source_to_mir("stage28.doria", source)
        .expect("Stage 28 source should lower through shared MIR");
    doriac::mir_interpreter::interpret(&mir).expect("Stage 28 MIR should execute")
}

#[test]
fn unit_payload_and_case_only_patterns_execute_with_arm_local_bindings() {
    let output = interpret(
        r#"
enum State { case Draft; case Ready; case Done; }
enum Result { case Waiting; case Sent(string $reference); case Failed(int $code, string $reason); }

function stateName(State $state): string
{
    return match ($state) {
        State::Draft => "draft",
        State::Ready => "ready",
        State::Done => "done",
    };
}

function resultName(Result $result): string
{
    return match ($result) {
        Result::Waiting => "waiting",
        Result::Sent($id) => "sent {$id}",
        Result::Failed($code, $reason) => "failed {$code}: {$reason}",
    };
}

function resultKind(Result $result): string
{
    return match ($result) {
        Result::Waiting => "waiting",
        Result::Sent => "sent",
        Result::Failed => "failed",
    };
}

function main(): void
{
    echo stateName(State::Draft) . " " . stateName(State::Ready) . " " . stateName(State::Done) . "\n";
    echo resultName(Result::Sent("R-12")) . "\n";
    echo resultName(Result::Failed(503, "offline")) . "\n";
    echo resultKind(Result::Sent("ignored"));
}
"#,
    );
    assert_eq!(
        output.stdout,
        b"draft ready done\nsent R-12\nfailed 503: offline\nsent"
    );

    assert_code(
        r#"
enum Result { case Value(int $value); }
function main(): void
{
    Result $result = Result::Value(42);
    int $selected = match ($result) { Result::Value($value) => $value, };
    echo $value;
}
"#,
        "E0101",
    );
}

#[test]
fn writable_payload_enum_locals_can_replace_their_active_case() {
    let output = interpret(
        r#"
enum DeliveryStatus
{
    case Pending;
    case InTransit(string $city);
    case Delivered(string $recipient);
}

function describe(DeliveryStatus $status): string
{
    return match ($status) {
        DeliveryStatus::Pending => "waiting to leave",
        DeliveryStatus::InTransit($city) => "in transit through {$city}",
        DeliveryStatus::Delivered($recipient) => "delivered to {$recipient}",
    };
}

function main(): void
{
    let writable $status = DeliveryStatus::InTransit("Lusaka");
    echo describe($status) . "\n";
    $status = DeliveryStatus::Delivered("Kitwe");
    echo describe($status) . "\n";
}
"#,
    );
    assert_eq!(
        output.stdout,
        b"in transit through Lusaka\ndelivered to Kitwe\n"
    );
}

#[test]
fn match_evaluates_one_scrutinee_and_only_the_selected_arm() {
    let output = interpret(
        r#"
enum State { case Draft; case Ready; }
function selectedState(): State { echo "scrutinee "; return State::Ready; }
function selected(string $name): string { echo "arm {$name} "; return $name; }
function main(): void
{
    echo match (selectedState()) {
        State::Draft => selected("draft"),
        State::Ready => selected("ready"),
    };
}
"#,
    );
    assert_eq!(output.stdout, b"scrutinee arm ready ready");
}

#[test]
fn payload_pattern_shape_and_enum_identity_are_checked() {
    assert_code(
        "enum Result { case Pair(int $left, int $right); } function main(): void { Result $r = Result::Pair(1, 2); int $v = match ($r) { Result::Pair($left) => $left, }; }",
        "E0590",
    );
    assert_code(
        "enum Result { case Pair(int $left, int $right); } function main(): void { Result $r = Result::Pair(1, 2); int $v = match ($r) { Result::Pair($item, $item) => $item, }; }",
        "E0103",
    );
    assert_code(
        "enum Left { case Ready; } enum Right { case Ready; } function main(): void { Left $v = Left::Ready; string $s = match ($v) { Right::Ready => \"wrong\", }; }",
        "E0588",
    );
}

#[test]
fn exhaustiveness_covers_finite_nullable_and_open_domains() {
    doriac::check_source(
        "stage28.doria",
        r#"
enum State { case Draft; case Ready; }
class Document {}
function boolName(bool $value): string { return match ($value) { true => "yes", false => "no", }; }
function maybeBool(?bool $value): string { return match ($value) { null => "none", true => "yes", false => "no", }; }
function maybeState(?State $value): string { return match ($value) { null => "none", State::Draft => "draft", State::Ready => "ready", }; }
function maybeDocument(?Document $value): string { return match ($value) { null => "none", Document $document => "document", }; }
function intName(int $value): string { return match ($value) { 42 => "answer", default => "other", }; }
function stringName(string $value): string { return match ($value) { "yes" => "yes", default => "other", }; }
function floatName(float $value): string { return match ($value) { 1.5 => "one", default => "other", }; }
function dynamicName(mixed $value): string { return match ($value) { int $number => "int", default => "other", }; }
"#,
    )
    .expect("finite and open exhaustive match forms should pass");

    assert_code(
        "enum State { case Draft; case Ready; } function f(State $v): string { return match ($v) { State::Draft => \"draft\", }; }",
        "E0585",
    );
    assert_code(
        "function f(bool $v): string { return match ($v) { true => \"yes\", }; }",
        "E0585",
    );
    assert_code(
        "function f(?bool $v): string { return match ($v) { true => \"yes\", false => \"no\", }; }",
        "E0585",
    );
    assert_code(
        "function f(int $v): string { return match ($v) { 42 => \"answer\", }; }",
        "E0585",
    );
    assert_code(
        "function f(mixed $v): string { return match ($v) { int $n => \"int\", string $s => \"string\", }; }",
        "E0585",
    );
}

#[test]
fn duplicate_unreachable_and_incompatible_patterns_are_rejected() {
    assert_code(
        "function f(bool $v): string { return match ($v) { true => \"a\", true => \"b\", false => \"c\", }; }",
        "E0586",
    );
    assert_code(
        "function f(int $v): string { return match ($v) { default => \"a\", 1 => \"b\", }; }",
        "E0589",
    );
    assert_code(
        "function f(bool $v): string { return match ($v) { true => \"a\", false => \"b\", default => \"c\", }; }",
        "E0589",
    );
    assert_code(
        "function f(int $v): string { return match ($v) { \"one\" => \"a\", default => \"b\", }; }",
        "E0588",
    );
    assert_code(
        "function f(int $v): string { return match ($v) { null => \"a\", default => \"b\", }; }",
        "E0588",
    );
}

#[test]
fn literal_constants_and_runtime_pattern_boundaries_are_enforced() {
    let output = interpret(
        r#"
const int ANSWER = 42;
class Codes { const string READY = "ready"; }
function integer(int $value): string { return match ($value) { ANSWER => "answer", default => "other", }; }
function text(string $value): string { return match ($value) { Codes::READY => "ready", default => "other", }; }
function decimal(float $value): string { return match ($value) { 1.5 => "one", default => "other", }; }
function main(): void { echo integer(42) . " " . text("ready") . " " . decimal(1.5); }
"#,
    );
    assert_eq!(output.stdout, b"answer ready one");

    assert_code(
        "function load(): int { return 42; } function f(int $v): string { return match ($v) { load() => \"loaded\", default => \"other\", }; }",
        "E0587",
    );

    doriac::check_source(
        "stage28.doria",
        r#"
function signed(int8 $value): string { return match ($value) { -1 => "minus", default => "other", }; }
function unsigned(uint8 $value): string { return match ($value) { 255 => "max", default => "other", }; }
"#,
    )
    .expect("integer patterns should preserve contextual width and signedness");
    assert_code(
        "function f(uint8 $v): string { return match ($v) { -1 => \"bad\", default => \"other\", }; }",
        "E0417",
    );
}

#[test]
fn exact_type_patterns_narrow_mixed_and_nullable_values() {
    let output = interpret(
        r#"
enum State { case Ready; }
class Document { function __construct(int $id) {} }

function inspect(mixed $value): string
{
    return match ($value) {
        int $number => "int {$value}",
        string $text => "string {$text}",
        State $state => "state",
        Document $document => "document {$document->id}",
        default => "other",
    };
}

function maybe(?Document $value): string
{
    return match ($value) {
        null => "missing",
        Document $present => "document {$present->id}",
    };
}

function main(): void
{
    echo inspect(42) . "\n" . inspect("text") . "\n" . inspect(State::Ready) . "\n";
    echo inspect(new Document(7)) . "\n" . maybe(null) . " " . maybe(new Document(9));
}
"#,
    );
    assert_eq!(
        output.stdout,
        b"int 42\nstring text\nstate\ndocument 7\nmissing document 9"
    );

    assert_code(
        "function f(int $v): string { return match ($v) { int $same => \"same\", }; }",
        "E0589",
    );
    assert_code(
        "function f(mixed $v): string { return match ($v) { mixed $same => \"same\", default => \"other\", }; }",
        "E0588",
    );
}

#[test]
fn match_true_is_strict_ordered_lazy_and_requires_default() {
    let output = interpret(
        r#"
function condition(string $name, bool $value): bool { echo $name; return $value; }
function choose(): string
{
    return match (true) {
        condition("a", false) => "A",
        condition("b", true) => "B",
        condition("c", true) => "C",
        default => "D",
    };
}
function main(): void { echo choose(); }
"#,
    );
    assert_eq!(output.stdout, b"abB");

    assert_code(
        "function f(int $v): string { return match (true) { $v => \"bad\", default => \"other\", }; }",
        "E0594",
    );
    assert_code(
        "function f(bool $v): string { return match (true) { $v => \"yes\", }; }",
        "E0585",
    );
}

#[test]
fn arm_results_share_one_strict_type_with_nullable_unification() {
    doriac::check_source(
        "stage28.doria",
        "function f(bool $v): ?string { return match ($v) { true => \"yes\", false => null, }; }",
    )
    .expect("string and null arms should unify to nullable string");
    assert_code(
        "function f(bool $v): string { return match ($v) { true => \"yes\", false => 1, }; }",
        "E0403",
    );
    assert_code(
        "function f(bool $v) { return match ($v) { true => null, false => null, }; }",
        "E0592",
    );
    assert_code(
        "function nothing(): void {} function f(bool $v): int { return match ($v) { true => nothing(), false => 1, }; }",
        "E0593",
    );
}

#[test]
fn surrounding_expected_types_reach_match_arms_in_every_user_call_form() {
    doriac::check_source(
        "stage28.doria",
        r#"
class Sink
{
    mixed $property = match (1 == 2) { true => 1, false => "text", };

    function __construct(take mixed $value) {}
    function accept(mixed $value): void {}
    static function acceptStatic(mixed $value): void {}
}

function accept(mixed $value): void {}
function mixedResult(bool $condition): mixed
{
    return match ($condition) { true => 1, false => "text", };
}
function nullResult(bool $condition): ?string
{
    return match ($condition) { true => null, false => null, };
}
enum State { case First; case Second; }
class Box {}

function main(): void
{
    bool $condition = true;
    mixed $local = match ($condition) { true => 1, false => "text", };
    int $left, $right = match ($condition) { true => 20, false => 22, };
    writable mixed $assigned = 0;
    $assigned = match (false) { true => 1, false => "text", };
    accept(match (false) { true => 1, false => "text", });
    let $sink = new Sink(match (false) { true => 1, false => "text", });
    $sink->accept(match (false) { true => 1, false => "text", });
    Sink::acceptStatic(match (false) { true => 1, false => "text", });
    mixed $nested = match ($condition) {
        true => match (false) { true => 1, false => "nested", },
        false => false,
    };
    mixed $ternary = true ? match (false) { true => 1, false => "ternary", } : false;
    State $state = match ($condition) {
        true => match (false) { true => State::First, false => State::Second, },
        false => State::First,
    };
    Box $box = match ($condition) { true => new Box(), false => new Box(), };
    mixed $mixed = mixedResult(true);
    ?string $nullable = nullResult(false);
}
"#,
    )
    .expect("every declared destination and user-call parameter should contextualize match arms");
}

#[test]
fn copy_pattern_bindings_mask_moved_outer_bindings_without_changing_outer_state() {
    let valid = r#"
class Box {}
function consume(take Box $value): void {}
function main(): void
{
    let $value = new Box();
    consume($value);
    mixed $subject = 42;
    int $selected = match ($subject) {
        int $value => $value,
        default => 0,
    };
    echo $selected;
}
"#;
    doriac::check_source("stage28.doria", valid)
        .expect("a Copy pattern binding should hide the moved outer binding inside its arm");

    let invalid = valid.replace("echo $selected;", "echo $selected; consume($value);");
    let diagnostics = diagnostics(&invalid);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"),
        "the outer binding must remain moved after the arm scope: {diagnostics:#?}"
    );
}

#[test]
fn named_and_temporary_move_scrutinees_are_borrowed_through_the_selected_arm() {
    let output = interpret(
        r#"
class Document { function __construct(int $id) {} }
enum LoadResult { case Loaded(Document $document); case Missing; }
function load(int $id): LoadResult { return LoadResult::Loaded(new Document($id)); }
function label(LoadResult $result): string
{
    string $text = match ($result) {
        LoadResult::Loaded($document) => "loaded {$document->id}",
        LoadResult::Missing => "missing",
    };
    bool $stillUsable = $result == LoadResult::Missing;
    return $text;
}
function main(): void { LoadResult $result = load(7); echo label($result) . " " . label(load(9)); }
"#,
    );
    assert_eq!(output.stdout, b"loaded 7 loaded 9");

    assert!(
        doriac::check_source(
            "stage28.doria",
            r#"
class Document {}
enum LoadResult { case Loaded(Document $document); case Missing; }
function escape(LoadResult $result): Document
{
    return match ($result) {
        LoadResult::Loaded($document) => $document,
        LoadResult::Missing => new Document(),
    };
}
"#,
        )
        .is_err(),
        "a borrowed move payload must not escape its scrutinee"
    );
}

#[test]
fn match_and_ternary_remain_bounded_inside_loops() {
    let output = interpret(
        r#"
enum Counter { case Even(int $value); case Odd(int $value); }
function main(): void
{
    let writable $index = 0;
    let writable $total = 0;
    while ($index < 1000) {
        Counter $counter = $index % 2 == 0 ? Counter::Even($index) : Counter::Odd($index);
        $total += match ($counter) {
            Counter::Even($value) => $value,
            Counter::Odd($value) => match (true) {
                $value > 0 => $value,
                default => 0,
            },
        };
        $index++;
    }
    echo $total;
}
"#,
    );
    assert_eq!(output.stdout, b"499500");
}

#[test]
fn ternary_is_right_associative_strict_lazy_and_rejects_elvis() {
    let output = interpret(
        r#"
function selected(string $name): string { echo $name; return $name; }
function main(): void
{
    bool $first = false;
    bool $second = true;
    echo $first ? selected("a") : $second ? selected("b") : selected("c");
}
"#,
    );
    assert_eq!(output.stdout, b"bb");

    assert_code(
        "function f(int $value): string { return $value ? \"yes\" : \"no\"; }",
        "E0595",
    );
    let elvis = doriac::parse_source(
        "stage28.doria",
        "function f(?string $value): string { return $value ?: \"fallback\"; }",
    )
    .expect_err("short ternary must be rejected by the parser");
    assert!(elvis.iter().any(|diagnostic| {
        diagnostic.message.contains("short ternary")
            && diagnostic.message.contains("`??`")
            && diagnostic.message.contains("full `? :`")
    }));
}

#[test]
fn pattern_guards_use_if_require_bool_and_reject_redundant_positions() {
    let output = interpret(
        r#"
enum State { case Ready(int $value); case Waiting; }
function mark(string $name, bool $result): bool { echo $name; return $result; }
function main(): void
{
    State $state = State::Ready(42);
    echo match ($state) {
        State::Waiting if mark("x", true) => "wrong",
        State::Ready($value) if mark("a", false) => "wrong",
        State::Ready($value) if mark("b", $value == 42) => "{$value}",
        State::Ready($value) => "fallback",
        State::Waiting => "waiting",
    };
}
"#,
    );
    assert_eq!(output.stdout, b"ab42");

    for keyword in ["when", "where"] {
        let source = format!(
            "enum State {{ case Ready; }} function f(State $state, bool $enabled): string {{ return match ($state) {{ State::Ready {keyword} $enabled => \"ready\", State::Ready => \"other\", }}; }}"
        );
        let diagnostics = doriac::parse_source("stage28.doria", &source)
            .expect_err("only `if` is a Doria match guard");
        assert_eq!(diagnostics.len(), 1, "{keyword}: {diagnostics:#?}");
        assert_eq!(diagnostics[0].code, "E0596");
        assert!(diagnostics[0].message.contains("use `if`"));
        assert_eq!(diagnostics[0].fixes[0].edits[0].replacement, "if");
    }

    assert_code(
        "enum State { case Ready; } function f(State $state): string { return match ($state) { State::Ready if 1 => \"ready\", State::Ready => \"other\", }; }",
        "E0597",
    );
    assert_code(
        "function f(int $value, bool $enabled): string { return match ($value) { default if $enabled => \"value\", }; }",
        "E0598",
    );
    assert_code(
        "function f(bool $enabled): string { return match (true) { $enabled if $enabled => \"yes\", default => \"no\", }; }",
        "E0599",
    );
}

#[test]
fn guarded_patterns_preserve_coverage_and_reachability_rules() {
    assert_code(
        "enum State { case Ready; case Waiting; } function f(State $state, bool $enabled): string { return match ($state) { State::Ready if $enabled => \"ready\", State::Waiting => \"waiting\", }; }",
        "E0585",
    );
    doriac::check_source(
        "stage28.doria",
        "enum State { case Ready; case Waiting; } function f(State $state, bool $first, bool $second): string { return match ($state) { State::Ready if $first => \"first\", State::Ready if $second => \"second\", State::Ready => \"ready\", State::Waiting => \"waiting\", }; }",
    )
    .expect("a later unguarded arm should complete guarded coverage");
    assert_code(
        "enum State { case Ready; } function f(State $state, bool $enabled): string { return match ($state) { State::Ready => \"ready\", State::Ready if $enabled => \"other\", }; }",
        "E0586",
    );
    assert_code(
        "enum State { case Ready; } function f(State $state): string { return match ($state) { State::Ready if true => \"ready\", }; }",
        "E0601",
    );
    assert_code(
        "enum State { case Ready; } function f(State $state): string { return match ($state) { State::Ready if false => \"ready\", State::Ready => \"other\", }; }",
        "E0589",
    );
}

#[test]
fn consuming_match_moves_whole_values_and_materializes_payloads_after_guards() {
    let output = interpret(
        r#"
class Document { function __construct(string $name) {} }
enum LoadResult { case Loaded(Document $document); case Missing; }
function select(take LoadResult $result): Document
{
    return match (take $result) {
        LoadResult::Loaded($document) if $document->name == "cached" => $document,
        LoadResult::Loaded($document) => $document,
        LoadResult::Missing => new Document("fallback"),
    };
}
function main(): void
{
    LoadResult $result = LoadResult::Loaded(new Document("ready"));
    let $document = select($result);
    echo $document->name;
}
"#,
    );
    assert_eq!(output.stdout, b"ready");

    assert_code(
        "enum State { case Ready; } function main(): void { State $state = State::Ready; string $name = match (take $state) { State::Ready => \"ready\", }; }",
        "E0600",
    );
    assert_code(
        "class Box {} enum Result { case Value(Box $box); } function main(): void { Result $result = Result::Value(new Box()); Box $box = match (take $result) { Result::Value($box) => $box, }; Result $again = $result; }",
        "E0470",
    );
}

#[test]
fn consuming_nullable_and_mixed_matches_execute_without_hidden_aliases() {
    let output = interpret(
        r#"
class Document { function __construct(string $name) {} }
function fromNullable(take ?Document $value): Document
{
    return match (take $value) { null => new Document("none"), Document $present => $present, };
}
function fromMixed(take mixed $value): Document
{
    return match (take $value) { Document $present => $present, default => new Document("other"), };
}
function main(): void
{
    ?Document $none = null;
    mixed $boxed = new Document("mixed");
    echo fromNullable($none)->name . " " . fromMixed($boxed)->name;
}
"#,
    );
    assert_eq!(output.stdout, b"none mixed");
}

#[test]
fn payload_level_take_and_writable_patterns_are_deliberately_rejected() {
    for (source, code) in [
        (
            "class Box {} enum Result { case Value(Box $box); } function f(Result $result): Box { return match ($result) { Result::Value(take $box) => $box, }; }",
            "E0603",
        ),
        (
            "enum Result { case Value(int $value); } function f(Result $result): int { return match (writable $result) { Result::Value($value) => $value, }; }",
            "E0602",
        ),
        (
            "enum Result { case Value(int $value); } function f(Result $result): int { return match ($result) { Result::Value(writable $value) => $value, }; }",
            "E0604",
        ),
    ] {
        let diagnostics = doriac::parse_source("stage28.doria", source)
            .expect_err("the rejected v1 pattern form must produce a parser diagnostic");
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code == code),
            "expected {code}, got {diagnostics:#?}"
        );
    }
}
