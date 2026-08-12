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
$clearFixturePath = 'examples/native/main_collection_clear.doria';
$clearOwnershipFixturePath = 'examples/native/main_collection_clear_ownership.doria';
$manifestPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';
$loweringPath = 'crates/doriac/src/mir_lowering.rs';
$runtimePath = 'crates/doria-rt/src/lib.rs';
$phpPath = 'crates/doriac/src/codegen_php.rs';

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
$clearFixture = required_contents($root, $clearFixturePath, $failures);
$clearOwnershipFixture = required_contents($root, $clearOwnershipFixturePath, $failures);
$manifest = required_contents($root, $manifestPath, $failures);
$lowering = required_contents($root, $loweringPath, $failures);
$runtime = required_contents($root, $runtimePath, $failures);
$php = required_contents($root, $phpPath, $failures);

require_all($decisionPath, $decision, [
    '**Slice 1 — Complete.**',
    '**Slice 2 — Complete.**',
    '**Slice 3 — Complete.**',
    '**Slice 4 — Complete.**',
    '**Decision 0113 — Complete.**',
    '**Stage 27 — Complete.**',
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
        'Decision 0113 Slice 4 — Complete',
        'Decision 0113 — Complete',
        'Stage 27 — Complete',
    ], $failures);
}

require_all($stdlibPath, $stdlib, [
    'All four Decision 0113 slices are implemented',
    '`indexOf(T): ?int`',
    'writable `remove(T): bool`',
    'executable O(n) `containsValue(V): bool`',
    'executable readonly `first: ?T` / `last: ?T` properties',
    'writable `clear(): void`',
], $failures);

require_all($parityPath, $parity, [
    'Decision 0113 Slice 3 collection members',
    'main_stage26_collection_slice3.doria',
    'Decision 0113 Slice 4 collection clear',
    'main_collection_clear.doria',
], $failures);

require_all($auditPath, $audit, [
    'Decision 0113 and all four slices are complete',
    '`clear(): void` executable in place',
], $failures);

require_all($tablePath, $table, [
    'COLLECTION_MEMBER_SUGGESTIONS',
    'pub fn suggestion_for(',
], $failures);

foreach (['PendingSlice3', 'PendingSlice4', 'pending_method_status'] as $stalePending) {
    if (str_contains($table, $stalePending)) {
        $failures[] = "{$tablePath}: completed Decision 0113 must not retain `{$stalePending}`";
    }
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
], $failures);

if (str_contains($semantics, '"E0559"')) {
    $failures[] = "{$semanticsPath}: completed Decision 0113 must not route a member to E0559";
}

require_all($mirPath, $mir, [
    'CollectionIndexOf',
    'ContainsValue',
    'CollectionClear',
], $failures);

require_all($loweringPath, $lowering, [
    'mir::Statement::CollectionClear',
    '"clear"',
], $failures);

require_all($runtimePath, $runtime, [
    'dr_v2_collection_reset_after_cleanup',
], $failures);

if (substr_count($php, 'public function clear(): void') !== 4) {
    $failures[] = "{$phpPath}: all four ordered PHP helpers must expose clear(): void";
}

foreach (['List::contains for', '"get" | "has"', '"containsKey" | "has"'] as $forbidden) {
    if (str_contains($semantics, $forbidden)) {
        $failures[] = "{$semanticsPath}: forbidden stale or executable `has` contract `{$forbidden}`";
    }
}

require_all($testsPath, $tests, [
    'map_membership_spellings_have_receiver_aware_applied_fixes',
    'property_invocation_has_safe_and_combined_fixes',
    'withdrawn_literal_constructors_preserve_source_and_context',
    'decision_0113_members_all_execute',
    'clear_uses_existing_collection_diagnostics',
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
    'examples/native/main_collection_clear.doria',
    'examples/native/main_collection_clear_ownership.doria',
], $failures);

foreach (['Andrew', 'Lucy', 'Masiye'] as $privateName) {
    if (str_contains($tests, $privateName)
        || str_contains($fixture, $privateName)
        || str_contains($clearFixture, $privateName)
        || str_contains($clearOwnershipFixture, $privateName)) {
        $failures[] = "Decision 0113 tests or fixtures contain private or family name `{$privateName}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "collection surface diagnostic check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "collection surface diagnostic check passed\n");
