<?php

declare(strict_types=1);

require_once __DIR__ . '/vet_commands.php';

/**
 * Everything CI's release jobs run, the local vet also runs.
 *
 * A check that exists only in CI is found after a push, on a two-hour round
 * trip. The local vet may run more than CI; it may not run less.
 *
 * Usage: php scripts/check_vet_covers_ci.php [--repo-root <path>]
 */

/**
 * @return list<string>
 */
function check_vet_covers_ci(string $root): array
{
    $workflow = (string) file_get_contents($root . '/.github/workflows/ci.yml');
    $vet = (string) file_get_contents($root . '/scripts/validate_work_unit.php');
    $errors = [];

    $step = 'Run shared validator and LLVM-focused tests';
    if (preg_match('/- name: ' . $step . '\n.*?(?=\n\s*- name:)/s', $workflow, $match) !== 1) {
        return ["ci.yml has no step named `{$step}`; update `check_vet_covers_ci.php` to the step's new name"];
    }
    preg_match_all('/--test (\S+)/', $match[0], $suites);
    foreach (array_diff($suites[1], llvm_test_suites()) as $suite) {
        $errors[] = "CI runs LLVM suite `{$suite}` but the local vet does not: add it to llvm_test_suites() in scripts/vet_commands.php";
    }

    $runtimeTests = array_map(static fn (array $command): string => implode(' ', $command), release_runtime_tests());
    preg_match_all('/cargo test -p doria-rt --release[^\n]*/', $workflow, $releases);
    foreach ($releases[0] as $line) {
        $command = trim(str_replace(' --verbose', '', $line));
        if (!in_array($command, $runtimeTests, true)) {
            $errors[] = "CI runs `{$command}` but the local vet does not: add it to release_runtime_tests() in scripts/vet_commands.php";
        }
    }

    preg_match_all('#php (scripts/check_[a-z0-9_]+\.php)#', $workflow, $checks);
    foreach (array_unique($checks[1]) as $check) {
        if (!str_contains($vet, "'{$check}'")) {
            $errors[] = "CI runs `{$check}` but the local vet does not: add it to the checks in scripts/validate_work_unit.php";
        }
    }

    return $errors;
}

if (realpath($_SERVER['SCRIPT_FILENAME']) === __FILE__) {
    $root = dirname(__DIR__);
    if ($argc === 3 && $argv[1] === '--repo-root') {
        $root = rtrim($argv[2], '/');
    } elseif ($argc !== 1) {
        fwrite(STDERR, "Usage: php scripts/check_vet_covers_ci.php [--repo-root <path>]\n");
        exit(2);
    }
    $errors = check_vet_covers_ci($root);
    foreach ($errors as $error) {
        fwrite(STDERR, $error . "\n");
    }
    if ($errors !== []) {
        exit(1);
    }
    echo "Vet covers CI check passed\n";
}
