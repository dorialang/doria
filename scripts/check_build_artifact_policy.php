#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/cargo_artifacts.php';
require_once __DIR__ . '/cargo_artifact_cleanup_tests.php';

$root = dirname(__DIR__);
$failures = [];
$identityOnly = in_array('--identity-only', $argv, true);

$cargo = read('Cargo.toml');
$agents = read('AGENTS.md');
$contributing = read('CONTRIBUTING.md');
$validator = read('scripts/validate_work_unit.php');
$refresh = read('scripts/refresh_development_toolchain.php');
$compilerMain = read('crates/doriac/src/main.rs');
$runtimeArtifact = read('crates/doriac/src/runtime_artifact.rs');
$compilerBuild = read('crates/doriac/build.rs');
$timeSource = read('crates/doria-time/src/lib.rs');
$runtimeReproducibility = read('crates/doriac/tests/runtime_reproducibility_tests.rs');
$runtimeDecision = read('docs/decisions/0073-stage-15-llvm-release-backend.md');
$ci = read('.github/workflows/ci.yml');

foreach ([
    '[profile.dev]' => $cargo,
    'debug = "line-tables-only"' => $cargo,
    '[profile.dev.package."*"]' => $cargo,
    '[profile.test.package."*"]' => $cargo,
    "php scripts/validate_work_unit.php\n" => $agents,
    'php scripts/validate_work_unit.php --llvm' => $agents,
    'php scripts/validate_work_unit.php --reclaim' => $contributing,
    "'llvm-backend'" => $validator,
    "'cargo', 'clean', '--target-dir'" => $validator,
    'doria_prepare_managed_target(' => $validator,
    'doria_write_artifact_owner(' => $validator,
    'doria_build_environment($target)' => $validator,
    'doria_obsolete_test_plan(' => $validator,
    'doria_reclaim_obsolete_tests(' => $validator,
    '--reclaim-obsolete --plan' => $contributing,
    '--reclaim-obsolete --approve-plan <planId>' => $agents,
    "'scripts/check_build_artifact_policy.php'" => $validator,
    "'scripts/check_docs_authority.php'" => $validator,
    "'scripts/check_diagnostic_style.php'" => $validator,
    "'scripts/check_panic_catalogue.php'" => $validator,
    "'scripts/check_runtime_diagnostic_architecture.php'" => $validator,
    "getenv('DORIA_VALIDATION_TARGET_DIR')" => $refresh,
    "'toolchain-install'" => $refresh,
    "'cargo', 'clean', '--target-dir', \$installTarget" => $refresh,
    'doria_build_environment($managedTarget)' => $refresh,
    'doria_prepare_managed_target($managedTarget, $root, true)' => $refresh,
    'verify_installed_native_compiler($compiler, $environment)' => $refresh,
    'verify_installed_php_compiler($compiler, $probeEnvironment, $directory)' => $refresh,
    'main_stream_writable_file' => $refresh,
    'ffi.enable=0' => $refresh,
    'Error[B3602]' => $refresh,
    'verify_php_runtime_assets($runtime)' => $refresh,
    'positive execution skipped' => $refresh,
    'php_publication_probe_without_ffi_extension_reports_unavailable' => $runtimeReproducibility,
    "unset(\$probeEnvironment['DORIA_RT_PATH'])" => $refresh,
    "\$probeEnvironment['CARGO_TARGET_DIR'] = \$probeTarget" => $refresh,
    'register_embedded_runtimes(' => $compilerMain,
    'register_embedded_shared_runtime(' => $compilerMain,
    'include_bytes!(env!("DORIA_RT_BUILT_DEBUG_PATH"))' => $compilerMain,
    'include_bytes!(env!("DORIA_RT_BUILT_RELEASE_PATH"))' => $compilerMain,
    'EMBEDDED_RUNTIME_CACHE_DIRECTORY' => $runtimeArtifact,
    'Cargo target directories contain only reclaimable build state.' => $contributing,
    'Installation acceptance therefore deletes the entire installation' => $runtimeDecision,
    'struct ScratchDirectory' => $runtimeReproducibility,
    'remove_dir_all(&self.path)' => $runtimeReproducibility,
    'php scripts/check_build_artifact_policy.php' => $ci,
] as $needle => $contents) {
    if (!str_contains($contents, $needle)) {
        $failures[] = "missing required artifact-policy marker: {$needle}";
    }
}

// The compiler and both runtime profiles share the native clock substrate.
// Its source and manifest must also retrigger bundled runtime construction;
// a normal compiler dependency alone does not rerun the embedding build script.
foreach ([
    ['#![no_std]', $timeSource],
    ['"doria-time"', $compilerBuild],
    ['.join(sibling).join("src")', $compilerBuild],
    ['.join(sibling)', $compilerBuild],
    ['.join("Cargo.toml")', $compilerBuild],
] as [$needle, $contents]) {
    if (!str_contains($contents, $needle)) {
        $failures[] = "missing shared clock build input: {$needle}";
    }
}

$llvmStep = strstr($ci, '- name: Run shared validator and LLVM-focused tests');
$llvmStep = $llvmStep === false ? '' : (string) strstr(substr($llvmStep, 6), '- name:', true);
if (!str_contains($llvmStep, 'cargo test -p doriac --features llvm-backend --locked --verbose --no-fail-fast')) {
    $failures[] = 'the tier-1 LLVM test step must be one cargo test command with --no-fail-fast';
}

foreach ([
    'stream_capability_tests',
    'time_contract_tests',
    'nullable_default_tests',
    'pending_stream_contract_tests',
    'pending_selected_storage_tests',
    'pending_selected_storage_mir_tests',
    'io_progress_native_structure_tests',
] as $suite) {
    if (
        !str_contains($validator, "'{$suite}'")
        || !str_contains($llvmStep, "--test {$suite}\n")
    ) {
        $failures[] = "missing canonical local or tier-1 LLVM stream suite: {$suite}";
    }
}

if (substr_count($cargo, 'debug = false') < 2) {
    $failures[] = 'Cargo.toml must disable dependency debug metadata in dev and test profiles';
}
if (str_contains($validator, "'rm'")) {
    $failures[] = 'the managed validator must reclaim through Cargo, not recursive deletion';
}
if (str_contains($ci, 'DORIA_RT_PATH')) {
    $failures[] = 'canonical CI must exercise compiler-selected runtimes; explicit override cases belong in isolated runtime-artifact tests';
}
if (str_contains($agents, 'cargo build --workspace --all-targets --locked --verbose')) {
    $failures[] = 'AGENTS.md still requires the redundant all-target development build';
}
$sizeProbe = read('scripts/check_cargo_target_size.php');
if (
    !str_contains($sizeProbe, 'escapeshellarg($target)')
    || !str_contains($sizeProbe, '" --reclaim`')
) {
    $failures[] = 'the size probe must preserve its selected target in the reclaim command';
}

$ownershipCheck = strpos($refresh, 'doria_prepare_managed_target($managedTarget, $root, true)');
$scratchSetup = strpos($refresh, 'doria_build_environment($managedTarget)');
if ($ownershipCheck === false || $scratchSetup === false || $ownershipCheck >= $scratchSetup) {
    $failures[] = 'tool refresh must verify cache ownership before creating build scratch files';
}
$installCleanup = strpos($refresh, "run(['cargo', 'clean', '--target-dir', \$installTarget]");
$installedProbe = strpos($refresh, 'verify_installed_native_compiler($compiler, $environment);');
if ($installCleanup === false || $installedProbe === false || $installCleanup >= $installedProbe) {
    $failures[] = 'installed compiler verification must run after installation-cache cleanup';
}

test_refresh_source_identity($refresh, $failures, $identityOnly);
if (!$identityOnly) {
    test_target_ownership($failures);
    test_php_dependency_probe($failures);
    test_selective_artifact_cleanup($failures);
    test_stale_linker_object_cleanup($failures);
    test_interrupted_test_cleanup($failures);
}

if ($failures !== []) {
    foreach ($failures as $failure) {
        fwrite(STDERR, "artifact-policy: {$failure}\n");
    }
    exit(1);
}

fwrite(STDOUT, $identityOnly
    ? "Read-only refresh identity guard passed.\n"
    : "Build artifact policy guard passed.\n");

/** @param list<string> $failures */
function test_php_dependency_probe(array &$failures): void
{
    $valid = ['status' => 1, 'stdout' => '', 'stderr' => 'Error[B3602]: PHP Native Runtime Dependency Is Unavailable'];
    foreach ([
        $valid,
        array_replace($valid, ['status' => 0]),
        array_replace($valid, ['stdout' => 'unexpected output']),
        array_replace($valid, ['stderr' => 'Error[B0001]: Unrelated Error']),
        array_replace($valid, ['stderr' => $valid['stderr'] . "\nFatal error"]),
        array_replace($valid, ['stderr' => $valid['stderr'] . "\nStack trace"]),
    ] as $index => $case) {
        $result = capture_result([
            PHP_BINARY, '-r',
            'require $argv[1]; require_php_dependency_diagnostic(json_decode($argv[2], true, 512, JSON_THROW_ON_ERROR));',
            __DIR__ . '/refresh_development_toolchain.php',
            json_encode($case, JSON_THROW_ON_ERROR),
        ], dirname(__DIR__));
        if ($result['status'] !== ($index === 0 ? 0 : 1)) {
            $failures[] = "PHP dependency probe incorrectly classified case {$index}";
        }
    }
}

/** @param list<string> $failures */
function test_refresh_source_identity(string $refresh, array &$failures, bool $required): void
{
    $revisions = strpos($refresh, '$languageServerRevision =');
    $branch = $revisions === false ? false : strpos($refresh, 'if ($identityJson) {', $revisions);
    $exit = $branch === false ? false : strpos($refresh, 'exit(0);', $branch);
    $install = strpos($refresh, '$cargoRoot = cargo_install_root();');
    if ($branch === false || $exit === false || $install === false || $exit >= $install) {
        $failures[] = 'source identity must exit before installation-root discovery';
        return; // Never exercise an identity query whose isolation fence failed.
    }
    foreach ([
        'detect_llvm18_prefix();',
        'run($installCompiler',
        'verify_installed_native_compiler($compiler, $environment);',
    ] as $call) {
        $position = strpos($refresh, $call);
        if ($position === false || $position <= $exit) {
            $failures[] = "source identity must exit before {$call}";
            return;
        }
    }

    // Import the actual helper, never another implementation of its hash.
    require_once __DIR__ . '/refresh_development_toolchain.php';
    $root = dirname(__DIR__);
    $head = trim(capture(['git', 'rev-parse', 'HEAD'], $root));
    if (source_revision($root, $head, '') !== $head) {
        $failures[] = 'clean source identity must remain the HEAD commit';
    }
    $expected = development_revision($root, $head);
    if (source_revision($root, $head, ' M fixture') !== $expected) {
        $failures[] = 'dirty source identity must use the production development fingerprint';
    }
    if (preg_match('/^dev-[0-9a-f]{64}$/', $expected) !== 1 || $expected === $head) {
        $failures[] = 'development fingerprints must not masquerade as HEAD';
    }

    $configured = configured_language_server_path($root);
    if ($configured === null) {
        if ($required) {
            $failures[] = 'identity CLI verification requires the explicitly configured language-server path';
        } else {
            fwrite(STDOUT, "Identity CLI check skipped: no configured language-server checkout.\n");
        }
        return;
    }
    $languageServer = absolute_path($configured, $root);
    $languageServer = realpath($languageServer) ?: $languageServer;

    $directory = sys_get_temp_dir()
        . DIRECTORY_SEPARATOR
        . 'doria-identity-'
        . bin2hex(random_bytes(6));
    try {
        if (!mkdir($directory, 0700)) {
            throw new RuntimeException('could not create identity verification fixture');
        }
        $environment = current_environment();
        $sentinels = ['keep' => 'identity queries must not install or clean'];
        // Existing files reject accidental directory creation without assuming
        // where Git, Cargo, package managers, or executable shims live.
        foreach ([
            'CARGO_INSTALL_ROOT' => 'install-root',
            'CARGO_HOME' => 'cargo-home',
            'CARGO_TARGET_DIR' => 'cargo-target',
            'DORIA_VALIDATION_TARGET_DIR' => 'validation-target',
        ] as $variable => $entry) {
            $environment[$variable] = $directory . DIRECTORY_SEPARATOR . $entry;
            $sentinels[$entry] = "{$variable} must remain an untouched file";
        }
        foreach ($sentinels as $entry => $contents) {
            $path = $directory . DIRECTORY_SEPARATOR . $entry;
            if (file_put_contents($path, $contents) !== strlen($contents)) {
                throw new RuntimeException('could not create identity isolation sentinel');
            }
        }
        $entries = ['.', '..', ...array_keys($sentinels)];
        sort($entries, SORT_STRING);
        foreach ([[], ['--language-server', $languageServer]] as $selection) {
            $process = proc_open(
                [PHP_BINARY, __DIR__ . '/refresh_development_toolchain.php', '--identity-json', ...$selection],
                [
                    0 => ['file', PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null', 'r'],
                    1 => ['pipe', 'w'],
                    2 => ['pipe', 'w'],
                ],
                $pipes,
                $directory,
                $environment,
            );
            if (!is_resource($process)) {
                throw new RuntimeException('could not start identity query');
            }
            $output = stream_get_contents($pipes[1]);
            $error = stream_get_contents($pipes[2]);
            fclose($pipes[1]);
            fclose($pipes[2]);
            if (proc_close($process) !== 0 || $error !== '') {
                throw new RuntimeException('identity query failed: ' . $error);
            }
            $identity = json_decode((string) $output, true, 512, JSON_THROW_ON_ERROR);
            $keys = [
                'schemaVersion',
                'compilerCommit',
                'compilerHeadCommit',
                'languageServerCommit',
                'languageServerHeadCommit',
            ];
            if (array_keys($identity) !== $keys || $identity['schemaVersion'] !== 1) {
                throw new RuntimeException('unexpected source-identity JSON schema');
            }
            foreach ([[$root, 'compiler'], [$languageServer, 'languageServer']] as [$repository, $prefix]) {
                $repositoryHead = trim(capture(['git', 'rev-parse', 'HEAD'], $repository));
                $revision = $identity[$prefix . 'Commit'];
                if ($identity[$prefix . 'HeadCommit'] !== $repositoryHead || !is_string($revision)) {
                    throw new RuntimeException('source-identity HEAD mismatch');
                }
                $dirty = repository_status($repository, true) !== '';
                $validRevision = $dirty
                    ? preg_match('/^dev-[0-9a-f]{64}$/', $revision) === 1
                    : $revision === $repositoryHead;
                if (!$validRevision) {
                    throw new RuntimeException('source-identity cleanliness mismatch');
                }
            }
            clearstatcache(true, $directory);
            if (is_link($directory) || !is_dir($directory) || scandir($directory) !== $entries) {
                throw new RuntimeException('identity query modified installation/cache sentinels');
            }
            foreach ($sentinels as $entry => $contents) {
                $path = $directory . DIRECTORY_SEPARATOR . $entry;
                clearstatcache(true, $path);
                if (is_link($path) || !is_file($path) || file_get_contents($path) !== $contents) {
                    throw new RuntimeException('identity query modified isolation sentinel ' . $entry);
                }
            }
        }
    } catch (Throwable $error) {
        $failures[] = 'read-only source-identity regression failed: ' . $error->getMessage();
    } finally {
        remove_test_tree($directory);
    }
}

function read(string $path): string
{
    $contents = file_get_contents(dirname(__DIR__) . DIRECTORY_SEPARATOR . $path);
    if ($contents === false) {
        fwrite(STDERR, "artifact-policy: could not read {$path}\n");
        exit(1);
    }

    return $contents;
}

/** @param list<string> $failures */
function test_target_ownership(array &$failures): void
{
    $directory = sys_get_temp_dir()
        . DIRECTORY_SEPARATOR
        . 'doria-artifact-policy-'
        . bin2hex(random_bytes(6));
    $repository = $directory . DIRECTORY_SEPARATOR . 'repository';
    $otherRepository = $directory . DIRECTORY_SEPARATOR . 'other-repository';
    $shared = $directory . DIRECTORY_SEPARATOR . 'shared';
    $dedicated = $directory . DIRECTORY_SEPARATOR . 'dedicated';

    try {
        foreach ([$repository, $otherRepository, $shared, $dedicated] as $path) {
            if (!mkdir($path, 0777, true) && !is_dir($path)) {
                throw new RuntimeException("could not create test directory {$path}");
            }
        }
        file_put_contents($shared . DIRECTORY_SEPARATOR . 'unrelated-artifact', 'keep');

        expect_target_rejection(
            static fn (): string => doria_prepare_managed_target('', $repository, false),
            'must not be empty',
            $failures,
        );
        expect_target_rejection(
            static fn (): string => doria_prepare_managed_target('.', $repository, false),
            'repository root or an ancestor',
            $failures,
        );
        expect_target_rejection(
            static fn (): string => doria_prepare_managed_target($directory, $repository, false),
            'repository root or an ancestor',
            $failures,
        );
        expect_target_rejection(
            static fn (): string => doria_prepare_managed_target($shared, $repository, false),
            'nonempty and has no Doria ownership marker',
            $failures,
        );

        $default = doria_prepare_managed_target('target', $repository, true);
        $environment = doria_build_environment($default);
        $scratch = $default . DIRECTORY_SEPARATOR . 'tmp';
        if (!is_dir($scratch) || $environment['CARGO_TARGET_DIR'] !== $default) {
            $failures[] = 'build environment must retain the selected target and create its scratch directory';
        }
        foreach (['TMPDIR', 'TMP', 'TEMP'] as $variable) {
            if (($environment[$variable] ?? null) !== $scratch) {
                $failures[] = "{$variable} must follow the selected build volume";
            }
        }
        if (!is_file($default . DIRECTORY_SEPARATOR . DORIA_ARTIFACT_OWNER_FILE)) {
            $failures[] = 'the canonical repository target was not marked as owned';
        }
        $cacheTag = $default . DIRECTORY_SEPARATOR . 'CACHEDIR.TAG';
        if (file_get_contents($cacheTag) !== DORIA_CARGO_CACHE_TAG) {
            $failures[] = 'the managed target did not receive Cargo cache identity';
        }
        unlink($cacheTag);
        doria_prepare_managed_target($default, $repository, true);
        if (file_get_contents($cacheTag) !== DORIA_CARGO_CACHE_TAG) {
            $failures[] = 'an existing managed target did not recover missing Cargo cache identity';
        }
        doria_prepare_managed_target($dedicated, $repository, true);
        expect_target_rejection(
            static fn (): string => doria_prepare_managed_target(
                $dedicated,
                $otherRepository,
                false,
            ),
            'belongs to another repository',
            $failures,
        );

        if (PHP_OS_FAMILY !== 'Windows' && function_exists('symlink')) {
            $linkedTarget = $repository . DIRECTORY_SEPARATOR . 'target-link';
            if (symlink($shared, $linkedTarget)) {
                expect_target_rejection(
                    static fn (): string => doria_prepare_managed_target(
                        $linkedTarget,
                        $repository,
                        false,
                    ),
                    'nonempty and has no Doria ownership marker',
                    $failures,
                );
            }
        }
        check_reclaimed_cache_identity($directory . DIRECTORY_SEPARATOR . 'reclaimed-cache', $failures);
    } catch (Throwable $error) {
        $failures[] = 'artifact ownership self-test failed: ' . $error->getMessage();
    } finally {
        remove_test_tree($directory);
    }
}

/** @param list<string> $failures */
function check_reclaimed_cache_identity(string $target, array &$failures): void
{
    doria_prepare_managed_target($target, dirname(__DIR__), true);
    $run = static function (string $option) use ($target): string {
        $process = proc_open(
            [PHP_BINARY, __DIR__ . '/validate_work_unit.php', '--target-dir', $target, $option],
            [
                0 => ['file', PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null', 'r'],
                1 => ['pipe', 'w'],
                2 => ['pipe', 'w'],
            ],
            $pipes,
        );
        if (!is_resource($process)) {
            throw new RuntimeException('could not start cache-identity regression');
        }
        $output = stream_get_contents($pipes[1]);
        $error = stream_get_contents($pipes[2]);
        fclose($pipes[1]);
        fclose($pipes[2]);
        if (proc_close($process) !== 0) {
            throw new RuntimeException('cache-identity regression failed: ' . $error);
        }
        return (string) $output;
    };

    $run('--reclaim');
    $stamp = $target . DIRECTORY_SEPARATOR . '.doria-artifact-cache.json';
    $identity = json_decode((string) file_get_contents($stamp), true, 512, JSON_THROW_ON_ERROR);
    if (!preg_match('/^[0-9a-f]{64}$/', $identity['identity'] ?? '')) {
        $failures[] = 'reclaimed cache must retain its verified configuration identity';
    }
    if (!str_contains($run('--plan'), 'reuse the compatible managed cache')) {
        $failures[] = 'reclaimed compatible cache must not require another cold rebuild';
    }
    file_put_contents($stamp, json_encode(['identity' => 'changed'], JSON_THROW_ON_ERROR));
    if (!str_contains($run('--plan'), 'profile policy changed')) {
        $failures[] = 'retaining cache identity must still reject a changed configuration';
    }
}

/** @param list<string> $failures */
function expect_target_rejection(
    callable $operation,
    string $messageFragment,
    array &$failures,
): void {
    try {
        $operation();
        $failures[] = "unsafe target was accepted; expected {$messageFragment}";
    } catch (RuntimeException $error) {
        if (!str_contains($error->getMessage(), $messageFragment)) {
            $failures[] = "unexpected target rejection: {$error->getMessage()}";
        }
    }
}

function remove_test_tree(string $path): void
{
    if (is_link($path) || is_file($path)) {
        @unlink($path);
        return;
    }
    if (!is_dir($path)) {
        return;
    }

    $entries = scandir($path) ?: [];
    foreach ($entries as $entry) {
        if ($entry === '.' || $entry === '..') {
            continue;
        }
        remove_test_tree($path . DIRECTORY_SEPARATOR . $entry);
    }
    @rmdir($path);
}
