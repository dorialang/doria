#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/cargo_artifacts.php';

// Policy tests import the production identity helpers without refreshing.
if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') !== __FILE__) {
    return;
}

$root = dirname(__DIR__);
$languageServerRoot = null;
$identityJson = false;

for ($index = 1; $index < count($argv); $index++) {
    if ($argv[$index] === '--identity-json') {
        if ($identityJson) {
            fail('--identity-json may only be specified once');
        }
        $identityJson = true;
        continue;
    }
    if ($argv[$index] !== '--language-server') {
        fail("unknown option `{$argv[$index]}`");
    }
    if ($languageServerRoot !== null) {
        fail('--language-server may only be specified once');
    }
    $index++;
    if (!isset($argv[$index]) || $argv[$index] === '') {
        fail('--language-server requires a repository path');
    }
    $languageServerRoot = absolute_path($argv[$index], getcwd() ?: $root);
}

// Repository layout is still never inferred. When `--language-server` is
// omitted the path comes from `doria.languageServerPath` in this checkout's
// local Git config, which a developer recorded there explicitly. That keeps the
// routine refresh a bare command instead of an absolute path retyped every
// time, which is what makes it get skipped.
if ($languageServerRoot === null) {
    $configured = configured_language_server_path($root);
    if ($configured === null) {
        fail(
            "missing --language-server <path>\n\n"
                . "Repository layout is not inferred; provide the Doria language-server\n"
                . "repository used in this environment, or record it once with:\n\n"
                . "    git config --local doria.languageServerPath /absolute/path/to/doria-language-server"
        );
    }
    $languageServerRoot = absolute_path($configured, $root);
}
$languageServerRoot = realpath($languageServerRoot) ?: $languageServerRoot;
if (!is_file($languageServerRoot . DIRECTORY_SEPARATOR . 'scripts' . DIRECTORY_SEPARATOR . 'build.php')) {
    fail("Doria language-server repository not found at:\n    {$languageServerRoot}");
}

$commit = trim(capture(['git', 'rev-parse', 'HEAD'], $root));
$compilerStatus = repository_status($root, $identityJson);
$compilerRevision = source_revision($root, $commit, $compilerStatus);
$languageServerCommit = trim(capture(['git', 'rev-parse', 'HEAD'], $languageServerRoot));
$languageServerStatus = repository_status($languageServerRoot, $identityJson);
$languageServerRevision = source_revision(
    $languageServerRoot,
    $languageServerCommit,
    $languageServerStatus,
);

// Read-only source provenance uses exactly the same identities as refresh.
// Return before install-root discovery, tool probes, builds, or cache cleanup.
if ($identityJson) {
    fwrite(STDOUT, json_encode([
        'schemaVersion' => 1,
        'compilerCommit' => $compilerRevision,
        'compilerHeadCommit' => $commit,
        'languageServerCommit' => $languageServerRevision,
        'languageServerHeadCommit' => $languageServerCommit,
    ], JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR) . "\n");
    exit(0);
}

$cargoRoot = cargo_install_root();
$executableSuffix = PHP_OS_FAMILY === 'Windows' ? '.exe' : '';
$compiler = $cargoRoot . DIRECTORY_SEPARATOR . 'bin' . DIRECTORY_SEPARATOR . 'doriac'
    . $executableSuffix;
$languageServer = $cargoRoot . DIRECTORY_SEPARATOR . 'bin' . DIRECTORY_SEPARATOR . 'doria-lsp'
    . $executableSuffix;

$managedTarget = getenv('DORIA_VALIDATION_TARGET_DIR');
$managedTarget = is_string($managedTarget) && $managedTarget !== ''
    ? absolute_path($managedTarget, $root)
    : $root . DIRECTORY_SEPARATOR . 'target';
try {
    $managedTarget = doria_prepare_managed_target($managedTarget, $root, true);
} catch (RuntimeException $error) {
    fail($error->getMessage());
}
$environment = doria_build_environment($managedTarget);
$environment['CARGO_INSTALL_ROOT'] = $cargoRoot;
$installTarget = $managedTarget
    . DIRECTORY_SEPARATOR
    . 'toolchain-install';
$environment['CARGO_TARGET_DIR'] = $installTarget;
$environment['DORIA_BUILD_COMMIT'] = $compilerRevision;

$installCompiler = [
    'cargo',
    'install',
    '--path',
    $root . DIRECTORY_SEPARATOR . 'crates' . DIRECTORY_SEPARATOR . 'doriac',
    '--locked',
    '--force',
    '--root',
    $cargoRoot,
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

// Installed tools are release artifacts, whereas Cargo's installation target
// is a reclaimable build cache. Delete that cache before verification so this
// workflow cannot accidentally certify a compiler that still references files
// beneath it.
run(['cargo', 'clean', '--target-dir', $installTarget], $root, $environment);
verify_installed_native_compiler($compiler, $environment);

$compilerIdentity = decode_json(capture([$compiler, '--version', '--json'], $root));
if (($compilerIdentity['commit'] ?? null) !== $compilerRevision) {
    fail(
        "installed doriac identifies compiler revision "
        . describe($compilerIdentity['commit'] ?? null)
        . ", expected {$compilerRevision}"
    );
}

$serverIdentity = decode_json(capture([$languageServer, '--version', '--json'], $root));
if (($serverIdentity['compilerCommit'] ?? null) !== $compilerRevision) {
    fail(
        "installed doria-lsp embeds compiler revision "
        . describe($serverIdentity['compilerCommit'] ?? null)
        . ", expected {$compilerRevision}"
    );
}

require_unshadowed('doriac' . $executableSuffix, $compiler);
require_unshadowed('doria-lsp' . $executableSuffix, $languageServer);

fwrite(
    STDOUT,
    "\nDevelopment toolchain refreshed:\n"
        . "    compiler source: {$compilerRevision}\n"
        . "    language-server source: {$languageServerRevision}\n"
        . "    {$compiler}\n"
        . "    {$languageServer}\n"
);

function source_revision(string $path, string $headCommit, string $status): string
{
    return $status === '' ? $headCommit : development_revision($path, $headCommit);
}

function repository_status(string $path, bool $readOnly = false): string
{
    $command = ['git'];
    if ($readOnly) {
        // Even an index-stat refresh is unnecessary for a read-only query.
        $command[] = '--no-optional-locks';
    }
    array_push($command, 'status', '--porcelain=v1', '--untracked-files=all');
    return trim(capture(
        $command,
        $path,
    ));
}

/**
 * Identify the exact source tree Cargo will compile. Clean development builds
 * keep their Git commit; dirty builds use this value to invalidate every
 * compiler-revision-keyed cache without masquerading as the HEAD commit.
 */
function development_revision(string $path, string $commit): string
{
    $listing = capture(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
        $path,
    );
    $files = array_values(array_filter(
        explode("\0", $listing),
        static fn (string $file): bool => $file !== '',
    ));
    sort($files, SORT_STRING);

    $hash = hash_init('sha256');
    hash_update($hash, "doria-development-revision-v1\0{$commit}\0");
    foreach ($files as $relative) {
        $absolute = $path . DIRECTORY_SEPARATOR . str_replace('/', DIRECTORY_SEPARATOR, $relative);
        hash_update($hash, $relative . "\0");
        if (is_link($absolute)) {
            $target = readlink($absolute);
            hash_update($hash, "link\0" . ($target === false ? '' : $target) . "\0");
            continue;
        }
        if (!is_file($absolute)) {
            hash_update($hash, "missing\0");
            continue;
        }

        $mode = fileperms($absolute);
        hash_update($hash, 'file:' . (($mode !== false && ($mode & 0111) !== 0) ? 'x' : '-') . "\0");
        $stream = fopen($absolute, 'rb');
        if ($stream === false) {
            fail("could not read development source while fingerprinting:\n    {$absolute}");
        }
        hash_update_stream($hash, $stream);
        fclose($stream);
        hash_update($hash, "\0");
    }

    return 'dev-' . hash_final($hash);
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
function capture(
    array $command,
    string $workingDirectory,
    ?array $environment = null,
): string
{
    $result = capture_result($command, $workingDirectory, $environment);
    if ($result['status'] !== 0) {
        fail(trim($result['stderr']) ?: 'command failed: ' . display_command($command));
    }
    return $result['stdout'];
}

/** @param list<string> $command @return array{status: int, stdout: string, stderr: string} */
function capture_result(
    array $command,
    string $workingDirectory,
    ?array $environment = null,
): array
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
        $environment,
    );
    if (!is_resource($process)) {
        fail('could not start command: ' . display_command($command));
    }
    $stdout = stream_get_contents($pipes[1]);
    $stderr = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    $status = proc_close($process);
    return [
        'status' => $status,
        'stdout' => $stdout === false ? '' : $stdout,
        'stderr' => $stderr === false ? '' : $stderr,
    ];
}

/**
 * Reads the explicitly recorded language-server checkout for this repository.
 * Returns null when nothing was recorded, so the caller still reports a missing
 * path rather than guessing one.
 */
function configured_language_server_path(string $root): ?string
{
    $process = proc_open(
        ['git', 'config', '--local', '--get', 'doria.languageServerPath'],
        [
            0 => ['file', PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null', 'r'],
            1 => ['pipe', 'w'],
            2 => ['pipe', 'w'],
        ],
        $pipes,
        $root,
    );
    if (!is_resource($process)) {
        return null;
    }
    $stdout = stream_get_contents($pipes[1]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    $status = proc_close($process);
    if ($status !== 0) {
        return null;
    }
    $value = trim($stdout === false ? '' : $stdout);

    return $value === '' ? null : $value;
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

/** @param array<string, string> $environment */
function verify_installed_native_compiler(string $compiler, array $environment): void
{
    $directory = $environment['TMPDIR']
        . DIRECTORY_SEPARATOR
        . 'doria-installed-compiler-smoke-'
        . bin2hex(random_bytes(8));
    if (!mkdir($directory, 0700, true) && !is_dir($directory)) {
        fail("could not create installed-compiler smoke directory:\n    {$directory}");
    }
    register_shutdown_function(static function () use ($directory): void {
        remove_tree($directory);
    });

    try {
        $probeEnvironment = $environment;
        unset($probeEnvironment['DORIA_RT_PATH']);
        $probeTarget = $directory . DIRECTORY_SEPARATOR . 'empty-cargo-target';
        if (!mkdir($probeTarget, 0700, true) && !is_dir($probeTarget)) {
            fail("could not create isolated compiler-probe target:\n    {$probeTarget}");
        }
        $probeEnvironment['CARGO_TARGET_DIR'] = $probeTarget;
        $source = $directory . DIRECTORY_SEPARATOR . 'main.doria';
        $executable = $directory
            . DIRECTORY_SEPARATOR
            . 'main'
            . (PHP_OS_FAMILY === 'Windows' ? '.exe' : '');
        $program = <<<'DORIA'
function main(): void
{
    echo "installed runtime ready\n";
}
DORIA;
        if (file_put_contents($source, $program . "\n") === false) {
            fail("could not write installed-compiler smoke source:\n    {$source}");
        }
        run(
            [
                $compiler,
                'compile',
                $source,
                '--target',
                'native',
                '--out',
                $executable,
                '--diagnostic-format',
                'human',
                '--diagnostic-color',
                'never',
            ],
            $directory,
            $probeEnvironment,
        );
        require_executable($executable, 'installed doriac native smoke artifact');
        $output = capture([$executable], $directory, $probeEnvironment);
        if ($output !== "installed runtime ready\n") {
            fail(
                "installed doriac native smoke emitted unexpected output:\n"
                    . describe($output)
            );
        }
        verify_installed_php_compiler($compiler, $probeEnvironment, $directory);
        if (scandir($probeTarget) !== ['.', '..']) {
            fail('installed compiler verification populated its isolated empty Cargo target');
        }
    } finally {
        remove_tree($directory);
    }
}

/** @param array<string, string> $environment @param list<string> $phpOptions */
function verify_installed_php_compiler(string $compiler, array $environment, string $directory, array $phpOptions = []): void
{
    $root = dirname(__DIR__);
    $availability = capture_result([
        PHP_BINARY, ...$phpOptions, '-r', 'echo extension_loaded("FFI") ? "available" : "absent";',
    ], $directory, $environment);
    if ($availability['status'] !== 0 || $availability['stderr'] !== ''
        || !in_array($availability['stdout'], ['available', 'absent'], true)) {
        fail('could not determine PHP FFI availability: ' . describe($availability));
    }
    $ffiAvailable = $availability['stdout'] === 'available';
    foreach (['main_stream_capabilities' => false, 'main_stream_writable_file' => true] as $fixture => $native) {
        $published = $directory . DIRECTORY_SEPARATOR . $fixture;
        $php = $published . DIRECTORY_SEPARATOR . 'main.php';
        run([
            $compiler, 'compile', $root . '/examples/native/' . $fixture . '.doria',
            '--target', 'php', '--out', $php,
            '--diagnostic-color', 'never',
        ], $directory, $environment);
        $relocated = $published . '-relocated';
        if (!rename($published, $relocated)) {
            fail('could not relocate installed-compiler PHP publication');
        }
        $php = $relocated . DIRECTORY_SEPARATOR . 'main.php';
        $runtime = $relocated . DIRECTORY_SEPARATOR . '.doria-runtime';
        if (is_dir($runtime) !== $native) {
            fail("installed compiler published incorrect PHP dependencies for {$fixture}");
        }
        $sidecars = $native ? verify_php_runtime_assets($runtime) : [];
        $result = capture_result(
            [PHP_BINARY, ...$phpOptions, '-d', $native ? 'ffi.enable=1' : 'ffi.enable=0', $php],
            $relocated,
            $environment,
        );
        if ($native && !$ffiAvailable) {
            require_php_dependency_diagnostic($result);
            fwrite(STDOUT, "PHP native-backed execution unavailable (FFI extension absent); positive execution skipped. Published assets and B3602 verified.\n");
            continue;
        }
        $golden = $root . '/crates/doriac/tests/fixtures/native_io/' . $fixture;
        foreach (['status', 'stdout', 'stderr'] as $field) {
            $expected = file_get_contents($golden . '/expected_' . $field);
            if ($expected === false || $result[$field] !== ($field === 'status' ? (int) trim($expected) : $expected)) {
                fail("installed compiler PHP probe {$fixture} has unexpected {$field}: " . describe($result[$field]));
            }
        }
        if (!$native) {
            continue;
        }
        foreach (['positioned.bin' => 'XBCDXXX', 'duplex.bin' => 'ABXX'] as $file => $expected) {
            if (file_get_contents($relocated . DIRECTORY_SEPARATOR . $file) !== $expected) {
                fail("installed compiler PHP probe produced incorrect file bytes for {$file}");
            }
        }
        require_php_dependency_diagnostic(capture_result(
            [PHP_BINARY, ...$phpOptions, '-d', 'ffi.enable=0', $php], $relocated, $environment,
        ));
        if (!unlink($sidecars[0])) {
            fail('could not isolate the published PHP runtime sidecar for the missing-dependency probe');
        }
        require_php_dependency_diagnostic(capture_result(
            [PHP_BINARY, ...$phpOptions, '-d', 'ffi.enable=1', $php], $relocated, $environment,
        ));
        fwrite(STDOUT, "PHP native-backed execution validated after relocation.\n");
    }
}

/** @return list<string> */
function verify_php_runtime_assets(string $runtime): array
{
    $sidecars = glob($runtime . '/*/*.doria-runtime.json');
    if ($sidecars === false || count($sidecars) !== 1) {
        fail('published PHP runtime must contain one identified shared library');
    }
    $metadata = json_decode(file_get_contents($sidecars[0]) ?: '', true);
    $library = substr($sidecars[0], 0, -strlen('.doria-runtime.json'));
    if (!is_array($metadata) || ($metadata['artifactKind'] ?? null) !== 'shared'
        || ($metadata['featureSet'] ?? null) !== ['hosted-shared-runtime']
        || !is_file($library) || filesize($library) !== ($metadata['bytes'] ?? null)
        || hash_file('sha256', $library) !== ($metadata['sha256'] ?? null)) {
        fail('published PHP runtime bytes or metadata are invalid');
    }
    return $sidecars;
}

/** @param array{status: int, stdout: string, stderr: string} $result */
function require_php_dependency_diagnostic(array $result): void
{
    if ($result['status'] !== 1 || $result['stdout'] !== ''
        || !str_contains($result['stderr'], 'Error[B3602]')
        || str_contains($result['stderr'], 'Fatal error')
        || str_contains($result['stderr'], 'Stack trace')) {
        fail('installed compiler PHP dependency probe did not report B3602: ' . describe($result));
    }
}

function remove_tree(string $path): void
{
    if (is_link($path) || is_file($path)) {
        @unlink($path);
        return;
    }
    if (!is_dir($path)) {
        return;
    }
    foreach (scandir($path) ?: [] as $entry) {
        if ($entry === '.' || $entry === '..') {
            continue;
        }
        remove_tree($path . DIRECTORY_SEPARATOR . $entry);
    }
    @rmdir($path);
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
