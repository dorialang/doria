<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$files = [
    'decision' => 'docs/decisions/0129-native-testing-foundation-behavioral-dsl-expectations-and-assertion-outcomes.md',
    'reference' => 'docs/native-testing-foundation.md',
    'identity' => 'crates/doriac/src/compiler_known_test.rs',
    'matchers' => 'crates/doriac/src/assertions.rs',
    'testing' => 'crates/doriac/src/testing.rs',
    'effects' => 'crates/doriac/src/checked_effects.rs',
    'semantics' => 'crates/doriac/src/semantics.rs',
    'hir' => 'crates/doriac/src/hir.rs',
    'mir' => 'crates/doriac/src/mir.rs',
    'validation' => 'crates/doriac/src/mir_validation.rs',
    'lowering' => 'crates/doriac/src/mir_lowering.rs',
    'runtime' => 'crates/doria-rt/src/lib.rs',
    'runtimeStrings' => 'crates/doria-rt/src/string_ops.rs',
    'diagnostics' => 'crates/doriac/src/diagnostics.rs',
    'catalogue' => 'crates/doria-diagnostic-catalogue/src/lib.rs',
    'cli' => 'crates/doriac/src/main.rs',
    'php' => 'crates/doriac/src/codegen_php.rs',
    'tests' => 'crates/doriac/tests/native_testing_slice1_tests.rs',
];

$contents = [];
foreach ($files as $key => $relative) {
    $text = file_get_contents($root . '/' . $relative);
    if ($text === false) {
        fwrite(STDERR, "Missing Native Testing Slice 2 file: {$relative}\n");
        exit(1);
    }
    $contents[$key] = $text;
}

$required = [
    'decision' => [
        '**Status:** Accepted',
        '**Implementation Status:** Slices 1 And 2 Implemented; Slice 3 Next; Foundation In Progress',
        'Native Testing Foundation Slice 1 - Complete',
        'Native Testing Foundation Slice 2 - Complete',
        'Native Testing Foundation Slice 3 - Next',
        'Slice 3 - Collection/Error Expectations, Baton Reporting, And Tooling Closure',
        'Stage 34 Single Class Inheritance - Blocked Until The Foundation Completes',
    ],
    'reference' => [
        'Baton regression integration',
        'Slice 3 owns collection/Error expectations',
    ],
    'identity' => [
        'IMPLEMENTED_MEMBERS: [&str; 6]',
        '[DESCRIBE, IT, TEST, EXPECT, FAIL, ASSERTION_ERROR]',
        'FUTURE_MEMBERS: [&str; 0]',
        'ASSERTION_FACT_PROPERTIES',
    ],
    'matchers' => [
        'pub enum AssertionMatcher',
        'pub const MATCHERS: [AssertionMatcher; 12]',
        'pub const FUTURE_MATCHERS:',
        'pub const PRESENTATION_LIMIT: usize = 4096',
    ],
    'effects' => [
        'Required',
        'AmbientIo',
        'TestAssertion',
        'pub test_assertion: Vec<ResolvedType>',
    ],
    'semantics' => [
        'expectation negation is the `not` property, not a method',
        'Expectation Cannot Be Negated More Than Once',
        'Expectation Value Cannot Escape',
        'Native Testing Foundation Slice 3',
    ],
    'hir' => ['Assertion(Box<Assertion>)', 'test_assertion_checked_effects'],
    'mir' => [
        'Assertion(Box<AssertionPlan>)',
        'pub struct AssertionPlan',
        'test_assertion_checked_effects',
    ],
    'validation' => [
        'assertion plan belongs to non-development source',
        'compiler-known AssertionError descriptor',
        'fail assertion plan has incompatible operands',
    ],
    'lowering' => [
        'lower_assertion_statement',
        'materialize_assertion_presentation',
        'ControlFlowPlan::Assertion',
    ],
    'runtime' => [
        'DORIAO4',
        'Error[R1001]: Assertion Failed',
        'put_u32(&mut header[14..18], 70)',
    ],
    'runtimeStrings' => [
        'ASSERTION_PRESENTATION_LIMIT: usize = 4096',
        'dr_v4_string_assertion_quote',
    ],
    'diagnostics' => ['RuntimeAssertion', 'runtimeAssertion', 'R1001'],
    'catalogue' => ['ASSERTION_FACT_NAMES', 'R1001'],
    'cli' => [
        'DORIA_RUNTIME_OUTCOME_V4',
        'DORIAO4',
        'runtime_assertion_transport_is_strict_and_catalogued',
    ],
    'php' => ['DORIAO4', '__doria_write_assertion_outcome_v4'],
    'tests' => [
        'slice_two_matchers_evaluate_once_in_order_and_execute_on_every_backend',
        'assertion_errors_are_catchable_and_helpers_need_no_throws_clause',
        'shared_validator_rejects_malformed_assertion_plans_and_descriptors',
        'escaping_assertion_uses_the_strict_v4_outcome_on_every_enabled_backend',
    ],
];

$failures = [];
foreach ($required as $key => $needles) {
    foreach ($needles as $needle) {
        if (!str_contains($contents[$key], $needle)) {
            $failures[] = "{$files[$key]}: missing `{$needle}`";
        }
    }
}

foreach (['testing', 'effects', 'semantics', 'hir', 'mir', 'lowering', 'runtime'] as $key) {
    if (str_contains($contents[$key], 'E0710')) {
        $failures[] = "{$files[$key]}: historical E0710 still has a live route";
    }
}

foreach (['ExpectationObject', 'AllocateExpectation', 'TestRegistry', 'RegisterMatcher'] as $forbidden) {
    foreach (['hir', 'mir', 'lowering', 'runtime'] as $key) {
        if (str_contains($contents[$key], $forbidden)) {
            $failures[] = "{$files[$key]}: forbidden runtime testing mechanism `{$forbidden}`";
        }
    }
}

foreach (['PHPUnit', 'Pest', 'assert('] as $forbidden) {
    if (str_contains($contents['php'], $forbidden)) {
        $failures[] = "{$files['php']}: generated PHP delegates assertion semantics to `{$forbidden}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Native Testing Slice 2 compiler/runtime guard failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Native Testing Slice 2 compiler/runtime guard passed\n");
