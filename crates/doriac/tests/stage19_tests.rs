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
fn unreachable_false_loop_body_does_not_move_its_owner() {
    doriac::check_source(
        "false-loop-move.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(take Guard $guard): void { while (false) { consume($guard); } consume($guard); }",
    )
    .expect("a literal-false loop body cannot give away its owner");
}

#[test]
fn unreachable_false_for_body_does_not_move_its_owner() {
    doriac::check_source(
        "false-for-move.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(take Guard $guard): void { for (; false; ) { consume($guard); } consume($guard); }",
    )
    .expect("a literal-false for body cannot give away its owner");
}

#[test]
fn inferred_mixed_returns_are_move_values_for_functions_and_methods() {
    for source in [
        "function make() { mixed $value = 1; return $value; } function duplicate(): void { let $first = make(); let $second = $first; let $third = $first; }",
        "class Factory { function make() { mixed $value = 1; return $value; } } function duplicate(Factory $factory): void { let $first = $factory->make(); let $second = $first; let $third = $first; }",
    ] {
        let diagnostics = doriac::check_source("inferred-mixed-move.doria", source)
            .expect_err("an inferred mixed return must not create multiple owners");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"));
    }
}

#[test]
fn inferred_mixed_return_transfers_the_return_expression() {
    let diagnostics = doriac::check_source(
        "inferred-mixed-return.doria",
        "function forward(mixed $value) { return $value; }",
    )
    .expect_err("returning a borrowed inferred-mixed value would create a second owner");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0474"));
}

#[test]
fn collection_take_parameters_transfer_their_owners() {
    for collection in ["List<Guard>", "Guard[]"] {
        let source = format!(
            "class Guard {{}} function sink(take {collection} $items): void {{}} function twice(take {collection} $items): void {{ sink($items); sink($items); }}"
        );
        let diagnostics = doriac::check_source("collection-take.doria", source)
            .expect_err("the collection owner cannot be transferred twice");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"));
    }
}

#[test]
fn assigning_an_array_to_mixed_moves_its_elements() {
    let diagnostics = doriac::check_source(
        "mixed-array-assignment.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(take Guard $guard): void { writable mixed $slot = 1; $slot = [$guard]; consume($guard); }",
    )
    .expect_err("the mixed array becomes the owner of its class element");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn property_borrows_overlap_with_taking_their_root_owner() {
    let diagnostics = doriac::check_source(
        "property-root-overlap.doria",
        "class Child {} class Parent { function __construct(take Child $child) {} } function inspect(Child $child, take Parent $parent): void {} function route(take Parent $parent): void { inspect($parent->child, $parent); }",
    )
    .expect_err("a property borrow cannot overlap taking its root owner");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0471"));
}

#[test]
fn borrowed_this_cannot_initialize_a_new_owner() {
    let diagnostics = doriac::check_source(
        "this-owner-copy.doria",
        "class Guard { function duplicate(): void { let $copy = $this; consume($copy); } } function consume(take Guard $guard): void {}",
    )
    .expect_err("a local owner cannot be created from the borrowed receiver");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0474"));
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

#[test]
fn loop_conditions_recheck_ownership_transfers_on_backedges() {
    let diagnostics = doriac::check_source(
        "loop-condition-move.doria",
        "class Guard {} function poll(take Guard $guard): bool { return false; } function repeat(take Guard $guard): void { while (poll($guard)) {} }",
    )
    .expect_err("each loop-condition evaluation would transfer the same owner");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn break_exits_contribute_to_post_loop_ownership() {
    let diagnostics = doriac::check_source(
        "loop-break-move.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(bool $again, take Guard $guard): void { while ($again) { consume($guard); break; } consume($guard); }",
    )
    .expect_err("a break path can reach the use after transferring ownership");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn promoted_take_parameter_is_owned_by_the_property_before_the_body() {
    let diagnostics = doriac::check_source(
        "promoted-take-body.doria",
        "class Child {} function consume(take Child $child): void {} class Parent { function __construct(take Child $child) { consume($child); } }",
    )
    .expect_err("the promoted property already owns the constructor argument");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn panic_branch_does_not_reach_the_fallthrough_owner() {
    doriac::check_source(
        "panic-terminates.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(bool $bad, take Guard $guard): void { if ($bad) { consume($guard); panic(\"stop\"); } consume($guard); }",
    )
    .expect("panic terminates the branch after its ownership transfer");
}

#[test]
fn borrowed_and_owned_branch_join_is_conservative() {
    let diagnostics = doriac::check_source(
        "borrowed-owned-join.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(bool $replace, writable Guard $slot): void { if ($replace) { $slot = new Guard(); } consume($slot); }",
    )
    .expect_err("a conditionally borrowed binding cannot be transferred");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0474"));
}

#[test]
fn assigning_a_class_owner_into_mixed_moves_the_source() {
    let diagnostics = doriac::check_source(
        "mixed-owner.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(take Guard $guard): void { writable mixed $slot = 1; $slot = $guard; consume($guard); }",
    )
    .expect_err("storing the owner in mixed must consume the source binding");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn for_increment_ownership_transfers_are_rechecked() {
    let diagnostics = doriac::check_source(
        "for-increment-move.doria",
        "class Guard {} function repeat(bool $again, take Guard $guard): void { writable mixed $slot = 1; for (; $again; $slot = $guard) {} }",
    )
    .expect_err("a later for increment would transfer an already moved owner");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn mixed_declaration_initializers_move_the_box() {
    let diagnostics = doriac::check_source(
        "mixed-declaration-move.doria",
        "function duplicate(): void { mixed $first = 1; mixed $second = $first; mixed $third = $first; }",
    )
    .expect_err("a mixed box has only one owner");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn promoted_mixed_parameter_requires_take() {
    let source = "class Box { function __construct(mixed $payload) {} }";
    let diagnostics = doriac::check_source("mixed-promotion.doria", source)
        .expect_err("promoting mixed must transfer its box");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0468")
        .expect("mixed promotion diagnostic");
    assert_eq!(
        diagnostic.fix.as_ref().map(|fix| fix.replacement.as_str()),
        Some("take ")
    );

    doriac::check_source(
        "mixed-promotion-ok.doria",
        "class Box { function __construct(take mixed $payload) {} }",
    )
    .expect("take mixed transfers the promoted box");
}

#[test]
fn take_mixed_parameters_are_tracked_as_move_owners() {
    let diagnostics = doriac::check_source(
        "take-mixed.doria",
        "function sink(take mixed $value): void {} function twice(take mixed $value): void { sink($value); sink($value); }",
    )
    .expect_err("the first call consumes the mixed box");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn a_call_cannot_borrow_and_take_the_same_owner() {
    for source in [
        "class Guard {} function inspect(Guard $borrowed, take Guard $owned): void {} function route(take Guard $guard): void { inspect($guard, $guard); }",
        "class Guard { function consume(take Guard $owned): void {} } function route(take Guard $guard): void { $guard->consume($guard); }",
    ] {
        let diagnostics = doriac::check_source("overlapping-call.doria", source)
            .expect_err("a call cannot borrow and take one owner simultaneously");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0471" && diagnostic.message.contains("same call")
        }));
    }
}

#[test]
fn assigning_through_a_writable_parameter_keeps_it_borrowed() {
    let diagnostics = doriac::check_source(
        "writable-owner.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(writable Guard $slot): void { $slot = new Guard(); consume($slot); }",
    )
    .expect_err("the caller owns the replacement stored through its borrowed slot");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0474"));
}

#[test]
fn boolean_short_circuiting_preserves_unreached_owners() {
    for condition in ["false && probe($guard)", "true || probe($guard)"] {
        let source = format!(
            "class Guard {{}} function probe(take Guard $guard): bool {{ return true; }} function consume(take Guard $guard): void {{}} function route(take Guard $guard): void {{ if ({condition}) {{}} consume($guard); }}"
        );
        doriac::check_source("short-circuit.doria", source)
            .expect("the right operand is unreachable and cannot consume the owner");
    }

    let diagnostics = doriac::check_source(
        "conditional-short-circuit.doria",
        "class Guard {} function probe(take Guard $guard): bool { return true; } function consume(take Guard $guard): void {} function route(bool $enabled, take Guard $guard): void { if ($enabled && probe($guard)) {} consume($guard); }",
    )
    .expect_err("a reachable right operand may consume the owner");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn inferred_mixed_locals_are_tracked_as_move_owners() {
    let diagnostics = doriac::check_source(
        "inferred-mixed-owner.doria",
        "function make(): mixed { mixed $value = 1; return $value; } function duplicate(): void { let $first = make(); let $second = $first; let $third = $first; }",
    )
    .expect_err("an inferred mixed local still owns one box");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn mixed_owner_moves_into_properties_remain_unsupported() {
    let diagnostics = doriac::check_source(
        "mixed-property-assignment.doria",
        "function sink(take mixed $value): void {} class Box { writable mixed $payload = 1; writable function store(take mixed $value): void { $this->payload = $value; sink($value); } }",
    )
    .expect_err("direct moves into properties remain unsupported");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0472" && diagnostic.message.contains("moves into")
    }));
}

#[test]
fn mixed_properties_are_move_values() {
    for source in [
        "class Box { mixed $payload = 1; function release(): mixed { return $this->payload; } }",
        "function sink(take mixed $value): void {} class Box { mixed $payload = 1; function release(): void { sink($this->payload); } }",
    ] {
        let diagnostics = doriac::check_source("mixed-property-move.doria", source)
            .expect_err("direct moves out of mixed properties remain unsupported");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0472" && diagnostic.message.contains("moves out")
        }));
    }
}

#[test]
fn owning_array_literals_move_their_elements() {
    let diagnostics = doriac::check_source(
        "array-element-move.doria",
        "class Guard {} function consume(take Guard $guard): void {} function route(take Guard $guard): void { mixed $payload = [$guard]; consume($guard); }",
    )
    .expect_err("the array literal owns its class element");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn collection_reassignment_moves_the_source_owner() {
    for collection in [
        "Guard[]",
        "List<Guard>",
        "Dictionary<string, Guard>",
        "Set<Guard>",
    ] {
        let source = format!(
            "class Guard {{}} function sink(take {collection} $items): void {{}} function route(take {collection} $src): void {{ writable {collection} $dst = []; $dst = $src; sink($src); }}"
        );
        let diagnostics = doriac::check_source("collection-reassignment.doria", source)
            .expect_err("assigning the collection transfers its owner");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"));
    }
}

#[test]
fn borrowed_array_arguments_still_move_owned_elements_into_the_temporary() {
    let diagnostics = doriac::check_source(
        "borrowed-array-element.doria",
        "class Guard {} function inspect(Guard[] $items): void {} function consume(take Guard $guard): void {} function route(take Guard $guard): void { inspect([$guard]); consume($guard); }",
    )
    .expect_err("the temporary array owns its class element");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0470"));
}

#[test]
fn promoted_collection_parameters_require_take() {
    for collection in [
        "Guard[]",
        "List<Guard>",
        "Dictionary<string, Guard>",
        "Set<Guard>",
    ] {
        let source = format!(
            "class Guard {{}} class Box {{ function __construct({collection} $items) {{}} }}"
        );
        let diagnostics = doriac::check_source("collection-promotion.doria", source)
            .expect_err("promotion must transfer collection ownership");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0468"));

        doriac::check_source(
            "collection-promotion-take.doria",
            format!(
                "class Guard {{}} class Box {{ function __construct(take {collection} $items) {{}} }}"
            ),
        )
        .expect("take transfers the collection into the promoted property");
    }
}

#[test]
fn collection_properties_are_move_values() {
    for collection in [
        "Guard[]",
        "List<Guard>",
        "Dictionary<string, Guard>",
        "Set<Guard>",
    ] {
        let source = format!(
            "class Guard {{}} function sink(take {collection} $items): void {{}} class Box {{ {collection} $items = []; function release(): void {{ sink($this->items); }} }}"
        );
        let diagnostics = doriac::check_source("collection-property-move.doria", source)
            .expect_err("direct collection-property moves remain unsupported");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0472" && diagnostic.message.contains("moves out")
        }));
    }
}

#[test]
fn literal_if_conditions_skip_unreachable_owner_moves() {
    for body in [
        "if (false) { consume($guard); } consume($guard);",
        "if (true) {} else { consume($guard); } consume($guard);",
    ] {
        let source = format!(
            "class Guard {{}} function consume(take Guard $guard): void {{}} function route(take Guard $guard): void {{ {body} }}"
        );
        doriac::check_source("literal-if.doria", source)
            .expect("unreachable branches cannot consume the owner");
    }
}
