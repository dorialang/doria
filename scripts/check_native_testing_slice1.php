<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$files = [
    'decision' => 'docs/decisions/0129-native-testing-foundation-behavioral-dsl-expectations-and-assertion-outcomes.md',
    'reference' => 'docs/native-testing-foundation.md',
    'identity' => 'crates/doriac/src/compiler_known_test.rs',
    'names' => 'crates/doriac/src/names.rs',
    'testing' => 'crates/doriac/src/testing.rs',
    'graph' => 'crates/doriac/src/compilation_graph.rs',
    'metadata' => 'crates/doriac/src/attributes.rs',
    'hir' => 'crates/doriac/src/hir.rs',
    'mir' => 'crates/doriac/src/mir.rs',
    'effects' => 'crates/doriac/src/checked_effects.rs',
    'semantics' => 'crates/doriac/src/semantics.rs',
    'runtime' => 'crates/doria-rt/src/lib.rs',
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
        '**Implementation Status:** Slices 1 And 2 Implemented; Slice 3 Next; Foundation In Progress',
        'Native Testing Foundation Slice 1 - Complete',
        'Native Testing Foundation Slice 2 - Complete',
        'Native Testing Foundation Slice 3 - Next',
        'Stage 34 Single Class Inheritance - Blocked Until The Foundation Completes',
        'no runtime registration',
        'no Baton source parsing',
    ],
    'reference' => [
        'Stage 33 - Complete',
        'Phase F - Complete',
        'Native Testing Foundation - In Progress, Not Complete',
        'Baton regression integration',
        'Slice 3 owns collection/Error expectations',
    ],
    'identity' => [
        'CompilerSymbolIdentity::StandardTest',
        'pub const DESCRIBE:',
        'pub const IT:',
        'pub const TEST:',
        'pub const DECLARATIONS: [&str; 3] = [DESCRIBE, IT, TEST]',
        'FUTURE_MEMBERS: [&str; 0]',
        'IMPLEMENTED_MEMBERS: [&str; 6]',
    ],
    'names' => [
        'GlobalSymbolKind::CompilerKnownTestDeclaration',
        'GlobalReferenceRole::TestDeclaration',
        'CompilerSymbolIdentity::StandardTest',
    ],
    'testing' => [
        'SourceSemanticContext',
        'is_development()',
        'resolved_test_member(expr.span())',
        'compiler_known_test::is_declaration(&name)',
        'BehavioralTestSuite',
        'TestSemanticInfo',
        'generated_function_spans',
        'evaluate_string_expression',
        '__doria_test_',
    ],
    'graph' => [
        'elaborate_source',
        'scope: source.scope',
        'package functions before resolving dispatcher references',
        'analyze_program_for_ide_with_graph_and_test_context',
    ],
    'metadata' => [
        'AttributeMetadataDocumentV1',
        'AttributeMetadataDocumentV2',
        'AttributeMetadataDocumentV3',
        'pub test_suites: Vec<MetadataTestSuiteV3>',
        'pub tests: Vec<MetadataTestV3>',
        'deny_unknown_fields',
    ],
    'hir' => [
        'Behavioral declaration calls and suite',
        'are absent from runtime items',
        'pub test_suites:',
        'pub tests:',
    ],
    'tests' => [
        'generated_dispatcher_executes_behavioral_callable_on_every_enabled_backend',
        'metadata_schema_three_is_strict_and_older_schemas_remain_disjoint',
        'generated_development_sources_are_accepted_but_main_sources_are_not',
        'user_functions_with_test_like_short_names_remain_ordinary',
        'behavioral_declarations_elaborate_into_unified_metadata_and_hir',
        'slice_two_matchers_evaluate_once_in_order_and_execute_on_every_backend',
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

foreach (['name == "describe"', 'name == "it"', 'name == "test"'] as $rawMatch) {
    if (str_contains($contents['testing'], $rawMatch)) {
        $failures[] = "{$files['testing']}: raw source-name matching is forbidden `{$rawMatch}`";
    }
}

foreach (['testing', 'effects', 'semantics', 'mir', 'runtime'] as $key) {
    if (str_contains($contents[$key], 'E0710')) {
        $failures[] = "{$files[$key]}: historical E0710 still has a live compiler/runtime route";
    }
}

$crateIterator = new RecursiveIteratorIterator(
    new RecursiveDirectoryIterator($root . '/crates', FilesystemIterator::SKIP_DOTS),
);
foreach ($crateIterator as $entry) {
    $relative = str_replace('\\', '/', substr($entry->getPathname(), strlen($root) + 1));
    if (preg_match('/(?:stage[_-]?34|inheritance)/i', $relative) === 1) {
        $failures[] = "{$relative}: Stage 34 implementation file exists before the testing foundation completes";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Native Testing Slice 1 guard failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Native Testing Slice 1 compiler guard passed\n");
