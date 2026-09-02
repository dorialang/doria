<?php

declare(strict_types=1);

/** @return list<string> */
function check_constructor_parameter_roles(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required constructor-role authority is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing constructor-role contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains forbidden constructor-role surface `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0131-constructor-property-overrides-and-constructor-only-parameters.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'lexer' => 'crates/doriac/src/lexer.rs',
        'ast' => 'crates/doriac/src/ast.rs',
        'parser' => 'crates/doriac/src/parser.rs',
        'semantics' => 'crates/doriac/src/semantics.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'init' => 'crates/doriac/src/constructor_init.rs',
        'ownership' => 'crates/doriac/src/ownership.rs',
        'attributes' => 'crates/doriac/tests/stage32_attribute_tests.rs',
        'tests' => 'crates/doriac/tests/stage34_inheritance_tests.rs',
        'php' => 'crates/doriac/tests/codegen_php_tests.rs',
        'parity' => 'crates/doriac/tests/fixtures/native_parity_examples.txt',
        'fixture' => 'examples/native/main_constructor_parameter_roles.doria',
        'catalogue' => 'crates/doria-diagnostic-catalogue/src/lib.rs',
        'metadata' => 'docs/attribute-metadata-protocol.md',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $require($paths['decision'], $files['decision'], [
        '# Decision 0131:',
        '**Status:** Accepted',
        '`parameter` is a reserved keyword',
        '`param` remains an identifier',
        'zero object fields',
        'E0727 remains',
        'schemas 1, 2, and 3',
        'processor protocol version 1',
        'next free decision number remains unused',
    ]);
    $require($paths['lexer'], $files['lexer'], ['Parameter,', '"parameter" => TokenKind::Parameter']);
    $forbid($paths['lexer'], $files['lexer'], ['"param" => TokenKind::Parameter']);
    foreach (['ast', 'hir'] as $key) {
        $require($paths[$key], $files[$key], [
            'ConstructorParameterRole',
            'InheritedPropertyOverride',
            'ConstructorOnly',
            'Promoted',
        ]);
    }
    $require($paths['parser'], $files['parser'], [
        'AttributeTargetRole::PromotedProperty',
        'constructor_role.is_promoted()',
        'Constructor Parameter Roles Conflict',
        'Constructor Parameter Role Is Duplicated',
        'Constructor Parameter Role Is Misordered',
    ]);
    $require($paths['semantics'], $files['semantics'], [
        'PropertyFamilySemanticInfo',
        'ConstructorParameterSemanticRole',
        'Constructor Parameter Role Is Required',
        'E0727',
        'E0741',
        'E0742',
        'E0743',
    ]);
    $require($paths['init'], $files['init'], ['constructor-only parameter', 'constructor_role.is_promoted()']);
    $require($paths['ownership'], $files['ownership'], ['constructor_role.is_promoted()']);
    $require($paths['attributes'], $files['attributes'], [
        'constructor_parameter_attributes_follow_storage_roles_exactly',
        'AttributeTargetRole::PromotedProperty',
    ]);
    $require($paths['tests'], $files['tests'], [
        'parameter_is_a_keyword_without_reserving_param_or_text_occurrences',
        'constructor_only_move_parameters_use_ordinary_borrowing_rules',
        'constructor_parameter_roles_reuse_storage_or_create_no_storage',
    ]);
    $require($paths['php'], $files['php'], ['php_backend_emits_constructor_roles_as_ordinary_parameters']);
    $require($paths['parity'], $files['parity'], ['main_constructor_parameter_roles']);
    $require($paths['fixture'], $files['fixture'], ['override take', 'parameter string']);
    $require($paths['catalogue'], $files['catalogue'], ['E0737', 'E0744']);
    $require($paths['metadata'], $files['metadata'], ['schema version 1', 'schema version 2', 'schema version 3']);
    $require($paths['plan'], $files['plan'], [
        'Constructor Parameter Roles',
        'Indexed Foreach And Scalar Display',
        'decision remains unauthored',
        'Stage 35 — Interfaces And Traits — Next',
    ]);
    $require($paths['pipeline'], $files['pipeline'], [
        'Stage 34 Single Class Inheritance — Complete.',
        'Constructor Parameter Roles Corrective Beat',
        'Stage 35 Interfaces And Traits — Next.',
    ]);
    $forbid($paths['decision'], $files['decision'], [
        'class-body property overrides are implemented',
        'runtime reflection is required',
        'Stage 35 is implemented',
    ]);
    if (glob($root . '/docs/decisions/0132-*.md') !== []) {
        $failures[] = 'docs/decisions: Decision 0132 must remain unused during this corrective beat';
    }
    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_constructor_parameter_roles(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Constructor parameter roles check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Constructor parameter roles check passed\n");
}
