<?php

declare(strict_types=1);

/**
 * What the local vet runs that CI's LLVM release jobs also run. One list, so
 * `check_vet_covers_ci.php` can hold CI to it and a release-only failure is
 * found before a push.
 */

/**
 * The integration suites run with `--features llvm-backend`.
 *
 * @return list<string>
 */
function llvm_test_suites(): array
{
    return [
        'mir_validation_tests',
        'llvm_mir_tests',
        'cli_tests',
        'constructor_owned_property_tests',
        'stage17_io_tests',
        'stage18_tests',
        'stage21_tests',
        'stage22_tests',
        'stage25a_tests',
        'stage27_tests',
        'return_borrow_alias_tests',
        'writable_borrow_writeback_tests',
        'returned_closure_transport_tests',
        'stage35_runtime_tests',
        'stage35a_optimizer_tests',
        'stage36_property_hook_tests',
        'property_hook_initializer_tests',
        'property_hook_cleanup_effect_tests',
        'php_integer_execution_tests',
        'bytes_allocate_tests',
        'byte_region_execution_tests',
        'byte_region_mir_validation_tests',
        'byte_region_transport_tests',
        'byte_region_native_structure_tests',
        'consuming_receiver_tests',
        'stream_capability_tests',
        'time_contract_tests',
        'nullable_default_tests',
        'registration_key_tests',
        'readiness_set_contract_tests',
        'transfer_plan_tests',
        'pending_stream_contract_tests',
        'pending_selected_storage_tests',
        'pending_selected_storage_mir_tests',
        'io_progress_native_structure_tests',
        'indexed_access_span_tests',
        'performance_report_tests',
        'native_testing_slice1_tests',
        'native_mir_parity_tests',
    ];
}

/**
 * The runtime's optimized test runs. Timing and inlining differ from a debug
 * build, so a debug pass does not stand in for these.
 *
 * @return list<list<string>>
 */
function release_runtime_tests(): array
{
    return [
        ['cargo', 'test', '-p', 'doria-rt', '--release', '--locked'],
        [
            'cargo',
            'test',
            '-p',
            'doria-rt',
            '--release',
            '--no-default-features',
            '--features',
            'hosted-shared-runtime',
            '--locked',
        ],
    ];
}
