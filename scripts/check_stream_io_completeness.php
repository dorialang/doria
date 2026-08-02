<?php

declare(strict_types=1);

/** @return list<string> */
function check_stream_io_completeness(string $root): array
{
    $failures = [];
    $manifestPath = $root . '/docs/notes/php-stream-capability-inventory.json';
    $auditPath = $root . '/docs/notes/io-surface-audit.md';
    $planPath = $root . '/docs/doria-end-to-end-plan.md';
    $pipelinePath = $root . '/docs/notes/current-pipeline.md';

    try {
        $manifest = json_decode(
            (string) file_get_contents($manifestPath),
            true,
            512,
            JSON_THROW_ON_ERROR,
        );
    } catch (Throwable $error) {
        return ["docs/notes/php-stream-capability-inventory.json: {$error->getMessage()}"];
    }

    $rows = $manifest['rows'] ?? null;
    if (!is_array($rows)) {
        return ['docs/notes/php-stream-capability-inventory.json: rows must be an array'];
    }

    $officialStreamFunctions = [
        'stream_bucket_append', 'stream_bucket_make_writeable', 'stream_bucket_new', 'stream_bucket_prepend',
        'stream_context_create', 'stream_context_get_default', 'stream_context_get_options', 'stream_context_get_params',
        'stream_context_set_default', 'stream_context_set_option', 'stream_context_set_options', 'stream_context_set_params',
        'stream_copy_to_stream', 'stream_filter_append', 'stream_filter_prepend', 'stream_filter_register',
        'stream_filter_remove', 'stream_get_contents', 'stream_get_filters', 'stream_get_line', 'stream_get_meta_data',
        'stream_get_transports', 'stream_get_wrappers', 'stream_is_local', 'stream_isatty',
        'stream_notification_callback', 'stream_register_wrapper', 'stream_resolve_include_path', 'stream_select',
        'stream_set_blocking', 'stream_set_chunk_size', 'stream_set_read_buffer', 'stream_set_timeout',
        'stream_set_write_buffer', 'stream_socket_accept', 'stream_socket_client', 'stream_socket_enable_crypto',
        'stream_socket_get_name', 'stream_socket_pair', 'stream_socket_recvfrom', 'stream_socket_sendto',
        'stream_socket_server', 'stream_socket_shutdown', 'stream_supports_lock', 'stream_wrapper_register',
        'stream_wrapper_restore', 'stream_wrapper_unregister',
    ];
    $officialWrapperMethods = array_map(
        static fn(string $name): string => "streamWrapper::{$name}",
        [
            '__construct', '__destruct', 'dir_closedir', 'dir_opendir', 'dir_readdir', 'dir_rewinddir', 'mkdir',
            'rename', 'rmdir', 'stream_cast', 'stream_close', 'stream_eof', 'stream_flush', 'stream_lock',
            'stream_metadata', 'stream_open', 'stream_read', 'stream_seek', 'stream_set_option', 'stream_stat',
            'stream_tell', 'stream_truncate', 'stream_write', 'unlink', 'url_stat',
        ],
    );
    $officialFilterMethods = [
        'php_user_filter::filter',
        'php_user_filter::onClose',
        'php_user_filter::onCreate',
    ];
    $officialFilesystemEntries = [
        'fclose', 'fdatasync', 'feof', 'fflush', 'fgetc', 'fgetcsv', 'fgets', 'fgetss', 'file',
        'file_get_contents', 'file_put_contents', 'flock', 'fopen', 'fpassthru', 'fputcsv', 'fputs', 'fread',
        'fscanf', 'fseek', 'fstat', 'fsync', 'ftell', 'ftruncate', 'fwrite', 'pclose', 'popen', 'readfile',
        'rewind', 'set_file_buffer', 'tmpfile',
    ];
    $officialProcessEntries = ['proc_close', 'proc_get_status', 'proc_open', 'proc_terminate'];
    $classifications = [
        'Existing Doria Intrinsic', 'Existing Doria Runtime Substrate', 'Proposed Doria Std Io',
        'Proposed Doria Std Fs', 'Proposed Doria Std Net', 'Proposed Doria Std Process',
        'Proposed Doria Std Term', 'Proposed Text Adapter', 'Proposed Encoding Adapter',
        'Proposed Compression Adapter', 'Proposed Async Integration', 'Derivable From Existing Surface',
        'Rejected PHP Alias', 'Rejected Dynamic Wrapper Mechanism', 'Rejected Global Configuration',
        'Rejected Resource-Oriented Shape', 'Deferred Post-v1', 'Not Applicable To Doria',
        'Unresolved Design Fork',
    ];
    $migrationActions = [
        'Direct Typed Rewrite', 'Rewrite With Semantic Warning', 'Rewrite Through Domain Module',
        'Rewrite Through Adapter', 'Derivable Composition', 'Requires Human Review',
        'No Doria Equivalent By Design', 'Deferred Until Named Stage',
    ];

    $indexes = [];
    $names = [];
    foreach ($rows as $index => $row) {
        if (!is_array($row)) {
            $failures[] = "manifest row {$index}: must be an object";
            continue;
        }
        $surface = $row['phpSurface'] ?? '';
        $kind = $row['phpKind'] ?? '';
        $name = $row['phpName'] ?? '';
        $key = "{$surface}|{$kind}|{$name}";
        $indexes[$key][] = $index;
        $names[$name][] = $index;

        if (!in_array($row['doriaClassification'] ?? null, $classifications, true)) {
            $failures[] = "manifest row {$index} ({$name}): missing or invalid Doria classification";
        }
        if (!in_array($row['migrationAction'] ?? null, $migrationActions, true)) {
            $failures[] = "manifest row {$index} ({$name}): missing or invalid migration action";
        }
        $status = $row['doriaStatus'] ?? '';
        if (in_array($status, ['v1-required', 'v1-recommended'], true) && empty($row['doriaOwner'])) {
            $failures[] = "manifest row {$index} ({$name}): required/recommended capability lacks an owner";
        }
        if ($status === 'deferred' && (empty($row['doriaOwner']) || empty($row['landingStage']) || empty($row['dependencies']))) {
            $failures[] = "manifest row {$index} ({$name}): deferred capability lacks owner, stage, or dependency";
        }
        if (str_starts_with((string) ($row['doriaClassification'] ?? ''), 'Proposed') && empty($row['byteTextSemantics'])) {
            $failures[] = "manifest row {$index} ({$name}): proposed capability lacks byte/text semantics";
        }
        $isRead = str_contains((string) ($row['phpCategory'] ?? ''), 'read')
            || str_contains((string) ($row['doriaCapability'] ?? ''), 'read');
        if ($isRead && ($row['wouldBlockRelevant'] ?? false) && !($row['eofRelevant'] ?? false)) {
            $failures[] = "manifest row {$index} ({$name}): non-blocking read does not distinguish EOF relevance";
        }
        $isWrite = str_contains((string) ($row['phpCategory'] ?? ''), 'write')
            || str_contains((string) ($row['doriaCapability'] ?? ''), 'write');
        if ($isWrite && !array_key_exists('partialProgress', $row)) {
            $failures[] = "manifest row {$index} ({$name}): write does not record partial-progress relevance";
        }
        $alias = $row['aliasOf'] ?? null;
        if ($alias !== null && !is_string($alias)) {
            $failures[] = "manifest row {$index} ({$name}): alias target must be a string";
        }
        if (in_array($row['doriaClassification'] ?? null, ['Rejected Dynamic Wrapper Mechanism', 'Rejected Global Configuration'], true)
            && empty($row['replacementCapabilities'])) {
            $failures[] = "manifest row {$index} ({$name}): rejected dynamic mechanism lacks replacement capabilities";
        }
    }

    foreach ($indexes as $key => $matchingIndexes) {
        if (count($matchingIndexes) !== 1) {
            $failures[] = "manifest duplicate row {$key}";
        }
    }

    $assertExact = static function (
        array $expected,
        string $surface,
        string $kind,
        string $label,
    ) use (&$failures, $indexes): void {
        foreach ($expected as $name) {
            $key = "{$surface}|{$kind}|{$name}";
            if (count($indexes[$key] ?? []) !== 1) {
                $failures[] = "manifest must contain {$label} {$name} exactly once";
            }
        }
        foreach ($indexes as $key => $_) {
            [$rowSurface, $rowKind, $rowName] = explode('|', $key, 3);
            if ($rowSurface === $surface && $rowKind === $kind && !in_array($rowName, $expected, true)) {
                $failures[] = "manifest has unexpected {$label} {$rowName}";
            }
        }
    };
    $assertExact($officialStreamFunctions, 'streams', 'function', 'PHP Stream Function');
    $assertExact($officialWrapperMethods, 'wrapper', 'method', 'streamWrapper method');
    $assertExact($officialFilterMethods, 'filter', 'method', 'php_user_filter method');
    $assertExact($officialFilesystemEntries, 'filesystem', 'function', 'filesystem stream entry');
    $assertExact($officialProcessEntries, 'process', 'function', 'process-pipe entry');

    if (count($indexes['filter|class|StreamBucket'] ?? []) !== 1) {
        $failures[] = 'manifest must classify StreamBucket exactly once';
    }
    foreach ($rows as $index => $row) {
        $alias = is_array($row) ? ($row['aliasOf'] ?? null) : null;
        if (is_string($alias) && count($names[$alias] ?? []) !== 1) {
            $failures[] = "manifest row {$index}: alias {$row['phpName']} must point to one canonical row {$alias}";
        }
    }

    $audit = $manifest['audit'] ?? [];
    foreach (['auditDate', 'phpManualVersionBanner', 'phpManualCopyrightYear', 'sourcePageTitles', 'sources', 'counts'] as $field) {
        if (empty($audit[$field])) {
            $failures[] = "manifest audit metadata lacks {$field}";
        }
    }
    if (($audit['counts']['totalRows'] ?? null) !== count($rows)) {
        $failures[] = 'manifest totalRows does not match the stored rows';
    }

    $auditText = (string) file_get_contents($auditPath);
    $plan = (string) file_get_contents($planPath);
    $pipeline = (string) file_get_contents($pipelinePath);
    foreach ([
        '# PHP Stream And I/O Completeness Audit',
        'supersedes the previous partial completeness scope',
        '## Required v1.0 recommendation matrix',
        '## PHP migration ledger',
        '## Designer review table',
    ] as $required) {
        if (!str_contains($auditText, $required)) {
            $failures[] = "docs/notes/io-surface-audit.md: missing {$required}";
        }
    }

    $stage36 = strpos($plan, '- **Stage 36 —');
    $stage36a = strpos($plan, '- **Stage 36a — Stream, readiness, and standard I/O foundation.**');
    $stage37 = strpos($plan, '- **Stage 37 —');
    if ($stage36 === false || $stage36a === false || $stage37 === false || !($stage36 < $stage36a && $stage36a < $stage37)) {
        $failures[] = 'docs/doria-end-to-end-plan.md: Stage 36a must appear between Stage 36 and Stage 37';
    }
    foreach ([
        'Stage 37 must consume Stage 36a readiness',
        'Stage 44 builds on Stage 36a duplex streams, readiness, timeouts, partial writes, and async integration',
        'Stage 46 reuses Stage 36a standard-stream access, readiness, blocking substrate, timeout integration, and platform-device abstraction',
    ] as $required) {
        if (!str_contains($plan, $required)) {
            $failures[] = "docs/doria-end-to-end-plan.md: missing {$required}";
        }
    }
    foreach ([
        'PHP Stream And I/O Completeness Audit — Implemented',
        'Andrew’s Stream API Completeness Review — Next',
        'Stage 26 — Blocked Pending Review',
        'Stage 36a — Scheduled',
        'Stage 36a Public Surface — Pending Review',
        'Stage 36a is scheduled, not implemented',
    ] as $required) {
        if (!str_contains($pipeline, $required)) {
            $failures[] = "docs/notes/current-pipeline.md: missing {$required}";
        }
    }
    foreach (['BlockingMode', 'ReadOutcome', 'Poller'] as $candidate) {
        if (str_contains($pipeline, "{$candidate} is implemented")) {
            $failures[] = "docs/notes/current-pipeline.md: falsely claims {$candidate} is implemented";
        }
    }

    return $failures;
}

if (realpath((string) ($_SERVER['SCRIPT_FILENAME'] ?? '')) === __FILE__) {
    $failures = check_stream_io_completeness(dirname(__DIR__));
    if ($failures !== []) {
        fwrite(STDERR, "stream I/O completeness check failed:\n");
        foreach ($failures as $failure) {
            fwrite(STDERR, "- {$failure}\n");
        }
        exit(1);
    }
    echo "stream I/O completeness check passed\n";
}
