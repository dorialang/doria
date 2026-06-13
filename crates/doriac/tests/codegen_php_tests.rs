#[test]
fn emits_php_for_simple_program() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
let writable $count = 0;
$count += 1;
echo $count;
"#,
    )
    .expect("compilation should succeed");

    assert!(php.starts_with("<?php"));
    assert!(php.contains("$count = 0;"));
    assert!(php.contains("$count += 1;"));
    assert!(php.contains("echo $count;"));
}

#[test]
fn strips_doria_writable_from_php_output() {
    let php = doriac::compile_source_to_php(
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
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("public string $name;"));
    assert!(php.contains("public function rename(string $name): void"));
    assert!(!php.contains("writable"));
}
