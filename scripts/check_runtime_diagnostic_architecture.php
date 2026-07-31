<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];
$production = [
    'crates/doriac/src',
    'crates/doria-rt/src',
];
$forbidden = [
    'Panic: ' => 'legacy panic heading',
    'Stack Trace:' => 'legacy stack-trace heading',
    'dr_v1_panic' => 'incompatible V1 panic ABI',
    'panic_static' => 'message-based panic helper',
    'string_runtime_panic' => 'message-based string panic helper',
    'legacy_runtime_panic' => 'legacy interpreter panic adapter',
    'RuntimePanicReport' => 'parallel public panic model',
    'PanicDiagnostic' => 'parallel public panic model',
    'NativePanicReport' => 'parallel public panic model',
    'CheckedErrorReport' => 'parallel checked-error model',
    'RuntimeErrorEnvelope' => 'parallel runtime envelope',
];

foreach ($production as $directory) {
    $iterator = new RecursiveIteratorIterator(
        new RecursiveDirectoryIterator($root . '/' . $directory),
    );
    foreach ($iterator as $file) {
        if (!$file->isFile() || $file->getExtension() !== 'rs') {
            continue;
        }
        $contents = file_get_contents($file->getPathname());
        if ($contents === false) {
            continue;
        }
        $relative = ltrim(str_replace($root, '', $file->getPathname()), '/\\');
        foreach ($forbidden as $needle => $description) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$relative}: contains {$description} `{$needle}`";
            }
        }
    }
}

$diagnostics = file_get_contents($root . '/crates/doriac/src/diagnostics.rs') ?: '';
foreach ([
    'DiagnosticKind::RuntimePanic',
    'RuntimeOutcomeDetails',
    'AbortWithoutCleanup',
    'Call Path',
    'Where',
    '"Process Exited With Status {}"',
] as $needle) {
    if (!str_contains($diagnostics, $needle)) {
        $failures[] = "crates/doriac/src/diagnostics.rs: missing shared runtime-outcome token `{$needle}`";
    }
}

$runner = file_get_contents($root . '/crates/doriac/src/main.rs') ?: '';
foreach (['DORIA_RUNTIME_OUTCOME_V2', 'decode_runtime_outcome', 'Diagnostic::runtime_panic'] as $needle) {
    if (!str_contains($runner, $needle)) {
        $failures[] = "crates/doriac/src/main.rs: missing private structured transport token `{$needle}`";
    }
}
foreach (['Stack Trace:', 'Panic: ', 'strip_prefix("Panic', 'split("Stack Trace'] as $needle) {
    if (str_contains($runner, $needle)) {
        $failures[] = "crates/doriac/src/main.rs: runtime host appears to parse rendered prose `{$needle}`";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "runtime diagnostic architecture check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "runtime diagnostic architecture check passed\n");
