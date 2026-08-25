<?php

declare(strict_types=1);

/**
 * @return list<string>
 */
function check_closure_capture_authority(string $root): array
{
    $failures = [];

    $read = static function (string $path) use ($root, &$failures): string {
        $contents = @file_get_contents($root . '/' . $path);
        if (!is_string($contents)) {
            $failures[] = "{$path}: required file is missing or unreadable";
            return '';
        }

        return $contents;
    };

    $require = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (!str_contains($contents, $needle)) {
                $failures[] = "{$path}: missing closure-capture authority `{$needle}`";
            }
        }
    };

    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: forbidden stale closure-capture authority `{$needle}`";
            }
        }
    };

    $decisionPath = 'docs/decisions/0120-explicit-closure-capture-lists.md';
    $planPath = 'docs/doria-end-to-end-plan.md';
    $specPath = 'SPEC.md';
    $pipelinePath = 'docs/notes/current-pipeline.md';
    $auditPath = 'docs/notes/plan-open-questions-audit.md';
    $examplesPath = 'examples/future/stage30/README.md';
    $lexerPath = 'crates/doriac/src/lexer.rs';
    $astPath = 'crates/doriac/src/ast.rs';
    $typesPath = 'crates/doriac/src/types.rs';
    $parserPath = 'crates/doriac/src/parser.rs';
    $semanticsPath = 'crates/doriac/src/semantics.rs';
    $cataloguePath = 'crates/doria-diagnostic-catalogue/src/lib.rs';
    $testsPath = 'crates/doriac/tests/pre_stage30_closure_grammar_tests.rs';
    $fixturePath = 'crates/doriac/tests/fixtures/accepted_syntax/closures.doria';

    $decision = $read($decisionPath);
    $plan = $read($planPath);
    $spec = $read($specPath);
    $pipeline = $read($pipelinePath);
    $audit = $read($auditPath);
    $examples = $read($examplesPath);
    $lexer = $read($lexerPath);
    $ast = $read($astPath);
    $types = $read($typesPath);
    $parser = $read($parserPath);
    $semantics = $read($semanticsPath);
    $catalogue = $read($cataloguePath);
    $tests = $read($testsPath);
    $fixture = $read($fixturePath);

    $require($decisionPath, $decision, [
        '# Decision 0120: Explicit Closure Capture Lists',
        '**Status:** Accepted',
        'All captures of surrounding local bindings are explicit.',
        'anonymous block functions use the same `with` capture list',
        'Copy values still require an explicit capture',
        'Move values still require an explicit capture',
        'There is no automatic capture for arrows, anonymous functions, Copy locals,',
        'A closure that uses no surrounding local omits `with`',
        '`with ($value)` borrows the binding readonly',
        '`with (writable $value)` takes an exclusive writable borrow',
        '`with (take $value)` transfers ownership',
        'namespace-import keyword and is not a closure alias',
        'Changing an arrow closure into an anonymous block closure must not change its',
        'Closure Must Capture `$minimum`',
        '## Pre-Stage-30 Grammar Slice',
        'two-clocks rule requires accepted syntax to parse',
        'anonymous-function expression tokens and productions',
        'source-preserving AST nodes',
        'accepted-syntax regression tests',
        '`E0641` is historical and remains reserved',
        'Stage 30b now validates free variables',
        'pre-Stage-30 grammar slice is complete',
        'Decision 0121 settles the questions this record deliberately left bounded',
        'Stage 30 consumes the source-preserving closure AST',
        'Decision 0119 owns source-ordered checked-effect sets',
        'Decision 0121 applies that model to closure function',
        'Decision 0121 settles the questions this record deliberately left bounded',
        'The later structured-concurrency stage owns async closures',
        'The audit found no accepted grant of',
        'runtime reflection occurs',
        'does not use PHP arrow automatic capture',
        'Rust bootstrap representation does not define the language model',
    ]);

    $require($planPath, $plan, [
        'Both forms require a `with` list when they reference enclosing local bindings',
        'Copy, readonly, writable, and Move bindings have no implicit-capture exception',
        'A closure with no surrounding-local dependency omits `with`',
        'Changing an arrow into a block closure preserves its capture list and ownership modes',
        'Decision 0120: explicit closure capture lists',
        '**Pre-Stage-30 Grammar Slice — Closure accepted syntax — Complete.**',
        'authoritative `function(T): R` type syntax',
        'catalogued `E0641` Stage 30 boundary',
        'No free-variable discovery',
        '**Stage 30 Closure Authority — Accepted And Implemented; Stage 30 — Complete.**',
        'Missing, duplicate, wrong-mode, unused, moved, and insufficient-lifetime captures',
        'Function types preserve checked effects',
        '`List<T>` alone receives `map`, Copy-only preserving `filter`, and writable-accumulator `reduce`',
        'Copy/Move values have no implicit exception',
    ]);
    $forbid($planPath, $plan, [
        'auto-capturing arrow functions',
        'arrow-function auto-capture',
        'arrow functions automatically capture',
    ]);

    $require($specPath, $spec, [
        'Closure captures are explicit for both arrow functions and anonymous block',
        'There is no automatic arrow capture',
        '`with ($value)` is a readonly borrow',
        '`with (writable $value)` is an exclusive',
        '`with (take $value)` transfers ownership',
        '`use` is not a closure-capture alias',
        'A closure that uses no enclosing local',
        'The compiler resolves readonly/writable/once structural function types',
        'function(int): int',
        '`E0641` is historical',
        '`with ($this)` borrows a',
        '`with (writable $this)` borrows it exclusively',
    ]);

    $require($pipelinePath, $pipeline, [
        'Decision 0120 accepts one explicit closure-capture model',
        'Stage 29 — Complete',
        'Stage 29 Slice 1 — Complete',
        'Stage 29 Slice 2 — Complete',
        'Corrective Beat: Native Collection Property Initializers — Complete',
        'Stage 29 Slice 3 — Complete',
        'Stage 30a Callable Grammar Completion — Complete',
        'Stage 30b Semantic Function Types And Captures — Complete',
        'Stage 30c Ownership, Lifetime, And Escape — Complete',
        'Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete',
        'Stage 30e Native Execution — Complete',
        'Stage 30f PHP Compatibility — Complete',
        'Stage 30g List Algorithms — Complete',
        'Stage 30 — Complete',
        'Decision 0120 — Accepted; Explicit Capture-List Foundation',
        'Decision 0121 — Accepted; Stage 30 Closure Authority',
        'Pre-Stage-30 Grammar Slice — Complete',
    ]);

    $require($auditPath, $audit, [
        'decision 0120 requires explicit `with` capture lists',
        'pre-Stage-30 grammar slice is complete and owns accepted lexer/parser/AST syntax',
        'No accepted authority grants those',
        'decision 0120 deliberately adds no public method',
    ]);

    $require($examplesPath, $examples, [
        'These snippets are accepted Stage 30 target-state documentation.',
        'Stages 30b and 30c check',
        'not an executable manifest',
        'Stage 30g List',
        'let $double = fn(int $value) => $value * 2;',
        'fn(int $score) with ($minimum) =>',
        'function (int $score): bool with ($minimum) {',
        'with (writable $count)',
        'with (take $payload)',
        'function(): string',
        'with ($bonus)',
        'fn(string $label) =>',
        'function main(): void',
        'Closure Must Capture',
        'between an arrow\'s',
        'ordinary moved-value diagnostic',
        'readonly capture is borrow-bound',
        'Decision 0121',
    ]);

    foreach (['Andrew', 'Lucy', 'Maya', 'Masiye'] as $personalName) {
        if (stripos($examples, $personalName) !== false) {
            $failures[] = "examples/future/stage30: personal or family name `{$personalName}` is forbidden";
        }
    }

    $require($lexerPath, $lexer, [
        'Fn,',
        'With,',
        '"fn" => TokenKind::Fn',
        '"with" => TokenKind::With',
    ]);
    $require($astPath, $ast, [
        'pub struct ClosureExpression',
        'pub enum ClosureForm',
        'pub struct ClosureCaptureClause',
        'pub enum ClosureCaptureMode',
        'pub enum ClosureBody',
        'Closure(Box<ClosureExpression>)',
    ]);
    $require($typesPath, $types, [
        'pub struct FunctionTypeRef',
        'pub struct FunctionTypeParameterRef',
        'function{invocation}({parameters}): {}',
    ]);
    $require($parserPath, $parser, [
        'fn parse_arrow_closure',
        'fn parse_anonymous_block_closure',
        'fn parse_closure_capture_clause',
        'fn parse_function_type_ref',
        'Doria closure captures use `with`, not PHP closure `use`',
        'a closure without captures omits the `with` clause',
        'Doria closure captures do not use PHP reference `&` syntax',
    ]);
    $require($semanticsPath, $semantics, [
        'fn check_closure_expression(',
        'source_binding_id: BindingId',
    ]);
    $forbid($semanticsPath, $semantics, ['"E0641"']);
    $require($cataloguePath, $catalogue, ['"E0641"']);
    $require($testsPath, $tests, [
        'capture_ast_preserves_modes_duplicates_order_and_exact_spans',
        'semantic_ide_and_php_paths_accept_php_compatible_closures',
        'malformed_closure_inventory_has_deliberate_diagnostics',
        'closure_recovery_does_not_cascade_into_following_syntax',
        'cli_ast_check_and_hir_accept_valid_closures',
    ]);
    $require($fixturePath, $fixture, [
        'fn(int $value) => $value * 2',
        'with (writable $count, take $message, $minimum, $minimum)',
        'function (int $value): bool',
        'function(int, string): bool',
        'function(): void',
    ]);

    foreach ([$planPath => $plan, $pipelinePath => $pipeline, $auditPath => $audit] as $path => $contents) {
        $forbid($path, $contents, [
            'Pre-Stage-30 Grammar Slice — Next',
            'Stage 30 — Blocked Until The Pre-Stage-30 Grammar Slice Completes',
            'pre-Stage-30 grammar slice is next',
        ]);
    }

    $grammarPosition = strpos($plan, '**Pre-Stage-30 Grammar Slice — Closure accepted syntax — Complete.**');
    $stage30Position = strpos($plan, '**Stage 30 Closure Authority — Accepted And Implemented; Stage 30 — Complete.**');
    if ($grammarPosition === false || $stage30Position === false || $grammarPosition >= $stage30Position) {
        $failures[] = "{$planPath}: the accepted-syntax grammar slice must precede Stage 30";
    }

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_closure_capture_authority(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "closure capture authority check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "closure capture authority check passed\n");
}
