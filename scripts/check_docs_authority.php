<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

function normalize_path(string $path): string
{
    return str_replace('\\', '/', $path);
}

function relative_path(string $root, string $path): string
{
    return ltrim(substr(normalize_path($path), strlen(normalize_path($root))), '/');
}

function is_skipped_path(string $path): bool
{
    foreach (['.git/', 'target/', 'node_modules/'] as $skip) {
        if (str_contains($path, $skip)) {
            return true;
        }
    }

    return false;
}

function is_historical_path(string $path): bool
{
    return str_starts_with($path, 'docs/notes/');
}

function is_decision_path(string $path): bool
{
    return str_starts_with($path, 'docs/decisions/');
}

function is_redirect_path(string $path): bool
{
    return $path === 'docs/doria-development-plan.md';
}

function is_end_to_end_plan(string $path): bool
{
    return $path === 'docs/doria-end-to-end-plan.md';
}

function is_active_scanned_path(string $path): bool
{
    if (is_historical_path($path) || is_decision_path($path) || is_redirect_path($path)) {
        return false;
    }

    return str_ends_with($path, '.md');
}

function line_is_negating_or_contextual(string $line): bool
{
    return preg_match('/\b(not|never|no|without|reject|rejected|invalid|reserved|literal|planned|future|PHP|interop|migration|historical|not Doria)\b/i', $line) === 1;
}

function add_failure(array &$failures, string $path, int $lineNumber, string $message, string $line): void
{
    $failures[] = "{$path}:{$lineNumber}: {$message}\n    {$line}";
}

$iterator = new RecursiveIteratorIterator(
    new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS)
);

$markdownFiles = [];
foreach ($iterator as $file) {
    if (!$file->isFile()) {
        continue;
    }

    $path = relative_path($root, $file->getPathname());
    if (is_skipped_path($path) || !str_ends_with(strtolower($path), '.md')) {
        continue;
    }

    $markdownFiles[] = $path;
}

sort($markdownFiles);

foreach ($markdownFiles as $path) {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: unable to read file";
        continue;
    }

    $lines = preg_split('/\R/', $contents) ?: [];
    $active = is_active_scanned_path($path);
    $inPhpFence = false;

    foreach ($lines as $index => $line) {
        $lineNumber = $index + 1;
        $trimmedLine = trim($line);

        if (str_starts_with($trimmedLine, '```')) {
            if ($inPhpFence) {
                $inPhpFence = false;
                continue;
            }

            $inPhpFence = preg_match('/^```php\b/i', $trimmedLine) === 1;
            continue;
        }

        if ($active && str_contains($line, 'ROADMAP.md')) {
            add_failure($failures, $path, $lineNumber, 'active docs must not instruct contributors to use ROADMAP.md', $line);
        }

        if ($active && str_contains($line, 'docs/doria-development-plan.md')) {
            add_failure($failures, $path, $lineNumber, 'active docs must not list the superseded development plan as an authority', $line);
        }

        if ($active && preg_match('/^#{1,3}\s*(Next Compiler Work|Future implementation order|Near-term roadmap)\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not contain duplicate roadmap headings', $line);
        }

        if ($active && preg_match('/^#{1,3}\s*Roadmap\b/i', $line) === 1 && !is_end_to_end_plan($path)) {
            add_failure($failures, $path, $lineNumber, 'only the end-to-end plan may own roadmap headings', $line);
        }

        if ($active && preg_match('/\bdefault-public\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not use old default-public wording', $line);
        }

        if ($active && preg_match('/\bvisibility modifiers\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not teach a stale visibility-modifier model', $line);
        }

        if ($active && !$inPhpFence && preg_match('/\b(public|private|protected)\s+(string|int|float|bool|mixed|function)\b/', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not show stale public/private/protected Doria member syntax', $line);
        }

        if ($active && preg_match('/\bobject\s+(as\s+a\s+)?(core\s+)?type\b|\bcore\s+object\s+type\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not present object as a Doria core type', $line);
        }

        if ($active && preg_match('/\bresource\s+(as\s+a\s+)?(core\s+)?type\b|\bcore\s+resource\s+type\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not present resource as a Doria core type', $line);
        }

        if ($active && preg_match('/\bnull\s+type\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not present null as a Doria source type', $line);
        }

        if ($active && preg_match('/\bMIR later\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not say MIR is merely later now that Stage 11 MIR is seeded', $line);
        }

        if ($active && preg_match('/\bdebug backend planned\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not say the debug backend is only planned', $line);
        }

        if ($active && preg_match('/debug.*wasm.*recognized planned targets/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must distinguish current debug support from planned wasm support', $line);
        }
    }
}

if ($failures !== []) {
    fwrite(STDERR, "docs authority check failed:\n");
    foreach ($failures as $failure) {
        fwrite(STDERR, "- {$failure}\n");
    }
    exit(1);
}

echo "docs authority check passed\n";
