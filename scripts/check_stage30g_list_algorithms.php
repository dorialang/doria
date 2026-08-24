<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage30g_list_algorithms(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 30g file is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 30g contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: forbidden Stage 30g drift `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md',
        'spec' => 'SPEC.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'stdlib' => 'docs/stdlib-reference.md',
        'audit' => 'docs/notes/temporary-language-restrictions-audit.md',
        'catalogue' => 'crates/doria-diagnostic-catalogue/src/lib.rs',
        'semantics' => 'crates/doriac/src/semantics.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'lowering' => 'crates/doriac/src/mir_lowering.rs',
        'validation' => 'crates/doriac/src/mir_validation.rs',
        'php' => 'crates/doriac/src/codegen_php.rs',
        'tests' => 'crates/doriac/tests/stage30g_list_algorithm_tests.rs',
        'nativeManifest' => 'crates/doriac/tests/fixtures/native_closures/manifest.txt',
        'phpManifest' => 'crates/doriac/tests/fixtures/php_closures/manifest.txt',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    foreach (['decision', 'spec', 'plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 30g List Algorithms',
            'Complete',
            'Stage 30h Cross-Repository Closure',
            'Next',
            'Stage 30',
            'Not Complete',
        ]);
    }
    $forbid($paths['decision'], $files['decision'], ['Stage 30g Next']);
    $require($paths['stdlib'], $files['stdlib'], [
        'map<U>(function(T): U $transform): List<U>',
        'filter(function(T): bool $predicate): List<T>',
        'reduce<A>(take A $initial, function(writable A, T): void $reducer): A',
        'No other collection family receives these algorithms in Stage 30',
    ]);
    $require($paths['audit'], $files['audit'], [
        'Implemented for `map`, Copy-only `filter`, and writable-accumulator `reduce`',
        'E0664-E0668',
    ]);

    $require($paths['catalogue'], $files['catalogue'], [
        '"E0641"', '"E0664"', '"E0665"', '"E0666"', '"E0667"', '"E0668"', '"I3002"',
    ]);
    $require($paths['semantics'], $files['semantics'], [
        'pub struct ListAlgorithmCallInfo',
        'list_algorithm_calls: HashMap',
        'matches!(kind, TypeKind::List(_)) && matches!(method, "map" | "filter" | "reduce")',
        'fn check_list_algorithm_call(',
        'fn report_list_callback_mismatch(',
        'ListCallbackAccess::Readonly',
        'ListCallbackAccess::Writable',
        '"E0664"', '"E0665"', '"E0666"', '"E0667"', '"E0668"',
    ]);
    $require($paths['hir'], $files['hir'], [
        'pub struct ListAlgorithmCall',
        'ListAlgorithmCall(Box<ListAlgorithmCall>)',
    ]);
    $require($paths['mir'], $files['mir'], [
        'ListAlgorithm(Box<ListAlgorithmPlan>)',
        'pub struct ListAlgorithmPlan',
        'pub checked_effects: Vec<CheckedEffect>',
        'pub callback_failure: Option<BlockId>',
    ]);
    $require($paths['lowering'], $files['lowering'], [
        'fn materialize_list_algorithm_call(',
        'Terminator::IndirectCall',
        'Terminator::CheckedIndirectCall',
        'ControlFlowPlan::ListAlgorithm',
        'Statement::CollectionAdd',
    ]);
    $require($paths['validation'], $files['validation'], [
        'fn validate_list_algorithm_cfg(',
        'fn validate_list_algorithm_checked_cleanup(',
        'fn validate_list_algorithm_region_does_not_mutate_sources(',
        'List algorithm traversal cannot mutate its source or consume its callback',
    ]);
    $require($paths['php'], $files['php'], [
        'fn emit_list_algorithm_call(',
        'Expr::ListAlgorithmCall(call)',
        'foreach',
        '__DoriaCell',
    ]);
    $forbid($paths['php'], $files['php'], ['array_map(', 'array_filter(', 'array_reduce(']);

    $require($paths['tests'], $files['tests'], [
        'semantic_plans_are_concrete_list_only_and_preserve_callback_access',
        'semantic_diagnostics_enforce_callback_and_collection_boundaries',
        'expected_context_specializes_empty_collection_results_and_accumulators',
        'source_list_borrow_uses_capture_provenance_and_ends_after_the_call',
        'checked_algorithm_cfg_reinitializes_loop_results_and_cleans_owned_state',
        'mir_validation_rejects_corrupt_traversal_results_and_checked_cleanup',
        'php_uses_explicit_ordered_loops_and_not_host_higher_order_functions',
    ]);
    $require($paths['nativeManifest'], $files['nativeManifest'], [
        'stage30g_list_algorithms',
        'stage30g_checked_cleanup',
    ]);
    $require($paths['phpManifest'], $files['phpManifest'], [
        'stage30g_list_algorithms',
        'stage30g_checked_algorithms',
    ]);

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage30g_list_algorithms(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 30g List algorithm check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 30g List algorithm check passed\n");
}
