<?php

declare(strict_types=1);

/** @return list<string> */
function check_baton_native_transition(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Baton transition authority is missing";
            return '';
        }

        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Baton transition contract `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0124-baton-bootstrap-doria-native-transition-and-toolchain-release-gate.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'readme' => 'README.md',
        'spec' => 'SPEC.md',
        'agents' => 'AGENTS.md',
        'architecture' => 'docs/information-architecture.md',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $shared = [
        'Pre-Stage-45',
        'dorialang/baton',
        '2026.03.1',
    ];
    foreach (['decision', 'plan', 'pipeline', 'spec'] as $key) {
        $require($paths[$key], $files[$key], $shared);
    }

    $require($paths['readme'], $files['readme'], [
        'Doria-native project and package tool',
        '`dorialang/baton`',
        'require no Baton PHP runtime or Composer payload',
    ]);

    $require($paths['decision'], $files['decision'], [
        '**Status:** Accepted',
        'Stage 33 therefore completes and validates the Baton product contract in the',
        'shared, implementation-neutral',
        'contain no Baton PHAR, Composer dependency, private',
        'unsuffixed `2026.03.1`',
    ]);
    $require($paths['plan'], $files['plan'], [
        'Stage 33 — Baton package and dependency workflow in the disposable PHP UX bootstrap',
        'Pre-Stage-45 Doria-Native Baton Transition — Mandatory',
        'Production archives drop the PHAR, Composer, PHP launcher, and private PHP runtime',
        'Stage 45 — Compiler self-hosting start',
        'The unsuffixed `2026.03.1` release cannot ship',
    ]);
    $require($paths['agents'], $files['agents'], [
        'disposable UX and distribution bootstrap',
        'ships without a Baton PHAR, Composer, or a private PHP runtime',
        'No unsuffixed',
    ]);
    $require($paths['architecture'], $files['architecture'], [
        'Duplication And Drift Control',
        'An unnumbered "later", "eventually", "exit strategy", or',
        'add a mechanical documentation',
    ]);

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_baton_native_transition(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Baton native transition check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Baton native transition check passed\n");
}
