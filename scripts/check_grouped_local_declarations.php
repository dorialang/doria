<?php

declare(strict_types=1);

/** @return list<string> */
function check_grouped_local_declarations(string $root): array
{
    $failures = [];
    $requirements = [
        'SPEC.md' => [
            'let $left, $right = 0;',
            'let writable $red, $green, $blue = 0;',
            'int $minimum, $maximum = 0;',
            'writable int $x, $y = 0;',
            'evaluated exactly once',
            'initialize from left to right',
            'clean up in reverse order',
            'Move values are rejected',
            '?Token $left, $right = null;',
            'untyped grouped `null` is rejected',
            'no runtime object',
            'Grouping is local-only',
        ],
        'README.md' => [
            'let writable $red, $green, $blue = 0;',
            'never hide cloning, sharing, or a runtime tuple',
        ],
        'docs/decisions/0111-grouped-local-declarations.md' => [
            '**Status:** Accepted',
            '## Canonical Syntax',
            '## Common Type And Mutability',
            '## Single Evaluation',
            '## Copy Eligibility',
            '## Move-Type Rejection',
            '## Nullable Empty Initialization',
            '## Scope And Name Resolution',
            '## Initialization Order',
            '## Destruction Order',
            '## Lowering Model',
            '## Performance Contract',
            '## Diagnostics',
            '## Explicit Non-Goals',
            '## Consequences',
            '## Invalidated Elsewhere',
            'never duplicates the string contents',
            'does not hide a `clone()`, `share()`',
            'no runtime group object',
            'target-state website',
        ],
        'docs/doria-end-to-end-plan.md' => [
            'Decision 0111: grouped local declarations',
            'Stage 26a — Grouped local declarations — Complete.',
            'Stage 26b — Performance Baseline Foundation — Next.',
            'Stage 27 — Enums + payload cases — Blocked Until Stage 26b Completes.',
            'Stage 35a — Optimizer Contracts, Dispatch, And Escape Audit — Scheduled.',
            'Stage 43 — Engine Performance And Optimization Hardening.',
            'Continuous performance rule',
            '`Performance Impact` section',
            'Stage 36a owns the initial',
        ],
        'docs/performance-and-benchmarking.md' => [
            '## Stage 26a grouped-local contract',
            '## Stage 26b performance baseline foundation',
            'Compiler Performance',
            'Generated-Program Performance',
            'Runtime-Subsystem Performance',
            '## Continuous performance impact rule',
            'Stage 36a\'s stream gate',
            'php ../benchmarks/fibonacci/fibonacci.php',
            'node ../benchmarks/fibonacci/fibonacci.js',
            'python3 ../benchmarks/fibonacci/fibonacci.py',
        ],
        'docs/notes/current-pipeline.md' => [
            'Stage 26 — Complete.',
            'Stage 26a — Complete.',
            'Stage 26b — Performance Baseline Foundation — Next.',
            'Stage 27 — Blocked Until Stage 26b Completes.',
            'Stage 35a — Optimizer Contracts, Dispatch, And Escape Audit — Scheduled.',
            'Stage 36a — Scheduled, Not Implemented.',
        ],
        'docs/notes/native-parity-matrix.md' => [
            '| Grouped local declarations | Covered | Covered | Covered | Covered |',
            'The Stage',
            '26a fixture covers all four grouped-local prefixes',
        ],
        '.github/workflows/ci.yml' => [
            'Leak-check Stage 26a grouped string locals with Cranelift',
            'Leak-check Stage 26a grouped string locals with LLVM',
            'main_stage26a_grouped_locals',
        ],
    ];

    foreach ($requirements as $path => $needles) {
        $contents = file_get_contents($root . '/' . $path);
        if ($contents === false) {
            $failures[] = "{$path}: unable to read grouped-local authority";
            continue;
        }
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing grouped-local authority {$needle}";
            }
        }
    }

    $performance = file_get_contents($root . '/docs/performance-and-benchmarking.md');
    if ($performance !== false) {
        foreach (
            [
                'benchmarks/cases/' => 'the obsolete in-repository cases directory',
                'fibonacci/php/' => 'a per-language PHP directory',
                'fibonacci/javascript/' => 'a per-language JavaScript directory',
                'fibonacci/python/' => 'a per-language Python directory',
            ] as $stalePath => $description
        ) {
            if (str_contains($performance, $stalePath)) {
                $failures[] = "docs/performance-and-benchmarking.md: contains {$description} ({$stalePath})";
            }
        }
    }

    return $failures;
}
