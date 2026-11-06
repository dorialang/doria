use crate::backend::NativeProfile;

fn assert_native_output(source: &str, expected: &str) {
    let program = crate::lower_source_to_mir("conditional-results.doria", source).unwrap();
    let output = crate::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.exit_status, 0);
    assert!(output.runtime_diagnostic.is_none(), "{output:?}");
    crate::native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    crate::native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
}

#[test]
fn conditional_return_borrows_preserve_branch_aliases_and_reject_escaping_owners() {
    let declarations = r#"
class Token { function __construct(int $value) {} }
function lend(Token $value): Token { return $value; }
function selected(Token $value, bool $left): Token {
    return lend(when ($left): Token {
        let $alias = $value;
        return $alias;
    } else {
        let $alias = lend($value);
        return $alias;
    });
}
function matched(Token $value, bool $left): Token {
    return lend(match ($left) { true => $value, false => selected($value, false), });
}
function optional(Token $value, bool $present): ?Token {
    return when ($present): ?Token { return lend($value); } else { return null; };
}
function nested(Token $value, bool $left): Token {
    return when ($left): Token {
        return match ($left) { true => $value, default => selected($value, false), };
    } else { return matched($value, false); } finally {
        let $value = new Token(99);
        if ($value->value != 99) { panic("shadow"); }
    };
}
"#;
    let source = format!(
        r#"{declarations}
function main(): void throws Error {{
    let $owner = new Token(7);
    echo selected($owner, true)->value; echo selected($owner, false)->value;
    echo matched($owner, true)->value; echo matched($owner, false)->value;
    echo nested($owner, true)->value; echo nested($owner, false)->value;
    echo optional($owner, true)?->value ?? 0;
    echo optional($owner, false)?->value ?? 0;
}}
"#
    );
    assert_native_output(&source, "77777770");
    for value in [
        "when ($left): Token { return $first; } else { return $second; }",
        "match ($left) { true => $first, false => $second, }",
        "when ($left): Token { let $owner = new Token(1); return lend($owner); } else { return $first; }",
        "when ($left): Token { return new Token(1); } else { return $first; }",
    ] {
        let source = format!("{declarations} function bad(Token $first, Token $second, bool $left): Token {{ return lend({value}); }} function main(): void {{}}");
        let (_, analysis) = crate::analyze_source_for_ide("conditional-borrow-rejection.doria", &source).unwrap();
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(diagnostic.code, "E0474" | "E0478" | "E0658")), "{source}\n{:?}", analysis.diagnostics);
    }
}

#[test]
fn conditional_borrowed_locals_preserve_selected_owners() {
    let source = r#"
class Token { function __construct(int $value) {} }
function lend(Token $value): Token { return $value; }
function exercise(bool $left): void throws Error {
    let writable $first = new Token(1);
    let writable $second = new Token(2);
    {
        let $view = lend(when ($left): Token {
            let $alias = lend($first);
            return $alias;
        } else { return lend($second); });
        echo $view->value;
    }
    $first = new Token(3);
    $second = new Token(4);
    echo $first->value; echo $second->value;
}
function main(): void throws Error { exercise(true); exercise(false); }
"#;
    assert_native_output(source, "134234");
}

#[test]
fn conditional_borrowed_results_reject_expired_branch_owners() {
    for expression in [
        "when (true): Token { let $local = new Token(1); return lend($local); } else { return $owner; }",
        "given { let $local = new Token(1); true; } when (true): Token { return lend($local); } else { return lend($local); }",
    ] {
        let source = format!(
            "class Token {{ function __construct(int $value) {{}} }}
             function lend(Token $value): Token {{ return $value; }}
             function main(): void throws Error {{ let $owner = new Token(2); echo lend({expression})->value; }}"
        );
        let (_, analysis) = crate::analyze_source_for_ide("conditional-expired-owner.doria", &source).unwrap();
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(diagnostic.code, "E0474" | "E0478" | "E0658")), "{source}\n{:?}", analysis.diagnostics);
    }
}

pub(crate) const PATTERN_RESULT_SOURCE: &str = r#"
class Token { function __construct(int $value) {} }
enum Packet { case First(Token $value); case Second(Token $value); }
enum Envelope { case Wrapped(Packet $packet); }
enum Work { case Run(function(): int $callback); }
function lend(Token $value): Token { return $value; }
function selected(Packet $packet): Token {
    return match ($packet) {
        Packet::First($value) if $value->value > 0 => lend($value),
        Packet::First($value) => lend($value),
        Packet::Second($value) => lend($value),
    };
}
function optional(?Packet $packet): ?Token {
    return match ($packet) {
        null => null,
        Packet::First($value) => lend($value),
        Packet::Second($value) => lend($value),
    };
}
function nested(Envelope $envelope): Token {
    return match ($envelope) { Envelope::Wrapped($packet) => selected($packet), };
}
function extracted(take Packet $packet): Token {
    return match (take $packet) {
        Packet::First($value) => $value,
        Packet::Second($value) => $value,
    };
}
function narrowed(?Token $token): ?Token {
    return match ($token) { null => null, Token $value => lend($value), };
}
function narrowedMixed(mixed $token): ?Token {
    return match ($token) { Token $value => lend($value), default => null, };
}
function callback(take Work $work): function(): int {
    return match (take $work) { Work::Run($run) => $run, };
}
function main(): void throws Error {
    let $first = Packet::First(new Token(1));
    let $second = Packet::Second(new Token(2));
    echo selected($first)->value; echo selected($second)->value;
    echo optional($first)?->value ?? 0; echo optional(null)?->value ?? 0;
    let $envelope = Envelope::Wrapped(Packet::Second(new Token(3)));
    echo nested($envelope)->value;
    echo extracted(Packet::First(new Token(4)))->value;
    ?Packet $maybe = Packet::First(new Token(5));
    echo optional($maybe)?->value ?? 0;
    let $work = Work::Run(fn() => 6);
    let $run = callback($work);
    echo $run();
    let $token = new Token(7);
    echo narrowed($token)?->value ?? 0; echo narrowed(null)?->value ?? 0;
    mixed $boxed = new Token(8);
    echo narrowedMixed($boxed)?->value ?? 0;
    echo narrowedMixed(9)?->value ?? 0;
}
"#;

#[test]
fn conditional_pattern_results_preserve_parameter_owners() {
    let source = PATTERN_RESULT_SOURCE;
    let borrowed_callback = source
        .replace("callback(take Work $work)", "callback(Work $work)")
        .replace("match (take $work)", "match ($work)");
    let (_, analysis) =
        crate::analyze_source_for_ide("pattern-callback-escape.doria", &borrowed_callback).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0657"),
        "a borrowed callback cannot escape its source: {:?}",
        analysis.diagnostics
    );
    assert_native_output(source, "121034567080");
    let mut malformed = crate::lower_source_to_mir("pattern-borrow-proof.doria", source).unwrap();
    let selected = malformed
        .functions
        .iter_mut()
        .find(|function| function.name == "selected")
        .unwrap();
    for block in &mut selected.blocks {
        block.statements.retain(|statement| {
            !matches!(
                statement,
                crate::mir::Statement::BindPayloadEnumFields { .. }
            )
        });
    }
    assert!(
        crate::mir_validation::validate_program(&malformed).is_err(),
        "a returned payload borrow requires independently validated binding provenance"
    );
}

#[test]
fn conditional_closure_returns_preserve_parameter_sources() {
    let source = r#"
class Token { function __construct(int $value) {} }
function lend(Token $value): Token { return $value; }
function main(): void throws Error {
    let $block = function(Token $value, bool $left): Token {
        return lend(when ($left): Token {
            let $alias = $value;
            return $alias;
        } else { return lend($value); } finally {
            let $value = new Token(99);
            if ($value->value != 99) { panic("shadow"); }
        });
    };
    let $arrow = fn(Token $value, bool $left) => lend(match ($left) {
        true => $value, false => lend($value),
    });
    let $nullable = function(Token $value, bool $present): ?Token {
        return when ($present): ?Token { return lend($value); } else { return null; };
    };
    let $owner = new Token(7);
    echo $block($owner, true)->value;
    echo $block($owner, false)->value;
    echo $arrow($owner, true)->value;
    echo $arrow($owner, false)->value;
    echo $nullable($owner, true)?->value ?? 0;
    echo $nullable($owner, false)?->value ?? 0;
}
"#;
    assert_native_output(source, "777770");
}

#[test]
fn conditional_borrows_keep_selected_sources_live_through_finalizers() {
    for action in ["$owner = new Token(9);", "drop($owner);"] {
        let source = format!(
            r#"
class Token {{ function __construct(int $value) {{}} }}
function lend(Token $value): Token {{ return $value; }}
function drop(take Token $value): void {{}}
function main(): void throws Error {{
    let writable $owner = new Token(7);
    echo lend(when (true): Token {{ return $owner; }} else {{ return $owner; }}
        finally {{ {action} }})->value;
}}
"#
        );
        let (_, analysis) =
            crate::analyze_source_for_ide("conditional-finalizer-loan.doria", &source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic.code, "E0471" | "E0477")),
            "{source}\n{:?}",
            analysis.diagnostics
        );
    }
    let source = r#"
class Token { function __construct(int $value) {} }
class Failure implements Error { function __construct(string $message) {} }
function fail(): void throws Failure { throw new Failure("finalizer"); }
function lend(Token $value): Token { return $value; }
function main(): void throws Error {
    let writable $owner = new Token(7);
    echo lend(when (true): Token {
        try { return $owner; } finally { if ($owner->value != 7) { panic("read"); } }
    } else { return $owner; } finally {
        let $owner = new Token(9);
        if ($owner->value != 9) { panic("shadow"); }
    })->value;
    $owner = new Token(8);
    echo $owner->value;
    let writable $number = 1;
    echo when (true): int { return $number; } else { return 0; }
        finally { $number = 2; };
    echo $number;
    try {
        echo lend(when (true): Token { return $owner; } else { return $owner; }
            finally { fail(); })->value;
    } catch (Failure) {
        $owner = new Token(9);
        echo $owner->value;
    }
}
"#;
    assert_native_output(source, "78129");
    let mut malformed = crate::lower_source_to_mir("finalizer-value-proof.doria", source).unwrap();
    let function = malformed
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .unwrap();
    let (source, result) = function.blocks.iter().flat_map(|block| &block.statements).find_map(|statement| {
        let crate::mir::Statement::ControlFlowPlan(crate::mir::ControlFlowPlan::Finalizer(plan)) = statement else { return None; };
        plan.parent.as_ref()?;
        plan.exits.iter().find_map(|exit| {
            let crate::mir::StructuredExitKind::WhenYield { result } = exit.kind else { return None; };
            function.blocks[exit.source.0].statements.iter().any(|statement| matches!(statement, crate::mir::Statement::AssignLocal { target, .. } if *target == result)).then_some((exit.source, result))
        })
    }).expect("nested finalizer with an acquired yielded value");
    function.blocks[source.0].statements.retain(|statement| !matches!(statement, crate::mir::Statement::AssignLocal { target, .. } if *target == result));
    assert!(
        crate::mir_validation::validate_program(&malformed).is_err(),
        "a child finalizer cannot forward an unacquired result"
    );
}

#[test]
fn conditional_checked_errors_preserve_inner_catches_and_outer_escapes() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }
function fail(): int throws Failure { throw new Failure("operand"); }
function outside(bool $left): int {
    try {
        return when ($left): int { return fail(); } else { return 3; };
    } catch (Failure) { return 7; }
}
function matched(bool $left): int {
    try { return match ($left) { true => fail(), false => 5, }; }
    catch (Failure) { return 11; }
}
function inside(): int {
    return when (true): int {
        try { return fail(); } catch (Failure) { return 13; }
    } else { return 17; };
}
function main(): void throws Error {
    echo outside(true); echo ":"; echo outside(false); echo ":";
    echo matched(true); echo ":"; echo matched(false); echo ":"; echo inside();
}
"#;
    assert_native_output(source, "7:3:11:5:13");
    let valid = crate::lower_source_to_mir("conditional-errors.doria", source).unwrap();
    let mut missing = valid.clone();
    let function = missing
        .functions
        .iter_mut()
        .find(|f| f.name == "inside")
        .unwrap();
    let result = function
        .blocks
        .iter()
        .flat_map(|b| &b.statements)
        .find_map(|s| match s {
            crate::mir::Statement::ControlFlowPlan(crate::mir::ControlFlowPlan::When(plan)) => {
                Some(plan.result)
            }
            _ => None,
        })
        .unwrap();
    for block in &mut function.blocks {
        block.statements.retain(|s| !matches!(s, crate::mir::Statement::AssignLocal { target, .. } if *target == result));
    }
    let error = crate::mir_validation::validate_program(&missing).unwrap_err();
    assert!(error.message.contains("0 result assignments"), "{error:?}");

    let mut unacquired = valid.clone();
    let function = unacquired
        .functions
        .iter_mut()
        .find(|f| f.name == "outside")
        .unwrap();
    let (branch, exit) = function
        .blocks
        .iter()
        .flat_map(|b| &b.statements)
        .find_map(|s| match s {
            crate::mir::Statement::ControlFlowPlan(crate::mir::ControlFlowPlan::When(plan)) => {
                Some((plan.branches[1], plan.error_exit.unwrap()))
            }
            _ => None,
        })
        .unwrap();
    function.blocks[branch.0].terminator = crate::mir::Terminator::Jump(exit.destination);
    let error = crate::mir_validation::validate_program(&unacquired).unwrap_err();
    assert!(
        error
            .message
            .contains("without acquiring its checked Error"),
        "{error:?}"
    );
}

#[test]
fn conditional_callees_preserve_selection_nullable_transport_and_order() {
    let source = r#"
class Trace { static writable int $order = 0; }
function mark(int $digit): int {
    Trace::order = Trace::order * 10 + $digit;
    return $digit;
}
function optional(bool $present): ?function(int): int {
    return when ($present): ?function(int): int {
        return fn(int $value) => $value + 4;
    } else { return null; };
}
function main(): void throws Error {
    let $first = fn(int $value) => $value + 1;
    let $second = fn(int $value) => $value + 2;
    echo (when (true): function(int): int {
        mark(1); return $first;
    } else { return $second; } finally { mark(2); })(mark(3));
    echo ":";
    echo Trace::order;
    echo (match (false) { true => $first, false => $second, })(5);
    echo (optional(false) ?? $first)(8);
    echo (optional(true) ?? $second)(8);
    let $present = when (false): ?function(int): int {
        return null;
    } else { return fn(int $value) => $value + 5; };
    if ($present != null) { echo $present(8); }
}
"#;
    assert_native_output(source, "4:123791213");
}

#[test]
fn conditional_once_callees_release_pending_captures_on_argument_failure() {
    let source = r#"
class Trace { static writable int $drops = 0; static writable int $calls = 0; }
class Token {
    function __construct(int $value) {}
    function __destruct(): void { Trace::drops++; }
}
class Failure implements Error { function __construct(string $message) {} }
function fail(): int throws Failure { throw new Failure("argument"); }
function drop(take Token $value): void {}
function make(int $value): function once(int): int {
    let $token = new Token($value);
    return function(int $number): int with (take $token) {
        Trace::calls++;
        let $result = $token->value + $number;
        drop($token);
        return $result;
    };
}
function main(): void throws Error {
    try {
        (when (true): function once(int): int {
            return make(7);
        } else { return make(9); })(fail());
    } catch (Failure) {}
    try {
        (match (true) { true => make(7), default => make(9), })(fail());
    } catch (Failure) {}
    if (Trace::drops != 2 || Trace::calls != 0) { panic("pending callee cleanup"); }
    echo (when (false): function once(int): int {
        return make(7);
    } else { return make(9); })(3);
    echo (match (true) { true => make(7), default => make(9), })(1);
    if (Trace::drops != 4 || Trace::calls != 2) { panic("callee handoff cleanup"); }
    try {
        (when (true): function once(int): int {
            return make(7);
        } else { return make(9); } finally { fail(); })(1);
    } catch (Failure) {}
    if (Trace::drops != 5 || Trace::calls != 2) { panic("selected callee finalizer cleanup"); }
}
"#;
    assert_native_output(source, "128");
}

#[test]
fn nullable_destinations_preserve_checked_scalar_results_across_expression_kinds() {
    let mut source = String::from("class Holder<T> { writable ?T $value = null; }\n");
    let mut main = String::from("function main(): void {\n");
    for (index, (ty, first, second)) in [
        ("int", "7", "8"),
        ("int8", "7", "8"),
        ("int16", "7", "8"),
        ("int32", "7", "8"),
        ("int64", "7", "8"),
        ("uint8", "7", "8"),
        ("uint16", "7", "8"),
        ("uint32", "7", "8"),
        ("uint64", "7", "8"),
        ("float", "1.5", "2.5"),
        ("float32", "1.5", "2.5"),
        ("float64", "1.5", "2.5"),
        ("bool", "true", "false"),
    ]
    .into_iter()
    .enumerate()
    {
        source.push_str(&format!(
            "function choose{index}(bool $first): ?{ty} {{
                return when ($first): {ty} {{ return {first}; }} else {{ return {second}; }};
            }}
            function matched{index}(bool $first): ?{ty} {{
                return match ($first) {{ true => {first}, false => {second}, }};
            }}
            function accept{index}(?{ty} $value): bool {{ return ($value ?? {second}) == {first}; }}
            function merge{index}(?{ty} $value): ?{ty} {{ return $value ?? {first}; }}\n"
        ));
        main.push_str(&format!(
            "let writable $holder{index} = new Holder<{ty}>();
             $holder{index}->value = when (true): {ty} {{ return {first}; }} else {{ return {second}; }};
             if (!accept{index}($holder{index}->value) || !accept{index}(choose{index}(true))
                 || !accept{index}(matched{index}(true)) || !accept{index}(merge{index}(null))
                 || !accept{index}(merge{index}({first}))
                 || !accept{index}(when (true): {ty} {{ return {first}; }} else {{ return {second}; }})
                 || (choose{index}(false) ?? {first}) != {second}
                 || (matched{index}(false) ?? {first}) != {second}) {{ panic(\"scalar conversion\"); }}\n"
        ));
    }
    source.push_str(
        "class Trace { static writable int $calls = 0; }
         function effect(): bool { Trace::calls++; return true; }
         function both(bool $value): ?bool { return $value && effect(); }
         function either(bool $value): ?bool { return $value || effect(); }\n",
    );
    main.push_str(
        "if ((both(false) ?? true) || !(either(true) ?? false) || Trace::calls != 0) {
             panic(\"short circuit\");
         }
         if (!(both(true) ?? false) || !(either(false) ?? false) || Trace::calls != 2) {
             panic(\"boolean destination\");
         }
         echo \"passed\";
         }",
    );
    source.push_str(&main);
    assert_native_output(&source, "passed");
}

#[test]
fn nullable_owning_fields_preserve_replacement_and_failed_construction_cleanup() {
    assert_native_output(
        include_str!("../../tests/fixtures/stored_assignment_regions.doria"),
        "6;6;6;6;5;new 1;new 2;drop 1;fail;drop 2;new 3;fail;drop 3;",
    );
}

#[test]
fn conditional_results_preserve_nominal_upcasts_and_nullable_managed_values() {
    let source = r#"
interface Named { function id(): int; }
open class Base implements Named {
    function __construct(int $value) {}
    function id(): int { return $this->value; }
}
class Derived extends Base {
    function __construct(parameter int $value) { parent::__construct($value); }
}
enum Mode { case First; case Second; }
enum Packet { case Number(int $value); case Empty; }
function base(bool $first): ?Base {
    return when ($first): Derived { return new Derived(1); } else { return new Derived(2); };
}
function named(bool $first): ?Named {
    return match ($first) { true => new Derived(3), false => new Derived(4), };
}
function text(bool $first): ?string {
    return when ($first): string { return "first"; } else { return "second"; };
}
function mode(bool $first): ?Mode {
    return match ($first) { true => Mode::First, false => Mode::Second, };
}
function packet(bool $first): ?Packet {
    return when ($first): Packet { return Packet::Number(5); } else { return Packet::Empty; };
}
function main(): void {
    let $base = base(false);
    let $named = named(true);
    if ($base == null || $named == null) { panic("missing nominal result"); }
    echo $base->id() . ";" . $named->id() . ";" . (text(true) ?? "missing") . ";";
    echo ((mode(false) ?? Mode::First) == Mode::Second) . ";";
    let $packet = packet(true) ?? Packet::Empty;
    echo match ($packet) { Packet::Number($value) => $value, Packet::Empty => 0, };
}
"#;
    assert_native_output(source, "2;3;first;true;5");
}
