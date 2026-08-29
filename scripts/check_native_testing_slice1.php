<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$files = [
    'decision' => 'docs/decisions/0129-native-testing-foundation-behavioral-dsl-expectations-and-assertion-outcomes.md',
    'identity' => 'crates/doriac/src/compiler_known_test.rs',
    'testing' => 'crates/doriac/src/testing.rs',
    'graph' => 'crates/doriac/src/compilation_graph.rs',
    'metadata' => 'crates/doriac/src/attributes.rs',
    'hir' => 'crates/doriac/src/hir.rs',
    'mir' => 'crates/doriac/src/mir.rs',
    'tests' => 'crates/doriac/tests/native_testing_slice1_tests.rs',
];

$contents = [];
foreach ($files as $key => $relative) {
    $path = $root . '/' . $relative;
    $text = is_file($path) ? file_get_contents($path) : false;
    if ($text === false) {
        fwrite(STDERR, "Missing Native Testing Slice 1 file: {$relative}\n");
        exit(1);
    }
    $contents[$key] = $text;
}

$required = [
    'decision' => [
        '**Status:** Accepted',
        'Slice 1 Compiler Implemented',
        'no runtime registration',
        'no Baton source parsing',
    ],
    'identity' => [
        'CompilerSymbolIdentity::StandardTest',
        'pub const DESCRIBE:',
        'pub const IT:',
        'pub const TEST:',
        'FUTURE_MEMBERS',
    ],
    'testing' => [
        'SourceSemanticContext',
        'BehavioralTestSuite',
        'TestSemanticInfo',
        'generated_function_spans',
        'evaluate_string_expression',
    ],
    'graph' => [
        'elaborate_source',
        'package functions before resolving dispatcher references',
        'analyze_program_for_ide_with_graph_and_test_context',
    ],
    'metadata' => [
        'AttributeMetadataDocumentV3',
        'pub test_suites: Vec<MetadataTestSuiteV3>',
        'pub tests: Vec<MetadataTestV3>',
        'deny_unknown_fields',
    ],
    'hir' => [
        'pub test_suites:',
        'pub tests:',
    ],
    'tests' => [
        'generated_dispatcher_executes_behavioral_callable_on_every_enabled_backend',
        'metadata_schema_three_is_strict_and_older_schemas_remain_disjoint',
        'future_test_type_has_one_slice_two_boundary',
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

foreach (['TestSuite', 'RegisterTest', 'RegisterSuite', 'TestRegistry'] as $runtimeNode) {
    if (str_contains($contents['mir'], $runtimeNode)) {
        $failures[] = "{$files['mir']}: forbidden test-specific runtime node `{$runtimeNode}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Native Testing Slice 1 guard failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Native Testing Slice 1 compiler guard passed\n");
