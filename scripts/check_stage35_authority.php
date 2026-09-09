#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

$read = static function (string $path) use ($root, &$failures): string {
    $contents = @file_get_contents($root . '/' . $path);
    if (!is_string($contents)) {
        $failures[] = "{$path}: required Stage 35 authority file is missing";
        return '';
    }

    return $contents;
};

$require = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (!str_contains($contents, $needle)) {
            $failures[] = "{$path}: missing Stage 35 authority `{$needle}`";
        }
    }
};

$forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (str_contains($contents, $needle)) {
            $failures[] = "{$path}: contains stale or forbidden Stage 35 claim `{$needle}`";
        }
    }
};

$paths = [
    'decision' => 'docs/decisions/0134-interfaces-traits-core-value-contracts-and-public-iteration.md',
    'agents' => 'AGENTS.md',
    'spec' => 'SPEC.md',
    'readme' => 'README.md',
    'plan' => 'docs/doria-end-to-end-plan.md',
    'pipeline' => 'docs/notes/current-pipeline.md',
    'stdlib' => 'docs/stdlib-reference.md',
    'api' => 'docs/api-design-guidelines.md',
    'inheritance' => 'docs/class-inheritance.md',
    'diagnostics' => 'docs/diagnostic-style.md',
    'metadata' => 'docs/attribute-metadata-protocol.md',
    'restrictions' => 'docs/notes/temporary-language-restrictions-audit.md',
    'nativeParity' => 'docs/notes/native-parity-matrix.md',
    'collectionsAudit' => 'docs/notes/collection-surface-audit.md',
    'selfHosting' => 'docs/self-hosting.md',
    'websiteGuidance' => 'docs/website-content-guidelines.md',
    'openQuestions' => 'docs/notes/plan-open-questions-audit.md',
];

$files = [];
foreach ($paths as $key => $path) {
    $files[$key] = $read($path);
}

$require($paths['decision'], $files['decision'], [
    '# Decision 0134:',
    '**Status:** Accepted',
    '**Implementation Status:** Stage 35 Authority Accepted; Slices 1, 2, 3, And 4 Complete; Slice 5 Next',
    'interface Equatable<T>',
    'function equals(T $other): bool;',
    'function hash(): uint64;',
    'function clone(): self;',
    'function iterator(): Iterator<T>;',
    'function hasCurrent(): bool;',
    'function getCurrent(): T;',
    '`borrow T $source`',
    'author obligation',
    'memory safety',
    'writable function advance(): void;',
    'The compiler-known Iterator methods declare',
    'no checked Errors',
    'concrete implementing type specialization',
    'built-in array or collection specialization',
    '| TraitRef "::" Name "as" Name ";"',
    '| TraitRef "::" Name "as" "internal" Name? ";"',
    'Type parameters remain excluded from attribute targets under Decision 0125.',
    'User interfaces remain method-only.',
    'Stage 36 remains their sole owner.',
    'all six existing families',
    'Slice 1: Grammar, Graphs, And Conformance',
    'Slice 5: Cross-Repository Closure',
]);

$require($paths['plan'], $files['plan'], [
    'Stage 35 — Interfaces And Traits — In Progress; Slices 1, 2, 3, And 4 Complete; Slice 5 Next',
    'Slice 1 — Complete: Grammar, Graphs, And Conformance',
    'Slice 2 — Complete: Interface Runtime And Ownership',
    'Slice 3 — Complete: Core Contracts And Public Iteration',
    'Slice 4 — Complete: Trait Composition',
    'Slice 5 — Next: Cross-Repository Closure',
]);

$require($paths['pipeline'], $files['pipeline'], [
    'Stage 35 Interfaces And Traits authority is accepted under Decision 0134',
    'Slice 1 Grammar, Graphs, And Conformance is complete',
    'Stage 35 Authority — Accepted',
    'Stage 35 Slice 1 Grammar, Graphs, And Conformance — Complete',
    'Stage 35 Slice 2 Interface Runtime And Ownership — Complete',
    'Stage 35 Slice 3 Core Contracts And Public Iteration — Complete',
    'Stage 35 Slice 4 Trait Composition — Complete',
    'Stage 35 Slice 5 Cross-Repository Closure — Next',
    'Stage 35 — In Progress',
]);

$require($paths['spec'], $files['spec'], [
    'Decision 0134 defines generic nominal interfaces',
    'User interfaces remain method-only',
    'Traits cannot declare lifecycle methods',
    'There is no runtime trait object',
]);

$require($paths['stdlib'], $files['stdlib'], [
    'equals(T $other): bool',
    'hash(): uint64',
    'clone(): self',
    'iterator(): Iterator<T>',
    'hasCurrent(): bool',
    'getCurrent(): T',
    'advance(): void',
    'Copy-or-Cloneable',
]);

foreach ([
    'agents', 'readme', 'api', 'inheritance', 'diagnostics', 'metadata',
    'restrictions', 'nativeParity', 'selfHosting', 'websiteGuidance',
    'openQuestions',
] as $key) {
    $require($paths[$key], $files[$key], ['Decision 0134']);
}

$require($paths['collectionsAudit'], $files['collectionsAudit'], [
    'Decision 0134',
    'foreach-only projections',
    'silently turn them into owned lists',
]);

$staleStatus = [
    'Slices 1 And 2 Complete; Slice 3 Next',
    'Slice 3 is not delivered yet',
    'core operations and public iteration: Slice 3 / E0759',
    'Slice 1 Complete; Slice 2 Next',
    'Stage 35 Slice 1 is complete and Slice 2 is next',
    'interface-typed values and general interface dispatch remain deferred',
    'Interface values, erased calls, conversions/tests, and shared-interface execution until Stage 35 Slice 2',
    'Collection/interface `is`',
    'The PHP backend still refuses shared ownership',
    'Slice 1 is next',
    'Slice 1 Next',
    'Stage 35 — Interfaces And Traits — Next',
    'Stage 35 Interfaces And Traits — Next',
    'Stage 35 Interfaces And Traits - Next',
    'Stage 35 interfaces and traits is next',
    'Stage 35 is next',
];

$require('crates/doriac/src/semantics/contracts.rs', $read('crates/doriac/src/semantics/contracts.rs'), [
    'ConformanceStatus', 'ContractMismatch', 'invalidate_erroneous_compositions',
]);
$require('crates/doriac/src/ast.rs', $read('crates/doriac/src/ast.rs'), [
    'enum FunctionBody', 'Requirement', 'TraitAdaptation',
]);
$require('crates/doriac/tests/stage35_contract_tests.rs', $read('crates/doriac/tests/stage35_contract_tests.rs'), [
    'every_callable_substitution_axis_is_retained_in_conformance_facts',
    'interface_value_matrix_checks_and_lowers_through_public_entrypoints',
    'unused_traits_are_compile_time_declarations_and_composition_checks_conformance',
]);

$forbid('crates/doriac/src/semantics/contracts.rs', $read('crates/doriac/src/semantics/contracts.rs'), ['InterfaceValue', 'E0758', 'E0493', 'DeferredComposition']);
$require('crates/doriac/src/trait_composition.rs', $read('crates/doriac/src/trait_composition.rs'), [
    'EffectiveMemberOrigin', 'MethodObligation', 'CompositionPlan', 'fn deduplicate',
]);
$require('crates/doriac/src/trait_composition/validation.rs', $read('crates/doriac/src/trait_composition/validation.rs'), [
    'malformed_class_plans_do_not_cross_executable_lowering',
    'duplicate physical trait property', 'unsatisfied trait method obligation',
]);
$require('crates/doriac/tests/stage35_trait_tests.rs', $read('crates/doriac/tests/stage35_trait_tests.rs'), [
    'durable_trait_fixtures_preserve_interpreter_results',
    'two_composers_and_aliases_keep_independent_expression_and_closure_identities',
    'invalid_composed_bodies_hierarchy_and_initialization_revoke_conformance',
    'trait_aliases_preserve_checked_effects_and_retained_cursor_loans',
]);
$require('crates/doriac/tests/stage35_runtime_tests.rs', $read('crates/doriac/tests/stage35_runtime_tests.rs'), [
    'interface_erasure_keeps_headerless_layout_and_constrained_calls_direct',
    'durable_interface_fixtures_preserve_php_execution_and_cleanup',
    'interface_dispatch_rejects_slot_receiver_and_entry_abi_mismatches',
    'interface_carrier_validation_rejects_unknown_and_mismatched_views',
    'generated_core_calls_require_nominal_selection_and_owned_duplication',
    'retained_iterator_mir_keeps_source_ownership_and_validates_lifetimes',
    'public_foreach_acquires_once_advances_on_continue_and_preserves_move_elements',
    'collection_core_failures_preserve_existing_owners_and_clean_partial_results',
    'local_collection_cursors_use_fixed_frame_storage_without_changing_returned_cursors',
]);

foreach ($files as $key => $contents) {
    $forbid($paths[$key], $contents, $staleStatus);
}

$forbid($paths['decision'], $files['decision'], [
    'function current(): T;',
    'TraitRef "::" Name "as" ("internal")? Name? ";"',
    'interface conversion allocates a wrapper',
    'primitives inhabit interface-typed slots',
    'runtime trait object is required',
    'property hooks are part of Stage 35',
]);

$forbid($paths['stdlib'], $files['stdlib'], ['`current(): T`']);

$require($paths['readme'], $files['readme'], [
    'currently provides `map`, `filter`',
    'preserves Copy-or-Cloneable elements',
]);

foreach ([
    '0029', '0030', '0079', '0082', '0087', '0089', '0093', '0096',
    '0100', '0102', '0105', '0106', '0110', '0113', '0119', '0121',
    '0125', '0129', '0130', '0131', '0132', '0133',
] as $number) {
    $matches = glob($root . "/docs/decisions/{$number}-*.md") ?: [];
    if (count($matches) !== 1) {
        $failures[] = "decision {$number}: expected one authored record";
        continue;
    }
    $contents = @file_get_contents($matches[0]);
    if (!is_string($contents) || !str_contains($contents, 'Decision 0134')) {
        $failures[] = basename($matches[0]) . ': missing Decision 0134 amendment pointer';
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Stage 35 authority check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Stage 35 authority check passed.\n");
