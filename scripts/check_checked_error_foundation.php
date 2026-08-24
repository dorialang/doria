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
$mirPath = 'crates/doriac/src/mir.rs';
$loweringPath = 'crates/doriac/src/mir_lowering.rs';
$validationPath = 'crates/doriac/src/mir_validation.rs';
$phpPath = 'crates/doriac/src/codegen_php.rs';
$testsPath = 'crates/doriac/tests/checked_error_tests.rs';
$validationTestsPath = 'crates/doriac/tests/mir_validation_tests.rs';
$parityPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';
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
$mir = $read($mirPath);
$lowering = $read($loweringPath);
$validation = $read($validationPath);
$php = $read($phpPath);
$tests = $read($testsPath);
$validationTests = $read($validationTestsPath);
$parity = $read($parityPath);
$fixture = $read($fixturePath);

$require($decisionPath, $decision, [
    '**Status:** Accepted',
    'Stage 29 Slices 1 through 3 complete',
    'native collection',
    'property initializer corrective beat complete',
    'Stage 29 complete',
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
    'Decision 0121 carries the same law',
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
    'erased Error carrier is two machine words',
    'B2901 remains a historical catalogue identity with no valid',
    'B2902',
    'Doria\\Std\\Io\\IoError',
    'Doria\\Std\\Io\\InvalidUtf8Error',
    '`Error[R1000]: Unhandled <ConcreteType>`',
    'status 70',
    'panic stays status 101',
    'Pending Available',
    'Runner** and non-blocking',
    'pre-Stage-30 closure grammar slice is complete',
    'checked indirect calls reuse',
    'debug interpreter, Cranelift, and LLVM',
    'Stage 30f E0641 boundary',
]);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    $require($path, $contents, [
        'Stage 28a — Complete',
        'Stage 29 — Complete',
        'Stage 29 Slice 1 — Complete',
        'Stage 29 Slice 2 — Complete',
        'Corrective Beat: Native Collection Property Initializers — Complete',
        'Stage 29 Slice 3 — Complete',
        'Pre-Stage-30 Grammar Slice — Complete',
        'Stage 30a Callable Grammar Completion — Complete',
        'Stage 30b Semantic Function Types And Captures — Complete',
        'Stage 30c Ownership, Lifetime, And Escape — Complete',
        'Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete',
        'Stage 30e Native Execution — Complete',
        'Stage 30f PHP Compatibility — Next',
        'Stage 30 — In Progress, Not Complete',
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
    'Stage 29 implements grammar',
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
$forbid($libPath, $lib, [
    'B2902',
    'reject_checked_error_execution',
    'Checked Error Execution Lands In Stage 29 Slice 2',
    'Unhandled Main Error Reporting Lands In Stage 29 Slice 3',
]);
$require($mirPath, $mir, [
    'pub error_descriptors: Vec<ErrorDescriptor>',
    'pub error_origins: Vec<ErrorOrigin>',
    'pub struct ErrorDescriptor',
    'pub struct ErrorOrigin',
    'EnsureErrorOrigin',
    'DropError',
    'CheckedCall',
    'CheckedConstruct',
    'ErrorSwitch',
    'PropagateError',
]);
$require($loweringPath, $lowering, [
    'StructuredExitKind::CheckedError',
    'mir::Statement::EnsureErrorOrigin',
    'mir::Terminator::CheckedCall',
    'mir::Terminator::CheckedConstruct',
]);
$require($validationPath, $validation, [
    'validate_error_metadata',
    'checked call has an incompatible success slot',
    'checked call has an incompatible Error slot',
    'Error dispatch does not own an Error carrier',
    'checked construction has an incompatible success slot',
    'checked-error finalizer exit does not own an Error carrier',
]);
$require($phpPath, $php, [
    'final class __DoriaCheckedError extends Exception',
    'public __DoriaErrorDescriptor $descriptor',
    '__doriaEnsureErrorOrigin',
    'catch (__DoriaCheckedError',
]);
$require($testsPath, $tests, [
    'error_conformance_accepts_both_property_forms_and_rejects_invalid_contracts',
    'throws_entries_are_error_types_unique_and_source_ordered',
    'direct_throw_requires_an_owned_explicit_error_value',
    'catches_subtract_only_protected_effects_and_catch_bodies_are_independent',
    'construction_effects_cannot_hide_in_initialization',
    'checked_errors_cannot_escape_finally_or_static_initialization',
    'checked_error_scopes_and_optional_bindings_follow_lexical_blocks',
    'stage29_slice3_executes_handled_and_escaping_main_errors',
    'nonthrowing programs must remain executable',
]);
$require($validationTestsPath, $validationTests, [
    'shared_validator_rejects_malformed_checked_error_metadata_and_origins',
    'shared_validator_rejects_malformed_checked_calls_catches_and_carrier_ownership',
    'shared_validator_rejects_malformed_checked_finalizer_and_construction_plans',
]);
$require($parityPath, $parity, [
    'main_checked_error_catch.doria',
    'main_checked_error_catch_all.doria',
    'main_checked_error_optional_binding.doria',
    'main_checked_error_rethrow.doria',
    'main_checked_error_finally.doria',
    'main_checked_error_control_finalizers.doria',
    'main_checked_error_constructor.doria',
    'main_checked_error_failed_construction.doria',
    'main_checked_error_values.doria',
    'main_checked_error_mixed.doria',
    'main_checked_error_collections.doria',
    'main_checked_error_origin.doria',
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
$forbid($parityPath, $parity, ['Andrew', 'Lucy', 'Maya', 'Person']);

if ($failures !== []) {
    fwrite(STDERR, "checked-error foundation check failed:\n");
    foreach ($failures as $failure) {
        fwrite(STDERR, "- {$failure}\n");
    }
    exit(1);
}

fwrite(STDOUT, "checked-error foundation check passed\n");
