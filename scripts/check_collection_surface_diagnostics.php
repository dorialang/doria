<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

/** @return string */
function required_contents(string $root, string $path, array &$failures): string
{
    $contents = @file_get_contents($root . '/' . $path);
    if (!is_string($contents)) {
        $failures[] = "{$path}: required file is missing or unreadable";
        return '';
    }
    return $contents;
}

/** @param list<string> $needles */
function require_all(string $path, string $contents, array $needles, array &$failures): void
{
    foreach ($needles as $needle) {
        if (!str_contains($contents, $needle)) {
            $failures[] = "{$path}: missing required collection-diagnostic contract `{$needle}`";
        }
    }
}

function require_suggestion(string $table, string $input, string $canonical, array &$failures): void
{
    $inputPattern = preg_quote($input, '/');
    $canonicalPattern = preg_quote($canonical, '/');
    if (preg_match(
        '/suggestion!\(\s*"' . $inputPattern . '"\s*,\s*"' . $canonicalPattern . '"/s',
        $table,
    ) !== 1) {
        $failures[] = "crates/doriac/src/collection_diagnostics.rs: missing `{$input}` to `{$canonical}` mapping";
    }
}

$decisionPath = 'docs/decisions/0113-collection-surface-completion.md';
$planPath = 'docs/doria-end-to-end-plan.md';
$pipelinePath = 'docs/notes/current-pipeline.md';
$tablePath = 'crates/doriac/src/collection_diagnostics.rs';
$semanticsPath = 'crates/doriac/src/semantics.rs';
$testsPath = 'crates/doriac/tests/collection_diagnostic_tests.rs';

$decision = required_contents($root, $decisionPath, $failures);
$plan = required_contents($root, $planPath, $failures);
$pipeline = required_contents($root, $pipelinePath, $failures);
$table = required_contents($root, $tablePath, $failures);
$semantics = required_contents($root, $semanticsPath, $failures);
$tests = required_contents($root, $testsPath, $failures);

require_all($decisionPath, $decision, [
    '**Slice 1 — Complete.**',
    '**Slice 2 — Complete.**',
    '**Slice 3 — Next.**',
    '**Slice 4 — Pending.**',
    '**Stage 27 — Sequenced after Decision 0113.**',
    'One bounded receiver-aware table lookup after failed collection-member resolution',
    'Generated runtime | No change',
    'Runtime representation or ABI | No change',
], $failures);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    require_all($path, $contents, [
        'Stage 26b — Complete',
        'Measurement Status: Pending Available Runner',
        'Decision 0113 Slice 2 — Complete',
        'Decision 0113 Slice 3 — Next',
        'Decision 0113 Slice 4 — Pending',
        'Stage 27 — Sequenced After Decision 0113',
    ], $failures);
}

require_all($tablePath, $table, [
    'COLLECTION_MEMBER_SUGGESTIONS',
    'pub fn suggestion_for(',
    'pending_method_status',
    '(List, "indexOf" | "remove")',
    '"clear",',
    'decision_owner: "Decision 0113"',
], $failures);

foreach ([
    ['has', 'containsKey'],
    ['in_array', 'contains'],
    ['length', 'count'],
    ['find', 'indexOf'],
    ['Enqueue', 'pushBack'],
] as [$input, $canonical]) {
    require_suggestion($table, $input, $canonical, $failures);
}

require_all($semanticsPath, $semantics, [
    '"E0521"',
    '.with_structured_fix(',
    '"E0557"',
    '"Property Is Not A Method"',
    'fn check_stage23_equatable_type(&mut self, ty: TypeId, span: Span, operation: &str)',
    '"E0558"',
    '"Use A Collection Literal"',
    'List and Dictionary use bracket literals',
    '"E0559"',
    'Accepted Collection Member Is Not Executable Yet',
], $failures);

foreach (['List::contains for', '"get" | "has"', '"containsKey" | "has"'] as $forbidden) {
    if (str_contains($semantics, $forbidden)) {
        $failures[] = "{$semanticsPath}: forbidden stale or executable `has` contract `{$forbidden}`";
    }
}

require_all($testsPath, $tests, [
    'map_membership_spellings_have_receiver_aware_applied_fixes',
    'property_invocation_has_safe_and_combined_fixes',
    'withdrawn_literal_constructors_preserve_source_and_context',
    'accepted_pending_members_stop_before_lowering_with_their_slice_owner',
    'equality_diagnostics_name_the_actual_collection_operation',
    'valid-from-families.doria',
    'Set::from([1])',
    'SortedSet::from([1])',
    'SortedDictionary::from',
    'PriorityQueue::from([1])',
    'Deque::from([1])',
], $failures);

foreach (['Andrew', 'Lucy', 'Masiye'] as $privateName) {
    if (str_contains($tests, $privateName)) {
        $failures[] = "{$testsPath}: private or family name `{$privateName}` is not allowed in fixtures";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "collection surface diagnostic check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "collection surface diagnostic check passed\n");
