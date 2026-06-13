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
    public string $name;
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
    public writable string $name;

    public function rename(string $name): void
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
    public writable string $name;

    public writable function rename(string $name): void
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
