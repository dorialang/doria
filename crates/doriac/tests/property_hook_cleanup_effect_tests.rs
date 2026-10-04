const ITEM: &str = r#"
class Item {
    int $value = 42;
    function __destruct() {
        try { echo "item dropped"; } catch (Error) {}
    }
}
"#;

fn assert_cleanup_effect(body: &str, blocks: bool) {
    let source = format!("{ITEM}\n{body}");
    let (_, analysis) =
        doriac::analyze_source_for_ide("property-hook-cleanup-effects.doria", source.clone())
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0770"),
        blocks,
        "{source}\n{:#?}\nCleanup facts: {:#?}",
        analysis.diagnostics,
        analysis.info.cleanup
    );
    // No unrelated diagnostic may make a cleanup fixture appear to exercise
    // a valid ownership path.
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| { blocks && diagnostic.code == "E0770" }),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn returned_owned_items_do_not_charge_the_callers_cleanup_to_the_hook() {
    for getter in [
        "get => new Item();",
        "get { let $item = new Item(); return $item; }",
        "get => makeItem();",
    ] {
        assert_cleanup_effect(
            &format!(
                r#"
function makeItem(): Item {{ return new Item(); }}
class Factory {{ Item $item {{ {getter} }} }}
function main(): int {{
    let $factory = new Factory();
    let $item = $factory->item;
    return $item->value;
}}
"#
            ),
            false,
        );
    }
}

#[test]
fn readonly_borrowed_items_are_not_dropped_by_the_hook() {
    assert_cleanup_effect(
        r#"
class Store {
    Item $stored = new Item();
    Item $view { borrowed get => $this->stored; }
    int $value {
        get {
            let $borrowed = $this->view;
            return $borrowed->value;
        }
    }
}
"#,
        false,
    );
}

#[test]
fn returned_callbacks_transfer_owned_capture_cleanup_out_of_the_hook() {
    for body in [
        "let $item = new Item(); return fn() with (take $item) => $item->value;",
        "let $item = new Item(); let $callback = fn() with (take $item) => $item->value; let $alias = $callback; return $alias;",
        "let $item = new Item(); let $callback = fn() with (take $item) => $item->value; if ($callback() == 42) { return $callback; } return $callback;",
    ] {
        assert_cleanup_effect(
            &format!("class Factory {{ function(): int $callback {{ get {{ {body} }} }} }}"),
            false,
        );
    }
}

#[test]
fn returned_collections_transfer_element_cleanup_out_of_the_hook() {
    assert_cleanup_effect(
        r#"
class Factory {
    List<Item> $items {
        get {
            List<Item> $items = [new Item()];
            return $items;
        }
    }
}
"#,
        false,
    );
}

#[test]
fn local_item_cleanup_makes_the_hook_blocking() {
    assert_cleanup_effect(
        r#"
class Factory {
    int $value {
        get {
            let $item = new Item();
            return $item->value;
        }
    }
}
"#,
        true,
    );
}

#[test]
fn discarded_results_and_temporary_receivers_charge_cleanup_to_the_hook() {
    for getter in [
        "get { new Item(); return 42; }",
        "get { makeItem(); return 42; }",
        "get => (new Item())->value;",
    ] {
        assert_cleanup_effect(
            &format!(
                "function makeItem(): Item {{ return new Item(); }} class Factory {{ int $value {{ {getter} }} }}"
            ),
            true,
        );
    }
}

#[test]
fn replacement_drops_the_old_item_even_when_the_new_item_is_returned() {
    assert_cleanup_effect(
        r#"
class Factory {
    Item $item {
        get {
            let writable $item = new Item();
            $item = new Item();
            return $item;
        }
    }
}
"#,
        true,
    );
}

#[test]
fn nested_owned_fields_contribute_their_destructor_effects() {
    assert_cleanup_effect(
        r#"
class Inner { Item $item = new Item(); }
class Outer { Inner $inner = new Inner(); }
class Factory {
    int $value {
        get {
            let $outer = new Outer();
            return $outer->inner->item->value;
        }
    }
}
"#,
        true,
    );
}

#[test]
fn dropped_collections_contribute_element_destructor_effects() {
    assert_cleanup_effect(
        r#"
class Factory {
    int $count {
        get {
            List<Item> $items = [new Item()];
            return $items->count;
        }
    }
}
"#,
        true,
    );
}

#[test]
fn dropped_callbacks_contribute_owned_capture_destructor_effects() {
    assert_cleanup_effect(
        r#"
class Factory {
    int $value {
        get {
            let $item = new Item();
            let $callback = fn() with (take $item) => $item->value;
            return $callback();
        }
    }
}
"#,
        true,
    );
}

#[test]
fn helper_cleanup_is_part_of_the_hooks_transitive_effect() {
    assert_cleanup_effect(
        r#"
function inspect(): int {
    let $item = new Item();
    return $item->value;
}
class Factory { int $value { get => inspect(); } }
"#,
        true,
    );
}

#[test]
fn transferring_on_every_branch_does_not_run_the_item_destructor() {
    assert_cleanup_effect(
        r#"
class Factory {
    function __construct(bool $choose) {}
    Item $item {
        get {
            let $item = new Item();
            if ($this->choose) { return $item; }
            return $item;
        }
    }
}
"#,
        false,
    );
}

#[test]
fn transferring_on_one_branch_does_not_hide_cleanup_on_another() {
    assert_cleanup_effect(
        r#"
class Factory {
    function __construct(bool $choose) {}
    Item $item {
        get {
            let $item = new Item();
            if ($this->choose) { return $item; }
            return new Item();
        }
    }
}
"#,
        true,
    );
}

#[test]
fn checked_error_cleanup_counts_even_when_success_transfers_the_item() {
    assert_cleanup_effect(
        r#"
class Fault implements Error { function __construct(string $message) {} }
function maybeFail(bool $fail): void throws Fault {
    if ($fail) { throw new Fault("failure"); }
}
class Factory {
    function __construct(bool $fail) {}
    Item $item {
        get throws Fault {
            let $item = new Item();
            maybeFail($this->fail);
            return $item;
        }
    }
}
"#,
        true,
    );
}

#[test]
fn caught_error_without_a_binding_still_has_cleanup_effects() {
    assert_cleanup_effect(
        r#"
class Fault implements Error {
    function __construct(string $message) {}
    function __destruct() {
        try { echo "fault dropped"; } catch (Error) {}
    }
}
function fail(): void throws Fault { throw new Fault("failure"); }
class Factory {
    int $value {
        get {
            try { fail(); } catch (Fault) {}
            return 42;
        }
    }
}
"#,
        true,
    );
}

#[test]
fn failed_construction_cleans_initialized_fields_inside_the_hook() {
    assert_cleanup_effect(
        r#"
class Fault implements Error { function __construct(string $message) {} }
class Container {
    Item $item = new Item();
    function __construct(bool $fail) throws Fault {
        if ($fail) { throw new Fault("failure"); }
    }
}
class Factory {
    function __construct(bool $fail) {}
    Container $container { get throws Fault => new Container($this->fail); }
}
"#,
        true,
    );
}

#[test]
fn consuming_match_transfers_payload_cleanup_to_the_returned_item() {
    assert_cleanup_effect(
        r#"
enum Result { case Value(Item $item); }
class Factory {
    Item $item {
        get {
            let $result = Result::Value(new Item());
            return match (take $result) { Result::Value($item) => $item };
        }
    }
}
"#,
        false,
    );
}

#[test]
fn consuming_match_drops_only_payloads_not_transferred_by_the_selected_arm() {
    for arm in [
        "Result::Value($item) => $item->value",
        "Result::Value => 42",
    ] {
        assert_cleanup_effect(
            &format!(
                r#"
enum Result {{ case Value(Item $item); }}
class Factory {{
    int $value {{
        get {{
            let $result = Result::Value(new Item());
            return match (take $result) {{ {arm} }};
        }}
    }}
}}
"#
            ),
            true,
        );
    }
}

#[test]
fn borrowed_match_never_destroys_the_stored_payload() {
    assert_cleanup_effect(
        r#"
enum Result { case Value(Item $item); }
class Store {
    Result $stored = Result::Value(new Item());
    int $value {
        get => match ($this->stored) { Result::Value($item) => $item->value };
    }
}
"#,
        false,
    );
}

#[test]
fn default_match_cleanup_excludes_cases_already_transferred_by_an_unguarded_arm() {
    assert_cleanup_effect(
        r#"
enum Result { case Value(Item $item); case Empty; }
class Factory {
    Item $item {
        get {
            let $result = Result::Value(new Item());
            return match (take $result) {
                Result::Value($item) => $item,
                default => new Item(),
            };
        }
    }
}
"#,
        false,
    );
}

#[test]
fn failed_match_guard_preserves_payload_ownership_for_the_following_arm() {
    assert_cleanup_effect(
        r#"
enum Result { case Value(Item $item); }
class Factory {
    function __construct(bool $choose) {}
    Item $item {
        get {
            let $result = Result::Value(new Item());
            return match (take $result) {
                Result::Value($item) if $this->choose => $item,
                Result::Value($item) => $item,
            };
        }
    }
}
"#,
        false,
    );
}

#[test]
fn checked_failure_in_a_match_guard_releases_the_still_owned_payload() {
    assert_cleanup_effect(
        r#"
class Fault implements Error { function __construct(string $message) {} }
function select(bool $fail): bool throws Fault {
    if ($fail) { throw new Fault("failure"); }
    return true;
}
enum Result { case Value(Item $item); }
class Factory {
    function __construct(bool $fail) {}
    Item $item {
        get throws Fault {
            let $result = Result::Value(new Item());
            return match (take $result) {
                Result::Value($item) if select($this->fail) => $item,
                Result::Value($item) => $item,
            };
        }
    }
}
"#,
        true,
    );
}

#[test]
fn consuming_type_binding_transfers_the_narrowed_owner_without_a_second_drop() {
    for ty in ["?Item", "mixed"] {
        assert_cleanup_effect(
            &format!(
                r#"
function maybeItem(bool $present): ?Item {{
    if ($present) {{ return new Item(); }}
    return null;
}}
class Factory {{
    bool $present = true;
    Item $item {{
        get {{
            {ty} $value = maybeItem($this->present);
            return match (take $value) {{
                Item $item => $item,
                default => new Item(),
            }};
        }}
    }}
}}
"#
            ),
            false,
        );
    }
}

#[test]
fn nullable_match_null_paths_do_not_destroy_the_nonnull_payload() {
    for arms in [
        "null => new Item(), Item $item => $item",
        "Item $item if $this->present => $item, Item $item => $item, null => new Item()",
    ] {
        assert_cleanup_effect(
            &format!(
                r#"
function maybeItem(bool $present): ?Item {{
    if ($present) {{ return new Item(); }}
    return null;
}}
class Factory {{
    bool $present = true;
    Item $item {{
        get {{
            ?Item $value = maybeItem($this->present);
            return match (take $value) {{ {arms} }};
        }}
    }}
}}
"#
            ),
            false,
        );
    }
}

#[test]
fn nullable_match_failed_guards_keep_the_nonnull_payload_owned() {
    assert_cleanup_effect(
        r#"
function maybeItem(bool $present): ?Item {
    if ($present) { return new Item(); }
    return null;
}
class Factory {
    bool $present = true;
    Item $item {
        get {
            ?Item $value = maybeItem($this->present);
            return match (take $value) {
                Item $item if $this->present => $item,
                default => new Item(),
            };
        }
    }
}
"#,
        true,
    );
}

#[test]
fn consuming_enum_callback_bindings_keep_pure_origins_through_invocation_and_guards() {
    for arm in [
        "Callbacks::Value($callback) => $callback()",
        "Callbacks::Value($callback) if $callback() == 42 => $callback(), Callbacks::Value($callback) => $callback()",
    ] {
        assert_cleanup_effect(
            &format!(
                r#"
enum Callbacks {{ case Value(function(): int $callback); }}
class Factory {{
    int $value {{
        get {{
            let $result = Callbacks::Value(fn() => 42);
            return match (take $result) {{ {arm} }};
        }}
    }}
}}
"#
            ),
            false,
        );
    }
}

#[test]
fn consuming_enum_callback_cleanup_keeps_named_payload_fields_distinct() {
    for arguments in ["fn() => 42, $owned", "owned: $owned, pure: fn() => 42"] {
        assert_cleanup_effect(
            &format!(
                r#"
enum Callbacks {{ case Pair(function(): int $pure, function(): int $owned); }}
class Factory {{
    function(): int $callback {{
        get {{
            let $item = new Item();
            let $owned = fn() with (take $item) => $item->value;
            let $result = Callbacks::Pair({arguments});
            let $selected = match (take $result) {{
                Callbacks::Pair($pure, $owned) => $owned,
            }};
            return $selected;
        }}
    }}
}}
"#
            ),
            false,
        );
    }
}

#[test]
fn consuming_enum_callback_cleanup_follows_the_payload_that_is_not_returned() {
    for arm in [
        "Callbacks::Pair($pure, $owned) => $pure",
        "Callbacks::Pair($pure, $owned) => fn() => 42",
    ] {
        assert_cleanup_effect(
            &format!(
                r#"
enum Callbacks {{ case Pair(function(): int $pure, function(): int $owned); }}
class Factory {{
    function(): int $callback {{
        get {{
            let $item = new Item();
            let $owned = fn() with (take $item) => $item->value;
            let $result = Callbacks::Pair(owned: $owned, pure: fn() => 42);
            return match (take $result) {{ {arm} }};
        }}
    }}
}}
"#
            ),
            true,
        );
    }
}

#[test]
fn default_enum_callback_cleanup_uses_remaining_payload_origins() {
    for (setup, callback, blocks) in [
        ("", "fn() => 42", false),
        (
            "let $item = new Item();",
            "fn() with (take $item) => $item->value",
            true,
        ),
    ] {
        assert_cleanup_effect(
            &format!(
                r#"
enum Callbacks {{ case Empty; case Value(function(): int $callback); }}
class Factory {{
    int $value {{
        get {{
            {setup}
            let $result = Callbacks::Value(callback: {callback});
            return match (take $result) {{
                Callbacks::Empty => 0,
                default => 42,
            }};
        }}
    }}
}}
"#
            ),
            blocks,
        );
    }
}

#[test]
fn known_callback_effects_are_precise_without_property_hooks() {
    for source in [
        r#"function main(): int { let $callback = fn() => 42; return $callback(); }"#,
        r#"
function invoke(function(): int $callback): int { return $callback(); }
function main(): int { let $callback = fn() => 42; return invoke($callback); }
"#,
        r#"
function main(): int { let $callback = make(); return $callback(); }
function make(): function(): int { return fn() => 42; }
"#,
        r#"
function main(): void {
    let $callback = function(): void { try { echo "handled"; } catch (Error) {} };
    $callback();
}
"#,
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("callback-effects.doria", source)
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        assert!(!analysis.info.callable_value_calls.is_empty(), "{source}");
        for call in analysis.info.callable_value_calls.values() {
            assert!(call.checked_effects.is_empty(), "{source}\n{call:#?}");
        }
    }
}

#[test]
fn unknown_and_io_callback_effects_are_not_erased() {
    for (source, ambient_count, required_count) in [
        (
            "function invoke(function(): void $callback): void { $callback(); }",
            2,
            0,
        ),
        (
            r#"function main(): void { let $callback = function(): void { echo "io"; }; $callback(); }"#,
            1,
            0,
        ),
        (
            r#"
class Failure implements Error { function __construct(string $message) {} }
function invoke(function(): void throws Failure $callback): void throws Failure { $callback(); }
"#,
            2,
            1,
        ),
        (
            r#"
function choose(bool $io): int {
    let writable $callback = fn() => 42;
    if ($io) { $callback = function(): int { echo "io"; return 42; }; }
    return $callback();
}
"#,
            1,
            0,
        ),
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("callback-effects.doria", source)
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        assert_eq!(analysis.info.callable_value_calls.len(), 1, "{source}");
        let call = analysis.info.callable_value_calls.values().next().unwrap();
        assert_eq!(
            call.ambient_checked_effects.len(),
            ambient_count,
            "{source}\n{call:#?}"
        );
        assert_eq!(
            call.required_checked_effects.len(),
            required_count,
            "{source}\n{call:#?}"
        );
    }
}

#[test]
fn collection_mutation_preserves_callback_effects_before_ownership() {
    for (ty, initial, mutation, iterable) in [
        (
            "List<function(): void>",
            "[function(): void {}]",
            "$callbacks->add(function(): void { echo \"io\"; });",
            "$callbacks",
        ),
        (
            "List<function(): void>",
            "[function(): void {}]",
            "$callbacks->insertAt(0, function(): void { echo \"io\"; });",
            "$callbacks",
        ),
        (
            "(function(): void)[]",
            "[function(): void {}]",
            "$callbacks[0] = function(): void { echo \"io\"; };",
            "$callbacks",
        ),
        (
            "Dictionary<string, function(): void>",
            "[\"pure\" => function(): void {}]",
            "$callbacks->set(\"io\", function(): void { echo \"io\"; });",
            "$callbacks->values",
        ),
        (
            "SortedDictionary<string, function(): void>",
            "SortedDictionary::from([])",
            "$callbacks->set(\"io\", function(): void { echo \"io\"; });",
            "$callbacks->values",
        ),
        (
            "Deque<function(): void>",
            "Deque::from([])",
            "$callbacks->pushFront(function(): void { echo \"io\"; });",
            "$callbacks",
        ),
        (
            "Deque<function(): void>",
            "Deque::from([])",
            "$callbacks->pushBack(function(): void { echo \"io\"; });",
            "$callbacks",
        ),
    ] {
        for via_helper in [false, true] {
            for performs_io in [false, true] {
                let mutation = if performs_io {
                    mutation.to_string()
                } else {
                    mutation.replace("echo \"io\";", "")
                };
                let helper = if via_helper {
                    format!("function update(writable {ty} $callbacks): void {{ {mutation} }}")
                } else {
                    String::new()
                };
                let update = if via_helper {
                    "update($callbacks);"
                } else {
                    &mutation
                };
                let source = format!(
                    r#"
{helper}
function main(): void {{
    writable {ty} $callbacks = {initial};
    {update}
    foreach ({iterable} as function(): void $callback) {{ $callback(); }}
}}
"#
                );
                let (_, analysis) =
                    doriac::analyze_source_for_ide("collection-callback-effects.doria", &source)
                        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
                assert!(
                    analysis.diagnostics.is_empty(),
                    "{source}\n{:#?}",
                    analysis.diagnostics
                );
                assert_eq!(analysis.info.callable_value_calls.len(), 1, "{source}");
                let call = analysis.info.callable_value_calls.values().next().unwrap();
                assert_eq!(
                    call.ambient_checked_effects.len(),
                    usize::from(performs_io),
                    "{source}\n{call:#?}"
                );
            }
        }
    }
}

#[test]
fn writable_callback_captures_propagate_collection_mutation_effects() {
    let source = r#"
function main(): void {
    writable List<function(): void> $callbacks = [function(): void {}];
    {
        let writable $update = function(): void with (writable $callbacks) {
            $callbacks->add(function(): void { echo "io"; });
        };
        $update();
    }
    foreach ($callbacks as function(): void $callback) { $callback(); }
}
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("collection-capture-effects.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let target = source.find("$callback();").unwrap();
    let call = analysis
        .info
        .callable_value_calls
        .values()
        .find(|call| call.callee_span.start == target)
        .unwrap();
    assert_eq!(call.ambient_checked_effects.len(), 1, "{call:#?}");
}

#[test]
fn returning_filled_arrays_does_not_transfer_fresh_seed_cleanup() {
    for collection in ["List<FillItem>", "FillItem[]"] {
        for count in [0, 2] {
            assert_cleanup_effect(
                &format!(
                    r#"
class FillItem implements Cloneable {{
    function clone(): self {{ return new FillItem(); }}
    function __destruct() {{
        try {{ echo "seed or clone dropped"; }} catch (Error) {{}}
    }}
}}
class Factory {{
    {collection} $items {{ get => [new FillItem(); {count}]; }}
}}
"#
                ),
                true,
            );
        }
    }
}

#[test]
fn returning_filled_arrays_does_not_destroy_borrowed_seeds() {
    for collection in ["List<FillItem>", "FillItem[]"] {
        for count in [0, 2] {
            assert_cleanup_effect(
                &format!(
                    r#"
class FillItem implements Cloneable {{
    function clone(): self {{ return new FillItem(); }}
    function __destruct() {{
        try {{ echo "seed or clone dropped"; }} catch (Error) {{}}
    }}
}}
class Factory {{
    FillItem $seed = new FillItem();
    {collection} $items {{ get => [$this->seed; {count}]; }}
}}
"#
                ),
                false,
            );
        }
    }
}

#[test]
fn filled_array_cleanup_values_keep_seed_and_clones_separately_owned() {
    use doriac::ownership::cleanup::Source;

    let source = r#"
class FillItem implements Cloneable {
    function clone(): self { return new FillItem(); }
}
class Factory {
    List<FillItem> $items { get => [new FillItem(); 2]; }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("fill-cleanup-origins.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let cleanup = &analysis.info.cleanup;
    let seed_start = source.rfind("new FillItem()").unwrap();
    let (seed_id, _) = cleanup
        .values
        .iter()
        .find(|(_, value)| matches!(value.source, Source::Expression(span) if span.start == seed_start))
        .expect("fresh fill seed has its own cleanup owner");
    let (clone_id, clone) = cleanup
        .values
        .iter()
        .find(|(_, value)| matches!(value.source, Source::CollectionElement { key: false, .. }))
        .expect("filled elements have their own cleanup owner");
    let Source::CollectionElement { collection, .. } = clone.source else {
        unreachable!();
    };
    let (_, array) = cleanup
        .values
        .iter()
        .find(|(_, value)| value.source == Source::Expression(collection))
        .expect("array owns the cloned elements");
    assert_ne!(seed_id, clone_id);
    assert!(array.contents.contains(clone_id));
    assert!(!array.contents.contains(seed_id));
    assert!(cleanup
        .releases
        .iter()
        .any(|release| release.values.contains(seed_id)));
    assert!(!cleanup
        .releases
        .iter()
        .any(|release| release.values.contains(clone_id)));
}
