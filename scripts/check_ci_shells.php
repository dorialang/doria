<?php

declare(strict_types=1);

/**
 * A multi-command step that can run on Windows names its shell.
 *
 * The default Windows shell is PowerShell, and GitHub fails such a step only
 * on the exit code of its last command. A failing `cargo test` on any earlier
 * line is reported as success. `shell: bash` stops at the first failure; a
 * single command needs neither.
 *
 * Usage: php scripts/check_ci_shells.php [--repo-root <path>]
 */

/**
 * @return list<string>
 */
function check_ci_shells(string $root): array
{
    $errors = [];
    $workflows = glob($root . '/.github/workflows/*.{yml,yaml}', GLOB_BRACE) ?: [];
    sort($workflows);
    foreach ($workflows as $workflow) {
        $relative = substr($workflow, strlen($root) + 1);
        $contents = (string) file_get_contents($workflow);
        if (preg_match('/^defaults:\s*\n\s+run:\s*\n\s+shell:\s*\S/m', $contents) === 1) {
            continue;
        }
        $lines = explode("\n", $contents);
        $mayRunOnWindows = false;
        foreach ($lines as $index => $line) {
            if (preg_match('/^  [A-Za-z0-9_-]+:\s*$/', $line) === 1) {
                $mayRunOnWindows = false;
            }
            if (preg_match('/^    runs-on:\s*(.+)$/', $line, $match) === 1) {
                $mayRunOnWindows = stripos($match[1], 'windows') !== false
                    || (str_contains($match[1], 'matrix.') && stripos($contents, 'windows') !== false);
            }
            if (!$mayRunOnWindows || preg_match('/^      - /', $line) !== 1) {
                continue;
            }
            $shell = false;
            $commands = 0;
            $name = trim($line, " -");
            $inBlock = false;
            for ($next = $index; $next < count($lines); $next++) {
                $step = $lines[$next];
                if ($next > $index && preg_match('/^ {0,6}\S/', $step) === 1) {
                    break;
                }
                if (preg_match('/^(?:      - |        )shell:\s*\S/', $step) === 1) {
                    $shell = true;
                }
                if (preg_match('/^(?:      - |        )run:\s*\|/', $step) === 1) {
                    $inBlock = true;
                    continue;
                }
                if ($inBlock && preg_match('/^          (?!#)\S/', $step) === 1) {
                    $commands++;
                }
            }
            if ($commands > 1 && !$shell) {
                $errors[] = sprintf(
                    '%s:%d: step `%s` runs %d commands on the default shell, so Windows reports only the last '
                    . 'exit code; add `shell: bash` to the step, or make it one command',
                    $relative,
                    $index + 1,
                    $name,
                    $commands,
                );
            }
        }
    }
    return $errors;
}

if (realpath($_SERVER['SCRIPT_FILENAME']) === __FILE__) {
    $root = dirname(__DIR__);
    if ($argc === 3 && $argv[1] === '--repo-root') {
        $root = rtrim($argv[2], '/');
    } elseif ($argc !== 1) {
        fwrite(STDERR, "Usage: php scripts/check_ci_shells.php [--repo-root <path>]\n");
        exit(2);
    }
    $errors = check_ci_shells($root);
    foreach ($errors as $error) {
        fwrite(STDERR, $error . "\n");
    }
    if ($errors !== []) {
        exit(1);
    }
    echo "CI shell check passed\n";
}
