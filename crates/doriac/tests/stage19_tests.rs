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

#[test]
fn borrowed_class_parameters_and_this_cannot_be_given_away() {
    for source in [
        "class Guard {} function consume(take Guard $guard): void {} function wrapper(Guard $guard): void { consume($guard); }",
        "class Guard { function giveSelf(): void { consume($this); } } function consume(take Guard $guard): void {}",
    ] {
        let diagnostics = doriac::check_source("borrowed.doria", source)
            .expect_err("borrowed class values cannot be transferred");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0474" && diagnostic.message.contains("cannot be given away")
        }));
    }
}

#[test]
fn method_and_static_take_parameters_move_their_arguments() {
    for source in [
        "class Guard {} class Box { function consume(take Guard $guard): void {} } function test(Box $box, take Guard $guard): void { $box->consume($guard); $box->consume($guard); }",
        "class Guard {} class Box { static function consume(take Guard $guard): void {} } function test(take Guard $guard): void { Box::consume($guard); Box::consume($guard); }",
    ] {
        let diagnostics = doriac::check_source("method-take.doria", source)
            .expect_err("the second ownership transfer must be rejected");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"));
    }
}

#[test]
fn class_typed_properties_cannot_be_moved_out_directly() {
    let source = "class Child {} class Parent { function __construct(take Child $child) {} function release(): void { consume($this->child); } } function consume(take Child $child): void {}";
    let diagnostics = doriac::check_source("property-move.doria", source)
        .expect_err("direct property moves remain unsupported");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0472" && diagnostic.message.contains("moves out")
    }));
}

#[test]
fn top_level_ownership_state_is_preserved_between_statements() {
    let source = "class Guard {} function consume(take Guard $guard): void {} let $guard = new Guard(); consume($guard); consume($guard);";
    let diagnostics = doriac::check_source("top-level-move.doria", source)
        .expect_err("top-level use after move must be rejected");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn terminating_branch_does_not_poison_the_fallthrough_owner() {
    doriac::check_source(
        "branch-return.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(bool $condition, take Guard $guard): void { if ($condition) { consume($guard); return; } consume($guard); }",
    )
    .expect("the moved branch cannot reach the second transfer");
}

#[test]
fn parenthesized_self_moves_are_rejected() {
    let program = doriac::parse_source(
        "grouped-self-move.doria",
        "class Guard {} function reset(take Guard $guard): void { $guard = ($guard); }",
    )
    .expect("self-move source should parse");
    let diagnostics = doriac::ownership::check_program(&program);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0471"));
}

#[test]
fn repeatable_loop_body_cannot_move_the_same_owner_twice() {
    let diagnostics = doriac::check_source(
        "loop-move.doria",
        "class Guard {} function consume(take Guard $guard): void {} function repeat(bool $again, take Guard $guard): void { while ($again) { consume($guard); } }",
    )
    .expect_err("a later loop iteration would transfer an already moved value");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn writable_move_promotion_fix_replaces_writable_with_take() {
    let source = "class Person {} class Team { function __construct(writable Person $manager) {} }";
    let diagnostics = doriac::check_source("writable-promotion.doria", source)
        .expect_err("writable promotion must transfer ownership instead");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0468")
        .expect("promotion diagnostic");
    let fix = diagnostic.fix.as_ref().expect("replacement fix");
    assert_eq!(fix.replacement, "take");
    assert_eq!(&source[fix.span.start..fix.span.end], "writable");
}
