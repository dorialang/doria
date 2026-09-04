#!/usr/bin/env php
<?php

declare(strict_types=1);

/** @return list<string> */
function check_indexed_foreach_and_scalar_display(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required indexed-foreach authority is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing indexed-foreach contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains forbidden indexed-foreach surface `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0132-indexed-sequence-foreach-bindings-and-scalar-display-materialization.md',
        'explicitTypesDecision' => 'docs/decisions/0133-explicit-foreach-binding-types.md',
        'decision131' => 'docs/decisions/0131-constructor-property-overrides-and-constructor-only-parameters.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'ast' => 'crates/doriac/src/ast.rs',
        'semantics' => 'crates/doriac/src/semantics.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'mirLowering' => 'crates/doriac/src/mir_lowering.rs',
        'mirValidation' => 'crates/doriac/src/mir_validation.rs',
        'php' => 'crates/doriac/src/codegen_php.rs',
        'tests' => 'crates/doriac/tests/indexed_foreach_tests.rs',
        'manifest' => 'crates/doriac/tests/fixtures/native_parity_examples.txt',
        'indexedFixture' => 'examples/native/main_indexed_foreach.doria',
        'displayFixture' => 'examples/native/main_scalar_display_materialization.doria',
        'catalogue' => 'crates/doria-diagnostic-catalogue/src/lib.rs',
        'metadata' => 'docs/attribute-metadata-protocol.md',
        'temporaryRestrictions' => 'docs/notes/temporary-language-restrictions-audit.md',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $require($paths['decision'], $files['decision'], [
        '# Decision 0132:',
        '**Status:** Accepted',
        'Implemented By The Post-Stage-34 Indexed Foreach And Scalar Display Corrective Beat',
        '`List<T>`',
        '`T[]`',
        '`Dictionary<K, V>`',
        '`SortedDictionary<K, V>`',
        'Integer ranges, `Set<T>`, `SortedSet<T>`, `Deque<T>`',
        '`PriorityQueue<T>` remains',
        '`Bytes` gains no `foreach`',
        'Decision 0104',
        'metadata schemas 1, 2, and 3',
        'processor protocol version 1',
    ]);
    $require($paths['explicitTypesDecision'], $files['explicitTypesDecision'], [
        '# Decision 0133:',
        '**Status:** Accepted',
        'Implemented By The Post-Stage-34 Explicit Foreach Binding Types Corrective Beat',
        'Every authored foreach binding has an explicit type',
        'Foreach Binding Type Is Required',
        'Source with E0748 does not enter HIR',
        'The next decision-record',
    ]);
    $require($paths['decision131'], $files['decision131'], [
        '**Implementation Status:** Implemented By Post-Stage-34 Constructor Parameter Roles Corrective Beat',
    ]);
    $require($paths['plan'], $files['plan'], [
        'Stage 34 — Single Class Inheritance — Complete',
        'Indexed Foreach And Scalar Display Corrective Beat — Complete',
        'Explicit Foreach Binding Types Corrective Beat — Complete',
        'Stage 35 — Interfaces And Traits — Next',
        'Stage 36 Property Hooks — Scheduled',
    ]);
    $require($paths['pipeline'], $files['pipeline'], [
        'Stage 34 Single Class Inheritance — Complete.',
        'Indexed Foreach And Scalar Display Corrective Beat — Complete.',
        'Explicit Foreach Binding Types Corrective Beat — Complete.',
        'Stage 35 Interfaces And Traits — Next.',
        'Stage 36 Property Hooks — Scheduled.',
    ]);

    $require($paths['ast'], $files['ast'], [
        'first_binding: Option<ForeachBinding>',
        'value_binding: ForeachBinding',
        'writable_span: Option<Span>',
        'type_span: Option<Span>',
        'name_span: Span',
    ]);
    $forbid($paths['ast'], $files['ast'], ['pub key: Option<ForeachBinding>']);
    $require($paths['semantics'], $files['semantics'], [
        'pub struct ForeachSemanticInfo',
        'pub enum ForeachIterableFamily',
        'ValueOnly',
        'SequenceIndex',
        'DictionaryKey',
        'IntegerRange',
        'TypedArray',
        'DictionaryKeysProjection',
        'PriorityQueue',
        'Bytes',
        'ForeachValueAccess::Writable',
        'E0745',
        'E0746',
        'E0747',
        'E0748',
        'require_foreach_binding_type',
    ]);
    $require($paths['hir'], $files['hir'], [
        'iteration_kind: crate::semantics::ForeachIterationKind',
        'first_binding_type',
        'value_binding_type',
    ]);
    $require($paths['mir'], $files['mir'], [
        'pub struct ForeachPlan',
        'SequenceIndex',
        'DictionaryKey',
        'Sequence Index',
        'Dictionary Key',
        'Value Only',
    ]);
    $require($paths['mirLowering'], $files['mirLowering'], [
        'mir::ForeachIterationKind::SequenceIndex',
        'mir::ForeachIterationKind::DictionaryKey',
        'positional: true',
    ]);
    $require($paths['mirValidation'], $files['mirValidation'], [
        'validate_foreach_plan',
        'ForeachIterationKind::SequenceIndex',
        'ForeachIterationKind::DictionaryKey',
        'one zero-based positional traversal CFG',
    ]);
    $require($paths['php'], $files['php'], [
        'ForeachIterationKind::SequenceIndex',
        '__doria_sequence_index',
        'ForeachIterationKind::DictionaryKey',
    ]);
    $require($paths['tests'], $files['tests'], [
        'parser_preserves_neutral_bindings_and_exact_authored_spans',
        'semantic_facts_distinguish_sequence_indexes_dictionary_keys_and_value_only_sources',
        'invalid_first_bindings_are_rejected_with_local_machine_fixes_and_no_backend_error',
        'every_foreach_binding_requires_an_explicit_type_with_local_fixes',
        'explicit_binding_types_cover_every_iterable_family_and_element_shape',
        'malformed_foreach_roles_ordinals_and_binding_sources_are_rejected',
        'indexed_sequences_execute_with_property_roots_and_control_flow',
        'php_uses_a_compiler_owned_sequence_ordinal_and_preserves_dictionary_keys',
        'canonical_scalar_display_materializes_ordinary_strings',
    ]);
    $require($paths['manifest'], $files['manifest'], [
        'examples/native/main_indexed_foreach.doria',
        'examples/native/main_scalar_display_materialization.doria',
    ]);
    $require($paths['indexedFixture'], $files['indexedFixture'], [
        '$this->contents as int $line',
        'foreach ($words as int $index => string $word)',
        'continue;',
        'break;',
        'Dictionary<string, int>',
        'SortedDictionary<int, string>',
    ]);
    $require($paths['displayFixture'], $files['displayFixture'], [
        'sprintf("%s"',
        'sprintf("%.2f"',
        'List<string>',
        'string[]',
        '0.0 / 0.0',
        '1.0 / 0.0',
    ]);
    $require($paths['catalogue'], $files['catalogue'], ['E0745', 'E0746', 'E0747', 'E0748']);
    $require($paths['metadata'], $files['metadata'], [
        'schema version 1',
        'schema version 2',
        'schema version 3',
        'protocol remains version 1',
    ]);
    $require($paths['temporaryRestrictions'], $files['temporaryRestrictions'], [
        '| Indexed sequence `foreach` first bindings',
        '| Complete | Decision 0132 | Post-Stage-34 corrective beat |',
        '| Explicit `foreach` binding types',
        '| Complete | Decision 0133 | Post-Stage-34 corrective beat |',
    ]);
    $forbid($paths['decision'], $files['decision'], ['inferred or explicit element type']);
    $forbid($paths['plan'], $files['plan'], ['typed/inferred/property-rooted']);
    $forbid($paths['indexedFixture'], $files['indexedFixture'], [
        'as $index =>',
        'as $word)',
    ]);
    foreach (glob($root . '/docs/decisions/*property-hook*.md') ?: [] as $path) {
        $failures[] = str_replace($root . '/', '', $path) . ': property-hook authority is out of scope';
    }
    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_indexed_foreach_and_scalar_display(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Indexed foreach and scalar display check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Indexed foreach and scalar display check passed\n");
}
