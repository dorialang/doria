<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage30d_closure_execution(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 30d file is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 30d contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: forbidden Stage 30d drift `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md',
        'spec' => 'SPEC.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'audit' => 'docs/notes/temporary-language-restrictions-audit.md',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'lowering' => 'crates/doriac/src/mir_lowering.rs',
        'validation' => 'crates/doriac/src/mir_validation.rs',
        'interpreter' => 'crates/doriac/src/mir_interpreter.rs',
        'semantics' => 'crates/doriac/src/semantics.rs',
        'backend' => 'crates/doriac/src/backend.rs',
        'cranelift' => 'crates/doriac/src/codegen_cranelift.rs',
        'llvm' => 'crates/doriac/src/codegen_llvm.rs',
        'php' => 'crates/doriac/src/codegen_php.rs',
        'executionTests' => 'crates/doriac/tests/stage30d_closure_execution_tests.rs',
        'malformedTests' => 'crates/doriac/tests/stage30d_malformed_mir_tests.rs',
        'manifestTests' => 'crates/doriac/tests/stage30d_debug_manifest_tests.rs',
        'manifest' => 'crates/doriac/tests/fixtures/debug_closures/manifest.txt',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    foreach (['decision', 'spec', 'plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 30d Closure HIR/MIR And Interpreter Oracle',
            'Complete',
            'Stage 30e Native Execution',
            'Stage 30f PHP Compatibility',
            'Stage 30g List Algorithms',
            'Stage 30',
            'Complete',
        ]);
    }
    $require($paths['decision'], $files['decision'], [
        'Authority Accepted; Stages 30a Through 30h Implemented; Stage 30 Complete',
    ]);
    $require($paths['audit'], $files['audit'], [
        'Historical and reserved after Stage 30h',
        'no active emitter or generic fallback remains',
    ]);

    $require($paths['hir'], $files['hir'], [
        'pub struct ClosureExpression',
        'pub struct CallableCall',
        'Expr::Closure',
        'Expr::CallableCall',
    ]);
    $require($paths['mir'], $files['mir'], [
        'pub struct FunctionTypeId',
        'NullableFunction(FunctionTypeId)',
        'pub struct ClosureDescriptor',
        'pub struct ClosureEnvironmentLayout',
        'BindClosureEnvironment',
        'IndirectCall',
        'CheckedIndirectCall',
        'DropFunction',
    ]);
    $require($paths['lowering'], $files['lowering'], [
        'lower_closure_function',
        'hir::Expr::Closure(closure)',
        'hir::Expr::CallableCall(call)',
        'logical_release_order',
        'hidden_environment',
    ]);
    $require($paths['validation'], $files['validation'], [
        'validate_closure_metadata',
        'validate_indirect_call',
        'reverse logical release order',
        'synthetic closure hidden environment',
    ]);
    $require($paths['interpreter'], $files['interpreter'], [
        'struct ClosureEnvironmentHandle',
        'enum InterpreterPlace',
        'FrameLocal',
        'EnvironmentField',
        'closure_environment_allocations',
        'descriptor.invocation_mode != invocation_mode',
        'FunctionInvocationMode::Once',
    ]);
    $forbid($paths['interpreter'], $files['interpreter'], [
        'Rc<RefCell<ClosureEnvironmentValue',
        'type SharedClosureEnvironment',
    ]);

    foreach (['semantics', 'lowering', 'validation', 'interpreter'] as $key) {
        $forbid($paths[$key], $files[$key], ['"E0641"']);
    }
    $require($paths['backend'], $files['backend'], [
        'Some(lower_validated_mir(program)?)',
        'codegen_php::generate(program, mir.as_ref())',
    ]);
    $forbid($paths['backend'], $files['backend'], [
        'Closure PHP Output Is Not Yet Available',
        'Diagnostic::unsupported_stage("E0641"',
    ]);
    $forbid($paths['backend'], $files['backend'], ['Closure Native Execution Is Not Yet Available']);
    $forbid($paths['cranelift'], $files['cranelift'], ['before the Stage 30e boundary']);
    $forbid($paths['llvm'], $files['llvm'], ['before the Stage 30e boundary']);
    $require($paths['php'], $files['php'], [
        'PhpClosurePlan',
        'validated MIR must back executable PHP closures',
    ]);

    $require($paths['executionTests'], $files['executionTests'], [
        'debug_preserves_readonly_and_writable_capture_places',
        'debug_propagates_and_catches_checked_closure_effects',
        'executable_closures_reach_native_and_php_compatibility',
        'type_only_function_syntax_does_not_trigger_target_boundaries',
    ]);
    $require($paths['malformedTests'], $files['malformedTests'], [
        'rejects_malformed_function_type_and_descriptor_tables',
        'rejects_malformed_environment_layouts_and_release_plans',
        'rejects_malformed_synthetic_closure_functions',
        'rejects_malformed_closure_construction_plans',
        'rejects_malformed_indirect_call_plans',
        'rejects_checked_and_unchecked_indirect_call_mismatches',
    ]);
    $require($paths['manifestTests'], $files['manifestTests'], [
        'durable_debug_closure_manifest_covers_every_fixture',
        'durable_debug_closure_manifest_executes_through_the_interpreter',
    ]);
    $require($paths['manifest'], $files['manifest'], [
        'no_capture',
        'writable_capture',
        'nested_factory',
        'checked_effect',
        'nullable_function',
    ]);

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage30d_closure_execution(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 30d closure execution check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 30d closure execution check passed\n");
}
