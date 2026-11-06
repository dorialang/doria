#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::mir;

const IMPORTS: &str = r#"
use Doria\Std\Io\{ReadinessSet, TransferPlan, TransferGroup, File, ReadOutcome,
    IoError, IoOperation, IoTarget, IoErrorReason};
function start(writable ReadinessSet $set, take TransferPlan $plan): TransferGroup throws IoError {
    return $set->begin($plan);
}
"#;

const BODY: &str = r#"
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    {
        let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 0));
        let $group = start($set, $plan);
    }
    $buffer[0] = 7;
    let writable $file = $set->unregister($key);
    let $outcome = $file->read($buffer->getWritableRegion(0, 1));
    if ($outcome != ReadOutcome::Data(1) || $buffer[0] != 104) {
        panic("group cleanup did not release the original file and buffer");
    }
    echo "group cleanup preserved ownership\n";
"#;

fn source(body: &str) -> String {
    format!("{IMPORTS}\nfunction main(): void {{ {body} }}")
}

fn assert_php_execution(source: &str, expected: &str) {
    let output = doriac::compile_source("group-php.doria", source, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("group.php");
    doriac::backend::publish_output(&path, output).unwrap();
    let result = std::process::Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for group parity");
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert_eq!(result.stdout, expected.as_bytes(), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
}

fn retained(function: &mir::Function) -> &mir::RetainedSourcesPlan {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) => {
                Some(plan)
            }
            _ => None,
        })
        .expect("group callable retains its source contracts")
}

#[test]
fn begin_retains_the_exclusive_set_and_inherits_the_consumed_plans_loans() {
    let program = doriac::lower_source_to_mir("group-begin.doria", source(BODY)).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let group = program
        .classes
        .iter()
        .find(|class| class.declaration_name == doriac::compiler_known_stream::TRANSFER_GROUP)
        .unwrap();
    assert_eq!(
        group.runtime_payload,
        Some(mir::RuntimeClassPayload::TransferGroup)
    );
    let payload = group.runtime_payload.unwrap();
    assert_eq!(
        payload.drop_entry_point(),
        "dr_v1_transfer_adapter_group_drop"
    );
    assert!(payload.drop_deallocates() && payload.drop_uses_frame());
    let helper = program
        .functions
        .iter()
        .find(|function| function.name == "start")
        .unwrap();
    let sources = &retained(helper).returns;
    assert_eq!(sources.len(), 2);
    assert!(sources.iter().any(|source| source.local == helper.params[0]
        && !source.inherited
        && source.writable
        && source.lifetime == mir::RetainedLifetime::Cleanup));
    assert!(sources.iter().any(|source| source.local == helper.params[1]
        && source.inherited
        && !source.writable
        && source.lifetime == mir::RetainedLifetime::Cleanup));
    assert!(program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .any(|block| {
            matches!(
                block.terminator,
                mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::BeginTransferGroup { .. },
                    ..
                }
            )
        }));
}

#[test]
fn live_group_preserves_cleanup_loans_even_without_a_later_value_use() {
    for invalid in [
        BODY.replace(
            "let $group = start($set, $plan);",
            "let $group = start($set, $plan); $buffer[0] = 9;",
        ),
        BODY.replace(
            "let $group = start($set, $plan);",
            "let $group = start($set, $plan); let $view = $set->getStream($key);",
        ),
        BODY.replace(
            "let $group = start($set, $plan);",
            "let $group = start($set, $plan); let $file = $set->unregister($key);",
        ),
        BODY.replace(
            "let $group = start($set, $plan);",
            "let $group = start($set, $plan); let $again = start($set, $plan);",
        ),
    ] {
        assert!(
            doriac::check_source("group-loan.doria", source(&invalid)).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn group_setup_requires_writable_set_access_and_cannot_return_a_loan_of_taken_set_storage() {
    for invalid in [
        source(BODY).replace(
            "writable ReadinessSet $set, take TransferPlan",
            "ReadinessSet $set, take TransferPlan",
        ),
        source(BODY).replace(
            "writable ReadinessSet $set, take TransferPlan",
            "take ReadinessSet $set, take TransferPlan",
        ),
        source(BODY).replace(
            "writable ReadinessSet $set = new",
            "ReadinessSet $set = new",
        ),
    ] {
        assert!(
            doriac::check_source("group-receiver.doria", &invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn group_loans_follow_owning_wrappers_and_collections_until_their_cleanup() {
    for (helper, value) in [
        (
            r#"class Envelope {
            TransferGroup $group;
            function __construct(parameter take TransferGroup $group) { $this->group = $group; }
        }
        function wrap(writable ReadinessSet $set, take TransferPlan $plan): Envelope throws IoError {
            return new Envelope($set->begin($plan));
        }"#,
            "wrap($set, $plan)",
        ),
        (
            r#"class Envelope {
            function __construct(take TransferGroup $group) {}
        }
        function wrap(writable ReadinessSet $set, take TransferPlan $plan): Envelope throws IoError {
            return new Envelope($set->begin($plan));
        }"#,
            "wrap($set, $plan)",
        ),
        (
            r#"function wrap(writable ReadinessSet $set, take TransferPlan $plan): List<TransferGroup> throws IoError {
            return [$set->begin($plan)];
        }"#,
            "wrap($set, $plan)",
        ),
    ] {
        let valid = format!(
            "{IMPORTS}{helper} function main(): void {{ {} }}",
            BODY.replace("start($set, $plan)", value)
        );
        assert_group_parity(valid.clone(), "group cleanup preserved ownership\n");
        for edit in ["$buffer[0] = 9;", "let $view = $set->getStream($key);"] {
            let invalid = valid.replace(
                "let $group = wrap($set, $plan);",
                &format!("let $group = wrap($set, $plan); {edit}"),
            );
            assert!(
                doriac::check_source("group-wrapper-loan.doria", &invalid).is_err(),
                "{invalid}"
            );
        }
        let escaping_helper = helper
            .replace(
                "function wrap(writable ReadinessSet $set, take TransferPlan $plan):",
                "function wrap(take ReadinessSet $set, take TransferPlan $plan):",
            )
            .replace(
                "return new Envelope($set->begin($plan));",
                "let writable $local = $set; return new Envelope($local->begin($plan));",
            )
            .replace(
                "return [$set->begin($plan)];",
                "let writable $local = $set; return [$local->begin($plan)];",
            );
        let escaping = format!("{IMPORTS}{escaping_helper} function main(): void {{}}");
        assert!(
            doriac::check_source("group-wrapper-escape.doria", &escaping).is_err(),
            "{escaping}"
        );
    }
}

#[test]
fn independent_validation_rejects_wrong_group_payload_and_borrowed_plan() {
    let original = doriac::lower_source_to_mir("group-mir.doria", source(BODY)).unwrap();
    let mut payload = original.clone();
    payload
        .classes
        .iter_mut()
        .find(|class| class.declaration_name == doriac::compiler_known_stream::TRANSFER_GROUP)
        .unwrap()
        .runtime_payload = Some(mir::RuntimeClassPayload::TransferPlan);
    assert!(doriac::mir_validation::validate_program(&payload).is_err());
    let mut borrowed = original;
    let mut changed = false;
    for function in &mut borrowed.functions {
        for block in &function.blocks {
            if let mir::Terminator::CheckedIo {
                operation: mir::CheckedIoOperation::BeginTransferGroup { plan, .. },
                ..
            } = block.terminator
            {
                function.locals[plan.0].owned = false;
                changed = true;
            }
        }
    }
    assert!(changed);
    assert!(doriac::mir_validation::validate_program(&borrowed).is_err());
}

#[test]
fn native_group_setup_and_cleanup_preserve_the_original_file_and_buffer() {
    let program = doriac::lower_source_to_mir("group-cleanup.doria", source(BODY)).unwrap();
    let expected = "group cleanup preserved ownership\n";
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert_eq!(output.exit_status, 0);
    assert!(output.stderr.is_empty());
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
}

#[test]
fn native_failed_setup_uses_configure_group_and_releases_all_loans() {
    let source = source(
        r#"
        write_file("input", "hello");
        writable ReadinessSet $set = new ReadinessSet();
        writable ReadinessSet $other = new ReadinessSet();
        let $key = $other->register(File::openRead("input"));
        writable Bytes $buffer = Bytes::allocate(1);
        try {
            let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 1));
            let $group = start($set, $plan);
            panic("foreign plan admitted");
        } catch (IoError $error) {
            if ($error->operation != IoOperation::Configure || $error->target != IoTarget::TransferGroup
                || $error->reason != IoErrorReason::InvalidInput || $error->systemCode != null
                || $error->transferredByteCount != null) { panic("wrong group setup labels"); }
        }
        $buffer[0] = 9;
        let writable $file = $other->unregister($key);
        if ($file->read($buffer->getWritableRegion(0, 1)) != ReadOutcome::Data(1)
            || $buffer[0] != 104) { panic("failed setup submitted work or retained the buffer"); }
        let $newKey = $set->register($file);
        let $returned = $set->unregister($newKey);
        echo "failed group setup preserved ownership\n";
    "#,
    );
    let program = doriac::lower_source_to_mir("group-failure.doria", source).unwrap();
    let expected = "failed group setup preserved ownership\n";
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert_eq!(output.exit_status, 0);
    assert!(output.stderr.is_empty());
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
}

#[test]
fn php_group_execution_preserves_ownership_and_drains_before_releasing_the_set() {
    let source = source(BODY);
    doriac::check_source("group-php.doria", &source).unwrap();
    assert_php_execution(&source, "group cleanup preserved ownership\n");
}

#[test]
fn observed_keys_are_initially_absent_readonly_directional_copy_values() {
    let body = BODY.replace(
        "let $group = start($set, $plan);",
        r#"let $group = start($set, $plan);
        let $read = observe($group);
        ?Doria\Std\Io\WriteTransferKey $write = $group->observedWriteKey;
        if ($read != null || $write != null) { panic("setup invented an observed operation"); }
        $saved = $read;"#,
    );
    let source = format!(
        r#"{IMPORTS}
function observe(TransferGroup $group): ?Doria\Std\Io\ReadTransferKey {{
    return $group->observedReadKey;
}}
function main(): void {{
    writable ?Doria\Std\Io\ReadTransferKey $saved = null;
    {body}
    if ($saved != null) {{ panic("observed key retained a group loan"); }}
}}
"#
    );
    assert_group_parity(source, "group cleanup preserved ownership\n");
    for property in ["observedReadKey", "observedWriteKey"] {
        let invalid = source_for_observation(&format!("$group->{property} = null;"));
        assert!(doriac::check_source("observed-key-write.doria", invalid).is_err());
    }
}

fn source_for_observation(body: &str) -> String {
    format!(
        "{IMPORTS} function observe(TransferGroup $group): void {{ {body} }} function main(): void {{}}"
    )
}

#[test]
fn read_region_access_keeps_pending_storage_unavailable_and_checks_foreign_identity() {
    for length in [0, 1] {
        assert_group_parity(
            format!(
                r#"{IMPORTS}
use Doria\Std\Io\ReadTransferKey as ReadKey;
use Doria\Std\Io\ByteRegion as Region;
function region(TransferGroup $group, ReadKey $key): ?Region throws IoError {{
    return $group->getReadRegion($key);
}}
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, {length}));
    let $readKey = $plan->getReadKey(0);
    writable Bytes $other = Bytes::allocate(1);
    let $foreignPlan = TransferPlan::read($key, $other->getWritableRegion(0, 0));
    let $foreignKey = $foreignPlan->getReadKey(0);
    {{
        let $group = $set->begin($plan);
        if (region($group, $readKey) != null || $group->getReadRegion($readKey) != null) {{
            panic("pending storage became accessible before completion dispatch");
        }}
        try {{
            let $invalid = $group->getReadRegion($foreignKey);
            panic("foreign read key was admitted");
        }} catch (IoError $error) {{
            if ($error->operation != IoOperation::Access || $error->target != IoTarget::TransferGroup
                || $error->reason != IoErrorReason::InvalidInput || $error->systemCode != null
                || $error->transferredByteCount != null) {{ panic("wrong read access error"); }}
        }}
        if ($group->getReadRegion($readKey) != null) {{ panic("invalid access retired work"); }}
    }}
    $buffer[0] = 9;
    let $file = $set->unregister($key);
    echo "pending read access preserved ownership\n";
}}
"#
            ),
            "pending read access preserved ownership\n",
        );
    }
}

#[test]
fn resubmission_rejects_unobserved_work_foreign_keys_and_invalid_ranges_without_submission() {
    for reading in [true, false] {
        let (open, region, plan, key, method) = if reading {
            (
                "File::openRead(\"input\")",
                "getWritableRegion",
                "read",
                "getReadKey",
                "resubmitRead",
            )
        } else {
            (
                "File::openWrite(\"output\", Doria\\Std\\Io\\FileCreation::CreateOrTruncate, Doria\\Std\\Io\\FileWriteMode::Positioned)",
                "getRegion",
                "writeTo",
                "getWriteKey",
                "resubmitWrite",
            )
        };
        let attempts = [(-1i64, 0i64), (0, -1), (1, 0), (0, 1), (i64::MAX, 1), (0, 0)]
            .into_iter()
            .flat_map(|(offset, length)| ["operationKey", "foreignKey"].map(|key| format!(r#"
    try {{
        $group->{method}(${key}, {offset}, {length});
        panic("resubmission discarded unobserved work or admitted an invalid range");
    }} catch (IoError $error) {{
        if ($error->operation != IoOperation::Configure || $error->target != IoTarget::TransferGroup
            || $error->reason != IoErrorReason::InvalidInput || $error->systemCode != null
            || $error->transferredByteCount != null) {{ panic("wrong resubmission error"); }}
    }}
    if ($group->observedReadKey != null || $group->observedWriteKey != null) {{ panic("validation observed a transfer"); }}
"#)))
            .collect::<String>();
        assert_group_parity(
            format!(
                r#"{IMPORTS}
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $streamKey = $set->register({open});
    writable Bytes $buffer = Bytes::allocate(1);
    writable Bytes $other = Bytes::allocate(1);
    let $plan = TransferPlan::{plan}($streamKey, $buffer->{region}(0, 0));
    let $operationKey = $plan->{key}(0);
    let $foreignPlan = TransferPlan::{plan}($streamKey, $other->{region}(0, 0));
    let $foreignKey = $foreignPlan->{key}(0);
    {{
        writable TransferGroup $group = $set->begin($plan);
        {attempts}
    }}
    $buffer[0] = 9;
    let $file = $set->unregister($streamKey);
    echo "resubmission validation preserved work\n";
}}
"#
            ),
            "resubmission validation preserved work\n",
        );
    }
}

#[test]
fn resubmission_requires_directional_keys_writable_access_and_last_use_of_read_regions() {
    let valid = source_for_region(
        r#"
function reuse(writable TransferGroup $group, ReadTransferKey $readKey, WriteTransferKey $writeKey): void throws IoError {
    let $region = $group->getReadRegion($readKey);
    if ($region != null) { echo $region->length; }
    $group->resubmitRead($readKey, 0, 1);
    $group->resubmitWrite($writeKey, 0, 1);
}"#,
    );
    let program = doriac::lower_source_to_mir("resubmit-last-use.doria", &valid).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for invalid in [
        valid.replace("writable TransferGroup $group", "TransferGroup $group"),
        valid.replace("resubmitRead($readKey", "resubmitRead($writeKey"),
        valid.replace("resubmitWrite($writeKey", "resubmitWrite($readKey"),
        valid.replace(
            "resubmitRead($readKey, 0, 1)",
            "resubmitRead($readKey, 0.0, 1)",
        ),
        valid.replace(
            "resubmitWrite($writeKey, 0, 1)",
            "resubmitWrite($writeKey, 0, 1.0)",
        ),
        valid
            .replace("    if ($region != null) { echo $region->length; }", "")
            .replace(
                "    $group->resubmitWrite",
                "    if ($region != null) { echo $region->length; }\n    $group->resubmitWrite",
            ),
    ] {
        assert!(
            doriac::check_source("resubmit-safety.doria", &invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn independent_validation_checks_resubmission_direction_range_access_and_void_result() {
    let original = doriac::lower_source_to_mir(
        "resubmit-proof.doria",
        source_for_region(
            r#"
function reuse(writable TransferGroup $group, ReadTransferKey $key): void throws IoError {
    $group->resubmitRead($key, 0, 1);
}"#,
        ),
    )
    .unwrap();
    doriac::mir_validation::validate_program(&original).unwrap();
    for mutation in 0..5 {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedIo {
                            operation: mir::CheckedIoOperation::ResubmitTransfer { .. },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let block = function
            .blocks
            .iter_mut()
            .find(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::CheckedIo {
                        operation: mir::CheckedIoOperation::ResubmitTransfer { .. },
                        ..
                    }
                )
            })
            .unwrap();
        let mir::Terminator::CheckedIo {
            operation:
                mir::CheckedIoOperation::ResubmitTransfer {
                    group,
                    key,
                    reading,
                    offset,
                    ..
                },
            result,
            ..
        } = &mut block.terminator
        else {
            unreachable!()
        };
        match mutation {
            0 => *reading = !*reading,
            1 => function.locals[group.0].writable = false,
            2 => {
                function.locals[offset.0].ty = mir::Type::Scalar(mir::ScalarType::Integer(
                    doriac::numeric::IntegerType::Int32,
                ))
            }
            3 => *result = Some(*offset),
            4 => {
                function.locals[key.0].ty = mir::Type::Scalar(mir::ScalarType::Inline(
                    doriac::inline_value::InlineValueType::Duration,
                ))
            }
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&forged).is_err(),
            "mutation {mutation}"
        );
        assert!(
            doriac::codegen_cranelift::lower_mir_to_object(&forged).is_err(),
            "mutation {mutation}"
        );
        #[cfg(feature = "llvm-backend")]
        assert!(
            doriac::codegen_llvm::lower_mir_to_object(&forged).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn php_resubmission_emits_the_shared_controller_for_both_directions() {
    for (method, key) in [
        ("resubmitRead", "ReadTransferKey"),
        ("resubmitWrite", "WriteTransferKey"),
    ] {
        let source = source_for_region(&format!(
            r#"
function reuse(writable TransferGroup $group, {key} $key): void throws IoError {{
    $group->{method}($key, 0, 1);
}}
"#
        ));
        doriac::check_source("resubmit-php.doria", &source).unwrap();
        assert_php_execution(&source, "");
    }
}

fn source_for_region(helper: &str) -> String {
    format!(
        r#"{IMPORTS}
use Doria\Std\Io\{{ReadTransferKey, WriteTransferKey, ByteRegion}};
{helper}
function main(): void {{}}
"#
    )
}

#[test]
fn read_regions_retain_the_group_and_reject_conflicting_access_and_escapes() {
    let valid = source_for_region(
        r#"
function region(TransferGroup $group, ReadTransferKey $key): ?ByteRegion throws IoError {
    return $group->getReadRegion($key);
}
function change(writable TransferGroup $group): void {}
function consume(take TransferGroup $group): void {}
function lastUse(writable TransferGroup $group, ReadTransferKey $key): void throws IoError {
    let $region = region($group, $key);
    if ($region != null) { echo $region->length; }
    change($group);
}
"#,
    );
    let program = doriac::lower_source_to_mir("read-region-last-use.doria", &valid).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    for invalid in [
        r#"function escape(take TransferGroup $group, ReadTransferKey $key): ?ByteRegion throws IoError {
            return $group->getReadRegion($key);
        }"#,
        r#"function escape(writable ReadinessSet $set, take TransferPlan $plan, ReadTransferKey $key): ?ByteRegion throws IoError {
            let $group = $set->begin($plan);
            return $group->getReadRegion($key);
        }"#,
        r#"function change(writable TransferGroup $group): void {}
        function conflict(writable TransferGroup $group, ReadTransferKey $key): void throws IoError {
            let $region = $group->getReadRegion($key);
            change($group);
            if ($region != null) { echo $region->length; }
        }"#,
        r#"function consume(take TransferGroup $group): void {}
        function conflict(take TransferGroup $group, ReadTransferKey $key): void throws IoError {
            let $region = $group->getReadRegion($key);
            consume($group);
            if ($region != null) { echo $region->length; }
        }"#,
        r#"function wrongKey(TransferGroup $group, WriteTransferKey $key): void throws IoError {
            let $region = $group->getReadRegion($key);
        }"#,
    ] {
        assert!(
            doriac::check_source("read-region-safety.doria", source_for_region(invalid)).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn independent_validation_requires_read_region_identity_result_and_retained_proof() {
    let original = doriac::lower_source_to_mir(
        "read-region-proof.doria",
        source_for_region(
            r#"
function region(TransferGroup $group, ReadTransferKey $key): ?ByteRegion throws IoError {
    return $group->getReadRegion($key);
}"#,
        ),
    )
    .unwrap();
    doriac::mir_validation::validate_program(&original).unwrap();
    for mutation in 0..5 {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedIo {
                            operation: mir::CheckedIoOperation::TransferGroupReadRegion { .. },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let (group, key, result) = function
            .blocks
            .iter()
            .find_map(|block| match block.terminator {
                mir::Terminator::CheckedIo {
                    operation: mir::CheckedIoOperation::TransferGroupReadRegion { group, key },
                    result: Some(result),
                    ..
                } => Some((group, key, result)),
                _ => None,
            })
            .unwrap();
        match mutation {
            0 => {
                function.locals[key.0].ty = mir::Type::Scalar(mir::ScalarType::Inline(
                    doriac::inline_value::InlineValueType::Duration,
                ))
            }
            1 => function.locals[result.0].owned = true,
            2 => function.locals[result.0].writable = true,
            3 => {
                for statement in function
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.statements)
                {
                    if let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(
                        plan,
                    )) = statement
                    {
                        plan.returns.clear();
                    }
                }
            }
            4 => {
                let mir::Type::Class(class) = function.locals[group.0].ty else {
                    unreachable!()
                };
                forged.classes[class.0].runtime_payload =
                    Some(mir::RuntimeClassPayload::TransferPlan);
            }
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&forged).is_err(),
            "mutation {mutation}"
        );
        assert!(
            doriac::codegen_cranelift::lower_mir_to_object(&forged).is_err(),
            "mutation {mutation}"
        );
        #[cfg(feature = "llvm-backend")]
        assert!(
            doriac::codegen_llvm::lower_mir_to_object(&forged).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn php_read_region_emission_accepts_direct_and_nullable_group_receivers() {
    for (receiver, access) in [
        ("TransferGroup", "$group->getReadRegion($key)"),
        ("?TransferGroup", "$group?->getReadRegion($key)"),
    ] {
        let source = source_for_region(&format!(
            r#"
function region({receiver} $group, ReadTransferKey $key): void throws IoError {{
    let $region = {access};
}}
"#
        ));
        doriac::check_source("read-region-php.doria", &source).unwrap();
        assert_php_execution(&source, "");
    }
}

#[test]
fn independent_validation_rejects_forged_observed_key_types_and_group_payloads() {
    let original =
        doriac::lower_source_to_mir("observed-key-types.doria", source_for_observation(""))
            .unwrap();
    doriac::mir_validation::validate_program(&original).unwrap();
    for wrong_key in [
        doriac::inline_value::InlineValueType::Duration,
        doriac::inline_value::InlineValueType::ReadTransferKey(
            original
                .classes
                .iter()
                .find(|class| {
                    class.declaration_name == doriac::compiler_known_stream::READINESS_SET
                })
                .unwrap()
                .id,
        ),
    ] {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::Return(mir::Rvalue::NullableScalar(
                            mir::NullableScalarExpression::TransferGroupKey { .. }
                        ))
                    )
                })
            })
            .unwrap();
        function.return_type = mir::ReturnType::Value(mir::Type::NullableScalar(
            mir::ScalarType::Inline(wrong_key),
        ));
        let expression = function
            .blocks
            .iter_mut()
            .find_map(|block| match &mut block.terminator {
                mir::Terminator::Return(mir::Rvalue::NullableScalar(
                    expression @ mir::NullableScalarExpression::TransferGroupKey { .. },
                )) => Some(expression),
                _ => None,
            })
            .unwrap();
        let mir::NullableScalarExpression::TransferGroupKey { key, .. } = expression else {
            unreachable!()
        };
        *key = wrong_key;
        assert!(doriac::mir_validation::validate_program(&forged).is_err());
    }
    let mut forged = original;
    forged
        .classes
        .iter_mut()
        .find(|class| class.declaration_name == doriac::compiler_known_stream::TRANSFER_GROUP)
        .unwrap()
        .runtime_payload = Some(mir::RuntimeClassPayload::TransferPlan);
    assert!(doriac::mir_validation::validate_program(&forged).is_err());
}

#[test]
fn php_group_property_access_emits_directional_keys_without_affecting_user_properties() {
    for property in ["observedReadKey", "observedWriteKey"] {
        for access in [
            format!("$group->{property}"),
            format!("$group?->{property}"),
        ] {
            let mut source = source_for_observation(&format!("let $key = {access};"));
            if access.contains("?->") {
                source = source.replace("observe(TransferGroup", "observe(?TransferGroup");
            }
            doriac::check_source("observed-key-php.doria", &source).unwrap();
            assert_php_execution(&source, "");
        }
    }
    let source = "class Other { ?int $observedReadKey { get => null; } } function main(): void { let $other = new Other(); if ($other->observedReadKey != null) { panic(\"ordinary property changed\"); } }";
    doriac::compile_source(
        "ordinary-observed-property.doria",
        source,
        BackendTarget::Php,
    )
    .unwrap();
}

const PREPARATION_FAILURE: &str = r#"
use Doria\Std\Io\{PendingReadableStream, ReadableFile, ReadinessSource, ReadRequest, WritableByteRegion};
class FailingReader implements PendingReadableStream {
    internal writable ReadableFile $source;
    static writable int $drops = 0;
    function __construct(parameter take ReadableFile $source) { $this->source = $source; }
    ReadinessSource $readinessSource { borrowed get => $this->source->readinessSource; }
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError {
        return $this->source->read($destination);
    }
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError {
        throw new IoError("original preparation error", IoOperation::Read, IoTarget::File("adapter"),
            IoErrorReason::Unsupported, 37, 2);
    }
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError {
        return null;
    }
    function __destruct() { self::drops++; }
}
function main(): void {
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new FailingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(2);
    try {
        let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 2));
        let $group = start($set, $plan);
        panic("preparation failure lost");
    } catch (IoError $error) {
        if ($error->message != "original preparation error" || $error->operation != IoOperation::Read
            || $error->target != IoTarget::File("adapter") || $error->reason != IoErrorReason::Unsupported
            || ($error->systemCode ?? 0) != 37 || ($error->transferredByteCount ?? 0) != 2) {
            panic("preparation error was relabeled or reconstructed");
        }
    }
    $buffer[0] = 9;
    {
        let writable $reader = $set->unregister($key);
        if ($reader->read($buffer->getWritableRegion(0, 1)) != ReadOutcome::Data(1)
            || $buffer[0] != 104) { panic("failed preparation submitted I/O or retained loans"); }
    }
    if (FailingReader::drops != 1) { panic("original reader was not dropped exactly once"); }
    echo "original preparation error preserved\n";
}
"#;

#[test]
fn native_group_setup_preserves_an_adapters_original_checked_error() {
    let program = doriac::lower_source_to_mir(
        "group-original-error.doria",
        format!("{IMPORTS}\n{PREPARATION_FAILURE}"),
    )
    .unwrap();
    let expected = "original preparation error preserved\n";
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert_eq!(output.exit_status, 0);
    assert!(output.stderr.is_empty());
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
}

#[test]
fn independent_validation_rejects_omitting_the_groups_retained_set_loan() {
    let mut program =
        doriac::lower_source_to_mir("group-retained-proof.doria", source(BODY)).unwrap();
    let helper = program
        .functions
        .iter_mut()
        .find(|function| function.name == "start")
        .unwrap();
    let mut changed = false;
    for statement in helper
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::RetainedSources(plan)) =
            statement
        {
            let before = plan.returns.len();
            plan.returns.retain(|source| source.inherited);
            changed |= plan.returns.len() != before;
        }
    }
    assert!(changed);
    assert!(doriac::mir_validation::validate_program(&program).is_err());
    assert!(doriac::codegen_cranelift::lower_mir_to_object(&program).is_err());
    #[cfg(feature = "llvm-backend")]
    assert!(doriac::codegen_llvm::lower_mir_to_object(&program).is_err());
}

fn recording_reader(preparation: &str, completion: &str) -> String {
    format!(
        r#"
use Doria\Std\Io\{{PendingReadableStream, ReadableFile, ReadinessSource, ReadRequest, WritableByteRegion}};
class RecordingReader implements PendingReadableStream {{
    internal writable ReadableFile $source;
    static writable int $prepares = 0;
    static writable int $completes = 0;
    static writable int $getters = 0;
    function __construct(parameter take ReadableFile $source) {{ $this->source = $source; }}
    writable function replace(take ReadableFile $next): void {{ $this->source = $next; }}
    ReadinessSource $readinessSource {{ borrowed get {{
        self::getters += 1;
        return $this->source->readinessSource;
    }} }}
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError {{
        return $this->source->read($destination);
    }}
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError {{
        self::prepares += 1;
        {preparation}
    }}
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError {{
        self::completes += 1;
        {completion}
    }}
}}
"#
    )
}

fn assert_group_parity(source: String, expected: &str) {
    let program = doriac::lower_source_to_mir("group-callback-parity.doria", &source).unwrap();
    let observed =
        doriac::mir_interpreter::interpret_with_io(&program, Default::default()).unwrap();
    assert_eq!(
        observed.output.exit_status,
        0,
        "{}",
        String::from_utf8_lossy(&observed.output.stderr)
    );
    assert_eq!(observed.output.stdout, expected.as_bytes());
    assert!(observed.output.stderr.is_empty());
    assert_eq!(
        observed.trace.file_opens,
        observed.trace.file_close_attempts
    );
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
    assert_php_execution(&source, expected);
}

#[test]
fn nullable_copy_result_writeback_can_clear_an_initially_present_home() {
    let source = r#"
use Doria\Std\Io\IoError;
enum ResultState { case Ready(int $count); case Empty; }
function create(): ResultState throws IoError { return ResultState::Ready(1); }
function probe(): ?ResultState throws IoError {
    writable ?ResultState $value = create();
    let $copy = $value;
    let $clear = function(writable ?ResultState $slot): void { $slot = null; };
    $clear($value);
    if ($copy != ResultState::Ready(1)) { panic("Copy result aliased its source"); }
    return $value;
}
function main(): void {
    if (probe() != null) { panic("nullable result remained present"); }
    echo "nullable result cleared\n";
}
"#;
    let program = doriac::lower_source_to_mir("nullable-copy-result.doria", source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, b"nullable result cleared\n");
    native_execution::assert_native_execution(
        &program,
        NativeProfile::Fast,
        "nullable result cleared\n",
    );
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        &program,
        NativeProfile::Release,
        "nullable result cleared\n",
    );
    let mut forged = program.clone();
    let function = forged
        .functions
        .iter_mut()
        .find(|function| function.name == "probe")
        .unwrap();
    let block = function
        .blocks
        .iter_mut()
        .find(|block| {
            matches!(
                block.terminator,
                mir::Terminator::CheckedIndirectCall { .. }
            )
        })
        .unwrap();
    let mir::Terminator::CheckedIndirectCall { args, .. } = &block.terminator else {
        unreachable!()
    };
    let mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Value(
        mir::PayloadEnumExpression::Use {
            ty,
            place: mir::PayloadEnumPlace::NullableLocalAssumeNonNull(target),
            ..
        },
    )) = &args[0]
    else {
        panic!("nullable call must use its proved present source")
    };
    block.statements.push(mir::Statement::AssignLocal {
        target: *target,
        value: mir::Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Null(*ty)),
    });
    let error = doriac::mir_validation::validate_program(&forged).unwrap_err();
    assert!(
        error
            .message
            .contains("without a dominating presence proof"),
        "{error:?}"
    );
}

#[test]
fn immediate_completions_retire_at_submission_and_reuse_without_repeat_callbacks() {
    for fetch in [false, true] {
        let reader = recording_reader(
            "$request->complete(ReadOutcome::Data(0));",
            "return $request->outcome;",
        );
        let reuse = if fetch {
            r#"
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(0)))) { panic("immediate result lost"); }
        $group->resubmitRead($key, 0, 0);
        if (RecordingReader::completes != 2) { panic("resubmission deferred completion"); }
        { let $region = $group->getReadRegion($key); if ($region == null || $region->length != 0) { panic("resubmitted prefix absent"); } }
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(0)))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) { panic("resubmitted result lost or repeated"); }
"#
        } else {
            ""
        };
        let count = if fetch { 2 } else { 1 };
        assert_group_parity(
            format!(
                r#"{IMPORTS}{reader}
use Doria\Std\Io\{{TransferWaitResult, TransferCompletion, Wait}};
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        if (RecordingReader::completes != 1 || $group->observedReadKey != null
            || $group->observedWriteKey != null) {{ panic("submission deferred or published completion"); }}
        {{ let $region = $group->getReadRegion($key); if ($region == null || $region->length != 0) {{ panic("immediate prefix absent before fetch"); }} }}
        {reuse}
    }}
    if (RecordingReader::completes != {count}) {{ panic("cleanup repeated completion"); }}
    let $reader = $set->unregister($registration);
    echo "immediate retirement preserved\n";
}}
"#
            ),
            "immediate retirement preserved\n",
        );
    }
}

#[test]
fn immediate_completion_errors_stay_owned_until_fetch_or_group_destruction() {
    for fetch in [false, true] {
        let reader = recording_reader(
            "$request->complete(ReadOutcome::Data(0));",
            "throw new IoError(\"queued completion failure\", IoOperation::Read, IoTarget::File(\"adapter\"), IoErrorReason::Unsupported, 37);",
        );
        let observation = if fetch {
            r#"
        try { $group->getNextCompletion(Wait::Immediate); panic("queued error lost"); }
        catch (IoError $error) {
            if ($error->message != "queued completion failure" || $error->operation != IoOperation::Read
                || $error->target != IoTarget::File("adapter") || $error->reason != IoErrorReason::Unsupported
                || ($error->systemCode ?? 0) != 37 || $error->transferredByteCount != null
                || $group->observedReadKey != $key || $group->observedWriteKey != null) { panic("original error identity lost"); }
        }
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) { panic("error repeated"); }
"#
        } else {
            ""
        };
        assert_group_parity(
            format!(
                r#"{IMPORTS}{reader}
use Doria\Std\Io\{{TransferWaitResult, Wait}};
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        if (RecordingReader::completes != 1
            || $group->observedReadKey != null || $group->getReadRegion($key) != null) {{ panic("queued error was lost or observed early"); }}
        {observation}
    }}
    if (RecordingReader::completes != 1) {{ panic("cleanup repeated completion"); }}
    let $reader = $set->unregister($registration);
    echo "queued error ownership preserved\n";
}}
"#
            ),
            "queued error ownership preserved\n",
        );
    }
}

#[test]
fn completion_fetch_preserves_each_actual_read_outcome() {
    for outcome in [
        "ReadOutcome::Data(0)",
        "ReadOutcome::WouldBlock",
        "ReadOutcome::EndOfStream",
        "ReadOutcome::TimedOut",
    ] {
        let region_check = if matches!(outcome, "ReadOutcome::Data(0)" | "ReadOutcome::EndOfStream")
        {
            "$region == null || $region->length != 0"
        } else {
            "$region != null"
        };
        let reader = recording_reader(
            "$request->complete(ReadOutcome::Data(0));",
            &format!("return {outcome};"),
        );
        assert_group_parity(
            format!(
                r#"{IMPORTS}{reader}
use Doria\Std\Io\{{TransferWaitResult, TransferCompletion, Wait}};
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        let $result = $group->getNextCompletion(Wait::Indefinite);
        if ($result != TransferWaitResult::Completed(TransferCompletion::Read($key, {outcome}))
            || $group->observedReadKey != $key || $group->observedWriteKey != null) {{ panic("nullable outcome or key changed"); }}
        {{ let $region = $group->getReadRegion($key); if ({region_check}) {{ panic("completed prefix does not match the complete adapter"); }} }}
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted
            || RecordingReader::completes != 1) {{ panic("outcome repeated"); }}
    }}
    let $original = $set->unregister($registration);
    echo "actual read outcome preserved\n";
}}
"#
            ),
            "actual read outcome preserved\n",
        );
    }
}

#[test]
fn completion_cannot_erase_an_actual_transfer_by_returning_null() {
    for preparation in [
        "$request->complete(ReadOutcome::Data(0));",
        "$this->source->prepareRead($request, $destination);",
    ] {
        let reader = recording_reader(preparation, "return null;");
        assert_group_parity(
            format!(
                r#"{IMPORTS}{reader}
use Doria\Std\Io\{{TransferWaitResult, Wait}};
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        try {{ $group->getNextCompletion(Wait::Indefinite); panic("actual outcome erased"); }}
        catch (IoError $error) {{
            if ($error->operation != IoOperation::Configure || $error->target != IoTarget::TransferGroup
                || $error->reason != IoErrorReason::InvalidInput || $group->observedReadKey != $key) {{ panic("wrong invalid completion failure"); }}
        }}
        if ($group->getReadRegion($key) != null) {{ panic("invalid completion fabricated a prefix"); }}
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted
            || RecordingReader::completes != 1) {{ panic("invalid completion repeated"); }}
    }}
    let $original = $set->unregister($registration);
    echo "actual outcome cannot become absent\n";
}}
"#
            ),
            "actual outcome cannot become absent\n",
        );
    }
}

#[test]
fn callback_region_rebinding_preserves_original_windows_across_completion_and_resubmission() {
    let reader = recording_reader(
        r#"
        if (self::prepares == 1) { $destination[0] = 65; }
        else if (self::prepares == 2) { $destination[0] = 66; }
        else { $destination[0] = 67; }
        $request->complete(ReadOutcome::Data(1));
        $destination = $destination->getWritableRegion(0, 0);
"#,
        r#"
        if ($destination->length != 4 - self::prepares) {
            panic("preparation replaced the completion window");
        }
        $destination = $destination->getWritableRegion(0, 0);
        return ReadOutcome::Data(1);
"#,
    );
    assert_group_parity(
        format!(
            r#"{IMPORTS}{reader}
use Doria\Std\Io\{{TransferWaitResult, TransferCompletion, Wait}};
function main(): void {{
    write_file("input", "unused");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(5);
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(1, 3));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        foreach (0..<3 as int $position) {{
            if ($position > 0) {{ $group->resubmitRead($key, $position, 3 - $position); }}
            if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(
                TransferCompletion::Read($key, ReadOutcome::Data(1)))) {{ panic("completion changed"); }}
            {{
                let $region = $group->getReadRegion($key);
                if ($region == null) {{ panic("completed prefix missing"); }}
                if ($region->length != 1) {{ panic("completed prefix length changed"); }}
                echo $region[0];
            }}
        }}
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{
            panic("completion repeated");
        }}
    }}
    if ($buffer[0] != 0 || $buffer[1] != 65 || $buffer[2] != 66
        || $buffer[3] != 67 || $buffer[4] != 0) {{ panic("original buffer window changed"); }}
    let $reader = $set->unregister($registration);
    echo " original callback windows\n";
}}
"#
        ),
        "656667 original callback windows\n",
    );
}

#[test]
fn mixed_completions_preserve_directional_keys_and_zero_byte_write_reuse() {
    assert_group_parity(
        format!(
            r#"{IMPORTS}
use Doria\Std\Io\{{FileCreation, FileWriteMode, WriteOutcome, TransferWaitResult, TransferCompletion, Wait}};
function main(): void {{
    write_file("input", "x");
    write_file("output", "");
    writable ReadinessSet $set = new ReadinessSet();
    let $reader = $set->register(File::openRead("input"));
    let $writer = $set->register(File::openReadWrite("output", FileCreation::OpenExisting, FileWriteMode::Positioned));
    writable Bytes $destination = Bytes::allocate(1);
    writable Bytes $source = Bytes::allocate(2);
    $source[0] = 65; $source[1] = 66;
    {{
        let $plan = TransferPlan::read($reader, $destination->getWritableRegion(0, 1))
            ->andWriteTo($writer, $source->getRegion(0, 2));
        let $read = $plan->getReadKey(0);
        let $write = $plan->getWriteKey(1);
        let writable $group = $set->begin($plan);
        writable bool $sawRead = false;
        writable bool $sawWrite = false;
        foreach (0..<2 as int $position) {{
            let $result = $group->getNextCompletion(Wait::Indefinite);
            if ($result == TransferWaitResult::Completed(TransferCompletion::Read($read, ReadOutcome::Data(1)))) {{
                if ($sawRead || $group->observedReadKey != $read || $group->observedWriteKey != null) {{ panic("read identity repeated or changed"); }}
                $sawRead = true;
            }} else if ($result == TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted(2)))) {{
                if ($sawWrite || $group->observedWriteKey != $write || $group->observedReadKey != null) {{ panic("write identity repeated or changed"); }}
                $sawWrite = true;
            }} else {{ panic("mixed completion changed"); }}
        }}
        if (!$sawRead || !$sawWrite) {{ panic("direction lost"); }}
        $group->resubmitWrite($write, 0, 0);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Write($write, WriteOutcome::Accepted(0)))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) {{ panic("zero-byte write or key reuse lost"); }}
    }}
    if ($destination[0] != 120 || read_file("output") != "AB") {{ panic("mixed transfer storage changed"); }}
    echo "mixed completion and write reuse\n";
}}
"#
        ),
        "mixed completion and write reuse\n",
    );
}

#[test]
fn completion_fetch_retires_once_exposes_prefixes_and_reuses_the_original_key() {
    let reader = recording_reader(
        "$this->source->prepareRead($request, $destination);",
        "return $request->outcome;",
    );
    assert_group_parity(
        format!(
            r#"{IMPORTS}{reader}
use Doria\Std\Io\{{TransferWaitResult, TransferCompletion, Wait}};
use Doria\Std\Time\{{Duration, Deadline}};
function main(): void {{
    write_file("input", "abcd");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(2);
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 2));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        if ($group->getReadRegion($key) != null) {{ panic("active storage exposed"); }}
        let $first = $group->getNextCompletion(Wait::Indefinite);
        let $saved = $first;
        if ($first != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(2)))
            || $group->observedReadKey != $key || $group->observedWriteKey != null) {{ panic("completion identity or outcome lost"); }}
        {{ let $region = $group->getReadRegion($key); if ($region == null || $region->length != 2) {{ panic("completed prefix absent"); }} }}
        if ($group->getNextCompletion(Wait::For(Duration::fromNanoseconds(-1))) != TransferWaitResult::Exhausted
            || $group->observedReadKey != null) {{ panic("completion repeated"); }}
        $group->resubmitRead($key, 1, 1);
        if ($group->getReadRegion($key) != null) {{ panic("resubmission exposed active buffer"); }}
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) {{ panic("reused range outcome lost"); }}
        $group->resubmitRead($key, 0, 1);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))) {{ panic("reused buffer outcome lost"); }}
        $group->resubmitRead($key, 0, 1);
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::EndOfStream))) {{ panic("EOF changed"); }}
        {{ let $empty = $group->getReadRegion($key); if ($empty == null || $empty->length != 0) {{ panic("known empty prefix became absent"); }} }}
        if ($group->getNextCompletion(Wait::Until(Deadline::fromNow(Duration::fromNanoseconds(-1)))) != TransferWaitResult::Exhausted) {{ panic("absolute exhausted wait changed"); }}
        if ($saved != $first || RecordingReader::completes != 4) {{ panic("Copy result retained a loan or completion repeated"); }}
    }}
    if ($buffer[0] != 100 || $buffer[1] != 99) {{ panic("selected range or stream position changed"); }}
    let $reader = $set->unregister($registration);
    echo "completion reuse preserved ownership\n";
}}
"#
        ),
        "completion reuse preserved ownership\n",
    );
}

#[test]
fn completion_fetch_preserves_original_checked_error_progress_and_operation_key() {
    let reader = recording_reader(
        "$this->source->prepareRead($request, $destination);",
        "throw new IoError(\"original completion error\", IoOperation::Read, IoTarget::File(\"adapter\"), IoErrorReason::Unsupported, 37, -17);",
    );
    assert_group_parity(
        format!(
            r#"{IMPORTS}{reader}
use Doria\Std\Io\{{TransferWaitResult, Wait}};
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(1);
    {{
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        try {{ $group->getNextCompletion(Wait::Indefinite); panic("original error lost"); }} catch (IoError $error) {{
            if ($group->observedReadKey != $key || $group->observedWriteKey != null
                || $error->message != "original completion error" || $error->operation != IoOperation::Read
                || $error->target != IoTarget::File("adapter") || ($error->systemCode ?? 0) != 37
                || ($error->transferredByteCount ?? 0) != -17) {{ panic("error relabeled, cloned or reconstructed"); }}
        }}
        if ($group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted
            || RecordingReader::completes != 1) {{ panic("owned error or completion repeated"); }}
    }}
    let $reader = $set->unregister($registration);
    echo "original completion error preserved\n";
}}
"#
        ),
        "original completion error preserved\n",
    );
}

#[test]
fn independent_validation_requires_canonical_completion_and_wait_schemas() {
    let original = doriac::lower_source_to_mir(
        "completion-schema.doria",
        format!(
            r#"{IMPORTS}
use Doria\Std\Io\{{Wait, TransferWaitResult}};
function next(writable TransferGroup $group, Wait $wait): TransferWaitResult throws IoError {{
    return $group->getNextCompletion($wait);
}}
function main(): void {{}}
"#
        ),
    )
    .unwrap();
    doriac::mir_validation::validate_program(&original).unwrap();
    for mutation in 0..7 {
        let mut forged = original.clone();
        let function = forged
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedIo {
                            operation: mir::CheckedIoOperation::ObserveTransferGroup { .. },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let (group, wait, result, outcome) = function
            .blocks
            .iter()
            .find_map(|block| match block.terminator {
                mir::Terminator::CheckedIo {
                    operation:
                        mir::CheckedIoOperation::ObserveTransferGroup {
                            group,
                            wait,
                            outcome,
                        },
                    result: Some(result),
                    ..
                } => Some((group, wait, result, outcome)),
                _ => None,
            })
            .unwrap();
        let mir::Type::PayloadEnum(wait_type) = function.locals[wait.0].ty else {
            unreachable!()
        };
        match mutation {
            0 => function.locals[group.0].writable = false,
            1 => function.locals[result.0].owned = true,
            2 => {
                forged
                    .enums
                    .iter_mut()
                    .find(|definition| definition.id == wait_type.id)
                    .unwrap()
                    .name = "OtherWait".into()
            }
            3 => {
                forged
                    .enums
                    .iter_mut()
                    .find(|definition| definition.id == outcome.id)
                    .unwrap()
                    .name = "OtherResult".into()
            }
            4 => {
                forged
                    .enums
                    .iter_mut()
                    .find(|definition| definition.id == outcome.id)
                    .unwrap()
                    .cases[0]
                    .name = "OtherCompletion".into()
            }
            5 | 6 => {
                let definition = forged
                    .enums
                    .iter()
                    .find(|definition| definition.id == outcome.id)
                    .unwrap();
                let mir::Type::PayloadEnum(completion) = definition.cases[0].payload[0].ty else {
                    unreachable!()
                };
                let definition = forged
                    .enums
                    .iter_mut()
                    .find(|definition| definition.id == completion.id)
                    .unwrap();
                if mutation == 5 {
                    definition.name = "OtherCompletion".into();
                } else {
                    definition.cases.swap(0, 1);
                }
            }
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&forged).is_err(),
            "mutation {mutation}"
        );
        assert!(
            doriac::codegen_cranelift::lower_mir_to_object(&forged).is_err(),
            "mutation {mutation}"
        );
        #[cfg(feature = "llvm-backend")]
        assert!(
            doriac::codegen_llvm::lower_mir_to_object(&forged).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn setup_failure_cleans_prepared_operations_without_submission() {
    let reader = recording_reader(
        "$this->source->prepareRead($request, $destination);",
        "if ($request->outcome != null) { panic(\"unsubmitted cleanup fabricated a transfer\"); } return null;",
    );
    let failure_class = PREPARATION_FAILURE
        .split_once("class FailingReader")
        .unwrap()
        .1
        .split("function main(): void")
        .next()
        .unwrap();
    assert_group_parity(
        format!(
            r#"{IMPORTS}{reader}class FailingReader{failure_class}
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $first = $set->register(new RecordingReader(File::openRead("input")));
    let $second = $set->register(new FailingReader(File::openRead("input")));
    writable Bytes $a = Bytes::allocate(1);
    writable Bytes $b = Bytes::allocate(1);
    $a[0] = 7;
    try {{
        let $plan = TransferPlan::read($first, $a->getWritableRegion(0, 1))
            ->andReadFrom($second, $b->getWritableRegion(0, 1));
        let $group = $set->begin($plan);
        panic("failing preparation succeeded");
    }} catch (IoError $error) {{
        if ($error->message != "original preparation error" || $error->operation != IoOperation::Read) {{
            panic("setup replaced the original error");
        }}
    }}
    if ($a[0] != 7 || RecordingReader::prepares != 1 || RecordingReader::completes != 1
        || RecordingReader::getters != 4) {{ panic("prepared cleanup or source selection drifted"); }}
    let writable $original = $set->unregister($first);
    if ($original->read($a->getWritableRegion(0, 1)) != ReadOutcome::Data(1) || $a[0] != 104) {{
        panic("setup failure performed I/O");
    }}
    echo "prepared cleanup without submission\n";
}}
"#
        ),
        "prepared cleanup without submission\n",
    );
}

#[test]
fn immediate_request_presence_and_repeated_configuration_cleanup_match_backends() {
    for (preparation, completion, body, expected) in [
        (
            "$request->complete(ReadOutcome::WouldBlock);",
            "if ($request->outcome != ReadOutcome::WouldBlock) { panic(\"immediate presence lost\"); } return $request->outcome;",
            "{ let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 1)); let $group = $set->begin($plan); }",
            "immediate preserved\n",
        ),
        (
            "$request->complete(ReadOutcome::WouldBlock); $request->complete(ReadOutcome::Data(0));",
            "if ($request->outcome != ReadOutcome::WouldBlock) { panic(\"first selection lost\"); } return $request->outcome;",
            "try { let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 1)); let $group = $set->begin($plan); panic(\"repeated selection admitted\"); } catch (IoError $error) { if ($error->operation != IoOperation::Configure || $error->target != IoTarget::TransferGroup || $error->reason != IoErrorReason::InvalidInput) { panic(\"wrong configuration failure\"); } }",
            "repeated selection rejected\n",
        ),
    ] {
        let reader = recording_reader(preparation, completion);
        assert_group_parity(
            format!(
                r#"{IMPORTS}{reader}
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(1);
    $buffer[0] = 7;
    {body}
    if ($buffer[0] != 7 || RecordingReader::prepares != 1 || RecordingReader::completes != 1
        || RecordingReader::getters != 3) {{ panic("immediate transfer performed I/O or repeated cleanup"); }}
    let $original = $set->unregister($key);
    echo "{expected}";
}}
"#
            ),
            expected,
        );
    }
}

#[test]
fn group_setup_follows_a_replaced_source_without_using_registration_time_storage() {
    let reader = recording_reader(
        "$this->source->prepareRead($request, $destination);",
        "if ($request->outcome != ReadOutcome::Data(0)) { panic(\"empty result lost\"); } return $request->outcome;",
    );
    assert_group_parity(
        format!(
            r#"{IMPORTS}{reader}
function main(): void {{
    write_file("first", "old");
    write_file("second", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new RecordingReader(File::openRead("first")));
    {{ let writable $view = $set->getWritableStream($key); $view->replace(File::openRead("second")); }}
    writable Bytes $buffer = Bytes::allocate(1);
    {{ let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 0)); let $group = $set->begin($plan); }}
    if (RecordingReader::getters != 4 || RecordingReader::prepares != 1 || RecordingReader::completes != 1) {{
        panic("registration did not follow the current source at each boundary");
    }}
    let writable $original = $set->unregister($key);
    if ($original->read($buffer->getWritableRegion(0, 1)) != ReadOutcome::Data(1) || $buffer[0] != 104) {{
        panic("group used the stale file or moved the current cursor");
    }}
    echo "group followed the replaced source\n";
}}
"#
        ),
        "group followed the replaced source\n",
    );
}

#[test]
fn invalid_preparation_preserves_its_original_cleanup_error() {
    let reader = recording_reader(
        "$request->complete(ReadOutcome::WouldBlock); $request->complete(ReadOutcome::Data(0));",
        "throw new IoError(\"cleanup rejection\", IoOperation::Read, IoTarget::File(\"original\"), IoErrorReason::Unsupported, 53, 0);",
    );
    assert_group_parity(
        format!(
            r#"{IMPORTS}{reader}
function main(): void {{
    write_file("input", "hello");
    writable ReadinessSet $set = new ReadinessSet();
    let $key = $set->register(new RecordingReader(File::openRead("input")));
    writable Bytes $buffer = Bytes::allocate(1);
    try {{
        let $plan = TransferPlan::read($key, $buffer->getWritableRegion(0, 1));
        let $group = $set->begin($plan);
        panic("invalid preparation succeeded");
    }} catch (IoError $error) {{
        if ($error->message != "cleanup rejection" || $error->operation != IoOperation::Read
            || $error->target != IoTarget::File("original") || ($error->systemCode ?? 0) != 53
            || ($error->transferredByteCount ?? -1) != 0) {{ panic("cleanup error was reconstructed"); }}
    }}
    if (RecordingReader::prepares != 1 || RecordingReader::completes != 1) {{ panic("cleanup count drifted"); }}
    $buffer[0] = 7;
    let $original = $set->unregister($key);
    echo "invalid preparation kept its cleanup error\n";
}}
"#
        ),
        "invalid preparation kept its cleanup error\n",
    );
}
