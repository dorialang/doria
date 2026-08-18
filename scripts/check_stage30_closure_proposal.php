<?php

declare(strict_types=1);

/**
 * @return list<string>
 */
function check_stage30_closure_proposal(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required file is missing or unreadable";
            return '';
        }

        return $contents;
    };

    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 30 proposal marker `{$needle}`";
            }
        }
    };

    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: forbidden Stage 30 proposal claim `{$needle}`";
            }
        }
    };

    $proposalPath = 'docs/notes/stage30-closure-authority-proposal.md';
    $pipelinePath = 'docs/notes/current-pipeline.md';
    $examplesPath = 'examples/future/stage30/README.md';
    $decisionPath = 'docs/decisions/0120-explicit-closure-capture-lists.md';
    $specPath = 'SPEC.md';
    $semanticsPath = 'crates/doriac/src/semantics.rs';

    $proposal = $read($proposalPath);
    $pipeline = $read($pipelinePath);
    $examples = $read($examplesPath);
    $decision = $read($decisionPath);
    $spec = $read($specPath);
    $semantics = $read($semanticsPath);

    $require($proposalPath, $proposal, [
        '# Stage 30 Closure Authority Proposal',
        '## Status',
        '**In Review.**',
        'supporting design proposal, not accepted language',
        'Stage 30 is not',
        '## Andrew Decision Checklist',
        '## Existing Authority',
        '## Current Implementation Inventory',
        '## Executive Recommendation',
        '## Detailed Decision Areas',
        '## Proposed Grammar Consequences',
        '## Proposed Semantic Model',
        '## Proposed ABI And Runtime Model',
        '## Proposed Collection Algorithm Surface',
        '## Proposed Diagnostics',
        '## Proposed Implementation Slices',
        '## Performance And Memory Contract',
        '## Compatibility And Tooling Consequences',
        '## Explicit Deferrals',
        '## Invalidated elsewhere',
    ]);

    for ($decisionNumber = 1; $decisionNumber <= 20; $decisionNumber++) {
        if (!str_contains($proposal, "| D{$decisionNumber} |")) {
            $failures[] = "{$proposalPath}: missing Andrew decision checklist row D{$decisionNumber}";
        }
    }

    for ($area = 1; $area <= 27; $area++) {
        if (preg_match('/^### ' . $area . '\\. /m', $proposal) !== 1) {
            $failures[] = "{$proposalPath}: missing detailed decision area {$area}";
        }
    }

    $areaLabels = [
        'Current authority.',
        'Question.',
        'Viable options.',
        'Tradeoffs.',
        'Recommendation.',
        'Consequences.',
        'Invalidated elsewhere.',
        'Andrew decision.',
    ];
    foreach ($areaLabels as $label) {
        if (substr_count($proposal, "**{$label}**") < 27) {
            $failures[] = "{$proposalPath}: every detailed area must contain `{$label}`";
        }
    }

    $forbid($proposalPath, $proposal, [
        '**Status:** Accepted',
        'Stage 30 is implemented',
        'Stage 30 — Complete',
    ]);

    $require($pipelinePath, $pipeline, [
        '[Stage 30 Closure Authority Proposal](stage30-closure-authority-proposal.md) — In Review.',
        'Stage 30 — Next, Not Implemented.',
    ]);
    $require($examplesPath, $examples, [
        '[Stage 30 Closure Authority Proposal](../../../docs/notes/stage30-closure-authority-proposal.md)',
        '**In Review**',
        '**Stage 30 is not implemented**',
    ]);
    $require($decisionPath, $decision, [
        '# Decision 0120: Explicit Closure Capture Lists',
        '**Status:** Accepted',
        'Stage 30 is next and remains unimplemented',
    ]);
    $require($specPath, $spec, [
        'catalogued `E0641` Stage 30 development boundary',
        '`$this` capture remains a bounded Stage 30',
    ]);
    $forbid($specPath, $spec, [
        'stage30-closure-authority-proposal.md',
        'Stage 30 Closure Authority Proposal',
    ]);
    $require($semanticsPath, $semantics, [
        '"E0641"',
        'Closure Semantics Await Stage 30',
    ]);

    $prematureRecords = glob($root . '/docs/decisions/*stage30*closure*authority*.md') ?: [];
    foreach ($prematureRecords as $path) {
        $failures[] = substr($path, strlen($root) + 1)
            . ': numbered Stage 30 closure authority record was allocated before proposal approval';
    }

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_stage30_closure_proposal(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 30 closure proposal check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Stage 30 closure proposal check passed\n");
}
