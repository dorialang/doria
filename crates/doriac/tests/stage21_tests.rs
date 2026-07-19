fn assert_diagnostic(source: &str, code: &str) {
    let diagnostics =
        doriac::check_source("stage21.doria", source).expect_err("source should be rejected");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}, got {diagnostics:#?}"));
    assert!(!diagnostic.message.contains("lifetime"));
    assert!(!diagnostic.message.contains("borrow checker"));
}

fn assert_valid_mir(source: &str) {
    let program = doriac::lower_source_to_mir("stage21-native.doria", source)
        .expect("source should lower to MIR");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    let object = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("validated MIR should lower through the fast native backend");
    assert!(!object.is_empty());
}

#[test]
fn readonly_borrows_of_one_owner_can_overlap_in_a_call() {
    doriac::check_source(
        "stage21-readonly-overlap.doria",
        r#"
class Guard {}

function inspect(Guard $left, Guard $right): void {}

function route(take Guard $guard): void
{
    inspect($guard, $guard);
    inspect($guard, $guard);
}
"#,
    )
    .expect("many readonly uses of one owner may overlap");
}

#[test]
fn writable_and_readonly_uses_of_one_owner_conflict_in_a_call() {
    assert_diagnostic(
        r#"
class Guard {}

function touch(writable Guard $slot, Guard $view): void {}

function route(writable Guard $guard): void
{
    touch($guard, $guard);
}
"#,
        "E0477",
    );
}

#[test]
fn two_writable_uses_of_one_owner_conflict_in_a_call() {
    assert_diagnostic(
        r#"
class Guard {}

function swap(writable Guard $left, writable Guard $right): void {}

function route(writable Guard $guard): void
{
    swap($guard, $guard);
}
"#,
        "E0477",
    );
}

#[test]
fn writable_method_receiver_conflicts_with_reading_the_same_owner_as_an_argument() {
    assert_diagnostic(
        r#"
class Guard
{
    writable function copyFrom(Guard $other): void {}
}

function route(writable Guard $guard): void
{
    $guard->copyFrom($guard);
}
"#,
        "E0477",
    );
}

#[test]
fn outer_call_borrows_remain_live_while_later_arguments_are_evaluated() {
    assert_diagnostic(
        r#"
class Guard {}

function observe(Guard $guard, string $label): void {}
function label(writable Guard $guard): string { return "updated"; }

function route(writable Guard $guard): void
{
    observe($guard, label($guard));
}
"#,
        "E0477",
    );
}

#[test]
fn ordinary_call_borrows_end_after_the_statement() {
    doriac::check_source(
        "stage21-nll-call-end.doria",
        r#"
class Guard {}

function observe(Guard $guard): void {}
function update(writable Guard $guard): void {}

function route(writable Guard $guard): void
{
    observe($guard);
    update($guard);
    observe($guard);
}
"#,
    )
    .expect("non-lexical call borrows end after their last use");
}

#[test]
fn self_returns_elide_to_the_receiver_borrow_and_support_chaining() {
    doriac::check_source(
        "stage21-self-return.doria",
        r#"
class Guard
{
    function inspect(): self { return $this; }
    writable function touch(): self { return $this; }
}

function route(writable Guard $guard): void
{
    $guard->inspect()->inspect();
    $guard->touch()->touch();
}
"#,
    )
    .expect("self returns should preserve the receiver borrow through a chain");
}

#[test]
fn owned_temporary_is_a_valid_writable_receiver() {
    doriac::check_source(
        "stage21-owned-temporary.doria",
        r#"
class Guard
{
    writable function touch(): void {}
}

function main(): void
{
    (new Guard())->touch();
}
"#,
    )
    .expect("a freshly owned temporary is an exclusive writable place");
}

#[test]
fn borrow_return_cannot_initialize_an_owning_let() {
    assert_diagnostic(
        r#"
class Guard
{
    function inspect(): self { return $this; }
}

function route(Guard $guard): void
{
    let $alias = $guard->inspect();
}
"#,
        "E0478",
    );
}

#[test]
fn property_assignment_holds_writable_access_while_evaluating_the_value() {
    assert_diagnostic(
        r#"
class Box
{
    writable int $value = 0;
}

function update(writable Box $box): int { return 1; }

function route(writable Box $box): void
{
    $box->value = update($box);
}
"#,
        "E0477",
    );
}

#[test]
fn borrowed_result_cannot_be_passed_to_take() {
    assert_diagnostic(
        r#"
class Guard
{
    function inspect(): self { return $this; }
}

function consume(take Guard $guard): void {}

function route(Guard $guard): void
{
    consume($guard->inspect());
}
"#,
        "E0474",
    );
}

#[test]
fn owned_factory_results_are_exclusive_writable_receivers() {
    assert_valid_mir(
        r#"
class Guard
{
    writable function touch(): void {}
}

function make(): Guard { return new Guard(); }

function main(): void
{
    make()->touch();
}
"#,
    );
}

#[test]
fn accessors_and_single_borrowed_parameters_use_return_elision() {
    doriac::check_source(
        "stage21-return-elision.doria",
        r#"
class Child {}

class Parent
{
    Child $child = new Child();
    function getChild(): Child { return $this->child; }
}

function identity(Child $child): Child { return $child; }
"#,
    )
    .expect("one unambiguous borrowed source should determine the returned borrow");

    assert_diagnostic(
        r#"
class Guard {}
function identity(Guard $guard): Guard { return $guard; }
function route(Guard $guard): void { let $alias = identity($guard); }
"#,
        "E0478",
    );
}

#[test]
fn returned_borrow_provenance_flows_through_calls() {
    assert_valid_mir(
        r#"
class Guard
{
    function inspect(): self { return $this; }
    function wrappedInspect(): self { return $this->inspect(); }
    static function identity(Guard $guard): Guard { return $guard; }
    static function wrappedIdentity(Guard $guard): Guard
    {
        return self::identity($guard);
    }
}

function wrap(Guard $guard): Guard { return identity($guard); }
function identity(Guard $guard): Guard { return $guard; }
function observe(Guard $guard): void {}

function main(): void
{
    let $guard = new Guard();
    observe(wrap($guard));
    observe($guard->wrappedInspect());
    observe(Guard::wrappedIdentity($guard));
}
"#,
    );
}

#[test]
fn returned_self_borrows_lower_and_validate_in_native_mir() {
    assert_valid_mir(
        r#"
class Guard
{
    writable function touch(): self { return $this; }
    writable function finish(): void {}
}

function main(): void
{
    let writable $guard = new Guard();
    $guard->touch()->finish();
}
"#,
    );
}

#[test]
fn compound_assignment_holds_writable_access_while_evaluating_the_value() {
    assert_diagnostic(
        r#"
class Box
{
    writable int $value = 0;
}

function update(writable Box $box): int { return 1; }

function route(writable Box $box): void
{
    $box->value += update($box);
}
"#,
        "E0477",
    );
}

#[test]
fn every_writable_move_parameter_requires_a_writable_binding() {
    assert_diagnostic(
        r#"
function update(writable mixed $value): void {}
function route(mixed $value): void { update($value); }
"#,
        "E0479",
    );
}

#[test]
fn borrowed_call_arguments_are_never_dropped_as_owned_temporaries() {
    let source = r#"
class Guard
{
    function inspect(): self { return $this; }
    function __destruct() { echo "drop\n"; }
}

function observe(Guard $guard): void {}

function main(): void
{
    let $guard = new Guard();
    observe($guard->inspect());
    echo "alive\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage21-borrowed-temporary.doria", source)
        .expect("borrowed arguments should lower to MIR");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, b"alive\ndrop\n");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("borrowed arguments should lower through Cranelift")
        .is_empty());
}

#[test]
fn temporary_sources_of_returned_borrows_live_through_the_enclosing_statement() {
    let source = r#"
class Guard
{
    function __destruct() { echo "drop\n"; }
}

function identity(Guard $guard): Guard { return $guard; }
function observe(Guard $guard): void { echo "observe\n"; }

function main(): void
{
    observe(identity(new Guard()));
    echo "after\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage21-temporary-borrow-source.doria", source)
        .expect("a borrowed temporary source should lower to MIR");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, b"observe\ndrop\nafter\n");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("a borrowed temporary source should lower through Cranelift")
        .is_empty());
}

#[test]
fn borrowed_results_cannot_be_stored_in_owning_collection_literals() {
    assert_diagnostic(
        r#"
class Guard
{
    function inspect(): self { return $this; }
}

function route(Guard $guard): void
{
    Guard[] $items = [$guard->inspect()];
}
"#,
        "E0478",
    );
}

#[test]
fn borrowed_results_cannot_initialize_owning_properties() {
    assert_diagnostic(
        r#"
class Child {}
function identity(Child $child): Child { return $child; }
class Box { Child $child = identity(new Child()); }
"#,
        "E0478",
    );
}

#[test]
fn shadowed_parameters_do_not_define_returned_borrow_provenance() {
    assert_valid_mir(
        r#"
class Guard {}

function make(Guard $guard): Guard
{
    let $guard = new Guard();
    return $guard;
}

function main(): void
{
    let $input = new Guard();
    let $output = make($input);
}
"#,
    );
}

#[test]
fn property_assignment_rejects_overlapping_direct_rhs_reads() {
    assert_diagnostic(
        r#"
class Box
{
    writable int $value = 0;
    int $other = 1;
}

function route(writable Box $box): void
{
    $box->value = $box->other;
}
"#,
        "E0477",
    );
}

#[test]
fn static_borrow_returns_preserve_parameter_numbering_in_mir() {
    assert_valid_mir(
        r#"
class Guard
{
    static function identity(Guard $guard): Guard { return $guard; }
}

function observe(Guard $guard): void {}

function main(): void
{
    let $guard = new Guard();
    observe(Guard::identity($guard));
}
"#,
    );
}

#[test]
fn borrowed_results_cannot_be_stored_in_owned_properties() {
    assert_diagnostic(
        r#"
class Child {}
class Box
{
    writable Child $child = new Child();
}

function identity(Child $child): Child { return $child; }

function route(writable Box $box, Child $child): void
{
    $box->child = identity($child);
}
"#,
        "E0478",
    );
}

#[test]
fn method_call_results_preserve_parameter_borrow_provenance() {
    assert_valid_mir(
        r#"
class Guard
{
    function inspect(): self { return $this; }
}

function alias(Guard $guard): Guard
{
    return $guard->inspect();
}

function main(): void
{
    let $guard = new Guard();
    alias($guard);
}
"#,
    );
}

#[test]
fn unreachable_returns_do_not_change_returned_borrow_inference() {
    assert_valid_mir(
        r#"
class Guard
{
    function direct(): self
    {
        return $this;
        return new Guard();
    }

    function conditional(): self
    {
        if (false) { return new Guard(); }
        return $this;
    }

    function looping(): self
    {
        while (true) { return $this; }
        return new Guard();
    }
}

function observe(Guard $guard): void {}

function main(): void
{
    let $guard = new Guard();
    observe($guard->direct());
    observe($guard->conditional());
    observe($guard->looping());
}
"#,
    );
}

#[test]
fn discarded_fluent_borrow_calls_lower_and_run_without_dropping_the_owner() {
    let source = r#"
class Guard
{
    writable function add(): self { return $this; }
    function __destruct() { echo "drop\n"; }
}

function main(): void
{
    let writable $guard = new Guard();
    $guard->add()->add();
    echo "alive\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage21-discarded-borrow.doria", source)
        .expect("discarded returned borrows should lower to MIR");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, b"alive\ndrop\n");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("discarded returned borrows should lower through Cranelift")
        .is_empty());
}

#[test]
fn this_property_assignment_holds_writable_access_during_rhs_evaluation() {
    for source in [
        r#"
class Box
{
    writable int $value = 0;
    int $other = 1;

    writable function copy(): void
    {
        $this->value = $this->other;
    }
}
"#,
        r#"
class Box
{
    writable int $value = 0;

    writable function update(): void
    {
        $this->value = replace($this);
    }
}

function replace(writable Box $box): int { return 1; }
"#,
    ] {
        assert_diagnostic(source, "E0477");
    }

    doriac::check_source(
        "stage21-self-read-modify-write.doria",
        r#"
class Counter
{
    writable int $value = 0;
    writable function advance(): void { $this->value = $this->value + 1; }
}
"#,
    )
    .expect("an assignment may read the exact property it is replacing");
}

#[test]
fn property_place_reads_remain_live_across_binary_operands() {
    assert_diagnostic(
        r#"
class Box { writable int $value = 0; }
function update(writable Box $box): int { return 1; }
function route(writable Box $box): int { return $box->value + update($box); }
"#,
        "E0477",
    );

    assert_diagnostic(
        r#"
class Box { writable int $value = 0; }
function consume(take Box $box): int { return 1; }
function route(take Box $box): int { return $box->value + consume($box); }
"#,
        "E0471",
    );
}

#[test]
fn property_place_reads_remain_live_across_interpolation_parts() {
    assert_diagnostic(
        r#"
class Box { int $value = 0; }
function update(writable Box $box): int { return 1; }
function render(writable Box $box): string
{
    return "{$box->value}{update($box)}";
}
"#,
        "E0477",
    );
}

#[test]
fn property_place_reads_remain_live_across_collection_elements() {
    assert_diagnostic(
        r#"
class Box { int $value = 0; }
function update(writable Box $box): int { return 1; }
function collect(writable Box $box): int[]
{
    return [$box->value, update($box)];
}
"#,
        "E0477",
    );
}

#[test]
fn nested_exact_assignment_target_reads_remain_accepted() {
    doriac::check_source(
        "stage21-nested-exact-target.doria",
        r#"
class Child { writable int $value = 0; }
class Box { writable Child $child = new Child(); }

function update(writable Box $box): void
{
    $box->child->value = $box->child->value + 1;
}
"#,
    )
    .expect("an assignment may read its exact nested property target");
}

#[test]
fn property_writes_through_owned_rvalues_require_a_stable_object_path() {
    for source in [
        r#"
class Box { writable int $value = 0; }
function main(): void { (new Box())->value = 1; }
"#,
        r#"
class Box { writable int $value = 0; }
function make(): Box { return new Box(); }
function main(): void { make()->value = 1; }
"#,
    ] {
        assert_diagnostic(source, "E0204");
    }
}

#[test]
fn readonly_move_properties_cannot_be_passed_as_writable() {
    assert_diagnostic(
        r#"
class Box
{
    mixed $payload = 1;
}

function update(writable mixed $payload): void {}

function route(writable Box $box): void
{
    update($box->payload);
}
"#,
        "E0479",
    );

    doriac::check_source(
        "stage21-writable-move-property.doria",
        r#"
class Box
{
    writable mixed $payload = 1;
}

function update(writable mixed $payload): void {}
function route(writable Box $box): void { update($box->payload); }
"#,
    )
    .expect("a writable move property remains a valid writable argument");

    assert_diagnostic(
        r#"
class Store { static int $payload = 1; }
function update(writable mixed $payload): void {}
function main(): void { update(Store::payload); }
"#,
        "E0479",
    );

    doriac::check_source(
        "stage21-writable-static-property.doria",
        r#"
class Store { static writable int $payload = 1; }
function update(writable mixed $payload): void {}
function main(): void { update(Store::payload); }
"#,
    )
    .expect("a writable static property remains a valid writable argument");
}

#[test]
fn class_constants_are_not_writable_mixed_storage() {
    assert_diagnostic(
        r#"
class Store { const VALUE = 1; }
function update(writable mixed $value): void {}
function main(): void { update(Store::VALUE); }
"#,
        "E0204",
    );
}

#[test]
fn static_properties_are_stable_borrow_roots() {
    assert_diagnostic(
        r#"
class Store { static writable int $payload = 1; }
function update(writable mixed $value): int { return 0; }
function observe(int $value, int $result): void {}
function main(): void { observe(Store::payload, update(Store::payload)); }
"#,
        "E0477",
    );

    doriac::check_source(
        "stage21-static-read-modify-write.doria",
        r#"
class Store
{
    static writable int $payload = 1;
    static function update(): void { self::payload = self::payload + 1; }
}
"#,
    )
    .expect("an assignment may read the exact static property it replaces");
}

#[test]
fn readonly_scalar_properties_cannot_be_passed_as_writable_mixed() {
    assert_diagnostic(
        r#"
class Box { int $value = 0; }
function update(writable mixed $value): void {}
function route(writable Box $box): void { update($box->value); }
"#,
        "E0479",
    );

    doriac::check_source(
        "stage21-writable-scalar-property.doria",
        r#"
class Box { writable int $value = 0; }
function update(writable mixed $value): void {}
function route(writable Box $box): void { update($box->value); }
"#,
    )
    .expect("a writable scalar property remains a valid writable argument");
}

#[test]
fn writable_mixed_requires_writable_scalar_storage() {
    for source in [
        r#"
function update(writable mixed $value): void {}
function main(): void { let $value = 1; update($value); }
"#,
        r#"
function update(writable mixed $value): void {}
function route(int $value): void { update($value); }
"#,
        r#"
function update(writable mixed $value): void {}
function main(): void { update(1); }
"#,
        r#"
function update(writable mixed $value): void {}
function main(): void { let $value = 1; update($value + 1); }
"#,
    ] {
        assert_diagnostic(source, "E0204");
    }

    doriac::check_source(
        "stage21-writable-scalar-storage.doria",
        r#"
function update(writable mixed $value): void {}
function route(writable int $value): void { update($value); }
function main(): void { writable int $value = 1; update($value); }
"#,
    )
    .expect("writable scalar bindings remain valid writable mixed arguments");
}

#[test]
fn property_initializers_resolve_self_qualified_borrow_returns() {
    assert_diagnostic(
        r#"
class Child {}
class Box
{
    static function identity(Child $child): Child { return $child; }
    Child $child = self::identity(new Child());
}
"#,
        "E0478",
    );
}

#[test]
fn readonly_class_properties_cannot_be_passed_as_writable_mixed() {
    assert_diagnostic(
        r#"
class Child {}
class Box { Child $child = new Child(); }
function update(writable mixed $value): void {}
function route(writable Box $box): void { update($box->child); }
"#,
        "E0479",
    );

    doriac::check_source(
        "stage21-writable-class-property-as-mixed.doria",
        r#"
class Child {}
class Box { writable Child $child = new Child(); }
function update(writable mixed $value): void {}
function route(writable Box $box): void { update($box->child); }
"#,
    )
    .expect("a writable class property remains a writable mixed argument");
}

#[test]
fn returned_borrow_writability_is_downgraded_across_reachable_paths() {
    assert_valid_mir(
        r#"
class Node
{
    function __construct(take Node $child) {}
}

function choose(writable Node $node, bool $direct): Node
{
    if ($direct) { return $node; }
    return $node->child;
}

function main(): void {}
"#,
    );
}

#[test]
fn nested_property_reads_remain_live_for_the_enclosing_operation() {
    for source in [
        r#"
class Box { int $value = 0; }
function update(writable Box $box): int { return 1; }
function observe(int $left, int $right): void {}
function route(writable Box $box): void
{
    observe(($box->value + 1) * 2, update($box));
}
"#,
        r#"
class Box { int $value = 0; }
function update(writable Box $box): int { return 1; }
function route(writable Box $box): int
{
    return (($box->value + 1) * 2) + update($box);
}
"#,
    ] {
        assert_diagnostic(source, "E0477");
    }
}

#[test]
fn writable_mixed_requires_every_property_path_segment_to_be_writable() {
    assert_diagnostic(
        r#"
class Child { writable mixed $payload = 1; }
class Box { Child $child = new Child(); }
function update(writable mixed $value): void {}
function route(writable Box $box): void { update($box->child->payload); }
"#,
        "E0479",
    );

    doriac::check_source(
        "stage21-writable-mixed-property-path.doria",
        r#"
class Child { writable mixed $payload = 1; }
class Box { writable Child $child = new Child(); }
function update(writable mixed $value): void {}
function route(writable Box $box): void { update($box->child->payload); }
"#,
    )
    .expect("a fully writable property path should satisfy writable mixed");
}

#[test]
fn readonly_returned_borrows_cannot_satisfy_writable_mixed() {
    assert_diagnostic(
        r#"
class Guard {}
function identity(Guard $guard): Guard { return $guard; }
function update(writable mixed $value): void {}
function route(writable Guard $guard): void { update(identity($guard)); }
"#,
        "E0479",
    );
}

#[test]
fn readonly_this_cannot_satisfy_writable_mixed() {
    assert_diagnostic(
        r#"
function update(writable mixed $value): void {}
class Guard
{
    function route(): void { update($this); }
}
"#,
        "E0479",
    );

    doriac::check_source(
        "stage21-writable-this-as-mixed.doria",
        r#"
function update(writable mixed $value): void {}
class Guard
{
    writable function route(): void { update($this); }
}
"#,
    )
    .expect("a writable receiver should satisfy writable mixed");
}

#[test]
fn parameter_return_elision_requires_one_borrowed_class_parameter() {
    assert_diagnostic(
        r#"
class Guard {}
function first(Guard $left, Guard $right): Guard { return $left; }
"#,
        "E0474",
    );

    doriac::check_source(
        "stage21-elision-with-copy-parameter.doria",
        r#"
class Guard {}
function choose(Guard $guard, bool $alternate): Guard { return $guard; }
"#,
    )
    .expect("copy-scalar parameters should not count as borrowed return sources");
}

#[test]
fn enclosing_borrow_reactivation_respects_constant_short_circuiting() {
    for condition in ["false && $box->flag", "true || $box->flag"] {
        doriac::check_source(
            "stage21-dead-short-circuit-borrow.doria",
            format!(
                r#"
class Box {{ bool $flag = false; }}
function update(writable Box $box): int {{ return 1; }}
function observe(bool $condition, int $value): void {{}}
function route(writable Box $box): void {{ observe({condition}, update($box)); }}
"#
            ),
        )
        .expect("a property in a dead short-circuit operand is not borrowed");
    }

    assert_diagnostic(
        r#"
class Box { bool $flag = false; }
function update(writable Box $box): int { return 1; }
function observe(bool $condition, int $value): void {}
function route(writable Box $box): void
{
    observe(true && $box->flag, update($box));
}
"#,
        "E0477",
    );
}

#[test]
fn nested_call_and_constructor_arguments_preserve_property_borrows() {
    for source in [
        r#"
class Box { int $value = 1; }
function copy(int $value): int { return $value; }
function update(writable Box $box): int { return 1; }
function observe(int $left, int $right): void {}
function route(writable Box $box): void
{
    observe(copy($box->value), update($box));
}
"#,
        r#"
class Box { int $value = 1; }
class Wrapper { function __construct(int $value) {} }
function update(writable Box $box): int { return 1; }
function observe(Wrapper $left, int $right): void {}
function route(writable Box $box): void
{
    observe(new Wrapper($box->value), update($box));
}
"#,
        r#"
class Box { int $value = 1; }
class Wrapper { function __construct(int $value) {} }
function update(writable Box $box): int { return 1; }
function observe(take Wrapper $left, int $right): void {}
function route(writable Box $box): void
{
    observe(new Wrapper($box->value), update($box));
}
"#,
    ] {
        assert_diagnostic(source, "E0477");
    }
}

#[test]
fn owned_array_elements_preserve_nested_property_borrows() {
    assert_diagnostic(
        r#"
class Box { int $value = 1; }
class Wrapper { function __construct(int $value) {} }
function update(writable Box $box): Wrapper { return new Wrapper(1); }
function route(writable Box $box): void
{
    let $values = [new Wrapper($box->value), update($box)];
}
"#,
        "E0477",
    );
}

#[test]
fn method_receiver_inputs_remain_borrowed_across_arguments() {
    assert_diagnostic(
        r#"
class Box { int $value = 1; }
class Target { function touch(int $value): void {} }
function make(int $value): Target { return new Target(); }
function update(writable Box $box): int { return 1; }
function route(writable Box $box): void
{
    make($box->value)->touch(update($box));
}
"#,
        "E0477",
    );
}

#[test]
fn writable_property_paths_require_stable_roots() {
    assert_diagnostic(
        r#"
class Child {}
class Box { writable Child $child = new Child(); }
function make(): Box { return new Box(); }
function update(writable Child $child): void {}
function route(): void { update(make()->child); }
"#,
        "E0204",
    );
}

#[test]
fn display_borrows_remain_live_across_interpolation_parts() {
    for displayed in ["$guard", "$guard->inspect()"] {
        assert_diagnostic(
            &format!(
                r#"
class Guard implements Displayable
{{
    function inspect(): self {{ return $this; }}
    function toString(): string {{ return "guard"; }}
}}
function update(writable Guard $guard): string {{ return "updated"; }}
function route(writable Guard $guard): string
{{
    return "{{{displayed}}}{{update($guard)}}";
}}
"#
            ),
            "E0477",
        );
    }
}

#[test]
fn local_compound_assignments_may_read_their_own_value() {
    assert_valid_mir(
        r#"
function main(): int
{
    let writable $value = 1;
    $value += $value;
    return $value;
}
"#,
    );
}

#[test]
fn nested_property_returns_stop_before_unlowerable_mir_places() {
    assert_diagnostic(
        r#"
class Leaf {}
class Child { Leaf $leaf = new Leaf(); }
class Parent
{
    Child $child = new Child();
    function leaf(): Leaf { return $this->child->leaf; }
}
"#,
        "E0472",
    );
}
