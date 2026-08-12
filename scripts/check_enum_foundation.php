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

$require = static function (
    string $path,
    string $contents,
    array $needles,
) use (&$failures): void {
    foreach ($needles as $needle) {
        if (!str_contains($contents, $needle)) {
            $failures[] = "{$path}: missing enum-foundation contract `{$needle}`";
        }
    }
};

$decisionPath = 'docs/decisions/0114-enums-backed-cases-payload-cases-and-inline-tagged-layout.md';
$planPath = 'docs/doria-end-to-end-plan.md';
$pipelinePath = 'docs/notes/current-pipeline.md';
$lexerPath = 'crates/doriac/src/lexer.rs';
$parserPath = 'crates/doriac/src/parser.rs';
$typesPath = 'crates/doriac/src/types.rs';
$enumsPath = 'crates/doriac/src/enums.rs';
$semanticsPath = 'crates/doriac/src/semantics.rs';
$mirPath = 'crates/doriac/src/mir.rs';
$validationPath = 'crates/doriac/src/mir_validation.rs';
$interpreterPath = 'crates/doriac/src/mir_interpreter.rs';
$craneliftPath = 'crates/doriac/src/codegen_cranelift.rs';
$llvmPath = 'crates/doriac/src/codegen_llvm.rs';
$phpPath = 'crates/doriac/src/codegen_php.rs';
$testsPath = 'crates/doriac/tests/stage27_tests.rs';
$manifestPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';

$decision = $read($decisionPath);
$plan = $read($planPath);
$pipeline = $read($pipelinePath);
$lexer = $read($lexerPath);
$parser = $read($parserPath);
$types = $read($typesPath);
$enums = $read($enumsPath);
$semantics = $read($semanticsPath);
$mir = $read($mirPath);
$validation = $read($validationPath);
$interpreter = $read($interpreterPath);
$cranelift = $read($craneliftPath);
$llvm = $read($llvmPath);
$php = $read($phpPath);
$tests = $read($testsPath);
$manifest = $read($manifestPath);

$require($decisionPath, $decision, [
    '**Status:** Accepted',
    '## Unit Cases',
    '## Backed Enums',
    '## Payload Cases',
    '## Inline Tagged Layout',
    '## Nullability',
    '## `mixed` Integration',
    '## Generic Enum Deferral',
    '## `match` Boundary',
    'Stage 27 Slice 2',
    '**Implementation status: Complete.**',
    'Generic enums remain deferred',
    'Pending Available Runner',
    'non-blocking',
]);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    $require($path, $contents, [
        'Stage 26b — Complete',
        'Measurement Status: Pending Available Runner',
        'Decision 0113 — Complete',
        'Stage 27 — Complete',
        'Stage 27 Slice 1 — Complete',
        'Stage 27 Slice 2 — Complete',
        'Stage 28 — In Progress',
        'Stage 28 Slice 1 — Complete',
        'Stage 28 Slice 2 — Next',
    ]);
}

$require($lexerPath, $lexer, ['Enum,', 'Case,', 'Match,', 'Default,']);
$require($parserPath, $parser, ['fn parse_enum(', 'fn parse_match_expression(']);
$require($typesPath, $types, ['Enum(EnumType)']);
$require($enumsPath, $enums, [
    'pub struct EnumCapabilities',
    'pub struct EnumLayout',
    'pub struct EnumCaseLayout',
    'pub struct EnumPayloadFieldLayout',
    'pub fn tag_width(',
]);
$require($semanticsPath, $semantics, [
    'Enum Name Must Use PascalCase',
    'Case Name Must Use PascalCase',
    'Backed Case Requires A Value',
    'Unit Enum Has No Value Property',
    'cannot be displayed by echo',
    'Different Enum Types Cannot Be Compared',
    'Recursive Inline Enum Layout',
    'Payload Enum Equality Is Unavailable',
    'Enum Payload Requires Pattern Matching',
    'fn check_match_expression(',
    'Generic Enums Are Not Implemented',
]);
$require($mirPath, $mir, [
    'pub enums: Vec<EnumDefinition>',
    'Enum(EnumId)',
    'EnumExpression',
    'EnumBacking',
    'PayloadEnumType',
    'PayloadEnumExpression',
    'NullablePayloadEnumExpression',
    'DropPayloadEnum',
    'PayloadEnumCompare',
]);
$require($validationPath, $validation, [
    'for (index, definition) in program.enums.iter().enumerate()',
    'enum case identity names another enum',
    'integer backing projection targets a non-int-backed enum',
    'string backing projection targets a non-string-backed enum',
    'fn validate_payload_enum_type(',
    'fn validate_payload_enum_expression(',
    'fn validate_payload_enum_place(',
    'move payload enum is copied instead of transferred',
]);
foreach ([$interpreterPath => $interpreter, $craneliftPath => $cranelift, $llvmPath => $llvm] as $path => $contents) {
    $require($path, $contents, ['EnumExpression', 'EnumBacking', 'PayloadEnumExpression', 'DropPayloadEnum']);
}
$require($phpPath, $php, ['fn emit_enum(', 'fn emit_payload_enum(', '__DoriaValueEquatable', '__doriaEquals']);
$require($testsPath, $tests, [
    'unit_and_backed_enums_execute_with_nominal_equality_and_value_projection',
    'nullable_enum_keeps_first_case_distinct_from_null_and_supports_narrowing',
    'enum_identity_survives_mixed_boxing_and_exact_narrowing',
    'payload_execution_and_core_match_are_both_available_after_stage_27',
    'payload_construction_copy_constants_defaults_and_generic_storage_execute',
    'payload_ownership_layout_equality_and_observation_boundaries_are_checked',
    'payload_case_calls_reuse_the_normal_argument_binding_rules',
]);
$require($manifestPath, $manifest, [
    'examples/native/main_unit_enums.doria',
    'examples/native/main_backed_enums.doria',
    'examples/native/main_nullable_enums.doria',
    'examples/native/main_enum_mixed.doria',
    'examples/native/main_enum_constants_and_defaults.doria',
    'examples/native/main_payload_enums_copy.doria',
    'examples/native/main_payload_enums_move.doria',
    'examples/native/main_payload_enums_nullable.doria',
    'examples/native/main_payload_enums_mixed.doria',
    'examples/native/main_payload_enums_collections.doria',
    'examples/native/main_payload_enums_abi.doria',
    'examples/native/main_payload_enums_drop_order.doria',
]);

if (str_contains($semantics, 'Payload Enum Execution Lands In Stage 27 Slice 2')) {
    $failures[] = "{$semanticsPath}: E0573 still has an active valid-source route";
}

foreach ([$craneliftPath => $cranelift, $llvmPath => $llvm] as $path => $contents) {
    if (str_contains($contents, 'dr_v1_enum_')) {
        $failures[] = "{$path}: unit/backed enums must not introduce an enum allocation ABI";
    }
}

$fixtureContents = $tests;
foreach (glob($root . '/examples/native/main_*enum*.doria') ?: [] as $fixture) {
    $contents = @file_get_contents($fixture);
    if (is_string($contents)) {
        $fixtureContents .= $contents;
    }
}
foreach (['Andrew', 'Lucy', 'Masiye'] as $privateName) {
    if (str_contains($fixtureContents, $privateName)) {
        $failures[] = "Stage 27 tests or fixtures contain private or family name `{$privateName}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "enum foundation check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "enum foundation check passed\n");
