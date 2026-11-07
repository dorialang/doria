<?php

declare(strict_types=1);

/**
 * Every GitHub Actions job declares `timeout-minutes`.
 *
 * Without one, GitHub lets a job run for six hours. A hung test then holds a
 * runner that long and publishes no log until someone cancels it. A job-level
 * bound turns a hang into a failed job with its log.
 *
 * Usage: php scripts/check_ci_timeouts.php [--repo-root <path>]
 */

/**
 * @return list<string>
 */
function check_ci_timeouts(string $root): array
{
    $errors = [];
    $workflows = glob($root . '/.github/workflows/*.{yml,yaml}', GLOB_BRACE) ?: [];
    sort($workflows);
    foreach ($workflows as $workflow) {
        $relative = substr($workflow, strlen($root) + 1);
        $job = null;
        $jobLine = 0;
        $bounded = false;
        $inJobs = false;
        $lines = explode("\n", (string) file_get_contents($workflow));
        $finish = static function () use (&$job, &$jobLine, &$bounded, &$errors, $relative): void {
            if ($job !== null && !$bounded) {
                $errors[] = "{$relative}:{$jobLine}: job `{$job}` has no timeout; add `timeout-minutes: <minutes>` "
                    . 'at the job level, above its steps';
            }
            $job = null;
            $bounded = false;
        };
        foreach ($lines as $index => $line) {
            if (preg_match('/^jobs:\s*$/', $line) === 1) {
                $inJobs = true;
                continue;
            }
            if (!$inJobs) {
                continue;
            }
            if (preg_match('/^\S/', $line) === 1) {
                $finish();
                $inJobs = false;
                continue;
            }
            if (preg_match('/^  ([A-Za-z0-9_-]+):\s*$/', $line, $match) === 1) {
                $finish();
                $job = $match[1];
                $jobLine = $index + 1;
                continue;
            }
            if ($job !== null && preg_match('/^    timeout-minutes:\s*\S/', $line) === 1) {
                $bounded = true;
            }
        }
        $finish();
    }
    return $errors;
}

if (PHP_SAPI === 'cli' && realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $root = dirname(__DIR__);
    for ($index = 1; $index < count($argv); $index++) {
        if ($argv[$index] === '--repo-root' && isset($argv[$index + 1])) {
            $root = rtrim($argv[++$index], '/');
            continue;
        }
        fwrite(STDERR, "Usage: php scripts/check_ci_timeouts.php [--repo-root <path>]\n");
        exit(2);
    }
    $errors = check_ci_timeouts($root);
    if ($errors !== []) {
        fwrite(STDERR, "CI timeout check failed:\n");
        foreach ($errors as $error) {
            fwrite(STDERR, "  - {$error}\n");
        }
        exit(1);
    }
    echo "CI timeout check passed: every workflow job declares timeout-minutes\n";
}
