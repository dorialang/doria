use doriac::types::{ByteRegionKind, ResolvedType};

const IMPORTS: &str = r#"
use Doria\Std\Io\ByteRegion;
use Doria\Std\Io\WritableByteRegion;
"#;

fn check(source: &str) -> doriac::semantics::SemanticInfo {
    let source = format!("{IMPORTS}\n{source}");
    let (_, analysis) = doriac::analyze_source_for_ide("byte-region.doria", &source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    assert!(
        analysis.diagnostics.is_empty(),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
    analysis.info
}

fn rejects(source: &str, code: &str) {
    let source = format!("{IMPORTS}\n{source}");
    let (_, analysis) = doriac::analyze_source_for_ide("byte-region.doria", &source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    assert!(
        analysis.diagnostics.iter().any(|error| error.code == code),
        "expected {code}: {source}\n{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn writable_descriptor_arguments_do_not_grant_byte_or_readonly_place_access() {
    for body in [
        "ByteRegion $view = $bytes->getRegion(0, 1); $replace($view);",
        "List<ByteRegion> $items = [$bytes->getRegion(0, 1)]; $replace($items[0]);",
        "writable List<ByteRegion> $items = [$bytes->getRegion(0, 1)]; $replace($items->first);",
        "writable Dictionary<string, ByteRegion> $items = [\"view\" => $bytes->getRegion(0, 1)]; $replace($items->get(\"view\"));",
        "let writable $view = $bytes->getRegion(0, 1); let $write = function(writable uint8 $byte): void { $byte = 9; }; $write($view[0]);",
    ] {
        rejects(
            &format!(
                r#"
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    let $replace = function(writable ?ByteRegion $view): void {{ $view = null; }};
    {body}
}}
"#,
            ),
            "E0651",
        );
    }
}

#[test]
fn writable_region_collection_borrows_require_writable_sources_and_exclusivity() {
    for (declaration, access) in [
        ("List<WritableByteRegion> $items = [$bytes->getWritableRegion(0, 1)];", "$items[0]"),
        ("List<WritableByteRegion> $items = [$bytes->getWritableRegion(0, 1)];", "$items->first"),
        ("Dictionary<string, WritableByteRegion> $items = [\"view\" => $bytes->getWritableRegion(0, 1)];", "$items[\"view\"]"),
        ("Dictionary<string, WritableByteRegion> $items = [\"view\" => $bytes->getWritableRegion(0, 1)];", "$items->get(\"view\")"),
    ] {
        let mutation = if access.contains('[') {
            "$view[0] = 7;"
        } else {
            "if ($view != null) { $view[0] = 7; }"
        };
        check(&format!(
            r#"
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    {{
        writable {declaration}
        let writable $view = {access};
        {mutation}
    }}
}}
"#
        ));
        rejects(
            &format!(
                r#"
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    {declaration}
    let writable $view = {access};
}}
"#
            ),
            "E0478",
        );
        rejects(
            &format!(
                r#"
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    writable {declaration}
    let writable $first = {access};
    let writable $second = {access};
    echo $first->length;
    echo $second->length;
}}
"#
            ),
            "E0477",
        );
    }
}

#[test]
fn regions_have_canonical_types_and_named_bounds() {
    let info = check(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(4);
    {
        ByteRegion $view = $bytes->getRegion(length: 2, offset: 1);
        ByteRegion $copy = $view;
        echo "{$view->length} {$copy[0]}";
    }
    {
        writable WritableByteRegion $view = $bytes->getWritableRegion(0, 4);
        $view[0] = 7;
        $view[0]++;
        echo $view->length;
    }
    $bytes[0] = 8;
}
"#,
    );
    for kind in [ByteRegionKind::Readonly, ByteRegionKind::Writable] {
        assert!(info
            .expression_types
            .values()
            .any(|ty| *ty == ResolvedType::ByteRegion(kind)));
    }
}

#[test]
fn null_safe_region_members_keep_nullable_results_and_access_modes() {
    check(
        r#"
function part(?ByteRegion $view): ?ByteRegion { return $view?->getRegion(0, 0); }
function bytes(?Bytes $source): ?ByteRegion { return $source?->getRegion(0, 0); }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        ?ByteRegion $view = $bytes->getRegion(0, 2);
        ?ByteRegion $part = $view?->getRegion(length: 0, offset: 2);
        ?int $length = $part?->length;
        echo $length ?? -1;
        ?ByteRegion $absent = null;
        let $none = $absent?->getRegion(0, 0);
    }
    {
        writable ?WritableByteRegion $view = $bytes->getWritableRegion(0, 2);
        let writable $part = $view?->getWritableRegion(0, 1);
        if ($part != null) { $part[0] = 7; }
    }
    $bytes[0] = 8;
}
"#,
    );
    rejects(
        "function bad(?ByteRegion $view): int { return $view->length; }",
        "E0506",
    );
    for ty in ["Bytes", "ByteRegion", "WritableByteRegion"] {
        rejects(
            &format!("function bad(?{ty} $view): void {{ $view->getRegion(0, 0); }}"),
            "E0506",
        );
    }
    rejects(
        "function bad(?WritableByteRegion $view): void { let $part = $view?->getWritableRegion(0, 0); }",
        "E0201",
    );
}

#[test]
fn callable_region_returns_preserve_argument_storage_provenance() {
    check(
        r#"
function apply(function(ByteRegion): ByteRegion $forward, ByteRegion $view): ByteRegion {
    return $forward($view);
}
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        function(ByteRegion): ByteRegion $forward = fn(ByteRegion $view) => $view;
        let $view = apply($forward, $bytes->getRegion(0, 1));
        echo $view[0];
        function(Bytes): ByteRegion $window = fn(Bytes $source) => $source->getRegion(0, 1);
        echo $window($bytes)[0];
        function(take ByteRegion): ByteRegion $copy = fn(take ByteRegion $view) => $view;
        echo $copy($view)[0];
    }
    $bytes[0] = 7;
}
"#,
    );
    for declaration in [
        "let $forward = fn(ByteRegion $source) => $source; let $view = $forward($bytes->getRegion(0, 1));",
        "function(ByteRegion): ByteRegion $forward = function (ByteRegion $source): ByteRegion { return $source->getRegion(0, 1); }; let $view = $forward($bytes->getRegion(0, 2));",
        "let $forward = fn(take ByteRegion $source) => $source; let $view = $forward($bytes->getRegion(0, 1));",
        "let $forward = fn(Bytes $source) => $source->getRegion(0, 1); let $view = $forward($bytes);",
    ] {
        rejects(
            &format!(
                "function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); {declaration} $bytes[0] = 7; echo $view[0]; }}"
            ),
            "E0763",
        );
    }
    rejects(
        r#"
function bad(): ByteRegion {
    let $forward = fn(ByteRegion $source) => $source;
    let $bytes = Bytes::allocate(2);
    return $forward($bytes->getRegion(0, 1));
}
"#,
        "E0762",
    );
}

#[test]
fn writable_region_returns_through_callables_keep_exclusive_access() {
    check(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        let $forward = fn(take WritableByteRegion $source) => $source;
        let writable $view = $forward($bytes->getWritableRegion(0, 2));
        $view[0] = 9;
    }
    $bytes[0] = 7;
}
"#,
    );
    rejects(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    let $forward = fn(take WritableByteRegion $source) => $source;
    let writable $view = $forward($bytes->getWritableRegion(0, 2));
    echo $bytes[0];
    $view[0] = 9;
}
"#,
        "E0763",
    );
}

#[test]
fn region_returns_obey_existing_closure_environment_and_single_root_boundaries() {
    for body in [
        "let $view = $bytes->getRegion(0, 1); let $forward = fn() with (take $view) => $view;",
        "let $view = $bytes->getRegion(0, 1); let $forward = fn() with ($view) => $view->getRegion(0, 1);",
        "let $forward = fn() with ($bytes) => $bytes->getRegion(0, 1);",
        "let $forward = fn() with (take $bytes) => $bytes->getRegion(0, 1);",
        "let $forward = function (): ByteRegion with ($bytes) { let $local = $bytes->getRegion(0, 1); return $local; };",
    ] {
        rejects(
            &format!("function main(): void {{ let $bytes = Bytes::allocate(2); {body} }}"),
            "E0653",
        );
    }
    rejects(
        r#"
function main(): void {
    let $forward = function (ByteRegion $left, ByteRegion $right, bool $first): ByteRegion {
        if ($first) { return $left; }
        return $right;
    };
}
"#,
        "E0659",
    );
    rejects(
        r#"
function outer(ByteRegion $view): void {
    let $forward = fn(ByteRegion $other) with (take $view) => $view;
}
"#,
        "E0653",
    );
}

#[test]
fn named_region_returns_require_one_source_across_all_paths() {
    for body in [
        "if ($first) { return $left->getRegion(0, 1); } return $right->getRegion(0, 1);",
        "ByteRegion $view = when ($first) { return $left->getRegion(0, 1); } else { return $right->getRegion(0, 1); }; return $view;",
        "let writable $view = $left->getRegion(0, 1); if ($first) { $view = $right->getRegion(0, 1); } return $view;",
    ] {
        rejects(
            &format!("function choose(bool $first, Bytes $left, Bytes $right): ByteRegion {{ {body} }}"),
            "E0478",
        );
    }
    rejects(
        r#"
function choose(bool $first, writable Bytes $left, writable Bytes $right): WritableByteRegion {
    if ($first) { return $left->getWritableRegion(0, 1); }
    return $right->getWritableRegion(0, 1);
}
"#,
        "E0478",
    );
    rejects(
        r#"
function choose(bool $first, ?ByteRegion $left, ?ByteRegion $right): ?ByteRegion {
    if ($first) { return $left?->getRegion(0, 1); }
    return $right?->getRegion(0, 1);
}
"#,
        "E0478",
    );
    check(
        r#"
function choose(bool $first, Bytes $source): ByteRegion {
    if ($first) { return $source->getRegion(0, 1); }
    return $source->getRegion(1, 1);
}
function optional(bool $present, Bytes $source): ?ByteRegion {
    if ($present) { return $source->getRegion(0, 1); }
    return null;
}
function localJoin(bool $first, Bytes $left, Bytes $right): uint8 {
    ByteRegion $view = when ($first) { return $left->getRegion(0, 1); } else { return $right->getRegion(0, 1); };
    return $view[0];
}
"#,
    );
}

#[test]
fn consuming_receiver_cannot_lend_storage_that_dies_with_it() {
    for (result, body, code) in [
        (
            "ByteRegion",
            "return $this->bytes->getRegion(0, 1);",
            "E0762",
        ),
        (
            "?ByteRegion",
            "return $this->optional?->getRegion(0, 1);",
            "E0762",
        ),
        (
            "function(): int",
            "let $view = $this->bytes->getRegion(0, 1); return fn() with (take $view) => $view->length;",
            "E0658",
        ),
    ] {
        rejects(
            &format!(
                "class Packet {{ Bytes $bytes = Bytes::allocate(2); ?Bytes $optional = Bytes::allocate(2); consuming function view(): {result} {{ {body} }} }}"
            ),
            code,
        );
    }
    check(
        r#"
class Packet {
    Bytes $bytes = Bytes::allocate(2);
    function view(): ByteRegion { return $this->bytes->getRegion(0, 1); }
    consuming function finish(Bytes $source): ByteRegion { return $source->getRegion(0, 1); }
}
function main(): void {
    let $bytes = Bytes::allocate(2);
    let $packet = new Packet();
    let $view = $packet->finish($bytes);
    echo $view[0];
}
"#,
    );
}

#[test]
fn regions_preserve_borrows_through_helpers_and_subregions() {
    check(
        r#"
function window(Bytes $source): ByteRegion { return $source->getRegion(1, 2); }
function smaller(ByteRegion $source): ByteRegion { return $source->getRegion(0, 1); }
function writableWindow(writable Bytes $source): WritableByteRegion {
    return $source->getWritableRegion(0, 4);
}
function main(): void {
    writable Bytes $bytes = Bytes::allocate(4);
    {
        ByteRegion $view = window($bytes);
        ByteRegion $small = smaller($view);
        echo $small[0];
    }
    {
        writable WritableByteRegion $view = writableWindow($bytes);
        {
            writable WritableByteRegion $small = $view->getWritableRegion(1, 2);
            $small[0] = 9;
        }
        $view[0] = 3;
    }
    $bytes[0] = 7;
}
"#,
    );
}

#[test]
fn region_loans_end_at_last_use_without_discarding_live_aliases() {
    check(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    let $read = $bytes->getRegion(0, 1);
    echo $read[0];
    $bytes[0] = 1;
    let writable $write = $bytes->getWritableRegion(0, 2);
    let writable $part = $write->getWritableRegion(0, 1);
    $part[0] = 2;
    $write[1] = 3;
    $bytes[0] = 4;
    ?ByteRegion $optional = $bytes->getRegion(0, 1);
    echo $optional?->length ?? 0;
    $bytes[0] = 5;
}
"#,
    );
    for (body, code) in [
        ("let $view = $bytes->getRegion(0, 1); let $alias = $view; echo $view[0]; $bytes[0] = 7; echo $alias[0];", "E0763"),
        ("let $view = $bytes->getRegion(0, 1); let $read = fn() with (take $view) => $view[0]; $bytes[0] = 7; echo $read();", "E0654"),
    ] {
        rejects(
            &format!("function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); {body} }}"),
            code,
        );
    }
}

#[test]
fn region_loans_exclude_conflicting_source_and_parent_access() {
    for body in [
        "let $view = $bytes->getRegion(0, 2); $bytes[0] = 1; echo $view[0];",
        "let $view = $bytes->getRegion(0, 2); sink($bytes); echo $view[0];",
        "let writable $view = $bytes->getWritableRegion(0, 2); echo $bytes[0]; $view[0] = 1;",
        "let writable $view = $bytes->getWritableRegion(0, 4); let $small = $view->getRegion(0, 2); $view[0] = 1; echo $small[0];",
        "let writable $view = $bytes->getWritableRegion(0, 4); let writable $small = $view->getWritableRegion(0, 2); echo $view[0]; $small[0] = 1;",
        "let $view = helper($bytes); $bytes[0] = 1; echo $view[0];",
    ] {
        rejects(
            &format!(
                r#"
function sink(take Bytes $value): void {{}}
function helper(Bytes $value): ByteRegion {{ return $value->getRegion(0, 1); }}
function main(): void {{ writable Bytes $bytes = Bytes::allocate(4); {body} }}
"#
            ),
            "E0763",
        );
    }
}

#[test]
fn generic_owned_region_returns_cannot_escape_their_original_storage() {
    for call in [
        "own($bytes->getRegion(0, 1))",
        "$concrete->own($bytes->getRegion(0, 1))",
        "$erased->own($bytes->getRegion(0, 1))",
    ] {
        rejects(
            &format!(
                r#"
interface Relay {{ function own<T>(take T $view): T; }}
class Echo implements Relay {{ function own<T>(take T $view): T {{ return $view; }} }}
function own<T>(take T $view): T {{ return $view; }}
function bad(): ByteRegion {{
    let $bytes = Bytes::allocate(1);
    let $concrete = new Echo();
    Relay $erased = new Echo();
    return {call};
}}
function main(): void {{}}
"#,
            ),
            "E0762",
        );
    }
}

#[test]
fn generic_owned_writable_regions_keep_exclusive_original_storage_loans() {
    for call in [
        "own($bytes->getWritableRegion(0, 1))",
        "$concrete->own($bytes->getWritableRegion(0, 1))",
        "$erased->own($bytes->getWritableRegion(0, 1))",
    ] {
        rejects(
            &format!(
                r#"
interface Relay {{ function own<T>(take T $view): T; }}
class Echo implements Relay {{ function own<T>(take T $view): T {{ return $view; }} }}
function own<T>(take T $view): T {{ return $view; }}
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    let $concrete = new Echo();
    Relay $erased = new Echo();
    let writable $view = {call};
    echo $bytes[0];
    $view[0] = 71;
}}
"#,
            ),
            "E0763",
        );
    }
}

#[test]
fn region_cannot_escape_its_storage() {
    for source in [
        "function bad(): ByteRegion { let $bytes = Bytes::allocate(2); return $bytes->getRegion(0, 1); }",
        "function bad(take Bytes $bytes): ByteRegion { return $bytes->getRegion(0, 1); }",
        "function main(): void { let $view = Bytes::allocate(2)->getRegion(0, 1); echo $view[0]; }",
        "function main(): void { writable ?ByteRegion $view = null; { let $bytes = Bytes::allocate(2); $view = $bytes->getRegion(0, 1); } if ($view != null) { echo $view[0]; } }",
    ] {
        rejects(source, "E0762");
    }
}

#[test]
fn copying_and_replacing_regions_tracks_each_retained_source() {
    check(
        r#"
function main(): void {
    writable Bytes $first = Bytes::allocate(2);
    writable Bytes $second = Bytes::allocate(2);
    writable ?ByteRegion $view = $first->getRegion(0, 1);
    $view = $view;
    $view = $second->getRegion(0, 1);
    $first[0] = 7;
    $view = null;
    $second[0] = 9;
    {
        ByteRegion $left, $right = $first->getRegion(0, 1);
        echo "{$left[0]} {$right[0]}";
    }
    $first[0] = 8;
}
"#,
    );
    for body in [
        "writable ?ByteRegion $left, $right = $bytes->getRegion(0, 1); $left = null; $bytes[0] = 1; if ($right != null) { echo $right[0]; }",
        "writable ?ByteRegion $left = null; $left = $bytes->getRegion(0, 1); let $right = $left; $left = null; $bytes[0] = 1; if ($right != null) { echo $right[0]; }",
    ] {
        rejects(
            &format!(
                "function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); {body} }}"
            ),
            "E0763",
        );
    }
}

#[test]
fn subregions_depend_on_storage_not_the_local_descriptor_lifetime() {
    check(
        r#"
function small(Bytes $source): ByteRegion {
    let $view = $source->getRegion(0, 2);
    return $view->getRegion(0, 1);
}
function main(): void {
    let $bytes = Bytes::allocate(2);
    writable ?ByteRegion $view = null;
    {
        let $parent = small($bytes);
        $view = $parent->getRegion(0, 1);
    }
    if ($view != null) { echo $view[0]; }
}
"#,
    );
}

#[test]
fn returned_callbacks_retain_region_storage_across_factory_boundaries() {
    for factory in [
        "function make(Bytes $bytes): function(): uint8 { let $view = $bytes->getRegion(0, 1); return fn() with (take $view) => $view[0]; }",
        "function make(ByteRegion $view): function(): uint8 { return fn() with (take $view) => $view[0]; }",
        "function make(take ByteRegion $view): function(): uint8 { return fn() with (take $view) => $view[0]; }",
        "function make(take WritableByteRegion $view): function(): uint8 { return fn() with (take $view) => $view[0]; }",
    ] {
        let source = if factory.contains("Bytes $bytes") {
            "$bytes"
        } else if factory.contains("WritableByteRegion") {
            "$bytes->getWritableRegion(0, 1)"
        } else {
            "$bytes->getRegion(0, 1)"
        };
        check(&format!(
            "{factory} function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); {{ let $read = make({source}); echo $read(); }} $bytes[0] = 7; }}"
        ));
        rejects(
            &format!(
                "{factory} function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); let $read = make({source}); $bytes[0] = 7; echo $read(); }}"
            ),
            "E0654",
        );
        rejects(
            &format!(
                "{factory} function escape(): function(): uint8 {{ writable Bytes $bytes = Bytes::allocate(2); return make({source}); }}"
            ),
            "E0658",
        );
    }
    let factory = "let $make = fn(take ByteRegion $view) => fn() with (take $view) => $view[0];";
    check(&format!(
        "function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); {factory} {{ let $read = $make($bytes->getRegion(0, 1)); echo $read(); }} $bytes[0] = 7; }}"
    ));
    rejects(
        &format!(
            "function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); {factory} let $read = $make($bytes->getRegion(0, 1)); $bytes[0] = 7; echo $read(); }}"
        ),
        "E0654",
    );
}

#[test]
fn taking_a_region_is_not_suggested_as_a_storage_lifetime_fix() {
    let source = format!(
        r#"{IMPORTS}
function main(): void {{
    writable List<function(): uint8> $callbacks = [];
    {{
        let $bytes = Bytes::allocate(1);
        let $view = $bytes->getRegion(0, 1);
        let $read = fn() with ($view) => $view[0];
        $callbacks->add($read);
    }}
    echo $callbacks[0]();
}}
"#
    );
    let (_, analysis) = doriac::analyze_source_for_ide("byte-region.doria", &source).unwrap();
    let error = analysis
        .diagnostics
        .iter()
        .find(|error| error.code == "E0658")
        .unwrap_or_else(|| panic!("{:#?}", analysis.diagnostics));
    assert!(error.fixes.is_empty(), "{error:#?}");
}

#[test]
fn taking_a_region_into_a_closure_preserves_the_storage_borrow() {
    check(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        let $view = $bytes->getRegion(0, 1);
        let $read = fn() with (take $view) => $view[0];
        echo $read();
    }
    $bytes[0] = 7;
}
"#,
    );
    rejects(
        r#"
function bad(): function(): uint8 {
    let $bytes = Bytes::allocate(2);
    let $view = $bytes->getRegion(0, 1);
    return fn() with (take $view) => $view[0];
}
"#,
        "E0658",
    );
    rejects(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    writable ?ByteRegion $view = $bytes->getRegion(0, 1);
    let $read = fn() with (take $view) => $view?->length ?? 0;
    $view = null;
    $bytes[0] = 7;
    echo $read();
}
"#,
        "E0654",
    );
}

#[test]
fn writable_captured_regions_reborrow_their_original_storage() {
    check(
        r#"
function update(writable WritableByteRegion $region): void { $region[0] = 7; }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(4);
    let writable $region = $bytes->getWritableRegion(0, 4);
    let writable $callback = function(): void with (writable $region) {
        update($region);
        update($region->getWritableRegion(0, 1));
    };
    $callback();
}
"#,
    );
}

#[test]
fn temporary_region_arguments_exclude_overlapping_access_until_call_completes() {
    for body in [
        "consume($bytes->getWritableRegion(0, 1), $bytes[0]);",
        "consume(helper($bytes), $bytes[0]);",
        "let writable $view = $bytes->getWritableRegion(0, 2); consume($view->getWritableRegion(0, 1), $view[0]);",
    ] {
        rejects(
            &format!(
                r#"
function consume(take WritableByteRegion $region, uint8 $other): void {{}}
function helper(writable Bytes $bytes): WritableByteRegion {{ return $bytes->getWritableRegion(0, 1); }}
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(2);
    {body}
}}
"#
            ),
            "E0477",
        );
    }
    rejects(
        r#"
function consume(take WritableByteRegion $region, uint8 $other): void {}
function alias(writable WritableByteRegion $view): void {
    consume($view->getWritableRegion(0, 1), $view[0]);
}
"#,
        "E0477",
    );
}

#[test]
fn readonly_region_cannot_gain_writable_access_and_writable_region_cannot_copy() {
    rejects(
        r#"
function main(): void {
    let $bytes = Bytes::allocate(2);
    writable ByteRegion $view = $bytes->getRegion(0, 1);
    $view[0] = 7;
}
"#,
        "E0201",
    );
    rejects(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    let writable $view = $bytes->getWritableRegion(0, 2);
    let writable $moved = $view;
    $moved[0] = 1;
    echo $view[0];
}
"#,
        "E0470",
    );
}

#[test]
fn interface_owned_copy_returns_do_not_erase_retained_source_contracts() {
    for methods in [
        "function view(ByteRegion $source): ByteRegion",
        "function view<T>(ByteRegion $source, T $extra): ByteRegion",
    ] {
        rejects(
            &format!(
                r#"
interface View {{ {methods}; }}
class Lending implements View {{ {methods} {{ return $source; }} }}
function main(): void {{}}
"#,
            ),
            "E0755",
        );
    }
}

#[test]
fn retained_region_cannot_be_hidden_in_unbounded_owned_storage() {
    for (declaration, body) in [
        (
            "function escape(): Holder { let $bytes = Bytes::allocate(2); return new Holder($bytes->getRegion(0, 1)); }",
            "let $holder = escape();",
        ),
        // A payload carries its region's loan, so it cannot outlive the bytes.
        (
            "function escape(): Packet { let $bytes = Bytes::allocate(2); return Packet::View($bytes->getRegion(0, 1)); }",
            "let $packet = escape();",
        ),
        ("", "store($view);"),
    ] {
        rejects(
            &format!(
                r#"
class Holder {{ function __construct(ByteRegion $view) {{}} }}
enum Packet {{ case View(ByteRegion $view); }}
class State {{ static writable ?ByteRegion $saved = null; }}
function store(ByteRegion $view): void {{ State::saved = $view; }}
{declaration}
function main(): void {{
    let $bytes = Bytes::allocate(2);
    let $view = $bytes->getRegion(0, 1);
    {body}
}}
"#
            ),
            "E0762",
        );
    }
    for retained in [
        "List<ByteRegion> $items = [$view];",
        "writable List<ByteRegion> $items = []; $items->add($view);",
    ] {
        rejects(
            &format!(
                "function escape(): List<ByteRegion> {{ let $bytes = Bytes::allocate(2); let $view = $bytes->getRegion(0, 1); {retained} return $items; }}"
            ),
            "E0762",
        );
    }
    check(
        r#"
class Holder { function __construct(ByteRegion $view) {} }
function main(): void {
    let $bytes = Bytes::allocate(1);
    let $holder = new Holder($bytes->getRegion(0, 1));
    echo $holder->view[0];
}
"#,
    );
}

#[test]
fn local_region_collections_keep_their_proved_source_loans() {
    for retained in [
        "List<ByteRegion> $items = [$view];",
        "writable List<ByteRegion> $items = []; $items->add($view);",
    ] {
        check(&format!(
            "function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); {{ let $view = $bytes->getRegion(0, 1); {retained} echo $items[0][0]; }} $bytes[0] = 7; }}"
        ));
        rejects(
            &format!(
                "function main(): void {{ writable Bytes $bytes = Bytes::allocate(2); let $view = $bytes->getRegion(0, 1); {retained} $bytes[0] = 7; echo $items[0][0]; }}"
            ),
            "E0763",
        );
    }
}

#[test]
fn region_call_arguments_and_return_summaries_preserve_access() {
    check(
        r#"
function read(ByteRegion $view): uint8 { return $view[0]; }
function mutate(writable WritableByteRegion $view): void { $view[0] = 9; }
function forward(take ByteRegion $view): ByteRegion { return $view; }
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    {
        let $call = fn(ByteRegion $view) => $view[0];
        echo $call($bytes->getRegion(0, 1));
        echo read(forward($bytes->getRegion(0, 1)));
    }
    {
        let writable $view = $bytes->getWritableRegion(0, 2);
        mutate($view);
        echo read($view->getRegion(0, 1));
    }
    $bytes[0] = 1;
}
"#,
    );
    rejects(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    let $call = fn(take WritableByteRegion $view, uint8 $other) => $view[0];
    echo $call($bytes->getWritableRegion(0, 1), $bytes[0]);
}
"#,
        "E0477",
    );
}

#[test]
fn region_copies_need_a_writable_destination_and_disjoint_storage() {
    check(
        r#"
function main(): void {
    writable Bytes $left = Bytes::allocate(2);
    Bytes $right = Bytes::allocate(2);
    let writable $target = $left->getWritableRegion(0, 2);
    int $copied = $target->copyFrom($right->getRegion(0, 2));
    echo $copied;
}
"#,
    );
    rejects(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(4);
    let writable $target = $bytes->getWritableRegion(0, 2);
    echo $target->copyFrom($bytes->getRegion(2, 2));
}
"#,
        "E0477",
    );
    rejects(
        r#"
function main(): void {
    writable Bytes $bytes = Bytes::allocate(2);
    Bytes $source = Bytes::allocate(2);
    let $target = $bytes->getWritableRegion(0, 2);
    echo $target->copyFrom($source->getRegion(0, 2));
}
"#,
        "E0201",
    );
}

#[test]
fn string_byte_regions_keep_the_string_borrowed_and_stay_readonly() {
    // Decision log entry 49.
    check(
        r#"
function main(): void {
    let $text = "abc";
    let $view = $text->byteRegion;
    echo $view->getRegion(1, 2)->length;
}
"#,
    );
    rejects(
        r#"
function main(): void {
    writable string $text = "abc";
    let $view = $text->byteRegion;
    $text = "xyz";
    echo $view->length;
}
"#,
        "E0763",
    );
    rejects(
        r#"
function leak(): ByteRegion { let $text = "abc"; return $text->byteRegion; }
function main(): void { echo leak()->length; }
"#,
        "E0762",
    );
    rejects(
        r#"
function main(): void {
    let $text = "abc";
    let writable $view = $text->byteRegion->getWritableRegion(0, 1);
}
"#,
        "E0521",
    );
}
