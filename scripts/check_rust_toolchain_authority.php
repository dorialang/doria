<?php

declare(strict_types=1);

/**
 * rust-toolchain.toml is the only authority for the Rust compiler.
 *
 * Every local vet and every CI job must compile with the same rustc. If a
 * workflow installs "stable" itself, CI silently moves to each new release
 * while local validation stays on the pinned one. A CI run can then fail, or
 * pass, for reasons the local vet never saw. That happened when CI took Rust
 * 1.99 and the local vet ran 1.98.1. This check therefore requires:
 *
 *  - rust-toolchain.toml names an exact release ("1.98.1", never "stable");
 *  - every workflow job that runs cargo installs that file's toolchain with
 *    `rustup toolchain install` and no toolchain argument;
 *  - no workflow installs, selects or overrides another stable or versioned
 *    toolchain. Nightly is the one exception, and only for commands that name
 *    it explicitly (`cargo +nightly ...`).
 *
 * Usage: php scripts/check_rust_toolchain_authority.php [--repo-root <path>]
 */

const RUST_TOOLCHAIN_FIX = 'run: rustup toolchain install --no-self-update';

/**
 * @return list<string>
 */
function check_rust_toolchain_authority(string $root): array
{
    $errors = [];
    if (is_file($root . '/rust-toolchain')) {
        $errors[] = 'rust-toolchain: remove this legacy file; rust-toolchain.toml is the only toolchain authority';
    }
    $channel = pinned_rust_channel($root, $errors);
    $workflows = glob($root . '/.github/workflows/*.{yml,yaml}', GLOB_BRACE) ?: [];
    sort($workflows);
    foreach ($workflows as $workflow) {
        $relative = substr($workflow, strlen($root) + 1);
        foreach (workflow_jobs((string) file_get_contents($workflow)) as $job => $lines) {
            check_workflow_job($relative, $job, $lines, $errors);
        }
    }
    if ($errors === [] && $channel === null) {
        $errors[] = 'rust-toolchain.toml: no channel found';
    }
    return $errors;
}

/**
 * @param list<string> $errors
 */
function pinned_rust_channel(string $root, array &$errors): ?string
{
    $path = $root . '/rust-toolchain.toml';
    $text = @file_get_contents($path);
    if ($text === false) {
        $errors[] = 'rust-toolchain.toml: missing; add `[toolchain]` with `channel = "<the rustc version the local vet uses>"`';
        return null;
    }
    if (preg_match('/^\s*\[toolchain\]\s*$(.*?)(?=^\s*\[|\z)/ms', $text, $section) !== 1) {
        $errors[] = 'rust-toolchain.toml: missing the [toolchain] table';
        return null;
    }
    if (preg_match('/^\s*channel\s*=\s*"([^"]*)"\s*$/m', $section[1], $match) !== 1) {
        $errors[] = 'rust-toolchain.toml: [toolchain] has no `channel = "..."` line';
        return null;
    }
    $channel = $match[1];
    if (preg_match('/^\d+\.\d+\.\d+$/', $channel) !== 1) {
        $errors[] = sprintf(
            'rust-toolchain.toml: channel "%s" floats; set it to an exact release such as "1.98.1" (the version `rustc --version` reports for the local vet)',
            $channel,
        );
        return null;
    }
    return $channel;
}

/**
 * Splits a workflow into its jobs by indentation, without a YAML parser.
 *
 * @return array<string, array<int, string>> job id => line number => line
 */
function workflow_jobs(string $text): array
{
    $jobs = [];
    $current = null;
    $inJobs = false;
    foreach (preg_split('/\R/', $text) ?: [] as $index => $line) {
        if (preg_match('/^jobs:\s*$/', $line) === 1) {
            $inJobs = true;
            continue;
        }
        if ($inJobs && preg_match('/^\S/', $line) === 1) {
            $inJobs = false;
            $current = null;
        }
        if (!$inJobs) {
            continue;
        }
        if (preg_match('/^  ([A-Za-z0-9_-]+):\s*$/', $line, $match) === 1) {
            $current = $match[1];
            $jobs[$current] = [];
            continue;
        }
        if ($current !== null) {
            $jobs[$current][$index + 1] = $line;
        }
    }
    return $jobs;
}

/**
 * @param array<int, string> $lines
 * @param list<string> $errors
 */
function check_workflow_job(string $file, string $job, array $lines, array &$errors): void
{
    $installsPin = false;
    $firstCargo = null;
    foreach ($lines as $number => $line) {
        $code = preg_replace('/\s+#.*$/', '', $line) ?? $line;
        $where = "{$file}:{$number} (job {$job})";
        if (preg_match('/uses:\s*dtolnay\/rust-toolchain@(\S+)/', $code, $match) === 1 && $match[1] !== 'nightly') {
            $errors[] = "{$where}: installs Rust `{$match[1]}` instead of rust-toolchain.toml's release; replace the step with `" . RUST_TOOLCHAIN_FIX . '`';
        }
        if (preg_match('/^\s*toolchain:\s*(\S+)/', $code, $match) === 1 && trim($match[1], '\'"') !== 'nightly') {
            $errors[] = "{$where}: selects toolchain `{$match[1]}`; remove it and install rust-toolchain.toml's release with `" . RUST_TOOLCHAIN_FIX . '`';
        }
        if (preg_match('/\brustup\s+toolchain\s+install\b(.*)$/', $code, $match) === 1) {
            $names = rustup_install_toolchain_names(trim($match[1]));
            if ($names === []) {
                $installsPin = true;
            } elseif ($names !== ['nightly']) {
                $errors[] = "{$where}: installs `" . implode(' ', $names) . "`; drop the toolchain name so rustup installs rust-toolchain.toml's release (`" . RUST_TOOLCHAIN_FIX . '`)';
            }
        }
        if (preg_match('/\brustup\s+(default|update|override)\b/', $code, $match) === 1) {
            $errors[] = "{$where}: `rustup {$match[1]}` overrides or moves the pinned toolchain; remove it, rust-toolchain.toml already selects the release";
        }
        if (preg_match('/\bcargo\s+\+(\S+)/', $code, $match) === 1 && $match[1] !== 'nightly') {
            $errors[] = "{$where}: `cargo +{$match[1]}` bypasses rust-toolchain.toml; remove the `+{$match[1]}` override";
        }
        if ($firstCargo === null && preg_match('/(^|[\s;&|(`])cargo\s+(?!\+nightly\b)/', $code) === 1
            && preg_match('/^\s*(-\s*)?name:/', $code) !== 1) {
            $firstCargo = $number;
        }
    }
    if ($firstCargo !== null && !$installsPin) {
        $errors[] = "{$file}:{$firstCargo} (job {$job}): runs cargo without installing rust-toolchain.toml's release; add a step `" . RUST_TOOLCHAIN_FIX . '` before it';
    }
}

/**
 * The toolchain names given to `rustup toolchain install`, skipping options
 * and the values of options that take one.
 *
 * @return list<string>
 */
function rustup_install_toolchain_names(string $arguments): array
{
    $valued = ['--profile', '-c', '--component', '-t', '--target'];
    $names = [];
    $words = preg_split('/\s+/', $arguments) ?: [];
    for ($index = 0; $index < count($words); $index++) {
        $word = $words[$index];
        if ($word === '') {
            continue;
        }
        if ($word[0] === '-') {
            if (in_array($word, $valued, true)) {
                $index++;
            }
            continue;
        }
        $names[] = $word;
    }
    return $names;
}

if (PHP_SAPI === 'cli' && realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $root = dirname(__DIR__);
    for ($index = 1; $index < count($argv); $index++) {
        if ($argv[$index] === '--repo-root' && isset($argv[$index + 1])) {
            $root = rtrim($argv[++$index], '/');
            continue;
        }
        fwrite(STDERR, "Usage: php scripts/check_rust_toolchain_authority.php [--repo-root <path>]\n");
        exit(2);
    }
    $errors = check_rust_toolchain_authority($root);
    if ($errors !== []) {
        fwrite(STDERR, "Rust toolchain authority check failed:\n");
        foreach ($errors as $error) {
            fwrite(STDERR, "  - {$error}\n");
        }
        exit(1);
    }
    $toolchain = (string) file_get_contents($root . '/rust-toolchain.toml');
    preg_match('/^\s*channel\s*=\s*"([^"]*)"/m', $toolchain, $match);
    echo "Rust toolchain authority check passed: every workflow job uses rust-toolchain.toml ({$match[1]})\n";
}
