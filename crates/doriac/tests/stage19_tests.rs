use doriac::ast::{ClassMember, Item};

#[test]
fn take_is_a_parameter_only_ownership_modifier() {
    let program = doriac::parse_source(
        "take.doria",
        "class Guard {} function consume(take Guard $guard): void { return; }",
    )
    .expect("take parameter should parse");
    let function = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) => Some(function),
            _ => None,
        })
        .expect("consume declaration");
    assert!(function.params[0].take);
    assert!(!function.params[0].writable);

    let error = doriac::parse_source(
        "take-local.doria",
        "function main(): void { take int $value = 1; }",
    )
    .expect_err("take must not become local-declaration syntax");
    assert!(error
        .iter()
        .any(|diagnostic| diagnostic.code.starts_with('P')));
}

#[test]
fn take_and_writable_are_rejected_in_both_orders() {
    for modifiers in ["take writable", "writable take"] {
        let source = format!(
            "class Guard {{}} function consume({modifiers} Guard $guard): void {{ return; }}"
        );
        let diagnostics = doriac::check_source("exclusive.doria", source)
            .expect_err("ownership transfer and writable borrow are exclusive");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "E0467")
            .expect("mutual-exclusion diagnostic");
        assert!(!diagnostic.message.contains("clone"));
    }
}

#[test]
fn promoted_move_parameter_requires_take_with_an_insertion_fix() {
    let source = "class Person {} class Team { function __construct(Person $manager) {} }";
    let diagnostics = doriac::check_source("promotion.doria", source)
        .expect_err("move promotion without transfer would create two owners");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0468")
        .expect("promotion diagnostic");
    let fix = diagnostic.fix.as_ref().expect("take insertion fix");
    assert_eq!(fix.replacement, "take ");
    assert_eq!(fix.span.start, source.find("Person $manager").unwrap());
    assert_eq!(fix.span.start, fix.span.end);

    doriac::check_source(
        "promotion-ok.doria",
        "class Person {} class Team { function __construct(take Person $manager) {} }",
    )
    .expect("taking promotion should be accepted");
}

#[test]
fn explicit_then_promoted_property_order_has_stable_ids() {
    let hir = doriac::lower_source(
        "order.doria",
        "class Person {} class Team { int8 $tag = 1; string $name = \"x\"; function __construct(take Person $manager, int $count) {} } function main(): void { return; }",
    )
    .expect("ordered class should check and lower");
    let team = hir
        .semantic_info
        .classes
        .iter()
        .find(|class| class.name == "Team")
        .expect("Team metadata");
    assert_eq!(
        team.properties
            .iter()
            .map(|property| property.name.as_str())
            .collect::<Vec<_>>(),
        ["tag", "name", "manager", "count"]
    );
    assert_eq!(
        team.properties
            .iter()
            .map(|property| property.id.index)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );

    let class = hir.items.iter().find_map(|item| match item {
        doriac::hir::Item::Class(class) if class.name == "Team" => Some(class),
        _ => None,
    });
    assert!(class.is_some());
}

#[test]
fn use_after_move_reports_the_give_away_point_without_clone_advice() {
    let source = r#"
class Guard {}
function consume(take Guard $guard): void { return; }
function main(): void {
    let $guard = new Guard();
    consume($guard);
    consume($guard);
}
"#;
    let diagnostics =
        doriac::check_source("move.doria", source).expect_err("second give must be rejected");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0470")
        .expect("use-after-move diagnostic");
    assert!(diagnostic.message.contains("given away"));
    assert!(diagnostic
        .help
        .as_deref()
        .unwrap()
        .contains("cannot be used afterward"));
    assert!(!format!("{diagnostic:?}").contains("clone"));
}

#[test]
fn move_on_one_branch_is_not_owned_after_the_join() {
    let source = r#"
class Guard {}
function consume(take Guard $guard): void { return; }
function choose(bool $condition): void {
    let $guard = new Guard();
    if ($condition) { consume($guard); }
    consume($guard);
}
"#;
    let diagnostics = doriac::check_source("branch.doria", source)
        .expect_err("a maybe-given value cannot be used after a join");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn runtime_class_layout_is_headerless_and_pointer_aware() {
    use doriac::class_layout::{compute_class_layout, ClassId, FieldType, PropertyId};
    use doriac::numeric::IntegerType;

    let class = ClassId(4);
    let layout = compute_class_layout(
        class,
        [
            (
                PropertyId { class, index: 0 },
                FieldType::Integer(IntegerType::Int8),
            ),
            (PropertyId { class, index: 1 }, FieldType::String),
            (PropertyId { class, index: 2 }, FieldType::Bool),
        ],
        8,
    );
    assert_eq!(layout.properties[0].offset, 0);
    assert_eq!(layout.properties[1].offset, 8);
    assert_eq!(layout.properties[2].offset, 16);
    assert_eq!(layout.size, 24);
    assert_eq!(layout.align, 8);
}

#[test]
fn constructor_parameter_is_still_a_promoted_property_in_the_ast() {
    let program = doriac::parse_source(
        "promotion-ast.doria",
        "class Person {} class Team { function __construct(take Person $manager) {} }",
    )
    .unwrap();
    let team = program.items.iter().find_map(|item| match item {
        Item::Class(class) if class.name == "Team" => Some(class),
        _ => None,
    });
    let constructor = team
        .unwrap()
        .members
        .iter()
        .find_map(|member| match member {
            ClassMember::Method(method) if method.name == "__construct" => Some(method),
            _ => None,
        });
    assert!(constructor.unwrap().params[0].promoted_access.is_some());
}
