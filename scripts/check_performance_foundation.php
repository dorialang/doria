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
            '## Slice 2 Structural Contract',
            '## Slice 3 Peer Fairness Contract',
            '## Slice 3 Threshold Contract',
            '## Slicing',
            'call_overhead',
            'checked_arith',
            'element_access',
            'Stage 27 remains blocked',
        ],
        'docs/doria-end-to-end-plan.md' => [
            'Decision 0112: performance baseline, provenance, and regression measurement',
            'Stage 26b — Performance Baseline Foundation — In Progress; Slices 1 And 2 Complete, Slice 3 In Progress.',
            'Stage 26b Slice 1 — Complete',
            'Stage 26b Slice 2 — Complete',
            'Stage 26b Slice 3 — In Progress',
            'Stage 26b Timing Threshold Review — Next',
            'Stage 27 — Enums + payload cases — Blocked Until Stage 26b Completes.',
        ],
        'docs/performance-and-benchmarking.md' => [
            'Decision 0112',
            'Slices 1 and 2 are complete',
            'Slice 3 Part 1 is delivered',
            'peer equivalence record',
            '--performance-report <file>',
            'five warmups',
            'exact structural baseline without timing thresholds',
        ],
        'docs/notes/current-pipeline.md' => [
            'Stage 26b — Performance Baseline Foundation — In Progress.',
            'Stage 26b — In Progress.',
            'Stage 26b Slice 1 — Complete.',
            'Stage 26b Slice 2 — Complete.',
            'Stage 26b Slice 3 — In Progress. Part 1 delivered.',
            'Stage 26b Timing Threshold Review — Next.',
            'Slices 1 and 2 are complete',
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
            'mirBasicBlockCount',
            'mirStatementCount',
            'mirTerminatorCount',
            'runtimeArtifactBytes',
            'borrowChecking',
            'integrated into semanticAnalysis',
        ],
        'crates/doriac/src/main.rs' => [
            '--performance-report',
            'Performance Report Could Not Be Written',
            'B2601',
        ],
        'crates/doriac/src/codegen_native.rs' => [
            'runtime_artifact: PathBuf',
            'generate_executable_with_performance',
        ],
        'benchmarks-revision.json' => [
            '"repository": "dorialang/benchmarks"',
            '"revision": "30c6e3996a89a9b11cda683db8e06136e914453a"',
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
