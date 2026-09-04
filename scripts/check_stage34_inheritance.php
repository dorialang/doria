<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage34_inheritance(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 34 file is missing";
            return '';
        }

        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 34 contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains stale Stage 34 boundary `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0130-single-class-inheritance-open-override-parent-construction-and-hierarchy-dispatch.md',
        'guide' => 'docs/class-inheritance.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'spec' => 'SPEC.md',
        'lexer' => 'crates/doriac/src/lexer.rs',
        'ast' => 'crates/doriac/src/ast.rs',
        'parser' => 'crates/doriac/src/parser.rs',
        'semantics' => 'crates/doriac/src/semantics.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'lowering' => 'crates/doriac/src/mir_lowering.rs',
        'validation' => 'crates/doriac/src/mir_validation.rs',
        'interpreter' => 'crates/doriac/src/mir_interpreter.rs',
        'cranelift' => 'crates/doriac/src/codegen_cranelift.rs',
        'llvm' => 'crates/doriac/src/codegen_llvm.rs',
        'php' => 'crates/doriac/src/codegen_php.rs',
        'runtime' => 'crates/doria-rt/src/lib.rs',
        'incremental' => 'crates/doriac/src/incremental.rs',
        'tests' => 'crates/doriac/tests/stage34_inheritance_tests.rs',
        'mirTests' => 'crates/doriac/tests/mir_validation_tests.rs',
        'nativeTests' => 'crates/doriac/tests/native_testing_slice1_tests.rs',
        'parity' => 'crates/doriac/tests/fixtures/native_parity_examples.txt',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $require($paths['decision'], $files['decision'], [
        '# Decision 0130:',
        '**Status:** Accepted',
        '**Accepted:** 2026-09-01',
        '**Implementation Status:** Implemented By Stage 34',
        'headerless and data-only',
        'closed exact class value keeps the existing one-word',
        'private two-word hierarchy carrier',
        'no `protected`',
        'Metadata schemas 1, 2, and',
        'processor protocol version 1 remain exact',
        'Stage 35 may',
        'reuse private descriptor infrastructure for interface fat pointers',
    ]);
    foreach (['guide', 'plan', 'pipeline', 'spec'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 34',
            'Stage 35',
            'open',
            'override',
            'parent::',
        ]);
    }
    $require($paths['plan'], $files['plan'], [
        'Stage 34 — Single Class Inheritance — Complete',
        'Stage 35 — Interfaces And Traits — Authority Accepted; Slice 1 Next',
        'Decision 0130',
    ]);
    $require($paths['pipeline'], $files['pipeline'], [
        'Stage 34 Single Class Inheritance — Complete.',
        'Stage 35 Interfaces And Traits — Authority Accepted; Slice 1 Next.',
    ]);
    $forbid($paths['spec'], $files['spec'], [
        'parent lookup and dispatch are Stage 34 semantics and are currently diagnosed as unsupported',
        'Hierarchy `is` is deferred to Stage 34',
        'Doria will support `extends` for inheritance',
    ]);
    $forbid($paths['pipeline'], $files['pipeline'], [
        'Stage 34 Single Class Inheritance is next',
        'Parent lookup/dispatch until Stage 34',
        'hierarchy `is` remains deferred until Stage 34',
    ]);

    $require($paths['lexer'], $files['lexer'], [
        'Open',
        'Override',
        '"open" => TokenKind::Open',
        '"override" => TokenKind::Override',
    ]);
    foreach (['ast', 'hir'] as $key) {
        $require($paths[$key], $files[$key], [
            'pub is_open: bool',
            'pub open_span: Option<Span>',
            'pub is_override: bool',
            'pub override_span: Option<Span>',
            'pub parent: Option<TypeRef>',
        ]);
    }
    $require($paths['parser'], $files['parser'], [
        'TokenKind::Open',
        'TokenKind::Override',
        'if let (Some(open), Some(overridden)) = (open_span, override_span)',
        '`open` and `override` cannot be combined',
        'is_override: override_span.is_some()',
    ]);
    $require($paths['semantics'], $files['semantics'], [
        'pub parent: Option<ClassType<ResolvedType>>',
        'pub ancestors: Vec<ClassType<ResolvedType>>',
        'direct_parent: bool',
        'if !parent_info.is_open',
        'if !method.is_override',
        'parent::__construct(...)',
        'interface conformance tests land in Stage 35',
    ]);
    $require($paths['mir'], $files['mir'], [
        'pub is_open: bool',
        'pub parent: Option<ClassId>',
        'pub ancestors: Vec<ClassId>',
        'pub virtual_methods: Vec<FunctionId>',
        'pub virtual_slot: Option<u32>',
        'ClassIs {',
        'DropClass {',
    ]);
    $require($paths['lowering'], $files['lowering'], [
        'function_virtual_slots',
        'class.ancestors.iter().rev()',
        'call_target_is_direct_parent',
        'error_descriptors_covered_by',
    ]);
    $require($paths['validation'], $files['validation'], [
        'class.ancestors.contains(&target)',
        'without a dominating hierarchy `is` proof',
        'has no explicit borrowed receiver',
    ]);
    $require($paths['interpreter'], $files['interpreter'], [
        'class_is_subtype',
        'callee.virtual_slot',
        'virtual_methods',
    ]);
    $require($paths['cranelift'], $files['cranelift'], [
        'class.virtual_methods',
        'mir_class_is_subtype',
        'callee_definition.virtual_slot',
        'lower_drop_function_carrier',
    ]);
    $require($paths['llvm'], $files['llvm'], [
        'class.virtual_methods',
        'mir_class_is_subtype',
        'callee_definition.virtual_slot',
        'class.dynamic-drop.call',
    ]);
    $require($paths['php'], $files['php'], [
        'direct_parent_calls',
        'parent::__construct();',
        'parent::__destruct();',
    ]);
    $require($paths['runtime'], $files['runtime'], [
        'payload_descriptor',
        'dr_v3_shared_payload_descriptor',
        'dr_v3_writable_shared_payload_descriptor',
    ]);
    $require($paths['incremental'], $files['incremental'], [
        'if let Some(parent) = &class.parent',
        'surface.push_str(&parent.to_string())',
    ]);
    $require($paths['tests'], $files['tests'], [
        'parser_preserves_open_override_and_generic_parent_syntax',
        'hierarchy_validation_rejects_closed_cycles_invalid_overrides_and_constructor_protocols',
        'error_hierarchy_coverage_and_dynamic_catches_use_ancestor_contracts',
        'covariant_virtual_returns_and_shadowed_initializers_execute_in_mir',
        'automatic_effects_are_closed_over_the_entire_virtual_family',
        'deep_hierarchies_and_many_virtual_slots_have_stable_linear_metadata',
    ]);
    $require($paths['mirTests'], $files['mirTests'], [
        'shared_validator_requires_a_dominating_hierarchy_proof_for_narrowed_class_locals',
    ]);
    $require($paths['nativeTests'], $files['nativeTests'], [
        'stage34_virtual_dispatch_preserves_test_assertion_effects',
        'stage34_throw_matchers_accept_error_superclass_inspectors',
    ]);

    $examples = glob($root . '/examples/native/main_stage34_inheritance_*.doria');
    if (!is_array($examples) || count($examples) !== 32) {
        $failures[] = 'examples/native: Stage 34 requires exactly 32 durable inheritance fixtures';
    } else {
        foreach ($examples as $example) {
            $relative = 'examples/native/' . basename($example);
            if (!str_contains($files['parity'], $relative)) {
                $failures[] = "{$paths['parity']}: missing Stage 34 fixture `{$relative}`";
            }
        }
    }

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage34_inheritance(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 34 inheritance check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Stage 34 inheritance check passed\n");
}
