#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/cargo_artifacts.php';

$root = dirname(__DIR__);
$target = getenv('DORIA_VALIDATION_TARGET_DIR') ?: getenv('CARGO_TARGET_DIR') ?: $root . '/target';

for ($index = 1; $index < count($argv); $index++) {
    if ($argv[$index] !== '--target-dir' || !isset($argv[$index + 1])) {
        fwrite(STDERR, "Usage: php scripts/check_cargo_target_size.php [--target-dir <path>]\n");
        exit(1);
    }
    $target = $argv[++$index];
}

try {
    $target = doria_canonical_path($target, $root);
    $bytes = doria_allocated_directory_bytes($target);
} catch (RuntimeException $error) {
    fwrite(STDERR, "ERROR: {$error->getMessage()}.\n");
    exit(1);
}

fwrite(
    STDOUT,
    "Cargo target allocated size: " . doria_format_bytes($bytes)
        . " ({$target}; warning threshold: 15 GiB)\n",
);

if ($bytes > DORIA_TARGET_WARNING_BYTES) {
    fwrite(
        STDERR,
        "WARNING: the Cargo target exceeds 15 GiB. Reclaim the managed cache with "
            . "`php scripts/validate_work_unit.php --target-dir "
            . escapeshellarg($target)
            . " --reclaim`.\n",
    );
    exit(2);
}
