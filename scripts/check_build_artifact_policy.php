#!/usr/bin/env php
<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$failures = [];

$cargo = read('Cargo.toml');
$agents = read('AGENTS.md');
$contributing = read('CONTRIBUTING.md');
$validator = read('scripts/validate_work_unit.php');
$refresh = read('scripts/refresh_development_toolchain.php');
$runtimeReproducibility = read('crates/doriac/tests/runtime_reproducibility_tests.rs');
$ci = read('.github/workflows/ci.yml');

foreach ([
    '[profile.dev]' => $cargo,
    'debug = "line-tables-only"' => $cargo,
    '[profile.dev.package."*"]' => $cargo,
    '[profile.test.package."*"]' => $cargo,
    "php scripts/validate_work_unit.php\n" => $agents,
    'php scripts/validate_work_unit.php --llvm' => $agents,
    'php scripts/validate_work_unit.php --reclaim' => $contributing,
    "'llvm-backend'" => $validator,
    "'cargo', 'clean', '--target-dir'" => $validator,
    "'CARGO_TARGET_DIR'" => $validator,
    "getenv('DORIA_VALIDATION_TARGET_DIR')" => $refresh,
    "'toolchain-install'" => $refresh,
    'struct ScratchDirectory' => $runtimeReproducibility,
    'remove_dir_all(&self.path)' => $runtimeReproducibility,
    'php scripts/check_build_artifact_policy.php' => $ci,
] as $needle => $contents) {
    if (!str_contains($contents, $needle)) {
        $failures[] = "missing required artifact-policy marker: {$needle}";
    }
}

if (substr_count($cargo, 'debug = false') < 2) {
    $failures[] = 'Cargo.toml must disable dependency debug metadata in dev and test profiles';
}
if (str_contains($validator, "'rm'")) {
    $failures[] = 'the managed validator must reclaim through Cargo, not recursive deletion';
}
if (str_contains($agents, 'cargo build --workspace --all-targets --locked --verbose')) {
    $failures[] = 'AGENTS.md still requires the redundant all-target development build';
}

if ($failures !== []) {
    foreach ($failures as $failure) {
        fwrite(STDERR, "artifact-policy: {$failure}\n");
    }
    exit(1);
}

fwrite(STDOUT, "Build artifact policy guard passed.\n");

function read(string $path): string
{
    $contents = file_get_contents(dirname(__DIR__) . DIRECTORY_SEPARATOR . $path);
    if ($contents === false) {
        fwrite(STDERR, "artifact-policy: could not read {$path}\n");
        exit(1);
    }

    return $contents;
}
