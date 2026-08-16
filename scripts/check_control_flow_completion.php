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
            $failures[] = "{$path}: missing Stage 28a contract `{$needle}`";
        }
    }
};

$decisionPath = 'docs/decisions/0116-when-given-control-flow-finally-and-do-while.md';
$planPath = 'docs/doria-end-to-end-plan.md';
$pipelinePath = 'docs/notes/current-pipeline.md';
$specPath = 'SPEC.md';
$parserPath = 'crates/doriac/src/parser.rs';
$semanticsPath = 'crates/doriac/src/semantics.rs';
$controlFlowPath = 'crates/doriac/src/control_flow.rs';
$ownershipPath = 'crates/doriac/src/ownership.rs';
$mirPath = 'crates/doriac/src/mir.rs';
$loweringPath = 'crates/doriac/src/mir_lowering.rs';
$validationPath = 'crates/doriac/src/mir_validation.rs';
$phpPath = 'crates/doriac/src/codegen_php.rs';
$parserTestsPath = 'crates/doriac/tests/parser_tests.rs';
$semanticTestsPath = 'crates/doriac/tests/semantic_tests.rs';
$malformedTestsPath = 'crates/doriac/tests/mir_validation_tests.rs';
$llvmTestsPath = 'crates/doriac/tests/llvm_mir_tests.rs';
$phpTestsPath = 'crates/doriac/tests/codegen_php_tests.rs';
$performancePath = 'crates/doriac/src/performance.rs';
$ownershipTestsPath = 'crates/doriac/tests/stage19_tests.rs';
$manifestPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';

$decision = $read($decisionPath);
$plan = $read($planPath);
$pipeline = $read($pipelinePath);
$spec = $read($specPath);
$parser = $read($parserPath);
$semantics = $read($semanticsPath);
$controlFlow = $read($controlFlowPath);
$ownership = $read($ownershipPath);
$mir = $read($mirPath);
$lowering = $read($loweringPath);
$validation = $read($validationPath);
$php = $read($phpPath);
$parserTests = $read($parserTestsPath);
$semanticTests = $read($semanticTestsPath);
$malformedTests = $read($malformedTestsPath);
$llvmTests = $read($llvmTestsPath);
$phpTests = $read($phpTestsPath);
$performance = $read($performancePath);
$ownershipTests = $read($ownershipTestsPath);
$manifest = $read($manifestPath);

$require($decisionPath, $decision, [
    '**Status:** Accepted',
    '**Implementation status:** Implemented; Stage 28a Slices 1 and 2 complete',
    '`when` is the value-returning form of `if`',
    '`else` is mandatory',
    '`return expression;` in a branch yields from the nearest enclosing `when`',
    'the explicit head annotation',
    'the surrounding expected type',
    'An unannotated all-null',
    'requires an expected nullable type',
    '`given` On `if`',
    '`given` On `when`',
    '`given` On `while`',
    'Setup after that boundary is rejected',
    'setup runs once',
    'before every condition check',
    'The first false predicate skips later',
    'predicates and the attached condition',
    'The ordinary form requires its terminating semicolon',
    'The v1 set is `if`, `when`, `while`, and `do ... while`',
    '`for`, `foreach`, `match`, and bare blocks reject `finally`',
    'Fatal panic remains abort-without-cleanup',
    'An outgoing `when` result or function return is acquired first',
    'Branch locals',
    'drop next',
    'Given locals drop afterward',
    'checked-error exit will identify crossed regions',
    'Fatal panic remains a separate',
    'abort-only edge and bypasses every region',
    'Controlled',
    '**Pending Available Runner**',
    'does not block development',
]);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    $require($path, $contents, [
        'Stage 26b — Complete',
        'Measurement Status: Pending Available Runner',
        'Stage 28 — Complete',
        'Stage 28a — Complete',
        'Stage 28a Slice 1 — Complete',
        'Stage 28a Slice 2 — Complete',
        'Stage 29 — In Progress',
        'Stage 29 Slice 1 — Complete',
        'Stage 29 Slice 2 — Complete',
        'Stage 29 Slice 3 — Next',
    ]);
}

$require($specPath, $spec, [
    '`when` is the value-returning form of `if`',
    'It requires a total `else`',
    'nearest enclosing',
    '`when` rather than returning from the function',
    'Setup',
    'runs once',
    'before every condition check',
    'A failed gate skips every attached conditional condition',
    '`do ... while`',
    'ordinary form requires its semicolon',
    'It runs once',
    'fatal panic',
]);
$require($parserPath, $parser, [
    'fn parse_given_prelude(',
    'fn parse_when_expression(',
    'fn parse_do_while(',
    'fn parse_optional_finally(',
    '`finally` attaches only to `if`, `when`, `while`, or `do ... while`',
    'expected `;` after do-while condition',
]);
$require($semanticsPath, $semantics, [
    'fn check_given_prelude(',
    'fn check_when_expression(',
    'Given Setup Appears After A Predicate',
    'Control Transfer Cannot Leave Finally',
    'E0612',
]);
$require($controlFlowPath, $controlFlow, [
    'route_finalizers',
    'finalizer_depth',
    'NodeKind::ReturnExit',
    'NodeKind::DivergeExit',
]);
$require($ownershipPath, $ownership, [
    'apply_finally_to_flow',
    'flow.returns',
    'flow.yields',
]);
$require($mirPath, $mir, [
    'GivenControlFlowPlan',
    'WhenResultPlan',
    'DoWhilePlan',
    'FinalizerRegionPlan',
    'StructuredExitKind',
    'FinalizerExitPlan',
]);
$require($loweringPath, $lowering, [
    'ControlFlowPlan::Given',
    'ControlFlowPlan::When',
    'ControlFlowPlan::DoWhile',
    'ControlFlowPlan::Finalizer',
    'route_structured_exit',
    'finish_finalizer_region',
]);
$require($validationPath, $validation, [
    'fn validate_control_flow_plans(',
    'given setup does not lead to its predicate phase',
    'given while continue skips predicate reevaluation',
    '{path_name} reaches its merge with {assignments} result assignments',
    'do-while continue does not target its condition',
    'finalizer region has an invalid lexical parent',
    'finalizer completion does not select its final continuation',
    'same-loop continue incorrectly routes through its loop finalizer',
    'finalizer entry edges disagree with its structured-exit table',
    'checked-error finalizer exit does not own an Error carrier',
]);
$require($phpPath, $php, [
    'fn emit_with_finally(',
    'writeln(output, indent, "try")',
    'writeln(output, indent, "finally")',
]);
$require($parserTestsPath, $parserTests, [
    'parses_given_when_and_preserves_its_finalizer',
    'parses_do_while_and_requires_its_ordinary_semicolon',
    'rejects_finally_on_excluded_control_flow_families',
    'rejects_when_without_else_and_given_on_do',
]);
$require($semanticTestsPath, $semanticTests, [
    'checks_stage28a_when_typing_and_nearest_yields',
    'checks_stage28a_given_phases_scope_and_do_while_conditions',
    'accepted_control_flow_finalizers_reach_validated_mir',
    'finalizer_transfers_are_checked_by_destination',
    'finalizer_scope_follows_the_complete_attached_construct',
    'constructor_finalizers_participate_in_definite_initialization',
]);
$require($malformedTestsPath, $malformedTests, [
    'shared_validator_rejects_malformed_stage28a_control_flow_plans',
    'when branch reaches its merge with 0 result assignments',
    'incompatible ownership',
    'given predicate does not have bool type',
    'do-while condition is not bool control flow between its body and exit',
    'shared_validator_rejects_malformed_finalizer_regions_and_exit_routes',
]);
$require($llvmTestsPath, $llvmTests, [
    'stage28a_control_flow_keeps_one_validated_cfg_without_runtime_objects',
    'dr_v1_when',
    'dr_v1_given',
    'dr_v1_do_while',
    'dr_v1_finalizer',
    'dr_v1_cleanup_stack',
    'StructuredExitKind::FunctionReturn',
    'StructuredExitKind::Break',
    'StructuredExitKind::Continue',
]);
$require($phpTestsPath, $phpTests, [
    'php_backend_executes_stage28a_slice1_control_flow',
    'php_backend_executes_stage28a_finalizers',
    'wrong inner',
    'wrong outer',
]);
$require($ownershipTestsPath, $ownershipTests, [
    'finalizer_ownership_updates_every_normally_crossing_exit',
    'fatal_panic_bypasses_finalizer_ownership_flow',
]);
$require($performancePath, $performance, [
    'whenExpressionCount',
    'elseWhenBranchCount',
    'givenPreludeCount',
    'givenPredicateCount',
    'doWhileCount',
    'finalizerCount',
    'structuredExitCount',
    'finalizedReturnCount',
    'finalizedBreakCount',
    'finalizedContinueCount',
    'maximumFinalizerNestingDepth',
]);

foreach ([
    'examples/native/main_when_basic.doria',
    'examples/native/main_when_expected_types.doria',
    'examples/native/main_given_if.doria',
    'examples/native/main_given_when.doria',
    'examples/native/main_given_while.doria',
    'examples/native/main_do_while.doria',
    'examples/native/main_when_ownership.doria',
    'examples/native/main_if_finally.doria',
    'examples/native/main_given_if_finally.doria',
    'examples/native/main_when_finally.doria',
    'examples/native/main_given_when_finally.doria',
    'examples/native/main_while_finally.doria',
    'examples/native/main_given_while_finally.doria',
    'examples/native/main_do_while_finally.doria',
    'examples/native/main_nested_finally.doria',
    'examples/native/main_finally_structured_exits.doria',
    'examples/native/main_finally_ownership.doria',
    'examples/native/main_finally_contained_control.doria',
    'examples/native/main_finally_panic.doria',
] as $fixture) {
    if (!str_contains($manifest, $fixture)) {
        $failures[] = "{$manifestPath}: missing durable Stage 28a fixture `{$fixture}`";
    }
}

$fixtureContents = '';
foreach (array_merge(
    glob($root . '/examples/native/main_{when,given,do_while}*.doria', GLOB_BRACE) ?: [],
    glob($root . '/examples/native/main_*finally*.doria') ?: [],
    glob($root . '/crates/doriac/tests/fixtures/stage28a_pending/*.doria') ?: []
) as $fixture) {
    $contents = @file_get_contents($fixture);
    if (is_string($contents)) {
        $fixtureContents .= $contents;
    }
}

foreach ([
    $decisionPath => $decision,
    $planPath => $plan,
    $pipelinePath => $pipeline,
    $specPath => $spec,
    $semanticsPath => $semantics,
    $mirPath => $mir,
] as $path => $contents) {
    foreach ([
        'Control-Flow Finally Is Not Executable Yet',
        'PendingFinally',
        'Stage 28a Slice 2 — Next',
        'Stage 29 — Blocked Until Stage 28a Completes',
    ] as $obsolete) {
        if (str_contains($contents, $obsolete)) {
            $failures[] = "{$path}: obsolete Stage 28a boundary returned: `{$obsolete}`";
        }
    }
}
foreach (['Andrew', 'Lucy', 'Masiye'] as $privateName) {
    if (str_contains($fixtureContents, $privateName)) {
        $failures[] = "Stage 28a fixtures contain private or family name `{$privateName}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "control-flow completion check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "control-flow completion check passed\n");
