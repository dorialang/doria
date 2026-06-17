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
null $empty = null;

function copy(resource $handle): void
{
    resource $same = $handle;
}
"#,
    )
    .expect("semantic check should succeed");
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
bool $less = 1 < 2;
bool $floatLess = 1.5 <= 2.5;
bool $stringLess = "a" < "b";
bool $same = "a" == "b";
bool $logic = true && false;
string $name = null ?? "Andrew";
"#,
    )
    .expect("semantic check should succeed");

    for source in [
        r#"string $value = 1 + 2;"#,
        r#"int $value = "x" . "y";"#,
        r#"bool $value = 1 < "2";"#,
        r#"bool $value = "2" >= 1;"#,
        r#"bool $value = true <= false;"#,
        r#"bool $value = 1 && 2;"#,
        r#"bool $value = "x" || "y";"#,
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
    function age(): int
    {
        return 37;
    }
}

int $fromFunction = age();
let $person = new Person();
int $fromMethod = $person->age();
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
    function age(): int
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
    function age(): int
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
    function age(): int
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
fn checks_function_call_arguments() {
    doriac::check_source(
        "test.doria",
        r#"
function greet(string $name, string $suffix = "!"): void
{
}

function sum(int $left, int $right): int
{
    return $left + $right;
}

function collect(List<int> $items): void
{
}

greet("Andrew");
greet("Andrew", "!");
int $total = sum(1, 2);
collect([1, 2, 3]);
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
    function makeName(string $name): string
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
    function makeName(string $name): string
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
fn allows_property_initializer_accessing_own_internal_static_method() {
    doriac::check_source(
        "test.doria",
        r#"
class Person
{
    string $name = Person::defaultName();

    internal function defaultName(): string
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
    writable string $name;
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
    writable string $name;
}

let writable $person = new Person();
$person->name = 123;
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

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0301"));
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

    assert!(err.iter().any(|diagnostic| diagnostic.code == "E0302"));
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
    internal int $position;

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
    internal function message(): string
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
    object $instance,
    resource $handle,
    null $empty,
): void
{
}
"#,
    )
    .expect("semantic check should succeed");
}

#[test]
fn resolves_null_typed_declarations() {
    doriac::check_source(
        "test.doria",
        r#"
null $empty = null;

function clear(): void
{
    null $value = null;
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
fn checks_object_assignment_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
class Person {}

object $person = new Person();
"#,
    )
    .expect("semantic check should succeed");

    assert_type_mismatch("object $value = 1;");
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
List<object> $objects = [new A(), new B()];
List<array> $arrays = [[1], ["k" => 2]];
Dictionary<string, int> $counts = [
    "apples" => 5,
];
Dictionary<string, object> $objectsByName = [
    "a" => new A(),
    "b" => new B(),
];
Dictionary<string, List<int>> $nestedCounts = [
    "apples" => [5],
    "oranges" => [],
];
Dictionary<int, int> $indexedCounts = [
    10,
    1 => 20,
];
Dictionary<string, int> $emptyCounts = [];
array $empty = [];
array $items = [1, 2, 3];
array $inventory = [
    "apples" => 5,
];
array $mixed = [1, "two"];
Set<string> $names = [];
array $itemsFromSet = $names;

class Inventory
{
    Dictionary<string, int> $counts = [];
    List<object> $objects = [new A(), new B()];
}

function readCounts(Dictionary<string, int> $counts = []): void
{
}

function readObjects(List<object> $objects = [new A(), new B()]): void
{
}
"#,
    )
    .expect("semantic check should succeed");

    for source in [
        r#"List<string> $numbers = [1, 2, 3];"#,
        r#"List<int> $numbers = [1, "two"];"#,
        r#"List<int> $numbers = [1, []];"#,
        r#"List<mixed> $numbers = [1, "two"];"#,
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
List<object> $objects = [new A(), 1];
"#,
        r#"
List<array> $arrays = [[1], 2];
"#,
        r#"
class A {}
Dictionary<string, object> $objectsByName = [
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
    ] {
        assert_type_mismatch(source);
    }
}

#[test]
fn checks_parameter_default_compatibility() {
    doriac::check_source(
        "test.doria",
        r#"
function greet(string $name = "Andrew"): void
{
}

class Person
{
    function __construct(string $name = "Andrew")
    {
    }

    function greet(string $name = Person::defaultName()): void
    {
    }

    function rename(string $name = "Lucy"): void
    {
    }

    internal function defaultName(): string
    {
        return "Andrew";
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
    Person $manager;

    function __construct(Person $owner)
    {
    }

    function index(List<Person> $people): Dictionary<string, Person>
    {
        foreach ($people as Person $person) {
            echo $person;
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
fn pascal_case_type_names_resolve_when_declared_as_classes() {
    doriac::check_source(
        "test.doria",
        r#"
class Int {}

Int $value = new Int();
"#,
    )
    .expect("semantic check should succeed");
}
