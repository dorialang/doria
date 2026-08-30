<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$decisionPath = $root . '/docs/decisions/0129-native-testing-foundation-behavioral-dsl-expectations-and-assertion-outcomes.md';
$referencePath = $root . '/docs/native-testing-foundation.md';

foreach ([$decisionPath, $referencePath] as $path) {
    if (!is_file($path)) {
        fwrite(STDERR, "Missing native testing authority file: {$path}\n");
        exit(1);
    }
}

$decision = file_get_contents($decisionPath);
$reference = file_get_contents($referencePath);

if ($decision === false || $reference === false) {
    fwrite(STDERR, "Unable to read native testing authority files.\n");
    exit(1);
}

$requiredDecisionFacts = [
    '# Decision 0129: Native Testing Foundation, Behavioral DSL, Fluent Expectations, And Assertion Outcomes',
    '**Status:** Accepted',
    '**Implementation Status:** Slice 1 Implemented; Slice 2 Next; Foundation In Progress',
    'Native Testing Foundation Slice 1 - Complete',
    'Native Testing Foundation Slice 2 - Next',
    'Native Testing Foundation - In Progress, Not Complete',
    'Slice 1 - Behavioral Test DSL And Unified Compiler Metadata',
    'Slice 2 - Fluent Expectation Kernel And Assertion Semantics',
    'Slice 3 - Collection/Error Expectations, Baton Reporting, And Tooling Closure',
    'Doria\\Std\\Test',
    'describe',
    'it',
    'test',
    'expect',
    'fail',
    'AssertionError',
    'TestAssertion',
    'metadata schema version 3',
    'Baton schema-3 orchestration',
    'language-server presentation',
    'Stage 34 Single Class Inheritance - Blocked Until The Foundation Completes',
    'This is a deferral, not a permanent rejection.',
];

$requiredReferenceFacts = [
    'Native Testing Foundation Slice 1 - Complete',
    'Native Testing Foundation Slice 2 - Next',
    'Native Testing Foundation - In Progress, Not Complete',
    'Stage 34 Single Class Inheritance - Blocked Until The Foundation Completes',
    'describe("Shopping Cart"',
    'expect($cart->total)->toEqual(0)',
    'expect($cart->items)->not->toContain',
    'Metadata schema versions 1 and 2 remain exact.',
    'Schema version 3 adds:',
    'no runtime suite registry',
    'no source parsing in Baton',
    '`expect(...)` in this document is accepted future surface',
    'beforeEach / afterEach',
    'They are deferred, not permanently rejected.',
];

$missing = [];
foreach ($requiredDecisionFacts as $fact) {
    if (!str_contains($decision, $fact)) {
        $missing[] = "decision: {$fact}";
    }
}
foreach ($requiredReferenceFacts as $fact) {
    if (!str_contains($reference, $fact)) {
        $missing[] = "reference: {$fact}";
    }
}

$forbidden = [
    'Assertion failure is panic',
    'Baton parses Doria source',
    'runtime suite registry: required',
    'Nested testing hooks are permanently rejected',
];

foreach ($forbidden as $fact) {
    if (str_contains($decision, $fact) || str_contains($reference, $fact)) {
        $missing[] = "forbidden wording: {$fact}";
    }
}

if ($missing !== []) {
    fwrite(STDERR, "Native testing foundation authority is incomplete:\n");
    foreach ($missing as $fact) {
        fwrite(STDERR, "- {$fact}\n");
    }
    exit(1);
}

echo "Native Testing Foundation authority is internally consistent.\n";
