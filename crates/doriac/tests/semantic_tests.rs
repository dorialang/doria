fn assert_type_mismatch(source: &str) {
    assert_diagnostic_code(source, "E0403");
}

fn assert_diagnostic_code(source: &str, code: &str) {
    let err = doriac::check_source("test.doria", source).expect_err("semantic check should fail");

    assert!(
        err.iter().any(|diagnostic| diagnostic.code == code),
        "expected {code}, got {err:?}"
    );
}

#[test]
fn rejects_undeclared_assignment() {
    let err = doriac::check_source("test.doria", r#"$name = "Andrew";"#)
        .expect_err("semantic check should fail");

    assert_eq!(err[0].code, "E0101");
}

#[test]
fn rejects_readonly_variable_assignment() {
    let err = doriac::check_source(
        "test.doria",
        r#"
let $count = 0;
$count = 1;
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0201"));
}

#[test]
fn allows_writable_variable_assignment() {
    doriac::check_source(
        "test.doria",
        r#"
let writable $count = 0;
$count = 1;
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn allows_compatible_scalar_typed_declarations() {
    doriac::check_source(
        "test.doria",
        r#"
int $age = 37;
float $ratio = 1.5;
string $name = "Andrew";
bool $active = true;
mixed $payload = "dynamic boundary";
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn allows_values_to_flow_into_mixed() {
    doriac::check_source(
        "test.doria",
        r#"
class User {}

mixed $count = 1;
mixed $name = "Doria";
mixed $active = true;
mixed $nothing = null;
mixed $user = new User();
mixed $numbers = [1, 2, 3];

List<mixed> $items = [1, "two", new User()];
Dictionary<string, mixed> $byName = [
    "count" => 1,
    "name" => "Doria",
    "user" => new User(),
];
"#,
    )
    .expect("semantic check should allow values to flow into mixed");
}

#[test]
fn rejects_broad_array_type_spelling() {
    let err = doriac::check_source(
        "test.doria",
        r#"
array $items = [];
"#,
    )
    .expect_err("semantic check should reject the PHP-style broad array type");

    assert!(err
        .iter()
        .any(|diagnostic| { diagnostic.code == "E0401" && diagnostic.message.contains("array") }));
}

#[test]
fn rejects_array_as_class_name() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class array
{
}

array $items = new array();
"#,
    )
    .expect_err("semantic check should reject array as a class/type spelling");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0309" && diagnostic.message.contains("`array`")
    }));
    assert!(err
        .iter()
        .any(|diagnostic| { diagnostic.code == "E0401" && diagnostic.message.contains("array") }));
}

#[test]
fn rejects_array_as_callable_name() {
    let err = doriac::check_source(
        "test.doria",
        r#"
function array(): void
{
}

class Bag
{
    function array(): void
    {
    }
}
"#,
    )
    .expect_err("semantic check should reject array as a callable spelling");

    assert_eq!(
        err.iter()
            .filter(
                |diagnostic| diagnostic.code == "E0310" && diagnostic.message.contains("`array`")
            )
            .count(),
        2
    );
}

#[test]
fn resolves_typed_array_types() {
    doriac::check_source(
        "test.doria",
        r#"
int[] $numbers = [1, 2, 3];
string[] $names = [];
int[][] $matrix = [[1], []];

function accept(int[] $items): void
{
}
"#,
    )
    .expect("typed array declarations should resolve");
}

#[test]
fn preserves_nested_mixed_collection_shape() {
    doriac::check_source(
        "test.doria",
        r#"
mixed $payload = 1;
let $rows = [[$payload], [1]];

List<List<mixed>> $copy = $rows;
"#,
    )
    .expect("nested collection shape should be preserved while widening inner mixed values");
}

#[test]
fn preserves_nested_mixed_collection_shape_after_clear_heterogeneous_elements() {
    doriac::check_source(
        "test.doria",
        r#"
function rows(take mixed $payload)
{
    return [[1], ["two"], [$payload]];
}

List<List<mixed>> $copy = rows(1);
"#,
    )
    .expect("nested mixed collection inference should not depend on literal element order");
}

#[test]
fn rejects_mixed_operations_before_narrowing() {
    for (source, code) in [
        (
            r#"
mixed $payload = 1;
let $name = $payload->name;
"#,
            "E0433",
        ),
        (
            r#"
class User
{
    writable string $name;
}

mixed $payload = new User();
$payload->name = "Ada";
"#,
            "E0433",
        ),
        (
            r#"
mixed $payload = 1;
let $sum = $payload + 1;
"#,
            "E0433",
        ),
        (
            r#"
mixed $payload = 1;
let $same = $payload == 1;
"#,
            "E0433",
        ),
        (
            r#"
mixed $payload = true;
if ($payload) {
}
"#,
            "E0416",
        ),
        (
            r#"
mixed $payload = [1];
foreach ($payload as string $item) {
    echo $item;
}
"#,
            "E0433",
        ),
        (
            r#"
mixed $payload = "Doria";
echo "{$payload}";
"#,
            "E0415",
        ),
        (
            r#"
mixed $payload = "Doria";
echo $payload;
"#,
            "E0433",
        ),
        (
            r#"
mixed $payload = "Doria";
string $name = $payload;
"#,
            "E0403",
        ),
        (
            r#"
function leak(mixed $payload)
{
    return $payload;
}

string $name = leak(1);
"#,
            "E0403",
        ),
        (
            r#"
function leak(mixed $payload)
{
    if (true) {
        return $payload;
    }
}

string $name = leak(1);
"#,
            "E0403",
        ),
        (
            r#"
class Box
{
    mixed $payload = "x";

    function leak()
    {
        return $this->payload;
    }
}

let $box = new Box();
string $name = $box->leak();
"#,
            "E0403",
        ),
        (
            r#"
function forward(mixed $payload)
{
    return identity($payload);
}

function identity(mixed $payload)
{
    return $payload;
}

string $name = forward(1);
"#,
            "E0403",
        ),
        (
            r#"
function leak(mixed $payload)
{
    return [$payload, 1];
}

List<int> $numbers = leak("x");
"#,
            "E0403",
        ),
        (
            r#"
class Box
{
    mixed $payload = "x";
}

function leak(Box $box)
{
    Box $same = $box;
    return $same->payload;
}

string $payload = leak(new Box());
"#,
            "E0403",
        ),
        (
            r#"
List<mixed> $items = [1];

foreach ($items as string $item) {
    echo $item;
}
"#,
            "E0433",
        ),
        (
            r#"
Dictionary<string, mixed> $items = [
    "count" => 1,
];

foreach ($items as string $key => int $value) {
    echo $key;
}
"#,
            "E0433",
        ),
        (
            r#"
List<mixed> $left = [1];
List<mixed> $right = [2];

bool $same = $left == $right;
"#,
            "E0433",
        ),
        (
            r#"
writable mixed $payload = 1;
$payload += 1;
"#,
            "E0433",
        ),
        (
            r#"
function leak(mixed $payload, bool $usePayload)
{
    writable mixed $value = "";
    if ($usePayload) {
        $value = $payload;
    } else {
        $value = "safe";
    }

    return $value;
}

string $value = leak(1, true);
"#,
            "E0403",
        ),
        (
            r#"
function leak(mixed $payload)
{
    mixed[] $items = [$payload];
    return $items;
}

List<int> $items = leak(1);
"#,
            "E0403",
        ),
        (
            r#"
mixed $payload = 1;
echo [$payload];
"#,
            "E0433",
        ),
        (
            r#"
mixed $payload = 1;
mixed[] $items = [$payload];

foreach ($items as string $item) {
    echo $item;
}
"#,
            "E0433",
        ),
        (
            r#"
function first(mixed[] $items)
{
    foreach ($items as $item) {
        return $item;
    }
}

mixed $payload = 1;
string $value = first([$payload]);
"#,
            "E0403",
        ),
        (
            r#"
class Box
{
    mixed[] $items;

    function __construct(mixed $payload)
    {
        $this->items = [$payload];
    }

    function leak()
    {
        foreach ($this->items as $item) {
            return $item;
        }
    }
}

let $box = new Box(1);
string $value = $box->leak();
"#,
            "E0403",
        ),
        (
            r#"
mixed $payload = 1;
echo [[$payload], [1]];
"#,
            "E0433",
        ),
        (
            r#"
function first(mixed[] $items)
{
    foreach ($items as $item) {
        return $item;
    }
}

function forward(mixed[] $items)
{
    return first($items);
}

mixed $payload = 1;
string $value = forward([$payload]);
"#,
            "E0403",
        ),
        (
            r#"
class Box
{
    writable mixed[] $items;

    writable function set(mixed[] $items): void
    {
        $this->items = $items;
    }

    function leak()
    {
        foreach ($this->items as $item) {
            return $item;
        }
    }
}

let writable $box = new Box();
mixed $payload = 1;
$box->set([$payload]);
string $value = $box->leak();
"#,
            "E0403",
        ),
        (
            r#"
class Box
{
    mixed[] $items;

    function __construct(mixed[] $items)
    {
        $this->items = $items;
    }

    function leak()
    {
        foreach ($this->items as $item) {
            return $item;
        }
    }
}

mixed $payload = 1;
let $box = new Box([$payload]);
string $value = $box->leak();
"#,
            "E0403",
        ),
        (
            r#"
function sourceMixed(): mixed
{
    return 1;
}

class Box
{
    mixed[] $items = [sourceMixed()];

    function leak()
    {
        foreach ($this->items as $item) {
            return $item;
        }
    }
}

let $box = new Box();
string $value = $box->leak();
"#,
            "E0403",
        ),
        (
            r#"
class Box
{
    writable mixed[] $items;
}

function first(Box $box)
{
    foreach ($box->items as $item) {
        return $item;
    }
}

let writable $box = new Box();
mixed $payload = 1;
$box->items = [$payload];
string $value = first($box);
"#,
            "E0403",
        ),
    ] {
        let err =
            doriac::check_source("test.doria", source).expect_err("semantic check should fail");
        assert!(
            err.iter().any(|diagnostic| diagnostic.code == code),
            "expected {code}, got {err:?}"
        );
    }
}

#[test]
fn propagates_mixed_return_through_long_method_chains() {
    let mut source = String::from("class Chain\n{\n");
    for index in 0..16 {
        source.push_str(&format!(
            "    function m{index}(mixed $value)\n    {{\n        return $this->m{}($value);\n    }}\n\n",
            index + 1
        ));
    }
    source.push_str(
        r#"    function m16(mixed $value)
    {
        return $value;
    }
}

let $chain = new Chain();
string $payload = $chain->m0(1);
"#,
    );

    assert_type_mismatch(&source);
}

#[test]
fn mixed_operation_help_points_to_is_narrowing() {
    let err = doriac::check_source(
        "test.doria",
        r#"
mixed $payload = 1;
let $sum = $payload + 1;
"#,
    )
    .expect_err("semantic check should fail");
    let diagnostic = err
        .iter()
        .find(|diagnostic| diagnostic.code == "E0433")
        .expect("mixed operation diagnostic should be present");
    let help = diagnostic
        .help
        .as_deref()
        .expect("diagnostic should have help");

    assert!(help.contains("narrow"));
    assert!(help.contains("`is`"));
    assert!(!help.contains("`match`"));
}

#[test]
fn null_type_help_points_to_concrete_nullable_types() {
    let err = doriac::check_source(
        "test.doria",
        r#"
null $value = null;
"#,
    )
    .expect_err("semantic check should fail");
    let diagnostic = err
        .iter()
        .find(|diagnostic| diagnostic.code == "E0431")
        .expect("null type diagnostic should be present");
    let help = diagnostic
        .help
        .as_deref()
        .expect("diagnostic should have help");

    assert!(help.contains("`?T`"));
    assert!(help.contains("`?string`"));
    assert!(help.contains("`?Person`"));
    assert!(!help.contains("planned"));
}

#[test]
fn isolates_branch_scopes_during_mixed_return_inference() {
    let err = doriac::check_source(
        "test.doria",
        r#"
function safeValue()
{
    return "safe";
}

function maybeString(mixed $payload, bool $usePayload)
{
    let writable $value = safeValue();

    if ($usePayload) {
        $value = $payload;
    } else {
        return $value;
    }

    return "safe";
}

string $value = maybeString(1, false);
"#,
    )
    .expect_err("mixed assignment into the local remains rejected");

    let assignment_errors = err
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0403")
        .collect::<Vec<_>>();
    assert_eq!(
        assignment_errors.len(),
        1,
        "then-branch mixed assignment should not also pollute the caller return type: {err:?}"
    );
    assert!(assignment_errors[0]
        .message
        .contains("cannot assign value of type `mixed` to `Unknown`"));
}

#[test]
fn still_tracks_mixed_branch_assignments_that_can_reach_later_returns() {
    let err = doriac::check_source(
        "test.doria",
        r#"
function safeValue()
{
    return "safe";
}

function leak(mixed $payload, bool $usePayload)
{
    let writable $value = safeValue();

    if ($usePayload) {
        $value = $payload;
    }

    return $value;
}

string $value = leak(1, true);
"#,
    )
    .expect_err("mixed assignment can reach the later return");

    assert!(
        err.iter().any(|diagnostic| diagnostic.code == "E0403"
            && diagnostic
                .message
                .contains("cannot assign value of type `mixed` to `string`")),
        "post-if return should still expose mixed to the caller: {err:?}"
    );
}
#[test]
fn merges_mixed_return_shapes_before_updating_unannotated_signatures() {
    assert_type_mismatch(
        r#"
function leak(mixed $payload, bool $asList)
{
    if ($asList) {
        return [$payload];
    }

    return $payload;
}

List<mixed> $payloads = leak(1, false);
"#,
    );
}

#[test]
fn tracks_mixed_assignments_in_for_increments() {
    assert_type_mismatch(
        r#"
function unknownValue()
{
    return 0;
}

function leak(mixed $payload)
{
    let writable $keepGoing = true;
    let writable $value = unknownValue();

    for (; $keepGoing; $value = $payload) {
        $keepGoing = false;
    }

    return $value;
}

string $value = leak(1);
"#,
    );
}

#[test]
fn rejects_d21_dynamic_boundary_type_positions() {
    let cases = [
        ("null $empty = null;", "E0431", "`null` is a literal"),
        (
            "function accept(resource $handle): void {}",
            "E0432",
            "`resource` is reserved for PHP interop",
        ),
        (
            "void $nothing = null;",
            "E0430",
            "`void` is only valid as a return type",
        ),
        (
            "function accept(void $value): void {}",
            "E0430",
            "`void` is only valid as a return type",
        ),
        (
            "function accept(List<void> $values): void {}",
            "E0430",
            "`void` is only valid as a return type",
        ),
    ];

    for (source, code, message) in cases {
        let err =
            doriac::check_source("test.doria", source).expect_err("semantic check should fail");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.code == code && diagnostic.message.contains(message)),
            "expected {code} containing {message}, got {err:?}"
        );
    }
}

#[test]
fn rejects_incompatible_scalar_typed_declarations() {
    for source in [
        r#"int $age = "37";"#,
        r#"string $name = 123;"#,
        r#"bool $active = 1;"#,
        r#"int $count = 1.5;"#,
        r#"float $ratio = 1;"#,
    ] {
        assert_type_mismatch(source);
    }
}

#[test]
fn rejects_integer_literals_outside_doria_int_range_before_lowering() {
    let source = r#"
function main(): int
{
    return 9223372036854775808;
}
"#;

    let check_err = doriac::check_source("test.doria", source)
        .expect_err("semantic check should reject out-of-range integer literals");
    assert!(
        check_err
            .iter()
            .any(|diagnostic| diagnostic.code == "E0417"),
        "expected E0417, got {check_err:?}"
    );

    let lower_err = doriac::lower_source("test.doria", source)
        .expect_err("lowering should not run after semantic integer range failure");
    assert!(
        lower_err
            .iter()
            .any(|diagnostic| diagnostic.code == "E0417"),
        "expected E0417, got {lower_err:?}"
    );
}

#[test]
fn accepts_constant_integer_arithmetic_that_is_checked_at_runtime() {
    for source in [
        r#"
function main(): int
{
    return 9223372036854775807 + 1;
}
"#,
        r#"
function main(): int
{
    let $max = 9223372036854775807;
    let $overflow = $max + 1;
    return 0;
}
"#,
        r#"
function main(): int
{
    let $large = 4611686018427387904;
    let $overflow = $large * 2;
    return 0;
}
"#,
    ] {
        doriac::check_source("test.doria", source)
            .expect("overflowing arithmetic is a runtime panic, not a semantic error");
        doriac::lower_source("test.doria", source)
            .expect("checked arithmetic should reach backend-neutral lowering");
    }
}

#[test]
fn checks_writable_local_assignment_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
writable int $age = 37;
$age = 38;
$age += 1;
$age -= 2;

writable float $total = 1.5;
$total += 2.5;

let writable $items = [];
$items = [1];
List<int> $numbers = $items;

let writable $counts = [];
$counts = ["apples" => 5];
Dictionary<string, int> $inventory = $counts;
"#,
    )
    .expect("semantic check should succeed");

    for source in [
        r#"
writable int $age = 37;
$age = "old";
"#,
        r#"
writable string $name = "a";
$name += "b";
"#,
        r#"
writable string $name = "a";
$name -= "b";
"#,
        r#"
writable int $count = 1;
$count += "two";
"#,
        r#"
let writable $items = [];
$items = 1;
"#,
        r#"
let writable $items = [];
$items = ["oops"];
List<int> $numbers = $items;
"#,
        r#"
let writable $items = [];
$items = [1];
$items = ["apples" => 5];
"#,
    ] {
        assert_type_mismatch(source);
    }
}

#[test]
fn infers_binary_expression_types_for_assignment_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
int $sum = 1 + 2;
float $total = 1.5 + 2.5;
string $message = "hello" . " world";
let $concatName = "Doria";
let $greeting = "Hello " . $concatName . "!";
bool $less = 1 < 2;
bool $floatLess = 1.5 <= 2.5;
bool $stringLess = "a" < "b";
bool $same = "a" == "b";
bool $different = "a" != "b";
bool $logic = true && false;
bool $wordAnd = true and false;
bool $wordOr = false or true;
bool $wordNot = not false;
bool $wordXor = true xor false;
string $name = null ?? "Andrew";
"#,
    )
    .expect("semantic check should succeed");

    doriac::check_source("test.doria", r#"let $message = "Count: " . 42;"#)
        .expect("display concatenation should succeed");

    for source in [
        r#"string $value = 1 + 2;"#,
        r#"int $value = "x" . "y";"#,
        r#"bool $value = 1 < "2";"#,
        r#"bool $value = "2" >= 1;"#,
        r#"bool $value = true <= false;"#,
        r#"bool $value = 1 && 2;"#,
        r#"bool $value = "x" || "y";"#,
        r#"bool $value = not 1;"#,
        r#"bool $value = 1 and true;"#,
        r#"bool $value = "x" xor false;"#,
        r#"
writable int $value = 0;
$value = "x" . "y";
"#,
        r#"
class Person
{
    string $name = 1 + 2;
}
"#,
        r#"function greet(string $name = 1 + 2): void {}"#,
    ] {
        assert_type_mismatch(source);
    }
}

#[test]
fn rejects_incompatible_typed_equality_operands() {
    for source in [r#"bool $value = 1 == "1";"#, r#"bool $value = true != 1;"#] {
        assert_diagnostic_code(source, "E0420");
    }
}

#[test]
fn reports_boolean_operator_operand_errors() {
    for source in [
        r#"bool $value = not 1;"#,
        r#"bool $value = 1 and true;"#,
        r#"bool $value = "x" xor false;"#,
    ] {
        assert_diagnostic_code(source, "E0419");
    }
}

#[test]
fn infers_call_return_types_for_assignment_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
function age(): int
{
    return 37;
}

class Person
{
    function instanceAge(): int
    {
        return 37;
    }

    static function age(): int
    {
        return 37;
    }
}

int $fromFunction = age();
let $person = new Person();
int $fromMethod = $person->instanceAge();
int $fromStatic = Person::age();
"#,
    )
    .expect("semantic check should succeed");

    for source in [
        r#"
function age(): int
{
    return 37;
}

string $name = age();
"#,
        r#"
class Person
{
    static function age(): int
    {
        return 37;
    }
}

let $person = new Person();
string $name = $person->age();
"#,
        r#"
class Person
{
    static function age(): int
    {
        return 37;
    }
}

string $name = Person::age();
"#,
        r#"
class Person
{
    string $name = Person::age();

    function age(): int
    {
        return 37;
    }
}
"#,
        r#"
class Person
{
    static function age(): int
    {
        return 37;
    }
}

function greet(string $name = Person::age()): void
{
}
"#,
    ] {
        assert_type_mismatch(source);
    }
}

#[test]
fn checks_stage_10_free_function_call_semantics() {
    doriac::check_source(
        "test.doria",
        r#"
function add(int $left, int $right): int
{
    return $left + $right;
}

function main(): int
{
    return add(20, 22);
}
"#,
    )
    .expect("semantic check should accept int helper calls");

    doriac::check_source(
        "test.doria",
        r#"
function printHello(): void
{
    echo "Hello";
}

function main(): void
{
    printHello();
}
"#,
    )
    .expect("semantic check should accept void helper statement calls");

    for (source, code) in [
        (
            r#"
function add(int $left, int $right): int
{
    return $left + $right;
}

function main(): int
{
    return add(42);
}
"#,
            "E0409",
        ),
        (
            r#"
function add(int $left, int $right): int
{
    return $left + $right;
}

function main(): int
{
    return add("20", 22);
}
"#,
            "E0408",
        ),
        (
            r#"
function doThing(): void
{
}

function main(): int
{
    return doThing();
}
"#,
            "E0404",
        ),
    ] {
        assert_diagnostic_code(source, code);
    }
}

#[test]
fn checks_function_call_arguments() {
    doriac::check_source(
        "test.doria",
        r#"
function greet(string $name, int $times = 1): void
{
}

function sum(int $left, int $right): int
{
    return $left + $right;
}

function collect(List<int> $items): void
{
}

function collectMixed(List<mixed> $items): void
{
}

greet("Andrew");
greet("Andrew", 2);
int $total = sum(1, 2);
collect([1, 2, 3]);
collectMixed([1, 2]);
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_invalid_function_call_arguments() {
    for (source, code) in [
        (
            r#"
function greet(string $name): void
{
}

greet(123);
"#,
            "E0408",
        ),
        (
            r#"
function greet(string $name): void
{
}

greet();
"#,
            "E0409",
        ),
        (
            r#"
function greet(string $name): void
{
}

greet("A", "B");
"#,
            "E0409",
        ),
        (r#"unknown();"#, "E0309"),
    ] {
        assert_diagnostic_code(source, code);
    }
}

#[test]
fn checks_method_call_arguments() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    function greet(string $name): void
    {
    }
}

let $person = new Person();
$person->greet("Andrew");
"#,
    )
    .expect("semantic check should succeed");

    for (source, code) in [
        (
            r#"
class Person
{
    function greet(string $name): void
    {
    }
}

let $person = new Person();
$person->greet(123);
"#,
            "E0408",
        ),
        (
            r#"
class Person
{
    function greet(string $name): void
    {
    }
}

let $person = new Person();
$person->greet();
"#,
            "E0409",
        ),
    ] {
        assert_diagnostic_code(source, code);
    }
}

#[test]
fn checks_static_call_arguments() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    static function makeName(string $name): string
    {
        return $name;
    }
}

string $name = Person::makeName("Andrew");
"#,
    )
    .expect("semantic check should succeed");

    assert_diagnostic_code(
        r#"
class Person
{
    static function makeName(string $name): string
    {
        return $name;
    }
}

string $bad = Person::makeName(123);
"#,
        "E0408",
    );
}

#[test]
fn checks_constructor_call_arguments() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    function __construct(string $name, int $age = 37)
    {
    }
}

let $a = new Person("Andrew");
let $b = new Person("Andrew", 37);
"#,
    )
    .expect("semantic check should succeed");

    doriac::check_source(
        "test.doria",
        r#"
class Person {}

let $person = new Person();
"#,
    )
    .expect("semantic check should succeed");

    for (source, code) in [
        (
            r#"
class Person
{
    function __construct(string $name, int $age = 37)
    {
    }
}

let $bad = new Person();
"#,
            "E0409",
        ),
        (
            r#"
class Person
{
    function __construct(string $name, int $age = 37)
    {
    }
}

let $bad = new Person("Andrew", "37");
"#,
            "E0408",
        ),
        (
            r#"
class Person
{
    function __construct(string $name, int $age = 37)
    {
    }
}

let $bad = new Person("Andrew", 37, true);
"#,
            "E0409",
        ),
        (
            r#"
class Person {}

let $bad = new Person("Andrew");
"#,
            "E0409",
        ),
    ] {
        assert_diagnostic_code(source, code);
    }
}

#[test]
fn checks_internal_constructor_access() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal function __construct(string $name)
    {
    }

    function create(): Person
    {
        return new Person("Andrew");
    }
}
"#,
    )
    .expect("semantic check should succeed");

    for source in [
        r#"
class Person
{
    internal function __construct(string $name)
    {
    }
}

let $person = new Person("Andrew");
"#,
        r#"
class Person
{
    internal function __construct(string $name)
    {
    }
}

function create(): Person
{
    return new Person("Andrew");
}
"#,
    ] {
        assert_diagnostic_code(source, "E0307");
    }
}

#[test]
fn rejects_invalid_lifecycle_signatures() {
    assert_diagnostic_code(
        r#"
class Person
{
    function __destruct(string $reason)
    {
    }
}
"#,
        "E0411",
    );
}

#[test]
fn rejects_required_parameters_after_optional_parameters() {
    for source in [
        r#"
function greet(string $prefix = "Hi", string $name): void
{
}
"#,
        r#"
class Person
{
    function greet(string $prefix = "Hi", string $name): void
    {
    }
}
"#,
        r#"
class Person
{
    function __construct(string $prefix = "Hi", string $name)
    {
    }
}
"#,
    ] {
        assert_diagnostic_code(source, "E0410");
    }
}

#[test]
fn checks_declared_function_return_types() {
    doriac::check_source(
        "test.doria",
        r#"
function age(): int
{
    return 37;
}

function name(): string
{
    return "Andrew";
}

function active(): bool
{
    return true;
}

function total(): float
{
    return 1.5 + 2.5;
}

function message(): string
{
    return "Hello" . " Doria";
}

function copyAge(): int
{
    return age();
}

function log(): void
{
    return;
}

function noop(): void
{
    echo "ok";
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn checks_declared_method_return_types() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    function name(): string
    {
        return "Andrew";
    }

    function age(): int
    {
        return 37;
    }

    function copyAge(): int
    {
        return $this->age();
    }

    function __construct()
    {
        return;
    }

    function __destruct()
    {
        return;
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn allows_void_main_to_fall_through_or_return_bare() {
    for source in [
        r#"
function main(): void
{
}
"#,
        r#"
function main(): void
{
    return;
}
"#,
        r#"
function main(): void
{
    echo "Hello Doria!";
}
"#,
    ] {
        doriac::check_source("test.doria", source).expect("semantic check should succeed");
    }
}

#[test]
fn keeps_int_main_as_explicit_status() {
    doriac::check_source(
        "test.doria",
        r#"
function main(): int
{
    return 42;
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn allows_lifecycle_methods_with_omitted_or_void_return_types() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    function __construct()
    {
    }

    function __destruct()
    {
    }
}
"#,
    )
    .expect("semantic check should succeed");

    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    function __construct(): void
    {
        return;
    }

    function __destruct(): void
    {
        return;
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn checks_bool_and_string_free_function_calls() {
    doriac::check_source(
        "test.doria",
        r#"
function isAnswer(int $value): bool
{
    return $value == 42;
}

function greet(string $name): void
{
    echo "Hello " . $name . "!";
}

function main(): void
{
    if (isAnswer(42)) {
        greet("Doria");
    }
}
"#,
    )
    .expect("semantic check should succeed");

    assert_diagnostic_code(
        r#"
function bad(): bool
{
    return 1;
}
"#,
        "E0404",
    );

    assert_diagnostic_code(
        r#"
function greet(string $name): void
{
    echo $name;
}

function main(): void
{
    greet(42);
}
"#,
        "E0408",
    );

    assert_diagnostic_code(
        r#"
function tick(): void
{
}

function main(): int
{
    if (tick()) {
        return 42;
    }

    return 0;
}
"#,
        "E0416",
    );
}

#[test]
fn rejects_declared_function_return_type_mismatches() {
    for source in [
        r#"
function age(): int
{
    return "37";
}
"#,
        r#"
function name(): string
{
    return 123;
}
"#,
        r#"
function active(): bool
{
    return 1;
}
"#,
        r#"
function ratio(): float
{
    return 1;
}
"#,
        r#"
function total(): string
{
    return 1 + 2;
}
"#,
        r#"
function name(): string
{
    return age();
}

function age(): int
{
    return 37;
}
"#,
        r#"
class Person
{
    function age(): int
    {
        return 37;
    }
}

function name(): string
{
    let $person = new Person();
    return $person->age();
}
"#,
        r#"
class Person
{
    function age(): int
    {
        return 37;
    }
}

function name(): string
{
    return Person::age();
}
"#,
        r#"
function numbers(): List<int>
{
    return [1, "two"];
}
"#,
    ] {
        assert_diagnostic_code(source, "E0404");
    }
}

#[test]
fn rejects_values_returned_from_void_functions_and_constructors() {
    for source in [
        r#"
function log(): void
{
    return "done";
}
"#,
        r#"
function main(): void
{
    return 0;
}
"#,
        r#"
class Person
{
    function clear(): void
    {
        return 1;
    }
}
"#,
        r#"
class Person
{
    function __construct()
    {
        return 1;
    }

    function __destruct()
    {
        return "done";
    }
}
"#,
    ] {
        assert_diagnostic_code(source, "E0405");
    }
}

#[test]
fn rejects_non_void_lifecycle_return_annotations() {
    for source in [
        r#"
class Person
{
    function __construct(): int
    {
        return 1;
    }
}
"#,
        r#"
class Person
{
    function __destruct(): string
    {
        return "done";
    }
}
"#,
    ] {
        assert_diagnostic_code(source, "E0407");
    }
}

#[test]
fn rejects_missing_values_from_non_void_returns() {
    for source in [
        r#"
function age(): int
{
    return;
}
"#,
        r#"
function age(): int
{
    echo "missing";
}
"#,
        r#"
function first(List<int> $items): int
{
    foreach ($items as int $item) {
        return $item;
    }
}
"#,
        r#"
function value(bool $flag): int
{
    if ($flag) {
        return 1;
    }
}
"#,
        r#"
function value(bool $left, bool $right): int
{
    if ($left) {
        return 1;
    } else if ($right) {
        return 2;
    }
}
"#,
        r#"
function value(bool $flag): int
{
    if ($flag) {
        echo "missing";
    } else {
        return 2;
    }
}
"#,
        r#"
class Person
{
    function age(): int
    {
        echo "missing";
    }
}
"#,
    ] {
        assert_diagnostic_code(source, "E0406");
    }
}

#[test]
fn counts_exhaustive_if_returns_for_non_void_functions() {
    doriac::check_source(
        "test.doria",
        r#"
function value(bool $flag): int
{
    if ($flag) {
        return 1;
    } else {
        return 2;
    }
}

function chained(bool $left, bool $right): int
{
    if ($left) {
        echo "left";
        return 1;
    } else if ($right) {
        return 2;
    } else {
        if ($left == $right) {
            return 3;
        } else {
            return 4;
        }
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn keeps_unannotated_returns_unchecked() {
    doriac::check_source(
        "test.doria",
        r#"
function value()
{
    return "anything";
}

function empty()
{
    return;
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_readonly_property_assignment() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $name;
}

let writable $person = new Person();
$person->name = "Lucy";
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0202"));
}

#[test]
fn checks_property_initializer_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $name = "Andrew";
    int $age = 37;
}
"#,
    )
    .expect("semantic check should succeed");

    assert_type_mismatch(
        r#"
class Person
{
    string $name = 123;
}
"#,
    );
}

#[test]
fn checks_string_interpolation_semantics() {
    doriac::check_source(
        "test.doria",
        r#"
function render(): void
{
    string $name = "Andrew";
    int $age = 37;
    float $ratio = 1.5;
    bool $active = true;
    echo "{$name}{$age}{$ratio}{$active}";
}
"#,
    )
    .expect("semantic check should succeed");

    doriac::check_source(
        "test.doria",
        r#"
class Profile
{
    string $displayName = "Andrew";
}

class Person
{
    function __construct(take Profile $profile)
    {
    }

    function greet(): void
    {
        echo "Hello, {$this->profile->displayName}";
    }
}

let $person = new Person(new Profile());
"#,
    )
    .expect("semantic check should succeed");

    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $message;

    function __construct(string $name)
    {
        $this->message = "Hello, {$name}";
    }
}

let $person = new Person("Andrew");
"#,
    )
    .expect("semantic check should succeed");

    assert_type_mismatch(
        r#"
class Person
{
    int $message;

    function __construct(string $name)
    {
        $this->message = "Hello, {$name}";
    }
}
"#,
    );
}

#[test]
fn rejects_invalid_string_interpolation_semantics() {
    for (source, code) in [
        (r#"echo "Hello, {$name}";"#, "E0101"),
        (r#"let $nothing = null; echo "{$nothing}";"#, "E0415"),
        (
            r#"
class Person {}

let $person = new Person();
echo "Hello, {$person->name}";
"#,
            "E0303",
        ),
        (
            r#"
class Secret
{
    internal string $value = "hidden";
}

let $secret = new Secret();
echo "Secret: {$secret->value}";
"#,
            "E0306",
        ),
        (
            r#"
class Person {}

let $person = new Person();
echo "{$person}";
"#,
            "E0462",
        ),
        (
            r#"
function show(List<int> $items): void
{
    echo "{$items}";
}
"#,
            "E0415",
        ),
        (
            r#"
function show(Dictionary<string, int> $items): void
{
    echo "{$items}";
}
"#,
            "E0415",
        ),
        (
            r#"
function show(Set<int> $items): void
{
    echo "{$items}";
}
"#,
            "E0415",
        ),
    ] {
        assert_diagnostic_code(source, code);
    }
}

#[test]
fn checks_compiler_known_displayable_conformance_and_display_contexts() {
    doriac::check_source(
        "test.doria",
        r#"
class Label implements Displayable
{
    function toString(): string
    {
        return "Doria";
    }
}

let $label = new Label();
echo $label;
echo "label={$label}";
echo "label=" . $label;
echo sprintf("%s", $label);
"#,
    )
    .expect("explicit Displayable conformance should enable every display context");

    for member in [
        "",
        "function toString(int $value): string { return \"Doria\"; }",
        "function toString(): int { return 1; }",
        "static function toString(): string { return \"Doria\"; }",
        "writable function toString(): string { return \"Doria\"; }",
        "internal function toString(): string { return \"Doria\"; }",
        "function ToString(): string { return \"Doria\"; }",
        "function to_string(): string { return \"Doria\"; }",
        "function __toString(): string { return \"Doria\"; }",
    ] {
        let source = format!("class Label implements Displayable {{ {member} }}");
        assert_diagnostic_code(&source, "E0463");
    }
}

#[test]
fn rejects_non_displayable_classes_in_every_display_context() {
    for display in [
        "echo $token;",
        "echo \"token={$token}\";",
        "echo \"token=\" . $token;",
        "echo sprintf(\"%s\", $token);",
    ] {
        let source = format!("class Token {{}} let $token = new Token(); {display}");
        let diagnostics = doriac::check_source("test.doria", &source)
            .expect_err("non-Displayable class should be rejected in display contexts");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0462"
                && diagnostic.message.contains("`Token` cannot be displayed")
                && diagnostic.message.contains("function toString(): string")
        }));
    }

    assert_diagnostic_code(
        r#"
class Token
{
    function toString(): string { return "coincidence"; }
}
let $token = new Token();
echo $token;
"#,
        "E0462",
    );
    assert_diagnostic_code(
        "class Token {} let $token = new Token(); string $text = $token;",
        "E0403",
    );
}

#[test]
fn reserves_displayable_and_defers_general_interfaces() {
    assert_diagnostic_code("class Displayable {}", "E0309");
    assert_diagnostic_code("class Label implements Other {}", "E0464");

    for (source, code) in [
        ("interface Displayable {}", "E0309"),
        ("interface Other {}", "E0464"),
    ] {
        doriac::parse_source("test.doria", source)
            .expect("accepted interface declarations should parse");
        let diagnostics = doriac::check_source("test.doria", source)
            .expect_err("interface semantics are not implemented yet");
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == code));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.code.starts_with('P')));
    }
}

#[test]
fn reports_stable_semantic_gaps_for_accepted_class_workflow_syntax() {
    let diagnostics = doriac::check_source(
        "test.doria",
        r#"
namespace Vendor\App;
class Child extends Vendor\Base implements Vendor\Contracts\Printable {}
"#,
    )
    .expect_err("namespace, inheritance, and general conformance are not implemented yet");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0475" && diagnostic.message.contains("namespace")));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0476" && diagnostic.message.contains("extends")));
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0464" && diagnostic.message.contains("interface")));
    assert!(diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.code.starts_with('P')));
}

#[test]
fn reports_qualified_type_names_as_semantic_coverage() {
    let diagnostics = doriac::check_source(
        "test.doria",
        "function accept(Vendor\\Contracts\\Input $input): void {}",
    )
    .expect_err("qualified-name resolution is not implemented yet");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0475" && diagnostic.message.contains("namespace")));
    assert!(diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.code.starts_with('P')));
}

#[test]
fn reports_qualified_bare_identifiers_as_semantic_coverage() {
    let diagnostics = doriac::check_source(
        "test.doria",
        r#"
let $value = Vendor\Lib\VALUE;
echo "{Vendor\Lib\LABEL}";
"#,
    )
    .expect_err("qualified-name resolution is not implemented yet");

    let qualified_name_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0475")
        .count();
    assert_eq!(qualified_name_diagnostics, 2, "{diagnostics:#?}");
    assert!(diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.code.starts_with('P')));
}

#[test]
fn allows_property_initializer_accessing_own_internal_static_method() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $name = Person::defaultName();

    internal static function defaultName(): string
    {
        return "Andrew";
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_this_in_property_initializer() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $name = $this->defaultName();

    internal function defaultName(): string
    {
        return "Andrew";
    }
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0102"));
}

#[test]
fn checks_property_assignment_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    writable string $name = "";
}

let writable $person = new Person();
$person->name = "Lucy";
"#,
    )
    .expect("semantic check should succeed");

    assert_type_mismatch(
        r#"
class Person
{
    writable string $name = "";
}

let writable $person = new Person();
$person->name = 123;
"#,
    );
}

#[test]
fn checks_basic_control_flow_semantics() {
    doriac::check_source(
        "test.doria",
        r#"
bool $flag = true;
if ($flag) {
    echo "yes";
}

writable int $age = 12;
if ($age < 13) {
    echo "child";
} else if ($age < 20) {
    echo "teen";
} else {
    echo "adult";
}

while ($age < 20) {
    $age += 1;
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn checks_loop_control_semantics() {
    doriac::check_source(
        "test.doria",
        r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        $code += 1;

        if ($code == 10) {
            continue;
        }

        if ($code == 42) {
            break;
        }
    }

    return $code;
}
"#,
    )
    .expect("semantic check should accept loop control inside loops");
}

#[test]
fn checks_stage_9_increment_and_for_semantics() {
    doriac::check_source(
        "test.doria",
        r#"
function main(): void
{
    let writable $i = 0;
    $i++;
    ++$i;
    $i--;
    --$i;

    for (let writable $j = 0; $j < 10; $j++) {
    }

    for (; $i < 10; ++$i) {
        continue;
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_invalid_stage_9_increment_targets() {
    assert_diagnostic_code(
        r#"
function main(): void
{
    let $i = 0;
    $i++;
}
"#,
        "E0201",
    );

    assert_diagnostic_code(
        r#"
function main(): void
{
    $i++;
}
"#,
        "E0101",
    );

    assert_diagnostic_code(
        r#"
function main(): void
{
    let writable $name = "Doria";
    $name++;
}
"#,
        "E0423",
    );

    assert_diagnostic_code(
        r#"
function main(): void
{
    let writable $name = "a";

    for ($name += "b"; false;) {
    }
}
"#,
        "E0403",
    );
}

#[test]
fn keeps_for_initializer_bindings_loop_local() {
    assert_diagnostic_code(
        r#"
function main(): int
{
    for (let writable $i = 0; $i < 10; $i++) {
    }

    return $i;
}
"#,
        "E0101",
    );
}

#[test]
fn checks_stage_9_foreach_range_semantics() {
    doriac::check_source(
        "test.doria",
        r#"
function main(): void
{
    foreach (0..<10 as $i) {
        let $copy = $i;
    }

    foreach ((0..10) as $j) {
        let $copy = $j;
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_standalone_range_expressions() {
    for source in [
        r#"
function main(): void
{
    let $range = 0..10;
}
"#,
        r#"
function main(): void
{
    echo 0..<10;
}
"#,
        r#"
function main(): void
{
    let $range = (0..10);
}
"#,
    ] {
        assert_diagnostic_code(source, "E0426");
    }
}

#[test]
fn rejects_invalid_stage_9_foreach_ranges() {
    assert_diagnostic_code(
        r#"
function main(): void
{
    foreach (0..10 as $i) {
        $i++;
    }
}
"#,
        "E0201",
    );

    assert_diagnostic_code(
        r#"
function main(): int
{
    foreach (0..10 as $i) {
    }

    return $i;
}
"#,
        "E0101",
    );

    assert_diagnostic_code(
        r#"
function main(): void
{
    foreach ("0"..10 as $i) {
    }
}
"#,
        "E0424",
    );
}

#[test]
fn rejects_loop_control_outside_loop() {
    for (source, code, message) in [
        (
            r#"
function main(): int
{
    break;

    return 0;
}
"#,
            "E0421",
            "`break` may only be used inside a loop",
        ),
        (
            r#"
function main(): int
{
    continue;

    return 0;
}
"#,
            "E0422",
            "`continue` may only be used inside a loop",
        ),
    ] {
        let err = doriac::check_source("test.doria", source).expect_err("loop control should fail");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.code == code && diagnostic.message.contains(message)),
            "expected {code} containing {message}, got {err:?}"
        );
    }
}

#[test]
fn rejects_non_bool_control_flow_conditions() {
    for source in [
        r#"
if (1) {
    echo "bad";
}
"#,
        r#"
while ("yes") {
    echo "bad";
}
"#,
    ] {
        assert_diagnostic_code(source, "E0416");
    }
}

#[test]
fn keeps_control_flow_block_scopes_local() {
    assert_diagnostic_code(
        r#"
if (true) {
    let $name = "Andrew";
}

echo $name;
"#,
        "E0101",
    );

    assert_diagnostic_code(
        r#"
while (true) {
    let $name = "Andrew";
}

echo $name;
"#,
        "E0101",
    );
}

#[test]
fn checks_mutation_rules_inside_control_flow() {
    doriac::check_source(
        "test.doria",
        r#"
let writable $count = 0;
while ($count < 10) {
    $count += 1;
}
"#,
    )
    .expect("semantic check should succeed");

    assert_diagnostic_code(
        r#"
let $count = 0;
while ($count < 10) {
    $count += 1;
}
"#,
        "E0201",
    );
}

#[test]
fn checks_property_mutation_rules_inside_control_flow() {
    doriac::check_source(
        "test.doria",
        r#"
class Counter
{
    writable int $count = 0;

    function __construct(int $start)
    {
        if ($start > 0) {
            $this->count = $start;
        }

        while ($this->count < 10) {
            $this->count += 1;
        }
    }
}
"#,
    )
    .expect("semantic check should succeed");

    assert_diagnostic_code(
        r#"
class Counter
{
    int $count;

    function update(int $start): void
    {
        if ($start > 0) {
            $this->count = $start;
        }
    }
}
"#,
        "E0202",
    );
}

#[test]
fn checks_constructor_readonly_init_inside_control_flow() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $id;

    function __construct(string $input)
    {
        if ($input == "") {
            $this->id = "unknown";
        } else {
            $this->id = $input;
        }
    }
}
"#,
    )
    .expect("every reachable branch initializes the readonly property");

    assert_diagnostic_code(
        r#"
class Person
{
    string $id;

    function __construct(string $input)
    {
        while ($input == "") {
            $this->id = "unknown";
        }
    }
}
"#,
        "E0504",
    );
}
#[test]
fn allows_constructor_init_access_for_readonly_properties() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $id;

    function __construct(string $givenId)
    {
        ($this)->id = $givenId;
    }
}

class Token
{
    internal string $value;

    function __construct(string $raw)
    {
        $this->value = $raw;
    }
}

class Counter
{
    writable int $count;

    function __construct(int $initial)
    {
        $this->count = $initial;
        $this->count = $initial + 1;
        $this->count += 1;
    }
}

class Accumulator
{
    writable int $count = 0;

    function __construct(take List<int> $items)
    {
        foreach ($this->items as int $item) {
            $this->count += $item;
        }
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_the_previously_accepted_writable_constructor_spelling() {
    let diagnostics = doriac::check_source(
        "test.doria",
        r#"
class Renamer
{
    writable string $name;

    writable function __construct(string $newName)
    {
        $this->rename($newName);
    }

    writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
"#,
    )
    .expect_err("writable construction must be rejected");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0466"));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0203"
            && diagnostic
                .message
                .contains("cannot call writable method `Renamer::rename`")
    }));
}

#[test]
fn writable_class_parameters_require_writable_argument_paths() {
    let diagnostics = doriac::check_source(
        "test.doria",
        r#"
class Box
{
    writable int $value = 0;
}

function update(writable Box $box): void
{
    $box->value = 1;
}

function main(): void
{
    let $box = new Box();
    update($box);
}
"#,
    )
    .expect_err("readonly class arguments cannot be passed for mutation");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0204"
            && diagnostic
                .message
                .contains("must be a writable class value")
    }));

    doriac::check_source(
        "test.doria",
        r#"
class Box
{
    writable int $value = 0;
}

function update(writable Box $box): void
{
    $box->value = 1;
}

function main(): void
{
    let writable $box = new Box();
    update($box);
}
"#,
    )
    .expect("writable class arguments should remain valid");
}

#[test]
fn rejects_invalid_constructor_init_access() {
    for (source, code) in [
        (
            r#"
class Person
{
    string $id;

    function __construct(string $givenId)
    {
        $this->id = $givenId;
        $this->id = "other";
    }
}
"#,
            "E0412",
        ),
        (
            r#"
class Person
{
    string $id = "default";

    function __construct(string $givenId)
    {
        $this->id = $givenId;
    }
}
"#,
            "E0412",
        ),
        (
            r#"
class Person
{
    function __construct(string $id)
    {
        $this->id = "other";
    }
}
"#,
            "E0412",
        ),
        (
            r#"
class Person
{
    int $id;

    function __construct(int $givenId)
    {
        $this->id += $givenId;
    }
}
"#,
            "E0413",
        ),
        (
            r#"
class Person
{
    string $id;

    function rename(string $id): void
    {
        $this->id = $id;
    }
}
"#,
            "E0202",
        ),
        (
            r#"
class Person
{
    string $id;

    function __destruct()
    {
        $this->id = "late";
    }
}
"#,
            "E0202",
        ),
        (
            r#"
class Child
{
    writable string $name;
}

class Person
{
    Child $child;

    function __construct(Child $newChild)
    {
        $this->child->name = "Lucy";
    }
}
"#,
            "E0201",
        ),
        (
            r#"
class Person
{
    writable string $name;

    function __construct(string $newName)
    {
        $this->rename($newName);
    }

    writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
"#,
            "E0203",
        ),
        (
            r#"
class Person
{
    string $id;

    function __construct(take List<string> $ids)
    {
        foreach ($ids as string $id) {
            $this->id = $id;
        }
    }
}
"#,
            "E0504",
        ),
    ] {
        assert_diagnostic_code(source, code);
    }
}

#[test]
fn rejects_direct_lifecycle_method_calls() {
    for source in [
        r#"
class Person
{
    string $id;

    function __construct(string $givenId)
    {
        $this->id = $givenId;
    }
}

let writable $person = new Person("a");
$person->__construct("b");
"#,
        r#"
class Person
{
    function __destruct()
    {
    }
}

let writable $person = new Person();
$person->__destruct();
"#,
        r#"
class Person
{
    function __construct()
    {
    }
}

Person::__construct();
"#,
    ] {
        assert_diagnostic_code(source, "E0414");
    }
}

#[test]
fn checks_constructor_init_assignment_compatibility() {
    assert_type_mismatch(
        r#"
class Person
{
    int $age;

    function __construct(string $value)
    {
        $this->age = $value;
    }
}
"#,
    );
}

#[test]
fn rejects_this_mutation_in_readonly_method() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    writable string $name;

    function rename(string $name): void
    {
        $this->name = $name;
    }
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0201"));
}

#[test]
fn rejects_writable_method_call_through_readonly_variable() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    writable string $name;

    writable function rename(string $name): void
    {
        $this->name = $name;
    }
}

let $person = new Person();
$person->rename("Lucy");
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0203"));
}

#[test]
fn rejects_duplicate_local_declaration() {
    let err = doriac::check_source(
        "test.doria",
        r#"
let $x = 1;
let $x = 2;
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0103"));
}

#[test]
fn rejects_duplicate_class_declaration() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person {}
class Person {}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0300"));
}

#[test]
fn rejects_duplicate_function_declaration() {
    let err = doriac::check_source(
        "test.doria",
        r#"
function greet(): void {}
function greet(): void {}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0308"));
}

#[test]
fn reserves_compiler_generated_top_level_function_namespace() {
    let err = doriac::check_source(
        "test.doria",
        r#"
function __doria_read_line(): void {}
function __Doria_mixed_case_helper(): void {}
function main(): void {}
"#,
    )
    .expect_err("compiler-generated top-level function names must be reserved");

    assert_eq!(
        err.iter()
            .filter(|diagnostic| diagnostic.code == "E0310")
            .count(),
        2,
        "every ASCII case variant of the generated helper prefix must be reserved"
    );
    assert!(err.iter().all(|diagnostic| {
        diagnostic.code == "E0310"
            && diagnostic.message.contains("`__doria_`")
            && diagnostic.help.as_deref()
                == Some("choose a function name that does not begin with `__doria_`")
    }));

    doriac::check_source(
        "test.doria",
        r#"
class Example
{
    function __doria_helper(): void {}
}
function main(): void {}
"#,
    )
    .expect("the generated global namespace must not reserve method names");
}

#[test]
fn reserves_stage23_io_intrinsic_names_after_implementation() {
    for name in [
        "append_file",
        "read_file_bytes",
        "write_file_bytes",
        "append_file_bytes",
        "read_stdin_bytes",
        "write_stdout_bytes",
        "write_stderr_bytes",
    ] {
        let declarations = format!(
            "function {name}(): void {{}}\nclass Example {{ function {name}(): void {{}} }}"
        );
        let diagnostics = doriac::check_source("test.doria", declarations)
            .expect_err("future intrinsic declarations must be reserved");
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code == "E0310"
                        && diagnostic.message.contains("intrinsic name")
                        && diagnostic.message.contains(name)
                })
                .count(),
            2,
            "{name} must be reserved globally and as a method name"
        );
    }
}

#[test]
fn reserves_top_level_print_with_echo_guidance() {
    let err = doriac::check_source(
        "test.doria",
        r#"
function print(string $value): void {}
function main(): void
{
    let $result = print("hello");
}
"#,
    )
    .expect_err("print must remain a rejected top-level spelling");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0310"
            && diagnostic.message.contains("`print`")
            && diagnostic.message.contains("`echo`")
    }));
    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0462"
            && diagnostic.message.contains("`print`")
            && diagnostic.message.contains("`echo`")
    }));

    doriac::check_source(
        "test.doria",
        r#"
class Logger
{
    function print(string $value): void {}
}
function main(): void {}
"#,
    )
    .expect("a receiver-qualified method named print is not the rejected top-level spelling");
}

#[test]
fn checks_duplicate_global_functions_against_their_own_return_annotations() {
    let err = doriac::check_source(
        "test.doria",
        r#"
function f(): string
{
    return 1;
}

function f(): int
{
    return 1;
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0308"));
    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0404"));
}

#[test]
fn rejects_duplicate_property_declaration() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $name;
    string $name;
}
"#,
    )
    .expect_err("semantic check should fail");

    let duplicate = err
        .iter()
        .find(|diagnostic| diagnostic.code == "E0481")
        .expect("duplicate member diagnostic");
    assert_eq!(duplicate.related.len(), 1);
}

#[test]
fn rejects_duplicate_method_declaration() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    function rename(string $name): void {}
    function rename(string $name): void {}
}
"#,
    )
    .expect_err("semantic check should fail");

    let duplicate = err
        .iter()
        .find(|diagnostic| diagnostic.code == "E0481")
        .expect("duplicate member diagnostic");
    assert_eq!(duplicate.related.len(), 1);
}

#[test]
fn rejects_unknown_property_read() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person {}

let $person = new Person();
echo $person->name;
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0303"));
}

#[test]
fn rejects_unknown_property_write() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person {}

let writable $person = new Person();
$person->name = "Lucy";
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0303"));
}

#[test]
fn rejects_unknown_method_call() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person {}

let $person = new Person();
$person->rename("Lucy");
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0304"));
}

#[test]
fn rejects_unknown_class_construction() {
    let err = doriac::check_source(
        "test.doria",
        r#"
let $person = new Person();
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0305"));
}

#[test]
fn allows_method_accessing_own_internal_method() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal function message(): string
    {
        return "Hello";
    }

    function greet(): void
    {
        echo $this->message();
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn allows_method_accessing_own_internal_property() {
    doriac::check_source(
        "test.doria",
        r#"
class Parser
{
    internal int $position = 0;

    function parse(): void
    {
        echo $this->position;
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_external_access_to_internal_property() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal string $secret;
}

let $person = new Person();
echo $person->secret;
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0306"));
}

#[test]
fn rejects_external_call_to_internal_method() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal function message(): string
    {
        return "Hello";
    }
}

let $person = new Person();
echo $person->message();
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0307"));
}

#[test]
fn rejects_external_static_call_to_internal_method() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal static function message(): string
    {
        return "Hello";
    }
}

echo Person::message();
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0307"));
}

#[test]
fn rejects_free_function_access_to_internal_property() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal string $secret;
}

function reveal(Person $person): void
{
    echo $person->secret;
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0306"));
}

#[test]
fn rejects_free_function_call_to_internal_method() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal function message(): string
    {
        return "Hello";
    }
}

function reveal(Person $person): void
{
    echo $person->message();
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0307"));
}

#[test]
fn rejects_other_class_access_to_internal_property() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal string $secret;
}

class Inspector
{
    function reveal(Person $person): void
    {
        echo $person->secret;
    }
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0306"));
}

#[test]
fn rejects_other_class_call_to_internal_method() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal function message(): string
    {
        return "Hello";
    }
}

class Inspector
{
    function reveal(Person $person): void
    {
        echo $person->message();
    }
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0307"));
}

#[test]
fn allows_constructor_accessing_own_internal_members() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    internal string $cacheKey = "person";

    function __construct(string $name)
    {
        echo $this->cacheKey;
        echo $this->buildCacheKey($name);
    }

    internal function buildCacheKey(string $name): string
    {
        return $name;
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn internal_does_not_imply_writable() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Parser
{
    internal int $position;

    writable function advance(): void
    {
        $this->position = 1;
    }
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0202"));
}

#[test]
fn resolves_lowercase_primitive_types() {
    doriac::check_source(
        "test.doria",
        r#"
function accept(
    int $id,
    float $ratio,
    string $name,
    bool $active,
    mixed $payload,
): void
{
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn resolves_declared_class_types() {
    doriac::check_source(
        "test.doria",
        r#"
class Person {}

function greet(Person $person): void
{
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn checks_class_assignment_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
class Person {}

Person $person = new Person();
"#,
    )
    .expect("semantic check should succeed");

    assert_type_mismatch(
        r#"
class Person {}
class Office {}

Person $person = new Office();
"#,
    );
}

#[test]
fn reports_unknown_explicit_type_names() {
    let err = doriac::check_source(
        "test.doria",
        r#"
UnknownThing $value = 1;
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0401" && diagnostic.message.contains("UnknownThing")
    }));
}

#[test]
fn reports_object_as_unknown_type_with_help() {
    let err = doriac::check_source("test.doria", "object $value = 1;")
        .expect_err("semantic check should fail");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0401"
            && diagnostic.message.contains("`object` does not exist")
            && diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("concrete class type") && help.contains("mixed"))
    }));
}

#[test]
fn resolves_collection_alias_types() {
    doriac::check_source(
        "test.doria",
        r#"
function accept(
    List<int> $ids,
    Dictionary<string, int> $counts,
    Set<string> $names,
    List<Dictionary<string, int>> $nested,
): void
{
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn checks_collection_assignment_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
class A {}
class B {}

List<int> $numbers = [1, 2, 3];
List<int> $emptyNumbers = [];
List<List<int>> $rows = [[1], []];
List<A> $objects = [new A(), new A()];
List<mixed> $mixedValues = [1, "two", new A()];
List<int[]> $arrays = [[1], []];
int[] $numberArray = [1, 2, 3];
string[] $emptyStringArray = [];
int[][] $arrayRows = [[1], []];
Dictionary<string, int> $counts = [
    "apples" => 5,
];
Dictionary<string, A> $objectsByName = [
    "a" => new A(),
    "b" => new A(),
];
Dictionary<string, mixed> $mixedByName = [
    "a" => new A(),
    "b" => 1,
];
Dictionary<string, List<int>> $nestedCounts = [
    "apples" => [5],
    "oranges" => [],
];
Dictionary<int, int> $indexedCounts = [
    0 => 10,
    1 => 20,
];
Dictionary<string, int> $emptyCounts = [];

class Inventory
{
    Dictionary<string, int> $counts = [];
    List<A> $objects = [new A(), new A()];
}
"#,
    )
    .expect("semantic check should succeed");

    for source in [
        r#"List<string> $numbers = [1, 2, 3];"#,
        r#"List<int> $numbers = [1, "two"];"#,
        r#"List<int> $numbers = [1, []];"#,
        r#"
Dictionary<string, string> $counts = [
    "apples" => 5,
];
"#,
        r#"
Dictionary<string, int> $counts = [
    "apples" => 5,
    "oranges" => "ten",
];
"#,
        r#"
Dictionary<string, int> $counts = [
    "apples" => 5,
    10,
];
"#,
        r#"
class A {}
List<A> $objects = [new A(), 1];
"#,
        r#"
List<int[]> $arrays = [[1], 2];
"#,
        r#"
int[] $numbers = [1, "two"];
"#,
        r#"
int[] $numbers = [
    "one" => 1,
];
"#,
        r#"
class A {}
Dictionary<string, A> $objectsByName = [
    "a" => new A(),
    "b" => 1,
];
"#,
        r#"
function collect(mixed $payload): void
{
    List<int> $numbers = [1, $payload, "two"];
}
"#,
        r#"
function collect(mixed $payload): void
{
    Dictionary<string, int> $counts = [
        "apples" => 5,
        "oranges" => $payload,
        "pears" => "ten",
    ];
}
"#,
        r#"
mixed $payload = "x";
let $values = [$payload, 1];
List<int> $numbers = $values;
"#,
        r#"
mixed $payload = "x";
let $values = [
    "first" => $payload,
    "second" => 1,
];
Dictionary<string, int> $numbers = $values;
"#,
        r#"Set<string> $names = [];"#,
        r#"
let $empty = [];
Set<string> $names = $empty;
"#,
    ] {
        assert_type_mismatch(source);
    }
}

#[test]
fn checks_parameter_default_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
function greet(int $count = 1): void
{
}

class Person
{
    function __construct(int $age = 37)
    {
    }

    function greet(int $count = 2): void
    {
    }

    function rename(int $count = 3): void
    {
    }
}
"#,
    )
    .expect("semantic check should succeed");

    for source in [
        r#"function greet(string $name = 123): void {}"#,
        r#"
class Person
{
    function rename(string $name = 123): void
    {
    }
}
"#,
        r#"
class Person
{
    function __construct(string $name = 123)
    {
    }
}
"#,
    ] {
        assert_type_mismatch(source);
    }
}

#[test]
fn rejects_this_in_parameter_defaults() {
    let err = doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $name = "Andrew";

    function rename(string $name = $this->name): void
    {
    }
}
"#,
    )
    .expect_err("semantic check should fail");

    assert!(
        err.iter().any(|diagnostic| diagnostic.code == "E0102"),
        "expected E0102, got {err:?}"
    );
}

#[test]
fn resolves_types_across_declaration_sites() {
    doriac::check_source(
        "test.doria",
        r#"
class Person {}

class Office
{
    Person $manager = new Person();

    function __construct(take Person $owner)
    {
    }

    function index(List<Person> $people): Dictionary<string, Person>
    {
        foreach ($people as Person $person) {
        }

        return [];
    }
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn rejects_invalid_collection_type_argument_counts() {
    for source in [
        "function accept(List<int, string> $value): void {}",
        "function accept(Dictionary<string> $value): void {}",
        "function accept(Dictionary<string, int, bool> $value): void {}",
        "function accept(Set<string, int> $value): void {}",
    ] {
        let err =
            doriac::check_source("test.doria", source).expect_err("semantic check should fail");

        assert!(err.iter().any(|diagnostic| diagnostic.code == "E0402"));
    }
}

#[test]
fn rejects_empty_collection_type_argument_list_as_parse_error() {
    doriac::parse_source("test.doria", "function accept(List<> $value): void {}")
        .expect_err("empty collection type arguments should not parse");
}

#[test]
fn pascal_case_primitive_companion_names_are_not_primitive_types() {
    let err = doriac::check_source(
        "test.doria",
        r#"
Int $value = 1;
"#,
    )
    .expect_err("semantic check should fail");

    assert!(err
        .iter()
        .any(|diagnostic| diagnostic.code == "E0401" && diagnostic.message.contains("Int")));
}

#[test]
fn non_companion_pascal_case_type_names_resolve_when_declared_as_classes() {
    doriac::check_source(
        "test.doria",
        r#"
class Invoice {}

Invoice $value = new Invoice();
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn tracks_mixed_return_through_grouped_assignment_targets() {
    assert_type_mismatch(
        r#"
function leak(mixed $payload)
{
    let writable $items = [];
    ($items) = [$payload];
    return $items;
}

List<int> $items = leak("x");
"#,
    );
}

#[test]
fn preserves_mixed_with_empty_collection_literals_in_return_inference() {
    assert_type_mismatch(
        r#"
function leak(mixed $payload)
{
    return [$payload, []];
}

List<int> $items = leak("x");
"#,
    );
}

#[test]
fn allows_body_locals_to_shadow_params_during_mixed_return_inference() {
    assert_type_mismatch(
        r#"
function leak(int $value, mixed $payload)
{
    let $value = $payload;
    return $value;
}

string $value = leak(0, 1);
"#,
    );
}
