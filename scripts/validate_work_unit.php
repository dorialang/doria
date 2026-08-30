#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/cargo_artifacts.php';

const CACHE_POLICY_VERSION = 1;
const UNTRACKED_CACHE_BYTES = 256 * 1024 * 1024;
const MINIMUM_FREE_BYTES = 5 * 1024 * 1024 * 1024;

$root = dirname(__DIR__);
$withLlvm = false;
$plan = false;
$reclaim = false;
$target = getenv('DORIA_VALIDATION_TARGET_DIR') ?: getenv('CARGO_TARGET_DIR') ?: $root . '/target';

for ($index = 1; $index < count($argv); $index++) {
    switch ($argv[$index]) {
        case '--llvm':
            $withLlvm = true;
            break;
        case '--plan':
            $plan = true;
            break;
        case '--reclaim':
            $reclaim = true;
            break;
        case '--target-dir':
            if (!isset($argv[++$index])) {
                fail('--target-dir requires a path');
            }
            $target = $argv[$index];
            break;
        default:
            fail("unknown option `{$argv[$index]}`");
    }
}

$target = absolute_path($target, $root);
$llvmTarget = $target . DIRECTORY_SEPARATOR . 'llvm-backend';
$stampPath = $target . DIRECTORY_SEPARATOR . '.doria-artifact-cache.json';
$identity = cache_identity($root);
$bytesBefore = measured_size($target);
$freeBefore = available_bytes($target);
$storedIdentity = stored_identity($stampPath);
$reclaimReason = null;

if ($bytesBefore > DORIA_TARGET_WARNING_BYTES) {
    $reclaimReason = 'the managed target exceeds the 15 GiB cap';
} elseif ($storedIdentity !== null && $storedIdentity !== $identity) {
    $reclaimReason = 'the Rust toolchain, lockfile, manifests, or profile policy changed';
} elseif ($storedIdentity === null && $bytesBefore > UNTRACKED_CACHE_BYTES) {
    $reclaimReason = 'the existing target predates managed cache identity tracking';
} elseif ($freeBefore < MINIMUM_FREE_BYTES && $bytesBefore > UNTRACKED_CACHE_BYTES) {
    $reclaimReason = 'the filesystem has less than 5 GiB free';
}

fwrite(
    STDOUT,
    "Managed Cargo target: {$target}\n"
        . 'Allocated before validation: ' . doria_format_bytes($bytesBefore) . "\n"
        . 'Filesystem free before validation: ' . doria_format_bytes($freeBefore) . "\n",
);

if ($reclaim) {
    if ($plan) {
        fwrite(STDOUT, "PLAN: reclaim the managed target with Cargo.\n");
        exit(0);
    }
    reclaim_target($root, $target, 'explicit --reclaim request');
    exit(0);
}

$commands = default_commands($root, $target);
if ($withLlvm) {
    array_push($commands, ...llvm_commands($llvmTarget));
}

if ($plan) {
    fwrite(STDOUT, $reclaimReason === null
        ? "PLAN: create or reuse the compatible managed cache.\n"
        : "PLAN: reclaim because {$reclaimReason}.\n");
    foreach ($commands as $command) {
        fwrite(STDOUT, '> ' . display_command($command['command']) . "\n");
    }
    exit(0);
}

if ($reclaimReason !== null) {
    reclaim_target($root, $target, $reclaimReason);
}

ensure_directory($target);
ensure_directory($root . DIRECTORY_SEPARATOR . 'build' . DIRECTORY_SEPARATOR . 'native');
write_stamp($stampPath, $identity);
$freeAfterReclaim = available_bytes($target);
if ($freeAfterReclaim < MINIMUM_FREE_BYTES) {
    fail(
        'validation requires at least 5 GiB of free space after managed-cache reclamation; '
            . doria_format_bytes($freeAfterReclaim) . ' remains',
    );
}

$failed = null;
try {
    foreach ($commands as $command) {
        run(
            $command['command'],
            $root,
            environment_for($command['target']),
            $command['discardOutput'],
        );
    }
} catch (RuntimeException $error) {
    $failed = $error->getMessage();
}

$bytesAfter = measured_size($target);
$freeAfter = available_bytes($target);
fwrite(
    STDOUT,
    "\nAllocated after validation: " . doria_format_bytes($bytesAfter) . "\n"
        . 'Filesystem free after validation: ' . doria_format_bytes($freeAfter) . "\n",
);

if ($bytesAfter > DORIA_TARGET_WARNING_BYTES) {
    reclaim_target($root, $target, 'the completed validation exceeded the 15 GiB cap');
    $failed ??= 'validation artifacts exceeded the 15 GiB cap and were reclaimed';
}

if ($failed !== null) {
    fail($failed);
}

fwrite(STDOUT, "Work-unit validation passed.\n");

/** @return list<array{command: list<string>, target: string, discardOutput: bool}> */
function default_commands(string $root, string $target): array
{
    $executable = $target . DIRECTORY_SEPARATOR . 'debug' . DIRECTORY_SEPARATOR . 'doriac'
        . (PHP_OS_FAMILY === 'Windows' ? '.exe' : '');
    $native = $root . DIRECTORY_SEPARATOR . 'build' . DIRECTORY_SEPARATOR . 'native';

    return [
        command(['cargo', 'fmt', '--all', '--', '--check'], $target),
        command(['cargo', 'build', '-p', 'doria-rt', '--locked'], $target),
        command(['cargo', 'build', '-p', 'doriac', '--bin', 'doriac', '--locked'], $target),
        command(['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'], $target),
        command(['cargo', 'test', '--workspace', '--all-targets', '--locked'], $target),
        command([$executable, 'check', 'examples/php/person.doria'], $target),
        command([$executable, 'hir', 'examples/php/person.doria'], $target, true),
        command([$executable, 'compile', 'examples/native/main_return_zero.doria', '--target', 'native', '--out', $native . DIRECTORY_SEPARATOR . 'main_return_zero'], $target),
        command([$executable, 'compile', 'examples/native/main_return_42.doria', '--target', 'native', '--out', $native . DIRECTORY_SEPARATOR . 'main_return_42'], $target),
        command([$executable, 'compile', 'examples/native/main_void_hello.doria', '--target', 'native', '--out', $native . DIRECTORY_SEPARATOR . 'main_void_hello'], $target),
    ];
}

/** @return list<array{command: list<string>, target: string, discardOutput: bool}> */
function llvm_commands(string $target): array
{
    $commands = [
        command(['cargo', 'build', '-p', 'doria-rt', '--locked'], $target),
        command(['cargo', 'build', '-p', 'doriac', '--bin', 'doriac', '--features', 'llvm-backend', '--locked'], $target),
        command(['cargo', 'clippy', '-p', 'doriac', '--all-targets', '--features', 'llvm-backend', '--locked', '--', '-D', 'warnings'], $target),
    ];
    foreach (['mir_validation_tests', 'llvm_mir_tests', 'cli_tests', 'stage17_io_tests', 'stage18_tests', 'native_mir_parity_tests'] as $suite) {
        $commands[] = command(
            ['cargo', 'test', '-p', 'doriac', '--test', $suite, '--features', 'llvm-backend', '--locked'],
            $target,
        );
    }

    return $commands;
}

/** @param list<string> $arguments @return array{command: list<string>, target: string, discardOutput: bool} */
function command(array $arguments, string $target, bool $discardOutput = false): array
{
    return ['command' => $arguments, 'target' => $target, 'discardOutput' => $discardOutput];
}

/** @return array<string, string> */
function environment_for(string $target): array
{
    $environment = getenv();
    $environment = is_array($environment) ? $environment : [];
    $environment['CARGO_TARGET_DIR'] = $target;

    return $environment;
}

/** @param list<string> $command @param array<string, string> $environment */
function run(
    array $command,
    string $workingDirectory,
    array $environment,
    bool $discardOutput = false,
): void
{
    fwrite(STDOUT, "\n> " . display_command($command) . "\n");
    $process = proc_open(
        $command,
        [
            0 => ['file', 'php://stdin', 'r'],
            1 => [
                'file',
                $discardOutput
                    ? (PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null')
                    : 'php://stdout',
                'w',
            ],
            2 => ['file', 'php://stderr', 'w'],
        ],
        $pipes,
        $workingDirectory,
        $environment,
    );
    if (!is_resource($process)) {
        throw new RuntimeException('could not start command: ' . display_command($command));
    }
    $status = proc_close($process);
    if ($status !== 0) {
        throw new RuntimeException(
            'command failed with exit code ' . $status . ': ' . display_command($command),
        );
    }
}

function reclaim_target(string $root, string $target, string $reason): void
{
    fwrite(STDOUT, "Reclaiming managed Cargo target because {$reason}.\n");
    run(['cargo', 'clean', '--target-dir', $target], $root, environment_for($target));
}

function cache_identity(string $root): string
{
    $paths = [$root . '/Cargo.toml', $root . '/Cargo.lock'];
    foreach ([$root . '/rust-toolchain.toml', $root . '/.cargo/config.toml'] as $optional) {
        if (is_file($optional)) {
            $paths[] = $optional;
        }
    }
    $manifests = glob($root . '/crates/*/Cargo.toml') ?: [];
    sort($manifests);
    array_push($paths, ...$manifests);
    $parts = ['policy=' . CACHE_POLICY_VERSION, capture(['rustc', '-Vv'], $root)];
    foreach (['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'] as $name) {
        $value = getenv($name);
        $parts[] = $name . '=' . (is_string($value) ? $value : '');
    }
    foreach ($paths as $path) {
        $parts[] = str_replace($root, '', $path) . '=' . hash_file('sha256', $path);
    }

    return hash('sha256', implode("\n", $parts));
}

function stored_identity(string $stampPath): ?string
{
    if (!is_file($stampPath)) {
        return null;
    }
    $decoded = json_decode((string) file_get_contents($stampPath), true);

    return is_array($decoded) && is_string($decoded['identity'] ?? null)
        ? $decoded['identity']
        : null;
}

function write_stamp(string $stampPath, string $identity): void
{
    $contents = json_encode(
        ['policyVersion' => CACHE_POLICY_VERSION, 'identity' => $identity],
        JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR,
    );
    if (file_put_contents($stampPath, $contents . "\n") === false) {
        fail("could not write managed cache identity to {$stampPath}");
    }
}

/** @param list<string> $command */
function capture(array $command, string $workingDirectory): string
{
    $process = proc_open(
        $command,
        [
            0 => ['file', PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null', 'r'],
            1 => ['pipe', 'w'],
            2 => ['pipe', 'w'],
        ],
        $pipes,
        $workingDirectory,
    );
    if (!is_resource($process)) {
        fail('could not start command: ' . display_command($command));
    }
    $stdout = stream_get_contents($pipes[1]);
    $stderr = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    $status = proc_close($process);
    if ($status !== 0) {
        fail(trim($stderr ?: '') ?: 'command failed: ' . display_command($command));
    }

    return $stdout === false ? '' : $stdout;
}

function measured_size(string $target): int
{
    try {
        return doria_allocated_directory_bytes($target);
    } catch (RuntimeException $error) {
        fail($error->getMessage());
    }
}

function available_bytes(string $path): int
{
    while (!is_dir($path)) {
        $parent = dirname($path);
        if ($parent === $path) {
            fail("could not find an existing filesystem parent for {$path}");
        }
        $path = $parent;
    }
    $bytes = disk_free_space($path);
    if ($bytes === false) {
        fail("could not measure free space for {$path}");
    }

    return (int) $bytes;
}

function ensure_directory(string $path): void
{
    if (!is_dir($path) && !mkdir($path, 0777, true) && !is_dir($path)) {
        fail("could not create directory {$path}");
    }
}

function absolute_path(string $path, string $base): string
{
    if (str_starts_with($path, '~' . DIRECTORY_SEPARATOR)) {
        $home = getenv(PHP_OS_FAMILY === 'Windows' ? 'USERPROFILE' : 'HOME');
        if (is_string($home) && $home !== '') {
            $path = $home . substr($path, 1);
        }
    }
    if (str_starts_with($path, '/') || preg_match('/^[A-Za-z]:[\\\\\/]/', $path)) {
        return rtrim($path, '/\\');
    }

    return rtrim($base . DIRECTORY_SEPARATOR . $path, '/\\');
}

/** @param list<string> $command */
function display_command(array $command): string
{
    return implode(' ', array_map(
        static fn (string $argument): string => preg_match('/^[A-Za-z0-9_\.\/:=+,-]+$/', $argument)
            ? $argument
            : escapeshellarg($argument),
        $command,
    ));
}

function fail(string $message): never
{
    fwrite(STDERR, "validate-work-unit: {$message}\n");
    exit(1);
}
