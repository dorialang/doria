use doriac::ast::ClosureCaptureMode;
use doriac::diagnostics::{Diagnostic, DiagnosticSeverity, FixApplicability};
use doriac::semantics::{CallableValueTargetKind, CaptureRequirement};
use doriac::symbols::BindingKind;
use doriac::types::{
    FunctionBorrowSource, FunctionInvocationMode, FunctionTypeParameterMode, ResolvedType,
};

fn analyze(source: &str) -> doriac::semantics::SemanticAnalysis {
    let (_, analysis) = doriac::analyze_source_for_ide("stage30b.doria", source)
        .expect("Stage 30b source should parse");
    analysis
}

fn permanent_errors(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error && diagnostic.code != "E0641"
        })
        .collect()
}

fn diagnostic<'a>(diagnostics: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}, got {diagnostics:#?}"))
}

#[test]
fn dedicated_stage30b_fixtures_preserve_type_capture_and_execution_boundaries() {
    let type_only = analyze(include_str!("fixtures/stage30b/type_only.doria"));
    assert!(
        type_only.diagnostics.is_empty(),
        "{:#?}",
        type_only.diagnostics
    );

    let valid = analyze(include_str!(
        "fixtures/stage30b/valid_execution_boundary.doria"
    ));
    assert!(valid.diagnostics.is_empty(), "{:#?}", valid.diagnostics);

    let invalid = analyze(include_str!("fixtures/stage30b/invalid_capture.doria"));
    diagnostic(&invalid.diagnostics, "E0642");
    assert!(invalid
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0641"));
}

#[test]
fn semantic_function_types_preserve_structure_effects_and_nested_types() {
    let source = r#"
class Failure implements Error
{
    function __construct(string $message)
    {
    }
}

function accept(
    function(int): int $readonly,
    function writable(writable string): void $writable,
    function once(take string): string $once,
    function((function(): int), string): void throws Failure $nested
): void
{
}
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.function_types_by_span.len(), 5);

    let mut types = analysis
        .info
        .function_types_by_span
        .values()
        .filter_map(|info| match &info.ty {
            ResolvedType::Function(function) => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    types.sort_by_key(|function| function.parameters.len());
    assert!(types.iter().any(|function| {
        function.invocation_mode == FunctionInvocationMode::Writable
            && function.parameters[0].ownership_mode == FunctionTypeParameterMode::Writable
    }));
    assert!(types.iter().any(|function| {
        function.invocation_mode == FunctionInvocationMode::Once
            && function.parameters[0].ownership_mode == FunctionTypeParameterMode::Take
    }));
    assert!(types
        .iter()
        .any(|function| !function.checked_effects.is_empty()));
}

#[test]
fn structural_function_assignment_uses_modes_types_effects_and_nullability() {
    let source = r#"
function main(): void
{
    let writable $count = 0;
    function(): int $wrongMode = function (): int with (writable $count) {
        $count += 1;
        return $count;
    };
    function(int): int $wrongParameter = fn(string $value) => 1;
    function(int): string $wrongReturn = fn(int $value) => $value;
    ?function(int): int $nullable = null;
    function(int): int $nonNull = $nullable;
}
"#;
    let analysis = analyze(source);
    let mismatches = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0648")
        .collect::<Vec<_>>();
    assert_eq!(mismatches.len(), 4, "{:#?}", analysis.diagnostics);
    assert!(mismatches
        .iter()
        .any(|diagnostic| diagnostic.message.contains("invocation access")));
    assert!(mismatches
        .iter()
        .any(|diagnostic| diagnostic.message.contains("parameter 1")));
    assert!(mismatches
        .iter()
        .any(|diagnostic| diagnostic.message.contains("expected return type")));
    assert!(mismatches
        .iter()
        .any(|diagnostic| diagnostic.message.contains("nullable callable")));
    assert!(analysis
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0403"));
}

#[test]
fn capture_plans_use_stable_binding_identity_and_infer_minimum_access() {
    let source = r#"
function main(): void
{
    let writable $count = 1;
    let $read = fn() with (writable $count) => $count;
    let $write = function (): int with (writable $count) {
        $count += 1;
        return $count;
    };
}
"#;
    let first = analyze(source);
    let second = analyze(source);
    assert!(
        permanent_errors(&first.diagnostics).is_empty(),
        "{:#?}",
        first.diagnostics
    );
    assert_eq!(
        first.info.binding_resolution,
        second.info.binding_resolution
    );
    assert_eq!(first.info.closures, second.info.closures);

    let mut closures = first.info.closures.values().collect::<Vec<_>>();
    closures.sort_by_key(|closure| closure.closure_id.start);
    assert_eq!(closures.len(), 2);
    assert_eq!(
        closures[0].inferred_invocation_mode,
        FunctionInvocationMode::Readonly
    );
    assert_eq!(
        closures[1].inferred_invocation_mode,
        FunctionInvocationMode::Writable
    );
    assert_eq!(closures[0].captures[0].mode, ClosureCaptureMode::Writable);
    assert_eq!(
        closures[0].captures[0].required_capability,
        CaptureRequirement::Readonly
    );
    assert_eq!(
        closures[1].captures[0].required_capability,
        CaptureRequirement::Writable
    );
    assert_eq!(
        closures[0].captures[0].source_binding_id,
        closures[1].captures[0].source_binding_id
    );
}

#[test]
fn binding_catalogue_preserves_specialized_kinds_and_real_source_spans() {
    let source = r#"
class Inspector
{
    function inspect(List<int> $values): void
    {
        given { let $ready = true; $ready; } if (true) {}
        for (let writable $index = 0; $index < 1; $index++) {}
        foreach ($values as int $value) {}
        Dictionary<string, int> $entries = [];
        foreach ($entries as string $key => int $entry) {}
    }
}
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let declarations = analysis
        .info
        .binding_resolution
        .declarations_by_id
        .values()
        .collect::<Vec<_>>();

    for (name, kind) in [
        ("ready", BindingKind::GivenBinding),
        ("index", BindingKind::LoopBinding),
        ("value", BindingKind::ForeachValue),
        ("key", BindingKind::ForeachKey),
        ("entry", BindingKind::ForeachValue),
    ] {
        let declaration = declarations
            .iter()
            .find(|declaration| declaration.name == name)
            .unwrap_or_else(|| panic!("missing binding metadata for ${name}"));
        assert_eq!(declaration.kind, kind);
        let span = declaration.span.expect("source binding span");
        assert_eq!(&source[span.start..span.end], format!("${name}"));
    }

    let receiver = declarations
        .iter()
        .find(|declaration| declaration.kind == BindingKind::MethodReceiver)
        .expect("method receiver identity");
    assert_eq!(receiver.name, "this");
    assert_eq!(receiver.span, None, "receiver has no invented declaration");
}

#[test]
fn missing_captures_are_grouped_by_binding_and_offer_one_safe_fix() {
    let source = r#"
function main(): void
{
    let $minimum = 2;
    let writable $total = 0;
    let $operation = function (): int {
        $total += $minimum;
        return $total + $minimum;
    };
}
"#;
    let analysis = analyze(source);
    let missing = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0642")
        .collect::<Vec<_>>();
    assert_eq!(missing.len(), 2, "{:#?}", analysis.diagnostics);
    assert!(missing
        .iter()
        .all(|diagnostic| diagnostic.cause_id.is_some()));
    assert_eq!(
        missing
            .iter()
            .filter(|diagnostic| !diagnostic.fixes.is_empty())
            .count(),
        1
    );
    let fix = missing
        .iter()
        .flat_map(|diagnostic| &diagnostic.fixes)
        .next()
        .expect("combined missing-capture fix");
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(
        fix.edits[0].replacement,
        " with (writable $total, $minimum)"
    );
    assert!(analysis
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0641"));
}

#[test]
fn duplicate_invalid_recursive_and_unused_captures_are_precise() {
    let source = r#"
const int LIMIT = 2;

function main(): void
{
    let $value = 1;
    let $duplicate = fn() with ($value, writable $value) => $value;
    let $constant = fn() with ($LIMIT) => LIMIT;
    let $recursive = fn() with ($recursive) => 1;
    let $unused = fn() with ($value) => 1;
}
"#;
    let analysis = analyze(source);
    for code in ["E0643", "E0644", "E0647", "E0646"] {
        diagnostic(&analysis.diagnostics, code);
    }
    assert!(analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code != "E0641")
        .all(|diagnostic| diagnostic.code.starts_with("E064")));
}

#[test]
fn this_capture_obeys_receiver_capability_and_is_never_implicit() {
    let source = r#"
class Counter
{
    writable int $value = 0;

    function readonlyClosure(): void
    {
        let $missing = fn() => $this->value;
        let $invalid = function (): int with (writable $this) { return $this->value; };
    }

    writable function writableClosure(): void
    {
        let $valid = function (): int with (writable $this) {
            $this->value += 1;
            return $this->value;
        };
    }
}
"#;
    let analysis = analyze(source);
    let missing = diagnostic(&analysis.diagnostics, "E0642");
    assert!(missing.title.contains("`$this`"), "{missing:#?}");
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0645" && diagnostic.message.contains("writable capture")
    }));
    assert!(analysis.info.closures.values().any(|closure| {
        closure.inferred_invocation_mode == FunctionInvocationMode::Writable
            && closure.captures.iter().any(|capture| {
                capture.mode == ClosureCaptureMode::Writable
                    && capture.required_capability == CaptureRequirement::Writable
            })
    }));
}

#[test]
fn callable_values_check_arguments_access_effects_and_return_types() {
    let source = r#"
function apply(function(int): string $callback): void
{
    string $result = $callback(42);
    $callback("wrong");
}

function mutate(function writable(writable int): void $callback, int $value): void
{
    $callback($value);
}
"#;
    let analysis = analyze(source);
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0408" && diagnostic.message.contains("expects `int`, got `string`")
    }));
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0651" && diagnostic.message.contains("writable access")
    }));
    assert_eq!(analysis.info.callable_value_calls.len(), 3);
    assert!(analysis.info.callable_value_calls.values().any(|call| {
        call.target_kind == CallableValueTargetKind::Value
            && call.return_type == ResolvedType::String
    }));
}

#[test]
fn nullable_callable_narrowing_works_for_functions_locals_and_closure_roots() {
    let source = r#"
function invoke(?function(int): int $callback): void
{
    if ($callback != null) {
        $callback(1);
    }
    $callback(2);
}

function local(?function(): int $input): void
{
    ?function(): int $callback = $input;
    if ($callback is function(): int) {
        $callback();
    }
}

function main(): void
{
    let $closure = function (?function(): int $callback): void {
        if ($callback != null) {
            $callback();
        }
    };
}
"#;
    let analysis = analyze(source);
    let nullable = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0650")
        .collect::<Vec<_>>();
    assert_eq!(nullable.len(), 1, "{:#?}", analysis.diagnostics);
    assert_eq!(analysis.info.callable_value_calls.len(), 3);
}

#[test]
fn callable_properties_are_resolved_semantically_before_method_fallback() {
    let source = r#"
class Worker
{
    function(int): string $format;

    function __construct(function(int): string $format)
    {
        $this->format = $format;
    }
}

function run(Worker $worker): void
{
    string $result = $worker->format(42);
    $worker->format(value: 42);
    $worker?->format(42);
}
"#;
    let analysis = analyze(source);
    assert!(analysis.info.callable_value_calls.values().any(|call| {
        call.target_kind == CallableValueTargetKind::Property
            && call.return_type == ResolvedType::String
    }));
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0652")
            .count(),
        2,
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn invalid_closures_do_not_receive_the_execution_boundary() {
    let source = r#"
function main(): void
{
    let $value = 1;
    let $invalid = fn() => $value;
}
"#;
    let analysis = analyze(source);
    diagnostic(&analysis.diagnostics, "E0642");
    assert!(analysis
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0641"));
}

#[test]
fn captured_move_return_requires_take_and_infers_once() {
    let source = r#"
class Payload
{
}

function main(): void
{
    let $borrowed = new Payload();
    let $invalid = function (): Payload with ($borrowed) { return $borrowed; };
    let $owned = new Payload();
    let $valid = function (): Payload with (take $owned) { return $owned; };
}
"#;
    let analysis = analyze(source);
    diagnostic(&analysis.diagnostics, "E0653");
    assert!(analysis.info.closures.values().any(|closure| {
        closure.inferred_invocation_mode == FunctionInvocationMode::Once
            && closure
                .captures
                .iter()
                .any(|capture| capture.required_capability == CaptureRequirement::Take)
    }));
}

#[test]
fn effect_sets_grouping_and_return_borrows_have_semantic_identity() {
    let source = r#"
class FirstError implements Error { function __construct(string $message) {} }
class SecondError implements Error { function __construct(string $message) {} }
class Payload {}

function accept(
    function(Payload): Payload $plain,
    (function(Payload): Payload) $grouped,
    function(): void throws FirstError, SecondError $first,
    function(): void throws SecondError, FirstError $second
): void
{
    function(Payload): Payload $same = $grouped;
}

function main(): void
{
    let $identity = function (Payload $value): Payload { return $value; };
}
"#;
    let analysis = analyze(source);
    assert!(
        permanent_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );

    let infos = analysis
        .info
        .function_types_by_span
        .values()
        .collect::<Vec<_>>();
    let borrowed = infos
        .iter()
        .filter_map(|info| match &info.ty {
            ResolvedType::Function(function)
                if function.return_borrow.is_some()
                    && function.parameters.len() == 1
                    && function.checked_effects.is_empty() =>
            {
                Some(function)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(borrowed.len(), 3);
    assert!(borrowed.iter().all(|function| *function == borrowed[0]));
    assert!(matches!(
        borrowed[0].return_borrow.unwrap().source,
        FunctionBorrowSource::Parameter(0)
    ));

    let effects = infos
        .iter()
        .filter_map(|info| match &info.ty {
            ResolvedType::Function(function) if function.checked_effects.len() == 2 => {
                Some(function)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    assert_eq!(
        effects[0], effects[1],
        "effect source order is not identity"
    );
    assert_ne!(
        infos
            .iter()
            .find(|info| info.authored_checked_effects.first()
                == Some(&ResolvedType::Class(doriac::types::ClassType::new(
                    "FirstError",
                    vec![]
                ))))
            .unwrap()
            .authored_checked_effects,
        infos
            .iter()
            .find(|info| info.authored_checked_effects.first()
                == Some(&ResolvedType::Class(doriac::types::ClassType::new(
                    "SecondError",
                    vec![]
                ))))
            .unwrap()
            .authored_checked_effects,
        "source-facing effect order remains authored"
    );

    let closure = analysis.info.closures.values().next().unwrap();
    let ResolvedType::Function(function) = &closure.function_type else {
        panic!("closure must have a semantic function type")
    };
    assert!(matches!(
        function.return_borrow.unwrap().source,
        FunctionBorrowSource::Parameter(0)
    ));
}

#[test]
fn invocation_capability_substitution_uses_one_ordered_rule() {
    let source = r#"
class Payload {}

function main(): void
{
    let writable $count = 0;
    let $readonly = fn() => 1;
    let $writable = function (): int with (writable $count) {
        $count += 1;
        return $count;
    };
    let $payload = new Payload();
    let $once = function (): Payload with (take $payload) { return $payload; };

    function(): int $rToR = $readonly;
    function writable(): int $rToW = $readonly;
    function once(): int $rToO = $readonly;
    function writable(): int $wToW = $writable;
    function once(): int $wToO = $writable;
    function once(): Payload $oToO = $once;

    function(): int $wToR = $writable;
    function(): Payload $oToR = $once;
    function writable(): Payload $oToW = $once;
}

"#;
    let analysis = analyze(source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0648"
                && diagnostic.message.contains("invocation access"))
            .count(),
        3,
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn compatible_closure_context_is_preserved_for_execution_without_erasing_inference() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }

function makeOnce(): function once(): int throws Failure
{
    let $value = 42;
    return fn() with (take $value) => $value;
}
"#;
    let analysis = analyze(source);
    assert!(
        permanent_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );

    let closure = analysis.info.closures.values().next().unwrap();
    let ResolvedType::Function(inferred) = &closure.function_type else {
        panic!("closure must have an inferred function type")
    };
    let ResolvedType::Function(execution) = &closure.execution_function_type else {
        panic!("closure must have an execution function type")
    };
    assert_eq!(inferred.invocation_mode, FunctionInvocationMode::Readonly);
    assert!(inferred.checked_effects.is_empty());
    assert_eq!(execution.invocation_mode, FunctionInvocationMode::Once);
    assert_eq!(execution.checked_effects.len(), 1);
}

#[test]
fn closure_effects_are_inferred_isolated_and_reintroduced_by_invocation() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }

function build(): void
{
    let $throwing = function (): void { throw new Failure("later"); };
    let $handled = function (): void {
        try { throw new Failure("handled"); } catch (Failure) {}
    };
}

function invoke(function(): void throws Failure $operation): void
{
    $operation();
}
"#;
    let analysis = analyze(source);
    let permanent = permanent_errors(&analysis.diagnostics);
    assert_eq!(permanent.len(), 1, "{:#?}", analysis.diagnostics);
    assert_eq!(permanent[0].code, "E0631");

    let mut closures = analysis.info.closures.values().collect::<Vec<_>>();
    closures.sort_by_key(|closure| closure.closure_id.start);
    assert_eq!(closures.len(), 2);
    assert_eq!(closures[0].inferred_checked_effects.len(), 1);
    assert!(closures[1].inferred_checked_effects.is_empty());
}

#[test]
fn nested_capture_lineage_and_control_flow_uses_resolve_by_binding() {
    let source = r#"
function main(): void
{
    let $base = 10;
    let $outer = fn(int $left) with (take $base) =>
        function (int $right): int with (take $left, take $base) {
            if ($right > 0) {
                return $left + $right + $base;
            }
            return $base;
        };
}
"#;
    let first = analyze(source);
    let second = analyze(source);
    assert!(
        permanent_errors(&first.diagnostics).is_empty(),
        "{:#?}",
        first.diagnostics
    );
    assert_eq!(
        first.info.binding_resolution,
        second.info.binding_resolution
    );

    let mut closures = first.info.closures.values().collect::<Vec<_>>();
    closures.sort_by_key(|closure| closure.closure_id.start);
    assert_eq!(closures.len(), 2);
    assert_eq!(closures[0].captures.len(), 1);
    assert_eq!(closures[1].captures.len(), 2);
    let outer_environment = closures[0].captures[0].environment_binding_id;
    assert!(closures[1]
        .captures
        .iter()
        .any(|capture| capture.source_binding_id == outer_environment));
}

#[test]
fn function_values_are_rejected_from_implicit_capability_positions() {
    let source = r#"
function main(): void
{
    let $callback = fn(int $value) => $value;
    Set<function(int): int> $set = [$callback];
    Dictionary<function(int): int, int> $dictionary = [$callback => 1];
    SortedSet<function(int): int> $sorted = SortedSet::from([$callback]);
    bool $same = $callback == $callback;
    echo "{$callback}";
}
"#;
    let analysis = analyze(source);
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("Hashable")));
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("Comparable")));
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("cannot be compared")));
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("cannot be displayed")
            || diagnostic.message.contains("cannot be interpolated")
    }));
}

#[test]
fn capture_fixes_preserve_trailing_commas_and_refuse_comment_loss() {
    let source = r#"
function main(): void
{
    let $base = 1;
    let $minimum = 2;
    let $extended = fn() with ($base) => $base + $minimum;
}
"#;
    let analysis = analyze(source);
    let missing = diagnostic(&analysis.diagnostics, "E0642");
    let fix = missing.fixes.first().expect("safe extension fix");
    assert_eq!(fix.edits.len(), 1);
    let edit = &fix.edits[0];
    let fixed = format!(
        "{}{}{}",
        &source[..edit.span.start],
        edit.replacement,
        &source[edit.span.end..]
    );
    assert!(fixed.contains("with ($base, $minimum)"), "{fixed}");
    let fixed_analysis = analyze(&fixed);
    assert!(fixed_analysis
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0642"));

    let commented = r#"
function main(): void
{
    let $value = 1;
    let $unused = fn() with (/* retained explanation */ $value) => 1;
}
"#;
    let commented_analysis = analyze(commented);
    let warning = diagnostic(&commented_analysis.diagnostics, "E0646");
    assert!(
        warning.fixes.is_empty(),
        "comment-owning removal is not safe"
    );

    let plain = r#"
function main(): void
{
    let $value = 1;
    let $unused = fn() with ($value) => 1;
}
"#;
    let plain_analysis = analyze(plain);
    let warning = diagnostic(&plain_analysis.diagnostics, "E0646");
    let fix = warning.fixes.first().expect("last capture clause removal");
    assert_eq!(
        &plain[fix.edits[0].span.start..fix.edits[0].span.end],
        "with ($value)"
    );
}

#[test]
fn function_types_flow_through_properties_collections_and_generic_inference() {
    let source = r#"
class Holder<T>
{
    function __construct(take T $value) {}
}

class Callbacks
{
    function(int): int $transform = fn(int $value) => $value;
    List<function(int): int> $steps = [];
}

function identity<T>(take T $value): T { return $value; }

function main(): void
{
    let $callback = fn(int $value) => $value + 1;
    let $holder = new Holder<function(int): int>($callback);
    let $identityInput = fn(int $value) => $value;
    function(int): int $same = identity($identityInput);
    List<function(int): int> $steps = [$same];
}
"#;
    let analysis = analyze(source);
    assert!(
        permanent_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis
        .info
        .generic_call_specializations
        .values()
        .any(|specialization| matches!(
            specialization.arguments.first(),
            Some(doriac::semantics::GenericArgument::Type(
                ResolvedType::Function(_)
            ))
        )));
}

#[test]
fn indexed_capture_mutation_requires_writable_invocation_access() {
    let source = r#"
function main(): void
{
    let writable $items = [1, 2];
    let $mutate = function (): void with (writable $items) {
        $items[0] += 1;
        $items[1]++;
    };
}
"#;
    let analysis = analyze(source);
    assert!(
        permanent_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let closure = analysis.info.closures.values().next().unwrap();
    assert_eq!(
        closure.inferred_invocation_mode,
        FunctionInvocationMode::Writable
    );
    assert_eq!(
        closure.captures[0].required_capability,
        CaptureRequirement::Writable
    );
}

#[test]
fn taking_copy_arguments_preserves_readonly_capture_access() {
    let source = r#"
function main(): void
{
    let $value = 1;
    function(take int): int $consume = fn(take int $input) => $input;
    let $operation = fn() with ($value, $consume) => $consume($value);
}
"#;
    let analysis = analyze(source);
    assert!(
        permanent_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis
        .info
        .closures
        .values()
        .all(|closure| { closure.inferred_invocation_mode == FunctionInvocationMode::Readonly }));
    assert!(analysis.info.closures.values().any(|closure| {
        closure.captures.iter().any(|capture| {
            matches!(capture.source_type, ResolvedType::Integer(_))
                && capture.required_capability == CaptureRequirement::Readonly
        })
    }));
}

#[test]
fn captures_conflict_only_with_bindings_in_the_closure_scope() {
    let source = r#"
class Failure implements Error
{
    function __construct(string $message) {}
}

enum Number
{
    case Value(int $value);
}

function main(): void
{
    let $value = 9;
    let $items = [1];
    let $operation = function (): int with ($value, $items) {
        {
            let $value = 1;
        }
        foreach ($items as int $value) {}
        try {
            throw new Failure("handled");
        } catch (Failure $value) {}
        int $matched = match (Number::Value(2)) {
            Number::Value($value) => $value
        };
        return $value + $matched;
    };
}
"#;
    let analysis = analyze(source);
    assert!(
        permanent_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );

    let same_scope = analyze(
        r#"
function main(): void
{
    let $value = 1;
    let $invalid = function (): int with ($value) {
        let $value = 2;
        return $value;
    };
}
"#,
    );
    diagnostic(&same_scope.diagnostics, "E0644");
}

#[test]
fn arrow_closures_receive_short_circuit_narrowing_facts() {
    let source = r#"
class Box
{
    bool $ready = true;
}

function main(): void
{
    let $andPredicate = fn(?Box $box) => $box != null && $box->ready;
    let $orPredicate = fn(?Box $box) => $box == null || $box->ready;
}
"#;
    let analysis = analyze(source);
    assert!(
        permanent_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn inferred_callable_modes_invalidate_narrowing_for_direct_and_aliased_calls() {
    let source = r#"
class Box
{
    bool $ready = true;
}

function main(): void
{
    writable ?Box $direct = new Box();
    writable ?Box $aliased = new Box();
    let $mutator = function (writable ?Box $value): void {
        $value = null;
    };
    let $alias = $mutator;

    if ($direct != null) {
        $mutator($direct);
        echo "{$direct->ready}";
    }
    if ($aliased != null) {
        $alias($aliased);
        echo "{$aliased->ready}";
    }
}
"#;
    let analysis = analyze(source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0506")
            .count(),
        2,
        "{:#?}",
        analysis.diagnostics
    );
}
