//! Selection effects must be proved from executable MIR, not source summaries.

use doriac::mir::{Function, LocalId, Program, Statement};

#[test]
fn stream_operations_follow_exact_canonical_method_and_getter_origins() {
    use doriac::ast::Item;
    use doriac::compiler_known_stream::{declarations, StreamOperation};
    let mut found = std::collections::HashSet::new();
    for item in &declarations().items {
        let Item::Interface(interface) = item else {
            continue;
        };
        for origin in interface
            .requirements
            .iter()
            .map(|method| method.span)
            .chain(
                interface
                    .properties
                    .iter()
                    .flat_map(|property| property.hooks.iter().map(|hook| hook.span)),
            )
        {
            if let Some(operation) = StreamOperation::from_requirement(origin) {
                assert!(found.insert(operation));
                assert_eq!(
                    StreamOperation::from_requirement(origin.at(origin.start, origin.end + 1)),
                    None
                );
            }
        }
    }
    assert_eq!(
        found,
        std::collections::HashSet::from([
            StreamOperation::Read,
            StreamOperation::Write,
            StreamOperation::ReadinessSource,
            StreamOperation::PrepareRead,
            StreamOperation::PrepareWrite,
            StreamOperation::CompleteRead,
            StreamOperation::CompleteWrite
        ])
    );
}

const HELPER: &str = r#"
function select(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): void {}
"#;

fn lower(preparation: &str, helpers: &str) -> Program {
    lower_with_owned_fields(preparation, helpers, "", "", "")
}

fn lower_with_owned_fields(
    preparation: &str,
    helpers: &str,
    fields: &str,
    parameters: &str,
    initialization: &str,
) -> Program {
    lower_with_contract(
        preparation,
        helpers,
        fields,
        parameters,
        initialization,
        "PendingReadableStream",
    )
}

fn lower_with_contract(
    preparation: &str,
    helpers: &str,
    fields: &str,
    parameters: &str,
    initialization: &str,
    contract: &str,
) -> Program {
    let source = format!(
        r#"
use Doria\Std\Io\{{PendingReadableStream, ReadableFile, ReadRequest, ReadOutcome,
    ReadinessSource, WritableByteRegion, IoError}};
{helpers}
class Pipeline implements {contract} {{
    internal writable ReadableFile $file;
    internal writable Bytes $buffer;
    internal writable int $counter = 0;
    {fields}
    function __construct(parameter take ReadableFile $file{parameters}) {{
        $this->file = $file;
        $this->buffer = Bytes::allocate(4);
        {initialization}
    }}
    ReadinessSource $readinessSource {{ borrowed get => $this->file->readinessSource; }}
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError {{
        return $this->file->read($destination);
    }}
    writable function prepareRead(writable ReadRequest $request,
        writable WritableByteRegion $destination): void throws IoError {{
        {preparation}
    }}
    writable function completeRead(ReadRequest $request,
        writable WritableByteRegion $destination): ?ReadOutcome throws IoError {{
        return ReadOutcome::WouldBlock;
    }}
}}
function main(): void {{}}
"#
    );
    doriac::lower_source_to_mir("pending-selected-storage-mir.doria", source).unwrap_or_else(
        |diagnostics| {
            let failures = diagnostics
                .iter()
                .map(|diagnostic| (&diagnostic.code, &diagnostic.message, diagnostic.span))
                .collect::<Vec<_>>();
            panic!("fixture must lower before forging: {failures:#?}")
        },
    )
}

fn helper(program: &mut Program) -> &mut Function {
    program
        .functions
        .iter_mut()
        .find(|function| function.name == "select")
        .expect("select helper")
}

fn local(function: &Function, name: &str) -> LocalId {
    function
        .locals
        .iter()
        .find(|local| local.name == name)
        .unwrap()
        .id
}

fn select_region(program: &mut Program, selected: &str) {
    use doriac::mir::{BasicBlock, BlockId, CheckedIoOperation, Terminator};
    let template = program
        .functions
        .iter()
        .find_map(|function| {
            function
                .blocks
                .iter()
                .find_map(|block| match block.terminator {
                    Terminator::CheckedIo {
                        operation: CheckedIoOperation::PreparePendingFile { .. },
                        error,
                        span,
                        ..
                    } => Some((function.locals[error.0].clone(), span)),
                    _ => None,
                })
        })
        .expect("canonical checked preparation intrinsic");
    let function = helper(program);
    let file = function.params[0];
    let request = function.params[1];
    let region = if selected == "region" {
        function.params[2]
    } else {
        local(function, selected)
    };
    let (block, position) = if selected == "region" {
        (function.entry_block.0, 0)
    } else {
        function
            .blocks
            .iter()
            .find_map(|block| {
                block
                    .statements
                    .iter()
                    .position(|statement| {
                        matches!(statement,
                Statement::AssignLocal { target, .. } if *target == region)
                    })
                    .map(|position| (block.id.0, position + 1))
            })
            .expect("selected descriptor assignment")
    };
    let success = BlockId(function.blocks.len());
    let failure = BlockId(success.0 + 1);
    let error = LocalId(function.locals.len());
    let mut error_local = template.0;
    error_local.id = error;
    function.locals.push(error_local);
    let statements = function.blocks[block].statements.split_off(position);
    let terminator = std::mem::replace(
        &mut function.blocks[block].terminator,
        Terminator::CheckedIo {
            operation: CheckedIoOperation::PreparePendingFile {
                file,
                request,
                region,
            },
            result: None,
            error,
            success,
            failure,
            span: template.1,
        },
    );
    function.blocks.push(BasicBlock {
        id: success,
        statements,
        terminator,
    });
    // This forged helper catches configuration errors so its original callable
    // contract stays infallible; the test changes only selected-storage proof.
    function.blocks.push(BasicBlock {
        id: failure,
        statements: vec![Statement::DropError { local: error }],
        terminator: Terminator::Jump(success),
    });
}

fn reject(program: &Program, fragment: &str) {
    let error = doriac::mir_validation::validate_program(program)
        .expect_err("forged pending storage must not validate");
    assert!(error.message.contains(fragment), "{error:?}");
}

fn repeated_call_fixture(second: bool) -> Program {
    repeated_call_fixture_at_depth(second, 1)
}

fn repeated_call_fixture_at_depth(second: bool, depth: usize) -> Program {
    use doriac::mir::{ClassExpression, Rvalue};
    let selected = if second { "second" } else { "first" };
    let untouched = if second { "left" } else { "right" };
    let mut left = "$this->left".to_string();
    let mut right = "$this->right".to_string();
    for _ in 0..depth {
        left = format!("same({left})");
        right = format!("same({right})");
    }
    let helpers = format!(
        r#"{HELPER}
class Buffer {{ writable Bytes $bytes = Bytes::allocate(4); }}
function same(writable Buffer $buffer): Buffer {{ return $buffer; }}
function choose(writable ReadableFile $file, writable ReadRequest $request,
    writable Buffer $first, writable Buffer $second): void {{
    select($file, $request, ${selected}->bytes->getWritableRegion(0, 4));
}}
"#
    );
    let mut program = lower_with_owned_fields(
        &format!(
            r#"
choose($this->file, $request, {left}, {right});
$this->{untouched}->bytes[0] = 1;
"#
        ),
        &helpers,
        "internal writable Buffer $left = new Buffer(); internal writable Buffer $right = new Buffer();",
        "",
        "",
    );
    let same = program
        .functions
        .iter()
        .find(|function| function.name == "same")
        .unwrap()
        .id;
    let choose = program
        .functions
        .iter()
        .find(|function| function.name == "choose")
        .unwrap()
        .id;
    select_region(&mut program, "region");
    doriac::mir_validation::validate_program(&program).unwrap();
    let caller = program.functions.iter().position(|function| function.blocks.iter().flat_map(|block| &block.statements)
        .any(|statement| matches!(statement, Statement::CallVoid { function, .. } if *function == choose))).unwrap();
    let staged: std::collections::HashMap<_, _> = program.functions[caller]
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            Statement::AssignLocal {
                target,
                value: value @ Rvalue::Class(ClassExpression::Call { function, .. }),
            } if *function == same => Some((*target, value.clone())),
            _ => None,
        })
        .collect();
    fn inline(
        value: &mut Rvalue,
        staged: &std::collections::HashMap<doriac::mir::LocalId, Rvalue>,
    ) {
        if let Rvalue::Class(ClassExpression::Local { local, .. }) = value {
            if let Some(call) = staged.get(local) {
                *value = call.clone();
            }
        }
        if let Rvalue::Class(ClassExpression::Call { args, .. }) = value {
            for argument in args {
                inline(argument, staged);
            }
        }
    }
    let args = program.functions[caller]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| {
            if let Statement::CallVoid { function, args, .. } = statement {
                (*function == choose).then_some(args)
            } else {
                None
            }
        })
        .expect("lowered choose invocation");
    // Validate both the ordered production lowering and equivalent inline MIR;
    // forged arguments below must not depend on the lowerer's choice of form.
    for value in &mut args[2..] {
        inline(value, &staged);
        assert!(
            matches!(value, Rvalue::Class(ClassExpression::Call { function, .. }) if *function == same),
            "actual lowered argument: {value:#?}"
        );
    }
    for block in &mut program.functions[caller].blocks {
        block.statements.retain(|statement| {
            !matches!(statement,
                Statement::AssignLocal { target, .. } | Statement::EndBorrow { local: target }
                    if staged.contains_key(target))
        });
    }
    program
}

#[test]
fn repeated_inline_helper_results_preserve_each_owned_field_origin() {
    for second in [false, true] {
        for depth in [1, 2] {
            let program = repeated_call_fixture_at_depth(second, depth);
            doriac::mir_validation::validate_program(&program).unwrap();
        }
    }
}

#[test]
fn forged_repeated_inline_arguments_still_reject_two_loans_of_one_owned_field() {
    let mut program = repeated_call_fixture(false);
    let choose = program
        .functions
        .iter()
        .find(|function| function.name == "choose")
        .unwrap()
        .id;
    let mut changed = 0;
    for statement in program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::CallVoid { function, args, .. } = statement {
            if *function == choose {
                args[3] = args[2].clone();
                changed += 1;
            }
        }
    }
    assert_eq!(changed, 1);
    reject(&program, "overlapping writable borrows");
}

#[test]
fn forged_repeated_inline_arguments_cannot_hide_the_selected_sibling() {
    for second in [false, true] {
        let mut program = repeated_call_fixture(second);
        let choose = program
            .functions
            .iter()
            .find(|function| function.name == "choose")
            .unwrap()
            .id;
        let mut changed = 0;
        for statement in program
            .functions
            .iter_mut()
            .flat_map(|function| &mut function.blocks)
            .flat_map(|block| &mut block.statements)
        {
            if let Statement::CallVoid { function, args, .. } = statement {
                if *function == choose {
                    args.swap(2, 3);
                    changed += 1;
                }
            }
        }
        assert_eq!(changed, 1);
        reject(&program, "before pending retirement");
    }
}

#[test]
fn named_helper_forwarding_preserves_the_group_region_without_source_effects() {
    let mut program = lower(
        "select(region: $destination, request: $request, file: $this->file);",
        HELPER,
    );
    select_region(&mut program, "region");
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn helper_local_storage_is_rejected_despite_a_live_descriptor_and_omitted_plan() {
    let mut program = lower(
        "select($this->file, $request, $destination);",
        r#"
function select(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): void {
    writable Bytes $bytes = Bytes::allocate(4);
    let writable $localRegion = $bytes->getWritableRegion(0, 4);
}
"#,
    );
    select_region(&mut program, "localRegion");
    reject(&program, "callback-local");
}

#[test]
fn standard_pending_selection_keeps_group_origin_and_rejects_helper_local_storage() {
    for selected in ["region", "localRegion"] {
        let mut program = lower(
            "select($this->file, $request, $destination);",
            r#"
function select(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): void {
    writable Bytes $bytes = Bytes::allocate(4);
    let writable $localRegion = $bytes->getWritableRegion(0, 4);
}
"#,
        );
        select_region(&mut program, selected);
        for block in &mut helper(&mut program).blocks {
            if let doriac::mir::Terminator::CheckedIo { operation, .. } = &mut block.terminator {
                if let doriac::mir::CheckedIoOperation::PreparePendingFile {
                    request, region, ..
                } = *operation
                {
                    *operation = doriac::mir::CheckedIoOperation::PreparePendingStandard {
                        stream: doriac::mir::StandardIoStream::Input,
                        request,
                        region,
                    };
                }
            }
        }
        if selected == "region" {
            doriac::mir_validation::validate_program(&program).unwrap();
        } else {
            reject(&program, "callback-local");
        }
    }
}

fn inherited_preparation_fixture(redeclared: bool, local_storage: bool) -> Program {
    let redeclaration = if redeclared {
        r#"
interface Prepared extends PendingReadableStream {
    writable function prepareRead(writable ReadRequest $request,
        writable WritableByteRegion $destination): void throws IoError;
}
"#
    } else {
        ""
    };
    let helpers = format!(
        r#"{redeclaration}
function select(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): void {{
    writable Bytes $bytes = Bytes::allocate(4);
    let writable $localRegion = $bytes->getWritableRegion(0, 4);
}}
"#
    );
    let mut program = lower_with_contract(
        "select($this->file, $request, $destination);",
        &helpers,
        "",
        "",
        "",
        if redeclared {
            "Prepared"
        } else {
            "PendingReadableStream"
        },
    );
    if redeclared {
        let method = program
            .interface_types
            .iter()
            .find(|interface| interface.name == "Prepared")
            .unwrap()
            .methods
            .iter()
            .find(|method| method.name == "prepareRead")
            .unwrap();
        assert_eq!(method.requirement_origins.len(), 2);
        assert_ne!(
            method.requirement_origins[0].source,
            doriac::compiler_known_stream::SOURCE_ID
        );
        assert_eq!(
            doriac::compiler_known_stream::StreamOperation::from_requirement(
                method.requirement_origins[0]
            ),
            None
        );
        assert_eq!(
            doriac::compiler_known_stream::StreamOperation::from_requirement(
                method.requirement_origins[1]
            ),
            Some(doriac::compiler_known_stream::StreamOperation::PrepareRead)
        );
    }
    select_region(
        &mut program,
        if local_storage {
            "localRegion"
        } else {
            "region"
        },
    );
    program
}

fn rename_stream_display_metadata(program: &mut Program) {
    for interface in &mut program.interface_types {
        if interface.methods.iter().any(|method| {
            method.requirement_origins.iter().any(|origin| {
                doriac::compiler_known_stream::StreamOperation::from_requirement(*origin).is_some()
            })
        }) {
            interface.name = format!("opaque-interface-{}", interface.id.0);
            for (slot, method) in interface.methods.iter_mut().enumerate() {
                method.name = format!("opaque-method-{slot}");
            }
        }
    }
}

#[test]
fn pending_storage_proof_preserves_redeclared_contracts_without_display_name_inference() {
    for redeclared in [false, true] {
        for renamed in [false, true] {
            for local_storage in [false, true] {
                let mut program = inherited_preparation_fixture(redeclared, local_storage);
                if renamed {
                    rename_stream_display_metadata(&mut program);
                }
                if local_storage {
                    reject(&program, "callback-local");
                } else {
                    doriac::mir_validation::validate_program(&program).unwrap();
                }
            }
        }
    }
}

#[test]
fn pending_write_storage_proof_preserves_redeclared_contracts_and_readonly_loans() {
    let source = r#"
use Doria\Std\Io\{PendingWritableStream, WritableFile, WriteRequest, WriteOutcome,
    ReadinessSource, ByteRegion, IoError};
interface PreparedWrite extends PendingWritableStream {
    writable function prepareWrite(writable WriteRequest $request,
        ByteRegion $source): void throws IoError;
}
function select(writable WritableFile $file, writable WriteRequest $request,
    ByteRegion $region): void {
    let $bytes = Bytes::allocate(4);
    let $localRegion = $bytes->getRegion(0, 4);
}
class Pipeline implements PreparedWrite {
    internal writable WritableFile $file;
    function __construct(parameter take WritableFile $file) { $this->file = $file; }
    ReadinessSource $readinessSource { borrowed get => $this->file->readinessSource; }
    writable function write(ByteRegion $source): WriteOutcome throws IoError {
        return $this->file->write($source);
    }
    writable function prepareWrite(writable WriteRequest $request,
        ByteRegion $source): void throws IoError {
        select($this->file, $request, $source);
    }
    writable function completeWrite(WriteRequest $request,
        ByteRegion $source): ?WriteOutcome throws IoError { return WriteOutcome::WouldBlock; }
}
function main(): void {}
"#;
    let original = doriac::lower_source_to_mir("pending-write-origins.doria", source).unwrap();
    for renamed in [false, true] {
        for local_storage in [false, true] {
            let mut program = original.clone();
            select_region(
                &mut program,
                if local_storage {
                    "localRegion"
                } else {
                    "region"
                },
            );
            if renamed {
                rename_stream_display_metadata(&mut program);
            }
            if local_storage {
                reject(&program, "callback-local");
            } else {
                doriac::mir_validation::validate_program(&program).unwrap();
            }
        }
    }
}

#[test]
fn interface_requirement_origins_cannot_be_missing_or_duplicated() {
    for missing in [false, true] {
        let mut program = inherited_preparation_fixture(true, false);
        let method = program
            .interface_types
            .iter_mut()
            .find(|interface| interface.name == "Prepared")
            .unwrap()
            .methods
            .iter_mut()
            .find(|method| method.name == "prepareRead")
            .unwrap();
        if missing {
            method.requirement_origins.clear();
            reject(&program, "no declaration origin");
        } else {
            method
                .requirement_origins
                .push(method.requirement_origins[0]);
            reject(&program, "duplicate declaration origins");
        }
    }
}

#[test]
fn callback_selection_preserves_static_receiver_and_original_erased_view() {
    use doriac::compiler_known_stream::StreamOperation as Op;
    use doriac::mir::{StreamCallbackTarget, Type};
    let original = inherited_preparation_fixture(true, false);
    doriac::mir_validation::validate_program(&original).unwrap();
    let class = original
        .classes
        .iter()
        .find(|class| class.name == "Pipeline")
        .unwrap()
        .id;
    let prepared = original
        .interface_types
        .iter()
        .find(|interface| interface.name == "Prepared")
        .unwrap()
        .id;
    let readiness = original
        .interface_types
        .iter()
        .find(|interface| interface.name == doriac::compiler_known_stream::READINESS_STREAM)
        .unwrap()
        .id;
    for renamed in [false, true] {
        let mut program = original.clone();
        if renamed {
            rename_stream_display_metadata(&mut program);
        }
        doriac::mir_validation::validate_program(&program).unwrap();
        for operation in [
            Op::Read,
            Op::ReadinessSource,
            Op::PrepareRead,
            Op::CompleteRead,
        ] {
            let concrete = program
                .stream_callback(Type::Class(class), operation)
                .unwrap();
            assert_eq!(concrete.receiver, Type::Class(class));
            assert_eq!(concrete.operation, operation);
            let StreamCallbackTarget::Static { vtable, entry } = concrete.target else {
                panic!("closed concrete callback must keep its direct entry");
            };
            assert_eq!(
                program.interface_vtables[vtable.0].methods[concrete.slot],
                entry
            );
            assert_eq!(
                program.functions[entry.0].return_type,
                program.function_types[concrete.signature.0].return_type
            );

            let erased = program
                .stream_callback(Type::Interface(prepared), operation)
                .unwrap();
            assert_eq!(erased.receiver, Type::Interface(prepared));
            assert_eq!(erased.interface, prepared);
            assert_eq!(erased.target, StreamCallbackTarget::InterfaceDescriptor);
            assert_eq!(
                program.interface_types[prepared.0].stream_operation(erased.slot),
                Some(operation)
            );
        }
        assert!(program
            .stream_callback(Type::Interface(readiness), Op::ReadinessSource)
            .is_some());
        for operation in [
            Op::Read,
            Op::Write,
            Op::PrepareRead,
            Op::PrepareWrite,
            Op::CompleteRead,
            Op::CompleteWrite,
        ] {
            assert!(program
                .stream_callback(Type::Interface(readiness), operation)
                .is_none());
        }
        assert!(program
            .stream_callback(Type::NullableClass(class), Op::Read)
            .is_none());
        assert!(program
            .stream_callback(Type::NullableInterface(prepared), Op::Read)
            .is_none());
        assert!(program.stream_callback(Type::String, Op::Read).is_none());
    }
}

#[test]
fn readiness_only_adapters_do_not_gain_structurally_similar_transfer_callbacks() {
    use doriac::compiler_known_stream::StreamOperation as Op;
    let program = lower_with_contract(
        "",
        "interface ReadyOnly extends Doria\\Std\\Io\\ReadinessStream {}",
        "",
        "",
        "",
        "ReadyOnly",
    );
    doriac::mir_validation::validate_program(&program).unwrap();
    let class = program
        .classes
        .iter()
        .find(|class| class.name == "Pipeline")
        .unwrap()
        .id;
    let receiver = doriac::mir::Type::Class(class);
    assert!(program
        .stream_callback(receiver, Op::ReadinessSource)
        .is_some());
    for operation in [
        Op::Read,
        Op::Write,
        Op::PrepareRead,
        Op::PrepareWrite,
        Op::CompleteRead,
        Op::CompleteWrite,
    ] {
        assert!(program.stream_callback(receiver, operation).is_none());
    }
}

#[test]
fn open_receiver_callback_selection_preserves_dynamic_class_dispatch() {
    use doriac::compiler_known_stream::StreamOperation as Op;
    use doriac::mir::{StreamCallbackTarget, Type};
    let source = r#"
use Doria\Std\Io\{ReadableFile, ReadinessStream, ReadinessSource};
open class Ready implements ReadinessStream {
    internal ReadableFile $file;
    function __construct(parameter take ReadableFile $file) { $this->file = $file; }
    ReadinessSource $readinessSource { borrowed get => $this->file->readinessSource; }
}
class Child extends Ready {
    function __construct(parameter take ReadableFile $file) { parent::__construct($file); }
}
function main(): void {}
"#;
    let program = doriac::lower_source_to_mir("open-stream-callback.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let class = program
        .classes
        .iter()
        .find(|class| class.name == "Ready")
        .unwrap()
        .id;
    let plan = program
        .stream_callback(Type::Class(class), Op::ReadinessSource)
        .unwrap();
    assert_eq!(plan.receiver, Type::Class(class));
    assert_eq!(plan.target, StreamCallbackTarget::ClassDescriptor);
    assert_eq!(
        program.interface_types[plan.interface.0].stream_operation(plan.slot),
        Some(Op::ReadinessSource)
    );
}

#[test]
fn nominal_callback_roles_cannot_be_forged_into_a_different_signature() {
    use doriac::compiler_known_stream::StreamOperation as Op;
    use doriac::mir::{FunctionInvocationMode, FunctionParameterMode, ReceiverMode, Type};
    let original = inherited_preparation_fixture(true, false);
    let prepared = original
        .interface_types
        .iter()
        .find(|interface| interface.name == "Prepared")
        .unwrap()
        .id;
    // Programs include only the std declarations they reach, and this one
    // reads only; take the canonical `WritableStream::write` requirement.
    let write_origin = doriac::compiler_known_stream::declarations()
        .items
        .iter()
        .find_map(|item| match item {
            doriac::ast::Item::Interface(interface)
                if interface.name == "Doria\\Std\\Io\\WritableStream" =>
            {
                interface
                    .requirements
                    .iter()
                    .find(|method| method.name == "write")
                    .map(|method| method.span)
            }
            _ => None,
        })
        .filter(|origin| Op::from_requirement(*origin) == Some(Op::Write))
        .unwrap();
    let read_result = original
        .stream_callback(Type::Interface(prepared), Op::Read)
        .unwrap()
        .signature;
    for fault in 0..8 {
        let mut invalid = original.clone();
        if fault <= 1 {
            // Keep declaration membership valid so the forged operation reaches
            // the independent stream-signature check rather than an earlier gate.
            invalid.interface_types[prepared.0]
                .requirement_origins
                .push(write_origin);
        }
        let method = invalid.interface_types[prepared.0]
            .methods
            .iter_mut()
            .find(|method| {
                method
                    .requirement_origins
                    .iter()
                    .any(|origin| Op::from_requirement(*origin) == Some(Op::PrepareRead))
            })
            .unwrap();
        let signature = method.signature;
        match fault {
            0 => method.requirement_origins.push(write_origin),
            1 => method.requirement_origins = vec![write_origin],
            2 => method.arguments.push(Type::String),
            3 => {
                method.receiver_mode = ReceiverMode::Readonly;
                invalid.function_types[signature.0].invocation_mode =
                    FunctionInvocationMode::Readonly;
                invalid.function_types[signature.0].parameters[0].mode =
                    FunctionParameterMode::Readonly;
            }
            4 => {
                invalid.function_types[signature.0].return_borrow =
                    Some(doriac::mir::ReturnBorrow {
                        source: doriac::mir::BorrowSource::Receiver,
                        writable: false,
                        kind: doriac::mir::ReturnBorrowKind::Value,
                        lifetime: doriac::mir::RetainedLifetime::LastUse,
                    })
            }
            5 => invalid.function_types[signature.0]
                .checked_effects
                .push(doriac::mir::CheckedEffect::Any),
            6 => {
                invalid.function_types[signature.0].parameters[1].mode =
                    FunctionParameterMode::Readonly
            }
            7 => {
                invalid.function_types[signature.0].return_type =
                    invalid.function_types[read_result.0].return_type
            }
            _ => unreachable!(),
        }
        let message = match fault {
            0 => "distinct nominal operations",
            1 => "canonical inputs",
            2..=4 => "canonical receiver or return loan",
            5 => "canonical checked effect",
            6 => "request loan",
            7 => "preparation must return void",
            _ => unreachable!(),
        };
        reject(&invalid, message);
    }
}

#[test]
fn a_local_closure_capturing_the_group_region_keeps_its_physical_owner() {
    let mut program = lower(
        r#"
let writable $callback = function(writable ReadableFile $file,
    writable ReadRequest $request): void with (writable $destination) {
    select($file, $request, $destination);
};
$callback($this->file, $request);
"#,
        HELPER,
    );
    select_region(&mut program, "region");
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn returned_callback_factories_do_not_join_unrelated_capture_origins() {
    let helpers = format!(
        r#"{HELPER}
function make(writable WritableByteRegion $region): function writable(writable ReadableFile, writable ReadRequest): void {{
    return function(writable ReadableFile $file, writable ReadRequest $request): void with (writable $region) {{
        select($file, $request, $region);
    }};
}}
function invoke(writable function writable(writable ReadableFile, writable ReadRequest): void $callback,
    writable ReadableFile $file, writable ReadRequest $request): void {{
    $callback($file, $request);
}}
"#
    );
    for invoked in ["safe", "unsafe"] {
        let mut program = lower(
            &format!(
                r#"
let writable $safe = make($destination);
writable Bytes $bytes = Bytes::allocate(4);
let writable $localRegion = $bytes->getWritableRegion(0, 4);
let writable $unsafe = make($localRegion);
invoke(${invoked}, $this->file, $request);
"#
            ),
            &helpers,
        );
        select_region(&mut program, "region");
        if invoked == "safe" {
            doriac::mir_validation::validate_program(&program).unwrap();
        } else {
            reject(&program, "callback-local");
        }
    }
}

#[test]
fn a_constructor_helper_preserves_actual_request_and_group_arguments() {
    let helpers = format!(
        r#"{HELPER}
class SelectNow {{
    function __construct(parameter writable ReadableFile $file,
        parameter writable ReadRequest $request,
        parameter writable WritableByteRegion $region) {{
        select($file, $request, $region);
    }}
}}
"#
    );
    let mut program = lower(
        "let $configured = new SelectNow($this->file, $request, $destination);",
        &helpers,
    );
    select_region(&mut program, "region");
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn a_taken_helper_owner_cannot_supply_pending_storage() {
    let mut program = lower(
        "select($this->file, $request, Bytes::allocate(4));",
        r#"
function select(writable ReadableFile $file, writable ReadRequest $request,
    take Bytes $bytes): void {
    let writable $owned = $bytes;
    let writable $localRegion = $owned->getWritableRegion(0, 4);
}
"#,
    );
    select_region(&mut program, "localRegion");
    reject(&program, "callback-local");
}

#[test]
fn helper_returned_region_aliases_keep_the_group_storage_origin() {
    let helpers = format!(
        r#"{HELPER}
function alias(writable WritableByteRegion $region): WritableByteRegion {{
    let $length = $region->length;
    return $region->getWritableRegion(0, $length);
}}
function forward(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): void {{
    select($file, $request, alias($region));
}}
"#
    );
    let mut program = lower("forward($this->file, $request, $destination);", &helpers);
    select_region(&mut program, "region");
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn selected_writable_storage_cannot_be_read_through_its_region() {
    let mut program = lower(
        "select($this->file, $request, $destination); let $observed = $destination[0];",
        HELPER,
    );
    select_region(&mut program, "region");
    reject(&program, "before pending retirement");
}

#[test]
fn observing_descriptor_length_does_not_access_selected_bytes() {
    let mut program = lower(
        "select($this->file, $request, $destination); let $length = $destination->length;",
        HELPER,
    );
    select_region(&mut program, "region");
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn forming_a_new_region_does_not_bypass_the_pending_storage_loan() {
    let mut program = lower(
        "select($this->file, $request, $destination); let $child = $destination->getWritableRegion(0, 4);",
        HELPER,
    );
    select_region(&mut program, "region");
    reject(&program, "before pending retirement");
}

#[test]
fn ordinary_file_io_cannot_access_selected_storage_before_retirement() {
    let helpers = format!(
        r#"{HELPER}
function idle(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): ReadOutcome throws IoError {{
    return ReadOutcome::WouldBlock;
}}
function ordinary(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): ReadOutcome throws IoError {{
    return $file->read($region);
}}
"#
    );
    let mut program = lower(
        "select($this->file, $request, $destination); let $outcome = idle($this->file, $request, $destination);",
        &helpers,
    );
    select_region(&mut program, "region");
    // Source correctly forbids blocking work in preparation. Forge the actual
    // call target only after that check, preserving its signature and checked
    // transport so the MIR proof must inspect the callee's real byte accesses.
    let idle = program
        .functions
        .iter()
        .find(|function| function.name == "idle")
        .unwrap()
        .id;
    let ordinary = program
        .functions
        .iter()
        .find(|function| function.name == "ordinary")
        .unwrap()
        .id;
    let mut changed = 0;
    for block in program
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
    {
        if let doriac::mir::Terminator::CheckedCall { function, .. } = &mut block.terminator {
            if *function == idle {
                *function = ordinary;
                changed += 1;
            }
        }
    }
    assert_eq!(changed, 1);
    reject(&program, "before pending retirement");
}

#[test]
fn a_selection_in_an_index_argument_precedes_the_actual_byte_read() {
    use doriac::mir::{
        ByteRegionExpression, ByteRegionKind, ClassExpression, IntegerExpression, Operand, Rvalue,
        ValueExpression,
    };
    use doriac::types::IntegerType;
    let helpers = format!(
        r#"{HELPER}
function selectIndex(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): int {{
    select($file, $request, $region);
    return 0;
}}
function observe(writable ReadableFile $file, writable ReadRequest $request,
    writable WritableByteRegion $region): void {{
    let $observed = $region[0];
    select($file, $request, $region);
}}
"#
    );
    let mut program = lower("observe($this->file, $request, $destination);", &helpers);
    select_region(&mut program, "region");
    let callee = program
        .functions
        .iter()
        .find(|function| function.name == "selectIndex")
        .unwrap()
        .id;
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "observe")
        .unwrap();
    let file = function.params[0];
    let doriac::mir::Type::Class(file_class) = function.locals[file.0].ty else {
        panic!("file class");
    };
    let request = function.params[1];
    let doriac::mir::Type::Class(request_class) = function.locals[request.0].ty else {
        panic!("request class");
    };
    let region = function.params[2];
    let args = vec![
        Rvalue::Class(ClassExpression::Local {
            class: file_class,
            local: file,
            transfer: false,
        }),
        Rvalue::Class(ClassExpression::Local {
            class: request_class,
            local: request,
            transfer: false,
        }),
        Rvalue::ByteRegion(ByteRegionExpression::Local {
            kind: ByteRegionKind::Writable,
            local: region,
            transfer: false,
        }),
    ];
    let mut changed = 0;
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::AssignLocal {
            value:
                Rvalue::Value(ValueExpression::Integer(IntegerExpression::Use {
                    operand: Operand::ByteRegionIndex { index, .. },
                    ..
                })),
            ..
        } = statement
        {
            **index = Rvalue::Value(ValueExpression::Integer(IntegerExpression::Call {
                span: function.source_span,
                ty: IntegerType::Int64,
                function: callee,
                args: args.clone(),
            }));
            changed += 1;
        }
    }
    assert_eq!(changed, 1);
    reject(&program, "before pending retirement");
}

#[test]
fn adapter_owned_storage_can_be_selected_without_freezing_unrelated_fields() {
    let mut program = lower(
        r#"
let writable $region = $this->buffer->getWritableRegion(0, 4);
select($this->file, $request, $region);
$this->counter++;
"#,
        HELPER,
    );
    select_region(&mut program, "region");
    doriac::mir_validation::validate_program(&program).unwrap();
}

#[test]
fn nested_selected_storage_protects_ancestors_but_not_owned_siblings() {
    let helpers = format!(
        r#"{HELPER}
class Buffers {{
    writable Bytes $left = Bytes::allocate(4);
    writable Bytes $right = Bytes::allocate(4);
}}
"#
    );
    for (access, valid) in [
        ("$this->storage->right[0] = 1;", true),
        ("$this->storage->left[0] = 1;", false),
        ("let $observed = $this->storage->left[0];", false),
        ("$this->storage = new Buffers();", false),
    ] {
        let mut program = lower_with_owned_fields(
            &format!(
                "let writable $region = $this->storage->left->getWritableRegion(0, 4);\n\
                 select($this->file, $request, $region); {access}"
            ),
            &helpers,
            "internal writable Buffers $storage = new Buffers();",
            "",
            "",
        );
        select_region(&mut program, "region");
        if valid {
            doriac::mir_validation::validate_program(&program).unwrap();
        } else {
            reject(&program, "before pending retirement");
        }
    }
}

const LINK: &str = r#"
class Link {
    writable Link $next;
    writable Bytes $left = Bytes::allocate(4);
    writable Bytes $right = Bytes::allocate(4);
    function __construct(parameter take Link $next) { $this->next = $next; }
}
"#;

#[test]
fn finite_repeated_pending_storage_paths_preserve_sibling_precision() {
    let helpers = format!("{HELPER}\n{LINK}");
    for (access, valid) in [
        ("$this->chain->next->next->right[0] = 1;", true),
        ("$this->chain->next->next->left[0] = 1;", false),
    ] {
        let mut program = lower_with_owned_fields(
            &format!(
                "let writable $region = $this->chain->next->next->left->getWritableRegion(0, 4);\n\
                 select($this->file, $request, $region); {access}"
            ),
            &helpers,
            "internal writable Link $chain;",
            ", parameter take Link $chain",
            "$this->chain = $chain;",
        );
        select_region(&mut program, "region");
        if valid {
            doriac::mir_validation::validate_program(&program).unwrap();
        } else {
            reject(&program, "before pending retirement");
        }
    }
}

#[test]
fn recursive_pending_storage_paths_terminate_and_keep_known_owner_prefixes() {
    let helpers = format!(
        r#"{HELPER}
{LINK}
function descend(writable Link $link, bool $stop, writable ReadableFile $file,
    writable ReadRequest $request): void {{
    if ($stop) {{
        let writable $region = $link->left->getWritableRegion(0, 4);
        select($file, $request, $region);
    }} else {{
        descend($link->next, true, $file, $request);
    }}
}}
"#
    );
    for (access, valid) in [
        ("$this->buffer[0] = 1;", true),
        (
            "let writable $again = $this->chain->next->left->getWritableRegion(0, 4);",
            false,
        ),
    ] {
        let mut program = lower_with_owned_fields(
            &format!("descend($this->chain, false, $this->file, $request); {access}"),
            &helpers,
            "internal writable Link $chain;",
            ", parameter take Link $chain",
            "$this->chain = $chain;",
        );
        select_region(&mut program, "region");
        if valid {
            doriac::mir_validation::validate_program(&program).unwrap();
        } else {
            reject(&program, "before pending retirement");
        }
    }
}

#[test]
fn replacing_selected_adapter_storage_is_rejected_after_descriptor_last_use() {
    let mut program = lower(
        r#"
let writable $region = $this->buffer->getWritableRegion(0, 4);
select($this->file, $request, $region);
$this->buffer = Bytes::allocate(4);
"#,
        HELPER,
    );
    select_region(&mut program, "region");
    reject(&program, "before pending retirement");
}

#[test]
fn ending_the_request_view_does_not_retire_its_storage() {
    let mut program = lower(
        r#"
let writable $region = $this->buffer->getWritableRegion(0, 4);
select($this->file, $request, $region);
$this->buffer = Bytes::allocate(4);
"#,
        HELPER,
    );
    select_region(&mut program, "region");
    let function = helper(&mut program);
    // The helper's ordinary return ends its borrowed request view. EndBorrow
    // is a byte-region-carrier instruction, not a request retirement operation.
    assert!(!function.locals[function.params[1].0].owned);
    let doriac::mir::Terminator::CheckedIo { success, .. } =
        function.blocks[function.entry_block.0].terminator
    else {
        panic!("selection uses checked preparation");
    };
    assert!(matches!(
        function.blocks[success.0].terminator,
        doriac::mir::Terminator::ReturnVoid
    ));
    reject(&program, "before pending retirement");
}
