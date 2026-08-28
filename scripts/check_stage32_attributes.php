<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage32_attributes(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 32 file is missing";
            return '';
        }

        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 32 contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains forbidden Stage 32 route `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0125-typed-attributes-const-evaluated-metadata-and-processor-protocol.md',
        'batonDecision' => 'docs/decisions/0124-baton-bootstrap-doria-native-transition-and-toolchain-release-gate.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'protocol' => 'docs/attribute-metadata-protocol.md',
        'lexer' => 'crates/doriac/src/lexer.rs',
        'ast' => 'crates/doriac/src/ast.rs',
        'parser' => 'crates/doriac/src/parser.rs',
        'names' => 'crates/doriac/src/names.rs',
        'constEval' => 'crates/doriac/src/const_eval.rs',
        'attributes' => 'crates/doriac/src/attributes.rs',
        'semantics' => 'crates/doriac/src/semantics.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'incremental' => 'crates/doriac/src/incremental.rs',
        'lib' => 'crates/doriac/src/lib.rs',
        'cli' => 'crates/doriac/src/main.rs',
        'tests' => 'crates/doriac/tests/stage32_attribute_tests.rs',
        'cliTests' => 'crates/doriac/tests/cli_tests.rs',
        'phpTests' => 'crates/doriac/tests/codegen_php_tests.rs',
        'parity' => 'crates/doriac/tests/fixtures/native_parity_examples.txt',
        'fixture' => 'examples/native/main_stage32_attributes.doria',
        'acceptedExample' => 'examples/metadata/attributes.doria',
        'invalidExample' => 'examples/errors/invalid_attribute_values.doria',
        'misplacedExample' => 'examples/errors/misplaced_attribute.doria',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    $require($paths['decision'], $files['decision'], [
        '**Status:** Accepted',
        '**Implementation Status:** Implemented By Stage 32',
        'Applying an attribute never constructs the class',
        'Stage 33 Slice 1',
        'Stage 41',
    ]);
    $require($paths['batonDecision'], $files['batonDecision'], ['**Status:** Accepted']);
    foreach (['plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 30',
            'Complete',
            'Stage 31',
            'Stage 32',
            'Stage 33 Slice 1',
            'Complete',
            'Stage 33 Slice 2',
            'Next',
            'E0632',
            'E0641',
            'E0671',
            'E0672',
            'historical',
            'reserved',
        ]);
    }
    $require($paths['protocol'], $files['protocol'], [
        'schema-version-1 JSON document',
        'Processor Request',
        'Processor Response',
        '### Generated sources',
        'Stage 33 Slice 3',
        'Stage 41',
    ]);

    $require($paths['lexer'], $files['lexer'], [
        'AttributeOpen',
        "b'#' if self.match_byte(b'[')",
        "self.peek() == Some(b'#') && self.peek_next() != Some(b'[')",
    ]);
    $require($paths['ast'], $files['ast'], [
        'pub struct AttributeGroup',
        'pub struct AttributeArgumentList',
        'pub arguments: Vec<Argument>',
        'pub struct AttributeAttachment',
        'pub enum AttributeTargetKind',
        'pub enum AttributeTargetRole',
    ]);
    $require($paths['parser'], $files['parser'], [
        'TokenKind::AttributeOpen',
        'parse_attribute_groups',
        'parse_argument_list_after_open',
        'Attribute Must Precede Declaration Modifiers',
    ]);
    $require($paths['names'], $files['names'], [
        'AttributeClass',
        'COMPILER_KNOWN_ATTRIBUTES',
        '["Attribute", "Test", "PHPExport"]',
    ]);
    $require($paths['constEval'], $files['constEval'], ['pub fn evaluate_attribute_value']);
    $require($paths['attributes'], $files['attributes'], [
        'pub struct AttributeSemanticInfo',
        'pub enum AttributeValueKind',
        'pub struct AttributeMetadataDocumentV1',
        'pub struct AttributeProcessorRequestV1',
        'pub struct AttributeProcessorResponseV1',
        'pub fn validate_processor_request',
        'pub fn parse_processor_request_json',
        'pub fn parse_processor_response_json',
        'deny_unknown_fields',
        'graph fingerprint does not match',
        'reserved by the compiler',
        'unsafe terminal control bytes',
    ]);
    $require($paths['semantics'], $files['semantics'], [
        'AttributeSemanticInfo',
        'attribute_metadata_type_is_compatible',
        'crate::arg_binding::bind_arguments',
        'evaluate_attribute_value',
        'Attribute Class Cannot Be Generic',
        'Attribute Schema Parameter Must Be Readonly',
    ]);
    $require($paths['hir'], $files['hir'], ['pub attribute_metadata: crate::attributes::AttributeSemanticInfo']);
    $forbid($paths['mir'], $files['mir'], ['AttributeMetadata', 'AttributeApplication', 'AttributeRegistry']);
    $require($paths['incremental'], $files['incremental'], [
        'for attachment in &source.authored.attributes',
        'declaration_fingerprint',
    ]);
    $require($paths['lib'], $files['lib'], [
        'pub fn metadata_source',
        'pub fn metadata_compilation_graph',
        'pub fn metadata_build_plan_file',
    ]);
    $require($paths['cli'], $files['cli'], ['"metadata"', 'metadata_compilation_graph', 'metadata_source']);

    $require($paths['tests'], $files['tests'], [
        'adjacent_attribute_opening_does_not_change_hash_comments_or_strings',
        'parser_attaches_attributes_to_the_complete_stage32_target_surface',
        'semantic_attributes_bind_named_arguments_and_defaults_without_execution',
        'metadata_document_is_strict_deterministic_typed_and_runtime_free',
        'namespaced_dependency_attributes_use_the_shared_graph_resolver',
        'internal_attribute_visibility_uses_package_identity',
        'transitive_attribute_dependencies_remain_hidden',
        'incremental_fingerprints_track_attribute_surfaces_and_schema_dependencies',
        'metadata_values_preserve_exact_scalar_nullable_and_enum_types',
        'runtime_expressions_and_nonconstant_defaults_never_enter_metadata',
    ]);
    $require($paths['cliTests'], $files['cliTests'], ['metadata_command_is_deterministic_for_standalone_and_build_plan_inputs']);
    $require($paths['phpTests'], $files['phpTests'], ['stage32_attributes_are_metadata_only_in_php_output']);
    $require($paths['parity'], $files['parity'], ['examples/native/main_stage32_attributes.doria']);
    $require($paths['fixture'], $files['fixture'], ['#[Attribute]', '#[Test]', '#[PHPExport]']);
    $require($paths['acceptedExample'], $files['acceptedExample'], [
        '#[Attribute]',
        '#[Route(path:',
        '#[Test]',
        '#[PHPExport]',
    ]);
    $require($paths['invalidExample'], $files['invalidExample'], [
        '#[Unmarked]',
        'runtimePath()',
        'read_file(',
        'new Unmarked()',
        'Factory::create()',
        '#[Route([',
    ]);
    $require($paths['misplacedExample'], $files['misplacedExample'], ['#[Test]', 'let $value = 1']);

    $forbid($paths['lib'], $files['lib'], ['Baton.toml', 'Baton.lock']);
    foreach (['crates/doriac/src/codegen_php.rs', 'crates/doriac/src/codegen_cranelift.rs'] as $path) {
        $contents = $read($path);
        $forbid($path, $contents, ['PHPExport', 'AttributeRegistry', 'TestRunner']);
    }

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage32_attributes(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 32 attribute check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Stage 32 attribute check passed\n");
}
