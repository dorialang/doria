<?php

declare(strict_types=1);

const DORIA_TARGET_WARNING_BYTES = 15 * 1024 * 1024 * 1024;

function doria_allocated_directory_bytes(string $directory): int
{
    if (!is_dir($directory)) {
        return 0;
    }

    $bytes = 0;
    $seenFiles = [];
    $entries = new RecursiveIteratorIterator(
        new RecursiveDirectoryIterator($directory, FilesystemIterator::SKIP_DOTS),
        RecursiveIteratorIterator::LEAVES_ONLY,
    );

    foreach ($entries as $entry) {
        if (!$entry->isFile() || $entry->isLink()) {
            continue;
        }
        $metadata = stat($entry->getPathname());
        if ($metadata === false) {
            throw new RuntimeException("could not measure {$entry->getPathname()}");
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

    return $bytes;
}

function doria_format_bytes(int|float $bytes): string
{
    $units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
    $value = (float) $bytes;
    $unit = 0;

    while ($value >= 1024 && $unit < count($units) - 1) {
        $value /= 1024;
        ++$unit;
    }

    return $unit === 0
        ? sprintf('%d %s', (int) $bytes, $units[$unit])
        : sprintf('%.2f %s', $value, $units[$unit]);
}
