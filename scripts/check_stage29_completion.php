<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

$read = static function (string $path) use ($root, &$failures): string {
    $contents = @file_get_contents($root . '/' . $path);
    if (!is_string($contents)) {
        $failures[] = "{$path}: required Stage 29 authority is missing";
        return '';
    }
    return $contents;
};

$require = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (!str_contains($contents, $needle)) {
            $failures[] = "{$path}: missing Stage 29 completion contract `{$needle}`";
        }
    }
};

$forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (str_contains($contents, $needle)) {
            $failures[] = "{$path}: stale Stage 29 contract `{$needle}`";
        }
    }
};

$decisionPath = 'docs/decisions/0119-checked-errors-error-values-throws-effects-propagation-and-runtime-outcomes.md';
$planPath = 'docs/doria-end-to-end-plan.md';
$pipelinePath = 'docs/notes/current-pipeline.md';
$specPath = 'SPEC.md';
$knownIoPath = 'crates/doriac/src/compiler_known_io.rs';
$builtinsPath = 'crates/doriac/src/builtins.rs';
$semanticsPath = 'crates/doriac/src/semantics.rs';
$cataloguePath = 'crates/doria-diagnostic-catalogue/src/lib.rs';
$diagnosticsPath = 'crates/doriac/src/diagnostics.rs';
$runtimePath = 'crates/doria-rt/src/checked_io.rs';
$mainPath = 'crates/doriac/src/main.rs';
$libPath = 'crates/doriac/src/lib.rs';
$testsPath = 'crates/doriac/tests/stage17_io_tests.rs';
$checkedTestsPath = 'crates/doriac/tests/checked_error_tests.rs';
$nativeTestsPath = 'crates/doriac/tests/native_backend_tests.rs';
$parityPath = 'crates/doriac/tests/fixtures/native_parity_examples.txt';

$decision = $read($decisionPath);
$plan = $read($planPath);
$pipeline = $read($pipelinePath);
$spec = $read($specPath);
$knownIo = $read($knownIoPath);
$knownIoCanonical = str_replace('\\\\', '\\', $knownIo);
$builtins = $read($builtinsPath);
$semantics = $read($semanticsPath);
$catalogue = $read($cataloguePath);
$diagnostics = $read($diagnosticsPath);
$runtime = $read($runtimePath);
$main = $read($mainPath);
$lib = $read($libPath);
$tests = $read($testsPath);
$checkedTests = $read($checkedTestsPath);
$nativeTests = $read($nativeTestsPath);
$parity = $read($parityPath);

$require($decisionPath, $decision, [
    '**Status:** Accepted',
    'Stage 29 Slices 1 through 3 complete',
    'property initializer corrective beat complete',
    'Stage 29 complete',
    'pre-Stage-30 closure grammar slice is complete',
    'Decision 0123 corrective beat are complete. All three',
    'Stage 33 slices and Phase F are complete under Decisions 0126 through 0128',
    'Stage 34 is next',
    'debug interpreter, Cranelift, LLVM, and PHP',
    'Decision 0122 implements owned-property move-in and writable replacement',
    'E0472 remains reachable only for the separate move-out boundary',
    'P1401 through P1407 are historical identities with no ordinary valid route',
    'P1206/P1302 remain allocation panics',
    'ordinary stdout/stderr',
    'status-0 rule',
    'R1000 is catalogue kind `runtimeError`',
    'termination `propagateWithCleanup`',
    'successful `main(): int` returning 70 is not an R1000',
    'no propagation path',
]);

foreach ([$planPath => $plan, $pipelinePath => $pipeline] as $path => $contents) {
    $require($path, $contents, [
        'Stage 29 — Complete',
        'Stage 29 Slice 1 — Complete',
        'Stage 29 Slice 2 — Complete',
        'Corrective Beat: Native Collection Property Initializers — Complete',
        'Stage 29 Slice 3 — Complete',
        'Corrective Beat: Inferred Main Checked Effects — Complete',
        'Pre-Stage-30 Grammar Slice — Complete',
        'Stage 30a Callable Grammar Completion — Complete',
        'Stage 30b Semantic Function Types And Captures — Complete',
        'Constructor Writable-Path And Owned-Property Corrective Beat — Complete',
        'Stage 30c Ownership, Lifetime, And Escape — Complete',
        'Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete',
        'Stage 30e Native Execution — Complete',
        'Stage 30f PHP Compatibility — Complete',
        'Stage 30g List Algorithms — Complete',
        'Stage 30h Cross-Repository Closure — Complete',
        'Stage 30 — Complete',
        'Stage 26b — Complete',
        'Measurement Status: Pending Available Runner',
    ]);
}

$require($knownIoPath, $knownIoCanonical, [
    'Doria\\Std\\Io\\IoOperation',
    'Doria\\Std\\Io\\IoTarget',
    'Doria\\Std\\Io\\IoErrorReason',
    'Doria\\Std\\Io\\Utf8InputSource',
    'Doria\\Std\\Io\\IoError',
    'Doria\\Std\\Io\\InvalidUtf8Error',
    'pub const CANONICAL_TYPES: [&str; 6]',
    'validate_reserved_identities',
    'visit_io_error_message_parts',
    'visit_invalid_utf8_message_parts',
]);
$forbid($knownIoPath, $knownIo, ['"IoError" =>', '"InvalidUtf8Error" =>']);

$require($builtinsPath, $builtins, [
    'pub const ECHO_CHECKED_ERROR_TYPES',
    'pub const fn signature',
    'read_line(string $prompt = \"\"): ?string',
    'pub const fn checked_error_types',
    'Self::ReadLine | Self::ReadFile',
    'crate::compiler_known_io::INVALID_UTF8_ERROR',
    'Self::Panic | Self::Sprintf => &[]',
    'Self::WriteStdoutBytes',
    'Self::WriteStderrBytes',
]);
$require($semanticsPath, $semantics, [
    'self.record_compiler_known_effects(',
    'crate::builtins::ECHO_CHECKED_ERROR_TYPES',
]);

$require($cataloguePath, $catalogue, [
    'code: "R1000"',
    'process_status: 70',
    'write_terminal_safe_runtime_text',
    'visit_io_error_message_parts',
    'visit_invalid_utf8_message_parts',
]);
$require($diagnosticsPath, $diagnostics, [
    'RuntimeError',
    '"runtimeError"',
    '"propagateWithCleanup"',
    'Self::build("R1000"',
    'runtime_error_messages_are_safe_in_terminals_and_exact_in_json',
]);
$require($runtimePath, $runtime, [
    'WriteOutcome::BrokenPipe => exit_process(0)',
    'panic_catalogued(frame, b"P1206")',
    'panic_catalogued(frame, b"P1302")',
]);
$require($mainPath, $main, [
    'DORIA_RUNTIME_OUTCOME_V3',
    'decode_runtime_error_outcome',
    'TerminationBehavior::PropagateWithCleanup',
    'process_status: 70',
    'runtime_transport_rejects_truncated_unknown_and_trailing_records',
]);
$forbid($libPath, $lib, ['B2902', 'reject_escaping_main_error']);

$require($specPath, $spec, [
    'read_line(string $prompt = ""): ?string',
    'Doria\\Std\\Io\\InvalidUtf8Error',
    '`sprintf` returns `string` and remains nonthrowing',
    'every `echo` statement carries the ambient',
    '`null` from `read_line` means EOF',
    'preserves empty lines',
    'P1401 through P1407 remain historical catalogue identities',
]);

$require($checkedTestsPath, $checkedTests, [
    'compiler_known_io_types_are_nominal_and_expose_typed_fields',
    'stage29_slice3_executes_handled_and_escaping_main_errors',
    'repository_doria_sources_cover_checked_io_effects_and_contain_finalizers',
    'selected_main_infers_exact_uncovered_effects_without_changing_source_syntax',
]);
$require($testsPath, $tests, [
    'prompted_read_line_failures_use_checked_errors_except_for_allocation',
    'interpreter_faults_preserve_every_io_reason_system_code_and_standard_error_target',
]);
$require($nativeTestsPath, $nativeTests, [
    'native_panic_stays_fatal_when_stderr_is_closed',
    'native_stderr_broken_pipe_exits_cleanly',
    'native_runtime_error_reporting_failure_exits_70_silently',
]);

foreach ([
    'main_io_error_missing_file.doria',
    'main_io_error_invalid_path.doria',
    'main_invalid_utf8_file.doria',
    'main_invalid_utf8_stdin.doria',
    'main_read_line_checked.doria',
    'main_checked_output.doria',
    'main_unhandled_error.doria',
    'main_unhandled_io_error.doria',
    'main_unhandled_error_cleanup.doria',
    'main_unhandled_error_multiline.doria',
    'main_status_70_success.doria',
] as $fixture) {
    if (!str_contains($parity, $fixture)) {
        $failures[] = "{$parityPath}: missing durable Stage 29 fixture `{$fixture}`";
    }
}

$examplePaths = [];
foreach (glob($root . '/examples/native/main_{checked,invalid,io,read,status,unhandled}*.doria', GLOB_BRACE) ?: [] as $path) {
    $examplePaths[] = $path;
}
foreach ($examplePaths as $path) {
    $contents = file_get_contents($path);
    if (!is_string($contents)) {
        continue;
    }
    foreach (['Andrew', 'Lucy', 'Maya', 'Masiye'] as $personalName) {
        if (stripos($contents, $personalName) !== false) {
            $failures[] = str_replace($root . '/', '', $path) . ": personal or family name `{$personalName}` is forbidden";
        }
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Stage 29 completion check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Stage 29 completion check passed\n");
