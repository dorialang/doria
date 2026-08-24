<?php

declare(strict_types=1);

/**
 * @return list<string>
 */
function check_inferred_main_effects(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required inferred-main authority is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing inferred-main contract `{$needle}`";
            }
        }
    };

    $decisionPath = 'docs/decisions/0119-checked-errors-error-values-throws-effects-propagation-and-runtime-outcomes.md';
    $specPath = 'SPEC.md';
    $readmePath = 'README.md';
    $planPath = 'docs/doria-end-to-end-plan.md';
    $pipelinePath = 'docs/notes/current-pipeline.md';
    $proposalPath = 'docs/notes/stage30-closure-authority-proposal.md';
    $compatibilityPath = 'examples/compile-only/main_explicit_throws_compatibility.doria';
    $semanticsPath = 'crates/doriac/src/semantics.rs';
    $hirPath = 'crates/doriac/src/hir.rs';
    $testsPath = 'crates/doriac/tests/checked_error_tests.rs';
    $cataloguePath = 'crates/doria-diagnostic-catalogue/src/lib.rs';
    $runtimePath = 'crates/doria-rt/src/checked_io.rs';

    $decision = $read($decisionPath);
    $spec = $read($specPath);
    $readme = $read($readmePath);
    $plan = $read($planPath);
    $pipeline = $read($pipelinePath);
    $proposal = $read($proposalPath);
    $compatibility = $read($compatibilityPath);
    $semantics = $read($semanticsPath);
    $hir = $read($hirPath);
    $tests = $read($testsPath);
    $catalogue = $read($cataloguePath);
    $runtime = $read($runtimePath);

    $require($decisionPath, $decision, [
        '**Entrypoint inference amendment accepted:** 2026-08-18',
        'Reusable callables declare checked effects; the selected program entrypoint',
        'Source syntax and effective effects are separate facts',
        'MIR and every backend select this ABI from the effective semantic effect set',
        'status 70',
        'status 101',
        'status-0',
    ]);
    $require($specPath, $spec, [
        'When the selected top-level',
        '`main` omits `throws`',
        'function main(): void',
        'Ordinary reusable functions, methods, constructors, and generic',
    ]);
    $require($readmePath, $readme, [
        "function main(): void\n{",
        'the selected program entrypoint infers what escapes it',
    ]);
    foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
        $require($path, $contents, [
            'Corrective Beat: Inferred Main Checked Effects — Complete',
            'Stage 30a Callable Grammar Completion — Complete',
            'Stage 30b Semantic Function Types And Captures — Complete',
            'Stage 30c Ownership, Lifetime, And Escape — Complete',
            'Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete',
            'Stage 30e Native Execution — Complete',
            'Stage 30f PHP Compatibility — Complete',
            'Stage 30g List Algorithms — Next',
            'Stage 30 — In Progress, Not Complete',
        ]);
    }
    $require($proposalPath, $proposal, [
        '**Superseded By Accepted Decision 0121.**',
        'Stages 30a through 30f are complete',
        'Stage 30g List Algorithms is next',
    ]);
    $require($compatibilityPath, $compatibility, [
        'function main(): void throws Doria\Std\Io\IoError',
    ]);
    $require($semanticsPath, $semantics, [
        'callable_effective_checked_effects',
        'is_accepted_program_entrypoint',
        'set_inferred_entrypoint_effects',
        '"E0631"',
    ]);
    $require($hirPath, $hir, [
        'Source-preserving `throws` syntax',
        'Effective semantic checked effects',
    ]);
    $require($testsPath, $tests, [
        'selected_main_infers_exact_uncovered_effects_without_changing_source_syntax',
        'selected_main_inference_covers_entry_shapes_and_nested_catch_subtraction',
        'inferred_main_contract_is_available_to_recursive_and_source_calls',
        'explicit_main_contract_remains_checked_and_ordinary_callables_remain_explicit',
        'inferred_main_diagnostics_are_not_duplicated_by_contract_discovery',
    ]);
    $require($cataloguePath, $catalogue, [
        'code: "R1000"',
        'process_status: 70',
        'process_status: 101',
    ]);
    $require($runtimePath, $runtime, ['WriteOutcome::BrokenPipe => exit_process(0)']);

    $explicitCompatibility = realpath($root . '/' . $compatibilityPath);
    $iterator = new RecursiveIteratorIterator(
        new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS),
    );
    foreach ($iterator as $file) {
        if (!$file->isFile() || strtolower($file->getExtension()) !== 'doria') {
            continue;
        }
        $path = str_replace('\\', '/', $file->getPathname());
        if (str_contains($path, '/target/') || str_contains($path, '/.git/')) {
            continue;
        }
        $contents = file_get_contents($file->getPathname());
        if (!is_string($contents) || preg_match('/^function main\([^\r\n]*\)\s*:\s*\S+\s+throws\s+/m', $contents) !== 1) {
            continue;
        }
        if (realpath($file->getPathname()) !== $explicitCompatibility) {
            $relative = ltrim(substr($path, strlen(str_replace('\\', '/', $root))), '/');
            $failures[] = "{$relative}: canonical main must omit its routine throws clause";
        }
    }

    if (is_dir($root . '/doria-website')) {
        $failures[] = 'doria-website/: website repository content must not be copied into the compiler tree';
    }

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_inferred_main_effects(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Inferred main effects check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Inferred main effects check passed\n");
}
