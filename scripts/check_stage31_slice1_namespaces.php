<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage31_slice1_namespaces(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 31 Slice 1 file is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 31 Slice 1 contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains forbidden Stage 31 Slice 1 route `{$needle}`";
            }
        }
    };

    $paths = [
        'decision0028' => 'docs/decisions/0028-namespaces-use-include-and-directives.md',
        'decision0117' => 'docs/decisions/0117-namespaces-compile-time-autoloading-hybrid-source-layout-and-package-compilation-graphs.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'source' => 'crates/doriac/src/source.rs',
        'lexer' => 'crates/doriac/src/lexer.rs',
        'parser' => 'crates/doriac/src/parser.rs',
        'names' => 'crates/doriac/src/names.rs',
        'lib' => 'crates/doriac/src/lib.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'php' => 'crates/doriac/src/codegen_php.rs',
        'tests' => 'crates/doriac/tests/stage31_namespace_tests.rs',
        'fixture' => 'examples/native/main_stage31_namespaces.doria',
        'manifest' => 'crates/doriac/tests/fixtures/native_parity_examples.txt',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    foreach (['decision0028', 'decision0117'] as $key) {
        $require($paths[$key], $files[$key], ['Accepted', 'Stage 31 Slice 1']);
    }
    foreach (['plan', 'pipeline', 'decision0117'] as $key) {
        $require($paths[$key], $files[$key], [
            'Stage 30',
            'Complete',
            'Stage 31 Slice 1',
            'Stage 31 Slice 2',
            'Complete',
            'Stage 31',
            'Stage 32',
            'Complete',
            'Stage 33 Slice 1',
            'Complete',
            'Stage 33 Slice 2',
            'Complete',
            'Stage 33 Slice 3',
            'Next',
            'Stage 33',
            'In Progress, Not Complete',
            'E0641',
            'Historical',
        ]);
    }

    $require($paths['source'], $files['source'], [
        'pub struct NameSegmentRef',
        'pub struct QualifiedNameRef',
        'pub segments: Vec<NameSegmentRef>',
        'pub separator_spans: Vec<Span>',
    ]);
    $require($paths['lexer'], $files['lexer'], [
        '"use" => TokenKind::Use',
        '"include" => TokenKind::Include',
    ]);
    $require($paths['parser'], $files['parser'], [
        'fn parse_namespace(',
        'fn parse_use_decl(',
        'fn parse_include_decl(',
        'Wildcard Import Is Not Supported',
        'Leading Namespace Separator Is Not Supported',
    ]);
    $require($paths['names'], $files['names'], [
        'pub enum Edition',
        'pub enum PackageIdentity',
        'pub struct SourceIdentity',
        'pub struct CompilationContext',
        'pub struct GlobalSymbolId',
        'pub struct GlobalSymbolDeclaration',
        'pub struct GlobalSymbolReference',
        'pub const EDITION_2026_PRELUDE',
        'fn resolve_name(',
        'self.imports.get(source_name)',
        'self.namespace.as_ref()',
        'edition_prelude(self.context.edition)',
        'if source_name.contains',
        '"E0681"',
    ]);
    $forbid($paths['names'], $files['names'], [
        'EXTERNAL_SYMBOL_BOUNDARY_CODE',
        'INCLUDE_BOUNDARY_CODE',
    ]);
    $require($paths['lib'], $files['lib'], [
        'resolve_program_for_ide',
        'check_source_with_context',
        'lower_source_with_context',
        'lower_source_to_mir_with_context',
        'compile_source_with_context',
    ]);
    $require($paths['hir'], $files['hir'], ['pub semantic_info: crate::semantics::SemanticInfo']);
    $require($paths['mir'], $files['mir'], ['CompilationContext', 'GlobalSymbolFacts']);
    $require($paths['php'], $files['php'], ['php_symbol_name', "SCRIPT_FILENAME", '__FILE__']);
    $require($paths['tests'], $files['tests'], [
        'one_resolver_canonicalizes_every_name_bearing_semantic_role',
        'canonical_io_requires_qualification_or_an_explicit_import',
        'generated_php_executes_namespaced_main_and_keeps_imports_compile_time_only',
        'resolver_scaling_is_structural_and_deterministic',
    ]);
    $require($paths['fixture'], $files['fixture'], [
        'namespace Acme\Parity;',
        'use Acme\Parity\{',
        'use Doria\Std\Io\IoError;',
    ]);
    $require($paths['manifest'], $files['manifest'], ['main_stage31_namespaces']);
    $forbid($paths['lib'], $files['lib'], ['Baton.toml']);
    $forbid($paths['hir'], $files['hir'], ['Item::Include', 'IncludeDecl']);
    $forbid($paths['mir'], $files['mir'], ['Statement::Include', 'IncludeDecl']);
    $forbid($paths['php'], $files['php'], ['spl_autoload_register']);

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage31_slice1_namespaces(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 31 Slice 1 namespace check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 31 Slice 1 namespace check passed\n");
}
