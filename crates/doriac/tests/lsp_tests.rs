use serde_json::Value;

use doriac::lsp::{
    byte_offset_to_position, code_actions_for_document, diagnostics_for_document,
    position_to_byte_offset,
};

#[test]
fn maps_byte_offsets_to_utf16_lsp_positions() {
    let text = "let $name = \"Zoë\";\nlet $emoji = \"😀\";\n";

    let first_newline = text.find('\n').expect("fixture should contain newline");
    let emoji = text.find('😀').expect("fixture should contain emoji");

    assert_eq!(byte_offset_to_position(text, 0).line, 0);
    assert_eq!(byte_offset_to_position(text, first_newline + 1).line, 1);
    assert_eq!(byte_offset_to_position(text, emoji).character, 14);
    assert_eq!(
        byte_offset_to_position(text, emoji + "😀".len()).character,
        16
    );
}

#[test]
fn maps_utf16_lsp_positions_to_byte_offsets() {
    let text = "let $emoji = \"😀\";\n";
    let emoji = text.find('😀').expect("fixture should contain emoji");

    assert_eq!(position_to_byte_offset(text, 0, 14), emoji);
    assert_eq!(position_to_byte_offset(text, 0, 15), emoji);
    assert_eq!(position_to_byte_offset(text, 0, 16), emoji + "😀".len());
}

#[test]
fn exposes_compiler_diagnostics_as_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///test.doria",
        r#"let $count = 0;
$count = 1;
"#,
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0]["source"],
        Value::String("doriac".to_string())
    );
    assert_eq!(diagnostics[0]["code"], Value::String("E0201".to_string()));
    assert_eq!(diagnostics[0]["range"]["start"]["line"], Value::from(1));
    assert_eq!(
        diagnostics[0]["range"]["start"]["character"],
        Value::from(0)
    );
    assert!(diagnostics[0]["message"]
        .as_str()
        .expect("message should be string")
        .contains("readonly variable"));
}

#[test]
fn exposes_literal_brace_fix_data_at_original_source_span() {
    let text = "echo \"literal {word}\";";
    let diagnostics = diagnostics_for_document("file:///brace.doria", text);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["code"] == "P0002")
        .expect("literal brace diagnostic should be published");

    assert_eq!(diagnostic["data"]["fix"]["newText"], "\\{");
    assert_eq!(diagnostic["data"]["fix"]["range"]["start"]["line"], 0);
    assert_eq!(
        diagnostic["data"]["fix"]["range"]["start"]["character"],
        text.find('{').expect("opening brace")
    );
}

#[test]
fn exposes_literal_brace_fix_as_a_preferred_code_action() {
    let uri = "file:///brace.doria";
    let text = "echo \"literal {word}\";";
    let actions = code_actions_for_document(uri, text);

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["kind"], "quickfix");
    assert_eq!(actions[0]["isPreferred"], true);
    assert_eq!(actions[0]["edit"]["changes"][uri][0]["newText"], "\\{");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["range"]["start"]["character"],
        text.find('{').expect("opening brace")
    );
}

#[test]
fn exposes_writable_constructor_removal_as_a_preferred_code_action() {
    let uri = "file:///lifecycle.doria";
    let text = "class Person { writable function __construct() {} }";
    let actions = code_actions_for_document(uri, text);
    let action = actions
        .iter()
        .find(|action| {
            action["title"]
                .as_str()
                .is_some_and(|title| title.contains("construction grants `__construct`"))
        })
        .expect("writable lifecycle diagnostic should expose a quick fix");

    assert_eq!(action["kind"], "quickfix");
    assert_eq!(action["isPreferred"], true);
    assert_eq!(action["edit"]["changes"][uri][0]["newText"], "");
    assert_eq!(
        action["edit"]["changes"][uri][0]["range"]["start"]["character"],
        text.find("writable").expect("writable modifier")
    );
}

#[test]
fn accepts_boolean_word_operators_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///operators.doria",
        r#"let $a = true and false;
let $b = false or true;
let $c = not false;
let $d = true xor false;
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn accepts_control_flow_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///control_flow.doria",
        r#"let writable $count = 0;

while ($count < 3) {
    if ($count == 0) {
        echo "zero";
    } else if ($count == 1) {
        echo "one";
    } else {
        echo "many";
    }

    echo "\n";
    $count += 1;
}
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn accepts_builtin_panic_without_lsp_diagnostics() {
    let diagnostics = diagnostics_for_document(
        "file:///main_explicit_panic.doria",
        r#"function main(): void
{
    panic("explicit panic");
}
"#,
    );

    assert_eq!(diagnostics, Vec::<Value>::new());
}

#[test]
fn publishes_stable_semantic_diagnostics_for_class_workflow_syntax() {
    let diagnostics = diagnostics_for_document(
        "file:///Child.doria",
        r#"namespace Vendor\App;
interface Printable {}
class Child extends Vendor\Base implements Vendor\Contracts\Printable {}
"#,
    );

    assert!(diagnostics.iter().all(|diagnostic| {
        !diagnostic["code"]
            .as_str()
            .is_some_and(|code| code.starts_with('P'))
    }));
    for code in ["E0475", "E0476", "E0464"] {
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == code));
    }
}
