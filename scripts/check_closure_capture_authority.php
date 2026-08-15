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

    $decision = $read($decisionPath);
    $plan = $read($planPath);
    $spec = $read($specPath);
    $pipeline = $read($pipelinePath);
    $audit = $read($auditPath);
    $examples = $read($examplesPath);

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
        'Stage 30 owns closure grammar, explicit capture lists, capture validation',
        'Decision 0119 owns source-ordered checked-effect sets',
        'effect inference. Stage 30 must integrate closure bodies',
        'Stage 30 must settle `$this` independently',
        'The later structured-concurrency stage owns async closures',
        'The audit found no accepted grant of',
        'runtime reflection occurs',
        'not PHP arrow-function',
        'Rust bootstrap representation does not define the language model',
    ]);

    $require($planPath, $plan, [
        'Both forms require a `with` list when they reference enclosing local bindings',
        'Copy, readonly, writable, and Move bindings have no implicit-capture exception',
        'A closure with no surrounding-local dependency omits `with`',
        'Changing an arrow into a block closure preserves its capture list and ownership modes',
        'Decision 0120: explicit closure capture lists',
        'Stage 30 remains blocked until Stage 29 completes and is not implemented',
        'A missing capture receives a capture-specific diagnostic',
        'Closure callable types preserve Stage 29',
        '`List<T>` `map`, `filter`, and `reduce` use this same closure model',
        'No automatic-capture exception exists for Copy values',
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
        '`$this` capture remains a bounded Stage 30',
    ]);

    $require($pipelinePath, $pipeline, [
        'Decision 0120 accepts one explicit closure-capture model',
        'Stage 29 — In Progress',
        'Stage 29 Slice 1 — Complete',
        'Stage 29 Slice 2 — Next',
        'Stage 29 Slice 3 — Pending',
        'Stage 30 — Blocked Until Stage 29 Completes',
        'Decision 0120 — Accepted; Stage 30 Authority Only',
    ]);

    $require($auditPath, $audit, [
        'decision 0120 requires explicit `with` capture lists',
        'No accepted authority grants those',
        'decision 0120 deliberately adds no public method',
    ]);

    $require($examplesPath, $examples, [
        'registered as native parity fixtures',
        'Closure Must Capture',
        'between an arrow\'s',
        'ordinary moved-value diagnostic',
        'readonly capture is borrow-bound',
    ]);

    $exampleFiles = [
        'examples/future/stage30/no_capture.doria',
        'examples/future/stage30/readonly_arrow_capture.doria',
        'examples/future/stage30/readonly_block_capture.doria',
        'examples/future/stage30/writable_capture.doria',
        'examples/future/stage30/taking_capture.doria',
        'examples/future/stage30/collection_pipeline.doria',
    ];

    $allExamples = $examples;
    foreach ($exampleFiles as $path) {
        $contents = $read($path);
        $allExamples .= "\n" . $contents;
        if (str_contains($contents, 'with ()')) {
            $failures[] = "{$path}: no-capture closures must omit `with`; empty capture lists are forbidden";
        }
    }

    $require('examples/future/stage30/no_capture.doria', $read($exampleFiles[0]), [
        'fn(int $value) =>',
        'function (int $value): bool {',
    ]);
    $require('examples/future/stage30/readonly_arrow_capture.doria', $read($exampleFiles[1]), [
        'fn(int $score) with ($minimum) =>',
    ]);
    $require('examples/future/stage30/readonly_block_capture.doria', $read($exampleFiles[2]), [
        'function (int $score): bool with ($minimum) {',
    ]);
    $require('examples/future/stage30/writable_capture.doria', $read($exampleFiles[3]), [
        'with (writable $count)',
    ]);
    $require('examples/future/stage30/taking_capture.doria', $read($exampleFiles[4]), [
        'with (take $payload)',
        'function(): string',
    ]);
    $require('examples/future/stage30/collection_pipeline.doria', $read($exampleFiles[5]), [
        'with ($minimum)',
        'with ($bonus)',
        'fn(string $label) =>',
        'Doria\Std\Io\IoError',
    ]);

    foreach (['Andrew', 'Lucy', 'Maya', 'Masiye'] as $personalName) {
        if (stripos($allExamples, $personalName) !== false) {
            $failures[] = "examples/future/stage30: personal or family name `{$personalName}` is forbidden";
        }
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
