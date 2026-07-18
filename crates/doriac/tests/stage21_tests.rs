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
fn owned_returns_reject_borrowed_call_results_without_provenance_elision() {
    assert_diagnostic(
        r#"
class Guard
{
    function inspect(): self { return $this; }
}

function alias(Guard $guard): Guard
{
    return $guard->inspect();
}
"#,
        "E0478",
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
}

function observe(Guard $guard): void {}

function main(): void
{
    let $guard = new Guard();
    observe($guard->direct());
    observe($guard->conditional());
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
