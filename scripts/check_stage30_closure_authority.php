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
    $loweringPath = 'crates/doriac/src/lowering.rs';
    $hirPath = 'crates/doriac/src/hir.rs';
    $mirPath = 'crates/doriac/src/mir.rs';
    $acceptedFixturePath = 'crates/doriac/tests/fixtures/accepted_syntax/stage30a_callable_grammar.doria';
    $rejectedFixturePath = 'crates/doriac/tests/fixtures/negative_syntax/stage30a/README.md';
    $semanticsPath = 'crates/doriac/src/semantics.rs';

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
    $lowering = $read($loweringPath);
    $hir = $read($hirPath);
    $mir = $read($mirPath);
    $acceptedFixture = $read($acceptedFixturePath);
    $rejectedFixture = $read($rejectedFixturePath);
    $semantics = $read($semanticsPath);

    $require($decisionPath, $decision, [
        '# Decision 0121: Closure Function Types, Capture Semantics, And Execution Model',
        '**Status:** Accepted',
        '**Accepted:** 2026-08-19',
        '**Implementation Status:** Authority Accepted; Stage 30a Implemented; Stage 30b Next; Stage 30 Not Complete',
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
        'Stage 30b — Next',
        'E0641 retires by route',
        'Measurement Status: Pending Available Runner',
        '## Invalidated elsewhere',
    ]);

    $forbid($decisionPath, $decision, [
        'function take(): Payload',
        'function I(T)',
        'function I(writable A, T)',
        'Stage 30 — Complete',
        'Stage 30 is implemented',
    ]);

    $require($proposalPath, $proposal, [
        '**Superseded By Accepted Decision 0121.**',
        'It is not normative authority',
        'Stage 30a Callable Grammar Completion is complete',
        'Stage 30b Semantic Function Types And Captures is next',
        'E0641 remains the current compiler boundary',
    ]);
    $forbid($proposalPath, $proposal, [
        '**In Review.**',
        '| Approve / Amend / Reject |',
    ]);

    $require($capturePath, $capture, [
        '# Decision 0120: Explicit Closure Capture Lists',
        '**Status:** Accepted',
        'Decision 0121 settles the remaining Stage 30 model',
        'reopens explicit capture for ordinary local bindings',
    ]);
    $require($effectsPath, $effects, [
        'Decision 0121 carries the same law into structural function and closure',
        'closure bodies infer their effects after local catch subtraction',
    ]);

    $require($planPath, $plan, [
        'Decision 0121: closure function types, capture semantics, and execution model',
        '**Stage 30 Closure Authority — Accepted; Stage 30 — In Progress, Not Complete.**',
        '**Stage 30b Semantic Function Types And Captures — Next**',
        'Stage 30b Semantic Function Types And Captures',
        'Stage 30h Cross-Repository Closure',
        '`function take()` is rejected',
        '`List<T>` alone receives `map`, Copy-only preserving `filter`, and writable-accumulator `reduce`',
        'E0641 retires by completed route',
        'Measurement Status: Pending Available Runner',
    ]);

    $require($pipelinePath, $pipeline, [
        'Decision 0121 accepts the complete Stage 30 closure authority',
        'Stage 30a Callable Grammar Completion — Complete',
        'Stage 30b Semantic Function Types And Captures — Next',
        'Stage 30 — In Progress, Not Complete',
        'E0641 remains active',
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
        'source-preserving parenthesized type grouping',
        'arbitrary postfix callable-value invocation',
        '`E0641` development boundary',
        'Stage 30 - In Progress, Not Complete',
    ]);

    $require($collectionPath, $collection, [
        '| `map` / `filter` / `reduce`          | S30g | —    | —   | —     | —    | —   | —     |',
        '`List<T>` alone receives',
    ]);
    $require($stdlibPath, $stdlib, [
        '`filter(function(T): bool $predicate): List<T>`',
        '`reduce<A>(take A $initial, function(writable A, T): void $reducer): A`',
        'writable function writable(...)` callback through an exclusive function-value borrow',
        'No other collection family receives them in Stage 30',
    ]);
    $require($examplesPath, $examples, [
        '[Decision 0121]',
        'Stage 30a Callable Grammar',
        'Stage 30 is in progress and not complete',
        'Stage 30a Callable Grammar Completion is complete',
        'Stage 30b semantic function types and',
        'E0641 remains the compiler boundary',
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
    $require($loweringPath, $lowering, [
        'callable-value invocation must stop at the Stage 30 semantic boundary',
    ]);
    $forbid($hirPath, $hir, ['CallableCall', 'ClosureExpression']);
    $forbid($mirPath, $mir, ['CallableCall', 'ClosureExpression']);

    $require($semanticsPath, $semantics, [
        '"E0641"',
        'Closure Semantics Await Stage 30',
        'diagnostic.code == "E0641"',
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
