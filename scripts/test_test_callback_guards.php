<?php

declare(strict_types=1);

/**
 * Fixture cases for scripts/check_test_callback_guards.php.
 *
 * Usage: php scripts/test_test_callback_guards.php
 */

require __DIR__ . '/check_test_callback_guards.php';

$root = sys_get_temp_dir() . '/doria-test-callback-guards-' . getmypid();
$write = static function (string $path, string $text) use ($root): void {
    $file = $root . '/crates/doria-rt/src/' . $path;
    if (!is_dir(dirname($file))) {
        mkdir(dirname($file), 0777, true);
    }
    file_put_contents($file, $text);
};
$cases = [
    'an unguarded callback in a tests file fails' => [
        ['a/tests.rs' => "unsafe extern \"C\" fn bare(x: u8) -> u8 { assert_eq!(x, 1); x }\n"],
        ['crates/doria-rt/src/a/tests.rs:1: test callback `bare`'],
    ],
    'a guarded callback passes' => [
        ['a/pending_tests.rs' => "unsafe extern \"C\" fn good(x: u8) -> u8 {\n    crate::test_callback::guard(|| x)\n}\n"],
        [],
    ],
    'function-pointer types and imports are not callbacks' => [
        ['a/tests.rs' => "type Pointer = unsafe extern \"C\" fn(u8) -> u8;\nextern \"system\" { fn Imported(x: u32) -> u32; }\n"],
        [],
    ],
    'runtime entry points outside test code are not checked' => [
        ['lib.rs' => "pub unsafe extern \"C\" fn dr_v1_runtime(x: u8) -> u8 { x }\n"],
        [],
    ],
    'a cfg(test) module is test code, and a brace literal does not end it' => [
        ['lib.rs' => "#[cfg(test)]\nmod tests {\n    const BRACE: char = '{';\n    extern \"system\" fn thread_proc(_: *mut u8) -> u32 { 0 }\n}\n"],
        ['crates/doria-rt/src/lib.rs:4: test callback `thread_proc`'],
    ],
    'a macro-generated callback is checked' => [
        ['a/owned_tests.rs' => "macro_rules! cb { (\$name:ident) => { unsafe extern \"C\" fn \$name() -> u8 { 0 } }; }\n"],
        ['crates/doria-rt/src/a/owned_tests.rs:1: test callback `$name`'],
    ],
];
$failures = 0;
foreach ($cases as $name => [$files, $expected]) {
    exec('rm -rf ' . escapeshellarg($root));
    foreach ($files as $path => $text) {
        $write($path, $text);
    }
    $errors = check_test_callback_guards($root);
    $matched = count($errors) === count($expected);
    foreach ($expected as $index => $prefix) {
        $matched = $matched && str_starts_with($errors[$index] ?? '', $prefix);
    }
    if (!$matched) {
        $failures++;
        fwrite(STDERR, "FAIL {$name}:\n  " . implode("\n  ", $errors ?: ['(no errors)']) . "\n");
    }
}
exec('rm -rf ' . escapeshellarg($root));
if ($failures > 0) {
    exit(1);
}
echo 'Test callback guard tests passed: ' . count($cases) . " cases\n";
