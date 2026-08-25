use doriac::diagnostics::{
    Diagnostic, DiagnosticFormat, DiagnosticSeverity, FixApplicability, RenderOptions,
};
use doriac::ownership::{
    CaptureAcquisitionKind, ClosureEscapeClassification, ClosureValueProvenance,
    InvocationConsumption,
};

fn analyze(source: &str) -> doriac::semantics::SemanticAnalysis {
    let (_, analysis) = doriac::analyze_source_for_ide("stage30c.doria", source)
        .expect("Stage 30c source should parse");
    analysis
}

fn language_errors(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.kind == doriac::diagnostics::DiagnosticKind::Language
        })
        .collect()
}

fn diagnostic<'a>(diagnostics: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}, got {diagnostics:#?}"))
}

fn apply_current_source_fix(source: &str, diagnostic: &Diagnostic) -> String {
    let mut edits = diagnostic
        .fixes
        .first()
        .expect("diagnostic should carry a structured fix")
        .edits
        .clone();
    edits.sort_by_key(|edit| std::cmp::Reverse((edit.span.start, edit.span.end)));
    let mut rewritten = source.to_string();
    for edit in edits {
        rewritten.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    rewritten
}

#[test]
fn ownership_transfer_fixes_are_review_only_and_preserve_trivia() {
    let capture_source = r#"
function main(): void
{
    let $value = 1;
    let $borrowed = fn() with (/* keep capture */ $value) => $value;
    List<function(): int> $items = [$borrowed];
}
"#;
    let capture = analyze(capture_source);
    let escape = diagnostic(&capture.diagnostics, "E0658");
    assert_eq!(escape.fixes.len(), 1, "{escape:#?}");
    assert_eq!(
        escape.fixes[0].applicability,
        FixApplicability::RequiresReview
    );
    let rewritten = apply_current_source_fix(capture_source, escape);
    assert!(rewritten.contains("/* keep capture */ take $value"));

    let parameter_source = r#"
class Store
{
    writable function(): int $callback = fn() => 0;
    writable function retain(/* keep parameter */ function(): int $input): void
    {
        $this->callback = $input;
    }
}
"#;
    let parameter = analyze(parameter_source);
    let retention = diagnostic(&parameter.diagnostics, "E0657");
    assert_eq!(retention.fixes.len(), 1, "{retention:#?}");
    assert_eq!(
        retention.fixes[0].applicability,
        FixApplicability::RequiresReview
    );
    let rewritten = apply_current_source_fix(parameter_source, retention);
    assert!(rewritten.contains("/* keep parameter */ take function(): int $input"));

    for fix in escape.fixes.iter().chain(&retention.fixes) {
        assert_ne!(fix.applicability, FixApplicability::MachineApplicable);
        assert!(fix.edits.iter().all(|edit| {
            !edit.replacement.contains("clone")
                && !edit.replacement.contains("SharedReference")
                && !edit.replacement.contains("lifetime")
        }));
    }
}

#[test]
fn once_invocation_from_stored_places_uses_the_move_out_boundary() {
    let property = analyze(
        r#"
class Store
{
    writable function once(): int $callback = fn() => 1;

    writable function invoke(): int
    {
        return $this->callback();
    }
}
"#,
    );
    assert_eq!(
        property
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0660")
            .count(),
        1,
        "{:#?}",
        property.diagnostics
    );
    assert!(property
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0651"));

    let aggregate = analyze(
        r#"
function main(): void
{
    List<function once(): int> $callbacks = [fn() => 1];
    $callbacks[0]();
}
"#,
    );
    assert_eq!(
        aggregate
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0660")
            .count(),
        1,
        "{:#?}",
        aggregate.diagnostics
    );
    assert!(aggregate
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0651"));
}

#[test]
fn capture_plans_preserve_authored_acquisition_and_reverse_release_order() {
    let source = r#"
class Payload {}

function main(): void
{
    let $read = 1;
    let writable $write = 2;
    let $copy = "copy";
    let $owned = new Payload();
    let $operation = function (): void with ($read, writable $write, take $copy, take $owned) {};
}
"#;
    let analysis = analyze(source);
    assert!(
        language_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let plan = analysis.info.closure_ownership.values().next().unwrap();
    assert_eq!(
        plan.acquisitions
            .iter()
            .map(|capture| capture.kind)
            .collect::<Vec<_>>(),
        vec![
            CaptureAcquisitionKind::ReadonlyLease,
            CaptureAcquisitionKind::WritableLease,
            CaptureAcquisitionKind::CopyIntoEnvironment,
            CaptureAcquisitionKind::MoveIntoEnvironment,
        ]
    );
    assert_eq!(plan.release_order, vec![3, 2, 1, 0]);
    assert!(matches!(
        plan.provenance,
        ClosureValueProvenance::BorrowBound(_)
    ));
    assert_eq!(
        plan.invocation_consumption,
        InvocationConsumption::Repeatable
    );
}

#[test]
fn taking_move_capture_consumes_but_taking_copy_capture_preserves_source() {
    let analysis = analyze(
        r#"
class Payload {}
function main(): void
{
    let $copy = "copy";
    let $payload = new Payload();
    let $operation = fn() with (take $copy, take $payload) => $copy;
    echo $copy;
    $payload;
}
"#,
    );
    let moved = diagnostic(&analysis.diagnostics, "E0470");
    assert!(moved.message.contains("payload"), "{moved:#?}");
    assert!(analysis
        .diagnostics
        .iter()
        .all(|diagnostic| !(diagnostic.code == "E0470" && diagnostic.message.contains("copy"))));
}

#[test]
fn closure_carrier_moves_once_and_transfers_its_capture_lease() {
    let analysis = analyze(
        r#"
function main(): void
{
    let writable $value = 1;
    let $first = fn() with ($value) => $value;
    let $second = $first;
    $first;
    $value = 2;
    $second();
}
"#,
    );
    diagnostic(&analysis.diagnostics, "E0655");
    let lease = diagnostic(&analysis.diagnostics, "E0654");
    assert_eq!(lease.title, "Closure Keeps Value In Readonly Use");
}

#[test]
fn capture_leases_end_after_the_closure_last_use() {
    let analysis = analyze(
        r#"
function main(): void
{
    let writable $value = 1;
    let $read = fn() with ($value) => $value;
    $read();
    $value = 2;

    let writable $write = function (): void with (writable $value) { $value += 1; };
    $write();
    $value = 3;
}
"#,
    );
    assert!(
        language_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn writable_and_once_invocations_enforce_access_and_consumption() {
    let analysis = analyze(
        r#"
class Payload {}

function inspect(function once(): Payload $borrowed): void
{
    $borrowed();
}

function main(): void
{
    let writable $count = 0;
    let $readonlyBinding = function (): int with (writable $count) {
        $count += 1;
        return $count;
    };
    $readonlyBinding();

    let $payload = new Payload();
    let $once = function (): Payload with (take $payload) { return $payload; };
    $once();
    $once();
}
"#,
    );
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0656" && diagnostic.title == "Once Invocation Requires Ownership"
    }));
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0656"
            && diagnostic.title == "Writable Invocation Requires Writable Access"
    }));
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0655" && diagnostic.title == "Once Function Was Already Consumed"
    }));
}

#[test]
fn nonescaping_callbacks_cannot_cross_retention_boundaries() {
    let analysis = analyze(
        r#"
class Store
{
    writable function(): int $callback = fn() => 0;
    writable function retain(function(): int $input): void
    {
        $this->callback = $input;
    }
}

function leak(function(): int $input): function(): int
{
    return $input;
}
"#,
    );
    let retained = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0657")
        .count();
    assert_eq!(retained, 2, "{:#?}", analysis.diagnostics);
}

#[test]
fn borrow_bound_closures_cannot_enter_owned_storage() {
    let analysis = analyze(
        r#"
function main(): void
{
    let $value = 1;
    let $borrowed = fn() with ($value) => $value;
    List<function(): int> $items = [$borrowed];
}
"#,
    );
    let escape = diagnostic(&analysis.diagnostics, "E0658");
    assert_eq!(
        escape.title,
        "Borrow-Bound Closure Cannot Enter Owned Storage"
    );
}

#[test]
fn returned_closures_require_owned_or_one_supported_borrow_root() {
    let owned = analyze(
        r#"
function makeOwned(): function(): int
{
    let $value = 1;
    return fn() with (take $value) => $value;
}
"#,
    );
    assert!(
        language_errors(&owned.diagnostics).is_empty(),
        "{:#?}",
        owned.diagnostics
    );
    assert!(owned.info.closure_ownership.values().any(|plan| {
        plan.escape == ClosureEscapeClassification::Owned
            && plan.provenance == ClosureValueProvenance::Owned
    }));

    let local = analyze(
        r#"
function invalid(): function(): int
{
    let $value = 1;
    return fn() with ($value) => $value;
}
"#,
    );
    diagnostic(&local.diagnostics, "E0658");

    let multiple = analyze(
        r#"
function invalid(int $left, int $right): function(): int
{
    return fn() with ($left, $right) => $left + $right;
}
"#,
    );
    diagnostic(&multiple.diagnostics, "E0659");
}

#[test]
fn returned_closure_locals_preserve_borrow_provenance_through_move_chains() {
    let analysis = analyze(
        r#"
function bind(int $value): function(): int
{
    let $callback = fn() with ($value) => $value;
    let $alias = $callback;
    return $alias;
}

function main(): void
{
    let $root = 1;
    let $callback = bind($root);
    List<function(): int> $callbacks = [$callback];
}
"#,
    );
    let escape = diagnostic(&analysis.diagnostics, "E0658");
    assert_eq!(
        escape.title,
        "Borrow-Bound Closure Cannot Enter Owned Storage"
    );
}

#[test]
fn returned_writable_closure_carriers_do_not_alias_their_capture_root() {
    let analysis = analyze(
        r#"
function bind(writable int $value): function writable(): int
{
    return function (): int with (writable $value) {
        $value += 1;
        return $value;
    };
}

function main(): void
{
    let writable $value = 40;
    writable function writable(): int $callback = bind($value);
    echo "{$callback()}\n";
    echo "{$value}\n";
}
"#,
    );
    assert!(
        language_errors(&analysis.diagnostics).is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn target_neutral_analysis_reports_ownership_without_execution_boundaries() {
    let invalid = analyze(
        r#"
class Payload {}
function main(): void
{
    let $value = new Payload();
    let $first = fn() with (take $value) => $value;
    let $second = fn() with (take $value) => $value;
}
"#,
    );
    diagnostic(&invalid.diagnostics, "E0655");
    assert!(invalid
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0641"));

    let valid = analyze("function main(): void { let $value = fn() => 1; }");
    assert!(valid.diagnostics.is_empty(), "{:#?}", valid.diagnostics);
}

#[test]
fn type_only_function_syntax_stays_below_the_execution_boundary() {
    let analysis = analyze("function accept(function(int): int $callback): void {}");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis.info.closure_ownership.is_empty());
}

#[test]
fn constructor_this_capture_obeys_definite_initialization() {
    let incomplete = analyze(
        r#"
class Box
{
    int $value;

    function __construct()
    {
        let $read = fn() with ($this) => $this->value;
        $this->value = 1;
    }
}
"#,
    );
    let diagnostic = diagnostic(&incomplete.diagnostics, "E0503");
    assert!(
        diagnostic.message.contains("cannot be observed"),
        "{diagnostic:#?}"
    );

    let complete = analyze(
        r#"
class Box
{
    int $value;

    function __construct()
    {
        $this->value = 1;
        let $read = fn() with ($this) => $this->value;
    }
}
"#,
    );
    assert!(
        complete
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "E0503"),
        "{:#?}",
        complete.diagnostics
    );

    let no_capture = analyze(
        r#"
class Box
{
    int $value;

    function __construct()
    {
        let $constant = fn() => 1;
        $this->value = 1;
    }
}
"#,
    );
    assert!(
        no_capture
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "E0503"),
        "{:#?}",
        no_capture.diagnostics
    );
}

#[test]
fn constructor_closure_body_does_not_inherit_construction_root() {
    let analysis = analyze(
        r#"
class Box
{
    int $value;

    function __construct()
    {
        let $initialize = function (): void with ($this) {
            $this->value = 1;
        };
        $this->value = 2;
    }
}
"#,
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "E0201" || diagnostic.code == "E0202" }),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn nullable_replacement_releases_old_capture_only_after_rhs_evaluation() {
    let released = analyze(
        r#"
function main(): void
{
    let writable $value = 1;
    writable ?function(): int $read = fn() with ($value) => $value;
    $read = null;
    $value = 2;
}
"#,
    );
    assert!(
        language_errors(&released.diagnostics).is_empty(),
        "{:#?}",
        released.diagnostics
    );

    let replacement = analyze(
        r#"
function main(): void
{
    let writable $value = 1;
    writable ?function(): int $read = fn() with ($value) => $value;
    $read = function (): int with (writable $value) { return $value; };
}
"#,
    );
    diagnostic(&replacement.diagnostics, "E0654");
}

#[test]
fn once_consumption_is_path_sensitive_across_branches_and_loops() {
    let branch = analyze(
        r#"
class Payload {}
function main(bool $condition): void
{
    let $payload = new Payload();
    let $once = function (): Payload with (take $payload) { return $payload; };
    if ($condition) {
        $once();
    }
    $once();
}
"#,
    );
    let branch_diagnostic = diagnostic(&branch.diagnostics, "E0655");
    assert!(
        branch_diagnostic.title.contains("May Already Be Consumed"),
        "{branch_diagnostic:#?}"
    );

    let looped = analyze(
        r#"
class Payload {}
function main(bool $condition): void
{
    let $payload = new Payload();
    let $once = function (): Payload with (take $payload) { return $payload; };
    while ($condition) {
        $once();
    }
}
"#,
    );
    diagnostic(&looped.diagnostics, "E0655");
}

#[test]
fn approved_owned_storage_moves_function_values_once() {
    let analysis = analyze(
        r#"
class Store
{
    writable function(): int $callback = fn() => 0;

    writable function retain(take function(): int $callback): void
    {
        $this->callback = $callback;
        $callback;
    }
}

function main(): void
{
    let $callback = fn() => 1;
    List<function(): int> $items = [$callback];
    $callback;
}
"#,
    );
    let moved = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0655")
        .count();
    assert_eq!(moved, 2, "{:#?}", analysis.diagnostics);
    assert!(analysis
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0472"));
}

#[test]
fn mixed_function_storage_is_complete_while_static_and_shared_storage_remain_deferred() {
    let mixed = analyze(
        r#"
function inspect(mixed $value): void {}
function consume(take mixed $value): void {}
function boxed(): mixed { return fn() => 13; }
function main(): void
{
    let $callback = fn() => 42;
    inspect($callback);
    $callback();
    consume(fn() => 1);
    mixed $returned = boxed();
    writable mixed $stored = fn() => 2;
    $stored = fn() => 3;
}
"#,
    );
    assert!(
        language_errors(&mixed.diagnostics).is_empty(),
        "{:#?}",
        mixed.diagnostics
    );

    let static_storage = analyze(
        r#"
class Callbacks
{
    static writable function(): int $callback = fn() => 0;
}
"#,
    );
    let static_boundary = diagnostic(&static_storage.diagnostics, "E0661");
    assert!(static_boundary.development_only);
    assert!(static_boundary
        .title
        .contains("Static Function-Value Storage"));

    let shared =
        analyze("function main(): void { let $shared = new WritableSharedReference(fn() => 1); }");
    let shared_boundary = diagnostic(&shared.diagnostics, "E0661");
    assert!(shared_boundary.development_only);
    assert!(shared_boundary
        .title
        .contains("Shared Function-Value Payload"));
}

#[test]
fn borrow_bound_storage_reports_lifetime_before_representation_boundaries() {
    let mixed = analyze(
        r#"
function main(): void
{
    let $value = 1;
    mixed $boxed = fn() with ($value) => $value;
}
"#,
    );
    diagnostic(&mixed.diagnostics, "E0658");
    assert!(mixed
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0661"));

    let shared = analyze(
        r#"
function main(): void
{
    let $value = 1;
    let $shared = new WritableSharedReference(fn() with ($value) => $value);
}
"#,
    );
    diagnostic(&shared.diagnostics, "E0658");
    assert!(shared
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0661"));
}

#[test]
fn enum_payloads_accept_only_fully_owned_function_values() {
    let owned = analyze(
        r#"
enum Work
{
    case Run(function(): int $callback);
}
function main(): void
{
    let $callback = fn() => 1;
    let $work = Work::Run($callback);
    $callback;
}
"#,
    );
    diagnostic(&owned.diagnostics, "E0655");

    let borrowed = analyze(
        r#"
enum Work
{
    case Run(function(): int $callback);
}
function main(): void
{
    let $value = 1;
    let $work = Work::Run(fn() with ($value) => $value);
}
"#,
    );
    diagnostic(&borrowed.diagnostics, "E0658");
}

#[test]
fn nested_function_capture_preserves_transitive_borrow_provenance() {
    let analysis = analyze(
        r#"
function main(): void
{
    let $value = 1;
    let $inner = fn() with ($value) => $value;
    let $outer = fn() with (take $inner) => $inner();
}
"#,
    );
    let mut plans = analysis.info.closure_ownership.values().collect::<Vec<_>>();
    plans.sort_by_key(|plan| plan.closure_id.start);
    assert_eq!(plans.len(), 2, "{:#?}", analysis.diagnostics);
    assert!(matches!(
        plans[1].provenance,
        ClosureValueProvenance::BorrowBound(_)
    ));
    assert!(!plans[1].acquisitions[0].roots.is_empty());
}

#[test]
fn warnings_do_not_suppress_capture_acquisition_or_ownership_errors() {
    let analysis = analyze(
        r#"
class Payload {}
function main(): void
{
    let $payload = new Payload();
    let $unused = fn() with (take $payload) => 1;
    $payload;
}
"#,
    );
    diagnostic(&analysis.diagnostics, "E0646");
    diagnostic(&analysis.diagnostics, "E0470");
    assert!(analysis
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0641"));
}

#[test]
fn direct_closure_invocation_is_valid_in_target_neutral_analysis() {
    let analysis = analyze("function main(): void { (fn() => 1)(); }");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn returned_function_borrow_from_temporary_receiver_is_not_owned() {
    let analysis = analyze(
        r#"
class Factory
{
    function callback(): function(): int
    {
        return fn() with ($this) => 1;
    }
}

function invalid(): function(): int
{
    return (new Factory())->callback();
}
"#,
    );
    diagnostic(&analysis.diagnostics, "E0658");
}

#[test]
fn no_capture_closures_are_owned_move_values_with_empty_plans() {
    let analysis = analyze(
        r#"
function main(): void
{
    let $first = fn() => 1;
    let $second = $first;
    $first;
    $second();
}
"#,
    );
    diagnostic(&analysis.diagnostics, "E0655");
    let plan = analysis.info.closure_ownership.values().next().unwrap();
    assert_eq!(plan.provenance, ClosureValueProvenance::Owned);
    assert!(plan.acquisitions.is_empty());
    assert!(plan.release_order.is_empty());
}

#[test]
fn invalid_capture_plans_are_atomic_and_never_used_leases_end_early() {
    let invalid = analyze(
        r#"
class Payload {}
function main(): void
{
    let $payload = new Payload();
    let writable $value = 1;
    let $read = fn() with ($value) => $value;
    let $invalid = fn() with (take $payload, writable $value) => $value;
    $payload;
    $read();
}
"#,
    );
    diagnostic(&invalid.diagnostics, "E0654");
    assert!(invalid.diagnostics.iter().all(|diagnostic| {
        !(diagnostic.code == "E0470" && diagnostic.message.contains("payload"))
    }));

    let never_used = analyze(
        r#"
function main(): void
{
    let writable $value = 1;
    let $unused = fn() with ($value) => $value;
    $value = 2;
}
"#,
    );
    assert!(
        language_errors(&never_used.diagnostics).is_empty(),
        "{:#?}",
        never_used.diagnostics
    );
}

#[test]
fn owned_callbacks_can_be_retained_but_borrowed_callbacks_and_receiver_cycles_cannot() {
    let owned = analyze(
        r#"
class Store
{
    function(): int $callback;
    function __construct(take function(): int $callback)
    {
        $this->callback = $callback;
    }
}
function main(): void
{
    let $callback = fn() => 1;
    let $store = new Store($callback);
    $callback;
}
"#,
    );
    diagnostic(&owned.diagnostics, "E0655");
    assert!(owned
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.code != "E0657"));

    let borrowed = analyze(
        r#"
class Store
{
    function(): int $callback;
    function __construct(function(): int $callback)
    {
        $this->callback = $callback;
    }
}
"#,
    );
    diagnostic(&borrowed.diagnostics, "E0657");

    let cycle = analyze(
        r#"
class Box
{
    writable function(): int $callback = fn() => 0;
    writable function install(): void
    {
        $this->callback = fn() with ($this) => 1;
    }
}
"#,
    );
    let cycle = diagnostic(&cycle.diagnostics, "E0658");
    assert_eq!(cycle.title, "Closure Cannot Borrow Its Stored Receiver");
}

#[test]
fn returned_single_roots_are_preserved_and_unrelated_roots_are_rejected() {
    let readonly = analyze(
        r#"
function bind(int $value): function(): int
{
    return fn() with ($value) => $value;
}
"#,
    );
    assert!(
        language_errors(&readonly.diagnostics).is_empty(),
        "{:#?}",
        readonly.diagnostics
    );
    let plan = readonly.info.closure_ownership.values().next().unwrap();
    assert_eq!(plan.escape, ClosureEscapeClassification::ReturnedBorrow);
    assert!(matches!(
        plan.provenance,
        ClosureValueProvenance::BorrowBound(ref roots) if roots.len() == 1
    ));

    let unrelated = analyze(
        r#"
class Box
{
    function bind(int $value): function(): int
    {
        return fn() with ($this, $value) => $value;
    }
}
"#,
    );
    diagnostic(&unrelated.diagnostics, "E0659");
}

#[test]
fn once_consumption_survives_checked_failure_and_is_structured_in_json() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }
class Payload {}
function main(): void
{
    let $payload = new Payload();
    let $once = function (): Payload with (take $payload) {
        if (true) { throw new Failure("failed"); }
        return $payload;
    };
    try { $once(); } catch (Failure) {}
    $once();
}
"#;
    let analysis = analyze(source);
    let consumed = diagnostic(&analysis.diagnostics, "E0655");
    assert!(consumed.cause_id.is_some(), "{consumed:#?}");
    assert!(!consumed.labels.is_empty(), "{consumed:#?}");

    let rendered = doriac::render_diagnostics_with_options(
        "stage30c.doria",
        source,
        &analysis.diagnostics,
        RenderOptions {
            format: DiagnosticFormat::Json,
            ..RenderOptions::default()
        },
    );
    let envelope: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    let value = envelope["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["code"] == "E0655")
        .unwrap();
    assert_eq!(value["kind"], "language");
    assert_eq!(value["severity"], "error");
    assert!(value["causeId"].is_string());
    assert!(!value["labels"].as_array().unwrap().is_empty());
    assert!(value["fixes"].is_array());
}

#[test]
fn ownership_metadata_scales_deterministically_without_runtime_layout() {
    let mut source = String::from("function main(): void\n{\n");
    for index in 0..64 {
        source.push_str(&format!("    let $value{index} = {index};\n"));
    }
    source.push_str("    let $wide = fn() with (");
    for index in 0..64 {
        if index > 0 {
            source.push_str(", ");
        }
        source.push_str(&format!("$value{index}"));
    }
    source.push_str(") => 1;\n");
    source.push_str("    let $nested0 = fn() with (take $wide) => $wide();\n");
    for index in 1..24 {
        source.push_str(&format!(
            "    let $nested{index} = fn() with (take $nested{}) => $nested{}();\n",
            index - 1,
            index - 1,
        ));
    }
    source.push_str("}\n");

    let first = analyze(&source);
    let second = analyze(&source);
    assert_eq!(first.info.closure_ownership, second.info.closure_ownership);
    assert_eq!(first.info.closure_ownership.len(), 25);
    let wide = first
        .info
        .closure_ownership
        .values()
        .find(|plan| plan.acquisitions.len() == 64)
        .unwrap();
    assert_eq!(wide.release_order.first(), Some(&63));
    assert_eq!(wide.release_order.last(), Some(&0));
}
