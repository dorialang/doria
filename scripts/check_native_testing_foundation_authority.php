<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$decisionPath = $root . '/docs/decisions/0129-native-testing-foundation-behavioral-dsl-expectations-and-assertion-outcomes.md';
$referencePath = $root . '/docs/native-testing-foundation.md';
$statusPaths = [
    $root . '/SPEC.md',
    $root . '/docs/doria-end-to-end-plan.md',
    $root . '/docs/stdlib-reference.md',
    $root . '/docs/self-hosting.md',
    $root . '/docs/notes/current-pipeline.md',
    $root . '/docs/notes/plan-open-questions-audit.md',
    $root . '/docs/notes/temporary-language-restrictions-audit.md',
];

foreach ([$decisionPath, $referencePath, ...$statusPaths] as $path) {
    if (!is_file($path)) {
        fwrite(STDERR, "Missing native testing authority file: {$path}\n");
        exit(1);
    }
}

$decision = file_get_contents($decisionPath);
$reference = file_get_contents($referencePath);
$status = '';

foreach ($statusPaths as $path) {
    $text = file_get_contents($path);
    if ($text === false) {
        fwrite(STDERR, "Unable to read native testing status file: {$path}\n");
        exit(1);
    }
    $status .= "\n" . $text;
}

if ($decision === false || $reference === false) {
    fwrite(STDERR, "Unable to read native testing authority files.\n");
    exit(1);
}

$requiredDecisionFacts = [
    '# Decision 0129: Native Testing Foundation, Behavioral DSL, Fluent Expectations, And Assertion Outcomes',
    '**Status:** Accepted',
    '**Implementation Status:** Implemented By Native Testing Foundation Slices 1 Through 3',
    'Native Testing Foundation Slice 1 - Complete',
    'Native Testing Foundation Slice 2 - Complete',
    'Native Testing Foundation Slice 3 - Complete',
    'Native Testing Foundation - Complete',
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
    'Stage 34 Single Class Inheritance - Complete',
    'Stage 35 Interfaces And Traits - Authority Accepted; Slice 1 Next',
    'Pre-Stage-45 Doria-Native Baton Transition - Scheduled',
    'This is a deferral, not a permanent rejection.',
];

$requiredReferenceFacts = [
    'Native Testing Foundation Slice 1 - Complete',
    'Native Testing Foundation Slice 2 - Complete',
    'Native Testing Foundation Slice 3 - Complete',
    'Native Testing Foundation - Complete',
    'Stage 34 Single Class Inheritance - Complete',
    'Stage 35 Interfaces And Traits - Authority Accepted; Slice 1 Next',
    'describe("Shopping Cart"',
    'expect($cart->total)->toEqual(0)',
    'expect($cart->items)->not->toContain',
    'Metadata schema versions 1 and 2 remain exact.',
    'Schema version 3 adds:',
    'no runtime suite registry',
    'no source parsing in Baton',
    'Baton regression integration',
    'Slice 3\'s collection/Error expectations are',
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
    'Compiler/Runtime Implemented, Baton/Tooling Pending',
    'Baton/tooling coordination remains pending',
    'Slice 2 is next',
    'Slice 2 — Next',
    'Slice 3 Compiler/Runtime In Progress',
    'Native Testing Foundation - In Progress, Not Complete',
    'Blocked Until The Foundation Completes',
];

foreach ($forbidden as $fact) {
    if (str_contains($decision, $fact) || str_contains($reference, $fact) || str_contains($status, $fact)) {
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
