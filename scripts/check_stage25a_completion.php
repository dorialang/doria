<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

$read = static function (string $path) use ($root, &$failures): string {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: unable to read file";
        return '';
    }
    return $contents;
};

$types = $read('crates/doriac/src/types.rs');
$kindBlockStart = strpos($types, 'pub const ALL: [SharedHandleKind; 6]');
$kindBlockEnd = $kindBlockStart === false ? false : strpos($types, '];', $kindBlockStart);
if ($kindBlockStart === false || $kindBlockEnd === false) {
    $failures[] = 'types.rs: missing the six-entry SharedHandleKind::ALL authority';
} else {
    $kindBlock = substr($types, $kindBlockStart, $kindBlockEnd - $kindBlockStart);
    foreach ([
        'SharedReference',
        'WeakReference',
        'WritableSharedReference',
        'WritableWeakReference',
        'ReadonlySharedReferenceAccess',
        'WritableSharedReferenceAccess',
    ] as $name) {
        if (!str_contains($kindBlock, "SharedHandleKind::{$name}")) {
            $failures[] = "types.rs: SharedHandleKind::ALL is missing {$name}";
        }
    }
}

$semantics = $read('crates/doriac/src/semantics.rs');
foreach ([
    'kind == SharedHandleKind::SharedReference && property == "referencedValue"',
    'kind != SharedHandleKind::SharedReference || property != "referencedValue"',
    'SharedHandleKind::WritableSharedReference => (',
    'SharedHandleKind::WeakReference | SharedHandleKind::WritableWeakReference => (',
] as $needle) {
    if (!str_contains($semantics, $needle)) {
        $failures[] = "semantics.rs: missing Stage 25a member rule `{$needle}`";
    }
}

$runtime = $read('crates/doria-rt/src/lib.rs');
$readonlyStart = strpos($runtime, 'pub struct DrSharedControlV1');
$readonlyEnd = $readonlyStart === false ? false : strpos($runtime, '}', $readonlyStart);
$writableStart = strpos($runtime, 'pub struct DrWritableSharedControlV1');
$writableEnd = $writableStart === false ? false : strpos($runtime, '}', $writableStart);
if ($readonlyStart === false || $readonlyEnd === false || $writableStart === false || $writableEnd === false) {
    $failures[] = 'doria-rt: missing distinct readonly/writable shared control structures';
} else {
    $readonly = substr($runtime, $readonlyStart, $readonlyEnd - $readonlyStart);
    $writable = substr($runtime, $writableStart, $writableEnd - $writableStart);
    foreach (['strong_references', 'weak_references', 'payload', 'drop_payload'] as $field) {
        if (!str_contains($readonly, $field) || !str_contains($writable, $field)) {
            $failures[] = "doria-rt: shared controls are missing {$field}";
        }
    }
    foreach (['readonly_accesses', 'writable_access_active'] as $field) {
        if (str_contains($readonly, $field) || !str_contains($writable, $field)) {
            $failures[] = "doria-rt: access-state field {$field} is not writable-family-only";
        }
    }
}
if (str_contains($runtime, 'AtomicUsize') || str_contains($runtime, 'AtomicBool')) {
    $failures[] = 'doria-rt: Stage 25a shared ownership must remain non-atomic';
}

$catalogue = $read('crates/doria-diagnostic-catalogue/src/lib.rs');
foreach ([
    'SHARED_ACCESS_CONFLICT_REASON_FACT',
    'Cannot Acquire Writable Access While Readonly Access Is Active',
    'Cannot Acquire Readonly Access While Writable Access Is Active',
    'Cannot Acquire Writable Access While Writable Access Is Active',
] as $needle) {
    if (!str_contains($catalogue, $needle)) {
        $failures[] = "diagnostic catalogue: missing conflict authority `{$needle}`";
    }
}

$parity = $read('crates/doriac/tests/fixtures/native_parity_examples.txt');
$ci = $read('.github/workflows/ci.yml');
foreach ([
    'main_stage25a_referenced_value_collision',
    'main_stage25a_shared_stress',
    'main_stage25a_weak_cycle',
    'main_stage25a_writable_shared_domains',
] as $fixture) {
    if (!str_contains($parity, $fixture)) {
        $failures[] = "native parity manifest: missing {$fixture}";
    }
    if (!str_contains($ci, $fixture)) {
        $failures[] = "CI leak coverage: missing {$fixture}";
    }
}

$plan = $read('docs/doria-end-to-end-plan.md');
$pipeline = $read('docs/notes/current-pipeline.md');
$decision = $read('docs/decisions/0106-shared-ownership-types-and-api.md');
$audit = $read('docs/notes/stage-25a-final-integration-audit.md');
foreach ([
    [$plan, '**Slice 4 — Final integration and LSP/editor sweep — Implemented.**'],
    [$plan, 'Stage 25a is complete'],
    [$pipeline, 'Stage 25a Slices 1 through 4 are implemented'],
    [$pipeline, 'Stage 25a is complete'],
    [$decision, 'Stage 25a Slice 4 completes'],
] as [$contents, $needle]) {
    if (!str_contains($contents, $needle)) {
        $failures[] = "Stage 25a authority: missing `{$needle}`";
    }
}

foreach (['Implementation Gap', 'Documentation Gap', 'Tooling Gap'] as $gap) {
    if (preg_match('/\|\s*' . preg_quote($gap, '/') . '\s*\|/', $audit) === 1) {
        $failures[] = "final audit: unresolved {$gap}";
    }
}

foreach ([
    'writable shared scalar and string payload execution',
    'shared handles through `mixed`',
    'atomic/thread-safe shared ownership',
    'PHP compatibility execution of shared ownership',
] as $deferred) {
    if (!str_contains(strtolower($audit), strtolower($deferred)) && !str_contains(strtolower($decision), strtolower($deferred))) {
        $failures[] = "Stage 25a authority: missing explicit deferral {$deferred}";
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Stage 25a completion check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Stage 25a completion check passed\n");
