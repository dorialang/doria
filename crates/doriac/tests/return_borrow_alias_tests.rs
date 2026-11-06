#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::ast::Item;
use doriac::diagnostics::DiagnosticSeverity;
use doriac::semantics::SemanticAnalysis;
use doriac::symbols::{BindingKind, BindingOwnership, BorrowSource, ReturnBorrow};

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

const DECLARATIONS: &str = r#"
class Child {
    int $value = 42;
    function selfRef(): self { return $this; }
    function make(): function(): int { return fn() with ($this) => $this->value; }
}
class Holder {
    Child $child = new Child();
    function childRef(): Child { return $this->child; }
}
function childFrom(bool $ignored, Holder $owner): Child { return $owner->childRef(); }
"#;

const WRITABLE_DECLARATIONS: &str = r#"
interface Counted { writable function bump(): void; function getValue(): int; }
open class Counter implements Counted {
    writable int $value = 0;
    writable function selfRef(): self { return $this; }
    Counter $current { writable borrowed get => $this->selfRef(); }
    writable function bump(): void { $this->value++; }
    function getValue(): int { return $this->value; }
}
class Holder {
    writable Counter $stored = new Counter();
    Counter $current { writable borrowed get => $this->stored; }
    writable function(): int $storedCallback = fn() => 0;
    function(): int $callback { writable borrowed get => $this->storedCallback; }
}
function lend(writable Counter $counter): Counter { return $counter; }
function peek(Counter $counter): Counter { return $counter; }
function observe(Counter $counter): void {}
function checked(writable Counter $counter): ?Counter throws Error { return $counter; }
function generic<T>(writable T $value): T { return $value; }
function consume(take Counter $counter): void {}
function lendCounted(writable Counted $counter): Counted { return $counter; }
function lendList(writable List<int> $values): List<int> { return $values; }
function lendDeque(writable Deque<int> $values): Deque<int> { return $values; }
"#;

#[test]
fn readonly_collection_element_loans_preserve_storage_and_cleanup() {
    let declarations = r#"
class Entry implements Comparable<Entry>, Equatable<Entry>, Hashable, Cloneable {
    function __construct(int $value) {}
    function compare(Entry $other): Ordering {
        if ($this->value < $other->value) { return Ordering::Less; }
        if ($this->value > $other->value) { return Ordering::Greater; }
        return Ordering::Equal;
    }
    function equals(Entry $other): bool { return $this->value == $other->value; }
    function hash(): uint64 { return 1; }
    function clone(): self { return new Entry($this->value); }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function retain(?Entry $entry): function(): int {
    return function(): int with ($entry) { return $entry?->value ?? -1; };
}
"#;
    for (ty, initializer, insertion, selection, missing) in [
        (
            "List<Entry>",
            "[]",
            "$source->add(new Entry(7));",
            "$source->first",
            "$empty->last",
        ),
        (
            "Dictionary<string, Entry>",
            "[]",
            "$source->set(\"key\", new Entry(7));",
            "$source->get(\"key\")",
            "$source->get(\"missing\")",
        ),
        (
            "SortedDictionary<string, Entry>",
            "SortedDictionary::from([])",
            "$source->set(\"key\", new Entry(7));",
            "$source->get(\"key\")",
            "$source->get(\"missing\")",
        ),
        (
            "Set<Entry>",
            "Set::from([])",
            "$source->add(new Entry(7));",
            "$source->first",
            "$empty->last",
        ),
        (
            "SortedSet<Entry>",
            "SortedSet::from([])",
            "$source->add(new Entry(7));",
            "$source->last",
            "$empty->first",
        ),
        (
            "PriorityQueue<Entry>",
            "PriorityQueue::from([])",
            "$source->push(new Entry(7));",
            "$source->peek",
            "$empty->peek",
        ),
        (
            "Deque<Entry>",
            "Deque::from([])",
            "$source->pushBack(new Entry(7));",
            "$source->peekFront",
            "$empty->peekBack",
        ),
    ] {
        let source = format!("{declarations} function main(): void {{ writable {ty} $source = {initializer}; {ty} $empty = {initializer}; {insertion} let $present = retain({selection}); let $missing = retain({missing}); echo \"{{$present()}} {{$missing()}};\"; }}");
        assert_borrow_execution(&source, "7 -1;drop7;");
    }
}

#[test]
fn inferred_writable_borrowed_locals_preserve_permission_and_nonownership() {
    for body in [
        "let writable $view = lend($owner); $view->bump();",
        "let writable $view = $owner->selfRef(); $view->bump();",
        "let writable $view = generic($owner); $view->bump();",
        "let writable $view = checked($owner); $view?->bump();",
        "let $select = function (writable Counter $counter): Counter { return $counter; }; let writable $view = $select($owner); $view->bump();",
        "let writable $view = lend($owner); let writable $alias = $view; $alias->bump(); $view->bump();",
        "let writable $view = lend($owner); let $alias = $view; echo $alias->value; $view->bump();",
    ] {
        let source = format!(
            "{WRITABLE_DECLARATIONS} function main(): void throws Error {{ let writable $owner = new Counter(); {body} echo $owner->value; }}"
        );
        let (_, analysis) = doriac::analyze_source_for_ide("writable-local.doria", &source).unwrap();
        assert!(analysis.diagnostics.is_empty(), "{body}\n{:#?}", analysis.diagnostics);
        assert_binding_ownership(&analysis, "view", BindingKind::Local, BindingOwnership::WritableBorrow);
        let program = doriac::lower_source_to_mir("writable-local.doria", &source)
            .unwrap_or_else(|diagnostics| panic!("{body}\n{diagnostics:#?}"));
        doriac::mir_validation::validate_program(&program).expect("proved writable local MIR");
        let output = doriac::mir_interpreter::interpret(&program).expect("borrowed local execution");
        assert_eq!(output.exit_status, 0, "{body}");
        assert_eq!(output.stderr, b"", "{body}");
        assert_eq!(output.stdout, if body.contains("echo $alias") { b"01".as_slice() } else if body.contains("$alias->bump") { b"2".as_slice() } else { b"1".as_slice() }, "{body}");
    }
    let source = format!(
        "{WRITABLE_DECLARATIONS} function main(): void {{ let writable $owner = new Counter(); let writable $view = $owner->current; $view->bump(); echo $owner->value; }}"
    );
    let program = doriac::lower_source_to_mir("writable-getter-local.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&program).unwrap().stdout,
        b"1"
    );
}

#[test]
fn writable_borrowed_locals_cannot_upgrade_readonly_or_bypass_live_loans() {
    for (body, code) in [
        ("let writable $view = peek($owner); $view->bump();", "E0478"),
        ("let writable $holder = new Holder(); let writable $view = $holder->current; $view->bump();", "E0478"),
        ("let $view = lend($owner); let writable $alias = $view; $alias->bump();", "E0478"),
        ("let writable $view = lend($owner); echo $owner->value; $view->bump();", "E0477"),
        ("let writable $view = lend($owner); $owner->bump(); $view->bump();", "E0477"),
        ("let writable $view = lend($owner); consume($owner); $view->bump();", "E0477"),
        ("let writable $view = lend($owner); let writable $other = lend($owner); $view->bump(); $other->bump();", "E0477"),
        ("let writable $view = lend($owner); let $other = peek($owner); $view->bump(); echo $other->value;", "E0477"),
        ("let writable $view = lend($owner); let writable $alias = $view; echo $view->value; $alias->bump();", "E0477"),
        ("let writable $view = lend($owner); let $read = fn() with ($owner) => $owner->value; $view->bump(); echo $read();", "E0654"),
        ("Counter $view = lend($owner); echo $view->value;", "E0478"),
        ("let writable $view = lend(new Counter()); $view->bump();", "E0478"),
        ("let writable $run = function(): void with (writable $owner) { consume($owner); }; $run();", "E0474"),
    ] {
        let source = format!("{WRITABLE_DECLARATIONS} function main(): void {{ let writable $owner = new Counter(); {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("writable-local-rejection.doria", &source).unwrap();
        assert_error(&analysis, code, &source);
    }
}

#[test]
fn writable_borrowed_aliases_follow_replacement_of_original_storage() {
    for (setup, replacement, observation, expected) in [
        (
            "let writable $owner = new Counter(); let writable $view = lend($owner);",
            "let $replace = function(writable Counter $counter): void { $counter = new Counter(); $counter->value = 7; }; $replace($alias);",
            "$view->bump(); echo $view->value;",
            "8",
        ),
        (
            "let writable $owner = new Counter(); let writable $view = $owner->current;",
            "let $replace = function(writable Counter $counter): void { $counter = new Counter(); $counter->value = 7; }; $replace($alias);",
            "$view->bump(); echo $view->value;",
            "8",
        ),
        (
            "writable Counted $owner = new Counter(); let writable $view = lendCounted($owner);",
            "let $replace = function(writable Counted $counter): void { let writable $next = new Counter(); $next->value = 7; $counter = $next; }; $replace($alias);",
            "$view->bump(); echo $view->getValue();",
            "8",
        ),
        (
            "let writable $owner = new Counter(); let writable $view = checked($owner);",
            "let $replace = function(writable ?Counter $counter): void { let writable $next = new Counter(); $next->value = 7; $counter = $next; }; $replace($alias);",
            "$view?->bump(); echo ($view?->getValue() ?? 0);",
            "8",
        ),
        (
            "writable List<int> $owner = [0]; let writable $view = lendList($owner);",
            "let $replace = function(writable List<int> $values): void { $values = [7]; }; $replace($alias);",
            "$view->add(8); echo $view[0]; echo $view[1];",
            "78",
        ),
        (
            "writable Deque<int> $owner = Deque::from([0]); let writable $view = lendDeque($owner);",
            "let $replace = function(writable Deque<int> $values): void { $values = Deque::from([7]); }; $replace($alias);",
            "$view->pushBack(8); echo ($view->popFront() ?? 0); echo ($view->popFront() ?? 0);",
            "78",
        ),
        (
            "let writable $owner = new Counter(); { let writable $run = function(): void with (writable $owner) { let writable $view = lend($owner);",
            "let $replace = function(writable Counter $counter): void { $counter = new Counter(); $counter->value = 7; }; $replace($alias);",
            "$view->bump(); echo $view->value; }; $run(); } echo $owner->value;",
            "88",
        ),
        (
            "let writable $owner = new Counter(); { let writable $run = function(): void with (writable $owner) { $owner = new Counter(); $owner->value = 7; let writable $view = lend($owner);",
            "let $replace = function(writable Counter $counter): void { let $inner = function(writable Counter $counter): void { $counter = new Counter(); $counter->value = 7; }; $inner($counter); $counter->bump(); }; $replace($alias);",
            "$view->bump(); echo $view->value; echo $owner->value; }; $run(); $run(); } echo $owner->value;",
            "99999",
        ),
    ] {
        let source = format!(
            "{WRITABLE_DECLARATIONS} function main(): void throws Error {{ {setup} let writable $alias = $view; {replacement} {observation} }}"
        );
        assert_borrow_execution(&source, expected);
    }
}

#[test]
fn absent_writable_nullable_return_keeps_its_original_storage_home() {
    // Existing closure writeback must retain writable storage even when empty.
    assert_borrow_execution(
        r#"
class Counter {
    writable int $value = 0;
    writable function bump(): void { $this->value++; }
}
function lendNullable(writable ?Counter $slot): ?Counter { return $slot; }
function main(): void {
    writable ?Counter $slot = null;
    {
        let writable $view = lendNullable($slot);
        let $install = function(writable ?Counter $slot): void { $slot = new Counter(); };
        $install($view);
        $view?->bump();
    }
    if (($slot?->value ?? -1) != 1) { panic("writable nullable home detached"); }
    echo "nullable storage preserved\n";
}
"#,
        "nullable storage preserved\n",
    );
}

const CONDITIONAL_BORROW_DECLARATIONS: &str = r#"
function nullableRead(bool $present, Counter $value): ?Counter {
    if ($present) { return $value; }
    return null;
}
function nullableWrite(bool $present, writable Counter $value): ?Counter {
    if ($present) { return $value; }
    return null;
}
"#;

#[test]
fn coalesced_borrowed_locals_preserve_alternative_owners_and_captures() {
    assert_borrow_execution(
        &format!(
            r#"
{WRITABLE_DECLARATIONS}
{CONDITIONAL_BORROW_DECLARATIONS}
function exercise(bool $present): void {{
    let writable $left = new Counter(); $left->value = 1;
    let writable $right = new Counter(); $right->value = 2;
    {{
        let $view = nullableRead($present, $left) ?? peek($right);
        let $nested = nullableRead(false, $right) ?? $view;
        let $read = fn() with ($nested) => $nested->value;
        echo $read();
    }}
    $left->bump(); $right->bump();
    echo $left->value; echo $right->value;
}}
function main(): void {{ exercise(true); exercise(false); }}
"#
        ),
        "123223",
    );
}

#[test]
fn coalesced_writable_borrows_preserve_the_selected_storage_home() {
    assert_borrow_execution(
        &format!(
            r#"
{WRITABLE_DECLARATIONS}
{CONDITIONAL_BORROW_DECLARATIONS}
function exercise(bool $present): void {{
    let writable $left = new Counter(); $left->value = 1;
    let writable $right = new Counter(); $right->value = 2;
    {{
        let writable $view = nullableWrite($present, $left) ?? lend($right);
        let writable $alias = $view;
        let $replace = function(writable Counter $counter): void {{
            $counter = new Counter(); $counter->value = 7;
        }};
        $replace($alias);
        $view->bump();
    }}
    echo $left->value; echo $right->value;
}}
function main(): void {{ exercise(true); exercise(false); }}
"#
        ),
        "8218",
    );
}

#[test]
fn coalesced_borrowed_locals_protect_every_possible_owner() {
    for (body, code) in [
        ("let $view = nullableRead(true, $left) ?? peek($right); $left->bump(); echo $view->value;", "E0477"),
        ("let $view = nullableRead(true, $left) ?? peek($right); $right->bump(); echo $view->value;", "E0477"),
        ("let $view = nullableRead(false, $left) ?? peek($right); consume($left); echo $view->value;", "E0477"),
        ("let $view = nullableRead(false, $left) ?? peek($right); consume($right); echo $view->value;", "E0477"),
        ("let writable $view = nullableWrite(true, $left) ?? lend($right); echo $right->value; $view->bump();", "E0477"),
        ("let writable $view = nullableRead(true, $left) ?? lend($right); $view->bump();", "E0478"),
        ("let $view = nullableRead(true, new Counter()) ?? peek($right); echo $view->value;", "E0478"),
        ("let $view = nullableRead(true, $left) ?? peek(new Counter()); echo $view->value;", "E0478"),
        ("let $view = nullableRead(true, $left) ?? peek($right); let $run = fn() with (take $view) => $view->value; echo $run();", "E0645"),
    ] {
        let source = format!("{WRITABLE_DECLARATIONS} {CONDITIONAL_BORROW_DECLARATIONS} function main(): void {{ let writable $left = new Counter(); let writable $right = new Counter(); {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("conditional-borrow-rejection.doria", &source).unwrap();
        assert_error(&analysis, code, &source);
    }
}

#[test]
fn coalesced_getter_borrows_preserve_null_and_known_branch_selection() {
    assert_borrow_execution(
        &format!(
            r#"
{WRITABLE_DECLARATIONS}
class OptionalCounter {{
    internal ?Counter $stored;
    function __construct(parameter take ?Counter $stored) {{ $this->stored = $stored; }}
    ?Counter $current {{ borrowed get => $this->stored; }}
}}
function main(): void {{
    let $full = new OptionalCounter(new Counter());
    let $empty = new OptionalCounter(null);
    let $same = $full->current ?? $full->current;
    let $fallback = $empty->current ?? $full->current;
    let $absent = $empty->current ?? null;
    let $read = fn() with ($absent) => $absent?->value ?? 9;
    echo $read();
    if ($same != null) {{
        let $known = $same ?? peek(new Counter());
        echo $known->value;
    }}
    echo ($fallback?->value ?? 7);
}}
"#
        ),
        "900",
    );
}

#[test]
fn coalesced_collection_borrows_preserve_selected_replacement_storage() {
    assert_borrow_execution(
        r#"
function optionalList(bool $present, writable List<int> $values): ?List<int> {
    if ($present) { return $values; }
    return null;
}
function listView(writable List<int> $values): List<int> { return $values; }
function exercise(bool $present): void {
    writable List<int> $left = [1];
    writable List<int> $right = [2];
    {
        let writable $view = optionalList($present, $left) ?? listView($right);
        let $replace = function(writable List<int> $values): void { $values = [7]; };
        $replace($view);
    }
    echo $left[0]; echo $right[0];
}
function main(): void { exercise(true); exercise(false); }
"#,
        "7217",
    );
}

#[test]
fn coalesced_borrowed_captures_return_through_one_parameter_source() {
    assert_borrow_execution(
        &format!(
            r#"
{WRITABLE_DECLARATIONS}
{CONDITIONAL_BORROW_DECLARATIONS}
function make(bool $present, Counter $owner): function(): int {{
    let $view = nullableRead($present, $owner) ?? peek($owner);
    return fn() with ($view) => $view->value;
}}
function main(): void {{
    let writable $owner = new Counter(); $owner->value = 3;
    {{
        let $first = make(true, $owner);
        let $second = make(false, $owner);
        echo $first(); echo $second();
    }}
}}
"#
        ),
        "33",
    );
}

#[test]
fn coalesced_borrowed_captures_reject_unrelated_and_local_escape_sources() {
    for (parameters, body, code) in [
        ("bool $present, Counter $left, Counter $right", "let $view = nullableRead($present, $left) ?? peek($right); return fn() with ($view) => $view->value;", "E0659"),
        ("bool $present", "let $left = new Counter(); let $right = new Counter(); let $view = nullableRead($present, $left) ?? peek($right); return fn() with ($view) => $view->value;", "E0658"),
    ] {
        let source = format!("{WRITABLE_DECLARATIONS} {CONDITIONAL_BORROW_DECLARATIONS} function make({parameters}): function(): int {{ {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("conditional-capture-escape.doria", &source).unwrap();
        assert_error(&analysis, code, &source);
    }
}

#[test]
fn coalesced_borrowed_sources_evaluate_once_and_keep_the_fallback_lazy() {
    assert_borrow_execution(
        &format!(
            r#"
{WRITABLE_DECLARATIONS}
class BorrowCalls {{
    static writable int $left = 0;
    static writable int $right = 0;
    static function select(bool $present, Counter $owner): ?Counter {{
        self::left++;
        if ($present) {{ return $owner; }}
        return null;
    }}
    static function fallback(Counter $owner): Counter {{ self::right++; return $owner; }}
}}
function main(): void {{
    let $left = new Counter(); let $right = new Counter();
    let $present = BorrowCalls::select(true, $left) ?? BorrowCalls::fallback($right);
    let $absent = BorrowCalls::select(false, $left) ?? BorrowCalls::fallback($right);
    echo $present->value; echo $absent->value;
    if (BorrowCalls::left != 2 or BorrowCalls::right != 1) {{ panic("coalesce repeated a source"); }}
}}
"#
        ),
        "00",
    );
}

#[test]
fn writable_captured_sources_preserve_checked_cleanup() {
    assert_borrow_execution(
        &format!(
            r#"
{WRITABLE_DECLARATIONS}
class Tracked extends Counter {{
    function __construct(int $id) {{}}
    function __destruct() {{ try {{ echo "drop{{$this->id}};"; }} catch (Error) {{}} }}
}}
class Failure implements Error {{ function __construct(string $message) {{}} }}
function main(): void throws Error {{
    writable Counter $owner = new Tracked(1);
    {{
        let writable $replace = function(): void with (writable $owner) {{
            $owner = new Tracked(2);
            $owner->value = 7;
            let writable $view = lend($owner);
            $view->bump();
            echo $view->value;
            throw new Failure("changed");
        }};
        try {{ $replace(); }} catch (Failure) {{ echo "caught;"; }}
    }}
    echo $owner->value;
}}
"#
        ),
        "drop1;8caught;8drop2;",
    );
}

#[test]
fn property_updates_preserve_transitive_receiver_borrow_homes() {
    assert_borrow_execution(
        &format!(
            r#"
{WRITABLE_DECLARATIONS}
class HookCounter extends Counter {{
    writable int $reading {{
        writable get {{ let $view = $this->selfRef(); return $view->value; }}
        set(int $value) {{ let writable $view = $this->selfRef(); $view->value = $value; }}
    }}
}}
function main(): void throws Error {{
    let writable $owner = new HookCounter();
    $owner->reading = 4;
    $owner->reading += 2;
    echo $owner->reading;
}}
"#
        ),
        "6",
    );
}

fn assert_borrow_execution(source: &str, expected: &str) {
    let hir = doriac::lower_source("writable-alias-replacement.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let program = doriac::mir_lowering::lower_program(&hir)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, expected.as_bytes(), "{source}");
    native_execution::assert_native_execution(
        &program,
        doriac::backend::NativeProfile::Fast,
        expected,
    );
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        &program,
        doriac::backend::NativeProfile::Release,
        expected,
    );
    let php = doriac::codegen_php::generate(&hir, Some(&program)).unwrap();
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let result = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stderr, b"", "{source}");
    assert_eq!(result.stdout, expected.as_bytes(), "{source}");
}

#[test]
fn mir_rejects_forged_writable_views_and_overlapping_owner_reads() {
    for (body, owner_name) in [
        ("let writable $owner = new Counter(); let writable $view = lend($owner); $view->bump(); observe($owner);", "owner"),
        ("let $run = function(writable Counter $counter): void { $counter = new Counter(); let writable $view = lend($counter); $view->bump(); observe($counter); }; let writable $owner = new Counter(); $run($owner);", "counter"),
    ] {
    let source = format!(
        "{WRITABLE_DECLARATIONS} function main(): void {{ {body} }}"
    );
    let mut program = doriac::lower_source_to_mir("forged-exclusive-view.doria", source).unwrap();
    let observer = program
        .functions
        .iter()
        .find(|function| function.name == "observe")
        .unwrap()
        .id;
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.locals.iter().any(|local| local.name == "view"))
        .unwrap();
    let view = function
        .locals
        .iter()
        .find(|local| local.name == "view")
        .unwrap()
        .id;
    let owner = function
        .locals
        .iter()
        .find(|local| local.name == owner_name)
        .unwrap();
    let doriac::mir::Type::Class(class) = owner.ty else {
        unreachable!();
    };
    let observation = doriac::mir::Statement::CallVoid {
        function: observer,
        args: vec![doriac::mir::Rvalue::Class(
            doriac::mir::ClassExpression::Local {
                class,
                local: owner.id,
                transfer: false,
            },
        )],
        span: Default::default(),
    };
    let block = function
        .blocks
        .iter_mut()
        .find(|block| {
            block.statements.iter().any(|statement|
        matches!(statement, doriac::mir::Statement::AssignLocal { target, .. } if *target == view))
        })
        .unwrap();
    let index = block.statements.iter().position(|statement|
        matches!(statement, doriac::mir::Statement::AssignLocal { target, .. } if *target == view)).unwrap();
    block.statements.insert(index + 1, observation);
    let error = doriac::mir_validation::validate_program(&program).unwrap_err();
    assert!(
        error
            .message
            .contains("source access conflicts with a live borrowed view"),
        "{error:?}"
    );
    }

    let source = format!(
        "{WRITABLE_DECLARATIONS} function main(): void {{ let writable $owner = new Counter(); let $view = peek($owner); echo $view->value; }}"
    );
    let mut program = doriac::lower_source_to_mir("forged-writable-view.doria", source).unwrap();
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    function
        .locals
        .iter_mut()
        .find(|local| local.name == "view")
        .unwrap()
        .writable = true;
    assert!(
        doriac::mir_validation::validate_program(&program).is_err(),
        "writable flag cannot invent a loan permission"
    );
}

fn analyze_capture(
    parameters: &str,
    result: &str,
    body: &str,
) -> (String, SemanticAnalysis, Option<ReturnBorrow>) {
    let source = format!("{DECLARATIONS}\nfunction capture({parameters}): {result} {{\n{body}\n}}");
    let (program, analysis) =
        doriac::analyze_source_for_ide("return-borrow-alias.doria", source.clone())
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let capture = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == "capture" => Some(function),
            _ => None,
        })
        .expect("capture declaration");
    let borrow = analysis.info.return_borrows.get(&capture.span).copied();
    (source, analysis, borrow)
}

fn assert_parameter_borrow(parameters: &str, body: &str) {
    let (source, analysis, borrow) = analyze_capture(parameters, "function(): int", body);
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
    assert_eq!(
        borrow,
        Some(ReturnBorrow {
            source: BorrowSource::Parameter(0),
            writable: false,
            kind: doriac::types::ReturnBorrowKind::Retained,
            lifetime: doriac::types::RetainedLifetime::LastUse,
        }),
        "{source}"
    );
}

fn assert_error(analysis: &SemanticAnalysis, code: &str, source: &str) {
    assert!(
        analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error && diagnostic.code == code
        }),
        "expected {code}: {source}\n{:#?}",
        analysis.diagnostics
    );
}

fn assert_binding_ownership(
    analysis: &SemanticAnalysis,
    name: &str,
    kind: BindingKind,
    ownership: BindingOwnership,
) {
    let declarations: Vec<_> = analysis
        .info
        .binding_resolution
        .declarations_by_id
        .values()
        .filter(|declaration| declaration.name == name && declaration.kind == kind)
        .collect();
    assert_eq!(declarations.len(), 1, "expected one {kind:?} named {name}");
    assert_eq!(declarations[0].ownership, ownership, "binding {name}");
}

#[test]
fn canonical_binding_ownership_distinguishes_callback_loans_and_retained_owners() {
    let source = r#"
class Source {
    int $value = 42;
    function __construct(take function(): int $stored) {}
    function(): int $callback { borrowed get => $this->stored; }
    function(): int $fresh { get => fn() with ($this) => $this->value; }
}
function inspect(Source $source, bool $choose): int {
    let $direct = $source->callback;
    let $grouped = (($source->callback));
    let $alias = $direct;
    let $fresh = $source->fresh;
    let $freshAlias = $fresh;
    let $ownedWhen = when ($choose) {
        return $source->fresh;
    } else {
        return $source->fresh;
    };
    return $direct() + $grouped() + $alias() + $freshAlias() + $ownedWhen();
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("callback-binding-ownership.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    // A borrowed existing carrier and a new owned carrier have different
    // cleanup obligations, even when both retain the same receiver.
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| { diagnostic.severity != DiagnosticSeverity::Error }),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
    for name in ["direct", "grouped", "alias"] {
        assert_binding_ownership(
            &analysis,
            name,
            BindingKind::Local,
            BindingOwnership::ReadonlyBorrow,
        );
    }
    for name in ["fresh", "freshAlias", "ownedWhen"] {
        assert_binding_ownership(&analysis, name, BindingKind::Local, BindingOwnership::Owned);
    }
    assert_binding_ownership(
        &analysis,
        "source",
        BindingKind::FunctionParameter,
        BindingOwnership::ReadonlyBorrow,
    );
    assert_binding_ownership(
        &analysis,
        "this",
        BindingKind::ClosureCapture,
        BindingOwnership::ReadonlyBorrow,
    );
}

#[test]
fn canonical_binding_ownership_preserves_ordinary_move_loans_and_grouped_copy_values() {
    let source = format!(
        r#"{DECLARATIONS}
function inspect(Holder $holder): int {{
    let $child = $holder->childRef();
    let $grouped = (($holder->childRef()));
    let $alias = $child;
    let $forwarded = $alias->selfRef();
    let $owned = new Child();
    let $left, $right = 1;
    return $child->value + $grouped->value + $alias->value + $forwarded->value
        + $owned->value + $left + $right;
}}
"#
    );
    let (_, analysis) =
        doriac::analyze_source_for_ide("move-binding-ownership.doria", source.clone())
            .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error),
        "{source}\n{:#?}",
        analysis.diagnostics
    );
    for name in ["child", "grouped", "alias", "forwarded"] {
        assert_binding_ownership(
            &analysis,
            name,
            BindingKind::Local,
            BindingOwnership::ReadonlyBorrow,
        );
    }
    assert_binding_ownership(
        &analysis,
        "owned",
        BindingKind::Local,
        BindingOwnership::Owned,
    );
    for name in ["left", "right"] {
        assert_binding_ownership(
            &analysis,
            name,
            BindingKind::GroupedLocal,
            BindingOwnership::Owned,
        );
    }
    assert_binding_ownership(
        &analysis,
        "holder",
        BindingKind::FunctionParameter,
        BindingOwnership::ReadonlyBorrow,
    );
}

#[test]
fn borrowed_method_result_alias_preserves_the_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder",
        "let $view = $holder->childRef(); return $view->make();",
    );
}

#[test]
fn borrowed_result_call_chains_preserve_the_original_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $view = ($holder->childRef());
let $next = $view->selfRef();
let $last = ($next->selfRef());
return $last->make();
"#,
    );
}

#[test]
fn named_arguments_follow_the_callees_parameter_identity() {
    for acquisition in [
        "childFrom(ignored: true, owner: $holder)",
        "childFrom(owner: $holder, ignored: true)",
    ] {
        assert_parameter_borrow(
            "Holder $holder",
            &format!("let $view = {acquisition}; return $view->make();"),
        );
    }
}

#[test]
fn nested_shadow_does_not_replace_the_outer_alias_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $view = $holder->childRef();
{
    let $view = new Child();
    let $observed = $view->value;
}
return $view->make();
"#,
    );
}

#[test]
fn alias_initializer_resolves_the_outer_binding_before_shadowing() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
{
    let $holder = $holder->childRef();
    return $holder->make();
}
"#,
    );
}

#[test]
fn branch_local_aliases_join_at_the_same_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder, bool $choose",
        r#"
if ($choose) {
    let $view = $holder->childRef();
    return $view->make();
} else {
    let $view = $holder->childRef();
    let $next = $view->selfRef();
    return $next->make();
}
"#,
    );
}

#[test]
fn returning_branch_does_not_leak_its_shadow_into_the_continuation() {
    assert_parameter_borrow(
        "Holder $holder, bool $choose",
        r#"
let $view = $holder->childRef();
if ($choose) {
    let $view = $holder->childRef();
    return $view->make();
}
return $view->make();
"#,
    );
}

#[test]
fn loop_backedges_and_exits_preserve_outer_and_iteration_aliases() {
    assert_parameter_borrow(
        "Holder $holder, bool $repeat, bool $stop",
        r#"
let $view = $holder->childRef();
while ($repeat) {
    let $iteration = $view->selfRef();
    if ($stop) { return $iteration->make(); }
    continue;
}
return $view->make();
"#,
    );
    assert_parameter_borrow(
        "Holder $holder, bool $repeat, bool $stop",
        r#"
let $view = $holder->childRef();
do {
    let $view = $holder->childRef();
    if ($stop) { break; }
    let $observed = $view->value;
} while ($repeat);
return $view->make();
"#,
    );
}

#[test]
fn aliases_of_owned_sources_do_not_acquire_parameter_lifetimes() {
    for (parameters, body) in [
        (
            "",
            "let $holder = new Holder(); let $view = $holder->childRef(); return $view->make();",
        ),
        (
            "take Holder $holder",
            "let $view = $holder->childRef(); return $view->make();",
        ),
        (
            "Holder $holder",
            "{ let $holder = new Holder(); let $view = $holder->childRef(); return $view->make(); }",
        ),
    ] {
        let (source, analysis, borrow) = analyze_capture(parameters, "function(): int", body);
        assert_error(&analysis, "E0658", &source);
        assert_eq!(borrow, None, "{source}");
    }
}

#[test]
fn different_borrowed_parameter_sources_remain_ambiguous() {
    let (source, analysis, borrow) = analyze_capture(
        "Holder $left, Holder $right, bool $choose",
        "Child",
        r#"
if ($choose) {
    let $view = $left->childRef();
    return $view;
}
let $view = $right->childRef();
return $view;
"#,
    );
    assert_error(&analysis, "E0474", &source);
    assert_eq!(borrow, None, "{source}");
}

#[test]
fn owned_and_borrowed_branch_results_do_not_share_an_inferred_lifetime() {
    let (source, analysis, borrow) = analyze_capture(
        "Holder $holder, bool $choose",
        "function(): int",
        r#"
if ($choose) {
    let $view = $holder->childRef();
    return $view->make();
}
let $owner = new Holder();
let $view = $owner->childRef();
return $view->make();
"#,
    );
    assert_error(&analysis, "E0658", &source);
    assert_eq!(borrow, None, "{source}");
}

#[test]
fn borrowed_aliases_do_not_enable_writable_rebinding() {
    let (source, analysis, _) = analyze_capture(
        "Holder $holder",
        "function(): int",
        r#"
let writable $view = $holder->childRef();
$view = new Child();
return $view->make();
"#,
    );
    assert_error(&analysis, "E0478", &source);
}

#[test]
fn direct_owned_property_move_out_remains_rejected() {
    let (source, analysis, _) = analyze_capture(
        "Holder $holder",
        "function(): int",
        "let $view = $holder->child; return $view->make();",
    );
    assert_error(&analysis, "E0472", &source);
}

#[test]
fn anonymous_function_argument_results_preserve_the_original_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $select = function (bool $ignored, Holder $owner): Child {
    let $child = $owner->childRef();
    return $child->selfRef();
};
let $view = $select(true, $holder);
return $view->make();
"#,
    );
}

#[test]
fn arrow_function_argument_results_preserve_the_original_parameter_source() {
    assert_parameter_borrow(
        "Holder $holder",
        r#"
let $select = fn(bool $ignored, Holder $owner) => $owner->childRef();
let $view = $select(true, $holder);
return $view->make();
"#,
    );
}

#[test]
fn indirect_borrowed_results_do_not_extend_temporary_owner_lifetimes() {
    for selector in [
        "function (bool $ignored, Holder $owner): Child { return $owner->childRef(); }",
        "fn(bool $ignored, Holder $owner) => $owner->childRef()",
    ] {
        for (body, code) in [
            ("return $select(true, new Holder())->make();", "E0658"),
            (
                "let $view = $select(true, new Holder()); return $view->make();",
                "E0478",
            ),
        ] {
            let (source, analysis, borrow) = analyze_capture(
                "",
                "function(): int",
                &format!("let $select = {selector};\n{body}"),
            );
            assert_error(&analysis, code, &source);
            assert_eq!(borrow, None, "{source}");
        }
    }
}

fn analyze_nullable_source(body: &str) -> (String, SemanticAnalysis) {
    let source = format!(
        r#"
class Owner {{ int $value = 42; }}
function identity(?Owner $owner): ?Owner {{ return $owner; }}
function captureReadonly(?Owner $owner): function(): bool {{
    return fn() with ($owner) => $owner == null;
}}
function forwardReadonly(?Owner $owner): function(): bool {{ return captureReadonly($owner); }}
function captureWritable(writable ?Owner $owner): function writable(): bool {{
    return function (): bool with (writable $owner) {{ return $owner == null; }};
}}
function result(): function(): bool {{ {body} }}
"#
    );
    let (_, analysis) = doriac::analyze_source_for_ide("nullable-source.doria", source.clone())
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    (source, analysis)
}

#[test]
fn immutable_null_sources_do_not_create_readonly_lifetime_obligations() {
    for body in [
        "return captureReadonly(null);",
        "let $callback = forwardReadonly((null)); return $callback;",
        "let $absent = identity(null); return captureReadonly($absent);",
        "let $absent = identity(null); let $next = $absent; return captureReadonly($next);",
        "let $absent = identity(null); return fn() with ($absent) => $absent == null;",
        "let $identity = fn(?Owner $owner) => $owner; let $absent = $identity(null); return captureReadonly($absent);",
    ] {
        let (source, analysis) = analyze_nullable_source(body);
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != DiagnosticSeverity::Error),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn readonly_absence_does_not_relax_temporary_or_writable_source_requirements() {
    for (body, code) in [
        ("return captureReadonly(new Owner());", "E0658"),
        (
            "let $view = identity(new Owner()); return captureReadonly($view);",
            "E0478",
        ),
        (
            "let writable $callback = captureWritable(null); return fn() => true;",
            "E0204",
        ),
        (
            "writable ?Owner $owner = null; return captureReadonly($owner);",
            "E0658",
        ),
        (
            "?Owner $owner = null; return captureReadonly($owner);",
            "E0658",
        ),
        (
            "let $identity = fn(?Owner $owner) => $owner; let $view = $identity(new Owner()); return captureReadonly($view);",
            "E0478",
        ),
    ] {
        let (source, analysis) = analyze_nullable_source(body);
        assert_error(&analysis, code, &source);
    }
}
