#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

$read = static function (string $path) use ($root, &$failures): string {
    $contents = @file_get_contents($root . '/' . $path);
    if (!is_string($contents)) {
        $failures[] = "{$path}: required Stage 35 authority file is missing";
        return '';
    }

    return $contents;
};

$require = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (!str_contains($contents, $needle)) {
            $failures[] = "{$path}: missing Stage 35 authority `{$needle}`";
        }
    }
};

$forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (str_contains($contents, $needle)) {
            $failures[] = "{$path}: contains stale or forbidden Stage 35 claim `{$needle}`";
        }
    }
};

$paths = [
    'decision' => 'docs/decisions/0134-interfaces-traits-core-value-contracts-and-public-iteration.md',
    'agents' => 'AGENTS.md',
    'spec' => 'SPEC.md',
    'readme' => 'README.md',
    'plan' => 'docs/doria-end-to-end-plan.md',
    'pipeline' => 'docs/notes/current-pipeline.md',
    'stdlib' => 'docs/stdlib-reference.md',
    'api' => 'docs/api-design-guidelines.md',
    'inheritance' => 'docs/class-inheritance.md',
    'diagnostics' => 'docs/diagnostic-style.md',
    'metadata' => 'docs/attribute-metadata-protocol.md',
    'restrictions' => 'docs/notes/temporary-language-restrictions-audit.md',
    'nativeParity' => 'docs/notes/native-parity-matrix.md',
    'collectionsAudit' => 'docs/notes/collection-surface-audit.md',
    'selfHosting' => 'docs/self-hosting.md',
    'websiteGuidance' => 'docs/website-content-guidelines.md',
    'openQuestions' => 'docs/notes/plan-open-questions-audit.md',
];

$files = [];
foreach ($paths as $key => $path) {
    $files[$key] = $read($path);
}

$require($paths['decision'], $files['decision'], [
    '# Decision 0134:',
    '**Status:** Accepted',
    '**Implementation Status:** Stage 35 Authority Accepted; Slice 1 Next',
    'interface Equatable<T>',
    'function equals(T $other): bool;',
    'function hash(): uint64;',
    'function clone(): self;',
    'function iterator(): Iterator<T>;',
    'function hasCurrent(): bool;',
    'function current(): T;',
    'writable function advance(): void;',
    'The compiler-known Iterator methods declare',
    'no checked Errors',
    'concrete implementing type specialization',
    'built-in array or collection specialization',
    '| TraitRef "::" Name "as" Name ";"',
    '| TraitRef "::" Name "as" "internal" Name? ";"',
    'Type parameters remain excluded from attribute targets under Decision 0125.',
    'User interfaces remain method-only.',
    'Stage 36 remains their sole owner.',
    'all six existing families',
    'Slice 1: Grammar, Graphs, And Conformance',
    'Slice 5: Cross-Repository Closure',
]);

$require($paths['plan'], $files['plan'], [
    'Stage 35 — Interfaces And Traits — Authority Accepted; Slice 1 Next',
    'Slice 1 — Next: Grammar, Graphs, And Conformance',
    'Slice 2 — Scheduled: Interface Runtime And Ownership',
    'Slice 3 — Scheduled: Core Contracts And Public Iteration',
    'Slice 4 — Scheduled: Trait Composition',
    'Slice 5 — Scheduled: Cross-Repository Closure',
]);

$require($paths['pipeline'], $files['pipeline'], [
    'Stage 35 Interfaces And Traits authority is accepted under Decision 0134',
    'Slice 1 Grammar, Graphs, And Conformance is next',
]);

$require($paths['spec'], $files['spec'], [
    'Decision 0134 defines generic nominal interfaces',
    'User interfaces remain method-only',
    'Traits cannot declare lifecycle methods',
    'There is no runtime trait object',
]);

$require($paths['stdlib'], $files['stdlib'], [
    'equals(T $other): bool',
    'hash(): uint64',
    'clone(): self',
    'iterator(): Iterator<T>',
    'hasCurrent(): bool',
    'current(): T',
    'advance(): void',
    'Copy-or-Cloneable',
]);

foreach ([
    'agents', 'readme', 'api', 'inheritance', 'diagnostics', 'metadata',
    'restrictions', 'nativeParity', 'selfHosting', 'websiteGuidance',
    'openQuestions',
] as $key) {
    $require($paths[$key], $files[$key], ['Decision 0134']);
}

$require($paths['collectionsAudit'], $files['collectionsAudit'], [
    'Decision 0134',
    'foreach-only projections',
    'silently turn them into owned lists',
]);

$staleStatus = [
    'Stage 35 — Interfaces And Traits — Next',
    'Stage 35 Interfaces And Traits — Next',
    'Stage 35 Interfaces And Traits - Next',
    'Stage 35 interfaces and traits is next',
    'Stage 35 is next',
];

foreach ($files as $key => $contents) {
    $forbid($paths[$key], $contents, $staleStatus);
}

$forbid($paths['decision'], $files['decision'], [
    'TraitRef "::" Name "as" ("internal")? Name? ";"',
    'interface conversion allocates a wrapper',
    'primitives inhabit interface-typed slots',
    'runtime trait object is required',
    'property hooks are part of Stage 35',
]);

$require($paths['readme'], $files['readme'], [
    'currently provides `map`, `filter`',
    '`filter` currently',
    'preserves Copy elements',
]);

foreach ([
    '0029', '0030', '0079', '0082', '0087', '0089', '0093', '0096',
    '0100', '0102', '0105', '0106', '0110', '0113', '0119', '0121',
    '0125', '0129', '0130', '0131', '0132', '0133',
] as $number) {
    $matches = glob($root . "/docs/decisions/{$number}-*.md") ?: [];
    if (count($matches) !== 1) {
        $failures[] = "decision {$number}: expected one authored record";
        continue;
    }
    $contents = @file_get_contents($matches[0]);
    if (!is_string($contents) || !str_contains($contents, 'Decision 0134')) {
        $failures[] = basename($matches[0]) . ': missing Decision 0134 amendment pointer';
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Stage 35 authority check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Stage 35 authority check passed.\n");
