<?php

declare(strict_types=1);

/** @return list<string> */
function check_stage31_slice2_package_graph(string $root): array
{
    $failures = [];
    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required Stage 31 Slice 2 file is missing";
            return '';
        }
        return $contents;
    };
    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing Stage 31 Slice 2 contract `{$needle}`";
            }
        }
    };
    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: contains forbidden Stage 31 Slice 2 route `{$needle}`";
            }
        }
    };

    $paths = [
        'decision' => 'docs/decisions/0117-namespaces-compile-time-autoloading-hybrid-source-layout-and-package-compilation-graphs.md',
        'plan' => 'docs/doria-end-to-end-plan.md',
        'pipeline' => 'docs/notes/current-pipeline.md',
        'schema' => 'docs/build-plan-schema.md',
        'buildPlan' => 'crates/doriac/src/build_plan.rs',
        'graph' => 'crates/doriac/src/compilation_graph.rs',
        'provider' => 'crates/doriac/src/source_provider.rs',
        'sourceMap' => 'crates/doriac/src/source_map.rs',
        'incremental' => 'crates/doriac/src/incremental.rs',
        'lib' => 'crates/doriac/src/lib.rs',
        'cli' => 'crates/doriac/src/main.rs',
        'hir' => 'crates/doriac/src/hir.rs',
        'mir' => 'crates/doriac/src/mir.rs',
        'mirValidation' => 'crates/doriac/src/mir_validation.rs',
        'tests' => 'crates/doriac/tests/stage31_package_graph_tests.rs',
        'parity' => 'crates/doriac/tests/stage31_package_parity_tests.rs',
        'manifest' => 'crates/doriac/tests/fixtures/stage31_package_graph/manifest.txt',
    ];
    $files = [];
    foreach ($paths as $key => $path) {
        $files[$key] = $read($path);
    }

    foreach (['decision', 'plan', 'pipeline'] as $key) {
        $require($paths[$key], $files[$key], [
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
            'Complete',
            'Stage 33',
            'Complete',
            'Phase F',
            'Stage 34',
            'E0671',
            'E0672',
            'Historical',
            'Reserved',
        ]);
    }
    $require($paths['schema'], $files['schema'], [
        'schemaVersion',
        'selectedTarget',
        'activeScopes',
        'generatedFor',
        'normal',
        'development',
        'include-once',
        'Complete And Partial Graphs',
        'doriac check --build-plan',
        'Baton.toml',
        'does not ask `doriac` to scan',
    ]);
    $require($paths['buildPlan'], $files['buildPlan'], [
        'BUILD_PLAN_SCHEMA_VERSION',
        'deny_unknown_fields',
        'pub struct BuildPlan',
        'pub enum SourceScope',
        'pub enum SourceOrigin',
        'pub fn parse_build_plan',
        'pub fn encode_build_plan',
    ]);
    $require($paths['graph'], $files['graph'], [
        'pub struct CompilationGraph',
        'pub struct GraphSource',
        'pub struct GraphLoadOptions',
        'pub enum ProjectStructureAuthority',
        'pub struct IncludeEdge',
        'pub fn load_compilation_graph',
        'pub fn load_compilation_graph_with_options',
        'pub fn analyze_compilation_graph_for_ide',
        'validate_package_cycles',
        'validate_resolved_source_shape',
        'validate_layout',
        'visible_packages',
        'direct_normal_dependencies',
        'direct_development_dependencies',
        'Duplicate Fully Qualified Declaration',
        'graph_fingerprint',
    ]);
    $require($paths['provider'], $files['provider'], [
        'pub trait SourceProvider',
        'pub struct FileSystemSourceProvider',
        'pub struct InMemorySourceProvider',
        'canonicalization resolves outside the package root',
        'path_uses_exact_case',
    ]);
    $require($paths['incremental'], $files['incremental'], [
        'pub struct CompilationSession',
        'load_graph_with_options',
        'body_only_changed_sources',
        'reused_declaration_indexes',
        'semantic_dependency_fingerprint',
        'reverse_include_dependencies',
        'backend_input_fingerprint',
        'declaration_fingerprint',
        'source_context_fingerprint',
    ]);
    $require($paths['lib'], $files['lib'], [
        'check_build_plan_file',
        'lower_build_plan_file',
        'compile_build_plan_file',
        'lower_compilation_graph_to_mir',
        'compile_compilation_graph',
    ]);
    $require($paths['cli'], $files['cli'], [
        '--build-plan',
        'cannot override compiler settings from a build plan',
        'build-plan run accepts no compiler overrides',
    ]);
    $require($paths['hir'], $files['hir'], ['pub sources: Vec<SourceUnit>', 'pub packages: Vec<PackageUnit>']);
    $require($paths['mir'], $files['mir'], ['pub sources: Vec<SourceUnit>', 'pub packages: Vec<PackageUnit>', 'pub selected_entry: Option<FunctionId>']);
    $require($paths['mirValidation'], $files['mirValidation'], ['validate_graph_metadata', 'validate_global_graph_facts']);
    $require($paths['tests'], $files['tests'], [
        'compilation_session_reuses_unchanged_parses_and_invalidates_changes',
        'compilation_session_additions_reconsider_prior_unresolved_references',
        'duplicate_fqn_diagnostics_render_all_source_files',
        'include_once_loads_a_recursive_source_once',
        'transitive_dependency_is_not_visible',
        'package_internal_members_are_rejected_across_packages',
        'library_graph_lowers_without_a_process_entry',
        'graph_identity_is_independent_of_source_inventory_order',
        'partial_tooling_graphs_do_not_fabricate_project_structure_authority',
    ]);
    $require($paths['parity'], $files['parity'], ['package_graph_executes_identically_across_all_enabled_backends']);
    $require($paths['manifest'], $files['manifest'], ['cross_file_execution']);

    $forbid($paths['lib'], $files['lib'], ['Baton.toml', 'Baton.lock']);
    $forbid($paths['graph'], $files['graph'], ['walkdir', 'glob::']);
    $forbid($paths['hir'], $files['hir'], ['Item::Include', 'IncludeDecl']);
    $forbid($paths['mir'], $files['mir'], ['Statement::Include', 'IncludeDecl']);

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stage31_slice2_package_graph(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 31 Slice 2 package-graph check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }
    fwrite(STDOUT, "Stage 31 Slice 2 package-graph check passed\n");
}
