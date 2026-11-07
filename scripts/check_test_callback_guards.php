<?php

declare(strict_types=1);

/**
 * Every test-authored `extern "C"` or `extern "system"` function in doria-rt
 * runs its body under `crate::test_callback::guard`.
 *
 * Tests hand these functions to the runtime as adapter, stream, drop, entry
 * and thread callbacks. A panic cannot leave a non-unwinding ABI: Rust aborts
 * the process. Under libtest that abort also discards the test's captured
 * output, so CI shows only SIGABRT, or 0xc0000409 on Windows, with no test
 * name, location or message. The guard reports the panic past libtest's
 * capture before aborting.
 *
 * Test code is every file whose path names a test (`tests.rs`, `*_tests.rs`,
 * `tests/`, `pending_tests/`), plus `#[cfg(test)] mod ... { }` blocks
 * elsewhere. Function-pointer types and extern declarations have no body and
 * are not callbacks.
 *
 * Usage: php scripts/check_test_callback_guards.php [--repo-root <path>]
 */

const TEST_CALLBACK_GUARD = 'crate::test_callback::guard(';

/**
 * @return list<string>
 */
function check_test_callback_guards(string $root): array
{
    $source = $root . '/crates/doria-rt/src';
    $errors = [];
    $files = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($source, FilesystemIterator::SKIP_DOTS));
    $paths = [];
    foreach ($files as $file) {
        if ($file->isFile() && str_ends_with($file->getFilename(), '.rs')) {
            $paths[] = $file->getPathname();
        }
    }
    sort($paths);
    foreach ($paths as $path) {
        $relative = substr($path, strlen($root) + 1);
        $text = (string) file_get_contents($path);
        $regions = test_regions($relative, $text);
        if ($regions === []) {
            continue;
        }
        preg_match_all(
            '/\b(?:unsafe\s+)?extern\s+"(?:C|system)"\s+fn\s+(\$?[A-Za-z_][A-Za-z0-9_]*)/',
            $text,
            $matches,
            PREG_OFFSET_CAPTURE
        );
        foreach ($matches[0] as $index => [$signature, $offset]) {
            if (!in_regions($offset, $regions)) {
                continue;
            }
            $body = body_start($text, $offset + strlen($signature));
            if ($body === null) {
                continue;
            }
            if (!str_starts_with(ltrim(substr($text, $body + 1, 200)), TEST_CALLBACK_GUARD)) {
                $line = substr_count($text, "\n", 0, $offset) + 1;
                $name = $matches[1][$index][0];
                $errors[] = "{$relative}:{$line}: test callback `{$name}` must wrap its body in "
                    . "`crate::test_callback::guard(|| { ... })`, so a panic names its test instead of aborting silently";
            }
        }
    }
    return $errors;
}

/**
 * @return list<array{int, int}>
 */
function test_regions(string $relative, string $text): array
{
    if (preg_match('#(^|/)tests?(/|\.rs$)|_tests(/|\.rs$)|(^|/)tests/#', $relative) === 1) {
        return [[0, strlen($text)]];
    }
    $regions = [];
    if (preg_match_all('/#\[cfg\(test\)\]\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+\w+\s*\{/', $text, $matches, PREG_OFFSET_CAPTURE)) {
        foreach ($matches[0] as [$header, $offset]) {
            $open = $offset + strlen($header) - 1;
            $regions[] = [$open, matching_brace($text, $open)];
        }
    }
    return $regions;
}

/**
 * @param list<array{int, int}> $regions
 */
function in_regions(int $offset, array $regions): bool
{
    foreach ($regions as [$start, $end]) {
        if ($offset >= $start && $offset < $end) {
            return true;
        }
    }
    return false;
}

/**
 * The offset of the body's `{` after a function name, or null for a body-less
 * declaration. Parameter lists contain no braces or semicolons.
 */
function body_start(string $text, int $offset): ?int
{
    $open = strpos($text, '(', $offset);
    if ($open === false) {
        return null;
    }
    $depth = 0;
    $length = strlen($text);
    for ($index = $open; $index < $length; $index++) {
        $character = $text[$index];
        if ($character === '(') {
            $depth++;
        } elseif ($character === ')' && --$depth === 0) {
            break;
        }
    }
    for ($index++; $index < $length; $index++) {
        if ($text[$index] === '{') {
            return $index;
        }
        if ($text[$index] === ';') {
            return null;
        }
    }
    return null;
}

/**
 * The offset of the brace closing the one at `$open`. Skips comments and
 * string and character literals.
 */
function matching_brace(string $text, int $open): int
{
    $depth = 0;
    $length = strlen($text);
    for ($index = $open; $index < $length; $index++) {
        $character = $text[$index];
        if ($character === '/' && ($text[$index + 1] ?? '') === '/') {
            $index = strpos($text, "\n", $index) ?: $length;
            continue;
        }
        if ($character === '/' && ($text[$index + 1] ?? '') === '*') {
            $index = (strpos($text, '*/', $index + 2) ?: $length) + 1;
            continue;
        }
        if ($character === '"') {
            for ($index++; $index < $length && $text[$index] !== '"'; $index++) {
                if ($text[$index] === '\\') {
                    $index++;
                }
            }
            continue;
        }
        if ($character === "'" && preg_match("/\\G'(?:\\\\(?:x[0-9a-fA-F]{2}|u\\{[0-9a-fA-F]+\\}|.)|[^\\\\'\\n])'/", $text, $match, 0, $index) === 1) {
            $index += strlen($match[0]) - 1;
            continue;
        }
        if ($character === '{') {
            $depth++;
        } elseif ($character === '}' && --$depth === 0) {
            return $index;
        }
    }
    return $length;
}

if (PHP_SAPI === 'cli' && realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $root = dirname(__DIR__);
    for ($index = 1; $index < count($argv); $index++) {
        if ($argv[$index] === '--repo-root' && isset($argv[$index + 1])) {
            $root = rtrim($argv[++$index], '/');
            continue;
        }
        fwrite(STDERR, "Usage: php scripts/check_test_callback_guards.php [--repo-root <path>]\n");
        exit(2);
    }
    $errors = check_test_callback_guards($root);
    if ($errors !== []) {
        fwrite(STDERR, "Test callback guard check failed:\n");
        foreach ($errors as $error) {
            fwrite(STDERR, "  - {$error}\n");
        }
        exit(1);
    }
    echo "Test callback guard check passed: every doria-rt test callback runs under crate::test_callback::guard\n";
}
