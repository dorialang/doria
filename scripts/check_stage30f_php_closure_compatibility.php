<?php

declare(strict_types=1);

(static function (): void {
    $root = dirname(__DIR__);
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = file_get_contents($root . '/' . $path);
        if ($contents === false) {
            $failures[] = "cannot read {$path}";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path} is missing `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path} still contains `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md',
        'spec' => 'SPEC.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'audit' => 'docs/notes/temporary-language-restrictions-audit.md',
        'backend' => 'crates/doriac/src/backend.rs',
        'planSource' => 'crates/doriac/src/php_closure.rs',
        'php' => 'crates/doriac/src/codegen_php.rs',
        'tests' => 'crates/doriac/tests/stage30f_php_closure_tests.rs',
        'manifest' => 'crates/doriac/tests/fixtures/php_closures/manifest.txt',
        'catalogue' => 'crates/doria-diagnostic-catalogue/src/lib.rs',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    foreach (['decision', 'spec', 'plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 30f PHP Compatibility',
            'Complete',
            'Stage 30g List Algorithms',
            'Next',
            'Stage 30',
            'Not Complete',
        ]);
    }
    $require($paths['decision'], $files['decision'], [
        'Authority Accepted; Stages 30a Through 30g Implemented; Stage 30h Next; Stage 30 Not Complete',
    ]);
    $require($paths['audit'], $files['audit'], [
        'No valid supported plain closure route emits it',
        'catalogue identity retained until Stage 30h',
    ]);

    $require($paths['backend'], $files['backend'], [
        'program.semantic_info.closures.is_empty()',
        'program.semantic_info.callable_value_calls.is_empty()',
        'Some(lower_validated_mir(program)?)',
        'codegen_php::generate(program, mir.as_ref())',
    ]);
    $forbid($paths['backend'], $files['backend'], [
        'Closure PHP Output Is Not Yet Available',
        'PHP closure lowering lands in Stage 30f',
        'Diagnostic::unsupported_stage("E0641"',
        'reject_executable_closure_hir_route',
    ]);

    $require($paths['planSource'], $files['planSource'], [
        'pub(crate) struct PhpClosurePlan',
        'BindingId',
        'ClosureId',
        'CaptureAcquisitionKind::WritableLease',
        'CaptureAcquisitionKind::MoveIntoEnvironment',
        'capture.source_binding_id',
        'descriptor.debug_identity.clone()',
    ]);
    $require($paths['php'], $files['php'], [
        'interface __DoriaFunctionValue',
        'final class __DoriaCell',
        'final class {} implements __DoriaFunctionValue',
        'format!("final class {name}")',
        'layout.logical_release_order',
        '__doria_take_cell',
        '__doria_drop_cell',
        '__doria_replace_cell',
        'new __DoriaCell(__doria_take_cell({cell}))',
        '$__doria_generated_closure_frames',
        'validated MIR must back executable PHP closures',
    ]);
    $forbid($paths['php'], $files['php'], [
        'call_user_func(',
        'call_user_func_array(',
        'PHP closure route reached code generation',
    ]);

    $require($paths['tests'], $files['tests'], [
        'php_closure_manifest_matches_the_interpreter_and_generated_php',
        'generated_php_uses_explicit_environments_cells_and_exact_carriers',
        'no_capture_php_closure_has_no_environment_and_property_drop_uses_a_temporary',
        'Command::new("php")',
        '.arg("-l")',
        '__doriaClosureEntry',
        '__DoriaClosureValue',
    ]);
    $require($paths['manifest'], $files['manifest'], [
        'compatibility_matrix',
        'taking_copy_capture',
        'taking_move_capture',
        'once_return',
        'nullable_function',
        'property_replacement',
        'payload_enum_storage',
        'checked_effect',
        'checked_once',
        'panic_no_cleanup',
    ]);
    $require($paths['catalogue'], $files['catalogue'], ['"E0641"']);

    if ($failures !== []) {
        fwrite(STDERR, "Stage 30f PHP closure compatibility check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Stage 30f PHP closure compatibility check passed.\n");
})();
