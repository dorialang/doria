<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$cataloguePath = $root . '/crates/doria-diagnostic-catalogue/src/lib.rs';
$catalogue = file_get_contents($cataloguePath);
$failures = [];

if ($catalogue === false) {
    $failures[] = 'crates/doria-diagnostic-catalogue/src/lib.rs: could not read runtime catalogue';
    $catalogue = '';
}

preg_match_all(
    '/RuntimeCatalogueEntry\s*\{\s*code:\s*"(?<code>P[1-9][0-9]{3})",\s*title:\s*"(?<title>[^"]+)".*?process_status:\s*(?<status>[0-9]+),/s',
    $catalogue,
    $entries,
    PREG_SET_ORDER,
);

$codes = [];
$titles = [];
foreach ($entries as $entry) {
    $code = $entry['code'];
    $title = $entry['title'];
    if (isset($codes[$code])) {
        $failures[] = "runtime catalogue: duplicate code {$code}";
    }
    if (isset($titles[$title])) {
        $failures[] = "runtime catalogue: duplicate title `{$title}`";
    }
    if ($entry['status'] !== '101') {
        $failures[] = "runtime catalogue: {$code} does not use status 101";
    }
    $codes[$code] = $title;
    $titles[$title] = $code;
}

if (($codes['P1203'] ?? null) !== 'String Padding Text Cannot Be Empty') {
    $failures[] = 'runtime catalogue: P1203 must mean `String Padding Text Cannot Be Empty`';
}
if (($codes['P1000'] ?? null) !== 'Program Panicked') {
    $failures[] = 'runtime catalogue: P1000 must remain the generic user-authored panic identity';
}

$productionRoots = [
    $root . '/crates/doriac/src',
    $root . '/crates/doria-rt/src',
];
foreach ($productionRoots as $productionRoot) {
    $iterator = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($productionRoot));
    foreach ($iterator as $file) {
        if (!$file->isFile() || $file->getExtension() !== 'rs') {
            continue;
        }
        $contents = file_get_contents($file->getPathname());
        if ($contents === false) {
            continue;
        }
        preg_match_all('/"(?<code>P[1-9][0-9]{3})"/', $contents, $uses);
        foreach ($uses['code'] as $code) {
            if (!isset($codes[$code])) {
                $relative = ltrim(str_replace($root, '', $file->getPathname()), '/\\');
                $failures[] = "{$relative}: runtime code {$code} is missing from the shared catalogue";
            }
        }
    }
}

if ($failures !== []) {
    fwrite(STDERR, "panic catalogue check failed:\n- " . implode("\n- ", $failures) . "\n");
    exit(1);
}

fwrite(STDOUT, 'panic catalogue check passed (' . count($codes) . " runtime codes)\n");
