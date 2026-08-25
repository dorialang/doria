<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage30h_closure_completion(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 30h file is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 30h contract `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md',
        'spec' => 'SPEC.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'audit' => 'docs/notes/temporary-language-restrictions-audit.md',
        'catalogue' => 'crates/doria-diagnostic-catalogue/src/lib.rs',
        'abi' => 'crates/doriac/src/native_abi.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'lowering' => 'crates/doriac/src/mir_lowering.rs',
        'runtime' => 'crates/doria-rt/src/mixed.rs',
        'runtimeExports' => 'crates/doria-rt/src/lib.rs',
        'nativeManifest' => 'crates/doriac/tests/fixtures/native_closures/manifest.txt',
        'phpManifest' => 'crates/doriac/tests/fixtures/php_closures/manifest.txt',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    foreach (['decision', 'plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 30h',
            'Complete',
            'Stage 30',
            'E0641',
            'Historical',
            'Stage 31',
            'Next',
        ]);
    }
    $require($paths['spec'], $files['spec'], [
        'Stage 30h Cross-Repository Closure - Complete',
        'Stage 30 - Complete',
        'E0641 - Historical And Reserved',
    ]);
    $require($paths['audit'], $files['audit'], [
        'Historical and reserved after Stage 30h',
        'no active emitter or generic fallback remains',
    ]);
    $require($paths['catalogue'], $files['catalogue'], ['"E0641"']);
    $require($paths['abi'], $files['abi'], [
        'MIXED_NEW_AGGREGATE_BORROWED',
        'MIXED_TAG_FUNCTION',
    ]);
    $require($paths['mir'], $files['mir'], [
        'Function(FunctionTypeId)',
        'BoxFunction {',
    ]);
    $require($paths['lowering'], $files['lowering'], ['MixedExpression::BoxFunction']);
    $require($paths['runtime'], $files['runtime'], ['borrowed_aggregate_shells_never_claim_the_copied_payload']);
    $require($paths['runtimeExports'], $files['runtimeExports'], ['dr_v3_mixed_new_aggregate_borrowed']);
    $require($paths['nativeManifest'], $files['nativeManifest'], ['stage30h_completion']);
    $require($paths['phpManifest'], $files['phpManifest'], ['stage30h_completion']);

    $sourceRoot = $root . '/crates/doriac/src';
    $iterator = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($sourceRoot));
    foreach ($iterator as $file) {
        if (!$file->isFile() || $file->getExtension() !== 'rs') {
            continue;
        }
        $contents = file_get_contents($file->getPathname());
        if (is_string($contents) && str_contains($contents, 'E0641')) {
            $relative = ltrim(substr($file->getPathname(), strlen($root)), '/');
            $failures[] = "{$relative}: E0641 must not have an active compiler emitter or fallback";
        }
    }

    $testsRoot = $root . '/crates/doriac/tests';
    $iterator = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($testsRoot));
    foreach ($iterator as $file) {
        if (!$file->isFile() || $file->getExtension() !== 'rs') {
            continue;
        }
        $contents = file_get_contents($file->getPathname());
        if (!is_string($contents)) {
            continue;
        }
        if (preg_match('/\.filter\s*\([^\n]*E0641|retain\s*\([^\n]*E0641/s', $contents) === 1) {
            $relative = ltrim(substr($file->getPathname(), strlen($root)), '/');
            $failures[] = "{$relative}: tests must not filter or suppress E0641 before asserting success";
        }
    }

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage30h_closure_completion(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 30h closure completion check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 30h closure completion check passed\n");
}
