<?php

declare(strict_types=1);

function check_constructor_owned_property_writes(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required constructor/property-write authority is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing constructor/property-write contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: stale constructor/property-write restriction `{$needle}`";
            }
        }
    };

    $decisionPath = 'docs/decisions/0122-constructor-rooted-writable-paths-and-owned-property-writes.md';
    $auditPath = 'docs/notes/temporary-language-restrictions-audit.md';
    $specPath = 'SPEC.md';
    $planPath = 'docs/doria-end-to-end-plan.md';
    $pipelinePath = 'docs/notes/current-pipeline.md';
    $semanticsPath = 'crates/doriac/src/semantics.rs';
    $constructorPath = 'crates/doriac/src/constructor_init.rs';
    $mirPath = 'crates/doriac/src/mir.rs';
    $validatorPath = 'crates/doriac/src/mir_validation.rs';
    $ownershipPath = 'crates/doriac/src/ownership.rs';
    $testsPath = 'crates/doriac/tests/constructor_owned_property_tests.rs';
    $manifestPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';

    $decision = $read($decisionPath);
    $audit = $read($auditPath);
    $spec = $read($specPath);
    $plan = $read($planPath);
    $pipeline = $read($pipelinePath);
    $semantics = $read($semanticsPath);
    $constructor = $read($constructorPath);
    $mir = $read($mirPath);
    $validator = $read($validatorPath);
    $ownership = $read($ownershipPath);
    $tests = $read($testsPath);
    $manifest = $read($manifestPath);

    $require($decisionPath, $decision, [
        '**Status:** Accepted',
        '**Accepted:** 2026-08-21',
        '**Implementation Status:** Implemented By The Pre-Stage-30c Corrective Beat',
        'The direct `$this` of the declaring `__construct` has `ConstructionRoot`',
        'it never makes `writable function __construct` valid',
        'Every intermediate property in a nested path must be definitely initialized',
        'nested readonly property does not receive constructor-only',
        'An independently owned Move value may initialize an uninitialized owning',
        'replace an initialized writable owning property',
        'after successful acquisition, install the new owner and destroy the old',
        'Move-out remains a separate',
    ]);
    $require($auditPath, $audit, [
        'Permanent Language Rejection',
        'Accepted But Not Implemented',
        'Open Design Question',
        'Temporary Soundness Fence',
        'Stale Restriction Now Provably Safe',
        'Historical Diagnostic',
        'E0472 move-in route',
        'E0641 closure execution boundary',
        'Collection `Cloneable` boundaries',
    ]);
    $require($specPath, $spec, [
        'Direct constructor `$this` is a construction root',
        '$this->window->title = $initialTitle;',
        '$this->window = new Window($initialTitle);',
        'Replacement evaluates and acquires the new value before destroying the old value',
    ]);
    foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
        $require($path, $contents, [
            'Stage 30b Semantic Function Types And Captures — Complete',
            'Constructor Writable-Path And Owned-Property Corrective Beat — Complete',
            'Stage 30c Ownership, Lifetime, And Escape — Next',
            'Stage 30 — In Progress, Not Complete',
        ]);
    }
    $require($semanticsPath, $semantics, [
        'enum ReceiverAccess',
        'ConstructionRoot',
        'writable_object_paths',
        'PropertyWriteSemanticInfo',
    ]);
    $require($constructorPath, $constructor, [
        'PropertyWriteKind::Initialize',
        'PropertyWriteKind::Replace',
        'PropertyWriteKind::InitializeOrReplace',
    ]);
    $require($mirPath, $mir, [
        'pub enum PropertyWriteKind',
        'InitializeOrReplace',
    ]);
    $require($validatorPath, $validator, [
        'conditional initialization must target a writable property',
        'stores a borrowed move value',
        'without a maybe-initialized writable obligation',
    ]);
    $require($ownershipPath, $ownership, [
        'Owning Property Needs An Owned Value',
        'direct moves out of owned properties are not supported',
        'Property Transfer Overlaps Its Destination',
    ]);
    $require($testsPath, $tests, [
        'constructor_root_derives_writable_access_through_initialized_properties',
        'constructor_can_initialize_owned_property_and_writable_method_can_replace_it',
        'property_move_out_and_self_move_remain_rejected',
        'constructor_write_kinds_preserve_conditional_initialization_and_replacement_order',
    ]);
    $require($manifestPath, $manifest, [
        'main_constructor_owned_property_writes.doria',
        'main_constructor_property_write_order.doria',
    ]);

    foreach ([$specPath => $spec, $planPath => $plan] as $path => $contents) {
        $forbid($path, $contents, [
            'Direct moves into or out of nested owned properties remain unsupported',
            'Direct moves into (nested) owned properties stay unsupported',
            'cannot use init access for nested object paths',
        ]);
    }

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_constructor_owned_property_writes(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "constructor/property-write check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "constructor/property-write check passed\n");
}
