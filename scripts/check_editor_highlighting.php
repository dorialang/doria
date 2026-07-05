#!/usr/bin/env php
<?php

declare(strict_types=1);

/**
 * Check that Doria editor highlighters cover the accepted token vocabulary.
 */

$root = dirname(__DIR__);

$vscodePackage = $root . '/editors/vscode/doria/package.json';
$vscodeGrammar = $root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json';
$vscodeExtension = $root . '/editors/vscode/doria/extension.js';
$intellijLexer = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt';
$intellijTokenTypes = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaTokenTypes.kt';
$intellijSyntaxHighlighter = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaSyntaxHighlighter.kt';
$intellijLspFiles = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/lsp/DoriaLspFiles.kt';
$fixture = $root . '/editors/fixtures/latest-tokens.doria';

$acceptedKeywords = [
    'class',
    'interface',
    'trait',
    'extends',
    'implements',
    'namespace',
    'use',
    'uses',
    'as',
    'include',
    'declare',
    'break',
    'continue',
    'when',
    'given',
    'finally',
];

$primitiveTypes = [
    'void',
    'int',
    'int8',
    'int16',
    'int32',
    'int64',
    'uint8',
    'uint16',
    'uint32',
    'uint64',
    'float',
    'float32',
    'float64',
    'string',
    'bool',
    'mixed',
];

$wordOperators = ['not', 'and', 'or', 'xor'];
$rejectedPreprocessor = [
    'include',
    'define',
    'undef',
    'if',
    'ifdef',
    'ifndef',
    'elif',
    'else',
    'endif',
    'warning',
    'error',
];
$strictComparison = ['===', '!=='];
$notKeywords = ['Option', 'Result'];

function fail_check(string $message): never
{
    fwrite(STDERR, "editor highlighting check failed: {$message}\n");
    exit(1);
}

function require_check(bool $condition, string $message): void
{
    if (!$condition) {
        fail_check($message);
    }
}

function any_match(iterable $items, callable $predicate): bool
{
    foreach ($items as $item) {
        if ($predicate($item)) {
            return true;
        }
    }

    return false;
}

function relative_path(string $path): string
{
    global $root;

    $prefix = $root . '/';
    if (str_starts_with($path, $prefix)) {
        return substr($path, strlen($prefix));
    }

    return $path;
}

function read_text(string $path): string
{
    $text = @file_get_contents($path);
    if ($text === false) {
        fail_check(relative_path($path) . ' could not be read');
    }

    return $text;
}

function load_json(string $path): mixed
{
    try {
        return json_decode(read_text($path), true, 512, JSON_THROW_ON_ERROR);
    } catch (JsonException $error) {
        fail_check(relative_path($path) . ' is not valid JSON: ' . $error->getMessage());
    }
}

/** @return Generator<array<string, mixed>> */
function walk_patterns(mixed $node): Generator
{
    if (!is_array($node)) {
        return;
    }

    if (array_key_exists('match', $node) || array_key_exists('begin', $node) || array_key_exists('name', $node)) {
        yield $node;
    }

    foreach ($node as $value) {
        yield from walk_patterns($value);
    }
}

function regex_matches(string $pattern, string $subject): bool
{
    $regex = '~' . str_replace('~', '\\~', $pattern) . '~';
    $result = @preg_match($regex, $subject);
    if ($result === false) {
        fail_check("TextMate regex {$pattern} could not be evaluated by PHP PCRE: " . preg_last_error_msg());
    }

    return $result === 1;
}

function check_vscode_package(): void
{
    global $vscodePackage;

    $package = load_json($vscodePackage);
    $grammars = $package['contributes']['grammars'] ?? [];
    require_check(
        any_match(
            $grammars,
            static fn (mixed $grammar): bool => is_array($grammar)
                && ($grammar['language'] ?? null) === 'doria'
                && ($grammar['scopeName'] ?? null) === 'source.doria'
                && ($grammar['path'] ?? null) === './syntaxes/doria.tmLanguage.json'
        ),
        'VS Code package.json must map doria/source.doria to ./syntaxes/doria.tmLanguage.json'
    );
}

function check_vscode_grammar(): void
{
    global $acceptedKeywords, $primitiveTypes, $wordOperators, $notKeywords, $strictComparison, $rejectedPreprocessor, $vscodeGrammar;

    $grammar = load_json($vscodeGrammar);
    $grammarText = json_encode($grammar, JSON_THROW_ON_ERROR);
    $patterns = iterator_to_array(walk_patterns($grammar), false);

    $tokens = array_unique([...$acceptedKeywords, ...$primitiveTypes, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($grammarText, $token), "VS Code grammar is missing '{$token}'");
    }

    sort($notKeywords);
    foreach ($notKeywords as $token) {
        require_check(!str_contains($grammarText, $token), "VS Code grammar must not treat '{$token}' as a keyword");
    }

    $normalOperatorMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'keyword.operator.doria') {
            $normalOperatorMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($normalOperatorMatches !== [], 'VS Code grammar must define normal operator highlighting');
    foreach ($strictComparison as $operator) {
        foreach ($normalOperatorMatches as $match) {
            require_check(
                !str_contains($match, $operator),
                "VS Code grammar must not highlight '{$operator}' as a normal operator"
            );
        }
    }

    $invalidOperatorPatterns = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'invalid.illegal.operator.strict-comparison.doria') {
            $invalidOperatorPatterns[] = (string) ($pattern['match'] ?? '');
        }
    }
    foreach ($strictComparison as $operator) {
        require_check(
            any_match($invalidOperatorPatterns, static fn (string $match): bool => str_contains($match, $operator)),
            "VS Code grammar must mark '{$operator}' invalid"
        );
    }

    $invalidPreprocessorPatterns = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'invalid.illegal.preprocessor.doria') {
            $invalidPreprocessorPatterns[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($invalidPreprocessorPatterns !== [], 'VS Code grammar must define invalid preprocessor highlighting');
    sort($rejectedPreprocessor);
    foreach ($rejectedPreprocessor as $directive) {
        require_check(
            any_match($invalidPreprocessorPatterns, static fn (string $match): bool => str_contains($match, $directive)),
            "VS Code grammar must mark #{$directive} invalid or unsupported"
        );
    }

    require_check(
        any_match($patterns, static fn (array $pattern): bool => ($pattern['name'] ?? null) === 'invalid.illegal.keyword.goto.doria'),
        'VS Code grammar must mark goto invalid or unsupported'
    );

    $importPatterns = array_values(array_filter(
        $patterns,
        static fn (array $pattern): bool => ($pattern['name'] ?? null) === 'meta.import.doria'
    ));
    $traitPatterns = array_values(array_filter(
        $patterns,
        static fn (array $pattern): bool => ($pattern['name'] ?? null) === 'meta.trait-composition.doria'
    ));
    require_check($importPatterns !== [], 'VS Code grammar must define a distinct import-use scope');
    require_check($traitPatterns !== [], 'VS Code grammar must define a distinct trait-composition scope');

    $importBegin = (string) ($importPatterns[0]['begin'] ?? '');
    $traitBegin = (string) ($traitPatterns[0]['begin'] ?? '');
    require_check(regex_matches($importBegin, 'use App\\Models\\User;'), 'VS Code import pattern must match namespace imports');
    require_check(!regex_matches($importBegin, '    uses HasSlug;'), 'VS Code import pattern must not match class-body trait composition');
    require_check(regex_matches($traitBegin, '    uses HasSlug;'), 'VS Code trait-composition pattern must match class-body uses');
    require_check(!regex_matches($traitBegin, 'use App\\Models\\User;'), 'VS Code trait-composition pattern must not match namespace imports');
    require_check(!regex_matches($traitBegin, '    use HasSlug;'), 'VS Code trait-composition pattern must not accept legacy class-body use');

    foreach ([
        'keyword.control.import.doria',
        'keyword.operator.alias.doria',
        'keyword.other.trait-uses.doria',
        'invalid.illegal.keyword.trait-use-old-spelling.doria',
        'entity.name.type.trait.doria',
    ] as $scope) {
        require_check(str_contains($grammarText, $scope), "VS Code grammar is missing '{$scope}'");
    }

    $attributePatterns = $grammar['repository']['attributes']['patterns'] ?? [];
    require_check($attributePatterns !== [], 'VS Code grammar must define attribute highlighting');
    $attributeIncludes = [];
    foreach ($attributePatterns as $attributePattern) {
        foreach (($attributePattern['patterns'] ?? []) as $pattern) {
            if (is_array($pattern) && array_key_exists('include', $pattern)) {
                $attributeIncludes[] = $pattern['include'];
            }
        }
    }
    $invalidIndex = array_search('#invalid', $attributeIncludes, true);
    $operatorsIndex = array_search('#operators', $attributeIncludes, true);
    require_check($invalidIndex !== false, 'VS Code attribute context must include invalid syntax patterns');
    require_check($operatorsIndex !== false, 'VS Code attribute context must include operator syntax patterns');
    require_check(
        $invalidIndex < $operatorsIndex,
        'VS Code attribute context must check invalid syntax before normal operators'
    );
}

function check_intellij_lexer(): void
{
    global $acceptedKeywords, $primitiveTypes, $wordOperators, $notKeywords, $strictComparison, $rejectedPreprocessor;
    global $intellijLexer, $intellijTokenTypes, $intellijSyntaxHighlighter;

    $lexerText = read_text($intellijLexer);
    $intellijHighlightingText = implode("\n", [
        $lexerText,
        read_text($intellijTokenTypes),
        read_text($intellijSyntaxHighlighter),
    ]);

    $tokens = array_unique([...$acceptedKeywords, ...$primitiveTypes, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($lexerText, '"' . $token . '"'), "IntelliJ lexer is missing '{$token}'");
    }

    sort($notKeywords);
    foreach ($notKeywords as $token) {
        require_check(!str_contains($lexerText, '"' . $token . '"'), "IntelliJ lexer must not treat '{$token}' as a keyword");
    }

    foreach ($strictComparison as $operator) {
        require_check(str_contains($lexerText, '"' . $operator . '"'), "IntelliJ lexer must recognize '{$operator}'");
    }
    require_check(
        str_contains($lexerText, 'STRICT_COMPARISON_OPERATORS') && str_contains($lexerText, 'DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must route strict comparison operators to invalid highlighting'
    );

    require_check(str_contains($lexerText, '"goto"') && str_contains($lexerText, 'INVALID_KEYWORDS'), 'IntelliJ lexer must mark goto invalid');
    sort($rejectedPreprocessor);
    foreach ($rejectedPreprocessor as $directive) {
        require_check(str_contains($lexerText, '"' . $directive . '"'), "IntelliJ lexer must recognize #{$directive} as unsupported");
    }
    require_check(
        str_contains($lexerText, 'firstNonWhitespace != tokenStart'),
        'IntelliJ preprocessor check must require # to be the first non-whitespace character on the line'
    );
    require_check(
        str_contains($lexerText, 'TRAIT_USES_LINE') && str_contains($lexerText, 'DoriaTokenTypes.TRAIT_USES_KEYWORD'),
        'IntelliJ lexer must recognize trait-composition uses'
    );
    require_check(
        str_contains($lexerText, 'LEGACY_TRAIT_USE_LINE') && str_contains($lexerText, 'isLegacyTraitUseLine() -> DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must mark legacy class-body trait use invalid'
    );

    foreach ([
        'DORIA_IMPORT_USE_KEYWORD',
        'DORIA_IMPORT_PATH',
        'DORIA_IMPORT_ALIAS_KEYWORD',
        'DORIA_IMPORT_ALIAS',
        'DORIA_TRAIT_USES_KEYWORD',
        'DORIA_TRAIT_NAME',
    ] as $tokenType) {
        require_check(str_contains($intellijHighlightingText, $tokenType), "IntelliJ highlighting is missing {$tokenType}");
    }
}

function check_editor_fixture_diagnostics_are_skipped(): void
{
    global $vscodeExtension, $intellijLspFiles;

    $vscodeText = read_text($vscodeExtension);
    $intellijText = read_text($intellijLspFiles);

    require_check(
        str_contains($vscodeText, '/editors/fixtures/') && str_contains($vscodeText, 'isDoriaSource'),
        'VS Code client must keep editor fixtures out of doria-lsp diagnostics'
    );
    require_check(
        str_contains($intellijText, '/editors/fixtures/') && str_contains($intellijText, 'isDoriaSourceFile'),
        'IntelliJ LSP adapter must keep editor fixtures out of doria-lsp diagnostics'
    );
}

function check_fixture(): void
{
    global $acceptedKeywords, $wordOperators, $fixture;

    $fixtureText = read_text($fixture);
    $tokens = array_unique([...$acceptedKeywords, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($fixtureText, $token), "shared editor fixture is missing '{$token}'");
    }

    $numericTokens = ['int8', 'int16', 'int32', 'int64', 'uint8', 'uint16', 'uint32', 'uint64', 'float32', 'float64'];
    sort($numericTokens);
    foreach ($numericTokens as $token) {
        require_check(str_contains($fixtureText, $token), "shared editor fixture is missing '{$token}'");
    }

    require_check(
        str_contains($fixtureText, 'use App\\Models\\Post;') && str_contains($fixtureText, 'uses HasSlug, TracksChanges;'),
        'shared editor fixture must include both import-use and trait-composition uses examples'
    );
}

function main(): int
{
    check_vscode_package();
    check_vscode_grammar();
    check_intellij_lexer();
    check_editor_fixture_diagnostics_are_skipped();
    check_fixture();
    echo "Doria editor highlighting checks passed.\n";
    return 0;
}

exit(main());