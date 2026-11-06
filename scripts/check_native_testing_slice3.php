<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$files = [
    'decision' => 'docs/decisions/0129-native-testing-foundation-behavioral-dsl-expectations-and-assertion-outcomes.md',
    'matchers' => 'crates/doriac/src/assertions.rs',
    'semantics' => 'crates/doriac/src/semantics.rs',
    'ownership' => 'crates/doriac/src/ownership.rs',
    'mir' => 'crates/doriac/src/mir.rs',
    'lowering' => 'crates/doriac/src/mir_lowering.rs',
    'validation' => 'crates/doriac/src/mir_validation.rs',
    'runtime' => 'crates/doria-rt/src/string_ops.rs',
    'php' => 'crates/doriac/src/codegen_php.rs',
    'cli' => 'crates/doriac/src/main.rs',
    'tests' => 'crates/doriac/tests/native_testing_slice1_tests.rs',
];

$contents = [];
foreach ($files as $key => $relative) {
    $text = file_get_contents($root . '/' . $relative);
    if ($text === false) {
        fwrite(STDERR, "Missing Native Testing Slice 3 file: {$relative}\n");
        exit(1);
    }
    $contents[$key] = $text;
}

$required = [
    'decision' => [
        '**Implementation Status:** Implemented By Native Testing Foundation Slices 1 Through 3',
        'Native Testing Foundation Slice 3 - Complete',
        'Native Testing Foundation - Complete',
        'Stage 34 Single Class Inheritance - Complete',
        '| `Bytes` | yes | yes | no | no | no |',
        '| `Dictionary<K, V>` / `SortedDictionary<K, V>` | yes | yes | no | yes | yes |',
        'ordinary once-call consumption',
        'bounded to 4 KiB',
        'most eight public-order entries',
        'DORIAO5',
    ],
    'matchers' => [
        'pub const MATCHER_SPECS: [MatcherSpec; 19]',
        '"StringContains"',
        '"StringEmpty"',
        '"CollectionContains"',
        '"CollectionEmpty"',
        '"CollectionCount"',
        '"DictionaryHasKey"',
        '"DictionaryHasValue"',
        '"Throws"',
        'pub fn matcher_candidates',
    ],
    'semantics' => [
        'check_throw_assertion',
        'assertion_callable_invocations',
        'Bytes Does Not Support Membership Expectations',
        'Dictionary Membership Expectation Is Ambiguous',
        'Negated Error Expectation Does Not Accept An Inspector',
    ],
    'ownership' => [
        'assertion_callable_invocations',
        'FunctionInvocationMode::Readonly',
        'FunctionInvocationMode::Writable',
        'FunctionInvocationMode::Once',
    ],
    'mir' => ['CollectionHas', 'AssertionPlan', 'CheckedIndirect'],
    'lowering' => [
        'lower_throw_assertion',
        'AssertionCollectionPresentation',
        'AssertionCountDifference',
        'AssertionErrorPresentation',
    ],
    'validation' => [
        'validate_assertion_collection_contract',
        'validate_throw_assertion_contract',
        'negated throw assertion cannot carry an inspector',
        'collection membership assertion uses an unsupported collection family',
    ],
    'runtime' => [
        'dr_v4_collection_assertion_presentation',
        'dr_v4_collection_assertion_count_difference',
        'dr_v4_error_assertion_presentation',
        'const ITEM_LIMIT: usize = 8',
    ],
    'php' => [
        '__doria_assertion_collection_presentation',
        '__DoriaCheckedError',
        '__doria_write_assertion_outcome_v4',
    ],
    'cli' => ['difference_present', 'difference'],
    'tests' => [
        'slice_three_collection_matchers_execute_on_every_backend',
        'slice_three_throw_matchers_intercept_checked_errors_exactly',
        'slice_three_throw_matchers_use_ordinary_invocation_modes_and_cleanup',
        'slice_three_failure_facts_match_on_every_backend',
        'slice_three_collection_presentations_are_bounded_and_public',
    ],
];

$failures = [];
foreach ($required as $key => $needles) {
    foreach ($needles as $needle) {
        if (!str_contains($contents[$key], $needle)) {
            $failures[] = "{$files[$key]}: missing `{$needle}`";
        }
    }
}

foreach (['FUTURE_MATCHERS', 'ReflectionClass', 'in_array($collection'] as $forbidden) {
    foreach (['matchers', 'semantics', 'mir', 'lowering', 'runtime', 'php'] as $key) {
        if (str_contains($contents[$key], $forbidden)) {
            $failures[] = "{$files[$key]}: forbidden Slice 3 mechanism `{$forbidden}`";
        }
    }
}

// Decision 0129 fixes assertion outcomes at V4. Later generic checked-I/O
// progress uses V5 through the existing Error writer, never assertion machinery.
$errorWriterPattern = '/^function __doria_write_error_outcome_v3\([\s\S]*?^\}/m';
$errorWriterCount = preg_match_all($errorWriterPattern, $contents['php'], $errorWriters, PREG_OFFSET_CAPTURE);
$errorWriter = '';
$phpOutsideErrorWriter = $contents['php'];
if ($errorWriterCount !== 1) {
    $failures[] = "{$files['php']}: expected exactly one generic runtime Error record writer";
} else {
    [$errorWriter, $offset] = $errorWriters[0][0];
    $phpOutsideErrorWriter = substr_replace($contents['php'], '', $offset, strlen($errorWriter));
}

foreach (['matchers', 'semantics', 'mir', 'lowering', 'runtime', 'php'] as $key) {
    $text = $key === 'php' ? $phpOutsideErrorWriter : $contents[$key];
    if (preg_match('/DORIAO(?:[5-9]|[1-9][0-9]+)(?![0-9])/', $text, $magic) === 1) {
        $failures[] = "{$files[$key]}: forbidden Slice 3 mechanism `{$magic[0]}` outside the generic Error writer";
    }
}
preg_match_all('/DORIAO[0-9]+/', $errorWriter, $writerMagics);
foreach ($writerMagics[0] as $magic) {
    if (!in_array($magic, ['DORIAO3', 'DORIAO5'], true)) {
        $failures[] = "{$files['php']}: unexpected generic Error transport `{$magic}`";
    }
}

$assertionWriterPattern = '/^function __doria_write_assertion_outcome_v4\([\s\S]*?^\}/m';
$assertionWriterCount = preg_match_all($assertionWriterPattern, $contents['php'], $assertionWriters);
if ($assertionWriterCount !== 1) {
    $failures[] = "{$files['php']}: expected exactly one V4 assertion record writer";
}
$assertionWriter = $assertionWriterCount === 1 ? $assertionWriters[0][0] : '';
$cliFunctions = [];
foreach ([
    'run_command',
    'decode_runtime_outcome',
    'decode_runtime_outcome_with_sources',
    'decode_runtime_assertion_outcome',
    'decode_runtime_error_outcome',
    'runtime_source_span',
] as $name) {
    $pattern = '/^fn ' . preg_quote($name, '/') . '\([\s\S]*?^\}/m';
    $count = preg_match_all($pattern, $contents['cli'], $matches);
    if ($count !== 1) {
        $failures[] = "{$files['cli']}: expected exactly one `{$name}` function";
    }
    $cliFunctions[$name] = $count === 1 ? $matches[0][0] : '';
}

$graphDispatchPattern = '/decode_runtime_outcome_with_sources\(\s*&payload,\s*fallback_path,\s*Some\(&graph\.source_map\),?\s*\)\?/';
if (preg_match($graphDispatchPattern, $cliFunctions['run_command']) !== 1) {
    $failures[] = "{$files['cli']}: build-plan runtime decoding must forward its compilation source inventory";
}

$scopedRequirements = [
    'generic Error writer' => [$errorWriter, [
        'bool $withTransferProgress = false',
        '$withTransferProgress ? "DORIAO5\0" : "DORIAO3\0"',
        '$withTransferProgress ? 5 : 3',
        '$caught->descriptor->hasTransferredByteCount',
        '? $caught->error()->transferredByteCount : null',
        '$count !== null ? 1 : 0',
    ]],
    'assertion writer' => [$assertionWriter, [
        '$record = "DORIAO4\0"',
        'pack("vVVCVCCVV", 4,',
    ]],
    'PHP descriptor and dispatch' => [$phpOutsideErrorWriter, [
        'if class_decl.name == crate::compiler_known_io::IO_ERROR',
        'getenv("DORIA_RUNTIME_OUTCOME_V4")',
        '__doria_write_assertion_outcome_v4($caught, $facts, $v4)',
        '!$assertion && is_string($v5) && __doria_write_error_outcome_v3($caught, $v5, true)',
        '!$assertion && !is_string($v5) && is_string($v3) && __doria_write_error_outcome_v3($caught, $v3)',
    ]],
    'CLI standalone dispatch' => [$cliFunctions['decode_runtime_outcome'], [
        'decode_runtime_outcome_with_sources(payload, source_path, None)',
    ]],
    'CLI build-plan source inventory' => [$cliFunctions['run_command'], [
        'decode_runtime_outcome_with_sources(',
        'Some(&graph.source_map)',
    ]],
    'CLI source-aware dispatch and panic decoder' => [$cliFunctions['decode_runtime_outcome_with_sources'], [
        'sources: Option<&doriac::source_map::SourceMap>',
        'if payload.starts_with(b"DORIAO4\0")',
        'return decode_runtime_assertion_outcome(payload, source_path, sources)',
        'if payload.starts_with(b"DORIAO3\0") || payload.starts_with(b"DORIAO5\0")',
        'return decode_runtime_error_outcome(payload, source_path, sources)',
        'runtime_source_span(sources, &record_path, primary_span)?',
        'runtime_source_span(sources, &frame_path, span)?',
        'Diagnostic::runtime_panic(',
    ]],
    'CLI source-aware assertion decoder' => [$cliFunctions['decode_runtime_assertion_outcome'], [
        'sources: Option<&doriac::source_map::SourceMap>',
        'runtime_source_span(sources, &record_path, span)?',
        'runtime_source_span(sources, &frame_path, frame_span)?',
        'Diagnostic::runtime_assertion(',
    ]],
    'CLI source-aware shared Error decoder' => [$cliFunctions['decode_runtime_error_outcome'], [
        'sources: Option<&doriac::source_map::SourceMap>',
        'magic == b"DORIAO3\0" && version == 3',
        'magic == b"DORIAO5\0" && version == 5',
        'transferred_byte_count.is_some() && error_type != doriac::compiler_known_io::IO_ERROR',
        'runtime_source_span(sources, &record_path, span)?',
        'Diagnostic::runtime_error(',
    ]],
    'CLI runtime location inventory ownership' => [$cliFunctions['runtime_source_span'], [
        'sources.by_path(path).ok_or_else(',
        'runtime source absent from the compilation inventory',
        'offsets.start > offsets.end || offsets.end > record.source.text.len()',
        'Span::in_source(',
        'record.source.id',
        'offsets.start',
        'offsets.end',
    ]],
];
foreach ($scopedRequirements as $scope => [$text, $needles]) {
    foreach ($needles as $needle) {
        if (!str_contains($text, $needle)) {
            $failures[] = "{$scope}: missing `{$needle}`";
        }
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Native Testing Slice 3 compiler/runtime guard failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Native Testing Slice 3 compiler/runtime guard passed\n");
