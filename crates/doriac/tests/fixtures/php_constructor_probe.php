<?php

declare(strict_types=1);

// Count instrumented objects and explicitly recorded address views weakly,
// counting each inherited object once. FFI calls are not instrumented; arrays,
// strings, closures and implicit constructors remain outside this probe.
final class __DoriaConstructorProbe
{
    private static ?WeakMap $instances = null;
    private static array $counts = [];

    public static function record(object $value): void
    {
        self::$instances ??= new WeakMap();
        if (!isset(self::$instances[$value])) {
            self::$instances[$value] = true;
            $type = $value::class;
            self::$counts[$type] = (self::$counts[$type] ?? 0) + 1;
        }
    }

    public static function reset(): void
    {
        self::$instances = null;
        self::$counts = [];
    }

    public static function count(string $type): int
    {
        return self::$counts[$type] ?? 0;
    }

    public static function live(): int
    {
        return self::$instances === null ? 0 : count(self::$instances);
    }
}

// Count lexical loop ranges; this does not establish runtime reachability.
function __doria_test_loop_tokens(string $source, string $function): array
{
    $tokens = PhpToken::tokenize($source, TOKEN_PARSE);
    $start = null;
    foreach ($tokens as $index => $token) {
        if (!$token->is(T_FUNCTION)) {
            continue;
        }
        do { ++$index; } while (isset($tokens[$index]) && $tokens[$index]->isIgnorable());
        if (($tokens[$index]->text ?? null) === $function) {
            $start = $index;
            break;
        }
    }
    if ($start === null) {
        throw new LogicException("generated function $function is missing");
    }
    $depth = 0;
    $counts = [];
    $found = false;
    for ($index = $start; isset($tokens[$index]); ++$index) {
        $token = $tokens[$index];
        if ($depth > 0 && $token->is([T_WHILE, T_FOR, T_FOREACH, T_DO])) {
            $end = __doria_test_loop_end($tokens, $index);
            for ($current = $index; $current <= $end; ++$current) {
                $id = $tokens[$current]->id;
                $counts[$id] = ($counts[$id] ?? 0) + 1;
            }
            // The complete outer range already includes its nested loops.
            $index = $end;
            $found = true;
            continue;
        }
        if ($token->is([ord('{'), T_CURLY_OPEN, T_DOLLAR_OPEN_CURLY_BRACES])) {
            ++$depth;
        } elseif ($token->is(ord('}')) && --$depth === 0) {
            break;
        }
    }
    if (!$found) {
        throw new LogicException("generated function $function has no structured loop");
    }
    return $counts;
}

function __doria_test_loop_end(array $tokens, int $loop): int
{
    $postcondition = $tokens[$loop]->is(T_DO);
    $index = $postcondition ? $loop + 1 : __doria_test_loop_condition_end($tokens, $loop + 1);
    while (isset($tokens[$index]) && $tokens[$index]->isIgnorable()) { ++$index; }
    if (!isset($tokens[$index]) || !$tokens[$index]->is(ord('{'))) {
        throw new LogicException('generated loop body is missing');
    }
    $depth = 0;
    for (; isset($tokens[$index]); ++$index) {
        if ($tokens[$index]->is([ord('{'), T_CURLY_OPEN, T_DOLLAR_OPEN_CURLY_BRACES])) {
            ++$depth;
        } elseif ($tokens[$index]->is(ord('}')) && --$depth === 0) {
            $end = $index;
            if ($postcondition) {
                do { ++$index; } while (isset($tokens[$index]) && $tokens[$index]->isIgnorable());
                if (!isset($tokens[$index]) || !$tokens[$index]->is(T_WHILE)) {
                    throw new LogicException('generated do-loop condition is missing');
                }
                $end = __doria_test_loop_condition_end($tokens, $index + 1) - 1;
            }
            return $end;
        }
    }
    throw new LogicException('generated loop body is unterminated');
}

function __doria_test_loop_condition_end(array $tokens, int $index): int
{
    while (isset($tokens[$index]) && $tokens[$index]->isIgnorable()) { ++$index; }
    if (!isset($tokens[$index]) || !$tokens[$index]->is(ord('('))) {
        throw new LogicException('generated loop condition is missing');
    }
    $depth = 0;
    for (; isset($tokens[$index]); ++$index) {
        if ($tokens[$index]->is(ord('('))) { ++$depth; }
        if ($tokens[$index]->is(ord(')')) && --$depth === 0) { return $index + 1; }
    }
    throw new LogicException('generated loop condition is unterminated');
}

function __doria_test_instrument_constructors(string $source): string
{
    $tokens = PhpToken::tokenize($source, TOKEN_PARSE);
    $classDepths = [];
    $classPending = false;
    $depth = 0;
    $previous = null;
    $insertions = [];
    foreach ($tokens as $index => $token) {
        if ($token->isIgnorable()) {
            continue;
        }
        if ($token->is([T_CLASS, T_INTERFACE, T_TRAIT]) && $previous !== T_DOUBLE_COLON) {
            $classPending = true;
        }
        if ($token->is([ord('{'), T_CURLY_OPEN, T_DOLLAR_OPEN_CURLY_BRACES])) {
            ++$depth;
            if ($classPending) {
                $classDepths[] = $depth;
                $classPending = false;
            }
        } elseif ($token->is(ord('}'))) {
            if (end($classDepths) === $depth) {
                array_pop($classDepths);
            }
            --$depth;
        } elseif ($token->is(T_FUNCTION) && end($classDepths) === $depth) {
            $cursor = $index + 1;
            while ($tokens[$cursor]->isIgnorable()) {
                ++$cursor;
            }
            if (!$tokens[$cursor]->is(T_STRING)
                || strcasecmp($tokens[$cursor]->text, '__construct') !== 0) {
                $previous = $token->id;
                continue;
            }
            while (!$tokens[$cursor]->is(ord('('))) {
                ++$cursor;
            }
            $parameters = 1;
            while ($parameters !== 0) {
                ++$cursor;
                $parameters += (int) $tokens[$cursor]->is(ord('('));
                $parameters -= (int) $tokens[$cursor]->is(ord(')'));
            }
            do {
                ++$cursor;
            } while ($tokens[$cursor]->isIgnorable());
            if ($tokens[$cursor]->is(ord('{'))) {
                $insertions[] = $tokens[$cursor]->pos + 1;
            }
        }
        $previous = $token->id;
    }
    $result = '';
    $offset = 0;
    foreach ($insertions as $insertion) {
        $result .= substr($source, $offset, $insertion - $offset)
            . '\\__DoriaConstructorProbe::record($this);';
        $offset = $insertion;
    }
    return $result . substr($source, $offset);
}
