<?php

declare(strict_types=1);

$root = dirname(__DIR__);
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
            $failures[] = "{$path}: missing checked-error foundation contract `{$needle}`";
        }
    }
};

$forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (str_contains($contents, $needle)) {
            $failures[] = "{$path}: forbidden checked-error foundation text `{$needle}`";
        }
    }
};

$decisionPath = 'docs/decisions/0119-checked-errors-error-values-throws-effects-propagation-and-runtime-outcomes.md';
$planPath = 'docs/doria-end-to-end-plan.md';
$pipelinePath = 'docs/notes/current-pipeline.md';
$specPath = 'SPEC.md';
$lexerPath = 'crates/doriac/src/lexer.rs';
$parserPath = 'crates/doriac/src/parser.rs';
$astPath = 'crates/doriac/src/ast.rs';
$hirPath = 'crates/doriac/src/hir.rs';
$symbolsPath = 'crates/doriac/src/symbols.rs';
$typesPath = 'crates/doriac/src/types.rs';
$semanticsPath = 'crates/doriac/src/semantics.rs';
$ownershipPath = 'crates/doriac/src/ownership.rs';
$libPath = 'crates/doriac/src/lib.rs';
$testsPath = 'crates/doriac/tests/checked_error_tests.rs';
$fixturePath = 'examples/compile-only/main_stage29_checked_error_foundation.doria';

$decision = $read($decisionPath);
$plan = $read($planPath);
$pipeline = $read($pipelinePath);
$spec = $read($specPath);
$lexer = $read($lexerPath);
$parser = $read($parserPath);
$ast = $read($astPath);
$hir = $read($hirPath);
$symbols = $read($symbolsPath);
$types = $read($typesPath);
$semantics = $read($semanticsPath);
$ownership = $read($ownershipPath);
$lib = $read($libPath);
$tests = $read($testsPath);
$fixture = $read($fixturePath);

$require($decisionPath, $decision, [
    '**Status:** Accepted',
    'Stage 29 Slice 1 complete; Slice 2 next; Slice 3 pending',
    '`Error` is a compiler-known core interface',
    'explicitly declares `implements Error`',
    'externally accessible, readonly, stored `string $message`',
    'A promoted readonly constructor parameter named `message` satisfies',
    'Error classes are ordinary owned Move classes',
    '`throws` follows an explicit return type',
    'may omit a return annotation and declare `throws`',
    'destructors may not declare',
    'order is preserved for HIR',
    'checking uses a normalized semantic set',
    'must carry the same law',
    '`throw expression;` is a statement',
    'transfers that ownership',
    'Rethrow is `throw $error;`',
    'A binding is optional',
    'owned readonly',
    'concrete catch matches exact concrete identity',
    '`catch (Error)`',
    'matches every checked error',
    'unable to match any',
    'protected effect are unreachable',
    'No checked error may escape finally',
    'Cleanup is not transactional',
    'ordinary `__destruct` does not run',
    'StructuredExitKind::CheckedError',
    'Slice 2 introduces the carrier',
    'Doria\\Std\\Io\\IoError',
    'Doria\\Std\\Io\\InvalidUtf8Error',
    '`Error[R1000]: Unhandled <ConcreteType>`',
    'status 70',
    'panic stays status 101',
    '**Pending Available Runner**',
    'Stage 30 is blocked until Stage 29 completes',
]);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    $require($path, $contents, [
        'Stage 28a — Complete',
        'Stage 29 — In Progress',
        'Stage 29 Slice 1 — Complete',
        'Stage 29 Slice 2 — Next',
        'Stage 29 Slice 3 — Pending',
        'Stage 30 — Blocked Until Stage 29 Completes',
        'Stage 26b — Complete',
        'Measurement Status: Pending Available Runner',
    ]);
}

$require($specPath, $spec, [
    '`Error` is a compiler-known',
    'explicitly declaring `implements Error`',
    '`throws` follows the explicit return type',
    '`throw expression;` is a statement',
    'Catch bindings are optional',
    '`catch (Error)` catches every checked error',
    'Checked propagation performs deterministic cleanup',
    'never rolls back completed side effects',
    'Stage 29 Slice 1 implements grammar',
]);

$require($lexerPath, $lexer, [
    'TokenKind::Try',
    'TokenKind::Catch',
    'TokenKind::Throw',
    'TokenKind::Throws',
    'TokenKind::Finally',
    '"try" => TokenKind::Try',
    '"catch" => TokenKind::Catch',
]);
$require($parserPath, $parser, [
    'fn parse_throws_clause(',
    'fn parse_try_statement(',
    'Stmt::Throw',
    'Stmt::Try',
    'bare `throw` is not supported',
    'Throw Is A Statement',
    'Try Requires Catch Or Finally',
]);
$require($astPath, $ast, [
    'pub struct ThrowsClause',
    'pub struct ThrowsEntry',
    'pub struct ThrowStmt',
    'pub struct TryStmt',
    'pub struct CatchClause',
]);
$require($hirPath, $hir, [
    'pub struct ThrowsClause',
    'pub struct ThrowsEntry',
    'pub struct ThrowStmt',
    'pub struct TryStmt',
    'pub uncovered_effects:',
]);
$require($symbolsPath, $symbols, [
    'pub enum BuiltinInterface',
    'Displayable,',
    'Error,',
    'pub checked_effects: Vec<TypeId>',
]);
$require($typesPath, $types, [
    'TypeKind::Error',
    'ResolvedType::Error',
]);
$forbid($typesPath, $types, ['"IoError" =>', '"InvalidUtf8Error" =>']);
$require($semanticsPath, $semantics, [
    'fn check_error_interface(',
    'Error Message Property Is Missing',
    'Error Message Property Must Be Readonly',
    'Error Message Property Must Be Externally Accessible',
    'Function Return Type Is Required',
    'Destructors Cannot Throw Checked Errors',
    'Duplicate Throws Entry',
    'Error Already Covers This Throws Entry',
    'fn type_implements_error(',
    'fn check_throw_statement(',
    'Class Must Explicitly Implement Error',
    'fn check_try_statement(',
    'Catch Must Name An Error Type',
    'Duplicate Catch',
    'Catch After Error Is Unreachable',
    'Catch Cannot Match An Error From This Try',
    'Checked Error Cannot Escape Finally',
    'Constant Initializer Cannot Throw',
    'Static Initializer Cannot Throw',
    'Implicit Constructor Cannot Hide A Throwing Initializer',
]);
$require($ownershipPath, $ownership, ['Stmt::Throw(statement)', 'UseMode::Give', 'Stmt::Try(statement)']);
$require($libPath, $lib, [
    'reject_checked_error_execution(&hir)?',
    'B2901',
    'Checked Error Execution Lands In Stage 29 Slice 2',
]);
$require($testsPath, $tests, [
    'error_conformance_accepts_both_property_forms_and_rejects_invalid_contracts',
    'throws_entries_are_error_types_unique_and_source_ordered',
    'direct_throw_requires_an_owned_explicit_error_value',
    'catches_subtract_only_protected_effects_and_catch_bodies_are_independent',
    'construction_effects_cannot_hide_in_initialization',
    'checked_errors_cannot_escape_finally_or_static_initialization',
    'checked_error_scopes_and_optional_bindings_follow_lexical_blocks',
    'stage29_slice1_stops_once_before_mir_and_backends',
    'nonthrowing programs must remain executable',
]);
$require($fixturePath, $fixture, [
    'implements Error',
    'throws RecordUnavailableError, StorageError',
    'throw new',
    'catch (RecordUnavailableError $error)',
    'catch (StorageError)',
    'finally',
]);
$forbid($fixturePath, $fixture, ['Andrew', 'Lucy', 'Maya', 'Person']);

if ($failures !== []) {
    fwrite(STDERR, "checked-error foundation check failed:\n");
    foreach ($failures as $failure) {
        fwrite(STDERR, "- {$failure}\n");
    }
    exit(1);
}

fwrite(STDOUT, "checked-error foundation check passed\n");
