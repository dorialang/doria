<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

$read = static function (string $path) use ($root, &$failures): string {
    $contents = @file_get_contents($root . '/' . $path);
    if (!is_string($contents)) {
        $failures[] = "{$path}: required file is missing or unreadable";
        return '';
    }
    return $contents;
};

$require = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (!str_contains($contents, $needle)) {
            $failures[] = "{$path}: missing match-foundation contract `{$needle}`";
        }
    }
};

$decisionPath = 'docs/decisions/0115-match-expressions-patterns-exhaustiveness-narrowing-and-ownership.md';
$planPath = 'docs/doria-end-to-end-plan.md';
$pipelinePath = 'docs/notes/current-pipeline.md';
$stdlibPath = 'docs/stdlib-reference.md';
$specPath = 'SPEC.md';
$cataloguePath = 'crates/doria-diagnostic-catalogue/src/lib.rs';
$parserPath = 'crates/doriac/src/parser.rs';
$semanticsPath = 'crates/doriac/src/semantics.rs';
$mirPath = 'crates/doriac/src/mir.rs';
$loweringPath = 'crates/doriac/src/mir_lowering.rs';
$validationPath = 'crates/doriac/src/mir_validation.rs';
$interpreterPath = 'crates/doriac/src/mir_interpreter.rs';
$craneliftPath = 'crates/doriac/src/codegen_cranelift.rs';
$llvmPath = 'crates/doriac/src/codegen_llvm.rs';
$phpPath = 'crates/doriac/src/codegen_php.rs';
$performancePath = 'crates/doriac/src/performance.rs';
$testsPath = 'crates/doriac/tests/stage28_tests.rs';
$malformedPath = 'crates/doriac/tests/mir_validation_tests.rs';
$llvmTestsPath = 'crates/doriac/tests/llvm_mir_tests.rs';
$manifestPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';

$decision = $read($decisionPath);
$plan = $read($planPath);
$pipeline = $read($pipelinePath);
$stdlib = $read($stdlibPath);
$spec = $read($specPath);
$catalogue = $read($cataloguePath);
$parser = $read($parserPath);
$semantics = $read($semanticsPath);
$mir = $read($mirPath);
$lowering = $read($loweringPath);
$validation = $read($validationPath);
$interpreter = $read($interpreterPath);
$cranelift = $read($craneliftPath);
$llvm = $read($llvmPath);
$php = $read($phpPath);
$performance = $read($performancePath);
$tests = $read($testsPath);
$malformed = $read($malformedPath);
$llvmTests = $read($llvmTestsPath);
$manifest = $read($manifestPath);

$require($decisionPath, $decision, [
    '**Status:** Accepted',
    '**Implementation status:** Implemented',
    '## Expression Shape',
    '## Exhaustiveness',
    '## Payload Destructuring',
    '## Exact Type-Binding Patterns',
    '## Readonly Ownership',
    '## `match (true)`',
    '## Ternary Desugaring',
    '## Pattern Guards',
    'The only guard spelling is `if`',
    '`when` is Doria\'s future value-returning conditional',
    '`where`',
    'must produce `bool`',
    'the pattern is tested first',
    'A runtime-guarded arm does not complete coverage',
    'A later unguarded copy of the pattern may complete',
    '`default` is unconditional and cannot have a guard',
    '`match (true)` arms do not',
    '## Consuming Match',
    '`match (take $value)` transfers the whole scrutinee',
    'Consumption is deliberately whole-value only',
    'Payload-level `take`',
    'writable payload bindings are rejected',
    'a Move payload becomes one owned binding',
    'If it fails, no ownership moves',
    '## Guarded Consumption',
    '## PR #132 Review Closure',
    'Stage 28 Slice 1 — Complete',
    'Stage 28 Slice 2 — Complete',
    'Stage 28 is **Complete**',
    'Decision 0116 now owns the separate Stage 28a',
    'backend-private',
    'former PHP B1301 boundary for valid exact numeric tests after `mixed` is',
    'Stage 28 controlled timing is **Pending Available Runner** and non-blocking',
]);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    $require($path, $contents, [
        'Stage 26b — Complete',
        'Measurement Status: Pending Available Runner',
        'Stage 27 — Complete',
        'Stage 28 — Complete',
        'Stage 28 Slice 1 — Complete',
        'Stage 28 Slice 2 — Complete',
    ]);
}

$require($specPath, $spec, [
    '### Match expressions',
    '`match` is an exhaustive expression',
    'case-only pattern ignores all payloads',
    'and `mixed` open domains use a',
    'move payload bindings are readonly borrows',
    'An arm guard is written `Pattern if',
    '`match (take $value)` explicitly consumes',
]);
$require($stdlibPath, $stdlib, [
    'implemented Decision 0115',
    'completes core `match`',
    '`if` pattern guards',
]);
$require($cataloguePath, $catalogue, ['"E0576"', '"E0585"', '"E0595"', '"E0604"', '"I2801"']);
$require($parserPath, $parser, [
    'fn parse_match_expression(',
    'fn parse_match_guard(',
    'fn parse_match_pattern(',
    '`when` is not a match guard; use `if`',
    '`where` is not a match guard; use `if`',
    'consume the complete value with `match (take $value)` instead',
    'fn parse_ternary(',
    'Doria does not support the short ternary `?:`',
]);
$require($semanticsPath, $semantics, [
    'fn check_match_expression(',
    'Non-Exhaustive Match',
    'Duplicate Match Pattern',
    'Match Pattern Is Not Constant',
    'Match Arm Type Mismatch',
    'Match Condition Must Be Bool',
    'match guard must have type `bool`',
    'Copy match scrutinee does not need `take`',
    'Ternary Condition Must Be Bool',
]);
$require($mirPath, $mir, [
    'PayloadEnumIsCase',
    'BindPayloadEnumFields',
    'MatchResultPlan',
    'MatchOwnershipMode',
    'MatchBindingMode',
    'GuardView',
    'ConsumedArm',
]);
$require($loweringPath, $lowering, [
    'fn lower_match_rvalue(',
    'fn lower_match_pattern_to_blocks(',
    'fn bind_match_arm(',
    'MatchResultPlan',
    'mir::MatchBindingMode::GuardView',
    'mir::MatchBindingMode::ConsumedArm',
]);
$require($validationPath, $validation, [
    'fn validate_match_result_plans(',
    'fn validate_match_binding_plans(',
    'match guard must branch through a success path to its final binding block',
    'is destructured without a dominating exact case proof',
    '{path_name} reaches its merge with {assignments} result assignments',
]);
foreach ([$interpreterPath => $interpreter, $craneliftPath => $cranelift, $llvmPath => $llvm] as $path => $contents) {
    $require($path, $contents, ['PayloadEnumIsCase', 'BindPayloadEnumFields', 'MatchResultPlan']);
}
$require($phpPath, $php, [
    'final class __DoriaMixedValue',
    'function __doria_box_mixed',
    'function __doria_mixed_is',
    'fn emit_match_expression(',
    'fn emit_php_match_condition(',
    'fn emit_php_match_bindings(',
    'only the final checked match arm may be unconditional',
]);
$require($performancePath, $performance, [
    '"matchExpressionCount"',
    '"matchArmCount"',
    '"enumMatchCount"',
    '"conditionMatchCount"',
    '"typePatternCount"',
    '"ternaryCount"',
]);
$require($testsPath, $tests, [
    'unit_payload_and_case_only_patterns_execute_with_arm_local_bindings',
    'match_evaluates_one_scrutinee_and_only_the_selected_arm',
    'exhaustiveness_covers_finite_nullable_and_open_domains',
    'exact_type_patterns_narrow_mixed_and_nullable_values',
    'match_true_is_strict_ordered_lazy_and_requires_default',
    'arm_results_share_one_strict_type_with_nullable_unification',
    'named_and_temporary_move_scrutinees_are_borrowed_through_the_selected_arm',
    'pattern_guards_use_if_require_bool_and_reject_redundant_positions',
    'guarded_patterns_preserve_coverage_and_reachability_rules',
    'consuming_match_moves_whole_values_and_materializes_payloads_after_guards',
    'consuming_nullable_and_mixed_matches_execute_without_hidden_aliases',
    'payload_level_take_and_writable_patterns_are_deliberately_rejected',
    'integer patterns should preserve contextual width and signedness',
    'ternary_is_right_associative_strict_lazy_and_rejects_elvis',
]);
$require($malformedPath, $malformed, [
    'shared_validator_rejects_malformed_match_dispatch_projection_and_result_plans',
]);
$require($llvmTestsPath, $llvmTests, [
    'match_ir_uses_inline_dispatch_and_projects_payloads_only_in_selected_arms',
    'guarded_consuming_match_ir_keeps_storage_inline_and_scratch_in_entry',
]);
$require($manifestPath, $manifest, [
    'examples/native/main_match_unit_enums.doria',
    'examples/native/main_match_payload_enums.doria',
    'examples/native/main_match_nullable.doria',
    'examples/native/main_match_mixed.doria',
    'examples/native/main_match_true.doria',
    'examples/native/main_match_ternary.doria',
    'examples/native/main_match_ownership.doria',
    'examples/native/main_match_guards.doria',
    'examples/native/main_match_guard_exhaustiveness.doria',
    'examples/native/main_match_take_enum.doria',
    'examples/native/main_match_take_nullable.doria',
    'examples/native/main_match_take_mixed.doria',
    'examples/native/main_match_guarded_take.doria',
    'examples/native/main_match_take_drop_order.doria',
]);

if (str_contains($semantics, '"E0576"')) {
    $failures[] = "{$semanticsPath}: E0576 still has an active semantic route";
}
if (str_contains($php, "non-exhaustive Doria match")) {
    $failures[] = "{$phpPath}: PHP lowering invented a runtime exhaustiveness failure";
}
if (str_contains($php, 'get_debug_type(')) {
    $failures[] = "{$phpPath}: PHP exact Doria type identity regressed to host reflection";
}

$fixtureContents = $tests;
foreach (glob($root . '/examples/native/main_match_*.doria') ?: [] as $fixture) {
    $contents = @file_get_contents($fixture);
    if (is_string($contents)) {
        $fixtureContents .= $contents;
    }
}
foreach (['Andrew', 'Lucy', 'Masiye'] as $privateName) {
    if (str_contains($fixtureContents, $privateName)) {
        $failures[] = "Stage 28 tests or fixtures contain private or family name `{$privateName}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "match foundation check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "match foundation check passed\n");
