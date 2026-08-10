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
$stdlibPath = 'docs/stdlib-reference.md';
$parityPath = 'docs/notes/native-parity-matrix.md';
$auditPath = 'docs/notes/collection-surface-audit.md';
$tablePath = 'crates/doriac/src/collection_diagnostics.rs';
$semanticsPath = 'crates/doriac/src/semantics.rs';
$mirPath = 'crates/doriac/src/mir.rs';
$testsPath = 'crates/doriac/tests/collection_diagnostic_tests.rs';
$stage26TestsPath = 'crates/doriac/tests/stage26_tests.rs';
$fixturePath = 'examples/native/main_stage26_collection_slice3.doria';
$manifestPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';

$decision = required_contents($root, $decisionPath, $failures);
$plan = required_contents($root, $planPath, $failures);
$pipeline = required_contents($root, $pipelinePath, $failures);
$stdlib = required_contents($root, $stdlibPath, $failures);
$parity = required_contents($root, $parityPath, $failures);
$audit = required_contents($root, $auditPath, $failures);
$table = required_contents($root, $tablePath, $failures);
$semantics = required_contents($root, $semanticsPath, $failures);
$mir = required_contents($root, $mirPath, $failures);
$tests = required_contents($root, $testsPath, $failures);
$stage26Tests = required_contents($root, $stage26TestsPath, $failures);
$fixture = required_contents($root, $fixturePath, $failures);
$manifest = required_contents($root, $manifestPath, $failures);

require_all($decisionPath, $decision, [
    '**Slice 1 — Complete.**',
    '**Slice 2 — Complete.**',
    '**Slice 3 — Complete.**',
    '**Slice 4 — Next.**',
    '**Stage 27 — Sequenced after Decision 0113.**',
    '`List::indexOf` | O(n) | None',
    '`List::remove` | O(n), including one tail shift | None',
    '`Dictionary::containsValue` | O(n) | None',
    '`Set::first` / `last` | O(1) | None',
], $failures);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    require_all($path, $contents, [
        'Stage 26b — Complete',
        'Measurement Status: Pending Available Runner',
        'Decision 0113 Slice 2 — Complete',
        'Decision 0113 Slice 3 — Complete',
        'Decision 0113 Slice 4 — Next',
        'Stage 27 — Sequenced After Decision 0113',
    ], $failures);
}

require_all($stdlibPath, $stdlib, [
    'Decision 0113 Slices 1-3 are implemented',
    '`indexOf(T): ?int`',
    'writable `remove(T): bool`',
    'executable O(n) `containsValue(V): bool`',
    'executable readonly `first: ?T` / `last: ?T` properties',
    'Writable `clear(): void` remains accepted pending Slice 4',
], $failures);

require_all($parityPath, $parity, [
    'Decision 0113 Slice 3 collection members',
    'main_stage26_collection_slice3.doria',
    'Slice 4',
    '`clear()` still stops before MIR with E0559',
], $failures);

require_all($auditPath, $audit, [
    'Decision 0113 Slices 1-3 are complete',
    'Slice 4 `clear()` is next',
], $failures);

require_all($tablePath, $table, [
    'COLLECTION_MEMBER_SUGGESTIONS',
    'pub fn suggestion_for(',
    'pending_method_status',
    '"clear",',
    'decision_owner: "Decision 0113"',
], $failures);

if (str_contains($table, 'PendingSlice3')) {
    $failures[] = "{$tablePath}: completed Slice 3 must not remain in the pending-state model";
}

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

require_all($mirPath, $mir, [
    'CollectionIndexOf',
    'ContainsValue',
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
    'slice_three_members_execute_and_only_slice_four_remains_pending',
    'equality_diagnostics_name_the_actual_collection_operation',
    'valid-from-families.doria',
    'Set::from([1])',
    'SortedSet::from([1])',
    'SortedDictionary::from',
    'PriorityQueue::from([1])',
    'Deque::from([1])',
], $failures);

require_all($stage26TestsPath, $stage26Tests, [
    'decision_0113_slice_three_example_executes_in_the_semantic_oracle',
    'collection_search_receiver_and_probe_evaluate_once_in_source_order',
], $failures);

require_all($manifestPath, $manifest, [
    'examples/native/main_stage26_collection_slice3.doria',
], $failures);

foreach (['Andrew', 'Lucy', 'Masiye'] as $privateName) {
    if (str_contains($tests, $privateName) || str_contains($fixture, $privateName)) {
        $failures[] = "Slice 3 tests or fixtures contain private or family name `{$privateName}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "collection surface diagnostic check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "collection surface diagnostic check passed\n");
