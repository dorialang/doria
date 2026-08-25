<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage30c_ownership(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 30c file is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 30c contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: forbidden Stage 30c drift `{$needle}`";
            }
        }
    };

    $decisionPath = 'docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md';
    $specPath = 'SPEC.md';
    $planPath = 'docs/doria-end-to-end-plan.md';
    $pipelinePath = 'docs/notes/current-pipeline.md';
    $auditPath = 'docs/notes/temporary-language-restrictions-audit.md';
    $ownershipPath = 'crates/doriac/src/ownership.rs';
    $semanticsPath = 'crates/doriac/src/semantics.rs';
    $constructorPath = 'crates/doriac/src/constructor_init.rs';
    $cataloguePath = 'crates/doria-diagnostic-catalogue/src/lib.rs';
    $testsPath = 'crates/doriac/tests/stage30c_ownership_tests.rs';
    $hirPath = 'crates/doriac/src/hir.rs';
    $mirPath = 'crates/doriac/src/mir.rs';

    $decision = $read($decisionPath);
    $spec = $read($specPath);
    $plan = $read($planPath);
    $pipeline = $read($pipelinePath);
    $audit = $read($auditPath);
    $ownership = $read($ownershipPath);
    $semantics = $read($semanticsPath);
    $constructor = $read($constructorPath);
    $catalogue = $read($cataloguePath);
    $tests = $read($testsPath);
    $hir = $read($hirPath);
    $mir = $read($mirPath);

    foreach ([$decisionPath => $decision, $planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
        $require($path, $contents, [
            'Stage 30c Ownership, Lifetime, And Escape — Complete',
            'Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete',
            'Stage 30e Native Execution — Complete',
            'Stage 30f PHP Compatibility — Complete',
            'Stage 30g List Algorithms — Complete',
            'Stage 30 — Complete',
        ]);
        $forbid($path, $contents, ['Stage 30c Ownership, Lifetime, And Escape — Next']);
    }
    $require($decisionPath, $decision, [
        'Stages 30a Through 30h Implemented',
        'acquisition at closure creation in authored order',
        'non-lexical readonly/writable leases',
        'Stage 30d — Complete',
    ]);
    $require($specPath, $spec, [
        'Stage 30c Ownership, Lifetime, And Escape - Complete',
        'Stage 30d Closure HIR/MIR And Interpreter Oracle - Complete',
        'Stage 30e Native Execution - Complete',
        'Stage 30f PHP Compatibility - Complete',
        'Stage 30g List Algorithms - Complete',
        'E0641 - Historical And Reserved',
    ]);
    $require($auditPath, $audit, [
        'Historical and reserved after Stage 30h',
        'no active emitter or generic fallback remains',
    ]);

    $require($ownershipPath, $ownership, [
        'pub enum ClosureBorrowRoot',
        'pub enum ClosureValueProvenance',
        'pub enum CaptureAcquisitionKind',
        'pub struct ClosureOwnershipInfo',
        'pub release_order: Vec<usize>',
        'nonescaping_parameter',
        'ClosureEscapeClassification',
        'InvocationConsumption',
    ]);
    $require($semanticsPath, $semantics, [
        'pub closure_ownership: HashMap<ClosureId, crate::ownership::ClosureOwnershipInfo>',
        'pub callable_value_calls: HashMap<(usize, usize), CallableValueCallInfo>',
    ]);
    $require($constructorPath, $constructor, ['Expr::Closure(closure)', 'report_incomplete_this(']);
    foreach (range(54, 63) as $suffix) {
        $require($cataloguePath, $catalogue, ['"E06' . $suffix . '"']);
    }
    $require($testsPath, $tests, [
        'capture_plans_preserve_authored_acquisition_and_reverse_release_order',
        'capture_leases_end_after_the_closure_last_use',
        'writable_and_once_invocations_enforce_access_and_consumption',
        'nonescaping_callbacks_cannot_cross_retention_boundaries',
        'returned_closures_require_owned_or_one_supported_borrow_root',
        'once_consumption_is_path_sensitive_across_branches_and_loops',
        'nested_function_capture_preserves_transitive_borrow_provenance',
        'warnings_do_not_suppress_capture_acquisition_or_ownership_errors',
        'no_capture_closures_are_owned_move_values_with_empty_plans',
        'invalid_capture_plans_are_atomic_and_never_used_leases_end_early',
        'owned_callbacks_can_be_retained_but_borrowed_callbacks_and_receiver_cycles_cannot',
        'returned_single_roots_are_preserved_and_unrelated_roots_are_rejected',
        'once_consumption_survives_checked_failure_and_is_structured_in_json',
        'ownership_metadata_scales_deterministically_without_runtime_layout',
    ]);

    $require($hirPath, $hir, ['ClosureExpression', 'CallableCall']);
    $require($mirPath, $mir, ['ClosureDescriptor', 'ClosureEnvironmentLayout']);

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage30c_ownership(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 30c ownership check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 30c ownership check passed\n");
}
