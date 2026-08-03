<?php

declare(strict_types=1);

/** @return list<string> */
function check_performance_foundation(string $root): array
{
    $requirements = [
        'docs/decisions/0112-performance-baseline-provenance-and-regression-measurement.md' => [
            '**Status:** Accepted',
            '## Correctness Contract',
            '## Manifest And Report Contract',
            '## Provenance Contract',
            '## Compiler Evidence',
            '## Initial Diagnostic Pairs',
            '## Regression Policy',
            '## Slicing',
            'call_overhead',
            'checked_arith',
            'element_access',
            'Stage 27 remains blocked',
        ],
        'docs/doria-end-to-end-plan.md' => [
            'Decision 0112: performance baseline, provenance, and regression measurement',
            'Stage 26b — Performance Baseline Foundation — In Progress; Slice 1 Complete, Slice 2 Next.',
            'Stage 26b Slice 1 — Complete',
            'Stage 26b Slice 2 — Next',
            'Stage 27 — Enums + payload cases — Blocked Until Stage 26b Completes.',
        ],
        'docs/performance-and-benchmarking.md' => [
            'Decision 0112',
            'Slice 1 is complete',
            '--performance-report <file>',
            'five warmups',
            'Quick reports are explicitly baseline-ineligible',
        ],
        'docs/notes/current-pipeline.md' => [
            'Stage 26b — Performance Baseline Foundation — In Progress.',
            'Stage 26b — In Progress.',
            'Stage 26b Slice 1 — Complete.',
            'Stage 26b Slice 2 — Next.',
            'Slice 1 is complete',
            'Slice 2, the controlled baseline matrix, is next',
            'Stage 27 — Blocked Until Stage 26b Completes.',
        ],
        'AGENTS.md' => [
            '`dorialang/benchmarks` repository',
            '`benchmarks-revision.json` pins the coordinated revision',
            'A future `baton bench` orchestrates that same engine',
        ],
        'README.md' => [
            '`dorialang/benchmarks`',
            'keeps Cranelift and',
            'LLVM results distinct',
            'Public comparisons are workload-specific and reproducible',
        ],
        'docs/notes/native-parity-matrix.md' => [
            'semantic and correctness authority, not a performance table',
            'interpreter as an oracle rather than a native competitor',
        ],
        'crates/doriac/src/performance.rs' => [
            'REPORT_SCHEMA_VERSION',
            'callableSpecializationCount',
            'classSpecializationCount',
            'borrowChecking',
            'integrated into semanticAnalysis',
        ],
        'crates/doriac/src/main.rs' => [
            '--performance-report',
            'Performance Report Could Not Be Written',
            'B2601',
        ],
        'benchmarks-revision.json' => [
            '"repository": "dorialang/benchmarks"',
            '"revision": "6ae7a8a40cc048936cace3b9c73c62e4fdfb04ab"',
        ],
        'scripts/check_benchmark_revision.php' => [
            'check_benchmark_revision',
            'sibling benchmarks repository',
            'manifest.json',
            'report-schema.json',
        ],
    ];

    $failures = [];
    foreach ($requirements as $path => $needles) {
        $contents = file_get_contents($root . '/' . $path);
        if ($contents === false) {
            $failures[] = "{$path}: unable to read performance-foundation authority";
            continue;
        }
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing performance-foundation authority {$needle}";
            }
        }
    }
    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_performance_foundation(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "performance foundation check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    echo "performance foundation check passed\n";
}
