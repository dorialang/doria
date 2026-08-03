<?php

declare(strict_types=1);

/** @return list<string> */
function check_benchmark_revision(string $root): array
{
    $pinPath = $root . '/benchmarks-revision.json';
    $raw = file_get_contents($pinPath);
    if ($raw === false) {
        return ['benchmarks-revision.json: unable to read benchmark revision pin'];
    }
    try {
        $pin = json_decode($raw, true, 512, JSON_THROW_ON_ERROR);
    } catch (JsonException $error) {
        return ['benchmarks-revision.json: invalid JSON: ' . $error->getMessage()];
    }
    if (!is_array($pin) || array_keys($pin) !== ['schemaVersion', 'repository', 'revision']) {
        return ['benchmarks-revision.json: expected only schemaVersion, repository, and revision'];
    }
    if (
        $pin['schemaVersion'] !== 1
        || $pin['repository'] !== 'dorialang/benchmarks'
        || !is_string($pin['revision'])
        || preg_match('/^[0-9a-f]{40}$/', $pin['revision']) !== 1
    ) {
        return ['benchmarks-revision.json: invalid schema, repository, or full revision'];
    }

    $sibling = dirname($root) . '/benchmarks';
    if (!is_dir($sibling . '/.git')) {
        return [];
    }
    $command = sprintf(
        'git -C %s rev-parse HEAD 2>&1',
        escapeshellarg($sibling)
    );
    exec($command, $output, $status);
    $actual = trim(implode("\n", $output));
    if ($status !== 0) {
        return ['sibling benchmarks repository: unable to read HEAD'];
    }
    if ($actual !== $pin['revision']) {
        return ["sibling benchmarks repository: expected {$pin['revision']}, found {$actual}"];
    }
    foreach (['manifest.json', 'report-schema.json', 'bench.php', 'tests/run.php'] as $required) {
        if (!is_file($sibling . '/' . $required)) {
            return ["sibling benchmarks repository: pinned checkout is missing {$required}"];
        }
    }
    return [];
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_benchmark_revision(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "benchmark revision check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    echo "benchmark revision check passed\n";
}
