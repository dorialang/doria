<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage30e_native_closure_execution(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 30e file is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 30e contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: forbidden Stage 30e regression `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md',
        'spec' => 'SPEC.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'audit' => 'docs/notes/temporary-language-restrictions-audit.md',
        'abi' => 'crates/doriac/src/native_closure_abi.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'validation' => 'crates/doriac/src/mir_validation.rs',
        'backend' => 'crates/doriac/src/backend.rs',
        'cranelift' => 'crates/doriac/src/codegen_cranelift.rs',
        'llvm' => 'crates/doriac/src/codegen_llvm.rs',
        'runtime' => 'crates/doria-rt/src/lib.rs',
        'parityTests' => 'crates/doriac/tests/native_mir_parity_tests.rs',
        'manifest' => 'crates/doriac/tests/fixtures/native_closures/manifest.txt',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    foreach (['decision', 'spec', 'plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 30e Native Execution',
            'Complete',
            'Stage 30f PHP Compatibility',
            'Next',
            'Stage 30',
            'Not Complete',
        ]);
    }
    $require($paths['decision'], $files['decision'], [
        'Authority Accepted; Stages 30a Through 30e Implemented; Stage 30f Next; Stage 30 Not Complete',
    ]);
    $require($paths['audit'], $files['audit'], [
        'implemented for debug/native',
        'PHP Stage 30f',
    ]);

    $require($paths['abi'], $files['abi'], [
        'pub const CARRIER_WORDS: u32 = 2',
        'pub const DESCRIPTOR_WORDS: u32 = 2',
        'pub const fn carrier_layout',
        'pub const fn descriptor_layout',
        'pub fn environment_layout',
        'NativeCallableHiddenInput::Environment',
        'NativeCallableHiddenInput::BorrowHome',
        'carrier_and_descriptor_are_two_aligned_words',
        'escape_analysis_selects_no_stack_or_single_heap_environment_storage',
    ]);
    $require($paths['mir'], $files['mir'], [
        'pub enum ClosureEnvironmentPlacement',
        'Stack',
        'Heap',
        'pub return_borrow: Option<ReturnBorrow>',
    ]);
    $require($paths['validation'], $files['validation'], [
        'environment_placement',
        'infer_function_expression_return_borrow',
    ]);

    foreach (['cranelift', 'llvm'] as $key) {
        $require($paths[$key], $files[$key], [
            'declare_closure_drop_functions',
            'define_closure_drop_function',
            'lower_indirect_call',
            'lower_checked_indirect_call',
            'BindClosureEnvironment',
            'DropFunction',
            'NullableFunctionIsPresent',
            'CLOSURE_ENVIRONMENT_ALLOCATE',
            'CLOSURE_ENVIRONMENT_FREE',
        ]);
        $forbid($paths[$key], $files[$key], [
            'before the Stage 30e boundary',
            'nullable function test reached LLVM before the Stage 30e boundary',
        ]);
    }
    $require($paths['runtime'], $files['runtime'], [
        'dr_v1_closure_environment_allocate',
        'dr_v1_closure_environment_free',
        'closure_environment_allocation_is_zeroed_aligned_and_null_safe_to_free',
    ]);
    $forbid($paths['runtime'], $files['runtime'], [
        'closure_retain',
        'closure_release',
    ]);
    $require($paths['backend'], $files['backend'], [
        'Closure PHP Output Is Not Yet Available',
        'PHP closure lowering lands in Stage 30f',
        'Diagnostic::unsupported_stage("E0641"',
        'BackendTarget::Native | BackendTarget::Debug | BackendTarget::Wasm => return Ok(())',
    ]);
    $forbid($paths['backend'], $files['backend'], [
        'Closure Native Execution Is Not Yet Available',
        'native closure execution lands in Stage 30e',
    ]);

    $require($paths['parityTests'], $files['parityTests'], [
        'native_closure_manifest_covers_every_fixture',
        'interpreter_cranelift_and_enabled_llvm_match_for_native_closures',
        'NativeProfile::Fast',
        'NativeProfile::Release',
    ]);
    foreach ([
        'no_capture', 'readonly_capture', 'writable_capture', 'writable_move_captures',
        'taking_copy_capture', 'taking_move_capture', 'transferred_capture_cleanup',
        'once_return', 'nested_factory', 'nullable_function',
        'parameter_callback', 'returned_closure', 'callable_property',
        'property_replacement', 'collection_storage', 'payload_enum_storage',
        'generic_closure', 'checked_effect', 'checked_once', 'destructor_order',
        'panic_no_cleanup',
    ] as $fixture) {
        $require($paths['manifest'], $files['manifest'], [$fixture]);
    }

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage30e_native_closure_execution(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 30e native closure execution check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 30e native closure execution check passed\n");
}
