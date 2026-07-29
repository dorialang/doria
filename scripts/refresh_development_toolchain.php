#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$languageServerRoot = dirname($root) . DIRECTORY_SEPARATOR . 'doria-language-server';

for ($index = 1; $index < count($argv); $index++) {
    if ($argv[$index] !== '--language-server') {
        fail("unknown option `{$argv[$index]}`");
    }
    $index++;
    if (!isset($argv[$index]) || $argv[$index] === '') {
        fail('--language-server requires a repository path');
    }
    $languageServerRoot = absolute_path($argv[$index], getcwd() ?: $root);
}

$languageServerRoot = realpath($languageServerRoot) ?: $languageServerRoot;
if (!is_file($languageServerRoot . DIRECTORY_SEPARATOR . 'scripts' . DIRECTORY_SEPARATOR . 'build.php')) {
    fail("Doria language-server repository not found at:\n    {$languageServerRoot}");
}

require_clean_repository($root, 'Doria compiler');
require_clean_repository($languageServerRoot, 'Doria language server');

$commit = trim(capture(['git', 'rev-parse', 'HEAD'], $root));
$cargoRoot = cargo_install_root();
$executableSuffix = PHP_OS_FAMILY === 'Windows' ? '.exe' : '';
$compiler = $cargoRoot . DIRECTORY_SEPARATOR . 'bin' . DIRECTORY_SEPARATOR . 'doriac'
    . $executableSuffix;
$languageServer = $cargoRoot . DIRECTORY_SEPARATOR . 'bin' . DIRECTORY_SEPARATOR . 'doria-lsp'
    . $executableSuffix;

$environment = current_environment();
$environment['CARGO_TARGET_DIR'] = $cargoRoot
    . DIRECTORY_SEPARATOR
    . 'doria'
    . DIRECTORY_SEPARATOR
    . 'build'
    . DIRECTORY_SEPARATOR
    . 'doriac';
$environment['DORIA_BUILD_COMMIT'] = $commit;

$installCompiler = [
    'cargo',
    'install',
    '--path',
    $root . DIRECTORY_SEPARATOR . 'crates' . DIRECTORY_SEPARATOR . 'doriac',
    '--locked',
    '--force',
];
$llvmPrefix = detect_llvm18_prefix();
if ($llvmPrefix !== null) {
    $installCompiler[] = '--features';
    $installCompiler[] = 'llvm-backend';
    $environment['LLVM_SYS_181_PREFIX'] = $llvmPrefix;
}

run($installCompiler, $root, $environment);
run(
    [
        PHP_BINARY,
        $languageServerRoot . DIRECTORY_SEPARATOR . 'scripts' . DIRECTORY_SEPARATOR . 'build.php',
        'install-server',
        '--compiler-path',
        $root,
    ],
    $languageServerRoot,
    $environment,
);

require_executable($compiler, 'installed doriac');
require_executable($languageServer, 'installed doria-lsp');

$compilerIdentity = decode_json(capture([$compiler, '--version', '--json'], $root));
if (($compilerIdentity['commit'] ?? null) !== $commit) {
    fail(
        "installed doriac identifies commit "
        . describe($compilerIdentity['commit'] ?? null)
        . ", expected {$commit}"
    );
}

$serverIdentity = decode_json(capture([$languageServer, '--version', '--json'], $root));
if (($serverIdentity['compilerCommit'] ?? null) !== $commit) {
    fail(
        "installed doria-lsp embeds compiler commit "
        . describe($serverIdentity['compilerCommit'] ?? null)
        . ", expected {$commit}"
    );
}

require_unshadowed('doriac' . $executableSuffix, $compiler);
require_unshadowed('doria-lsp' . $executableSuffix, $languageServer);

fwrite(
    STDOUT,
    "\nDevelopment toolchain refreshed from commit {$commit}:\n"
        . "    {$compiler}\n"
        . "    {$languageServer}\n"
);

function require_clean_repository(string $path, string $label): void
{
    $status = capture(['git', 'status', '--porcelain'], $path);
    if (trim($status) !== '') {
        fail("{$label} repository must be committed and clean before refreshing installed tools.");
    }
}

/** @param list<string> $command @param array<string, string>|null $environment */
function run(array $command, string $workingDirectory, ?array $environment = null): void
{
    fwrite(STDOUT, "\n> " . display_command($command) . "\n");
    $process = proc_open(
        $command,
        [
            0 => ['file', 'php://stdin', 'r'],
            1 => ['file', 'php://stdout', 'w'],
            2 => ['file', 'php://stderr', 'w'],
        ],
        $pipes,
        $workingDirectory,
        $environment,
    );
    if (!is_resource($process)) {
        fail('could not start command: ' . display_command($command));
    }
    $status = proc_close($process);
    if ($status !== 0) {
        fail("command failed with exit code {$status}: " . display_command($command));
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

function cargo_install_root(): string
{
    foreach (['CARGO_INSTALL_ROOT', 'CARGO_HOME'] as $name) {
        $value = getenv($name);
        if (is_string($value) && $value !== '') {
            return rtrim($value, '/\\');
        }
    }
    $home = getenv(PHP_OS_FAMILY === 'Windows' ? 'USERPROFILE' : 'HOME');
    if (!is_string($home) || $home === '') {
        fail('could not determine Cargo install root; set CARGO_INSTALL_ROOT or CARGO_HOME');
    }

    return rtrim($home, '/\\') . DIRECTORY_SEPARATOR . '.cargo';
}

/** @return array<string, string> */
function current_environment(): array
{
    $environment = getenv();
    return is_array($environment) ? $environment : [];
}

function require_executable(string $path, string $label): void
{
    if (!is_file($path) || (PHP_OS_FAMILY !== 'Windows' && !is_executable($path))) {
        fail("{$label} was not created at:\n    {$path}");
    }
}

function require_unshadowed(string $name, string $expected): void
{
    $resolved = find_on_path($name);
    if ($resolved === null) {
        fail(
            "{$name} was installed at:\n    {$expected}\n"
                . "but that directory is not on PATH"
        );
    }
    $actual = realpath($resolved) ?: $resolved;
    $wanted = realpath($expected) ?: $expected;
    $matches = PHP_OS_FAMILY === 'Windows'
        ? strtolower($actual) === strtolower($wanted)
        : $actual === $wanted;
    if (!$matches) {
        fail(
            "PATH resolves {$name} to a different executable:\n"
                . "    {$resolved}\n\nInstalled artifact:\n"
                . "    {$expected}\n\n"
                . 'Remove or reorder the shadowing entry; source launchers must not occupy '
                . 'installed tool names.'
        );
    }
}

function find_on_path(string $name): ?string
{
    $path = getenv('PATH');
    if (!is_string($path)) {
        return null;
    }
    foreach (explode(PATH_SEPARATOR, $path) as $directory) {
        if ($directory === '') {
            continue;
        }
        $candidate = rtrim($directory, '/\\') . DIRECTORY_SEPARATOR . $name;
        if (is_file($candidate) && (PHP_OS_FAMILY === 'Windows' || is_executable($candidate))) {
            return $candidate;
        }
    }

    return null;
}

/** @return array<string, mixed> */
function decode_json(string $json): array
{
    $value = json_decode($json, true);
    if (!is_array($value)) {
        fail('tool identity was not a JSON object');
    }

    return $value;
}

function detect_llvm18_prefix(): ?string
{
    $fromEnvironment = getenv('LLVM_SYS_181_PREFIX');
    if (is_string($fromEnvironment) && llvm_config_is_18($fromEnvironment)) {
        return $fromEnvironment;
    }
    foreach ([
        '/opt/homebrew/opt/llvm@18',
        '/usr/local/opt/llvm@18',
        '/usr/lib/llvm-18',
        '/opt/homebrew/opt/llvm',
        '/usr/local/opt/llvm',
    ] as $prefix) {
        if (llvm_config_is_18($prefix)) {
            return $prefix;
        }
    }

    return null;
}

function llvm_config_is_18(string $prefix): bool
{
    $executable = rtrim($prefix, '/\\')
        . DIRECTORY_SEPARATOR
        . 'bin'
        . DIRECTORY_SEPARATOR
        . (PHP_OS_FAMILY === 'Windows' ? 'llvm-config.exe' : 'llvm-config');
    if (!is_file($executable)) {
        return false;
    }
    $version = capture([$executable, '--version'], getcwd() ?: '.');

    return str_starts_with(trim($version), '18');
}

function absolute_path(string $path, string $base): string
{
    if (
        str_starts_with($path, '/')
        || str_starts_with($path, '\\')
        || preg_match('/^[A-Za-z]:[\\\\\/]/', $path) === 1
    ) {
        return $path;
    }

    return $base . DIRECTORY_SEPARATOR . $path;
}

/** @param list<string> $command */
function display_command(array $command): string
{
    return implode(' ', array_map(
        static fn (string $argument): string => preg_match('/^[A-Za-z0-9_.\/:\\\\=@-]+$/', $argument)
            ? $argument
            : escapeshellarg($argument),
        $command,
    ));
}

function describe(mixed $value): string
{
    return is_scalar($value) || $value === null ? var_export($value, true) : get_debug_type($value);
}

function fail(string $message): never
{
    fwrite(STDERR, "refresh-development-toolchain: {$message}\n");
    exit(1);
}
