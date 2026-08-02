<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

$required = [
    'docs/diagnostic-style.md',
    'docs/decisions/0108-human-first-diagnostic-model-and-presentation.md',
    'docs/decisions/0109-unified-diagnostic-presentation-and-runtime-outcomes.md',
    'docs/notes/diagnostic-catalogue-audit.md',
];

foreach ($required as $path) {
    if (!is_file($root . '/' . $path)) {
        $failures[] = "{$path}: required diagnostic authority is missing";
    }
}

$renderer = file_get_contents($root . '/crates/doriac/src/diagnostics.rs');
if ($renderer === false) {
    $failures[] = 'crates/doriac/src/diagnostics.rs: could not read renderer';
} else {
    foreach ([
        '"Error"',
        '"Warning"',
        '"Note"',
        '"Help"',
        '"Why"',
        'Where\\n',
        'Related',
        'Call Path',
        'Suggested Fix',
        'Process Exited With Status',
        '"Internal Compiler Error"',
        '"Compilation Failed"',
        'DIAGNOSTIC_SCHEMA_VERSION: u32 = 1',
    ] as $needle) {
        if (!str_contains($renderer, $needle)) {
            $failures[] = "crates/doriac/src/diagnostics.rs: missing canonical token {$needle}";
        }
    }

}

$cataloguedCodes = [];
$catalogue = file_get_contents($root . '/crates/doria-diagnostic-catalogue/src/lib.rs');
if (is_string($catalogue)
    && preg_match('/DIAGNOSTIC_CODES[^=]*=\s*&\[(.*?)\];/s', $catalogue, $catalogueMatch) === 1
    && preg_match_all('/"([A-Z][0-9]{4})"/', $catalogueMatch[1], $codeMatches) !== false
) {
    $cataloguedCodes = array_fill_keys($codeMatches[1], true);
}

foreach (glob($root . '/crates/doriac/src/*.rs') ?: [] as $path) {
    $contents = file_get_contents($path);
    $relative = str_replace($root . '/', '', $path);
    if ($contents === false) {
        $failures[] = "{$relative}: could not audit diagnostic constructions";
        continue;
    }

    if (preg_match_all('/\\.with_title\\("([^"]+)"\\)/', $contents, $matches) !== false) {
        foreach ($matches[1] as $title) {
            if (preg_match('/^[^A-Za-z]*[a-z]/', $title) === 1
                || str_ends_with($title, '.')
                || str_contains($title, "\n")
            ) {
                $failures[] = "{$relative}: non-Title-Case title `{$title}`";
            }
        }
    }

    if (preg_match_all(
        '/Diagnostic::(?:new|unsupported_stage)\\(\\s*"([A-Z][0-9]{4})"/',
        $contents,
        $uses,
    ) !== false) {
        foreach ($uses[1] as $code) {
            if (!isset($cataloguedCodes[$code])) {
                $failures[] = "{$relative}: diagnostic code {$code} is missing from CATALOGUED_CODES";
            }
        }
    }
}

foreach ([
    'crates/doria-rt/src/lib.rs',
    'crates/doriac/src/mir_interpreter.rs',
    'crates/doriac/src/codegen_php.rs',
] as $path) {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: could not audit runtime headings";
        continue;
    }
    foreach (['"panic: ', '"stack trace: ', 'b"panic: ', 'b"stack trace: '] as $forbidden) {
        if (str_contains($contents, $forbidden)) {
            $failures[] = "{$path}: lowercase runtime heading literal `{$forbidden}`";
        }
    }
}

if ($failures !== []) {
    fwrite(STDERR, "diagnostic style check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "diagnostic style check passed\n");
