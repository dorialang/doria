<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$files = [
    'decision' => 'docs/decisions/0129-native-testing-foundation-behavioral-dsl-expectations-and-assertion-outcomes.md',
    'matchers' => 'crates/doriac/src/assertions.rs',
    'semantics' => 'crates/doriac/src/semantics.rs',
    'ownership' => 'crates/doriac/src/ownership.rs',
    'mir' => 'crates/doriac/src/mir.rs',
    'lowering' => 'crates/doriac/src/mir_lowering.rs',
    'validation' => 'crates/doriac/src/mir_validation.rs',
    'runtime' => 'crates/doria-rt/src/string_ops.rs',
    'php' => 'crates/doriac/src/codegen_php.rs',
    'cli' => 'crates/doriac/src/main.rs',
    'tests' => 'crates/doriac/tests/native_testing_slice1_tests.rs',
];

$contents = [];
foreach ($files as $key => $relative) {
    $text = file_get_contents($root . '/' . $relative);
    if ($text === false) {
        fwrite(STDERR, "Missing Native Testing Slice 3 file: {$relative}\n");
        exit(1);
    }
    $contents[$key] = $text;
}

$required = [
    'decision' => [
        '**Implementation Status:** Implemented By Native Testing Foundation Slices 1 Through 3',
        'Native Testing Foundation Slice 3 - Complete',
        'Native Testing Foundation - Complete',
        'Stage 34 Single Class Inheritance - Complete',
        'Stage 35 Interfaces And Traits - Authority Accepted; Slice 1 Next',
        '| `Bytes` | yes | yes | no | no | no |',
        '| `Dictionary<K, V>` / `SortedDictionary<K, V>` | yes | yes | no | yes | yes |',
        'ordinary once-call consumption',
        'bounded to 4 KiB',
        'most eight public-order entries',
        'DORIAO5',
    ],
    'matchers' => [
        'pub const MATCHER_SPECS: [MatcherSpec; 19]',
        '"StringContains"',
        '"StringEmpty"',
        '"CollectionContains"',
        '"CollectionEmpty"',
        '"CollectionCount"',
        '"DictionaryHasKey"',
        '"DictionaryHasValue"',
        '"Throws"',
        'pub fn matcher_candidates',
    ],
    'semantics' => [
        'check_throw_assertion',
        'assertion_callable_invocations',
        'Bytes Does Not Support Membership Expectations',
        'Dictionary Membership Expectation Is Ambiguous',
        'Negated Error Expectation Does Not Accept An Inspector',
    ],
    'ownership' => [
        'assertion_callable_invocations',
        'FunctionInvocationMode::Readonly',
        'FunctionInvocationMode::Writable',
        'FunctionInvocationMode::Once',
    ],
    'mir' => ['CollectionHas', 'AssertionPlan', 'CheckedIndirect'],
    'lowering' => [
        'lower_throw_assertion',
        'AssertionCollectionPresentation',
        'AssertionCountDifference',
        'AssertionErrorPresentation',
    ],
    'validation' => [
        'validate_assertion_collection_contract',
        'validate_throw_assertion_contract',
        'negated throw assertion cannot carry an inspector',
        'collection membership assertion uses an unsupported collection family',
    ],
    'runtime' => [
        'dr_v4_collection_assertion_presentation',
        'dr_v4_collection_assertion_count_difference',
        'dr_v4_error_assertion_presentation',
        'const ITEM_LIMIT: usize = 8',
    ],
    'php' => [
        '__doria_assertion_collection_presentation',
        '__DoriaCheckedError',
        '__doria_write_assertion_outcome_v4',
    ],
    'cli' => ['difference_present', 'difference'],
    'tests' => [
        'slice_three_collection_matchers_execute_on_every_backend',
        'slice_three_throw_matchers_intercept_checked_errors_exactly',
        'slice_three_throw_matchers_use_ordinary_invocation_modes_and_cleanup',
        'slice_three_failure_facts_match_on_every_backend',
        'slice_three_collection_presentations_are_bounded_and_public',
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

foreach (['FUTURE_MATCHERS', 'DORIAO5', 'ReflectionClass', 'in_array($collection'] as $forbidden) {
    foreach (['matchers', 'semantics', 'mir', 'lowering', 'runtime', 'php'] as $key) {
        if (str_contains($contents[$key], $forbidden)) {
            $failures[] = "{$files[$key]}: forbidden Slice 3 mechanism `{$forbidden}`";
        }
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Native Testing Slice 3 compiler/runtime guard failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Native Testing Slice 3 compiler/runtime guard passed\n");
