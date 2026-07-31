<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$manifestPath = $root . '/docs/notes/php-string-capability-inventory.json';
$decisionPath = $root . '/docs/decisions/0103-string-companion-surface.md';
$planPath = $root . '/docs/doria-end-to-end-plan.md';

$failures = [];

function fail(array &$failures, string $message): void
{
    $failures[] = $message;
}

function non_empty_string(mixed $value): bool
{
    return is_string($value) && trim($value) !== '';
}

try {
    $manifest = json_decode(
        (string) file_get_contents($manifestPath),
        true,
        512,
        JSON_THROW_ON_ERROR,
    );
} catch (Throwable $error) {
    fwrite(STDERR, "string API completeness check failed: {$error->getMessage()}\n");
    exit(1);
}

$allowedClassifications = [
    'Existing String Intrinsic',
    'Existing String Companion',
    'Proposed String Companion',
    'Bytes Companion',
    'Encoding Domain',
    'Unicode Domain',
    'Text Analysis Domain',
    'Text Layout Domain',
    'Collation Domain',
    'Regular Expression Domain',
    'HTML Domain',
    'MIME Domain',
    'URL Or Query Domain',
    'CSV Domain',
    'Hash Or Crypto Domain',
    'Locale Domain',
    'Random Domain',
    'Formatting Domain',
    'I/O Domain',
    'Source Escaping Domain',
    'Derivable Without New API',
    'Rejected Alias',
    'Rejected Legacy Behavior',
    'Not Applicable To Doria',
    'Deferred Pending Another Type',
    'Unresolved Design Fork',
];
$allowedUnits = [
    'bytes',
    'code-points',
    'graphemes',
    'display-width',
    'locale',
    'encoding-dependent',
    'not-applicable',
];
$allowedMigrationActions = [
    'Direct Rewrite',
    'Rewrite With Semantic Warning',
    'Rewrite Requires Domain Module',
    'Rewrite Requires Human Review',
    'No Doria Equivalent By Design',
    'Deprecated PHP Input',
    'Unsupported Until Named Dependency Lands',
];
$requiredRowFields = [
    'phpSurface',
    'phpName',
    'phpAliases',
    'phpCategory',
    'phpUnit',
    'phpSummary',
    'doriaClassification',
    'doriaOwner',
    'doriaCanonicalSpelling',
    'doriaStatus',
    'migrationAction',
    'semanticDifferences',
    'decisionRequired',
    'dependency',
    'canonicalPhpName',
    'notes',
];

if (($manifest['schemaVersion'] ?? null) !== 1) {
    fail($failures, 'manifest schemaVersion must be 1');
}

$audit = $manifest['audit'] ?? null;
if (!is_array($audit)) {
    fail($failures, 'manifest audit metadata is missing');
    $audit = [];
}
foreach (['date', 'phpReleaseAtAudit', 'phpManualCopyright', 'versionNotes', 'sources', 'intlBoundaryReviews'] as $field) {
    if (!array_key_exists($field, $audit) || $audit[$field] === [] || $audit[$field] === '') {
        fail($failures, "audit metadata is missing {$field}");
    }
}
if (!preg_match('/^\d{4}-\d{2}-\d{2}$/', (string) ($audit['date'] ?? ''))) {
    fail($failures, 'audit date must use YYYY-MM-DD');
}
if (!preg_match('/\b20\d{2}\b/', (string) ($audit['phpManualCopyright'] ?? ''))) {
    fail($failures, 'PHP manual copyright year is missing');
}

$reviewResolution = $manifest['reviewResolution'] ?? null;
if (!is_array($reviewResolution)) {
    fail($failures, 'manifest reviewResolution metadata is missing');
    $reviewResolution = [];
}
if (($reviewResolution['date'] ?? null) !== '2026-07-31') {
    fail($failures, 'reviewResolution must record the 2026-07-31 approval');
}
$approvedStringOperations = $reviewResolution['approvedCanonicalStringOperations'] ?? [];
$requiredApprovedStringOperations = [
    'String::containsIgnoreCase',
    'String::startsWithIgnoreCase',
    'String::endsWithIgnoreCase',
    'String::indexOfIgnoreCase',
    'String::lastIndexOfIgnoreCase',
    'String::lowerFirst',
    'String::upperFirst',
    'String::countOccurrences',
];
foreach ($requiredApprovedStringOperations as $operation) {
    if (!is_array($approvedStringOperations) || !in_array($operation, $approvedStringOperations, true)) {
        fail($failures, "reviewResolution is missing approved operation {$operation}");
    }
}

$entries = $manifest['entries'] ?? null;
if (!is_array($entries)) {
    fail($failures, 'manifest entries must be an array');
    $entries = [];
}

$rowsBySurface = [];
$rowKeys = [];
foreach ($entries as $index => $row) {
    if (!is_array($row)) {
        fail($failures, "entry {$index} is not an object");
        continue;
    }
    foreach ($requiredRowFields as $field) {
        if (!array_key_exists($field, $row)) {
            fail($failures, "entry {$index} is missing {$field}");
        }
    }

    $surface = (string) ($row['phpSurface'] ?? '');
    $name = (string) ($row['phpName'] ?? '');
    $key = "{$surface}:{$name}";
    if (isset($rowKeys[$key])) {
        fail($failures, "duplicate capability row {$key}");
    }
    $rowKeys[$key] = true;
    $rowsBySurface[$surface][$name] = ($rowsBySurface[$surface][$name] ?? 0) + 1;

    foreach (['phpSurface', 'phpName', 'phpCategory', 'phpSummary', 'doriaClassification', 'doriaOwner', 'doriaStatus', 'migrationAction', 'notes'] as $field) {
        if (!non_empty_string($row[$field] ?? null)) {
            fail($failures, "{$key} has an empty {$field}");
        }
    }
    if (!in_array($row['doriaClassification'] ?? null, $allowedClassifications, true)) {
        fail($failures, "{$key} uses an unknown Doria classification");
    }
    if (!in_array($row['phpUnit'] ?? null, $allowedUnits, true)) {
        fail($failures, "{$key} uses an unknown unit");
    }
    if (!in_array($row['migrationAction'] ?? null, $allowedMigrationActions, true)) {
        fail($failures, "{$key} uses an unknown migration action");
    }
    if (!is_array($row['phpAliases'] ?? null) || !is_array($row['semanticDifferences'] ?? null)) {
        fail($failures, "{$key} aliases and semanticDifferences must be arrays");
    }
    if (!is_bool($row['decisionRequired'] ?? null)) {
        fail($failures, "{$key} decisionRequired must be boolean");
    }
    if (($row['doriaStatus'] ?? null) === 'deferred'
        && (!non_empty_string($row['doriaOwner'] ?? null) || !non_empty_string($row['dependency'] ?? null))
    ) {
        fail($failures, "{$key} is deferred without an owner and named dependency");
    }
    if (($row['doriaClassification'] ?? null) === 'Proposed String Companion'
        && !in_array($row['phpUnit'] ?? null, ['bytes', 'code-points', 'graphemes', 'display-width'], true)
    ) {
        fail($failures, "{$key} proposes a String operation without a concrete text unit");
    }
}
foreach ($entries as $row) {
    if (!is_array($row) || ($row['doriaClassification'] ?? null) !== 'Rejected Alias') {
        continue;
    }
    $surface = (string) ($row['phpSurface'] ?? '');
    $name = (string) ($row['phpName'] ?? '');
    $canonical = $row['canonicalPhpName'] ?? null;
    if (!non_empty_string($canonical) || !isset($rowKeys["{$surface}:{$canonical}"])) {
        fail($failures, "{$surface}:{$name} does not point to a canonical PHP row");
    }
}

$catalogueSurfaces = ['core', 'mbstring', 'grapheme'];
$seenSources = [];
foreach (($audit['sources'] ?? []) as $source) {
    if (!is_array($source)) {
        fail($failures, 'audit source metadata contains a non-object');
        continue;
    }
    $surface = (string) ($source['surface'] ?? '');
    if (!in_array($surface, $catalogueSurfaces, true)) {
        fail($failures, "unexpected source surface {$surface}");
        continue;
    }
    if (isset($seenSources[$surface])) {
        fail($failures, "duplicate source metadata for {$surface}");
    }
    $seenSources[$surface] = true;

    $expected = $source['expectedNames'] ?? null;
    if (!is_array($expected) || $expected === []) {
        fail($failures, "{$surface} expectedNames is missing");
        continue;
    }
    if (count($expected) !== count(array_unique($expected))) {
        fail($failures, "{$surface} expectedNames contains duplicates");
    }
    if (($source['count'] ?? null) !== count($expected)) {
        fail($failures, "{$surface} declared count does not match expectedNames");
    }

    $actual = array_keys($rowsBySurface[$surface] ?? []);
    sort($actual);
    $expectedSorted = $expected;
    sort($expectedSorted);
    if ($actual !== $expectedSorted) {
        $missing = array_values(array_diff($expectedSorted, $actual));
        $extra = array_values(array_diff($actual, $expectedSorted));
        fail(
            $failures,
            "{$surface} inventory mismatch; missing=[" . implode(', ', $missing)
            . '] extra=[' . implode(', ', $extra) . ']',
        );
    }
    foreach (($rowsBySurface[$surface] ?? []) as $name => $count) {
        if ($count !== 1) {
            fail($failures, "{$surface}:{$name} appears {$count} times");
        }
    }
}
foreach ($catalogueSurfaces as $surface) {
    if (!isset($seenSources[$surface])) {
        fail($failures, "source metadata for {$surface} is missing");
    }
}

$concepts = array_column(
    is_array($audit['intlBoundaryReviews'] ?? null) ? $audit['intlBoundaryReviews'] : [],
    'concept',
);
foreach (['Normalizer', 'Collator', 'Transliterator', 'BreakIterator', 'IntlChar', 'UConverter'] as $concept) {
    if (!in_array($concept, $concepts, true)) {
        fail($failures, "Intl boundary review is missing {$concept}");
    }
}

$decision = (string) file_get_contents($decisionPath);
$plan = (string) file_get_contents($planPath);
if (!str_contains($decision, 'reviewed v1 inventory')) {
    fail($failures, 'Decision 0103 does not record the completed review');
}
if (!str_contains($decision, 'string-api-completeness-audit.md')) {
    fail($failures, 'Decision 0103 does not link the completeness audit');
}
foreach ($requiredApprovedStringOperations as $operation) {
    if (!str_contains($decision, $operation)) {
        fail($failures, "Decision 0103 is missing approved operation {$operation}");
    }
}
if (!str_contains($plan, 'Minimum String Runtime Surface — Implemented')) {
    fail($failures, 'Minimum String Runtime Surface is not recorded as implemented');
}
if (!str_contains($plan, 'Decision 0103 Completeness Review — Implemented')) {
    fail($failures, 'the completed Decision 0103 completeness review is missing');
}

if ($failures !== []) {
    fwrite(STDERR, "string API completeness check failed:\n");
    foreach ($failures as $failure) {
        fwrite(STDERR, "- {$failure}\n");
    }
    exit(1);
}

$counts = [];
foreach ($catalogueSurfaces as $surface) {
    $counts[] = "{$surface}=" . count($rowsBySurface[$surface] ?? []);
}
echo 'string API completeness check passed: ' . implode(', ', $counts)
    . ', total=' . count($entries) . "\n";
