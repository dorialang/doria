<?php

declare(strict_types=1);

const DORIA_TARGET_WARNING_BYTES = 15 * 1024 * 1024 * 1024;
const DORIA_ARTIFACT_OWNER_FILE = '.doria-artifact-owner.json';
const DORIA_CARGO_CACHE_TAG = "Signature: 8a477f597d28d172789f06886806bc55\n"
    . "# This file is a cache directory tag created by cargo.\n"
    . "# For information about cache directory tags see https://bford.info/cachedir/\n";

function doria_canonical_path(string $path, string $base): string
{
    if ($path === '') {
        throw new RuntimeException('the Cargo target path must not be empty');
    }

    if (preg_match('/^~[\\\\\/]/', $path)) {
        $home = getenv(PHP_OS_FAMILY === 'Windows' ? 'USERPROFILE' : 'HOME');
        if (!is_string($home) || $home === '') {
            throw new RuntimeException("could not expand home-relative path {$path}");
        }
        $path = $home . substr($path, 1);
    }

    $resolvedBase = realpath($base);
    if ($resolvedBase === false) {
        throw new RuntimeException('the repository root could not be resolved');
    }
    if (preg_match('/^[A-Za-z]:[^\\\\\/]/', $path)) {
        throw new RuntimeException("drive-relative paths are not supported: {$path}");
    }
    $absolute = str_starts_with($path, '/')
        || preg_match('/^[A-Za-z]:[\\\\\/]/', $path) === 1
        || preg_match('/^[\\\\\/]{2}[^\\\\\/]+[\\\\\/][^\\\\\/]+/', $path) === 1;
    if (!$absolute) {
        $path = $resolvedBase . DIRECTORY_SEPARATOR . $path;
    }

    $resolved = realpath($path);
    if ($resolved !== false) {
        return $resolved;
    }

    $parent = realpath(dirname($path));
    $name = basename(rtrim($path, '/\\'));
    if ($parent === false || $name === '' || $name === '.' || $name === '..') {
        throw new RuntimeException(
            "new Cargo targets require an existing parent directory: {$path}",
        );
    }

    return $parent . DIRECTORY_SEPARATOR . $name;
}

function doria_prepare_managed_target(
    string $target,
    string $repository,
    bool $writeOwner,
): string {
    $repository = doria_canonical_path($repository, $repository);
    $target = doria_canonical_path($target, $repository);
    doria_assert_safe_target_path($target, $repository);

    if (file_exists($target) && !is_dir($target)) {
        throw new RuntimeException("Cargo target is not a directory: {$target}");
    }

    $ownerPath = $target . DIRECTORY_SEPARATOR . DORIA_ARTIFACT_OWNER_FILE;
    if (is_file($ownerPath)) {
        $owner = json_decode((string) file_get_contents($ownerPath), true);
        $recordedRepository = is_array($owner) ? ($owner['repository'] ?? null) : null;
        if (!is_string($recordedRepository)) {
            throw new RuntimeException("Cargo target has an invalid ownership marker: {$ownerPath}");
        }
        $recordedRepository = doria_canonical_path($recordedRepository, $repository);
        if (!doria_paths_equal($recordedRepository, $repository)) {
            throw new RuntimeException(
                "Cargo target belongs to another repository: {$recordedRepository}",
            );
        }
        if ($writeOwner) {
            doria_write_cargo_cache_tag($target);
        }

        return $target;
    }

    // A physical repository target is trusted for migration. A `target` symlink
    // resolving to an external cache is not: it must already carry our marker.
    $defaultTarget = $repository . DIRECTORY_SEPARATOR . 'target';
    $claimable = doria_paths_equal($target, $defaultTarget)
        || !is_dir($target)
        || doria_directory_is_empty($target);
    if (!$claimable) {
        throw new RuntimeException(
            "Cargo target is nonempty and has no Doria ownership marker: {$target}; "
                . 'choose a dedicated empty directory instead of a shared Cargo cache',
        );
    }

    if ($writeOwner) {
        doria_write_artifact_owner($target, $repository);
    }

    return $target;
}

function doria_write_artifact_owner(string $target, string $repository): void
{
    $repository = doria_canonical_path($repository, $repository);
    $target = doria_canonical_path($target, $repository);
    doria_assert_safe_target_path($target, $repository);
    if (!is_dir($target) && !mkdir($target, 0777, true) && !is_dir($target)) {
        throw new RuntimeException("could not create managed Cargo target {$target}");
    }

    $ownerPath = $target . DIRECTORY_SEPARATOR . DORIA_ARTIFACT_OWNER_FILE;
    $contents = json_encode(
        ['schemaVersion' => 1, 'repository' => $repository],
        JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR,
    );
    if (file_put_contents($ownerPath, $contents . "\n") === false) {
        throw new RuntimeException("could not write Cargo target ownership marker {$ownerPath}");
    }
    doria_write_cargo_cache_tag($target);
}

function doria_write_cargo_cache_tag(string $target): void
{
    $path = $target . DIRECTORY_SEPARATOR . 'CACHEDIR.TAG';
    if (is_file($path) && file_get_contents($path) === DORIA_CARGO_CACHE_TAG) {
        return;
    }
    if (file_put_contents($path, DORIA_CARGO_CACHE_TAG) === false) {
        throw new RuntimeException("could not write Cargo cache tag {$path}");
    }
}

function doria_assert_safe_target_path(string $target, string $repository): void
{
    if (
        doria_paths_equal($target, $repository)
        || doria_path_contains($target, $repository)
    ) {
        throw new RuntimeException(
            "refusing unsafe Cargo target {$target}: it is the repository root or an ancestor",
        );
    }
}

function doria_directory_is_empty(string $directory): bool
{
    $entries = scandir($directory);
    if ($entries === false) {
        throw new RuntimeException("could not inspect Cargo target {$directory}");
    }

    return count($entries) === 2;
}

function doria_path_contains(string $directory, string $path): bool
{
    $directory = rtrim(doria_comparable_path($directory), '/');
    $path = doria_comparable_path($path);
    if ($directory === '') {
        return str_starts_with($path, '/');
    }

    return str_starts_with($path, $directory . '/');
}

function doria_paths_equal(string $left, string $right): bool
{
    return doria_comparable_path($left) === doria_comparable_path($right);
}

function doria_comparable_path(string $path): string
{
    $path = str_replace('\\', '/', rtrim($path, '/\\'));

    return PHP_OS_FAMILY === 'Windows' ? strtolower($path) : $path;
}

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
