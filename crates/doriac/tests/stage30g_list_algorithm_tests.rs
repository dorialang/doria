use doriac::diagnostics::{Diagnostic, DiagnosticSeverity};
use doriac::hir;
use doriac::mir::{self, ControlFlowPlan, ListAlgorithmKind, Statement};

fn analyze(source: &str) -> doriac::semantics::SemanticAnalysis {
    let (_, analysis) = doriac::analyze_source_for_ide("stage30g.doria", source)
        .expect("Stage 30g source should parse");
    analysis
}

fn error_codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn algorithm_plans(program: &mir::Program) -> Vec<&mir::ListAlgorithmPlan> {
    program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            Statement::ControlFlowPlan(ControlFlowPlan::ListAlgorithm(plan)) => Some(plan.as_ref()),
            _ => None,
        })
        .collect()
}

fn valid_source() -> &'static str {
    r#"
function main(): void
{
    List<int> $values = [1, 2, 3, 4];
    List<int> $mapped = $values->map(fn(int $value) => $value * 2);
    List<int> $filtered = $values->filter(fn(int $value) => $value > 2);
    int $total = $values->reduce(
        0,
        function (writable int $sum, int $value): void {
            $sum += $value;
        },
    );

    foreach ($mapped as int $value) { echo "m={$value}\n"; }
    foreach ($filtered as int $value) { echo "f={$value}\n"; }
    echo "sum={$total}\n";
}
"#
}

#[test]
fn semantic_plans_are_concrete_list_only_and_preserve_callback_access() {
    let source = r#"
function main(): void
{
    List<int> $values = [1, 2, 3];
    let writable $calls = 0;
    let writable $transform = function (int $value): string with (writable $calls) {
        $calls += 1;
        return "{$value}";
    };
    List<string> $mapped = $values->map($transform);
    List<int> $filtered = $values->filter(fn(int $value) => $value > 1);
    int $total = $values->reduce(0, function (writable int $sum, int $value): void {
        $sum += $value;
    });
}
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.list_algorithm_calls.len(), 3);
    assert!(analysis.info.list_algorithm_calls.values().any(|plan| {
        plan.kind == doriac::semantics::ListAlgorithmKind::Map
            && plan.callback_access == doriac::semantics::ListCallbackAccess::Writable
            && plan.result_type
                == doriac::types::ResolvedType::List(Box::new(doriac::types::ResolvedType::String))
    }));
}

#[test]
fn semantic_diagnostics_enforce_callback_and_collection_boundaries() {
    let once = analyze(
        r#"
class Token {}
function main(): void {
    List<int> $values = [1];
    let $token = new Token();
    function once(int): int $callback = function (int $value): int with (take $token) {
        let $owned = $token;
        return $value;
    };
    let $mapped = $values->map($callback);
}
"#,
    );
    assert!(
        error_codes(&once.diagnostics).contains(&"E0664"),
        "{:#?}",
        once.diagnostics
    );

    let readonly_writable = analyze(
        r#"
function main(): void {
    List<int> $values = [1];
    let writable $calls = 0;
    let $callback = function (int $value): int with (writable $calls) {
        $calls += 1;
        return $value;
    };
    let $mapped = $values->map($callback);
}
"#,
    );
    assert!(
        error_codes(&readonly_writable.diagnostics).contains(&"E0668"),
        "{:#?}",
        readonly_writable.diagnostics
    );

    let move_filter = analyze(
        r#"
class Item {}
function main(): void {
    List<Item> $items = [new Item()];
    let $filtered = $items->filter(fn(Item $item) => true);
}
"#,
    );
    assert!(
        error_codes(&move_filter.diagnostics).contains(&"E0666"),
        "{:#?}",
        move_filter.diagnostics
    );

    let wrong_shape = analyze(
        r#"
function main(): void {
    List<int> $values = [1];
    let $reduced = $values->reduce(0, fn(int $sum, int $value) => $sum + $value);
}
"#,
    );
    assert!(
        error_codes(&wrong_shape.diagnostics).contains(&"E0665"),
        "{:#?}",
        wrong_shape.diagnostics
    );

    let named = analyze(
        r#"
function main(): void {
    List<int> $values = [1];
    let $mapped = $values->map(transform: fn(int $value) => $value);
}
"#,
    );
    assert!(
        error_codes(&named.diagnostics).contains(&"E0519"),
        "{:#?}",
        named.diagnostics
    );

    let other_collection = analyze(
        r#"
function main(): void {
    Set<int> $values = Set::from([1]);
    let $mapped = $values->map(fn(int $value) => $value);
}
"#,
    );
    assert!(
        error_codes(&other_collection.diagnostics).contains(&"E0521"),
        "{:#?}",
        other_collection.diagnostics
    );
}

#[test]
fn source_list_borrow_uses_capture_provenance_and_ends_after_the_call() {
    let readonly = analyze(
        r#"
function main(): void {
    writable List<int> $values = [1, 2];
    let $callback = fn(int $value) with ($values) => $value + $values->count;
    let $mapped = $values->map($callback);
    $values->add(3);
}
"#,
    );
    assert!(
        readonly.diagnostics.is_empty(),
        "{:#?}",
        readonly.diagnostics
    );

    let writable = analyze(
        r#"
function main(): void {
    writable List<int> $values = [1, 2];
    let writable $callback = function (int $value): int with (writable $values) {
        $values->add($value);
        return $value;
    };
    let $mapped = $values->map($callback);
}
"#,
    );
    assert!(
        error_codes(&writable.diagnostics).contains(&"E0654"),
        "{:#?}",
        writable.diagnostics
    );
}

#[test]
fn mir_plans_validate_and_reject_corrupted_algorithm_authority() {
    let program = doriac::lower_source_to_mir("stage30g.doria", valid_source())
        .expect("valid List algorithms should lower");
    doriac::mir_validation::validate_program(&program).expect("valid algorithm MIR should pass");
    let plans = algorithm_plans(&program);
    assert_eq!(plans.len(), 3);
    assert!(plans.iter().any(|plan| plan.kind == ListAlgorithmKind::Map));
    assert!(plans
        .iter()
        .any(|plan| plan.kind == ListAlgorithmKind::Filter));
    assert!(plans
        .iter()
        .any(|plan| plan.kind == ListAlgorithmKind::Reduce));

    let mut non_list = program.clone();
    let map = non_list
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            Statement::ControlFlowPlan(ControlFlowPlan::ListAlgorithm(plan))
                if plan.kind == ListAlgorithmKind::Map =>
            {
                Some(plan)
            }
            _ => None,
        })
        .expect("map plan should exist");
    map.element_type = mir::Type::String;
    let error = doriac::mir_validation::validate_program(&non_list)
        .expect_err("corrupt List algorithm MIR must fail");
    assert!(
        error.message.contains("source metadata"),
        "{}",
        error.message
    );

    let mut once = program;
    let plan = once
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            Statement::ControlFlowPlan(ControlFlowPlan::ListAlgorithm(plan)) => Some(plan),
            _ => None,
        })
        .expect("algorithm plan should exist");
    plan.callback_access = mir::FunctionInvocationMode::Once;
    let error =
        doriac::mir_validation::validate_program(&once).expect_err("once algorithm MIR must fail");
    assert!(error.message.contains("callback mode"), "{}", error.message);
}

#[test]
fn mir_validation_rejects_corrupt_traversal_results_and_checked_cleanup() {
    let program = doriac::lower_source_to_mir("stage30g.doria", valid_source())
        .expect("valid List algorithms should lower");

    let mut traversal = program.clone();
    let plan = traversal
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            Statement::ControlFlowPlan(ControlFlowPlan::ListAlgorithm(plan))
                if plan.kind == ListAlgorithmKind::Map =>
            {
                Some(plan)
            }
            _ => None,
        })
        .expect("map plan should exist");
    plan.count = plan.index;
    let error = doriac::mir_validation::validate_program(&traversal)
        .expect_err("corrupt traversal locals must fail");
    assert!(
        error.message.contains("count and index"),
        "{}",
        error.message
    );

    let mut result = program;
    let plan = result
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            Statement::ControlFlowPlan(ControlFlowPlan::ListAlgorithm(plan))
                if plan.kind == ListAlgorithmKind::Filter =>
            {
                Some(plan)
            }
            _ => None,
        })
        .expect("filter plan should exist");
    plan.callback_result = None;
    let error = doriac::mir_validation::validate_program(&result)
        .expect_err("missing predicate result must fail");
    assert!(
        error.message.contains("Copy-preserving shape"),
        "{}",
        error.message
    );

    let checked_source =
        include_str!("fixtures/native_closures/stage30g_checked_cleanup/source.doria");
    let mut checked = doriac::lower_source_to_mir("stage30g-checked.doria", checked_source)
        .expect("checked algorithms should lower");
    let (function_index, map_plan) = checked
        .functions
        .iter()
        .enumerate()
        .find_map(|(function_index, function)| {
            function.blocks.iter().find_map(|block| {
                block
                    .statements
                    .iter()
                    .find_map(|statement| match statement {
                        Statement::ControlFlowPlan(ControlFlowPlan::ListAlgorithm(plan))
                            if plan.kind == ListAlgorithmKind::Map =>
                        {
                            Some((function_index, plan.clone()))
                        }
                        _ => None,
                    })
            })
        })
        .expect("checked map plan should exist");
    let output = map_plan.output.expect("map output exists");
    let failure = map_plan.callback_failure.expect("checked failure exists");
    checked.functions[function_index].blocks[failure.0]
        .statements
        .retain(|statement| {
            !matches!(
                statement,
                Statement::DropCollection { local, .. } if *local == output
            )
        });
    let error = doriac::mir_validation::validate_program(&checked)
        .expect_err("missing partial-result cleanup must fail");
    assert!(
        error.message.contains("partial-result cleanup count"),
        "{}",
        error.message
    );
}

#[test]
fn ambient_list_callbacks_keep_checked_transport_and_validate_their_effect_profile() {
    let source = r#"
function main(): void
{
    List<int> $values = [1, 2];
    List<int> $mapped = $values->map(function (int $value): int {
        echo "{$value}";
        return $value;
    });
}
"#;
    let program = doriac::lower_source_to_mir("stage30g-ambient.doria", source)
        .expect("ambient List callback should lower");
    let plan = algorithm_plans(&program)
        .into_iter()
        .find(|plan| plan.kind == ListAlgorithmKind::Map)
        .expect("ambient map plan should exist");
    assert!(plan.required_checked_effects.is_empty());
    assert!(!plan.ambient_checked_effects.is_empty());
    assert_eq!(plan.checked_effects, plan.ambient_checked_effects);
    assert!(plan.callback_failure.is_some());
    let callback = &program.function_types[plan.callback_type.0];
    assert!(callback.checked_effects.is_empty());
    assert_eq!(
        callback.ambient_checked_effects,
        plan.ambient_checked_effects
    );
    assert!(matches!(
        program
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .find(|block| block.id == plan.body)
            .map(|block| &block.terminator),
        Some(mir::Terminator::CheckedIndirectCall { .. })
    ));
    doriac::mir_validation::validate_program(&program)
        .expect("ambient List callback MIR should validate");

    let mut malformed = program;
    let plan = malformed
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match statement {
            Statement::ControlFlowPlan(ControlFlowPlan::ListAlgorithm(plan))
                if plan.kind == ListAlgorithmKind::Map =>
            {
                Some(plan)
            }
            _ => None,
        })
        .expect("ambient map plan should exist");
    plan.ambient_checked_effects.clear();
    let error = doriac::mir_validation::validate_program(&malformed)
        .expect_err("ambient profile loss must be rejected");
    assert!(
        error.message.contains("effect profile disagrees"),
        "{}",
        error.message
    );
}

#[test]
fn debug_interpreter_executes_shared_list_algorithm_cfg() {
    let output = doriac::compile_source_to_debug("stage30g.doria", valid_source())
        .expect("Stage 30g algorithms should execute through debug MIR");
    assert_eq!(
        output,
        "exit_status: 0\nstdout: m=2\nm=4\nm=6\nm=8\nf=3\nf=4\nsum=10\n\n"
    );
}

#[test]
fn expected_context_specializes_empty_collection_results_and_accumulators() {
    let source = r#"
function main(): void
{
    List<int> $values = [1, 2];
    List<List<int>> $mapped = $values->map(fn(int $value) => []);
    List<int> $reduced = $values->reduce(
        [],
        function (writable List<int> $result, int $value): void {
            $result->add($value);
        },
    );
}
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let expected_list = doriac::types::ResolvedType::List(Box::new(
        doriac::types::ResolvedType::Integer(doriac::numeric::IntegerType::Int64),
    ));
    let map = analysis
        .info
        .list_algorithm_calls
        .values()
        .find(|call| call.kind == doriac::semantics::ListAlgorithmKind::Map)
        .expect("map plan should exist");
    assert_eq!(
        map.result_type,
        doriac::types::ResolvedType::List(Box::new(expected_list.clone()))
    );
    let reduce = analysis
        .info
        .list_algorithm_calls
        .values()
        .find(|call| call.kind == doriac::semantics::ListAlgorithmKind::Reduce)
        .expect("reduce plan should exist");
    assert_eq!(reduce.accumulator_type, Some(expected_list.clone()));
    assert_eq!(reduce.result_type, expected_list);
}

#[test]
fn hir_preserves_concrete_algorithm_facts_and_checked_effects() {
    let source = r#"
class Failure implements Error
{
    function __construct(string $message) {}
}

function main(): void
{
    List<int> $values = [1];
    try {
        List<string> $mapped = $values->map(function (int $value): string {
            throw new Failure("failed");
        });
    } catch (Failure $error) {
    }
}
"#;
    let program = doriac::lower_source("stage30g-hir.doria", source)
        .expect("checked List::map should lower to HIR");
    let main = program
        .items
        .iter()
        .find_map(|item| match item {
            hir::Item::Function(function) if function.name == "main" => Some(function),
            _ => None,
        })
        .expect("main should exist");
    let map = main
        .body
        .statements
        .iter()
        .find_map(|statement| match statement {
            hir::Stmt::Try(statement) => statement.body.statements.iter().find_map(|statement| {
                let hir::Stmt::VarDecl(declaration) = statement else {
                    return None;
                };
                match &declaration.initializer {
                    hir::Expr::ListAlgorithmCall(call) => Some(call.as_ref()),
                    _ => None,
                }
            }),
            _ => None,
        })
        .expect("map should be explicit in HIR");
    assert_eq!(map.kind, hir::ListAlgorithmKind::Map);
    assert_eq!(map.callback_access, hir::ListCallbackAccess::Readonly);
    assert_eq!(map.required_checked_effects.len(), 1);
    assert_eq!(map.ambient_checked_effects.len(), 2);
    let mut complete_effects = map.required_checked_effects.clone();
    complete_effects.extend(map.ambient_checked_effects.iter().cloned());
    assert_eq!(map.checked_effects, complete_effects);
    assert_eq!(
        map.result_type,
        doriac::types::ResolvedType::List(Box::new(doriac::types::ResolvedType::String))
    );
    assert!(map.receiver_span.start < map.callback_span.start);
}

#[test]
fn nullable_and_callback_ownership_boundaries_are_precise() {
    let nullable = analyze(
        r#"
function inspect(?List<int> $values): void {
    let $invalid = $values->map(fn(int $value) => $value);
    if ($values != null) {
        List<int> $valid = $values->map(fn(int $value) => $value);
    }
}
function main(): void {}
"#,
    );
    assert_eq!(
        nullable.info.list_algorithm_calls.len(),
        1,
        "only the narrowed receiver should produce an algorithm plan"
    );
    assert!(!error_codes(&nullable.diagnostics).is_empty());

    let borrowed_result = analyze(
        r#"
class Item {}
function main(): void {
    List<Item> $items = [new Item()];
    List<Item> $same = $items->map(fn(Item $item) => $item);
}
"#,
    );
    assert_eq!(error_codes(&borrowed_result.diagnostics), vec!["E0667"]);

    let readonly_in_writable_binding = analyze(
        r#"
function main(): void {
    List<int> $values = [1];
    let writable $callback = fn(int $value) => $value;
    List<int> $mapped = $values->map($callback);
}
"#,
    );
    assert!(readonly_in_writable_binding.diagnostics.is_empty());
    assert!(readonly_in_writable_binding
        .info
        .list_algorithm_calls
        .values()
        .all(|call| call.callback_access == doriac::semantics::ListCallbackAccess::Readonly));

    let writable_inline = analyze(
        r#"
function main(): void {
    List<int> $values = [1];
    let writable $calls = 0;
    List<int> $mapped = $values->map(function (int $value): int with (writable $calls) {
        $calls += 1;
        return $value;
    });
}
"#,
    );
    assert!(writable_inline.diagnostics.is_empty());
    assert!(writable_inline
        .info
        .list_algorithm_calls
        .values()
        .all(|call| { call.callback_access == doriac::semantics::ListCallbackAccess::Writable }));
}

#[test]
fn checked_algorithm_cfg_reinitializes_loop_results_and_cleans_owned_state() {
    let source = include_str!("fixtures/native_closures/stage30g_checked_cleanup/source.doria");
    let program = doriac::lower_source_to_mir("stage30g-checked-cleanup.doria", source)
        .expect("checked Move-result algorithms should produce valid loop MIR");
    let plans = algorithm_plans(&program);
    assert_eq!(plans.len(), 2);
    assert!(plans.iter().all(|plan| !plan.checked_effects.is_empty()));
    assert!(program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .any(|block| matches!(
            block.terminator,
            mir::Terminator::CheckedIndirectCall { .. }
        )));
    let interpreted = doriac::mir_interpreter::interpret(&program)
        .expect("checked Stage 30g cleanup fixture should execute");
    assert_eq!(
        String::from_utf8(interpreted.stdout).expect("fixture output is UTF-8"),
        include_str!("fixtures/native_closures/stage30g_checked_cleanup/expected_stdout")
    );
    assert_eq!(interpreted.exit_status, 0);
}

#[test]
fn php_uses_explicit_ordered_loops_and_not_host_higher_order_functions() {
    let source = include_str!("fixtures/php_closures/stage30g_list_algorithms/source.doria");
    let php = doriac::compile_source_to_php("stage30g-php.doria", source)
        .expect("supported Stage 30g surface should compile to PHP");
    for forbidden in [
        "array_map",
        "array_filter",
        "array_reduce",
        "call_user_func",
        "call_user_func_array",
    ] {
        assert!(!php.contains(forbidden), "generated PHP used `{forbidden}`");
    }
    assert!(php.contains("foreach ($__doriaAlgorithmSource as $__doriaAlgorithmElement)"));
    assert!(php.contains("catch (__DoriaCheckedError $__doriaAlgorithmError)"));
    assert!(php.contains("__doria_drop_cell($__doriaAlgorithmAccumulator)"));
}
