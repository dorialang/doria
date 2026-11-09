<?php

declare(strict_types=1);

require __DIR__ . '/check_rust_toolchain_authority.php';

/**
 * Fixture tests for check_rust_toolchain_authority.php. Each case writes a
 * rust-toolchain.toml and one workflow into a temporary repository and expects
 * a set of error fragments, or none.
 */

$pinned = "[toolchain]\nchannel = \"1.98.1\"\ncomponents = [\"rustfmt\", \"clippy\"]\n";
$install = "      - run: rustup toolchain install --no-self-update\n";

$cases = [
    'compliant job and nightly-only fuzzing pass' => [
        $pinned,
        "jobs:\n  rust:\n    steps:\n{$install}      - run: cargo test --locked\n"
            . "  fuzz:\n    steps:\n{$install}      - uses: dtolnay/rust-toolchain@nightly\n"
            . "      - run: cargo +nightly fuzz run parser\n",
        [],
    ],
    'floating channel' => [
        "[toolchain]\nchannel = \"stable\"\n",
        "jobs:\n  rust:\n    steps:\n{$install}      - run: cargo test\n",
        ['channel "stable" floats'],
    ],
    'action installs stable' => [
        $pinned,
        "jobs:\n  rust:\n    steps:\n      - uses: dtolnay/rust-toolchain@stable\n      - run: cargo build\n",
        ['installs Rust `stable`', 'runs cargo without installing'],
    ],
    'versioned install and default' => [
        $pinned,
        "jobs:\n  rust:\n    steps:\n      - run: |\n          rustup toolchain install stable --profile minimal\n          rustup default stable\n      - run: cargo build\n",
        ['installs `stable`', '`rustup default`', 'runs cargo without installing'],
    ],
    'cargo override' => [
        $pinned,
        "jobs:\n  rust:\n    steps:\n{$install}      - run: cargo +1.99.0 build\n",
        ['`cargo +1.99.0`'],
    ],
    'toolchain input' => [
        $pinned,
        "jobs:\n  rust:\n    steps:\n      - uses: dtolnay/rust-toolchain@master\n        with:\n          toolchain: 1.99.1\n{$install}      - run: cargo build\n",
        ['installs Rust `master`', 'selects toolchain `1.99.1`'],
    ],
    'nightly-only job needs no pin' => [
        $pinned,
        "jobs:\n  fuzz:\n    steps:\n      - uses: dtolnay/rust-toolchain@nightly\n      - run: cargo +nightly fuzz run parser\n",
        [],
    ],
];

$failures = 0;
foreach ($cases as $name => [$toolchain, $workflow, $expected]) {
    $root = sys_get_temp_dir() . '/doria-toolchain-authority-' . getmypid() . '-' . md5($name);
    @mkdir($root . '/.github/workflows', 0777, true);
    file_put_contents($root . '/rust-toolchain.toml', $toolchain);
    file_put_contents($root . '/.github/workflows/ci.yml', $workflow);
    $errors = check_rust_toolchain_authority($root);
    unlink($root . '/.github/workflows/ci.yml');
    unlink($root . '/rust-toolchain.toml');
    rmdir($root . '/.github/workflows');
    rmdir($root . '/.github');
    rmdir($root);
    $text = implode("\n", $errors);
    $ok = $expected === [] ? $errors === [] : count($errors) === count($expected);
    foreach ($expected as $fragment) {
        $ok = $ok && str_contains($text, $fragment);
    }
    if (!$ok) {
        $failures++;
        fwrite(STDERR, "FAIL {$name}\n  expected: " . json_encode($expected) . "\n  got:\n    " . str_replace("\n", "\n    ", $text) . "\n");
    }
}

if ($failures > 0) {
    fwrite(STDERR, "{$failures} Rust toolchain authority case(s) failed\n");
    exit(1);
}
echo 'Rust toolchain authority tests passed: ' . count($cases) . " cases\n";
