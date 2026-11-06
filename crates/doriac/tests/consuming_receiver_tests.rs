#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

const DIRECT: &str = include_str!("../../../examples/native/main_consuming_receiver_direct.doria");
const CHECKED: &str =
    include_str!("../../../examples/native/main_consuming_receiver_checked.doria");
const DISPATCH: &str =
    include_str!("../../../examples/native/main_consuming_receiver_dispatch.doria");
const RETAINED: &str =
    include_str!("../../../examples/native/main_consuming_retained_receiver.doria");

fn assert_execution(source: &str, stdout: &str) -> doriac::mir::Program {
    let mir = doriac::lower_source_to_mir("consuming-receiver.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    doriac::mir_validation::validate_program(&mir)
        .unwrap_or_else(|diagnostic| panic!("{diagnostic:?}\n{mir}"));
    let result = doriac::mir_interpreter::interpret(&mir)
        .unwrap_or_else(|diagnostic| panic!("{diagnostic:?}\n{mir}"));
    assert_eq!(result.stdout, stdout.as_bytes());
    assert_eq!(result.stderr, b"");
    assert_eq!(result.exit_status, 0);
    assert!(result.runtime_diagnostic.is_none());
    native_execution::assert_native_execution(&mir, doriac::backend::NativeProfile::Fast, stdout);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        &mir,
        doriac::backend::NativeProfile::Release,
        stdout,
    );

    let php = doriac::compile_source_to_php("consuming-receiver.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; consuming receiver compatibility execution skipped");
        return mir;
    };
    assert!(version.status.success());
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let result = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    assert_eq!(result.stdout, stdout.as_bytes());
    mir
}

fn assert_error(source: &str, code: &str) {
    let (_, analysis) = doriac::analyze_source_for_ide("consuming-receiver.doria", source)
        .unwrap_or_else(|diagnostics| {
            panic!("must parse before semantic rejection:\n{source}\n{diagnostics:#?}")
        });
    assert!(
        analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == code
                && diagnostic.severity == doriac::diagnostics::DiagnosticSeverity::Error
        }),
        "expected {code}:\n{source}\n{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn consuming_owned_readonly_locals_temporaries_and_self_results_drop_exactly_once() {
    let mir = assert_execution(
        DIRECT,
        include_str!("fixtures/native_io/main_consuming_receiver_direct/expected_stdout"),
    );
    for function in &mir.functions {
        if function.receiver_mode != Some(doriac::mir::ReceiverMode::Consuming) {
            continue;
        }
        let receiver = &function.locals[function.params[0].0];
        assert!(receiver.owned, "{} must own its receiver", function.name);
        assert!(
            !receiver.writable,
            "{} has a readonly receiver",
            function.name
        );
        assert_eq!(
            function.parameter_modes[0],
            doriac::mir::FunctionParameterMode::Take
        );
        assert!(
            function.return_borrow.is_none(),
            "{} returns ownership",
            function.name
        );
    }
}

#[test]
fn consuming_checked_success_and_failure_release_receiver_locals_and_fields_once() {
    assert_execution(
        CHECKED,
        include_str!("fixtures/native_io/main_consuming_receiver_checked/expected_stdout"),
    );
}

#[test]
fn consuming_trait_generic_interface_and_virtual_calls_preserve_dynamic_cleanup() {
    assert_execution(
        DISPATCH,
        include_str!("fixtures/native_io/main_consuming_receiver_dispatch/expected_stdout"),
    );
}

#[test]
fn consuming_receiver_forwards_all_inherited_loans_without_borrowing_its_owned_self() {
    let mir = assert_execution(
        RETAINED,
        include_str!("fixtures/native_io/main_consuming_retained_receiver/expected_stdout"),
    );
    let function = mir
        .functions
        .iter()
        .find(|function| {
            function
                .method
                .as_ref()
                .is_some_and(|method| method.name == "pass")
        })
        .unwrap();
    assert!(function.return_borrow.is_none());
    let plan = function.blocks[function.entry_block.0]
        .statements
        .iter()
        .find_map(|statement| {
            if let doriac::mir::Statement::ControlFlowPlan(
                doriac::mir::ControlFlowPlan::RetainedSources(plan),
            ) = statement
            {
                Some(plan)
            } else {
                None
            }
        })
        .expect("consuming forward retains the caller's source loans");
    assert!(plan
        .returns
        .iter()
        .any(|source| source.local == function.params[0] && source.inherited));

    for source in ["$left[0] = 0;", "$right[0] = 0;"] {
        assert_error(
            &RETAINED.replace("echo \"value", &format!("{source} echo \"value")),
            "E0763",
        );
    }
    assert_error(
        &RETAINED.replace(
            "return $this;",
            "return new Cursor($this->owned, $this->right);",
        ),
        "E0762",
    );

    let mut forged = mir.clone();
    let function = consuming_method_mut(&mut forged);
    for statement in &mut function.blocks[function.entry_block.0].statements {
        if let doriac::mir::Statement::ControlFlowPlan(
            doriac::mir::ControlFlowPlan::RetainedSources(plan),
        ) = statement
        {
            for source in &mut plan.returns {
                source.inherited = false;
            }
        }
    }
    assert_malformed(
        &forged,
        "retained return cannot borrow from a consuming receiver",
    );

    let mut erased = mir.clone();
    let function = erased
        .functions
        .iter_mut()
        .find(|function| function.name == "forwardErased")
        .unwrap();
    let plan = function.blocks[function.entry_block.0]
        .statements
        .iter_mut()
        .find_map(|statement| {
            if let doriac::mir::Statement::ControlFlowPlan(
                doriac::mir::ControlFlowPlan::RetainedSources(plan),
            ) = statement
            {
                Some(plan)
            } else {
                None
            }
        })
        .unwrap();
    assert!(plan
        .returns
        .iter()
        .any(|source| source.local == function.params[0] && source.inherited));
    plan.returns.clear();
    assert_malformed(&erased, "return loses or outlives a retained source loan");
}

#[test]
fn consuming_method_grammar_preserves_class_interface_and_trait_receiver_modes() {
    use doriac::ast::{ClassMember, Item};
    let ast = doriac::parse_source(
        "consuming-receiver.doria",
        r#"
class Builder { consuming function finish(): self { return $this; } }
interface Finisher { consuming function finish(): int; }
trait Finishing { consuming function finish(): int { return 42; } }
"#,
    )
    .expect("accepted consuming declarations must parse without a stage gate");
    let Item::Class(class) = &ast.items[0] else {
        panic!("class");
    };
    let ClassMember::Method(class_method) = &class.members[0] else {
        panic!("class method");
    };
    let Item::Interface(interface) = &ast.items[1] else {
        panic!("interface");
    };
    let Item::Trait(trait_decl) = &ast.items[2] else {
        panic!("trait");
    };
    let ClassMember::Method(trait_method) = &trait_decl.members[0] else {
        panic!("trait method");
    };
    for method in [class_method, &interface.requirements[0], trait_method] {
        assert!(method.consuming_this);
        assert!(!method.writable_this);
        assert!(!method.is_static);
    }
}

#[test]
fn consuming_receiver_gives_away_old_binding_even_when_method_returns_self() {
    for invocation in ["$value->finish();", "let $next = $value->pass();"] {
        let source = format!(
            r#"
class Resource {{
    int $value = 42;
    consuming function finish(): void {{}}
    consuming function pass(): self {{ return $this; }}
}}
function main(): void {{
    let $value = new Resource();
    {invocation}
    echo $value->value;
}}
"#
        );
        assert_error(&source, "E0470");
    }
}

#[test]
fn checked_consuming_call_does_not_restore_receiver_ownership_in_catch() {
    assert_error(
        r#"
class Failure implements Error { function __construct(string $message) {} }
class Resource {
    int $value = 42;
    consuming function finish(): void throws Failure { throw new Failure("failed"); }
}
function main(): void {
    let $value = new Resource();
    try { $value->finish(); }
    catch (Failure) { echo $value->value; }
}
"#,
        "E0470",
    );
}

#[test]
fn consuming_calls_reject_readonly_writable_and_erased_borrowed_receivers() {
    for parameter in [
        "Resource $value",
        "writable Resource $value",
        "Finisher $value",
    ] {
        let source = format!(
            r#"
interface Finisher {{ consuming function finish(): void; }}
class Resource implements Finisher {{ consuming function finish(): void {{}} }}
function borrowed({parameter}): void {{ $value->finish(); }}
"#
        );
        assert_error(&source, "E0474");
    }
}

#[test]
fn consuming_calls_reject_borrowed_self_results() {
    assert_error(
        r#"
class Resource {
    function view(): self { return $this; }
    consuming function finish(): void {}
}
function main(): void { let $value = new Resource(); $value->view()->finish(); }
"#,
        "E0474",
    );
}

#[test]
fn consuming_call_cannot_transfer_receiver_while_an_argument_borrows_it() {
    assert_error(
        r#"
class Resource { consuming function finish(): int { return 42; } }
function observe(Resource $borrowed, int $answer): void {}
function main(): void {
    let $value = new Resource();
    observe($value, $value->finish());
}
"#,
        "E0471",
    );
}

#[test]
fn consuming_receiver_ownership_does_not_implicitly_grant_writable_access() {
    assert_error(
        r#"
class Resource {
    writable int $value = 0;
    consuming function finish(): void { $this->value = 7; }
}
"#,
        "E0201",
    );
}

#[test]
fn consuming_is_not_a_free_or_static_function_modifier() {
    for source in [
        "consuming function finish(): void {}",
        "class Resource { static consuming function finish(): void {} }",
    ] {
        assert_error(source, "E0497");
    }
}

#[test]
fn consuming_cannot_change_compiler_invoked_lifecycle_protocols() {
    for method in ["__construct", "__destruct"] {
        assert_error(
            &format!("class Resource {{ consuming function {method}() {{}} }}"),
            "E0466",
        );
    }
}

#[test]
fn consuming_conflicts_with_borrow_modifiers_and_has_no_take_or_taker_alias() {
    for prefix in [
        "readonly consuming",
        "consuming readonly",
        "writable consuming",
        "consuming writable",
        "consuming static",
        "writable static",
        "take",
        "taker",
    ] {
        let source = format!("class Resource {{ {prefix} function finish(): void {{}} }}");
        assert!(
            doriac::parse_source("consuming-receiver.doria", &source).is_err(),
            "invalid method modifier must be rejected: {source}"
        );
    }
}

#[test]
fn interface_implementations_cannot_change_whether_receiver_is_consumed() {
    for (required, implemented) in [
        ("consuming ", ""),
        ("consuming ", "writable "),
        ("", "consuming "),
        ("writable ", "consuming "),
    ] {
        assert_error(
            &format!(
                "interface Finisher {{ {required}function finish(): int; }}\n\
                 class Resource implements Finisher {{ {implemented}function finish(): int {{ return 42; }} }}"
            ),
            "E0755",
        );
    }
}

#[test]
fn overrides_cannot_change_whether_receiver_is_consumed() {
    for (parent, child) in [
        ("consuming ", ""),
        ("consuming ", "writable "),
        ("", "consuming "),
        ("writable ", "consuming "),
    ] {
        assert_error(
            &format!(
                "open class Base {{ open {parent}function finish(): int {{ return 1; }} }}\n\
                 class Child extends Base {{ override {child}function finish(): int {{ return 2; }} }}"
            ),
            "E0729",
        );
    }
}

const SIMPLE_CALL: &str = r#"
class Resource { consuming function finish(): int { return 42; } }
function main(): void {
    let $resource = new Resource();
    let $answer = $resource->finish();
    echo $answer . "\n";
}
"#;

fn simple_call_mir() -> doriac::mir::Program {
    doriac::lower_source_to_mir("consuming-receiver.doria", SIMPLE_CALL)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
}

fn consuming_method_mut(mir: &mut doriac::mir::Program) -> &mut doriac::mir::Function {
    mir.functions
        .iter_mut()
        .find(|function| function.receiver_mode == Some(doriac::mir::ReceiverMode::Consuming))
        .expect("consuming method")
}

fn direct_receiver_argument_mut(
    mir: &mut doriac::mir::Program,
) -> &mut doriac::mir::ClassExpression {
    use doriac::mir::{IntegerExpression, Rvalue, Statement, ValueExpression};
    let callee = consuming_method_mut(mir).id;
    let main = mir
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    for block in &mut main.blocks {
        for statement in &mut block.statements {
            if let Statement::AssignLocal {
                value:
                    Rvalue::Value(ValueExpression::Integer(IntegerExpression::Call {
                        function,
                        args,
                        ..
                    })),
                ..
            } = statement
            {
                if *function == callee {
                    let Rvalue::Class(receiver) = &mut args[0] else {
                        panic!("class receiver");
                    };
                    return receiver;
                }
            }
        }
    }
    panic!("consuming scalar call not found");
}

fn assert_malformed(mir: &doriac::mir::Program, message: &str) {
    let error = doriac::mir_validation::validate_program(mir)
        .expect_err("corrupted ownership must fail shared validation");
    assert!(error.message.contains(message), "{error:?}\n{mir}");
    assert!(doriac::codegen_cranelift::lower_mir_to_object(mir).is_err());
    #[cfg(feature = "llvm-backend")]
    assert!(doriac::codegen_llvm::lower_mir_to_object(mir).is_err());
}

#[test]
fn malformed_consuming_method_cannot_borrow_or_mutably_borrow_its_receiver() {
    for corruption in 0..3 {
        let mut mir = simple_call_mir();
        let function = consuming_method_mut(&mut mir);
        let receiver = function.params[0];
        match corruption {
            0 => function.locals[receiver.0].owned = false,
            1 => function.parameter_modes[0] = doriac::mir::FunctionParameterMode::Readonly,
            2 => function.locals[receiver.0].writable = true,
            _ => unreachable!(),
        }
        assert_malformed(&mir, "has an invalid receiver parameter");
    }
}

#[test]
fn malformed_consuming_call_must_transfer_its_receiver() {
    let mut mir = simple_call_mir();
    let doriac::mir::ClassExpression::Local { transfer, .. } =
        direct_receiver_argument_mut(&mut mir)
    else {
        panic!("receiver must be an owning local");
    };
    assert!(*transfer);
    *transfer = false;
    assert_malformed(
        &mir,
        "consuming method receiver receives borrowed class local",
    );
}

#[test]
fn malformed_consuming_call_cannot_claim_transfer_of_a_borrowed_local() {
    use doriac::mir::{ClassExpression, IntegerExpression, Rvalue, Statement, ValueExpression};
    let mut mir = doriac::lower_source_to_mir(
        "consuming-receiver.doria",
        r#"
class Resource {
    function inspect(): int { return 42; }
    consuming function finish(): int { return 42; }
}
function inspect(Resource $borrowed): int {
    let $answer = $borrowed->inspect();
    return $answer;
}
function main(): void {
    let $resource = new Resource();
    echo inspect($resource) . "\n";
    let $answer = (new Resource())->finish();
    echo $answer . "\n";
}
"#,
    )
    .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    doriac::mir_validation::validate_program(&mir).expect("ordinary borrowed call is valid");
    let consuming = consuming_method_mut(&mut mir).id;
    let inspect = mir
        .functions
        .iter_mut()
        .find(|function| function.name == "inspect")
        .unwrap();
    let mut changed = false;
    for block in &mut inspect.blocks {
        for statement in &mut block.statements {
            if let Statement::AssignLocal {
                value:
                    Rvalue::Value(ValueExpression::Integer(IntegerExpression::Call {
                        function,
                        args,
                        ..
                    })),
                ..
            } = statement
            {
                let Rvalue::Class(ClassExpression::Local {
                    local, transfer, ..
                }) = &mut args[0]
                else {
                    panic!("borrowed call receiver");
                };
                assert!(!inspect.locals[local.0].owned);
                assert!(!*transfer);
                *function = consuming;
                *transfer = true;
                changed = true;
                break;
            }
        }
        if changed {
            break;
        }
    }
    assert!(changed, "borrowed receiver call not found");
    assert_malformed(&mir, "class rvalue transfers borrowed local");
}

#[test]
fn malformed_checked_consuming_call_must_transfer_its_receiver() {
    use doriac::mir::{ClassExpression, Rvalue, Terminator};
    let mut mir = doriac::lower_source_to_mir("consuming-receiver.doria", CHECKED)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let finish = mir
        .functions
        .iter()
        .find(|function| {
            function.receiver_mode == Some(doriac::mir::ReceiverMode::Consuming)
                && function
                    .method
                    .as_ref()
                    .is_some_and(|method| method.name == "finish")
        })
        .unwrap()
        .id;
    let mut changed = false;
    for function in &mut mir.functions {
        for block in &mut function.blocks {
            if let Terminator::CheckedCall { function, args, .. } = &mut block.terminator {
                if *function == finish {
                    let Rvalue::Class(ClassExpression::Local { transfer, .. }) = &mut args[0]
                    else {
                        panic!("checked consuming receiver must be a staged local");
                    };
                    assert!(*transfer);
                    *transfer = false;
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            break;
        }
    }
    assert!(changed, "checked consuming call not found");
    assert_malformed(
        &mir,
        "consuming method receiver receives borrowed class local",
    );
}

#[test]
fn malformed_erased_consuming_call_must_transfer_its_dispatch_receiver() {
    use doriac::mir::{IndirectCallee, InterfaceValue, Rvalue, Terminator};
    let mut mir = doriac::lower_source_to_mir("consuming-receiver.doria", DISPATCH)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let erased = mir
        .functions
        .iter_mut()
        .find(|function| function.name == "erased")
        .unwrap();
    let mut changed = false;
    for block in &mut erased.blocks {
        let args = match &mut block.terminator {
            Terminator::IndirectCall {
                callee: IndirectCallee::InterfaceMethod { .. },
                args,
                ..
            }
            | Terminator::CheckedIndirectCall {
                callee: IndirectCallee::InterfaceMethod { .. },
                args,
                ..
            } => args,
            _ => continue,
        };
        let Rvalue::Interface(receiver) = &mut args[0] else {
            panic!("interface receiver");
        };
        let InterfaceValue::Local { transfer, .. } = &mut receiver.value else {
            panic!("staged interface receiver");
        };
        assert!(*transfer);
        *transfer = false;
        changed = true;
        break;
    }
    assert!(changed, "erased consuming call not found");
    assert_malformed(
        &mir,
        "interface call argument zero must use its dispatch receiver ownership mode",
    );
}

#[test]
fn explicit_take_callback_parameters_and_function_once_remain_distinct() {
    assert_execution(
        r#"
class Token {
    function __construct(int $value) {}
    function __destruct() {
        try { echo "drop token\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function repeatable(take function(): int $callback): int { return $callback(); }
function consumeOnce(take function once(): Token $callback): Token { return $callback(); }
function main(): void {
    let $number = repeatable(fn() => 42);
    echo "number {$number}\n";
    let $token = new Token(7);
    function once(): Token $callback = fn() with (take $token) => $token;
    let $returned = consumeOnce($callback);
    echo "returned {$returned->value}\n";
}
"#,
        "number 42\nreturned 7\ndrop token\n",
    );
}

#[test]
fn consuming_receiver_cannot_return_a_borrow_of_its_owned_self() {
    assert_error(
        r#"
class Resource {
    function view(): self { return $this; }
    consuming function escape(): self { return $this->view(); }
}
"#,
        "E0478",
    );
}

#[test]
fn consuming_receiver_cannot_escape_through_a_borrowing_closure() {
    assert_error(
        r#"
class Resource {
    int $value = 42;
    consuming function escape(): function(): int {
        return fn() with ($this) => $this->value;
    }
}
"#,
        "E0658",
    );
}

#[test]
fn consuming_receiver_can_transfer_into_a_once_callback_environment() {
    assert_execution(
        r#"
class Resource {
    function __construct(int $value) {}
    function __destruct() {
        try { echo "drop resource\n"; } catch (Doria\Std\Io\IoError) {}
    }
    consuming function intoCallback(): function once(): int {
        return fn() with (take $this) => $this->value;
    }
}
function main(): void {
    let $resource = new Resource(42);
    let $callback = $resource->intoCallback();
    echo "callback live\n";
    let $answer = $callback();
    echo "answer {$answer}\n";
}
"#,
        "callback live\ndrop resource\nanswer 42\n",
    );
}

#[test]
fn consuming_override_can_transfer_to_parent_without_losing_dynamic_cleanup() {
    assert_execution(
        r#"
open class Base {
    open consuming function finish(): int {
        echo "parent finish\n";
        return 42;
    }
    function __destruct() {
        try { echo "drop base\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
class Child extends Base {
    override consuming function finish(): int {
        echo "child finish\n";
        return parent::finish();
    }
    function __destruct() {
        try { echo "drop child\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function main(): void {
    let $answer = (new Child())->finish();
    echo "answer {$answer}\n";
}
"#,
        "child finish\nparent finish\ndrop child\ndrop base\nanswer 42\n",
    );
}

#[test]
fn nullable_consuming_calls_transfer_present_receiver_and_skip_null_arguments() {
    assert_execution(
        r#"
class Resource {
    function __construct(string $label) { echo "new {$this->label}\n"; }
    consuming function finish(int $value): int {
        echo "finish {$this->label} {$value}\n";
        return $value;
    }
    function __destruct() {
        try { echo "drop {$this->label}\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function make(bool $present): ?Resource {
    if ($present) { return new Resource("present"); }
    return null;
}
function argument(): int { echo "argument\n"; return 42; }
function main(): void {
    ?Resource $present = make(true);
    ?Resource $absent = make(false);
    let $first = $present?->finish(argument());
    let $second = $absent?->finish(argument());
    echo "results {$first ?? 0} {$second ?? 0}\n";
}
"#,
        "new present\nargument\nfinish present 42\ndrop present\nresults 42 0\n",
    );
}

#[test]
fn consuming_take_moves_one_field_and_destroys_the_rest_of_the_receiver() {
    // The taken source survives; the remaining buffer is destroyed at the take.
    assert_execution(
        include_str!("fixtures/consuming_field_extraction.doria"),
        "7;0:1;1:2\n",
    );
    // Strings, Move payload enums and Copy fields leave vacated slots behind.
    assert_execution(
        include_str!("fixtures/consuming_field_extraction_layouts.doria"),
        "alpha;2;3;3;5;4\n",
    );
    // The take consumes its receiver: the class's own destructor does not run
    // for it, while an ordinary drop of another instance still runs it.
    assert_execution(
        include_str!("fixtures/consuming_field_extraction_destructor.doria"),
        "0:1:0;1:2:1\n",
    );
}

#[test]
fn consuming_take_requires_a_closed_owner_without_ancestor_destructors_and_one_extraction() {
    let declarations = "class Source { function __construct(int $id) {} }\n\
        class Counter { function __construct(writable int $value) {} }\n";
    for (class, code) in [
        ("class A { function __construct(take Source $source) {} function get(): Source { return take $this->source; } }", "E0780"),
                ("open class C { function __construct(take Source $source) {} consuming function get(): Source { return take $this->source; } }", "E0780"),
        ("class D { function __construct(borrow writable Counter $counter) {} consuming function get(): Counter { return take $this->counter; } }", "E0780"),
        ("class G { function __construct(take Source $a) {} consuming function get(take G $other): Source { return take $other->a; } }", "E0780"),
        ("open class P { function __destruct(): void {} } class H extends P { function __construct(take Source $a) {} consuming function get(): Source { return take $this->a; } }", "E0780"),
        // Extraction gives away the receiver; any later use is a use after move.
        ("class E { function __construct(take Source $a, take Source $b) {} consuming function get(): Source { let $x = take $this->a; return take $this->b; } }", "E0470"),
        ("class F { function __construct(take Source $a) {} consuming function get(): Source { let $x = take $this->a; echo $this->a->id; return $x; } }", "E0470"),
    ] {
        assert_error(&format!("{declarations}{class}\nfunction main(): void {{}}"), code);
    }
}
