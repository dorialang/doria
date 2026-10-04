#[path = "common/native_execution.rs"]
mod native_execution;

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

fn assert_linked_native_execution(program: &doriac::mir::Program, stdout: &str) {
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(program)
        .expect("validated MIR should lower through LLVM")
        .is_empty());
    native_execution::assert_native_execution(
        program,
        doriac::backend::NativeProfile::Fast,
        stdout,
    );
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        program,
        doriac::backend::NativeProfile::Release,
        stdout,
    );
}

#[test]
fn temporary_returned_borrow_homes_preserve_statement_cleanup_across_native_profiles() {
    let source = include_str!("../../../examples/native/main_temporary_returned_borrows.doria");
    let stdout = include_str!("fixtures/native_io/main_temporary_returned_borrows/expected_stdout");
    let program = doriac::lower_source_to_mir("temporary-returned-borrows.doria", source)
        .expect("temporary returned borrows should lower to MIR");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, stdout.as_bytes());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.exit_status, 0);
    assert!(output.runtime_diagnostic.is_none());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("temporary returned borrows should lower through Cranelift")
        .is_empty());
    assert_linked_native_execution(&program, stdout);
}

#[test]
fn materialized_call_results_preserve_owned_and_borrowed_writable_capabilities() {
    let source = r#"
class Counter
{
    writable int $value = 0;
    function __construct(parameter int $initial) { $this->value = $initial; }
    writable function touch(): self { return $this; }
    writable function bump(): void
    {
        $this->value++;
        echo "{$this->value};";
    }
}
class Failure implements Error
{
    function __construct(string $message) {}
}
function maybe(bool $present, bool $fail, int $initial = 10): ?Counter throws Failure
{
    if ($fail) { throw new Failure("factory"); }
    if ($present) { return new Counter($initial); }
    return null;
}
function direct(writable Counter $counter): Counter { return $counter; }
function checked(writable Counter $counter, bool $fail): Counter throws Failure
{
    if ($fail) { throw new Failure("borrow"); }
    return $counter;
}
function readonlyResult(Counter $counter): Counter { return $counter; }
function observe(Counter $counter): void { echo "read{$counter->value};"; }
function main(): void throws Error
{
    maybe(true, false)?->bump();
    maybe(false, false)?->bump();
    echo "afterNullable;";

    let writable $owner = new Counter(0);
    direct($owner)->bump();
    checked($owner, false)->bump();

    writable ?Counter $nullable = maybe(true, false, 20);
    $nullable?->touch()?->bump();

    let $indirect = function (writable Counter $counter): Counter { return $counter; };
    observe($indirect($owner));
    let $checkedIndirect = function (writable Counter $counter, bool $fail): Counter {
        if ($fail) { throw new Failure("indirect borrow"); }
        return $counter;
    };
    observe($checkedIndirect($owner, false));
    echo "owner{$owner->value};";
}
"#;
    let program = doriac::lower_source_to_mir("call-result-capabilities.doria", source)
        .expect("materialized call results should retain their checked capabilities");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    // Source-level borrowed locals remain readonly. Inspect the intermediate
    // call results directly instead of granting a writable source alias.
    // Decision 0123 gives every structural function value ambient checked
    // transport, including the body with no required effects. Exercise both
    // ambient-only and required-Failure profiles, not an infallible ABI.
    let mut indirect_profiles = [0; 2];
    for function in &program.functions {
        for block in &function.blocks {
            let (function_type, result) = match &block.terminator {
                doriac::mir::Terminator::CheckedIndirectCall {
                    function_type,
                    result: Some(result),
                    ..
                } => (*function_type, *result),
                _ => continue,
            };
            let definition = &program.function_types[function_type.0];
            if definition
                .return_borrow
                .is_some_and(|borrow| borrow.writable)
            {
                assert!(!definition.ambient_checked_effects.is_empty());
                assert!(function.locals[result.0].writable);
                assert!(!function.locals[result.0].owned);
                indirect_profiles[usize::from(!definition.checked_effects.is_empty())] += 1;
            }
        }
    }
    assert_eq!(indirect_profiles, [1, 1]);
    let stdout = "11;afterNullable;1;2;21;read2;read2;owner2;";
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, stdout.as_bytes());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.exit_status, 0);
    assert!(output.runtime_diagnostic.is_none());
    assert_linked_native_execution(&program, stdout);

    assert_diagnostic(
        &source.replace("direct($owner)->bump();", "readonlyResult($owner)->bump();"),
        "E0203",
    );
}

#[test]
fn borrowed_collections_remain_owned_by_the_source() {
    let source = include_str!("../../../examples/native/main_returned_collection_borrows.doria");
    let program = doriac::lower_source_to_mir("borrows.doria", source).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        result.stdout,
        include_bytes!("fixtures/native_io/main_returned_collection_borrows/expected_stdout")
    );
}

#[test]
fn returned_collection_borrows_use_the_same_provenance_as_object_results() {
    for ty in ["List<int>", "int[]", "Dictionary<string, int>"] {
        let initializer = if ty.starts_with("Dictionary") {
            "[\"x\" => 1]"
        } else {
            "[1]"
        };
        let key = if ty.starts_with("Dictionary") {
            "\"x\""
        } else {
            "0"
        };
        let declarations = format!(
            r#"
class Store {{
    {ty} $values = {initializer};
    function getValues(): {ty} {{ return $this->values; }}
    writable function change(): void {{}}
}}
function forward(Store $store): {ty} {{ return $store->getValues(); }}
"#
        );
        assert_valid_mir(&format!(
            r#"{declarations}
function main(): void {{
    let writable $store = new Store();
    let $values = forward($store);
    echo $values[{key}];
    $store->change();
}}
"#
        ));
        assert_diagnostic(
            &format!(
                r#"{declarations}
function main(): void {{
    let writable $store = new Store();
    let $values = forward($store);
    $store->change();
    echo $values[{key}];
}}
"#
            ),
            "E0477",
        );
    }
}

#[test]
fn returned_move_borrows_do_not_own_their_payloads() {
    let source = include_str!("../../../examples/native/main_returned_move_borrows.doria");
    let program = doriac::lower_source_to_mir("borrows.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        result.stdout,
        include_bytes!("fixtures/native_io/main_returned_move_borrows/expected_stdout")
    );
    assert_eq!(result.stderr, b"");
    assert_eq!(result.exit_status, 0);
    assert!(result.runtime_diagnostic.is_none());
    assert_linked_native_execution(
        &program,
        include_str!("fixtures/native_io/main_returned_move_borrows/expected_stdout"),
    );
}

#[test]
fn returned_borrow_elision_counts_resolved_move_parameters() {
    for ty in ["List<int>", "SharedReference<Item>", "Packet"] {
        let declarations =
            "class Item {} enum Packet { case Value(Item $item); } enum Flag { case On; }";
        doriac::check_source(
            "borrows.doria",
            format!(
            "{declarations} function select({ty} $value, Flag $flag): {ty} {{ return $value; }}"
        ),
        )
        .expect("Copy enums must not count as borrowed sources");
        assert_diagnostic(
            &format!(
                "{declarations} function select({ty} $left, {ty} $right): {ty} {{ return $left; }}"
            ),
            "E0474",
        );
    }
}

#[test]
fn null_return_paths_preserve_the_other_paths_borrow() {
    for ty in [
        "Item",
        "Readable",
        "List<Item>",
        "Packet",
        "SharedReference<Item>",
        "mixed",
    ] {
        for body in [
            "if ($present) { return $value; } return (null);",
            "if (!$present) { return null; } return $value;",
        ] {
            for body in [
                body.to_owned(),
                format!("let $scratch = new Item(); {body}"),
                format!(
                    "if (true) {{ {body} }} finally {{ let $scratch = new Item(); }} return null;"
                ),
            ] {
                assert_valid_mir(&format!(
                    "interface Readable {{ function read(): int; }} class Item implements Readable {{ function read(): int {{ return 1; }} }} enum Packet {{ case Value(Item $item); }} function select({ty} $value, bool $present): ?{ty} {{ {body} }} function main(): void {{}}"
                ));
            }
        }
    }
}

#[test]
fn nullable_return_temporary_cannot_hide_an_owned_value() {
    use doriac::mir::{ClassExpression, NullableClassExpression, Rvalue, Statement, Type};
    let source = r#"
class Item {}
function select(Item $value, bool $present): ?Item {
    let $scratch = new Item();
    if ($present) { return $value; }
    return null;
}
function main(): void {}
"#;
    let mut program = doriac::lower_source_to_mir("borrows.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let function = program
        .functions
        .iter_mut()
        .find(|f| f.name == "select")
        .unwrap();
    let scratch = function
        .locals
        .iter()
        .find(|local| local.name == "scratch")
        .unwrap();
    let Type::Class(class) = scratch.ty else {
        panic!("expected class temporary")
    };
    let replacement =
        Rvalue::NullableClass(NullableClassExpression::Class(ClassExpression::Local {
            local: scratch.id,
            class,
            transfer: false,
        }));
    let (target, block) = function
        .blocks
        .iter_mut()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find_map(|statement| match statement {
                    Statement::AssignLocal { target, value }
                        if value.ty() == Type::NullableClass(class) =>
                    {
                        Some(*target)
                    }
                    _ => None,
                })
                .map(|target| (target, block))
        })
        .expect("saved borrowed return");
    block.statements.push(Statement::AssignLocal {
        target,
        value: replacement,
    });
    let error = doriac::mir_validation::validate_program(&program).unwrap_err();
    assert!(
        error
            .message
            .contains("mixes owned and borrowed assignments"),
        "{error:?}"
    );
}

#[test]
fn null_return_has_no_saved_payload_through_cleanup_and_finalizers() {
    use doriac::mir::{Statement, Terminator};
    let source = r#"
class Item {}
function select(Item $value, bool $present): ?Item {
    let $scratch = new Item();
    if ($present) { return $value; } else { return null; }
    finally { let $finalizer = new Item(); }
}
function main(): void {}
"#;
    let program = doriac::lower_source_to_mir("borrows.doria", source).unwrap();
    let function = program
        .functions
        .iter()
        .find(|f| f.name == "select")
        .unwrap();
    assert!(function.blocks.iter().any(|block| {
        matches!(&block.terminator, Terminator::Return(value) if value.is_null_value())
    }));
    assert!(!function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| {
            matches!(statement, Statement::AssignLocal { value, .. } if value.is_null_value())
        }));
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn move_calls_cannot_erase_returned_borrow() {
    use doriac::mir::{PayloadEnumExpression, Rvalue, SharedReferenceExpression, Statement};
    for shared in [false, true] {
        let source = if shared {
            "class Item {} function lend(SharedReference<Item> $item): SharedReference<Item> { return $item; } function main(): void { let $owner = shared new Item(); let $view = lend($owner); }"
        } else {
            "class Item {} enum Packet { case Value(Item $item); } function lend(Packet $item): Packet { return $item; } function main(): void { let $owner = Packet::Value(new Item()); let $view = lend($owner); }"
        };
        let mut program = doriac::lower_source_to_mir("borrows.doria", source).unwrap();
        let borrow = program
            .functions
            .iter_mut()
            .flat_map(|function| &mut function.blocks)
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match statement {
                Statement::AssignLocal {
                    value: Rvalue::PayloadEnum(PayloadEnumExpression::Call { return_borrow, .. }),
                    ..
                }
                | Statement::AssignLocal {
                    value:
                        Rvalue::SharedReference(SharedReferenceExpression::Call {
                            return_borrow, ..
                        }),
                    ..
                } => Some(return_borrow),
                _ => None,
            })
            .expect("borrowing call");
        assert!(borrow.take().is_some());
        let error = doriac::mir_validation::validate_program(&program).unwrap_err();
        assert!(error.message.contains("return ownership"), "{error:?}");
    }
}

#[test]
fn nullable_collection_call_cannot_erase_returned_borrow() {
    use doriac::mir::{NullableCollectionExpression, Rvalue, Statement};
    let source = r#"
function borrowList(?List<int> $items): ?List<int> { return $items; }
function main(): void {
    ?List<int> $items = [1];
    let $alias = borrowList($items);
    if ($alias != null) { echo $alias[0]; }
}
"#;
    let mut program = doriac::lower_source_to_mir("borrows.doria", source).unwrap();
    let value = program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            Statement::AssignLocal {
                value:
                    Rvalue::NullableCollection(NullableCollectionExpression::Call {
                        return_borrow, ..
                    }),
                ..
            } => Some(return_borrow),
            _ => None,
        })
        .expect("nullable borrowing call");
    assert!(value.take().is_some());
    let error = doriac::mir_validation::validate_program(&program).unwrap_err();
    assert!(error.message.contains("return ownership"), "{error:?}");
}

fn constructor_diagnostic_snapshot(source: &str, code: &str) -> String {
    let diagnostics =
        doriac::check_source("stage21-constructor.doria", source).expect_err("invalid constructor");
    let diagnostic = diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"));
    let help = diagnostic
        .help
        .as_deref()
        .map(|help| format!(" {help}"))
        .unwrap_or_default();
    format!(
        "code: {}\nmessage: {}\nhelp:{}\nspan: {}..{}\n",
        diagnostic.code, diagnostic.message, help, diagnostic.span.start, diagnostic.span.end,
    )
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

function route(writable Guard $guard): void throws Doria\Std\Io\IoError
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
fn borrow_return_can_initialize_a_borrowed_let() {
    doriac::check_source(
        "stage21-borrowed-let.doria",
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
    )
    .expect("returned borrows may initialize readonly borrowed locals");
}

#[test]
fn borrowed_local_ends_at_its_final_use() {
    doriac::check_source(
        "stage21-borrowed-let-final-use.doria",
        r#"
class Guard
{
    function inspect(): self { return $this; }
    writable function touch(): void {}
}

function route(writable Guard $guard): void
{
    let $alias = $guard->inspect();
    $alias->inspect();
    $guard->touch();
}
"#,
    )
    .expect("a borrowed local should stop blocking its owner after its final use");
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

    doriac::check_source(
        "stage21-borrowed-local.doria",
        r#"
class Guard {}
function identity(Guard $guard): Guard { return $guard; }
function route(Guard $guard): void { let $alias = identity($guard); }
"#,
    )
    .expect("a local may retain a returned borrow while its owner remains live");
}

#[test]
fn borrowed_local_results_retain_their_owner_provenance() {
    assert_diagnostic(
        r#"
class Guard
{
    writable int $value = 0;
    writable function mutate(): void { $this->value++; }
}
function identity(Guard $guard): Guard { return $guard; }
function route(writable Guard $guard): void throws Doria\Std\Io\IoError
{
    let $alias = identity($guard);
    $guard->mutate();
    echo "{$alias->value}";
}
"#,
        "E0477",
    );

    assert_diagnostic(
        r#"
class Guard {}
function make(): Guard { return new Guard(); }
function identity(Guard $guard): Guard { return $guard; }
function route(): void { let $alias = identity(make()); }
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
fn borrowed_return_calls_are_evaluated_before_local_cleanup() {
    let source = r#"
class Guard
{
    function __destruct()
    {
        try { echo "drop\n"; } catch (Doria\Std\Io\IoError) {}
    }
}

function identity(Guard $guard): Guard throws Doria\Std\Io\IoError
{
    echo "identity\n";
    return $guard;
}

function forward(Guard $guard): Guard throws Doria\Std\Io\IoError
{
    let $temporary = new Guard();
    return identity($guard);
}

function main(): void throws Doria\Std\Io\IoError
{
    let $owner = new Guard();
    forward($owner);
    echo "done\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage21-borrowed-return-cleanup.doria", source)
        .expect("borrowed return calls should lower to MIR");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, b"identity\ndrop\ndone\ndrop\n");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("borrowed return cleanup should lower through Cranelift")
        .is_empty());
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
    function __destruct()
    {
        try { echo "drop\n"; } catch (Doria\Std\Io\IoError) {}
    }
}

function observe(Guard $guard): void {}

function main(): void throws Doria\Std\Io\IoError
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
    function __destruct()
    {
        try { echo "drop\n"; } catch (Doria\Std\Io\IoError) {}
    }
}

function identity(Guard $guard): Guard { return $guard; }
function observe(Guard $guard): void throws Doria\Std\Io\IoError { echo "observe\n"; }

function main(): void throws Doria\Std\Io\IoError
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
    assert_eq!(output.stderr, b"");
    assert_eq!(output.exit_status, 0);
    assert!(output.runtime_diagnostic.is_none());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("a borrowed temporary source should lower through Cranelift")
        .is_empty());
    assert_linked_native_execution(&program, "observe\ndrop\nafter\n");
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
    let source = r#"
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
"#;
    let program = doriac::lower_source_to_mir("stage21-unreachable-borrow-return.doria", source)
        .expect("source should lower to MIR");
    doriac::mir_validation::validate_program(&program).expect("MIR should validate");
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
    assert_eq!(output.exit_status, 0);
    assert!(output.runtime_diagnostic.is_none());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("validated MIR should lower through the fast native backend")
        .is_empty());
    assert_linked_native_execution(&program, "");
}

#[test]
fn discarded_fluent_borrow_calls_lower_and_run_without_dropping_the_owner() {
    let source = r#"
class Guard
{
    writable function add(): self { return $this; }
    function __destruct()
    {
        try { echo "drop\n"; } catch (Doria\Std\Io\IoError) {}
    }
}

function main(): void throws Doria\Std\Io\IoError
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
fn constructor_initialization_merges_every_reachable_branch() {
    assert_valid_mir(
        r#"
class Label
{
    string $text;

    function __construct(bool $formal, string $input)
    {
        if ($formal) {
            $this->text = "Hello " . $input;
        } else {
            $this->text = $input;
        }
    }

    function show(): void throws Doria\Std\Io\IoError { echo $this->text . "\n"; }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $formal = new Label(true, "Doria");
    let $plain = new Label(false, "Welcome");
    $formal->show();
    $plain->show();
}
"#,
    );
}

#[test]
fn panic_terminated_constructor_paths_do_not_require_initialization() {
    assert_valid_mir(
        r#"
class Token
{
    string $value;
    function __construct(bool $valid, string $input)
    {
        if ($valid) {
            $this->value = $input;
        } else {
            panic("invalid token");
        }
    }
}
function main(): void { let $token = new Token(true, "ok"); }
"#,
    );

    assert_valid_mir(
        r#"
class Token
{
    string $value;
    function __construct(bool $short, bool $alternate)
    {
        if ($short) {
            $this->value = "short";
            return;
        }
        if ($alternate) {
            if (true) { $this->value = "alternate"; }
        } else {
            $this->value = "ordinary";
        }
    }
}
function main(): void { let $token = new Token(false, true); }
"#,
    );
}

#[test]
fn writable_constructor_assignment_can_establish_and_then_mutate_state() {
    assert_valid_mir(
        r#"
class Counter
{
    writable int $value;
    function __construct(bool $seeded)
    {
        if ($seeded) { $this->value = 40; }
        $this->value = 41;
        $this->value += 1;
    }
}
function main(): void throws Doria\Std\Io\IoError { let $counter = new Counter(false); echo $counter->value; }
"#,
    );

    assert_valid_mir(
        r#"
class Pair
{
    writable int $left;
    writable int $right;
    function __construct(bool $chooseLeft)
    {
        if ($chooseLeft) { $this->left = 1; }
        else { $this->right = 2; }
        $this->left = 3;
        $this->right = 4;
    }
}
function main(): void throws Doria\Std\Io\IoError { let $pair = new Pair(true); echo $pair->left + $pair->right; }
"#,
    );
}

#[test]
fn non_repeating_for_initializer_can_establish_constructor_initialization() {
    assert_valid_mir(
        r#"
class Token
{
    string $value;
    function __construct()
    {
        for ($this->value = "ready"; false;) {}
    }
}
function main(): void throws Doria\Std\Io\IoError { let $token = new Token(); echo $token->value; }
"#,
    );
}

#[test]
fn constructor_rejects_missing_and_partial_initialization() {
    for source in [
        r#"
class Token
{
    string $value;
    function __construct(bool $set)
    {
        if ($set) { $this->value = "set"; }
    }
}
"#,
        r#"
class Token
{
    string $value;
    function __construct() {}
}
"#,
        r#"class Token { string $value; }"#,
        r#"
class Token
{
    string $value;
    function __construct(bool $set)
    {
        if ($set) { $this->value = "set"; }
        return;
    }
}
"#,
    ] {
        assert_diagnostic(source, "E0500");
    }
}

#[test]
fn readonly_partial_merge_cannot_be_repaired_by_an_unconditional_assignment() {
    assert_diagnostic(
        r#"
class Token
{
    string $value;
    function __construct(bool $set)
    {
        if ($set) { $this->value = "first"; }
        $this->value = "fallback";
    }
}
"#,
        "E0502",
    );
}

#[test]
fn constructor_rejects_property_reads_and_incomplete_this_exposure() {
    for (source, code) in [
        (
            r#"
class Token
{
    string $value;
    function __construct() { let $copy = $this->value; }
}
"#,
            "E0501",
        ),
        (
            r#"
class Token
{
    string $value;
    function __construct() { expose($this); }
}
function expose(Token $token): void {}
"#,
            "E0503",
        ),
        (
            r#"
class Token
{
    string $value;
    function __construct() { $this->inspect(); }
    function inspect(): void {}
}
"#,
            "E0503",
        ),
    ] {
        assert_diagnostic(source, code);
    }
}

#[test]
fn repeatable_bodies_do_not_establish_constructor_initialization() {
    for source in [
        r#"
class Token
{
    string $value;
    function __construct(bool $set)
    {
        while ($set) { $this->value = "set"; }
    }
}
"#,
        r#"
class Token
{
    string $value;
    function __construct(bool $set)
    {
        for (let $i = 0; $i < 1; $i++) { $this->value = "set"; }
    }
}
"#,
        r#"
class Token
{
    string $value;
    function __construct(take List<int> $items)
    {
        foreach ($items as int $item) { $this->value = "set"; }
    }
}
"#,
    ] {
        assert_diagnostic(source, "E0504");
    }

    assert_diagnostic(
        r#"
class Counter
{
    writable int $value;
    function __construct(bool $set)
    {
        while ($set) { $this->value = 1; }
    }
}
"#,
        "E0500",
    );

    assert_diagnostic(
        r#"
class Counter
{
    writable int $value;
    function __construct(bool $set)
    {
        while ($set) {
            let $copy = $this->value;
            $this->value = 1;
        }
        $this->value = 2;
    }
}
"#,
        "E0501",
    );
}

#[test]
fn constructor_initialization_diagnostics_match_snapshots() {
    let cases = [
        (
            "missing on one branch",
            r#"class A { string $x; function __construct(bool $b) { if ($b) { $this->x = "x"; } } }"#,
            "E0500",
        ),
        (
            "missing on every path",
            r#"class A { string $x; function __construct() {} }"#,
            "E0500",
        ),
        (
            "early return",
            r#"class A { string $x; function __construct() { return; } }"#,
            "E0500",
        ),
        (
            "read before initialization",
            r#"class A { string $x; function __construct() { let $copy = $this->x; } }"#,
            "E0501",
        ),
        (
            "incomplete this",
            r#"class A { string $x; function __construct() { expose($this); } } function expose(A $a): void {}"#,
            "E0503",
        ),
        (
            "duplicate readonly initialization",
            r#"class A { string $x; function __construct() { $this->x = "a"; $this->x = "b"; } }"#,
            "E0412",
        ),
        (
            "partial readonly repair",
            r#"class A { string $x; function __construct(bool $b) { if ($b) { $this->x = "a"; } $this->x = "b"; } }"#,
            "E0502",
        ),
        (
            "readonly loop initialization",
            r#"class A { string $x; function __construct(bool $b) { while ($b) { $this->x = "a"; } } }"#,
            "E0504",
        ),
        (
            "writable loop initialization",
            r#"class A { writable string $x; function __construct(bool $b) { while ($b) { $this->x = "a"; } } }"#,
            "E0500",
        ),
    ];
    let actual = cases
        .into_iter()
        .map(|(label, source, code)| {
            format!(
                "[{label}]\n{}",
                constructor_diagnostic_snapshot(source, code)
            )
        })
        .collect::<String>();
    assert_eq!(
        actual,
        include_str!("fixtures/diagnostics/stage21_constructor_initialization.txt")
            .replace("\r\n", "\n")
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
fn nested_calls_preserve_writable_borrows() {
    for later_argument in ["$box->value", "update($box)"] {
        assert_diagnostic(
            &format!(
                r#"
class Box {{ int $value = 1; }}
function update(writable Box $box): int {{ return 1; }}
function observe(int $left, int $right): void {{}}
function route(writable Box $box): void
{{
    observe(update($box), {later_argument});
}}
"#
            ),
            "E0477",
        );
    }

    assert_diagnostic(
        r#"
class Box
{
    int $value = 1;
    writable function update(): int { return 1; }
}
function observe(int $left, int $right): void {}
function route(writable Box $box): void
{
    observe($box->update(), $box->value);
}
"#,
        "E0477",
    );
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
fn nested_property_returns_preserve_receiver_provenance() {
    doriac::check_source(
        "stage21-nested-property-return.doria",
        r#"
class Leaf {}
class Child { Leaf $leaf = new Leaf(); }
class Parent
{
    Child $child = new Child();
    function leaf(): Leaf { return $this->child->leaf; }
}
"#,
    )
    .expect("nested readonly property projections borrow transitively from the receiver");
}
