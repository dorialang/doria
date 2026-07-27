<?php

declare(strict_types=1);

const TARGET_WARNING_BYTES = 15 * 1024 * 1024 * 1024;

$target = dirname(__DIR__) . DIRECTORY_SEPARATOR . 'target';

if (!is_dir($target)) {
    fwrite(STDOUT, "Cargo target size: 0 B (target/ is absent; warning threshold: 15 GiB)\n");
    exit(0);
}

$bytes = 0;
$seenFiles = [];
$entries = new RecursiveIteratorIterator(
    new RecursiveDirectoryIterator($target, FilesystemIterator::SKIP_DOTS),
    RecursiveIteratorIterator::LEAVES_ONLY,
);

foreach ($entries as $entry) {
    if ($entry->isFile() && !$entry->isLink()) {
        $metadata = stat($entry->getPathname());
        if ($metadata === false) {
            fwrite(STDERR, "ERROR: could not measure {$entry->getPathname()}.\n");
            exit(1);
        }

        $identity = "{$metadata['dev']}:{$metadata['ino']}";
        if ($metadata['ino'] !== 0 && isset($seenFiles[$identity])) {
            continue;
        }

        if ($metadata['ino'] !== 0) {
            $seenFiles[$identity] = true;
        }

        $bytes += array_key_exists('blocks', $metadata)
            ? $metadata['blocks'] * 512
            : $metadata['size'];
    }
}

$size = format_bytes($bytes);
fwrite(STDOUT, "Cargo target allocated size: {$size} (warning threshold: 15 GiB)\n");

if ($bytes > TARGET_WARNING_BYTES) {
    fwrite(
        STDERR,
        "WARNING: target/ exceeds 15 GiB. Cargo does not garbage-collect obsolete project artifacts.\n"
        . "Inspect the directory and use `cargo clean --dry-run` to preview cleanup. "
        . "Do not clean it without Andrew's approval.\n",
    );
    exit(2);
}

function format_bytes(int $bytes): string
{
    $units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
    $value = (float) $bytes;
    $unit = 0;

    while ($value >= 1024 && $unit < count($units) - 1) {
        $value /= 1024;
        ++$unit;
    }

    return $unit === 0
        ? sprintf('%d %s', $bytes, $units[$unit])
        : sprintf('%.2f %s', $value, $units[$unit]);
}
