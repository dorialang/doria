<?php

declare(strict_types=1);

/** @return list<string> */
function check_stream_io_completeness(string $root): array
{
    $failures = [];
    $manifestPath = $root . '/docs/notes/php-stream-capability-inventory.json';
    $agentsPath = $root . '/AGENTS.md';
    $readmePath = $root . '/README.md';
    $auditPath = $root . '/docs/notes/io-surface-audit.md';
    $planPath = $root . '/docs/doria-end-to-end-plan.md';
    $pipelinePath = $root . '/docs/notes/current-pipeline.md';
    $decisionPath = $root . '/docs/decisions/0110-stream-readiness-standard-io-blocking-and-performance-model.md';
    $specPath = $root . '/SPEC.md';
    $stdlibPath = $root . '/docs/stdlib-reference.md';
    $apiGuidelinesPath = $root . '/docs/api-design-guidelines.md';
    $websiteGuidelinesPath = $root . '/docs/website-content-guidelines.md';
    $performancePath = $root . '/docs/performance-and-benchmarking.md';

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
    if (count($rows) !== 153) {
        $failures[] = 'manifest must preserve the audited 153-row inventory';
    }

    $review = $manifest['review'] ?? [];
    $expectedReview = [
        'status' => 'approved',
        'reviewDate' => '2026-08-02',
        'decisionRecord' => '0110',
        'semanticDirection' => 'accepted',
        'performanceContract' => 'accepted',
        'publicSpellingStatus' => 'deferred',
        'stage26Blocked' => false,
        'stage26Status' => 'next',
        'stage36aStatus' => 'scheduled',
        'stage36aImplemented' => false,
    ];
    foreach ($expectedReview as $field => $expected) {
        if (($review[$field] ?? null) !== $expected) {
            $failures[] = "manifest review metadata {$field} must be " . json_encode($expected);
        }
    }

    $performance = $manifest['performance'] ?? [];
    if (($performance['status'] ?? null) !== 'accepted'
        || ($performance['decisionRecord'] ?? null) !== '0110'
        || ($performance['initialGateOwner'] ?? null) !== 'Stage 36a'
        || ($performance['continuationOwner'] ?? null) !== 'Stage 43') {
        $failures[] = 'manifest performance metadata must bind decision 0110, Stage 36a, and Stage 43';
    }
    $profileCapabilities = [];
    foreach (($performance['profiles'] ?? []) as $profileIndex => $profile) {
        if (!is_array($profile) || empty($profile['name']) || empty($profile['capabilities'])) {
            $failures[] = "manifest performance profile {$profileIndex} lacks a name or capabilities";
            continue;
        }
        foreach (['allocationSensitivity', 'copySensitivity', 'partialProgress', 'reusableBufferRequirement', 'readinessReuse', 'backpressureRequirement', 'asyncCostIsolation'] as $field) {
            if (empty($profile[$field])) {
                $failures[] = "manifest performance profile {$profile['name']} lacks {$field}";
            }
        }
        foreach ($profile['capabilities'] as $capability) {
            $profileCapabilities[$capability] = true;
        }
    }
    $requiredPerformanceCapabilities = [
        'stream-operation', 'typed-operation-progress', 'bounded-streaming-copy',
        'typed-adapter-buffer-flow', 'portable-readiness-wait',
        'owned-child-process-and-pipes',
    ];
    foreach ($requiredPerformanceCapabilities as $capability) {
        if (!isset($profileCapabilities[$capability])) {
            $failures[] = "manifest performance profiles do not cover {$capability}";
        }
        if (!array_filter($rows, static fn(mixed $row): bool => is_array($row) && ($row['doriaCapability'] ?? null) === $capability)) {
            $failures[] = "manifest performance profile names unknown capability {$capability}";
        }
    }

    $resolvedReviewNames = [
        'stream_context_create',
        'stream_context_get_options',
        'stream_context_get_params',
        'stream_context_set_option',
        'stream_context_set_options',
        'stream_context_set_params',
        'stream_notification_callback',
    ];
    foreach ($rows as $index => $row) {
        if (($row['doriaStatus'] ?? null) === 'unresolved'
            || ($row['doriaClassification'] ?? null) === 'Unresolved Design Fork') {
            $failures[] = "manifest row {$index}: semantic review must not remain unresolved";
        }
        if (in_array($row['phpName'] ?? null, $resolvedReviewNames, true)) {
            if (($row['semanticStatus'] ?? null) !== 'accepted'
                || ($row['publicSpellingStatus'] ?? null) !== 'deferred'
                || ($row['decisionRecord'] ?? null) !== '0110'
                || !array_key_exists('doriaCandidateSpelling', $row)
                || $row['doriaCandidateSpelling'] !== null
                || ($row['designerDecisionRequired'] ?? true) !== false) {
                $failures[] = "manifest row {$index}: approved semantics and deferred spelling metadata are incomplete";
            }
        }
    }

    $auditText = (string) file_get_contents($auditPath);
    $agents = (string) file_get_contents($agentsPath);
    $readme = (string) file_get_contents($readmePath);
    $plan = (string) file_get_contents($planPath);
    $pipeline = (string) file_get_contents($pipelinePath);
    $decision = (string) file_get_contents($decisionPath);
    $spec = (string) file_get_contents($specPath);
    $stdlib = (string) file_get_contents($stdlibPath);
    $apiGuidelines = (string) file_get_contents($apiGuidelinesPath);
    $websiteGuidelines = (string) file_get_contents($websiteGuidelinesPath);
    $performanceAuthority = (string) file_get_contents($performancePath);
    foreach ([
        '# PHP Stream And I/O Completeness Audit',
        'supersedes the previous partial completeness scope',
        '## Required v1.0 recommendation matrix',
        '## PHP migration ledger',
        '## Designer review table',
        'Andrew approved these recommendations on 2026-08-02',
        'Performance constraints',
        'Semantic status',
        'Public spelling status',
        'Stage 26 is unblocked and next',
        'Stage 36a is scheduled, not implemented',
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
        'Stage 44 builds on Stage 36a duplex streams, readiness, timeouts, partial writes, cancellation, backpressure, reusable buffers/byte regions, readiness reuse, and async-cost isolation',
        'Stage 46 reuses Stage 36a standard-stream views, readiness, blocking substrate, timeout/deadline integration, cancellation, and platform-device abstraction',
        'Decision 0110: stream, readiness, standard I/O, blocking-mode, and performance model',
        'Stage 26 — Remaining collection family — Complete.',
        'Stage 26b — Performance Baseline Foundation — In Progress; Slices 1 And 2 Complete, Slice 3 In Progress',
        'Stage 27 — Enums + payload cases — Blocked Until Stage 26b Completes',
        'Stage 36a Public Spellings — Deferred',
        'Stage 36a — Not Implemented',
    ] as $required) {
        if (!str_contains($plan, $required)) {
            $failures[] = "docs/doria-end-to-end-plan.md: missing {$required}";
        }
    }
    foreach ([
        'PHP Stream And I/O Completeness Audit — Implemented',
        'Andrew’s Stream API Completeness Review — Complete',
        'Stream, Readiness, Standard I/O, Blocking Mode, And Performance Model — Accepted (decision 0110)',
        'Stage 26 — Complete',
        'Stage 26b — Performance Baseline Foundation — In Progress',
        'Stage 27 — Blocked Until Stage 26b Completes',
        'Stage 36a — Scheduled',
        'Stage 36a Public Spellings — Deferred',
        'Stage 36a — Not Implemented',
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

    $decisionHeadings = [
        '# Decision 0110: Stream, Readiness, Standard I/O, Blocking Mode, And Performance Model',
        '- **Status:** Accepted',
        '## Context', '## Decision', '## Core Stream Architecture',
        '## Ownership And Lifetime', '## First-Class Standard Streams',
        '## Blocking Modes', '## Read Results', '## Write Results',
        '## Readiness', '## Timeouts, Deadlines, And Cancellation',
        '## Buffering And Text', '## Files', '## Child Processes And Pipes',
        '## Typed Adapters And Domain Ownership', '## Cross-Domain I/O Unification',
        '## Performance And Memory Contract',
        '## Reusable Buffer And Byte-Region Requirement',
        '## Dispatch And Specialization', '## Readiness Efficiency',
        '## Async Cost Isolation', '## Benchmark And Regression Requirements',
        '## Stage Boundaries', '## Deferred Public Spellings',
        '## Reopening Rules', '## Consequences', '## Invalidated Elsewhere',
    ];
    foreach ($decisionHeadings as $heading) {
        if (!str_contains($decision, $heading)) {
            $failures[] = "decision 0110: missing {$heading}";
        }
    }
    $decisionSemantics = [
        'There is no universal stream god object',
        'Ordinary stream handles are owned move values',
        'Explicit close or finish consumes the owned handle',
        'first-class, non-owning, nonclosable views',
        'Blocking mode is a named typed state',
        'data, would-block, end-of-stream, and timed-out outcomes',
        'partial progress is observable',
        'One multi-stream readiness core',
        'Durations and absolute deadlines are both semantic concepts',
        'Buffering is typed and per value',
        'File opening uses typed request/options values',
        'Advisory locking is v1 functionality represented by an ownership guard',
        'Active ownership must be resolved explicitly by waiting, detaching, or terminating',
        'Typed composition replaces PHP wrappers, filters, contexts, registries, string options, and mixed bags',
        'Sync and async I/O, networking, processes, and terminals share one ownership, read, write, readiness, time, cancellation, and backpressure model',
        'Stage 37 consumes it in the async design',
        'Stage 44 owns network-specific',
        'Stage 46 owns terminal raw mode',
    ];
    foreach ($decisionSemantics as $semantic) {
        if (!str_contains($decision, $semantic)) {
            $failures[] = "decision 0110: missing accepted semantic rule {$semantic}";
        }
    }
    $decisionPerformance = [
        'without allocating on every iteration',
        'must not require allocation by design',
        'Common data, would-block, EOF, timeout, partial-progress, readable, writable, and closure outcomes use compact inline representations',
        'must support reusable caller-owned or adapter-owned byte storage',
        'must not construct a second buffer merely to return progress',
        'rather than allocating and copying an unwritten suffix',
        'retain monomorphization, inlining, devirtualization, and static-dispatch opportunities',
        'Dynamic dispatch remains valid for genuine runtime heterogeneity or deliberate type erasure',
        'Stable watched sets reuse registration storage, event storage, platform handles, and watcher identity',
        'one thread per watched stream are rejected as the ordinary model',
        'Backpressure is bounded',
        'does not initialize an executor, worker pool, timer wheel, async task allocator, readiness thread, or async cancellation registry',
        'UTF-8 adapters validate incrementally',
        'Stage 36a owns its first performance acceptance gate',
        'Stage 43 incorporates and extends this suite',
    ];
    foreach ($decisionPerformance as $constraint) {
        if (!str_contains($decision, $constraint)) {
            $failures[] = "decision 0110: missing accepted performance constraint {$constraint}";
        }
    }
    $deferralRules = [
        'before Stage 36a implementation begins',
        'Reopen in `Doria\\Std\\Fs` design',
        'no later than Stage 36a surface finalization',
        'after the Stage 36a foundation and before their v1 implementation',
        'Reopen in the Stage 44 `Doria\\Std\\Net` design',
        'first operation or adapter that requires it',
    ];
    foreach ($deferralRules as $rule) {
        if (!str_contains($decision, $rule)) {
            $failures[] = "decision 0110: safe deferral lacks owner or reopen trigger {$rule}";
        }
    }
    foreach ([
        'Stage 36a is scheduled and not implemented',
        'Exact public interface, member',
        'remain deferred under decision 0110',
    ] as $required) {
        if (!str_contains($spec, $required)) {
            $failures[] = "SPEC.md: missing accepted stream authority {$required}";
        }
    }
    foreach ([
        'small readable/writable/duplex/seekable/flushable/blocking/readiness capabilities',
        'Stage 36a scheduled and not implemented; semantics and performance contract accepted, public spellings deferred',
    ] as $required) {
        if (!str_contains($stdlib, $required)) {
            $failures[] = "docs/stdlib-reference.md: missing {$required}";
        }
    }
    foreach ([
        'Prefer small capability interfaces over universal god objects',
        'Use typed outcomes for ordinary state',
        'Keep resource use bounded by default',
        'Make steady-state reuse possible',
        'Keep asynchronous machinery isolated',
    ] as $required) {
        if (!str_contains($apiGuidelines, $required)) {
            $failures[] = "docs/api-design-guidelines.md: missing {$required}";
        }
    }
    foreach ([
        'The public website represents the completed language and toolchain target',
        'Never downgrade target-state documentation or playground examples',
        'Do not invent exact API spellings',
        'A current compiler failure against a valid target-state example is UAT evidence',
        'Performance copy must reflect an accepted target-state contract',
    ] as $required) {
        if (!str_contains($websiteGuidelines, $required)) {
            $failures[] = "docs/website-content-guidelines.md: missing {$required}";
        }
    }
    foreach ([
        '`doria-website` is the completed-language BDD/UAT authority',
        'Never downgrade that target-state',
        'website or its examples to match current implementation lag',
    ] as $required) {
        if (!str_contains($agents, $required)) {
            $failures[] = "AGENTS.md: missing website target-state boundary {$required}";
        }
    }

    foreach ([
        '## 10. Stage 36a stream performance gate',
        '- compile time',
        'large streaming file copy',
        'non-blocking pipe transfer',
        'synchronous startup proving zero executor/task/scheduler initialization',
        '- generic specialization count',
        '- runtime library growth',
        '- development binary size',
        '- release binary size',
        '- stripped binary size',
        'equivalent direct OS, C, or Rust baseline',
        'Ordinary CI enforces deterministic structural invariants',
        'Timing thresholds run on curated, controlled runners',
    ] as $required) {
        if (!str_contains($performanceAuthority, $required)) {
            $failures[] = "docs/performance-and-benchmarking.md: missing {$required}";
        }
    }

    $activeAuthorities = [
        'AGENTS.md' => $agents,
        'README.md' => $readme,
        'SPEC.md' => $spec,
        'docs/doria-end-to-end-plan.md' => $plan,
        'docs/notes/current-pipeline.md' => $pipeline,
        'docs/decisions/0110-stream-readiness-standard-io-blocking-and-performance-model.md' => $decision,
        'docs/stdlib-reference.md' => $stdlib,
        'docs/api-design-guidelines.md' => $apiGuidelines,
        'docs/website-content-guidelines.md' => $websiteGuidelines,
        'docs/performance-and-benchmarking.md' => $performanceAuthority,
    ];
    $staleReviewStates = [
        "Andrew’s Stream API Completeness Review — Next",
        'Stage 26 — Blocked Pending Review',
        'Stage 36a Public Surface — Pending Review',
        'unauthored **Stream, Readiness, Standard I/O, And Blocking-Mode Model**',
    ];
    foreach ($activeAuthorities as $path => $authority) {
        foreach ($staleReviewStates as $stale) {
            if (str_contains($authority, $stale)) {
                $failures[] = "{$path}: active authority retains stale stream-review state {$stale}";
            }
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
