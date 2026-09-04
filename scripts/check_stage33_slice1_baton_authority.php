<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage33_slice1_baton_authority(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 33 Slice 1 authority is missing";
            return '';
        }

        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 33 Slice 1 authority `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains stale Stage 33 Slice 1 authority `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0126-baton-schema-2-local-package-target-and-selection-spellings.md',
        'namespaceDecision' => 'docs/decisions/0117-namespaces-compile-time-autoloading-hybrid-source-layout-and-package-compilation-graphs.md',
        'packageDecision' => 'docs/decisions/0118-baton-manifests-package-dependencies-resolution-lockfiles-workspaces-and-caches.md',
        'transitionDecision' => 'docs/decisions/0124-baton-bootstrap-doria-native-transition-and-toolchain-release-gate.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'spec' => 'SPEC.md',
        'readme' => 'README.md',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $require($paths['decision'], $files['decision'], [
        '# Decision 0126:',
        '**Status:** Accepted',
        '**Accepted:** 2026-08-27',
        '**Implementation Status:** Implemented By Stage 33 Slice 1',
        '**Amends:** Decisions 0117, 0118, and 0124',
        "Decision 0125's metadata and processor protocol",
        'compiler build-plan schema 1',
        'Package identity, namespace identity, and filesystem location remain separate',
        'direct dependency visibility',
        'package-wide `internal`',
        'publishable = false',
        'publishable by default',
        'local/<name>',
        'synthetic vendor `local` is reserved',
        'kind = "binary"',
        '[targets.library]',
        '[[targets.binary]]',
        '--binary <name>',
        '--library',
        'There is no generic',
        '`--target` option',
        '`default-target`',
        'build/<host-target>/<profile>/<target-name>/',
        'build/<host-target>/<profile>/',
        '"artifact": null',
        'Decision 0127 owns the implemented path',
        'Implemented Stage 33 Slice 3',
        'Stage 33 and Phase F are complete',
        'Pre-Stage-45 transition',
        'dorialang/baton',
    ]);

    foreach (['namespaceDecision', 'packageDecision', 'transitionDecision', 'plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 33',
            'Slice 3',
        ]);
    }
    foreach (['plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 33 — Complete',
            'Phase F — Complete',
            'Stage 34',
            'Pre-Stage-45 Doria-Native Baton Transition',
        ]);
    }

    $require($paths['spec'], $files['spec'], [
        'Decision 0126 fixes schema-2 local/scoped package identity',
        '`publishable = false`',
        '`local/<name>`',
        '`[targets.library]`',
        '`[[targets.binary]]`',
        '`--library`',
        '`--binary <name>`',
    ]);
    $require($paths['readme'], $files['readme'], [
        'Baton is Doria',
        'exact schema 1 compatibility',
        'strict schema 2',
        'projects with local/scoped package identity',
        'binary and library targets',
        'compile-time `autoload` discovery',
        '`dorialang/baton-php` product-contract',
        'to install PHP or Composer',
    ]);

    $staleStatuses = [
        'Stage 33 Slice 1 — Next',
        'Stage 33 Slice 1 is next',
        'Stage 33 — Scheduled, Not Implemented',
        'Stage 33 is scheduled, not implemented',
        'Stage 33 remains scheduled',
        'Stage 33 Slice 2 — Next',
        'Stage 33 Slice 2 is next',
        'Stage 33 Slice 2 Next',
    ];
    foreach (['docs', 'scripts'] as $directory) {
        $iterator = new RecursiveIteratorIterator(
            new RecursiveDirectoryIterator($root . '/' . $directory, FilesystemIterator::SKIP_DOTS),
        );
        foreach ($iterator as $entry) {
            if (!$entry->isFile() || !in_array($entry->getExtension(), ['md', 'php'], true)) {
                continue;
            }
            $path = substr($entry->getPathname(), strlen($root) + 1);
            if (in_array($path, [
                'scripts/check_stage33_slice1_baton_authority.php',
                'scripts/check_stage33_slice2_dependencies_and_lockfile.php',
                'scripts/check_phase_f_package_authority.php',
            ], true)) {
                continue;
            }
            $contents = $read($path);
            if (!str_contains($contents, 'Stage 33')) {
                continue;
            }
            $forbid($path, $contents, $staleStatuses);
        }
    }

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_stage33_slice1_baton_authority(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 33 Slice 1 Baton authority check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Stage 33 Slice 1 Baton authority check passed\n");
}
