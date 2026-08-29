<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage33_slice2_dependencies_and_lockfile(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 33 Slice 2 authority is missing";
            return '';
        }

        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 33 Slice 2 authority `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains stale Stage 33 Slice 2 authority `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0127-baton-dependency-resolution-lockfile-cache-and-offline-semantics.md',
        'namespaceDecision' => 'docs/decisions/0117-namespaces-compile-time-autoloading-hybrid-source-layout-and-package-compilation-graphs.md',
        'packageDecision' => 'docs/decisions/0118-baton-manifests-package-dependencies-resolution-lockfiles-workspaces-and-caches.md',
        'transitionDecision' => 'docs/decisions/0124-baton-bootstrap-doria-native-transition-and-toolchain-release-gate.md',
        'slice1Decision' => 'docs/decisions/0126-baton-schema-2-local-package-target-and-selection-spellings.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'spec' => 'SPEC.md',
        'readme' => 'README.md',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    // Executable resolver behavior is proved in dorialang/baton-php CI. This
    // guard keeps Doria's accepted authority synchronized with that evidence.
    $require($paths['decision'], $files['decision'], [
        '# Decision 0127:',
        '**Status:** Accepted',
        '**Accepted:** 2026-08-28',
        '**Implementation Status:** Implemented By Stage 33 Slice 2',
        '**Amends:** Decisions 0118, 0124, and 0126',
        "Decision 0117's compiler build-plan authority",
        "Decision 0125's attribute processor protocol",
        'schema-1 manifest compatibility',
        'direct-dependency compiler visibility',
        'package-wide `internal`',
        '[dependencies]',
        'exactly one of `rev`, `tag`, or',
        '`branch`',
        '[dev-dependencies]` remains deferred to Stage 33 Slice 3',
        'Path dependencies are live inputs',
        'Git dependencies require scoped package',
        'SemVer constraint',
        'One resolved graph contains one node for each compiler package identity',
        'source substitution',
        'every normal dependency cycle',
        'declare a library target',
        '`Baton.lock` uses strict deterministic JSON schema 1',
        'Lock writes are atomic',
        'Git commits are immutable locked identities',
        '`baton install`',
        '`baton add`',
        '`baton remove`',
        '`baton update`',
        '`baton fetch`',
        '`baton tree` and `baton why` remain recognized Stage 33 Slice 3 commands',
        'Offline behavior is one resolver-level network policy',
        'global cache outside project trees',
        'No project-local vendor directory, registry',
        'multi-package compiler build plans',
        'lockfile SHA-256',
        'path-dependency content',
        '`doriac` does not parse `Baton.toml` or `Baton.lock`',
        'Stage 33 Slice 3 is next',
        'Stage 33 remains In Progress, Not Complete',
        'mandatory Pre-Stage-45 transition remains scheduled',
    ]);

    foreach (['namespaceDecision', 'packageDecision', 'transitionDecision', 'slice1Decision'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 33',
            'Slice 3',
        ]);
    }
    $require($paths['plan'], $files['plan'], [
        'Decision 0127',
        'Slice 1 — Complete under Decision 0126',
        'Slice 2 — Complete under Decision 0127',
        'Stage 33 Slice 3 — In Progress',
        'Stage 33 — In Progress, Not Complete',
        'Pre-Stage-45 Doria-Native Baton Transition',
    ]);
    $require($paths['pipeline'], $files['pipeline'], [
        'Decision 0127',
        'Stage 33 Slice 1 — Complete',
        'Stage 33 Slice 2 — Complete',
        'Stage 33 Slice 3 — In Progress',
        'Stage 33 — In Progress, Not Complete',
        'Pre-Stage-45 Doria-Native Baton Transition',
    ]);
    $require($paths['spec'], $files['spec'], [
        'Decision 0127 fixes the implemented normal path/Git dependency resolver',
        'One compiler package identity resolves to one source and version',
        '`doriac` does not parse `Baton.toml` or `Baton.lock`',
    ]);
    $require($paths['readme'], $files['readme'], [
        '`dorialang/baton-php` product-contract',
        'normal path or Git dependencies',
        'one package identity resolves once',
        '`Baton.lock` files pin exact Git commits',
        '`install`, `add`, `remove`, `update`, and',
        '`fetch`',
        'Development dependencies, workspaces, graph inspection, tests, processors',
    ]);

    $staleStatuses = [
        'Stage 33 Slice 2 — Next',
        'Stage 33 Slice 2 is next',
        'Stage 33 Slice 2 Next',
        'Slice 2 dependency resolution and lockfiles are next',
        'dependency resolution remains a separate Stage 33 Slice 2 responsibility',
        'Stage 33 Slice 1 is complete and Slice 2 is next',
        'Slice 1 is complete under Decision 0126, Slice 2 is next',
    ];
    $guardPaths = [
        'scripts/check_stage33_slice1_baton_authority.php',
        'scripts/check_stage33_slice2_dependencies_and_lockfile.php',
        'scripts/check_phase_f_package_authority.php',
    ];
    foreach (['README.md', 'SPEC.md', 'docs', 'scripts'] as $entryPath) {
        $absolute = $root . '/' . $entryPath;
        $entries = is_dir($absolute)
            ? new RecursiveIteratorIterator(new RecursiveDirectoryIterator($absolute, FilesystemIterator::SKIP_DOTS))
            : [new SplFileInfo($absolute)];

        foreach ($entries as $entry) {
            if (!$entry->isFile() || !in_array($entry->getExtension(), ['md', 'php'], true)) {
                continue;
            }
            $path = substr($entry->getPathname(), strlen($root) + 1);
            if (in_array($path, $guardPaths, true)) {
                continue;
            }
            $contents = $read($path);
            if (str_contains($contents, 'Stage 33') || str_contains($contents, 'Baton')) {
                $forbid($path, $contents, $staleStatuses);
            }
        }
    }

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_stage33_slice2_dependencies_and_lockfile(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 33 Slice 2 dependency authority check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Stage 33 Slice 2 dependency authority check passed\n");
}
