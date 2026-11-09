#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;
#[path = "common/native_structure.rs"]
mod native_structure;

use doriac::backend::NativeProfile;
use doriac::compiler_known_concurrency::{CANCELLATION_SOURCE, SOURCE_ID};
use doriac::mir::{self, RuntimeClassPayload};

const SOURCE: &str = r#"
use Doria\Std\Concurrency\CancellationError;
class Source {
    function __construct() {}
}
function describe(CancellationError $error): string { return $error->message; }
function create(): Source { return new Source(); }
function inspect(Source $source): void {}
function update(writable Source $source): void {}
function exercise(): void {
    let writable $control = new Source();
    inspect($control);
    update($control);
    let writable $iteration = 0;
    while ($iteration < 1000) {
        let writable $local = new Source();
        inspect($local);
        update($local);
        let $returned = create();
        $iteration++;
    }
}
function main(): void {
    exercise();
    echo "source factories retired\n";
}
"#;

// Exercise typed MIR transport independently of authored invocation lowering.
fn source_fixture() -> mir::Program {
    let source_text = SOURCE.replace(
        "class Source {\n    function __construct() {}\n}",
        "use Doria\\Std\\Concurrency\\CancellationSource as Source;",
    );
    let mut program = doriac::lower_source_to_mir("source-factory.doria", source_text).unwrap();
    let source = program
        .classes
        .iter_mut()
        .find(|class| class.name == CANCELLATION_SOURCE)
        .unwrap();
    source.name = CANCELLATION_SOURCE.to_owned();
    source.declaration_name = source.name.clone();
    source.source_span = doriac::source::Span::in_source(SOURCE_ID, 0, 0);
    source.runtime_payload = Some(RuntimeClassPayload::CancellationSource);
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

fn request_fixture() -> mir::Program {
    let mut program = doriac::lower_source_to_mir(
        "source-request.doria",
        r#"
use Doria\Std\Concurrency\CancellationError;
use Doria\Std\Concurrency\CancellationSource as Source;
function request(writable Source $source): void throws CancellationError {}
function status(Source $source): bool { return false; }
function main(): void {
    let writable $source = new Source();
    echo "before: {status($source)}\n";
    try { request($source); request($source); }
    catch (CancellationError $error) { echo "failure: {$error->message}\n"; }
    echo "after: {status($source)}\n";
}
"#,
    )
    .unwrap();
    let source = program
        .classes
        .iter_mut()
        .find(|class| class.name == CANCELLATION_SOURCE)
        .unwrap();
    source.name = CANCELLATION_SOURCE.into();
    source.declaration_name = source.name.clone();
    source.source_span = doriac::source::Span::in_source(SOURCE_ID, 0, 0);
    source.runtime_payload = Some(RuntimeClassPayload::CancellationSource);
    let request = program
        .functions
        .iter_mut()
        .find(|function| function.name == "request")
        .unwrap();
    let error = mir::LocalId(request.locals.len());
    request.locals.push(mir::Local {
        id: error,
        name: "request-error".into(),
        ty: mir::Type::ERROR,
        writable: false,
        owned: true,
        synthetic: true,
    });
    request.entry_block = mir::BlockId(0);
    request.blocks = vec![
        mir::BasicBlock {
            id: mir::BlockId(0),
            statements: vec![],
            terminator: mir::Terminator::CheckedCancellationRequest {
                source: request.params[0],
                result: None,
                error,
                success: mir::BlockId(1),
                failure: mir::BlockId(2),
                span: request.source_span,
            },
        },
        mir::BasicBlock {
            id: mir::BlockId(1),
            statements: vec![],
            terminator: mir::Terminator::ReturnVoid,
        },
        mir::BasicBlock {
            id: mir::BlockId(2),
            statements: vec![],
            terminator: mir::Terminator::PropagateError { error },
        },
    ];
    let status = program
        .functions
        .iter_mut()
        .find(|function| function.name == "status")
        .unwrap();
    status.entry_block = mir::BlockId(0);
    status.blocks = vec![mir::BasicBlock {
        id: mir::BlockId(0),
        statements: vec![],
        terminator: mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Bool(
            mir::BoolExpression::CancellationRequested {
                source: status.params[0],
            },
        ))),
    }];
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

fn token_fixture() -> mir::Program {
    use doriac::inline_value::InlineValueType;
    let mut program = source_fixture();
    let source = program
        .classes
        .iter()
        .find(|class| class.name == CANCELLATION_SOURCE)
        .unwrap()
        .id;
    let token_class = program
        .classes
        .iter()
        .find(|class| class.name == doriac::compiler_known_concurrency::CANCELLATION_TOKEN)
        .unwrap()
        .id;
    let ty = InlineValueType::CancellationToken(token_class);
    let value_type = mir::Type::Scalar(mir::ScalarType::Inline(ty));

    let mut factory = program
        .functions
        .iter()
        .find(|function| function.name == "inspect")
        .unwrap()
        .clone();
    factory.id = mir::FunctionId(program.functions.len());
    factory.name = "produceToken".into();
    factory.return_type = mir::ReturnType::Value(value_type);
    factory.entry_block = mir::BlockId(0);
    factory.blocks = vec![mir::BasicBlock {
        id: mir::BlockId(0),
        statements: vec![],
        terminator: mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Inline(
            mir::InlineValueExpression::CancellationToken {
                ty,
                source: factory.params[0],
                span: factory.source_span,
            },
        ))),
    }];
    let factory_id = factory.id;
    program.functions.push(factory);
    let source_factory = program
        .functions
        .iter()
        .find(|function| function.name == "exercise")
        .unwrap()
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement {
            mir::Statement::AssignLocal {
                value: value @ mir::Rvalue::Class(mir::ClassExpression::New { .. }),
                ..
            } => Some(value.clone()),
            _ => None,
        })
        .unwrap();
    let main = &mut program.functions[program.entry.0];
    main.locals = [
        mir::Type::Class(source),
        value_type,
        value_type,
        mir::Type::NullableScalar(mir::ScalarType::Inline(ty)),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, ty)| mir::Local {
        id: mir::LocalId(id),
        name: format!("claim-{id}"),
        ty,
        writable: false,
        owned: true,
        synthetic: true,
    })
    .collect();
    let copy = |local| {
        mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
            ty,
            operand: mir::Operand::Local(mir::LocalId(local)),
            mode: mir::InlineValueUseMode::Copy,
        })
    };
    main.entry_block = mir::BlockId(0);
    main.blocks = vec![mir::BasicBlock {
        id: mir::BlockId(0),
        statements: vec![
            mir::Statement::AssignLocal {
                target: mir::LocalId(0),
                value: source_factory,
            },
            mir::Statement::AssignLocal {
                target: mir::LocalId(1),
                value: mir::Rvalue::Value(mir::ValueExpression::Inline(
                    mir::InlineValueExpression::Call {
                        ty,
                        function: factory_id,
                        args: vec![mir::Rvalue::Class(mir::ClassExpression::Local {
                            class: source,
                            local: mir::LocalId(0),
                            transfer: false,
                        })],
                        span: main.source_span,
                    },
                )),
            },
            mir::Statement::DropClass {
                local: mir::LocalId(0),
                class: source,
                runs_destructor: true,
            },
            mir::Statement::AssignLocal {
                target: mir::LocalId(2),
                value: mir::Rvalue::Value(copy(1)),
            },
            mir::Statement::DropInlineValue {
                local: mir::LocalId(1),
                ty,
                nullable: false,
            },
            mir::Statement::AssignLocal {
                target: mir::LocalId(3),
                value: mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(copy(2))),
            },
            mir::Statement::DropInlineValue {
                local: mir::LocalId(2),
                ty,
                nullable: false,
            },
            mir::Statement::DropInlineValue {
                local: mir::LocalId(3),
                ty,
                nullable: true,
            },
        ],
        terminator: mir::Terminator::ReturnVoid,
    }];
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

#[test]
fn token_factory_returns_an_independent_copy_claim_that_outlives_its_source() {
    let program = token_fixture();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 0);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, "");
    }
}

#[test]
fn token_production_uses_only_the_shared_retain_runtime_operation() {
    let program = token_fixture();
    let roots = native_structure::authored_symbols(&program, &["produceToken"]);
    let assert_references = |references: std::collections::BTreeSet<String>| {
        assert!(references.contains(doriac::native_abi::CANCELLATION_RETAIN));
        for forbidden in [
            doriac::native_abi::CANCELLATION_NEW,
            doriac::native_abi::CANCELLATION_RELEASE,
            doriac::native_abi::CHECKED_CANCELLATION_REQUEST,
            doriac::native_abi::CLASS_ALLOCATE,
            doriac::native_abi::CLASS_FREE,
            doriac::native_abi::CHECKED_IO,
        ] {
            assert!(
                !references.contains(forbidden),
                "unexpected token factory call: {forbidden}"
            );
        }
    };
    assert_references(native_structure::reachable_references(
        &native_structure::cranelift_references(&program),
        roots.clone(),
    ));
    #[cfg(feature = "llvm-backend")]
    {
        let (_, graph) = native_structure::llvm_references(&program);
        assert_references(native_structure::reachable_references(&graph, roots));
    }
}

#[test]
fn token_factory_rejects_wrong_nominal_source_and_ownership_claims() {
    let program = token_fixture();
    let factory = program
        .functions
        .iter()
        .find(|function| function.name == "produceToken")
        .unwrap()
        .id;
    for fault in 0..6 {
        let mut invalid = program.clone();
        let function = &mut invalid.functions[factory.0];
        let mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Inline(
            mir::InlineValueExpression::CancellationToken { ty, source, .. },
        ))) = &mut function.blocks[0].terminator
        else {
            unreachable!()
        };
        match fault {
            0 => *ty = doriac::inline_value::InlineValueType::Duration,
            1 => function.locals[source.0].ty = mir::Type::String,
            2 => {
                invalid.classes[ty.nominal_class().unwrap().0]
                    .source_span
                    .source = doriac::source::SourceId(0)
            }
            3 => invalid.functions[program.entry.0].locals[1].owned = false,
            4 => invalid.functions[program.entry.0].blocks[0]
                .statements
                .swap(1, 2),
            5 => {
                *ty = doriac::inline_value::InlineValueType::CancellationToken(
                    doriac::class_layout::ClassId(usize::MAX),
                )
            }
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "fault {fault}"
        );
    }
}

#[test]
fn cancellation_request_publishes_shared_state_and_is_idempotent_across_native_profiles() {
    let program = request_fixture();
    let expected = "before: false\nafter: true\n";
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, expected);
    }
}

#[test]
fn cancellation_request_and_status_use_the_concurrency_runtime_not_io() {
    let program = request_fixture();
    let roots = native_structure::authored_symbols(&program, &["request", "status"]);
    let graph = native_structure::cranelift_references(&program);
    let references = native_structure::reachable_references(&graph, roots.clone());
    assert!(references.contains(doriac::native_abi::CHECKED_CANCELLATION_REQUEST));
    assert!(references.contains(doriac::native_abi::CANCELLATION_IS_REQUESTED));
    assert!(!references.contains(doriac::native_abi::CHECKED_IO));
    #[cfg(feature = "llvm-backend")]
    {
        let (_, graph) = native_structure::llvm_references(&program);
        let references = native_structure::reachable_references(&graph, roots);
        assert!(references.contains(doriac::native_abi::CHECKED_CANCELLATION_REQUEST));
        assert!(references.contains(doriac::native_abi::CANCELLATION_IS_REQUESTED));
        assert!(!references.contains(doriac::native_abi::CHECKED_IO));
    }
}

#[test]
fn cancellation_request_rejects_wrong_source_effect_slots_and_edges() {
    let program = request_fixture();
    let request_id = program
        .functions
        .iter()
        .find(|function| function.name == "request")
        .unwrap()
        .id;
    for fault in 0..6 {
        let mut invalid = program.clone();
        let request = &mut invalid.functions[request_id.0];
        let mir::Terminator::CheckedCancellationRequest {
            source,
            result,
            error,
            success,
            failure,
            ..
        } = &mut request.blocks[0].terminator
        else {
            unreachable!()
        };
        match fault {
            0 => request.locals[source.0].writable = false,
            1 => request.locals[source.0].ty = mir::Type::String,
            2 => *result = Some(*source),
            3 => request.locals[error.0].owned = false,
            4 => request.checked_effects.clear(),
            5 => *failure = *success,
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "fault {fault}"
        );
    }
}

#[test]
fn source_factory_and_release_belong_to_concurrency_runtime_identity() {
    let payload = RuntimeClassPayload::CancellationSource;
    assert_eq!(
        RuntimeClassPayload::for_declaration(CANCELLATION_SOURCE, SOURCE_ID),
        Some(payload)
    );
    assert_eq!(
        RuntimeClassPayload::for_declaration(CANCELLATION_SOURCE, doriac::source::SourceId(0)),
        None
    );
    assert_eq!(
        RuntimeClassPayload::for_declaration("Source", SOURCE_ID),
        None
    );
    assert_eq!(
        payload.constructor_factory(),
        Some(doriac::native_abi::CANCELLATION_NEW)
    );
    assert_eq!(
        payload.drop_entry_point(),
        doriac::native_abi::CANCELLATION_RELEASE
    );
    assert!(payload.drop_deallocates());
    assert!(!payload.drop_uses_frame());
    for other in [
        RuntimeClassPayload::OwnedFile,
        RuntimeClassPayload::ReadinessSet,
        RuntimeClassPayload::TransferPlan,
        RuntimeClassPayload::TransferGroup,
    ] {
        assert_eq!(other.constructor_factory(), None);
    }
}

#[test]
fn opaque_source_storage_cannot_be_stack_promoted_or_forge_declaration_ownership() {
    let program = source_fixture();
    let source = program
        .classes
        .iter()
        .find(|class| class.name == CANCELLATION_SOURCE)
        .unwrap();
    let facts = doriac::mir_validation::optimization_facts(&program);
    assert!(facts.functions.iter().all(|function| !function
        .stack_classes
        .values()
        .any(|class| *class == source.id)));
    for function in &program.functions {
        for (index, local) in function.params.iter().enumerate() {
            if function.locals[local.0].ty == mir::Type::Class(source.id) {
                let parameter = facts.functions[function.id.0].parameters[index];
                assert!(!parameter.readonly && !parameter.noalias && !parameter.nocapture);
                assert!(!parameter.nonnull);
                assert_eq!(parameter.dereferenceable, 0);
                assert_eq!(parameter.alignment, 0);
            }
        }
    }
    let ordinary = doriac::lower_source_to_mir("ordinary-source.doria", SOURCE).unwrap();
    let ordinary_source = ordinary
        .classes
        .iter()
        .find(|class| class.name == "Source")
        .unwrap();
    assert!(doriac::mir_validation::optimization_facts(&ordinary)
        .functions
        .iter()
        .any(|function| function
            .stack_classes
            .values()
            .any(|class| *class == ordinary_source.id)));
    for fault in 0..5 {
        let mut invalid = program.clone();
        let class = &mut invalid.classes[source.id.0];
        match fault {
            0 => class.source_span.source = doriac::source::SourceId(0),
            1 => class.runtime_payload = None,
            2 => class.declaration_name = "Source".to_owned(),
            3 => class.destructor = class.constructor,
            4 => class.is_open = true,
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "fault {fault}"
        );
    }
}

#[test]
fn runtime_factory_rejects_authored_storage_initialization_and_arguments() {
    let program = source_fixture();
    for properties in [false, true] {
        let mut invalid = program.clone();
        let value = invalid
            .functions
            .iter_mut()
            .flat_map(|function| &mut function.blocks)
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match statement {
                mir::Statement::AssignLocal { value, .. }
                    if matches!(value, mir::Rvalue::Class(mir::ClassExpression::New { .. })) =>
                {
                    Some(value)
                }
                _ => None,
            })
            .unwrap();
        let argument = value.clone();
        let mir::Rvalue::Class(mir::ClassExpression::New {
            concrete_class,
            properties: fields,
            args,
            ..
        }) = value
        else {
            unreachable!();
        };
        if properties {
            fields.push(mir::PropertyValue {
                property: doriac::class_layout::PropertyId {
                    class: *concrete_class,
                    index: 0,
                },
                source: mir::PropertyValueSource::ConstructorBody,
            });
        } else {
            args.push(argument);
        }
        let error = doriac::mir_validation::validate_program(&invalid).unwrap_err();
        assert!(
            error
                .message
                .contains("opaque runtime factory requires its exact class"),
            "{error:?}"
        );
    }
}

#[test]
fn source_factory_uses_one_runtime_state_and_never_ordinary_allocation_or_cancellation() {
    let program = source_fixture();
    let roots = native_structure::authored_symbols(&program, &["exercise"]);
    let references = native_structure::reachable_references(
        &native_structure::cranelift_references(&program),
        roots.clone(),
    );
    assert_source_references(&references);
    #[cfg(feature = "llvm-backend")]
    {
        let (_, graph) = native_structure::llvm_references(&program);
        assert_source_references(&native_structure::reachable_references(&graph, roots));
    }
}

fn assert_source_references(references: &std::collections::BTreeSet<String>) {
    assert!(references.contains(doriac::native_abi::CANCELLATION_NEW));
    assert!(references.contains(doriac::native_abi::CANCELLATION_RELEASE));
    for forbidden in [
        doriac::native_abi::CLASS_ALLOCATE,
        doriac::native_abi::CLASS_FREE,
        doriac::native_abi::CHECKED_CANCELLATION_REQUEST,
        doriac::native_abi::CANCELLATION_RETAIN,
    ] {
        assert!(
            !references.contains(forbidden),
            "unexpected source lifecycle call: {forbidden}"
        );
    }
}

#[test]
fn source_factory_and_returned_owner_execute_and_retire_across_native_profiles() {
    let program = source_fixture();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"source factories retired\n");
    assert!(output.stderr.is_empty());
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, "source factories retired\n");
    }
}
