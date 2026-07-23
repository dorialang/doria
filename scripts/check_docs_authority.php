<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

// Keys are "path:line:number". Keep this empty unless the repository contains
// a verified decision-shaped token that is not a citation. Every entry requires
// an inline rationale.
const DECISION_CITATION_ALLOWLIST = [];

function normalize_path(string $path): string
{
    return str_replace('\\', '/', $path);
}

function relative_path(string $root, string $path): string
{
    return ltrim(substr(normalize_path($path), strlen(normalize_path($root))), '/');
}

function is_skipped_path(string $path): bool
{
    foreach (['.git/', 'target/', 'node_modules/'] as $skip) {
        if (str_contains($path, $skip)) {
            return true;
        }
    }

    return false;
}

function is_historical_path(string $path): bool
{
    return str_starts_with($path, 'docs/notes/');
}

function is_decision_path(string $path): bool
{
    return str_starts_with($path, 'docs/decisions/');
}

function is_redirect_path(string $path): bool
{
    return $path === 'docs/doria-development-plan.md';
}

function is_end_to_end_plan(string $path): bool
{
    return $path === 'docs/doria-end-to-end-plan.md';
}

function is_active_scanned_path(string $path): bool
{
    if (is_historical_path($path) || is_decision_path($path) || is_redirect_path($path)) {
        return false;
    }

    return str_ends_with($path, '.md');
}

function is_naming_scanned_path(string $path): bool
{
    if (
        is_historical_path($path)
        || is_redirect_path($path)
        || $path === 'docs/php-interop-and-migration.md'
        || $path === 'editors/fixtures/rejected-syntax.doria'
    ) {
        return false;
    }

    if (is_decision_path($path)) {
        return true;
    }

    if (str_ends_with(strtolower($path), '.md')) {
        return true;
    }

    if ($path === 'editors/fixtures/latest-tokens.doria') {
        return true;
    }

    return str_starts_with($path, 'examples/')
        && str_ends_with(strtolower($path), '.doria');
}

/**
 * Doria source that must be charter-clean, checked strictly with no contextual
 * exemption. Prose may legitimately name a rejected spelling (fixit tables,
 * migration mappings, "considered and rejected" rationale); code never may.
 *
 * examples/errors/ and editors/fixtures/rejected-syntax.doria are exempt: those
 * corpora exist to demonstrate rejected spellings and their diagnostics.
 * examples/future/ is NOT exempt — per plan section 0 (two clocks), future
 * examples are accepted Doria that has not been implemented yet, so they are
 * held to the same charter as any other source.
 */
function is_doria_strict_code_path(string $path): bool
{
    if (!str_ends_with(strtolower($path), '.doria')) {
        return false;
    }

    if (str_starts_with($path, 'examples/errors/') || $path === 'editors/fixtures/rejected-syntax.doria') {
        return false;
    }

    return str_starts_with($path, 'examples/')
        || $path === 'editors/fixtures/latest-tokens.doria';
}

function line_is_negating_or_contextual(string $line): bool
{
    return preg_match('/\b(not|never|no|without|reject|rejected|invalid|reserved|literal|planned|future|PHP|interop|migration|historical|not Doria)\b/i', $line) === 1;
}

/**
 * `std::` is forbidden as a Doria stdlib spelling, but other languages'
 * standard libraries are legitimately discussed in rationale and prior art.
 */
function line_is_foreign_stdlib_context(string $line): bool
{
    return preg_match('/\b(Rust|C\+\+|Cargo|crate)\b/i', $line) === 1;
}

function add_failure(array &$failures, string $path, int $lineNumber, string $message, string $line): void
{
    $failures[] = "{$path}:{$lineNumber}: {$message}\n    {$line}";
}

/**
 * Return decision-shaped tokens outside Markdown code fences.
 *
 * @return list<array{line: int, number: string, text: string}>
 */
function find_decision_citations(string $contents): array
{
    $citations = [];
    $seen = [];
    $fenceMarker = null;
    $lines = preg_split('/\R/', $contents) ?: [];

    foreach ($lines as $index => $line) {
        if (preg_match('/^\s*(`{3,}|~{3,})/', $line, $fence) === 1) {
            $marker = $fence[1][0];
            if ($fenceMarker === null) {
                $fenceMarker = $marker;
            } elseif ($fenceMarker === $marker) {
                $fenceMarker = null;
            }
            continue;
        }

        if ($fenceMarker !== null) {
            continue;
        }

        if (preg_match_all('/\b(?:record|decision)s?\s+(\d{4})\b|\b(0\d{3})\b/i', $line, $matches, PREG_SET_ORDER) === 0) {
            continue;
        }

        foreach ($matches as $match) {
            $number = $match[1] !== '' ? $match[1] : $match[2];
            $key = ($index + 1) . ':' . $number;
            if (isset($seen[$key])) {
                continue;
            }

            $seen[$key] = true;
            $citations[] = [
                'line' => $index + 1,
                'number' => $number,
                'text' => $line,
            ];
        }
    }

    return $citations;
}

/**
 * @param array<string, list<string>> $recordFilesByNumber
 * @param array<string, string> $allowlist
 * @return list<string>
 */
function validate_decision_citations(
    string $path,
    string $contents,
    array $recordFilesByNumber,
    array $allowlist
): array {
    $citationFailures = [];

    foreach (find_decision_citations($contents) as $citation) {
        $allowlistKey = "{$path}:{$citation['line']}:{$citation['number']}";
        if (array_key_exists($allowlistKey, $allowlist)) {
            continue;
        }

        $matches = $recordFilesByNumber[$citation['number']] ?? [];
        if (count($matches) !== 1) {
            $count = count($matches);
            $citationFailures[] = "{$path}:{$citation['line']}: decision citation {$citation['number']} resolves to {$count} authored records; expected exactly one docs/decisions/{$citation['number']}-*.md — cite the subject until authored (§12 numbering policy)\n    {$citation['text']}";
        }
    }

    return $citationFailures;
}

/** @return list<string> */
function decision_citation_self_test_failures(): array
{
    $authored = ['0040' => ['docs/decisions/0040-panics-and-overflow-policy.md']];
    $tests = [
        'unauthored citation fails' => validate_decision_citations(
            'docs/decisions/fixture.md',
            'See decision 9999.',
            $authored,
            []
        ) !== [],
        'authored citation passes' => validate_decision_citations(
            'docs/decisions/fixture.md',
            'See record 0040.',
            $authored,
            []
        ) === [],
        'duplicate authored number fails' => validate_decision_citations(
            'docs/decisions/fixture.md',
            'See record 0040.',
            ['0040' => ['first.md', 'second.md']],
            []
        ) !== [],
        'plural citations pass' => validate_decision_citations(
            'docs/decisions/fixture.md',
            'Decisions 0040 and records 0040.',
            $authored,
            []
        ) === [],
        'bare unauthored token fails' => validate_decision_citations(
            'docs/decisions/fixture.md',
            'Legacy token 0999.',
            $authored,
            []
        ) !== [],
        'fenced citation is ignored' => validate_decision_citations(
            'docs/decisions/fixture.md',
            "```text\ndecision 9999\n```",
            $authored,
            []
        ) === [],
        'allowlisted citation passes' => validate_decision_citations(
            'docs/decisions/fixture.md',
            'Legacy token 0999.',
            $authored,
            ['docs/decisions/fixture.md:1:0999' => 'synthetic self-test']
        ) === [],
    ];

    $selfTestFailures = [];
    foreach ($tests as $name => $passed) {
        if (!$passed) {
            $selfTestFailures[] = "internal docs-authority error: decision citation self-test failed: {$name}";
        }
    }

    return $selfTestFailures;
}

$iterator = new RecursiveIteratorIterator(
    new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS)
);

$markdownFiles = [];
$namingFiles = [];
$doriaCodeFiles = [];
foreach ($iterator as $file) {
    if (!$file->isFile()) {
        continue;
    }

    $path = relative_path($root, $file->getPathname());
    if (is_skipped_path($path)) {
        continue;
    }

    if (str_ends_with(strtolower($path), '.md')) {
        $markdownFiles[] = $path;
    }

    if (is_naming_scanned_path($path)) {
        $namingFiles[] = $path;
    }

    if (is_doria_strict_code_path($path)) {
        $doriaCodeFiles[] = $path;
    }
}

sort($markdownFiles);
sort($namingFiles);
sort($doriaCodeFiles);

array_push($failures, ...decision_citation_self_test_failures());

$recordFilesByNumber = [];
foreach (glob($root . '/docs/decisions/[0-9][0-9][0-9][0-9]-*.md') ?: [] as $recordPath) {
    $filename = basename($recordPath);
    $number = substr($filename, 0, 4);
    $recordFilesByNumber[$number][] = relative_path($root, $recordPath);
}

$decisionCitationFiles = ['AGENTS.md', 'docs/doria-end-to-end-plan.md'];
foreach ($markdownFiles as $path) {
    if (is_decision_path($path) && !is_historical_path($path) && !is_redirect_path($path)) {
        $decisionCitationFiles[] = $path;
    }
}
$decisionCitationFiles = array_values(array_unique($decisionCitationFiles));
sort($decisionCitationFiles);

foreach ($decisionCitationFiles as $path) {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: unable to read file for decision citation checks";
        continue;
    }

    array_push(
        $failures,
        ...validate_decision_citations($path, $contents, $recordFilesByNumber, DECISION_CITATION_ALLOWLIST)
    );
}

if (array_filter($namingFiles, 'is_decision_path') === []) {
    $failures[] = 'internal docs-authority error: decision records are missing from naming checks';
}

foreach ($markdownFiles as $path) {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: unable to read file";
        continue;
    }

    $lines = preg_split('/\R/', $contents) ?: [];
    $active = is_active_scanned_path($path);
    $inPhpFence = false;

    foreach ($lines as $index => $line) {
        $lineNumber = $index + 1;
        $trimmedLine = trim($line);

        if (str_starts_with($trimmedLine, '```')) {
            if ($inPhpFence) {
                $inPhpFence = false;
                continue;
            }

            $inPhpFence = preg_match('/^```php\b/i', $trimmedLine) === 1;
            continue;
        }

        if ($active && str_contains($line, 'ROADMAP.md')) {
            add_failure($failures, $path, $lineNumber, 'active docs must not instruct contributors to use ROADMAP.md', $line);
        }

        if ($active && str_contains($line, 'docs/doria-development-plan.md')) {
            add_failure($failures, $path, $lineNumber, 'active docs must not list the superseded development plan as an authority', $line);
        }

        if ($active && preg_match('/^#{1,3}\s*(Next Compiler Work|Future implementation order|Near-term roadmap)\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not contain duplicate roadmap headings', $line);
        }

        if ($active && preg_match('/^#{1,3}\s*Roadmap\b/i', $line) === 1 && !is_end_to_end_plan($path)) {
            add_failure($failures, $path, $lineNumber, 'only the end-to-end plan may own roadmap headings', $line);
        }

        if ($active && preg_match('/\bdefault-public\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not use old default-public wording', $line);
        }

        if ($active && preg_match('/\bvisibility modifiers\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not teach a stale visibility-modifier model', $line);
        }

        if ($active && !$inPhpFence && preg_match('/\b(public|private|protected)\s+(string|int|float|bool|mixed|function)\b/', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not show stale public/private/protected Doria member syntax', $line);
        }

        if ($active && preg_match('/\bobject\s+(as\s+a\s+)?(core\s+)?type\b|\bcore\s+object\s+type\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not present object as a Doria core type', $line);
        }

        if ($active && preg_match('/\bresource\s+(as\s+a\s+)?(core\s+)?type\b|\bcore\s+resource\s+type\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not present resource as a Doria core type', $line);
        }

        if ($active && preg_match('/\bnull\s+type\b/i', $line) === 1 && !line_is_negating_or_contextual($line)) {
            add_failure($failures, $path, $lineNumber, 'active docs must not present null as a Doria source type', $line);
        }

        if ($active && preg_match('/\bMIR later\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not say MIR is merely later now that Stage 11 MIR is seeded', $line);
        }

        if ($active && preg_match('/\bdebug backend planned\b/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must not say the debug backend is only planned', $line);
        }

        if ($active && preg_match('/debug.*wasm.*recognized planned targets/i', $line) === 1) {
            add_failure($failures, $path, $lineNumber, 'active docs must distinguish current debug support from planned wasm support', $line);
        }
    }
}

$forbiddenNamingExamples = [
    'Int::wrapping_add',
    '->is_empty',
    '->retry_after',
    '->find_by_id',
    '->tenant_id',
    '->status_code',
];

foreach ($namingFiles as $path) {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: unable to read file for naming checks";
        continue;
    }

    $lines = preg_split('/\R/', $contents) ?: [];
    foreach ($lines as $index => $line) {
        $lineNumber = $index + 1;

        foreach ($forbiddenNamingExamples as $example) {
            if (str_contains($line, $example)) {
                add_failure(
                    $failures,
                    $path,
                    $lineNumber,
                    "active Doria guidance must not use stale snake_case member example {$example}",
                    $line
                );
            }
        }

        // The namespace-model direction: stdlib modules are namespaces under the reserved Doria\Std
        // root. `std::term` and friends were a Rust-shaped spelling that leaked
        // through the plan, decision records, and agent prompts before it was
        // caught; this guard prevents the regression.
        if (preg_match('/\bstd::/', $line) === 1 && !line_is_foreign_stdlib_context($line)) {
            add_failure(
                $failures,
                $path,
                $lineNumber,
                'Doria stdlib modules are namespaces (Doria\\Std\\Term, Doria\\Std\\Math), never Rust-shaped std:: paths',
                $line
            );
        }

        // Section 9.1: namespace segments are PascalCase with acronyms folded.
        if (preg_match('/\bDoria(?:\\\\[A-Za-z0-9_]+)*\\\\[A-Z]{2,}/', $line) === 1) {
            add_failure(
                $failures,
                $path,
                $lineNumber,
                'namespace segments fold acronyms: Doria\\Std\\Io / Doria\\Std\\Http / Doria\\Orm, never IO / HTTP / ORM',
                $line
            );
        }
    }
}

/**
 * Strict charter checks over Doria source. No contextual exemption: prose may
 * name a rejected spelling, code may not.
 */
$forbiddenVisibilityPattern = '/^\s*(public|private|protected)\b/';
foreach (['public Person $owner;', 'private List<int> $items;'] as $visibilityExample) {
    if (preg_match($forbiddenVisibilityPattern, $visibilityExample) !== 1) {
        $failures[] = "internal docs-authority error: visibility guard does not cover {$visibilityExample}";
    }
}
$forbiddenPrintStatementPattern = '/^\s*print\b/';
foreach (['print "text";', 'print 1;', 'print true;', 'print getName();'] as $printExample) {
    if (preg_match($forbiddenPrintStatementPattern, $printExample) !== 1) {
        $failures[] = "internal docs-authority error: print guard does not cover {$printExample}";
    }
}

$forbiddenCodeSpellings = [
    ['/\binstanceof\b/', 'instanceof is rejected permanently; the namespace-model decision uses the type-test and narrowing operator `is`'],
    ['/\breadline\s*\(/', 'readline is rejected as a fused name; the stdin built-in is read_line'],
    ['/__toString/', 'Doria has no __toString magic method; display conversion is Displayable::toString'],
    [$forbiddenPrintStatementPattern, 'print is rejected; echo is the spelling'],
    ['/::\s*\$/', 'Doria static access is sigil-free; use Foo::prop rather than Foo::$prop'],
    ['/\bstatic\s*::/', 'Doria has no late static binding; use the reserved self:: qualifier'],
    ['/\bstd::/', 'Doria stdlib modules are namespaces (Doria\\Std\\Term), never std:: paths'],
    [
        $forbiddenVisibilityPattern,
        'Doria has no public/private/protected; members are accessible by default and internal marks implementation details',
    ],
];

foreach ($doriaCodeFiles as $path) {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: unable to read file for Doria source charter checks";
        continue;
    }

    $lines = preg_split('/\R/', $contents) ?: [];
    foreach ($lines as $index => $line) {
        foreach ($forbiddenCodeSpellings as [$pattern, $message]) {
            if (preg_match($pattern, $line) === 1) {
                add_failure($failures, $path, $index + 1, $message, $line);
            }
        }
    }
}

$namingAuthorityPath = 'docs/doria-end-to-end-plan.md';
$namingAuthority = file_get_contents($root . '/' . $namingAuthorityPath);
if ($namingAuthority === false) {
    $failures[] = "{$namingAuthorityPath}: unable to read naming authority";
} else {
    foreach (['Int::wrappingAdd', '->isEmpty', '->retryAfter', '->findById', '->tenantId'] as $example) {
        if (!str_contains($namingAuthority, $example)) {
            $failures[] = "{$namingAuthorityPath}: missing required corrected naming example {$example}";
        }
    }

    foreach (['ClassName::member', 'Foo::prop', 'self::age', 'self::create()', 'Foo::$prop', 'static::'] as $spelling) {
        if (!str_contains($namingAuthority, $spelling)) {
            $failures[] = "{$namingAuthorityPath}: missing required static-access authority spelling {$spelling}";
        }
    }

    // The bullet the examples live under. Previously an unenforced convention
    // communicated by hand to contributors and agents; now a checked invariant.
    $namingBullet = 'Canonical member-casing examples (normative; preserve these spellings)';
    if (!str_contains($namingAuthority, $namingBullet)) {
        $failures[] = "{$namingAuthorityPath}: missing required naming-authority bullet \"{$namingBullet}\"";
    }
}

if ($namingAuthority !== false) {
    $requiredIoGuidance = [
        'Formatted I/O — the v1.0 minimal set (record 0074)',
        '`read_file(string $path): string`',
        '`read_file_bytes(string $path): Bytes`',
        '`write_file_bytes(string $path, Bytes $contents): void`',
        '`append_file_bytes(string $path, Bytes $contents): void`',
    ];
    foreach ($requiredIoGuidance as $guidance) {
        if (!str_contains($namingAuthority, $guidance)) {
            $failures[] = "{$namingAuthorityPath}: missing required I/O authority guidance {$guidance}";
        }
    }

    foreach (['Formatted I/O — the v1.0 minimal set (record 0071)', '`read_file(): string`', '`read_file_bytes(): Bytes`', '`read_file_bytes(string $path, ...): Bytes`'] as $staleGuidance) {
        if (str_contains($namingAuthority, $staleGuidance)) {
            $failures[] = "{$namingAuthorityPath}: contains stale I/O authority guidance {$staleGuidance}";
        }
    }

    // ---------------------------------------------------------------------
    // Namespace-model authority.
    //
    // PAIRING NOTE: these assertions land WITH the plan commit that performs
    // the Doria\Std sweep and records the namespace-model direction. Enabling
    // them against a plan that still carries `std::term` spellings will fail
    // CI. Land both, or neither.
    // ---------------------------------------------------------------------
    $requiredNamespaceGuidance = [
        'Doria\Std\Term',
        'Doria\Std\Math',
        'read_line',
    ];
    foreach ($requiredNamespaceGuidance as $guidance) {
        if (!str_contains($namingAuthority, $guidance)) {
            $failures[] = "{$namingAuthorityPath}: missing required namespace/naming authority guidance {$guidance}";
        }
    }
}

// The repository is not published until the end-to-end plan is complete.
// Public entry documents therefore describe the released product instead of
// exposing the compiler's interim stage or implementation status.
$publicEntryPaths = ['README.md', 'CONTRIBUTING.md'];
$forbiddenPublicStatusPatterns = [
    '/^##\s+Status\s*$/mi' => 'public entry documents must not contain an interim status section',
    '/\b(?:stage|phase)\s+[A-Z0-9]+/i' => 'public entry documents must not expose internal stage or phase labels',
    '/\b(?:planned|not yet|coming soon|work in progress|prototype|current compiler branch|current slice|supported today|available today|future work)\b/i' => 'public entry documents must not expose interim implementation status',
    '/\bearly(?:,\s+active)?\s+development\b/i' => 'public entry documents must not carry a pre-release development disclaimer',
    '/\bsyntax highlighting is editor UX\b|\bnot a language implementation\b|\bhighlighting (?:does not|doesn\'t) mean\b|\bcompiler correctly reports it as unsupported\b/i' => 'public entry documents must not explain interim highlighting/compiler drift',
];

foreach ($publicEntryPaths as $path) {
    $contents = file_get_contents($root . '/' . $path);
    if ($contents === false) {
        $failures[] = "{$path}: unable to read public entry document";
        continue;
    }

    foreach ($forbiddenPublicStatusPatterns as $pattern => $message) {
        if (preg_match($pattern, $contents) === 1) {
            $failures[] = "{$path}: {$message}";
        }
    }
}

// Editor and language-server ownership is external to this compiler repository.
// Guard both authorities because an in-repo stage obligation can otherwise
// contradict the repository boundary while every individual sentence remains
// plausible in isolation.
$agentsPath = 'AGENTS.md';
$agents = file_get_contents($root . '/' . $agentsPath);
$languageServerRepo = 'dorialang/doria-language-server';

foreach ([$namingAuthorityPath => $namingAuthority, $agentsPath => $agents] as $path => $contents) {
    if ($contents === false) {
        $failures[] = "{$path}: unable to read language-server ownership guidance";
        continue;
    }

    if (!str_contains($contents, $languageServerRepo)) {
        $failures[] = "{$path}: missing external language-server ownership guidance {$languageServerRepo}";
    }
}

if ($namingAuthority !== false) {
    foreach ([
        'updated editor token guardrails when vocabulary changes',
        'Every stage that activates syntax must ship an **LSP no-false-diagnostics** test',
        '**LSP no-false-diagnostics test** per §0',
    ] as $staleOwnership) {
        if (str_contains($namingAuthority, $staleOwnership)) {
            $failures[] = "{$namingAuthorityPath}: contains stale in-repo editor/LSP obligation {$staleOwnership}";
        }
    }
}

if ($agents !== false && str_contains($agents, 'Every stage that activates syntax ships an LSP no-false-diagnostics test')) {
    $failures[] = "{$agentsPath}: contains stale in-repo LSP test ownership guidance";
}

if ($failures !== []) {
    fwrite(STDERR, "docs authority check failed:\n");
    foreach ($failures as $failure) {
        fwrite(STDERR, "- {$failure}\n");
    }
    exit(1);
}

echo "docs authority check passed\n";
