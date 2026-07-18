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
