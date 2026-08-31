<?php

declare(strict_types=1);

/**
 * @return list<string>
 */
function check_ambient_io_and_fallible_finalizers(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required ambient-I/O/finalizer authority is missing";
            return '';
        }

        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing ambient-I/O/finalizer contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: obsolete ambient-I/O/finalizer contract `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0123-ambient-canonical-io-effects-and-fallible-finalizer-precedence.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'spec' => 'SPEC.md',
        'readme' => 'README.md',
        'effects' => 'crates/doriac/src/checked_effects.rs',
        'builtins' => 'crates/doriac/src/builtins.rs',
        'semantics' => 'crates/doriac/src/semantics.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'lowering' => 'crates/doriac/src/mir_lowering.rs',
        'validation' => 'crates/doriac/src/mir_validation.rs',
        'php' => 'crates/doriac/src/codegen_php.rs',
        'tests' => 'crates/doriac/tests/checked_error_tests.rs',
        'validationTests' => 'crates/doriac/tests/mir_validation_tests.rs',
        'algorithmTests' => 'crates/doriac/tests/stage30g_list_algorithm_tests.rs',
        'graphTests' => 'crates/doriac/tests/stage31_package_graph_tests.rs',
        'phpTests' => 'crates/doriac/tests/codegen_php_tests.rs',
        'parity' => 'crates/doriac/tests/fixtures/native_parity_examples.txt',
        'fixture' => 'examples/native/main_ambient_io_fallible_finalizers.doria',
        'callbackFixture' => 'examples/native/main_ambient_io_callbacks.doria',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $require($paths['decision'], $files['decision'], [
        '**Status:** Accepted',
        'Doria\\Std\\Io\\IoError',
        'Doria\\Std\\Io\\InvalidUtf8Error',
        'attached to a `try`',
        'supersedes',
        'E0632',
        'Historical And Reserved',
    ]);
    foreach (['plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Ambient I/O And Fallible Finalizer Corrective Beat — Complete',
            'E0632 — Historical And Reserved',
        ]);
    }
    foreach (['spec', 'readme'] as $key) {
        $require($paths[$key], $files[$key], [
            'Doria\\Std\\Io\\IoError',
            'Doria\\Std\\Io\\InvalidUtf8Error',
            'ambient',
        ]);
    }
    $require($paths['effects'], $files['effects'], [
        'pub enum CheckedEffectClass',
        'AmbientIo',
        'pub fn is_ambient_io_effect',
    ]);
    $require($paths['builtins'], $files['builtins'], [
        'pub const fn ambient_error_types(',
        'pub const fn required_error_types(',
    ]);
    $require($paths['semantics'], $files['semantics'], [
        'ambient_effect_seed',
        'callable_observed_checked_effects',
        'fn inferred_ambient_effects(',
        'fn apply_ambient_effect_seed(',
        'required_checked_effects:',
        'ambient_checked_effects:',
    ]);
    $forbid($paths['semantics'], $files['semantics'], ['"E0632"']);
    $require($paths['hir'], $files['hir'], [
        'pub required_checked_effects:',
        'pub ambient_checked_effects:',
    ]);
    $require($paths['mir'], $files['mir'], [
        'pub required_checked_effects:',
        'pub ambient_checked_effects:',
        'pub struct FinalizerReplacementPlan',
        'pub replacements: Vec<FinalizerReplacementPlan>',
    ]);
    $require($paths['lowering'], $files['lowering'], [
        'fn replace_finalizer_pending_exit(',
        'drop_obligation_for_pending_payload',
        'FinalizerReplacementPlan',
    ]);
    $require($paths['validation'], $files['validation'], [
        'validate_ambient_checked_effects',
        'finalizer replacement',
        'replacement Error',
    ]);
    $require($paths['php'], $files['php'], [
        'private bool $__doriaLive = true',
        'public function takeError(): __DoriaErrorValue',
        '$error = $this->error;',
        '$this->error = null;',
        '__doria_drop_value($error)',
    ]);
    $require($paths['tests'], $files['tests'], [
        'canonical_io_is_ambient_in_source_but_retained_in_hir_and_mir',
        'only_exact_compiler_known_io_errors_are_ambient',
        'structural_callables_ignore_ambient_identity_but_keep_ambient_transport',
        'fallible_finalizers_escape_same_try_catches_and_reach_outer_catches',
        'finalizer_error_replaces_pending_return_and_earlier_error',
    ]);
    $require($paths['validationTests'], $files['validationTests'], [
        'shared_validator_rejects_malformed_ambient_profiles_and_finalizer_replacement',
    ]);
    $require($paths['algorithmTests'], $files['algorithmTests'], [
        'ambient_list_callbacks_keep_checked_transport_and_validate_their_effect_profile',
        'Terminator::CheckedIndirectCall',
    ]);
    $require($paths['graphTests'], $files['graphTests'], [
        'ambient_and_finalizer_effects_flow_across_source_graphs',
    ]);
    $require($paths['phpTests'], $files['phpTests'], [
        'ambient-io-fallible-finalizers.doria',
        'php_backend_preserves_ambient_effects_through_callbacks_and_list_algorithms',
    ]);
    $require($paths['parity'], $files['parity'], [
        'examples/native/main_ambient_io_callbacks.doria',
        'examples/native/main_ambient_io_fallible_finalizers.doria',
    ]);
    $require($paths['fixture'], $files['fixture'], [
        'function replaceNormal()',
        'function replaceReturn()',
        'function replaceWhen()',
        'function replaceBreak()',
        'function replaceContinue()',
        'function replaceError()',
        'function replaceNested()',
        'function preserveReturnWithLocalCatch()',
    ]);
    $require($paths['callbackFixture'], $files['callbackFixture'], [
        'function(): void $callback = function (): void',
        'take function(): void $callback',
        'List<function(): void>',
        '$values->map(',
        '$mapped->filter(',
        '$filtered->reduce(',
    ]);

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_ambient_io_and_fallible_finalizers(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Ambient I/O and fallible finalizer check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Ambient I/O and fallible finalizer check passed\n");
}
