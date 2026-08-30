<?php

declare(strict_types=1);

/**
 * @return list<string>
 */
function check_stage30_closure_authority(string $root): array
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
                $failures[] = "{$path}: missing Stage 30 authority marker `{$needle}`";
            }
        }
    };

    $forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
        foreach ($needles as $needle) {
            if (str_contains($contents, $needle)) {
                $failures[] = "{$path}: forbidden stale Stage 30 authority `{$needle}`";
            }
        }
    };

    $decisionPath = 'docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md';
    $proposalPath = 'docs/notes/stage30-closure-authority-proposal.md';
    $planPath = 'docs/doria-end-to-end-plan.md';
    $pipelinePath = 'docs/notes/current-pipeline.md';
    $specPath = 'SPEC.md';
    $capturePath = 'docs/decisions/0120-explicit-closure-capture-lists.md';
    $effectsPath = 'docs/decisions/0119-checked-errors-error-values-throws-effects-propagation-and-runtime-outcomes.md';
    $collectionPath = 'docs/decisions/0113-collection-surface-completion.md';
    $stdlibPath = 'docs/stdlib-reference.md';
    $examplesPath = 'examples/future/stage30/README.md';
    $lexerPath = 'crates/doriac/src/lexer.rs';
    $parserPath = 'crates/doriac/src/parser.rs';
    $astPath = 'crates/doriac/src/ast.rs';
    $typesPath = 'crates/doriac/src/types.rs';
    $symbolsPath = 'crates/doriac/src/symbols.rs';
    $loweringPath = 'crates/doriac/src/lowering.rs';
    $hirPath = 'crates/doriac/src/hir.rs';
    $mirPath = 'crates/doriac/src/mir.rs';
    $acceptedFixturePath = 'crates/doriac/tests/fixtures/accepted_syntax/stage30a_callable_grammar.doria';
    $rejectedFixturePath = 'crates/doriac/tests/fixtures/negative_syntax/stage30a/README.md';
    $semanticsPath = 'crates/doriac/src/semantics.rs';
    $stage30bTestsPath = 'crates/doriac/tests/stage30b_semantics_tests.rs';

    $decision = $read($decisionPath);
    $proposal = $read($proposalPath);
    $plan = $read($planPath);
    $pipeline = $read($pipelinePath);
    $spec = $read($specPath);
    $capture = $read($capturePath);
    $effects = $read($effectsPath);
    $collection = $read($collectionPath);
    $stdlib = $read($stdlibPath);
    $examples = $read($examplesPath);
    $lexer = $read($lexerPath);
    $parser = $read($parserPath);
    $ast = $read($astPath);
    $types = $read($typesPath);
    $symbols = $read($symbolsPath);
    $lowering = $read($loweringPath);
    $hir = $read($hirPath);
    $mir = $read($mirPath);
    $acceptedFixture = $read($acceptedFixturePath);
    $rejectedFixture = $read($rejectedFixturePath);
    $semantics = $read($semanticsPath);
    $stage30bTests = $read($stage30bTestsPath);

    $require($decisionPath, $decision, [
        '# Decision 0121: Closure Function Types, Capture Semantics, And Execution Model',
        '**Status:** Accepted',
        '**Accepted:** 2026-08-19',
        '**Implementation Status:** Authority Accepted; Stages 30a Through 30h Implemented; Stage 30 Complete',
        '**Elaborates:** Decision 0120',
        'function writable(int): int',
        'function once(): Payload',
        '`function take()` is rejected',
        'function(writable Counter): void',
        'function(take Payload): string',
        'function(string): Record throws ParseError, StorageError',
        'There is no closure-expression `throws`',
        'with ($this)',
        'with (writable $this)',
        '`with (take $this)` is rejected',
        'stable semantic binding identities',
        'Closure creation selects or allocates environment storage',
        'may reorder privately to reduce padding',
        'Descriptors are lean',
        'Stage 30g adds higher-order algorithms only to `List<T>`',
        'where T: Copy',
        'map<U>(function(T): U transform): List<U> effects(transform)',
        'map<U>(writable function writable(T): U transform): List<U> effects(transform)',
        'filter(writable function writable(T): bool predicate): List<T> effects(predicate) where T: Copy',
        'reduce<A>(take A initial, writable function writable(writable A, T): void reducer): A effects(reducer)',
        'A writable callback can never pass through the readonly',
        'Stage 30a - Callable Grammar Completion',
        '## Accepted Amendment: Parenthesized Type Grouping',
        '**Accepted: 2026-08-20**',
        'It is not a tuple',
        'Stage 30a — Complete',
        'Stage 30b — Complete',
        'Stage 30c — Complete',
        'Stage 30d — Complete',
        'Stage 30e — Complete',
        'Stage 30f — Complete',
        'Stage 30g — Complete',
        'Stage 30h — Complete',
        'E0641 retires by route',
        'Measurement Status: Pending Available Runner',
        '## Invalidated elsewhere',
    ]);

    $forbid($decisionPath, $decision, [
        'function take(): Payload',
        'function I(T)',
        'function I(writable A, T)',
        'Stage 30 is implemented',
    ]);

    $require($proposalPath, $proposal, [
        '**Superseded By Accepted Decision 0121.**',
        'It is not normative authority',
        'Stages 30a through 30h and Stage 30 are complete',
        'E0641 is historical and reserved',
        'Stages 31 through 33 and Phase F are complete',
        'Native Testing Foundation Slice 1 is complete',
        'Stage 34',
        'waits for the foundation',
    ]);
    $forbid($proposalPath, $proposal, [
        '**In Review.**',
        '| Approve / Amend / Reject |',
    ]);

    $require($capturePath, $capture, [
        '# Decision 0120: Explicit Closure Capture Lists',
        '**Status:** Accepted',
        'Decision 0121 settles the questions this record deliberately left bounded',
        'reopens explicit capture for ordinary local bindings',
    ]);
    $require($effectsPath, $effects, [
        'Decision 0121 carries the same law into structural function and closure',
        'closure bodies infer their effects after local catch subtraction',
    ]);

    $require($planPath, $plan, [
        'Decision 0121: closure function types, capture semantics, and execution model',
        '**Stage 30 Closure Authority — Accepted And Implemented; Stage 30 — Complete.**',
        'Stage 30b Semantic Function Types And Captures — Complete',
        'Stage 30c Ownership, Lifetime, And Escape — Complete',
        'Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete',
        'Stage 30e Native Execution — Complete',
        'Stage 30f PHP Compatibility — Complete',
        'Stage 30g List Algorithms — Complete',
        'Stage 30h Cross-Repository Closure',
        '`function take()` is rejected',
        '`List<T>` alone receives `map`, Copy-only preserving `filter`, and writable-accumulator `reduce`',
        'E0641 has retired by completed route',
        'Measurement Status: Pending Available Runner',
    ]);

    $require($pipelinePath, $pipeline, [
        'Decision 0121 accepts the complete Stage 30 closure authority',
        'Stage 30a Callable Grammar Completion — Complete',
        'Stage 30b Semantic Function Types And Captures — Complete',
        'Stage 30c Ownership, Lifetime, And Escape — Complete',
        'Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete',
        'Stage 30e Native Execution — Complete',
        'Stage 30f PHP Compatibility — Complete',
        'Stage 30g List Algorithms — Complete',
        'Stage 30 — Complete',
        'E0641 is historical and remains reserved',
        'Stage 31 Slice 1 — Complete',
        'Stage 31 Slice 2 — Complete',
        'Stage 31 — Complete',
        'Stage 32 — Complete',
        'Stage 33 Slice 1 — Complete',
        'Stage 33 Slice 2 — Complete',
        'Stage 33 Slice 3 — Complete',
        'Stage 33 — Complete',
        'Phase F — Complete',
        'Native Testing Foundation Slice 1 — Complete',
        'Native Testing Foundation Slice 2 — Compiler/Runtime Implemented, Baton/Tooling Pending',
        'Stage 34 Single Class Inheritance — Blocked Until The Foundation Completes',
    ]);
    $forbid($pipelinePath, $pipeline, [
        'Stage 30 Closure Authority Proposal — In Review',
        'Stage 30 — Next, Not Implemented',
    ]);

    $require($specPath, $spec, [
        '### Accepted Stage 30 closure semantics',
        'function writable(int): int',
        'function once(): Payload',
        '`function take()` is not Doria',
        'with ($this)',
        'with (writable $this)',
        '### Current compiler support',
        'readonly/writable/once structural function types',
        'grouping remains transparent',
        'callable-value calls',
        '`E0641` is historical',
        'debug interpreter',
        'Stage 30 - Complete',
    ]);

    $require($collectionPath, $collection, [
        '| `map` / `filter` / `reduce`          | yes  | —    | —   | —     | —    | —   | —     |',
        '`List<T>` alone receives',
    ]);
    $require($stdlibPath, $stdlib, [
        '`filter(function(T): bool $predicate): List<T>`',
        '`reduce<A>(take A $initial, function(writable A, T): void $reducer): A`',
        'writable function writable(...)` callback through an exclusive function-value borrow',
        'No other collection family receives these algorithms in Stage 30',
    ]);
    $require($examplesPath, $examples, [
        '[Decision 0121]',
        'Stages 30a through 30h and Stage 30 are complete',
        'E0641 is historical and reserved',
        'Stages 30b and 30c check',
        'Stage 30g List algorithms are implemented',
    ]);

    $require($lexerPath, $lexer, [
        'Once,',
        '"once" => TokenKind::Once',
    ]);
    $require($typesPath, $types, [
        'pub enum FunctionInvocationMode',
        'pub enum FunctionTypeParameterMode',
        'pub struct FunctionTypeThrowsRef',
        'pub struct GroupedTypeRef',
        'pub struct SemanticFunctionParameter<T>',
        'pub struct SemanticFunctionType<T>',
        'pub checked_effects: Vec<T>',
        'Function(SemanticFunctionType<TypeId>)',
        'Function(Box<SemanticFunctionType<ResolvedType>>)',
    ]);
    $require($symbolsPath, $symbols, [
        'pub struct BindingId',
        'pub struct ClosureId',
        'pub struct BindingResolution',
        'pub declarations_by_id: HashMap<BindingId, BindingDeclaration>',
        'pub uses_by_span: HashMap<Span, BindingId>',
    ]);
    $require($astPath, $ast, ['CallableCall {']);
    $require($parserPath, $parser, [
        'FunctionInvocationMode::Once',
        'FunctionTypeParameterMode::Writable',
        'FunctionTypeParameterMode::Take',
        'parse_function_type_throws_clause',
        'Nested Function Type Effects Need Grouping',
        'Tuple Type Is Not Supported',
        'Callable Value Argument Cannot Be Named',
        'Expr::CallableCall',
    ]);
    $require($acceptedFixturePath, $acceptedFixture, [
        'function once(): Payload',
        'function(): int throws ParseError',
        '$factory()(2);',
    ]);
    $require($rejectedFixturePath, $rejectedFixture, [
        'invalid-invocation-mode.doria',
        'tuple-like-group.doria',
        'named-callable-argument.doria',
    ]);
    $require($loweringPath, $lowering, ['Expr::Closure', 'Expr::CallableCall']);
    $require($hirPath, $hir, ['CallableCall', 'ClosureExpression']);
    $require($mirPath, $mir, ['ClosureDescriptor', 'ClosureEnvironmentLayout']);

    $require($semanticsPath, $semantics, [
        'fn check_closure_expression(',
        'fn function_type_compatibility(',
        'pub struct CallableValueCallInfo',
        'source_binding_id: BindingId',
    ]);
    $forbid($semanticsPath, $semantics, ['"E0641"']);
    $require($stage30bTestsPath, $stage30bTests, [
        'semantic_function_types_preserve_structure_effects_and_nested_types',
        'capture_plans_use_stable_binding_identity_and_infer_minimum_access',
        'nullable_callable_narrowing_works_for_functions_locals_and_closure_roots',
        'function_types_flow_through_properties_collections_and_generic_inference',
    ]);

    return $failures;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $failures = check_stage30_closure_authority(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "Stage 30 closure authority check failed:\n- " . implode("\n- ", $failures) . "\n");
        exit(1);
    }

    fwrite(STDOUT, "Stage 30 closure authority check passed\n");
}
