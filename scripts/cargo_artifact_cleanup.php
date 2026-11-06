<?php

declare(strict_types=1);

require_once __DIR__ . '/cargo_artifacts.php';

enum DoriaTestArtifactKind: string
{
    case Library = 'test-lib';
    case Binary = 'test-bin';
    case Integration = 'test-integration-test';

    public function matches(array $target): bool
    {
        $kind = $target['kind'] ?? [];
        return match ($this) {
            self::Library => in_array('lib', $kind, true),
            self::Binary => in_array('bin', $kind, true),
            self::Integration => in_array('test', $kind, true),
        };
    }
}

enum DoriaTestOutputKind: string
{
    case SupersededExecutable = 'superseded-test-executable';
    case StaleLinkedObject = 'stale-test-linker-object';
    case InterruptedObject = 'interrupted-test-linker-object';
}

enum DoriaTestCleanupMode: string
{
    case Obsolete = 'obsolete';
    case Interrupted = 'interrupted';
}

function doria_mtime_fingerprint_record(array $fingerprint, string $expected): bool
{
    $local = $fingerprint['local'] ?? null;
    if (!is_array($local) || count($local) !== 1 || !isset($local[0]['CheckDepInfo'])
        || count($local[0]) !== 1) {
        return false;
    }
    $record = $local[0]['CheckDepInfo'];
    return is_array($record) && count($record) === 2 && ($record['checksum'] ?? null) === false
        && is_string($record['dep_info'] ?? null)
        && doria_paths_equal($record['dep_info'], $expected);
}

/** Read Cargo's versioned binary format, never rendered compiler output or Makefile text.
 *  https://doc.rust-lang.org/stable/nightly-rustc/src/cargo/compiler/fingerprint/dep_info.rs.html
 *  @return list<string> Package-relative paths; checksum freshness is deliberately unsupported.
 */
function doria_mtime_dep_info(string $bytes): array
{
    $offset = 0;
    $read = static function (int $length) use ($bytes, &$offset): string {
        if ($length < 0 || $length > strlen($bytes) - $offset) {
            throw new RuntimeException('truncated Cargo dependency record');
        }
        $value = substr($bytes, $offset, $length);
        $offset += $length;
        return $value;
    };
    $number = static fn (): int => unpack('V', $read(4))[1];
    $string = static fn (): string => $read($number());
    if ($read(6) !== "\x01\x00\x00\x00\xff\x01") {
        throw new RuntimeException('unsupported Cargo dependency record version');
    }
    $paths = [];
    $count = $number();
    if ($count > strlen($bytes)) {
        throw new RuntimeException('invalid Cargo dependency count');
    }
    for ($index = 0; $index < $count; ++$index) {
        $kind = ord($read(1));
        $path = $string();
        if ($kind > 1 || $path === '' || str_contains($path, "\0")) {
            throw new RuntimeException('invalid Cargo dependency path');
        }
        if ($read(1) !== "\0") {
            throw new RuntimeException('checksum-based dependency records are preserved');
        }
        if ($kind === 0) {
            $paths[] = $path;
        }
    }
    $count = $number();
    if ($count > strlen($bytes)) {
        throw new RuntimeException('invalid Cargo environment count');
    }
    for ($index = 0; $index < $count; ++$index) {
        $string();
        $present = ord($read(1));
        if ($present > 1) {
            throw new RuntimeException('invalid Cargo environment record');
        }
        if ($present === 1) {
            $string();
        }
    }
    if ($offset !== strlen($bytes)) {
        throw new RuntimeException('unrecognized trailing Cargo dependency data');
    }
    return $paths;
}

/** @return array<string, int> */
function doria_cleanup_file_identity(string $path): array
{
    clearstatcache(true, $path);
    $stat = @lstat($path);
    if ($stat === false || ($stat['mode'] & 0170000) !== 0100000 || $stat['nlink'] !== 1) {
        throw new RuntimeException("not an exclusive regular build file: {$path}");
    }
    if (!doria_paths_equal(realpath($path) ?: '', $path)) {
        throw new RuntimeException("build file traverses a symbolic link: {$path}");
    }
    return array_intersect_key($stat, array_flip(['dev', 'ino', 'mode', 'nlink', 'size', 'mtime', 'ctime', 'blocks']));
}

function doria_cleanup_owned_target(string $target, string $repository): string
{
    $target = doria_prepare_managed_target($target, $repository, false);
    $ownerPath = $target . DIRECTORY_SEPARATOR . DORIA_ARTIFACT_OWNER_FILE;
    doria_cleanup_file_identity($ownerPath);
    $owner = json_decode((string) file_get_contents($ownerPath), true, 512, JSON_THROW_ON_ERROR);
    if (($owner['schemaVersion'] ?? null) !== 1) {
        throw new RuntimeException('selective cleanup requires an existing version-1 ownership marker');
    }
    return $target;
}

/** @return list<string> */
function doria_cleanup_profiles(string $target): array
{
    $profiles = [];
    $entries = new RecursiveIteratorIterator(
        new RecursiveDirectoryIterator($target, FilesystemIterator::SKIP_DOTS),
        RecursiveIteratorIterator::SELF_FIRST,
    );
    foreach ($entries as $entry) {
        if (!$entry->isLink() && $entry->isDir() && $entry->getFilename() === '.fingerprint') {
            $profile = dirname($entry->getPathname());
            if (doria_paths_equal(realpath($profile) ?: '', $profile)) {
                $profiles[] = $profile;
            }
        }
    }
    sort($profiles);
    return $profiles;
}

/** @return array<string, mixed> */
function doria_obsolete_test_plan(
    string $target,
    string $repository,
    array $metadata,
    DoriaTestCleanupMode $selection = DoriaTestCleanupMode::Obsolete,
): array
{
    if (($metadata['version'] ?? null) !== 1 || !is_array($metadata['packages'] ?? null)
        || !is_array($metadata['workspace_members'] ?? null)) {
        throw new RuntimeException('unsupported Cargo workspace metadata');
    }
    $repository = realpath($repository) ?: throw new RuntimeException('repository is unavailable');
    $target = doria_cleanup_owned_target($target, $repository);
    $packages = [];
    foreach ($metadata['packages'] as $package) {
        if (!in_array($package['id'], $metadata['workspace_members'], true)) {
            continue;
        }
        $packageRoot = realpath(dirname($package['manifest_path']));
        if ($packageRoot !== false && (doria_paths_equal($repository, $packageRoot)
            || doria_path_contains($repository, $packageRoot))) {
            $packages[$package['name']] = ['root' => $packageRoot, 'targets' => $package['targets']];
        }
    }
    $groups = [];
    $preserved = [];
    $profiles = doria_cleanup_profiles($target);
    foreach ($profiles as $profile) {
        foreach (glob($profile . '/.fingerprint/*/test-*.json') ?: [] as $jsonPath) {
            try {
                if (!preg_match('/^(.*)-([a-f0-9]{16})$/D', basename(dirname($jsonPath)), $unit)
                    || !isset($packages[$unit[1]])
                    || !preg_match('/^(test-lib|test-bin|test-integration-test)-(.+)\.json$/D', basename($jsonPath), $name)) {
                    continue;
                }
                $kind = DoriaTestArtifactKind::from($name[1]);
                $package = $packages[$unit[1]];
                $targets = array_filter($package['targets'], static fn (array $t): bool => $kind->matches($t)
                    && str_replace('-', '_', $t['name']) === $name[2]);
                if (count($targets) !== 1) {
                    continue;
                }
                $stem = $profile . '/deps/' . $name[2] . '-' . $unit[2];
                $objects = glob($stem . '.*.rcgu.o') ?: [];
                $executable = $stem;
                if (!is_file($executable) && is_file($executable . '.exe')) {
                    $executable .= '.exe';
                }
                if (!file_exists($executable) && $objects === []) {
                    continue;
                }
                $file = file_exists($executable) ? doria_cleanup_file_identity($executable) : null;
                $jsonFile = doria_cleanup_file_identity($jsonPath);
                $jsonBytes = (string) file_get_contents($jsonPath);
                $fingerprint = json_decode($jsonBytes, true, 512, JSON_THROW_ON_ERROR | JSON_BIGINT_AS_STRING);
                $completionPath = substr($jsonPath, 0, -5);
                $completionFile = doria_cleanup_file_identity($completionPath);
                $completionBytes = (string) file_get_contents($completionPath);
                $completed = preg_match('/^[a-f0-9]{16}$/D', $completionBytes) === 1;
                $invocation = null;
                if ($selection === DoriaTestCleanupMode::Interrupted) {
                    if ($completed) {
                        continue;
                    }
                    if ($completionBytes !== '' || $file === null) {
                        throw new RuntimeException('interrupted recovery requires an empty completion marker and retained executable');
                    }
                    $invokedPath = dirname($jsonPath) . '/invoked.timestamp';
                    $invokedFile = doria_cleanup_file_identity($invokedPath);
                    $invokedBytes = (string) file_get_contents($invokedPath);
                    if ($invokedBytes !== 'This file has an mtime of when this was started.'
                        || $completionFile['mtime'] <= max($file['mtime'], $jsonFile['mtime'])
                        || $invokedFile['mtime'] < $completionFile['mtime']) {
                        throw new RuntimeException('unrecognized interrupted Cargo invocation');
                    }
                    $invocation = ['path' => $invokedPath, 'file' => $invokedFile, 'sha256' => hash('sha256', $invokedBytes)];
                } elseif (!$completed
                    || ($file !== null && ($completionFile['mtime'] < $file['mtime'] || $jsonFile['mtime'] < $file['mtime']))) {
                    throw new RuntimeException('test compilation has no completed link proof');
                }
                $depPath = dirname($jsonPath) . '/dep-' . basename($jsonPath, '.json');
                $expectedDep = basename($profile) . '/.fingerprint/' . basename(dirname($jsonPath)) . '/' . basename($depPath);
                if (!doria_mtime_fingerprint_record($fingerprint, $expectedDep)) {
                    throw new RuntimeException('unrecognized mtime fingerprint layout');
                }
                $depFile = doria_cleanup_file_identity($depPath);
                $depBytes = (string) file_get_contents($depPath);
                $paths = doria_mtime_dep_info($depBytes);
                if ($invocation !== null && $depFile['mtime'] >= $completionFile['mtime']) {
                    throw new RuntimeException('interrupted unit lacks prior completed dependency evidence');
                }
                $stale = null;
                foreach ($paths as $path) {
                    $source = realpath($package['root'] . '/' . $path);
                    if ($source === false || !doria_path_contains($repository, $source)
                        || doria_path_contains($target, $source) || !is_file($source)) {
                        continue;
                    }
                    $sourceTime = filemtime($source);
                    $linkedTime = $file['mtime'] ?? $completionFile['mtime'];
                    if ($sourceTime !== false && $sourceTime > $depFile['mtime'] && $sourceTime > $linkedTime
                        && ($stale === null || $sourceTime > $stale['mtime'])) {
                        $stale = ['path' => $source, 'mtime' => $sourceTime];
                    }
                }
                $configuration = [];
                foreach (['rustc', 'features', 'declared_features', 'target', 'profile', 'path', 'rustflags', 'config', 'compile_kind'] as $field) {
                    if (!array_key_exists($field, $fingerprint)) {
                        throw new RuntimeException("missing fingerprint field {$field}");
                    }
                    $configuration[$field] = $fingerprint[$field];
                }
                $group = hash('sha256', json_encode([$profile, $unit[1], $name[0], $configuration], JSON_THROW_ON_ERROR));
                $groups[$group][] = [
                    'id' => hash('sha256', $jsonPath),
                    'path' => $executable, 'file' => $file, 'profile' => $profile,
                    'objectNamePattern' => '/^' . preg_quote(basename($stem) . '.' . $name[2], '/')
                        . '\.[a-f0-9]{16}-cgu\.[0-9]+\.rcgu\.o$/D',
                    'fingerprint' => ['path' => $jsonPath, 'file' => $jsonFile, 'sha256' => hash('sha256', $jsonBytes)],
                    'completion' => ['path' => $completionPath, 'file' => $completionFile, 'sha256' => hash('sha256', $completionBytes)],
                    'depInfo' => ['path' => $depPath, 'file' => $depFile, 'sha256' => hash('sha256', $depBytes)],
                    'invocation' => $invocation,
                    'staleInput' => $stale, 'configuration' => $configuration,
                    'objects' => $objects,
                ];
            } catch (RuntimeException | JsonException $error) {
                $preserved[] = ['path' => $jsonPath, 'reason' => $error->getMessage()];
            }
        }
    }
    $candidates = [];
    $proofs = [];
    foreach ($groups as $artifacts) {
        usort($artifacts, static fn (array $a, array $b): int => $b['depInfo']['file']['mtime'] <=> $a['depInfo']['file']['mtime']);
        $linked = array_values(array_filter($artifacts, static fn (array $a): bool => $a['file'] !== null));
        if ($linked === []) {
            continue;
        }
        $successor = $linked[0];
        foreach ($artifacts as $artifact) {
            $interrupted = $artifact['invocation'] !== null;
            if (!$interrupted && $artifact['staleInput'] === null) {
                continue;
            }
            $superseded = !$interrupted && $artifact['depInfo']['file']['mtime'] < $successor['depInfo']['file']['mtime'];
            if ($artifact['file'] === null && !$superseded) {
                continue;
            }
            $retained = $superseded ? $successor : $artifact;
            foreach ([$artifact, $retained] as $proof) {
                unset($proof['objects']);
                $proofs[$proof['id']] = $proof;
            }
            if ($superseded && $artifact['file'] !== null && $artifact['file']['mtime'] < $successor['file']['mtime']) {
                $candidates[] = [
                    'kind' => DoriaTestOutputKind::SupersededExecutable->value,
                    'path' => $artifact['path'], 'file' => $artifact['file'],
                    'compilation' => $artifact['id'], 'retainedBy' => $retained['id'],
                ];
            }
            // Only exact test-unit linker intermediates qualify. Incremental
            // state, libraries, dependency records and retained binaries stay.
            $linkedTime = $artifact['file']['mtime'] ?? $artifact['completion']['file']['mtime'];
            foreach ($artifact['objects'] as $object) {
                try {
                    if (!preg_match($artifact['objectNamePattern'], basename($object))) {
                        throw new RuntimeException('unrecognized test linker object name');
                    }
                    $file = doria_cleanup_file_identity($object);
                    if ($interrupted) {
                        $started = $artifact['invocation']['file']['mtime'];
                        if ($file['mtime'] < $started || $file['ctime'] < $started) {
                            continue;
                        }
                    } elseif ($file['mtime'] > $linkedTime || $file['mtime'] > $artifact['completion']['file']['mtime']
                        || $file['mtime'] >= $artifact['staleInput']['mtime']) {
                        continue;
                    }
                    $candidates[] = [
                        'kind' => ($interrupted ? DoriaTestOutputKind::InterruptedObject : DoriaTestOutputKind::StaleLinkedObject)->value,
                        'path' => $object, 'file' => $file,
                        'compilation' => $artifact['id'], 'retainedBy' => $retained['id'],
                    ];
                } catch (RuntimeException $error) {
                    $preserved[] = ['path' => $object, 'reason' => $error->getMessage()];
                }
            }
        }
    }
    $referenced = [];
    foreach ($candidates as $candidate) {
        $referenced[$candidate['compilation']] = true;
        $referenced[$candidate['retainedBy']] = true;
    }
    $proofs = array_intersect_key($proofs, $referenced);
    ksort($proofs);
    usort($candidates, static fn (array $a, array $b): int => strcmp($a['path'], $b['path']));
    $plan = [
        'schemaVersion' => 3, 'mode' => 'dry-run', 'selection' => $selection->value,
        'policySha256' => hash_file('sha256', __FILE__),
        'repository' => $repository, 'target' => $target,
        'profiles' => $profiles, 'proofs' => $proofs,
        'candidates' => $candidates, 'preservedUnrecognized' => $preserved,
        'allocatedCandidateBytes' => array_sum(array_map(static fn (array $a): int => isset($a['file']['blocks'])
            ? $a['file']['blocks'] * 512 : $a['file']['size'], $candidates)),
    ];
    $plan['planId'] = hash('sha256', json_encode($plan, JSON_THROW_ON_ERROR));
    return $plan;
}

/** @return array{status: int, stdout: string, stderr: string} */
function doria_cleanup_probe(array $command, string $input = ''): array
{
    $process = @proc_open($command, [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
    if (!is_resource($process)) {
        throw new RuntimeException('could not start active-use check');
    }
    fwrite($pipes[0], $input);
    fclose($pipes[0]);
    $stdout = stream_get_contents($pipes[1]);
    $stderr = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    return ['status' => proc_close($process), 'stdout' => (string) $stdout, 'stderr' => (string) $stderr];
}

function doria_assert_tests_unused(array $paths): void
{
    foreach (array_chunk(array_values(array_unique($paths)), 32) as $batch) {
        if (PHP_OS_FAMILY === 'Windows') {
            // Exclusive sharing fails for running or otherwise open outputs.
            $result = doria_cleanup_probe(['powershell', '-NoProfile', '-NonInteractive', '-Command',
                '$ErrorActionPreference="Stop"; $files=@(); try { foreach ($p in (ConvertFrom-Json ([Console]::In.ReadToEnd()))) '
                . '{ $files += [IO.File]::Open($p, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) } '
                . '} finally { foreach ($f in $files) { $f.Dispose() } }'], json_encode($batch, JSON_THROW_ON_ERROR));
            $unused = $result['status'] === 0;
        } else {
            $result = doria_cleanup_probe(['lsof', '-nP', '-F0pfn', '--', ...$batch]);
            $unused = $result['status'] === 1;
        }
        if (!$unused || $result['stdout'] !== '' || $result['stderr'] !== '') {
            throw new RuntimeException('active-use check failed or found an open build output; no further cleanup: '
                . trim($result['stderr'] . ' ' . $result['stdout']));
        }
    }
}

/** @return array<string, mixed> */
function doria_reclaim_obsolete_tests(
    string $target,
    string $repository,
    array $metadata,
    string $approvedPlan,
    DoriaTestCleanupMode $selection = DoriaTestCleanupMode::Obsolete,
): array
{
    $locks = [];
    try {
        $target = doria_cleanup_owned_target($target, $repository);
        foreach (doria_cleanup_profiles($target) as $profile) {
            $lockPath = $profile . '/.cargo-lock';
            doria_cleanup_file_identity($lockPath);
            $lock = fopen($lockPath, 'r+');
            if ($lock === false) {
                throw new RuntimeException("could not open Cargo lock {$lockPath}");
            }
            $locks[] = $lock;
            if (!flock($lock, LOCK_EX | LOCK_NB)) {
                throw new RuntimeException("Cargo target is active: {$profile}");
            }
            $held = fstat($lock);
            $current = doria_cleanup_file_identity($lockPath);
            if ($held === false || $held['dev'] !== $current['dev'] || $held['ino'] !== $current['ino']) {
                throw new RuntimeException("Cargo lock was replaced: {$lockPath}");
            }
        }
        $plan = doria_obsolete_test_plan($target, $repository, $metadata, $selection);
        if (!hash_equals($plan['planId'], $approvedPlan)) {
            throw new RuntimeException('selective cleanup plan changed; inspect a fresh dry-run');
        }
        $paths = [];
        foreach ($plan['candidates'] as $candidate) {
            $paths[] = $candidate['path'];
            $paths[] = $plan['proofs'][$candidate['retainedBy']]['path'];
            $compilation = $plan['proofs'][$candidate['compilation']];
            if ($compilation['file'] !== null) {
                $paths[] = $compilation['path'];
            }
        }
        doria_assert_tests_unused($paths);
        // Complete revalidation precedes the first deletion. No directory tree,
        // library, dependency record, or latest executable is ever removed.
        if (doria_obsolete_test_plan($target, $repository, $metadata, $selection)['planId'] !== $approvedPlan) {
            throw new RuntimeException('selective cleanup proof changed during active-use checks');
        }
        $removed = [];
        foreach ($plan['candidates'] as $candidate) {
            try {
                if (doria_cleanup_file_identity($candidate['path']) !== $candidate['file']
                    || !unlink($candidate['path'])) {
                    throw new RuntimeException('file changed or could not be removed');
                }
            } catch (RuntimeException $error) {
                throw new RuntimeException('cleanup stopped at ' . $candidate['path'] . ': ' . $error->getMessage()
                    . '; already removed: ' . json_encode($removed, JSON_THROW_ON_ERROR), previous: $error);
            }
            $removed[] = $candidate['path'];
        }
        $plan['mode'] = 'applied';
        $plan['removed'] = $removed;
        return $plan;
    } finally {
        foreach (array_reverse($locks) as $lock) {
            flock($lock, LOCK_UN);
            fclose($lock);
        }
    }
}
