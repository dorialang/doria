<?php

declare(strict_types=1);

/**
 * Tests for the cross-repository pin verifier.
 *
 * Everything here runs offline against temporary fixtures. Reachability itself
 * needs a remote and is exercised by the coordinated CI job; what these tests
 * hold is the part that must never silently weaken: the manifest is strict, the
 * revision is read from the file that declares it rather than restated, and an
 * unverifiable pin is reported as unverified instead of passed.
 */

require_once __DIR__ . '/check_cross_repo_pins.php';

$tests = [];

$tests['a well-formed manifest loads'] = function (): void {
    [$directory] = make_pin_fixture();
    [$manifest, $failures] = cross_repo_load_manifest($directory . '/cross-repo-pins.json');
    assert_same([], $failures);
    assert_same(1, count($manifest['pins']));
};

$tests['unknown manifest fields are refused'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    $manifest['extra'] = true;
    write_pin_manifest($directory, $manifest);
    assert_contains(cross_repo_load_manifest($directory . '/cross-repo-pins.json')[1], 'expected only schemaVersion');
};

$tests['unknown pin fields are refused'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    $manifest['pins'][0]['sneaky'] = 'value';
    write_pin_manifest($directory, $manifest);
    assert_contains(cross_repo_load_manifest($directory . '/cross-repo-pins.json')[1], 'unknown field sneaky');
};

$tests['a pin may not restate the revision'] = function (): void {
    // The revision belongs to the declaring file. Allowing it here would create
    // a second copy that eventually disagrees with the first.
    [$directory, $manifest] = make_pin_fixture();
    $manifest['pins'][0]['revision'] = str_repeat('a', 40);
    write_pin_manifest($directory, $manifest);
    assert_contains(cross_repo_load_manifest($directory . '/cross-repo-pins.json')[1], 'unknown field revision');
};

$tests['duplicate pin ids are refused'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    $manifest['pins'][] = $manifest['pins'][0];
    write_pin_manifest($directory, $manifest);
    assert_contains(cross_repo_load_manifest($directory . '/cross-repo-pins.json')[1], 'duplicates pin id');
};

$tests['published refs must be fully qualified'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    $manifest['pins'][0]['publishedRefs'] = ['main'];
    write_pin_manifest($directory, $manifest);
    assert_contains(cross_repo_load_manifest($directory . '/cross-repo-pins.json')[1], 'not a full refs/heads or refs/tags path');
};

$tests['a source file outside the repository is refused'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    $manifest['pins'][0]['source']['file'] = '../elsewhere/pin.json';
    write_pin_manifest($directory, $manifest);
    assert_contains(cross_repo_load_manifest($directory . '/cross-repo-pins.json')[1], 'must stay inside the repository');
};

$tests['the revision is read from the declaring JSON file'] = function (): void {
    [$directory] = make_pin_fixture();
    [$revision, $error] = cross_repo_read_revision($directory, ['file' => 'pin.json', 'format' => 'json-pointer', 'pointer' => '/revision']);
    assert_same(null, $error);
    assert_same(str_repeat('b', 40), $revision);
};

$tests['the revision is read from a declaring file by pattern'] = function (): void {
    [$directory] = make_pin_fixture();
    file_put_contents($directory . '/Cargo.toml', "doriac = { git = \"x\", rev = \"" . str_repeat('c', 40) . "\" }\n");
    [$revision, $error] = cross_repo_read_revision($directory, ['file' => 'Cargo.toml', 'format' => 'regex', 'pattern' => '/rev\s*=\s*"([0-9a-f]{40})"/']);
    assert_same(null, $error);
    assert_same(str_repeat('c', 40), $revision);
};

$tests['a missing declared revision is a failure, not a pass'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    unlink($directory . '/pin.json');
    [, $failures] = cross_repo_verify('m', $manifest, $directory, true);
    assert_contains($failures, 'unable to read pin.json');
};

$tests['an abbreviated revision is refused'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    file_put_contents($directory . '/pin.json', json_encode(['revision' => 'abc1234'], JSON_THROW_ON_ERROR));
    [, $failures] = cross_repo_verify('m', $manifest, $directory, true);
    assert_contains($failures, 'not a full lowercase commit id');
};

$tests['the zero revision requires an explicit placeholder'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    file_put_contents($directory . '/pin.json', json_encode(['revision' => CROSS_REPO_ZERO_REVISION], JSON_THROW_ON_ERROR));
    [, $failures] = cross_repo_verify('m', $manifest, $directory, true);
    assert_contains($failures, 'without declaring placeholder');
};

$tests['a declared placeholder may not name a real revision'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    $manifest['pins'][0]['placeholder'] = true;
    [, $failures] = cross_repo_verify('m', $manifest, $directory, true);
    assert_contains($failures, 'declared a placeholder but names a real revision');
};

$tests['a declared placeholder is reported, not verified'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    $manifest['pins'][0]['placeholder'] = true;
    file_put_contents($directory . '/pin.json', json_encode(['revision' => CROSS_REPO_ZERO_REVISION], JSON_THROW_ON_ERROR));
    [$results, $failures] = cross_repo_verify('m', $manifest, $directory, true);
    assert_same([], $failures);
    assert_same('placeholder', $results[0]->status);
};

$tests['offline verification reports unverified rather than passed'] = function (): void {
    [$directory, $manifest] = make_pin_fixture();
    [$results, $failures] = cross_repo_verify('m', $manifest, $directory, true);
    assert_same([], $failures);
    assert_same('unverified', $results[0]->status);
    // The distinction that matters: an unverified pin is never a reachable one.
    if ($results[0]->status === 'reachable') {
        throw new RuntimeException('offline mode must not claim reachability');
    }
};

$failures = 0;
foreach ($tests as $name => $test) {
    try {
        $test();
        echo "ok - {$name}\n";
    } catch (Throwable $error) {
        $failures++;
        fwrite(STDERR, "not ok - {$name}: " . $error->getMessage() . "\n");
    }
}
printf("%d tests, %d failures\n", count($tests), $failures);
exit($failures === 0 ? 0 : 1);

function assert_same(mixed $expected, mixed $actual): void
{
    if ($expected !== $actual) {
        throw new RuntimeException('expected ' . var_export($expected, true) . ', got ' . var_export($actual, true));
    }
}

/** @param list<string> $failures */
function assert_contains(array $failures, string $fragment): void
{
    foreach ($failures as $failure) {
        if (str_contains($failure, $fragment)) {
            return;
        }
    }
    throw new RuntimeException("expected a failure containing '{$fragment}', got: " . implode(' | ', $failures ?: ['(none)']));
}

/** @return array{0:string,1:array<string,mixed>} */
function make_pin_fixture(): array
{
    $directory = sys_get_temp_dir() . '/doria-pin-test-' . bin2hex(random_bytes(6));
    if (!mkdir($directory)) {
        throw new RuntimeException('could not create fixture directory');
    }
    file_put_contents($directory . '/pin.json', json_encode(['revision' => str_repeat('b', 40)], JSON_THROW_ON_ERROR));
    $manifest = [
        'schemaVersion' => 1,
        'repository' => 'dorialang/example',
        'pins' => [[
            'id' => 'example',
            'repository' => 'dorialang/other',
            'publishedRefs' => ['refs/heads/main'],
            'source' => ['file' => 'pin.json', 'format' => 'json-pointer', 'pointer' => '/revision'],
            'purpose' => 'fixture',
        ]],
    ];
    write_pin_manifest($directory, $manifest);
    return [$directory, $manifest];
}

/** @param array<string,mixed> $manifest */
function write_pin_manifest(string $directory, array $manifest): void
{
    file_put_contents($directory . '/cross-repo-pins.json', json_encode($manifest, JSON_THROW_ON_ERROR | JSON_PRETTY_PRINT));
}
