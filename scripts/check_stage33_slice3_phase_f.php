<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage33_slice3_phase_f(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 33 Slice 3 authority is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 33 Slice 3 authority `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0128-baton-workspaces-development-tests-processors-and-project-inventory.md',
        'protocol' => 'docs/attribute-metadata-protocol.md',
        'attributes' => 'crates/doriac/src/attributes.rs',
        'library' => 'crates/doriac/src/lib.rs',
        'cli' => 'crates/doriac/src/main.rs',
        'tests' => 'crates/doriac/tests/stage33_metadata_tests.rs',
        'cliTests' => 'crates/doriac/tests/cli_tests.rs',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $require($paths['decision'], $files['decision'], [
        '# Decision 0128:',
        '**Status:** Accepted',
        '**Accepted:** 2026-08-29',
        '**Implementation Status:** Implemented By Stage 33 Slice 3',
        '**Amends:** Decisions 0118, 0124, 0125, 0126, and 0127',
        'compiler build-plan schema 1',
        'schema-1 Baton compatibility',
        'processor protocol version 1',
        'source = "path"',
        'source = "git"',
        'workspace uses one root `Baton.lock`',
        '[dev-dependencies]',
        '`baton test`',
        '`doriac metadata` remains schema 1 by default',
        '[processors]',
        'performs no recursive processor pass',
        'build/generated/',
        '`baton tree`',
        '`baton why <package>`',
        '`baton project --json`',
        'language server invokes this command off the UI thread',
        'does not claim sandboxing',
        'mandatory Pre-Stage-45 native transition',
        'Stage 33 and Phase F are complete',
    ]);
    $require($paths['protocol'], $files['protocol'], [
        '--schema-version 2',
        'Schema version 1 remains the default',
        'plus `callables`',
        'processor requests and responses remain strict schema version 1',
    ]);
    $require($paths['attributes'], $files['attributes'], [
        'pub const ATTRIBUTE_METADATA_SCHEMA_VERSION: u32 = 1;',
        'pub const ATTRIBUTE_METADATA_SCHEMA_VERSION_2: u32 = 2;',
        'pub const ATTRIBUTE_PROCESSOR_SCHEMA_VERSION: u32 = 1;',
        'pub struct AttributeMetadataDocumentV2',
        'pub struct MetadataCallableV2',
        'pub fn metadata_document_v2',
    ]);
    $require($paths['library'], $files['library'], [
        'pub fn metadata_source_v2',
        'pub fn metadata_compilation_graph_v2',
    ]);
    $require($paths['cli'], $files['cli'], [
        'metadata_schema_version',
        'unsupported metadata schema version',
        '[--schema-version 1|2]',
    ]);
    $require($paths['tests'], $files['tests'], [
        'metadata_schema_2_exposes_exact_callable_signatures_without_runtime_data',
        'metadata_schema_2_includes_main_development_and_generated_sources',
        'processor_protocol_remains_schema_1',
    ]);
    $require($paths['cliTests'], $files['cliTests'], [
        'the default metadata protocol must remain byte-identical to explicit schema 1',
        'unsupported metadata schema version `3`',
    ]);

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_stage33_slice3_phase_f(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 33 Slice 3 Phase F check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 33 Slice 3 Phase F check passed\n");
}
