#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::backend::NativeProfile;
use doriac::diagnostics::RuntimeFactValue;
use doriac::mir::Type;
use doriac::types::ByteRegionKind;

const PATH: &str = "byte-region-execution.doria";
const IMPORTS: &str = "use Doria\\Std\\Io\\{ByteRegion, WritableByteRegion};\n";

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir(PATH, source).unwrap_or_else(|errors| {
        let hir = doriac::lower_source(PATH, source).unwrap();
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
        panic!("{source}\n{errors:#?}\n{mir}")
    })
}

#[test]
fn nullable_region_callback_replacements_keep_original_slots() {
    for (declaration, access, observation) in [
        ("writable List<ByteRegion> $items = [$bytes->getRegion(0, 1)];", "$items[0]", "echo $items[0][0];"),
        ("writable List<ByteRegion> $items = [$bytes->getRegion(0, 1)]; let writable $copy = $items->first;", "$copy", "if ($copy != null) { echo $copy[0]; } echo $items[0][0];"),
        ("writable List<?ByteRegion> $items = [null];", "$items[0]", "let $view = $items[0]; if ($view != null) { echo $view[0]; }"),
        ("writable Dictionary<string, ByteRegion> $items = [\"key\" => $bytes->getRegion(0, 1)];", "$items[\"key\"]", "echo $items[\"key\"][0];"),
        ("writable Dictionary<string, ByteRegion> $items = [\"key\" => $bytes->getRegion(0, 1)]; let writable $copy = $items->get(\"key\");", "$copy", "if ($copy != null) { echo $copy[0]; } echo $items[\"key\"][0];"),
    ] {
        executes(
            &format!(r#"
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(2);
    $bytes[0] = 7;
    $bytes[1] = 9;
    {{
        {declaration}
        let $change = function(writable ?ByteRegion $view, ByteRegion $replacement): void {{
            $view = $replacement;
        }};
        $change({access}, $bytes->getRegion(1, 1));
        {observation}
    }}
    $bytes[1] = 8;
    echo $bytes[1];
}}
"#),
            if access == "$copy" { "978" } else { "98" },
        );
    }
}

#[test]
fn collection_region_projections_preserve_copy_and_nullable_values() {
    executes(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 7;
    {
        let $view = $bytes->getRegion(0, 1);
        writable List<ByteRegion> $items = [$view];
        let $first = $items->first;
        if ($first != null) { echo $first[0]; }
        let $last = $items->last;
        if ($last != null) { echo $last[0]; }
        foreach ($items as ByteRegion $item) { echo $item[0]; }
        let $removed = $items->removeAt(0);
        echo $removed[0];
        $items->add($removed);
        let $popped = $items->pop();
        if ($popped != null) { echo $popped[0]; }
        let $empty = $items->first;
        if ($empty == null) { echo "empty;"; }
        writable Dictionary<string, ByteRegion> $map = ["one" => $view];
        echo $map["one"][0];
        let $found = $map->get("one");
        if ($found != null) { echo $found[0]; }
        let $missing = $map->get("missing");
        if ($missing == null) { echo "missing;"; }
        let $taken = $map->remove("one");
        if ($taken != null) { echo $taken[0]; }
        writable List<?ByteRegion> $nullable = [null, $view];
        let $absent = $nullable[0];
        if ($absent == null) { echo "null;"; }
        let $present = $nullable[1];
        if ($present != null) { echo $present[0]; }
        let $optional = $nullable->pop();
        if ($optional != null) { echo $optional[0]; }
    }
    $bytes[0] = 9;
    echo $bytes[0];
}
"#,
        "77777empty;77missing;7null;779",
    );
}

#[test]
fn collection_writable_region_loans_keep_exclusivity_and_retire() {
    executes(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(1);
    {
        writable List<WritableByteRegion> $items = [$bytes->getWritableRegion(0, 1)];
        {
            let writable $view = $items[0];
            $view[0] = 5;
        }
        echo $items[0][0];
        {
            let writable $first = $items->first;
            if ($first != null) { $first[0] = 6; }
        }
        echo $items[0][0];
        let writable $removed = $items->removeAt(0);
        $removed[0] = 7;
        echo $removed[0];
        $items->add($removed);
        let writable $popped = $items->pop();
        if ($popped != null) { $popped[0] = 8; echo $popped[0]; }
    }
    $bytes[0] = 9;
    echo $bytes[0];
}
"#,
        "56789",
    );
}

fn php(source: &str) -> Option<std::process::Output> {
    let text = doriac::compile_source_to_php(PATH, source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    if std::process::Command::new("php")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("PHP unavailable; region compatibility execution skipped");
        return None;
    }
    Some(
        php_execution::Script::new(&format!(
            "{}\n__DoriaFunction_6d61696e();",
            text.strip_prefix("<?php").unwrap()
        ))
        .command(&[])
        .output()
        .unwrap(),
    )
}

fn executes(body: &str, stdout: &str) {
    let source = format!("{IMPORTS}{body}");
    let mir = lower(&source);
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, stdout.as_bytes(), "{source}");
    assert_eq!(output.stderr, b"", "{source}");
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&mir, profile, stdout);
    }
    if let Some(output) = php(&source) {
        assert_eq!(output.status.code(), Some(0), "{source}\n{output:?}");
        assert_eq!(output.stdout, stdout.as_bytes(), "{source}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn region_carriers_are_inline_and_nullable_presence_is_separate() {
    for kind in [ByteRegionKind::Readonly, ByteRegionKind::Writable] {
        for width in [4, 8] {
            let ty = Type::ByteRegion(kind);
            let nullable = Type::NullableByteRegion(kind);
            assert_eq!(
                doriac::native_abi::collection_value_width(ty, width),
                Some(width * 2)
            );
            assert_eq!(
                doriac::native_abi::collection_value_width(nullable, width),
                Some(width * 3)
            );
            assert_eq!(
                doriac::native_closure_abi::type_layout(ty, u32::from(width)).size,
                u32::from(width) * 2
            );
            assert_eq!(
                doriac::native_closure_abi::type_layout(nullable, u32::from(width)).size,
                u32::from(width) * 3
            );
            assert!(!doriac::native_closure_abi::needs_drop(ty));
            assert!(!doriac::native_closure_abi::needs_drop(nullable));
        }
        assert_eq!(
            Type::ByteRegion(kind).has_move_ownership(),
            kind.is_writable()
        );
    }
}

#[test]
fn readonly_region_callback_snapshots_survive_descriptor_rebinding() {
    for nullable in [false, true] {
        let ty = if nullable {
            "?ByteRegion"
        } else {
            "ByteRegion"
        };
        let view = if nullable {
            "if ($view != null) { echo $view[0]; }"
        } else {
            "echo $view[0];"
        };
        let snapshots = if nullable {
            "if ($snapshot != null) { echo $snapshot[0]; } if ($previous != null) { echo $previous[0]; }"
        } else {
            "echo $snapshot[0]; echo $previous[0];"
        };
        executes(
            &format!(
                r#"
function forward<T>(take T $value): T {{ return $value; }}
function through(function({ty}): {ty} $call, {ty} $value): {ty} {{ return $call($value); }}
function main(): void {{
    writable Bytes $first = Bytes::allocate(1);
    writable Bytes $second = Bytes::allocate(2);
    $first[0] = 3;
    $second[1] = 5;
    let $call = fn({ty} $value) => $value;
    {{
        writable {ty} $view = $first->getRegion(0, 1);
        let $previous = forward($view);
        let $snapshot = through($call, $previous);
        $view = $second->getRegion(1, 1);
        {view}
        $second[1] = 9;
        {snapshots}
    }}
    $first[0] = 7;
    echo $first[0];
    echo $second[1];
}}
"#
            ),
            "53379",
        );
    }
}

#[test]
fn readonly_copies_and_writable_reborrows_reuse_the_same_bytes() {
    executes(
        r#"
function window(Bytes $source): ByteRegion { return $source->getRegion(1, 2); }
function narrow(writable WritableByteRegion $source): WritableByteRegion { return $source->getWritableRegion(1, 1); }
function change(writable WritableByteRegion $view): void { $view[0] += 2; $view[0]++; }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(4);
    $bytes[1] = 7;
    {
        let $view = window($bytes);
        let $copy = $view;
        echo "{$copy[0]}:{$view->length}\n";
    }
    {
        let writable $parent = $bytes->getWritableRegion(0, 4);
        {
            let writable $child = narrow($parent);
            change($child);
        }
        echo "{$parent[1]}\n";
    }
    $bytes[1] = 12;
    echo "{$bytes[1]}\n";
}
"#,
        "7:2\n10\n12\n",
    );
}

#[test]
fn region_last_use_releases_storage_and_parent_for_reuse() {
    executes(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    let $read = $bytes->getRegion(0, 1);
    echo "{$read[0]}\n";
    $bytes[0] = 1;
    let writable $write = $bytes->getWritableRegion(0, 2);
    let writable $part = $write->getWritableRegion(0, 1);
    $part[0] = 2;
    $write[1] = 3;
    $bytes[0] = 4;
    ?ByteRegion $optional = $bytes->getRegion(0, 1);
    echo "{$optional?->length ?? 0}\n";
    $bytes[0] = 5;
    echo "{$bytes[0]}:{$bytes[1]}\n";
}
"#,
        "0\n1\n5:3\n",
    );
}

#[test]
fn owned_sibling_loans_execute_with_unchanged_reverse_cleanup() {
    executes(
        r#"
class Item {
    writable int $value = 0;
    writable Bytes $buffer = Bytes::allocate(1);
    function __construct(string $label) {}
    function __destruct() {
        try { echo "drop{$this->label};"; }
        catch (Error) { panic("cleanup output failed"); }
    }
}
class Pair {
    writable Item $left = new Item("left");
    writable Item $right = new Item("right");
    function __destruct() {
        try { echo "pair;"; }
        catch (Error) { panic("cleanup output failed"); }
    }
}
function change(writable Item $left, writable Item $right): void {
    $left->value = 1;
    $right->value = 2;
}
function region(writable Pair $pair): WritableByteRegion {
    return $pair->left->buffer->getWritableRegion(0, 1);
}
function main(): void {
    let writable $pair = new Pair();
    change($pair->left, $pair->right);
    let writable $view = region($pair);
    $pair->right->buffer[0] = 9;
    $view[0] = 7;
    echo "{$pair->left->value}:{$pair->right->value};{$view[0]}:{$pair->right->buffer[0]};";
    $pair->left->buffer[0] = 3;
    echo "{$pair->left->buffer[0]};";
}
"#,
        "1:2;7:9;3;pair;dropright;dropleft;",
    );
}

#[test]
fn multi_operand_display_reads_are_evaluated_in_source_order() {
    for (body, expected) in [
        (
            r#"echo "{$first}{change($middle)}{$last}";"#,
            "old;change;end;old:new:end",
        ),
        (
            r#"echo "" . $first . change($middle) . $last;"#,
            "old;change;end;old:new:end",
        ),
        (
            r#"printf("%s%s%s", $first, change($middle), $last);"#,
            "old;change;end;old:new:end",
        ),
        (
            r#"printf("%d%s%d", $first->getNumber(), change($middle), $last->getNumber());"#,
            "old;change;end;1:new:3",
        ),
    ] {
        executes(
            &format!(
                r#"
class State implements Displayable {{
    function __construct(writable string $label, int $number) {{}}
    function toString(): string {{ echo "{{$this->label}};"; return $this->label; }}
    function getNumber(): int {{ echo "{{$this->label}};"; return $this->number; }}
}}
function change(writable State $state): string {{
    echo "change;";
    $state->label = "new";
    return ":{{$state->label}}:";
}}
function main(): void {{
    let $first = new State("old", 1);
    let writable $middle = new State("middle", 2);
    let $last = new State("end", 3);
    {body}
}}
"#
            ),
            expected,
        );
    }
}

#[test]
fn local_collections_retain_regions_and_callbacks_without_owning_storage() {
    for retained in [
        "List<ByteRegion> $items = [$view];",
        "writable List<ByteRegion> $items = []; $items->add($view);",
    ] {
        executes(
            &format!(
                r#"
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 7;
    {{
        let $view = $bytes->getRegion(0, 1);
        {retained}
        let $read = fn() with ($view) => $view[0];
        List<function(): uint8> $callbacks = [$read];
        echo "{{$items[0][0]}}:{{$callbacks[0]()}};";
    }}
    $bytes[0] = 9;
    echo $bytes[0];
}}
"#
            ),
            "7:7;9",
        );
    }
}

#[test]
fn nested_sibling_arguments_keep_original_homes_and_reverse_cleanup() {
    executes(
        r#"
class Item {
    writable int $value = 0;
    function __construct(string $label) {}
    function __destruct() {
        try { echo "drop{$this->label};"; }
        catch (Error) { panic("cleanup output failed"); }
    }
}
class Branch {
    writable Item $left = new Item("left");
    writable Item $right = new Item("right");
}
class Tree { writable Branch $first = new Branch(); }
function change(writable Item $left, writable Item $right): void {
    $left->value = 4;
    $right->value = 9;
}
function identity(Item $item): Item { return $item; }
function observe(Item $left, writable Item $right): void {
    echo $left->value;
    $right->value++;
}
function main(): void {
    let writable $tree = new Tree();
    change($tree->first->left, $tree->first->right);
    let $left = identity($tree->first->left);
    observe($left, $tree->first->right);
    echo $left->value;
    let $replace = function(writable Item $left, writable Item $right): void {
        $left = new Item("next-left");
        $right = new Item("next-right");
        $left->value = 7;
        $right->value = 8;
    };
    $replace($tree->first->left, $tree->first->right);
    echo "{$tree->first->left->value}:{$tree->first->right->value};";
}
"#,
        "44dropleft;dropright;7:8;dropnext-right;dropnext-left;",
    );
}

#[test]
fn repeated_inline_sibling_helper_loans_execute_across_backends() {
    executes(
        r#"
class Buffer { writable Bytes $bytes = Bytes::allocate(1); }
class Pair {
    writable Buffer $left = new Buffer();
    writable Buffer $right = new Buffer();
}
function same(writable Buffer $buffer): Buffer { return $buffer; }
function change(writable Buffer $first, writable Buffer $second): void {
    $first->bytes[0] = 4;
    $second->bytes[0] = 9;
}
function main(): void {
    let writable $pair = new Pair();
    change(same($pair->left), same($pair->right));
    echo "{$pair->left->bytes[0]}:{$pair->right->bytes[0]};";
    change(same(same($pair->right)), same(same($pair->left)));
    echo "{$pair->left->bytes[0]}:{$pair->right->bytes[0]};";
}
"#,
        "4:9;9:4;",
    );
}

#[test]
fn writable_collection_helper_loans_execute_across_backends() {
    for (ty, initializer) in [
        ("Bytes", "Bytes::allocate(1)"),
        ("List<int>", "[0]"),
        ("int[]", "[0]"),
        ("Dictionary<int, int>", "[0 => 0]"),
    ] {
        executes(
            &format!(
                r#"
class Pair {{
    writable {ty} $left = {initializer};
    writable {ty} $right = {initializer};
}}
function same(writable {ty} $source): {ty} {{ return $source; }}
function change(writable {ty} $first, writable {ty} $second): void {{
    $first[0] = 4;
    $second[0] = 9;
}}
function main(): void {{
    let writable $pair = new Pair();
    change(same($pair->left), same($pair->right));
    echo "{{$pair->left[0]}}:{{$pair->right[0]}};";
    change(same(same($pair->right)), same(same($pair->left)));
    echo "{{$pair->left[0]}}:{{$pair->right[0]}};";
}}
"#
            ),
            "4:9;9:4;",
        );
    }
}

#[test]
fn collection_loans_preserve_storage_through_specialized_methods_and_closures() {
    for (ty, initializer) in [
        ("List<int>", "[0]"),
        ("int[]", "[0]"),
        ("Dictionary<int, int>", "[0 => 0]"),
    ] {
        for expression in [
            "same($holder->value)",
            "$helper->forward($holder->value)",
            "Utility::forward($holder->value)",
            "$arrow($holder->value)",
            "$block($holder->value)",
        ] {
            executes(
                &format!(
                    r#"
class Holder<T> {{
    writable T $value;
    function __construct(parameter take T $value) {{ $this->value = $value; }}
    function lend(): T {{ return $this->value; }}
    T $view {{ borrowed get => $this->value; }}
}}
class Helper<T> {{ function forward(writable T $source): T {{ return $source; }} }}
class Utility {{ static function forward<T>(writable T $source): T {{ return $source; }} }}
function same<T>(writable T $source): T {{ return $source; }}
function change(writable {ty} $source): void {{ $source[0] = 7; }}
function main(): void {{
    let writable $holder = new Holder<{ty}>({initializer});
    let $helper = new Helper<{ty}>();
    let $arrow = fn(writable {ty} $source) => $source;
    let $block = function(writable {ty} $source): {ty} {{ return $source; }};
    let $read = fn({ty} $source) => $source;
    change({expression});
    {expression}[0] = 8;
    {expression}[0] += 3;
    let $view = $read($holder->value);
    let $lent = $holder->lend();
    echo "{{$holder->value[0]}}:{{$holder->view[0]}}:{{$view[0]}}:{{$lent[0]}};";
}}
"#
                ),
                "11:11:11:11;",
            );
        }
    }
}

#[test]
fn nullable_collection_loans_preserve_presence_and_storage() {
    executes(
        r#"
function same(writable ?List<int> $source): ?List<int> { return $source ?? null; }
function change(writable ?List<int> $source): void {
    if ($source != null) { $source[0] = 6; }
}
function main(): void {
    writable List<int> $present = [0];
    writable ?List<int> $absent = null;
    change(same($present));
    change(same($absent));
    echo "{$present[0]}:";
    echo same($absent) == null;
}
"#,
        "6:true",
    );
}

#[test]
fn set_helper_loans_execute_natively_and_preserve_the_php_method_boundary() {
    let body = r#"
class Pair {
    writable Set<int> $left = Set::from([]);
    writable Set<int> $right = Set::from([]);
}
function same(writable Set<int> $source): Set<int> { return $source; }
function change(writable Set<int> $first, writable Set<int> $second): void {
    $first->add(4);
    $second->add(9);
}
function main(): void {
    let writable $pair = new Pair();
    change(same($pair->left), same($pair->right));
    echo "{$pair->left->contains(4)}:{$pair->left->contains(9)};";
    echo "{$pair->right->contains(4)}:{$pair->right->contains(9)};";
}
"#;
    let source = format!("{IMPORTS}{body}");
    let mir = lower(&source);
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, b"true:false;false:true;");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&mir, profile, "true:false;false:true;");
    }
    let errors = doriac::compile_source_to_php(PATH, &source).unwrap_err();
    assert!(
        errors.iter().any(|error| error.code == "B2301"
            && error.message.contains("collection method `add`")),
        "{errors:#?}"
    );
}

#[test]
fn nullable_property_loans_preserve_present_and_absent_collection_results() {
    executes(
        r#"
class Store { List<int> $values = [7]; }
function lend(?Store $source): ?List<int> { return $source?->values; }
function main(): void {
    let $store = new Store();
    let $values = lend($store);
    if ($values != null) { echo $values[0]; }
    echo lend(null) == null;
}
"#,
        "7true",
    );
}

#[test]
fn inherited_generic_owned_fields_borrow_independently_across_backends() {
    executes(
        r#"
open class Base<T> {
    writable T $left;
    function __construct(parameter take T $left) { $this->left = $left; }
}
class Pair extends Base<Bytes> {
    writable Bytes $right = Bytes::allocate(1);
    function __construct() { parent::__construct(Bytes::allocate(1)); }
}
function change(writable Bytes $left, writable Bytes $right): void {
    $left[0] = 4;
    $right[0] = 9;
}
function main(): void {
    let writable $pair = new Pair();
    change($pair->left, $pair->right);
    echo "{$pair->left[0]}:{$pair->right[0]};";
    let writable $view = $pair->left->getWritableRegion(0, 1);
    $pair->right[0] = 11;
    $view[0] = 7;
    echo "{$view[0]}:{$pair->right[0]};";
    $pair->left[0] = 8;
    echo $pair->left[0];
}
"#,
        "4:9;7:11;8",
    );
}

#[test]
fn grouped_readonly_regions_copy_descriptors_and_keep_nullable_presence() {
    executes(
        r#"
function window(Bytes $source): ByteRegion {
    echo "window;";
    return $source->getRegion(0, 2);
}
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    $bytes[0] = 5;
    {
        ByteRegion $left, $right = window($bytes);
        echo "{$left[0]}:{$right[0]}\n";
    }
    $bytes[0] = 7;
    {
        writable ?ByteRegion $left, $right = window($bytes);
        $left = null;
        if ($right != null) { echo "{$right[0]}:{$right->length}\n"; }
    }
    ?ByteRegion $missing, $alsoMissing = null;
    if ($missing == null && $alsoMissing == null) { echo "absent\n"; }
    $bytes[0] = 9;
    echo "{$bytes[0]}\n";
}
"#,
        "window;5:5\nwindow;7:2\nabsent\n9\n",
    );
}

#[test]
fn named_region_bounds_evaluate_in_source_order_exactly_once() {
    executes(
        r#"
function bound(string $name, int $value): int { echo "{$name}\n"; return $value; }
function main(): void {
    let $bytes = Bytes::allocate(4);
    let $view = $bytes->getRegion(length: bound("length", 2), offset: bound("offset", 1));
    let $tail = $view->getRegion(length: bound("tail length", 0), offset: bound("tail offset", 2));
    echo "{$view->length}:{$tail->length}\n";
}
"#,
        "length\noffset\ntail length\ntail offset\n2:0\n",
    );
}

#[test]
fn local_region_branch_joins_preserve_all_possible_source_roots() {
    executes(
        r#"
function observe(bool $first, Bytes $left, Bytes $right): void {
    ByteRegion $view = when ($first): ByteRegion {
        return $left->getRegion(0, 1);
    } else {
        return $right->getRegion(0, 1);
    };
    echo "{$view[0]}\n";
}
function main(): void {
    writable Bytes $left = Bytes::allocate(1);
    writable Bytes $right = Bytes::allocate(1);
    $left[0] = 3;
    $right[0] = 9;
    observe(true, $left, $right);
    observe(false, $left, $right);
    $left[0] = 5;
    $right[0] = 7;
    echo "{$left[0]}:{$right[0]}\n";
}
"#,
        "3\n9\n5:7\n",
    );
}

#[test]
fn named_region_returns_through_branches_preserve_one_source_root() {
    executes(
        r#"
function choose(bool $first, Bytes $source): ByteRegion {
    if ($first) { return $source->getRegion(0, 1); }
    return $source->getRegion(1, 1);
}
function main(): void {
    writable Bytes $source = Bytes::allocate(2);
    $source[0] = 3;
    $source[1] = 9;
    { let $view = choose(true, $source); echo "{$view[0]}\n"; }
    { let $view = choose(false, $source); echo "{$view[0]}\n"; }
    $source[0] = 5;
    $source[1] = 7;
    echo "{$source[0]}:{$source[1]}\n";
}
"#,
        "3\n9\n5:7\n",
    );
}

#[test]
fn null_safe_and_discarded_regions_follow_ordinary_evaluation_rules() {
    executes(
        r#"
function bound(string $name, int $value): int { echo "{$name}\n"; return $value; }
function main(): void {
    let $bytes = Bytes::allocate(2);
    ?ByteRegion $present = $bytes->getRegion(0, 2);
    let $part = $present?->getRegion(length: bound("length", 1), offset: bound("offset", 1));
    if ($part != null) { echo "{$part->length}:{$part[0]}\n"; }
    ?ByteRegion $missing = null;
    let $absent = $missing?->getRegion(bound("unexpected offset", 0), bound("unexpected length", 0));
    if ($absent == null) { echo "absent\n"; }
    let $view = $bytes->getRegion(0, 2);
    $view->getRegion(bound("discard offset", 1), bound("discard length", 0));
}
"#,
        "length\noffset\n1:0\nabsent\ndiscard offset\ndiscard length\n",
    );
}

#[test]
fn empty_nullable_regions_remain_present_across_returns_and_callbacks() {
    executes(
        r#"
function empty(Bytes $source): ?ByteRegion { return $source->getRegion(0, 0); }
function apply(function(ByteRegion): ByteRegion $call, ByteRegion $view): ByteRegion { return $call($view); }
function main(): void {
    let $bytes = Bytes::allocate(0);
    ?ByteRegion $view = empty($bytes);
    if ($view != null) { echo "present:{$view->length}\n"; }
    let $copy = $view;
    let $call = fn(ByteRegion $source) => $source;
    let $result = apply($call, $copy ?? $bytes->getRegion(0, 0));
    echo "{$result->length}\n";
    ?ByteRegion $missing = null;
    let $fallback = $missing ?? $bytes->getRegion(0, 0);
    echo "{$fallback->length}\n";
}
"#,
        "present:0\n0\n0\n",
    );
}

#[test]
fn region_captures_preserve_storage_and_writable_descriptor_transfers() {
    executes(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    $bytes[0] = 21;
    {
        let $view = $bytes->getRegion(0, 2);
        let $read = fn() with (take $view) => $view[0];
        echo "{$read()}\n";
    }
    {
        let writable $view = $bytes->getWritableRegion(0, 2);
        let writable $moved = $view;
        let writable $write = function (): void with (writable $moved) { $moved[0] = 33; };
        $write();
    }
    echo "{$bytes[0]}\n";
}
"#,
        "21\n33\n",
    );
}

#[test]
fn writable_captures_reborrow_objects_and_regions_through_helpers() {
    executes(
        r#"
class Counter {
    writable int $value = 0;
    writable function update(): void { $this->value++; }
}
function advance(writable Counter $counter, writable WritableByteRegion $region): void {
    $counter->update();
    $region[0]++;
}
function main(): void {
    let writable $counter = new Counter();
    writable Bytes $bytes = Bytes::allocate(1);
    $bytes[0] = 40;
    {
        let writable $region = $bytes->getWritableRegion(0, 1);
        let writable $step = function(): void with (writable $counter, writable $region) {
            advance($counter, $region);
            advance($counter, $region->getWritableRegion(0, 1));
            $counter->update();
            echo "{$counter->value}:{$region[0]}\n";
        };
        $step();
        $step();
    }
    echo "{$counter->value}:{$bytes[0]}\n";
}
"#,
        "3:42\n6:44\n6:44\n",
    );
}

#[test]
fn byte_read_modify_write_uses_checked_uint8_operations_for_storage_and_regions() {
    for binding in [
        "writable Bytes $view = Bytes::allocate(1);",
        "writable Bytes $storage = Bytes::allocate(1); let writable $view = $storage->getWritableRegion(0, 1);",
    ] {
        executes(
            &format!(
                r#"
function main(): void {{
    {binding}
    $view[0] = 20;
    $view[0] += 10; echo "{{$view[0]}}\n";
    $view[0] -= 3; echo "{{$view[0]}}\n";
    $view[0] *= 2; echo "{{$view[0]}}\n";
    $view[0] /= 3; echo "{{$view[0]}}\n";
    $view[0] %= 7; echo "{{$view[0]}}\n";
    $view[0] <<= 2; echo "{{$view[0]}}\n";
    $view[0] >>= 1; echo "{{$view[0]}}\n";
    $view[0] |= 3; echo "{{$view[0]}}\n";
    $view[0] &= 7; echo "{{$view[0]}}\n";
    $view[0] ^= 2; echo "{{$view[0]}}\n";
    $view[0]++; echo "{{$view[0]}}\n";
    $view[0]--; echo "{{$view[0]}}\n";
}}
"#
            ),
            "30\n27\n54\n18\n4\n16\n8\n11\n3\n1\n2\n1\n",
        );
    }
}

#[test]
fn invalid_regions_report_checked_bounds_without_cleanup() {
    for (operation, selection, code, values) in [
        (
            "let $view = $bytes->getRegion(bound(-1), bound(1));",
            "$bytes->getRegion(bound(-1), bound(1))",
            "P1304",
            vec![
                ("offset", RuntimeFactValue::Signed(-1)),
                ("requestedLength", RuntimeFactValue::Signed(1)),
                ("length", RuntimeFactValue::Unsigned(2)),
            ],
        ),
        (
            "let $view = $bytes->getRegion(bound(2), bound(1));",
            "$bytes->getRegion(bound(2), bound(1))",
            "P1304",
            vec![
                ("offset", RuntimeFactValue::Signed(2)),
                ("requestedLength", RuntimeFactValue::Signed(1)),
                ("length", RuntimeFactValue::Unsigned(2)),
            ],
        ),
        (
            "let $view = $bytes->getRegion(1, 1); echo $view[bound(1)];",
            "$view[bound(1)]",
            "P1301",
            vec![
                ("index", RuntimeFactValue::Signed(1)),
                ("length", RuntimeFactValue::Unsigned(1)),
            ],
        ),
        (
            "let writable $view = $bytes->getWritableRegion(0, 2); $view[bound(-1)] = 9;",
            "$view[bound(-1)]",
            "P1301",
            vec![
                ("index", RuntimeFactValue::Signed(-1)),
                ("length", RuntimeFactValue::Unsigned(2)),
            ],
        ),
        (
            "let writable $view = $bytes->getWritableRegion(0, 2); $view->copyWithin(bound(1), bound(2), bound(0));",
            "$view->copyWithin(bound(1), bound(2), bound(0))",
            "P1304",
            vec![
                ("offset", RuntimeFactValue::Signed(1)),
                ("requestedLength", RuntimeFactValue::Signed(2)),
                ("length", RuntimeFactValue::Unsigned(2)),
            ],
        ),
        (
            "let writable $view = $bytes->getWritableRegion(0, 2); $view->copyWithin(bound(0), bound(1), bound(2));",
            "$view->copyWithin(bound(0), bound(1), bound(2))",
            "P1304",
            vec![
                ("offset", RuntimeFactValue::Signed(2)),
                ("requestedLength", RuntimeFactValue::Signed(1)),
                ("length", RuntimeFactValue::Unsigned(2)),
            ],
        ),
    ] {
        let source = format!(
            r#"{IMPORTS}
class Marker {{ function __destruct() {{ try {{ echo "unexpected cleanup\n"; }} catch (Doria\Std\Io\IoError) {{}} }} }}
function bound(int $value): int {{ echo "bound\n"; return $value; }}
function main(): void {{
    let $marker = new Marker();
    writable Bytes $bytes = Bytes::allocate(2);
    echo "before\n";
    {operation}
    echo "unexpected continuation\n";
}}
"#
        );
        let mir = lower(&source);
        let output = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(output.exit_status, 101);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("unexpected"));
        let diagnostic = output.runtime_diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.code, code);
        let start = source.rfind(selection).unwrap();
        assert_eq!(
            (diagnostic.span.start, diagnostic.span.end),
            (start, start + selection.len())
        );
        let facts = &diagnostic.runtime_outcome.as_ref().unwrap().facts;
        assert_eq!(facts.len(), values.len());
        for (fact, (name, value)) in facts.iter().zip(values) {
            assert_eq!(fact.name, name);
            assert_eq!(fact.value, value);
        }
        for &profile in NativeProfile::AVAILABLE {
            let native = native_execution::execute_native(&mir, profile);
            assert_eq!(native.status.code(), Some(101));
            assert_eq!(native.stdout, output.stdout);
            assert_eq!(native.stderr, output.stderr);
        }
        if let Some(php) = php(&source) {
            assert_eq!(php.status.code(), Some(101), "{php:?}");
            assert_eq!(php.stdout, output.stdout);
            assert!(String::from_utf8_lossy(&php.stderr).contains(&format!("Panic[{code}]")));
        }
    }
}

#[test]
fn string_byte_regions_borrow_utf8_bytes_without_copying() {
    // Decision log entry 49: `byteRegion` is the string's own UTF-8 bytes as
    // a readonly region; ranges use the region's `getRegion`.
    let source = format!(
        "{IMPORTS}{}",
        r#"
function size(ByteRegion $region): int { return $region->length; }
function main(): void {
    let $text = "héllo";
    let $all = $text->byteRegion;
    echo $all->length . ":" . $all[0] . "," . $all[1] . "," . $all[2] . ":";
    echo size($text->byteRegion->getRegion(3, 3)) . ":";
    writable Bytes $copy = Bytes::allocate(6);
    echo $copy->getWritableRegion(0, 6)->copyFrom($text->byteRegion) . ":";
    echo (String::fromBytes($copy) ?? "invalid") . ":";
    let $empty = "";
    echo $empty->byteRegion->length . "\n";
}
"#
    );
    let expected = "6:104,195,169:3:6:h\u{e9}llo:0\n";
    let mir = lower(&source);
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, expected.as_bytes(), "{source}");
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&mir, profile, expected);
    }
    // PHP has no string intrinsic properties yet, exactly as for `bytes`.
    let errors = doriac::compile_source_to_php(PATH, &source)
        .expect_err("PHP output rejects string intrinsic properties");
    assert!(
        errors.iter().any(|error| error.code == "B2501"),
        "{errors:#?}"
    );
}

#[test]
fn constrained_calls_bound_to_an_interface_pass_regions_on_every_backend() {
    // PHP looked up the method of `$stream->put($source)` through the
    // authored target, which is a constrained call on `T`, not an interface
    // call, and panicked once `T` was bound to the interface itself.
    executes(
        r#"
interface Sink { writable function put(ByteRegion $source): int; }
interface Later extends Sink { writable function later(): int; }
class Box implements Later {
    writable function put(ByteRegion $source): int { return $source->length; }
    writable function later(): int { return 0; }
}
function send<T implements Sink>(writable T $stream, ByteRegion $source): int { return $stream->put($source); }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(3);
    writable Sink $sink = new Box();
    echo send($sink, $bytes->getRegion(0, 1));
    writable Later $later = new Box();
    echo send($later, $bytes->getRegion(0, 3));
    writable Box $box = new Box();
    echo send($box, $bytes->getRegion(0, 2));
}
"#,
        "132",
    );
}

#[test]
fn region_copies_move_the_shorter_length_on_every_backend() {
    executes(
        r#"
function main(): void {
    writable Bytes $source = Bytes::fromArray([1, 2, 3, 4]);
    writable Bytes $target = Bytes::allocate(6);
    {
        let writable $whole = $target->getWritableRegion(0, 6);
        echo $whole->copyFrom($source->getRegion(0, 4));
    }
    {
        let writable $short = $target->getWritableRegion(4, 2);
        let $copied = $short->copyFrom($source->getRegion(1, 3));
        echo $copied;
    }
    {
        let writable $tail = $target->getWritableRegion(5, 1);
        $tail->copyFrom($source->getRegion(0, 0));
    }
    echo $target->getWritableRegion(1, 1)->copyFrom($source->getRegion(3, 1));
    echo ":";
    writable int $index = 0;
    while ($index < $target->length) {
        echo $target[$index];
        $index += 1;
    }
    echo "\n";
}
"#,
        "421:143423\n",
    );
}

#[test]
fn region_copies_within_one_region_handle_overlap_on_every_backend() {
    executes(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::fromArray([1, 2, 3, 4, 5, 6]);
    {
        let writable $all = $bytes->getWritableRegion(0, 6);
        $all->copyWithin(2, 4, 0);
    }
    $bytes->getWritableRegion(0, 6)->copyWithin(offset: 0, length: 3, destination: 2);
    $bytes->getWritableRegion(1, 2)->copyWithin(0, 0, 2);
    writable int $index = 0;
    while ($index < $bytes->length) {
        echo $bytes[$index];
        $index += 1;
    }
    echo "\n";
}
"#,
        "343456\n",
    );
}
