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
