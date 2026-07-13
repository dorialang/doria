#!/usr/bin/env php
<?php

declare(strict_types=1);

/**
 * Check that Doria editor highlighters cover the accepted token vocabulary.
 */

$root = dirname(__DIR__);

$vscodePackage = $root . '/editors/vscode/doria/package.json';
$vscodeGrammar = $root . '/editors/vscode/doria/syntaxes/doria.tmLanguage.json';
$vscodeLanguageConfiguration = $root . '/editors/vscode/doria/language-configuration.json';
$vscodeExtension = $root . '/editors/vscode/doria/extension.js';
$intellijLexer = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaLexer.kt';
$intellijBuildGradle = $root . '/editors/intellij/doria/build.gradle';
$intellijTokenTypes = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaTokenTypes.kt';
$intellijSyntaxHighlighter = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/highlighting/DoriaSyntaxHighlighter.kt';
$intellijLspFiles = $root . '/editors/intellij/doria/src/main/kotlin/dev/doria/intellij/lsp/DoriaLspFiles.kt';
$intellijPluginXml = $root . '/editors/intellij/doria/src/main/resources/META-INF/plugin.xml';
$intellijPluginIcon = $root . '/editors/intellij/doria/src/main/resources/META-INF/pluginIcon.svg';
$doriaLogo = $root . '/res/images/doria-app-icon-warm.svg';
$lspServer = $root . '/crates/doriac/src/lsp.rs';
$fixture = $root . '/editors/fixtures/latest-tokens.doria';
$rejectedFixture = $root . '/editors/fixtures/rejected-syntax.doria';

$acceptedKeywords = [
    'class',
    'interface',
    'trait',
    'extends',
    'implements',
    'function',
    'let',
    'writable',
    'readonly',
    'internal',
    'namespace',
    'use',
    'uses',
    'as',
    'include',
    'declare',
    'echo',
    'return',
    'if',
    'else',
    'while',
    'for',
    'foreach',
    'break',
    'continue',
    'true',
    'false',
    'null',
    'new',
    'throw',
    'throws',
    'try',
    'catch',
    'finally',
    'enum',
    'case',
    'match',
    'when',
    'given',
    'async',
    'await',
    'unsafe',
    'extern',
    'open',
    'override',
    'with',
    'take',
];

$plannedKeywords = [
    'enum',
    'case',
    'match',
    'async',
    'await',
    'unsafe',
    'extern',
    'open',
    'override',
    'with',
    'take',
    'throw',
    'throws',
    'try',
    'catch',
    'finally',
    'when',
    'given',
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
    'never',
];

$implementedIntegerTypes = [
    'int',
    'int8',
    'int16',
    'int32',
    'int64',
    'uint8',
    'uint16',
    'uint32',
    'uint64',
];

$implementedStage14ScalarTypes = [
    'float',
    'float32',
    'float64',
    'bool',
];

$reservedTypes = [
    'resource',
];

$plannedTypes = [
    'Shared',
    'Weak',
    'SharedMut',
    'Sendable',
    'Shareable',
    'Ptr',
    'MutPtr',
    'Bytes',
    'List',
    'Dictionary',
    'Set',
];

$rejectedTypes = [
    'array',
    'object',
];

$lspSupportedTypes = [
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
    'List',
    'Dictionary',
    'Set',
];
$wordOperators = ['not', 'and', 'or', 'xor'];
$stage13SymbolOperators = [
    '-',
    '~',
    '+',
    '*',
    '/',
    '%',
    '<<',
    '>>',
    '&',
    '|',
    '^',
    '==',
    '!=',
    '<',
    '<=',
    '>',
    '>=',
    '++',
    '--',
    '+=',
    '-=',
    '*=',
    '/=',
    '%=',
    '<<=',
    '>>=',
    '&=',
    '|=',
    '^=',
];
$booleanSymbolOperators = ['!', '&&', '||'];
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
$rejectedKeywords = ['goto', 'require', 'require_once', 'include_once', 'print'];
$strictComparison = ['===', '!=='];
$notKeywords = ['public', 'private', 'protected', 'Result', 'object'];
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

function regex_fully_matches(string $pattern, string $subject): bool
{
    return regex_matches('\\A(?:' . $pattern . ')\\z', $subject);
}

function check_vscode_package(): void
{
    global $vscodePackage;

    $package = load_json($vscodePackage);
    require_check(
        ($package['version'] ?? null) === '2026.3.1-canary' &&
            ($package['doriaToolchainVersion'] ?? null) === '2026.03.1-canary',
        'VS Code package must carry the pre-1.0 canary encoding of Doria CalVer 2026.03.1-canary'
    );
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

function check_vscode_language_configuration(): void
{
    global $vscodeLanguageConfiguration;

    $config = load_json($vscodeLanguageConfiguration);
    foreach (['brackets', 'autoClosingPairs', 'surroundingPairs'] as $key) {
        $json = json_encode($config[$key] ?? null, JSON_THROW_ON_ERROR);
        require_check(str_contains($json, '#[') && str_contains($json, ']'), 'VS Code language configuration must include #[...] behavior in ' . $key);
    }
}

function check_vscode_grammar(): void
{
    global $acceptedKeywords, $primitiveTypes, $reservedTypes, $plannedTypes, $wordOperators, $stage13SymbolOperators, $booleanSymbolOperators;
    global $notKeywords, $strictComparison, $rejectedPreprocessor, $rejectedKeywords, $rejectedTypes, $vscodeGrammar;

    $grammar = load_json($vscodeGrammar);
    $grammarText = json_encode($grammar, JSON_THROW_ON_ERROR);
    $patterns = iterator_to_array(walk_patterns($grammar), false);

    $interpolationPatterns = array_values(array_filter(
        $patterns,
        static fn (array $pattern): bool => ($pattern['name'] ?? null) === 'meta.interpolation.doria'
    ));
    require_check($interpolationPatterns !== [], 'VS Code grammar must define interpolation scopes');
    $interpolationJson = json_encode($interpolationPatterns, JSON_THROW_ON_ERROR);
    require_check(
        str_contains($interpolationJson, '$self') && !str_contains($interpolationJson, '(?=\\$)'),
        'VS Code interpolation must embed the ordinary Doria grammar rather than a variable-only grammar'
    );
    require_check(
        str_contains($grammarText, 'keyword.declaration.implements.doria'),
        'VS Code must scope active implements syntax as a declaration keyword'
    );

    $tokens = array_unique([...$acceptedKeywords, ...$primitiveTypes, ...$reservedTypes, ...$plannedTypes, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($grammarText, $token), "VS Code grammar is missing '{$token}'");
    }

    foreach ($rejectedTypes as $type) {
        require_check(!str_contains($grammarText, $type), "VS Code grammar must not highlight rejected type '{$type}'");
    }

    $primitiveTypeMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'storage.type.primitive.doria') {
            $primitiveTypeMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($primitiveTypeMatches !== [], 'VS Code grammar must define primitive type highlighting');
    foreach ($primitiveTypes as $type) {
        require_check(
            any_match($primitiveTypeMatches, static fn (string $match): bool => regex_fully_matches($match, $type)),
            "VS Code grammar must classify '{$type}' as a primitive type"
        );
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
    foreach (array_unique([...$stage13SymbolOperators, ...$booleanSymbolOperators]) as $operator) {
        require_check(
            any_match($normalOperatorMatches, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must highlight '{$operator}' as one complete operator token"
        );
    }

    $wordOperatorMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'keyword.operator.logical.word.doria') {
            $wordOperatorMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    foreach ($wordOperators as $operator) {
        require_check(
            any_match($wordOperatorMatches, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must classify '{$operator}' as a word operator"
        );
    }

    $logicalSymbolMatches = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'keyword.operator.logical.symbol.doria') {
            $logicalSymbolMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    foreach ($booleanSymbolOperators as $operator) {
        require_check(
            any_match($logicalSymbolMatches, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must classify '{$operator}' as a logical operator"
        );
    }
    require_check(
        !any_match($logicalSymbolMatches, static fn (string $match): bool => regex_matches($match, '!=')),
        "VS Code logical-operator patterns must not split the accepted '!=' operator"
    );

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
            any_match($invalidOperatorPatterns, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must mark '{$operator}' invalid"
        );
    }
    foreach (['&', '|'] as $operator) {
        require_check(
            !any_match($invalidOperatorPatterns, static fn (string $match): bool => regex_fully_matches($match, $operator)),
            "VS Code grammar must not mark single '{$operator}' invalid"
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

    $invalidKeywordPatterns = [];
    foreach ($patterns as $pattern) {
        if (($pattern['name'] ?? null) === 'invalid.illegal.keyword.rejected.doria') {
            $invalidKeywordPatterns[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($invalidKeywordPatterns !== [], 'VS Code grammar must define rejected keyword highlighting');
    foreach ($rejectedKeywords as $keyword) {
        require_check(
            any_match($invalidKeywordPatterns, static fn (string $match): bool => str_contains($match, $keyword)),
            'VS Code grammar must mark ' . $keyword . ' invalid or unsupported'
        );
    }
    require_check(
        any_match($invalidKeywordPatterns, static fn (string $match): bool => str_contains($match, 'use') && str_contains($match, '(?=')),
        'VS Code grammar must mark closure use capture invalid or unsupported'
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

    $callPatterns = $grammar['repository']['calls']['patterns'] ?? [];
    $functionCallMatches = [];
    foreach ($callPatterns as $pattern) {
        if (($pattern['name'] ?? null) === 'entity.name.function.call.doria') {
            $functionCallMatches[] = (string) ($pattern['match'] ?? '');
        }
    }
    require_check($functionCallMatches !== [], 'VS Code grammar must define contextual function-call highlighting');
    foreach (['read_file', 'calculateReport', 'saveReport', 'formatReport'] as $call) {
        require_check(
            any_match($functionCallMatches, static fn (string $match): bool => regex_matches($match, $call . ' (')),
            "VS Code grammar must highlight arbitrary call name '{$call}' before an opening parenthesis"
        );
    }
    require_check(
        !any_match($functionCallMatches, static fn (string $match): bool => regex_matches($match, 'calculateReport;')),
        'VS Code grammar must not classify a bare identifier as a function call'
    );

    $attributePatterns = $grammar['repository']['attributes']['patterns'] ?? [];
    require_check($attributePatterns !== [], 'VS Code grammar must define attribute highlighting');
    $attributeBegin = (string) ($attributePatterns[0]['begin'] ?? '');
    require_check(regex_matches($attributeBegin, '#[Module]'), 'VS Code attribute pattern must match simple attributes');
    require_check(
        regex_matches($attributeBegin, '#[App\\Routing\\Route(path: "/parser")]'),
        'VS Code attribute pattern must match namespaced attributes'
    );
    foreach ([
        'punctuation.definition.attribute.begin.doria',
        'punctuation.definition.attribute.end.doria',
        'entity.name.type.attribute.doria',
        'variable.parameter.attribute.doria',
    ] as $scope) {
        require_check(str_contains($grammarText, $scope), "VS Code grammar is missing '{$scope}'");
    }
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
    global $acceptedKeywords, $primitiveTypes, $reservedTypes, $plannedTypes, $wordOperators, $stage13SymbolOperators, $booleanSymbolOperators;
    global $notKeywords, $strictComparison, $rejectedPreprocessor, $rejectedKeywords, $rejectedTypes;
    global $intellijLexer, $intellijBuildGradle, $intellijTokenTypes, $intellijSyntaxHighlighter, $intellijPluginXml, $intellijPluginIcon, $doriaLogo;

    require_check(
        str_contains(read_text($intellijBuildGradle), "version = '2026.03.1-canary'"),
        'IntelliJ package must carry the pre-1.0 Doria CalVer canary suffix'
    );

    $lexerText = read_text($intellijLexer);
    $intellijHighlightingText = implode("\n", [
        $lexerText,
        read_text($intellijTokenTypes),
        read_text($intellijSyntaxHighlighter),
    ]);

    $tokens = array_unique([...$acceptedKeywords, ...$primitiveTypes, ...$reservedTypes, ...$plannedTypes, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($lexerText, '"' . $token . '"'), "IntelliJ lexer is missing '{$token}'");
    }

    require_check(
        preg_match('/private val PRIMITIVE_TYPES = setOf\\((.*?)\\n\\s*\\)/s', $lexerText, $primitiveMatches) === 1,
        'IntelliJ primitive type classification could not be found'
    );
    $primitiveTypeList = $primitiveMatches[1];
    foreach ($primitiveTypes as $type) {
        require_check(
            str_contains($primitiveTypeList, '"' . $type . '"'),
            "IntelliJ lexer must classify '{$type}' as a primitive type"
        );
    }

    foreach ($rejectedTypes as $type) {
        require_check(!str_contains($lexerText, chr(34) . $type . chr(34)), "IntelliJ lexer must not highlight rejected type '{$type}'");
    }

    sort($notKeywords);
    foreach ($notKeywords as $token) {
        require_check(!str_contains($lexerText, '"' . $token . '"'), "IntelliJ lexer must not treat '{$token}' as a keyword");
    }

    foreach ($strictComparison as $operator) {
        require_check(str_contains($lexerText, '"' . $operator . '"'), "IntelliJ lexer must recognize '{$operator}'");
    }
    require_check(
        preg_match('/private val STRICT_COMPARISON_OPERATORS = setOf\\((.*?)\\)/s', $lexerText, $strictOperatorMatches) === 1,
        'IntelliJ invalid strict-comparison classification could not be found'
    );
    $strictOperatorList = $strictOperatorMatches[1];
    foreach (['&', '|'] as $operator) {
        require_check(
            !str_contains($strictOperatorList, '"' . $operator . '"'),
            "IntelliJ lexer must not mark single '{$operator}' invalid"
        );
    }
    require_check(
        str_contains($lexerText, 'STRICT_COMPARISON_OPERATORS') && str_contains($lexerText, 'DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must route strict comparison operators to invalid highlighting'
    );

    require_check(
        preg_match('/private val THREE_CHAR_OPERATORS =\\s*(.*?)\\n\\s*private val TWO_CHAR_OPERATORS/s', $lexerText, $threeCharacterMatches) === 1,
        'IntelliJ three-character operator classification could not be found'
    );
    require_check(
        preg_match('/private val TWO_CHAR_OPERATORS = setOf\\((.*?)\\n\\s*\\)/s', $lexerText, $twoCharacterMatches) === 1,
        'IntelliJ two-character operator classification could not be found'
    );
    $threeCharacterOperatorList = $threeCharacterMatches[1];
    $twoCharacterOperatorList = $twoCharacterMatches[1];
    foreach (array_unique([...$stage13SymbolOperators, ...$booleanSymbolOperators]) as $operator) {
        $operatorLength = strlen($operator);
        if ($operatorLength === 1) {
            continue;
        }

        $operatorList = $operatorLength === 3 ? $threeCharacterOperatorList : $twoCharacterOperatorList;
        require_check(
            str_contains($operatorList, '"' . $operator . '"'),
            "IntelliJ lexer must tokenize '{$operator}' as one complete operator token"
        );
    }
    require_check(
        str_contains($lexerText, 'three in THREE_CHAR_OPERATORS -> three') &&
            str_contains($lexerText, 'two in TWO_CHAR_OPERATORS -> two'),
        'IntelliJ lexer must prefer three-character operators before two-character operators'
    );

    foreach ($rejectedKeywords as $keyword) {
        require_check(
            str_contains($lexerText, chr(34) . $keyword . chr(34)) && str_contains($lexerText, 'INVALID_KEYWORDS'),
            'IntelliJ lexer must mark ' . $keyword . ' invalid'
        );
    }
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
    require_check(
        str_contains($lexerText, 'LEGACY_CLOSURE_USE_LINE') && str_contains($lexerText, 'isLegacyClosureUseLine() -> DoriaTokenTypes.INVALID'),
        'IntelliJ lexer must mark legacy closure use capture invalid'
    );
    require_check(
        str_contains($lexerText, 'MODE_ATTRIBUTE') && str_contains($lexerText, 'scanAttributeToken'),
        'IntelliJ lexer must define an attribute scanning mode'
    );
    require_check(
        str_contains($lexerText, 'MODE_INTERPOLATION_DOUBLE_STRING') &&
            str_contains($lexerText, 'MODE_INTERPOLATION_SINGLE_STRING') &&
            str_contains($lexerText, 'scanCodeToken(doubleQuoteStartsInterpolatedString = false)'),
        'IntelliJ interpolation must reuse ordinary code scanning and preserve nested quoted strings'
    );
    require_check(
        str_contains($lexerText, 'DoriaTokenTypes.ATTRIBUTE_DELIMITER') &&
            str_contains($lexerText, 'DoriaTokenTypes.ATTRIBUTE_NAME') &&
            str_contains($lexerText, 'DoriaTokenTypes.ATTRIBUTE_ARGUMENT'),
        'IntelliJ lexer must emit dedicated attribute tokens'
    );
    require_check(
        str_contains($lexerText, "private fun isCallName(): Boolean = nextNonWhitespace(tokenEnd) == '('") &&
            str_contains($lexerText, 'isCallName() -> callableTokenType()') &&
            str_contains($lexerText, 'callableTokenType()') &&
            str_contains($lexerText, '"->" -> DoriaTokenTypes.METHOD_CALL') &&
            str_contains($lexerText, '"::" -> DoriaTokenTypes.STATIC_METHOD_CALL') &&
            str_contains($lexerText, 'else -> DoriaTokenTypes.FUNCTION_CALL'),
        'IntelliJ lexer must classify calls from parenthesis and accessor context rather than a name list'
    );
    require_check(
        str_contains($lexerText, 'isConstructorTypeName() -> DoriaTokenTypes.TYPE_NAME') &&
            str_contains($lexerText, "buffer[cursor] == '\\\\'") &&
            str_contains($lexerText, 'toString() == "new"') &&
            strpos($lexerText, 'isConstructorTypeName() -> DoriaTokenTypes.TYPE_NAME') <
                strpos($lexerText, 'isCallName() -> callableTokenType()') &&
            strpos($lexerText, 'isCallName() -> callableTokenType()') <
                strpos($lexerText, 'text.first().isUpperCase() -> DoriaTokenTypes.TYPE_NAME'),
        'IntelliJ lexer must preserve constructor type names while prioritizing other call syntax over capitalization'
    );
    foreach (['FUNCTION_CALL', 'METHOD_CALL', 'STATIC_METHOD_CALL'] as $callStyle) {
        require_check(
            preg_match(
                '/val ' . $callStyle . ': TextAttributesKey = TextAttributesKey\.createTextAttributesKey\(\s*"DORIA_' . $callStyle . '",\s*DefaultLanguageHighlighterColors\.FUNCTION_DECLARATION,/s',
                $intellijHighlightingText
            ) === 1,
            'IntelliJ ' . $callStyle . ' must inherit the visible function-declaration color instead of a theme-white call style'
        );
    }

    $pluginXml = read_text($intellijPluginXml);
    require_check(
        str_contains($pluginXml, 'language=' . chr(34) . 'doria' . chr(34)),
        'IntelliJ plugin must register the lowercase doria language id for Markdown fences'
    );
    require_check(is_file($intellijPluginIcon), 'IntelliJ plugin must package META-INF/pluginIcon.svg');
    require_check(
        rtrim(read_text($intellijPluginIcon)) === rtrim(read_text($doriaLogo)),
        'IntelliJ plugin icon must use the canonical Doria README SVG'
    );

    foreach ([
        'DORIA_IMPORT_USE_KEYWORD',
        'DORIA_IMPORT_PATH',
        'DORIA_IMPORT_ALIAS_KEYWORD',
        'DORIA_IMPORT_ALIAS',
        'DORIA_TRAIT_USES_KEYWORD',
        'DORIA_TRAIT_NAME',
        'DORIA_ATTRIBUTE_DELIMITER',
        'DORIA_ATTRIBUTE_NAME',
        'DORIA_ATTRIBUTE_ARGUMENT',
        'DORIA_LOGICAL_OPERATOR',
        'DORIA_PROPERTY',
        'DORIA_FUNCTION_CALL',
        'DORIA_METHOD_CALL',
        'DORIA_STATIC_METHOD_CALL',
    ] as $tokenType) {
        require_check(str_contains($intellijHighlightingText, $tokenType), "IntelliJ highlighting is missing {$tokenType}");
    }
}

function check_lsp_completion_vocabulary(): void
{
    global $acceptedKeywords, $plannedKeywords, $primitiveTypes, $reservedTypes, $plannedTypes, $wordOperators, $notKeywords, $lspServer, $lspSupportedTypes, $rejectedTypes;
    global $implementedIntegerTypes, $implementedStage14ScalarTypes;

    $lspText = read_text($lspServer);
    $tokens = array_unique([...$acceptedKeywords, ...$wordOperators]);
    sort($tokens);
    foreach ($tokens as $token) {
        require_check(str_contains($lspText, chr(34) . $token . chr(34)), 'LSP completion list is missing ' . $token);
    }

    foreach ($plannedKeywords as $keyword) {
        require_check(str_contains($lspText, chr(34) . $keyword . chr(34)), 'LSP completion list is missing planned keyword ' . $keyword);
    }
    require_check(str_contains($lspText, 'planned Doria keyword'), 'LSP planned keyword completions must be clearly marked as planned');
    require_check(
        str_contains($lspText, 'Accepted planned Doria syntax; compiler support lands in a later stage.'),
        'LSP planned keyword completions must explain that compiler support lands later'
    );

    require_check(
        preg_match('/let types = \[(.*?)\];/s', $lspText, $matches) === 1,
        'LSP completion type list could not be found'
    );
    $lspTypeList = $matches[1];

    require_check(
        preg_match('/let reserved_types = \[(.*?)\];/s', $lspText, $reservedMatches) === 1,
        'LSP completion reserved type list could not be found'
    );
    $lspReservedTypeList = $reservedMatches[1];

    sort($lspSupportedTypes);
    foreach ($lspSupportedTypes as $type) {
        require_check(str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list is missing ' . $type);
    }

    foreach ($implementedIntegerTypes as $type) {
        require_check(
            str_contains($lspTypeList, chr(34) . $type . chr(34)),
            'LSP must classify Stage 13 integer type ' . $type . ' as implemented'
        );
    }
    require_check(
        str_contains($lspText, '`int` is an exact alias for `int64`'),
        'LSP completion or hover text must document int as the exact int64 alias'
    );

    foreach ($implementedStage14ScalarTypes as $type) {
        require_check(
            str_contains($lspTypeList, chr(34) . $type . chr(34)),
            'LSP must classify Stage 14 scalar type ' . $type . ' as implemented'
        );
    }
    require_check(
        str_contains($lspText, 'exact alias of `float64`'),
        'LSP completion or hover text must document float as the exact float64 alias'
    );

    sort($reservedTypes);
    foreach ($reservedTypes as $type) {
        require_check(str_contains($lspReservedTypeList, chr(34) . $type . chr(34)), 'LSP completion reserved type list is missing ' . $type);
        require_check(!str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list must not advertise reserved type ' . $type);
    }

    foreach ($rejectedTypes as $type) {
        require_check(!str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list must not advertise rejected type ' . $type);
        require_check(!str_contains($lspReservedTypeList, chr(34) . $type . chr(34)), 'LSP completion reserved type list must not advertise rejected type ' . $type);
    }

    $unsupportedTypes = array_diff(array_unique([...$primitiveTypes, ...$plannedTypes]), $lspSupportedTypes);
    sort($unsupportedTypes);
    foreach ($unsupportedTypes as $type) {
        require_check(!str_contains($lspTypeList, chr(34) . $type . chr(34)), 'LSP completion type list must not advertise unsupported type ' . $type);
    }

    foreach (['Int', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64'] as $companion) {
        require_check(
            str_contains($lspText, chr(34) . $companion . '::from' . chr(34)),
            'LSP completion list is missing ' . $companion . '::from'
        );
    }
    require_check(
        str_contains($lspText, 'Doria integer conversion intrinsic') &&
            str_contains($lspText, 'Out-of-range conversion panics'),
        'LSP must provide completion and hover details for explicit integer conversions'
    );
    foreach (['Int::toFloat', 'Float::toInt'] as $intrinsic) {
        require_check(
            str_contains($lspText, chr(34) . $intrinsic . chr(34)),
            'LSP completion list is missing ' . $intrinsic
        );
    }
    require_check(
        str_contains($lspText, 'Doria scalar conversion intrinsic'),
        'LSP must provide completion and hover details for Stage 14 scalar conversions'
    );

    sort($notKeywords);
    foreach ($notKeywords as $token) {
        require_check(!str_contains($lspText, chr(34) . $token . chr(34)), 'LSP completion list must not advertise ' . $token);
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
    global $fixture, $rejectedFixture, $strictComparison, $rejectedKeywords;

    $fixtureText = read_text($fixture);
    $requiredSnippets = [
        'internal',
        'uses HasSlug, TracksChanges;',
        'with ($base)',
        'with (take $resource)',
        'open class Model',
        'override function save',
        'throws StorageError',
        'enum Option',
        'case Some',
        'match ($option)',
        'unsafe',
        'extern',
        '#[PhpExport]',
        '#[App\Routing\Route(path: "/parser", name: "parser.show")]',
        '0..<10',
        '0..10',
        'float32 $delta = 0.016;',
        'float $ratio = 1.0 / 3.0;',
        'Int::toFloat($whole)',
        'Float::toInt($wideAlias)',
        'let $both = $ready and not false;',
        'bool $symbolNegated = !$input;',
        'bool $symbolBoth = $ready && $symbolNegated;',
        'bool $symbolEither = $symbolBoth || $different;',
        'echo "Profile: {$this->profile->displayName}";',
        'echo "Count: {$count}";',
        'echo "sum: {left() + right()}";',
        'implements Displayable',
        'function toString(): string',
        "echo 'Literal {\$name}';",
        'read_file("input.txt")',
        'calculateReport($text)',
        '$repository->saveReport($customResult)',
        'ReportFormatter::formatReport($customResult)',
        'new App\Report()',
        '?User',
        '\n\t\r\s',
        'use App\Repositories\UserRepository;',
        'get_time()',
        'str_starts_with($name, "Dor")',
        'Int::wrappingAdd(1, 2)',
        '$name->isEmpty()',
        '$message->retryAfter(seconds: 30)',
        '$message->tenantId',
        '$repository->findById($id)',
    ];
    foreach ($requiredSnippets as $snippet) {
        require_check(str_contains($fixtureText, $snippet), 'shared editor fixture is missing ' . $snippet);
    }

    $stage13TypeSnippets = [
        'int8 $minimumInt8 = -128;',
        'int16 $maximumInt16 = 32767;',
        'int32 $maximumInt32 = 2147483647;',
        'int64 $maximumInt64 = 9223372036854775807;',
        'uint8 $maximumUInt8 = 255;',
        'uint16 $maximumUInt16 = 65535;',
        'uint32 $maximumUInt32 = 4294967295;',
        'uint64 $maximumUInt64 = 18446744073709551615;',
    ];
    foreach ($stage13TypeSnippets as $snippet) {
        require_check(str_contains($fixtureText, $snippet), 'shared editor fixture is missing Stage 13 type snippet ' . $snippet);
    }

    foreach (['Int', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64'] as $companion) {
        require_check(
            str_contains($fixtureText, $companion . '::from('),
            'shared editor fixture is missing Stage 13 conversion ' . $companion . '::from'
        );
    }

    $stage13OperatorSnippets = [
        'let $negated = -$defaultInt;',
        'let $complemented = ~$defaultInt;',
        'let $sum = $defaultInt + 2;',
        'let $difference = $defaultInt - 2;',
        'let $product = $defaultInt * 2;',
        'let $quotient = $defaultInt / 2;',
        'let $remainder = $defaultInt % 2;',
        'let $shiftedLeft = $defaultInt << 1;',
        'let $shiftedRight = $defaultInt >> 1;',
        'let $masked = $defaultInt & 255;',
        'let $combined = $defaultInt | 1;',
        'let $toggled = $defaultInt ^ 1;',
        'let $equal = $defaultInt == 42;',
        'let $notEqual = $defaultInt != 0;',
        'let $less = $defaultInt < 100;',
        'let $lessOrEqual = $defaultInt <= 42;',
        'let $greater = $defaultInt > 0;',
        'let $greaterOrEqual = $defaultInt >= 42;',
        'let $booleanXor = true xor false;',
        '$accumulator += 3;',
        '$accumulator -= 2;',
        '$accumulator *= 2;',
        '$accumulator /= 2;',
        '$accumulator %= 5;',
        '$accumulator <<= 1;',
        '$accumulator >>= 1;',
        '$accumulator &= 7;',
        '$accumulator |= 8;',
        '$accumulator ^= 3;',
        '$accumulator++;',
        '$accumulator--;',
    ];
    foreach ($stage13OperatorSnippets as $snippet) {
        require_check(str_contains($fixtureText, $snippet), 'shared editor fixture is missing Stage 13 operator snippet ' . $snippet);
    }

    require_check(is_file($rejectedFixture), 'negative editor fixture must exist');
    $rejectedText = read_text($rejectedFixture);
    foreach (['public', 'private', 'protected', 'use HasSlug;', 'use ($x)', '#define', '#include'] as $snippet) {
        require_check(str_contains($rejectedText, $snippet), 'negative editor fixture is missing ' . $snippet);
    }
    foreach ($strictComparison as $operator) {
        require_check(str_contains($rejectedText, $operator), 'negative editor fixture is missing ' . $operator);
    }
    foreach ($rejectedKeywords as $keyword) {
        require_check(str_contains($rejectedText, $keyword), 'negative editor fixture is missing ' . $keyword);
    }
}

function main(): int
{
    check_vscode_package();
    check_vscode_language_configuration();
    check_vscode_grammar();
    check_intellij_lexer();
    check_lsp_completion_vocabulary();
    check_editor_fixture_diagnostics_are_skipped();
    check_fixture();
    echo "Doria editor highlighting checks passed.\n";
    return 0;
}

exit(main());
