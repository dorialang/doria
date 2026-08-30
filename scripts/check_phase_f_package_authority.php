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
            $failures[] = "{$path}: missing Phase F authority `{$needle}`";
        }
    }
};

$forbid = static function (string $path, string $contents, array $needles) use (&$failures): void {
    foreach ($needles as $needle) {
        if (str_contains($contents, $needle)) {
            $failures[] = "{$path}: contains stale or duplicate Phase F authority `{$needle}`";
        }
    }
};

$namespacePath = 'docs/decisions/0117-namespaces-compile-time-autoloading-hybrid-source-layout-and-package-compilation-graphs.md';
$batonPath = 'docs/decisions/0118-baton-manifests-package-dependencies-resolution-lockfiles-workspaces-and-caches.md';
$slice1Path = 'docs/decisions/0126-baton-schema-2-local-package-target-and-selection-spellings.md';
$slice2Path = 'docs/decisions/0127-baton-dependency-resolution-lockfile-cache-and-offline-semantics.md';
$planPath = 'docs/doria-end-to-end-plan.md';
$pipelinePath = 'docs/notes/current-pipeline.md';
$auditPath = 'docs/notes/plan-open-questions-audit.md';
$specPath = 'SPEC.md';
$readmePath = 'README.md';
$decision0028Path = 'docs/decisions/0028-namespaces-use-include-and-directives.md';
$decision0115Path = 'docs/decisions/0115-match-expressions-patterns-exhaustiveness-narrowing-and-ownership.md';
$websiteGuidelinesPath = 'docs/website-content-guidelines.md';

$namespace = $read($namespacePath);
$baton = $read($batonPath);
$slice1 = $read($slice1Path);
$slice2 = $read($slice2Path);
$plan = $read($planPath);
$pipeline = $read($pipelinePath);
$audit = $read($auditPath);
$spec = $read($specPath);
$readme = $read($readmePath);
$decision0028 = $read($decision0028Path);
$decision0115 = $read($decision0115Path);
$websiteGuidelines = $read($websiteGuidelinesPath);

$require($namespacePath, $namespace, [
    '**Status:** Accepted',
    'The public manifest term is `autoload`',
    'autoloading happens during compilation',
    'autoloader. A finished program never searches',
    '[autoload.namespaces]',
    '[autoload-dev.namespaces]',
    'Generated roots are injected by Baton',
    'Every discovered file is parsed, indexed, and checked',
    'The default file pattern is `**/*.doria`',
    'namespace prefixes overlap',
    'Namespace directories are strict',
    'externally accessible type named `PostController` belongs in',
    'one primary externally accessible type',
    '`internal` helper declarations may share',
    'Free functions and constants may use descriptive bundle files',
    'may contain bundle',
    'only a selected binary target entry file may contain',
    '`include` adds one specifically named same-package file',
    'required and include-once',
    'through several includes or both autoload',
    'canonical publishable identity is lowercase `vendor/package`',
    'Package identity and Doria namespace identity are separate',
    '`internal` means accessible anywhere inside the declaring package',
    'Several packages may',
    'Duplicate fully qualified symbols are compile errors',
    'versioned JSON build plan',
    'make `doriac` parse `Baton.toml`',
    'parse Doria declarations',
    '## Namespace Syntax And Resolution',
    'Any name containing `\` is absolute',
    'explicit imports -> current namespace -> edition prelude',
    'Wildcard imports such as `use Doria\Std\Math\*;` are rejected',
    'standard-library root is `Doria\Std`',
    'compiler injects a small documented prelude',
    'Prelude additions are edition-scoped',
    'Language intrinsics are resolved before namespace lookup',
    'cannot be redeclared',
    'Stage 31 Slice 1',
    'Stage 31 Slice 2',
    'Stage 31 Slices 1 and 2 implemented',
    'Stage 31 Slice 1',
    'Stage 31 Slice 2',
]);

$require($batonPath, $baton, [
    '**Status:** Accepted',
    'manifest-version = 2',
    'Manifest schema 1 keeps its original bootstrap meaning',
    'no autoload, no dependencies',
    'Transitive dependencies',
    'may not expose a type from an undeclared direct',
    'one version of each canonical package',
    'Every package dependency cycle is rejected',
    'first resolver supports path and Git dependencies',
    'always records the exact commit',
    'Source transport and artifact role are separate',
    'canonical source URL without embedded credentials',
    'arbitrary ZIP or tarball URLs',
    'Packages use SemVer',
    'toolchain',
    'CalVer',
    '`Baton.lock` is machine-generated, deterministic JSON',
    'one lockfile at the workspace root',
    'Build and machine facts do not belong in `Baton.lock`',
    '`baton install` uses an existing lockfile exactly',
    'never silently update a valid lockfile',
    'content-addressed dependency cache',
    'never grants package-internal access',
    '[dev-dependencies]',
    '[processors]',
    'Processors are explicitly declared',
    'no arbitrary `build.doria`',
    'global content-addressed dependency cache',
    'Offline mode never reaches the network',
    'Stage 33 has three implementation slices',
    'all three Stage 33 slices are complete',
    'Stage 33 and Phase F',
    'are complete',
    'Native Testing Foundation Slice 1 is complete',
    'Stage 34 waits for the foundation',
]);
$require($slice1Path, $slice1, [
    '**Status:** Accepted',
    '**Implementation Status:** Implemented By Stage 33 Slice 1',
    '**Amends:** Decisions 0117, 0118, and 0124',
    'publishable = false',
    'local/<name>',
    '[targets.library]',
    '[[targets.binary]]',
    '--binary <name>',
    '--library',
    'Decision 0127 owns the implemented path',
    'Implemented Stage 33 Slice 3',
]);
$require($slice2Path, $slice2, [
    '**Status:** Accepted',
    '**Implementation Status:** Implemented By Stage 33 Slice 2',
    '**Amends:** Decisions 0118, 0124, and 0126',
    '[dependencies]',
    'Path dependencies are live inputs',
    'Git dependencies require scoped package',
    'One resolved graph contains one node for each compiler package identity',
    '`Baton.lock` uses strict deterministic JSON schema 1',
    'A missing lock is the normal first-install state',
    'without reporting a missing-lock diagnostic',
    'Lock absence is command-specific',
    'manifest-incompatible existing locks always receive precise diagnostics',
    '`baton install`',
    '`baton add`',
    '`baton remove`',
    '`baton update`',
    '`baton fetch`',
    'Offline behavior is one resolver-level network policy',
    'Stage 33 Slices 1 through 3 are complete',
]);

$require($planPath, $plan, [
    'Decision 0117:',
    'Decision 0118:',
    'Stage 31 — Namespaces and package compilation graph',
    'Stage 33 — Baton package and dependency workflow',
    'Stage 29 is complete',
    'Decision 0117 owns the complete namespace syntax and resolution contract',
]);
$require($pipelinePath, $pipeline, [
    'Phase F Namespace, Autoload, Package, And Dependency Authority — Accepted',
    'Stage 29 — Complete',
    'Stage 29 Slice 2 — Complete',
    'Corrective Beat: Native Collection Property Initializers — Complete',
    'Stage 29 Slice 3 — Complete',
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
$require($auditPath, $audit, [
    'Resolved. Decision 0117',
    'Decision 0118 accepts schema 2',
]);
$require($decision0028Path, $decision0028, [
    'Phase F amendment: Decision 0117',
    "record's distinction among",
]);
$require($decision0115Path, $decision0115, [
    'Decision 0117 satisfies the former Stage 31 authority prerequisite',
    'Decision 0118 owns dependency discovery and resolution',
]);
$require($specPath, $spec, [
    'Decision 0117 defines compile-time autoloading',
    'Decision 0126 fixes schema-2 local/scoped package identity',
    'Decision 0127 fixes the implemented normal path/Git dependency resolver',
    'The public manifest term `autoload`',
    '`internal` is accessible throughout',
]);
$require($readmePath, $readme, [
    "Baton is Doria's accepted project and package tool",
    'exact schema 1 compatibility',
    'strict schema 2',
    'projects with local/scoped package identity',
    'normal path or Git dependencies',
    '`Baton.lock` files pin exact Git commits',
    'neither compiled programs',
    'nor the compiler search for project source',
]);
$require($websiteGuidelinesPath, $websiteGuidelines, [
    '## Package And Autoload Positioning',
    'uses `autoload` for Baton-managed source discovery',
    'compiled programs do not search for or load Doria source files at runtime',
    '`use` gives a shorter name',
    '`include` explicitly adds one same-package source file at compile time',
    'dependencies add other packages',
    'Do not add stage numbers',
]);

$forbid($decision0115Path, $decision0115, [
    'Stage 31 still requires a pre-implementation authority amendment',
]);
$forbid($planPath, $plan, [
    '### Decision 0117 detail',
    '**Any name containing `\` is absolute.',
    '**Group `use`, no wildcards.**',
    '**The prelude is a small documented list',
    '**Intrinsics are language, not library',
]);
$forbid($readmePath, $readme, [
    'Baton resolves that project',
    'dependency resolution is recorded in JSON `Baton.lock`',
    'workspaces share one lockfile',
]);
$forbid($planPath, $plan, [
    'Stage 33 Slice 2 — Next',
    'Stage 33 Slice 2 is next',
]);

foreach (['Andrew', 'Lucy', 'Masiye'] as $privateName) {
    foreach ([$namespacePath => $namespace, $batonPath => $baton, $slice1Path => $slice1, $slice2Path => $slice2] as $path => $contents) {
        if (str_contains($contents, $privateName)) {
            $failures[] = "{$path}: contains private or family name `{$privateName}`";
        }
    }
}

if ($failures !== []) {
    fwrite(STDERR, "Phase F package authority check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, "Phase F package authority check passed\n");
