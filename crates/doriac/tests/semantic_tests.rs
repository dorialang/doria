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
