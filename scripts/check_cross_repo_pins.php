<?php

declare(strict_types=1);

/**
 * Verify that every cross-repository revision pin points at a commit that is
 * reachable from a published ref of the repository it names.
 *
 * A pin is a promise that some other checkout can obtain exactly this revision.
 * Comparing a pin against a local sibling checkout cannot keep that promise: a
 * sibling may sit on unpushed work, so the pin passes locally while naming a
 * commit nobody else can fetch. That is not a hypothetical -- it is how a
 * completed slice was once reported missing, and how a pinned engine revision
 * became invisible to a normal fetch after existing only as an unreferenced
 * object on the remote.
 *
 * So reachability is checked against the remote, and existence alone is not
 * accepted: a commit can exist on a forge while being unreachable from every
 * branch, which makes it unfetchable in the ordinary way.
 *
 * The mechanism is shared rather than per-pin. A repository declares its pins in
 * cross-repo-pins.json, saying where each revision is written and how to read
 * it. The revision is never restated in the manifest, because a pin duplicated
 * in two files is two pins that will eventually disagree. Adding a new pin is a
 * declaration, not another bespoke checker.
 *
 * Modes:
 *   (default)         structural checks always; reachability attempted, and
 *                     reported as unverified rather than passed when the remote
 *                     cannot be reached. Ordinary local work with unpushed
 *                     siblings is not blocked.
 *   --require-remote  inability to verify reachability is a failure. Use in
 *                     credentialed coordinated CI and before publishing
 *                     anything that depends on a pin resolving elsewhere.
 *   --offline         skip remote work entirely and report every pin unverified.
 *
 * Targets:
 *   --manifest <path>   verify a specific manifest instead of this repository's
 *   --repo-root <path>  resolve declared source files against this root
 *   --siblings          additionally verify manifests found in sibling checkouts
 */

const CROSS_REPO_PIN_SCHEMA_VERSION = 1;
const CROSS_REPO_PIN_FORMATS = ['json-pointer', 'regex'];
const CROSS_REPO_ZERO_REVISION = '0000000000000000000000000000000000000000';

final class PinResult
{
    public function __construct(
        public string $manifest,
        public string $id,
        public string $repository,
        public string $revision,
        public string $status,
        public string $detail,
    ) {
    }
}

/**
 * Read the manifest and reject anything it does not model. Unknown fields are
 * refused so a typo cannot silently disable a pin.
 *
 * @return array{0:?array<string,mixed>,1:list<string>}
 */
function cross_repo_load_manifest(string $path): array
{
    $raw = @file_get_contents($path);
    if ($raw === false) {
        return [null, ["{$path}: unable to read pin manifest"]];
    }
    try {
        $manifest = json_decode($raw, true, 512, JSON_THROW_ON_ERROR);
    } catch (JsonException $error) {
        return [null, ["{$path}: invalid JSON: " . $error->getMessage()]];
    }
    if (!is_array($manifest) || array_keys($manifest) !== ['schemaVersion', 'repository', 'pins']) {
        return [null, ["{$path}: expected only schemaVersion, repository, and pins"]];
    }
    if ($manifest['schemaVersion'] !== CROSS_REPO_PIN_SCHEMA_VERSION) {
        return [null, ["{$path}: unsupported schemaVersion"]];
    }
    if (!is_string($manifest['repository']) || $manifest['repository'] === '') {
        return [null, ["{$path}: repository must be a non-empty string"]];
    }
    if (!is_array($manifest['pins']) || !array_is_list($manifest['pins'])) {
        return [null, ["{$path}: pins must be a list"]];
    }
    $failures = [];
    $seen = [];
    foreach ($manifest['pins'] as $index => $pin) {
        $context = "{$path}: pin {$index}";
        if (!is_array($pin)) {
            $failures[] = "{$context} must be an object";
            continue;
        }
        $allowed = ['id', 'repository', 'publishedRefs', 'source', 'purpose', 'placeholder'];
        foreach (array_keys($pin) as $field) {
            if (!in_array($field, $allowed, true)) {
                $failures[] = "{$context} contains unknown field {$field}";
            }
        }
        foreach (['id', 'repository', 'purpose'] as $field) {
            if (!isset($pin[$field]) || !is_string($pin[$field]) || $pin[$field] === '') {
                $failures[] = "{$context} requires a non-empty {$field}";
            }
        }
        if (isset($pin['id'])) {
            if (isset($seen[$pin['id']])) {
                $failures[] = "{$context} duplicates pin id {$pin['id']}";
            }
            $seen[$pin['id']] = true;
        }
        if (!isset($pin['publishedRefs']) || !is_array($pin['publishedRefs']) || $pin['publishedRefs'] === []) {
            $failures[] = "{$context} requires at least one published ref";
        } else {
            foreach ($pin['publishedRefs'] as $ref) {
                // A pin must name a ref that exists for everyone, so only fully
                // qualified branch and tag refs are accepted.
                if (!is_string($ref) || preg_match('#^refs/(heads|tags)/[A-Za-z0-9._/-]+$#', $ref) !== 1) {
                    $failures[] = "{$context} has a published ref that is not a full refs/heads or refs/tags path";
                }
            }
        }
        if (isset($pin['placeholder']) && !is_bool($pin['placeholder'])) {
            $failures[] = "{$context}.placeholder must be a boolean";
        }
        $failures = array_merge($failures, cross_repo_validate_source($pin['source'] ?? null, $context));
    }
    return [$failures === [] ? $manifest : null, $failures];
}

/** @param mixed $source @return list<string> */
function cross_repo_validate_source($source, string $context): array
{
    if (!is_array($source)) {
        return ["{$context} requires a source object"];
    }
    if (!isset($source['file']) || !is_string($source['file']) || $source['file'] === '') {
        return ["{$context}.source requires a file"];
    }
    if (str_contains($source['file'], '..')) {
        return ["{$context}.source.file must stay inside the repository"];
    }
    $format = $source['format'] ?? null;
    if (!is_string($format) || !in_array($format, CROSS_REPO_PIN_FORMATS, true)) {
        return ["{$context}.source.format must be one of " . implode(', ', CROSS_REPO_PIN_FORMATS)];
    }
    $expected = $format === 'json-pointer' ? ['file', 'format', 'pointer'] : ['file', 'format', 'pattern'];
    sort($expected);
    $actual = array_keys($source);
    sort($actual);
    if ($actual !== $expected) {
        return ["{$context}.source for {$format} expects exactly " . implode(', ', $expected)];
    }
    if ($format === 'json-pointer' && (!is_string($source['pointer']) || !str_starts_with($source['pointer'], '/'))) {
        return ["{$context}.source.pointer must be a JSON pointer beginning with /"];
    }
    if ($format === 'regex') {
        if (!is_string($source['pattern']) || $source['pattern'] === '') {
            return ["{$context}.source.pattern must be a non-empty expression"];
        }
        if (@preg_match($source['pattern'], '') === false) {
            return ["{$context}.source.pattern is not a valid expression"];
        }
    }
    return [];
}

/**
 * Read the revision from the file that actually declares it, so the manifest
 * never becomes a second copy of the pin.
 *
 * @param array<string,mixed> $source
 * @return array{0:?string,1:?string}
 */
function cross_repo_read_revision(string $repoRoot, array $source): array
{
    $path = $repoRoot . '/' . $source['file'];
    $raw = @file_get_contents($path);
    if ($raw === false) {
        return [null, "unable to read {$source['file']}"];
    }
    if ($source['format'] === 'json-pointer') {
        try {
            $document = json_decode($raw, true, 512, JSON_THROW_ON_ERROR);
        } catch (JsonException $error) {
            return [null, "{$source['file']} is not valid JSON"];
        }
        $node = $document;
        foreach (array_slice(explode('/', $source['pointer']), 1) as $segment) {
            $segment = str_replace(['~1', '~0'], ['/', '~'], $segment);
            if (!is_array($node) || !array_key_exists($segment, $node)) {
                return [null, "{$source['file']} has no value at {$source['pointer']}"];
            }
            $node = $node[$segment];
        }
        return is_string($node) ? [$node, null] : [null, "{$source['file']} value at {$source['pointer']} is not a string"];
    }
    if (preg_match($source['pattern'], $raw, $match) !== 1 || !isset($match[1])) {
        return [null, "{$source['file']} did not match the declared pattern"];
    }
    return [$match[1], null];
}

/**
 * Ask the remote whether the revision is reachable from one of the declared
 * published refs. Fetching the ref and testing ancestry is deliberate: it proves
 * the revision can actually be obtained, which mere existence on the forge does
 * not.
 *
 * @param list<string> $refs
 * @return array{0:string,1:string}
 */
function cross_repo_check_reachable(string $repoRoot, string $repository, array $refs, string $revision): array
{
    $url = "https://github.com/{$repository}.git";
    $specs = [];
    foreach ($refs as $index => $ref) {
        $specs[] = escapeshellarg($ref . ':refs/remotes/cross-repo-pin-probe/' . $index);
    }
    // Anonymous fetches cover public repositories. Private repositories use the
    // GitHub CLI credential helper only when gh has an authenticated account or
    // token; merely having the executable installed does not provide access.
    $helper = '';
    exec('command -v gh >/dev/null 2>&1 && gh auth status --hostname github.com >/dev/null 2>&1', $ignoredProbe, $ghStatus);
    if ($ghStatus === 0) {
        $helper = '-c credential.helper=' . escapeshellarg('!gh auth git-credential') . ' ';
    }
    $fetch = sprintf(
        'git -C %s %sfetch --quiet --force %s %s 2>&1',
        escapeshellarg($repoRoot),
        $helper,
        escapeshellarg($url),
        implode(' ', $specs),
    );
    exec($fetch, $output, $status);
    if ($status !== 0) {
        return ['unverified', 'could not reach ' . $repository . ': ' . trim(implode(' ', array_slice($output, -1)))];
    }
    foreach (array_keys($refs) as $index) {
        $probe = 'refs/remotes/cross-repo-pin-probe/' . $index;
        $command = sprintf(
            'git -C %s merge-base --is-ancestor %s %s 2>/dev/null',
            escapeshellarg($repoRoot),
            escapeshellarg($revision),
            escapeshellarg($probe),
        );
        exec($command, $ignored, $ancestor);
        if ($ancestor === 0) {
            return ['reachable', 'reachable from ' . $refs[$index]];
        }
    }
    return ['unreachable', 'not reachable from ' . implode(', ', $refs)];
}

/**
 * @param array<string,mixed> $manifest
 * @return array{0:list<PinResult>,1:list<string>}
 */
function cross_repo_verify(string $manifestPath, array $manifest, string $repoRoot, bool $offline): array
{
    $results = [];
    $failures = [];
    foreach ($manifest['pins'] as $pin) {
        [$revision, $error] = cross_repo_read_revision($repoRoot, $pin['source']);
        if ($revision === null) {
            $failures[] = "{$manifestPath}: pin {$pin['id']}: {$error}";
            continue;
        }
        if (preg_match('/^[0-9a-f]{40}$/', $revision) !== 1) {
            $failures[] = "{$manifestPath}: pin {$pin['id']}: revision is not a full lowercase commit id";
            continue;
        }
        $placeholder = ($pin['placeholder'] ?? false) === true;
        if ($placeholder !== ($revision === CROSS_REPO_ZERO_REVISION)) {
            $failures[] = $placeholder
                ? "{$manifestPath}: pin {$pin['id']}: declared a placeholder but names a real revision"
                : "{$manifestPath}: pin {$pin['id']}: names the zero revision without declaring placeholder";
            continue;
        }
        if ($placeholder) {
            $results[] = new PinResult($manifestPath, $pin['id'], $pin['repository'], $revision, 'placeholder', 'declared unresolved');
            continue;
        }
        if ($offline) {
            $results[] = new PinResult($manifestPath, $pin['id'], $pin['repository'], $revision, 'unverified', 'offline mode requested');
            continue;
        }
        [$status, $detail] = cross_repo_check_reachable($repoRoot, $pin['repository'], $pin['publishedRefs'], $revision);
        $results[] = new PinResult($manifestPath, $pin['id'], $pin['repository'], $revision, $status, $detail);
        if ($status === 'unreachable') {
            $failures[] = "{$manifestPath}: pin {$pin['id']}: {$revision} is {$detail} of {$pin['repository']}";
        }
    }
    return [$results, $failures];
}

/** @return list<array{0:string,1:string}> */
function cross_repo_discover(string $root, bool $siblings): array
{
    $targets = [[$root . '/cross-repo-pins.json', $root]];
    if (!$siblings) {
        return $targets;
    }
    foreach (glob(dirname($root) . '/*', GLOB_ONLYDIR) ?: [] as $directory) {
        if ($directory === $root || !is_file($directory . '/cross-repo-pins.json')) {
            continue;
        }
        $targets[] = [$directory . '/cross-repo-pins.json', $directory];
    }
    return $targets;
}

if (realpath($_SERVER['SCRIPT_FILENAME'] ?? '') === __FILE__) {
    $root = dirname(__DIR__);
    $offline = false;
    $requireRemote = false;
    $siblings = false;
    $manifestOverride = null;
    $rootOverride = null;
    $argv = $_SERVER['argv'] ?? [];
    for ($index = 1; $index < count($argv); $index++) {
        $argument = $argv[$index];
        if ($argument === '--offline') { $offline = true; continue; }
        if ($argument === '--require-remote') { $requireRemote = true; continue; }
        if ($argument === '--siblings') { $siblings = true; continue; }
        if ($argument === '--manifest') { $manifestOverride = $argv[++$index] ?? null; continue; }
        if ($argument === '--repo-root') { $rootOverride = $argv[++$index] ?? null; continue; }
        fwrite(STDERR, "unknown option: {$argument}\n");
        exit(2);
    }
    if ($offline && $requireRemote) {
        fwrite(STDERR, "--offline and --require-remote cannot be combined\n");
        exit(2);
    }

    $targets = $manifestOverride !== null
        ? [[$manifestOverride, $rootOverride ?? dirname($manifestOverride)]]
        : cross_repo_discover($root, $siblings);

    $allFailures = [];
    $allResults = [];
    foreach ($targets as [$manifestPath, $repoRoot]) {
        [$manifest, $failures] = cross_repo_load_manifest($manifestPath);
        if ($manifest === null) {
            $allFailures = array_merge($allFailures, $failures);
            continue;
        }
        [$results, $pinFailures] = cross_repo_verify($manifestPath, $manifest, $repoRoot, $offline);
        $allResults = array_merge($allResults, $results);
        $allFailures = array_merge($allFailures, $pinFailures);
    }

    foreach ($allResults as $result) {
        printf(
            "%-22s %-22s %s  %s\n",
            $result->id,
            $result->repository,
            substr($result->revision, 0, 12),
            strtoupper($result->status) . ': ' . $result->detail,
        );
    }

    $unverified = array_values(array_filter($allResults, static fn (PinResult $r): bool => $r->status === 'unverified'));
    if ($requireRemote && $unverified !== []) {
        foreach ($unverified as $result) {
            $allFailures[] = "{$result->manifest}: pin {$result->id}: reachability could not be verified and --require-remote was requested";
        }
    }

    if ($allFailures !== []) {
        fwrite(STDERR, "cross-repository pin check failed:\n- " . implode("\n- ", $allFailures) . "\n");
        exit(1);
    }
    if ($unverified !== []) {
        // Reported, not passed off as verified.
        echo "cross-repository pin check completed with " . count($unverified) . " unverified pin(s)\n";
        exit(0);
    }
    echo "cross-repository pin check passed\n";
}
