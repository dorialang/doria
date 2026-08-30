#!/usr/bin/env php
<?php

declare(strict_types=1);

require_once __DIR__ . '/cargo_artifacts.php';

$root = dirname(__DIR__);
$failures = [];

$cargo = read('Cargo.toml');
$agents = read('AGENTS.md');
$contributing = read('CONTRIBUTING.md');
$validator = read('scripts/validate_work_unit.php');
$refresh = read('scripts/refresh_development_toolchain.php');
$runtimeReproducibility = read('crates/doriac/tests/runtime_reproducibility_tests.rs');
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
    "'CARGO_TARGET_DIR'" => $validator,
    "getenv('DORIA_VALIDATION_TARGET_DIR')" => $refresh,
    "'toolchain-install'" => $refresh,
    'struct ScratchDirectory' => $runtimeReproducibility,
    'remove_dir_all(&self.path)' => $runtimeReproducibility,
    'php scripts/check_build_artifact_policy.php' => $ci,
] as $needle => $contents) {
    if (!str_contains($contents, $needle)) {
        $failures[] = "missing required artifact-policy marker: {$needle}";
    }
}

if (substr_count($cargo, 'debug = false') < 2) {
    $failures[] = 'Cargo.toml must disable dependency debug metadata in dev and test profiles';
}
if (str_contains($validator, "'rm'")) {
    $failures[] = 'the managed validator must reclaim through Cargo, not recursive deletion';
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

test_target_ownership($failures);

if ($failures !== []) {
    foreach ($failures as $failure) {
        fwrite(STDERR, "artifact-policy: {$failure}\n");
    }
    exit(1);
}

fwrite(STDOUT, "Build artifact policy guard passed.\n");

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
    } catch (Throwable $error) {
        $failures[] = 'artifact ownership self-test failed: ' . $error->getMessage();
    } finally {
        remove_test_tree($directory);
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
