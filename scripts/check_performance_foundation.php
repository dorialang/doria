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
            '## Native Runtime Acceptance Standard',
            '## Development And Release Evidence Gates',
            '## Portable Stage 26b Closure Contract',
            '## Slicing',
            'call_overhead',
            'checked_arith',
            'element_access',
            'Eligible timing evidence does not gate language or compiler stage progression',
            'Measurement Status: Pending Available Runner',
            'Stage 27 is sequenced after Decision 0113',
            'Doria Median / Fastest Valid Native Peer Median <= 1.30',
            'A ratio greater than `1.30` is',
            'Inconclusive**, never Pass',
            'does not change it into a pass',
            'fastest valid,',
            'C, C++, or Rust result',
            'PHP is adoption evidence',
            'do not create a qualified pass',
            "Only Andrew, as Doria's language designer, may change the `1.30` boundary",
            'Compile time, link time, startup, peak memory,',
        ],
        'docs/doria-end-to-end-plan.md' => [
            'Decision 0112: performance baseline, provenance, and regression measurement',
            'Stage 26b — Performance Baseline Foundation — Complete; All Three Slices Complete.',
            'Stage 26b Slice 1 — Complete',
            'Stage 26b Slice 2 — Complete',
            'Stage 26b Slice 3 — Complete',
            'Measurement Status: Pending Available Runner',
            'Decision 0113 Slices 2-4 — Next',
            'Stage 27 — Enums + payload cases — Sequenced After Decision 0113; No Performance-Evidence Dependency.',
        ],
        'docs/performance-and-benchmarking.md' => [
            'Decision 0112',
            'All three slices are complete',
            'Slice 3 Part 1 is delivered',
            'peer equivalence record',
            '--performance-report <file>',
            'five warmups',
            'exact structural baseline without timing thresholds',
            'ratio <= 1.30',
            'Measurement Status: Pending Available Runner',
            'are not Stage 26b or Stage 27 closure conditions',
        ],
        'docs/notes/current-pipeline.md' => [
            'Stage 26b — Performance Baseline Foundation — Complete.',
            'Stage 26b — Complete.',
            'Stage 26b Slice 1 — Complete.',
            'Stage 26b Slice 2 — Complete.',
            'Stage 26b Slice 3 — Complete.',
            'Measurement Status: Pending Available Runner.',
            'Decision 0113 Slices 2-4 — Next.',
            'all three slices are complete',
            'Stage 27 — Sequenced After Decision 0113; No Performance-Evidence Dependency.',
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
            '"linker"',
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
            'link_object_with_metadata',
            'link_command',
            'generate_executable_with_performance',
        ],
        'benchmarks-revision.json' => [
            '"repository": "dorialang/benchmarks"',
            '"revision": "6234ccd6ac34589734998f3ab0967a5fdb5051e8"',
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

    $activeStatusFiles = [
        'docs/decisions/0112-performance-baseline-provenance-and-regression-measurement.md',
        'docs/doria-end-to-end-plan.md',
        'docs/notes/current-pipeline.md',
        'docs/performance-and-benchmarking.md',
    ];
    $hardwareGatePatterns = [
        '/Stage (?:26b|27)[^\n]*(?:blocked|wait)[^\n]*(?:Linux|affinity|Callgrind|DHAT|hardware|timing)/i',
        '/(?:Linux|affinity|Callgrind|DHAT|hardware|timing)[^\n]*(?:blocks?|wait)[^\n]*Stage (?:26b|27)/i',
    ];
    foreach ($activeStatusFiles as $path) {
        $contents = file_get_contents($root . '/' . $path);
        if ($contents === false) {
            continue;
        }
        foreach ($hardwareGatePatterns as $pattern) {
            if (preg_match($pattern, $contents) === 1) {
                $failures[] = "{$path}: hardware availability must not gate Stage 26b or Stage 27";
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
