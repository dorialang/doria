<?php

declare(strict_types=1);

require_once __DIR__ . '/cargo_artifact_cleanup.php';

/** @param list<string> $paths */
function cleanup_fixture_dep_info(array $paths): string
{
    $bytes = "\x01\x00\x00\x00\xff\x01" . pack('V', count($paths));
    foreach ($paths as $path) {
        $bytes .= "\0" . pack('V', strlen($path)) . $path . "\0";
    }
    return $bytes . pack('V', 0);
}

/** @return string Path to a fixture executable. */
function cleanup_fixture_artifact(
    string $profile,
    string $name,
    string $hash,
    int $time,
    string $source = 'src/test input.rs',
    string $features = '[]',
    int $buildProfile = 1,
): string {
    $fingerprint = $profile . '/.fingerprint/fixture-crate-' . $hash;
    if (!is_dir($fingerprint)) {
        mkdir($fingerprint, 0700, true);
    }
    $depPath = $fingerprint . '/dep-' . $name;
    file_put_contents($depPath, cleanup_fixture_dep_info([$source]));
    touch($depPath, $time);
    $json = [
        'rustc' => '18446744073709551615', 'features' => $features, 'declared_features' => '["extra"]',
        'target' => $name, 'profile' => $buildProfile, 'path' => 1, 'deps' => [],
        'local' => [['CheckDepInfo' => ['dep_info' => 'debug/.fingerprint/fixture-crate-' . $hash . '/dep-' . $name, 'checksum' => false]]],
        'rustflags' => [], 'config' => 1, 'compile_kind' => 0,
    ];
    file_put_contents($fingerprint . '/' . $name . '.json', json_encode($json, JSON_THROW_ON_ERROR));
    file_put_contents($fingerprint . '/' . $name, '1234567890abcdef');
    touch($fingerprint . '/' . $name . '.json', $time + 1);
    touch($fingerprint . '/' . $name, $time + 1);
    preg_match('/^test-(?:integration-test|lib|bin)-(.+)$/D', $name, $target);
    $executable = $profile . '/deps/' . $target[1] . '-' . $hash;
    file_put_contents($executable, 'test fixture only');
    touch($executable, $time + 1);
    return $executable;
}

function cleanup_fixture_object(string $executable, int $time, int $index = 0): string
{
    $name = preg_replace('/-[a-f0-9]{16}$/D', '', basename($executable));
    $path = $executable . '.' . $name . '.0123456789abcdef-cgu.' . $index . '.rcgu.o';
    file_put_contents($path, 'owned object fixture');
    touch($path, $time);
    return $path;
}

/** @param list<string> $failures */
function test_stale_linker_object_cleanup(array &$failures): void
{
    $directory = sys_get_temp_dir() . '/doria-linker-cleanup-' . bin2hex(random_bytes(6));
    $open = null;
    try {
        mkdir($directory, 0700);
        $directory = realpath($directory);
        $repository = $directory . '/repository';
        $package = $repository . '/package';
        $target = $directory . '/cache';
        $profile = $target . '/debug';
        mkdir($package . '/src', 0700, true);
        mkdir($profile . '/deps', 0700, true);
        $now = time();
        file_put_contents($package . '/src/test input.rs', 'changed source');
        touch($package . '/src/test input.rs', $now - 10);
        file_put_contents($package . '/src/unchanged.rs', 'healthy source');
        touch($package . '/src/unchanged.rs', $now - 1000);
        file_put_contents($profile . '/.cargo-lock', '');
        file_put_contents($profile . '/deps/libhealthy.rlib', 'keep library');
        doria_write_artifact_owner($target, $repository);
        $metadata = ['version' => 1, 'workspace_members' => ['fixture'], 'packages' => [[
            'id' => 'fixture', 'name' => 'fixture-crate', 'manifest_path' => $package . '/Cargo.toml',
            'targets' => [['kind' => ['lib'], 'name' => 'fixture_crate']],
        ]]];
        $kind = 'test-lib-fixture_crate';
        $old = cleanup_fixture_artifact($profile, $kind, str_repeat('1', 16), $now - 100);
        $latest = cleanup_fixture_artifact($profile, $kind, str_repeat('2', 16), $now - 50);
        $oldObject = cleanup_fixture_object($old, $now - 101);
        $latestObject = cleanup_fixture_object($latest, $now - 51);
        unlink($old);
        $healthy = cleanup_fixture_artifact($profile, $kind, str_repeat('3', 16), $now - 50, 'src/unchanged.rs', '["healthy"]');
        $healthyObject = cleanup_fixture_object($healthy, $now - 51);
        $late = cleanup_fixture_artifact($profile, $kind, str_repeat('4', 16), $now - 50, features: '["partial"]');
        $lateObject = cleanup_fixture_object($late, $now - 1);
        $failed = cleanup_fixture_artifact($profile, $kind, str_repeat('5', 16), $now - 50, features: '["failed"]');
        $failedObject = cleanup_fixture_object($failed, $now - 51);
        file_put_contents($profile . '/.fingerprint/fixture-crate-' . str_repeat('5', 16) . '/' . $kind, '');
        $orphan = cleanup_fixture_artifact($profile, $kind, str_repeat('6', 16), $now - 50, features: '["orphan"]');
        $orphanObject = cleanup_fixture_object($orphan, $now - 51);
        unlink($orphan);
        $unknown = $latest . '.user-file.rcgu.o';
        file_put_contents($unknown, 'keep unknown');
        touch($unknown, $now - 51);
        $linkedObjects = [];
        if (PHP_OS_FAMILY !== 'Windows') {
            $symlink = $latest . '.fixture_crate.0123456789abcdef-cgu.1.rcgu.o';
            $hardlink = $latest . '.fixture_crate.0123456789abcdef-cgu.2.rcgu.o';
            symlink($healthyObject, $symlink);
            link($healthyObject, $hardlink);
            $linkedObjects = [$symlink, $hardlink];
        }
        $expect = static function (bool $condition, string $message) use (&$failures): void {
            if (!$condition) {
                $failures[] = 'linker cleanup: ' . $message;
            }
        };
        $plan = doria_obsolete_test_plan($target, $repository, $metadata);
        $expect(array_column($plan['candidates'], 'path') === [$oldObject, $latestObject], 'select stale linked objects, including superseded units without an executable');
        $expect(count($plan['proofs']) === 2, 'deduplicate compilation proof while retaining the linked executable');
        $expect(count($plan['preservedUnrecognized']) === 2 + count($linkedObjects), 'preserve failed completions, unknown names and linked objects');
        foreach ($plan['candidates'] as $candidate) {
            $expect($candidate['kind'] === DoriaTestOutputKind::StaleLinkedObject->value, 'retain every linked executable in this fixture');
        }
        $open = fopen($latestObject, 'r');
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']),
            'active-use check', $failures,
        );
        fclose($open);
        $open = null;
        $expect(is_file($oldObject) && is_file($latestObject), 'an active object must prevent all deletions');
        $completion = $profile . '/.fingerprint/fixture-crate-' . str_repeat('2', 16) . '/' . $kind;
        file_put_contents($completion, 'fedcba9876543210');
        touch($completion, $now - 49);
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']),
            'plan changed', $failures,
        );
        $plan = doria_obsolete_test_plan($target, $repository, $metadata);
        $result = doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']);
        $expect(count($result['removed']) === 2 && !is_file($oldObject) && !is_file($latestObject), 'remove exact reviewed stale objects');
        foreach ([$latest, $healthy, $healthyObject, $late, $lateObject, $failed, $failedObject, $orphanObject, $unknown, $profile . '/deps/libhealthy.rlib'] as $retained) {
            $expect(is_file($retained), 'preserve ' . basename($retained));
        }
        foreach ($linkedObjects as $retained) {
            $expect(is_file($retained), 'preserve linked object ' . basename($retained));
        }
        $expect(doria_obsolete_test_plan($target, $repository, $metadata)['candidates'] === [], 'object cleanup is idempotent');
        $expected = 'debug/.fingerprint/unit/dep-test';
        $record = ['local' => [['CheckDepInfo' => ['checksum' => false, 'dep_info' => str_replace('/', '\\', $expected)]]]];
        $expect(doria_mtime_fingerprint_record($record, $expected), 'recognize platform path separators and JSON field order');
        $record['local'][0]['CheckDepInfo']['checksum'] = true;
        $expect(!doria_mtime_fingerprint_record($record, $expected), 'preserve checksum freshness records');
        $record['local'][0]['CheckDepInfo']['checksum'] = false;
        $record['local'][0]['CheckDepInfo']['dep_info'] = '../' . $expected;
        $expect(!doria_mtime_fingerprint_record($record, $expected), 'reject noncanonical dependency paths');
    } catch (Throwable $error) {
        $failures[] = 'linker cleanup self-test failed: ' . $error->getMessage();
    } finally {
        if (is_resource($open)) {
            fclose($open);
        }
        remove_test_tree($directory);
    }
}

/** @param list<string> $failures */
function test_interrupted_test_cleanup(array &$failures): void
{
    $directory = sys_get_temp_dir() . '/doria-interrupted-cleanup-' . bin2hex(random_bytes(6));
    $open = null;
    $lock = null;
    try {
        mkdir($directory, 0700);
        $directory = realpath($directory);
        $repository = $directory . '/repository';
        $package = $repository . '/package';
        $target = $directory . '/cache';
        $profile = $target . '/debug';
        mkdir($package . '/src', 0700, true);
        mkdir($profile . '/deps', 0700, true);
        file_put_contents($package . '/src/test input.rs', 'source stays');
        file_put_contents($profile . '/.cargo-lock', '');
        doria_write_artifact_owner($target, $repository);
        $metadata = ['version' => 1, 'workspace_members' => ['fixture'], 'packages' => [[
            'id' => 'fixture', 'name' => 'fixture-crate', 'manifest_path' => $package . '/Cargo.toml',
            'targets' => [['kind' => ['lib'], 'name' => 'fixture_crate']],
        ]]];
        $now = time();
        $hash = str_repeat('1', 16);
        $kind = 'test-lib-fixture_crate';
        $executable = cleanup_fixture_artifact($profile, $kind, $hash, $now - 100);
        $fingerprint = $profile . '/.fingerprint/fixture-crate-' . $hash;
        $completion = $fingerprint . '/' . $kind;
        $invoked = $fingerprint . '/invoked.timestamp';
        file_put_contents($completion, '');
        touch($completion, $now - 30);
        file_put_contents($invoked, 'This file has an mtime of when this was started.');
        touch($invoked, $now - 29);
        $object = cleanup_fixture_object($executable, $now - 20);
        $oldObject = cleanup_fixture_object($executable, $now - 101, 1);
        $unknown = $executable . '.user-file.rcgu.o';
        file_put_contents($unknown, 'unrecognized file stays');
        $healthy = cleanup_fixture_artifact($profile, $kind, str_repeat('2', 16), $now - 100);
        $healthyObject = cleanup_fixture_object($healthy, $now - 101);
        $preserved = [$executable, $oldObject, $unknown, $healthy, $healthyObject,
            $completion, $invoked, $fingerprint . '/dep-' . $kind, $fingerprint . '/' . $kind . '.json',
            $package . '/src/test input.rs'];
        $hashes = array_map(static fn (string $path): string => hash_file('sha256', $path), $preserved);
        $selection = DoriaTestCleanupMode::Interrupted;
        $expect = static function (bool $condition, string $message) use (&$failures): void {
            if (!$condition) {
                $failures[] = 'interrupted cleanup: ' . $message;
            }
        };
        $plan = doria_obsolete_test_plan($target, $repository, $metadata, $selection);
        $expect(array_column($plan['candidates'], 'path') === [$object], 'select only proven interrupted-unit objects');
        $expect($plan['candidates'][0]['kind'] === DoriaTestOutputKind::InterruptedObject->value, 'explicit output classification');
        $expect($plan === doria_obsolete_test_plan($target, $repository, $metadata, $selection), 'read-only deterministic planning');
        $expect(!in_array($object, array_column(doria_obsolete_test_plan($target, $repository, $metadata)['candidates'], 'path'), true), 'ordinary cleanup does not recover interrupted work');
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']),
            'plan changed', $failures,
        );
        $lock = fopen($profile . '/.cargo-lock', 'r+');
        flock($lock, LOCK_EX);
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId'], $selection),
            'target is active', $failures,
        );
        flock($lock, LOCK_UN);
        fclose($lock);
        $lock = null;
        foreach ([$object, $executable] as $active) {
            $open = fopen($active, 'r');
            expect_target_rejection(
                static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId'], $selection),
                'active-use check', $failures,
            );
            fclose($open);
            $open = null;
            $expect(is_file($object), 'active users prevent deletion');
        }
        foreach (['not a completion marker', '1234567890abcdef'] as $marker) {
            file_put_contents($completion, $marker);
            $expect(doria_obsolete_test_plan($target, $repository, $metadata, $selection)['candidates'] === [], 'preserve malformed or completed units');
        }
        file_put_contents($completion, '');
        touch($completion, $now - 30);
        foreach (['unrecognized invocation', 'This file has an mtime of when this was started.'] as $content) {
            file_put_contents($invoked, $content);
            touch($invoked, $now - 200);
            $expect(doria_obsolete_test_plan($target, $repository, $metadata, $selection)['candidates'] === [], 'reject unknown or stale invocation proof');
        }
        touch($invoked, $now - 29);
        $expect(doria_obsolete_test_plan($target, $repository, ['version' => 1, 'packages' => [], 'workspace_members' => []], $selection)['candidates'] === [], 'require current workspace ownership');
        $saved = $executable . '.saved';
        rename($executable, $saved);
        $expect(doria_obsolete_test_plan($target, $repository, $metadata, $selection)['candidates'] === [], 'retain outputs without executable evidence');
        rename($saved, $executable);
        $dep = $fingerprint . '/dep-' . $kind;
        $depBytes = file_get_contents($dep);
        file_put_contents($dep, 'unknown dependency format');
        $expect(doria_obsolete_test_plan($target, $repository, $metadata, $selection)['candidates'] === [], 'require versioned dependency evidence');
        file_put_contents($dep, $depBytes);
        touch($dep, $now - 100);
        if (PHP_OS_FAMILY !== 'Windows') {
            $alias = $directory . '/shared-object';
            link($object, $alias);
            $expect(doria_obsolete_test_plan($target, $repository, $metadata, $selection)['candidates'] === [], 'preserve multiply linked outputs');
            unlink($alias);
            rename($object, $alias);
            symlink($alias, $object);
            $expect(doria_obsolete_test_plan($target, $repository, $metadata, $selection)['candidates'] === [], 'preserve symbolic links');
            unlink($object);
            rename($alias, $object);
        }
        $plan = doria_obsolete_test_plan($target, $repository, $metadata, $selection);
        file_put_contents($object, 'changed output identity');
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId'], $selection),
            'plan changed', $failures,
        );
        $plan = doria_obsolete_test_plan($target, $repository, $metadata, $selection);
        $result = doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId'], $selection);
        $expect($result['removed'] === [$object] && !file_exists($object), 'remove only reviewed interrupted output');
        $expect($hashes === array_map(static fn (string $path): string => hash_file('sha256', $path), $preserved), 'preserve source, evidence and healthy artifacts byte-for-byte');
        $expect(doria_obsolete_test_plan($target, $repository, $metadata, $selection)['candidates'] === [], 'idempotent recovery');
    } catch (Throwable $error) {
        $failures[] = 'interrupted cleanup self-test failed: ' . $error->getMessage();
    } finally {
        if (is_resource($open)) {
            fclose($open);
        }
        if (is_resource($lock)) {
            flock($lock, LOCK_UN);
            fclose($lock);
        }
        remove_test_tree($directory);
    }
}

function cleanup_fixture_cargo_lock(string $target): void
{
    $lock = fopen($target . '/debug/.cargo-lock', 'r+');
    if ($lock === false || !flock($lock, LOCK_EX | LOCK_NB)) {
        throw new RuntimeException('could not hold fixture Cargo lock');
    }
    $process = null;
    try {
        $process = proc_open(
            ['cargo', 'clean', '--package', 'doriac', '--dry-run', '--verbose', '--offline', '--locked', '--target-dir', $target],
            [0 => ['file', PHP_OS_FAMILY === 'Windows' ? 'NUL' : '/dev/null', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']],
            $pipes,
            dirname(__DIR__),
        );
        if (!is_resource($process)) {
            throw new RuntimeException('could not verify Cargo lock interoperability');
        }
        stream_set_blocking($pipes[1], false);
        stream_set_blocking($pipes[2], false);
        $output = '';
        $deadline = microtime(true) + 3;
        do {
            $output .= stream_get_contents($pipes[1]) . stream_get_contents($pipes[2]);
            $running = proc_get_status($process)['running'];
            if (!$running || str_contains($output, 'file lock')) {
                break;
            }
            usleep(20_000);
        } while (microtime(true) < $deadline);
        if (!$running || !str_contains($output, 'file lock')) {
            throw new RuntimeException('Cargo did not honor the held profile lock: ' . $output);
        }
        flock($lock, LOCK_UN);
        $deadline = microtime(true) + 3;
        do {
            $output .= stream_get_contents($pipes[1]) . stream_get_contents($pipes[2]);
            $status = proc_get_status($process);
            if (!$status['running']) {
                break;
            }
            usleep(20_000);
        } while (microtime(true) < $deadline);
        if ($status['running'] || $status['exitcode'] !== 0) {
            throw new RuntimeException('Cargo dry-run did not finish after releasing its profile lock: ' . $output);
        }
    } finally {
        flock($lock, LOCK_UN);
        fclose($lock);
        if (is_resource($process)) {
            if (proc_get_status($process)['running']) {
                proc_terminate($process);
            }
            fclose($pipes[1]);
            fclose($pipes[2]);
            proc_close($process);
        }
    }
}

/** @param list<string> $failures */
function test_selective_artifact_cleanup(array &$failures): void
{
    $directory = sys_get_temp_dir() . '/doria-selective-cleanup-' . bin2hex(random_bytes(6));
    $open = null;
    $lock = null;
    try {
        mkdir($directory, 0700);
        $directory = realpath($directory);
        $repository = $directory . '/repository';
        $package = $repository . '/package';
        $target = $directory . '/cache with spaces';
        mkdir($package . '/src', 0700, true);
        mkdir($target . '/debug/deps', 0700, true);
        $source = $package . '/src/test input.rs';
        $now = time();
        file_put_contents($source, 'source fixture');
        touch($source, $now - 10);
        file_put_contents($package . '/src/unchanged.rs', 'unchanged fixture');
        touch($package . '/src/unchanged.rs', $now - 1000);
        doria_write_artifact_owner($target, $repository);
        file_put_contents($target . '/debug/.cargo-lock', '');
        file_put_contents($target . '/debug/deps/libhealthy.rlib', 'keep library');
        file_put_contents($target . '/debug/deps/unrecognized-output', 'keep unknown');
        $metadata = ['version' => 1, 'workspace_members' => ['fixture'], 'packages' => [[
            'id' => 'fixture', 'name' => 'fixture-crate', 'manifest_path' => $package . '/Cargo.toml',
            'targets' => [
                ['kind' => ['lib'], 'name' => 'fixture_crate'],
                ['kind' => ['bin'], 'name' => 'command'],
                ['kind' => ['test'], 'name' => 'integration_case'],
            ],
        ]]];
        $old = [];
        $latest = [];
        foreach (['test-lib-fixture_crate', 'test-bin-command', 'test-integration-test-integration_case'] as $index => $name) {
            $old[] = cleanup_fixture_artifact($target . '/debug', $name, str_repeat((string) ($index + 1), 16), $now - 100);
            $latest[] = cleanup_fixture_artifact($target . '/debug', $name, str_repeat((string) ($index + 4), 16), $now - 50);
        }
        $healthy = cleanup_fixture_artifact($target . '/debug', 'test-lib-fixture_crate', str_repeat('a', 16), $now - 200, 'src/unchanged.rs');
        $differentFeatures = cleanup_fixture_artifact($target . '/debug', 'test-lib-fixture_crate', str_repeat('b', 16), $now - 200, features: '["extra"]');
        $differentProfile = cleanup_fixture_artifact($target . '/debug', 'test-lib-fixture_crate', str_repeat('c', 16), $now - 200, buildProfile: 2);
        $plan = doria_obsolete_test_plan($target, $repository, $metadata);
        $expect = static function (bool $condition, string $message) use (&$failures): void {
            if (!$condition) {
                $failures[] = 'selective cleanup: ' . $message;
            }
        };
        $expect(count($plan['candidates']) === 3, 'select exactly stale superseded lib/bin/integration test executables');
        $expect(doria_obsolete_test_plan($target, $repository, $metadata) === $plan, 'dry-run must be deterministic and read-only');
        $expect(is_file($healthy) && is_file($differentFeatures) && is_file($differentProfile), 'retain healthy and alternate-feature/profile artifacts');
        expect_target_rejection(
            static fn (): array => doria_obsolete_test_plan($target, $repository, []),
            'workspace metadata', $failures,
        );
        cleanup_fixture_cargo_lock($target);
        $expect(count(array_filter($old, 'is_file')) === 3, 'Cargo interoperability probe must remain a dry-run');
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, str_repeat('0', 64)),
            'plan changed', $failures,
        );
        $expect(count(array_filter($old, 'is_file')) === 3, 'wrong plan must remove nothing');
        $lock = fopen($target . '/debug/.cargo-lock', 'r+');
        flock($lock, LOCK_EX | LOCK_NB);
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']),
            'target is active', $failures,
        );
        flock($lock, LOCK_UN);
        fclose($lock);
        $lock = null;
        $open = fopen($old[0], 'r');
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']),
            'active-use check', $failures,
        );
        fclose($open);
        $open = null;
        $expect(count(array_filter($old, 'is_file')) === 3, 'active consumer must remove nothing');
        if (PHP_OS_FAMILY !== 'Windows') {
            $path = getenv('PATH');
            try {
                putenv('PATH=' . $directory);
                expect_target_rejection(
                    static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']),
                    'active-use check', $failures,
                );
            } finally {
                $path === false ? putenv('PATH') : putenv('PATH=' . $path);
            }
            $expect(count(array_filter($old, 'is_file')) === 3, 'unavailable active-use probe must remove nothing');
        }
        touch($source, $now - 5);
        expect_target_rejection(
            static fn (): array => doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']),
            'plan changed', $failures,
        );
        $plan = doria_obsolete_test_plan($target, $repository, $metadata);
        $result = doria_reclaim_obsolete_tests($target, $repository, $metadata, $plan['planId']);
        $expect(count($result['removed']) === 3 && count(array_filter($old, 'is_file')) === 0, 'apply removes only reviewed files');
        $expect(count(array_filter($latest, 'is_file')) === 3 && is_file($healthy) && is_file($differentFeatures) && is_file($differentProfile), 'retain latest and reusable artifacts after apply');
        $expect(is_file($target . '/debug/deps/libhealthy.rlib') && is_file($target . '/debug/deps/unrecognized-output'), 'preserve libraries and unrelated files');
        $expect(doria_obsolete_test_plan($target, $repository, $metadata)['candidates'] === [], 'apply must be idempotent');
        $paths = ['src/a file.rs', 'src/back\\slash.rs'];
        $expect(doria_mtime_dep_info(cleanup_fixture_dep_info($paths)) === $paths, 'parse paths without shell/Makefile escaping');
        foreach (['', "\x01\0\0\0\xff\x02", substr(cleanup_fixture_dep_info($paths), 0, -1), cleanup_fixture_dep_info([]) . 'trailing'] as $bytes) {
            expect_target_rejection(static fn (): array => doria_mtime_dep_info($bytes), 'Cargo', $failures);
        }
        if (PHP_OS_FAMILY !== 'Windows') {
            $linked = $target . '/debug/deps/test-link';
            symlink($healthy, $linked);
            expect_target_rejection(static fn (): array => doria_cleanup_file_identity($linked), 'regular build file', $failures);
            unlink($linked);
            link($healthy, $linked);
            expect_target_rejection(static fn (): array => doria_cleanup_file_identity($healthy), 'regular build file', $failures);
            unlink($linked);
        }
        $expect(file_get_contents($source) === 'source fixture', 'source content stays unchanged');
    } catch (Throwable $error) {
        $failures[] = 'selective cleanup self-test failed: ' . $error->getMessage();
    } finally {
        if (is_resource($open)) {
            fclose($open);
        }
        if (is_resource($lock)) {
            flock($lock, LOCK_UN);
            fclose($lock);
        }
        remove_test_tree($directory);
    }
}
